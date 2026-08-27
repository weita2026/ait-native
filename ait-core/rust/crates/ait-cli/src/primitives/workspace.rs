use super::*;
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;
use ait_core::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotReadStore, LocalSnapshotTreeReadStore,
};
use ait_core::snapshot_store::SnapshotStore;
#[cfg(test)]
use ait_core::task_workflow_store::get_task_with_task_workflow_task_store;

pub fn snapshot_create(repo: &RepoRuntime, message: Option<&str>) -> Result<JsonValue, String> {
    guard_repo_root_pinned_bound_worktree(repo, None, "ait snapshot create")?;
    guard_current_worktree_task_bound_authoring(repo, "snapshot create")?;
    snapshot_create_in_current_workspace(repo, message)
}

/// Snapshot the current workspace without the task-bound authoring guard.
/// Reserved for callers that already own the authoring boundary (fixtures and
/// internal orchestration); the public command path stays fail-closed.
pub(in crate::primitives) fn snapshot_create_in_current_workspace(
    repo: &RepoRuntime,
    message: Option<&str>,
) -> Result<JsonValue, String> {
    guard_no_active_line_merge(repo, None, "creating a Snapshot")?;
    guard_no_planning_only_artifact_drift(repo, "ait snapshot create")?;
    let workspace_root = repo.workspace_root();
    let line_name = repo.current_line_name()?;
    let snapshot_store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let snapshot = snapshot_store.create_snapshot(
        &repo.repo_name(),
        &line_name,
        message,
        repo.is_worktree(),
    )?;
    repo.set_worktree_materialized_snapshot(string_field(&snapshot, "snapshot_id").as_deref())?;
    Ok(snapshot)
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the explicit snapshot creation command contract"
)]
pub fn snapshot_create_explicit(
    repo: &RepoRuntime,
    repo_name: &str,
    line_name: &str,
    message: Option<&str>,
    parent_snapshot_id: Option<&str>,
    update_line_ref: bool,
    touch_line: bool,
    record_workflow_metadata: bool,
) -> Result<JsonValue, String> {
    guard_current_worktree_task_bound_authoring(repo, "snapshot create")?;
    guard_no_active_line_merge(repo, None, "creating a Snapshot")?;
    guard_no_planning_only_artifact_drift(repo, "ait snapshot create")?;
    let workspace_root = repo.workspace_root();
    let line_name =
        normalized_text(Some(line_name)).ok_or_else(|| "line_name is required".to_string())?;
    let previous_head_snapshot_id = local_line_head_snapshot_id(repo, &line_name)?;
    let previous_line_updated_at = local_line_updated_at(repo, &line_name)?;
    let parent_snapshot_id = normalized_text(parent_snapshot_id);
    let snapshot_store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    if let Some(parent_snapshot_id) = parent_snapshot_id.as_deref() {
        set_local_line_head(repo, &line_name, Some(parent_snapshot_id))?;
    }
    let snapshot = snapshot_store
        .create_snapshot(repo_name, &line_name, message, repo.is_worktree())
        .inspect_err(|_| {
            if parent_snapshot_id.is_some() {
                let _ = set_local_line_head(repo, &line_name, previous_head_snapshot_id.as_deref());
                if !touch_line {
                    let _ = restore_local_line_updated_at(
                        repo,
                        &line_name,
                        previous_line_updated_at.as_deref(),
                    );
                }
            }
        })?;
    let snapshot_id = string_field(&snapshot, "snapshot_id");
    if !update_line_ref {
        set_local_line_head(repo, &line_name, previous_head_snapshot_id.as_deref())?;
    }
    if !touch_line {
        restore_local_line_updated_at(repo, &line_name, previous_line_updated_at.as_deref())?;
    }
    if record_workflow_metadata {
        repo.set_worktree_materialized_snapshot(snapshot_id.as_deref())?;
    }
    Ok(snapshot)
}

pub(super) fn guard_repo_root_pinned_bound_worktree(
    repo: &RepoRuntime,
    task_id: Option<&str>,
    operation: &str,
) -> Result<(), String> {
    if repo.is_worktree() {
        return Ok(());
    }
    let Some(worktree_name) = active_root_worktree_binding_name(repo) else {
        return Ok(());
    };
    let Ok(metadata_payload) = load_worktree_metadata(repo, &worktree_name) else {
        return Ok(());
    };
    let metadata = worktree_metadata_from_payload(
        &JsonValue::Object(metadata_payload.clone()),
        &worktree_name,
    );
    let Some(bound_task_id) = metadata.bound_task_id.as_deref() else {
        return Ok(());
    };
    if let Some(requested_task_id) = normalized_text(task_id) {
        if requested_task_id != bound_task_id {
            return Ok(());
        }
    }
    let path_hint = metadata_payload
        .get("path")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .map(|value| format!(" at `{value}`"))
        .unwrap_or_default();
    Err(format!(
        "Repo root is pinned to bound worktree `{}` for task `{}`. Continue in that task workspace{} before running `{}`. This operation requires a task-bound worktree; if that workspace is missing, run `ait worktree recreate {}`.",
        metadata.name, bound_task_id, path_hint, operation, metadata.name
    ))
}

pub(super) fn guard_patchset_worktree_retarget(
    repo: &RepoRuntime,
    base_line: &str,
    target_base_snapshot_id: &str,
    revision_snapshot_id: &str,
) -> Result<(), String> {
    guard_current_worktree_retarget(
        repo,
        base_line,
        Some(target_base_snapshot_id),
        Some(revision_snapshot_id),
        "publishing",
    )
}

pub(super) fn guard_current_worktree_retarget(
    repo: &RepoRuntime,
    base_line: &str,
    authoritative_target_base_snapshot_id: Option<&str>,
    revision_snapshot_id: Option<&str>,
    action_phrase: &str,
) -> Result<(), String> {
    let Some(metadata) = current_worktree_metadata(repo)? else {
        return Ok(());
    };
    if metadata.rebase_state == "conflicted" {
        let sample = if metadata.rebase_conflict_paths.is_empty() {
            "resolve conflicts first".to_string()
        } else {
            metadata
                .rebase_conflict_paths
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(
            format!(
                "Current worktree has a conflicted rebase in progress. Run `ait worktree rebase --continue` or `--abort` before {action_phrase}. {sample}"
            )
        );
    }
    let target_base_line = metadata
        .target_base_line
        .clone()
        .unwrap_or_else(|| base_line.to_string());
    let target_base_snapshot_id = match normalized_text(authoritative_target_base_snapshot_id) {
        Some(snapshot_id) => Some(snapshot_id),
        None => local_line_head_snapshot_id(repo, &target_base_line)?,
    };
    let fork_snapshot_id = match metadata.fork_snapshot_id.clone() {
        Some(value) => Some(value),
        None => {
            let revision_snapshot_id = match revision_snapshot_id {
                Some(value) => Some(value.to_string()),
                None => {
                    let current_line_name = repo.current_line_name()?;
                    local_line_head_snapshot_id(repo, &current_line_name)?
                }
            };
            match revision_snapshot_id {
                Some(value) => {
                    latest_common_snapshot(repo, &value, target_base_snapshot_id.as_deref())?
                }
                None => None,
            }
        }
    };
    if let (Some(fork_snapshot_id), Some(target_base_snapshot_id)) =
        (fork_snapshot_id, target_base_snapshot_id)
    {
        if fork_snapshot_id != target_base_snapshot_id {
            return Err(format!(
                "Current worktree is still based on `{fork_snapshot_id}` while `{target_base_line}` moved to `{target_base_snapshot_id}`. Run `ait worktree rebase --onto {target_base_line}` before {action_phrase}."
            ));
        }
    }
    Ok(())
}

pub(super) fn guard_patchset_revision_scope(
    repo: &RepoRuntime,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    change_id: &str,
    line_name: &str,
) -> Result<(), String> {
    let lineage_snapshot_ids = local_snapshot_chain_segment(
        repo,
        base_snapshot_id,
        revision_snapshot_id,
        "patchset publish",
    )?;
    if !repo.is_worktree() || lineage_snapshot_ids.is_empty() {
        return Ok(());
    }
    let Some(metadata) = current_worktree_metadata(repo)? else {
        return Ok(());
    };
    if line_name == repo.default_line_name() && !metadata.auto_created_for_task {
        return Ok(());
    }
    let Some(expected_task_id) = metadata.bound_task_id.clone() else {
        return Ok(());
    };
    let mut expected_change_aliases = BTreeSet::new();
    if let Some(expected_change_id) = normalized_text(Some(change_id)) {
        expected_change_aliases.insert(expected_change_id);
    }
    if let Some(bound_change_id) = metadata.bound_change_id.clone() {
        expected_change_aliases.insert(bound_change_id);
    }
    let ownership_rows = snapshot_ownership_rows(repo, &lineage_snapshot_ids)?;
    let mut ownership_issues = Vec::new();
    for snapshot_id in &lineage_snapshot_ids {
        let ownership = ownership_rows.iter().find(|row| {
            row.get("snapshot_id").and_then(JsonValue::as_str) == Some(snapshot_id.as_str())
        });
        let Some(ownership) = ownership else {
            ownership_issues.push(format!("{snapshot_id} (missing bound snapshot ownership)"));
            continue;
        };
        let owner_task_id = string_field(ownership, "task_id");
        let owner_change_id = string_field(ownership, "change_id");
        let owner_worktree_name = string_field(ownership, "worktree_name");
        if owner_task_id.as_deref() != Some(expected_task_id.as_str()) {
            ownership_issues.push(format!(
                "{snapshot_id} (task {})",
                owner_task_id.unwrap_or_else(|| "none".to_string())
            ));
            continue;
        }
        if !expected_change_aliases.is_empty()
            && owner_change_id
                .as_deref()
                .map(|value| !expected_change_aliases.contains(value))
                .unwrap_or(false)
        {
            ownership_issues.push(format!(
                "{snapshot_id} (change {})",
                owner_change_id.unwrap_or_default()
            ));
            continue;
        }
        if owner_worktree_name
            .as_deref()
            .map(|value| value != metadata.name)
            .unwrap_or(false)
        {
            ownership_issues.push(format!(
                "{snapshot_id} (worktree {})",
                owner_worktree_name.unwrap_or_default()
            ));
        }
    }
    if ownership_issues.is_empty() {
        return Ok(());
    }
    let issue_sample = ownership_issues
        .iter()
        .take(3)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    let change_fragment = expected_change_aliases
        .iter()
        .next()
        .map(|value| format!(" / change `{value}`"))
        .unwrap_or_default();
    Err(format!(
        "Current Line head `{revision_snapshot_id}` includes Snapshots that are not owned by bound Task `{expected_task_id}`{change_fragment} between base `{base_snapshot_id}` and the current head: {issue_sample}. Restore or reopen the correct Task worktree before running `ait patchset publish`."
    ))
}

pub(super) fn ensure_patchset_not_empty(
    change_id: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
) -> Result<(), String> {
    if base_snapshot_id != revision_snapshot_id {
        return Ok(());
    }
    Err(format!(
        "Refusing to publish empty patchset for {change_id}: base and revision both point to {base_snapshot_id}. Empty patchsets are prohibited; create a non-empty implementation snapshot before publishing."
    ))
}

pub(super) fn local_snapshot_chain_segment(
    repo: &RepoRuntime,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    command_name: &str,
) -> Result<Vec<String>, String> {
    if base_snapshot_id == revision_snapshot_id {
        return Ok(Vec::new());
    }
    let store = snapshot_store(repo)?;
    let revision_lineage = snapshot_ancestor_closure(
        &store,
        &[revision_snapshot_id.to_string()],
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        SnapshotDagLimits::default(),
    )?;
    if !revision_lineage.contains(base_snapshot_id) {
        return Err(format!(
            "Current line head `{revision_snapshot_id}` does not descend from selected base `{base_snapshot_id}`. Rebase, restore, or retarget the bound worktree before running `ait {command_name}`."
        ));
    }
    let base_lineage = snapshot_ancestor_closure(
        &store,
        &[base_snapshot_id.to_string()],
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        SnapshotDagLimits::default(),
    )?
    .parent_snapshot_ids
    .into_keys()
    .collect::<BTreeSet<_>>();
    Ok(revision_lineage
        .topological_snapshot_ids
        .into_iter()
        .filter(|snapshot_id| !base_lineage.contains(snapshot_id))
        .collect())
}

pub(super) fn latest_common_snapshot(
    repo: &RepoRuntime,
    left_snapshot_id: &str,
    right_snapshot_id: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(right_snapshot_id) = right_snapshot_id else {
        return Ok(None);
    };
    let store = snapshot_store(repo)?;
    Ok(snapshot_merge_bases(
        &store,
        left_snapshot_id,
        right_snapshot_id,
        SnapshotDagLimits::default(),
    )?
    .into_iter()
    .next())
}

pub(super) fn local_line_head_snapshot_id(
    repo: &RepoRuntime,
    line_name: &str,
) -> Result<Option<String>, String> {
    match local_line_row(repo, line_name) {
        Ok(line) => Ok(string_field(&line, "head_snapshot_id")),
        Err(err) if err.contains("Unknown line") => Ok(None),
        Err(err) => Err(err),
    }
}

pub(super) fn local_line_row(repo: &RepoRuntime, line_name: &str) -> Result<JsonValue, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    store.get_line(line_name)
}

pub(super) fn local_line_updated_at(
    repo: &RepoRuntime,
    line_name: &str,
) -> Result<Option<String>, String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?
        .line_updated_at(line_name)
}

pub(super) fn restore_local_line_updated_at(
    repo: &RepoRuntime,
    line_name: &str,
    updated_at: Option<&str>,
) -> Result<(), String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?
        .set_line_updated_at(line_name, updated_at)
}

fn snapshot_store(repo: &RepoRuntime) -> Result<impl SnapshotStore, String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
}

pub(super) fn current_worktree_metadata(
    repo: &RepoRuntime,
) -> Result<Option<CurrentWorktreeMetadata>, String> {
    if !repo.is_worktree() {
        return Ok(None);
    }
    let Some(worktree_name) = repo
        .config
        .get("worktree_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
    else {
        return Ok(None);
    };
    let metadata_path = repo
        .ait_dir
        .join("worktrees")
        .join(format!("{worktree_name}.json"));
    let payload = read_json_value(&metadata_path);
    Ok(Some(worktree_metadata_from_payload(
        &payload,
        &worktree_name,
    )))
}

pub(super) fn worktree_metadata_from_payload(
    payload: &JsonValue,
    fallback_name: &str,
) -> CurrentWorktreeMetadata {
    let bound_task_id = string_field(payload, "bound_task_id");
    let raw_bound_change_id = string_field(payload, "bound_change_id");
    let bound_change_id = raw_bound_change_id
        .as_deref()
        .and_then(|value| ChangeJson::stateless().canonical_change_id(value).ok());
    let bound_change_ref = bound_change_id
        .as_deref()
        .and_then(|value| {
            ChangeJson::stateless()
                .rolling_server_change_id(bound_task_id.as_deref(), value)
                .ok()
        })
        .or_else(|| string_field(payload, "bound_change_ref"));
    CurrentWorktreeMetadata {
        name: string_field(payload, "name").unwrap_or_else(|| fallback_name.to_string()),
        bound_task_id,
        bound_change_id,
        bound_change_ref,
        auto_created_for_task: payload
            .get("auto_created_for_task")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
        created_at: string_field(payload, "created_at"),
        fork_snapshot_id: string_field(payload, "fork_snapshot_id"),
        target_base_line: string_field(payload, "target_base_line"),
        rebase_state: string_field(payload, "rebase_state").unwrap_or_else(|| "idle".to_string()),
        rebase_conflict_paths: payload
            .get("rebase_conflict_paths")
            .and_then(JsonValue::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .filter_map(|value| normalized_text(Some(value)))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default(),
    }
}

pub(super) fn require_fresh_bound_task_worktree(
    repo: &RepoRuntime,
    task_id: Option<&str>,
    change_id: Option<&str>,
    operation: &str,
) -> Result<(), String> {
    if let Some(message) = bound_task_worktree_retarget_error(repo, task_id, change_id, operation)?
    {
        return Err(message);
    }
    Ok(())
}

pub(super) fn bound_task_worktree_retarget_error(
    repo: &RepoRuntime,
    task_id: Option<&str>,
    change_id: Option<&str>,
    operation: &str,
) -> Result<Option<String>, String> {
    let Some(metadata) = bound_task_worktree_metadata(repo, task_id, change_id)? else {
        return Ok(None);
    };
    let target_base_line = metadata
        .target_base_line
        .clone()
        .unwrap_or_else(|| "main".to_string());
    if metadata.rebase_state == "conflicted" {
        let sample = if metadata.rebase_conflict_paths.is_empty() {
            "resolve conflicts first".to_string()
        } else {
            metadata
                .rebase_conflict_paths
                .iter()
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Ok(Some(format!(
            "Bound worktree `{}` has a conflicted rebase in progress: {}. Run `ait worktree rebase --continue` or `ait worktree rebase --abort` before {}.",
            metadata.name, sample, operation
        )));
    }
    let target_base_snapshot_id = local_line_head_snapshot_id(repo, &target_base_line)?;
    if let (Some(fork_snapshot_id), Some(target_base_snapshot_id)) =
        (metadata.fork_snapshot_id.clone(), target_base_snapshot_id)
    {
        if fork_snapshot_id != target_base_snapshot_id {
            return Ok(Some(format!(
                "Bound worktree `{}` still forks from `{}` while `{}` now points at `{}`. Run `ait worktree rebase --onto {}` before {}.",
                metadata.name,
                fork_snapshot_id,
                target_base_line,
                target_base_snapshot_id,
                target_base_line,
                operation
            )));
        }
    }
    Ok(None)
}

pub(super) fn bound_task_worktree_metadata(
    repo: &RepoRuntime,
    task_id: Option<&str>,
    change_id: Option<&str>,
) -> Result<Option<CurrentWorktreeMetadata>, String> {
    let registry_dir = repo.ait_dir.join("worktrees");
    if !registry_dir.is_dir() {
        return Ok(None);
    }
    let normalized_change_id = normalized_text(change_id);
    let normalized_task_id = normalized_text(task_id);
    let canonical_change_id = normalized_change_id
        .as_deref()
        .map(canonical_change_id)
        .transpose()?;
    let requested_change_ref = match (
        normalized_change_id.as_deref(),
        canonical_change_id.as_deref(),
    ) {
        (Some(change_id), Some(canonical))
            if change_id != canonical
                || normalized_task_id.is_some()
                || !is_short_change_id(canonical) =>
        {
            Some(change_reference_for_context(
                normalized_task_id.as_deref(),
                change_id,
            )?)
        }
        _ => None,
    };
    let mut change_matches = Vec::new();
    let mut task_matches = Vec::new();
    for entry in fs::read_dir(&registry_dir).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let payload = read_json_value(&path);
        let fallback_name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("worktree")
            .to_string();
        let metadata = worktree_metadata_from_payload(&payload, &fallback_name);
        let change_matches_request = match requested_change_ref.as_deref() {
            Some(change_ref) => metadata.bound_change_ref.as_deref() == Some(change_ref),
            None => {
                canonical_change_id.is_some()
                    && metadata.bound_change_id.as_ref() == canonical_change_id.as_ref()
            }
        };
        if change_matches_request {
            change_matches.push(metadata);
            continue;
        }
        if normalized_task_id.is_some()
            && metadata.bound_task_id.as_ref() == normalized_task_id.as_ref()
        {
            task_matches.push(metadata);
        }
    }
    let mut candidates = if !change_matches.is_empty() {
        if requested_change_ref.is_none() && change_matches.len() > 1 {
            let scopes = change_matches
                .iter()
                .map(|metadata| {
                    metadata
                        .bound_change_ref
                        .clone()
                        .or_else(|| metadata.bound_task_id.clone())
                        .unwrap_or_else(|| metadata.name.clone())
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Short change_id `{}` matches multiple task worktrees ({scopes}); provide task context or an explicit change_ref.",
                canonical_change_id.unwrap_or_default()
            ));
        }
        change_matches
    } else {
        task_matches
    };
    if candidates.is_empty() {
        return Ok(None);
    }
    candidates.sort_by(|left, right| {
        (
            right.auto_created_for_task,
            right.created_at.clone().unwrap_or_default(),
        )
            .cmp(&(
                left.auto_created_for_task,
                left.created_at.clone().unwrap_or_default(),
            ))
    });
    Ok(candidates.into_iter().next())
}

pub(super) fn guard_repo_root_bound_task_worktree(
    repo: &RepoRuntime,
    task_id: Option<&str>,
    change_id: Option<&str>,
    operation: &str,
) -> Result<(), String> {
    if repo.is_worktree() {
        return Ok(());
    }
    let Some(metadata) = bound_task_worktree_metadata(repo, task_id, change_id)? else {
        return Ok(());
    };
    let task_hint = metadata
        .bound_task_id
        .as_deref()
        .or(task_id)
        .unwrap_or("unknown task");
    Err(format!(
        "Repo root has bound worktree `{}` for task `{}`. Continue in that task workspace before running `{}`. This operation requires a task-bound worktree; if that workspace is missing, run `ait worktree recreate {}`.",
        metadata.name, task_hint, operation, metadata.name
    ))
}

#[derive(Clone, Debug)]
struct TaskScopedWorktreeBinding {
    registry_name: String,
    metadata: CurrentWorktreeMetadata,
    payload: JsonValue,
}

fn task_scoped_worktree_bindings(
    repo: &RepoRuntime,
) -> Result<Vec<TaskScopedWorktreeBinding>, String> {
    let registry_dir = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("worktrees");
    if !registry_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut paths = fs::read_dir(&registry_dir)
        .map_err(|err| err.to_string())?
        .map(|entry| {
            entry
                .map(|value| value.path())
                .map_err(|err| err.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut bindings = Vec::new();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let payload = read_json_value(&path);
        let Some(registry_name) = path
            .file_stem()
            .and_then(|value| value.to_str())
            .and_then(|value| normalized_text(Some(value)))
        else {
            continue;
        };
        let metadata = worktree_metadata_from_payload(&payload, &registry_name);
        if metadata.bound_task_id.is_none() && metadata.bound_change_id.is_none() {
            continue;
        }
        bindings.push(TaskScopedWorktreeBinding {
            registry_name,
            metadata,
            payload,
        });
    }
    Ok(bindings)
}

fn syntactic_task_id(reference: &str) -> Option<String> {
    let reference = normalized_text(Some(reference))?;
    if let Some((task_id, _change_id)) = reference.split_once('/') {
        if exact_task_or_change_reference_family(task_id) == Some('T') {
            return Some(task_id.to_string());
        }
    }
    (exact_task_or_change_reference_family(&reference) == Some('T')).then_some(reference)
}

fn exact_task_or_change_reference_family(reference: &str) -> Option<char> {
    let text = reference.trim().to_ascii_uppercase();
    let (prefix, ordinal) = text.rsplit_once('-')?;
    if ordinal.is_empty() || !ordinal.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match prefix.chars().last()? {
        family @ ('T' | 'C') => Some(family),
        _ => None,
    }
}

fn task_id_from_direct_change_binding(
    bindings: &[TaskScopedWorktreeBinding],
    reference: &str,
) -> Result<Option<String>, String> {
    let reference = normalized_text(Some(reference));
    let mut matches = bindings
        .iter()
        .filter(|binding| {
            metadata_change_matches_reference(&binding.metadata, reference.as_deref())
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.registry_name.cmp(&right.registry_name));
    if matches.len() > 1 {
        let names = matches
            .iter()
            .map(|binding| binding.registry_name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Exact worktree routing is ambiguous for `{}`: matching bindings are {names}. Use a task-scoped Change reference such as `<task-id>/C-01`.",
            reference.unwrap_or_default()
        ));
    }
    Ok(matches
        .first()
        .and_then(|binding| binding.metadata.bound_task_id.clone()))
}

fn metadata_change_matches_reference(
    metadata: &CurrentWorktreeMetadata,
    reference: Option<&str>,
) -> bool {
    let reference = normalized_text(reference);
    let canonical = reference
        .as_deref()
        .and_then(|value| canonical_change_id(value).ok());
    let exact_ref_match = metadata.bound_change_ref.as_ref() == reference.as_ref();
    let canonical_match = reference.as_ref() == canonical.as_ref()
        && metadata.bound_change_id.as_ref() == canonical.as_ref();
    exact_ref_match || canonical_match
}

fn task_id_from_change_authority(
    repo: &RepoRuntime,
    reference: &str,
    lookup_local_change: bool,
    lookup_remote_change: bool,
    remote_name: Option<&str>,
) -> Option<String> {
    if lookup_local_change {
        if let Ok(change) = change_show(repo, reference, true, None, None) {
            if let Some(task_id) = change_task_id_from_payload(&change) {
                return Some(task_id);
            }
        }
    }
    if lookup_remote_change {
        if let Ok(change) = change_show(repo, reference, false, remote_name, None) {
            if let Some(task_id) = change_task_id_from_payload(&change) {
                return Some(task_id);
            }
        }
    }
    None
}

fn bound_task_id_for_alias(
    repo: &RepoRuntime,
    bindings: &[TaskScopedWorktreeBinding],
    requested_task_id: &str,
) -> Result<String, String> {
    if bindings
        .iter()
        .any(|binding| binding.metadata.bound_task_id.as_deref() == Some(requested_task_id))
    {
        return Ok(requested_task_id.to_string());
    }
    let mut matches = BTreeMap::<String, BTreeSet<String>>::new();
    for binding in bindings {
        let Some(bound_task_id) = binding.metadata.bound_task_id.as_deref() else {
            continue;
        };
        let Ok(task) = task_show(repo, bound_task_id, true, None) else {
            continue;
        };
        let alias_matches = [
            string_field(&task, "task_id"),
            string_field(&task, "published_task_id"),
        ]
        .into_iter()
        .flatten()
        .any(|alias| alias == requested_task_id);
        if alias_matches {
            matches
                .entry(bound_task_id.to_string())
                .or_default()
                .insert(binding.registry_name.clone());
        }
    }
    if matches.len() > 1 {
        let scopes = matches
            .iter()
            .map(|(task_id, names)| {
                format!(
                    "{task_id} ({})",
                    names.iter().cloned().collect::<Vec<_>>().join(", ")
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        return Err(format!(
            "Refusing exact worktree routing because Task alias `{requested_task_id}` maps to multiple bound Tasks: {scopes}."
        ));
    }
    Ok(matches
        .into_keys()
        .next()
        .unwrap_or_else(|| requested_task_id.to_string()))
}

fn synchronized_plan_artifact_at_root(
    repo: &RepoRuntime,
    path: &str,
    plan_heads: &BTreeMap<String, BTreeSet<String>>,
) -> Result<bool, String> {
    let Some(expected_blob_ids) = plan_heads.get(path) else {
        return Ok(false);
    };
    let Some(actual_blob_id) =
        current_artifact_blob_id(&repo.authoritative_repo_root().join(path))?
    else {
        return Ok(false);
    };
    Ok(expected_blob_ids.contains(&actual_blob_id))
}

fn verified_nested_worktree_prefixes(
    repo: &RepoRuntime,
    bindings: &[TaskScopedWorktreeBinding],
) -> BTreeSet<String> {
    let Ok(root_path) = repo.authoritative_repo_root().canonicalize() else {
        return BTreeSet::new();
    };
    bindings
        .iter()
        .filter_map(|binding| {
            if binding.metadata.name != binding.registry_name {
                return None;
            }
            let registered_path = required_path_field(&binding.payload, "path").ok()?;
            let registered_line = binding
                .payload
                .get("line_name")
                .and_then(JsonValue::as_str)
                .and_then(|value| normalized_text(Some(value)))?;
            let registered_path = registered_path.canonicalize().ok()?;
            let relative = registered_path.strip_prefix(&root_path).ok()?;
            if relative.as_os_str().is_empty() {
                return None;
            }
            let worktree_repo = discover_worktree_repo(&registered_path)?;
            let worktree_root = worktree_repo.workspace_root().canonicalize().ok()?;
            let worktree_repo_root = worktree_repo
                .authoritative_repo_root()
                .canonicalize()
                .ok()?;
            let overlay_name = worktree_repo
                .config
                .get("worktree_name")
                .and_then(JsonValue::as_str)
                .and_then(|value| normalized_text(Some(value)))?;
            let overlay_line = worktree_repo.current_line_name().ok()?;
            (worktree_root == registered_path
                && worktree_repo_root == root_path
                && overlay_name == binding.registry_name
                && overlay_line == registered_line)
                .then(|| relative.to_string_lossy().replace('\\', "/"))
        })
        .collect()
}

fn path_is_inside_verified_worktree(path: &str, prefixes: &BTreeSet<String>) -> bool {
    prefixes
        .iter()
        .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
}

fn guard_repo_root_control_surface_clean(
    repo: &RepoRuntime,
    bindings: &[TaskScopedWorktreeBinding],
    operation: &str,
) -> Result<(), String> {
    if repo.is_worktree() {
        return Ok(());
    }
    let status = workflow_workspace_status(repo, None, None)?;
    let changed_paths = json_string_list(status.get("changed_paths"));
    if changed_paths.is_empty() {
        return Ok(());
    }
    let plan_heads = current_markdown_plan_head_blob_ids(repo)?;
    let nested_worktree_prefixes = verified_nested_worktree_prefixes(repo, bindings);
    let mut authoring_paths = Vec::new();
    for path in changed_paths {
        if !path_is_inside_verified_worktree(&path, &nested_worktree_prefixes)
            && !synchronized_plan_artifact_at_root(repo, &path, &plan_heads)?
        {
            authoring_paths.push(path);
        }
    }
    if authoring_paths.is_empty() {
        return Ok(());
    }
    Err(format!(
        "Refusing to run `{operation}` from the Repository root while code/workspace drift is present there: {}. Task code must be authored in its bound worktree; move or revert these root edits before retrying.",
        summarize_path_sample(&authoring_paths)
    ))
}

fn canonical_existing_path(path: &Path, label: &str) -> Result<PathBuf, String> {
    path.canonicalize()
        .map_err(|err| format!("Unable to verify {label} `{}`: {err}", path.display()))
}

fn verified_task_scoped_worktree_repo(
    root_repo: &RepoRuntime,
    task_id: &str,
    binding: &TaskScopedWorktreeBinding,
    operation: &str,
) -> Result<RepoRuntime, String> {
    if binding.metadata.name != binding.registry_name {
        return Err(format!(
            "Refusing to route `{operation}` because worktree registry `{}` declares mismatched name `{}`.",
            binding.registry_name, binding.metadata.name
        ));
    }
    let payload = binding.payload.as_object().ok_or_else(|| {
        format!(
            "Refusing to route `{operation}` because worktree registry `{}` is malformed.",
            binding.registry_name
        )
    })?;
    let registered_path = required_path_field(&binding.payload, "path").map_err(|error| {
        format!(
            "Refusing to route `{operation}` because worktree `{}` has no valid registered path: {error}",
            binding.registry_name
        )
    })?;
    let registered_repo_root = required_path_field(&binding.payload, "repo_root").map_err(|error| {
        format!(
            "Refusing to route `{operation}` because worktree `{}` has no valid Repository root: {error}",
            binding.registry_name
        )
    })?;
    let registered_line = payload
        .get("line_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .ok_or_else(|| {
            format!(
                "Refusing to route `{operation}` because worktree `{}` has no registered Line.",
                binding.registry_name
            )
        })?;
    let root_path = canonical_existing_path(
        &root_repo.authoritative_repo_root(),
        "authoritative Repository root",
    )?;
    let registry_root_path =
        canonical_existing_path(&registered_repo_root, "registered Repository root")?;
    if registry_root_path != root_path {
        return Err(format!(
            "Refusing to route `{operation}` because worktree `{}` belongs to a different Repository root `{}`.",
            binding.registry_name,
            registered_repo_root.display()
        ));
    }
    if !registered_path.is_dir() {
        return Err(format!(
            "Bound worktree `{}` for Task `{task_id}` is missing or detached at `{}`. Run `ait worktree recreate {}`.",
            binding.registry_name,
            registered_path.display(),
            binding.registry_name
        ));
    }
    let registered_path = canonical_existing_path(&registered_path, "registered worktree path")?;
    let worktree_repo = discover_worktree_repo(&registered_path).ok_or_else(|| {
        format!(
            "Bound worktree `{}` for Task `{task_id}` is missing or detached at `{}`. Run `ait worktree recreate {}`.",
            binding.registry_name,
            registered_path.display(),
            binding.registry_name
        )
    })?;
    if !worktree_repo.is_worktree() {
        return Err(format!(
            "Refusing to route `{operation}` because `{}` is not a worktree runtime.",
            registered_path.display()
        ));
    }
    let worktree_root = canonical_existing_path(&worktree_repo.workspace_root(), "worktree root")?;
    let worktree_repo_root = canonical_existing_path(
        &worktree_repo.authoritative_repo_root(),
        "worktree authoritative Repository root",
    )?;
    let overlay_name = worktree_repo
        .config
        .get("worktree_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)));
    let overlay_line = worktree_repo.current_line_name()?;
    if worktree_root != registered_path
        || worktree_repo_root != root_path
        || overlay_name.as_deref() != Some(binding.registry_name.as_str())
        || overlay_line != registered_line
        || binding.metadata.bound_task_id.as_deref() != Some(task_id)
    {
        return Err(format!(
            "Refusing to route `{operation}` because worktree `{}` no longer exactly matches its registered Task, path, Repository, or Line binding.",
            binding.registry_name
        ));
    }
    Ok(worktree_repo)
}

/// Resolve an exact Task- or Change-scoped command to its verified bound
/// worktree for read-only inspection. A Repository-root invocation never
/// becomes an authoring context, and a different current worktree is rejected
/// instead of implicitly switching workspaces. Mutations must use
/// `run_task_scoped_workspace_command`, which also rejects root authoring drift.
pub fn resolve_task_scoped_execution_repo(
    repo: &RepoRuntime,
    reference: &str,
    lookup_local_change: bool,
    lookup_remote_change: bool,
    remote_name: Option<&str>,
    operation: &str,
) -> Result<RepoRuntime, String> {
    resolve_task_scoped_execution_repo_with_root_guard(
        repo,
        reference,
        lookup_local_change,
        lookup_remote_change,
        remote_name,
        operation,
        false,
    )
}

fn resolve_task_scoped_execution_repo_with_root_guard(
    repo: &RepoRuntime,
    reference: &str,
    lookup_local_change: bool,
    lookup_remote_change: bool,
    remote_name: Option<&str>,
    operation: &str,
    reject_root_drift: bool,
) -> Result<RepoRuntime, String> {
    let reference = normalized_text(Some(reference))
        .ok_or_else(|| "Task or Change ID must not be empty.".to_string())?;
    let bindings = task_scoped_worktree_bindings(repo)?;
    let current_metadata = if repo.is_worktree() {
        current_worktree_metadata(repo)?
    } else {
        None
    };
    let task_id = if let Some(task_id) = syntactic_task_id(&reference) {
        Some(task_id)
    } else if current_metadata
        .as_ref()
        .is_some_and(|metadata| metadata_change_matches_reference(metadata, Some(&reference)))
    {
        current_metadata
            .as_ref()
            .and_then(|metadata| metadata.bound_task_id.clone())
    } else if repo.is_worktree() {
        task_id_from_change_authority(
            repo,
            &reference,
            lookup_local_change,
            lookup_remote_change,
            remote_name,
        )
    } else {
        task_id_from_direct_change_binding(&bindings, &reference)?.or_else(|| {
            task_id_from_change_authority(
                repo,
                &reference,
                lookup_local_change,
                lookup_remote_change,
                remote_name,
            )
        })
    }
    .map(|task_id| bound_task_id_for_alias(repo, &bindings, &task_id))
    .transpose()?;

    if repo.is_worktree() {
        let metadata = current_metadata.ok_or_else(|| {
            format!(
                "Current worktree metadata is unavailable; refusing to run `{operation}` without a verified Task binding."
            )
        })?;
        let bound_task_id = metadata.bound_task_id.as_deref().ok_or_else(|| {
            format!(
                "Current worktree `{}` is not bound to a Task; refusing to run `{operation}`.",
                metadata.name
            )
        })?;
        if let Some(task_id) = task_id.as_deref() {
            if task_id != bound_task_id {
                return Err(format!(
                    "Current worktree `{}` is bound to Task `{bound_task_id}`, not Task `{task_id}`. Continue in the matching Task worktree before running `{operation}`.",
                    metadata.name
                ));
            }
        }
        return Ok(repo.clone());
    }

    let Some(task_id) = task_id else {
        return Ok(repo.clone());
    };
    let mut matches = bindings
        .iter()
        .filter(|binding| binding.metadata.bound_task_id.as_deref() == Some(task_id.as_str()))
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| left.registry_name.cmp(&right.registry_name));
    if matches.len() > 1 {
        let names = matches
            .iter()
            .map(|binding| binding.registry_name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Refusing to route `{operation}` because Task `{task_id}` has multiple bound worktrees: {names}. Repair the duplicate binding before retrying."
        ));
    }
    let Some(binding) = matches.first() else {
        // Preserve pre-worktree repositories and remote-only workflows. The
        // downstream authority still decides whether the operation itself is
        // valid; no implicit workspace routing occurs in this compatibility
        // path.
        return Ok(repo.clone());
    };
    if reject_root_drift {
        guard_repo_root_control_surface_clean(repo, &bindings, operation)?;
    }
    verified_task_scoped_worktree_repo(repo, &task_id, binding, operation)
}

/// Execute a mutating Task-scoped command under the resolved workspace lock.
/// Resolution is repeated after locking so a concurrent binding change fails
/// rather than redirecting the operation to another workspace.
pub fn run_task_scoped_workspace_command<T, F>(
    repo: &RepoRuntime,
    reference: &str,
    lookup_local_change: bool,
    lookup_remote_change: bool,
    remote_name: Option<&str>,
    operation: &str,
    command: F,
) -> Result<T, String>
where
    F: FnOnce(&RepoRuntime) -> Result<T, String>,
{
    let execution_repo = resolve_task_scoped_execution_repo_with_root_guard(
        repo,
        reference,
        lookup_local_change,
        lookup_remote_change,
        remote_name,
        operation,
        true,
    )?;
    run_locked_workspace_command(&execution_repo, operation, || {
        let verified_repo = resolve_task_scoped_execution_repo_with_root_guard(
            repo,
            reference,
            lookup_local_change,
            lookup_remote_change,
            remote_name,
            operation,
            true,
        )?;
        let before = canonical_existing_path(&execution_repo.workspace_root(), "execution root")?;
        let after = canonical_existing_path(&verified_repo.workspace_root(), "verified root")?;
        if before != after {
            return Err(format!(
                "Refusing to run `{operation}` because its Task worktree binding changed while acquiring the workspace lock; retry the command."
            ));
        }
        command(&verified_repo)
    })
}

pub(super) fn remote_change_task_id<R>(
    task_remote: &mut R,
    repo_name: &str,
    change: &JsonValue,
    requested_change_id: &str,
    resolved_change_id: &str,
) -> Result<Option<String>, String>
where
    R: TaskWorkflowRemoteChangeDetailReader + TaskWorkflowRemoteChangeLister + ?Sized,
{
    if let Some(task_id) = change_task_id_from_payload(change) {
        return Ok(Some(task_id));
    }
    if let Ok(detail) =
        workspace_remote_change_detail_with_task_remote(task_remote, repo_name, resolved_change_id)
    {
        if let Some(task_id) = change_task_id_from_payload(&detail) {
            return Ok(Some(task_id));
        }
    }
    let mut aliases = BTreeSet::new();
    if let Some(value) = normalized_text(Some(requested_change_id)) {
        aliases.insert(value);
    }
    if let Some(value) = normalized_text(Some(resolved_change_id)) {
        aliases.insert(value);
    }
    if aliases.is_empty() {
        return Ok(None);
    }
    let Ok(rows) = workspace_remote_change_rows_with_task_remote(task_remote, repo_name) else {
        return Ok(None);
    };
    for row in rows {
        let row_change_id = string_field(&row, "change_id");
        if row_change_id
            .as_ref()
            .map(|value| aliases.contains(value))
            .unwrap_or(false)
        {
            if let Some(task_id) = change_task_id_from_payload(&row) {
                return Ok(Some(task_id));
            }
        }
    }
    Ok(None)
}

pub(super) fn workspace_remote_change_detail_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    change_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeDetailReader + ?Sized,
{
    task_remote
        .get_change_detail(change_id, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub(super) fn workspace_remote_change_rows_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
) -> Result<Vec<JsonValue>, String>
where
    R: TaskWorkflowRemoteChangeLister + ?Sized,
{
    task_remote
        .list_changes(repo_name)
        .map_err(|err| err.to_string())
}

pub(super) fn change_task_id_from_payload(payload: &JsonValue) -> Option<String> {
    string_field(payload, "task_id")
        .or_else(|| {
            payload
                .get("change")
                .and_then(|value| string_field(value, "task_id"))
        })
        .or_else(|| {
            payload
                .get("task")
                .and_then(|value| string_field(value, "task_id"))
        })
}

pub(super) fn repo_root_has_bound_worktree_metadata(repo: &RepoRuntime) -> Result<bool, String> {
    let registry_dir = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("worktrees");
    if !registry_dir.is_dir() {
        return Ok(false);
    }
    for entry in fs::read_dir(&registry_dir).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let payload = read_json_value(&path);
        let fallback_name = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("worktree")
            .to_string();
        let metadata = worktree_metadata_from_payload(&payload, &fallback_name);
        if metadata.bound_task_id.is_some() || metadata.bound_change_id.is_some() {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(test)]
pub(super) fn repository_has_task_workflow_context_with_task_store<S>(
    task_store: &S,
) -> Result<bool, String>
where
    S: TaskStore + ?Sized,
{
    has_tasks_with_task_store(task_store).map_err(|err| err.to_string())
}

pub(super) fn guard_current_worktree_task_bound_authoring(
    repo: &RepoRuntime,
    command_name: &str,
) -> Result<(), String> {
    let Some(metadata) = current_worktree_metadata(repo)? else {
        // The repo root stays writable only while the repository has never
        // adopted task governance. Once any Task exists, authoring moves to
        // task-bound worktrees and the root fails closed.
        let task_store = repo.task_store()?;
        if ait_core::task_store::has_tasks_with_task_store(&task_store)
            .map_err(|err| err.to_string())?
        {
            return Err(format!(
                "The repo root is not an authoring workspace once tasks govern this repository. `ait {}` requires a task-bound worktree. Start with `ait task start` and author inside the worktree it prints, or continue in the matching existing task worktree.",
                command_name
            ));
        }
        return Ok(());
    };
    if metadata.bound_task_id.is_some() {
        return Ok(());
    }
    Err(format!(
        "Worktree `{}` is not bound to a task. `ait {}` requires a task-bound worktree. Start with `ait task start` or continue in the matching task worktree. If that task worktree is missing, use `ait worktree recreate`.",
        metadata.name, command_name
    ))
}

pub(super) fn guard_current_worktree_task_scope(
    repo: &RepoRuntime,
    requested_task_id: &str,
    operation: &str,
) -> Result<(), String> {
    let Some(metadata) = current_worktree_metadata(repo)? else {
        return Ok(());
    };
    let Some(bound_task_id) = metadata.bound_task_id.as_deref() else {
        return Ok(());
    };
    let Some(requested_task_id) = normalized_text(Some(requested_task_id)) else {
        return Ok(());
    };
    if requested_task_id == bound_task_id {
        return Ok(());
    }
    Err(format!(
        "Current worktree `{}` is bound to Task `{}`, not Task `{}`. Continue in the matching Task worktree before running `{}`.",
        metadata.name, bound_task_id, requested_task_id, operation
    ))
}

fn current_markdown_plan_head_blob_ids(
    repo: &RepoRuntime,
) -> Result<BTreeMap<String, BTreeSet<String>>, String> {
    let mut tracked = BTreeMap::<String, BTreeSet<String>>::new();
    let heads = repo.local_plan_head_artifacts().map_err(|err| {
        format!(
            "Unable to inspect local Markdown Plan history before creating workflow data: {err}"
        )
    })?;
    for head in heads {
        if matches!(head.status.as_str(), "archived" | "superseded") {
            continue;
        }
        track_markdown_plan_head(
            &mut tracked,
            &head.artifact_path,
            head.artifact_blob_id.as_deref(),
        );
    }
    Ok(tracked)
}

fn track_markdown_plan_head(
    tracked: &mut BTreeMap<String, BTreeSet<String>>,
    artifact_path: &str,
    artifact_blob_id: Option<&str>,
) {
    if !is_markdown_artifact_path(artifact_path) {
        return;
    }
    let Some(blob_id) = artifact_blob_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return;
    };
    tracked
        .entry(artifact_path.to_string())
        .or_default()
        .insert(blob_id.to_string());
}

fn artifact_path_candidates(repo: &RepoRuntime, artifact_path: &str) -> Vec<PathBuf> {
    let mut seen = BTreeSet::new();
    let mut candidates = Vec::new();
    // The active workspace is the authored surface. The authoritative root is
    // only a fallback for Plan artifacts that are intentionally not
    // materialized in a task worktree; it must never hide workspace drift.
    for root in [repo.workspace_root(), repo.authoritative_repo_root()] {
        let path = root.join(artifact_path);
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            candidates.push(path);
        }
    }
    candidates
}

fn current_artifact_blob_id(path: &Path) -> Result<Option<String>, String> {
    if !path.exists() {
        return Ok(None);
    }
    let metadata = fs::metadata(path).map_err(|err| err.to_string())?;
    if !metadata.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).map_err(|err| err.to_string())?;
    let sha256 = sha256_hex_bytes(&bytes);
    Ok(Some(format!("BLB-{}", &sha256[..20])))
}

fn collect_tracked_markdown_drift_paths(repo: &RepoRuntime) -> Result<Vec<String>, String> {
    let tracked = current_markdown_plan_head_blob_ids(repo)?;
    let mut dirty = BTreeSet::new();
    for (artifact_path, head_blob_ids) in tracked {
        let mut current_blob_id = None;
        for candidate in artifact_path_candidates(repo, &artifact_path) {
            let Some(candidate_blob_id) = current_artifact_blob_id(&candidate)? else {
                continue;
            };
            current_blob_id = Some(candidate_blob_id);
            break;
        }
        if !current_blob_id.is_some_and(|blob_id| head_blob_ids.contains(&blob_id)) {
            dirty.insert(artifact_path);
        }
    }
    Ok(dirty.into_iter().collect())
}

pub(super) fn collect_planning_only_artifact_drift_paths(
    repo: &RepoRuntime,
) -> Result<Vec<String>, String> {
    let mut paths = BTreeSet::new();
    for path in collect_tracked_markdown_drift_paths(repo)? {
        paths.insert(path);
    }
    Ok(paths.into_iter().collect())
}

pub(super) fn guard_no_planning_only_artifact_drift(
    repo: &RepoRuntime,
    operation: &str,
) -> Result<(), String> {
    let dirty_paths = collect_planning_only_artifact_drift_paths(repo)?;
    if dirty_paths.is_empty() {
        return Ok(());
    }
    let sample = summarize_path_sample(&dirty_paths);
    let first = dirty_paths
        .first()
        .cloned()
        .unwrap_or_else(|| "docs/plan.md".to_string());
    Err(format!(
        "Refusing to run `{operation}` while authored Markdown drift is present. Reconcile it first with `ait plan sync {first}` and add `--remote <name>` only when the Markdown update must reach shared plan state. Planning-only paths: {sample}."
    ))
}

#[cfg(test)]
pub(super) fn workspace_change_identity_aliases_with_change_store<S>(
    change_store: &S,
    change_id: Option<&str>,
) -> Result<BTreeSet<String>, String>
where
    S: TaskWorkflowChangeReader + ?Sized,
{
    let Some(change_id) = change_id.and_then(|value| normalized_text(Some(value))) else {
        return Ok(BTreeSet::new());
    };
    let mut aliases = BTreeSet::from([change_id.clone()]);
    let Ok(change) = workspace_local_change_read_with_change_store(change_store, &change_id) else {
        return Ok(aliases);
    };
    if let Some(canonical_change_id) = string_field(&change, "change_id") {
        aliases.insert(canonical_change_id);
    }
    if let Some(published_change_id) = string_field(&change, "published_change_id") {
        aliases.insert(published_change_id);
    }
    Ok(aliases)
}

#[cfg(test)]
pub(super) fn workspace_local_change_read_with_change_store<S>(
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

#[cfg(test)]
pub(super) fn workspace_task_identity_aliases_with_task_store<S>(
    task_store: &S,
    task_id: Option<&str>,
) -> Result<BTreeSet<String>, String>
where
    S: TaskWorkflowTaskReader + ?Sized,
{
    let Some(task_id) = task_id.and_then(|value| normalized_text(Some(value))) else {
        return Ok(BTreeSet::new());
    };
    let mut aliases = BTreeSet::from([task_id.clone()]);
    let Ok(task) = workspace_local_task_read_with_task_store(task_store, &task_id) else {
        return Ok(aliases);
    };
    if let Some(canonical_task_id) = string_field(&task, "task_id") {
        aliases.insert(canonical_task_id);
    }
    if let Some(published_task_id) = string_field(&task, "published_task_id") {
        aliases.insert(published_task_id);
    }
    Ok(aliases)
}

#[cfg(test)]
pub(super) fn workspace_local_task_read_with_task_store<S>(
    task_store: &S,
    task_id: &str,
) -> Result<JsonValue, String>
where
    S: TaskWorkflowTaskReader + ?Sized,
{
    get_task_with_task_workflow_task_store(task_store, task_id).map_err(|err| err.to_string())
}

pub(crate) fn snapshot_ownership_rows(
    repo: &RepoRuntime,
    snapshot_ids: &[String],
) -> Result<Vec<JsonValue>, String> {
    if snapshot_ids.is_empty() {
        return Ok(Vec::new());
    }
    let Some(metadata) = current_worktree_metadata(repo)? else {
        return Ok(Vec::new());
    };
    let line_name = repo.current_line_name()?;
    let store = snapshot_store(repo)?;
    bound_worktree_snapshot_ownership_rows_with_snapshot_store(
        &store,
        snapshot_ids,
        &metadata,
        &line_name,
    )
}

pub(super) fn bound_worktree_snapshot_ownership_rows_with_snapshot_store<S>(
    snapshot_store: &S,
    snapshot_ids: &[String],
    metadata: &CurrentWorktreeMetadata,
    line_name: &str,
) -> Result<Vec<JsonValue>, String>
where
    S: SnapshotStore + ?Sized,
{
    let Some(task_id) = metadata.bound_task_id.as_deref() else {
        return Ok(Vec::new());
    };
    if !metadata.auto_created_for_task || metadata.fork_snapshot_id.is_none() {
        return Ok(Vec::new());
    }
    let task_lines = task_feature_line_candidates(task_id)?;
    if !task_lines.iter().any(|candidate| candidate == line_name) {
        return Ok(Vec::new());
    }

    let mut rows = Vec::with_capacity(snapshot_ids.len());
    for snapshot_id in snapshot_ids {
        let Some(snapshot) = snapshot_store.snapshot_by_id(snapshot_id)? else {
            continue;
        };
        if snapshot.snapshot_id != *snapshot_id {
            return Err(format!(
                "Snapshot lookup for {snapshot_id} returned mismatched id {}.",
                snapshot.snapshot_id
            ));
        }
        if snapshot.snapshot_kind != "line" || snapshot.line_name != line_name {
            continue;
        }
        rows.push(json!({
            "snapshot_id": snapshot.snapshot_id,
            "task_id": task_id,
            "change_id": metadata.bound_change_id.clone(),
            "worktree_name": metadata.name.clone(),
            "line_name": snapshot.line_name,
            "author_mode": JsonValue::Null,
            "model_name": JsonValue::Null,
            "created_at": snapshot.created_at,
            "ownership_source": "bound_worktree_binary_snapshot_line",
        }));
    }
    Ok(rows)
}

pub(super) fn filtered_snapshot_tree_delta(
    repo: &RepoRuntime,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    ignore_rules_text: Option<&str>,
) -> Result<(Vec<String>, BTreeMap<String, String>), String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let delta = store.snapshot_tree_path_delta(old_snapshot_id, new_snapshot_id)?;
    let workspace_root = repo.workspace_root();
    let workspace_root_text = workspace_root.to_string_lossy().to_string();
    let ignore_matcher = ignore_rules_text.map(parse_workspace_ignore_matcher);
    let mut status_by_path = BTreeMap::new();
    for (path, status) in delta.status_by_path {
        if path_is_projected_out_for_workspace(&workspace_root_text, &path, repo.is_worktree()) {
            continue;
        }
        if ignore_matcher
            .as_ref()
            .map(|matcher| workspace_relative_path_is_ignored_with_matcher(&path, matcher))
            .unwrap_or(false)
        {
            continue;
        }
        if path == WORKTREE_CARGO_CONFIG_RELATIVE_PATH {
            let row_snapshot_id = if status == "deleted" {
                old_snapshot_id
            } else {
                new_snapshot_id
            };
            let Some(row_snapshot_id) = row_snapshot_id else {
                continue;
            };
            let Some(row) = store.snapshot_tree_path_row(row_snapshot_id, &path)? else {
                continue;
            };
            if !snapshot_row_visible(repo, &row, ignore_matcher.as_ref())? {
                continue;
            }
        }
        status_by_path.insert(path, status);
    }
    let affected_paths = status_by_path.keys().cloned().collect::<Vec<_>>();
    Ok((affected_paths, status_by_path))
}

pub(super) fn read_snapshot_blob_text(repo: &RepoRuntime, blob_id: &str) -> Result<String, String> {
    let bytes = read_selected_snapshot_blob_bytes(repo, blob_id)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub(super) fn read_selected_snapshot_blob_bytes(
    repo: &RepoRuntime,
    blob_id: &str,
) -> Result<Vec<u8>, String> {
    let workspace_root = repo.workspace_root();
    let snapshot_store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    snapshot_store.read_blob_bytes(blob_id)
}

pub(super) fn read_selected_snapshot_blob_bytes_batch(
    repo: &RepoRuntime,
    blob_ids: &[String],
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let workspace_root = repo.workspace_root();
    let snapshot_store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    snapshot_store.read_blob_bytes_batch(blob_ids)
}

pub(super) fn current_line_head_snapshot_id(
    repo: &RepoRuntime,
) -> Result<(String, Option<String>), String> {
    let current_line_name = repo.current_line_name()?;
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let current_line_row = store.get_line(&current_line_name)?;
    Ok((
        current_line_name,
        string_field(&current_line_row, "head_snapshot_id"),
    ))
}

pub(super) fn require_current_line_head_snapshot(
    repo: &RepoRuntime,
    expected_snapshot_id: &str,
    command_name: &str,
) -> Result<(String, String), String> {
    let (current_line_name, current_head_snapshot_id) = current_line_head_snapshot_id(repo)?;
    let Some(current_head_snapshot_id) = current_head_snapshot_id else {
        return Err(format!(
            "Current line {current_line_name} has no head snapshot to revert from."
        ));
    };
    if current_head_snapshot_id != expected_snapshot_id {
        return Err(format!(
            "`{command_name}` currently supports reverting only the current line head snapshot. Current line {current_line_name} points at {current_head_snapshot_id}, not {expected_snapshot_id}."
        ));
    }
    Ok((current_line_name, current_head_snapshot_id))
}

pub(super) fn require_current_line_target(
    repo: &RepoRuntime,
    onto_line: &str,
    command_name: &str,
) -> Result<(String, Option<String>), String> {
    let (current_line_name, current_head_snapshot_id) = current_line_head_snapshot_id(repo)?;
    if current_line_name != onto_line {
        return Err(format!(
            "`{command_name}` currently replays only onto the current line workspace. Current line is {current_line_name}, not {onto_line}."
        ));
    }
    Ok((current_line_name, current_head_snapshot_id))
}

pub(super) fn apply_workspace_revert_range(
    repo: &RepoRuntime,
    base_snapshot_id: Option<&str>,
    head_snapshot_id: &str,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let affected_paths = store
        .snapshot_tree_path_delta(base_snapshot_id, Some(head_snapshot_id))?
        .affected_paths;
    let result = restore_workspace_paths_selected(
        repo,
        base_snapshot_id,
        &affected_paths,
        Some(head_snapshot_id),
        force,
        dry_run,
    )?;
    let Some(result_obj) = result.as_object() else {
        return Err("workspace revert result must be an object".to_string());
    };
    if !dry_run && !affected_paths.is_empty() {
        repo.set_worktree_materialized_snapshot(None)?;
    }
    let mut payload = result_obj.clone();
    payload.insert(
        "affected_paths".to_string(),
        JsonValue::Array(
            affected_paths
                .iter()
                .cloned()
                .map(JsonValue::String)
                .collect(),
        ),
    );
    payload.insert(
        "affected_path_count".to_string(),
        JsonValue::from(affected_paths.len() as u64),
    );
    Ok(JsonValue::Object(payload))
}

pub(super) fn apply_workspace_replay_range(
    repo: &RepoRuntime,
    source_base_snapshot_id: &str,
    source_head_snapshot_id: &str,
    baseline_snapshot_id: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let snapshot_rules_text =
        snapshot_rules_state_from_snapshot_id(repo, Some(source_head_snapshot_id))?.text;
    let (affected_paths, delta_status_by_path) = filtered_snapshot_tree_delta(
        repo,
        Some(source_base_snapshot_id),
        Some(source_head_snapshot_id),
        snapshot_rules_text.as_deref(),
    )?;
    let effective_ignore_rules = effective_ignore_rules_text(repo, snapshot_rules_text.as_deref())?;
    let dirty =
        workspace_delta_payload(repo, baseline_snapshot_id, snapshot_rules_text.as_deref())?;
    let workspace_files = workspace_state(repo, effective_ignore_rules.as_deref())?;
    let source_head_entries = filtered_snapshot_path_rows(
        repo,
        Some(source_head_snapshot_id),
        &affected_paths,
        snapshot_rules_text.as_deref(),
    )?;
    let dirty_changed_paths = json_string_list(dirty.get("changed_paths"));
    let requested_set = affected_paths.iter().cloned().collect::<BTreeSet<_>>();
    let dirty_selected_paths = dirty_changed_paths
        .iter()
        .filter(|path| requested_set.contains(*path))
        .cloned()
        .collect::<Vec<_>>();
    let dirty_outside_paths = dirty_changed_paths
        .iter()
        .filter(|path| !requested_set.contains(*path))
        .cloned()
        .collect::<Vec<_>>();

    let mut write_paths = Vec::new();
    let mut remove_paths = Vec::new();
    let mut unchanged_paths = Vec::new();
    for rel in &affected_paths {
        let status = delta_status_by_path
            .get(rel)
            .cloned()
            .unwrap_or_else(|| "unchanged".to_string());
        let current = workspace_files.get(rel);
        if status == "deleted" {
            if current.is_none() {
                unchanged_paths.push(rel.clone());
            } else {
                remove_paths.push(rel.clone());
            }
            continue;
        }
        let source_row = source_head_entries.get(rel).ok_or_else(|| {
            format!(
                "Replay source snapshot `{source_head_snapshot_id}` is missing the changed file `{rel}`."
            )
        })?;
        let source_sha256 = file_map_row_sha256(source_row).unwrap_or_default();
        let source_mode = file_map_row_mode(source_row).unwrap_or_default();
        match current {
            Some(state) if state.sha256 == source_sha256 && state.mode == source_mode => {
                unchanged_paths.push(rel.clone());
            }
            _ => write_paths.push(rel.clone()),
        }
    }

    let mut result = json!({
        "source_base_snapshot_id": source_base_snapshot_id,
        "source_head_snapshot_id": source_head_snapshot_id,
        "baseline_snapshot_id": baseline_snapshot_id,
        "force": force,
        "dry_run": dry_run,
        "applied": false,
        "workspace_dirty": !dirty.get("clean").and_then(JsonValue::as_bool).unwrap_or(false),
        "would_overwrite_selected_changes": !dirty_selected_paths.is_empty(),
        "dirty_workspace": dirty,
        "dirty_selected_paths": dirty_selected_paths,
        "dirty_outside_paths": dirty_outside_paths,
        "affected_paths": affected_paths,
        "affected_path_count": affected_paths.len(),
        "delta_summary": {
            "added": delta_summary_paths(&delta_status_by_path, "added"),
            "deleted": delta_summary_paths(&delta_status_by_path, "deleted"),
            "modified": delta_summary_paths(&delta_status_by_path, "modified"),
            "mode_changed": delta_summary_paths(&delta_status_by_path, "mode_changed"),
        },
        "plan": {
            "write_count": write_paths.len(),
            "remove_count": remove_paths.len(),
            "unchanged_count": unchanged_paths.len(),
            "requested_paths": affected_paths,
            "write_paths": sort_paths(write_paths.clone()),
            "remove_paths": reverse_depth_sort_paths(remove_paths.clone()),
            "unchanged_paths": unchanged_paths,
        },
    });
    if !dirty_selected_paths.is_empty() && !force && !dry_run {
        let baseline_label = baseline_snapshot_id.unwrap_or("empty workspace");
        return Err(format!(
            "Selected paths have unsaved changes relative to {baseline_label}: {}",
            summarize_path_sample(&dirty_selected_paths)
        ));
    }
    if dry_run {
        return Ok(result);
    }

    let workspace_root = repo.workspace_root();
    for rel in reverse_depth_sort_paths(remove_paths.clone()) {
        let abs_path = workspace_root.join(&rel);
        if abs_path.exists() {
            fs::remove_file(&abs_path).map_err(|err| err.to_string())?;
            prune_empty_parent_dirs(&workspace_root, &abs_path)?;
        }
    }
    for rel in sort_paths(write_paths.clone()) {
        let source_row = source_head_entries
            .get(rel.as_str())
            .ok_or_else(|| format!("Replay source snapshot is missing `{rel}`."))?;
        let blob_id = file_map_row_blob_id(source_row)
            .ok_or_else(|| format!("Replay source snapshot row is missing blob_id for `{rel}`."))?;
        let abs_path = workspace_root.join(&rel);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        if abs_path.exists() && abs_path.is_dir() {
            return Err(format!("Cannot replay file over directory: {rel}"));
        }
        let data = read_selected_snapshot_blob_bytes(repo, &blob_id)?;
        fs::write(&abs_path, data).map_err(|err| err.to_string())?;
        let mode = parse_mode_bits(file_map_row_mode(source_row).as_deref())?;
        set_portable_mode(&abs_path, mode).map_err(|err| err.to_string())?;
    }
    result
        .as_object_mut()
        .expect("snapshot replay payload")
        .insert("applied".to_string(), JsonValue::Bool(true));
    if !affected_paths.is_empty() && (!write_paths.is_empty() || !remove_paths.is_empty()) {
        repo.set_worktree_materialized_snapshot(None)?;
    }
    Ok(result)
}

pub(super) fn restore_workspace_paths_selected(
    repo: &RepoRuntime,
    target_snapshot_id: Option<&str>,
    paths: &[String],
    baseline_snapshot_id: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let requested_paths = paths
        .iter()
        .map(|path| normalize_workspace_restore_path(path))
        .collect::<Result<Vec<_>, _>>()?;
    let snapshot_rules_text = snapshot_rules_state_from_snapshot_id(repo, target_snapshot_id)?.text;
    let target_entries = filtered_snapshot_path_rows(
        repo,
        target_snapshot_id,
        &requested_paths,
        snapshot_rules_text.as_deref(),
    )?;
    let baseline_entries =
        filtered_snapshot_path_rows(repo, baseline_snapshot_id, &requested_paths, None)?;
    let workspace_files = workspace_state_for_exact_paths(repo, &requested_paths)?;
    let dirty_selected_paths = requested_paths
        .iter()
        .filter(|path| {
            match (
                baseline_entries.get(path.as_str()),
                workspace_files.get(*path),
            ) {
                (None, None) => false,
                (Some(baseline), Some(current)) => {
                    current.sha256 != file_map_row_sha256(baseline).unwrap_or_default()
                        || current.mode != file_map_row_mode(baseline).unwrap_or_default()
                }
                _ => true,
            }
        })
        .cloned()
        .collect::<Vec<_>>();

    let mut write_paths = Vec::new();
    let mut remove_paths = Vec::new();
    let mut unchanged_paths = Vec::new();
    for rel in &requested_paths {
        let target = target_entries.get(rel.as_str());
        let current = workspace_files.get(rel);
        match (target, current) {
            (None, None) => unchanged_paths.push(rel.clone()),
            (None, Some(_)) => remove_paths.push(rel.clone()),
            (Some(target_row), Some(current_state))
                if current_state.sha256 == file_map_row_sha256(target_row).unwrap_or_default()
                    && current_state.mode == file_map_row_mode(target_row).unwrap_or_default() =>
            {
                unchanged_paths.push(rel.clone());
            }
            (Some(_), _) => write_paths.push(rel.clone()),
        }
    }

    let mut result = json!({
        "target_snapshot_id": target_snapshot_id,
        "baseline_snapshot_id": baseline_snapshot_id,
        "force": force,
        "dry_run": dry_run,
        "applied": false,
        "workspace_dirty": !dirty_selected_paths.is_empty(),
        "would_overwrite_selected_changes": !dirty_selected_paths.is_empty(),
        "dirty_workspace": {
            "scope": "requested_paths",
            "clean": dirty_selected_paths.is_empty(),
            "changed_count": dirty_selected_paths.len(),
            "changed_paths": dirty_selected_paths,
        },
        "dirty_selected_paths": dirty_selected_paths,
        "dirty_outside_paths": [],
        "outside_paths_enumerated": false,
        "workspace_read_scope": "requested_paths",
        "plan": {
            "write_count": write_paths.len(),
            "remove_count": remove_paths.len(),
            "unchanged_count": unchanged_paths.len(),
            "requested_paths": requested_paths,
            "write_paths": sort_paths(write_paths.clone()),
            "remove_paths": reverse_depth_sort_paths(remove_paths.clone()),
            "unchanged_paths": unchanged_paths,
        },
    });
    if !dirty_selected_paths.is_empty() && !force && !dry_run {
        let baseline_label = baseline_snapshot_id.unwrap_or("empty workspace");
        return Err(format!(
            "Selected paths have unsaved changes relative to {baseline_label}: {}",
            summarize_path_sample(&dirty_selected_paths)
        ));
    }
    if dry_run {
        return Ok(result);
    }

    let workspace_root = repo.workspace_root();
    for rel in reverse_depth_sort_paths(remove_paths.clone()) {
        let abs_path = workspace_root.join(&rel);
        if abs_path.exists() {
            fs::remove_file(&abs_path).map_err(|err| err.to_string())?;
            prune_empty_parent_dirs(&workspace_root, &abs_path)?;
        }
    }
    let sorted_write_paths = sort_paths(write_paths.clone());
    let mut write_blob_ids = Vec::with_capacity(sorted_write_paths.len());
    for rel in &sorted_write_paths {
        let target = target_entries
            .get(rel.as_str())
            .ok_or_else(|| format!("Snapshot row is missing `{rel}`."))?;
        let blob_id = file_map_row_blob_id(target)
            .ok_or_else(|| format!("Snapshot row is missing blob_id for `{rel}`."))?;
        write_blob_ids.push(blob_id);
    }
    let blob_bytes_by_id = read_selected_snapshot_blob_bytes_batch(repo, &write_blob_ids)?;
    for rel in sorted_write_paths {
        let target = target_entries
            .get(rel.as_str())
            .ok_or_else(|| format!("Snapshot row is missing `{rel}`."))?;
        let blob_id = file_map_row_blob_id(target)
            .ok_or_else(|| format!("Snapshot row is missing blob_id for `{rel}`."))?;
        let abs_path = workspace_root.join(&rel);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        if abs_path.exists() && abs_path.is_dir() {
            return Err(format!("Cannot restore file over directory: {rel}"));
        }
        let data = blob_bytes_by_id
            .get(&blob_id)
            .ok_or_else(|| format!("Snapshot blob payload is missing for `{rel}` ({blob_id})."))?;
        fs::write(&abs_path, data).map_err(|err| err.to_string())?;
        let mode = parse_mode_bits(file_map_row_mode(target).as_deref())?;
        set_portable_mode(&abs_path, mode).map_err(|err| err.to_string())?;
    }
    result
        .as_object_mut()
        .expect("restore workspace payload")
        .insert("applied".to_string(), JsonValue::Bool(true));
    Ok(result)
}

pub(super) fn restore_workspace_all(
    repo: &RepoRuntime,
    target_snapshot_id: Option<&str>,
    baseline_snapshot_id: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let snapshot_rules_text = snapshot_rules_state_from_snapshot_id(repo, target_snapshot_id)?.text;
    let target_rows =
        filtered_snapshot_rows_json(repo, target_snapshot_id, snapshot_rules_text.as_deref())?;
    let total_target_row_count = target_rows.len();
    let effective_ignore_rules = effective_ignore_rules_text(repo, snapshot_rules_text.as_deref())?;
    let dirty =
        workspace_delta_payload(repo, baseline_snapshot_id, snapshot_rules_text.as_deref())?;
    let mut workspace_files = workspace_state(repo, effective_ignore_rules.as_deref())?;

    let mut write_rows = Vec::new();
    for target_row in target_rows {
        let rel = snapshot_row_path(&target_row)
            .ok_or_else(|| "snapshot row is missing path".to_string())?;
        let current = workspace_files.remove(&rel);
        match current {
            Some(current_state)
                if current_state.sha256 == file_map_row_sha256(&target_row).unwrap_or_default()
                    && current_state.mode == file_map_row_mode(&target_row).unwrap_or_default() => {
            }
            _ => write_rows.push((rel, target_row)),
        }
    }
    let remove_paths =
        reverse_depth_sort_paths(workspace_files.keys().cloned().collect::<Vec<_>>());
    let result = json!({
        "target_snapshot_id": target_snapshot_id,
        "baseline_snapshot_id": baseline_snapshot_id,
        "force": force,
        "dry_run": dry_run,
        "applied": false,
        "workspace_dirty": !dirty.get("clean").and_then(JsonValue::as_bool).unwrap_or(false),
        "would_overwrite_workspace_changes": !dirty.get("clean").and_then(JsonValue::as_bool).unwrap_or(false),
        "dirty_workspace": dirty,
        "plan": {
            "write_count": write_rows.len(),
            "remove_count": remove_paths.len(),
            "unchanged_count": total_target_row_count.saturating_sub(write_rows.len()),
            "write_paths": sort_paths(write_rows.iter().map(|(path, _)| path.clone()).collect::<Vec<_>>()),
            "remove_paths": remove_paths.clone(),
        },
    });
    let dirty_changed_paths = json_string_list(
        result
            .get("dirty_workspace")
            .and_then(JsonValue::as_object)
            .and_then(|payload| payload.get("changed_paths")),
    );
    if !dirty_changed_paths.is_empty() && !force && !dry_run {
        let baseline_label = baseline_snapshot_id.unwrap_or("empty workspace");
        return Err(format!(
            "Workspace has unsaved changes relative to {baseline_label}: {}",
            summarize_path_sample(&dirty_changed_paths)
        ));
    }
    if dry_run {
        return Ok(result);
    }

    let workspace_root = repo.workspace_root();
    for rel in &remove_paths {
        let abs_path = workspace_root.join(rel);
        if abs_path.exists() {
            fs::remove_file(&abs_path).map_err(|err| err.to_string())?;
            prune_empty_parent_dirs(&workspace_root, &abs_path)?;
        }
    }
    write_rows.sort_by(|left, right| left.0.cmp(&right.0));
    for (rel, target) in write_rows {
        let blob_id = file_map_row_blob_id(&target)
            .ok_or_else(|| format!("Snapshot row is missing blob_id for `{rel}`."))?;
        let abs_path = workspace_root.join(&rel);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        if abs_path.exists() && abs_path.is_dir() {
            return Err(format!("Cannot restore file over directory: {rel}"));
        }
        let data = read_selected_snapshot_blob_bytes(repo, &blob_id)?;
        fs::write(&abs_path, data).map_err(|err| err.to_string())?;
        let mode = parse_mode_bits(file_map_row_mode(&target).as_deref())?;
        set_portable_mode(&abs_path, mode).map_err(|err| err.to_string())?;
    }
    let mut applied = result;
    applied
        .as_object_mut()
        .expect("restore workspace payload")
        .insert("applied".to_string(), JsonValue::Bool(true));
    Ok(applied)
}

pub(super) fn workspace_delta_payload(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    snapshot_rules_text: Option<&str>,
) -> Result<JsonValue, String> {
    let _delta_range = perfetto_range!("ait.cli.workspace_delta");
    let total_started = Instant::now();
    let baseline_started = Instant::now();
    let (effective_ignore_rules, ignore_rules_hash, baseline_manifest) = {
        let _range = perfetto_range!("ait.cli.workspace_delta.baseline");
        let effective_ignore_rules = effective_ignore_rules_text(repo, snapshot_rules_text)?;
        let ignore_rules_hash =
            status_manifest_ignore_rules_hash(repo, effective_ignore_rules.as_deref());
        let baseline_manifest = status_baseline_manifest(
            repo,
            snapshot_id,
            effective_ignore_rules.as_deref(),
            &ignore_rules_hash,
        )?;
        (effective_ignore_rules, ignore_rules_hash, baseline_manifest)
    };
    let baseline_snapshot_read = elapsed_ms(baseline_started);
    let workspace_started = Instant::now();
    let snapshot_key = snapshot_id
        .and_then(|value| normalized_text(Some(value)))
        .unwrap_or_else(|| "empty".to_string());
    let workspace_scan = {
        let _range = perfetto_range!("ait.cli.workspace_delta.scan_hash");
        workspace_state_for_status(
            repo,
            &snapshot_key,
            &ignore_rules_hash,
            &baseline_manifest.index,
            effective_ignore_rules.as_deref(),
            baseline_manifest.hash_cache.as_ref(),
        )?
    };
    let workspace_projection_filter = elapsed_ms(workspace_started);
    let WorkspaceStatusScan {
        files,
        tracked_fingerprints,
        operational_external_roots,
        reused_paths,
        rehashed_paths,
        cache_read,
    } = workspace_scan;
    let compare_started = Instant::now();
    let (mut modified_paths, mut missing_paths, mut untracked_paths) = {
        let _range = perfetto_range!("ait.cli.workspace_delta.compare");
        let mut modified_paths = Vec::new();
        let mut missing_paths = Vec::new();
        let mut remaining = files;
        for snapshot_row in &baseline_manifest.index.rows {
            let path = baseline_manifest.index.row_path(snapshot_row)?;
            let current = remaining.remove(path);
            match current {
                None => missing_paths.push(path.to_string()),
                Some(current) => {
                    if current.sha256 != snapshot_row.sha256 || current.mode != snapshot_row.mode {
                        modified_paths.push(path.to_string());
                    }
                }
            }
        }
        let untracked_paths = remaining.keys().cloned().collect::<Vec<_>>();
        (modified_paths, missing_paths, untracked_paths)
    };
    modified_paths.sort();
    missing_paths.sort();
    untracked_paths.sort();
    let changed_paths = modified_paths
        .iter()
        .chain(missing_paths.iter())
        .chain(untracked_paths.iter())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let compare_manifest = elapsed_ms(compare_started);
    let cache_write = repair_workspace_hash_cache_after_clean_status(
        repo,
        snapshot_id,
        &baseline_manifest,
        &tracked_fingerprints,
        changed_paths.is_empty(),
    )?;
    let runtime_root = active_workspace_runtime_root(&repo.workspace_root());
    let baseline_manifest_source = baseline_manifest.source.clone();
    let baseline_manifest_path = baseline_manifest
        .manifest_path
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    let baseline_manifest_row_count = baseline_manifest.index.rows.len();
    Ok(json!({
        "snapshot_id": snapshot_id,
        "clean": changed_paths.is_empty(),
        "changed_count": changed_paths.len(),
        "changed_paths": changed_paths,
        "modified_paths": modified_paths,
        "missing_paths": missing_paths,
        "untracked_paths": untracked_paths,
        "baseline_manifest": {
            "source": baseline_manifest_source,
            "path": baseline_manifest_path,
            "ignore_rules_hash": ignore_rules_hash,
            "row_count": baseline_manifest_row_count,
        },
        "ignore_policy": ignore_policy_payload(
            effective_ignore_rules.as_deref(),
            runtime_root.as_deref(),
            &operational_external_roots,
        ),
        "phase_timings_ms": {
            "baseline_snapshot_read": baseline_snapshot_read,
            "workspace_scan": workspace_projection_filter,
            "ignore_filtering": 0.0,
            "hashing": workspace_projection_filter,
            "hashing_cache": {
                "reused_paths": reused_paths,
                "rehashed_paths": rehashed_paths,
                "state_read": cache_read,
                "state_write": cache_write,
            },
            "workspace_projection_filter": workspace_projection_filter,
            "compare_manifest": compare_manifest,
            "total": elapsed_ms(total_started),
        },
    }))
}

pub fn workflow_workspace_status(
    repo: &RepoRuntime,
    snapshot_id: Option<&str>,
    line_name: Option<&str>,
) -> Result<JsonValue, String> {
    if snapshot_id.is_some() && line_name.is_some() {
        return Err("Choose either snapshot_id or line_name, not both.".to_string());
    }
    let current_line_name = repo.current_line_name()?;
    let mut baseline_source = if snapshot_id.is_some() {
        "snapshot".to_string()
    } else {
        "line".to_string()
    };
    let mut baseline_line_name = None::<String>;
    let baseline_snapshot_id = if let Some(snapshot_id) = normalized_text(snapshot_id) {
        Some(snapshot_id)
    } else {
        let resolved_line_name =
            normalized_text(line_name).unwrap_or_else(|| current_line_name.clone());
        let baseline_line_row = local_line_row(repo, &resolved_line_name)?;
        baseline_source = if resolved_line_name == current_line_name {
            "current_line_head".to_string()
        } else {
            "line_head".to_string()
        };
        baseline_line_name = Some(resolved_line_name);
        string_field(&baseline_line_row, "head_snapshot_id")
    };
    let delta = workspace_delta_payload(repo, baseline_snapshot_id.as_deref(), None)?;
    Ok(json!({
        "repo_name": repo.repo_name(),
        "workspace_root": repo.workspace_root().to_string_lossy().to_string(),
        "is_worktree": repo.is_worktree(),
        "worktree_name": repo
            .worktree_config_path
            .as_ref()
            .and_then(|path| {
                read_json_document(path)
                    .as_object()
                    .and_then(|value| value.get("worktree_name"))
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            }),
        "current_line": current_line_name,
        "baseline_source": baseline_source,
        "baseline_line_name": baseline_line_name,
        "baseline_snapshot_id": baseline_snapshot_id,
        "clean": delta.get("clean").cloned().unwrap_or(JsonValue::Bool(false)),
        "changed_count": delta.get("changed_count").cloned().unwrap_or(JsonValue::from(0)),
        "changed_paths": delta.get("changed_paths").cloned().unwrap_or(JsonValue::Array(Vec::new())),
        "modified_paths": delta.get("modified_paths").cloned().unwrap_or(JsonValue::Array(Vec::new())),
        "missing_paths": delta.get("missing_paths").cloned().unwrap_or(JsonValue::Array(Vec::new())),
        "untracked_paths": delta.get("untracked_paths").cloned().unwrap_or(JsonValue::Array(Vec::new())),
        "baseline_manifest": delta.get("baseline_manifest").cloned().unwrap_or(JsonValue::Null),
        "ignore_policy": delta.get("ignore_policy").cloned().unwrap_or(JsonValue::Null),
        "phase_timings_ms": delta.get("phase_timings_ms").cloned().unwrap_or(JsonValue::Null),
    }))
}

pub fn workspace_dirty_diff(
    repo: &RepoRuntime,
    paths: &[String],
    max_bytes: usize,
) -> Result<JsonValue, String> {
    let current_line_name = repo.current_line_name()?;
    let baseline_line_row = local_line_row(repo, &current_line_name)?;
    let baseline_snapshot_id = string_field(&baseline_line_row, "head_snapshot_id");
    let effective_ignore_rules = effective_ignore_rules_text(repo, None)?;
    let ignore_rules_hash =
        status_manifest_ignore_rules_hash(repo, effective_ignore_rules.as_deref());
    let baseline_manifest = status_baseline_manifest(
        repo,
        baseline_snapshot_id.as_deref(),
        effective_ignore_rules.as_deref(),
        &ignore_rules_hash,
    )?;
    let snapshot_key = baseline_snapshot_id
        .as_deref()
        .and_then(|value| normalized_text(Some(value)))
        .unwrap_or_else(|| "empty".to_string());
    let workspace_scan = workspace_state_for_status(
        repo,
        &snapshot_key,
        &ignore_rules_hash,
        &baseline_manifest.index,
        effective_ignore_rules.as_deref(),
        baseline_manifest.hash_cache.as_ref(),
    )?;
    let path_filters = normalize_workspace_diff_paths(paths)?;
    let path_filter_set = path_filters.iter().cloned().collect::<BTreeSet<_>>();

    let mut remaining = workspace_scan.files;
    let mut entries = Vec::new();
    for row in &baseline_manifest.index.rows {
        let path = baseline_manifest.index.row_path(row)?.to_string();
        let current = remaining.remove(&path);
        match current {
            None => {
                if workspace_diff_path_selected(&path, &path_filter_set) {
                    let blob_id = baseline_manifest.index.row_blob_id(row)?;
                    entries.push(WorkspaceDiffEntry {
                        path,
                        status: "missing".to_string(),
                        old_bytes: Some(read_selected_snapshot_blob_bytes(repo, blob_id)?),
                        new_bytes: None,
                        old_mode: Some(row.mode.clone()),
                        new_mode: None,
                    });
                }
            }
            Some(current) => {
                if (current.sha256 != row.sha256 || current.mode != row.mode)
                    && workspace_diff_path_selected(&path, &path_filter_set)
                {
                    let blob_id = baseline_manifest.index.row_blob_id(row)?;
                    entries.push(WorkspaceDiffEntry {
                        path: path.clone(),
                        status: "modified".to_string(),
                        old_bytes: Some(read_selected_snapshot_blob_bytes(repo, blob_id)?),
                        new_bytes: Some(read_workspace_file_bytes(repo, &path)?),
                        old_mode: Some(row.mode.clone()),
                        new_mode: Some(current.mode),
                    });
                }
            }
        }
    }
    for (path, current) in remaining {
        if workspace_diff_path_selected(&path, &path_filter_set) {
            entries.push(WorkspaceDiffEntry {
                path: path.clone(),
                status: "untracked".to_string(),
                old_bytes: None,
                new_bytes: Some(read_workspace_file_bytes(repo, &path)?),
                old_mode: None,
                new_mode: Some(current.mode),
            });
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let baseline_label = baseline_snapshot_id.as_deref().unwrap_or("baseline");
    let mut payload =
        workspace_diff_from_entries(&entries, baseline_label, "workspace", true, max_bytes);
    let worktree_name = repo.worktree_config_path.as_ref().and_then(|path| {
        read_json_document(path)
            .as_object()
            .and_then(|value| value.get("worktree_name"))
            .and_then(JsonValue::as_str)
            .map(str::to_string)
    });
    if let Some(root) = payload.as_object_mut() {
        root.insert("repo_name".to_string(), JsonValue::String(repo.repo_name()));
        root.insert(
            "workspace_root".to_string(),
            JsonValue::String(repo.workspace_root().to_string_lossy().to_string()),
        );
        root.insert(
            "is_worktree".to_string(),
            JsonValue::Bool(repo.is_worktree()),
        );
        root.insert(
            "worktree_name".to_string(),
            worktree_name
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        );
        root.insert(
            "current_line".to_string(),
            JsonValue::String(current_line_name.clone()),
        );
        root.insert(
            "baseline_source".to_string(),
            JsonValue::String("current_line_head".to_string()),
        );
        root.insert(
            "baseline_line_name".to_string(),
            JsonValue::String(current_line_name),
        );
        root.insert(
            "baseline_snapshot_id".to_string(),
            baseline_snapshot_id
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        );
        root.insert("clean".to_string(), JsonValue::Bool(entries.is_empty()));
        root.insert(
            "path_filters".to_string(),
            JsonValue::Array(path_filters.into_iter().map(JsonValue::String).collect()),
        );
    }
    Ok(payload)
}

pub fn workspace_delta(repo: &RepoRuntime, snapshot_id: Option<&str>) -> Result<JsonValue, String> {
    workspace_delta_payload(repo, snapshot_id, None)
}

pub fn workspace_restore(
    repo: &RepoRuntime,
    target_snapshot_id: Option<&str>,
    baseline_snapshot_id: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    restore_workspace_all(
        repo,
        target_snapshot_id,
        baseline_snapshot_id,
        force,
        dry_run,
    )
}

pub fn workspace_restore_paths(
    repo: &RepoRuntime,
    target_snapshot_id: Option<&str>,
    paths: &[String],
    baseline_snapshot_id: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    restore_workspace_paths_selected(
        repo,
        target_snapshot_id,
        paths,
        baseline_snapshot_id,
        force,
        dry_run,
    )
}

#[cfg(test)]
mod selected_binary_line_tests {
    use super::*;
    use crate::primitives::workflow::workflow_repo_root_restore_after_land;
    use ait_core::line_store::LineStore;
    use ait_core::local_snapshot::LocalSnapshotWriteStore;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn worktree_metadata_derives_change_ref_from_task_and_short_change_id() {
        let metadata = worktree_metadata_from_payload(
            &json!({
                "name": "task-worktree",
                "bound_task_id": "RT-2",
                "bound_change_id": "RT-2/C-01",
                "bound_change_ref": "RT-WRONG/C-01"
            }),
            "fallback",
        );
        assert_eq!(metadata.bound_task_id.as_deref(), Some("RT-2"));
        assert_eq!(metadata.bound_change_id.as_deref(), Some("C-01"));
        assert_eq!(metadata.bound_change_ref.as_deref(), Some("RT-2/C-01"));
    }

    #[test]
    fn bound_worktree_lookup_scopes_duplicate_short_ids_by_task_or_change_ref() {
        let (_temp, repo) = binary_snapshot_repo();
        let registry = repo.ait_dir.join("worktrees");
        write_file(
            &registry.join("rt-1.json"),
            &json!({
                "name": "rt-1",
                "bound_task_id": "RT-1",
                "bound_change_id": "C-01",
                "bound_change_ref": "RT-1/C-01",
                "auto_created_for_task": true,
            })
            .to_string(),
        );
        write_file(
            &registry.join("rt-2.json"),
            &json!({
                "name": "rt-2",
                "bound_task_id": "RT-2",
                "bound_change_id": "C-01",
                "bound_change_ref": "RT-2/C-01",
                "auto_created_for_task": true,
            })
            .to_string(),
        );

        let by_task = bound_task_worktree_metadata(&repo, Some("RT-2"), Some("C-01"))
            .expect("task-scoped lookup")
            .expect("task-scoped worktree");
        assert_eq!(by_task.name, "rt-2");

        let by_ref = bound_task_worktree_metadata(&repo, None, Some("RT-1/C-01"))
            .expect("ref-scoped lookup")
            .expect("ref-scoped worktree");
        assert_eq!(by_ref.name, "rt-1");

        let error = bound_task_worktree_metadata(&repo, None, Some("C-01"))
            .expect_err("bare duplicate short id must fail closed");
        assert!(error.contains("matches multiple task worktrees"));
        assert!(error.contains("RT-1/C-01"));
        assert!(error.contains("RT-2/C-01"));
    }

    #[cfg(unix)]
    fn register_routable_worktree(
        repo: &RepoRuntime,
        target_root: &Path,
        name: &str,
        task_id: &str,
        change_id: &str,
        registered_line: &str,
        overlay_line: &str,
    ) -> RepoRuntime {
        fs::create_dir_all(target_root.join("src")).expect("create worktree root");
        std::os::unix::fs::symlink(&repo.ait_dir, target_root.join(".ait"))
            .expect("link shared .ait");
        write_file(
            &target_root.join(".ait-worktree.json"),
            &json!({
                "worktree_name": name,
                "current_line": overlay_line,
                "repo_root": repo.authoritative_repo_root(),
                "workspace_root": target_root,
            })
            .to_string(),
        );
        write_file(
            &repo.ait_dir.join("worktrees").join(format!("{name}.json")),
            &json!({
                "name": name,
                "path": target_root,
                "repo_root": repo.authoritative_repo_root(),
                "line_name": registered_line,
                "bound_task_id": task_id,
                "bound_change_id": change_id,
                "bound_change_ref": format!("{task_id}/{change_id}"),
                "auto_created_for_task": true,
            })
            .to_string(),
        );
        RepoRuntime::discover_from_path(target_root).expect("discover worktree")
    }

    #[cfg(unix)]
    #[test]
    fn exact_task_id_routes_root_to_verified_bound_worktree() {
        let (_root_temp, repo) = binary_snapshot_repo();
        let target = TempDir::new().expect("worktree tempdir");
        register_routable_worktree(
            &repo,
            target.path(),
            "rt-1",
            "RT-1",
            "C-01",
            "feature/rt-1",
            "feature/rt-1",
        );

        let routed =
            resolve_task_scoped_execution_repo(&repo, "RT-1", false, false, None, "test route")
                .expect("route exact Task");

        assert!(routed.is_worktree());
        assert_eq!(
            routed.workspace_root().canonicalize().unwrap(),
            target.path().canonicalize().unwrap()
        );

        let mut observed_target_lock = false;
        run_task_scoped_workspace_command(
            &repo,
            "RT-1",
            false,
            false,
            None,
            "test locked route",
            |execution_repo| {
                assert_eq!(
                    execution_repo.workspace_root().canonicalize().unwrap(),
                    target.path().canonicalize().unwrap()
                );
                let lock = crate::workspace_lock::workspace_command_lock_path(execution_repo);
                let metadata = fs::read_to_string(lock).expect("target lock metadata");
                assert!(metadata.contains("test locked route"));
                observed_target_lock = true;
                Ok(())
            },
        )
        .expect("run under exact target lock");
        assert!(observed_target_lock);
    }

    #[cfg(unix)]
    #[test]
    fn root_routing_rejects_duplicate_task_bindings_and_ambiguous_short_changes() {
        let (_root_temp, repo) = binary_snapshot_repo();
        let first = TempDir::new().expect("first worktree tempdir");
        let second = TempDir::new().expect("second worktree tempdir");
        register_routable_worktree(
            &repo,
            first.path(),
            "rt-1-a",
            "RT-1",
            "C-01",
            "feature/rt-1-a",
            "feature/rt-1-a",
        );
        register_routable_worktree(
            &repo,
            second.path(),
            "rt-1-b",
            "RT-1",
            "C-01",
            "feature/rt-1-b",
            "feature/rt-1-b",
        );

        let duplicate =
            resolve_task_scoped_execution_repo(&repo, "RT-1", false, false, None, "test route")
                .expect_err("duplicate Task bindings must fail");
        assert!(duplicate.contains("multiple bound worktrees"));

        let ambiguous =
            resolve_task_scoped_execution_repo(&repo, "C-01", false, false, None, "test route")
                .expect_err("ambiguous short Change must fail");
        assert!(ambiguous.contains("routing is ambiguous"));
    }

    #[test]
    fn root_routing_rejects_missing_bound_worktree_path() {
        let (_root_temp, repo) = binary_snapshot_repo();
        let missing = repo.workspace_root().join("missing-task-worktree");
        write_file(
            &repo.ait_dir.join("worktrees/rt-missing.json"),
            &json!({
                "name": "rt-missing",
                "path": missing,
                "repo_root": repo.authoritative_repo_root(),
                "line_name": "feature/rt-missing",
                "bound_task_id": "RT-404",
                "bound_change_id": "C-01",
            })
            .to_string(),
        );

        let error =
            resolve_task_scoped_execution_repo(&repo, "RT-404", false, false, None, "test route")
                .expect_err("missing worktree must fail");
        assert!(error.contains("missing or detached"));
        assert!(error.contains("ait worktree recreate rt-missing"));
    }

    #[cfg(unix)]
    #[test]
    fn root_routing_rejects_mismatched_overlay_and_wrong_current_worktree() {
        let (_root_temp, repo) = binary_snapshot_repo();
        let first = TempDir::new().expect("first worktree tempdir");
        let second = TempDir::new().expect("second worktree tempdir");
        let first_repo = register_routable_worktree(
            &repo,
            first.path(),
            "rt-1",
            "RT-1",
            "C-01",
            "feature/rt-1",
            "feature/wrong-overlay",
        );
        register_routable_worktree(
            &repo,
            second.path(),
            "rt-2",
            "RT-2",
            "C-01",
            "feature/rt-2",
            "feature/rt-2",
        );

        let mismatch =
            resolve_task_scoped_execution_repo(&repo, "RT-1", false, false, None, "test route")
                .expect_err("overlay mismatch must fail");
        assert!(mismatch.contains("no longer exactly matches"));

        let wrong_worktree = resolve_task_scoped_execution_repo(
            &first_repo,
            "RT-2",
            false,
            false,
            None,
            "test route",
        )
        .expect_err("wrong current worktree must fail");
        assert!(wrong_worktree.contains("bound to Task `RT-1`, not Task `RT-2`"));
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, content).expect("write file");
    }

    fn binary_snapshot_repo() -> (TempDir, RepoRuntime) {
        let temp = TempDir::new().expect("repo tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join(".ait")).expect("create .ait");
        write_file(
            &root.join(".ait/config.json"),
            r#"{"repo_name":"fixture-ait","default_line":"main","snapshot_binary_db_storage":"binary"}"#,
        );
        let repo = RepoRuntime::discover_from_path(root).expect("discover runtime");
        repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .lines()
            .create_line("main", None, "2026-07-08T00:00:00Z")
            .expect("create Binary DB line");
        (temp, repo)
    }

    #[test]
    fn selected_binary_line_head_helpers_restore_binary_head_without_retired_backend_line_fallback()
    {
        let (_temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        write_file(&root.join("hello.txt"), "first\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let snapshot = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("first"), false)
            .expect("create Binary DB snapshot");
        let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");

        assert_eq!(
            local_line_head_snapshot_id(&repo, "main")
                .expect("selected line head")
                .as_deref(),
            Some(snapshot_id.as_str())
        );

        set_local_line_head(&repo, "main", None).expect("clear selected line head");
        assert_eq!(
            local_line_head_snapshot_id(&repo, "main").expect("selected cleared line head"),
            None
        );
        restore_local_line_updated_at(&repo, "main", Some("2026-07-08T00:10:00Z"))
            .expect("restore selected line updated_at");
        assert_eq!(
            local_line_updated_at(&repo, "main").expect("selected line updated_at"),
            Some("2026-07-08T00:10:00Z".to_string())
        );

        let binary_line = repo
            .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .lines()
            .line_by_name("main")
            .expect("read Binary DB line")
            .expect("Binary DB line exists");
        assert_eq!(binary_line.head_snapshot_id, None);
        assert_eq!(
            binary_line.updated_at.as_deref(),
            Some("2026-07-08T00:10:00Z")
        );
    }

    #[test]
    fn selected_binary_workspace_restore_all_reads_manifest_without_retired_backend_fallback() {
        let (_temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        write_file(&root.join("hello.txt"), "target\n");
        write_file(&root.join("nested/file.txt"), "nested target\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let snapshot = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("target"), false)
            .expect("create Binary DB snapshot");
        let snapshot_id = required_string_field(&snapshot, "snapshot_id").expect("snapshot id");

        write_file(&root.join("hello.txt"), "dirty\n");
        write_file(&root.join("extra.txt"), "remove me\n");
        let restored = workspace_restore(&repo, Some(&snapshot_id), None, true, false)
            .expect("restore selected Binary DB workspace");

        assert_eq!(restored["applied"], JsonValue::Bool(true));
        assert_eq!(
            fs::read_to_string(root.join("hello.txt")).expect("restored hello"),
            "target\n"
        );
        assert_eq!(
            fs::read_to_string(root.join("nested/file.txt")).expect("restored nested file"),
            "nested target\n"
        );
        assert!(!root.join("extra.txt").exists());
    }

    #[test]
    fn first_land_restores_empty_line_delta_into_canonical_workspace() {
        let (_temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        write_file(&root.join("first-land.txt"), "landed from empty line\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let target = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("first land"), false)
            .expect("create first Binary DB snapshot");
        let target_id = required_string_field(&target, "snapshot_id").expect("target snapshot id");

        fs::remove_file(root.join("first-land.txt")).expect("simulate absent canonical file");
        let restored = workflow_repo_root_restore_after_land(&repo, "main", None, Some(&target_id))
            .expect("restore first landed snapshot");

        assert_eq!(restored["landed_diff_paths"], json!(["first-land.txt"]));
        assert_eq!(restored["plan"]["write_paths"], json!(["first-land.txt"]));
        assert_eq!(restored["outside_paths_enumerated"], json!(false));
        assert_eq!(
            fs::read_to_string(root.join("first-land.txt")).expect("restored first-land file"),
            "landed from empty line\n"
        );
    }

    #[test]
    fn selected_binary_land_delta_restores_deletions_and_preserves_dirty_outside_paths() {
        let (_temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        write_file(&root.join("replace.txt"), "old\n");
        write_file(&root.join("removed.txt"), "old\n");
        write_file(&root.join("outside.txt"), "kept\n");
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");
        let old = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("old"), false)
            .expect("create old Binary DB snapshot");
        let old_id = required_string_field(&old, "snapshot_id").expect("old snapshot id");

        write_file(&root.join("replace.txt"), "landed\n");
        fs::remove_file(root.join("removed.txt")).expect("remove target path");
        let target = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("target"), false)
            .expect("create target Binary DB snapshot");
        let target_id = required_string_field(&target, "snapshot_id").expect("target snapshot id");
        let delta = snapshot_store
            .snapshot_tree_path_delta(Some(&old_id), Some(&target_id))
            .expect("exact landed path delta");
        assert_eq!(
            delta.affected_paths,
            vec!["removed.txt".to_string(), "replace.txt".to_string()]
        );

        write_file(&root.join("replace.txt"), "stale\n");
        write_file(&root.join("removed.txt"), "stale\n");
        write_file(&root.join("outside.txt"), "dirty but unrelated\n");
        let restored = restore_workspace_paths_selected(
            &repo,
            Some(&target_id),
            &delta.affected_paths,
            Some(&old_id),
            true,
            false,
        )
        .expect("apply exact landed Binary DB delta");

        assert_eq!(restored["applied"], JsonValue::Bool(true));
        assert_eq!(
            fs::read_to_string(root.join("replace.txt")).expect("restored landed file"),
            "landed\n"
        );
        assert!(!root.join("removed.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("outside.txt")).expect("preserved unrelated file"),
            "dirty but unrelated\n"
        );
        assert_eq!(restored["workspace_read_scope"], json!("requested_paths"));
        assert_eq!(restored["outside_paths_enumerated"], json!(false));
        assert_eq!(restored["dirty_outside_paths"], json!([]));
        assert!(!restored.to_string().contains("outside.txt"));

        write_file(&root.join("replace.txt"), "dirty selected path\n");
        let error = restore_workspace_paths_selected(
            &repo,
            Some(&target_id),
            &["replace.txt".to_string()],
            Some(&target_id),
            false,
            false,
        )
        .expect_err("non-forced exact restore must reject a dirty selected path");
        assert!(error.contains("replace.txt"));
    }
}
