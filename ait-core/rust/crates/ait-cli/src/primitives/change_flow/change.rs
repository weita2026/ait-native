use super::*;
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;

pub fn change_create(
    repo: &RepoRuntime,
    task_id: &str,
    title: &str,
    base_line: Option<&str>,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    change_create_revision(repo, task_id, title, base_line, None, local, remote_name)
}

pub fn change_create_revision(
    repo: &RepoRuntime,
    task_id: &str,
    title: &str,
    base_line: Option<&str>,
    fork_snapshot: Option<&str>,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    guard_current_worktree_task_bound_authoring(repo, "change create")?;
    create_with_fork(
        repo,
        task_id,
        title,
        base_line,
        fork_snapshot,
        local,
        remote_name,
    )
}

/// Create a change without the task-bound authoring guard. Reserved for the
/// task-start worktree bootstrap, which runs at the repo root before the
/// bound worktree exists; the public command path stays fail-closed.
#[cfg(test)]
pub(in crate::primitives) fn change_create_for_worktree_bootstrap(
    repo: &RepoRuntime,
    task_id: &str,
    title: &str,
    base_line: Option<&str>,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    create_with_fork(repo, task_id, title, base_line, None, local, remote_name)
}

fn create_with_fork(
    repo: &RepoRuntime,
    task_id: &str,
    title: &str,
    base_line: Option<&str>,
    fork_snapshot: Option<&str>,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    guard_repo_root_pinned_bound_worktree(repo, Some(task_id), "ait change create")?;
    guard_current_worktree_task_scope(repo, task_id, "ait change create")?;
    let use_local = change_uses_local_scope(repo, local, remote_name)?;
    let bound_base_line = current_worktree_metadata(repo)?
        .and_then(|metadata| metadata.target_base_line)
        .and_then(|line| normalized_text(Some(&line)));
    let local_revision_line = if use_local {
        Some(repo.current_line_name()?)
    } else {
        None
    };
    let resolved_base_line = normalized_text(base_line)
        .or(local_revision_line)
        .or(bound_base_line)
        .unwrap_or_else(|| repo.default_line_name());
    if use_local {
        guard_current_worktree_retarget(
            repo,
            &resolved_base_line,
            None,
            None,
            "creating a new change",
        )?;
        guard_no_planning_only_artifact_drift(repo, "ait change create")?;
        let task_store = repo.task_store()?;
        let change_store = repo.change_store()?;
        let local_task = task_local_read_with_task_store(&task_store, task_id)?;
        let line_row = local_line_row(repo, &resolved_base_line)?;
        let fork =
            normalized_text(fork_snapshot).or_else(|| string_field(&line_row, "head_snapshot_id"));
        if let Some(reviewed) = normalized_text(fork_snapshot) {
            if !workflow_workspace_status(repo, None, None)?["clean"]
                .as_bool()
                .unwrap_or(false)
            {
                return Err("Create the revision Change from its reviewed Snapshot before editing the workspace".into());
            }
            let owners = task_store
                .snapshot_ownership_rows(std::slice::from_ref(&reviewed))
                .map_err(|error| error.to_string())?;
            if !owners
                .iter()
                .any(|owner| string_field(owner, "task_id").as_deref() == Some(task_id))
            {
                return Err("Reviewed fork Snapshot is not owned by this Task".into());
            }
        }
        let repo_name = repo.repo_name();
        let task_repo_name = required_string_field(&local_task, "repo_name")?;
        if task_repo_name != repo_name {
            return Err(format!(
                "Local task {task_id} belongs to repository {task_repo_name}, not {repo_name}"
            ));
        }
        return change_local_create_with_change_store(
            &change_store,
            &repo_name,
            task_id,
            title,
            &resolved_base_line,
            Some(&repo.id_namespace_prefix()),
            fork.as_deref(),
        );
    }
    guard_no_planning_only_artifact_drift(repo, "ait change create")?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    guard_capture_publication_target(repo, &remote_row)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let target_head =
        change_base_line_head_with_task_remote(&mut task_remote, &repo_name, &resolved_base_line)?;
    guard_current_worktree_retarget(
        repo,
        &resolved_base_line,
        Some(&target_head),
        None,
        "creating a new change",
    )?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    change_create_remote_revision_with_remotes(
        &mut task_remote,
        &mut closeout_remote,
        &repo_name,
        task_id,
        title,
        &resolved_base_line,
        fork_snapshot,
        Some(&target_head),
    )
}

fn remote_revision_current_candidate(
    task: &JsonValue,
    task_id: &str,
) -> Result<Option<String>, String> {
    if task["status"].as_str() != Some("active")
        || ait_core::land_json::LandJson::stateless()
            .task_successful_receipt(task, task_id, None)?
            .is_some()
    {
        return Err(
            "Revision creation requires the exact active Task without an accepted receipt".into(),
        );
    }
    match task.get("current_patchset_id") {
        Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(id)) if !id.is_empty() && id.trim() == id => Ok(Some(id.clone())),
        _ => Err("Task revision creation requires nullable current_patchset_id".into()),
    }
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn change_create_remote_revision_with_remotes<T, C>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    repo_name: &str,
    task_id: &str,
    title: &str,
    base_line: &str,
    explicit_fork: Option<&str>,
    expected_target_head: Option<&str>,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteTaskReader
        + TaskWorkflowRemoteChangeCreator
        + TaskWorkflowLineReader
        + TaskWorkflowLineagePayloadBuilder
        + ?Sized,
    C: TaskWorkflowPatchsetReader + ?Sized,
{
    let task = task_remote
        .get_task(task_id, Some(repo_name))
        .map_err(|error| error.to_string())?;
    let current = remote_revision_current_candidate(&task, task_id)?;
    let candidate = current
        .as_deref()
        .map(|id| {
            closeout_remote
                .get_patchset(id, Some(repo_name), None)
                .map_err(|error| error.to_string())
        })
        .transpose()?;
    let expected = if let Some(row) = candidate.as_ref() {
        ait_core::workflow_expected_identity::validate_patchset_owner_fields(row, task_id)?;
        let expected = ait_core::workflow_expected_identity::WorkflowExpectedIdentity {
            repository_index: 0,
            patchset_id: current.clone().unwrap(),
            base_snapshot_id: required_string_field(row, "base_snapshot_id")?,
            revision_snapshot_id: required_string_field(row, "revision_snapshot_id")?,
        };
        expected.validate_patchset(0, row)?;
        if normalized_text(explicit_fork).is_some_and(|fork| fork != expected.revision_snapshot_id)
        {
            return Err(
                "Explicit fork Snapshot differs from the exact current Task candidate revision"
                    .into(),
            );
        }
        Some(expected)
    } else {
        None
    };
    let (line, mut lineage) =
        change_create_remote_lineage_with_task_remote(task_remote, repo_name, base_line)?;
    let target_head = change_base_line_head_from_row(&line, repo_name, base_line)?;
    if expected_target_head.is_some_and(|head| head != target_head)
        || expected
            .as_ref()
            .is_some_and(|identity| identity.base_snapshot_id != target_head)
    {
        return Err("Remote target Line advanced before revision creation".into());
    }
    let fork = expected
        .as_ref()
        .map(|identity| identity.revision_snapshot_id.clone())
        .or_else(|| normalized_text(explicit_fork));
    if let Some(fork) = fork.as_ref() {
        lineage["fork_snapshot_id"] = json!(fork);
    }
    let refreshed = task_remote
        .get_task(task_id, Some(repo_name))
        .map_err(|error| error.to_string())?;
    let refreshed_current = remote_revision_current_candidate(&refreshed, task_id)?;
    match (candidate.as_ref(), refreshed_current) {
        (Some(row), Some(id)) => {
            let mut identity = expected.clone().unwrap();
            identity.patchset_id = id;
            identity.validate_patchset(0, row).map_err(|error| {
                format!("Current Task candidate changed before revision creation: {error}")
            })?;
            let current = closeout_remote
                .get_patchset(&identity.patchset_id, Some(repo_name), None)
                .map_err(|error| error.to_string())?;
            ait_core::workflow_expected_identity::validate_patchset_owner_fields(
                &current, task_id,
            )?;
            identity.validate_patchset(0, &current)?;
        }
        (None, None) => {}
        _ => return Err("Current Task candidate changed before revision creation".into()),
    }
    let refreshed_head = change_base_line_head_with_task_remote(task_remote, repo_name, base_line)?;
    if refreshed_head != target_head {
        return Err("Remote target Line advanced before revision creation".into());
    }
    let created = change_create_with_task_remote(
        task_remote,
        repo_name,
        task_id,
        title,
        base_line,
        None,
        &lineage,
    )?;
    if fork
        .as_deref()
        .is_some_and(|fork| created["fork_snapshot_id"].as_str() != Some(fork))
    {
        return Err(
            "Created revision readback differs from its exact reviewed fork Snapshot".into(),
        );
    }
    Ok(created)
}

#[cfg(test)]
pub(in crate::primitives) fn change_create_remote_flow_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    task_id: &str,
    title: &str,
    base_line: &str,
    change_id: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeCreator
        + TaskWorkflowLineReader
        + TaskWorkflowLineagePayloadBuilder
        + ?Sized,
{
    let (_line_row, lineage_payload) =
        change_create_remote_lineage_with_task_remote(task_remote, repo_name, base_line)?;
    change_create_with_task_remote(
        task_remote,
        repo_name,
        task_id,
        title,
        base_line,
        change_id,
        &lineage_payload,
    )
}

pub(in crate::primitives) fn change_create_remote_lineage_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    base_line: &str,
) -> Result<(JsonValue, JsonValue), String>
where
    R: TaskWorkflowLineReader + TaskWorkflowLineagePayloadBuilder + ?Sized,
{
    let line_row = change_base_line_read_with_task_remote(task_remote, repo_name, base_line)?;
    let lineage_payload = task_remote.change_lineage_payload(base_line, Some(&line_row))?;
    Ok((line_row, lineage_payload))
}

pub(in crate::primitives) fn change_local_create_with_change_store<S>(
    change_store: &S,
    repo_name: &str,
    task_id: &str,
    title: &str,
    base_line: &str,
    namespace_prefix: Option<&str>,
    fork_snapshot_id: Option<&str>,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowChangeCreator + ?Sized,
{
    change_store
        .create_change(
            repo_name,
            task_id,
            title,
            base_line,
            namespace_prefix,
            fork_snapshot_id,
        )
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn change_create_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    task_id: &str,
    title: &str,
    base_line: &str,
    change_id: Option<&str>,
    lineage_payload: &JsonValue,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeCreator + ?Sized,
{
    let created = task_remote
        .create_change(
            repo_name,
            task_id,
            title,
            base_line,
            change_id,
            string_field(lineage_payload, "fork_snapshot_id").as_deref(),
            string_field(lineage_payload, "forked_from_line").as_deref(),
        )
        .map_err(|err| err.to_string())?;
    let created =
        ChangeJson::stateless().normalize_remote_change_payload(&created, Some(task_id))?;
    if change_id.is_none() {
        validate_short_remote_change_id(&created, task_id)?;
    }
    Ok(created)
}

pub(in crate::primitives) fn validate_short_remote_change_id(
    created: &JsonValue,
    task_id: &str,
) -> Result<(), String> {
    let change_id = required_string_field(created, "change_id")?;
    let Some(ordinal) = change_id.strip_prefix("C-") else {
        return Err(format!(
            "Remote change creation returned non-short change id `{change_id}` for task `{task_id}`. Normal Binary workflow creation must return `C-01`-style task-local ids; legacy `LC-*`/`RC-*` and composite ids are compatibility-only."
        ));
    };
    if ordinal.len() != 2 || !ordinal.bytes().all(|byte| byte.is_ascii_digit()) || ordinal == "00" {
        return Err(format!(
            "Remote change creation returned malformed short change id `{change_id}` for task `{task_id}`. Expected `C-01`-style two-digit child ordinal."
        ));
    }
    Ok(())
}

pub fn change_list(
    repo: &RepoRuntime,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let use_local = change_uses_local_scope(repo, local, remote_name)?;
    if use_local {
        let store = repo.change_store()?;
        return change_local_list_with_change_store(&store).map(JsonValue::Array);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    change_list_with_task_remote(&mut task_remote, &repo_name)
}

pub(in crate::primitives) fn change_local_list_with_change_store<S>(
    change_store: &S,
) -> Result<Vec<JsonValue>, String>
where
    S: TaskWorkflowChangeLister + ?Sized,
{
    change_store.list_changes().map_err(|err| err.to_string())
}

pub(in crate::primitives) fn change_list_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeLister + ?Sized,
{
    let rows = task_remote
        .list_changes(repo_name)
        .map_err(|err| err.to_string())?;
    rows.into_iter()
        .map(|row| ChangeJson::stateless().normalize_remote_change_payload(&row, None))
        .collect::<Result<Vec<_>, _>>()
        .map(JsonValue::Array)
}

pub fn change_show(
    repo: &RepoRuntime,
    change_id: &str,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    let resolved = resolve_task_inspect_change_input(repo, change_id, local, remote_name)?;
    let change_id = resolved.as_str();
    let use_local = change_uses_local_scope(repo, local, remote_name)?;
    if use_local {
        let store = repo.change_store()?;
        return change_local_read_with_change_store(&store, change_id);
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    change_show_with_task_remote(&mut task_remote, change_id, &repo_name)
}

pub(in crate::primitives) fn change_local_read_with_change_store<S>(
    change_store: &S,
    change_id: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowChangeReader + ?Sized,
{
    change_store
        .get_change(change_id)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn change_show_with_task_remote<R>(
    task_remote: &mut R,
    change_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeReader + ?Sized,
{
    let change = task_remote
        .get_change(change_id, Some(repo_name))
        .map_err(|err| err.to_string())?;
    ChangeJson::stateless().normalize_remote_change_payload(&change, None)
}

pub(in crate::primitives) fn change_base_line_head_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    base_line: &str,
) -> Result<String, String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    let base_line_row = change_base_line_read_with_task_remote(task_remote, repo_name, base_line)?;
    change_base_line_head_from_row(&base_line_row, repo_name, base_line)
}

fn change_base_line_head_from_row(
    base_line_row: &JsonValue,
    repo_name: &str,
    base_line: &str,
) -> Result<String, String> {
    if base_line_row["line_name"].as_str() != Some(base_line)
        || base_line_row
            .get("repo_name")
            .is_some_and(|name| name.as_str() != Some(repo_name))
    {
        return Err(
            "Remote target Line authority differs from the selected Repository/Line".into(),
        );
    }
    let head = required_string_field(base_line_row, "head_snapshot_id")?;
    if base_line_row["head_snapshot_id"].as_str() != Some(head.as_str()) {
        return Err("Malformed Remote target Line head".into());
    }
    Ok(head)
}

pub(in crate::primitives) fn change_base_line_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    base_line: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    task_remote
        .get_line(repo_name, base_line)
        .map_err(|err| err.to_string())
}

fn snapshot_store(repo: &RepoRuntime) -> Result<impl SnapshotStore, String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
}

fn change_revision_snapshot_id(
    repo: &RepoRuntime,
    change: &JsonValue,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<String, String> {
    let change_id = required_string_field(change, "change_id")?;
    let change_ref = change_reference_from_payload(change, None)?;
    for field in ["revision_snapshot_id", "landed_snapshot_id"] {
        if let Some(snapshot_id) = string_field(change, field) {
            return Ok(snapshot_id);
        }
    }

    if change_uses_local_scope(repo, local, remote_name)? {
        if let Some(snapshot_id) = repo
            .task_store()?
            .latest_change_snapshot_id(&change_ref)
            .map_err(|error| error.to_string())?
        {
            return Ok(snapshot_id);
        }
        return Err(format!(
            "Change {change_ref} has no recorded local revision Snapshot. Create an explicitly owned Snapshot in its Task worktree; legacy ownership cannot be inferred from a Line head."
        ));
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let (_, current) =
        change_identity_with_task_remote(&mut task_remote, &change_ref, Some(&repo_name))?;
    let patchset_id = current
        .ok_or_else(|| format!("Task for Change {change_id} has no current remote Patchset."))?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    let patchset = closeout_remote
        .get_patchset(&patchset_id, Some(&repo_name), Some(&change_ref))
        .map_err(|err| err.to_string())?;
    if !payload_belongs_to_change(&patchset, &change_id, &change_ref) {
        let patchset_change_ref = string_field(&patchset, "change_ref")
            .or_else(|| string_field(&patchset, "change_id"))
            .unwrap_or_else(|| "unknown".to_string());
        return Err(format!(
            "Patchset {patchset_id} belongs to change {patchset_change_ref}, not {change_ref}."
        ));
    }
    required_string_field(&patchset, "revision_snapshot_id")
}

pub(in crate::primitives) fn change_snapshot_lineage_with_snapshot_store<S>(
    snapshot_store: &S,
    change_id: &str,
    latest_snapshot_id: &str,
    fork_snapshot_id: &str,
) -> Result<Vec<String>, String>
where
    S: SnapshotStore + ?Sized,
{
    if !snapshot_store.snapshot_exists(latest_snapshot_id)? {
        return Err(format!(
            "Latest recorded change snapshot is not available locally: {latest_snapshot_id}"
        ));
    }
    if !snapshot_store.snapshot_exists(fork_snapshot_id)? {
        return Err(format!(
            "Change fork snapshot is not available locally: {fork_snapshot_id}"
        ));
    }
    let latest_lineage = snapshot_ancestor_closure(
        snapshot_store,
        &[latest_snapshot_id.to_string()],
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        SnapshotDagLimits::default(),
    )?;
    if !latest_lineage.contains(fork_snapshot_id) {
        return Err(format!(
            "Change {change_id} fork snapshot {fork_snapshot_id} is not an ancestor of latest recorded change snapshot {latest_snapshot_id}."
        ));
    }
    Ok(latest_lineage.topological_snapshot_ids)
}

pub fn change_revert(
    repo: &RepoRuntime,
    change_id: &str,
    force: bool,
    dry_run: bool,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait change revert")?;
    let change = change_show(repo, change_id, local, remote_name, repo_name_override)?;
    let resolved_change_id = required_string_field(&change, "change_id")?;
    let latest_snapshot_id =
        change_revision_snapshot_id(repo, &change, local, remote_name, repo_name_override)?;
    let fork_snapshot_id = string_field(&change, "fork_snapshot_id").ok_or_else(|| {
        format!(
            "Change {resolved_change_id} is missing its fork Snapshot history, so `ait change revert` cannot determine a safe base."
        )
    })?;
    let snapshot_store = snapshot_store(repo)?;
    change_snapshot_lineage_with_snapshot_store(
        &snapshot_store,
        &resolved_change_id,
        &latest_snapshot_id,
        &fork_snapshot_id,
    )?;
    let (current_line_name, current_head_snapshot_id) = current_line_head_snapshot_id(repo)?;
    let result = apply_workspace_replay_range(
        repo,
        &latest_snapshot_id,
        &fork_snapshot_id,
        current_head_snapshot_id.as_deref(),
        force,
        dry_run,
    )?;
    let mut payload = result
        .as_object()
        .cloned()
        .ok_or_else(|| "change revert payload must be an object".to_string())?;
    payload.insert("target_snapshot_id".to_string(), json!(fork_snapshot_id));
    payload.insert("repo_name".to_string(), JsonValue::String(repo.repo_name()));
    payload.insert(
        "change_id".to_string(),
        JsonValue::String(resolved_change_id.clone()),
    );
    payload.insert(
        "task_id".to_string(),
        string_field(&change, "task_id")
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "fork_snapshot_id".to_string(),
        JsonValue::String(fork_snapshot_id.clone()),
    );
    payload.insert(
        "latest_change_snapshot_id".to_string(),
        JsonValue::String(latest_snapshot_id.clone()),
    );
    payload.insert(
        "current_line".to_string(),
        JsonValue::String(current_line_name.clone()),
    );
    payload.insert(
        "current_line_head_snapshot_id".to_string(),
        json!(current_head_snapshot_id),
    );
    payload.insert(
        "mutation_scope".to_string(),
        JsonValue::String("workspace_only".to_string()),
    );
    payload.insert("moves_line_head".to_string(), JsonValue::Bool(false));
    payload.insert("creates_snapshot".to_string(), JsonValue::Bool(false));
    Ok(JsonValue::Object(payload))
}

#[expect(
    clippy::too_many_arguments,
    reason = "replay arguments mirror the explicit change event command surface"
)]
pub fn change_replay(
    repo: &RepoRuntime,
    change_id: &str,
    onto_line: &str,
    force: bool,
    dry_run: bool,
    local: bool,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait change replay")?;
    let change = change_show(repo, change_id, local, remote_name, repo_name_override)?;
    let resolved_change_id = required_string_field(&change, "change_id")?;
    let latest_snapshot_id =
        change_revision_snapshot_id(repo, &change, local, remote_name, repo_name_override)?;
    let fork_snapshot_id = string_field(&change, "fork_snapshot_id").ok_or_else(|| {
        format!(
            "Change {resolved_change_id} is missing its fork Snapshot history, so `ait change replay` cannot determine a safe base."
        )
    })?;
    let snapshot_store = snapshot_store(repo)?;
    change_snapshot_lineage_with_snapshot_store(
        &snapshot_store,
        &resolved_change_id,
        &latest_snapshot_id,
        &fork_snapshot_id,
    )?;
    let (current_line_name, current_head_snapshot_id) =
        require_current_line_target(repo, onto_line, "change replay")?;
    let result = apply_workspace_replay_range(
        repo,
        &fork_snapshot_id,
        &latest_snapshot_id,
        current_head_snapshot_id.as_deref(),
        force,
        dry_run,
    )?;
    let mut payload = result
        .as_object()
        .cloned()
        .ok_or_else(|| "change replay payload must be an object".to_string())?;
    payload.insert("repo_name".to_string(), JsonValue::String(repo.repo_name()));
    payload.insert(
        "change_id".to_string(),
        JsonValue::String(resolved_change_id.clone()),
    );
    payload.insert(
        "task_id".to_string(),
        string_field(&change, "task_id")
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "fork_snapshot_id".to_string(),
        JsonValue::String(fork_snapshot_id.clone()),
    );
    payload.insert(
        "latest_change_snapshot_id".to_string(),
        JsonValue::String(latest_snapshot_id.clone()),
    );
    payload.insert(
        "onto_line".to_string(),
        JsonValue::String(current_line_name.clone()),
    );
    payload.insert(
        "onto_line_head_snapshot_id".to_string(),
        current_head_snapshot_id
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "mutation_scope".to_string(),
        JsonValue::String("workspace_only".to_string()),
    );
    payload.insert("moves_line_head".to_string(), JsonValue::Bool(false));
    payload.insert("creates_snapshot".to_string(), JsonValue::Bool(false));
    Ok(JsonValue::Object(payload))
}

pub fn change_close(
    repo: &RepoRuntime,
    change_id: &str,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait change close")?;
    let use_local = change_uses_local_scope(repo, local, remote_name)?;
    if use_local {
        let store = repo.change_store()?;
        return change_local_close_with_change_store(&store, change_id, "archived");
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    change_close_with_task_remote(&mut task_remote, change_id, &repo_name)
}

pub(in crate::primitives) fn change_local_close_with_change_store<S>(
    change_store: &S,
    change_id: &str,
    status: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowChangeCloser + ?Sized,
{
    change_store
        .close_change(change_id, status)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn change_close_with_task_remote<R>(
    task_remote: &mut R,
    change_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeCloser + ?Sized,
{
    task_remote
        .close_change(change_id, "archived", Some(repo_name))
        .map_err(|err| err.to_string())
}

pub fn change_publish(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    change_publish_inner(repo, change_id, remote_name, false, None)
}

fn change_publish_inner(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    allow_landed_local: bool,
    fork_snapshot_id_override: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_planning_only_artifact_drift(repo, "ait change publish")?;
    let change_store = repo.change_store()?;
    let task_store = repo.task_store()?;
    let local_change = change_local_read_with_change_store(&change_store, change_id)?;
    let status = change_publish_status_gate(&local_change, change_id, allow_landed_local)?;
    let task_id = required_string_field(&local_change, "task_id")?;
    let local_task = task_local_read_with_task_store(&task_store, &task_id)?;
    change_publish_task_gate(&local_task, change_id)?;
    if !(allow_landed_local && status == "landed") {
        require_fresh_bound_task_worktree(
            repo,
            Some(&task_id),
            Some(change_id),
            &format!("publishing change {change_id}"),
        )?;
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    change_publish_with_local_stores_and_task_remote_with_fork_override(
        &change_store,
        &mut task_remote,
        &local_change,
        &local_task,
        change_id,
        &repo_name,
        remote_row.name.as_str(),
        allow_landed_local,
        fork_snapshot_id_override,
    )
}

fn change_publish_status_gate(
    local_change: &JsonValue,
    change_id: &str,
    allow_landed_local: bool,
) -> Result<String, String> {
    let status = required_string_field(local_change, "status")?;
    if status == "landed" && !allow_landed_local {
        return Err(format!(
            "Local change {change_id} is already finished locally. {COMPLETED_LOCAL_FINAL_SNAPSHOT_PROMOTION_GUIDANCE}"
        ));
    }
    if status != "draft" && !(allow_landed_local && status == "landed") {
        return Err(format!(
            "Local change {change_id} is {status} and cannot be published"
        ));
    }
    Ok(status)
}

fn change_publish_task_gate(local_task: &JsonValue, change_id: &str) -> Result<(), String> {
    if required_string_field(local_task, "publication_state")? != "published" {
        return Err(format!(
            "Local task {} must be published before publishing change {change_id}",
            required_string_field(local_task, "task_id")?
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(in crate::primitives) fn change_publish_with_local_stores_and_task_remote<C, R>(
    change_store: &C,
    task_remote: &mut R,
    local_change: &JsonValue,
    local_task: &JsonValue,
    change_id: &str,
    repo_name: &str,
    remote_name: &str,
    allow_landed_local: bool,
) -> Result<JsonValue, String>
where
    C: TaskWorkflowChangePublisher + ?Sized,
    R: TaskWorkflowRemoteChangeCreator + ?Sized,
{
    change_publish_with_local_stores_and_task_remote_with_fork_override(
        change_store,
        task_remote,
        local_change,
        local_task,
        change_id,
        repo_name,
        remote_name,
        allow_landed_local,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn change_publish_with_local_stores_and_task_remote_with_fork_override<C, R>(
    change_store: &C,
    task_remote: &mut R,
    local_change: &JsonValue,
    local_task: &JsonValue,
    change_id: &str,
    repo_name: &str,
    remote_name: &str,
    allow_landed_local: bool,
    fork_snapshot_id_override: Option<&str>,
) -> Result<JsonValue, String>
where
    C: TaskWorkflowChangePublisher + ?Sized,
    R: TaskWorkflowRemoteChangeCreator + ?Sized,
{
    let publication_remote_name = contextual_publication_remote_name(remote_name)?;
    change_publish_status_gate(local_change, change_id, allow_landed_local)?;
    let task_id = required_string_field(local_change, "task_id")?;
    change_publish_task_gate(local_task, change_id)?;
    let local_repo_name = required_string_field(local_change, "repo_name")?;
    if local_repo_name != repo_name {
        return Err(format!(
            "Local change {change_id} belongs to repository {local_repo_name}, not {repo_name}"
        ));
    }
    let remote_task_id =
        string_field(local_task, "published_task_id").unwrap_or_else(|| task_id.clone());
    let requested_change_id =
        required_string_field(local_change, "change_id").unwrap_or_else(|_| change_id.to_string());
    let remote_change = change_publish_with_task_remote_with_fork_override(
        task_remote,
        repo_name,
        &remote_task_id,
        local_change,
        &requested_change_id,
        fork_snapshot_id_override,
    )?;
    let remote_change_id = required_string_field(&remote_change, "change_id")?;
    let published_change_id = string_field(&remote_change, "published_change_id")
        .unwrap_or_else(|| remote_change_id.clone());
    change_local_mark_published_with_change_store(
        change_store,
        change_id,
        Some(publication_remote_name),
        Some(&published_change_id),
        allow_landed_local,
    )
}

pub(in crate::primitives) fn change_local_mark_published_with_change_store<S>(
    change_store: &S,
    change_id: &str,
    remote_name: Option<&str>,
    published_change_id: Option<&str>,
    allow_landed: bool,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowChangePublisher + ?Sized,
{
    change_store
        .mark_change_published(change_id, remote_name, published_change_id, allow_landed)
        .map_err(|err| err.to_string())
}

#[cfg(test)]
pub(in crate::primitives) fn change_publish_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    remote_task_id: &str,
    local_change: &JsonValue,
    requested_change_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeCreator + ?Sized,
{
    change_publish_with_task_remote_with_fork_override(
        task_remote,
        repo_name,
        remote_task_id,
        local_change,
        requested_change_id,
        None,
    )
}

fn change_publish_with_task_remote_with_fork_override<R>(
    task_remote: &mut R,
    repo_name: &str,
    remote_task_id: &str,
    local_change: &JsonValue,
    requested_change_id: &str,
    fork_snapshot_id_override: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeCreator + ?Sized,
{
    let remote_change = task_remote
        .create_change(
            repo_name,
            remote_task_id,
            &required_string_field(local_change, "title")?,
            &required_string_field(local_change, "base_line")?,
            Some(requested_change_id),
            normalized_text(fork_snapshot_id_override)
                .or_else(|| string_field(local_change, "fork_snapshot_id"))
                .as_deref(),
            string_field(local_change, "forked_from_line")
                .or_else(|| string_field(local_change, "base_line"))
                .as_deref(),
        )
        .map_err(|err| err.to_string())?;
    let remote_change_id = required_string_field(&remote_change, "change_id")?;
    if remote_change_id != requested_change_id {
        return Err(format!(
            "Remote server returned change_id {remote_change_id:?} while publishing local change {requested_change_id}. Shared publish must preserve the requested canonical id."
        ));
    }
    Ok(remote_change)
}
