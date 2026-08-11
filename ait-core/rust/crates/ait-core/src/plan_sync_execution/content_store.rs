use super::content_ports::{
    PlanSyncArtifactTreeRootLocator, PlanSyncLocalBlobStore, PlanSyncZstdObjectPackMetadata,
    PlanSyncZstdPackStore, PlanSyncZstdTreePackMetadata,
};
use super::{
    current_timestamp, json_i64_field, json_string_field, plan_sync_blob_pack_entry_name,
    read_tree_pack_index_with_format, rfc3339_timestamp_from_epoch_s, sha256_hex,
    write_plan_revision_zstd_tree_pack, JsonValue, PackFormatKind, PlanSyncZstdObjectPackBundle,
    PlanSyncZstdTreePackBundle, TreePackFormatKind,
};
use crate::binary_db::{BinaryDb, BinaryDbCommandScope, BinaryDbIndexAppender};
use crate::content_binary_db::{
    BinaryDbBlobStore, BinaryDbContentWriteCoordinator, BinaryDbObjectPackMemberWriteInput,
    BinaryDbObjectPackStore, BinaryDbObjectPackWriteInput, BinaryDbSnapshotStore,
    BinaryDbTreePackStore, BinaryDbTreePackTreeWriteInput, BinaryDbTreePackWriteInput,
    BinaryDbTreeStore, BinaryObjectPackMemberKind, BinaryObjectPackMemberRecord,
};
use crate::content_store::BlobStore;
use crate::json_support::json;
use crate::pack_substrate::{
    build_pack_members, default_object_pack_relative_path, pack_index_checksum_with_format,
    tree_pack_index_checksum_with_format, write_pack_archive_with_format,
    CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
};
use crate::repository_pack_json::ZstdBulkTreeLocatorRow;
use crate::repository_pack_policy::{
    zstd_only_object_pack_write_format, zstd_only_tree_pack_write_format,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;

#[allow(dead_code)]
pub(super) struct BinaryDbPlanSyncLocalContentStore<B, const WRITE_LAYOUT: u32>
where
    B: BinaryDb,
{
    blobs: BinaryDbBlobStore<B, WRITE_LAYOUT>,
    object_packs: BinaryDbObjectPackStore<B, WRITE_LAYOUT>,
    tree_packs: BinaryDbTreePackStore<B, WRITE_LAYOUT>,
    trees: BinaryDbTreeStore<B, WRITE_LAYOUT>,
    snapshots: BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
}

#[allow(dead_code)]
impl<B, const WRITE_LAYOUT: u32> BinaryDbPlanSyncLocalContentStore<B, WRITE_LAYOUT>
where
    B: BinaryDb,
{
    pub(super) fn new(
        blobs: BinaryDbBlobStore<B, WRITE_LAYOUT>,
        object_packs: BinaryDbObjectPackStore<B, WRITE_LAYOUT>,
        tree_packs: BinaryDbTreePackStore<B, WRITE_LAYOUT>,
        trees: BinaryDbTreeStore<B, WRITE_LAYOUT>,
        snapshots: BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
    ) -> Self {
        Self {
            blobs,
            object_packs,
            tree_packs,
            trees,
            snapshots,
        }
    }

    fn write_coordinator(&self) -> BinaryDbContentWriteCoordinator<'_, B, WRITE_LAYOUT> {
        BinaryDbContentWriteCoordinator::new(
            &self.blobs,
            &self.object_packs,
            &self.tree_packs,
            &self.trees,
            &self.snapshots,
        )
    }
}

impl<B, const WRITE_LAYOUT: u32> PlanSyncLocalBlobStore
    for BinaryDbPlanSyncLocalContentStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    fn ensure_blob_bytes(&self, data: &[u8], path_hint: Option<&str>) -> Result<String, String> {
        let digest_hex = sha256_hex(data);
        let blob_id = format!("BLB-{}", &digest_hex[..20]);
        if self.blobs.get_blob(&blob_id)?.is_some() {
            return Ok(blob_id);
        }
        let created_at = current_timestamp();
        let pack_seed = format!("BLOB-{blob_id}|[\"{blob_id}\"]");
        let pack_id = format!(
            "PCK-{}",
            sha256_hex(pack_seed.as_bytes())[..12].to_ascii_uppercase()
        );
        let pack_rel_path = default_object_pack_relative_path(&pack_id);
        let pack_path = self.blobs.repo_root().as_path().join(&pack_rel_path);
        if let Some(parent) = pack_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let members = build_pack_members(
            &json!([{
                "entry_name": format!("blobs/{blob_id}"),
                "blob_id": blob_id.clone(),
                "data": data,
                "path_hint": path_hint.unwrap_or(""),
            }]),
            crate::pack_substrate::DEFAULT_MAX_DELTA_CHAIN_DEPTH,
            None,
        )?;
        let member_obj = members
            .as_array()
            .and_then(|rows| rows.first())
            .and_then(JsonValue::as_object)
            .cloned()
            .ok_or_else(|| "Failed to build Binary DB blob pack member.".to_string())?;
        let archive_stats = write_pack_archive_with_format(
            pack_path.to_string_lossy().as_ref(),
            &pack_id,
            CONTENT_ADDRESSED_PACK_INDEX_CREATED_AT,
            &members,
            zstd_only_object_pack_write_format(),
        )?;
        self.write_coordinator().record_object_pack_metadata(
            BinaryDbCommandScope::PlanSyncLocal,
            &BinaryDbObjectPackWriteInput {
                pack_id,
                pack_rel_path,
                pack_format: json_string_field(&archive_stats, "pack_format")?,
                member_count: json_i64_field(&archive_stats, "member_count")?,
                total_bytes: json_i64_field(&archive_stats, "total_bytes")?,
                created_at: created_at.clone(),
                members: vec![BinaryDbObjectPackMemberWriteInput {
                    blob_id: blob_id.clone(),
                    sha256: digest_hex,
                    size_bytes: data.len() as i64,
                    pack_entry_type: member_obj
                        .get("entry_type")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("full")
                        .to_string(),
                    pack_base_blob_id: member_obj
                        .get("base_blob_id")
                        .and_then(JsonValue::as_str)
                        .map(str::to_string),
                    pack_chain_depth: member_obj
                        .get("chain_depth")
                        .and_then(JsonValue::as_i64)
                        .unwrap_or(0),
                    created_at,
                }],
            },
        )?;
        Ok(blob_id)
    }

    fn read_blob_bytes(&self, blob_id: &str) -> Result<Vec<u8>, String> {
        self.blobs.read_blob_bytes(blob_id)
    }

    fn blob_chain_depth(&self, blob_id: &str) -> Result<Option<i64>, String> {
        Ok(self
            .blobs
            .get_blob(blob_id)?
            .and_then(|record| record.pack_chain_depth))
    }
}

impl<B, const WRITE_LAYOUT: u32> PlanSyncZstdPackStore
    for BinaryDbPlanSyncLocalContentStore<B, WRITE_LAYOUT>
where
    B: BinaryDb + BinaryDbIndexAppender,
{
    fn existing_zstd_object_pack_bundle(
        &self,
        blob_id: &str,
        expected_sha256: &str,
        expected_size_bytes: i64,
    ) -> Result<Option<PlanSyncZstdObjectPackBundle>, String> {
        if expected_size_bytes < 0 {
            return Err(format!(
                "Blob {blob_id} expected_size_bytes is negative during Binary DB plan sync."
            ));
        }
        let read = self.blobs.begin_read_txn();
        let Some(blob) = self.blobs.get_blob_view(&read, blob_id)? else {
            return Ok(None);
        };
        if blob.sha256 != expected_sha256 {
            return Err(format!(
                "Blob {blob_id} sha256 drifted from canonical metadata during packed plan sync."
            ));
        }
        if blob.size_bytes != expected_size_bytes as u64 {
            return Err(format!(
                "Blob {blob_id} size drifted from canonical metadata during packed plan sync."
            ));
        }
        let Some(member_index) = blob.record.pack_member_index() else {
            return Ok(None);
        };
        let member = self
            .object_packs
            .object_pack_member_view_at(&read, member_index)?;
        let pack = self
            .object_packs
            .object_pack_view_at(&read, member.record.pack_index)?;
        if pack.pack_format != zstd_only_object_pack_write_format() {
            return Ok(None);
        }
        let pack_abs_path = self.blobs.repo_root().as_path().join(&pack.pack_path);
        if !pack_abs_path.is_file() {
            return Err(format!(
                "Existing object pack {} for blob {blob_id} is missing local archive {}.",
                pack.pack_id,
                pack_abs_path.display()
            ));
        }
        let pack_index_checksum = pack_index_checksum_with_format(
            pack_abs_path.to_string_lossy().as_ref(),
            &pack.pack_format,
        )?
        .unwrap_or_default();
        let total_bytes = i64::try_from(pack.record.total_bytes)
            .map_err(|_| format!("Object pack {} total_bytes overflows i64.", pack.pack_id))?;
        let created_at = rfc3339_timestamp_from_epoch_s(pack.record.created_at_s)?;
        let pack_index_entry_name = PackFormatKind::from_persisted(&pack.pack_format)?
            .index_entry_name()
            .to_string();
        Ok(Some(PlanSyncZstdObjectPackBundle {
            blob_id: blob_id.to_string(),
            sha256: blob.sha256,
            byte_count: expected_size_bytes,
            pack_id: pack.pack_id.clone(),
            pack_path: pack_abs_path,
            pack_format: pack.pack_format,
            member_count: i64::from(pack.record.member_count),
            total_bytes,
            pack_index_entry_name,
            pack_index_checksum,
            pack_entry_name: plan_sync_blob_pack_entry_name(blob_id),
            pack_entry_type: member_pack_entry_type(&member.record),
            pack_base_blob_id: member.base_blob_id,
            pack_chain_depth: i64::from(member.record.delta_chain_depth),
            created_at,
        }))
    }

    fn upsert_zstd_object_pack_metadata(
        &self,
        metadata: PlanSyncZstdObjectPackMetadata<'_>,
    ) -> Result<(), String> {
        // Binary DB derives the canonical index location and checksum from the
        // persisted pack. Keep the caller-provided values in the port contract
        // for alternate stores without duplicating them in Binary DB records.
        let _derived_index_metadata =
            (metadata.pack_index_entry_name, metadata.pack_index_checksum);
        Ok(self.write_coordinator().record_object_pack_metadata(
            BinaryDbCommandScope::PlanSyncLocal,
            &BinaryDbObjectPackWriteInput {
                pack_id: metadata.pack_id.to_string(),
                pack_rel_path: metadata.pack_rel_path.to_string(),
                pack_format: metadata.pack_format.to_string(),
                member_count: metadata.member_count,
                total_bytes: metadata.total_bytes,
                created_at: metadata.created_at.to_string(),
                members: vec![BinaryDbObjectPackMemberWriteInput {
                    blob_id: metadata.blob_id.to_string(),
                    sha256: metadata.sha256.to_string(),
                    size_bytes: metadata.size_bytes,
                    pack_entry_type: metadata.pack_entry_type.to_string(),
                    pack_base_blob_id: metadata.pack_base_blob_id.map(str::to_string),
                    pack_chain_depth: metadata.pack_chain_depth,
                    created_at: metadata.created_at.to_string(),
                }],
            },
        )?)
    }

    fn existing_zstd_tree_pack_bundle(
        &self,
        generation_key: &str,
        root_tree_id: &str,
        _tree_rows: &[JsonValue],
    ) -> Result<Option<PlanSyncZstdTreePackBundle>, String> {
        let read = self.trees.begin_read_txn();
        let Some(root_tree) = self.trees.get_tree_view(&read, root_tree_id)? else {
            return Ok(None);
        };
        let Some(tree_pack_id) = root_tree.tree_pack_id.as_deref() else {
            return Ok(None);
        };
        let Some(pack) = self.tree_packs.get_tree_pack_view(&read, tree_pack_id)? else {
            return Ok(None);
        };
        if pack.pack_format != zstd_only_tree_pack_write_format() {
            return Ok(None);
        }
        let created_at = rfc3339_timestamp_from_epoch_s(pack.record.created_at_s)?;
        let pack_abs_path = self.blobs.repo_root().as_path().join(&pack.pack_path);
        if !pack_abs_path.is_file() {
            return Err(format!(
                "Existing tree pack {} for tree {root_tree_id} is missing local archive {}.",
                pack.pack_id,
                pack_abs_path.display()
            ));
        }
        let pack_index = read_tree_pack_index_with_format(
            pack_abs_path.to_string_lossy().as_ref(),
            &pack.pack_format,
        )?;
        let checksum_by_tree_id = pack_index
            .get("trees")
            .and_then(JsonValue::as_array)
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|tree| {
                Some((
                    tree.get("tree_id").and_then(JsonValue::as_str)?.to_string(),
                    tree.get("checksum")
                        .and_then(JsonValue::as_str)
                        .unwrap_or_default()
                        .to_string(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let root_entry = pack_index
            .get("trees")
            .and_then(JsonValue::as_array)
            .and_then(|trees| {
                trees.iter().find(|tree| {
                    tree.get("tree_id").and_then(JsonValue::as_str) == Some(root_tree_id)
                })
            })
            .ok_or_else(|| {
                format!(
                    "Existing tree pack {} does not index root tree {root_tree_id}.",
                    pack.pack_id
                )
            })?;
        let root_entry_ordinal = root_entry
            .get("entry_ordinal")
            .and_then(JsonValue::as_i64)
            .ok_or_else(|| {
                format!(
                    "Existing tree pack {} is missing entry_ordinal for root tree {root_tree_id}.",
                    pack.pack_id
                )
            })?;
        let root_entry_count = root_entry
            .get("entry_count")
            .and_then(JsonValue::as_i64)
            .unwrap_or_else(|| i64::from(root_tree.record.entry_count));
        let root_tree_checksum = checksum_by_tree_id
            .get(root_tree_id)
            .cloned()
            .unwrap_or_default();
        let empty_trees = Vec::new();
        let tree_locators = pack_index
            .get("trees")
            .and_then(JsonValue::as_array)
            .unwrap_or(&empty_trees)
            .iter()
            .filter_map(|tree| {
                let tree_id = tree.get("tree_id").and_then(JsonValue::as_str)?;
                let checksum = checksum_by_tree_id
                    .get(tree_id)
                    .cloned()
                    .unwrap_or_default();
                Some(ZstdBulkTreeLocatorRow {
                    generation_key: Some(generation_key.to_string()),
                    tree_id: tree_id.to_string(),
                    entry_count: tree.get("entry_count").and_then(JsonValue::as_i64),
                    tree_pack_id: Some(pack.pack_id.clone()),
                    tree_pack_checksum: Some(checksum),
                    created_at: Some(created_at.clone()),
                })
            })
            .collect::<Vec<_>>();
        let pack_index_checksum = tree_pack_index_checksum_with_format(
            pack_abs_path.to_string_lossy().as_ref(),
            &pack.pack_format,
        )?
        .unwrap_or_default();
        let total_bytes = i64::try_from(pack.record.total_bytes)
            .map_err(|_| format!("Tree pack {} total_bytes overflows i64.", pack.pack_id))?;
        let pack_index_entry_name = TreePackFormatKind::from_persisted(&pack.pack_format)?
            .index_entry_name()
            .to_string();
        Ok(Some(PlanSyncZstdTreePackBundle {
            root_tree_id: root_tree_id.to_string(),
            root_entry_count,
            root_entry_ordinal,
            root_tree_checksum,
            pack_id: pack.pack_id.clone(),
            pack_path: pack_abs_path,
            pack_format: pack.pack_format,
            tree_count: i64::from(pack.record.tree_count),
            total_bytes,
            pack_index_entry_name,
            pack_index_checksum,
            tree_locators,
            created_at,
        }))
    }

    fn unrecorded_tree_ids(&self, tree_ids: &[String]) -> Result<BTreeSet<String>, String> {
        let read = self.trees.begin_read_txn();
        let mut unrecorded = BTreeSet::new();
        for tree_id in tree_ids {
            match self.trees.get_tree_view(&read, tree_id)? {
                None => {
                    unrecorded.insert(tree_id.clone());
                }
                Some(tree) if tree.record.is_tombstone() => {
                    return Err(format!(
                        "Existing tree {tree_id} is tombstoned and cannot be reused by Plan sync."
                    ));
                }
                Some(_) => {}
            }
        }
        Ok(unrecorded)
    }

    fn upsert_zstd_tree_pack_metadata(
        &self,
        metadata: PlanSyncZstdTreePackMetadata<'_>,
    ) -> Result<(), String> {
        // Binary DB derives the canonical index location and checksum from the
        // persisted pack. Keep the caller-provided values in the port contract
        // for alternate stores without duplicating them in Binary DB records.
        let _derived_index_metadata =
            (metadata.pack_index_entry_name, metadata.pack_index_checksum);
        Ok(self.write_coordinator().record_tree_pack_metadata(
            BinaryDbCommandScope::PlanSyncLocal,
            &BinaryDbTreePackWriteInput {
                pack_id: metadata.pack_id.to_string(),
                pack_rel_path: metadata.pack_rel_path.to_string(),
                pack_format: metadata.pack_format.to_string(),
                tree_count: metadata.tree_count,
                total_bytes: metadata.total_bytes,
                created_at: metadata.created_at.to_string(),
                trees: metadata
                    .tree_locators
                    .iter()
                    .map(|locator| {
                        Ok(BinaryDbTreePackTreeWriteInput {
                            tree_id: locator.tree_id.clone(),
                            entry_count: locator.entry_count.ok_or_else(|| {
                                format!(
                                    "Tree locator {} is missing entry_count metadata.",
                                    locator.tree_id
                                )
                            })?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            },
        )?)
    }

    fn prepare_artifact_tree_root_locator(
        &self,
        generation_key: &str,
        artifact_path: &str,
        artifact_blob_id: &str,
        byte_count: i64,
        created_at: &str,
    ) -> Result<Option<PlanSyncArtifactTreeRootLocator>, String> {
        let bundle = write_plan_revision_zstd_tree_pack(
            self,
            self.tree_packs.repo_root().as_path(),
            generation_key,
            artifact_path,
            artifact_blob_id,
            byte_count,
            created_at,
        )?;
        let read = self.tree_packs.begin_read_txn();
        let pack = self
            .tree_packs
            .get_tree_pack_view(&read, &bundle.pack_id)?
            .ok_or_else(|| {
                format!(
                    "Binary DB plan sync tree pack {} was not recorded before plan revision commit.",
                    bundle.pack_id
                )
            })?;
        let root_tree = self
            .trees
            .get_tree_view(&read, &bundle.root_tree_id)?
            .ok_or_else(|| {
                format!(
                    "Binary DB plan sync root tree {} was not recorded before plan revision commit.",
                    bundle.root_tree_id
                )
            })?;
        if root_tree.tree_pack_id.as_deref() != Some(bundle.pack_id.as_str()) {
            return Err(format!(
                "Binary DB plan sync root tree {} points at {:?}, not tree pack {}.",
                bundle.root_tree_id, root_tree.tree_pack_id, bundle.pack_id
            ));
        }
        let root_entry_ordinal = u32::try_from(bundle.root_entry_ordinal).map_err(|_| {
            format!(
                "Binary DB plan sync root tree {} entry ordinal {} is outside u32.",
                bundle.root_tree_id, bundle.root_entry_ordinal
            )
        })?;
        Ok(Some(PlanSyncArtifactTreeRootLocator {
            root_tree_pack_index_plus1: pack.tree_pack_index.checked_add(1).ok_or_else(|| {
                format!(
                    "Binary DB plan sync tree pack {} index overflow.",
                    bundle.pack_id
                )
            })?,
            root_entry_ordinal,
        }))
    }
}

fn member_pack_entry_type(record: &BinaryObjectPackMemberRecord) -> String {
    match record.member_kind() {
        BinaryObjectPackMemberKind::Full => "full".to_string(),
        BinaryObjectPackMemberKind::Delta => "delta".to_string(),
        BinaryObjectPackMemberKind::Reserved(value) => format!("reserved-{value}"),
    }
}
