use super::*;
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;
use ait_core::line_store::{LineRecord, LineStore};
use ait_core::snapshot_store::SnapshotStore;
#[cfg(test)]
use ait_core::task_workflow_store::{
    list_changes_with_task_workflow_change_store, list_tasks_with_task_workflow_task_store,
};

mod bootstrap;
mod cleanup;
mod main_seed;
mod rebase;
mod registry;
mod restore;
mod status;

pub(crate) use bootstrap::task_start_with_progress;
pub(in crate::primitives) use bootstrap::*;
pub use bootstrap::{task_resolve_worktree_location, task_start, worktree_recover_task};
pub(super) use cleanup::{
    cleanup_candidate_sort_key, cleanup_registered_worktree_cargo_build_dir,
    coerce_worktree_datetime, finalize_promoted_worktree_registration, remove_one_worktree,
    remove_one_worktree_after_authoritative_task_land, worktree_cleanup_decision,
};
pub use cleanup::{
    worktree_cleanup, worktree_cleanup_candidates, worktree_prune_stale, worktree_remove,
};
pub(in crate::primitives) use main_seed::*;
pub use main_seed::{task_ensure_main_seed_mirror, task_resolve_main_seed_mirror_location};
pub(super) use rebase::{
    apply_prepared_worktree_rebase, prepare_worktree_rebase_to_snapshot, set_local_line_head,
};
pub(in crate::primitives) use rebase::{
    read_worktree_snapshot_blob_bytes, write_workspace_snapshot_row,
};
pub use rebase::{
    worktree_abort_rebase, worktree_continue_rebase, worktree_preview_rebase, worktree_rebase,
    worktree_restore_owned_head,
};
pub use registry::worktree_bind_existing;
pub(in crate::primitives) use registry::*;
pub use restore::{
    worktree_recreate, worktree_restore, worktree_sync, worktree_sync_all, worktree_touch_usage,
};
pub(in crate::primitives) use status::*;
pub use status::{worktree_doctor, worktree_get, worktree_list, worktree_status};

pub(super) fn create_local_line(
    repo: &RepoRuntime,
    line_name: &str,
    head_snapshot_id: Option<&str>,
) -> Result<JsonValue, String> {
    let created_at = system_event_timestamp();
    let store = line_store(repo)?;
    create_local_line_with_line_store(&store, line_name, head_snapshot_id, &created_at)
}

pub(super) fn list_local_lines(repo: &RepoRuntime) -> Result<Vec<JsonValue>, String> {
    let store = line_store(repo)?;
    list_local_lines_with_line_store(&store)
}

pub(super) fn count_local_lines(repo: &RepoRuntime) -> Result<usize, String> {
    line_store(repo)?.line_count()
}

pub(super) fn list_local_lines_with_line_store<S>(store: &S) -> Result<Vec<JsonValue>, String>
where
    S: LineStore + ?Sized,
{
    store
        .list_lines()?
        .into_iter()
        .map(|line| Ok(line_record_json(&line)))
        .collect()
}

pub(super) fn create_local_line_with_line_store<S>(
    store: &S,
    line_name: &str,
    head_snapshot_id: Option<&str>,
    created_at: &str,
) -> Result<JsonValue, String>
where
    S: LineStore + ?Sized,
{
    store
        .create_line(line_name, head_snapshot_id, created_at)
        .map(|line| line_record_json(&line))
}

pub(super) fn line_record_json(line: &LineRecord) -> JsonValue {
    json!({
        "line_id": &line.line_id,
        "line_name": &line.line_name,
        "status": &line.status,
        "archived_at": &line.archived_at,
        "created_at": &line.created_at,
        "updated_at": &line.updated_at,
        "head_snapshot_id": &line.head_snapshot_id,
    })
}

pub(super) fn archive_local_line(repo: &RepoRuntime, line_name: &str) -> Result<JsonValue, String> {
    let archived_at = system_event_timestamp();
    let store = line_store(repo)?;
    archive_local_line_with_line_store(&store, line_name, &archived_at)
}

pub(super) fn archive_local_line_with_line_store<S>(
    store: &S,
    line_name: &str,
    archived_at: &str,
) -> Result<JsonValue, String>
where
    S: LineStore + ?Sized,
{
    store
        .archive_line(line_name, archived_at)
        .map(|line| line_record_json(&line))
}

#[cfg(test)]
pub(super) fn set_local_line_head_with_line_store<S>(
    store: &S,
    line_name: &str,
    head_snapshot_id: Option<&str>,
    updated_at: &str,
) -> Result<JsonValue, String>
where
    S: LineStore + ?Sized,
{
    store
        .set_line_head(line_name, head_snapshot_id, updated_at)
        .map(|line| line_record_json(&line))
}

fn line_store(repo: &RepoRuntime) -> Result<impl LineStore, String> {
    repo.line_store()
}

fn snapshot_store(repo: &RepoRuntime) -> Result<impl SnapshotStore, String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
}

pub(super) fn normalize_line_cleanup_kind(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(text) = normalized_text(value) else {
        return Ok(None);
    };
    let normalized = text.to_ascii_lowercase().replace('-', "_");
    match normalized.as_str() {
        "review_base" | "review" | "wip" => Ok(Some(normalized)),
        _ => Err("`--kind` must be one of `review_base`, `review`, or `wip`.".to_string()),
    }
}

pub(super) fn line_cleanup_profile(line_name: &str) -> (&'static str, &'static str, &'static str) {
    if line_name.starts_with("review-base/") {
        return (
            "review_base",
            "after_idle",
            "review-base line idle past threshold",
        );
    }
    if line_name.starts_with("review/") {
        return ("review", "after_idle", "review line idle past threshold");
    }
    if line_name.starts_with("wip/") {
        return ("wip", "after_idle", "wip line idle past threshold");
    }
    ("manual", "manual_only", "manual line")
}

#[derive(Default)]
pub(super) struct LineUsageIndexes {
    pub(super) worktrees: BTreeMap<String, Vec<String>>,
    pub(super) changes: BTreeMap<String, Vec<String>>,
}

pub(super) fn collect_line_usage_indexes(repo: &RepoRuntime) -> Result<LineUsageIndexes, String> {
    let mut worktree_index: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in worktree_list(repo, false)?
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let worktree_name = string_field(&row, "name").unwrap_or_default();
        if worktree_name.is_empty() {
            continue;
        }
        for candidate_line in [
            string_field(&row, "current_line"),
            string_field(&row, "registered_line_name"),
        ] {
            if let Some(line_name) = candidate_line.filter(|value| !value.is_empty()) {
                worktree_index
                    .entry(line_name)
                    .or_default()
                    .insert(worktree_name.clone());
            }
        }
    }

    let change_store = repo.change_store()?;
    let changes = line_change_usage_index_with_change_store(&change_store)?;
    Ok(LineUsageIndexes {
        worktrees: worktree_index
            .into_iter()
            .map(|(key, value)| (key, value.into_iter().collect()))
            .collect(),
        changes,
    })
}

pub(super) fn remote_line_change_usage_index(
    repo: &RepoRuntime,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut rows = Vec::new();
    for remote_name in list_remote_names(repo)? {
        let (remote_row, repo_name) = remote_context(repo, Some(&remote_name), None)?;
        let mut remote = http_task_remote(repo, &remote_row)?;
        rows.extend(remote.list_changes(&repo_name).map_err(|error| {
            format!("Cannot verify active Change references on remote {remote_name}: {error}")
        })?);
    }
    line_change_usage_index_from_rows(rows)
}

pub(super) fn line_change_usage_index_with_change_store<S>(
    change_store: &S,
) -> Result<BTreeMap<String, Vec<String>>, String>
where
    S: ChangeStore + ?Sized,
{
    line_change_usage_index_from_rows(
        list_changes_with_change_store(change_store).map_err(|err| err.to_string())?,
    )
}

fn line_change_usage_index_from_rows(
    rows: impl IntoIterator<Item = JsonValue>,
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut change_index: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for row in rows {
        if matches!(
            string_field(&row, "status").as_deref(),
            Some("archived" | "landed" | "closed")
        ) {
            continue;
        }
        let Some(change_id) = string_field(&row, "change_ref")
            .or_else(|| string_field(&row, "change_id"))
            .and_then(|value| normalized_text(Some(&value)))
        else {
            continue;
        };
        for line_name in [
            string_field(&row, "base_line"),
            string_field(&row, "target_line"),
        ]
        .into_iter()
        .flatten()
        .filter_map(|value| normalized_text(Some(&value)))
        {
            change_index
                .entry(line_name)
                .or_default()
                .insert(change_id.clone());
        }
    }
    Ok(change_index
        .into_iter()
        .map(|(key, value)| (key, value.into_iter().collect()))
        .collect())
}

pub(super) fn line_usage_summary(line_name: &str, indexes: &LineUsageIndexes) -> JsonValue {
    let worktree_names = indexes
        .worktrees
        .get(line_name)
        .cloned()
        .unwrap_or_default();
    let active_change_ids = indexes.changes.get(line_name).cloned().unwrap_or_default();
    json!({
        "worktree_count": worktree_names.len(),
        "worktree_names": worktree_names,
        "active_change_count": active_change_ids.len(),
        "active_change_ids": active_change_ids,
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "cleanup decision keeps each retention signal explicit"
)]
pub(super) fn line_cleanup_decision(
    _repo: &RepoRuntime,
    row: &JsonValue,
    idle_for_delta: ChronoDuration,
    idle_for_label: &str,
    cleanup_kind: Option<&str>,
    indexes: &LineUsageIndexes,
    current_line_name: &str,
    default_line_name: &str,
    reference_now: DateTime<Utc>,
) -> JsonMap<String, JsonValue> {
    let line_name = string_field(row, "line_name").unwrap_or_default();
    let (lifecycle_kind, cleanup_policy, cleanup_reason) = line_cleanup_profile(&line_name);
    let usage = line_usage_summary(&line_name, indexes);
    let updated_at = string_field(row, "updated_at")
        .or_else(|| string_field(row, "created_at"))
        .unwrap_or_else(system_event_timestamp);
    let idle_long_enough =
        reference_now - coerce_worktree_datetime(Some(updated_at.as_str())) >= idle_for_delta;
    let usage_obj = usage.as_object().cloned().unwrap_or_default();

    let mut protected_reason = None::<String>;
    let mut cleanup_candidate = false;
    let mut cleanup_class = "protected".to_string();

    if let Some(requested_kind) = cleanup_kind {
        if lifecycle_kind != requested_kind {
            protected_reason = Some(format!(
                "line kind {lifecycle_kind} does not match requested cleanup kind {requested_kind}"
            ));
        }
    }
    if protected_reason.is_none()
        && string_field(row, "status").unwrap_or_else(|| "active".to_string()) == "archived"
    {
        protected_reason = Some("line is already archived".to_string());
    }
    if protected_reason.is_none() && line_name == default_line_name {
        protected_reason = Some("default line".to_string());
    }
    if protected_reason.is_none() && line_name == current_line_name {
        protected_reason = Some("current line".to_string());
    }
    if protected_reason.is_none()
        && usage_obj
            .get("worktree_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            > 0
    {
        protected_reason = Some("line is still used by a worktree".to_string());
    }
    if protected_reason.is_none()
        && usage_obj
            .get("active_change_count")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0)
            > 0
    {
        protected_reason = Some("line is still used by an active local change".to_string());
    }
    if protected_reason.is_none() && cleanup_policy == "manual_only" {
        protected_reason = Some("line lifecycle is manual_only".to_string());
    }
    if protected_reason.is_none() && !idle_long_enough {
        protected_reason = Some(format!("idle threshold {idle_for_label} not reached"));
    }
    if protected_reason.is_none() {
        cleanup_class = "safe_cleanup_candidate".to_string();
        cleanup_candidate = true;
    }

    JsonMap::from_iter([
        ("line_name".to_string(), JsonValue::String(line_name)),
        (
            "lifecycle_kind".to_string(),
            JsonValue::String(lifecycle_kind.to_string()),
        ),
        (
            "cleanup_policy".to_string(),
            JsonValue::String(cleanup_policy.to_string()),
        ),
        (
            "cleanup_class".to_string(),
            JsonValue::String(cleanup_class),
        ),
        (
            "cleanup_candidate".to_string(),
            JsonValue::Bool(cleanup_candidate),
        ),
        (
            "cleanup_reason".to_string(),
            if cleanup_candidate {
                JsonValue::String(cleanup_reason.to_string())
            } else {
                JsonValue::Null
            },
        ),
        (
            "protected_reason".to_string(),
            protected_reason
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "last_activity_at".to_string(),
            JsonValue::String(updated_at),
        ),
        (
            "idle_for".to_string(),
            JsonValue::String(idle_for_label.to_string()),
        ),
        ("usage".to_string(), usage),
    ])
}
