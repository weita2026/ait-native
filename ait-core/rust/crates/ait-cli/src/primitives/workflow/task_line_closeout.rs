use super::task_land::{task_land_task_id, task_land_task_status};
use super::*;

fn task_land_unknown_line(error: &str) -> bool {
    error.contains("Unknown line") || error.contains("failed: 404")
}

fn task_land_bound_line_row(
    task_id: &str,
    row: JsonValue,
    binding_source: &str,
) -> Result<JsonValue, String> {
    let line_id = required_string_field(&row, "line_id")?;
    let line_name = required_string_field(&row, "line_name")?;
    Ok(json!({
        "task_id": task_id,
        "line_id": line_id,
        "line_name": line_name,
        "head_snapshot_id": string_field(&row, "head_snapshot_id"),
        "status": string_field(&row, "status").unwrap_or_else(|| "active".to_string()),
        "binding_source": binding_source,
    }))
}

pub(in crate::primitives) fn task_land_capture_bound_line(
    repo: &RepoRuntime,
    task_id: &str,
) -> Result<Option<JsonValue>, String> {
    let root_repo = workflow_root_repo(repo)?;
    let candidates = task_feature_line_candidates(task_id)?;
    let allowed = candidates
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();

    if let Some(metadata) = workflow_find_bound_task_worktree_metadata(&root_repo, task_id)? {
        if let Some(line_name) = string_field(&metadata, "line_name")
            .or_else(|| string_field(&metadata, "registered_line_name"))
        {
            if !allowed.contains(line_name.as_str()) {
                return Err(format!(
                    "Task {task_id} worktree is bound to `{line_name}`, which is not an exact task-derived feature Line candidate."
                ));
            }
            let row = local_line_row(&root_repo, &line_name).map_err(|error| {
                format!(
                    "Task {task_id} worktree names bound Line `{line_name}`, but that Line cannot be read: {error}"
                )
            })?;
            return task_land_bound_line_row(task_id, row, "bound_worktree_registry").map(Some);
        }
    }

    let mut matched = Vec::new();
    for line_name in candidates {
        match local_line_row(&root_repo, &line_name) {
            Ok(row) => matched.push(row),
            Err(error) if task_land_unknown_line(&error) => {}
            Err(error) => {
                return Err(format!(
                    "Failed to inspect task-derived feature Line `{line_name}` for Task {task_id}: {error}"
                ));
            }
        }
    }
    if matched.len() > 1 {
        let mut active = matched
            .iter()
            .filter(|row| string_field(row, "status").as_deref().unwrap_or("active") == "active")
            .cloned()
            .collect::<Vec<_>>();
        if active.len() == 1 {
            return task_land_bound_line_row(
                task_id,
                active.pop().expect("one active Task feature Line"),
                "task_identity_fallback",
            )
            .map(Some);
        }
        let names = matched
            .iter()
            .filter_map(|row| string_field(row, "line_name"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Task {task_id} has multiple task-derived feature Lines ({names}); refusing ambiguous closeout."
        ));
    }
    matched
        .pop()
        .map(|row| task_land_bound_line_row(task_id, row, "task_identity_fallback"))
        .transpose()
}

fn task_land_remote_revision_snapshot_id(output: &JsonValue) -> Option<String> {
    output
        .get("patchset")
        .and_then(|patchset| string_field(patchset, "revision_snapshot_id"))
        .or_else(|| string_field(output, "patchset_revision_snapshot_id"))
        .or_else(|| workflow_nested_text(output, "freshness", "patchset_revision_snapshot_id"))
        .or_else(|| {
            workflow_nested_text(output, "patchset_refresh", "patchset_revision_snapshot_id")
        })
}

fn task_land_landed_result_snapshot_id(output: &JsonValue) -> Option<String> {
    output
        .get("change")
        .and_then(|change| change.get("landing_summary"))
        .and_then(|summary| summary.get("result"))
        .and_then(|result| string_field(result, "landed_snapshot_id"))
        .or_else(|| {
            output
                .get("landing_summary")
                .and_then(|summary| summary.get("result"))
                .and_then(|result| string_field(result, "landed_snapshot_id"))
        })
        .or_else(|| {
            output
                .get("applied_actions")
                .and_then(JsonValue::as_array)?
                .iter()
                .rev()
                .find_map(|action| {
                    let result = action.get("result")?;
                    string_field(result, "landed_snapshot_id").or_else(|| {
                        result
                            .get("result")
                            .and_then(|nested| string_field(nested, "landed_snapshot_id"))
                    })
                })
        })
}

fn task_land_expected_revision_snapshot_id(
    output: &JsonValue,
    use_local_scope: bool,
) -> Option<String> {
    if use_local_scope {
        string_field(output, "landed_snapshot_id")
            .or_else(|| task_land_remote_revision_snapshot_id(output))
            .or_else(|| task_land_landed_result_snapshot_id(output))
    } else {
        task_land_remote_revision_snapshot_id(output)
            .or_else(|| task_land_landed_result_snapshot_id(output))
    }
}

fn task_land_selected_patchset_id(output: &JsonValue) -> Result<String, String> {
    let selected = output
        .get("change")
        .and_then(|change| string_field(change, "selected_patchset_id"));
    let projected = output
        .get("patchset")
        .and_then(|patchset| string_field(patchset, "patchset_id"))
        .or_else(|| string_field(output, "patchset_id"));
    if let (Some(selected), Some(projected)) = (selected.as_deref(), projected.as_deref()) {
        if selected != projected {
            return Err(format!(
                "Task finish selected Patchset `{selected}`, but the finish result names `{projected}`."
            ));
        }
    }
    selected.or(projected).ok_or_else(|| {
        "Task finish completed, but its selected Patchset identity is unavailable for feature-Line closeout."
            .to_string()
    })
}

fn task_land_change_identity(output: &JsonValue) -> Result<(String, String), String> {
    let change = output
        .get("change")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            "Task finish completed, but its recorded Change is unavailable for feature-Line cleanup."
                .to_string()
        })?;
    let change_id = required_string_field(change, "change_id")?;
    let change_ref = change_reference_from_payload(change, None)?;
    Ok((change_id, change_ref))
}

pub(in crate::primitives) fn task_land_selected_patchset_revision_with_closeout_remote<R>(
    closeout_remote: &mut R,
    repo_name: &str,
    output: &JsonValue,
) -> Result<String, String>
where
    R: TaskWorkflowPatchsetReader + ?Sized,
{
    if output
        .get("patchset_is_authoritative")
        .and_then(JsonValue::as_bool)
        != Some(true)
    {
        return Err(
            "Task finish did not confirm the selected Patchset as its final revision.".to_string(),
        );
    }
    let patchset_id = task_land_selected_patchset_id(output)?;
    let (change_id, change_ref) = task_land_change_identity(output)?;
    let patchset = workflow_land_patchset_read_with_closeout_remote(
        closeout_remote,
        repo_name,
        &patchset_id,
        Some(&change_ref),
    )?;
    let returned_patchset_id = required_string_field(&patchset, "patchset_id")?;
    if returned_patchset_id != patchset_id {
        return Err(format!(
            "Selected Patchset lookup for `{patchset_id}` returned `{returned_patchset_id}`."
        ));
    }
    if !payload_belongs_to_change(&patchset, &change_id, &change_ref) {
        return Err(format!(
            "Selected Patchset `{patchset_id}` does not belong to authoritative Change `{change_ref}`."
        ));
    }
    string_field(&patchset, "revision_snapshot_id").ok_or_else(|| {
        format!("Selected Patchset `{patchset_id}` does not expose its accepted revision Snapshot.")
    })
}

fn task_land_expected_revision_snapshot_id_for_closeout(
    repo: &RepoRuntime,
    output: &JsonValue,
    use_local_scope: bool,
    remote_name: Option<&str>,
) -> Result<String, String> {
    if let Some(snapshot_id) = task_land_expected_revision_snapshot_id(output, use_local_scope) {
        return Ok(snapshot_id);
    }
    if use_local_scope {
        return Err(
            "Task finish completed, but the accepted revision Snapshot is unavailable for feature-Line closeout."
                .to_string(),
        );
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    task_land_selected_patchset_revision_with_closeout_remote(
        &mut closeout_remote,
        &repo_name,
        output,
    )
}

fn task_land_target_line(output: &JsonValue) -> Option<String> {
    string_field(output, "target_line")
        .or_else(|| workflow_nested_text(output, "landing_summary", "target_line"))
        .or_else(|| {
            output
                .get("landing_summary")
                .and_then(|summary| summary.get("result"))
                .and_then(|result| string_field(result, "target_line"))
        })
        .or_else(|| {
            output
                .get("change")
                .and_then(|change| string_field(change, "target_line"))
        })
        .or_else(|| {
            output
                .get("change")
                .and_then(|change| string_field(change, "base_line"))
        })
}

fn task_land_worktree_cleanup_allows_line_closeout(output: &JsonValue) -> Result<(), String> {
    let Some(cleanup) = output
        .get("bound_worktree_cleanup")
        .and_then(JsonValue::as_object)
    else {
        return Ok(());
    };
    let status = cleanup
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let reason = cleanup
        .get("reason")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    if status == "failed" {
        return Err(format!(
            "Bound worktree cleanup failed before Line closeout{}.",
            if reason.is_empty() {
                String::new()
            } else {
                format!(": {reason}")
            }
        ));
    }
    if status == "skipped" && reason == "current_worktree" {
        return Err(
            "The current bound worktree remains registered, so its feature Line cannot be archived."
                .to_string(),
        );
    }
    Ok(())
}

fn task_land_registered_line_users(
    repo: &RepoRuntime,
    line_name: &str,
) -> Result<Vec<String>, String> {
    Ok(workflow_registered_worktree_metadata_rows(repo)?
        .into_iter()
        .filter_map(|(worktree_name, metadata)| {
            let uses_line = ["line_name", "registered_line_name", "current_line"]
                .into_iter()
                .filter_map(|key| metadata_string(&metadata, key))
                .any(|candidate| candidate == line_name);
            uses_line.then_some(worktree_name)
        })
        .collect())
}

fn task_land_validate_local_bound_line(
    repo: &RepoRuntime,
    candidate: &JsonValue,
    expected_revision_snapshot_id: &str,
    target_line: Option<&str>,
    allow_empty_head: bool,
) -> Result<JsonValue, String> {
    let line_id = required_string_field(candidate, "line_id")?;
    let line_name = required_string_field(candidate, "line_name")?;
    let default_line = repo.default_line_name();
    if line_name == default_line {
        return Err(format!(
            "Task feature Line `{line_name}` is the default Line and cannot be archived."
        ));
    }
    if target_line == Some(line_name.as_str()) {
        return Err(format!(
            "Task feature Line `{line_name}` is also the Land target and cannot be archived."
        ));
    }
    let current_line = repo.current_line_name()?;
    if current_line == line_name {
        return Err(format!(
            "Task feature Line `{line_name}` is still the current root Line and cannot be archived."
        ));
    }
    let users = task_land_registered_line_users(repo, &line_name)?;
    if !users.is_empty() {
        return Err(format!(
            "Task feature Line `{line_name}` is still registered to worktree(s): {}.",
            users.join(", ")
        ));
    }

    let current = local_line_row(repo, &line_name)?;
    let current_line_id = required_string_field(&current, "line_id")?;
    if current_line_id != line_id {
        return Err(format!(
            "Task feature Line `{line_name}` changed stable identity from `{line_id}` to `{current_line_id}`; refusing closeout."
        ));
    }
    let current_head = string_field(&current, "head_snapshot_id");
    if current_head.as_deref() != Some(expected_revision_snapshot_id)
        && !(allow_empty_head && current_head.is_none())
    {
        return Err(format!(
            "Task feature Line `{line_name}` head drifted from accepted revision `{expected_revision_snapshot_id}` to `{}`; refusing to hide unlanded content.",
            current_head.as_deref().unwrap_or("<none>")
        ));
    }
    Ok(current)
}

pub(in crate::primitives) fn task_land_archive_local_bound_line(
    repo: &RepoRuntime,
    candidate: &JsonValue,
    expected_revision_snapshot_id: &str,
    target_line: Option<&str>,
    allow_empty_head: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait task finish feature line closeout", || {
        let current = task_land_validate_local_bound_line(
            repo,
            candidate,
            expected_revision_snapshot_id,
            target_line,
            allow_empty_head,
        )?;
        let line_name = required_string_field(candidate, "line_name")?;
        let empty_head = string_field(&current, "head_snapshot_id").is_none();
        let already_archived = string_field(&current, "status").as_deref() == Some("archived");
        let archived = if already_archived {
            current
        } else {
            archive_local_line(repo, &line_name)?
        };
        let archived_line_id = required_string_field(&archived, "line_id")?;
        let expected_line_id = required_string_field(candidate, "line_id")?;
        let archived_head = string_field(&archived, "head_snapshot_id");
        if archived_line_id != expected_line_id
            || (archived_head.as_deref() != Some(expected_revision_snapshot_id)
                && !(allow_empty_head && archived_head.is_none()))
            || string_field(&archived, "status").as_deref() != Some("archived")
        {
            return Err(format!(
                "Local feature Line `{line_name}` archive result did not preserve its stable identity, accepted head, and archived status."
            ));
        }
        Ok(json!({
            "status": if already_archived { "already_archived" } else { "archived" },
            "scope": "local",
            "head_state": if empty_head { "empty_remote_placeholder" } else { "accepted_revision" },
            "line": archived,
        }))
    })
}

fn task_land_remote_line_optional<R>(
    task_remote: &mut R,
    repo_name: &str,
    line_name: &str,
) -> Result<Option<JsonValue>, String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    match task_remote.get_line(repo_name, line_name) {
        Ok(line) => Ok(Some(line)),
        Err(error) if task_land_unknown_line(&error.to_string()) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn task_land_validate_remote_line(
    line: &JsonValue,
    line_name: &str,
    expected_revision_snapshot_id: &str,
) -> Result<(), String> {
    let returned_name = required_string_field(line, "line_name")?;
    if returned_name != line_name {
        return Err(format!(
            "Remote Line lookup for `{line_name}` returned `{returned_name}`."
        ));
    }
    let head = string_field(line, "head_snapshot_id");
    if head.as_deref() != Some(expected_revision_snapshot_id) {
        return Err(format!(
            "Remote task feature Line `{line_name}` head drifted from accepted revision `{expected_revision_snapshot_id}` to `{}`; refusing closeout.",
            head.as_deref().unwrap_or("<none>")
        ));
    }
    Ok(())
}

pub(in crate::primitives) fn task_land_archive_remote_bound_line_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    line_name: &str,
    expected_revision_snapshot_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader + TaskWorkflowLineCloser + ?Sized,
{
    let Some(current) = task_land_remote_line_optional(task_remote, repo_name, line_name)? else {
        return Ok(json!({
            "status": "absent",
            "scope": "remote",
            "reason": "remote_line_absent",
            "line_name": line_name,
        }));
    };
    task_land_validate_remote_line(&current, line_name, expected_revision_snapshot_id)?;
    let current_status = string_field(&current, "status").unwrap_or_else(|| "active".to_string());
    if current_status == "archived" {
        return Ok(json!({
            "status": "already_archived",
            "scope": "remote",
            "line": current,
        }));
    }
    if current_status != "active" {
        return Err(format!(
            "Remote task feature Line `{line_name}` has unsupported closeout status `{current_status}`."
        ));
    }

    let expected_line_id = string_field(&current, "line_id");
    let archived = task_remote
        .close_line(repo_name, line_name, "archived")
        .map_err(|error| error.to_string())?;
    task_land_validate_remote_line(&archived, line_name, expected_revision_snapshot_id)?;
    if string_field(&archived, "status").as_deref() != Some("archived") {
        return Err(format!(
            "Remote feature Line `{line_name}` close response is not archived."
        ));
    }
    if expected_line_id.is_some() && string_field(&archived, "line_id") != expected_line_id {
        return Err(format!(
            "Remote feature Line `{line_name}` changed stable identity during closeout."
        ));
    }
    Ok(json!({
        "status": "archived",
        "scope": "remote",
        "line": archived,
    }))
}

fn task_land_archive_remote_bound_line(
    repo: &RepoRuntime,
    line_name: &str,
    expected_revision_snapshot_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    task_land_archive_remote_bound_line_with_task_remote(
        &mut task_remote,
        &repo_name,
        line_name,
        expected_revision_snapshot_id,
    )
}

pub(in crate::primitives) fn task_land_attach_bound_line_closeout(
    repo: &RepoRuntime,
    output: &mut JsonValue,
    use_local_scope: bool,
    remote_name: Option<&str>,
    captured: Result<Option<JsonValue>, String>,
) {
    if output.get("apply_status").and_then(JsonValue::as_str) != Some("done") {
        return;
    }
    let task_id = task_land_task_id(output);
    let task_status = task_land_task_status(output).unwrap_or_else(|| "unknown".to_string());
    let result = if task_status != "completed" {
        json!({
            "status": "deferred",
            "reason": "task_still_active",
            "task_id": task_id,
            "task_status": task_status,
        })
    } else {
        (|| -> Result<JsonValue, String> {
            task_land_worktree_cleanup_allows_line_closeout(output)?;
            let Some(candidate) = captured? else {
                return Ok(json!({
                    "status": "skipped",
                    "reason": "no_task_feature_line",
                    "task_id": task_id,
                }));
            };
            let expected_revision_snapshot_id =
                task_land_expected_revision_snapshot_id_for_closeout(
                    repo,
                    output,
                    use_local_scope,
                    remote_name,
                )?;
            let line_name = required_string_field(&candidate, "line_name")?;
            let target_line = task_land_target_line(output);
            task_land_validate_local_bound_line(
                repo,
                &candidate,
                &expected_revision_snapshot_id,
                target_line.as_deref(),
                !use_local_scope,
            )?;
            let remote = if use_local_scope {
                json!({
                    "status": "skipped",
                    "scope": "remote",
                    "reason": "local_task_land",
                })
            } else {
                task_land_archive_remote_bound_line(
                    repo,
                    &line_name,
                    &expected_revision_snapshot_id,
                    remote_name,
                )?
            };
            let local = task_land_archive_local_bound_line(
                repo,
                &candidate,
                &expected_revision_snapshot_id,
                target_line.as_deref(),
                !use_local_scope,
            )?;
            let already_archived = string_field(&local, "status").as_deref()
                == Some("already_archived")
                && matches!(
                    string_field(&remote, "status").as_deref(),
                    Some("already_archived" | "absent" | "skipped")
                );
            Ok(json!({
                "status": if already_archived { "already_archived" } else { "archived" },
                "reason": "final_task_completed",
                "task_id": task_id,
                "task_status": task_status,
                "line_id": candidate["line_id"].clone(),
                "line_name": line_name,
                "binding_source": candidate["binding_source"].clone(),
                "expected_revision_snapshot_id": expected_revision_snapshot_id,
                "target_line": target_line,
                "local": local,
                "remote": remote,
            }))
        })()
        .unwrap_or_else(|error| {
            json!({
                "status": "failed",
                "reason": "feature_line_closeout_failed",
                "task_id": task_id,
                "task_status": task_status,
                "error": error,
                "detail": "The Land record and Task completion are already authoritative. Repair the reported Line condition, then rerun the same `ait task finish` command to resume closeout without creating a second Land.",
            })
        })
    };
    if let Some(object) = output.as_object_mut() {
        object.insert("bound_line_closeout".to_string(), result);
    }
}
