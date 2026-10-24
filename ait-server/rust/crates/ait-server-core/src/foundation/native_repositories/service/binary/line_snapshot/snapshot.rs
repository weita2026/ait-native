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
        let scope = if self.snapshot_capture_authority.is_some() {
            BinaryDbReadScope::CONTENT.union(BinaryDbReadScope::WORKFLOW)
        } else {
            BinaryDbReadScope::CONTENT
        };
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, scope);
        let mut value =
            self.latest_snapshot_value_optional_with_read(&read, repo_name, snapshot_id)?;
        if let Some(value) = value
            .as_mut()
            .filter(|_| self.snapshot_capture_authority.is_some())
        {
            let (index, _) = self
                .content_snapshots()
                .snapshot_by_id(&read, snapshot_id)
                .map_err(binary_native_repository_store_error)?
                .ok_or_else(|| NativeRepositoryError::not_found("Capture Snapshot disappeared"))?;
            value["capture_binding"] = self
                .snapshot_capture_workflow_store()?
                .capture_binding_with_read(&read, index)
                .map_err(NativeRepositoryError::bad_request)?
                .unwrap_or(JsonValue::Null);
        }
        Ok(value)
    }

    pub(in super::super) fn snapshot_capture_workflow_store(
        &self,
    ) -> Result<
        crate::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store<D>,
        NativeRepositoryError,
    > {
        let (index, namespace) = self.snapshot_capture_authority.as_ref().ok_or_else(|| {
            NativeRepositoryError::bad_request("Numeric Snapshot capture authority is unavailable")
        })?;
        crate::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store::new_remote_frozen(self.db.clone(), namespace)
            .map(|store| store.with_repository_index(*index)).map_err(NativeRepositoryError::bad_request)
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
        actor_index_plus1: u32,
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
        let parent_ids = snapshot_parent_ids(value)?;
        let mut parent_indexes = Vec::new();
        for parent_id in parent_ids {
            let index=self
                .content_snapshots()
                .snapshot_by_id_in_write(tx, &parent_id)
                .map_err(binary_native_repository_store_error)?
                .map(|(index, _)| index)
                .ok_or_else(|| {
                    NativeRepositoryError::bad_request(format!(
                        "Snapshot {snapshot_id} parent {parent_id} is not present in repository {repo_name}"
                    ))
                })?;
            parent_indexes.push(index);
        }
        let parent_snapshot_index_plus1 = parent_indexes.first().map_or(0, |index| index + 1);
        let line_name = binary_json_text(value, "line_name").unwrap_or_else(default_main_line);
        let line_index_plus1 = if line_name.starts_with("import/") {
            0
        } else {
            self.content_lines()
                .line_by_name_in_write(tx, &line_name)
                .map_err(binary_native_repository_store_error)?
                .map(|(index, _)| index.saturating_add(1))
                .unwrap_or(0)
        };
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
            actor_index_plus1,
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
        let snapshot_index = self
            .content_snapshots()
            .append_snapshot_in_tx(tx, snapshot_id, record, &payload)
            .map_err(binary_native_repository_store_error)?;
        if parent_indexes.len() > 1 {
            use crate::foundation::server_content_binary_db::{
                ServerBinarySnapshotParentEdgeCodec, ServerBinarySnapshotParentEdgeRecord,
            };
            // The existing Snapshot append writes parent ordinal zero.
            for (ordinal, parent) in parent_indexes.into_iter().enumerate().skip(1) {
                let record = ServerBinarySnapshotParentEdgeRecord {
                    child_snapshot_index: snapshot_index,
                    parent_snapshot_index: parent,
                    parent_ordinal: ordinal as u16,
                    flags: 0,
                };
                tx.append_record(
                    ServerBinarySnapshotParentEdgeCodec::<1>::record_file(),
                    &ServerBinarySnapshotParentEdgeCodec::<1>::encode_record(record)
                        .map_err(binary_native_repository_store_error)?,
                )
                .map_err(binary_native_repository_store_error)?;
            }
        }
        Ok(true)
    }

    /// Preparation may race another content writer. Recheck every declared
    /// Snapshot under the composite writer, including IDs that now exist.
    pub(in super::super) fn require_package_snapshot_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        value: &JsonValue,
    ) -> Result<(), NativeRepositoryError> {
        use crate::foundation::server_content_binary_db::ServerBinarySnapshotParentEdgeCodec;
        let object = json_object(value, "package Snapshot")?;
        let id = required_json_text(object, "snapshot_id")
            .map_err(NativeRepositoryError::bad_request)?;
        let (index, record) = self
            .content_snapshots()
            .snapshot_by_id_in_write(tx, &id)
            .map_err(binary_native_repository_store_error)?
            .ok_or_else(|| {
                NativeRepositoryError::conflict("Package Snapshot is absent after staging")
            })?;
        let root = required_json_text(object, "root_tree_pack_id")
            .map_err(NativeRepositoryError::bad_request)?;
        let pack = self
            .repository_content()
            .tree_pack_in_write(tx, &root)
            .map_err(binary_native_repository_store_error)?
            .ok_or_else(|| {
                NativeRepositoryError::conflict("Package root pack is absent after staging")
            })?;
        let mut expected_parents = Vec::new();
        for id in snapshot_parent_ids(value)? {
            let (index, _) = self
                .content_snapshots()
                .snapshot_by_id_in_write(tx, &id)
                .map_err(binary_native_repository_store_error)?
                .ok_or_else(|| {
                    NativeRepositoryError::conflict("Package parent is absent after staging")
                })?;
            expected_parents.push(index);
        }
        let actual_parents = if record.has_parent_edges_authority() {
            let mut edges = Vec::new();
            for edge_index in 0..tx
                .record_count(ServerBinarySnapshotParentEdgeCodec::<1>::record_file())
                .map_err(binary_native_repository_store_error)?
            {
                let edge = ServerBinarySnapshotParentEdgeCodec::<1>::decode_record(
                    &tx.read_record(
                        ServerBinarySnapshotParentEdgeCodec::<1>::record_file(),
                        edge_index,
                    )
                    .map_err(binary_native_repository_store_error)?,
                )
                .map_err(binary_native_repository_store_error)?;
                if edge.child_snapshot_index == index {
                    edges.push((edge.parent_ordinal, edge.parent_snapshot_index));
                }
            }
            edges.sort_unstable();
            if edges
                .iter()
                .enumerate()
                .any(|(ordinal, (actual, _))| ordinal != usize::from(*actual))
            {
                return Err(NativeRepositoryError::conflict(
                    "Package Snapshot has invalid parent ordinals",
                ));
            }
            edges
                .into_iter()
                .map(|(_, parent)| parent)
                .collect::<Vec<_>>()
        } else {
            record
                .parent_snapshot_index_plus1
                .checked_sub(1)
                .into_iter()
                .collect()
        };
        let payload = ServerBinarySnapshotCodec::<2>::decode_payload(
            &tx.read_payload(
                ServerBinarySnapshotCodec::<2>::payload_file(),
                record.payload_offset,
                u32::from(record.payload_len),
            )
            .map_err(binary_native_repository_store_error)?,
            record.has_line_name_payload(),
        )
        .map_err(binary_native_repository_store_error)?;
        let line = optional_json_text(object, "line_name").unwrap_or_else(default_main_line);
        let line_matches = if record.line_index_plus1 == 0 {
            payload.line_name == line
        } else {
            self.content_lines()
                .line_by_name_in_write(tx, &line)
                .map_err(binary_native_repository_store_error)?
                .is_some_and(|(index, _)| record.line_index_plus1 == index + 1)
        };
        let created =
            required_json_text(object, "created_at").map_err(NativeRepositoryError::bad_request)?;
        if record.root_tree_pack_index_plus1 != pack.pack_index + 1
            || record.root_entry_ordinal != json_u32(value, "root_entry_ordinal")?
            || record.manifest_hash
                != decode_optional_sha256(binary_json_text(value, "manifest_hash"))?
            || record.file_count != json_u32(value, "file_count")?
            || record.total_bytes != json_u64(value, "total_bytes")?
            || record.created_at_s != timestamp_s(&created)?
            || actual_parents != expected_parents
            || record.parent_snapshot_index_plus1
                != expected_parents.first().map_or(0, |index| index + 1)
            || !line_matches
            || payload.message != optional_json_text(object, "message")
        {
            return Err(NativeRepositoryError::conflict(format!(
                "Package Snapshot {id} differs from immutable content during publication"
            )));
        }
        Ok(())
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
                    .read_record(ServerBinarySnapshotCodec::<2>::record_file(), parent_index)
                    .map_err(binary_native_repository_store_error)?;
                let parent = ServerBinarySnapshotCodec::<2>::decode_record(&raw)
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
            "actor_key_index": record.actor_index_plus1.checked_sub(1),
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
