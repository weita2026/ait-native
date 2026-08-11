use super::*;

impl<const WRITE_LAYOUT: u32> SnapshotStore for LocalContentBinaryDb<WRITE_LAYOUT> {
    fn snapshot_exists(&self, snapshot_id: &str) -> SnapshotStoreResult<bool> {
        self.snapshots.snapshot_exists(snapshot_id)
    }

    fn snapshot_parent_link(
        &self,
        snapshot_id: &str,
    ) -> SnapshotStoreResult<Option<SnapshotParentLink>> {
        self.snapshots.snapshot_parent_link(snapshot_id)
    }

    fn snapshot_parent_links(
        &self,
        snapshot_ids: &[String],
    ) -> SnapshotStoreResult<Vec<Option<SnapshotParentLink>>> {
        self.snapshots.snapshot_parent_links(snapshot_ids)
    }

    fn snapshot_parent_link_page(
        &self,
        cursor: usize,
        limit: usize,
    ) -> SnapshotStoreResult<SnapshotParentLinkPage> {
        self.snapshots.snapshot_parent_link_page(cursor, limit)
    }

    fn snapshot_by_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<SnapshotRecord>> {
        self.snapshots.snapshot_by_id(snapshot_id)
    }

    fn list_line_snapshots(&self) -> SnapshotStoreResult<Vec<SnapshotRecord>> {
        self.snapshots.list_line_snapshots()
    }

    fn snapshot_total_bytes(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<i64>> {
        self.snapshots.snapshot_total_bytes(snapshot_id)
    }

    fn snapshot_root_tree_pack_id(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        self.snapshots.snapshot_root_tree_pack_id(snapshot_id)
    }

    fn snapshot_kind(&self, snapshot_id: &str) -> SnapshotStoreResult<Option<String>> {
        self.snapshots.snapshot_kind(snapshot_id)
    }

    fn snapshot_chain(&self, snapshot_id: &str) -> SnapshotStoreResult<Vec<String>> {
        self.snapshots.snapshot_chain(snapshot_id)
    }

    fn set_snapshot_kind(
        &self,
        snapshot_id: &str,
        snapshot_kind: &str,
    ) -> SnapshotStoreResult<usize> {
        self.snapshots
            .update_snapshot_kind(snapshot_id, snapshot_kind)
            .map_err(String::from)
    }
}

impl<const WRITE_LAYOUT: u32> LocalSnapshotWriteStore for LocalContentBinaryDb<WRITE_LAYOUT> {
    fn create_snapshot(
        &self,
        _repo_name: &str,
        _line_name: &str,
        _message: Option<&str>,
        _is_worktree: bool,
    ) -> Result<JsonValue, String> {
        Err("LocalContentBinaryDb snapshot create requires the Binary DB snapshot write coordinator".to_string())
    }
}

impl<const WRITE_LAYOUT: u32> LocalSnapshotReadStore for LocalContentBinaryDb<WRITE_LAYOUT> {
    fn get_snapshot(&self, snapshot_id: &str) -> Result<JsonValue, String> {
        let record = self
            .snapshots
            .snapshot_by_id(snapshot_id)?
            .ok_or_else(|| format!("Unknown snapshot: {snapshot_id}"))?;
        snapshot_payload_with_files(&self.snapshots, record)
    }

    fn list_snapshots(&self) -> Result<JsonValue, String> {
        let snapshots = self
            .snapshots
            .list_line_snapshot_records_with_manifest_paths()?
            .into_iter()
            .map(|(record, manifest_path)| {
                snapshot_payload_with_resolved_manifest_path(record, manifest_path)
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(JsonValue::Array(snapshots))
    }

    fn get_line(&self, _line_name: &str) -> Result<JsonValue, String> {
        Err("LocalContentBinaryDb line reads require a Binary DB line store selector".to_string())
    }
}

impl<const WRITE_LAYOUT: u32> LocalSnapshotBlobReadStore for LocalContentBinaryDb<WRITE_LAYOUT> {
    fn read_blob_bytes(&self, blob_id: &str) -> Result<Vec<u8>, String> {
        self.blobs.read_blob_bytes(blob_id)
    }

    fn read_blob_bytes_batch(
        &self,
        blob_ids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, String> {
        crate::object_diff_ports::BlobReader::read_blob_bytes_batch(&self.blobs, blob_ids)
    }
}

impl<const WRITE_LAYOUT: u32> LocalSnapshotTreeReadStore for LocalContentBinaryDb<WRITE_LAYOUT> {
    fn snapshot_tree_root_locator(
        &self,
        snapshot_id: &str,
    ) -> Result<SnapshotTreeRootLocator, String> {
        self.snapshots.snapshot_tree_root_locator(snapshot_id)
    }

    fn snapshot_tree_manifest_path(&self, snapshot_id: &str) -> Result<String, String> {
        self.snapshots.snapshot_tree_manifest_path(snapshot_id)
    }

    fn snapshot_tree_path_delta(
        &self,
        old_snapshot_id: Option<&str>,
        new_snapshot_id: Option<&str>,
    ) -> Result<SnapshotPathDelta, String> {
        self.snapshots
            .snapshot_tree_path_delta(old_snapshot_id, new_snapshot_id)
    }

    fn snapshot_tree_file_rows(
        &self,
        snapshot_id: Option<&str>,
    ) -> Result<Vec<SnapshotFileRow>, String> {
        self.snapshots.snapshot_tree_file_rows(snapshot_id)
    }

    fn snapshot_tree_path_file_rows(
        &self,
        snapshot_id: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, SnapshotFileRow>, String> {
        self.snapshots
            .snapshot_tree_path_file_rows(snapshot_id, paths)
    }

    fn snapshot_tree_path_rows(
        &self,
        snapshot_id: &str,
        paths: &[String],
    ) -> Result<BTreeMap<String, JsonValue>, String> {
        self.snapshots.snapshot_tree_path_rows(snapshot_id, paths)
    }

    fn snapshot_tree_path_rows_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &str,
    ) -> Result<BTreeMap<String, JsonValue>, String> {
        self.snapshots
            .snapshot_tree_path_rows_for_snapshots(snapshot_ids, path)
    }

    fn snapshot_tree_path_blob_rows_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &str,
    ) -> Result<Vec<SnapshotPathBlobRow>, String> {
        self.snapshots
            .snapshot_tree_path_blob_rows_for_snapshots(snapshot_ids, path)
    }

    fn visit_snapshot_tree_path_blobs_reverse(
        &self,
        snapshot_ids: &[String],
        path: &str,
        visitor: &mut dyn FnMut(usize, Option<String>) -> Result<bool, String>,
    ) -> Result<(), String> {
        self.snapshots
            .visit_snapshot_tree_path_blobs_reverse(snapshot_ids, path, visitor)
    }

    fn snapshot_tree_path_row(
        &self,
        snapshot_id: &str,
        path: &str,
    ) -> Result<Option<JsonValue>, String> {
        self.snapshots.snapshot_tree_path_row(snapshot_id, path)
    }
}

fn snapshot_payload_with_files<const WRITE_LAYOUT: u32>(
    snapshots: &BinaryDbSnapshotStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    record: SnapshotRecord,
) -> Result<JsonValue, String> {
    let snapshot_id = record.snapshot_id.clone();
    let mut payload = snapshot_payload_with_manifest_path(snapshots, record)?
        .as_object()
        .cloned()
        .ok_or_else(|| "snapshot payload must be an object".to_string())?;
    let files = snapshots
        .snapshot_tree_file_rows(Some(&snapshot_id))?
        .into_iter()
        .map(|row| SnapshotJson::stateless().snapshot_file_row_payload(&row))
        .collect();
    payload.insert("files".to_string(), JsonValue::Array(files));
    Ok(JsonValue::Object(payload))
}

fn snapshot_payload_with_manifest_path<const WRITE_LAYOUT: u32>(
    snapshots: &BinaryDbSnapshotStore<LocalBinaryDbFs, WRITE_LAYOUT>,
    record: SnapshotRecord,
) -> Result<JsonValue, String> {
    let manifest_path = snapshots.snapshot_tree_manifest_path(&record.snapshot_id)?;
    snapshot_payload_with_resolved_manifest_path(record, manifest_path)
}

fn snapshot_payload_with_resolved_manifest_path(
    record: SnapshotRecord,
    manifest_path: String,
) -> Result<JsonValue, String> {
    let mut payload = SnapshotJson::stateless()
        .snapshot_record_payload(&record)
        .as_object()
        .cloned()
        .ok_or_else(|| "snapshot payload must be an object".to_string())?;
    payload.insert(
        "manifest_path".to_string(),
        JsonValue::String(manifest_path),
    );
    Ok(JsonValue::Object(payload))
}
