use super::{JsonValue, PlanSyncZstdObjectPackBundle, PlanSyncZstdTreePackBundle};
use crate::repository_pack_json::ZstdBulkTreeLocatorRow;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PlanSyncArtifactTreeRootLocator {
    pub(super) root_tree_pack_index_plus1: u32,
    pub(super) root_entry_ordinal: u32,
}

pub(super) trait PlanSyncLocalBlobStore {
    fn ensure_blob_bytes(&self, data: &[u8], path_hint: Option<&str>) -> Result<String, String>;

    fn read_blob_bytes(&self, blob_id: &str) -> Result<Vec<u8>, String>;

    fn blob_chain_depth(&self, blob_id: &str) -> Result<Option<i64>, String>;
}

pub(super) trait PlanSyncZstdPackStore {
    fn existing_zstd_object_pack_bundle(
        &self,
        blob_id: &str,
        expected_sha256: &str,
        expected_size_bytes: i64,
    ) -> Result<Option<PlanSyncZstdObjectPackBundle>, String>;

    fn upsert_zstd_object_pack_metadata(
        &self,
        metadata: PlanSyncZstdObjectPackMetadata<'_>,
    ) -> Result<(), String>;

    fn existing_zstd_tree_pack_bundle(
        &self,
        generation_key: &str,
        root_tree_id: &str,
        tree_rows: &[JsonValue],
    ) -> Result<Option<PlanSyncZstdTreePackBundle>, String>;

    fn unrecorded_tree_ids(&self, tree_ids: &[String]) -> Result<BTreeSet<String>, String> {
        Ok(tree_ids.iter().cloned().collect())
    }

    fn upsert_zstd_tree_pack_metadata(
        &self,
        metadata: PlanSyncZstdTreePackMetadata<'_>,
    ) -> Result<(), String>;

    fn prepare_artifact_tree_root_locator(
        &self,
        generation_key: &str,
        artifact_path: &str,
        artifact_blob_id: &str,
        byte_count: i64,
        created_at: &str,
    ) -> Result<Option<PlanSyncArtifactTreeRootLocator>, String> {
        let _ = (
            generation_key,
            artifact_path,
            artifact_blob_id,
            byte_count,
            created_at,
        );
        Ok(None)
    }
}

pub(super) trait PlanSyncLocalContentStore:
    PlanSyncLocalBlobStore + PlanSyncZstdPackStore
{
}

impl<T> PlanSyncLocalContentStore for T where
    T: PlanSyncLocalBlobStore + PlanSyncZstdPackStore + ?Sized
{
}

pub(super) struct PlanSyncZstdObjectPackMetadata<'a> {
    pub(super) blob_id: &'a str,
    pub(super) sha256: &'a str,
    pub(super) size_bytes: i64,
    pub(super) pack_id: &'a str,
    pub(super) pack_rel_path: &'a str,
    pub(super) pack_format: &'a str,
    pub(super) member_count: i64,
    pub(super) total_bytes: i64,
    pub(super) pack_index_entry_name: &'a str,
    pub(super) pack_index_checksum: &'a str,
    pub(super) pack_entry_type: &'a str,
    pub(super) pack_base_blob_id: Option<&'a str>,
    pub(super) pack_chain_depth: i64,
    pub(super) created_at: &'a str,
}

pub(super) struct PlanSyncZstdTreePackMetadata<'a> {
    pub(super) pack_id: &'a str,
    pub(super) pack_rel_path: &'a str,
    pub(super) pack_format: &'a str,
    pub(super) tree_count: i64,
    pub(super) total_bytes: i64,
    pub(super) pack_index_entry_name: &'a str,
    pub(super) pack_index_checksum: &'a str,
    pub(super) tree_locators: &'a [ZstdBulkTreeLocatorRow],
    pub(super) created_at: &'a str,
}

pub(super) fn ensure_blob_bytes_with_plan_sync_local_blob_store<B>(
    store: &B,
    data: &[u8],
    path_hint: Option<&str>,
) -> Result<String, String>
where
    B: PlanSyncLocalBlobStore + ?Sized,
{
    store.ensure_blob_bytes(data, path_hint)
}

pub(super) fn read_blob_bytes_with_plan_sync_local_blob_store<B>(
    store: &B,
    blob_id: &str,
) -> Result<Vec<u8>, String>
where
    B: PlanSyncLocalBlobStore + ?Sized,
{
    store.read_blob_bytes(blob_id)
}

pub(super) fn blob_chain_depth_with_plan_sync_local_blob_store<B>(
    store: &B,
    blob_id: &str,
) -> Result<Option<i64>, String>
where
    B: PlanSyncLocalBlobStore + ?Sized,
{
    store.blob_chain_depth(blob_id)
}

pub(super) fn existing_zstd_object_pack_bundle_with_plan_sync_zstd_pack_store<B>(
    store: &B,
    blob_id: &str,
    expected_sha256: &str,
    expected_size_bytes: i64,
) -> Result<Option<PlanSyncZstdObjectPackBundle>, String>
where
    B: PlanSyncZstdPackStore + ?Sized,
{
    store.existing_zstd_object_pack_bundle(blob_id, expected_sha256, expected_size_bytes)
}

pub(super) fn upsert_zstd_object_pack_metadata_with_plan_sync_zstd_pack_store<B>(
    store: &B,
    metadata: PlanSyncZstdObjectPackMetadata<'_>,
) -> Result<(), String>
where
    B: PlanSyncZstdPackStore + ?Sized,
{
    store.upsert_zstd_object_pack_metadata(metadata)
}

pub(super) fn existing_zstd_tree_pack_bundle_with_plan_sync_zstd_pack_store<B>(
    store: &B,
    generation_key: &str,
    root_tree_id: &str,
    tree_rows: &[JsonValue],
) -> Result<Option<PlanSyncZstdTreePackBundle>, String>
where
    B: PlanSyncZstdPackStore + ?Sized,
{
    store.existing_zstd_tree_pack_bundle(generation_key, root_tree_id, tree_rows)
}

pub(super) fn upsert_zstd_tree_pack_metadata_with_plan_sync_zstd_pack_store<B>(
    store: &B,
    metadata: PlanSyncZstdTreePackMetadata<'_>,
) -> Result<(), String>
where
    B: PlanSyncZstdPackStore + ?Sized,
{
    store.upsert_zstd_tree_pack_metadata(metadata)
}

pub(super) fn prepare_artifact_tree_root_locator_with_plan_sync_zstd_pack_store<B>(
    store: &B,
    generation_key: &str,
    artifact_path: &str,
    artifact_blob_id: &str,
    byte_count: i64,
    created_at: &str,
) -> Result<Option<PlanSyncArtifactTreeRootLocator>, String>
where
    B: PlanSyncZstdPackStore + ?Sized,
{
    store.prepare_artifact_tree_root_locator(
        generation_key,
        artifact_path,
        artifact_blob_id,
        byte_count,
        created_at,
    )
}
