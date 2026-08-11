use super::*;
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;
use ait_core::local_snapshot::{LocalSnapshotBlobReadStore, LocalSnapshotReadStore};
use ait_core::snapshot_store::{
    snapshot_exists_with_snapshot_store as core_snapshot_exists_with_snapshot_store, SnapshotStore,
};

pub const DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_MAX_DEPTH: usize = 10_000;
pub const DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_LIMIT: usize = 10_000;
const SNAPSHOT_ANCESTRY_CONTRACT: &str = "snapshot-ancestry/v1";
const SNAPSHOT_IS_ANCESTOR_CONTRACT: &str = "snapshot-is-ancestor/v1";
const SNAPSHOT_MERGE_BASE_CONTRACT: &str = "snapshot-merge-base/v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SnapshotAncestryDirection {
    Ancestors,
    Descendants,
}

impl SnapshotAncestryDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Ancestors => "ancestors",
            Self::Descendants => "descendants",
        }
    }
}

pub fn snapshot_list(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let _range = perfetto_range!("ait.cli.snapshot_list.storage");
    let workspace_root = repo.workspace_root();
    let store = {
        let _range = perfetto_range!("ait.cli.snapshot_list.store_select");
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?
    };
    let _range = perfetto_range!("ait.cli.snapshot_list.binary_records_and_projection");
    store.list_snapshots()
}

pub fn snapshot_show(repo: &RepoRuntime, snapshot_id: &str) -> Result<JsonValue, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    store.get_snapshot(snapshot_id)
}

pub fn snapshot_chain(repo: &RepoRuntime, snapshot_id: &str) -> Result<JsonValue, String> {
    let store = snapshot_store(repo)?;
    snapshot_chain_with_snapshot_store(&store, snapshot_id)
}

pub(super) fn snapshot_chain_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<JsonValue, String>
where
    S: SnapshotStore + ?Sized,
{
    Ok(JsonValue::Array(
        snapshot_first_parent_chain(store, snapshot_id, None, SnapshotDagLimits::default())?
            .into_iter()
            .map(JsonValue::String)
            .collect(),
    ))
}

pub fn snapshot_exists(repo: &RepoRuntime, snapshot_id: &str) -> Result<bool, String> {
    let store = snapshot_store(repo)?;
    snapshot_exists_with_snapshot_store(&store, snapshot_id)
}

pub(super) fn snapshot_exists_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
) -> Result<bool, String>
where
    S: SnapshotStore + ?Sized,
{
    core_snapshot_exists_with_snapshot_store(store, snapshot_id)
}

pub fn snapshot_ancestry(
    repo: &RepoRuntime,
    snapshot_id: &str,
    direction: SnapshotAncestryDirection,
    first_parent: bool,
    max_depth: usize,
    limit: usize,
) -> Result<JsonValue, String> {
    let store = snapshot_store(repo)?;
    snapshot_ancestry_with_snapshot_store(
        &store,
        snapshot_id,
        direction,
        first_parent,
        max_depth,
        limit,
    )
}

pub(super) fn snapshot_ancestry_with_snapshot_store<S>(
    store: &S,
    snapshot_id: &str,
    direction: SnapshotAncestryDirection,
    first_parent: bool,
    max_depth: usize,
    limit: usize,
) -> Result<JsonValue, String>
where
    S: SnapshotStore + ?Sized,
{
    let max_results = limit
        .checked_add(1)
        .ok_or_else(|| "Snapshot ancestry limit is too large.".to_string())?;
    if limit == 0 {
        return Err("Snapshot ancestry limit must be greater than zero.".to_string());
    }
    let parent_mode = if first_parent {
        SnapshotParentMode::FirstParent
    } else {
        SnapshotParentMode::AllParents
    };
    let limits = SnapshotDagLimits {
        max_depth,
        max_results,
        limit_mode: SnapshotDagLimitMode::Truncate,
        ..SnapshotDagLimits::default()
    };
    let traversal = match direction {
        SnapshotAncestryDirection::Ancestors => snapshot_ancestor_closure(
            store,
            &[snapshot_id.to_string()],
            &BTreeSet::new(),
            parent_mode,
            limits,
        )?,
        SnapshotAncestryDirection::Descendants => {
            snapshot_descendant_closure(store, &[snapshot_id.to_string()], parent_mode, limits)?
        }
    };
    let snapshots = traversal
        .topological_snapshot_ids
        .iter()
        .filter(|candidate| candidate.as_str() != snapshot_id)
        .map(|candidate| {
            json!({
                "snapshot_id": candidate,
                "depth": traversal.depth_by_snapshot_id.get(candidate).copied(),
                "parent_snapshot_ids": traversal.parent_snapshot_ids.get(candidate).cloned().unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "contract": SNAPSHOT_ANCESTRY_CONTRACT,
        "query_snapshot_id": snapshot_id,
        "direction": direction.as_str(),
        "parent_mode": if first_parent { "first_parent" } else { "all_parents" },
        "order": "topological",
        "includes_query_snapshot": false,
        "max_depth": max_depth,
        "limit": limit,
        "truncated": traversal.truncated,
        "result_count": snapshots.len(),
        "snapshots": snapshots,
    }))
}

pub fn snapshot_is_ancestor_query(
    repo: &RepoRuntime,
    older_snapshot_id: &str,
    newer_snapshot_id: &str,
) -> Result<(JsonValue, bool), String> {
    let store = snapshot_store(repo)?;
    snapshot_is_ancestor_query_with_snapshot_store(&store, older_snapshot_id, newer_snapshot_id)
}

pub(super) fn snapshot_is_ancestor_query_with_snapshot_store<S>(
    store: &S,
    older_snapshot_id: &str,
    newer_snapshot_id: &str,
) -> Result<(JsonValue, bool), String>
where
    S: SnapshotStore + ?Sized,
{
    require_known_snapshot(store, older_snapshot_id)?;
    require_known_snapshot(store, newer_snapshot_id)?;
    let distance = snapshot_is_ancestor(
        store,
        older_snapshot_id,
        newer_snapshot_id,
        SnapshotDagLimits::default(),
    )?;
    let is_ancestor = distance.is_some();
    Ok((
        json!({
            "contract": SNAPSHOT_IS_ANCESTOR_CONTRACT,
            "older_snapshot_id": older_snapshot_id,
            "newer_snapshot_id": newer_snapshot_id,
            "is_ancestor": is_ancestor,
            "distance": distance,
        }),
        is_ancestor,
    ))
}

pub fn snapshot_merge_base_query(
    repo: &RepoRuntime,
    left_snapshot_id: &str,
    right_snapshot_id: &str,
    all: bool,
) -> Result<(JsonValue, bool), String> {
    let store = snapshot_store(repo)?;
    snapshot_merge_base_query_with_snapshot_store(&store, left_snapshot_id, right_snapshot_id, all)
}

pub(super) fn snapshot_merge_base_query_with_snapshot_store<S>(
    store: &S,
    left_snapshot_id: &str,
    right_snapshot_id: &str,
    all: bool,
) -> Result<(JsonValue, bool), String>
where
    S: SnapshotStore + ?Sized,
{
    require_known_snapshot(store, left_snapshot_id)?;
    require_known_snapshot(store, right_snapshot_id)?;
    let all_merge_base_snapshot_ids = snapshot_merge_bases(
        store,
        left_snapshot_id,
        right_snapshot_id,
        SnapshotDagLimits::default(),
    )?;
    let found = !all_merge_base_snapshot_ids.is_empty();
    let merge_base_snapshot_ids = if all {
        all_merge_base_snapshot_ids.clone()
    } else {
        all_merge_base_snapshot_ids
            .first()
            .cloned()
            .into_iter()
            .collect()
    };
    Ok((
        json!({
            "contract": SNAPSHOT_MERGE_BASE_CONTRACT,
            "left_snapshot_id": left_snapshot_id,
            "right_snapshot_id": right_snapshot_id,
            "all": all,
            "merge_base_snapshot_id": merge_base_snapshot_ids.first(),
            "merge_base_snapshot_ids": merge_base_snapshot_ids,
            "available_merge_base_count": all_merge_base_snapshot_ids.len(),
            "ambiguous": all_merge_base_snapshot_ids.len() > 1,
        }),
        found,
    ))
}

fn require_known_snapshot<S>(store: &S, snapshot_id: &str) -> Result<(), String>
where
    S: SnapshotStore + ?Sized,
{
    if store.snapshot_parent_link(snapshot_id)?.is_some() {
        Ok(())
    } else {
        Err(format!("Unknown snapshot: {snapshot_id}"))
    }
}

fn snapshot_store(repo: &RepoRuntime) -> Result<impl SnapshotStore, String> {
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)
}

pub fn blob_read_bytes(repo: &RepoRuntime, blob_id: &str) -> Result<Vec<u8>, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    store.read_blob_bytes(blob_id)
}

pub fn blob_ensure_bytes(
    repo: &RepoRuntime,
    data: &[u8],
    path_hint: Option<&str>,
) -> Result<String, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    store.ensure_blob_bytes(data, path_hint)
}

pub fn snapshot_diff(
    repo: &RepoRuntime,
    old_snapshot_id: &str,
    new_snapshot_id: &str,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let snapshot_reader = LocalRepoSnapshotReader {
        snapshot_store: &store,
        tree_read_store: &store,
        tree_pack_store: None,
    };
    let blob_reader = LocalRepoBlobReader { blob_store: &store };
    snapshot_diff_from_readers(
        &snapshot_reader,
        Some(&blob_reader),
        Some(old_snapshot_id),
        Some(new_snapshot_id),
        include_text,
        max_bytes,
    )
}

pub fn snapshot_revert(
    repo: &RepoRuntime,
    snapshot_id: &str,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    guard_no_active_line_merge(repo, None, "reverting Snapshot content")?;
    let snapshot = snapshot_show(repo, snapshot_id)?;
    let snapshot_kind =
        string_field(&snapshot, "snapshot_kind").unwrap_or_else(|| "line".to_string());
    if snapshot_kind != "line" {
        return Err(format!(
            "Snapshot {snapshot_id} is `{snapshot_kind}`. Use the matching first-class surface instead of `snapshot revert`."
        ));
    }
    let parent_snapshot_id = string_field(&snapshot, "parent_snapshot_id");
    let (current_line_name, current_head_snapshot_id) =
        require_current_line_head_snapshot(repo, snapshot_id, "snapshot revert")?;
    let result = apply_workspace_revert_range(
        repo,
        parent_snapshot_id.as_deref(),
        &current_head_snapshot_id,
        force,
        dry_run,
    )?;
    let mut payload = result
        .as_object()
        .cloned()
        .ok_or_else(|| "snapshot revert payload must be an object".to_string())?;
    payload.insert("repo_name".to_string(), JsonValue::String(repo.repo_name()));
    payload.insert(
        "snapshot_id".to_string(),
        JsonValue::String(snapshot_id.to_string()),
    );
    payload.insert(
        "parent_snapshot_id".to_string(),
        parent_snapshot_id
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
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

pub fn snapshot_replay(
    repo: &RepoRuntime,
    snapshot_id: &str,
    onto_line: &str,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    guard_no_active_line_merge(repo, None, "replaying Snapshot content")?;
    let snapshot = snapshot_show(repo, snapshot_id)?;
    let snapshot_kind =
        string_field(&snapshot, "snapshot_kind").unwrap_or_else(|| "line".to_string());
    if snapshot_kind != "line" {
        return Err(format!(
            "Snapshot {snapshot_id} is `{snapshot_kind}`. Use the matching first-class surface instead of `snapshot replay`."
        ));
    }
    let parent_snapshot_id = string_field(&snapshot, "parent_snapshot_id");
    let parent_snapshot_id = parent_snapshot_id.ok_or_else(|| {
        format!(
            "Snapshot {snapshot_id} has no parent snapshot, so `snapshot replay` cannot compute a replay delta."
        )
    })?;
    let (current_line_name, current_head_snapshot_id) =
        require_current_line_target(repo, onto_line, "snapshot replay")?;
    let result = apply_workspace_replay_range(
        repo,
        &parent_snapshot_id,
        snapshot_id,
        current_head_snapshot_id.as_deref(),
        force,
        dry_run,
    )?;
    let mut payload = result
        .as_object()
        .cloned()
        .ok_or_else(|| "snapshot replay payload must be an object".to_string())?;
    payload.insert("repo_name".to_string(), JsonValue::String(repo.repo_name()));
    payload.insert(
        "snapshot_id".to_string(),
        JsonValue::String(snapshot_id.to_string()),
    );
    payload.insert(
        "parent_snapshot_id".to_string(),
        JsonValue::String(parent_snapshot_id.clone()),
    );
    payload.insert(
        "source_line".to_string(),
        string_field(&snapshot, "line_name")
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
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
