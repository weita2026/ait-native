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
    guard_current_worktree_task_bound_authoring(repo, "change create")?;
    change_create_for_worktree_bootstrap(repo, task_id, title, base_line, local, remote_name)
}

/// Create a change without the task-bound authoring guard. Reserved for the
/// task-start worktree bootstrap, which runs at the repo root before the
/// bound worktree exists; the public command path stays fail-closed.
pub(in crate::primitives) fn change_create_for_worktree_bootstrap(
    repo: &RepoRuntime,
    task_id: &str,
    title: &str,
    base_line: Option<&str>,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    guard_repo_root_pinned_bound_worktree(repo, Some(task_id), "ait change create")?;
    guard_current_worktree_task_scope(repo, task_id, "ait change create")?;
    let bound_base_line = current_worktree_metadata(repo)?
        .and_then(|metadata| metadata.target_base_line)
        .and_then(|line| normalized_text(Some(&line)));
    let resolved_base_line = normalized_text(base_line)
        .or(bound_base_line)
        .unwrap_or_else(|| repo.default_line_name());
    guard_current_worktree_retarget(
        repo,
        &resolved_base_line,
        None,
        None,
        "creating a new change",
    )?;
    guard_no_planning_only_artifact_drift(repo, "ait change create")?;
    let use_local = change_uses_local_scope(repo, local, remote_name)?;
    if use_local {
        let task_store = repo.task_store()?;
        let change_store = repo.change_store()?;
        let local_task = task_local_read_with_task_store(&task_store, task_id)?;
        let line_row = local_line_row(repo, &resolved_base_line)?;
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
            string_field(&line_row, "head_snapshot_id").as_deref(),
        );
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    change_create_remote_flow_with_task_remote(
        &mut task_remote,
        &repo_name,
        task_id,
        title,
        &resolved_base_line,
        None,
    )
}

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
    required_string_field(&base_line_row, "head_snapshot_id")
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
    let change_task_id = string_field(change, "task_id");
    if let Some(metadata) = current_worktree_metadata(repo)? {
        let exact_binding = metadata.bound_change_ref.as_deref() == Some(change_ref.as_str())
            && metadata.bound_change_id.as_deref() == Some(change_id.as_str())
            && change_task_id
                .as_deref()
                .is_none_or(|task_id| metadata.bound_task_id.as_deref() == Some(task_id));
        if exact_binding {
            let (line_name, head_snapshot_id) = current_line_head_snapshot_id(repo)?;
            return head_snapshot_id.ok_or_else(|| {
                format!(
                    "Bound worktree {} for change {change_id} has no head snapshot on line {line_name}. Create a snapshot before retrying.",
                    metadata.name
                )
            });
        }
    }

    for field in ["revision_snapshot_id", "landed_snapshot_id"] {
        if let Some(snapshot_id) = string_field(change, field) {
            return Ok(snapshot_id);
        }
    }

    if change_uses_local_scope(repo, local, remote_name)? {
        return Err(format!(
            "Change {change_id} is not bound to the current worktree, so its authoritative local revision snapshot cannot be resolved from this workspace. Enter its bound task worktree."
        ));
    }
    let patchset_id = string_field(change, "selected_patchset_id")
        .or_else(|| string_field(change, "current_patchset_id"))
        .ok_or_else(|| {
            format!(
                "Change {change_id} has no selected remote patchset and is not bound to the current worktree, so no authoritative revision snapshot can be resolved."
            )
        })?;
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
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
    let (current_line_name, current_head_snapshot_id) =
        require_current_line_head_snapshot(repo, &latest_snapshot_id, "change revert")?;
    let result = apply_workspace_revert_range(
        repo,
        Some(&fork_snapshot_id),
        &current_head_snapshot_id,
        force,
        dry_run,
    )?;
    let mut payload = result
        .as_object()
        .cloned()
        .ok_or_else(|| "change revert payload must be an object".to_string())?;
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
        JsonValue::String(current_head_snapshot_id.clone()),
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
            "Local change {change_id} is already landed locally. {COMPLETED_LOCAL_FINAL_SNAPSHOT_PROMOTION_GUIDANCE}"
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
