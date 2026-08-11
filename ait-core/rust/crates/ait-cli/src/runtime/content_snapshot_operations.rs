use super::*;
use ait_core::snapshot_dag::{
    snapshot_ancestor_closure_from_parent_map, SnapshotDagLimits, SnapshotParentMode,
};

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

impl<const WRITE_LAYOUT: u32> SnapshotStore
    for RepoBinaryDbLocalSnapshotOperationStore<WRITE_LAYOUT>
{
    fn snapshot_exists(&self, snapshot_id: &str) -> SnapshotStoreResult<bool> {
        self.content.snapshot_exists(snapshot_id)
    }

    fn snapshot_parent_link(
        &self,
        snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<SnapshotParentLink>> {
        self.content.snapshot_parent_link(snapshot_id)
    }

    fn snapshot_parent_links(
        &self,
        snapshot_ids: &[String],
    ) -> SnapshotStoreResult<Vec<Option<SnapshotParentLink>>> {
        self.content.snapshot_parent_links(snapshot_ids)
    }

    fn snapshot_parent_link_page(
        &self,
        cursor: usize,
        limit: usize,
    ) -> SnapshotStoreResult<SnapshotParentLinkPage> {
        self.content.snapshot_parent_link_page(cursor, limit)
    }

    fn snapshot_by_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<SnapshotRecord>> {
        self.content.snapshot_by_id(snapshot_id)
    }

    fn list_line_snapshots(&self) -> SnapshotStoreResult<Vec<SnapshotRecord>> {
        self.content.list_line_snapshots()
    }

    fn snapshot_total_bytes(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<i64>> {
        self.content.snapshot_total_bytes(snapshot_id)
    }

    fn snapshot_root_tree_pack_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        self.content.snapshot_root_tree_pack_id(snapshot_id)
    }

    fn snapshot_kind(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        self.content.snapshot_kind(snapshot_id)
    }

    fn snapshot_chain(&self, snapshot_id: &str) -> SnapshotStoreResult<Vec<String>> {
        self.content.snapshot_chain(snapshot_id)
    }

    fn set_snapshot_kind(
        &self,
        snapshot_id: &str,
        snapshot_kind: &str,
    ) -> SnapshotStoreResult<usize> {
        self.content.set_snapshot_kind(snapshot_id, snapshot_kind)
    }
}

impl<const WRITE_LAYOUT: u32> LocalSnapshotWriteStore
    for RepoBinaryDbLocalSnapshotOperationStore<WRITE_LAYOUT>
{
    fn create_snapshot(
        &self,
        repo_name: &str,
        line_name: &str,
        message: Option<&str>,
        is_worktree: bool,
    ) -> Result<JsonValue, String> {
        let line = self
            .lines
            .line_by_name(line_name)?
            .ok_or_else(|| format!("Current line does not exist: {line_name}"))?;
        if line.status == "archived" {
            return Err(format!(
                "Current line {line_name} is archived and cannot create snapshots"
            ));
        }
        let mut parent_snapshot_id = line
            .head_snapshot_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if parent_snapshot_id.is_none() && is_worktree {
            if let Some(materialized_snapshot_id) =
                self.worktree_materialized_snapshot_id(line_name)
            {
                if self.content.snapshot_exists(&materialized_snapshot_id)? {
                    let fallback_updated_at = line
                        .updated_at
                        .as_deref()
                        .or(line.created_at.as_deref())
                        .unwrap_or("0");
                    self.lines.set_line_head(
                        line_name,
                        Some(&materialized_snapshot_id),
                        fallback_updated_at,
                    )?;
                    parent_snapshot_id = Some(materialized_snapshot_id);
                }
            }
        }
        if let Some(existing_parent_snapshot_id) = parent_snapshot_id.clone() {
            if !self.content.snapshot_exists(&existing_parent_snapshot_id)? {
                let materialized_snapshot_id = if is_worktree {
                    self.worktree_materialized_snapshot_id(line_name)
                } else {
                    None
                };
                match materialized_snapshot_id {
                    Some(materialized_snapshot_id)
                        if self.content.snapshot_exists(&materialized_snapshot_id)? =>
                    {
                        let fallback_updated_at = line
                            .updated_at
                            .as_deref()
                            .or(line.created_at.as_deref())
                            .unwrap_or("0");
                        self.lines.set_line_head(
                            line_name,
                            Some(&materialized_snapshot_id),
                            fallback_updated_at,
                        )?;
                        parent_snapshot_id = Some(materialized_snapshot_id);
                    }
                    _ => {
                        return Err(format!(
                            "Current line {line_name} points at missing snapshot {existing_parent_snapshot_id}."
                        ));
                    }
                }
            }
        }
        let payload = self.content.create_snapshot_content(
            repo_name,
            line_name,
            parent_snapshot_id.as_deref(),
            message,
            is_worktree,
        )?;
        let snapshot_id = payload
            .get("snapshot_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "Binary DB snapshot create payload is missing snapshot_id".to_string())?
            .to_string();
        let created_at = payload
            .get("created_at")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "Binary DB snapshot create payload is missing created_at".to_string())?;
        self.lines
            .set_line_head(line_name, Some(&snapshot_id), created_at)?;
        Ok(payload)
    }

    fn create_snapshot_with_parents(
        &self,
        repo_name: &str,
        line_name: &str,
        parent_snapshot_ids: &[String],
        message: Option<&str>,
        is_worktree: bool,
    ) -> Result<JsonValue, String> {
        let line = self
            .lines
            .line_by_name(line_name)?
            .ok_or_else(|| format!("Current line does not exist: {line_name}"))?;
        if line.status == "archived" {
            return Err(format!(
                "Current line {line_name} is archived and cannot create snapshots"
            ));
        }
        let expected_head_snapshot_id = parent_snapshot_ids.first().map(String::as_str);
        if line.head_snapshot_id.as_deref() != expected_head_snapshot_id {
            return Err(format!(
                "Line {line_name} compare-and-swap expected primary parent {} but found {} before Snapshot authoring.",
                expected_head_snapshot_id.unwrap_or("none"),
                line.head_snapshot_id.as_deref().unwrap_or("none")
            ));
        }
        for parent_snapshot_id in parent_snapshot_ids {
            if !self.content.snapshot_exists(parent_snapshot_id)? {
                return Err(format!("Snapshot parent is missing: {parent_snapshot_id}"));
            }
        }
        let payload = self.content.create_snapshot_content_with_parents(
            repo_name,
            line_name,
            parent_snapshot_ids,
            message,
            is_worktree,
        )?;
        let snapshot_id = payload
            .get("snapshot_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "Binary DB snapshot create payload is missing snapshot_id".to_string())?
            .to_string();
        let created_at = payload
            .get("created_at")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "Binary DB snapshot create payload is missing created_at".to_string())?;
        self.lines
            .compare_and_swap_line_head(
                line_name,
                expected_head_snapshot_id,
                Some(&snapshot_id),
                created_at,
            )
            .map_err(|error| {
                format!(
                    "Created immutable Snapshot {snapshot_id}, but target line compare-and-swap failed: {error}"
                )
            })?;
        Ok(payload)
    }
}

impl<const WRITE_LAYOUT: u32> LocalSnapshotReadStore
    for RepoBinaryDbLocalSnapshotOperationStore<WRITE_LAYOUT>
{
    fn get_snapshot(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        self.content.get_snapshot(snapshot_id)
    }

    fn list_snapshots(&self) -> Result<JsonValue, String> {
        self.content.list_snapshots()
    }

    fn get_line(&self, line_name: &str) -> Result<JsonValue, String> {
        let line = self
            .lines
            .line_by_name(line_name)?
            .ok_or_else(|| format!("Unknown line: {line_name}"))?;
        Ok(line_record_json(&line))
    }
}

impl<const WRITE_LAYOUT: u32> LocalSnapshotBlobReadStore
    for RepoBinaryDbLocalSnapshotOperationStore<WRITE_LAYOUT>
{
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Vec<u8>, String> {
        self.content.read_blob_bytes(blob_id)
    }

    fn read_blob_bytes_batch(
        &self,
        blob_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, String> {
        self.content.read_blob_bytes_batch(blob_ids)
    }
}

impl<const WRITE_LAYOUT: u32> LocalSnapshotTreeReadStore
    for RepoBinaryDbLocalSnapshotOperationStore<WRITE_LAYOUT>
{
    fn snapshot_tree_root_locator(
        &self,
        snapshot_id: &str,
    ) -> Result<SnapshotTreeRootLocator, String> {
        self.content.snapshot_tree_root_locator(snapshot_id)
    }

    fn snapshot_tree_manifest_path(&self, snapshot_id: &str) -> Result<String, String> {
        self.content.snapshot_tree_manifest_path(snapshot_id)
    }

    fn snapshot_tree_path_delta(
        &self,
        old_snapshot_id: Option<&str>,
        new_snapshot_id: Option<&str>,
    ) -> Result<SnapshotPathDelta, String> {
        self.content
            .snapshot_tree_path_delta(old_snapshot_id, new_snapshot_id)
    }

    fn snapshot_tree_file_rows(
        &self,
        snapshot_id: Option<&str>,
    ) -> Result<Vec<SnapshotFileRow>, String> {
        self.content.snapshot_tree_file_rows(snapshot_id)
    }

    fn snapshot_tree_path_file_rows(
        &self,
        snapshot_id: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, SnapshotFileRow>, String> {
        self.content
            .snapshot_tree_path_file_rows(snapshot_id, paths)
    }

    fn snapshot_tree_path_rows(
        &self,
        snapshot_id: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, JsonValue>, String> {
        self.content.snapshot_tree_path_rows(snapshot_id, paths)
    }

    fn snapshot_tree_path_rows_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &str,
    ) -> Result<BTreeMap<String, JsonValue>, String> {
        self.content
            .snapshot_tree_path_rows_for_snapshots(snapshot_ids, path)
    }

    fn snapshot_tree_path_blob_rows_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &str,
    ) -> Result<Vec<SnapshotPathBlobRow>, String> {
        self.content
            .snapshot_tree_path_blob_rows_for_snapshots(snapshot_ids, path)
    }

    fn visit_snapshot_tree_path_blobs_reverse(
        &self,
        snapshot_ids: &[String],
        path: &str,
        visitor: &mut dyn FnMut(usize, Option<String>) -> Result<bool, String>,
    ) -> Result<(), String> {
        self.content
            .visit_snapshot_tree_path_blobs_reverse(snapshot_ids, path, visitor)
    }

    fn snapshot_tree_path_row(
        &self,
        snapshot_id: &str,
        path: &str,
    ) -> Result<Option<JsonValue>, String> {
        self.content.snapshot_tree_path_row(snapshot_id, path)
    }
}

impl<const WRITE_LAYOUT: u32> RemoteSyncLocalInventorySource
    for RepoRemoteSyncBinaryDbLocalStore<WRITE_LAYOUT>
{
    fn snapshot_inventory_metadata(
        &self,
        _ctx: &RemoteSyncLocalStoreContext,
        snapshot_ids: &[String],
    ) -> Result<RemoteSyncLocalInventoryMetadata, String> {
        let read = self.snapshots.begin_read_txn();
        let mut tree_formats = BTreeSet::new();
        for snapshot_id in snapshot_ids {
            let snapshot = self
                .snapshots
                .get_snapshot_view(&read, snapshot_id)
                .map_err(|err| err.to_string())?
                .ok_or_else(|| format!("Local snapshot {snapshot_id} is missing."))?;
            let root_tree_pack_id = snapshot
                .root_tree_pack_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    format!("Snapshot {snapshot_id} is missing root_tree_pack_id metadata.")
                })?;
            let tree_pack = self
                .tree_packs
                .get_tree_pack_view(&read, root_tree_pack_id)
                .map_err(|err| err.to_string())?
                .ok_or_else(|| {
                    format!("Tree pack {root_tree_pack_id} is missing local pack metadata.")
                })?;
            tree_formats.insert(tree_pack.pack_format);
        }

        // Match the legacy inventory contract: object-pack format negotiation is
        // repo-level and conservative, while tree-pack formats are scoped to the
        // requested snapshot roots.
        let mut object_formats = BTreeSet::new();
        for pack in self
            .object_packs
            .list_object_pack_views(&read)
            .map_err(|err| err.to_string())?
        {
            object_formats.insert(pack.pack_format);
        }
        Ok(RemoteSyncLocalInventoryMetadata {
            object_pack_formats: object_formats,
            tree_pack_formats: tree_formats,
        })
    }
}

impl<const WRITE_LAYOUT: u32> RemoteSyncLocalSnapshotSource
    for RepoRemoteSyncBinaryDbLocalStore<WRITE_LAYOUT>
{
    fn snapshot_parent_rows(
        &self,
        _ctx: &RemoteSyncLocalStoreContext,
    ) -> Result<Vec<RemoteSyncLocalSnapshotParent>, String> {
        let read = self.snapshots.begin_read_txn();
        let mut parent_by_snapshot = BTreeMap::new();
        let mut ordered_views = self
            .snapshots
            .list_snapshot_views(&read)
            .map_err(|err| err.to_string())?;
        for view in &ordered_views {
            parent_by_snapshot.insert(view.snapshot_id.clone(), view.parent_snapshot_ids.clone());
        }

        let mut head_snapshot_ids = Vec::new();
        for line in self.lines.list_lines_with_read(&read)? {
            let Some(head_snapshot_id) = line
                .head_snapshot_id
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            head_snapshot_ids.push(head_snapshot_id);
        }
        let reachable_snapshot_ids = snapshot_ancestor_closure_from_parent_map(
            &parent_by_snapshot,
            &head_snapshot_ids,
            &BTreeSet::new(),
            SnapshotParentMode::AllParents,
            SnapshotDagLimits::default(),
        )?
        .parent_snapshot_ids
        .into_keys()
        .collect::<BTreeSet<_>>();

        ordered_views.sort_by(|left, right| {
            left.created_at_s
                .cmp(&right.created_at_s)
                .then_with(|| left.snapshot_index.cmp(&right.snapshot_index))
        });
        Ok(ordered_views
            .into_iter()
            .filter(|view| reachable_snapshot_ids.contains(&view.snapshot_id))
            .map(|view| RemoteSyncLocalSnapshotParent {
                snapshot_id: view.snapshot_id,
                parent_snapshot_ids: view.parent_snapshot_ids,
                primary_parent_snapshot_id: view.primary_parent_snapshot_id,
                parent_snapshot_id: view.parent_snapshot_id,
            })
            .collect())
    }

    fn snapshot_content_complete(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        snapshot_id: &str,
    ) -> Result<bool, String> {
        let read = self.snapshots.begin_read_txn();
        let Some(snapshot) = self
            .snapshots
            .get_snapshot_view(&read, snapshot_id)
            .map_err(|err| err.to_string())?
        else {
            return Ok(false);
        };
        let graph_stats =
            match binary_snapshot_graph_stats(&read, &self.blobs, &self.trees, &snapshot) {
                Ok(stats) => stats,
                Err(_) => return Ok(false),
            };
        if graph_stats.file_count != snapshot.file_count
            || graph_stats.total_bytes != snapshot.total_bytes
        {
            return Ok(false);
        }
        if snapshot.record.is_remote_head_history_boundary() {
            // A boundary Snapshot is intentionally not an upload source. The
            // graph walk above has already proved every materializable tree
            // and blob and matched the signed file/byte totals; requiring a
            // reconstructed upload pack closure would reject valid sparse
            // local reuse of content already owned by another pack.
            return Ok(true);
        }
        let snapshot_ids = vec![snapshot_id.to_string()];
        let mut object_packs = BTreeMap::new();
        let mut tree_packs = BTreeMap::new();
        let mut tree_pack_order = Vec::new();
        let mut blob_locators = BTreeMap::new();
        let mut tree_locators = BTreeMap::new();
        let content_catalog = match binary_snapshot_content_catalog(
            &read,
            &self.blobs,
            &self.tree_packs,
            &self.trees,
        ) {
            Ok(catalog) => catalog,
            Err(_) => return Ok(false),
        };
        let content_closure = match binary_snapshot_content_closure(
            &read,
            &self.trees,
            &self.snapshots,
            &snapshot_ids,
            content_catalog,
        ) {
            Ok(closure) => closure,
            Err(_) => return Ok(false),
        };
        if binary_collect_snapshot_zstd_tree_packs(
            ctx,
            &read,
            &self.trees,
            &content_closure,
            &mut tree_packs,
            &mut tree_pack_order,
            &mut tree_locators,
        )
        .is_err()
        {
            return Ok(false);
        }
        if binary_collect_snapshot_zstd_object_packs(
            ctx,
            &read,
            &self.blobs,
            &self.object_packs,
            &content_closure,
            &BTreeSet::new(),
            &mut object_packs,
            &mut blob_locators,
        )
        .is_err()
        {
            return Ok(false);
        }
        Ok(true)
    }
}

impl<const WRITE_LAYOUT: u32> RemoteSyncZstdLocalPlanSource
    for RepoRemoteSyncBinaryDbLocalStore<WRITE_LAYOUT>
{
    fn zstd_bulk_local_plan(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        snapshot_ids: &[String],
        present_set: &BTreeSet<String>,
    ) -> Result<ZstdBulkLocalPlan, String> {
        let read = self.snapshots.begin_read_txn();
        let mut snapshot_order = Vec::new();
        let mut snapshots = BTreeMap::new();
        let mut object_packs = BTreeMap::new();
        let mut tree_packs = BTreeMap::new();
        let mut tree_pack_order = Vec::new();
        let mut blob_locators = BTreeMap::new();
        let mut tree_locators = BTreeMap::new();

        for snapshot_id in snapshot_ids {
            let snapshot = self
                .snapshots
                .get_snapshot_view(&read, snapshot_id)
                .map_err(|err| err.to_string())?
                .ok_or_else(|| format!("Local snapshot {snapshot_id} is missing."))?;
            let _root_tree_pack_id = snapshot
                .root_tree_pack_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    format!("Snapshot {snapshot_id} is missing root_tree_pack_id metadata.")
                })?
                .to_string();
            if !present_set.contains(snapshot_id) {
                snapshot_order.push(snapshot_id.clone());
                snapshots.insert(snapshot_id.clone(), binary_snapshot_upload_row(&snapshot)?);
            }
        }

        let content_catalog =
            binary_snapshot_content_catalog(&read, &self.blobs, &self.tree_packs, &self.trees)?;
        let locator_content_closure = binary_snapshot_content_closure(
            &read,
            &self.trees,
            &self.snapshots,
            &snapshot_order,
            Arc::clone(&content_catalog),
        )?;
        let boundary_snapshot_ids = snapshot_ids
            .iter()
            .filter(|snapshot_id| present_set.contains(*snapshot_id))
            .cloned()
            .collect::<Vec<_>>();
        let (content_closure, boundary_blob_ids) = if boundary_snapshot_ids.is_empty() {
            (locator_content_closure.clone(), BTreeSet::new())
        } else {
            let boundary_content_closure = binary_snapshot_content_closure(
                &read,
                &self.trees,
                &self.snapshots,
                &boundary_snapshot_ids,
                content_catalog,
            )?;
            let boundary_blob_ids = boundary_content_closure
                .blob_indices
                .iter()
                .map(|blob_index| {
                    self.blobs
                        .blob_view_at(&read, *blob_index)
                        .map(|blob| blob.blob_id)
                        .map_err(|err| err.to_string())
                })
                .collect::<Result<BTreeSet<_>, _>>()?;
            (
                locator_content_closure.union(&boundary_content_closure)?,
                boundary_blob_ids,
            )
        };

        binary_collect_snapshot_zstd_tree_packs(
            ctx,
            &read,
            &self.trees,
            &content_closure,
            &mut tree_packs,
            &mut tree_pack_order,
            &mut tree_locators,
        )?;
        binary_collect_snapshot_zstd_object_packs(
            ctx,
            &read,
            &self.blobs,
            &self.object_packs,
            &locator_content_closure,
            &boundary_blob_ids,
            &mut object_packs,
            &mut blob_locators,
        )?;

        Ok(ZstdBulkLocalPlan {
            snapshot_order,
            snapshots,
            object_packs,
            tree_packs,
            tree_pack_order,
            blob_locators,
            tree_locators,
        })
    }
}

impl<const WRITE_LAYOUT: u32> RemoteSyncZstdImportSource
    for RepoRemoteSyncBinaryDbLocalStore<WRITE_LAYOUT>
{
    fn zstd_import_download_plan(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        manifest: &ZstdImportManifestPayload,
    ) -> Result<ZstdImportDownloadPlan, String> {
        self.import_store.zstd_import_download_plan(ctx, manifest)
    }

    fn import_zstd_manifest(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        manifest: &ZstdImportManifestPayload,
        history_mode: ZstdImportHistoryMode,
        plan: &ZstdImportDownloadPlan,
        object_pack_bytes: &BTreeMap<String, Vec<u8>>,
        tree_pack_bytes: &BTreeMap<String, Vec<u8>>,
    ) -> Result<ZstdImportApplyResult, String> {
        self.import_store.import_zstd_manifest(
            ctx,
            manifest,
            history_mode,
            plan,
            object_pack_bytes,
            tree_pack_bytes,
        )
    }

    fn stage_zstd_import_pack_batch(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        object_packs: &[ZstdBulkObjectPackRow],
        tree_packs: &[ZstdBulkTreePackRow],
        object_pack_bytes: &BTreeMap<String, Vec<u8>>,
        tree_pack_bytes: &BTreeMap<String, Vec<u8>>,
    ) -> Result<ZstdImportPackStageResult, String> {
        self.import_store.stage_zstd_import_pack_batch(
            ctx,
            object_packs,
            tree_packs,
            object_pack_bytes,
            tree_pack_bytes,
        )
    }

    fn import_zstd_snapshot_rows(
        &self,
        ctx: &RemoteSyncLocalStoreContext,
        snapshots: &[ZstdBulkSnapshotRow],
    ) -> Result<Vec<String>, String> {
        self.import_store.import_zstd_snapshot_rows(ctx, snapshots)
    }
}

pub(super) fn binary_snapshot_upload_row(
    snapshot: &BinarySnapshotView,
) -> Result<JsonValue, String> {
    Ok(json!({
        "snapshot_id": snapshot.snapshot_id.clone(),
        "parent_snapshot_ids": snapshot.parent_snapshot_ids.clone(),
        "primary_parent_snapshot_id": snapshot.primary_parent_snapshot_id.clone(),
        "parent_snapshot_id": snapshot.parent_snapshot_id.clone(),
        "root_tree_pack_id": snapshot.root_tree_pack_id.clone(),
        "root_entry_ordinal": i64::from(snapshot.root_entry_ordinal),
        "manifest_hash": snapshot.manifest_hash.clone(),
        "message": snapshot.payload.message.clone(),
        "line_name": snapshot.payload.line_name.clone(),
        "snapshot_kind": snapshot.snapshot_kind.clone(),
        "file_count": checked_i64(snapshot.file_count, "snapshot file_count")?,
        "total_bytes": checked_i64(snapshot.total_bytes, "snapshot total_bytes")?,
        "created_at": binary_created_at(snapshot.created_at_s)?,
    }))
}

type ZstdBulkLocalPlanPack = ait_core::remote_sync_local_store::ZstdBulkLocalPack;

#[derive(Clone)]
pub(super) struct BinarySnapshotContentClosure {
    tree_indices: Vec<u32>,
    blob_indices: BTreeSet<u32>,
    catalog: Arc<BinarySnapshotContentCatalog>,
}

impl BinarySnapshotContentClosure {
    fn union(&self, other: &Self) -> Result<Self, String> {
        if !Arc::ptr_eq(&self.catalog, &other.catalog) {
            return Err(
                "Cannot merge Binary Snapshot closures from different catalogs.".to_string(),
            );
        }
        let mut tree_indices = self.tree_indices.clone();
        let mut seen = tree_indices.iter().copied().collect::<BTreeSet<_>>();
        tree_indices.extend(
            other
                .tree_indices
                .iter()
                .copied()
                .filter(|tree_index| seen.insert(*tree_index)),
        );
        let mut blob_indices = self.blob_indices.clone();
        blob_indices.extend(other.blob_indices.iter().copied());
        Ok(Self {
            tree_indices,
            blob_indices,
            catalog: Arc::clone(&self.catalog),
        })
    }
}

struct BinarySnapshotContentCatalog {
    pack_by_record_index: BTreeMap<u32, BinaryTreePackView>,
    tree_pack_by_tree_index: BTreeMap<u32, BinaryTreePackView>,
    tree_record_by_tree_index: BTreeMap<u32, BinaryTreeRecord>,
    tree_index_by_tree_id: BTreeMap<String, u32>,
    blob_index_by_blob_id: BTreeMap<String, u32>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct BinarySnapshotGraphStats {
    file_count: u64,
    total_bytes: u64,
}

fn binary_snapshot_graph_stats<const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    blobs: &BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    snapshot: &BinarySnapshotView,
) -> Result<BinarySnapshotGraphStats, String> {
    let root_tree_index = snapshot.root_tree_index.ok_or_else(|| {
        format!(
            "Snapshot {} is missing a resolvable root tree index.",
            snapshot.snapshot_id
        )
    })?;
    let mut tree_pack_cache = BinaryDbTreeReadCache::default();
    let mut visiting = BTreeSet::new();
    let mut memoized = BTreeMap::new();
    binary_tree_graph_stats(
        read,
        blobs,
        trees,
        root_tree_index,
        snapshot.file_count,
        snapshot.total_bytes,
        &mut tree_pack_cache,
        &mut visiting,
        &mut memoized,
    )
}

#[allow(clippy::too_many_arguments)]
fn binary_tree_graph_stats<const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    blobs: &BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    tree_index: u32,
    declared_file_count: u64,
    declared_total_bytes: u64,
    tree_pack_cache: &mut BinaryDbTreeReadCache,
    visiting: &mut BTreeSet<u32>,
    memoized: &mut BTreeMap<u32, BinarySnapshotGraphStats>,
) -> Result<BinarySnapshotGraphStats, String> {
    if let Some(stats) = memoized.get(&tree_index).copied() {
        return Ok(stats);
    }
    if !visiting.insert(tree_index) {
        return Err(format!(
            "Cycle detected while validating Binary DB snapshot tree index {tree_index}."
        ));
    }
    let result = (|| {
        let tree = trees
            .tree_view_at(read, tree_index)
            .map_err(|err| err.to_string())?;
        if tree.record.is_tombstone() {
            return Err(format!(
                "Binary DB snapshot references tombstoned tree index {tree_index}."
            ));
        }
        let mut stats = BinarySnapshotGraphStats::default();
        for entry in trees
            .list_tree_entry_views_with_cache(read, &tree.tree_id, tree_pack_cache)
            .map_err(|err| err.to_string())?
        {
            let contribution = match entry.entry_type.as_str() {
                "blob" => {
                    let blob = blobs
                        .get_blob_view(read, &entry.target_id)
                        .map_err(|err| err.to_string())?
                        .ok_or_else(|| format!("Unknown Binary DB blob: {}", entry.target_id))?;
                    if blob.record.is_tombstone() {
                        return Err(format!(
                            "Binary DB snapshot references tombstoned blob {}.",
                            entry.target_id
                        ));
                    }
                    BinarySnapshotGraphStats {
                        file_count: 1,
                        total_bytes: blob.size_bytes,
                    }
                }
                "tree" => {
                    let child = trees
                        .get_tree_view(read, &entry.target_id)
                        .map_err(|err| err.to_string())?
                        .ok_or_else(|| format!("Unknown Binary DB tree: {}", entry.target_id))?;
                    binary_tree_graph_stats(
                        read,
                        blobs,
                        trees,
                        child.tree_index,
                        declared_file_count,
                        declared_total_bytes,
                        tree_pack_cache,
                        visiting,
                        memoized,
                    )?
                }
                value => {
                    return Err(format!(
                        "Unsupported Binary DB tree entry kind {value} at ordinal {}.",
                        entry.entry_ordinal
                    ));
                }
            };
            stats.file_count = stats
                .file_count
                .checked_add(contribution.file_count)
                .ok_or_else(|| "Binary DB snapshot file count overflow".to_string())?;
            stats.total_bytes = stats
                .total_bytes
                .checked_add(contribution.total_bytes)
                .ok_or_else(|| "Binary DB snapshot total bytes overflow".to_string())?;
            if stats.file_count > declared_file_count || stats.total_bytes > declared_total_bytes {
                return Err(format!(
                    "Binary DB snapshot tree graph exceeds declared file count or total bytes at tree index {tree_index}."
                ));
            }
        }
        Ok(stats)
    })();
    visiting.remove(&tree_index);
    let stats = result?;
    memoized.insert(tree_index, stats);
    Ok(stats)
}

fn binary_snapshot_content_catalog<const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    blobs: &BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    tree_packs: &BinaryDbTreePackStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
) -> Result<Arc<BinarySnapshotContentCatalog>, String> {
    let pack_views = tree_packs
        .list_tree_pack_views(read)
        .map_err(|err| err.to_string())?;
    let pack_by_record_index = pack_views
        .iter()
        .map(|pack| (pack.tree_pack_index, pack.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut tree_pack_by_tree_index = BTreeMap::new();
    let mut tree_record_by_tree_index = BTreeMap::new();
    let mut tree_index_by_tree_id = BTreeMap::new();
    for pack in &pack_views {
        for offset in 0..pack.record.tree_count {
            let tree_index = pack
                .record
                .first_tree_index
                .checked_add(offset)
                .ok_or_else(|| format!("tree pack {} range overflows", pack.pack_id))?;
            if let Some(existing) = tree_pack_by_tree_index.insert(tree_index, pack.clone()) {
                return Err(format!(
                    "Tree packs {} and {} overlap at tree index {tree_index}.",
                    existing.pack_id, pack.pack_id
                ));
            }
            let record = trees
                .read_tree_record(read, tree_index)
                .map_err(|err| err.to_string())?;
            let tree_id = tree_id_from_hash80(&record.tree_hash80);
            if !record.is_tombstone() {
                // Match the Binary DB id-index read contract: later active
                // candidates take precedence, while tombstones do not hide an
                // older active candidate.
                tree_index_by_tree_id.insert(tree_id.to_ascii_lowercase(), tree_index);
            }
            tree_record_by_tree_index.insert(tree_index, record);
        }
    }

    let blob_file = BinaryDbBlobStore::<LocalBinaryDbFs, WRITE_LAYOUT>::blob_file();
    let blob_count = read
        .record_count(blob_file)
        .map_err(|err| err.to_string())?;
    let mut blob_index_by_blob_id = BTreeMap::new();
    for blob_index in 0..blob_count {
        let blob = blobs
            .blob_view_at(read, blob_index)
            .map_err(|err| err.to_string())?;
        if !blob.record.is_tombstone() {
            // Match the Binary DB id-index read contract: later active
            // candidates take precedence, while tombstones do not hide an
            // older active candidate.
            blob_index_by_blob_id.insert(blob.blob_id.to_ascii_lowercase(), blob_index);
        }
    }

    Ok(Arc::new(BinarySnapshotContentCatalog {
        pack_by_record_index,
        tree_pack_by_tree_index,
        tree_record_by_tree_index,
        tree_index_by_tree_id,
        blob_index_by_blob_id,
    }))
}

fn binary_snapshot_content_closure<const WRITE_LAYOUT: u32>(
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    snapshots: &BinaryDbSnapshotStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    snapshot_ids: &[String],
    catalog: Arc<BinarySnapshotContentCatalog>,
) -> Result<BinarySnapshotContentClosure, String> {
    let mut visiting = BTreeSet::new();
    let mut emitted = BTreeSet::new();
    let mut tree_indices = Vec::new();
    let mut blob_indices = BTreeSet::new();
    let mut tree_pack_cache = BinaryDbTreeReadCache::default();
    for snapshot_id in snapshot_ids {
        let snapshot = snapshots
            .get_snapshot_view(read, snapshot_id)
            .map_err(|err| err.to_string())?
            .ok_or_else(|| format!("Local snapshot {snapshot_id} is missing."))?;
        let root_pack_index = snapshot.record.root_tree_pack_index().ok_or_else(|| {
            format!("Snapshot {snapshot_id} is missing root tree-pack metadata for zstd upload.")
        })?;
        let root_pack = catalog
            .pack_by_record_index
            .get(&root_pack_index)
            .ok_or_else(|| {
                format!(
                    "Snapshot {snapshot_id} references missing tree-pack record {root_pack_index}."
                )
            })?;
        if snapshot.root_entry_ordinal >= root_pack.record.tree_count {
            return Err(format!(
                "Snapshot {snapshot_id} root ordinal {} is outside tree pack {} count {}.",
                snapshot.root_entry_ordinal, root_pack.pack_id, root_pack.record.tree_count
            ));
        }
        let root_tree_index = root_pack
            .record
            .first_tree_index
            .checked_add(snapshot.root_entry_ordinal)
            .ok_or_else(|| format!("Snapshot {snapshot_id} root tree index overflows."))?;
        binary_collect_snapshot_content_indices_for_tree(
            trees,
            read,
            root_tree_index,
            &catalog,
            &mut tree_pack_cache,
            &mut visiting,
            &mut emitted,
            &mut tree_indices,
            &mut blob_indices,
        )?;
    }
    binary_expand_snapshot_content_to_tree_pack_closure(
        trees,
        read,
        &catalog,
        &mut tree_indices,
        &mut blob_indices,
    )?;
    Ok(BinarySnapshotContentClosure {
        tree_indices,
        blob_indices,
        catalog,
    })
}

fn binary_expand_snapshot_content_to_tree_pack_closure<const WRITE_LAYOUT: u32>(
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    catalog: &BinarySnapshotContentCatalog,
    tree_indices: &mut Vec<u32>,
    blob_indices: &mut BTreeSet<u32>,
) -> Result<(), String> {
    let mut pack_by_index = BTreeMap::new();
    let mut queued_pack_indices = BTreeSet::new();
    let mut pack_queue = VecDeque::new();
    for tree_index in tree_indices.iter() {
        let pack = catalog
            .tree_pack_by_tree_index
            .get(tree_index)
            .ok_or_else(|| {
                format!("Tree index {tree_index} is missing zstd tree-pack metadata.")
            })?;
        pack_by_index
            .entry(pack.tree_pack_index)
            .or_insert_with(|| pack.clone());
        if queued_pack_indices.insert(pack.tree_pack_index) {
            pack_queue.push_back(pack.tree_pack_index);
        }
    }

    let mut emitted_tree_indices = tree_indices.iter().copied().collect::<BTreeSet<_>>();
    let mut tree_pack_cache = BinaryDbTreeReadCache::default();
    while let Some(pack_index) = pack_queue.pop_front() {
        let pack = pack_by_index
            .get(&pack_index)
            .cloned()
            .ok_or_else(|| format!("Tree-pack record {pack_index} is missing from closure."))?;
        for offset in 0..pack.record.tree_count {
            let tree_index = pack
                .record
                .first_tree_index
                .checked_add(offset)
                .ok_or_else(|| format!("Tree pack {} range overflows.", pack.pack_id))?;
            let tree = catalog
                .tree_record_by_tree_index
                .get(&tree_index)
                .ok_or_else(|| {
                    format!("Tree index {tree_index} is missing from the Binary DB tree catalog.")
                })?;
            if tree.is_tombstone() {
                return Err(format!(
                    "Tree pack {} contains tombstoned tree index {tree_index}.",
                    pack.pack_id
                ));
            }
            if emitted_tree_indices.insert(tree_index) {
                tree_indices.push(tree_index);
            }
            for entry in trees
                .list_tree_entry_views_for_record_in_pack_with_cache(
                    read,
                    tree_index,
                    tree,
                    &pack,
                    &mut tree_pack_cache,
                )
                .map_err(|err| err.to_string())?
            {
                match entry.entry_type.as_str() {
                    "blob" => {
                        let blob_index = catalog
                            .blob_index_by_blob_id
                            .get(&entry.target_id.to_ascii_lowercase())
                            .copied()
                            .ok_or_else(|| {
                                format!("Unknown Binary DB blob: {}", entry.target_id)
                            })?;
                        blob_indices.insert(blob_index);
                    }
                    "tree" => {
                        let child_tree_index = catalog
                            .tree_index_by_tree_id
                            .get(&entry.target_id.to_ascii_lowercase())
                            .copied()
                            .ok_or_else(|| {
                                format!("Unknown Binary DB tree: {}", entry.target_id)
                            })?;
                        let child_pack = catalog
                            .tree_pack_by_tree_index
                            .get(&child_tree_index)
                            .ok_or_else(|| {
                                format!(
                                    "Tree {} is missing zstd tree-pack metadata.",
                                    entry.target_id
                                )
                            })?;
                        pack_by_index
                            .entry(child_pack.tree_pack_index)
                            .or_insert_with(|| child_pack.clone());
                        if queued_pack_indices.insert(child_pack.tree_pack_index) {
                            pack_queue.push_back(child_pack.tree_pack_index);
                        }
                    }
                    value => {
                        return Err(format!(
                            "Unsupported Binary DB tree entry kind {value} at ordinal {}.",
                            entry.entry_ordinal
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

#[expect(
    clippy::too_many_arguments,
    reason = "tree traversal keeps Binary DB readers, caches, and outputs explicit"
)]
fn binary_collect_snapshot_content_indices_for_tree<const WRITE_LAYOUT: u32>(
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    tree_index: u32,
    catalog: &BinarySnapshotContentCatalog,
    tree_pack_cache: &mut BinaryDbTreeReadCache,
    visiting: &mut BTreeSet<u32>,
    emitted: &mut BTreeSet<u32>,
    tree_indices: &mut Vec<u32>,
    blob_indices: &mut BTreeSet<u32>,
) -> Result<(), String> {
    if emitted.contains(&tree_index) {
        return Ok(());
    }
    if !visiting.insert(tree_index) {
        return Err(format!(
            "Cycle detected while collecting Binary DB snapshot tree index {tree_index}."
        ));
    }
    let result = (|| {
        let tree = catalog
            .tree_record_by_tree_index
            .get(&tree_index)
            .ok_or_else(|| {
                format!("Tree index {tree_index} is missing from the Binary DB tree catalog.")
            })?;
        if tree.is_tombstone() {
            return Err(format!(
                "Binary DB snapshot references tombstoned tree index {tree_index}."
            ));
        }
        let tree_pack = catalog
            .tree_pack_by_tree_index
            .get(&tree_index)
            .ok_or_else(|| {
                format!("Tree index {tree_index} is missing zstd tree-pack metadata.")
            })?;
        for entry in trees
            .list_tree_entry_views_for_record_in_pack_with_cache(
                read,
                tree_index,
                tree,
                tree_pack,
                tree_pack_cache,
            )
            .map_err(|err| err.to_string())?
        {
            match entry.entry_type.as_str() {
                "blob" => {
                    let blob_index = catalog
                        .blob_index_by_blob_id
                        .get(&entry.target_id.to_ascii_lowercase())
                        .copied()
                        .ok_or_else(|| format!("Unknown Binary DB blob: {}", entry.target_id))?;
                    blob_indices.insert(blob_index);
                }
                "tree" => {
                    let child_tree_index = catalog
                        .tree_index_by_tree_id
                        .get(&entry.target_id.to_ascii_lowercase())
                        .copied()
                        .ok_or_else(|| format!("Unknown Binary DB tree: {}", entry.target_id))?;
                    binary_collect_snapshot_content_indices_for_tree(
                        trees,
                        read,
                        child_tree_index,
                        catalog,
                        tree_pack_cache,
                        visiting,
                        emitted,
                        tree_indices,
                        blob_indices,
                    )?;
                }
                value => {
                    return Err(format!(
                        "Unsupported Binary DB tree entry kind {value} at ordinal {}.",
                        entry.entry_ordinal
                    ));
                }
            }
        }
        if emitted.insert(tree_index) {
            tree_indices.push(tree_index);
        }
        Ok(())
    })();
    visiting.remove(&tree_index);
    result
}

#[expect(
    clippy::too_many_arguments,
    reason = "pack collection keeps Binary DB context and bounded outputs explicit"
)]
pub(super) fn binary_collect_snapshot_zstd_object_packs<const WRITE_LAYOUT: u32>(
    ctx: &RemoteSyncLocalStoreContext,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    blobs: &BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    object_packs: &BinaryDbObjectPackStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    content_closure: &BinarySnapshotContentClosure,
    boundary_blob_ids: &BTreeSet<String>,
    object_pack_plans: &mut BTreeMap<String, ZstdBulkLocalPlanPack>,
    blob_locators: &mut BTreeMap<String, JsonValue>,
) -> Result<(), String> {
    let mut pack_indices = BTreeSet::new();
    let mut seen_blobs = BTreeSet::new();
    for blob_index in &content_closure.blob_indices {
        let authoritative_blob = blobs
            .blob_view_at(read, *blob_index)
            .map_err(|err| err.to_string())?;
        if authoritative_blob.record.is_tombstone() {
            return Err(format!(
                "Snapshot references tombstoned blob {}.",
                authoritative_blob.blob_id
            ));
        }
        binary_collect_blob_zstd_pack_closure_until_boundary(
            blobs,
            object_packs,
            read,
            authoritative_blob.blob_index,
            boundary_blob_ids,
            &mut seen_blobs,
            &mut pack_indices,
        )?;
    }
    for pack_index in pack_indices {
        binary_collect_zstd_object_pack(
            ctx,
            read,
            blobs,
            object_packs,
            pack_index,
            object_pack_plans,
            blob_locators,
        )?;
    }
    Ok(())
}

pub(super) fn binary_collect_blob_zstd_pack_closure_until_boundary<const WRITE_LAYOUT: u32>(
    blobs: &BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    object_packs: &BinaryDbObjectPackStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    blob_index: u32,
    boundary_blob_ids: &BTreeSet<String>,
    seen_blobs: &mut BTreeSet<u32>,
    pack_indices: &mut BTreeSet<u32>,
) -> Result<(), String> {
    if !seen_blobs.insert(blob_index) {
        return Ok(());
    }
    let blob = blobs
        .blob_view_at(read, blob_index)
        .map_err(|err| err.to_string())?;
    if boundary_blob_ids.contains(&blob.blob_id) {
        return Ok(());
    }
    let member_index = blob.pack_member_index.ok_or_else(|| {
        format!(
            "Blob {} is missing zstd object-pack member metadata.",
            blob.blob_id
        )
    })?;
    let member = object_packs
        .object_pack_member_view_at(read, member_index)
        .map_err(|err| err.to_string())?;
    if member.record.blob_index != blob_index {
        return Err(format!(
            "Blob {} points to object-pack member {} for blob index {}.",
            blob.blob_id, member_index, member.record.blob_index
        ));
    }
    if pack_indices.insert(member.record.pack_index) {
        let pack = object_packs
            .object_pack_view_at(read, member.record.pack_index)
            .map_err(|err| err.to_string())?;
        for offset in 0..pack.record.member_count {
            let member_index = pack
                .record
                .first_member_index
                .checked_add(offset)
                .ok_or_else(|| format!("Object pack {} member range overflows.", pack.pack_id))?;
            let pack_member = object_packs
                .object_pack_member_view_at(read, member_index)
                .map_err(|err| err.to_string())?;
            if pack_member.record.pack_index != member.record.pack_index {
                return Err(format!(
                    "Object pack member {} belongs to {}, not {}.",
                    pack_member.member_index, pack_member.pack_id, pack.pack_id
                ));
            }
            if let Some(base_blob_index) = pack_member.record.base_blob_index() {
                binary_collect_blob_zstd_pack_closure_until_boundary(
                    blobs,
                    object_packs,
                    read,
                    base_blob_index,
                    boundary_blob_ids,
                    seen_blobs,
                    pack_indices,
                )?;
            }
        }
    }
    Ok(())
}

pub(super) fn binary_collect_snapshot_zstd_tree_packs<const WRITE_LAYOUT: u32>(
    ctx: &RemoteSyncLocalStoreContext,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    content_closure: &BinarySnapshotContentClosure,
    tree_pack_plans: &mut BTreeMap<String, ZstdBulkLocalPlanPack>,
    tree_pack_order: &mut Vec<String>,
    tree_locators: &mut BTreeMap<String, JsonValue>,
) -> Result<(), String> {
    let mut pack_discovery_order = Vec::new();
    let mut pack_by_index = BTreeMap::new();
    for tree_index in &content_closure.tree_indices {
        let pack = content_closure
            .catalog
            .tree_pack_by_tree_index
            .get(tree_index)
            .ok_or_else(|| {
                format!("Tree index {tree_index} is missing zstd tree-pack metadata.")
            })?;
        if pack_by_index.contains_key(&pack.tree_pack_index) {
            continue;
        }
        pack_discovery_order.push(pack.tree_pack_index);
        pack_by_index.insert(pack.tree_pack_index, pack.clone());
        binary_collect_zstd_tree_pack(ctx, read, trees, pack, tree_pack_plans, tree_locators)?;
    }

    let mut dependencies_by_pack = pack_discovery_order
        .iter()
        .map(|pack_index| (*pack_index, BTreeSet::new()))
        .collect::<BTreeMap<_, _>>();
    let mut tree_pack_cache = BinaryDbTreeReadCache::default();
    for tree_index in &content_closure.tree_indices {
        let parent_pack = content_closure
            .catalog
            .tree_pack_by_tree_index
            .get(tree_index)
            .ok_or_else(|| {
                format!("Tree index {tree_index} is missing zstd tree-pack metadata.")
            })?;
        let tree = content_closure
            .catalog
            .tree_record_by_tree_index
            .get(tree_index)
            .ok_or_else(|| {
                format!("Tree index {tree_index} is missing from the Binary DB tree catalog.")
            })?;
        for entry in trees
            .list_tree_entry_views_for_record_in_pack_with_cache(
                read,
                *tree_index,
                tree,
                parent_pack,
                &mut tree_pack_cache,
            )
            .map_err(|err| err.to_string())?
        {
            if entry.entry_type != "tree" {
                continue;
            }
            let child_tree_index = content_closure
                .catalog
                .tree_index_by_tree_id
                .get(&entry.target_id.to_ascii_lowercase())
                .copied()
                .ok_or_else(|| format!("Unknown Binary DB tree: {}", entry.target_id))?;
            let child_pack = content_closure
                .catalog
                .tree_pack_by_tree_index
                .get(&child_tree_index)
                .ok_or_else(|| {
                    format!(
                        "Tree {} is missing zstd tree-pack metadata.",
                        entry.target_id
                    )
                })?;
            if !pack_by_index.contains_key(&child_pack.tree_pack_index) {
                return Err(format!(
                    "Tree-pack closure is missing dependency {} required by {}.",
                    child_pack.pack_id, parent_pack.pack_id
                ));
            }
            if child_pack.tree_pack_index != parent_pack.tree_pack_index {
                dependencies_by_pack
                    .entry(parent_pack.tree_pack_index)
                    .or_default()
                    .insert(child_pack.tree_pack_index);
            }
        }
    }

    for pack_index in
        binary_dependency_ordered_tree_pack_indices(&pack_discovery_order, &dependencies_by_pack)?
    {
        let pack = pack_by_index
            .get(&pack_index)
            .ok_or_else(|| format!("Ordered tree-pack record {pack_index} is missing."))?;
        tree_pack_order.push(pack.pack_id.clone());
    }
    Ok(())
}

fn binary_dependency_ordered_tree_pack_indices(
    pack_discovery_order: &[u32],
    dependencies_by_pack: &BTreeMap<u32, BTreeSet<u32>>,
) -> Result<Vec<u32>, String> {
    let rank_by_pack = pack_discovery_order
        .iter()
        .enumerate()
        .map(|(rank, pack_index)| (*pack_index, rank))
        .collect::<BTreeMap<_, _>>();
    let mut remaining_dependency_count = pack_discovery_order
        .iter()
        .map(|pack_index| {
            (
                *pack_index,
                dependencies_by_pack
                    .get(pack_index)
                    .map_or(0, BTreeSet::len),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut dependents_by_pack = BTreeMap::<u32, BTreeSet<u32>>::new();
    for (pack_index, dependencies) in dependencies_by_pack {
        if !rank_by_pack.contains_key(pack_index) {
            return Err(format!(
                "Tree-pack dependency graph contains unknown pack record {pack_index}."
            ));
        }
        for dependency in dependencies {
            if !rank_by_pack.contains_key(dependency) {
                return Err(format!(
                    "Tree-pack record {pack_index} depends on unknown pack record {dependency}."
                ));
            }
            dependents_by_pack
                .entry(*dependency)
                .or_default()
                .insert(*pack_index);
        }
    }

    let mut ready = remaining_dependency_count
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(pack_index, _)| (rank_by_pack[pack_index], *pack_index))
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(pack_discovery_order.len());
    while let Some((_, pack_index)) = ready.pop_first() {
        ordered.push(pack_index);
        if let Some(dependents) = dependents_by_pack.get(&pack_index) {
            for dependent in dependents {
                let remaining = remaining_dependency_count
                    .get_mut(dependent)
                    .ok_or_else(|| {
                        format!("Tree-pack dependency record {dependent} is missing.")
                    })?;
                *remaining = remaining.checked_sub(1).ok_or_else(|| {
                    format!("Tree-pack dependency count underflow for record {dependent}.")
                })?;
                if *remaining == 0 {
                    ready.insert((rank_by_pack[dependent], *dependent));
                }
            }
        }
    }
    if ordered.len() != pack_discovery_order.len() {
        let cyclic = pack_discovery_order
            .iter()
            .filter(|pack_index| !ordered.contains(pack_index))
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Tree-pack dependency cycle prevents ordered remote commit: {cyclic}."
        ));
    }
    Ok(ordered)
}

pub(super) fn binary_collect_zstd_object_pack<const WRITE_LAYOUT: u32>(
    ctx: &RemoteSyncLocalStoreContext,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    blobs: &BinaryDbBlobStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    object_packs: &BinaryDbObjectPackStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    pack_record_index: u32,
    object_pack_plans: &mut BTreeMap<String, ZstdBulkLocalPlanPack>,
    blob_locators: &mut BTreeMap<String, JsonValue>,
) -> Result<(), String> {
    let pack = object_packs
        .object_pack_view_at(read, pack_record_index)
        .map_err(|err| err.to_string())?;
    let pack_id = pack.pack_id.as_str();
    if object_pack_plans.contains_key(pack_id) {
        return Ok(());
    }
    if !pack.record.is_ready() {
        return Err(format!("Object pack {pack_id} is not ready: pending"));
    }
    validate_binary_zstd_object_pack_format(pack_id, &pack.pack_format)?;
    let pack_abs_path = repo_stored_path_runtime(ctx, &pack.pack_path);
    let pack_path = pack_abs_path.to_string_lossy();
    let pack_archive_index = read_pack_index_with_format(pack_path.as_ref(), &pack.pack_format)
        .map_err(|err| format!("Object pack {pack_id} failed zstd validation: {err}"))?;
    if pack_archive_index
        .get("pack_id")
        .and_then(JsonValue::as_str)
        != Some(pack_id)
    {
        return Err(format!("Object pack {pack_id} index pack_id mismatch."));
    }
    if pack_archive_index
        .get("pack_format")
        .and_then(JsonValue::as_str)
        != Some(PACK_FORMAT_ZSTD_CHUNKED_V1)
    {
        return Err(format!("Object pack {pack_id} index pack_format mismatch."));
    }

    for offset in 0..pack.record.member_count {
        let member_index = pack
            .record
            .first_member_index
            .checked_add(offset)
            .ok_or_else(|| format!("Object pack {pack_id} member range overflows."))?;
        let member = object_packs
            .object_pack_member_view_at(read, member_index)
            .map_err(|err| err.to_string())?;
        if member.pack_id != pack_id {
            return Err(format!(
                "Object pack member {} belongs to {}, not {pack_id}.",
                member.member_index, member.pack_id
            ));
        }
        if blob_locators.contains_key(&member.blob_id) {
            continue;
        }
        let blob = blobs
            .blob_view_at(read, member.record.blob_index)
            .map_err(|err| err.to_string())?;
        let pack_entry_type = match member.record.member_kind() {
            BinaryObjectPackMemberKind::Full => "full".to_string(),
            BinaryObjectPackMemberKind::Delta => "delta".to_string(),
            BinaryObjectPackMemberKind::Reserved(value) => format!("reserved:{value}"),
        };
        blob_locators.insert(
            member.blob_id.clone(),
            json!({
                "blob_id": member.blob_id,
                "sha256": blob.sha256,
                "storage_path": format!(".ait/objects/packs/{}.packref", blob.blob_id),
                "size_bytes": checked_i64(blob.size_bytes, "blob size_bytes")?,
                "storage_kind": "pack_full",
                "pack_id": pack_id,
                "pack_entry_name": member.entry_name,
                "pack_entry_type": pack_entry_type,
                "pack_base_blob_id": member.base_blob_id,
                "pack_chain_depth": i64::from(member.record.delta_chain_depth),
                "created_at": binary_created_at(if blob.record.created_at_s != 0 {
                    blob.record.created_at_s
                } else {
                    pack.record.created_at_s
                })?,
            }),
        );
    }

    let pack_index_entry_name = pack_archive_index
        .get("index_entry_name")
        .and_then(JsonValue::as_str)
        .unwrap_or("zstd-chunked-object-index")
        .to_string();
    let pack_index_checksum =
        pack_index_checksum_with_format(pack_path.as_ref(), &pack.pack_format)?
            .ok_or_else(|| format!("Object pack {pack_id} is missing zstd index checksum."))?;
    let metadata = json!({
        "pack_id": pack_id,
        "status": "ready",
        "member_count": i64::from(pack.record.member_count),
        "total_bytes": checked_i64(pack.record.total_bytes, "object pack total_bytes")?,
        "pack_path": pack.pack_path,
        "pack_format": pack.pack_format,
        "pack_index_entry_name": pack_index_entry_name,
        "pack_index_checksum": pack_index_checksum,
        "created_at": binary_created_at(pack.record.created_at_s)?,
        "pack_index": pack_archive_index,
    });
    object_pack_plans.insert(
        pack_id.to_string(),
        ZstdBulkLocalPlanPack {
            pack_id: pack_id.to_string(),
            pack_abs_path,
            metadata,
        },
    );
    Ok(())
}

pub(super) fn binary_collect_zstd_tree_pack<const WRITE_LAYOUT: u32>(
    ctx: &RemoteSyncLocalStoreContext,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    pack: &BinaryTreePackView,
    tree_pack_plans: &mut BTreeMap<String, ZstdBulkLocalPlanPack>,
    tree_locators: &mut BTreeMap<String, JsonValue>,
) -> Result<(), String> {
    let pack_id = pack.pack_id.as_str();
    if tree_pack_plans.contains_key(pack_id) {
        return Ok(());
    }
    if !pack.record.is_ready() {
        return Err(format!("Tree pack {pack_id} is not ready: pending"));
    }
    validate_binary_zstd_tree_pack_format(pack_id, &pack.pack_format)?;
    let pack_abs_path = repo_stored_path_runtime(ctx, &pack.pack_path);
    let pack_path = pack_abs_path.to_string_lossy();
    let pack_index = read_tree_pack_index_with_format(pack_path.as_ref(), &pack.pack_format)
        .map_err(|err| format!("Tree pack {pack_id} failed zstd validation: {err}"))?;
    if pack_index.get("pack_id").and_then(JsonValue::as_str) != Some(pack_id) {
        return Err(format!("Tree pack {pack_id} index pack_id mismatch."));
    }
    if pack_index.get("pack_format").and_then(JsonValue::as_str)
        != Some(TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
    {
        return Err(format!("Tree pack {pack_id} index pack_format mismatch."));
    }
    let physical_tree_count = pack_index
        .get("tree_count")
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| format!("Tree pack {pack_id} index is missing tree_count."))?;
    let logical_tree_count = i64::from(pack.record.tree_count);
    if physical_tree_count < logical_tree_count {
        return Err(format!(
            "Tree pack {pack_id} physical tree_count {physical_tree_count} is smaller than logical tree_count {logical_tree_count}."
        ));
    }
    if !pack.record.has_sparse_physical_ordinals() && physical_tree_count != logical_tree_count {
        return Err(format!(
            "Tree pack {pack_id} dense tree_count {logical_tree_count} does not match physical tree_count {physical_tree_count}."
        ));
    }
    let index_checksums = tree_pack_index_checksums_by_tree_id(pack_id, &pack_index)?;

    for tree in binary_tree_pack_trees(trees, read, pack)? {
        if tree.tree_pack_id.as_deref() != Some(pack_id) {
            return Err(format!(
                "Tree {} belongs to tree pack {:?}, not zstd tree pack {pack_id}.",
                tree.tree_id, tree.tree_pack_id
            ));
        }
        let checksum = index_checksums.get(&tree.tree_id).cloned().ok_or_else(|| {
            format!(
                "Tree pack {pack_id} index is missing checksum for tree {}.",
                tree.tree_id
            )
        })?;
        tree_locators.insert(
            tree.tree_id.clone(),
            json!({
                "tree_id": tree.tree_id,
                "entry_count": i64::from(tree.record.entry_count),
                "tree_pack_id": pack_id,
                "tree_pack_checksum": checksum,
                "created_at": binary_created_at(pack.record.created_at_s)?,
            }),
        );
    }

    let pack_index_entry_name = pack_index
        .get("index_entry_name")
        .and_then(JsonValue::as_str)
        .unwrap_or("zstd-chunked-tree-index")
        .to_string();
    let pack_index_checksum =
        tree_pack_index_checksum_with_format(pack_path.as_ref(), &pack.pack_format)?
            .ok_or_else(|| format!("Tree pack {pack_id} is missing zstd index checksum."))?;
    let metadata = json!({
        "pack_id": pack_id,
        "status": "ready",
        "tree_count": physical_tree_count,
        "total_bytes": checked_i64(pack.record.total_bytes, "tree pack total_bytes")?,
        "pack_path": pack.pack_path,
        "pack_format": pack.pack_format,
        "pack_index_entry_name": pack_index_entry_name,
        "pack_index_checksum": pack_index_checksum,
        "created_at": binary_created_at(pack.record.created_at_s)?,
        "pack_index": pack_index,
    });
    tree_pack_plans.insert(
        pack_id.to_string(),
        ZstdBulkLocalPlanPack {
            pack_id: pack_id.to_string(),
            pack_abs_path,
            metadata,
        },
    );
    Ok(())
}

pub(super) fn binary_tree_pack_trees<const WRITE_LAYOUT: u32>(
    trees: &BinaryDbTreeStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    read: &BinaryDbReadTxn<'_, LocalBinaryDbFs>,
    pack: &BinaryTreePackView,
) -> Result<Vec<BinaryTreeView>, String> {
    let mut result = Vec::new();
    for offset in 0..pack.record.tree_count {
        let tree_index = pack
            .record
            .first_tree_index
            .checked_add(offset)
            .ok_or_else(|| "tree-pack tree index overflow".to_string())?;
        let record = trees
            .read_tree_record(read, tree_index)
            .map_err(|err| err.to_string())?;
        if record.is_tombstone() {
            return Err(format!(
                "Tree pack {} references tombstoned tree index {tree_index}.",
                pack.pack_id
            ));
        }
        result.push(BinaryTreeView {
            tree_index,
            tree_id: tree_id_from_hash80(&record.tree_hash80),
            tree_pack_id: Some(pack.pack_id.clone()),
            record,
        });
    }
    Ok(result)
}

pub(super) fn tree_pack_index_checksums_by_tree_id(
    pack_id: &str,
    pack_index: &JsonValue,
) -> Result<BTreeMap<String, String>, String> {
    let mut checksums = BTreeMap::new();
    let trees = pack_index
        .get("trees")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("Tree pack {pack_id} index is missing trees."))?;
    for tree in trees {
        let tree_id = tree
            .get("tree_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| format!("Tree pack {pack_id} index has a tree without tree_id."))?;
        let checksum = tree
            .get("checksum")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                format!("Tree pack {pack_id} index tree {tree_id} is missing checksum.")
            })?;
        checksums.insert(tree_id.to_string(), checksum.to_string());
    }
    Ok(checksums)
}

pub(super) fn validate_binary_zstd_object_pack_format(
    pack_id: &str,
    pack_format: &str,
) -> Result<(), String> {
    if pack_format == PACK_FORMAT_ZSTD_CHUNKED_V1 {
        Ok(())
    } else {
        Err(format!(
            "Zstd bulk upload requires zstd object pack {pack_id}; found pack_format {pack_format:?}."
        ))
    }
}

pub(super) fn validate_binary_zstd_tree_pack_format(
    pack_id: &str,
    pack_format: &str,
) -> Result<(), String> {
    if pack_format == TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
        Ok(())
    } else {
        Err(format!(
            "Zstd bulk upload requires zstd tree pack {pack_id}; found pack_format {pack_format:?}."
        ))
    }
}

pub(super) fn repo_stored_path_runtime(
    ctx: &RemoteSyncLocalStoreContext,
    stored_path: &str,
) -> PathBuf {
    let path = PathBuf::from(stored_path);
    if path.is_absolute() {
        path
    } else {
        ctx.repo_root().join(path)
    }
}

pub(super) fn binary_created_at(created_at_s: u64) -> Result<String, String> {
    let created_at_s = i64::try_from(created_at_s).map_err(|_| {
        format!("Binary DB epoch seconds exceed the RFC 3339 projection boundary: {created_at_s}")
    })?;
    chrono::DateTime::<chrono::Utc>::from_timestamp(created_at_s, 0)
        .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .ok_or_else(|| {
            format!("Binary DB epoch seconds cannot be rendered as RFC 3339: {created_at_s}")
        })
}

pub(super) fn checked_i64(value: u64, field: &str) -> Result<i64, String> {
    i64::try_from(value).map_err(|_| format!("{field} overflows i64: {value}"))
}

impl<const WRITE_LAYOUT: u32> LineStore for RepoBinaryDbLocalSnapshotOperationStore<WRITE_LAYOUT> {
    fn list_lines(&self) -> Result<Vec<LineRecord>, String> {
        self.lines.list_lines()
    }

    fn line_count(&self) -> Result<usize, String> {
        self.lines.line_count()
    }

    fn line_by_name(&self, line_name: &str) -> Result<Option<LineRecord>, String> {
        self.lines.line_by_name(line_name)
    }

    fn create_line(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        created_at: &str,
    ) -> Result<LineRecord, String> {
        self.lines
            .create_line(line_name, head_snapshot_id, created_at)
    }

    fn archive_line(&self, line_name: &str, archived_at: &str) -> Result<LineRecord, String> {
        self.lines.archive_line(line_name, archived_at)
    }

    fn rename_line(
        &self,
        old_line_name: &str,
        new_line_name: &str,
        updated_at: &str,
    ) -> Result<LineRecord, String> {
        self.lines
            .rename_line(old_line_name, new_line_name, updated_at)
    }

    fn delete_line(&self, line_name: &str, deleted_at: &str) -> Result<LineRecord, String> {
        self.lines.delete_line(line_name, deleted_at)
    }

    fn set_line_head(
        &self,
        line_name: &str,
        head_snapshot_id: Option<&str>,
        updated_at: &str,
    ) -> Result<LineRecord, String> {
        self.lines
            .set_line_head(line_name, head_snapshot_id, updated_at)
    }

    fn line_updated_at(&self, line_name: &str) -> Result<Option<String>, String> {
        self.lines.line_updated_at(line_name)
    }

    fn set_line_updated_at(&self, line_name: &str, updated_at: Option<&str>) -> Result<(), String> {
        self.lines.set_line_updated_at(line_name, updated_at)
    }

    fn touch_line_updated_at(&self, line_name: &str, updated_at: &str) -> Result<(), String> {
        self.lines.touch_line_updated_at(line_name, updated_at)
    }
}
