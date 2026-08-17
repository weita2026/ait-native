use super::super::*;
use super::backend::{HttpRemoteSyncBackend, RemoteSyncBackend};
#[cfg(test)]
use super::local_store::RemoteSyncLocalInventorySource;
use super::local_store::{
    RemoteSyncLocalStoreContext, RemoteSyncZstdImportSource, RemoteSyncZstdLocalPlanSource,
    ZstdBulkLocalPlan, ZstdImportApplyResult, ZstdImportHistoryMode, ZstdImportPackStageResult,
};
use ait_core::remote_sync_local_store::RemoteSyncLocalSnapshotSource;
use ait_core::server_operational::RepositoryIndex;

fn verify_remote_pull_line(
    remote_line: &JsonValue,
    repo_name: &str,
    line_name: &str,
) -> Result<(), String> {
    let remote_repo_name = string_field(remote_line, "repo_name")
        .ok_or_else(|| "Remote pull returned a line without repo_name.".to_string())?;
    if remote_repo_name != repo_name {
        return Err(format!(
            "Remote pull returned unexpected repository {remote_repo_name:?} (expected {repo_name:?})"
        ));
    }
    let remote_line_name = string_field(remote_line, "line_name")
        .ok_or_else(|| "Remote pull returned a line without line_name.".to_string())?;
    if remote_line_name != line_name {
        return Err(format!(
            "Remote pull returned unexpected line {remote_line_name:?} (expected {line_name:?})"
        ));
    }
    Ok(())
}

fn verify_remote_line_identity_when_present(
    remote_line: &JsonValue,
    repo_name: &str,
    line_name: &str,
) -> Result<(), String> {
    if let Some(remote_repo_name) = string_field(remote_line, "repo_name") {
        if remote_repo_name != repo_name {
            return Err(format!(
                "Remote pull returned unexpected repository {remote_repo_name:?} (expected {repo_name:?})"
            ));
        }
    }
    if let Some(remote_line_name) = string_field(remote_line, "line_name") {
        if remote_line_name != line_name {
            return Err(format!(
                "Remote pull returned unexpected line {remote_line_name:?} (expected {line_name:?})"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
fn verify_remote_snapshot_metadata(
    snapshot: &JsonValue,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<(), String> {
    let remote_snapshot_id = string_field(snapshot, "snapshot_id")
        .ok_or_else(|| "Remote snapshot response is missing snapshot_id.".to_string())?;
    if remote_snapshot_id != snapshot_id {
        return Err(format!(
            "Remote snapshot verification returned unexpected snapshot {remote_snapshot_id:?} (expected {snapshot_id:?})"
        ));
    }
    let remote_repo_name = string_field(snapshot, "repo_name")
        .ok_or_else(|| "Remote snapshot response is missing repo_name.".to_string())?;
    if remote_repo_name != repo_name {
        return Err(format!(
            "Remote snapshot verification returned unexpected repository {remote_repo_name:?} (expected {repo_name:?})"
        ));
    }
    Ok(())
}

#[derive(Debug, Default)]
struct ZstdImportChainResult {
    imported_snapshots: i64,
    imported_snapshot_ids: Vec<String>,
    downloaded_object_packs: i64,
    reused_object_packs: i64,
    downloaded_tree_packs: i64,
    reused_tree_packs: i64,
    upserted_blob_locators: i64,
    upserted_tree_locators: i64,
    manifest_ancestry_ms: f64,
    pack_download_ms: f64,
    metadata_import_ms: f64,
    total_ms: f64,
    remote_round_trips: i64,
    transferred_pack_bytes: u64,
    pack_parallelism: usize,
}

fn remote_sync_local_line_record(
    repo: &RepoRuntime,
    line_name: &str,
) -> Result<Option<ait_core::line_store::LineRecord>, String> {
    let source = selected_remote_sync_local_store(repo)?;
    source.line_by_name(line_name)
}

fn local_line_state(repo: &RepoRuntime, line_name: &str) -> Result<(bool, Option<String>), String> {
    match remote_sync_local_line_record(repo, line_name)? {
        Some(row) => Ok((true, row.head_snapshot_id)),
        None => Ok((false, None)),
    }
}

fn remote_sync_local_line_head_snapshot_id(
    repo: &RepoRuntime,
    line_name: &str,
) -> Result<Option<String>, String> {
    Ok(remote_sync_local_line_record(repo, line_name)?.and_then(|line| line.head_snapshot_id))
}

pub(crate) fn remote_sync_snapshot_content_complete_for_repo(
    repo: &RepoRuntime,
    snapshot_id: &str,
) -> Result<bool, String> {
    let source = selected_remote_sync_local_store(repo)?;
    let ctx = remote_sync_local_store_context(repo);
    source.snapshot_content_complete(&ctx, snapshot_id)
}

#[cfg(test)]
pub(in crate::primitives) fn require_snapshot_dag_upload_capability_with_source<S>(
    source: &S,
    repo: &RepoRuntime,
    snapshot_ids: &[String],
    capabilities: &RemoteSyncCapabilities,
) -> Result<(), String>
where
    S: RemoteSyncLocalSnapshotSource + ?Sized,
{
    let selected = snapshot_ids.iter().collect::<BTreeSet<_>>();
    let ctx = remote_sync_local_store_context(repo);
    let affected = source
        .snapshot_parent_rows(&ctx)?
        .into_iter()
        .filter(|row| selected.contains(&row.snapshot_id) && row.parent_snapshot_ids.len() > 1)
        .map(|row| row.snapshot_id)
        .collect::<Vec<_>>();
    require_snapshot_dag_remote_capability(capabilities, &affected)
}

fn remote_sync_collect_snapshot_dag(
    repo: &RepoRuntime,
    snapshot_id: &str,
) -> Result<SnapshotDagTraversal, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    snapshot_ancestor_closure(
        &store,
        &[snapshot_id.to_string()],
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        SnapshotDagLimits::default(),
    )
}

struct RemoteSnapshotSyncPlan {
    traversal: SnapshotDagTraversal,
    window: ait_core::local_snapshot::SnapshotSyncWindow,
}

impl RemoteSnapshotSyncPlan {
    fn multi_parent_snapshot_ids(&self) -> Vec<String> {
        let selected = self.window.snapshot_ids.iter().collect::<BTreeSet<_>>();
        self.traversal
            .parent_snapshot_ids
            .iter()
            .filter(|(snapshot_id, parents)| selected.contains(snapshot_id) && parents.len() > 1)
            .map(|(snapshot_id, _)| snapshot_id.clone())
            .collect()
    }

    fn contains_snapshot(&self, snapshot_id: &str) -> bool {
        self.traversal.contains(snapshot_id)
    }

    fn snapshot_ids_bounded_by(
        &self,
        boundary_snapshot_id: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let Some(boundary_snapshot_id) = normalized_text(boundary_snapshot_id) else {
            return Ok(deduplicate_snapshot_ids(&self.window.snapshot_ids));
        };
        if !self.contains_snapshot(&boundary_snapshot_id) {
            return Ok(deduplicate_snapshot_ids(&self.window.snapshot_ids));
        }
        let boundary_ancestry = snapshot_ancestor_closure_from_parent_map(
            &self.traversal.parent_snapshot_ids,
            std::slice::from_ref(&boundary_snapshot_id),
            &BTreeSet::new(),
            SnapshotParentMode::AllParents,
            SnapshotDagLimits::default(),
        )?
        .topological_snapshot_ids
        .into_iter()
        .collect::<BTreeSet<_>>();
        Ok(self
            .window
            .snapshot_ids
            .iter()
            .filter(|snapshot_id| !boundary_ancestry.contains(*snapshot_id))
            .cloned()
            .collect())
    }
}

fn deduplicate_snapshot_ids(snapshot_ids: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    snapshot_ids
        .iter()
        .filter(|snapshot_id| seen.insert((*snapshot_id).clone()))
        .cloned()
        .collect()
}

fn remote_sync_snapshot_sync_plan(
    repo: &RepoRuntime,
    snapshot_id: &str,
    remote_head_snapshot_id: Option<&str>,
) -> Result<RemoteSnapshotSyncPlan, String> {
    let traversal = remote_sync_collect_snapshot_dag(repo, snapshot_id)?;
    let chain = traversal.topological_snapshot_ids.clone();
    let resolved_remote_head_snapshot_id = remote_head_snapshot_id
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let Some(remote_head_snapshot_id) = resolved_remote_head_snapshot_id else {
        return Ok(RemoteSnapshotSyncPlan {
            traversal,
            window: ait_core::local_snapshot::SnapshotSyncWindow {
                snapshot_ids: chain,
                sync_scope: "full_chain",
                sync_reason: "no_remote_head",
                remote_head_snapshot_id: None,
                bounded_by_snapshot_id: None,
            },
        });
    };
    if !traversal.contains(remote_head_snapshot_id) {
        return Ok(RemoteSnapshotSyncPlan {
            traversal,
            window: ait_core::local_snapshot::SnapshotSyncWindow {
                snapshot_ids: chain,
                sync_scope: "full_chain",
                sync_reason: "remote_head_not_in_local_ancestry",
                remote_head_snapshot_id: Some(remote_head_snapshot_id.to_string()),
                bounded_by_snapshot_id: None,
            },
        });
    }
    let remote_ancestry = snapshot_ancestor_closure_from_parent_map(
        &traversal.parent_snapshot_ids,
        &[remote_head_snapshot_id.to_string()],
        &BTreeSet::new(),
        SnapshotParentMode::AllParents,
        SnapshotDagLimits::default(),
    )?
    .topological_snapshot_ids
    .into_iter()
    .collect::<BTreeSet<_>>();
    let snapshot_ids = chain
        .into_iter()
        .filter(|snapshot| !remote_ancestry.contains(snapshot))
        .collect::<Vec<_>>();
    Ok(RemoteSnapshotSyncPlan {
        traversal,
        window: ait_core::local_snapshot::SnapshotSyncWindow {
            snapshot_ids: snapshot_ids.clone(),
            sync_scope: "bounded_suffix",
            sync_reason: if snapshot_ids.is_empty() {
                "remote_head_matches_local_head"
            } else {
                "remote_head_is_local_ancestor"
            },
            remote_head_snapshot_id: Some(remote_head_snapshot_id.to_string()),
            bounded_by_snapshot_id: Some(remote_head_snapshot_id.to_string()),
        },
    })
}

fn remote_sync_local_store_context(repo: &RepoRuntime) -> RemoteSyncLocalStoreContext {
    RemoteSyncLocalStoreContext::new(repo.workspace_root())
}

#[cfg(test)]
pub(in crate::primitives) fn local_repo_snapshot_ids_topological_with_source<S>(
    source: &S,
    repo: &RepoRuntime,
) -> Result<Vec<String>, String>
where
    S: RemoteSyncLocalSnapshotSource + ?Sized,
{
    let ctx = remote_sync_local_store_context(repo);
    let rows = source.snapshot_parent_rows(&ctx)?;
    let parent_map = rows
        .into_iter()
        .map(|row| (row.snapshot_id, row.parent_snapshot_ids))
        .collect::<BTreeMap<_, _>>();
    topological_snapshot_order(&parent_map, &BTreeSet::new())
}

#[cfg(test)]
pub(in crate::primitives) fn local_remote_sync_inventory_for_snapshots(
    repo: &RepoRuntime,
    snapshot_ids: &[String],
) -> Result<RemoteSyncSnapshotInventory, String> {
    if snapshot_ids.is_empty() {
        return Ok(RemoteSyncSnapshotInventory::empty());
    }
    let source = selected_remote_sync_local_store(repo)?;
    local_remote_sync_inventory_for_snapshots_with_source(&source, repo, snapshot_ids)
}

#[cfg(test)]
pub(in crate::primitives) fn local_remote_sync_inventory_for_snapshots_with_source<S>(
    source: &S,
    repo: &RepoRuntime,
    snapshot_ids: &[String],
) -> Result<RemoteSyncSnapshotInventory, String>
where
    S: RemoteSyncLocalInventorySource + ?Sized,
{
    if snapshot_ids.is_empty() {
        return Ok(RemoteSyncSnapshotInventory::empty());
    }
    let ctx = remote_sync_local_store_context(repo);
    let metadata = source.snapshot_inventory_metadata(&ctx, snapshot_ids)?;
    let inventory = RemoteSyncSnapshotInventory::from_pack_formats(
        snapshot_ids.iter().cloned(),
        metadata.object_pack_formats,
        metadata.tree_pack_formats,
    );
    inventory.validate_formats()?;
    Ok(inventory)
}

fn require_zstd_bulk_upload_backend(
    snapshot_ids: &[String],
    present_set: &BTreeSet<String>,
    capabilities: &RemoteSyncCapabilities,
) -> Result<(RemoteSyncBackendNegotiation, RemoteSyncInventoryDiff), String> {
    if !capabilities.zstd_pack_bulk {
        return Err(format!(
            "Remote sync requires capability {REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK}."
        ));
    }
    Ok((
        RemoteSyncBackendNegotiation {
            backend: RemoteSyncBackendKind::ZstdPackBulk,
            reason: "zstd_only_policy_requires_zstd_pack_bulk",
            capabilities: capabilities.clone(),
        },
        RemoteSyncInventoryDiff::from_present_snapshot_ids(snapshot_ids, present_set),
    ))
}

#[derive(Debug)]
struct ZstdBulkUploadResult {
    commit_response: ZstdBulkCommitResponse,
    uploaded_snapshots: i64,
    skipped_snapshots: i64,
    uploaded_object_packs: i64,
    skipped_object_packs: i64,
    uploaded_tree_packs: i64,
    skipped_tree_packs: i64,
    remote_plan: ZstdBulkPlanResponse,
    phase_timings_ms: JsonValue,
    remote_round_trips: i64,
    transferred_pack_bytes: u64,
    pack_parallelism: usize,
}

const DEFAULT_REMOTE_SYNC_PACK_PARALLELISM: usize = 4;

fn remote_sync_pack_parallelism() -> usize {
    DEFAULT_REMOTE_SYNC_PACK_PARALLELISM
}

fn remote_sync_elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

pub(in crate::primitives) fn build_zstd_bulk_local_plan(
    repo: &RepoRuntime,
    snapshot_ids: &[String],
    present_set: &BTreeSet<String>,
) -> Result<ZstdBulkLocalPlan, String> {
    let source = selected_remote_sync_local_store(repo)?;
    build_zstd_bulk_local_plan_with_source(&source, repo, snapshot_ids, present_set)
}

pub(in crate::primitives) fn build_zstd_bulk_local_plan_with_source<S>(
    source: &S,
    repo: &RepoRuntime,
    snapshot_ids: &[String],
    present_set: &BTreeSet<String>,
) -> Result<ZstdBulkLocalPlan, String>
where
    S: RemoteSyncZstdLocalPlanSource + ?Sized,
{
    let ctx = remote_sync_local_store_context(repo);
    source.zstd_bulk_local_plan(&ctx, snapshot_ids, present_set)
}

fn zstd_bulk_plan_request(
    snapshot_ids: &[String],
    local_plan: &ZstdBulkLocalPlan,
    ordered_tree_packs: Vec<JsonValue>,
) -> Result<ZstdBulkPlanRequest, String> {
    validate_zstd_object_pack_locator_coverage(local_plan)?;
    ZstdBulkPlanRequest::from_json_rows(
        snapshot_ids.to_vec(),
        ordered_object_pack_metadata(local_plan)?,
        ordered_tree_packs,
    )
}

fn zstd_bulk_commit_request(
    local_plan: &ZstdBulkLocalPlan,
    ordered_tree_packs: Vec<JsonValue>,
    missing_snapshot_ids: &[String],
    line_name: Option<&str>,
    head_snapshot_id: Option<&str>,
    expected_head_snapshot_id: Option<&str>,
) -> Result<ZstdBulkCommitRequest, String> {
    validate_zstd_object_pack_locator_coverage(local_plan)?;
    let missing = missing_snapshot_ids
        .iter()
        .filter_map(|snapshot_id| local_plan.snapshots.get(snapshot_id).cloned())
        .collect::<Vec<_>>();
    let line_update = line_name.map(|line_name| ZstdBulkLineUpdate {
        line_name: line_name.to_string(),
        head_snapshot_id: head_snapshot_id.map(str::to_string),
        expected_head_snapshot_id: expected_head_snapshot_id.map(str::to_string),
    });
    ZstdBulkCommitRequest::from_json_rows(
        Some("ait.remote_sync.zstd_bulk.commit.v1".to_string()),
        None,
        ordered_object_pack_metadata(local_plan)?,
        ordered_tree_packs,
        local_plan
            .blob_locators
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        local_plan
            .tree_locators
            .values()
            .cloned()
            .collect::<Vec<_>>(),
        missing,
        line_update,
    )
}

pub(in crate::primitives) fn validate_zstd_object_pack_locator_coverage(
    local_plan: &ZstdBulkLocalPlan,
) -> Result<(), String> {
    for (pack_id, pack) in &local_plan.object_packs {
        let entries = pack
            .metadata
            .get("pack_index")
            .and_then(|index| index.get("entries"))
            .and_then(JsonValue::as_array)
            .ok_or_else(|| {
                format!("Object pack {pack_id} metadata is missing pack_index.entries.")
            })?;
        let mut expected_blob_ids = BTreeSet::new();
        for entry in entries {
            let blob_id = string_field(entry, "blob_id").ok_or_else(|| {
                format!("Object pack {pack_id} index contains an entry without blob_id.")
            })?;
            if !expected_blob_ids.insert(blob_id.clone()) {
                return Err(format!(
                    "Object pack {pack_id} index contains duplicate blob entry {blob_id}."
                ));
            }
        }
        let selected_blob_ids = local_plan
            .blob_locators
            .iter()
            .filter_map(|(blob_id, locator)| {
                (string_field(locator, "pack_id").as_deref() == Some(pack_id.as_str()))
                    .then_some(blob_id.clone())
            })
            .collect::<BTreeSet<_>>();
        let unexpected = selected_blob_ids
            .difference(&expected_blob_ids)
            .cloned()
            .collect::<Vec<_>>();
        if !unexpected.is_empty() {
            return Err(format!(
                "Object pack {pack_id} has {} index entries but {} selected blob locators name members absent from the physical pack: {:?}.",
                expected_blob_ids.len(),
                selected_blob_ids.len(),
                unexpected,
            ));
        }
    }
    Ok(())
}

pub(in crate::primitives) fn ordered_object_pack_metadata(
    local_plan: &ZstdBulkLocalPlan,
) -> Result<Vec<JsonValue>, String> {
    let mut blob_pack_by_id = BTreeMap::new();
    for (blob_id, locator) in &local_plan.blob_locators {
        let pack_id = string_field(locator, "pack_id")
            .ok_or_else(|| format!("Blob locator {blob_id} is missing pack_id."))?;
        if !local_plan.object_packs.contains_key(&pack_id) {
            return Err(format!(
                "Blob locator {blob_id} references object pack {pack_id} outside the local upload plan."
            ));
        }
        blob_pack_by_id.insert(blob_id.clone(), pack_id);
    }

    let mut dependencies = local_plan
        .object_packs
        .keys()
        .map(|pack_id| (pack_id.clone(), BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    for (blob_id, locator) in &local_plan.blob_locators {
        let Some(base_blob_id) = string_field(locator, "pack_base_blob_id") else {
            continue;
        };
        let pack_id = blob_pack_by_id
            .get(blob_id)
            .expect("validated blob locator pack");
        let Some(base_pack_id) = blob_pack_by_id.get(&base_blob_id) else {
            continue;
        };
        if base_pack_id != pack_id {
            dependencies
                .get_mut(pack_id)
                .expect("all local packs have dependency rows")
                .insert(base_pack_id.clone());
        }
    }

    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::with_capacity(local_plan.object_packs.len());
    while ordered.len() < local_plan.object_packs.len() {
        let next = dependencies
            .iter()
            .find(|(pack_id, requires)| {
                !emitted.contains(*pack_id)
                    && requires.iter().all(|required| emitted.contains(required))
            })
            .map(|(pack_id, _)| pack_id.clone())
            .ok_or_else(|| "Object pack delta dependency graph contains a cycle.".to_string())?;
        ordered.push(
            local_plan
                .object_packs
                .get(&next)
                .expect("ordered object pack exists")
                .metadata
                .clone(),
        );
        emitted.insert(next);
    }
    Ok(ordered)
}

pub(in crate::primitives) fn ordered_tree_pack_metadata(
    local_plan: &ZstdBulkLocalPlan,
) -> Result<Vec<JsonValue>, String> {
    if local_plan.tree_pack_order.len() != local_plan.tree_packs.len() {
        return Err(format!(
            "Local tree pack order has {} entries for {} planned tree packs.",
            local_plan.tree_pack_order.len(),
            local_plan.tree_packs.len()
        ));
    }
    let mut emitted = BTreeSet::new();
    let mut ordered = Vec::with_capacity(local_plan.tree_packs.len());
    for pack_id in &local_plan.tree_pack_order {
        if !emitted.insert(pack_id.clone()) {
            return Err(format!(
                "Local tree pack order contains duplicate pack {pack_id}."
            ));
        }
        ordered.push(
            local_plan
                .tree_packs
                .get(pack_id)
                .ok_or_else(|| format!("Local tree pack order references unknown pack {pack_id}."))?
                .metadata
                .clone(),
        );
    }
    if let Some(missing) = local_plan
        .tree_packs
        .keys()
        .find(|pack_id| !emitted.contains(*pack_id))
    {
        return Err(format!(
            "Local tree pack order is missing planned pack {missing}."
        ));
    }
    Ok(ordered)
}

pub(in crate::primitives) fn zstd_bulk_commit_local_plan(
    repo: &RepoRuntime,
    planned: &ZstdBulkLocalPlan,
    snapshot_ids: &[String],
    missing_snapshot_ids: &[String],
    missing_object_pack_ids: &[String],
    missing_tree_pack_ids: &[String],
) -> Result<ZstdBulkLocalPlan, String> {
    let missing_snapshot_set = missing_snapshot_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut commit_plan = if missing_snapshot_set
        .iter()
        .all(|snapshot_id| planned.snapshots.contains_key(snapshot_id))
    {
        let _range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk.commit_plan.reuse");
        planned.clone()
    } else {
        // The existence query and zstd plan endpoint should agree. Preserve a
        // fail-closed recovery path for a remote that changes between those
        // reads, but do not rescan the complete local Binary DB closure on the
        // normal, internally consistent path.
        let _range =
            perfetto_range!("ait.workflow_ready.publish.zstd_bulk.commit_plan.rebuild_fallback");
        let effective_present_set = snapshot_ids
            .iter()
            .filter(|snapshot_id| !missing_snapshot_set.contains(*snapshot_id))
            .cloned()
            .collect::<BTreeSet<_>>();
        build_zstd_bulk_local_plan(repo, snapshot_ids, &effective_present_set)?
    };
    commit_plan
        .snapshot_order
        .retain(|snapshot_id| missing_snapshot_set.contains(snapshot_id));
    commit_plan
        .snapshots
        .retain(|snapshot_id, _| missing_snapshot_set.contains(snapshot_id));
    if let Some(missing) = missing_snapshot_set
        .iter()
        .find(|snapshot_id| !commit_plan.snapshots.contains_key(*snapshot_id))
    {
        return Err(format!(
            "Remote requested unknown snapshot {missing} in the zstd commit plan."
        ));
    }
    let missing_object_pack_set = missing_object_pack_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing_tree_pack_set = missing_tree_pack_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for pack_id in missing_object_pack_ids {
        if commit_plan.object_packs.contains_key(pack_id) {
            continue;
        }
        let pack = planned
            .object_packs
            .get(pack_id)
            .ok_or_else(|| format!("Remote requested unknown object pack {pack_id}."))?;
        commit_plan
            .object_packs
            .insert(pack_id.clone(), pack.clone());
    }
    for pack_id in missing_tree_pack_ids {
        if commit_plan.tree_packs.contains_key(pack_id) {
            continue;
        }
        let pack = planned
            .tree_packs
            .get(pack_id)
            .ok_or_else(|| format!("Remote requested unknown tree pack {pack_id}."))?;
        commit_plan.tree_packs.insert(pack_id.clone(), pack.clone());
    }

    for (blob_id, locator) in &planned.blob_locators {
        let pack_id = string_field(locator, "pack_id")
            .ok_or_else(|| format!("Blob locator {blob_id} is missing pack_id."))?;
        if missing_object_pack_set.contains(&pack_id) {
            commit_plan
                .blob_locators
                .insert(blob_id.clone(), locator.clone());
        }
    }
    for (tree_id, locator) in &planned.tree_locators {
        let pack_id = string_field(locator, "tree_pack_id")
            .ok_or_else(|| format!("Tree locator {tree_id} is missing tree_pack_id."))?;
        if missing_tree_pack_set.contains(&pack_id) {
            commit_plan
                .tree_locators
                .insert(tree_id.clone(), locator.clone());
        }
    }

    commit_plan
        .object_packs
        .retain(|pack_id, _| missing_object_pack_set.contains(pack_id));
    commit_plan.blob_locators.retain(|_, locator| {
        string_field(locator, "pack_id")
            .is_some_and(|pack_id| missing_object_pack_set.contains(&pack_id))
    });
    commit_plan
        .tree_packs
        .retain(|pack_id, _| missing_tree_pack_set.contains(pack_id));
    commit_plan.tree_locators.retain(|_, locator| {
        string_field(locator, "tree_pack_id")
            .is_some_and(|pack_id| missing_tree_pack_set.contains(&pack_id))
    });

    let mut merged_tree_pack_order = Vec::with_capacity(commit_plan.tree_packs.len());
    let mut emitted_tree_pack_ids = BTreeSet::new();
    for pack_id in planned
        .tree_pack_order
        .iter()
        .chain(commit_plan.tree_pack_order.iter())
    {
        if commit_plan.tree_packs.contains_key(pack_id)
            && emitted_tree_pack_ids.insert(pack_id.clone())
        {
            merged_tree_pack_order.push(pack_id.clone());
        }
    }
    if let Some(missing) = commit_plan
        .tree_packs
        .keys()
        .find(|pack_id| !emitted_tree_pack_ids.contains(*pack_id))
    {
        return Err(format!(
            "Commit tree pack {missing} is absent from the dependency-ordered local plan."
        ));
    }
    commit_plan.tree_pack_order = merged_tree_pack_order;
    Ok(commit_plan)
}

#[expect(
    clippy::too_many_arguments,
    reason = "bulk upload coordinates explicit repository, remote, and progress ports"
)]
fn run_zstd_bulk_upload<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    snapshot_ids: &[String],
    present_set: &BTreeSet<String>,
    locator_boundary_snapshot_id: Option<&str>,
    line_name: Option<&str>,
    head_snapshot_id: Option<&str>,
    expected_head_snapshot_id: Option<&str>,
) -> Result<ZstdBulkUploadResult, String>
where
    R: TaskWorkflowZstdPackUploader + ?Sized,
{
    let _bulk_range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk");
    let _remote_sync_range = perfetto_range!("ait.remote_sync.push.zstd_bulk");
    let total_started = Instant::now();
    let mut local_plan_snapshot_ids = snapshot_ids.to_vec();
    let mut local_plan_present_set = present_set.clone();
    if let Some(boundary_snapshot_id) = normalized_text(locator_boundary_snapshot_id) {
        if !local_plan_snapshot_ids.contains(&boundary_snapshot_id) {
            local_plan_snapshot_ids.push(boundary_snapshot_id.clone());
        }
        local_plan_present_set.insert(boundary_snapshot_id);
    }
    let local_plan_started = Instant::now();
    let local_plan = {
        let _range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk.local_plan");
        let _stable_range = perfetto_range!("ait.remote_sync.push.pack_assembly");
        build_zstd_bulk_local_plan(repo, &local_plan_snapshot_ids, &local_plan_present_set)?
    };
    let local_plan_ms = remote_sync_elapsed_ms(local_plan_started);
    let ordered_local_tree_packs = {
        let _range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk.tree_order");
        ordered_tree_pack_metadata(&local_plan)?
    };
    let ordered_local_tree_pack_ids = ordered_local_tree_packs
        .iter()
        .map(|metadata| {
            string_field(metadata, "pack_id")
                .ok_or_else(|| "Ordered tree pack metadata is missing pack_id.".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let plan_request = zstd_bulk_plan_request(snapshot_ids, &local_plan, ordered_local_tree_packs)?;
    let plan_http_started = Instant::now();
    let remote_plan = {
        let _range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk.plan_http");
        let _stable_range = perfetto_range!("ait.remote_sync.push.plan_http");
        task_remote
            .plan_remote_zstd_bulk(repo_name, &plan_request)
            .map_err(|err| err.to_string())?
    };
    let plan_http_ms = remote_sync_elapsed_ms(plan_http_started);
    let mut missing_snapshot_ids = if !remote_plan.missing_snapshot_ids.is_empty()
        || !remote_plan.present_snapshot_ids.is_empty()
    {
        remote_plan.missing_snapshot_ids.clone()
    } else {
        local_plan.snapshot_order.clone()
    };
    missing_snapshot_ids.retain(|snapshot_id| local_plan.snapshots.contains_key(snapshot_id));

    let missing_object_pack_ids = remote_plan.missing_object_pack_ids.clone();
    let present_object_pack_ids = remote_plan.present_object_pack_ids.clone();
    let missing_tree_pack_ids = remote_plan.missing_tree_pack_ids.clone();
    let present_tree_pack_ids = remote_plan.present_tree_pack_ids.clone();

    let pack_prepare_started = Instant::now();
    let object_uploads = {
        let _range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk.object_uploads");
        let _prepare_range = perfetto_range!("ait.remote_sync.push.pack_prepare.object");
        let mut uploads = Vec::with_capacity(missing_object_pack_ids.len());
        for pack_id in &missing_object_pack_ids {
            let pack = local_plan
                .object_packs
                .get(pack_id)
                .ok_or_else(|| format!("Remote requested unknown object pack {pack_id}."))?;
            ait_core::pack_substrate::validate_pack_archive_with_format(
                pack.pack_abs_path.to_string_lossy().as_ref(),
                ait_core::pack_substrate::PACK_FORMAT_ZSTD_CHUNKED_V1,
            )
            .map_err(|err| {
                format!(
                    "Object pack {} failed zstd validation before upload from {}: {err}",
                    pack.pack_id,
                    pack.pack_abs_path.display()
                )
            })?;
            let pack_bytes = fs::read(&pack.pack_abs_path).map_err(|err| {
                format!(
                    "failed to read zstd object pack {} from {}: {err}",
                    pack.pack_id,
                    pack.pack_abs_path.display()
                )
            })?;
            uploads.push((pack_id.clone(), pack_bytes));
        }
        uploads
    };

    let missing_tree_pack_set = missing_tree_pack_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    for pack_id in &missing_tree_pack_ids {
        if !local_plan.tree_packs.contains_key(pack_id) {
            return Err(format!("Remote requested unknown tree pack {pack_id}."));
        }
    }
    let tree_uploads = {
        let _range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk.tree_uploads");
        let _prepare_range = perfetto_range!("ait.remote_sync.push.pack_prepare.tree");
        let mut uploads = Vec::with_capacity(missing_tree_pack_ids.len());
        for pack_id in ordered_local_tree_pack_ids
            .iter()
            .filter(|pack_id| missing_tree_pack_set.contains(*pack_id))
        {
            let pack = local_plan
                .tree_packs
                .get(pack_id)
                .ok_or_else(|| format!("Remote requested unknown tree pack {pack_id}."))?;
            ait_core::pack_substrate::validate_tree_pack_archive_with_format(
                pack.pack_abs_path.to_string_lossy().as_ref(),
                ait_core::pack_substrate::TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            )
            .map_err(|err| {
                format!(
                    "Tree pack {} failed zstd validation before upload from {}: {err}",
                    pack.pack_id,
                    pack.pack_abs_path.display()
                )
            })?;
            let pack_bytes = fs::read(&pack.pack_abs_path).map_err(|err| {
                format!(
                    "failed to read zstd tree pack {} from {}: {err}",
                    pack.pack_id,
                    pack.pack_abs_path.display()
                )
            })?;
            uploads.push((pack_id.clone(), pack_bytes));
        }
        uploads
    };
    let pack_prepare_ms = remote_sync_elapsed_ms(pack_prepare_started);
    let transferred_pack_bytes = object_uploads
        .iter()
        .chain(tree_uploads.iter())
        .fold(0_u64, |total, (_, bytes)| {
            total.saturating_add(bytes.len() as u64)
        });
    let pack_parallelism = remote_sync_pack_parallelism();
    let pack_upload_started = Instant::now();
    let (uploaded_object_pack_responses, uploaded_tree_pack_responses) = {
        let _range = perfetto_range!("ait.remote_sync.push.pack_upload_pipeline");
        task_remote
            .put_remote_zstd_packs_bounded(
                repo_name,
                &object_uploads,
                &tree_uploads,
                pack_parallelism,
            )
            .map_err(|err| err.to_string())?
    };
    let pack_upload_ms = remote_sync_elapsed_ms(pack_upload_started);
    let uploaded_object_packs = uploaded_object_pack_responses.len() as i64;
    let uploaded_tree_packs = uploaded_tree_pack_responses.len() as i64;

    let commit_assembly_started = Instant::now();
    let commit_request = {
        let _range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk.commit_plan");
        let _stable_range = perfetto_range!("ait.remote_sync.push.commit_assembly");
        let commit_plan = zstd_bulk_commit_local_plan(
            repo,
            &local_plan,
            &local_plan_snapshot_ids,
            &missing_snapshot_ids,
            &missing_object_pack_ids,
            &missing_tree_pack_ids,
        )?;
        let ordered_commit_tree_packs = ordered_tree_pack_metadata(&commit_plan)?;
        zstd_bulk_commit_request(
            &commit_plan,
            ordered_commit_tree_packs,
            &missing_snapshot_ids,
            line_name,
            head_snapshot_id,
            expected_head_snapshot_id,
        )?
    };
    let commit_assembly_ms = remote_sync_elapsed_ms(commit_assembly_started);
    let commit_http_started = Instant::now();
    let commit_response = {
        let _range = perfetto_range!("ait.workflow_ready.publish.zstd_bulk.commit_http");
        let _stable_range = perfetto_range!("ait.remote_sync.push.commit_http");
        task_remote
            .commit_remote_zstd_bulk(repo_name, &commit_request)
            .map_err(|err| err.to_string())?
    };
    let commit_http_ms = remote_sync_elapsed_ms(commit_http_started);
    let skipped_snapshots = snapshot_ids.len() as i64 - missing_snapshot_ids.len() as i64;
    Ok(ZstdBulkUploadResult {
        commit_response,
        uploaded_snapshots: missing_snapshot_ids.len() as i64,
        skipped_snapshots,
        uploaded_object_packs,
        skipped_object_packs: present_object_pack_ids.len() as i64,
        uploaded_tree_packs,
        skipped_tree_packs: present_tree_pack_ids.len() as i64,
        remote_plan,
        phase_timings_ms: json!({
            "local_plan": local_plan_ms,
            "plan_http": plan_http_ms,
            "pack_prepare": pack_prepare_ms,
            "pack_upload_pipeline": pack_upload_ms,
            "commit_assembly": commit_assembly_ms,
            "commit_http": commit_http_ms,
            "total": remote_sync_elapsed_ms(total_started),
        }),
        remote_round_trips: 2 + uploaded_object_packs + uploaded_tree_packs,
        transferred_pack_bytes,
        pack_parallelism,
    })
}

pub(in crate::primitives) fn set_or_create_local_line_head(
    repo: &RepoRuntime,
    line_name: &str,
    snapshot_id: Option<&str>,
) -> Result<JsonValue, String> {
    let source = selected_remote_sync_local_store(repo)?;
    let timestamp = system_event_timestamp();
    match source.line_by_name(line_name)? {
        Some(row) => {
            let status = row.status;
            if status == "archived" {
                return Err(format!("Line {line_name} is archived and cannot move"));
            }
            source.set_line_head(line_name, snapshot_id, &timestamp)
        }
        None => source.create_line(line_name, snapshot_id, &timestamp),
    }
}

fn ensure_local_line_can_move(repo: &RepoRuntime, line_name: &str) -> Result<(), String> {
    match remote_sync_local_line_record(repo, line_name)? {
        Some(row) => {
            let status = row.status;
            if status == "archived" {
                return Err(format!("Line {line_name} is archived and cannot move"));
            }
            Ok(())
        }
        None => Ok(()),
    }
}

fn require_zstd_download_capability(
    capabilities: &RemoteSyncCapabilities,
) -> Result<RemoteSyncBackendNegotiation, String> {
    if !capabilities.zstd_pack_bulk_download {
        return Err(format!(
            "Remote sync requires capability {REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK_DOWNLOAD}."
        ));
    }
    Ok(RemoteSyncBackendNegotiation {
        backend: RemoteSyncBackendKind::ZstdPackBulk,
        reason: "zstd_only_policy_requires_zstd_pack_bulk_download",
        capabilities: capabilities.clone(),
    })
}

fn zstd_download_backend_payload(
    negotiation: &RemoteSyncBackendNegotiation,
    snapshot_ids: &[String],
) -> JsonValue {
    remote_sync_backend_payload(
        negotiation,
        &RemoteSyncInventoryDiff {
            checked_snapshot_ids: snapshot_ids.to_vec(),
            present_snapshot_ids: Vec::new(),
            missing_snapshot_ids: snapshot_ids.to_vec(),
        },
    )
}

fn zstd_manifest_snapshot_metadata(manifest: &ZstdImportManifestPayload) -> JsonValue {
    let snapshot = manifest.snapshots.first();
    json!({
        "repo_name": manifest.repo_name,
        "snapshot_id": manifest.snapshot_id,
        "parent_snapshot_ids": snapshot.map(|row| row.parent_snapshot_ids.clone()).unwrap_or_default(),
        "primary_parent_snapshot_id": snapshot.and_then(|row| row.primary_parent_snapshot_id.clone()),
        "parent_snapshot_id": snapshot.and_then(|row| row.parent_snapshot_id.clone()),
        "root_tree_pack_id": snapshot.and_then(|row| row.root_tree_pack_id.clone()),
        "root_entry_ordinal": snapshot.and_then(|row| row.root_entry_ordinal),
        "manifest_hash": snapshot.and_then(|row| row.manifest_hash.clone()),
        "message": snapshot.and_then(|row| row.message.clone()),
        "line_name": snapshot.and_then(|row| row.line_name.clone()),
        "snapshot_kind": snapshot.and_then(|row| row.snapshot_kind.clone()),
        "file_count": snapshot.and_then(|row| row.file_count),
        "total_bytes": snapshot.and_then(|row| row.total_bytes),
        "created_at": snapshot.and_then(|row| row.created_at.clone()),
    })
}

fn ordered_parent_snapshot_ids_from_zstd_manifest(
    manifest: &ZstdImportManifestPayload,
) -> Result<Vec<String>, String> {
    let snapshot = manifest.snapshots.first().ok_or_else(|| {
        format!(
            "Zstd import manifest for {} is missing snapshot row.",
            manifest.snapshot_id
        )
    })?;
    if snapshot.snapshot_id != manifest.snapshot_id {
        return Err(format!(
            "Zstd import manifest {} contains snapshot row {}.",
            manifest.snapshot_id, snapshot.snapshot_id
        ));
    }
    normalize_snapshot_parent_set(
        Some(&snapshot.snapshot_id),
        Some(snapshot.parent_snapshot_ids.clone()),
        snapshot.primary_parent_snapshot_id.clone(),
        snapshot.parent_snapshot_id.clone(),
    )
    .map(|(parents, _, _)| parents)
}

fn get_remote_zstd_import_manifest_for_import<R>(
    task_remote: &mut R,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<ZstdImportManifestPayload, String>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    let manifest = task_remote
        .get_remote_zstd_import_manifest(repo_name, snapshot_id)
        .map_err(|err| err.to_string())?;
    if manifest.repo_name != repo_name {
        return Err(format!(
            "Remote zstd import manifest returned unexpected repository {:?} (expected {:?})",
            manifest.repo_name, repo_name
        ));
    }
    if manifest.snapshot_id != snapshot_id {
        return Err(format!(
            "Remote zstd import manifest returned unexpected snapshot {:?} (expected {:?})",
            manifest.snapshot_id, snapshot_id
        ));
    }
    Ok(manifest)
}

fn get_remote_zstd_pull_manifest_for_import<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    head_snapshot_id: &str,
) -> Result<(ZstdPullManifestPayload, i64), String>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    let source = selected_remote_sync_local_store(repo)?;
    let ctx = remote_sync_local_store_context(repo);
    let mut have_snapshot_ids = source
        .snapshot_parent_rows(&ctx)?
        .into_iter()
        .map(|row| row.snapshot_id)
        .collect::<BTreeSet<_>>();
    let mut round_trips = 0_i64;
    loop {
        let request = ZstdPullManifestRequest {
            contract: ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME.to_string(),
            head_snapshot_id: head_snapshot_id.to_string(),
            have_snapshot_ids: have_snapshot_ids.iter().cloned().collect(),
        };
        round_trips += 1;
        let manifest = task_remote
            .get_remote_zstd_pull_manifest(repo_name, &request)
            .map_err(|err| err.to_string())?;
        if manifest.repo_name != repo_name {
            return Err(format!(
                "Remote zstd pull manifest returned unexpected repository {:?} (expected {:?})",
                manifest.repo_name, repo_name
            ));
        }
        if manifest.head_snapshot_id != head_snapshot_id {
            return Err(format!(
                "Remote zstd pull manifest returned unexpected head {:?} (expected {:?})",
                manifest.head_snapshot_id, head_snapshot_id
            ));
        }

        let mut incomplete_boundaries = Vec::new();
        for snapshot_id in &manifest.boundary_snapshot_ids {
            if !have_snapshot_ids.contains(snapshot_id) {
                return Err(format!(
                    "Remote zstd pull manifest selected boundary {snapshot_id}, but the client did not advertise it."
                ));
            }
            if !source.snapshot_content_complete(&ctx, snapshot_id)? {
                incomplete_boundaries.push(snapshot_id.clone());
            }
        }
        if incomplete_boundaries.is_empty() {
            return Ok((manifest, round_trips));
        }
        let previous_have_count = have_snapshot_ids.len();
        for snapshot_id in incomplete_boundaries {
            have_snapshot_ids.remove(&snapshot_id);
        }
        if have_snapshot_ids.len() == previous_have_count {
            return Err(
                "Remote zstd pull manifest selected an incomplete local boundary that was not advertised by the client."
                    .to_string(),
            );
        }
    }
}

fn import_zstd_manifest_into_local_store(
    repo: &RepoRuntime,
    manifest: &ZstdImportManifestPayload,
    plan: &super::local_store::ZstdImportDownloadPlan,
    object_pack_bytes: &BTreeMap<String, Vec<u8>>,
    tree_pack_bytes: &BTreeMap<String, Vec<u8>>,
) -> Result<ZstdImportApplyResult, String> {
    let source = selected_remote_sync_local_store(repo)?;
    let ctx = remote_sync_local_store_context(repo);
    source.import_zstd_manifest(
        &ctx,
        manifest,
        ZstdImportHistoryMode::CompleteAncestry,
        plan,
        object_pack_bytes,
        tree_pack_bytes,
    )
}

fn zstd_import_download_plan(
    repo: &RepoRuntime,
    manifest: &ZstdImportManifestPayload,
) -> Result<super::local_store::ZstdImportDownloadPlan, String> {
    let source = selected_remote_sync_local_store(repo)?;
    let ctx = remote_sync_local_store_context(repo);
    source.zstd_import_download_plan(&ctx, manifest)
}

fn selected_remote_sync_local_store(
    repo: &RepoRuntime,
) -> Result<crate::runtime::RepoRemoteSyncLocalStore<REMOTE_SYNC_BINARY_DB_WRITE_LAYOUT>, String> {
    repo.remote_sync_local_store::<REMOTE_SYNC_BINARY_DB_WRITE_LAYOUT>()
}

fn import_remote_zstd_snapshot_boundary<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    manifest: ZstdImportManifestPayload,
) -> Result<ZstdImportChainResult, String>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    let _range = perfetto_range!("ait.remote_sync.external.import_boundary");
    let total_started = Instant::now();
    let pack_parallelism = remote_sync_pack_parallelism();
    let plan = zstd_import_download_plan(repo, &manifest)?;
    let pack_download_started = Instant::now();
    let (object_pack_bytes, tree_pack_bytes) = {
        let _range = perfetto_range!("ait.remote_sync.external.pack_download_pipeline");
        task_remote
            .get_remote_zstd_packs_bounded(
                repo_name,
                &plan.missing_object_pack_ids,
                &plan.missing_tree_pack_ids,
                pack_parallelism,
            )
            .map_err(|err| err.to_string())?
    };
    let pack_download_ms = remote_sync_elapsed_ms(pack_download_started);
    let transferred_pack_bytes = object_pack_bytes
        .values()
        .chain(tree_pack_bytes.values())
        .fold(0_u64, |total, bytes| {
            total.saturating_add(bytes.len() as u64)
        });
    let metadata_import_started = Instant::now();
    let applied = {
        let _range = perfetto_range!("ait.remote_sync.external.metadata_import");
        let source = selected_remote_sync_local_store(repo)?;
        let ctx = remote_sync_local_store_context(repo);
        source.import_zstd_manifest(
            &ctx,
            &manifest,
            ZstdImportHistoryMode::RemoteHeadBoundary,
            &plan,
            &object_pack_bytes,
            &tree_pack_bytes,
        )?
    };
    let metadata_import_ms = remote_sync_elapsed_ms(metadata_import_started);
    let mut result = ZstdImportChainResult {
        downloaded_object_packs: applied.downloaded_object_packs,
        reused_object_packs: applied.reused_object_packs,
        downloaded_tree_packs: applied.downloaded_tree_packs,
        reused_tree_packs: applied.reused_tree_packs,
        upserted_blob_locators: applied.upserted_blob_locators,
        upserted_tree_locators: applied.upserted_tree_locators,
        pack_download_ms,
        metadata_import_ms,
        remote_round_trips: (plan.missing_object_pack_ids.len() + plan.missing_tree_pack_ids.len())
            as i64,
        transferred_pack_bytes,
        pack_parallelism,
        ..ZstdImportChainResult::default()
    };
    if applied.imported_snapshot {
        result.imported_snapshots = 1;
        result.imported_snapshot_ids.push(applied.snapshot_id);
    }
    result.total_ms = remote_sync_elapsed_ms(total_started);
    Ok(result)
}

fn stage_zstd_pull_manifest_packs<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    manifest: &ZstdImportManifestPayload,
    plan: &super::local_store::ZstdImportDownloadPlan,
    result: &mut ZstdImportChainResult,
) -> Result<(), String>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    let source = selected_remote_sync_local_store(repo)?;
    let ctx = remote_sync_local_store_context(repo);
    let object_packs = manifest
        .object_packs
        .iter()
        .map(|pack| (pack.pack_id.as_str(), pack))
        .collect::<BTreeMap<_, _>>();
    let tree_packs = manifest
        .tree_packs
        .iter()
        .map(|pack| (pack.pack_id.as_str(), pack))
        .collect::<BTreeMap<_, _>>();

    let record_batch = |result: &mut ZstdImportChainResult,
                        object_rows: Vec<ZstdBulkObjectPackRow>,
                        tree_rows: Vec<ZstdBulkTreePackRow>,
                        object_pack_bytes: BTreeMap<String, Vec<u8>>,
                        tree_pack_bytes: BTreeMap<String, Vec<u8>>|
     -> Result<(), String> {
        result.transferred_pack_bytes = result.transferred_pack_bytes.saturating_add(
            object_pack_bytes
                .values()
                .chain(tree_pack_bytes.values())
                .fold(0_u64, |total, bytes| {
                    total.saturating_add(bytes.len() as u64)
                }),
        );
        let staged = source.stage_zstd_import_pack_batch(
            &ctx,
            &object_rows,
            &tree_rows,
            &object_pack_bytes,
            &tree_pack_bytes,
        )?;
        accumulate_zstd_pack_stage_result(result, &staged);
        Ok(())
    };

    for pack_ids in plan.missing_object_pack_ids.chunks(result.pack_parallelism) {
        let rows = pack_ids
            .iter()
            .map(|pack_id| {
                object_packs
                    .get(pack_id.as_str())
                    .map(|pack| (*pack).clone())
                    .ok_or_else(|| {
                        format!(
                            "Zstd pull manifest download plan references unknown object pack {pack_id}."
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (object_pack_bytes, tree_pack_bytes) = task_remote
            .get_remote_zstd_packs_bounded(repo_name, pack_ids, &[], result.pack_parallelism)
            .map_err(|err| err.to_string())?;
        result.remote_round_trips += pack_ids.len() as i64;
        record_batch(result, rows, Vec::new(), object_pack_bytes, tree_pack_bytes)?;
    }
    for pack_ids in plan.missing_tree_pack_ids.chunks(result.pack_parallelism) {
        let rows = pack_ids
            .iter()
            .map(|pack_id| {
                tree_packs
                    .get(pack_id.as_str())
                    .map(|pack| (*pack).clone())
                    .ok_or_else(|| {
                        format!(
                            "Zstd pull manifest download plan references unknown tree pack {pack_id}."
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let (object_pack_bytes, tree_pack_bytes) = task_remote
            .get_remote_zstd_packs_bounded(repo_name, &[], pack_ids, result.pack_parallelism)
            .map_err(|err| err.to_string())?;
        result.remote_round_trips += pack_ids.len() as i64;
        record_batch(result, Vec::new(), rows, object_pack_bytes, tree_pack_bytes)?;
    }
    Ok(())
}

fn accumulate_zstd_pack_stage_result(
    result: &mut ZstdImportChainResult,
    staged: &ZstdImportPackStageResult,
) {
    result.downloaded_object_packs += staged.downloaded_object_packs;
    result.reused_object_packs += staged.reused_object_packs;
    result.downloaded_tree_packs += staged.downloaded_tree_packs;
    result.reused_tree_packs += staged.reused_tree_packs;
}

fn import_remote_zstd_pull_manifest<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    head_snapshot_id: &str,
) -> Result<ZstdImportChainResult, String>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    let _range = perfetto_range!("ait.remote_sync.pull.bulk_manifest");
    let total_started = Instant::now();
    let manifest_started = Instant::now();
    let (manifest, manifest_round_trips) =
        get_remote_zstd_pull_manifest_for_import(repo, task_remote, repo_name, head_snapshot_id)?;
    let manifest_ancestry_ms = remote_sync_elapsed_ms(manifest_started);
    if manifest.snapshots.is_empty() {
        return Ok(ZstdImportChainResult {
            manifest_ancestry_ms,
            total_ms: remote_sync_elapsed_ms(total_started),
            remote_round_trips: manifest_round_trips,
            pack_parallelism: remote_sync_pack_parallelism(),
            ..ZstdImportChainResult::default()
        });
    }

    let ZstdPullManifestPayload {
        snapshots,
        object_packs,
        tree_packs,
        blob_locators,
        tree_locators,
        ..
    } = manifest;
    let first_snapshot = snapshots
        .first()
        .cloned()
        .ok_or_else(|| "Zstd pull manifest lost its first snapshot row.".to_string())?;
    let union_manifest = ZstdImportManifestPayload {
        contract: ZSTD_IMPORT_MANIFEST_CONTRACT_NAME.to_string(),
        repo_name: repo_name.to_string(),
        snapshot_id: first_snapshot.snapshot_id.clone(),
        snapshots: vec![first_snapshot],
        object_packs,
        tree_packs,
        blob_locators,
        tree_locators,
        line_update: None,
    };
    let plan = zstd_import_download_plan(repo, &union_manifest)?;
    let mut result = ZstdImportChainResult {
        manifest_ancestry_ms,
        remote_round_trips: manifest_round_trips,
        pack_parallelism: remote_sync_pack_parallelism(),
        reused_object_packs: plan.reusable_object_pack_ids.len() as i64,
        reused_tree_packs: plan.reusable_tree_pack_ids.len() as i64,
        ..ZstdImportChainResult::default()
    };

    let pack_download_started = Instant::now();
    {
        let _range = perfetto_range!("ait.remote_sync.pull.pack_download_pipeline");
        stage_zstd_pull_manifest_packs(
            repo,
            task_remote,
            repo_name,
            &union_manifest,
            &plan,
            &mut result,
        )?;
    }
    result.pack_download_ms = remote_sync_elapsed_ms(pack_download_started);

    let metadata_import_started = Instant::now();
    {
        let _range = perfetto_range!("ait.remote_sync.pull.metadata_import");
        let empty_object_packs = BTreeMap::new();
        let empty_tree_packs = BTreeMap::new();
        let applied = import_zstd_manifest_into_local_store(
            repo,
            &union_manifest,
            &plan,
            &empty_object_packs,
            &empty_tree_packs,
        )?;
        if applied.imported_snapshot {
            result.imported_snapshots += 1;
            result.imported_snapshot_ids.push(applied.snapshot_id);
        }
        result.upserted_blob_locators += applied.upserted_blob_locators;
        result.upserted_tree_locators += applied.upserted_tree_locators;

        let source = selected_remote_sync_local_store(repo)?;
        let ctx = remote_sync_local_store_context(repo);
        let imported_snapshot_ids = source.import_zstd_snapshot_rows(&ctx, &snapshots[1..])?;
        result.imported_snapshots += imported_snapshot_ids.len() as i64;
        result.imported_snapshot_ids.extend(imported_snapshot_ids);
    }
    result.metadata_import_ms = remote_sync_elapsed_ms(metadata_import_started);
    result.total_ms = remote_sync_elapsed_ms(total_started);
    Ok(result)
}

fn import_remote_zstd_snapshot_chain<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    head_snapshot_id: Option<&str>,
    initial_manifest: Option<ZstdImportManifestPayload>,
    remote_sync_capabilities: &RemoteSyncCapabilities,
) -> Result<ZstdImportChainResult, String>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    let _range = perfetto_range!("ait.remote_sync.pull.import_chain");
    let total_started = Instant::now();
    let Some(head_snapshot_id) = normalized_text(head_snapshot_id) else {
        return Ok(ZstdImportChainResult::default());
    };
    if remote_sync_capabilities.zstd_pull_manifest && initial_manifest.is_none() {
        return import_remote_zstd_pull_manifest(repo, task_remote, repo_name, &head_snapshot_id);
    }
    let limits = SnapshotDagLimits::default();
    let mut pending = VecDeque::from([head_snapshot_id.clone()]);
    let mut queued = BTreeSet::from([head_snapshot_id]);
    let mut parent_map = BTreeMap::new();
    let mut known_local = BTreeSet::new();
    let mut manifests = BTreeMap::new();
    let mut initial_manifest = initial_manifest;
    let mut manifest_round_trips = 0_i64;
    let manifest_started = Instant::now();
    {
        let _manifest_range = perfetto_range!("ait.remote_sync.pull.manifest_ancestry");
        while let Some(snapshot_id) = pending.pop_front() {
            if remote_sync_snapshot_content_complete_for_repo(repo, &snapshot_id)? {
                known_local.insert(snapshot_id);
                continue;
            }
            let manifest = match initial_manifest.take() {
                Some(value) => {
                    if value.snapshot_id != snapshot_id {
                        return Err(format!(
                            "Initial zstd import manifest is for {}, expected {snapshot_id}.",
                            value.snapshot_id
                        ));
                    }
                    value
                }
                None => {
                    manifest_round_trips += 1;
                    get_remote_zstd_import_manifest_for_import(
                        task_remote,
                        repo_name,
                        &snapshot_id,
                    )?
                }
            };
            let parents = ordered_parent_snapshot_ids_from_zstd_manifest(&manifest)?;
            for parent in &parents {
                if queued.insert(parent.clone()) {
                    if queued.len() > limits.max_results {
                        return Err(format!(
                            "Remote zstd Snapshot DAG import exceeded max_results {} at parent {parent} of {snapshot_id}.",
                            limits.max_results
                        ));
                    }
                    pending.push_back(parent.clone());
                }
            }
            parent_map.insert(snapshot_id.clone(), parents);
            manifests.insert(snapshot_id, manifest);
        }
    }

    let mut result = ZstdImportChainResult {
        manifest_ancestry_ms: remote_sync_elapsed_ms(manifest_started),
        remote_round_trips: manifest_round_trips,
        pack_parallelism: remote_sync_pack_parallelism(),
        ..ZstdImportChainResult::default()
    };
    let order = topological_snapshot_order(&parent_map, &known_local)?;
    for snapshot_id in order {
        let manifest = manifests.remove(&snapshot_id).ok_or_else(|| {
            format!("Remote zstd Snapshot DAG import lost manifest for {snapshot_id}.")
        })?;
        let plan = zstd_import_download_plan(repo, &manifest)?;
        let pack_download_started = Instant::now();
        let (object_pack_bytes, tree_pack_bytes) = {
            let _range = perfetto_range!("ait.remote_sync.pull.pack_download_pipeline");
            task_remote
                .get_remote_zstd_packs_bounded(
                    repo_name,
                    &plan.missing_object_pack_ids,
                    &plan.missing_tree_pack_ids,
                    result.pack_parallelism,
                )
                .map_err(|err| err.to_string())?
        };
        result.pack_download_ms += remote_sync_elapsed_ms(pack_download_started);
        result.remote_round_trips +=
            (plan.missing_object_pack_ids.len() + plan.missing_tree_pack_ids.len()) as i64;
        result.transferred_pack_bytes = result.transferred_pack_bytes.saturating_add(
            object_pack_bytes
                .values()
                .chain(tree_pack_bytes.values())
                .fold(0_u64, |total, bytes| {
                    total.saturating_add(bytes.len() as u64)
                }),
        );
        let metadata_import_started = Instant::now();
        let applied = {
            let _range = perfetto_range!("ait.remote_sync.pull.metadata_import");
            import_zstd_manifest_into_local_store(
                repo,
                &manifest,
                &plan,
                &object_pack_bytes,
                &tree_pack_bytes,
            )?
        };
        result.metadata_import_ms += remote_sync_elapsed_ms(metadata_import_started);
        if applied.imported_snapshot {
            result.imported_snapshots += 1;
            result.imported_snapshot_ids.push(applied.snapshot_id);
        }
        result.downloaded_object_packs += applied.downloaded_object_packs;
        result.reused_object_packs += applied.reused_object_packs;
        result.downloaded_tree_packs += applied.downloaded_tree_packs;
        result.reused_tree_packs += applied.reused_tree_packs;
        result.upserted_blob_locators += applied.upserted_blob_locators;
        result.upserted_tree_locators += applied.upserted_tree_locators;
    }
    result.total_ms = remote_sync_elapsed_ms(total_started);
    Ok(result)
}

pub(in crate::primitives) fn remote_sync_line_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    line_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    let remote_line = task_remote
        .get_line(repo_name, line_name)
        .map_err(|err| err.to_string())?;
    verify_remote_pull_line(&remote_line, repo_name, line_name)?;
    Ok(remote_line)
}

pub(in crate::primitives) fn remote_sync_line_head_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    line_name: &str,
) -> Result<Option<String>, String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    let Some(line) =
        remote_sync_line_head_row_read_with_task_remote(task_remote, repo_name, line_name)?
    else {
        return Ok(None);
    };
    Ok(string_field(&line, "head_snapshot_id"))
}

pub(super) fn remote_sync_line_head_row_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    line_name: &str,
) -> Result<Option<JsonValue>, String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    match task_remote.get_line(repo_name, line_name) {
        Ok(line) => {
            verify_remote_line_identity_when_present(&line, repo_name, line_name)?;
            Ok(Some(line))
        }
        Err(message) => {
            let message = message.to_string();
            if message.contains("failed: 404") || message.contains("Unknown line") {
                Ok(None)
            } else {
                Err(message)
            }
        }
    }
}

#[cfg(test)]
pub(in crate::primitives) fn remote_sync_snapshot_metadata_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowSnapshotMetadataReader + ?Sized,
{
    let remote_snapshot = task_remote
        .get_remote_snapshot(repo_name, snapshot_id, false, None)
        .map_err(|err| err.to_string())?;
    verify_remote_snapshot_metadata(&remote_snapshot, repo_name, snapshot_id)?;
    Ok(remote_snapshot)
}

pub(in crate::primitives) fn remote_sync_present_snapshot_ids_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    snapshot_ids: &[String],
) -> Result<BTreeSet<String>, String>
where
    R: TaskWorkflowSnapshotExistenceReader + ?Sized,
{
    if snapshot_ids.is_empty() {
        return Ok(BTreeSet::new());
    }
    let existence = task_remote
        .get_remote_snapshots_existence(repo_name, snapshot_ids)
        .map_err(|err| err.to_string())?;
    Ok(json_string_list(existence.get("present"))
        .into_iter()
        .collect())
}

pub(in crate::primitives) fn remote_sync_line_update_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    line_name: &str,
    head_snapshot_id: Option<&str>,
    expected_head_snapshot_id: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineHeadUpdater + ?Sized,
{
    task_remote
        .update_remote_line(
            repo_name,
            line_name,
            head_snapshot_id,
            expected_head_snapshot_id,
        )
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn hydrate_remote_snapshot_chain_with_task_remote_and_capabilities<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    remote_name: &str,
    repo_name: &str,
    snapshot_id: &str,
    remote_sync_capabilities: &RemoteSyncCapabilities,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    let negotiation = require_zstd_download_capability(remote_sync_capabilities)?;
    let initial_manifest =
        get_remote_zstd_import_manifest_for_import(task_remote, repo_name, snapshot_id)?;
    let remote_snapshot = zstd_manifest_snapshot_metadata(&initial_manifest);
    let import = import_remote_zstd_snapshot_chain(
        repo,
        task_remote,
        repo_name,
        Some(snapshot_id),
        Some(initial_manifest),
        remote_sync_capabilities,
    )?;
    Ok(zstd_hydration_payload(
        remote_name,
        repo_name,
        snapshot_id,
        remote_snapshot,
        &negotiation,
        import,
    ))
}

pub(in crate::primitives) fn hydrate_remote_snapshot_boundary_with_task_remote_and_capabilities<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    remote_name: &str,
    repo_name: &str,
    snapshot_id: &str,
    remote_sync_capabilities: &RemoteSyncCapabilities,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowZstdPackReader + ?Sized,
{
    let negotiation = require_zstd_download_capability(remote_sync_capabilities)?;
    let manifest = get_remote_zstd_import_manifest_for_import(task_remote, repo_name, snapshot_id)?;
    let remote_snapshot = zstd_manifest_snapshot_metadata(&manifest);
    let import = import_remote_zstd_snapshot_boundary(repo, task_remote, repo_name, manifest)?;
    Ok(zstd_hydration_payload(
        remote_name,
        repo_name,
        snapshot_id,
        remote_snapshot,
        &negotiation,
        import,
    ))
}

fn zstd_hydration_payload(
    remote_name: &str,
    repo_name: &str,
    snapshot_id: &str,
    remote_snapshot: JsonValue,
    negotiation: &RemoteSyncBackendNegotiation,
    import: ZstdImportChainResult,
) -> JsonValue {
    json!({
        "remote": remote_name,
        "repo_name": repo_name,
        "snapshot_id": snapshot_id,
        "remote_snapshot": remote_snapshot,
        "imported_snapshots": import.imported_snapshots,
        "imported_snapshot_ids": import.imported_snapshot_ids,
        "remote_sync_backend": zstd_download_backend_payload(negotiation, &import.imported_snapshot_ids),
        "zstd_bulk": {
            "downloaded_object_packs": import.downloaded_object_packs,
            "reused_object_packs": import.reused_object_packs,
            "downloaded_tree_packs": import.downloaded_tree_packs,
            "reused_tree_packs": import.reused_tree_packs,
            "upserted_blob_locators": import.upserted_blob_locators,
            "upserted_tree_locators": import.upserted_tree_locators,
        },
    })
}

pub(in crate::primitives) fn remote_repository_authority_http_config(
    mut config: PlanHttpClientConfig,
    remote_repository: &JsonValue,
    expected_repository_index: RepositoryIndex,
    expected_repo_name: &str,
) -> Result<PlanHttpClientConfig, String> {
    let expected_repo_name = normalized_text(Some(expected_repo_name))
        .ok_or_else(|| "External hydration repository name must be non-empty.".to_string())?;
    let repository = remote_repository
        .get("repository")
        .unwrap_or(remote_repository);
    let repository_index = repository
        .get("repository_index")
        .ok_or_else(|| {
            format!(
                "Remote repository {expected_repo_name} is missing its canonical repository_index before external snapshot hydration."
            )
        })
        .and_then(RepositoryIndex::parse_config_value)?;
    if repository_index != expected_repository_index {
        return Err(format!(
            "Remote repository index drifted before external snapshot hydration: declared source {expected_repository_index}, received {repository_index}."
        ));
    }
    config.repository_index = Some(expected_repository_index);
    Ok(config)
}

pub(crate) fn hydrate_remote_snapshot_boundary_for_repo(
    repo: &RepoRuntime,
    remote_name: &str,
    source_repository_index: RepositoryIndex,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<JsonValue, String> {
    let (remote_row, resolved_repo_name) =
        remote_context(repo, Some(remote_name), Some(repo_name))?;
    let mut discovery_remote = http_task_remote(repo, &remote_row)?;
    let remote_repository = discovery_remote
        .get_repository_by_index(source_repository_index)
        .map_err(|err| {
        format!(
            "Remote repository {resolved_repo_name} could not be read before external snapshot hydration: {err}"
        )
    })?;
    let remote_sync_capabilities =
        RemoteSyncCapabilities::from_server_payload(Some(&remote_repository));
    let authority_config = remote_repository_authority_http_config(
        http_config(repo, &remote_row),
        &remote_repository,
        source_repository_index,
        &resolved_repo_name,
    )?;
    let mut authority_remote =
        HttpTaskRemote::new(authority_config).map_err(|err| err.to_string())?;
    let mut payload = hydrate_remote_snapshot_boundary_with_task_remote_and_capabilities(
        repo,
        &mut authority_remote,
        &remote_row.name,
        &resolved_repo_name,
        snapshot_id,
        &remote_sync_capabilities,
    )?;
    payload
        .as_object_mut()
        .ok_or_else(|| "snapshot hydration payload is malformed".to_string())?
        .insert("remote_repository".to_string(), remote_repository);
    Ok(payload)
}

fn restore_pulled_line_workspace(
    repo: &RepoRuntime,
    line_name: &str,
    target_snapshot_id: Option<&str>,
    baseline_snapshot_id: Option<&str>,
    force: bool,
) -> Result<JsonValue, String> {
    let current_line_before = repo.current_line_name()?;
    let mut restored =
        restore_workspace_all(repo, target_snapshot_id, baseline_snapshot_id, force, false)?;
    let result_obj = restored
        .as_object_mut()
        .ok_or_else(|| "pull restore payload must be an object".to_string())?;
    result_obj.insert("repo_name".to_string(), JsonValue::String(repo.repo_name()));
    result_obj.insert(
        "workspace_root".to_string(),
        JsonValue::String(repo.workspace_root().to_string_lossy().to_string()),
    );
    result_obj.insert(
        "worktree_name".to_string(),
        repo.config
            .get("worktree_name")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    result_obj.insert(
        "current_line_before".to_string(),
        JsonValue::String(current_line_before.clone()),
    );
    result_obj.insert(
        "current_line".to_string(),
        JsonValue::String(line_name.to_string()),
    );
    result_obj.insert(
        "line_name".to_string(),
        JsonValue::String(line_name.to_string()),
    );
    result_obj.insert(
        "line_head_snapshot_id".to_string(),
        target_snapshot_id
            .map(|value| JsonValue::String(value.to_string()))
            .unwrap_or(JsonValue::Null),
    );
    set_runtime_current_line(repo, line_name)?;
    repo.set_worktree_materialized_snapshot(target_snapshot_id)?;
    Ok(restored)
}

fn validate_pull_workspace_options(merge: bool, restore: bool, force: bool) -> Result<(), String> {
    if merge && force {
        return Err(
            "--force cannot be used with --merge; divergent merge requires a clean workspace."
                .to_string(),
        );
    }
    if merge && !restore {
        return Err(
            "--merge requires --restore because a divergent merge materializes the workspace."
                .to_string(),
        );
    }
    if force && !restore {
        return Err("--force only applies together with --restore".to_string());
    }
    Ok(())
}

pub fn pull(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    line_name: Option<&str>,
    merge: bool,
    restore: bool,
    force: bool,
) -> Result<JsonValue, String> {
    let mut backend = HttpRemoteSyncBackend;
    pull_with_remote_sync_backend(
        &mut backend,
        repo,
        remote_name,
        line_name,
        merge,
        restore,
        force,
    )
}

pub(in crate::primitives) fn pull_with_remote_sync_backend<B>(
    backend: &mut B,
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    line_name: Option<&str>,
    merge: bool,
    restore: bool,
    force: bool,
) -> Result<JsonValue, String>
where
    B: RemoteSyncBackend + ?Sized,
{
    validate_pull_workspace_options(merge, restore, force)?;
    let (remote_row, repo_name) = backend.remote_context(repo, remote_name)?;
    let resolved_line_name = match normalized_text(line_name) {
        Some(value) => value,
        None => repo.current_line_name()?,
    };
    if resolved_line_name.is_empty() {
        return Err("Current line is not configured; pass --line.".to_string());
    }
    backend.pull_line(
        repo,
        &remote_row,
        &repo_name,
        &resolved_line_name,
        merge,
        restore,
        force,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "remote pull keeps capability and history policies explicit"
)]
pub(in crate::primitives) fn pull_line_with_task_remote_and_capabilities<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    remote_name: &str,
    repo_name: &str,
    line_name: &str,
    merge: bool,
    restore: bool,
    force: bool,
    remote_sync_capabilities: &RemoteSyncCapabilities,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader + TaskWorkflowZstdPackReader + ?Sized,
{
    let _pull_range = perfetto_range!("ait.remote_sync.pull");
    validate_pull_workspace_options(merge, restore, force)?;
    let remote_line = {
        let _range = perfetto_range!("ait.remote_sync.pull.line_read");
        remote_sync_line_read_with_task_remote(task_remote, repo_name, line_name)?
    };
    let (local_line_present, previous_line_head_snapshot_id) = {
        let _range = perfetto_range!("ait.remote_sync.pull.local_state");
        local_line_state(repo, line_name)?
    };
    let current_line_before = repo.current_line_name()?;
    let restore_baseline_snapshot_id =
        remote_sync_local_line_head_snapshot_id(repo, &current_line_before)?;
    let head_snapshot_id = string_field(&remote_line, "head_snapshot_id");
    let negotiation = require_zstd_download_capability(remote_sync_capabilities)?;
    let import = {
        let _range = perfetto_range!("ait.remote_sync.pull.pack_import");
        import_remote_zstd_snapshot_chain(
            repo,
            task_remote,
            repo_name,
            head_snapshot_id.as_deref(),
            None,
            remote_sync_capabilities,
        )?
    };
    let imported_snapshots = import.imported_snapshots;
    let phase_timings_ms = json!({
        "manifest_ancestry": import.manifest_ancestry_ms,
        "pack_download_pipeline": import.pack_download_ms,
        "metadata_import": import.metadata_import_ms,
        "total_import": import.total_ms,
    });
    let remote_sync_metrics = json!({
        "remote_round_trips": import.remote_round_trips + 1,
        "transferred_pack_bytes": import.transferred_pack_bytes,
        "pack_parallelism": import.pack_parallelism,
    });
    let imported_snapshot_ids = import.imported_snapshot_ids;
    let remote_sync_backend = zstd_download_backend_payload(&negotiation, &imported_snapshot_ids);
    let zstd_bulk = json!({
        "downloaded_object_packs": import.downloaded_object_packs,
        "reused_object_packs": import.reused_object_packs,
        "downloaded_tree_packs": import.downloaded_tree_packs,
        "reused_tree_packs": import.reused_tree_packs,
        "upserted_blob_locators": import.upserted_blob_locators,
        "upserted_tree_locators": import.upserted_tree_locators,
    });
    let relationship = {
        let _range = perfetto_range!("ait.remote_sync.pull.ancestry_relationship");
        classify_pull_head_relationship(
            repo,
            local_line_present,
            previous_line_head_snapshot_id.as_deref(),
            head_snapshot_id.as_deref(),
        )?
    };
    if relationship == "local_ahead" && restore {
        return Err(format!(
            "Cannot apply --restore because local Line {line_name} at {} is ahead of remote {remote_name}/{line_name} at {}. Imported {imported_snapshots} missing Snapshot(s), which remain available; no Line head or workspace was moved. Run `ait pull --remote {remote_name} --line {line_name}` without --restore for import-only synchronization.",
            previous_line_head_snapshot_id.as_deref().unwrap_or("none"),
            head_snapshot_id.as_deref().unwrap_or("none"),
        ));
    }
    if relationship == "divergent" && !merge {
        let remote_flag = format!("--remote {remote_name}");
        let tracking_line = format!("remote/{remote_name}/{line_name}");
        return Err(format!(
            "Local line {line_name} at {} and remote {remote_name}/{line_name} at {} have diverged. Imported {imported_snapshots} missing Snapshot(s); no line or workspace was moved. Choose merge: `ait pull {remote_flag} --line {line_name} --merge --restore`; or rebase: `ait line create {tracking_line} --from-snapshot {}` then `ait worktree rebase --onto {tracking_line}`.",
            previous_line_head_snapshot_id.as_deref().unwrap_or("none"),
            head_snapshot_id.as_deref().unwrap_or("none"),
            head_snapshot_id.as_deref().unwrap_or("none"),
        ));
    }

    if relationship == "divergent" {
        let _range = perfetto_range!("ait.remote_sync.pull.merge");
        let remote_head = head_snapshot_id
            .as_deref()
            .ok_or_else(|| "A divergent pull merge requires a remote head Snapshot.".to_string())?;
        let source_label = format!("{remote_name}/{line_name}");
        let mut merge_payload = super::super::line_merge::start_line_merge_from_snapshot_unlocked(
            repo,
            &source_label,
            remote_head,
            None,
            Some(line_name),
            Some(&format!("Merge remote {source_label} into {line_name}")),
        )?;
        let merge_status =
            string_field(&merge_payload, "status").unwrap_or_else(|| "merged".to_string());
        let merge_obj = merge_payload
            .as_object_mut()
            .ok_or_else(|| "line merge payload is malformed".to_string())?;
        merge_obj.insert(
            "remote".to_string(),
            JsonValue::String(remote_name.to_string()),
        );
        merge_obj.insert(
            "repo_name".to_string(),
            JsonValue::String(repo_name.to_string()),
        );
        merge_obj.insert("mode".to_string(), JsonValue::String("line".to_string()));
        merge_obj.insert("line".to_string(), JsonValue::String(line_name.to_string()));
        merge_obj.insert(
            "relationship".to_string(),
            JsonValue::String("divergent".to_string()),
        );
        merge_obj.insert(
            "action".to_string(),
            JsonValue::String(if merge_status == "conflicted" {
                "merge_conflicted".to_string()
            } else {
                "merged".to_string()
            }),
        );
        merge_obj.insert(
            "local_line_present".to_string(),
            JsonValue::Bool(local_line_present),
        );
        merge_obj.insert(
            "local_line_head_snapshot_id".to_string(),
            previous_line_head_snapshot_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        );
        merge_obj.insert(
            "head_snapshot_id".to_string(),
            head_snapshot_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        );
        merge_obj.insert(
            "imported_snapshots".to_string(),
            JsonValue::from(imported_snapshots),
        );
        merge_obj.insert(
            "imported_snapshot_ids".to_string(),
            JsonValue::Array(
                imported_snapshot_ids
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
        merge_obj.insert(
            "line_head_updated".to_string(),
            JsonValue::Bool(merge_status != "conflicted"),
        );
        merge_obj.insert("workspace_restored".to_string(), JsonValue::Bool(true));
        merge_obj.insert("restore_applied".to_string(), JsonValue::Bool(true));
        merge_obj.insert("remote_sync_backend".to_string(), remote_sync_backend);
        merge_obj.insert("phase_timings_ms".to_string(), phase_timings_ms);
        merge_obj.insert("remote_sync_metrics".to_string(), remote_sync_metrics);
        merge_obj.insert("zstd_bulk".to_string(), zstd_bulk);
        return Ok(merge_payload);
    }

    let should_advance = matches!(relationship.as_str(), "remote_ahead" | "new_remote_line");
    let restore_payload = if restore {
        let _range = perfetto_range!("ait.remote_sync.pull.materialization");
        ensure_local_line_can_move(repo, line_name)?;
        Some(restore_pulled_line_workspace(
            repo,
            line_name,
            head_snapshot_id.as_deref(),
            restore_baseline_snapshot_id.as_deref(),
            force,
        )?)
    } else {
        None
    };
    let updated_line = if should_advance {
        let _range = perfetto_range!("ait.remote_sync.pull.head_movement");
        match compare_and_swap_or_create_pulled_line(
            repo,
            line_name,
            local_line_present,
            previous_line_head_snapshot_id.as_deref(),
            head_snapshot_id.as_deref(),
        ) {
            Ok(line) => line,
            Err(error) => {
                if restore_payload.is_some() {
                    let _ = restore_workspace_all(
                        repo,
                        restore_baseline_snapshot_id.as_deref(),
                        head_snapshot_id.as_deref(),
                        true,
                        false,
                    );
                    let _ = set_runtime_current_line(repo, &current_line_before);
                    let _ = repo.set_worktree_materialized_snapshot(
                        restore_baseline_snapshot_id.as_deref(),
                    );
                }
                return Err(error);
            }
        }
    } else {
        remote_sync_local_line_record(repo, line_name)?
            .map(|line| line_record_json(&line))
            .unwrap_or(JsonValue::Null)
    };
    let line_head_updated = should_advance;
    let restore_applied = restore_payload
        .as_ref()
        .and_then(|payload| payload.get("applied"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    Ok(json!({
        "remote": remote_name,
        "repo_name": repo_name,
        "mode": "line",
        "line": line_name,
        "relationship": relationship,
        "action": if line_head_updated { "fast_forward" } else if relationship == "local_ahead" { "imported_only" } else { "none" },
        "remote_line": remote_line,
        "local_line_present": local_line_present,
        "local_line_head_snapshot_id": previous_line_head_snapshot_id,
        "updated_line": updated_line,
        "imported_snapshots": imported_snapshots,
        "imported_snapshot_ids": imported_snapshot_ids,
        "head_snapshot_id": head_snapshot_id,
        "line_head_updated": line_head_updated,
        "workspace_restored": restore_applied,
        "restore_applied": restore_applied,
        "remote_sync_backend": remote_sync_backend,
        "phase_timings_ms": phase_timings_ms,
        "remote_sync_metrics": remote_sync_metrics,
        "zstd_bulk": zstd_bulk,
        "restore": restore_payload.unwrap_or(JsonValue::Null),
    }))
}

fn classify_pull_head_relationship(
    repo: &RepoRuntime,
    local_line_present: bool,
    local_head_snapshot_id: Option<&str>,
    remote_head_snapshot_id: Option<&str>,
) -> Result<String, String> {
    if !local_line_present {
        return Ok("new_remote_line".to_string());
    }
    if local_head_snapshot_id == remote_head_snapshot_id {
        return Ok("equal".to_string());
    }
    match (local_head_snapshot_id, remote_head_snapshot_id) {
        (None, Some(_)) => return Ok("remote_ahead".to_string()),
        (Some(_), None) => return Ok("local_ahead".to_string()),
        (None, None) => return Ok("equal".to_string()),
        (Some(_), Some(_)) => {}
    }
    let local_head_snapshot_id = local_head_snapshot_id.expect("matched Some above");
    let remote_head_snapshot_id = remote_head_snapshot_id.expect("matched Some above");
    let store = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    if snapshot_is_ancestor(
        &store,
        local_head_snapshot_id,
        remote_head_snapshot_id,
        SnapshotDagLimits::default(),
    )?
    .is_some()
    {
        return Ok("remote_ahead".to_string());
    }
    if snapshot_is_ancestor(
        &store,
        remote_head_snapshot_id,
        local_head_snapshot_id,
        SnapshotDagLimits::default(),
    )?
    .is_some()
    {
        return Ok("local_ahead".to_string());
    }
    Ok("divergent".to_string())
}

fn compare_and_swap_or_create_pulled_line(
    repo: &RepoRuntime,
    line_name: &str,
    local_line_present: bool,
    expected_head_snapshot_id: Option<&str>,
    new_head_snapshot_id: Option<&str>,
) -> Result<JsonValue, String> {
    let store = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    let timestamp = system_event_timestamp();
    if local_line_present {
        store
            .compare_and_swap_line_head(
                line_name,
                expected_head_snapshot_id,
                new_head_snapshot_id,
                &timestamp,
            )
            .map(|line| line_record_json(&line))
    } else {
        store
            .create_line(line_name, new_head_snapshot_id, &timestamp)
            .map(|line| line_record_json(&line))
    }
}

pub fn push(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    line_name: Option<&str>,
) -> Result<JsonValue, String> {
    let mut backend = HttpRemoteSyncBackend;
    push_with_remote_sync_backend(&mut backend, repo, remote_name, line_name)
}

pub(in crate::primitives) fn push_with_remote_sync_backend<B>(
    backend: &mut B,
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    line_name: Option<&str>,
) -> Result<JsonValue, String>
where
    B: RemoteSyncBackend + ?Sized,
{
    let (remote_row, repo_name) = backend.remote_context(repo, remote_name)?;
    let resolved_line_name = match normalized_text(line_name) {
        Some(value) => value,
        None => repo.current_line_name()?,
    };
    if resolved_line_name.is_empty() {
        return Err("Current line is not configured; pass --line.".to_string());
    }
    backend.push_line(repo, &remote_row, &repo_name, &resolved_line_name)
}

pub fn upload_snapshot_chain(
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    snapshot_id: &str,
    line_name: Option<&str>,
    reason: Option<&str>,
) -> Result<JsonValue, String> {
    let mut backend = HttpRemoteSyncBackend;
    upload_snapshot_chain_with_remote_sync_backend(
        &mut backend,
        repo,
        remote_name,
        snapshot_id,
        line_name,
        reason,
    )
}

pub(in crate::primitives) fn upload_snapshot_chain_with_remote_sync_backend<B>(
    backend: &mut B,
    repo: &RepoRuntime,
    remote_name: Option<&str>,
    snapshot_id: &str,
    line_name: Option<&str>,
    reason: Option<&str>,
) -> Result<JsonValue, String>
where
    B: RemoteSyncBackend + ?Sized,
{
    let (remote_row, repo_name) = backend.remote_context(repo, remote_name)?;
    backend.upload_snapshot_chain(
        repo,
        &remote_row,
        &repo_name,
        snapshot_id,
        line_name,
        reason,
    )
}

pub(in crate::primitives) fn upload_snapshot_chain_to_remote_with_task_remote_and_capabilities<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    snapshot_id: &str,
    line_name: Option<&str>,
    remote_sync_capabilities: &RemoteSyncCapabilities,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader
        + TaskWorkflowSnapshotExistenceReader
        + TaskWorkflowZstdPackUploader
        + ?Sized,
{
    let remote_head_snapshot_id = match line_name {
        Some(resolved_line_name) => {
            remote_sync_line_head_with_task_remote(task_remote, repo_name, resolved_line_name)?
        }
        None => None,
    };
    let sync_plan =
        remote_sync_snapshot_sync_plan(repo, snapshot_id, remote_head_snapshot_id.as_deref())?;
    let sync_window = &sync_plan.window;
    if sync_window.snapshot_ids.is_empty() {
        return Ok(json!({
            "repo_name": repo_name,
            "checked_snapshots": 0,
            "uploaded_snapshots": 0,
            "skipped_snapshots": 0,
            "sync_scope": sync_window.sync_scope,
            "sync_reason": sync_window.sync_reason,
            "remote_head_snapshot_id": sync_window.remote_head_snapshot_id,
            "bounded_by_snapshot_id": sync_window.bounded_by_snapshot_id,
        }));
    }
    require_snapshot_dag_remote_capability(
        remote_sync_capabilities,
        &sync_plan.multi_parent_snapshot_ids(),
    )?;
    if remote_sync_capabilities.zstd_pack_bulk {
        let zstd_snapshot_ids =
            sync_plan.snapshot_ids_bounded_by(sync_window.bounded_by_snapshot_id.as_deref())?;
        let (zstd_backend_negotiation, _) = require_zstd_bulk_upload_backend(
            &zstd_snapshot_ids,
            &BTreeSet::new(),
            remote_sync_capabilities,
        )?;
        let boundary_present_set = sync_window
            .bounded_by_snapshot_id
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let upload = run_zstd_bulk_upload(
            repo,
            task_remote,
            repo_name,
            &zstd_snapshot_ids,
            &boundary_present_set,
            sync_window.bounded_by_snapshot_id.as_deref(),
            None,
            None,
            None,
        )?;
        let remote_present_set = upload
            .remote_plan
            .present_snapshot_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let zstd_inventory_diff = RemoteSyncInventoryDiff::from_present_snapshot_ids(
            &zstd_snapshot_ids,
            &remote_present_set,
        );
        let remote_plan_payload = ZstdBulkPlanResponseJson::stateless()
            .encode_value(&upload.remote_plan)
            .map_err(|err| err.to_string())?;
        let commit_payload = ZstdBulkCommitResponseJson::stateless()
            .encode_value(&upload.commit_response)
            .map_err(|err| err.to_string())?;
        return Ok(json!({
            "repo_name": repo_name,
            "checked_snapshots": zstd_snapshot_ids.len(),
            "uploaded_snapshots": upload.uploaded_snapshots,
            "skipped_snapshots": upload.skipped_snapshots,
            "sync_scope": sync_window.sync_scope,
            "sync_reason": sync_window.sync_reason,
            "remote_head_snapshot_id": sync_window.remote_head_snapshot_id,
            "bounded_by_snapshot_id": sync_window.bounded_by_snapshot_id,
            "remote_sync_backend": remote_sync_backend_payload(&zstd_backend_negotiation, &zstd_inventory_diff),
            "phase_timings_ms": {
                "zstd_bulk": upload.phase_timings_ms.clone(),
            },
            "remote_sync_metrics": {
                "remote_round_trips": upload.remote_round_trips,
                "transferred_pack_bytes": upload.transferred_pack_bytes,
                "pack_parallelism": upload.pack_parallelism,
            },
            "zstd_bulk": {
                "uploaded_object_packs": upload.uploaded_object_packs,
                "skipped_object_packs": upload.skipped_object_packs,
                "uploaded_tree_packs": upload.uploaded_tree_packs,
                "skipped_tree_packs": upload.skipped_tree_packs,
                "remote_plan": remote_plan_payload,
                "commit": commit_payload,
            },
        }));
    }
    Err(format!(
        "Remote sync requires capability {REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK}."
    ))
}

pub(in crate::primitives) fn push_line_to_remote_with_task_remote_and_capabilities<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    remote_name: &str,
    repo_name: &str,
    line_name: &str,
    remote_sync_capabilities: &RemoteSyncCapabilities,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader
        + TaskWorkflowLineHeadUpdater
        + TaskWorkflowSnapshotExistenceReader
        + TaskWorkflowZstdPackUploader
        + ?Sized,
{
    push_line_to_remote_with_task_remote_and_capabilities_and_zstd_boundary(
        repo,
        task_remote,
        remote_name,
        repo_name,
        line_name,
        remote_sync_capabilities,
        None,
    )
}

fn require_solo_local_default_line_push_authority(
    repo: &RepoRuntime,
    remote_name: &str,
    line_name: &str,
    local_head_snapshot_id: Option<&str>,
    remote_head_snapshot_id: Option<&str>,
) -> Result<(), String> {
    if repo.effective_workflow_mode() != "solo_local"
        || line_name != repo.default_line_name()
        || remote_head_snapshot_id.is_none()
        || remote_head_snapshot_id == local_head_snapshot_id
    {
        return Ok(());
    }
    Err(format!(
        "Refusing to advance initialized remote `{remote_name}` target Line `{line_name}` with \
         `ait push` while `workflow_mode=solo_local` ({remote_head} -> {local_head}). Immutable \
         Snapshot and pack upload remains available to workflow preparation, but only \
         authoritative remote Task Land may move this governed Line. Promote the latest \
         completed local Change with `ait workflow ready <local-change-id> --apply --remote \
         {remote_name}`, then hand it to a reviewer running `ait workflow land \
         <local-change-id> --apply --remote {remote_name}`.",
        remote_head = remote_head_snapshot_id.unwrap_or("none"),
        local_head = local_head_snapshot_id.unwrap_or("none"),
    ))
}

#[expect(
    clippy::too_many_arguments,
    reason = "remote push keeps capability and zstd boundary inputs explicit"
)]
fn push_line_to_remote_with_task_remote_and_capabilities_and_zstd_boundary<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    remote_name: &str,
    repo_name: &str,
    line_name: &str,
    remote_sync_capabilities: &RemoteSyncCapabilities,
    zstd_bulk_boundary_snapshot_id: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader
        + TaskWorkflowLineHeadUpdater
        + TaskWorkflowSnapshotExistenceReader
        + TaskWorkflowZstdPackUploader
        + ?Sized,
{
    let _range = perfetto_range!("ait.workflow_ready.publish.push_line");
    let _push_range = perfetto_range!("ait.remote_sync.push");
    let line_row = {
        let _range = perfetto_range!("ait.remote_sync.push.local_line_read");
        remote_sync_local_line_record(repo, line_name)?
            .ok_or_else(|| format!("Unknown line: {line_name}"))?
    };
    let head_snapshot_id = line_row.head_snapshot_id;
    let expected_remote_line = {
        let _range = perfetto_range!("ait.remote_sync.push.remote_line_read");
        remote_sync_line_head_row_read_with_task_remote(task_remote, repo_name, line_name)?
    };
    let expected_remote_head_snapshot_id = expected_remote_line
        .as_ref()
        .and_then(|line| string_field(line, "head_snapshot_id"));
    require_solo_local_default_line_push_authority(
        repo,
        remote_name,
        line_name,
        head_snapshot_id.as_deref(),
        expected_remote_head_snapshot_id.as_deref(),
    )?;
    let Some(head_snapshot_id) = head_snapshot_id else {
        let remote_line = remote_sync_line_update_with_task_remote(
            task_remote,
            repo_name,
            line_name,
            None,
            expected_remote_head_snapshot_id.as_deref(),
        )?;
        return Ok(json!({
            "remote": remote_name,
            "repo_name": repo_name,
            "line": line_name,
            "pushed_snapshots": 0,
            "checked_snapshots": 0,
            "uploaded_snapshots": 0,
            "skipped_snapshots": 0,
            "head_snapshot_id": JsonValue::Null,
            "remote_line": remote_line,
        }));
    };
    let sync_plan = {
        let _range = perfetto_range!("ait.remote_sync.push.have_want_frontier");
        remote_sync_snapshot_sync_plan(
            repo,
            &head_snapshot_id,
            expected_remote_head_snapshot_id.as_deref(),
        )?
    };
    require_snapshot_dag_remote_capability(
        remote_sync_capabilities,
        &sync_plan.multi_parent_snapshot_ids(),
    )?;
    if expected_remote_head_snapshot_id.as_deref() != Some(head_snapshot_id.as_str()) {
        let head_snapshot_ids = vec![head_snapshot_id.clone()];
        let present_set = {
            let _range = perfetto_range!("ait.remote_sync.push.head_existence_read");
            remote_sync_present_snapshot_ids_with_task_remote(
                task_remote,
                repo_name,
                &head_snapshot_ids,
            )?
        };
        if present_set.contains(&head_snapshot_id) {
            let sync_reason = if expected_remote_head_snapshot_id.is_some() {
                "remote_line_stale_head_snapshot_present"
            } else {
                "remote_line_missing_head_snapshot_present"
            };
            let remote_line = {
                let _range = perfetto_range!("ait.remote_sync.push.line_compare_and_swap");
                remote_sync_line_update_with_task_remote(
                    task_remote,
                    repo_name,
                    line_name,
                    Some(&head_snapshot_id),
                    expected_remote_head_snapshot_id.as_deref(),
                )?
            };
            return Ok(json!({
                "remote": remote_name,
                "repo_name": repo_name,
                "line": line_name,
                "pushed_snapshots": 0,
                "checked_snapshots": 1,
                "uploaded_snapshots": 0,
                "skipped_snapshots": 1,
                "head_snapshot_id": head_snapshot_id,
                "sync_scope": "line_only",
                "sync_reason": sync_reason,
                "remote_head_snapshot_id": expected_remote_head_snapshot_id,
                "bounded_by_snapshot_id": JsonValue::Null,
                "remote_line": remote_line,
            }));
        }
    }
    let sync_window = &sync_plan.window;
    if sync_window.snapshot_ids.is_empty() {
        let remote_line = expected_remote_line.unwrap_or(JsonValue::Null);
        return Ok(json!({
            "remote": remote_name,
            "repo_name": repo_name,
            "line": line_name,
            "pushed_snapshots": 0,
            "checked_snapshots": 0,
            "uploaded_snapshots": 0,
            "skipped_snapshots": 0,
            "head_snapshot_id": head_snapshot_id,
            "sync_scope": sync_window.sync_scope,
            "sync_reason": sync_window.sync_reason,
            "remote_head_snapshot_id": sync_window.remote_head_snapshot_id,
            "bounded_by_snapshot_id": sync_window.bounded_by_snapshot_id,
            "remote_line": remote_line,
        }));
    }
    if remote_sync_capabilities.zstd_pack_bulk {
        let zstd_bulk_boundary_snapshot_id =
            sync_window.bounded_by_snapshot_id.as_deref().or_else(|| {
                zstd_bulk_boundary_snapshot_id
                    .filter(|snapshot_id| sync_plan.contains_snapshot(snapshot_id))
            });
        let zstd_snapshot_ids =
            sync_plan.snapshot_ids_bounded_by(zstd_bulk_boundary_snapshot_id)?;
        let (zstd_backend_negotiation, _) = require_zstd_bulk_upload_backend(
            &zstd_snapshot_ids,
            &BTreeSet::new(),
            remote_sync_capabilities,
        )?;
        let boundary_present_set = zstd_bulk_boundary_snapshot_id
            .map(str::to_string)
            .into_iter()
            .collect::<BTreeSet<_>>();
        let upload = run_zstd_bulk_upload(
            repo,
            task_remote,
            repo_name,
            &zstd_snapshot_ids,
            &boundary_present_set,
            zstd_bulk_boundary_snapshot_id,
            Some(line_name),
            Some(&head_snapshot_id),
            expected_remote_head_snapshot_id.as_deref(),
        )?;
        let remote_present_set = upload
            .remote_plan
            .present_snapshot_ids
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let zstd_inventory_diff = RemoteSyncInventoryDiff::from_present_snapshot_ids(
            &zstd_snapshot_ids,
            &remote_present_set,
        );
        let remote_plan_payload = ZstdBulkPlanResponseJson::stateless()
            .encode_value(&upload.remote_plan)
            .map_err(|err| err.to_string())?;
        let commit_payload = ZstdBulkCommitResponseJson::stateless()
            .encode_value(&upload.commit_response)
            .map_err(|err| err.to_string())?;
        let remote_line = commit_payload
            .get("remote_line")
            .cloned()
            .unwrap_or(JsonValue::Null);
        return Ok(json!({
            "remote": remote_name,
            "repo_name": repo_name,
            "line": line_name,
            "pushed_snapshots": upload.uploaded_snapshots,
            "checked_snapshots": zstd_snapshot_ids.len(),
            "uploaded_snapshots": upload.uploaded_snapshots,
            "skipped_snapshots": upload.skipped_snapshots,
            "head_snapshot_id": head_snapshot_id,
            "sync_scope": sync_window.sync_scope,
            "sync_reason": sync_window.sync_reason,
            "remote_head_snapshot_id": sync_window.remote_head_snapshot_id,
            "bounded_by_snapshot_id": sync_window.bounded_by_snapshot_id,
            "remote_sync_backend": remote_sync_backend_payload(&zstd_backend_negotiation, &zstd_inventory_diff),
            "remote_line": remote_line,
            "phase_timings_ms": {
                "zstd_bulk": upload.phase_timings_ms.clone(),
            },
            "remote_sync_metrics": {
                "remote_round_trips": upload.remote_round_trips,
                "transferred_pack_bytes": upload.transferred_pack_bytes,
                "pack_parallelism": upload.pack_parallelism,
            },
            "zstd_bulk": {
                "uploaded_object_packs": upload.uploaded_object_packs,
                "skipped_object_packs": upload.skipped_object_packs,
                "uploaded_tree_packs": upload.uploaded_tree_packs,
                "skipped_tree_packs": upload.skipped_tree_packs,
                "remote_plan": remote_plan_payload,
                "commit": commit_payload,
            },
        }));
    }
    Err(format!(
        "Remote sync requires capability {REMOTE_SYNC_CAPABILITY_ZSTD_PACK_BULK}."
    ))
}

pub(in crate::primitives) fn sync_patchset_revision_snapshot(
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
    repo_name: &str,
    line_name: &str,
    revision_snapshot_id: &str,
    base_line: &str,
) -> Result<JsonValue, String> {
    let mut backend = HttpRemoteSyncBackend;
    sync_patchset_revision_snapshot_with_remote_sync_backend(
        &mut backend,
        repo,
        remote_row,
        repo_name,
        line_name,
        revision_snapshot_id,
        base_line,
    )
}

pub(in crate::primitives) fn sync_patchset_revision_snapshot_with_remote_sync_backend<B>(
    backend: &mut B,
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
    repo_name: &str,
    line_name: &str,
    revision_snapshot_id: &str,
    base_line: &str,
) -> Result<JsonValue, String>
where
    B: RemoteSyncBackend + ?Sized,
{
    backend.sync_patchset_revision_snapshot(
        repo,
        remote_row,
        repo_name,
        line_name,
        revision_snapshot_id,
        base_line,
    )
}

pub(in crate::primitives) fn sync_patchset_revision_snapshot_with_task_remote<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    remote_name: &str,
    repo_name: &str,
    line_name: &str,
    revision_snapshot_id: &str,
    base_line: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader
        + TaskWorkflowLineHeadUpdater
        + TaskWorkflowRepositoryReader
        + TaskWorkflowSnapshotExistenceReader
        + TaskWorkflowZstdPackUploader
        + ?Sized,
{
    let _range = perfetto_range!("ait.workflow_ready.publish.snapshot_sync.detail");
    let remote_repository = {
        let _range = perfetto_range!("ait.workflow_ready.publish.read_repository_authority");
        read_remote_repository_authority(repo, task_remote, repo_name)?
    };
    let remote_sync_capabilities =
        RemoteSyncCapabilities::from_server_payload(Some(&remote_repository));
    let (line_updated, skipped_reason, mut sync) = if line_name == base_line {
        (
            false,
            Some("current line is the change base line"),
            upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
                repo,
                task_remote,
                repo_name,
                revision_snapshot_id,
                Some(line_name),
                &remote_sync_capabilities,
            )?,
        )
    } else if line_name == repo.default_line_name() {
        (
            false,
            Some("current line is the default integration line"),
            upload_snapshot_chain_to_remote_with_task_remote_and_capabilities(
                repo,
                task_remote,
                repo_name,
                revision_snapshot_id,
                Some(line_name),
                &remote_sync_capabilities,
            )?,
        )
    } else {
        let remote_base_head_snapshot_id = {
            let _range = perfetto_range!("ait.workflow_ready.publish.base_head");
            remote_sync_line_head_with_task_remote(task_remote, repo_name, base_line)?
        };
        let zstd_bulk_boundary_snapshot_id = {
            let _range = perfetto_range!("ait.workflow_ready.publish.bulk_boundary");
            normalized_text(remote_base_head_snapshot_id.as_deref())
        };
        (
            true,
            None,
            push_line_to_remote_with_task_remote_and_capabilities_and_zstd_boundary(
                repo,
                task_remote,
                remote_name,
                repo_name,
                line_name,
                &remote_sync_capabilities,
                zstd_bulk_boundary_snapshot_id.as_deref(),
            )?,
        )
    };
    let payload = sync
        .as_object_mut()
        .ok_or_else(|| "patchset snapshot sync payload is malformed".to_string())?;
    payload.insert("line".to_string(), json!(line_name));
    payload.insert("line_updated".to_string(), json!(line_updated));
    payload.insert(
        "line_update_skipped_reason".to_string(),
        skipped_reason
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert("head_snapshot_id".to_string(), json!(revision_snapshot_id));
    payload.insert("remote_repository".to_string(), remote_repository);
    Ok(sync)
}
