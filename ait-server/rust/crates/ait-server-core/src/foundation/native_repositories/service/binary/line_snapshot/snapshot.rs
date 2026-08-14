use super::*;
use crate::foundation::server_content_binary_db::{
    ServerBinaryTreePackView, ServerBinaryTreeReadCache,
};

impl<D> BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub(in super::super) fn latest_snapshot_value(
        &self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        self.latest_snapshot_value_optional(repo_name, snapshot_id)?
            .ok_or_else(|| {
                NativeRepositoryError::not_found(format!(
                    "Unknown snapshot {snapshot_id} for repository {repo_name}"
                ))
            })
    }

    pub(in super::super) fn latest_snapshot_value_optional(
        &self,
        repo_name: &str,
        snapshot_id: &str,
    ) -> Result<Option<JsonValue>, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        self.latest_snapshot_value_optional_with_read(&read, repo_name, snapshot_id)
    }

    pub(in super::super) fn latest_snapshot_value_optional_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        repo_name: &str,
        snapshot_id: &str,
    ) -> Result<Option<JsonValue>, NativeRepositoryError> {
        self.ensure_repository(repo_name)?;
        let Some((snapshot_index, record)) = self
            .content_snapshots()
            .snapshot_by_id(read, snapshot_id)
            .map_err(binary_native_repository_store_error)?
        else {
            return Ok(None);
        };
        Ok(Some(self.canonical_snapshot_value(
            read,
            repo_name,
            snapshot_index,
            &record,
        )?))
    }

    pub(in super::super) fn upsert_snapshot_value_in_tx<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        repo_name: &str,
        snapshot_id: &str,
        value: &JsonValue,
    ) -> Result<bool, NativeRepositoryError>
    where
        F: BinaryDbFsyncPolicy,
    {
        self.ensure_repository(repo_name)?;
        if self
            .content_snapshots()
            .snapshot_by_id_in_write(tx, snapshot_id)
            .map_err(binary_native_repository_store_error)?
            .is_some()
        {
            return Ok(false);
        }
        let parent_snapshot_index_plus1 = match binary_json_text(value, "parent_snapshot_id") {
            Some(parent_id) => self
                .content_snapshots()
                .snapshot_by_id_in_write(tx, &parent_id)
                .map_err(binary_native_repository_store_error)?
                .map(|(index, _)| index.saturating_add(1))
                .ok_or_else(|| {
                    NativeRepositoryError::bad_request(format!(
                        "Snapshot {snapshot_id} parent {parent_id} is not present in repository {repo_name}"
                    ))
                })?,
            None => 0,
        };
        let line_name = binary_json_text(value, "line_name").unwrap_or_else(default_main_line);
        let line_index_plus1 = self
            .content_lines()
            .line_by_name_in_write(tx, &line_name)
            .map_err(binary_native_repository_store_error)?
            .map(|(index, _)| index.saturating_add(1))
            .unwrap_or(0);
        let root_tree_pack_id = binary_json_text(value, "root_tree_pack_id").ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "Snapshot {snapshot_id} is missing root_tree_pack_id"
            ))
        })?;
        let root_tree_pack = self
            .repository_content()
            .tree_pack_in_write(tx, &root_tree_pack_id)
            .map_err(binary_native_repository_store_error)?
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Snapshot {snapshot_id} root tree pack {root_tree_pack_id} is not present in repository {repo_name}"
                ))
            })?;
        let root_entry_ordinal = value
            .get("root_entry_ordinal")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(0);
        self.repository_content()
            .tree_for_pack_entry_ordinal_in_write(tx, &root_tree_pack, root_entry_ordinal)
            .map_err(binary_native_repository_store_error)?;
        let record = ServerBinarySnapshotRecord {
            snapshot_meta: snapshot_meta_from_value(value)?,
            history_flags: 0,
            payload_len: 0,
            payload_offset: 0,
            snapshot_hash48: server_snapshot_hash48_from_id(snapshot_id)
                .map_err(binary_native_repository_store_error)?,
            parent_snapshot_index_plus1,
            root_tree_pack_index_plus1: root_tree_pack
                .pack_index
                .checked_add(1)
                .ok_or_else(|| NativeRepositoryError::internal("tree pack index overflow"))?,
            root_entry_ordinal,
            line_index_plus1,
            manifest_hash: decode_optional_sha256(binary_json_text(value, "manifest_hash"))?,
            file_count: json_u32(value, "file_count")?,
            total_bytes: json_u64(value, "total_bytes")?,
            created_at_s: binary_json_text(value, "created_at")
                .map(|value| timestamp_s(&value))
                .transpose()?
                .unwrap_or_else(now_timestamp_s),
        };
        let payload = ServerBinarySnapshotPayload {
            line_name,
            message: binary_json_text(value, "message"),
        };
        self.content_snapshots()
            .append_snapshot_in_tx(tx, snapshot_id, record, &payload)
            .map_err(binary_native_repository_store_error)?;
        Ok(true)
    }

    pub(super) fn canonical_snapshot_value(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        repo_name: &str,
        snapshot_index: u32,
        record: &ServerBinarySnapshotRecord,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let parent_indexes = self
            .content_snapshots()
            .snapshot_parent_indexes(read, snapshot_index, record)
            .map_err(binary_native_repository_store_error)?;
        let parent_snapshot_ids = parent_indexes
            .into_iter()
            .map(|parent_index| {
                let raw = read
                    .read_record(
                        ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
                        parent_index,
                    )
                    .map_err(binary_native_repository_store_error)?;
                let parent =
                    ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(
                        &raw,
                    )
                    .map_err(binary_native_repository_store_error)?;
                Ok(server_snapshot_id_from_hash48(parent.snapshot_hash48))
            })
            .collect::<Result<Vec<_>, NativeRepositoryError>>()?;
        self.canonical_snapshot_value_with_parent_snapshot_ids(
            read,
            repo_name,
            snapshot_index,
            record,
            &parent_snapshot_ids,
        )
    }

    pub(super) fn canonical_snapshot_value_with_parent_snapshot_ids(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        repo_name: &str,
        snapshot_index: u32,
        record: &ServerBinarySnapshotRecord,
        parent_snapshot_ids: &[String],
    ) -> Result<JsonValue, NativeRepositoryError> {
        let root_tree_pack_index = canonical_snapshot_root_tree_pack_index(snapshot_index, record)?;
        let root_tree_pack = self
            .repository_content()
            .tree_pack_at_with_read(read, root_tree_pack_index)
            .map_err(binary_native_repository_store_error)?;
        self.repository_content()
            .tree_for_pack_entry_ordinal_with_read(read, &root_tree_pack, record.root_entry_ordinal)
            .map_err(binary_native_repository_store_error)?;
        self.canonical_snapshot_value_with_root_tree_pack(
            read,
            repo_name,
            snapshot_index,
            record,
            parent_snapshot_ids,
            root_tree_pack,
        )
    }

    pub(super) fn canonical_snapshot_value_with_parent_snapshot_ids_and_manifest_cache(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        manifest_cache: &ServerBinaryTreeReadCache,
        repo_name: &str,
        snapshot_index: u32,
        record: &ServerBinarySnapshotRecord,
        parent_snapshot_ids: &[String],
    ) -> Result<JsonValue, NativeRepositoryError> {
        let root_tree_pack_index = canonical_snapshot_root_tree_pack_index(snapshot_index, record)?;
        let root_tree_pack = manifest_cache
            .projected_tree_pack_at(root_tree_pack_index)
            .map_err(binary_native_repository_store_error)?;
        manifest_cache
            .projected_tree_for_pack_entry_ordinal(&root_tree_pack, record.root_entry_ordinal)
            .map_err(binary_native_repository_store_error)?;
        self.canonical_snapshot_value_with_root_tree_pack(
            read,
            repo_name,
            snapshot_index,
            record,
            parent_snapshot_ids,
            root_tree_pack,
        )
    }

    fn canonical_snapshot_value_with_root_tree_pack(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        repo_name: &str,
        snapshot_index: u32,
        record: &ServerBinarySnapshotRecord,
        parent_snapshot_ids: &[String],
        root_tree_pack: ServerBinaryTreePackView,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let snapshot_id = server_snapshot_id_from_hash48(record.snapshot_hash48);
        let payload = self
            .content_snapshots()
            .snapshot_payload(read, record)
            .map_err(binary_native_repository_store_error)?;
        let parent_snapshot_id = parent_snapshot_ids.first().cloned();
        let snapshot_kind = match record.snapshot_meta & ServerBinarySnapshotRecord::META_KIND_MASK
        {
            0 => "line",
            1 => "stash",
            value => {
                return Err(NativeRepositoryError::internal(format!(
                    "Canonical snapshot {snapshot_index} has unsupported kind {value}"
                )))
            }
        };
        Ok(json!({
            "snapshot_id": snapshot_id,
            "repo_name": repo_name,
            "repo_id": self.repo_id(),
            "parent_snapshot_id": parent_snapshot_id,
            "parent_snapshot_ids": parent_snapshot_ids,
            "remote_head_history_boundary": record.is_remote_head_history_boundary(),
            "root_tree_pack_id": root_tree_pack.pack_id,
            "root_entry_ordinal": record.root_entry_ordinal,
            "manifest_hash": manifest_hash_text(&record.manifest_hash),
            "manifest_path": format!("binary-db:zstd-snapshot/{snapshot_id}"),
            "message": payload.message,
            "line_name": payload.line_name,
            "snapshot_kind": snapshot_kind,
            "file_count": record.file_count,
            "total_bytes": record.total_bytes,
            "created_at": timestamp_string(record.created_at_s)?,
            "capabilities": remote_sync_capabilities(),
        }))
    }
}

fn canonical_snapshot_root_tree_pack_index(
    snapshot_index: u32,
    record: &ServerBinarySnapshotRecord,
) -> Result<u32, NativeRepositoryError> {
    record
        .root_tree_pack_index_plus1
        .checked_sub(1)
        .ok_or_else(|| {
            NativeRepositoryError::internal(format!(
                "Canonical snapshot {snapshot_index} has no root tree pack"
            ))
        })
}

fn snapshot_meta_from_value(value: &JsonValue) -> Result<u8, NativeRepositoryError> {
    let kind = match binary_json_text(value, "snapshot_kind").as_deref() {
        None | Some("line") => 0,
        Some("stash") => 1,
        Some(kind) => {
            return Err(NativeRepositoryError::bad_request(format!(
                "unsupported snapshot_kind {kind}"
            )))
        }
    };
    Ok(kind | ServerBinarySnapshotRecord::META_HAS_ROOT_LOCATOR)
}
