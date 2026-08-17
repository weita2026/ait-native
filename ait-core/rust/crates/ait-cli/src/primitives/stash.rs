use super::*;
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;
use ait_core::snapshot_store::{
    set_snapshot_kind_with_snapshot_store as core_set_snapshot_kind_with_snapshot_store,
    snapshot_kind_with_snapshot_store as core_snapshot_kind_with_snapshot_store, SnapshotStore,
};
use ait_core::stash_store::{
    drop_stash_with_stash_store, list_stashes_with_stash_store, stash_by_id_with_stash_store,
    DroppedStashRecord, NewStashRecord, StashRecord, StashStore,
};

fn stash_record_json(stash: &StashRecord) -> JsonValue {
    json!({
        "stash_id": &stash.stash_id,
        "snapshot_id": &stash.snapshot_id,
        "source_line_name": &stash.source_line_name,
        "base_snapshot_id": &stash.base_snapshot_id,
        "message": &stash.message,
        "workspace_cleared": stash.workspace_cleared,
        "created_at": &stash.created_at,
        "snapshot_created_at": &stash.snapshot_created_at,
        "snapshot_kind": &stash.snapshot_kind,
        "parent_snapshot_id": &stash.parent_snapshot_id,
        "file_count": stash.file_count,
        "total_bytes": stash.total_bytes,
    })
}

fn dropped_stash_record_json(dropped: &DroppedStashRecord) -> JsonValue {
    let mut stash = stash_record_json(&dropped.stash);
    if let Some(obj) = stash.as_object_mut() {
        obj.insert("dropped".to_string(), JsonValue::Bool(true));
        obj.insert(
            "snapshot_deleted".to_string(),
            JsonValue::Bool(dropped.snapshot_deleted),
        );
    }
    stash
}

pub(super) fn stash_list_with_stash_store<S>(store: &S) -> Result<JsonValue, String>
where
    S: StashStore + ?Sized,
{
    let rows = list_stashes_with_stash_store(store)?
        .into_iter()
        .map(|stash| stash_record_json(&stash))
        .collect();
    Ok(JsonValue::Array(rows))
}

pub(super) fn stash_show_with_stash_store<S>(store: &S, stash_id: &str) -> Result<JsonValue, String>
where
    S: StashStore + ?Sized,
{
    stash_by_id_with_stash_store(store, stash_id)?
        .map(|stash| stash_record_json(&stash))
        .ok_or_else(|| format!("Unknown stash: {stash_id}"))
}

pub(super) fn drop_stash_record_with_stash_store<S>(
    store: &S,
    stash_id: &str,
) -> Result<JsonValue, String>
where
    S: StashStore + ?Sized,
{
    drop_stash_with_stash_store(store, stash_id)?
        .map(|dropped| dropped_stash_record_json(&dropped))
        .ok_or_else(|| format!("Unknown stash: {stash_id}"))
}

fn create_stash_record(
    repo: &RepoRuntime,
    snapshot_id: &str,
    source_line_name: &str,
    base_snapshot_id: Option<&str>,
    message: Option<&str>,
    workspace_cleared: bool,
) -> Result<JsonValue, String> {
    let workspace_root = repo.workspace_root();
    let snapshot_store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let stash_store = repo.stash_store()?;
    create_stash_record_with_stores(
        repo,
        &snapshot_store,
        &stash_store,
        snapshot_id,
        source_line_name,
        base_snapshot_id,
        message,
        workspace_cleared,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "stash record fields mirror the durable stash contract"
)]
fn create_stash_record_with_stores<S, T>(
    repo: &RepoRuntime,
    snapshot_store: &S,
    stash_store: &T,
    snapshot_id: &str,
    source_line_name: &str,
    base_snapshot_id: Option<&str>,
    message: Option<&str>,
    workspace_cleared: bool,
) -> Result<JsonValue, String>
where
    S: SnapshotStore + ?Sized,
    T: StashStore + ?Sized,
{
    let snapshot_kind = core_snapshot_kind_with_snapshot_store(snapshot_store, snapshot_id)?
        .ok_or_else(|| format!("Unknown snapshot: {snapshot_id}"))?;
    if snapshot_kind != "stash" {
        return Err(format!("Snapshot {snapshot_id} is not a stash snapshot."));
    }
    let stash_id = generate_workflow_id(repo, "STH")?;
    let created_at = system_event_timestamp();
    let stash = stash_store.create_stash(NewStashRecord {
        stash_id: &stash_id,
        snapshot_id,
        source_line_name,
        base_snapshot_id,
        message,
        workspace_cleared,
        created_at: &created_at,
    })?;
    Ok(stash_record_json(&stash))
}

fn mark_stash_snapshot_kind_with_snapshot_store<S>(
    snapshot_store: &S,
    snapshot_id: &str,
) -> Result<(), String>
where
    S: SnapshotStore + ?Sized,
{
    core_set_snapshot_kind_with_snapshot_store(snapshot_store, snapshot_id, "stash")?;
    Ok(())
}

pub fn stash_save(
    repo: &RepoRuntime,
    message: Option<&str>,
    keep_workspace: bool,
) -> Result<JsonValue, String> {
    guard_repo_root_pinned_bound_worktree(repo, None, "ait stash save")?;
    guard_current_worktree_task_bound_authoring(repo, "stash save")?;
    guard_no_active_line_merge(repo, None, "saving a stash")?;
    let line_name = repo.current_line_name()?;
    let base_snapshot_id = local_line_head_snapshot_id(repo, &line_name)?;
    let dirty = workspace_delta_payload(repo, base_snapshot_id.as_deref(), None)?;
    if dirty
        .get("clean")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return Err(
            "Workspace is already clean; stash save requires local changes to park.".to_string(),
        );
    }
    let workspace_root = repo.workspace_root();
    let snapshot_store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let snapshot = snapshot_store.create_snapshot(
        &repo.repo_name(),
        &line_name,
        message,
        repo.is_worktree(),
    )?;
    let snapshot_id = required_string_field(&snapshot, "snapshot_id")?;
    set_local_line_head(repo, &line_name, base_snapshot_id.as_deref())?;
    mark_stash_snapshot_kind_with_snapshot_store(&snapshot_store, &snapshot_id)?;
    let mut stash = create_stash_record(
        repo,
        &snapshot_id,
        &line_name,
        base_snapshot_id.as_deref(),
        message,
        !keep_workspace,
    )?;
    if keep_workspace {
        repo.set_worktree_materialized_snapshot(Some(&snapshot_id))?;
    } else {
        restore_workspace_all(repo, base_snapshot_id.as_deref(), None, true, false)?;
        repo.set_worktree_materialized_snapshot(base_snapshot_id.as_deref())?;
    }
    let line_head_after = local_line_head_snapshot_id(repo, &line_name)?;
    let obj = stash
        .as_object_mut()
        .ok_or_else(|| "Native stash save produced a non-object payload.".to_string())?;
    obj.insert("current_line".to_string(), JsonValue::String(line_name));
    obj.insert(
        "line_head_snapshot_id_before".to_string(),
        base_snapshot_id
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    obj.insert(
        "line_head_snapshot_id_after".to_string(),
        line_head_after
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    obj.insert(
        "workspace_cleared".to_string(),
        JsonValue::Bool(!keep_workspace),
    );
    obj.insert("dirty_workspace".to_string(), dirty);
    Ok(stash)
}

pub fn stash_list(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let store = repo.stash_store()?;
    stash_list_with_stash_store(&store)
}

pub fn stash_show(repo: &RepoRuntime, stash_id: &str) -> Result<JsonValue, String> {
    let store = repo.stash_store()?;
    stash_show_with_stash_store(&store, stash_id)
}

fn drop_stash_record(repo: &RepoRuntime, stash_id: &str) -> Result<JsonValue, String> {
    let store = repo.stash_store()?;
    drop_stash_record_with_stash_store(&store, stash_id)
}

fn worktree_materialized_snapshot_id(repo: &RepoRuntime) -> Option<String> {
    let path = repo.worktree_config_path.as_ref()?;
    read_json_value(path)
        .get("materialized_snapshot_id")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
}

pub fn stash_apply(repo: &RepoRuntime, stash_id: &str, force: bool) -> Result<JsonValue, String> {
    stash_apply_inner(repo, stash_id, force, false)
}

pub fn stash_pop(repo: &RepoRuntime, stash_id: &str, force: bool) -> Result<JsonValue, String> {
    stash_apply_inner(repo, stash_id, force, true)
}

pub(super) fn guard_stash_source_line(
    stash_id: &str,
    source_line_name: &str,
    current_line_name: &str,
    operation: &str,
) -> Result<(), String> {
    if source_line_name == current_line_name {
        return Ok(());
    }
    Err(format!(
        "Cannot {operation} stash {stash_id}: it was saved from Line {source_line_name}, but the current Line is {current_line_name}. Switch to Line {source_line_name} before restoring it. --force only overwrites unsaved managed-workspace changes and cannot bypass this Line check."
    ))
}

fn stash_apply_inner(
    repo: &RepoRuntime,
    stash_id: &str,
    force: bool,
    drop: bool,
) -> Result<JsonValue, String> {
    guard_repo_root_pinned_bound_worktree(repo, None, "ait stash apply")?;
    guard_current_worktree_task_bound_authoring(repo, "stash apply")?;
    guard_no_active_line_merge(repo, None, "applying a stash")?;
    let stash = stash_show(repo, stash_id)?;
    let snapshot_id = required_string_field(&stash, "snapshot_id")?;
    let source_line_name = required_string_field(&stash, "source_line_name")?;
    let line_name = repo.current_line_name()?;
    let operation = if drop { "pop" } else { "apply" };
    guard_stash_source_line(stash_id, &source_line_name, &line_name, operation)?;
    let line_head_before = local_line_head_snapshot_id(repo, &line_name)?;
    restore_workspace_all(
        repo,
        Some(&snapshot_id),
        line_head_before.as_deref(),
        force,
        false,
    )?;
    repo.set_worktree_materialized_snapshot(Some(&snapshot_id))?;
    let mut payload = stash;
    let obj = payload
        .as_object_mut()
        .ok_or_else(|| "Native stash apply produced a non-object payload.".to_string())?;
    obj.insert(
        "current_line".to_string(),
        JsonValue::String(line_name.clone()),
    );
    obj.insert(
        "line_head_snapshot_id_before".to_string(),
        line_head_before
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    obj.insert(
        "line_head_snapshot_id_after".to_string(),
        local_line_head_snapshot_id(repo, &line_name)?
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    obj.insert("applied".to_string(), JsonValue::Bool(true));
    obj.insert("dropped".to_string(), JsonValue::Bool(false));
    obj.insert(
        "workspace_restored_from_stash".to_string(),
        JsonValue::Bool(true),
    );
    if !drop {
        return Ok(payload);
    }
    let dropped = drop_stash_record(repo, stash_id)?;
    if dropped
        .get("snapshot_deleted")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        repo.set_worktree_materialized_snapshot(None)?;
    }
    let obj = payload
        .as_object_mut()
        .ok_or_else(|| "Native stash pop produced a non-object payload.".to_string())?;
    obj.insert("dropped".to_string(), JsonValue::Bool(true));
    obj.insert(
        "snapshot_deleted".to_string(),
        dropped
            .get("snapshot_deleted")
            .cloned()
            .unwrap_or(JsonValue::Bool(false)),
    );
    Ok(payload)
}

pub fn stash_drop(repo: &RepoRuntime, stash_id: &str) -> Result<JsonValue, String> {
    let dropped = drop_stash_record(repo, stash_id)?;
    let snapshot_id = required_string_field(&dropped, "snapshot_id")?;
    if dropped
        .get("snapshot_deleted")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
        && worktree_materialized_snapshot_id(repo).as_deref() == Some(snapshot_id.as_str())
    {
        repo.set_worktree_materialized_snapshot(None)?;
    }
    Ok(dropped)
}

#[cfg(test)]
mod tests;
