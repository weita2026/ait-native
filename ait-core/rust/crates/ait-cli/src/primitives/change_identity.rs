use super::*;

pub(super) fn canonical_change_id(value: &str) -> Result<String, String> {
    ChangeJson::stateless().canonical_change_id(value)
}

pub(super) fn is_short_change_id(value: &str) -> bool {
    let Some(ordinal) = value.strip_prefix("C-") else {
        return false;
    };
    !ordinal.is_empty() && ordinal.bytes().all(|byte| byte.is_ascii_digit())
}

pub(super) fn is_task_id(value: &str) -> bool {
    let Some((prefix, ordinal)) = value.rsplit_once('-') else {
        return false;
    };
    prefix.ends_with('T')
        && prefix
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && !ordinal.is_empty()
        && ordinal.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Clone, Copy)]
pub(super) enum TaskChangeSelection {
    Author,
    Finish,
}

/// Select from the complete authority inventory, never from a bounded UI list
/// or the worktree's initial Change hint. Explicit Change inputs bypass this.
pub(super) fn select_task_change_reference(
    rows: &[JsonValue],
    task_id: &str,
    selection: TaskChangeSelection,
) -> Result<Option<String>, String> {
    let mut candidates = Vec::new();
    for row in rows {
        if change_task_id_from_payload(row).as_deref() != Some(task_id) {
            continue;
        }
        let status = required_string_field(row, "status")?;
        let accepted = match status.as_str() {
            "archived" | "superseded" | "canceled" | "abandoned" => continue,
            "draft" | "active" | "review" => false,
            "ready" | "review_pending" | "blocked" | "gated" | "approved" | "landable" => {
                if matches!(selection, TaskChangeSelection::Author) {
                    continue;
                }
                false
            }
            "landed" => {
                if matches!(selection, TaskChangeSelection::Author) {
                    continue;
                }
                true
            }
            _ => return Err(format!("Task {task_id} has an unsupported work status: {status}. Inspect `ait task audit {task_id}` before continuing.")),
        };
        let reference = change_reference_from_payload(row, None)?;
        // Validate a supplied compound reference against its owning Task too.
        change_reference_for_context(Some(task_id), &reference)?;
        candidates.push((reference, accepted));
    }
    if candidates.iter().any(|(_, accepted)| !accepted) {
        candidates.retain(|(_, accepted)| !accepted);
    }
    if candidates.len() > 1 {
        let kind = match selection {
            TaskChangeSelection::Author => "writable",
            TaskChangeSelection::Finish => "finishable",
        };
        return Err(format!(
            "Task {task_id} has multiple internal {kind} work records; no target was selected. Inspect `ait task audit {task_id} --json`."
        ));
    }
    Ok(candidates.pop().map(|(reference, _)| reference))
}

pub(super) fn change_reference_for_context(
    task_id: Option<&str>,
    change_id: &str,
) -> Result<String, String> {
    let requested = normalized_text(Some(change_id))
        .ok_or_else(|| "change_id must not be empty.".to_string())?;
    let canonical = canonical_change_id(&requested)?;
    if requested != canonical {
        if let (Some(expected_task_id), Some((actual_task_id, child))) =
            (normalized_text(task_id), requested.rsplit_once('/'))
        {
            if child == canonical && actual_task_id != expected_task_id {
                return Err(format!(
                    "Change reference `{requested}` belongs to task `{actual_task_id}`, not `{expected_task_id}`."
                ));
            }
        }
        return Ok(requested);
    }
    ChangeJson::stateless().rolling_server_change_id(task_id, &canonical)
}

pub(super) fn change_reference_from_payload(
    payload: &JsonValue,
    fallback: Option<&str>,
) -> Result<String, String> {
    if let Some(change_ref) = string_field(payload, "change_ref") {
        return Ok(change_ref);
    }
    let change_id = required_string_field(payload, "change_id")?;
    let canonical = canonical_change_id(&change_id)?;
    if change_id != canonical {
        return Ok(change_id);
    }
    if let Some(task_id) = string_field(payload, "task_id") {
        return change_reference_for_context(Some(&task_id), &canonical);
    }
    if let Some(fallback) = normalized_text(fallback) {
        if canonical_change_id(&fallback)? == canonical && fallback != canonical {
            return Ok(fallback);
        }
    }
    ChangeJson::stateless().rolling_server_change_id(None, &canonical)
}

pub(super) fn payload_belongs_to_change(
    payload: &JsonValue,
    expected_change_id: &str,
    expected_change_ref: &str,
) -> bool {
    if let Some(change_ref) = string_field(payload, "change_ref") {
        return change_ref == expected_change_ref;
    }
    let Some(raw_change_id) = string_field(payload, "change_id") else {
        return false;
    };
    if raw_change_id == expected_change_ref {
        return true;
    }
    if expected_change_id == expected_change_ref && raw_change_id == expected_change_id {
        return true;
    }
    let Some(task_id) = string_field(payload, "task_id") else {
        return false;
    };
    change_reference_for_context(Some(&task_id), &raw_change_id)
        .is_ok_and(|actual| actual == expected_change_ref)
}

fn resolve_task_change_input(
    repo: &RepoRuntime,
    requested: &str,
    local: bool,
    remote_name: Option<&str>,
    selection: TaskChangeSelection,
) -> Result<String, String> {
    if !is_task_id(requested) {
        return Ok(requested.to_string());
    }
    if !repo.change_uses_local_scope(local, remote_name) {
        return super::workflow::workflow_task_change_reference(repo, requested, remote_name);
    }
    let task = task_show(repo, requested, true, None)?;
    if string_field(&task, "task_id").as_deref() != Some(requested) {
        return Err(format!(
            "Task lookup for {requested} returned another Task."
        ));
    }
    let rows = change_list(repo, true, None)?;
    let rows = rows.as_array().ok_or("Change inventory must be an array")?;
    select_task_change_reference(rows, requested, selection)?.ok_or_else(|| {
        format!(
            "Task {requested} has no applicable work. Inspect `ait task audit {requested} --json`."
        )
    })
}

pub fn resolve_task_author_change_input(
    repo: &RepoRuntime,
    requested: &str,
    local: bool,
    remote_name: Option<&str>,
) -> Result<String, String> {
    resolve_task_change_input(
        repo,
        requested,
        local,
        remote_name,
        TaskChangeSelection::Author,
    )
}

pub fn resolve_task_finish_change_input(
    repo: &RepoRuntime,
    requested: &str,
    local: bool,
    remote_name: Option<&str>,
) -> Result<String, String> {
    resolve_task_change_input(
        repo,
        requested,
        local,
        remote_name,
        TaskChangeSelection::Finish,
    )
}

pub(super) fn inspect_task_change_reference(
    rows: &[JsonValue],
    task_id: &str,
) -> Result<String, String> {
    let mut references = Vec::new();
    for row in rows {
        if change_task_id_from_payload(row).as_deref() != Some(task_id)
            || row["tombstoned"].as_bool() == Some(true)
        {
            continue;
        }
        let reference = change_reference_from_payload(row, None)?;
        change_reference_for_context(Some(task_id), &reference)?;
        references.push(reference);
    }
    match references.as_slice() {
        [reference] => Ok(reference.clone()),
        [] => Err(format!("Task {task_id} has no Change.")),
        _ => Err(format!(
            "Task {task_id} has multiple Changes; inspect an exact {task_id}/C-## reference."
        )),
    }
}

pub fn resolve_task_inspect_change_input(
    repo: &RepoRuntime,
    requested: &str,
    local: bool,
    remote_name: Option<&str>,
) -> Result<String, String> {
    let use_local = repo.change_uses_local_scope(local, remote_name);
    if !is_task_id(requested) {
        if !use_local
            && requested
                .split_once('/')
                .is_some_and(|(owner, _)| owner.starts_with('L') && is_task_id(owner))
        {
            return Err("Use the exact remote Task/Change reference; local and remote revision ordinals do not establish correspondence.".into());
        }
        return Ok(requested.into());
    }
    if use_local {
        task_show(repo, requested, true, None)?;
        let rows = change_list(repo, true, None)?;
        return inspect_task_change_reference(
            rows.as_array().ok_or("Change inventory must be an array")?,
            requested,
        );
    }
    let task_id = if requested.starts_with('L') {
        resolve_remote_task_reference(repo, requested, remote_name)?
    } else {
        requested.into()
    };
    if task_id.starts_with('L') {
        return Err(format!(
            "Task {requested} has no published remote Task identity."
        ));
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut remote = http_task_remote(repo, &remote_row)?;
    remote
        .get_task(&task_id, Some(&repo_name))
        .map_err(|error| error.to_string())?;
    let rows = remote
        .list_changes(&repo_name)
        .map_err(|error| error.to_string())?;
    inspect_task_change_reference(&rows, &task_id)
}

pub fn resolve_task_remote_change_input(
    repo: &RepoRuntime,
    requested: &str,
    remote_name: Option<&str>,
) -> Result<String, String> {
    super::workflow::workflow_task_change_reference(repo, requested, remote_name)
}

pub(crate) fn resolve_public_patchset_input(
    repo: &RepoRuntime,
    requested: &str,
    remote_name: Option<&str>,
) -> Result<String, String> {
    let requested = normalized_text(Some(requested))
        .ok_or_else(|| "Patchset ID must be non-empty.".to_string())?;
    let Some((task_id, patchset_ordinal)) =
        ait_core::public_references::split_public_patchset_reference(&requested)
    else {
        return Ok(requested);
    };
    let task = resolve_remote_task_reference(repo, task_id, remote_name)?;
    Ok(format!("{task}/{patchset_ordinal}"))
}

pub(crate) fn resolve_remote_task_reference(
    repo: &RepoRuntime,
    task_id: &str,
    remote_name: Option<&str>,
) -> Result<String, String> {
    let namespace = ait_core::workflow_primitives::workflow_origin_namespace_prefix(
        "L",
        Some(&repo.id_namespace_prefix()),
    )?;
    if !task_id.starts_with(&format!("{namespace}T-")) {
        return Ok(task_id.to_string());
    }
    let task = task_local_read_with_task_store(&repo.task_store()?, task_id)?;
    let selected = normalized_text(remote_name)
        .or_else(|| repo.default_remote_name())
        .ok_or("Task publication requires a configured remote")?;
    if string_field(&task, "published_remote_name").as_deref() != Some(&selected) {
        return Err(format!(
            "Task {task_id} has no publication to the selected remote {selected}."
        ));
    }
    required_string_field(&task, "published_task_id")
}

pub fn resolve_task_remote_revision_input(
    repo: &RepoRuntime,
    requested: &str,
    explicit_patchset_id: Option<&str>,
    remote_name: Option<&str>,
) -> Result<(String, String), String> {
    let owner = if requested.contains("/P-") {
        requested
            .rsplit_once("/P-")
            .ok_or("Patchset has no Task owner")?
            .0
            .to_string()
    } else {
        resolve_task_remote_change_input(repo, requested, remote_name)?
    };
    let task_id = if is_task_id(&owner) {
        owner.as_str()
    } else {
        owner.rsplit_once('/').ok_or("Change has no Task owner")?.0
    };
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let task = task_remote
        .get_task(task_id, Some(&repo_name))
        .map_err(|err| err.to_string())?;
    let current = required_string_field(&task, "current_patchset_id")?;
    let patchset_id = explicit_patchset_id.unwrap_or(if requested.contains("/P-") {
        requested
    } else {
        &current
    });
    let patchset = patchset_show(repo, patchset_id, remote_name)?;
    let association =
        ait_core::workflow_expected_identity::validate_patchset_owner_fields(&patchset, task_id)?;
    if !is_task_id(&owner) && association.as_deref() != Some(owner.as_str()) {
        return Err(
            "Patchset does not belong to the exact requested Change; no review was recorded".into(),
        );
    }
    let expected = ait_core::workflow_expected_identity::WorkflowExpectedIdentity {
        repository_index: 0,
        patchset_id: current,
        base_snapshot_id: required_string_field(&patchset, "base_snapshot_id")?,
        revision_snapshot_id: required_string_field(&patchset, "revision_snapshot_id")?,
    };
    expected.validate_patchset(0, &patchset)?;
    Ok((
        association.unwrap_or_else(|| task_id.to_string()),
        required_string_field(&patchset, "patchset_id")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspection_uses_all_retained_changes_without_finish_eligibility() {
        for status in ["canceled", "archived", "completed", "landed", "draft"] {
            let rows = vec![
                json!({"task_id":"LT-0003","change_id":"C-01","change_ref":"LT-0003/C-01","status":status}),
            ];
            assert_eq!(
                inspect_task_change_reference(&rows, "LT-0003").unwrap(),
                "LT-0003/C-01"
            );
            assert!(inspect_task_change_reference(&rows, "LT-0001")
                .unwrap_err()
                .contains("has no Change"));
            let mut multiple = rows;
            multiple.push(json!({"task_id":"LT-0003","change_id":"C-02","change_ref":"LT-0003/C-02","status":"canceled"}));
            assert!(inspect_task_change_reference(&multiple, "LT-0003")
                .unwrap_err()
                .contains("exact LT-0003/C-##"));
        }
    }

    #[test]
    fn task_selection_uses_complete_inventory_and_ignores_terminal_siblings() {
        let mut rows = (1..=30)
            .map(|i| {
                json!({
                    "task_id": format!("RT-{i}"), "change_id": "C-01", "status": "active",
                })
            })
            .collect::<Vec<_>>();
        rows.extend([
            json!({"task_id":"RT-31", "change_id":"C-01", "status":"superseded"}),
            json!({"task_id":"RT-31", "change_id":"C-02", "status":"landed"}),
            json!({"task_id":"RT-31", "change_id":"C-03", "status":"active"}),
        ]);
        for selection in [TaskChangeSelection::Author, TaskChangeSelection::Finish] {
            assert_eq!(
                select_task_change_reference(&rows, "RT-31", selection)
                    .unwrap()
                    .as_deref(),
                Some("RT-31/C-03")
            );
        }
        rows.push(json!({"task_id":"RT-31", "change_id":"C-04", "status":"draft"}));
        assert!(
            select_task_change_reference(&rows, "RT-31", TaskChangeSelection::Finish)
                .unwrap_err()
                .contains("multiple internal finishable")
        );
        rows.pop();
        rows.last_mut().unwrap()["change_ref"] = json!("RT-32/C-03");
        assert!(select_task_change_reference(&rows, "RT-31", TaskChangeSelection::Author).is_err());
    }

    #[test]
    fn completed_task_selection_does_not_guess_among_accepted_changes() {
        let rows = vec![
            json!({"task_id":"LT-1", "change_id":"C-01", "status":"landed"}),
            json!({"task_id":"LT-1", "change_id":"C-02", "status":"landed"}),
        ];
        assert!(select_task_change_reference(&rows, "LT-1", TaskChangeSelection::Finish).is_err());
        assert_eq!(
            select_task_change_reference(&rows, "LT-1", TaskChangeSelection::Author).unwrap(),
            None
        );
        let unknown = vec![json!({"task_id":"LT-1", "change_id":"C-01", "status":"unknown"})];
        assert!(
            select_task_change_reference(&unknown, "LT-1", TaskChangeSelection::Finish).is_err()
        );
    }

    #[test]
    fn same_short_change_id_isolated_by_derived_reference() {
        let first = json!({
            "change_id": "C-01",
            "change_ref": "RT-1/C-01",
        });
        assert!(payload_belongs_to_change(&first, "C-01", "RT-1/C-01"));
        assert!(!payload_belongs_to_change(&first, "C-01", "RT-2/C-01"));
    }

    #[test]
    fn raw_composite_owner_is_accepted_but_unscoped_short_owner_fails_closed() {
        assert!(payload_belongs_to_change(
            &json!({"change_id": "RT-1/C-01"}),
            "C-01",
            "RT-1/C-01"
        ));
        assert!(!payload_belongs_to_change(
            &json!({"change_id": "C-01"}),
            "C-01",
            "RT-1/C-01"
        ));
        assert!(payload_belongs_to_change(
            &json!({"change_id": "C-01", "task_id": "RT-1"}),
            "C-01",
            "RT-1/C-01"
        ));
    }

    #[test]
    fn explicit_reference_rejects_conflicting_task_context() {
        let err = change_reference_for_context(Some("RT-1"), "RT-2/C-01")
            .expect_err("task mismatch must fail closed");
        assert!(err.contains("belongs to task `RT-2`, not `RT-1`"));
    }
}
