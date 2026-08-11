use super::*;
use crate::foundation::pack_substrate::{
    read_zstd_object_pack_blob_from_bytes, TreePackEntryArchive, MAX_DELTA_CHAIN_READ_DEPTH,
    TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
};
use crate::foundation::remote_binary_db::{BinaryDbReadScope, BinaryIndexKeyRef};
use serde_json::{json, Value as JsonValue};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::path::PathBuf;

#[cfg(test)]
std::thread_local! {
    static TEST_CONTENT_RECORD_READ_RANGES: std::cell::RefCell<BTreeMap<String, Vec<(u32, u32)>>> =
        std::cell::RefCell::new(BTreeMap::new());
    static TEST_CONTENT_PAYLOAD_READ_RANGES: std::cell::RefCell<BTreeMap<String, Vec<(u64, u32)>>> =
        std::cell::RefCell::new(BTreeMap::new());
}

#[cfg(test)]
fn observe_test_content_record_read(file: &BinaryFileId, first_index: u32, count: u32) {
    TEST_CONTENT_RECORD_READ_RANGES.with(|reads| {
        reads
            .borrow_mut()
            .entry(file.as_str().to_string())
            .or_default()
            .push((first_index, count));
    });
}

#[cfg(test)]
fn observe_test_content_payload_read(file: &BinaryPayloadFileId, offset: u64, len: u32) {
    TEST_CONTENT_PAYLOAD_READ_RANGES.with(|reads| {
        reads
            .borrow_mut()
            .entry(file.as_str().to_string())
            .or_default()
            .push((offset, len));
    });
}

#[cfg(test)]
pub(crate) fn reset_test_content_read_ranges() {
    TEST_CONTENT_RECORD_READ_RANGES.with(|reads| reads.borrow_mut().clear());
    TEST_CONTENT_PAYLOAD_READ_RANGES.with(|reads| reads.borrow_mut().clear());
}

#[cfg(test)]
pub(crate) fn test_content_record_read_ranges(file: &str) -> Vec<(u32, u32)> {
    TEST_CONTENT_RECORD_READ_RANGES
        .with(|reads| reads.borrow().get(file).cloned().unwrap_or_default())
}

#[cfg(test)]
pub(crate) fn test_content_payload_read_ranges(file: &str) -> Vec<(u64, u32)> {
    TEST_CONTENT_PAYLOAD_READ_RANGES
        .with(|reads| reads.borrow().get(file).cloned().unwrap_or_default())
}

pub const SERVER_BLOB_RECORD_SIZE: u32 = 64;
pub const SERVER_OBJECT_PACK_RECORD_SIZE: u32 = 32;
pub const SERVER_OBJECT_PACK_MEMBER_RECORD_SIZE: u32 = 16;
pub const SERVER_TREE_PACK_RECORD_SIZE: u32 = 32;
pub const SERVER_TREE_RECORD_SIZE: u32 = 20;
pub const SERVER_TREE_ENTRY_RECORD_SIZE: u32 = 16;
pub const SERVER_TREE_ENTRY_RANGE_RECORD_SIZE: u32 = 4;

const BLOB_BIN: &str = "blob.bin";
const BLOB_ID_IDX: &str = "blob_id.idx";
const OBJECT_PACK_BIN: &str = "object_pack.bin";
const OBJECT_PACK_ID_IDX: &str = "object_pack_id.idx";
const OBJECT_PACK_MEMBER_BIN: &str = "object_pack_member.bin";
const TREE_PACK_BIN: &str = "tree_pack.bin";
const TREE_PACK_ID_IDX: &str = "tree_pack_id.idx";
const TREE_BIN: &str = "tree.bin";
const TREE_ID_IDX: &str = "tree_id.idx";
const TREE_ENTRY_BIN: &str = "tree_entry.bin";
const TREE_ENTRY_RANGE_BIN: &str = "tree_entry_range.bin";
const TREE_NAME_PAYLOAD_BIN: &str = "tree_name_payload.bin";

const META_READY: u8 = 0b0000_0001;
const META_CORRUPT: u8 = 0b0000_0010;
const META_SPARSE_PHYSICAL_ORDINALS: u8 = 0b0000_0100;
const META_HAS_PACK_MEMBER: u8 = 0b0000_0001;
const META_TOMBSTONE: u8 = 0b1000_0000;
const SHA256_HASH_KIND: u8 = 0;
const ZSTD_PACK_FORMAT_KIND: u8 = 1;
const ZSTD_MEMBER_COMPRESSION_META: u8 = 2 << 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryBlobRecord {
    pub blob_meta: u8,
    pub hash_kind: u8,
    pub size_bytes: u64,
    pub pack_member_index_plus1: u32,
    pub created_at_s: u64,
    pub pruned_at_s: u64,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryObjectPackRecord {
    pub pack_meta: u8,
    pub pack_format_kind: u8,
    pub pack_hash48: u64,
    pub first_member_index: u32,
    pub member_count: u32,
    pub total_bytes: u64,
    pub created_at_s: u64,
}

impl ServerBinaryObjectPackRecord {
    pub fn is_ready(&self) -> bool {
        self.pack_meta & META_READY != 0 && self.pack_meta & META_TOMBSTONE == 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryObjectPackMemberRecord {
    pub member_meta: u8,
    pub delta_chain_depth: u8,
    pub pack_index: u32,
    pub blob_index: u32,
    pub base_blob_index_plus1: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryTreePackRecord {
    pub pack_meta: u8,
    pub pack_format_kind: u8,
    pub pack_hash48: u64,
    pub first_tree_index: u32,
    pub tree_count: u32,
    pub total_bytes: u64,
    pub created_at_s: u64,
}

impl ServerBinaryTreePackRecord {
    pub fn is_ready(&self) -> bool {
        self.pack_meta & META_READY != 0 && self.pack_meta & (META_CORRUPT | META_TOMBSTONE) == 0
    }

    pub fn has_sparse_physical_ordinals(&self) -> bool {
        self.pack_meta & META_SPARSE_PHYSICAL_ORDINALS != 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryTreeRecord {
    pub tree_meta: u8,
    pub pack_entry_ordinal: u32,
    pub entry_count: u32,
    pub tree_hash80: [u8; 10],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerBinaryTreeEntryRangeRecord {
    pub first_entry_index: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerBinaryTreeEntryRecord {
    pub entry_meta: u8,
    pub name_len: u8,
    pub mode_bits: u16,
    pub name_offset: u64,
    pub target_index: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryBlobView {
    pub blob_index: u32,
    pub blob_id: String,
    pub record: ServerBinaryBlobRecord,
    pub member_index: u32,
    pub member: ServerBinaryObjectPackMemberRecord,
    pub pack_id: String,
    pub pack: ServerBinaryObjectPackRecord,
    pub base_blob_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryObjectPackView {
    pub pack_index: u32,
    pub pack_id: String,
    pub record: ServerBinaryObjectPackRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryTreeView {
    pub tree_index: u32,
    pub tree_id: String,
    pub record: ServerBinaryTreeRecord,
    pub pack_index: u32,
    pub pack_id: String,
    pub pack: ServerBinaryTreePackRecord,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryTreeEntryView {
    pub entry_index: u32,
    pub entry_name: String,
    pub entry_type: String,
    pub target_id: String,
    pub size_bytes: Option<u64>,
    pub sha256: Option<String>,
    pub mode: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinarySnapshotFileIdentity {
    pub blob_id: String,
    pub size_bytes: u64,
    pub mode: String,
    pub sha256: String,
}

#[derive(Debug, Default)]
pub struct ServerBinaryTreeReadCache {
    archives: BTreeMap<String, TreePackEntryArchive>,
    object_packs_by_id: BTreeMap<String, ServerBinaryObjectPackView>,
    object_packs_by_index: BTreeMap<u32, ServerBinaryObjectPackView>,
    tree_packs_by_id: BTreeMap<String, ServerBinaryTreePackView>,
    tree_packs_by_index: BTreeMap<u32, ServerBinaryTreePackView>,
    blobs_by_id: BTreeMap<String, ServerBinaryBlobView>,
    blobs_by_index: BTreeMap<u32, ServerBinaryBlobView>,
    blobs_by_member_index: BTreeMap<u32, ServerBinaryBlobView>,
    trees_by_id: BTreeMap<String, ServerBinaryTreeView>,
    trees_by_index: BTreeMap<u32, ServerBinaryTreeView>,
    normalized_entry_ranges: Option<Vec<ServerBinaryTreeEntryRangeRecord>>,
    normalized_entries: Option<Vec<ServerBinaryTreeEntryRecord>>,
    tree_name_payload_body: Option<Vec<u8>>,
    manifest_object_projection_complete: bool,
    manifest_tree_projection_complete: bool,
    #[cfg(test)]
    archive_open_count: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ServerBinaryTreeAggregate {
    file_count: u32,
    total_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ServerBinaryTreeAggregateNode {
    direct: ServerBinaryTreeAggregate,
    child_tree_ids: Vec<String>,
}

impl ServerBinaryTreeReadCache {
    fn cache_object_packs(&mut self, packs: Vec<ServerBinaryObjectPackView>) {
        for pack in packs {
            self.object_packs_by_index
                .insert(pack.pack_index, pack.clone());
            if pack.record.pack_meta & META_TOMBSTONE == 0 {
                self.object_packs_by_id
                    .entry(pack.pack_id.to_ascii_uppercase())
                    .and_modify(|existing| {
                        if pack.pack_index < existing.pack_index {
                            *existing = pack.clone();
                        }
                    })
                    .or_insert(pack);
            }
        }
    }

    fn cache_tree_packs(&mut self, packs: Vec<ServerBinaryTreePackView>) {
        for pack in packs {
            self.tree_packs_by_index
                .insert(pack.pack_index, pack.clone());
            if pack.record.pack_meta & META_TOMBSTONE == 0 {
                self.tree_packs_by_id
                    .entry(pack.pack_id.to_ascii_uppercase())
                    .and_modify(|existing| {
                        if pack.pack_index < existing.pack_index {
                            *existing = pack.clone();
                        }
                    })
                    .or_insert(pack);
            }
        }
    }

    fn cache_blobs(&mut self, blobs: BTreeMap<String, ServerBinaryBlobView>) {
        for (_, blob) in blobs {
            self.cache_blob_view(blob);
        }
    }

    fn cache_projected_blobs(&mut self, blobs: Vec<ServerBinaryBlobView>) {
        for blob in blobs {
            self.cache_blob_view(blob);
        }
    }

    fn cache_blob_view(&mut self, blob: ServerBinaryBlobView) {
        self.blobs_by_member_index
            .entry(blob.member_index)
            .or_insert_with(|| blob.clone());
        self.blobs_by_index
            .entry(blob.blob_index)
            .or_insert_with(|| blob.clone());
        self.blobs_by_id
            .entry(blob.blob_id.to_ascii_uppercase())
            .and_modify(|existing| {
                if blob.blob_index < existing.blob_index {
                    *existing = blob.clone();
                }
            })
            .or_insert(blob);
    }

    fn cache_trees(&mut self, trees: BTreeMap<String, ServerBinaryTreeView>) {
        for (_, tree) in trees {
            self.cache_tree_view(tree);
        }
    }

    fn cache_projected_trees(&mut self, trees: Vec<ServerBinaryTreeView>) {
        for tree in trees {
            self.cache_tree_view(tree);
        }
    }

    fn cache_tree_view(&mut self, tree: ServerBinaryTreeView) {
        self.trees_by_index
            .entry(tree.tree_index)
            .or_insert_with(|| tree.clone());
        self.trees_by_id
            .entry(tree.tree_id.to_ascii_uppercase())
            .and_modify(|existing| {
                if tree.tree_index < existing.tree_index {
                    *existing = tree.clone();
                }
            })
            .or_insert(tree);
    }

    pub(crate) fn projected_object_pack(
        &self,
        pack_id: &str,
    ) -> StoreResult<Option<ServerBinaryObjectPackView>> {
        if !self.manifest_object_projection_complete {
            return Err(BinaryDbError::corruption(
                "manifest Object authority projection is incomplete",
            ));
        }
        Ok(self
            .object_packs_by_id
            .get(&pack_id.to_ascii_uppercase())
            .cloned())
    }

    pub(crate) fn projected_tree_pack(
        &self,
        pack_id: &str,
    ) -> StoreResult<Option<ServerBinaryTreePackView>> {
        if !self.manifest_tree_projection_complete {
            return Err(BinaryDbError::corruption(
                "manifest Tree authority projection is incomplete",
            ));
        }
        Ok(self
            .tree_packs_by_id
            .get(&pack_id.to_ascii_uppercase())
            .cloned())
    }

    pub(crate) fn projected_blob(
        &self,
        blob_id: &str,
    ) -> StoreResult<Option<ServerBinaryBlobView>> {
        if !self.manifest_object_projection_complete {
            return Err(BinaryDbError::corruption(
                "manifest Blob authority projection is incomplete",
            ));
        }
        Ok(self.blobs_by_id.get(&blob_id.to_ascii_uppercase()).cloned())
    }

    pub(crate) fn projected_tree(
        &self,
        tree_id: &str,
    ) -> StoreResult<Option<ServerBinaryTreeView>> {
        if !self.manifest_tree_projection_complete {
            return Err(BinaryDbError::corruption(
                "manifest Tree authority projection is incomplete",
            ));
        }
        Ok(self.trees_by_id.get(&tree_id.to_ascii_uppercase()).cloned())
    }

    pub(crate) fn projected_blobs_for_object_pack(
        &self,
        pack: &ServerBinaryObjectPackView,
    ) -> StoreResult<Vec<ServerBinaryBlobView>> {
        if !self.manifest_object_projection_complete {
            return Err(BinaryDbError::corruption(
                "manifest Object authority projection is incomplete",
            ));
        }
        let end = pack
            .record
            .first_member_index
            .checked_add(pack.record.member_count)
            .ok_or_else(|| BinaryDbError::corruption("object-pack member range overflow"))?;
        let mut blobs = Vec::with_capacity(pack.record.member_count as usize);
        for member_index in pack.record.first_member_index..end {
            let blob = self
                .blobs_by_member_index
                .get(&member_index)
                .cloned()
                .ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "object pack {} references missing or tombstoned member {member_index}",
                        pack.pack_id
                    ))
                })?;
            if blob.pack_id != pack.pack_id || blob.member.pack_index != pack.pack_index {
                return Err(BinaryDbError::corruption(format!(
                    "object pack {} member {member_index} ownership disagrees with blob {}",
                    pack.pack_id, blob.blob_id
                )));
            }
            blobs.push(blob);
        }
        Ok(blobs)
    }

    pub(crate) fn projected_trees_for_tree_pack(
        &self,
        pack: &ServerBinaryTreePackView,
    ) -> StoreResult<Vec<ServerBinaryTreeView>> {
        if !self.manifest_tree_projection_complete {
            return Err(BinaryDbError::corruption(
                "manifest Tree authority projection is incomplete",
            ));
        }
        let end = pack
            .record
            .first_tree_index
            .checked_add(pack.record.tree_count)
            .ok_or_else(|| BinaryDbError::corruption("tree-pack Tree range overflow"))?;
        let mut trees = Vec::with_capacity(pack.record.tree_count as usize);
        for tree_index in pack.record.first_tree_index..end {
            let tree = self
                .trees_by_index
                .get(&tree_index)
                .cloned()
                .ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "tree pack {} references missing or tombstoned tree index {tree_index}",
                        pack.pack_id
                    ))
                })?;
            if tree.pack_id != pack.pack_id || tree.pack_index != pack.pack_index {
                return Err(BinaryDbError::corruption(format!(
                    "tree pack {} logical range crosses pack {}",
                    pack.pack_id, tree.pack_id
                )));
            }
            trees.push(tree);
        }
        Ok(trees)
    }

    pub(crate) fn projected_tree_for_pack_entry_ordinal(
        &self,
        pack: &ServerBinaryTreePackView,
        physical_ordinal: u32,
    ) -> StoreResult<ServerBinaryTreeView> {
        let mut found = None;
        for tree in self.projected_trees_for_tree_pack(pack)? {
            if tree.record.pack_entry_ordinal == physical_ordinal && found.replace(tree).is_some() {
                return Err(BinaryDbError::corruption(format!(
                    "tree pack {} repeats physical ordinal {physical_ordinal}",
                    pack.pack_id
                )));
            }
        }
        found.ok_or_else(|| {
            BinaryDbError::corruption(format!(
                "tree pack {} has no physical entry ordinal {physical_ordinal}",
                pack.pack_id
            ))
        })
    }

    fn cache_normalized_tree_authority(
        &mut self,
        access: &impl ReadAccess,
        tree_count: u32,
        entry_count: u32,
    ) -> StoreResult<()> {
        let ranges = access
            .read_records(tree_entry_range_file(), 0, tree_count)?
            .into_iter()
            .map(|raw| decode_tree_entry_range(&raw))
            .collect::<StoreResult<Vec<_>>>()?;
        if ranges.len() != tree_count as usize {
            return Err(BinaryDbError::corruption(
                "tree entry range batch read returned a misaligned result",
            ));
        }
        let entries = access
            .read_records(tree_entry_file(), 0, entry_count)?
            .into_iter()
            .map(|raw| decode_tree_entry(&raw))
            .collect::<StoreResult<Vec<_>>>()?;
        if entries.len() != entry_count as usize {
            return Err(BinaryDbError::corruption(
                "tree entry batch read returned a misaligned result",
            ));
        }
        self.normalized_entry_ranges = Some(ranges);
        self.normalized_entries = Some(entries);
        self.tree_name_payload_body = Some(access.read_payload_body(tree_name_payload_file())?);
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn archive_open_count(&self) -> usize {
        self.archive_open_count
    }

    #[cfg(test)]
    pub(crate) fn cached_zstd_chunk_count(&self) -> usize {
        self.archives
            .values()
            .map(TreePackEntryArchive::cached_zstd_chunk_count)
            .sum()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServerBinaryTreePackView {
    pub pack_index: u32,
    pub pack_id: String,
    pub record: ServerBinaryTreePackRecord,
}

#[derive(Clone, Debug)]
pub struct ServerBinaryRepositoryContentStore<D>
where
    D: ServerRemoteBinaryDb + Clone,
{
    db: D,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ServerBinaryRemoteSyncLineWrite {
    Create,
    Update,
}

impl<D> ServerBinaryRepositoryContentStore<D>
where
    D: ServerRemoteBinaryDb + Clone,
{
    pub fn new(db: D) -> Self {
        Self { db }
    }

    pub fn object_pack_path(&self, pack_id: &str) -> PathBuf {
        ServerRemoteBinaryDb::authority_root(&self.db)
            .as_path()
            .join(".ait/objects/packs")
            .join(format!("{pack_id}.zstpack"))
    }

    pub fn tree_pack_path(&self, pack_id: &str) -> PathBuf {
        ServerRemoteBinaryDb::authority_root(&self.db)
            .as_path()
            .join(".ait/objects/tree-packs")
            .join(format!("{pack_id}.zstpack"))
    }

    pub(crate) fn manifest_object_read_cache_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
    ) -> StoreResult<ServerBinaryTreeReadCache> {
        let packs = bulk_object_pack_views(read)?;
        let blobs = bulk_blob_views(read, &packs)?
            .into_iter()
            .filter(|blob| blob.record.blob_meta & META_TOMBSTONE == 0)
            .collect();
        let mut cache = ServerBinaryTreeReadCache::default();
        cache.cache_object_packs(packs);
        cache.cache_projected_blobs(blobs);
        cache.manifest_object_projection_complete = true;
        Ok(cache)
    }

    pub(crate) fn manifest_tree_read_cache_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
    ) -> StoreResult<ServerBinaryTreeReadCache> {
        let mut cache = self.manifest_object_read_cache_with_read(read)?;
        let packs = bulk_tree_pack_views(read)?;
        let trees = bulk_tree_views(read, &packs)?
            .into_iter()
            .filter(|tree| tree.record.tree_meta & META_TOMBSTONE == 0)
            .collect();
        cache.cache_tree_packs(packs);
        cache.cache_projected_trees(trees);

        let tree_count = optional_record_count(read, tree_file())?;
        let range_count = optional_record_count(read, tree_entry_range_file())?;
        if range_count != tree_count {
            return Err(BinaryDbError::corruption(format!(
                "tree_entry_range.bin count {range_count} disagrees with tree.bin count {tree_count}"
            )));
        }
        let entry_count = optional_record_count(read, tree_entry_file())?;
        if tree_count == 0 {
            if entry_count != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "tree_entry.bin has {entry_count} rows without Trees"
                )));
            }
            cache.normalized_entry_ranges = Some(Vec::new());
            cache.normalized_entries = Some(Vec::new());
            cache.tree_name_payload_body = Some(Vec::new());
        } else {
            cache.cache_normalized_tree_authority(read, tree_count, entry_count)?;
        }
        cache.manifest_tree_projection_complete = true;
        Ok(cache)
    }

    pub(crate) fn validate_manifest_identity_indexes_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        cache: &ServerBinaryTreeReadCache,
        object_pack_ids: &BTreeSet<String>,
        tree_pack_ids: &BTreeSet<String>,
        blob_ids: &BTreeSet<String>,
        tree_ids: &BTreeSet<String>,
    ) -> StoreResult<()> {
        validate_selected_projection_index(
            read,
            object_pack_index(),
            object_pack_ids,
            &cache.object_packs_by_id,
            |id| prefixed_hash48_index_key(id, "PCK-").map(|key| key.to_vec()),
            |pack| pack.pack_index,
            "Object Pack",
        )?;
        validate_selected_projection_index(
            read,
            tree_pack_index(),
            tree_pack_ids,
            &cache.tree_packs_by_id,
            |id| prefixed_hash48_index_key(id, "TPK-").map(|key| key.to_vec()),
            |pack| pack.pack_index,
            "Tree Pack",
        )?;
        validate_selected_projection_index(
            read,
            blob_index_file(),
            blob_ids,
            &cache.blobs_by_id,
            |id| prefixed_hex_key(id, "BLB-", 10),
            |blob| blob.blob_index,
            "Blob",
        )?;
        validate_selected_projection_index(
            read,
            tree_index_file(),
            tree_ids,
            &cache.trees_by_id,
            |id| prefixed_hex_key(id, "TRE-", 10),
            |tree| tree.tree_index,
            "Tree",
        )
    }

    pub fn object_pack(&self, pack_id: &str) -> StoreResult<Option<ServerBinaryObjectPackView>> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        object_pack_with_read(&read, pack_id)
    }

    pub fn object_pack_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        pack_id: &str,
    ) -> StoreResult<Option<ServerBinaryObjectPackView>> {
        object_pack_with_read(read, pack_id)
    }

    pub fn blobs_for_object_pack_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        pack_id: &str,
    ) -> StoreResult<Vec<ServerBinaryBlobView>> {
        let Some(pack) = object_pack_with_read(read, pack_id)? else {
            return Ok(Vec::new());
        };
        let mut blobs = Vec::with_capacity(pack.record.member_count as usize);
        for offset in 0..pack.record.member_count {
            let member_index = pack
                .record
                .first_member_index
                .checked_add(offset)
                .ok_or_else(|| BinaryDbError::corruption("object-pack member index overflow"))?;
            let member = decode_object_pack_member(
                &read.read_record(object_pack_member_file(), member_index)?,
            )?;
            if member.pack_index != pack.pack_index {
                return Err(BinaryDbError::corruption(format!(
                    "object-pack member {member_index} points to pack index {}, expected {}",
                    member.pack_index, pack.pack_index
                )));
            }
            let blob = blob_at(read, member.blob_index)?;
            if blob.record.blob_meta & META_TOMBSTONE != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "object pack {pack_id} references tombstoned blob {}",
                    blob.blob_id
                )));
            }
            if blob.member_index != member_index || blob.pack_id != pack.pack_id {
                return Err(BinaryDbError::corruption(format!(
                    "object pack {pack_id} member {member_index} ownership disagrees with blob {}",
                    blob.blob_id
                )));
            }
            blobs.push(blob);
        }
        Ok(blobs)
    }

    pub fn tree_pack(&self, pack_id: &str) -> StoreResult<Option<ServerBinaryTreePackView>> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        tree_pack_with_read(&read, pack_id)
    }

    pub fn tree_pack_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        pack_id: &str,
    ) -> StoreResult<Option<ServerBinaryTreePackView>> {
        tree_pack_with_read(read, pack_id)
    }

    pub fn trees_for_tree_pack_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        pack_id: &str,
    ) -> StoreResult<Vec<ServerBinaryTreeView>> {
        let Some(pack) = tree_pack_with_read(read, pack_id)? else {
            return Ok(Vec::new());
        };
        let mut trees = Vec::with_capacity(pack.record.tree_count as usize);
        for offset in 0..pack.record.tree_count {
            let tree_index = pack
                .record
                .first_tree_index
                .checked_add(offset)
                .ok_or_else(|| BinaryDbError::corruption("tree-pack tree index overflow"))?;
            let record = decode_tree(&read.read_record(tree_file(), tree_index)?)?;
            if record.tree_meta & META_TOMBSTONE != 0 {
                return Err(BinaryDbError::corruption(format!(
                    "tree pack {pack_id} references tombstoned tree index {tree_index}"
                )));
            }
            trees.push(ServerBinaryTreeView {
                tree_index,
                tree_id: format!("TRE-{}", hex_upper(&record.tree_hash80)),
                record,
                pack_index: pack.pack_index,
                pack_id: pack.pack_id.clone(),
                pack: pack.record.clone(),
            });
        }
        Ok(trees)
    }

    pub fn tree_pack_at_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        pack_index: u32,
    ) -> StoreResult<ServerBinaryTreePackView> {
        let view = tree_pack_at(read, pack_index)?;
        if view.record.pack_meta & META_TOMBSTONE != 0 {
            return Err(BinaryDbError::corruption(format!(
                "snapshot references tombstoned tree pack index {pack_index}"
            )));
        }
        Ok(view)
    }

    pub fn blob(&self, blob_id: &str) -> StoreResult<Option<ServerBinaryBlobView>> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        blob_with_read(&read, blob_id)
    }

    pub fn blob_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        blob_id: &str,
    ) -> StoreResult<Option<ServerBinaryBlobView>> {
        blob_with_read(read, blob_id)
    }

    pub fn blobs_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        blob_ids: &BTreeSet<String>,
    ) -> StoreResult<BTreeMap<String, ServerBinaryBlobView>> {
        let normalized_ids = blob_ids
            .iter()
            .map(|blob_id| blob_id.to_ascii_uppercase())
            .collect::<BTreeSet<_>>();
        blobs_with_access(read, &normalized_ids)
    }

    pub fn blob_bytes_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        blob_id: &str,
    ) -> StoreResult<Option<Vec<u8>>> {
        let mut visited = HashSet::new();
        self.blob_bytes_with_read_inner(read, blob_id, &mut visited)
    }

    pub fn tree(&self, tree_id: &str) -> StoreResult<Option<ServerBinaryTreeView>> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        tree_with_read(&read, tree_id)
    }

    pub fn tree_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        tree_id: &str,
    ) -> StoreResult<Option<ServerBinaryTreeView>> {
        tree_with_read(read, tree_id)
    }

    pub fn trees_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        tree_ids: &BTreeSet<String>,
    ) -> StoreResult<BTreeMap<String, ServerBinaryTreeView>> {
        let normalized_ids = tree_ids
            .iter()
            .map(|tree_id| tree_id.to_ascii_uppercase())
            .collect::<BTreeSet<_>>();
        trees_with_access(read, &normalized_ids)
    }

    pub fn tree_at_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        tree_index: u32,
    ) -> StoreResult<ServerBinaryTreeView> {
        let view = tree_at(read, tree_index)?;
        if view.record.tree_meta & META_TOMBSTONE != 0 {
            return Err(BinaryDbError::corruption(format!(
                "snapshot references tombstoned tree index {tree_index}"
            )));
        }
        Ok(view)
    }

    pub fn tree_for_pack_entry_ordinal_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        pack: &ServerBinaryTreePackView,
        physical_ordinal: u32,
    ) -> StoreResult<ServerBinaryTreeView> {
        tree_for_pack_entry_ordinal(read, pack, physical_ordinal)
    }

    pub fn tree_entries_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        tree_id: &str,
    ) -> StoreResult<Vec<ServerBinaryTreeEntryView>> {
        let mut cache = ServerBinaryTreeReadCache::default();
        self.tree_entries_with_read_cache(read, tree_id, &mut cache)
    }

    pub fn tree_entries_with_read_cache(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        tree_id: &str,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> StoreResult<Vec<ServerBinaryTreeEntryView>> {
        let Some(tree) = tree_with_access(read, tree_id)? else {
            return Ok(Vec::new());
        };
        self.tree_entries_for_view_with_cache(read, &tree, cache)
    }

    pub fn tree_entries_for_tree_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        tree: &ServerBinaryTreeView,
    ) -> StoreResult<Vec<ServerBinaryTreeEntryView>> {
        let mut cache = ServerBinaryTreeReadCache::default();
        self.tree_entries_for_tree_with_read_cache(read, tree, &mut cache)
    }

    pub fn tree_entries_for_tree_with_read_cache(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        tree: &ServerBinaryTreeView,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> StoreResult<Vec<ServerBinaryTreeEntryView>> {
        self.tree_entries_for_view_with_cache(read, tree, cache)
    }

    pub(crate) fn projected_tree_entries_for_tree_with_read_cache(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        tree: &ServerBinaryTreeView,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> StoreResult<Vec<ServerBinaryTreeEntryView>> {
        if !cache.manifest_tree_projection_complete {
            return Err(BinaryDbError::corruption(
                "manifest normalized Tree projection is incomplete",
            ));
        }
        normalized_tree_entries(read, tree, cache)
    }

    pub(crate) fn tree_pack_index_metadata_with_read_cache(
        &self,
        pack: &ServerBinaryTreePackView,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> StoreResult<(JsonValue, String)> {
        let archive = self.tree_pack_archive_with_read_cache(pack, cache)?;
        archive
            .index_json_and_checksum()
            .map_err(|error| BinaryDbError::corruption(error))
    }

    pub(crate) fn tree_pack_tree_checksum_with_read_cache(
        &self,
        pack: &ServerBinaryTreePackView,
        tree_id: &str,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> StoreResult<String> {
        let archive = self.tree_pack_archive_with_read_cache(pack, cache)?;
        archive
            .tree_checksum(tree_id)
            .map(str::to_string)
            .ok_or_else(|| {
                BinaryDbError::corruption(format!(
                    "tree pack {} has no checksum for {tree_id}",
                    pack.pack_id
                ))
            })
    }

    pub fn snapshot_file_map_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        snapshot: &ServerBinarySnapshotRecord,
    ) -> StoreResult<BTreeMap<String, ServerBinarySnapshotFileIdentity>> {
        let mut cache = ServerBinaryTreeReadCache::default();
        self.snapshot_file_map_with_read_cache(read, snapshot, &mut cache)
    }

    pub fn snapshot_file_map_with_read_cache(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        snapshot: &ServerBinarySnapshotRecord,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> StoreResult<BTreeMap<String, ServerBinarySnapshotFileIdentity>> {
        self.snapshot_file_map_with_access(read, snapshot, cache)
    }

    fn snapshot_file_map_with_access(
        &self,
        access: &impl ReadAccess,
        snapshot: &ServerBinarySnapshotRecord,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> StoreResult<BTreeMap<String, ServerBinarySnapshotFileIdentity>> {
        let Some(root_tree) = snapshot_root_tree_with_access(access, snapshot)? else {
            return Ok(BTreeMap::new());
        };
        let mut active_trees = BTreeSet::new();
        let mut files = BTreeMap::new();
        collect_snapshot_file_identities(
            self,
            access,
            &root_tree.tree_id,
            "",
            &mut active_trees,
            &mut files,
            cache,
        )?;
        let actual_file_count = u32::try_from(files.len())
            .map_err(|_| BinaryDbError::corruption("snapshot file count exceeds u32"))?;
        let actual_total_bytes = files.values().try_fold(0_u64, |total, file| {
            total
                .checked_add(file.size_bytes)
                .ok_or_else(|| BinaryDbError::corruption("snapshot total bytes overflow"))
        })?;
        if actual_file_count != snapshot.file_count || actual_total_bytes != snapshot.total_bytes {
            return Err(BinaryDbError::corruption(format!(
                "snapshot Tree aggregates ({actual_file_count}, {actual_total_bytes}) disagree with fixed record ({}, {})",
                snapshot.file_count, snapshot.total_bytes
            )));
        }
        Ok(files)
    }

    pub fn object_packs(&self) -> StoreResult<Vec<ServerBinaryObjectPackView>> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let count = optional_record_count(&read, object_pack_file())?;
        (0..count)
            .map(|index| object_pack_at(&read, index))
            .filter_map(|value| match value {
                Ok(view) if view.record.pack_meta & META_TOMBSTONE == 0 => Some(Ok(view)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    pub fn tree_packs(&self) -> StoreResult<Vec<ServerBinaryTreePackView>> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        self.tree_packs_with_read(&read)
    }

    pub fn tree_packs_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
    ) -> StoreResult<Vec<ServerBinaryTreePackView>> {
        let count = optional_record_count(read, tree_pack_file())?;
        (0..count)
            .map(|index| tree_pack_at(read, index))
            .filter_map(|value| match value {
                Ok(view) if view.record.pack_meta & META_TOMBSTONE == 0 => Some(Ok(view)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    pub fn blobs(&self) -> StoreResult<Vec<ServerBinaryBlobView>> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let count = optional_record_count(&read, blob_file())?;
        (0..count)
            .map(|index| blob_at(&read, index))
            .filter_map(|value| match value {
                Ok(view) if view.record.blob_meta & META_TOMBSTONE == 0 => Some(Ok(view)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    pub fn trees(&self) -> StoreResult<Vec<ServerBinaryTreeView>> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let count = optional_record_count(&read, tree_file())?;
        (0..count)
            .map(|index| tree_at(&read, index))
            .filter_map(|value| match value {
                Ok(view) if view.record.tree_meta & META_TOMBSTONE == 0 => Some(Ok(view)),
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect()
    }

    fn blob_bytes_with_read_inner(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        blob_id: &str,
        visited: &mut HashSet<String>,
    ) -> StoreResult<Option<Vec<u8>>> {
        let visit_key = blob_id.to_ascii_lowercase();
        if !visited.insert(visit_key.clone()) {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB blob delta chain contains a cycle at {blob_id}"
            )));
        }
        let result = (|| {
            let Some(blob) = blob_with_read(read, blob_id)? else {
                return Ok(None);
            };
            if blob.record.hash_kind != SHA256_HASH_KIND {
                return Err(BinaryDbError::corruption(format!(
                    "Binary DB blob {blob_id} uses unsupported hash kind {}",
                    blob.record.hash_kind
                )));
            }
            if blob.pack.pack_format_kind != ZSTD_PACK_FORMAT_KIND {
                return Err(BinaryDbError::corruption(format!(
                    "Binary DB object pack {} for blob {blob_id} is not canonical zstd",
                    blob.pack_id
                )));
            }
            if usize::from(blob.member.delta_chain_depth) > MAX_DELTA_CHAIN_READ_DEPTH {
                return Err(BinaryDbError::corruption(format!(
                    "Binary DB blob {blob_id} delta chain depth {} exceeds {}",
                    blob.member.delta_chain_depth, MAX_DELTA_CHAIN_READ_DEPTH
                )));
            }

            let member_kind = blob.member.member_meta & 0b0000_0011;
            let mut base_blob_map = BTreeMap::new();
            match (member_kind, blob.base_blob_id.as_deref()) {
                (0, None) => {}
                (1, Some(base_blob_id)) => {
                    let base_bytes = self
                        .blob_bytes_with_read_inner(read, base_blob_id, visited)?
                        .ok_or_else(|| {
                            BinaryDbError::corruption(format!(
                                "Binary DB delta blob {blob_id} is missing base blob {base_blob_id}"
                            ))
                        })?;
                    base_blob_map.insert(base_blob_id.to_string(), base_bytes);
                }
                (0, Some(base_blob_id)) => {
                    return Err(BinaryDbError::corruption(format!(
                        "Binary DB full blob {blob_id} unexpectedly references base blob {base_blob_id}"
                    )));
                }
                (1, None) => {
                    return Err(BinaryDbError::corruption(format!(
                        "Binary DB delta blob {blob_id} has no base blob"
                    )));
                }
                (kind, _) => {
                    return Err(BinaryDbError::corruption(format!(
                        "Binary DB blob {blob_id} has unsupported object-pack member kind {kind}"
                    )));
                }
            }

            let pack_path = self.object_pack_path(&blob.pack_id);
            let pack_bytes = fs::read(&pack_path).map_err(|error| {
                BinaryDbError::io(
                    format!("read Binary DB object pack {}", pack_path.display()),
                    error,
                )
            })?;
            let bytes = read_zstd_object_pack_blob_from_bytes(
                &pack_bytes,
                &blob.blob_id,
                (!base_blob_map.is_empty()).then_some(&base_blob_map),
                MAX_DELTA_CHAIN_READ_DEPTH,
            )
            .map_err(|error| {
                BinaryDbError::corruption(format!(
                    "Binary DB object pack {} cannot resolve blob {}: {error}",
                    blob.pack_id, blob.blob_id
                ))
            })?;
            if bytes.len() as u64 != blob.record.size_bytes {
                return Err(BinaryDbError::corruption(format!(
                    "Binary DB blob {} size {} disagrees with record {}",
                    blob.blob_id,
                    bytes.len(),
                    blob.record.size_bytes
                )));
            }
            let digest = Sha256::digest(&bytes);
            if digest.as_slice() != blob.record.sha256 {
                return Err(BinaryDbError::corruption(format!(
                    "Binary DB blob {} checksum disagrees with blob.bin",
                    blob.blob_id
                )));
            }
            Ok(Some(bytes))
        })();
        visited.remove(&visit_key);
        result
    }

    fn tree_pack_archive_with_read_cache<'a>(
        &self,
        pack: &ServerBinaryTreePackView,
        cache: &'a mut ServerBinaryTreeReadCache,
    ) -> StoreResult<&'a mut TreePackEntryArchive> {
        if pack.record.pack_format_kind != ZSTD_PACK_FORMAT_KIND {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree pack {} is not canonical zstd",
                pack.pack_id
            )));
        }
        if !cache.archives.contains_key(&pack.pack_id) {
            #[cfg(feature = "perfetto-tracing")]
            let _trace =
                crate::perfetto_trace::PerfettoRange::new("ait.server.content.tree_pack.open");
            let pack_path = self.tree_pack_path(&pack.pack_id);
            let pack_path = pack_path.to_str().ok_or_else(|| {
                BinaryDbError::corruption(format!(
                    "Binary DB tree pack path is not UTF-8: {}",
                    pack_path.display()
                ))
            })?;
            let archive =
                TreePackEntryArchive::open_with_format(pack_path, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
                    .map_err(|error| {
                        BinaryDbError::corruption(format!(
                            "Binary DB tree pack {} cannot be opened: {error}",
                            pack.pack_id
                        ))
                    })?;
            cache.archives.insert(pack.pack_id.clone(), archive);
            #[cfg(test)]
            {
                cache.archive_open_count += 1;
            }
        }
        cache.archives.get_mut(&pack.pack_id).ok_or_else(|| {
            BinaryDbError::corruption(format!(
                "Binary DB tree pack {} archive cache entry is missing",
                pack.pack_id
            ))
        })
    }

    fn tree_entries_for_view_with_cache(
        &self,
        access: &impl ReadAccess,
        tree: &ServerBinaryTreeView,
        cache: &mut ServerBinaryTreeReadCache,
    ) -> StoreResult<Vec<ServerBinaryTreeEntryView>> {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.content.tree_pack.member_decode");
        let ordinal = usize::try_from(tree.record.pack_entry_ordinal)
            .map_err(|_| BinaryDbError::corruption("tree pack entry ordinal exceeds usize"))?;
        let pack = ServerBinaryTreePackView {
            pack_index: tree.pack_index,
            pack_id: tree.pack_id.clone(),
            record: tree.pack.clone(),
        };
        let payload = self
            .tree_pack_archive_with_read_cache(&pack, cache)?
            .read_tree_by_ordinal(ordinal)
            .map_err(|error| {
                BinaryDbError::corruption(format!(
                    "Binary DB tree pack {} ordinal {ordinal} cannot be decoded: {error}",
                    tree.pack_id
                ))
            })?;
        if payload.get("tree_id").and_then(JsonValue::as_str) != Some(tree.tree_id.as_str()) {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree pack {} ordinal {ordinal} does not contain {}",
                tree.pack_id, tree.tree_id
            )));
        }
        let mut rows = payload
            .get("rows")
            .and_then(JsonValue::as_array)
            .cloned()
            .ok_or_else(|| {
                BinaryDbError::corruption(format!(
                    "Binary DB tree pack {} tree {} has no rows",
                    tree.pack_id, tree.tree_id
                ))
            })?;
        if usize::try_from(tree.record.entry_count).ok() != Some(rows.len()) {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree {} entry_count {} disagrees with pack member row count {}",
                tree.tree_id,
                tree.record.entry_count,
                rows.len()
            )));
        }
        rows.sort_by(|left, right| {
            left.get("entry_name")
                .and_then(JsonValue::as_str)
                .cmp(&right.get("entry_name").and_then(JsonValue::as_str))
        });
        let mut names = BTreeSet::new();
        let mut entries = Vec::with_capacity(rows.len());
        let mut required_blob_ids = BTreeSet::new();
        let mut required_tree_ids = BTreeSet::new();
        for (offset, row) in rows.iter().enumerate() {
            let entry_index = u32::try_from(offset)
                .map_err(|_| BinaryDbError::corruption("tree entry ordinal exceeds u32"))?;
            let entry_name = required_text(row, "entry_name")?;
            validate_tree_entry_name(&entry_name)?;
            if !names.insert(entry_name.clone()) {
                return Err(BinaryDbError::corruption(format!(
                    "Binary DB tree {} repeats entry name {entry_name:?}",
                    tree.tree_id
                )));
            }
            let entry_type = required_text(row, "entry_type")?;
            tree_entry_kind(&entry_type)?;
            let target_id = required_text(row, "target_id")?;
            let mode = required_text(row, "mode")?;
            tree_entry_mode_bits(&entry_type, &mode)?;
            let size_bytes = match row.get("size_bytes") {
                None | Some(JsonValue::Null) => None,
                Some(value) => Some(value.as_u64().ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "Binary DB tree {} entry {entry_name:?} size_bytes is not u64 or null",
                        tree.tree_id
                    ))
                })?),
            };
            match entry_type.as_str() {
                "blob" => {
                    required_blob_ids.insert(target_id.to_ascii_uppercase());
                }
                "tree" => {
                    if size_bytes.is_some() {
                        return Err(BinaryDbError::corruption(format!(
                            "Binary DB tree {} child-tree entry {entry_name:?} has size_bytes",
                            tree.tree_id
                        )));
                    }
                    required_tree_ids.insert(target_id.to_ascii_uppercase());
                }
                _ => unreachable!("tree_entry_kind already validated the entry type"),
            }
            entries.push(ServerBinaryTreeEntryView {
                entry_index,
                entry_name,
                entry_type,
                target_id,
                size_bytes,
                sha256: None,
                mode,
            });
        }
        let missing_blob_ids = required_blob_ids
            .iter()
            .filter(|blob_id| !cache.blobs_by_id.contains_key(blob_id.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>();
        cache.cache_blobs(blobs_with_access(access, &missing_blob_ids)?);
        let missing_tree_ids = required_tree_ids
            .iter()
            .filter(|tree_id| !cache.trees_by_id.contains_key(tree_id.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>();
        cache.cache_trees(trees_with_access(access, &missing_tree_ids)?);
        for entry in &mut entries {
            let target_id = entry.target_id.to_ascii_uppercase();
            match entry.entry_type.as_str() {
                "blob" => {
                    let blob = cache.blobs_by_id.get(&target_id).ok_or_else(|| {
                        BinaryDbError::corruption(format!(
                            "Binary DB tree {} entry {:?} references missing blob {}",
                            tree.tree_id, entry.entry_name, entry.target_id
                        ))
                    })?;
                    if entry
                        .size_bytes
                        .is_some_and(|size| size != blob.record.size_bytes)
                    {
                        return Err(BinaryDbError::corruption(format!(
                            "Binary DB tree {} entry {:?} size disagrees with blob {}",
                            tree.tree_id, entry.entry_name, entry.target_id
                        )));
                    }
                    entry.sha256 = Some(hex_lower(&blob.record.sha256));
                }
                "tree" => {
                    if !cache.trees_by_id.contains_key(&target_id) {
                        return Err(BinaryDbError::corruption(format!(
                            "Binary DB tree {} entry {:?} references missing tree {}",
                            tree.tree_id, entry.entry_name, entry.target_id
                        )));
                    }
                }
                _ => unreachable!("tree_entry_kind already validated the entry type"),
            }
        }
        let normalized = normalized_tree_entries(access, tree, cache)?;
        if normalized.len() != entries.len() {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree {} normalized entry count disagrees with pack payload",
                tree.tree_id
            )));
        }
        for (fixed, packed) in normalized.iter().zip(&entries) {
            let mode_matches = fixed.entry_type == packed.entry_type
                && tree_entry_modes_match(&fixed.entry_type, &fixed.mode, &packed.mode)?;
            if fixed.entry_index != packed.entry_index
                || fixed.entry_name != packed.entry_name
                || fixed.entry_type != packed.entry_type
                || !fixed.target_id.eq_ignore_ascii_case(&packed.target_id)
                || fixed.size_bytes != packed.size_bytes
                || fixed.sha256 != packed.sha256
                || !mode_matches
            {
                return Err(BinaryDbError::corruption(format!(
                    "Binary DB tree {} normalized entry {:?} disagrees with pack payload",
                    tree.tree_id, fixed.entry_name
                )));
            }
        }
        Ok(normalized)
    }
}

impl<D> ServerBinaryRepositoryContentStore<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub(crate) fn snapshot_file_map_in_write<F>(
        &self,
        write: &BinaryDbWriteTxn<'_, D, F>,
        snapshot: &ServerBinarySnapshotRecord,
    ) -> StoreResult<BTreeMap<String, ServerBinarySnapshotFileIdentity>>
    where
        F: BinaryDbFsyncPolicy,
    {
        let mut cache = ServerBinaryTreeReadCache::default();
        self.snapshot_file_map_with_access(write, snapshot, &mut cache)
    }

    pub(crate) fn prepare_remote_sync_write_set<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        include_object_pack: bool,
        include_tree_pack: bool,
        include_snapshot: bool,
        line_write: Option<ServerBinaryRemoteSyncLineWrite>,
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        let mut record_files = Vec::new();
        let mut payload_files = Vec::new();
        let mut index_files = Vec::new();
        if include_object_pack {
            record_files.extend([object_pack_file(), blob_file(), object_pack_member_file()]);
            index_files.extend([object_pack_index(), blob_index_file()]);
        }
        if include_tree_pack {
            record_files.extend([
                tree_pack_file(),
                tree_file(),
                tree_entry_file(),
                tree_entry_range_file(),
            ]);
            payload_files.push(tree_name_payload_file());
            index_files.extend([tree_pack_index(), tree_index_file()]);
        }
        if include_snapshot {
            record_files.extend([
                ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(),
                ServerBinarySnapshotParentEdgeCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file(
                ),
            ]);
            payload_files
                .push(ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::payload_file());
            index_files
                .push(ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::id_index());
        }
        if let Some(line_write) = line_write {
            record_files
                .push(ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file());
            if line_write == ServerBinaryRemoteSyncLineWrite::Create {
                payload_files
                    .push(ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::payload_file());
                index_files
                    .push(ServerBinaryLineCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::name_index());
            }
        }
        tx.prepare_write_set(&record_files, &payload_files, &index_files)
    }

    pub fn tree_pack_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        pack_id: &str,
    ) -> StoreResult<Option<ServerBinaryTreePackView>>
    where
        F: BinaryDbFsyncPolicy,
    {
        tree_pack_with_access(tx, pack_id)
    }

    pub fn tree_for_pack_entry_ordinal_in_write<F>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        pack: &ServerBinaryTreePackView,
        physical_ordinal: u32,
    ) -> StoreResult<ServerBinaryTreeView>
    where
        F: BinaryDbFsyncPolicy,
    {
        tree_for_pack_entry_ordinal(tx, pack, physical_ordinal)
    }

    pub fn append_object_pack_in_tx<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        pack: &JsonValue,
        locators: &[JsonValue],
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.content.append_object_pack");
        let pack_id = required_text(pack, "pack_id")?;
        if object_pack_with_access(tx, &pack_id)?.is_some() {
            return Ok(());
        }
        let pack_index_value = pack
            .get("pack_index")
            .ok_or_else(|| "object pack metadata is missing pack_index".to_string())?;
        let entries = pack_index_value
            .get("entries")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "object pack index is missing entries".to_string())?;
        let mut locator_by_id = locators
            .iter()
            .map(|locator| Ok((required_text(locator, "blob_id")?, locator.clone())))
            .collect::<StoreResult<BTreeMap<_, _>>>()?;
        if locator_by_id.len() != locators.len() {
            return Err("object pack contains duplicate blob locators".into());
        }

        let mut physical_blob_ids = BTreeSet::new();
        for entry in entries {
            let blob_id = required_text(entry, "blob_id")?;
            if !physical_blob_ids.insert(blob_id.clone()) {
                return Err(
                    format!("object pack {pack_id} contains duplicate blob {blob_id}").into(),
                );
            }
        }
        let unexpected_locator_ids = locator_by_id
            .keys()
            .filter(|blob_id| !physical_blob_ids.contains(*blob_id))
            .cloned()
            .collect::<Vec<_>>();
        if !unexpected_locator_ids.is_empty() {
            return Err(format!(
                "object pack {pack_id} has locators for blobs absent from its physical index: {unexpected_locator_ids:?}"
            )
            .into());
        }
        let pack_created_at = pack.get("created_at").cloned().unwrap_or(JsonValue::Null);
        for entry in entries {
            let blob_id = required_text(entry, "blob_id")?;
            if locator_by_id.contains_key(&blob_id) {
                continue;
            }
            let sha256 = required_text(entry, "checksum")?;
            let size_bytes = entry
                .get("uncompressed_byte_length")
                .and_then(JsonValue::as_u64)
                .ok_or_else(|| {
                    format!(
                        "object pack {pack_id} blob {blob_id} has invalid uncompressed_byte_length"
                    )
                })?;
            let pack_entry_type = required_text(entry, "entry_type")?;
            let pack_chain_depth = entry
                .get("chain_depth")
                .and_then(JsonValue::as_u64)
                .ok_or_else(|| {
                    format!("object pack {pack_id} blob {blob_id} has invalid chain_depth")
                })?;
            locator_by_id.insert(
                blob_id.clone(),
                json!({
                    "blob_id": blob_id,
                    "sha256": sha256,
                    "size_bytes": size_bytes,
                    "pack_entry_type": pack_entry_type,
                    "pack_base_blob_id": entry.get("base_blob_id").cloned().unwrap_or(JsonValue::Null),
                    "pack_chain_depth": pack_chain_depth,
                    "created_at": pack_created_at.clone(),
                }),
            );
        }

        let mut required_blob_ids = BTreeSet::new();
        for entry in entries {
            let blob_id = required_text(entry, "blob_id")?;
            let locator = locator_by_id
                .get(&blob_id)
                .ok_or_else(|| format!("object pack {pack_id} is missing locator for {blob_id}"))?;
            required_blob_ids.insert(blob_id.to_ascii_uppercase());
            if let Some(base_blob_id) = locator
                .get("pack_base_blob_id")
                .and_then(JsonValue::as_str)
                .filter(|value| !value.is_empty())
            {
                required_blob_ids.insert(base_blob_id.to_ascii_uppercase());
            }
        }
        #[cfg(feature = "perfetto-tracing")]
        let existing_lookup_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.content.append_object_pack.existing_lookup",
        );
        let existing_blobs = blobs_with_access(tx, &required_blob_ids)?;
        #[cfg(feature = "perfetto-tracing")]
        drop(existing_lookup_trace);

        #[cfg(feature = "perfetto-tracing")]
        let write_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.content.append_object_pack.write",
        );
        let pack_index = tx.record_count(object_pack_file())?;
        let first_member_index = tx.record_count(object_pack_member_file())?;
        let first_blob_index = tx.record_count(blob_file())?;
        let pack_hash48 = prefixed_hash48(&pack_id, "PCK-")?;
        let pack_record = ServerBinaryObjectPackRecord {
            pack_meta: META_READY,
            pack_format_kind: ZSTD_PACK_FORMAT_KIND,
            pack_hash48,
            first_member_index,
            member_count: u32::try_from(entries.len())
                .map_err(|_| "object pack member count exceeds u32")?,
            total_bytes: pack
                .get("total_bytes")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0),
            created_at_s: timestamp_s(pack.get("created_at"))?,
        };
        let mut blob_indices = BTreeMap::new();
        let mut expected_blob_indices = Vec::with_capacity(entries.len());
        let mut blob_records = Vec::with_capacity(entries.len());
        let mut blob_index_candidates = Vec::with_capacity(entries.len());
        for (offset, entry) in entries.iter().enumerate() {
            let blob_id = required_text(entry, "blob_id")?;
            let locator = locator_by_id
                .get(&blob_id)
                .ok_or_else(|| format!("object pack {pack_id} is missing locator for {blob_id}"))?;
            let sha256 = parse_sha256(required_text(locator, "sha256")?.as_str())?;
            if blob_id_from_sha256(&sha256) != blob_id {
                return Err(format!("blob {blob_id} does not match its sha256").into());
            }
            let size_bytes = locator
                .get("size_bytes")
                .and_then(JsonValue::as_u64)
                .ok_or_else(|| "blob locator is missing size_bytes".to_string())?;
            if let Some(existing) = existing_blobs.get(&blob_id.to_ascii_uppercase()) {
                if existing.record.sha256 != sha256 || existing.record.size_bytes != size_bytes {
                    return Err(BinaryDbError::invalid_domain_data(format!(
                        "blob {blob_id} already exists with different content before new pack {pack_id}"
                    )));
                }
            }
            let member_index = first_member_index
                .checked_add(u32::try_from(offset).map_err(|_| "member offset exceeds u32")?)
                .ok_or_else(|| "member index overflow".to_string())?;
            let blob_index = first_blob_index
                .checked_add(u32::try_from(offset).map_err(|_| "blob offset exceeds u32")?)
                .ok_or_else(|| "blob index overflow".to_string())?;
            let record = ServerBinaryBlobRecord {
                blob_meta: META_HAS_PACK_MEMBER,
                hash_kind: SHA256_HASH_KIND,
                size_bytes,
                pack_member_index_plus1: member_index
                    .checked_add(1)
                    .ok_or_else(|| "member index plus one overflow".to_string())?,
                created_at_s: timestamp_s(locator.get("created_at"))?,
                pruned_at_s: 0,
                sha256,
            };
            expected_blob_indices.push(blob_index);
            blob_records.push(encode_blob(&record));
            blob_index_candidates
                .push((prefixed_hex_key(&blob_id, "BLB-", 10)?.to_vec(), blob_index));
            blob_indices.insert(blob_id.to_ascii_uppercase(), blob_index);
        }

        let mut member_records = Vec::with_capacity(entries.len());
        let mut expected_member_indices = Vec::with_capacity(entries.len());
        for (offset, entry) in entries.iter().enumerate() {
            let blob_id = required_text(entry, "blob_id")?;
            let locator = locator_by_id
                .get(&blob_id)
                .ok_or_else(|| format!("object pack {pack_id} is missing locator for {blob_id}"))?;
            let base_blob_id = locator
                .get("pack_base_blob_id")
                .and_then(JsonValue::as_str)
                .filter(|value| !value.is_empty());
            let base_blob_index_plus1 = match base_blob_id {
                Some(base_id) => {
                    let normalized_base_id = base_id.to_ascii_uppercase();
                    let base_index = match blob_indices.get(&normalized_base_id).copied() {
                        Some(index) => index,
                        None => existing_blobs
                            .get(&normalized_base_id)
                            .map(|view| view.blob_index)
                            .ok_or_else(|| format!("delta base blob {base_id} is missing"))?,
                    };
                    base_index
                        .checked_add(1)
                        .ok_or_else(|| "base blob index overflow".to_string())?
                }
                None => 0,
            };
            let member_kind = usize::from(
                locator.get("pack_entry_type").and_then(JsonValue::as_str) == Some("delta"),
            ) as u8;
            let member = ServerBinaryObjectPackMemberRecord {
                member_meta: ZSTD_MEMBER_COMPRESSION_META | member_kind,
                delta_chain_depth: locator
                    .get("pack_chain_depth")
                    .and_then(JsonValue::as_u64)
                    .and_then(|value| u8::try_from(value).ok())
                    .unwrap_or(0),
                pack_index,
                blob_index: *blob_indices
                    .get(&blob_id.to_ascii_uppercase())
                    .expect("blob index was inserted"),
                base_blob_index_plus1,
            };
            member_records.push(encode_object_pack_member(&member));
            expected_member_indices.push(
                first_member_index
                    .checked_add(u32::try_from(offset).map_err(|_| "member offset exceeds u32")?)
                    .ok_or_else(|| "member index overflow".to_string())?,
            );
        }

        let actual_pack =
            tx.append_record(object_pack_file(), &encode_object_pack(&pack_record))?;
        if actual_pack != pack_index {
            return Err("object pack append index drift".into());
        }
        tx.append_index_candidate(
            object_pack_index(),
            &prefixed_hash48_index_key(&pack_id, "PCK-")?,
            pack_index,
        )?;
        if tx.append_records(blob_file(), &blob_records)? != expected_blob_indices {
            return Err("blob record batch append index drift".into());
        }
        tx.append_index_candidates(blob_index_file(), &blob_index_candidates)?;
        if tx.append_records(object_pack_member_file(), &member_records)? != expected_member_indices
        {
            return Err("object pack member batch append index drift".into());
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(write_trace);
        Ok(())
    }

    pub fn append_tree_pack_in_tx<F>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        pack: &JsonValue,
        locators: &[JsonValue],
    ) -> StoreResult<()>
    where
        F: BinaryDbFsyncPolicy,
    {
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.content.append_tree_pack");
        let pack_id = required_text(pack, "pack_id")?;
        if tree_pack_with_access(tx, &pack_id)?.is_some() {
            return Ok(());
        }
        let pack_format = required_text(pack, "pack_format")?;
        if pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(format!(
                "tree pack {pack_id} format {pack_format} is not the canonical {TREE_PACK_FORMAT_ZSTD_CHUNKED_V1}"
            )
            .into());
        }
        let pack_index_value = pack
            .get("pack_index")
            .ok_or_else(|| "tree pack metadata is missing pack_index".to_string())?;
        let entries = pack_index_value
            .get("trees")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| "tree pack index is missing trees".to_string())?;
        let mut locator_by_id = locators
            .iter()
            .map(|locator| Ok((required_text(locator, "tree_id")?, locator.clone())))
            .collect::<StoreResult<BTreeMap<_, _>>>()?;
        if locator_by_id.len() != locators.len() {
            return Err("tree pack contains duplicate tree locators".into());
        }

        let mut physical_tree_ids = BTreeSet::new();
        for entry in entries {
            let tree_id = required_text(entry, "tree_id")?;
            if !physical_tree_ids.insert(tree_id.clone()) {
                return Err(
                    format!("tree pack {pack_id} contains duplicate tree {tree_id}").into(),
                );
            }
        }
        let unexpected_locator_ids = locator_by_id
            .keys()
            .filter(|tree_id| !physical_tree_ids.contains(*tree_id))
            .cloned()
            .collect::<Vec<_>>();
        if !unexpected_locator_ids.is_empty() {
            return Err(format!(
                "tree pack {pack_id} has locators for trees absent from its physical index: {unexpected_locator_ids:?}"
            )
            .into());
        }
        let pack_created_at = pack.get("created_at").cloned().unwrap_or(JsonValue::Null);
        for entry in entries {
            let tree_id = required_text(entry, "tree_id")?;
            if locator_by_id.contains_key(&tree_id) {
                continue;
            }
            let entry_count = entry
                .get("entry_count")
                .and_then(JsonValue::as_u64)
                .ok_or_else(|| {
                    format!("tree pack {pack_id} tree {tree_id} has invalid entry_count")
                })?;
            locator_by_id.insert(
                tree_id.clone(),
                json!({
                    "tree_id": tree_id,
                    "entry_count": entry_count,
                    "tree_pack_id": pack_id,
                    "created_at": pack_created_at.clone(),
                }),
            );
        }

        #[cfg(feature = "perfetto-tracing")]
        let archive_decode_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.content.append_tree_pack.archive_decode",
        );
        let pack_path = self.tree_pack_path(&pack_id);
        let pack_path_text = pack_path
            .to_str()
            .ok_or_else(|| format!("tree pack path is not UTF-8: {}", pack_path.display()))?;
        let mut archive = TreePackEntryArchive::open_with_format(
            pack_path_text,
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )?;
        let mut rows_by_tree = BTreeMap::<String, Vec<JsonValue>>::new();
        let mut ordinal_by_tree = BTreeMap::<String, u32>::new();
        let mut seen_ordinals = BTreeSet::new();
        for (fallback_ordinal, entry) in entries.iter().enumerate() {
            let tree_id = required_text(entry, "tree_id")?;
            let ordinal = entry
                .get("entry_ordinal")
                .and_then(JsonValue::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(fallback_ordinal);
            let ordinal_u32 = u32::try_from(ordinal)
                .map_err(|_| format!("tree pack {pack_id} ordinal exceeds u32"))?;
            if !seen_ordinals.insert(ordinal_u32) {
                return Err(
                    format!("tree pack {pack_id} repeats entry ordinal {ordinal_u32}").into(),
                );
            }
            let tree_payload = archive.read_tree_by_ordinal(ordinal)?;
            if tree_payload.get("tree_id").and_then(JsonValue::as_str) != Some(tree_id.as_str()) {
                return Err(format!(
                    "tree pack {pack_id} ordinal {ordinal} does not contain {tree_id}"
                )
                .into());
            }
            let mut rows = tree_payload
                .get("rows")
                .and_then(JsonValue::as_array)
                .cloned()
                .ok_or_else(|| format!("tree pack {pack_id} tree {tree_id} has no rows"))?;
            rows.sort_by(|left, right| {
                left.get("entry_name")
                    .and_then(JsonValue::as_str)
                    .cmp(&right.get("entry_name").and_then(JsonValue::as_str))
            });
            let locator = locator_by_id
                .get(&tree_id)
                .ok_or_else(|| format!("tree pack {pack_id} is missing locator for {tree_id}"))?;
            let expected_count = locator
                .get("entry_count")
                .and_then(JsonValue::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| "tree locator is missing entry_count".to_string())?;
            if usize::try_from(expected_count).ok() != Some(rows.len()) {
                return Err(format!(
                    "tree {tree_id} entry_count {expected_count} does not match {} tree entries",
                    rows.len()
                )
                .into());
            }
            if rows_by_tree.insert(tree_id, rows).is_some() {
                return Err(format!("tree pack {pack_id} contains a duplicate tree").into());
            }
            ordinal_by_tree.insert(required_text(entry, "tree_id")?, ordinal_u32);
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(archive_decode_trace);

        #[cfg(feature = "perfetto-tracing")]
        let dependency_lookup_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.content.append_tree_pack.dependency_lookup",
        );
        let mut referenced_blob_ids = BTreeSet::new();
        let mut referenced_tree_ids = BTreeSet::new();
        for entry in entries {
            referenced_tree_ids.insert(required_text(entry, "tree_id")?.to_ascii_uppercase());
        }
        for rows in rows_by_tree.values() {
            for row in rows {
                let target_id = required_text(row, "target_id")?;
                match tree_entry_kind(required_text(row, "entry_type")?.as_str())? {
                    0 => {
                        referenced_blob_ids.insert(target_id.to_ascii_uppercase());
                    }
                    1 => {
                        referenced_tree_ids.insert(target_id.to_ascii_uppercase());
                    }
                    _ => unreachable!("tree_entry_kind only returns blob or tree"),
                }
            }
        }
        let existing_blobs = blobs_with_access(tx, &referenced_blob_ids)?;
        let existing_trees = trees_with_access(tx, &referenced_tree_ids)?;
        #[cfg(feature = "perfetto-tracing")]
        drop(dependency_lookup_trace);

        #[cfg(feature = "perfetto-tracing")]
        let prepare_trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.content.append_tree_pack.prepare_records",
        );
        let first_tree_index = tx.record_count(tree_file())?;
        let incoming_tree_ids = entries
            .iter()
            .map(|entry| Ok(required_text(entry, "tree_id")?.to_ascii_uppercase()))
            .collect::<StoreResult<BTreeSet<_>>>()?;
        let mut existing_tree_cache = ServerBinaryTreeReadCache::default();
        for entry in entries {
            let tree_id = required_text(entry, "tree_id")?;
            if let Some(existing) = existing_trees.get(&tree_id.to_ascii_uppercase()) {
                validate_existing_tree_content(
                    self,
                    tx,
                    existing,
                    rows_by_tree
                        .get(&tree_id)
                        .ok_or_else(|| format!("tree pack {pack_id} has no rows for {tree_id}"))?,
                    &pack_id,
                    &mut existing_tree_cache,
                )?;
            }
            for row in rows_by_tree
                .get(&tree_id)
                .ok_or_else(|| format!("tree pack {pack_id} has no rows for {tree_id}"))?
            {
                let entry_name = required_text(row, "entry_name")?;
                let entry_kind = tree_entry_kind(required_text(row, "entry_type")?.as_str())?;
                let target_id = required_text(row, "target_id")?;
                tree_entry_mode_bits(
                    required_text(row, "entry_type")?.as_str(),
                    required_text(row, "mode")?.as_str(),
                )?;
                match entry_kind {
                    0 => {
                        let blob = existing_blobs
                            .get(&target_id.to_ascii_uppercase())
                            .ok_or_else(|| {
                                format!("blob {target_id} is missing for tree entry {entry_name}")
                            })?;
                        match row.get("size_bytes") {
                            None | Some(JsonValue::Null) => {}
                            Some(value) if value.as_u64() == Some(blob.record.size_bytes) => {}
                            Some(_) => {
                                return Err(format!(
                                    "blob {target_id} size disagrees for tree entry {entry_name}"
                                )
                                .into())
                            }
                        }
                    }
                    1 => {
                        if row.get("size_bytes").is_some_and(|value| !value.is_null()) {
                            return Err(format!(
                                "child tree {target_id} has size_bytes for entry {entry_name}"
                            )
                            .into());
                        }
                        if !incoming_tree_ids.contains(&target_id.to_ascii_uppercase())
                            && !existing_trees.contains_key(&target_id.to_ascii_uppercase())
                        {
                            return Err(format!(
                                "tree {target_id} is missing for tree entry {entry_name}"
                            )
                            .into());
                        }
                    }
                    _ => unreachable!("tree_entry_kind only returns blob or tree"),
                }
            }
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(prepare_trace);

        #[cfg(feature = "perfetto-tracing")]
        let write_trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.content.append_tree_pack.write");
        let range_count = tx.record_count(tree_entry_range_file())?;
        if range_count != first_tree_index {
            return Err(BinaryDbError::corruption(format!(
                "tree_entry_range.bin count {range_count} disagrees with tree.bin count {first_tree_index}"
            )));
        }
        let first_entry_index = tx.record_count(tree_entry_file())?;
        let incoming_tree_indices = entries
            .iter()
            .enumerate()
            .map(|(offset, entry)| {
                let tree_id = required_text(entry, "tree_id")?.to_ascii_uppercase();
                let tree_index = first_tree_index
                    .checked_add(u32::try_from(offset).map_err(|_| "tree offset exceeds u32")?)
                    .ok_or_else(|| "tree index overflow".to_string())?;
                Ok((tree_id, tree_index))
            })
            .collect::<StoreResult<BTreeMap<_, _>>>()?;
        let mut next_entry_index = first_entry_index;
        let mut entry_records = Vec::new();
        let mut expected_entry_indices = Vec::new();
        let mut range_records = Vec::with_capacity(entries.len());
        for entry in entries {
            let tree_id = required_text(entry, "tree_id")?;
            range_records.push(encode_tree_entry_range(ServerBinaryTreeEntryRangeRecord {
                first_entry_index: next_entry_index,
            }));
            let rows = rows_by_tree
                .get(&tree_id)
                .ok_or_else(|| format!("tree pack {pack_id} has no rows for {tree_id}"))?;
            for row in rows {
                let entry_name = required_text(row, "entry_name")?;
                validate_tree_entry_name(&entry_name)?;
                let name_len = u8::try_from(entry_name.len()).map_err(|_| {
                    BinaryDbError::invalid_domain_data(format!(
                        "tree entry name exceeds 255 bytes: {entry_name:?}"
                    ))
                })?;
                let name_range =
                    tx.append_payload(tree_name_payload_file(), entry_name.as_bytes())?;
                let entry_type = required_text(row, "entry_type")?;
                let entry_kind = tree_entry_kind(&entry_type)?;
                let target_id = required_text(row, "target_id")?;
                let target_index = match entry_kind {
                    0 => existing_blobs
                        .get(&target_id.to_ascii_uppercase())
                        .map(|blob| blob.blob_index)
                        .ok_or_else(|| {
                            BinaryDbError::invalid_domain_data(format!(
                                "blob {target_id} is missing for normalized tree entry {entry_name}"
                            ))
                        })?,
                    1 => incoming_tree_indices
                        .get(&target_id.to_ascii_uppercase())
                        .copied()
                        .or_else(|| {
                            existing_trees
                                .get(&target_id.to_ascii_uppercase())
                                .map(|tree| tree.tree_index)
                        })
                        .ok_or_else(|| {
                            BinaryDbError::invalid_domain_data(format!(
                                "tree {target_id} is missing for normalized tree entry {entry_name}"
                            ))
                        })?,
                    _ => unreachable!("tree_entry_kind only returns blob or tree"),
                };
                let record = ServerBinaryTreeEntryRecord {
                    entry_meta: entry_kind,
                    name_len,
                    mode_bits: tree_entry_mode_bits(&entry_type, &required_text(row, "mode")?)?,
                    name_offset: name_range.payload_offset,
                    target_index,
                };
                entry_records.push(encode_tree_entry(record)?);
                expected_entry_indices.push(next_entry_index);
                next_entry_index = next_entry_index
                    .checked_add(1)
                    .ok_or_else(|| "tree entry index overflow".to_string())?;
            }
        }
        if !entry_records.is_empty()
            && tx.append_records(tree_entry_file(), &entry_records)? != expected_entry_indices
        {
            return Err("tree entry batch append index drift".into());
        }
        let expected_range_indices = (0..u32::try_from(range_records.len())
            .map_err(|_| "tree range count exceeds u32")?)
            .map(|offset| first_tree_index + offset)
            .collect::<Vec<_>>();
        if tx.append_records(tree_entry_range_file(), &range_records)? != expected_range_indices {
            return Err("tree entry range batch append index drift".into());
        }
        for dependency in [TREE_NAME_PAYLOAD_BIN, TREE_ENTRY_BIN, TREE_ENTRY_RANGE_BIN] {
            tx.fsync_policy().sync_file_data(
                &ServerRemoteBinaryDb::authority_root(&self.db)
                    .as_path()
                    .join(dependency),
            )?;
        }
        let mut tree_records = Vec::with_capacity(entries.len());
        let mut tree_index_candidates = Vec::with_capacity(entries.len());
        let mut expected_tree_indices = Vec::with_capacity(entries.len());
        for (offset, entry) in entries.iter().enumerate() {
            let tree_id = required_text(entry, "tree_id")?;
            let locator = locator_by_id
                .get(&tree_id)
                .ok_or_else(|| format!("tree pack {pack_id} is missing locator for {tree_id}"))?;
            let record = ServerBinaryTreeRecord {
                tree_meta: 0,
                pack_entry_ordinal: *ordinal_by_tree.get(&tree_id).ok_or_else(|| {
                    format!("tree pack {pack_id} has no entry ordinal for {tree_id}")
                })?,
                entry_count: locator
                    .get("entry_count")
                    .and_then(JsonValue::as_u64)
                    .and_then(|value| u32::try_from(value).ok())
                    .ok_or_else(|| "tree locator is missing entry_count".to_string())?,
                tree_hash80: prefixed_hex_key_array::<10>(&tree_id, "TRE-")?,
            };
            let tree_index = first_tree_index
                .checked_add(u32::try_from(offset).map_err(|_| "tree offset exceeds u32")?)
                .ok_or_else(|| "tree index overflow".to_string())?;
            tree_records.push(encode_tree(&record));
            tree_index_candidates.push((record.tree_hash80.to_vec(), tree_index));
            expected_tree_indices.push(tree_index);
        }
        if tx.append_records(tree_file(), &tree_records)? != expected_tree_indices {
            return Err("tree record batch append index drift".into());
        }
        tx.append_index_candidates(tree_index_file(), &tree_index_candidates)?;
        let pack_index = tx.record_count(tree_pack_file())?;
        let sparse_ordinals = entries.iter().enumerate().any(|(logical, entry)| {
            entry
                .get("entry_ordinal")
                .and_then(JsonValue::as_u64)
                .is_some_and(|physical| physical != logical as u64)
        });
        let record = ServerBinaryTreePackRecord {
            pack_meta: META_READY
                | if sparse_ordinals {
                    META_SPARSE_PHYSICAL_ORDINALS
                } else {
                    0
                },
            pack_format_kind: ZSTD_PACK_FORMAT_KIND,
            pack_hash48: prefixed_hash48(&pack_id, "TPK-")?,
            first_tree_index,
            tree_count: u32::try_from(entries.len()).map_err(|_| "tree count exceeds u32")?,
            total_bytes: pack
                .get("total_bytes")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0),
            created_at_s: timestamp_s(pack.get("created_at"))?,
        };
        if tx.append_record(tree_pack_file(), &encode_tree_pack(&record))? != pack_index {
            return Err("tree pack append index drift".into());
        }
        tx.append_index_candidate(
            tree_pack_index(),
            &prefixed_hash48_index_key(&pack_id, "TPK-")?,
            pack_index,
        )?;
        #[cfg(feature = "perfetto-tracing")]
        drop(write_trace);
        Ok(())
    }
}

fn validate_existing_tree_content<D>(
    store: &ServerBinaryRepositoryContentStore<D>,
    access: &impl ReadAccess,
    existing: &ServerBinaryTreeView,
    incoming_rows: &[JsonValue],
    incoming_pack_id: &str,
    cache: &mut ServerBinaryTreeReadCache,
) -> StoreResult<()>
where
    D: ServerRemoteBinaryDb + Clone,
{
    let existing_rows = store.tree_entries_for_view_with_cache(access, existing, cache)?;
    if existing_rows.len() != incoming_rows.len() {
        return Err(BinaryDbError::invalid_domain_data(format!(
            "tree {} already exists with {} entries, but new pack {incoming_pack_id} carries {}",
            existing.tree_id,
            existing_rows.len(),
            incoming_rows.len()
        )));
    }
    for (existing_row, incoming_row) in existing_rows.iter().zip(incoming_rows) {
        let entry_name = required_text(incoming_row, "entry_name")?;
        let entry_type = required_text(incoming_row, "entry_type")?;
        let target_id = required_text(incoming_row, "target_id")?;
        let mode = required_text(incoming_row, "mode")?;
        tree_entry_mode_bits(&entry_type, &mode)?;
        let size_bytes = match incoming_row.get("size_bytes") {
            None | Some(JsonValue::Null) => None,
            Some(value) => Some(value.as_u64().ok_or_else(|| {
                BinaryDbError::invalid_domain_data(format!(
                    "tree {} incoming entry {entry_name:?} size_bytes is not u64 or null",
                    existing.tree_id
                ))
            })?),
        };
        let mode_matches = existing_row.entry_type == entry_type
            && tree_entry_modes_match(&entry_type, &existing_row.mode, &mode)?;
        let matches = existing_row.entry_name == entry_name
            && existing_row.entry_type == entry_type
            && existing_row.target_id.eq_ignore_ascii_case(&target_id)
            && existing_row.size_bytes == size_bytes
            && mode_matches;
        if !matches {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "tree {} already exists with different content before new pack {incoming_pack_id}",
                existing.tree_id
            )));
        }
    }
    Ok(())
}

fn blob_file() -> BinaryFileId {
    BinaryFileId::new(
        BLOB_BIN,
        1,
        SERVER_BLOB_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    )
}
fn blob_index_file() -> BinaryIndexId {
    BinaryIndexId::new_fixed(BLOB_ID_IDX, 1, 10, true, BinaryDbFileFamily::Content)
}
fn object_pack_file() -> BinaryFileId {
    BinaryFileId::new(
        OBJECT_PACK_BIN,
        1,
        SERVER_OBJECT_PACK_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    )
}
fn object_pack_index() -> BinaryIndexId {
    BinaryIndexId::new_fixed(OBJECT_PACK_ID_IDX, 1, 8, true, BinaryDbFileFamily::Content)
}
fn object_pack_member_file() -> BinaryFileId {
    BinaryFileId::new(
        OBJECT_PACK_MEMBER_BIN,
        1,
        SERVER_OBJECT_PACK_MEMBER_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    )
}
fn tree_pack_file() -> BinaryFileId {
    BinaryFileId::new(
        TREE_PACK_BIN,
        1,
        SERVER_TREE_PACK_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    )
}
fn tree_pack_index() -> BinaryIndexId {
    BinaryIndexId::new_fixed(TREE_PACK_ID_IDX, 1, 8, true, BinaryDbFileFamily::Content)
}
fn tree_file() -> BinaryFileId {
    BinaryFileId::new(
        TREE_BIN,
        1,
        SERVER_TREE_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    )
}
fn tree_index_file() -> BinaryIndexId {
    BinaryIndexId::new_fixed(TREE_ID_IDX, 1, 10, true, BinaryDbFileFamily::Content)
}
fn tree_entry_file() -> BinaryFileId {
    BinaryFileId::new(
        TREE_ENTRY_BIN,
        1,
        SERVER_TREE_ENTRY_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    )
}
fn tree_entry_range_file() -> BinaryFileId {
    BinaryFileId::new(
        TREE_ENTRY_RANGE_BIN,
        1,
        SERVER_TREE_ENTRY_RANGE_RECORD_SIZE,
        BinaryDbFileFamily::Content,
    )
}
fn tree_name_payload_file() -> BinaryPayloadFileId {
    BinaryPayloadFileId::new(TREE_NAME_PAYLOAD_BIN, 1, BinaryDbFileFamily::Content)
}

trait ReadAccess {
    fn read_record(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>>;
    fn read_records(
        &self,
        file: BinaryFileId,
        first_index: u32,
        count: u32,
    ) -> StoreResult<Vec<Vec<u8>>>;
    fn lookup(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>>;
    fn lookup_many(
        &self,
        index: BinaryIndexId,
        keys: &[BinaryIndexKeyRef<'_>],
    ) -> StoreResult<Vec<Vec<u32>>>;
    fn count(&self, file: BinaryFileId) -> StoreResult<u32>;
    fn layout(&self, file: BinaryFileId) -> StoreResult<u32>;
    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>>;
    fn read_payload_body(&self, file: BinaryPayloadFileId) -> StoreResult<Vec<u8>>;
}

impl<B: BinaryDb + ?Sized> ReadAccess for BinaryDbReadTxn<'_, B> {
    fn read_record(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>> {
        #[cfg(test)]
        observe_test_content_record_read(&file, index, 1);
        self.read_record(file, index)
    }
    fn read_records(
        &self,
        file: BinaryFileId,
        first_index: u32,
        count: u32,
    ) -> StoreResult<Vec<Vec<u8>>> {
        #[cfg(test)]
        observe_test_content_record_read(&file, first_index, count);
        self.read_records(file, first_index, count)
    }
    fn lookup(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>> {
        self.lookup_index(index, key)
    }
    fn lookup_many(
        &self,
        index: BinaryIndexId,
        keys: &[BinaryIndexKeyRef<'_>],
    ) -> StoreResult<Vec<Vec<u32>>> {
        self.lookup_index_many(index, keys)
    }
    fn count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.record_count(file)
    }
    fn layout(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.layout_id(file)
    }
    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        #[cfg(test)]
        observe_test_content_payload_read(&file, offset, len);
        self.read_payload(file, offset, len)
    }
    fn read_payload_body(&self, file: BinaryPayloadFileId) -> StoreResult<Vec<u8>> {
        let path = self
            .db()
            .authority_root()
            .as_path()
            .join(file.relative_path().as_path());
        let byte_size = self.db().metadata_len(&path)?.ok_or_else(|| {
            BinaryDbError::missing_data(format!("payload file '{}' is missing", file.as_str()))
        })?;
        let body_size = byte_size.checked_sub(4).ok_or_else(|| {
            BinaryDbError::corruption("payload file is missing its layout header")
        })?;
        let body_len = u32::try_from(body_size)
            .map_err(|_| BinaryDbError::corruption("payload body exceeds u32"))?;
        #[cfg(test)]
        observe_test_content_payload_read(&file, 4, body_len);
        self.read_payload(file, 4, body_len)
    }
}

impl<B, F> ReadAccess for BinaryDbWriteTxn<'_, B, F>
where
    B: BinaryDb,
    F: BinaryDbFsyncPolicy,
{
    fn read_record(&self, file: BinaryFileId, index: u32) -> StoreResult<Vec<u8>> {
        self.read_record(file, index)
    }
    fn read_records(
        &self,
        file: BinaryFileId,
        first_index: u32,
        count: u32,
    ) -> StoreResult<Vec<Vec<u8>>> {
        self.read_records(file, first_index, count)
    }
    fn lookup(&self, index: BinaryIndexId, key: &[u8]) -> StoreResult<Vec<u32>> {
        self.lookup_index(index, key)
    }
    fn lookup_many(
        &self,
        index: BinaryIndexId,
        keys: &[BinaryIndexKeyRef<'_>],
    ) -> StoreResult<Vec<Vec<u32>>> {
        self.lookup_index_many(index, keys)
    }
    fn count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.record_count(file)
    }
    fn layout(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.db().layout_id(file)
    }
    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.read_payload(file, offset, len)
    }
    fn read_payload_body(&self, file: BinaryPayloadFileId) -> StoreResult<Vec<u8>> {
        let path = self
            .db()
            .authority_root()
            .as_path()
            .join(file.relative_path().as_path());
        let byte_size = self.db().metadata_len(&path)?.ok_or_else(|| {
            BinaryDbError::missing_data(format!("payload file '{}' is missing", file.as_str()))
        })?;
        let body_size = byte_size.checked_sub(4).ok_or_else(|| {
            BinaryDbError::corruption("payload file is missing its layout header")
        })?;
        let body_len = u32::try_from(body_size)
            .map_err(|_| BinaryDbError::corruption("payload body exceeds u32"))?;
        self.read_payload(file, 4, body_len)
    }
}

fn optional_record_count(access: &impl ReadAccess, file: BinaryFileId) -> StoreResult<u32> {
    match access.layout(file.clone()) {
        Ok(1) => access.count(file),
        Ok(layout) => Err(BinaryDbError::layout_mismatch(format!(
            "unsupported {} layout {layout}",
            file.as_str()
        ))),
        Err(error) if error.kind() == BinaryDbErrorKind::MissingData => Ok(0),
        Err(error) => Err(error),
    }
}

fn validate_selected_projection_index<T, K, I>(
    access: &impl ReadAccess,
    index: BinaryIndexId,
    selected_ids: &BTreeSet<String>,
    projected: &BTreeMap<String, T>,
    key_for_id: K,
    physical_index: I,
    label: &str,
) -> StoreResult<()>
where
    K: Fn(&str) -> StoreResult<Vec<u8>>,
    I: Fn(&T) -> u32,
{
    if selected_ids.is_empty() {
        return Ok(());
    }
    let normalized_ids = selected_ids
        .iter()
        .map(|id| id.to_ascii_uppercase())
        .collect::<Vec<_>>();
    let keys = normalized_ids
        .iter()
        .map(|id| key_for_id(id))
        .collect::<StoreResult<Vec<_>>>()?;
    let key_refs = keys.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let candidates_by_id = access.lookup_many(index, &key_refs)?;
    if candidates_by_id.len() != normalized_ids.len() {
        return Err(BinaryDbError::corruption(format!(
            "Binary DB batch {label} lookup returned a misaligned result"
        )));
    }
    for (id, candidates) in normalized_ids.into_iter().zip(candidates_by_id) {
        let view = projected.get(&id).ok_or_else(|| {
            BinaryDbError::corruption(format!(
                "manifest references missing or tombstoned {label} {id}"
            ))
        })?;
        let expected_index = physical_index(view);
        if !candidates.contains(&expected_index) {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB {label} ID index does not resolve {id} to physical index {expected_index}"
            )));
        }
    }
    Ok(())
}

fn object_pack_with_read<B: BinaryDb + ?Sized>(
    read: &BinaryDbReadTxn<'_, B>,
    id: &str,
) -> StoreResult<Option<ServerBinaryObjectPackView>> {
    object_pack_with_access(read, id)
}
fn object_pack_with_access(
    access: &impl ReadAccess,
    id: &str,
) -> StoreResult<Option<ServerBinaryObjectPackView>> {
    if optional_record_count(access, object_pack_file())? == 0 {
        return Ok(None);
    }
    for index in access.lookup(object_pack_index(), &prefixed_hash48_index_key(id, "PCK-")?)? {
        let view = object_pack_at(access, index)?;
        if view.pack_id.eq_ignore_ascii_case(id) && view.record.pack_meta & META_TOMBSTONE == 0 {
            return Ok(Some(view));
        }
    }
    Ok(None)
}
fn object_pack_at(access: &impl ReadAccess, index: u32) -> StoreResult<ServerBinaryObjectPackView> {
    let record = decode_object_pack(&access.read_record(object_pack_file(), index)?)?;
    Ok(ServerBinaryObjectPackView {
        pack_index: index,
        pack_id: format!("PCK-{:012X}", record.pack_hash48),
        record,
    })
}

fn tree_pack_with_read<B: BinaryDb + ?Sized>(
    read: &BinaryDbReadTxn<'_, B>,
    id: &str,
) -> StoreResult<Option<ServerBinaryTreePackView>> {
    tree_pack_with_access(read, id)
}
fn tree_pack_with_access(
    access: &impl ReadAccess,
    id: &str,
) -> StoreResult<Option<ServerBinaryTreePackView>> {
    if optional_record_count(access, tree_pack_file())? == 0 {
        return Ok(None);
    }
    for index in access.lookup(tree_pack_index(), &prefixed_hash48_index_key(id, "TPK-")?)? {
        let view = tree_pack_at(access, index)?;
        if view.pack_id.eq_ignore_ascii_case(id) && view.record.pack_meta & META_TOMBSTONE == 0 {
            return Ok(Some(view));
        }
    }
    Ok(None)
}
fn tree_pack_at(access: &impl ReadAccess, index: u32) -> StoreResult<ServerBinaryTreePackView> {
    let record = decode_tree_pack(&access.read_record(tree_pack_file(), index)?)?;
    Ok(ServerBinaryTreePackView {
        pack_index: index,
        pack_id: format!("TPK-{:012X}", record.pack_hash48),
        record,
    })
}

fn blob_with_read<B: BinaryDb + ?Sized>(
    read: &BinaryDbReadTxn<'_, B>,
    id: &str,
) -> StoreResult<Option<ServerBinaryBlobView>> {
    blob_with_access(read, id)
}
fn blob_with_access(
    access: &impl ReadAccess,
    id: &str,
) -> StoreResult<Option<ServerBinaryBlobView>> {
    let normalized_id = id.to_ascii_uppercase();
    let mut ids = BTreeSet::new();
    ids.insert(normalized_id.clone());
    Ok(blobs_with_access(access, &ids)?.remove(&normalized_id))
}
fn blobs_with_access(
    access: &impl ReadAccess,
    ids: &BTreeSet<String>,
) -> StoreResult<BTreeMap<String, ServerBinaryBlobView>> {
    let mut views = BTreeMap::new();
    if ids.is_empty() || optional_record_count(access, blob_file())? == 0 {
        return Ok(views);
    }
    let requested_ids = ids.iter().collect::<Vec<_>>();
    let keys = requested_ids
        .iter()
        .map(|id| prefixed_hex_key(id, "BLB-", 10))
        .collect::<StoreResult<Vec<_>>>()?;
    let key_refs = keys.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let candidates_by_id = access.lookup_many(blob_index_file(), &key_refs)?;
    if candidates_by_id.len() != requested_ids.len() {
        return Err(BinaryDbError::corruption(
            "Binary DB batch blob lookup returned a misaligned result",
        ));
    }
    for (id, candidates) in requested_ids.into_iter().zip(candidates_by_id) {
        for index in candidates {
            let view = blob_at(access, index)?;
            if view.blob_id.eq_ignore_ascii_case(id) && view.record.blob_meta & META_TOMBSTONE == 0
            {
                views.insert(id.clone(), view);
                break;
            }
        }
    }
    Ok(views)
}
fn blob_at(access: &impl ReadAccess, index: u32) -> StoreResult<ServerBinaryBlobView> {
    let record = decode_blob(&access.read_record(blob_file(), index)?)?;
    let member_index = record
        .pack_member_index_plus1
        .checked_sub(1)
        .ok_or_else(|| "blob has no pack member".to_string())?;
    let member =
        decode_object_pack_member(&access.read_record(object_pack_member_file(), member_index)?)?;
    if member.blob_index != index {
        return Err("blob and object-pack member indexes disagree".into());
    }
    let pack = object_pack_at(access, member.pack_index)?;
    let base_blob_id = member
        .base_blob_index_plus1
        .checked_sub(1)
        .map(|base| {
            access
                .read_record(blob_file(), base)
                .and_then(|raw| decode_blob(&raw))
                .map(|record| blob_id_from_sha256(&record.sha256))
        })
        .transpose()?;
    Ok(ServerBinaryBlobView {
        blob_index: index,
        blob_id: blob_id_from_sha256(&record.sha256),
        record,
        member_index,
        member,
        pack_id: pack.pack_id,
        pack: pack.record,
        base_blob_id,
    })
}

fn bulk_object_pack_views(
    access: &impl ReadAccess,
) -> StoreResult<Vec<ServerBinaryObjectPackView>> {
    let count = optional_record_count(access, object_pack_file())?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let records = access.read_records(object_pack_file(), 0, count)?;
    if records.len() != count as usize {
        return Err(BinaryDbError::corruption(
            "object_pack.bin bulk read returned a misaligned result",
        ));
    }
    records
        .into_iter()
        .enumerate()
        .map(|(pack_index, raw)| {
            let pack_index = u32::try_from(pack_index)
                .map_err(|_| BinaryDbError::corruption("object-pack index exceeds u32"))?;
            let record = decode_object_pack(&raw)?;
            Ok(ServerBinaryObjectPackView {
                pack_index,
                pack_id: format!("PCK-{:012X}", record.pack_hash48),
                record,
            })
        })
        .collect()
}

fn bulk_blob_views(
    access: &impl ReadAccess,
    packs: &[ServerBinaryObjectPackView],
) -> StoreResult<Vec<ServerBinaryBlobView>> {
    let blob_count = optional_record_count(access, blob_file())?;
    let member_count = optional_record_count(access, object_pack_member_file())?;
    if blob_count == 0 && member_count == 0 {
        return Ok(Vec::new());
    }
    let blob_records = access
        .read_records(blob_file(), 0, blob_count)?
        .into_iter()
        .map(|raw| decode_blob(&raw))
        .collect::<StoreResult<Vec<_>>>()?;
    let member_records = access
        .read_records(object_pack_member_file(), 0, member_count)?
        .into_iter()
        .map(|raw| decode_object_pack_member(&raw))
        .collect::<StoreResult<Vec<_>>>()?;
    if blob_records.len() != blob_count as usize || member_records.len() != member_count as usize {
        return Err(BinaryDbError::corruption(
            "Blob authority bulk read returned a misaligned result",
        ));
    }

    let mut views = Vec::with_capacity(blob_records.len());
    for (blob_index, record) in blob_records.iter().enumerate() {
        let blob_index = u32::try_from(blob_index)
            .map_err(|_| BinaryDbError::corruption("blob index exceeds u32"))?;
        let member_index = record
            .pack_member_index_plus1
            .checked_sub(1)
            .ok_or_else(|| BinaryDbError::corruption("blob has no pack member"))?;
        let member = member_records
            .get(member_index as usize)
            .cloned()
            .ok_or_else(|| BinaryDbError::corruption("blob pack-member index is out of range"))?;
        if member.blob_index != blob_index {
            return Err(BinaryDbError::corruption(
                "blob and object-pack member indexes disagree",
            ));
        }
        let pack = packs.get(member.pack_index as usize).ok_or_else(|| {
            BinaryDbError::corruption("object-pack member pack index is out of range")
        })?;
        let member_range_end = pack
            .record
            .first_member_index
            .checked_add(pack.record.member_count)
            .ok_or_else(|| BinaryDbError::corruption("object-pack member range overflow"))?;
        if member_index < pack.record.first_member_index || member_index >= member_range_end {
            return Err(BinaryDbError::corruption(format!(
                "object pack {} does not own member {member_index}",
                pack.pack_id
            )));
        }
        let base_blob_id = member
            .base_blob_index_plus1
            .checked_sub(1)
            .map(|base_index| {
                blob_records
                    .get(base_index as usize)
                    .map(|base| blob_id_from_sha256(&base.sha256))
                    .ok_or_else(|| BinaryDbError::corruption("base Blob index is out of range"))
            })
            .transpose()?;
        views.push(ServerBinaryBlobView {
            blob_index,
            blob_id: blob_id_from_sha256(&record.sha256),
            record: record.clone(),
            member_index,
            member,
            pack_id: pack.pack_id.clone(),
            pack: pack.record.clone(),
            base_blob_id,
        });
    }
    Ok(views)
}

fn bulk_tree_pack_views(access: &impl ReadAccess) -> StoreResult<Vec<ServerBinaryTreePackView>> {
    let count = optional_record_count(access, tree_pack_file())?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let records = access.read_records(tree_pack_file(), 0, count)?;
    if records.len() != count as usize {
        return Err(BinaryDbError::corruption(
            "tree_pack.bin bulk read returned a misaligned result",
        ));
    }
    records
        .into_iter()
        .enumerate()
        .map(|(pack_index, raw)| {
            let pack_index = u32::try_from(pack_index)
                .map_err(|_| BinaryDbError::corruption("tree-pack index exceeds u32"))?;
            let record = decode_tree_pack(&raw)?;
            Ok(ServerBinaryTreePackView {
                pack_index,
                pack_id: format!("TPK-{:012X}", record.pack_hash48),
                record,
            })
        })
        .collect()
}

fn bulk_tree_views(
    access: &impl ReadAccess,
    packs: &[ServerBinaryTreePackView],
) -> StoreResult<Vec<ServerBinaryTreeView>> {
    let count = optional_record_count(access, tree_file())?;
    if count == 0 {
        return Ok(Vec::new());
    }
    let records = access
        .read_records(tree_file(), 0, count)?
        .into_iter()
        .map(|raw| decode_tree(&raw))
        .collect::<StoreResult<Vec<_>>>()?;
    if records.len() != count as usize {
        return Err(BinaryDbError::corruption(
            "tree.bin bulk read returned a misaligned result",
        ));
    }
    records
        .iter()
        .enumerate()
        .map(|(tree_index, record)| {
            let tree_index = u32::try_from(tree_index)
                .map_err(|_| BinaryDbError::corruption("tree index exceeds u32"))?;
            tree_view_from_preloaded_records(tree_index, record, packs)
        })
        .collect()
}

fn preloaded_blob_views_with_validated_index(
    access: &impl ReadAccess,
) -> StoreResult<BTreeMap<String, ServerBinaryBlobView>> {
    let blob_count = access.count(blob_file())?;
    let blob_records = access
        .read_records(blob_file(), 0, blob_count)?
        .into_iter()
        .map(|raw| decode_blob(&raw))
        .collect::<StoreResult<Vec<_>>>()?;
    let member_count = access.count(object_pack_member_file())?;
    let member_records = access
        .read_records(object_pack_member_file(), 0, member_count)?
        .into_iter()
        .map(|raw| decode_object_pack_member(&raw))
        .collect::<StoreResult<Vec<_>>>()?;
    let pack_count = access.count(object_pack_file())?;
    let pack_records = access
        .read_records(object_pack_file(), 0, pack_count)?
        .into_iter()
        .map(|raw| decode_object_pack(&raw))
        .collect::<StoreResult<Vec<_>>>()?;

    let mut all_views = Vec::with_capacity(blob_records.len());
    for (blob_index, record) in blob_records.iter().enumerate() {
        let blob_index = u32::try_from(blob_index)
            .map_err(|_| BinaryDbError::corruption("blob index exceeds u32"))?;
        let member_index = record
            .pack_member_index_plus1
            .checked_sub(1)
            .ok_or_else(|| "blob has no pack member".to_string())?;
        let member = member_records
            .get(member_index as usize)
            .cloned()
            .ok_or_else(|| BinaryDbError::corruption("blob pack-member index is out of range"))?;
        if member.blob_index != blob_index {
            return Err("blob and object-pack member indexes disagree".into());
        }
        let pack = pack_records
            .get(member.pack_index as usize)
            .cloned()
            .ok_or_else(|| BinaryDbError::corruption("object-pack index is out of range"))?;
        let base_blob_id = member
            .base_blob_index_plus1
            .checked_sub(1)
            .map(|base_index| {
                blob_records
                    .get(base_index as usize)
                    .map(|base| blob_id_from_sha256(&base.sha256))
                    .ok_or_else(|| BinaryDbError::corruption("base Blob index is out of range"))
            })
            .transpose()?;
        all_views.push(ServerBinaryBlobView {
            blob_index,
            blob_id: blob_id_from_sha256(&record.sha256),
            record: record.clone(),
            member_index,
            member,
            pack_id: format!("PCK-{:012X}", pack.pack_hash48),
            pack,
            base_blob_id,
        });
    }

    let requested_ids = all_views
        .iter()
        .filter(|view| view.record.blob_meta & META_TOMBSTONE == 0)
        .map(|view| view.blob_id.to_ascii_uppercase())
        .collect::<BTreeSet<_>>();
    if requested_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let ordered_ids = requested_ids.iter().collect::<Vec<_>>();
    let keys = ordered_ids
        .iter()
        .map(|id| prefixed_hex_key(id, "BLB-", 10))
        .collect::<StoreResult<Vec<_>>>()?;
    let key_refs = keys.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let candidates_by_id = access.lookup_many(blob_index_file(), &key_refs)?;
    if candidates_by_id.len() != ordered_ids.len() {
        return Err(BinaryDbError::corruption(
            "Binary DB batch blob lookup returned a misaligned result",
        ));
    }
    let mut resolved = BTreeMap::new();
    for (id, candidates) in ordered_ids.into_iter().zip(candidates_by_id) {
        for candidate in candidates {
            let view = all_views.get(candidate as usize).ok_or_else(|| {
                BinaryDbError::corruption("Blob ID index candidate is out of range")
            })?;
            if view.blob_id.eq_ignore_ascii_case(id) && view.record.blob_meta & META_TOMBSTONE == 0
            {
                resolved.insert(id.clone(), view.clone());
                break;
            }
        }
        if !resolved.contains_key(id) {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB blob ID index does not resolve non-tombstoned blob {id}"
            )));
        }
    }
    Ok(resolved)
}

fn tree_with_read<B: BinaryDb + ?Sized>(
    read: &BinaryDbReadTxn<'_, B>,
    id: &str,
) -> StoreResult<Option<ServerBinaryTreeView>> {
    tree_with_access(read, id)
}
fn tree_with_access(
    access: &impl ReadAccess,
    id: &str,
) -> StoreResult<Option<ServerBinaryTreeView>> {
    let normalized_id = id.to_ascii_uppercase();
    let mut ids = BTreeSet::new();
    ids.insert(normalized_id.clone());
    Ok(trees_with_access(access, &ids)?.remove(&normalized_id))
}
fn trees_with_access(
    access: &impl ReadAccess,
    ids: &BTreeSet<String>,
) -> StoreResult<BTreeMap<String, ServerBinaryTreeView>> {
    let mut views = BTreeMap::new();
    if ids.is_empty() || optional_record_count(access, tree_file())? == 0 {
        return Ok(views);
    }
    let requested_ids = ids.iter().collect::<Vec<_>>();
    let keys = requested_ids
        .iter()
        .map(|id| prefixed_hex_key(id, "TRE-", 10))
        .collect::<StoreResult<Vec<_>>>()?;
    let key_refs = keys.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let candidates_by_id = access.lookup_many(tree_index_file(), &key_refs)?;
    if candidates_by_id.len() != requested_ids.len() {
        return Err(BinaryDbError::corruption(
            "Binary DB batch tree lookup returned a misaligned result",
        ));
    }
    for (id, candidates) in requested_ids.into_iter().zip(candidates_by_id) {
        for index in candidates {
            let view = tree_at(access, index)?;
            if view.tree_id.eq_ignore_ascii_case(id) && view.record.tree_meta & META_TOMBSTONE == 0
            {
                views.insert(id.clone(), view);
                break;
            }
        }
    }
    Ok(views)
}
fn tree_at(access: &impl ReadAccess, index: u32) -> StoreResult<ServerBinaryTreeView> {
    let record = decode_tree(&access.read_record(tree_file(), index)?)?;
    let pack_count = optional_record_count(access, tree_pack_file())?;
    let mut low = 0_u32;
    let mut high = pack_count;
    let mut owner = None;
    while low < high {
        let pack_index = low + (high - low) / 2;
        let pack = tree_pack_at(access, pack_index)?;
        let end = pack
            .record
            .first_tree_index
            .checked_add(pack.record.tree_count)
            .ok_or_else(|| "tree pack range overflow".to_string())?;
        if index < pack.record.first_tree_index {
            high = pack_index;
        } else if index >= end {
            low = pack_index + 1;
        } else {
            owner = Some(pack);
            break;
        }
    }
    let owner = owner.ok_or_else(|| format!("tree index {index} has no tree pack"))?;
    let logical_ordinal = index
        .checked_sub(owner.record.first_tree_index)
        .ok_or_else(|| "tree logical ordinal underflow".to_string())?;
    if !owner.record.has_sparse_physical_ordinals()
        && (record.pack_entry_ordinal != logical_ordinal
            || record.pack_entry_ordinal >= owner.record.tree_count)
    {
        return Err(BinaryDbError::corruption(format!(
            "dense tree pack {} tree {index} has physical ordinal {}, expected {logical_ordinal}",
            owner.pack_id, record.pack_entry_ordinal
        )));
    }
    Ok(ServerBinaryTreeView {
        tree_index: index,
        tree_id: format!("TRE-{}", hex_upper(&record.tree_hash80)),
        record,
        pack_index: owner.pack_index,
        pack_id: owner.pack_id,
        pack: owner.record,
    })
}

fn tree_view_from_preloaded_records(
    index: u32,
    record: &ServerBinaryTreeRecord,
    packs: &[ServerBinaryTreePackView],
) -> StoreResult<ServerBinaryTreeView> {
    let mut low = 0_usize;
    let mut high = packs.len();
    let mut owner = None;
    while low < high {
        let pack_index = low + (high - low) / 2;
        let pack = &packs[pack_index];
        let end = pack
            .record
            .first_tree_index
            .checked_add(pack.record.tree_count)
            .ok_or_else(|| "tree pack range overflow".to_string())?;
        if index < pack.record.first_tree_index {
            high = pack_index;
        } else if index >= end {
            low = pack_index + 1;
        } else {
            owner = Some(pack);
            break;
        }
    }
    let owner = owner.ok_or_else(|| format!("tree index {index} has no tree pack"))?;
    let logical_ordinal = index
        .checked_sub(owner.record.first_tree_index)
        .ok_or_else(|| "tree logical ordinal underflow".to_string())?;
    if !owner.record.has_sparse_physical_ordinals()
        && (record.pack_entry_ordinal != logical_ordinal
            || record.pack_entry_ordinal >= owner.record.tree_count)
    {
        return Err(BinaryDbError::corruption(format!(
            "dense tree pack {} tree {index} has physical ordinal {}, expected {logical_ordinal}",
            owner.pack_id, record.pack_entry_ordinal
        )));
    }
    Ok(ServerBinaryTreeView {
        tree_index: index,
        tree_id: format!("TRE-{}", hex_upper(&record.tree_hash80)),
        record: record.clone(),
        pack_index: owner.pack_index,
        pack_id: owner.pack_id.clone(),
        pack: owner.record.clone(),
    })
}

fn preloaded_tree_views_with_validated_index(
    access: &impl ReadAccess,
    tree_records: &[ServerBinaryTreeRecord],
    packs: &[ServerBinaryTreePackView],
) -> StoreResult<(
    Vec<ServerBinaryTreeView>,
    BTreeMap<String, ServerBinaryTreeView>,
)> {
    let all_views = tree_records
        .iter()
        .enumerate()
        .map(|(tree_index, record)| {
            let tree_index = u32::try_from(tree_index)
                .map_err(|_| BinaryDbError::corruption("tree index exceeds u32"))?;
            tree_view_from_preloaded_records(tree_index, record, packs)
        })
        .collect::<StoreResult<Vec<_>>>()?;
    let requested_ids = all_views
        .iter()
        .filter(|view| view.record.tree_meta & META_TOMBSTONE == 0)
        .map(|view| view.tree_id.to_ascii_uppercase())
        .collect::<BTreeSet<_>>();
    if requested_ids.is_empty() {
        return Ok((all_views, BTreeMap::new()));
    }
    let ordered_ids = requested_ids.iter().collect::<Vec<_>>();
    let keys = ordered_ids
        .iter()
        .map(|id| prefixed_hex_key(id, "TRE-", 10))
        .collect::<StoreResult<Vec<_>>>()?;
    let key_refs = keys.iter().map(Vec::as_slice).collect::<Vec<_>>();
    let candidates_by_id = access.lookup_many(tree_index_file(), &key_refs)?;
    if candidates_by_id.len() != ordered_ids.len() {
        return Err(BinaryDbError::corruption(
            "Binary DB batch tree lookup returned a misaligned result",
        ));
    }
    let mut resolved = BTreeMap::new();
    for (id, candidates) in ordered_ids.into_iter().zip(candidates_by_id) {
        for candidate in candidates {
            let view = all_views.get(candidate as usize).ok_or_else(|| {
                BinaryDbError::corruption("Tree ID index candidate is out of range")
            })?;
            if view.tree_id.eq_ignore_ascii_case(id) && view.record.tree_meta & META_TOMBSTONE == 0
            {
                resolved.insert(id.clone(), view.clone());
                break;
            }
        }
        if !resolved.contains_key(id) {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree ID index does not resolve non-tombstoned tree {id}"
            )));
        }
    }
    Ok((all_views, resolved))
}

fn tree_for_pack_entry_ordinal(
    access: &impl ReadAccess,
    pack: &ServerBinaryTreePackView,
    physical_ordinal: u32,
) -> StoreResult<ServerBinaryTreeView> {
    let end = pack
        .record
        .first_tree_index
        .checked_add(pack.record.tree_count)
        .ok_or_else(|| BinaryDbError::corruption("tree pack logical range overflow"))?;
    let mut found = None;
    for tree_index in pack.record.first_tree_index..end {
        let tree = tree_at(access, tree_index)?;
        if tree.pack_id != pack.pack_id {
            return Err(BinaryDbError::corruption(format!(
                "tree pack {} logical range crosses another pack",
                pack.pack_id
            )));
        }
        if tree.record.pack_entry_ordinal == physical_ordinal {
            if found.replace(tree).is_some() {
                return Err(BinaryDbError::corruption(format!(
                    "tree pack {} repeats physical ordinal {physical_ordinal}",
                    pack.pack_id
                )));
            }
        }
    }
    found.ok_or_else(|| {
        BinaryDbError::corruption(format!(
            "tree pack {} has no physical entry ordinal {physical_ordinal}",
            pack.pack_id
        ))
    })
}

fn snapshot_root_tree_with_access(
    access: &impl ReadAccess,
    snapshot: &ServerBinarySnapshotRecord,
) -> StoreResult<Option<ServerBinaryTreeView>> {
    let Some(root_pack_index) = snapshot.root_tree_pack_index_plus1.checked_sub(1) else {
        if snapshot.file_count != 0 || snapshot.total_bytes != 0 {
            return Err(BinaryDbError::corruption(
                "snapshot without a root Tree locator has non-zero file aggregates",
            ));
        }
        return Ok(None);
    };
    if !snapshot.has_root_locator() {
        return Err(BinaryDbError::corruption(
            "snapshot root Tree locator bit disagrees with its pack index",
        ));
    }
    let root_pack = tree_pack_at(access, root_pack_index)?;
    if root_pack.record.pack_meta & META_TOMBSTONE != 0 {
        return Err(BinaryDbError::corruption(format!(
            "snapshot references tombstoned tree pack index {root_pack_index}"
        )));
    }
    if !root_pack.record.is_ready() {
        return Err(BinaryDbError::corruption(format!(
            "snapshot root tree pack {} is not ready",
            root_pack.pack_id
        )));
    }
    let root_tree = tree_for_pack_entry_ordinal(access, &root_pack, snapshot.root_entry_ordinal)?;
    if root_tree.record.tree_meta & META_TOMBSTONE != 0 {
        return Err(BinaryDbError::corruption(format!(
            "snapshot root locator references tombstoned tree {}",
            root_tree.tree_id
        )));
    }
    Ok(Some(root_tree))
}

fn snapshot_root_tree_with_validated_locators(
    snapshot: &ServerBinarySnapshotRecord,
    tree_packs: &[ServerBinaryTreePackView],
    tree_locators: &BTreeMap<(u32, u32), Vec<ServerBinaryTreeView>>,
) -> StoreResult<Option<ServerBinaryTreeView>> {
    let Some(root_pack_index) = snapshot.root_tree_pack_index_plus1.checked_sub(1) else {
        if snapshot.file_count != 0 || snapshot.total_bytes != 0 {
            return Err(BinaryDbError::corruption(
                "snapshot without a root Tree locator has non-zero file aggregates",
            ));
        }
        return Ok(None);
    };
    if !snapshot.has_root_locator() {
        return Err(BinaryDbError::corruption(
            "snapshot root Tree locator bit disagrees with its pack index",
        ));
    }
    let root_pack = tree_packs.get(root_pack_index as usize).ok_or_else(|| {
        BinaryDbError::corruption(format!(
            "snapshot references missing tree pack index {root_pack_index}"
        ))
    })?;
    if root_pack.record.pack_meta & META_TOMBSTONE != 0 {
        return Err(BinaryDbError::corruption(format!(
            "snapshot references tombstoned tree pack index {root_pack_index}"
        )));
    }
    if !root_pack.record.is_ready() {
        return Err(BinaryDbError::corruption(format!(
            "snapshot root tree pack {} is not ready",
            root_pack.pack_id
        )));
    }
    let candidates = tree_locators
        .get(&(root_pack_index, snapshot.root_entry_ordinal))
        .map(Vec::as_slice)
        .unwrap_or_default();
    let root_tree = match candidates {
        [] => {
            return Err(BinaryDbError::corruption(format!(
                "tree pack {} has no physical entry ordinal {}",
                root_pack.pack_id, snapshot.root_entry_ordinal
            )));
        }
        [tree] => tree.clone(),
        _ => {
            return Err(BinaryDbError::corruption(format!(
                "tree pack {} repeats physical ordinal {}",
                root_pack.pack_id, snapshot.root_entry_ordinal
            )));
        }
    };
    if root_tree.record.tree_meta & META_TOMBSTONE != 0 {
        return Err(BinaryDbError::corruption(format!(
            "snapshot root locator references tombstoned tree {}",
            root_tree.tree_id
        )));
    }
    Ok(Some(root_tree))
}

fn tree_aggregate_node(
    tree_id: &str,
    entries: &[ServerBinaryTreeEntryView],
) -> StoreResult<ServerBinaryTreeAggregateNode> {
    let mut direct = ServerBinaryTreeAggregate::default();
    let mut child_tree_ids = Vec::new();
    for entry in entries {
        if entry.entry_type == "tree" {
            child_tree_ids.push(entry.target_id.clone());
            continue;
        }
        let size_bytes = entry.size_bytes.ok_or_else(|| {
            BinaryDbError::corruption(format!(
                "snapshot blob entry {tree_id}/{:?} has no fixed size",
                entry.entry_name
            ))
        })?;
        if entry.sha256.is_none() {
            return Err(BinaryDbError::corruption(format!(
                "snapshot blob entry {tree_id}/{:?} has no fixed checksum",
                entry.entry_name
            )));
        }
        direct.file_count = direct
            .file_count
            .checked_add(1)
            .ok_or_else(|| BinaryDbError::corruption("snapshot file count exceeds u32"))?;
        direct.total_bytes = direct
            .total_bytes
            .checked_add(size_bytes)
            .ok_or_else(|| BinaryDbError::corruption("snapshot total bytes overflow"))?;
    }
    Ok(ServerBinaryTreeAggregateNode {
        direct,
        child_tree_ids,
    })
}

fn fold_snapshot_tree_aggregate<F>(
    tree_id: &str,
    active_trees: &mut BTreeSet<String>,
    aggregate_cache: &mut BTreeMap<String, ServerBinaryTreeAggregate>,
    load_node: &mut F,
) -> StoreResult<ServerBinaryTreeAggregate>
where
    F: FnMut(&str) -> StoreResult<ServerBinaryTreeAggregateNode>,
{
    if let Some(aggregate) = aggregate_cache.get(tree_id) {
        return Ok(*aggregate);
    }
    if !active_trees.insert(tree_id.to_string()) {
        return Err(BinaryDbError::corruption(format!(
            "snapshot Tree graph contains a cycle at {tree_id}"
        )));
    }
    let result = (|| {
        let node = load_node(tree_id)?;
        let mut aggregate = node.direct;
        for child_tree_id in node.child_tree_ids {
            let child = fold_snapshot_tree_aggregate(
                &child_tree_id,
                active_trees,
                aggregate_cache,
                load_node,
            )?;
            aggregate.file_count = aggregate
                .file_count
                .checked_add(child.file_count)
                .ok_or_else(|| BinaryDbError::corruption("snapshot file count exceeds u32"))?;
            aggregate.total_bytes = aggregate
                .total_bytes
                .checked_add(child.total_bytes)
                .ok_or_else(|| BinaryDbError::corruption("snapshot total bytes overflow"))?;
        }
        Ok(aggregate)
    })();
    active_trees.remove(tree_id);
    if let Ok(aggregate) = &result {
        aggregate_cache.insert(tree_id.to_string(), *aggregate);
    }
    result
}

fn collect_snapshot_file_identities<D>(
    store: &ServerBinaryRepositoryContentStore<D>,
    access: &impl ReadAccess,
    tree_id: &str,
    prefix: &str,
    active_trees: &mut BTreeSet<String>,
    files: &mut BTreeMap<String, ServerBinarySnapshotFileIdentity>,
    cache: &mut ServerBinaryTreeReadCache,
) -> StoreResult<()>
where
    D: ServerRemoteBinaryDb + Clone,
{
    if !active_trees.insert(tree_id.to_string()) {
        return Err(BinaryDbError::corruption(format!(
            "snapshot Tree graph contains a cycle at {tree_id}"
        )));
    }
    let result = (|| {
        let tree = tree_with_access(access, tree_id)?.ok_or_else(|| {
            BinaryDbError::corruption(format!(
                "snapshot Tree graph references missing tree {tree_id}"
            ))
        })?;
        for entry in store.tree_entries_for_view_with_cache(access, &tree, cache)? {
            let path = if prefix.is_empty() {
                entry.entry_name.clone()
            } else {
                format!("{prefix}/{}", entry.entry_name)
            };
            if entry.entry_type == "tree" {
                collect_snapshot_file_identities(
                    store,
                    access,
                    &entry.target_id,
                    &path,
                    active_trees,
                    files,
                    cache,
                )?;
                continue;
            }
            let identity = ServerBinarySnapshotFileIdentity {
                blob_id: entry.target_id,
                size_bytes: entry.size_bytes.ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "snapshot blob entry {path} has no fixed size"
                    ))
                })?,
                mode: entry.mode,
                sha256: entry.sha256.ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "snapshot blob entry {path} has no fixed checksum"
                    ))
                })?,
            };
            if files.insert(path.clone(), identity).is_some() {
                return Err(BinaryDbError::corruption(format!(
                    "snapshot Tree graph repeats path {path:?}"
                )));
            }
        }
        Ok(())
    })();
    active_trees.remove(tree_id);
    result
}

fn normalized_tree_entries(
    access: &impl ReadAccess,
    tree: &ServerBinaryTreeView,
    cache: &mut ServerBinaryTreeReadCache,
) -> StoreResult<Vec<ServerBinaryTreeEntryView>> {
    let range_count = match cache.normalized_entry_ranges.as_ref() {
        Some(ranges) => u32::try_from(ranges.len())
            .map_err(|_| BinaryDbError::corruption("tree entry range count exceeds u32"))?,
        None => optional_record_count(access, tree_entry_range_file())?,
    };
    if tree.tree_index >= range_count {
        return Err(BinaryDbError::corruption(format!(
            "Binary DB tree {} is missing its normalized entry range",
            tree.tree_id
        )));
    }
    let range = match cache.normalized_entry_ranges.as_ref() {
        Some(ranges) => *ranges.get(tree.tree_index as usize).ok_or_else(|| {
            BinaryDbError::corruption(format!(
                "Binary DB tree {} is missing its cached normalized entry range",
                tree.tree_id
            ))
        })?,
        None => {
            decode_tree_entry_range(&access.read_record(tree_entry_range_file(), tree.tree_index)?)?
        }
    };
    let entry_count = match cache.normalized_entries.as_ref() {
        Some(entries) => u32::try_from(entries.len())
            .map_err(|_| BinaryDbError::corruption("tree entry count exceeds u32"))?,
        None => optional_record_count(access, tree_entry_file())?,
    };
    let end = range
        .first_entry_index
        .checked_add(tree.record.entry_count)
        .ok_or_else(|| BinaryDbError::corruption("tree normalized entry range overflow"))?;
    if end > entry_count {
        return Err(BinaryDbError::corruption(format!(
            "Binary DB tree {} normalized range {}..{} exceeds {} rows",
            tree.tree_id, range.first_entry_index, end, entry_count
        )));
    }
    let mut names = BTreeSet::new();
    let mut previous_name: Option<Vec<u8>> = None;
    let mut entries = Vec::with_capacity(tree.record.entry_count as usize);
    for offset in 0..tree.record.entry_count {
        let physical_index = range
            .first_entry_index
            .checked_add(offset)
            .ok_or_else(|| BinaryDbError::corruption("tree entry index overflow"))?;
        let record = match cache.normalized_entries.as_ref() {
            Some(cached_entries) => *cached_entries
                .get(physical_index as usize)
                .ok_or_else(|| BinaryDbError::corruption("cached tree entry index is missing"))?,
            None => decode_tree_entry(&access.read_record(tree_entry_file(), physical_index)?)?,
        };
        let name_bytes = match cache.tree_name_payload_body.as_ref() {
            Some(payload) => {
                let body_offset = record.name_offset.checked_sub(4).ok_or_else(|| {
                    BinaryDbError::corruption("tree entry name offset precedes payload body")
                })?;
                let start = usize::try_from(body_offset).map_err(|_| {
                    BinaryDbError::corruption("tree entry name offset exceeds usize")
                })?;
                let end = start
                    .checked_add(usize::from(record.name_len))
                    .ok_or_else(|| BinaryDbError::corruption("tree entry name range overflow"))?;
                payload
                    .get(start..end)
                    .ok_or_else(|| {
                        BinaryDbError::corruption(
                            "tree entry name range exceeds cached payload body",
                        )
                    })?
                    .to_vec()
            }
            None => access.read_payload(
                tree_name_payload_file(),
                record.name_offset,
                u32::from(record.name_len),
            )?,
        };
        if previous_name
            .as_ref()
            .is_some_and(|previous| previous.as_slice() >= name_bytes.as_slice())
        {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree {} normalized names are not strictly byte-sorted",
                tree.tree_id
            )));
        }
        previous_name = Some(name_bytes.clone());
        let entry_name = String::from_utf8(name_bytes).map_err(|_| {
            BinaryDbError::corruption(format!(
                "Binary DB tree {} entry name is not UTF-8",
                tree.tree_id
            ))
        })?;
        validate_tree_entry_name(&entry_name)?;
        if !names.insert(entry_name.clone()) {
            return Err(BinaryDbError::corruption(format!(
                "Binary DB tree {} repeats normalized entry name {entry_name:?}",
                tree.tree_id
            )));
        }
        let (entry_type, target_id, size_bytes, sha256, mode) = match record.entry_meta & 0b11 {
            0 => {
                let blob = match cache.blobs_by_index.get(&record.target_index) {
                    Some(blob) => blob.clone(),
                    None if cache.manifest_object_projection_complete => {
                        return Err(BinaryDbError::corruption(format!(
                            "Binary DB tree {} references missing or tombstoned projected blob index {}",
                            tree.tree_id, record.target_index
                        )));
                    }
                    None => {
                        let blob = blob_at(access, record.target_index)?;
                        cache.cache_blobs(BTreeMap::from([(
                            blob.blob_id.to_ascii_uppercase(),
                            blob.clone(),
                        )]));
                        blob
                    }
                };
                (
                    "blob".to_string(),
                    blob.blob_id,
                    Some(blob.record.size_bytes),
                    Some(hex_lower(&blob.record.sha256)),
                    format!("{:06o}", record.mode_bits),
                )
            }
            1 => {
                let child = match cache.trees_by_index.get(&record.target_index) {
                    Some(tree) => tree.clone(),
                    None if cache.manifest_tree_projection_complete => {
                        return Err(BinaryDbError::corruption(format!(
                            "Binary DB tree {} references missing or tombstoned projected tree index {}",
                            tree.tree_id, record.target_index
                        )));
                    }
                    None => {
                        let child = tree_at(access, record.target_index)?;
                        cache.cache_trees(BTreeMap::from([(
                            child.tree_id.to_ascii_uppercase(),
                            child.clone(),
                        )]));
                        child
                    }
                };
                (
                    "tree".to_string(),
                    child.tree_id,
                    None,
                    None,
                    "tree".to_string(),
                )
            }
            _ => unreachable!("decode_tree_entry validates entry kind"),
        };
        entries.push(ServerBinaryTreeEntryView {
            entry_index: offset,
            entry_name,
            entry_type,
            target_id,
            size_bytes,
            sha256,
            mode,
        });
    }
    Ok(entries)
}

fn validate_tree_entry_name(name: &str) -> StoreResult<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(BinaryDbError::corruption(format!(
            "tree entry name is not a safe path segment: {name:?}"
        )));
    }
    Ok(())
}

fn encode_blob(record: &ServerBinaryBlobRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(SERVER_BLOB_RECORD_SIZE as usize);
    out.push(record.blob_meta);
    out.push(record.hash_kind);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&record.size_bytes.to_le_bytes());
    out.extend_from_slice(&record.pack_member_index_plus1.to_le_bytes());
    out.extend_from_slice(&record.created_at_s.to_le_bytes());
    out.extend_from_slice(&record.pruned_at_s.to_le_bytes());
    out.extend_from_slice(&record.sha256);
    out
}
fn decode_blob(raw: &[u8]) -> StoreResult<ServerBinaryBlobRecord> {
    require_len(raw, SERVER_BLOB_RECORD_SIZE as usize, "blob")?;
    if raw[1] != SHA256_HASH_KIND {
        return Err(BinaryDbError::corruption(format!(
            "Binary DB blob record uses unsupported hash kind {}",
            raw[1]
        )));
    }
    let mut sha = [0; 32];
    sha.copy_from_slice(&raw[32..64]);
    Ok(ServerBinaryBlobRecord {
        blob_meta: raw[0],
        hash_kind: raw[1],
        size_bytes: u64::from_le_bytes(raw[4..12].try_into().unwrap()),
        pack_member_index_plus1: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
        created_at_s: u64::from_le_bytes(raw[16..24].try_into().unwrap()),
        pruned_at_s: u64::from_le_bytes(raw[24..32].try_into().unwrap()),
        sha256: sha,
    })
}
fn encode_object_pack(record: &ServerBinaryObjectPackRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(SERVER_OBJECT_PACK_RECORD_SIZE as usize);
    out.push(record.pack_meta);
    out.push(record.pack_format_kind);
    out.extend_from_slice(&((record.pack_hash48 >> 32) as u16).to_le_bytes());
    out.extend_from_slice(&(record.pack_hash48 as u32).to_le_bytes());
    out.extend_from_slice(&record.first_member_index.to_le_bytes());
    out.extend_from_slice(&record.member_count.to_le_bytes());
    out.extend_from_slice(&record.total_bytes.to_le_bytes());
    out.extend_from_slice(&record.created_at_s.to_le_bytes());
    out
}
fn decode_object_pack(raw: &[u8]) -> StoreResult<ServerBinaryObjectPackRecord> {
    require_len(raw, SERVER_OBJECT_PACK_RECORD_SIZE as usize, "object pack")?;
    Ok(ServerBinaryObjectPackRecord {
        pack_meta: raw[0],
        pack_format_kind: raw[1],
        pack_hash48: (u64::from(u16::from_le_bytes(raw[2..4].try_into().unwrap())) << 32)
            | u64::from(u32::from_le_bytes(raw[4..8].try_into().unwrap())),
        first_member_index: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
        member_count: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
        total_bytes: u64::from_le_bytes(raw[16..24].try_into().unwrap()),
        created_at_s: u64::from_le_bytes(raw[24..32].try_into().unwrap()),
    })
}
fn encode_object_pack_member(record: &ServerBinaryObjectPackMemberRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(16);
    out.push(record.member_meta);
    out.push(record.delta_chain_depth);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&record.pack_index.to_le_bytes());
    out.extend_from_slice(&record.blob_index.to_le_bytes());
    out.extend_from_slice(&record.base_blob_index_plus1.to_le_bytes());
    out
}
fn decode_object_pack_member(raw: &[u8]) -> StoreResult<ServerBinaryObjectPackMemberRecord> {
    require_len(raw, 16, "object pack member")?;
    Ok(ServerBinaryObjectPackMemberRecord {
        member_meta: raw[0],
        delta_chain_depth: raw[1],
        pack_index: u32::from_le_bytes(raw[4..8].try_into().unwrap()),
        blob_index: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
        base_blob_index_plus1: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
    })
}
fn encode_tree_pack(record: &ServerBinaryTreePackRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(SERVER_TREE_PACK_RECORD_SIZE as usize);
    out.push(record.pack_meta);
    out.push(record.pack_format_kind);
    out.extend_from_slice(&((record.pack_hash48 >> 32) as u16).to_le_bytes());
    out.extend_from_slice(&(record.pack_hash48 as u32).to_le_bytes());
    out.extend_from_slice(&record.first_tree_index.to_le_bytes());
    out.extend_from_slice(&record.tree_count.to_le_bytes());
    out.extend_from_slice(&record.total_bytes.to_le_bytes());
    out.extend_from_slice(&record.created_at_s.to_le_bytes());
    out
}
fn decode_tree_pack(raw: &[u8]) -> StoreResult<ServerBinaryTreePackRecord> {
    require_len(raw, SERVER_TREE_PACK_RECORD_SIZE as usize, "tree pack")?;
    let record = ServerBinaryTreePackRecord {
        pack_meta: raw[0],
        pack_format_kind: raw[1],
        pack_hash48: (u64::from(u16::from_le_bytes(raw[2..4].try_into().unwrap())) << 32)
            | u64::from(u32::from_le_bytes(raw[4..8].try_into().unwrap())),
        first_tree_index: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
        tree_count: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
        total_bytes: u64::from_le_bytes(raw[16..24].try_into().unwrap()),
        created_at_s: u64::from_le_bytes(raw[24..32].try_into().unwrap()),
    };
    if record.pack_meta
        & !(META_READY | META_CORRUPT | META_SPARSE_PHYSICAL_ORDINALS | META_TOMBSTONE)
        != 0
        || record.pack_format_kind > ZSTD_PACK_FORMAT_KIND
    {
        return Err(BinaryDbError::corruption(
            "tree pack metadata contains reserved values",
        ));
    }
    Ok(record)
}
fn encode_tree(record: &ServerBinaryTreeRecord) -> Vec<u8> {
    let mut out = Vec::with_capacity(20);
    out.push(record.tree_meta);
    out.push(0);
    out.extend_from_slice(&record.pack_entry_ordinal.to_le_bytes());
    out.extend_from_slice(&record.entry_count.to_le_bytes());
    out.extend_from_slice(&record.tree_hash80);
    out
}
fn decode_tree(raw: &[u8]) -> StoreResult<ServerBinaryTreeRecord> {
    require_len(raw, 20, "tree")?;
    if raw[1] != 0 {
        return Err(BinaryDbError::corruption(
            "tree record reserved byte is non-zero",
        ));
    }
    let mut hash = [0; 10];
    hash.copy_from_slice(&raw[10..20]);
    Ok(ServerBinaryTreeRecord {
        tree_meta: raw[0],
        pack_entry_ordinal: u32::from_le_bytes(raw[2..6].try_into().unwrap()),
        entry_count: u32::from_le_bytes(raw[6..10].try_into().unwrap()),
        tree_hash80: hash,
    })
}

fn encode_tree_entry_range(record: ServerBinaryTreeEntryRangeRecord) -> Vec<u8> {
    record.first_entry_index.to_le_bytes().to_vec()
}

fn decode_tree_entry_range(raw: &[u8]) -> StoreResult<ServerBinaryTreeEntryRangeRecord> {
    require_len(raw, 4, "tree entry range")?;
    Ok(ServerBinaryTreeEntryRangeRecord {
        first_entry_index: u32::from_le_bytes(raw.try_into().unwrap()),
    })
}

fn encode_tree_entry(record: ServerBinaryTreeEntryRecord) -> StoreResult<Vec<u8>> {
    if record.entry_meta & !0b11 != 0
        || record.entry_meta & 0b11 > 1
        || record.name_len == 0
        || record.name_offset < 4
        || (record.entry_meta & 0b11 == 1 && record.mode_bits != 0)
    {
        return Err(BinaryDbError::invalid_domain_data(
            "tree entry fixed fields are invalid",
        ));
    }
    let mut out = Vec::with_capacity(SERVER_TREE_ENTRY_RECORD_SIZE as usize);
    out.push(record.entry_meta);
    out.push(record.name_len);
    out.extend_from_slice(&record.mode_bits.to_le_bytes());
    out.extend_from_slice(&record.name_offset.to_le_bytes());
    out.extend_from_slice(&record.target_index.to_le_bytes());
    Ok(out)
}

fn decode_tree_entry(raw: &[u8]) -> StoreResult<ServerBinaryTreeEntryRecord> {
    require_len(raw, SERVER_TREE_ENTRY_RECORD_SIZE as usize, "tree entry")?;
    let record = ServerBinaryTreeEntryRecord {
        entry_meta: raw[0],
        name_len: raw[1],
        mode_bits: u16::from_le_bytes(raw[2..4].try_into().unwrap()),
        name_offset: u64::from_le_bytes(raw[4..12].try_into().unwrap()),
        target_index: u32::from_le_bytes(raw[12..16].try_into().unwrap()),
    };
    if record.entry_meta & !0b11 != 0
        || record.entry_meta & 0b11 > 1
        || record.name_len == 0
        || record.name_offset < 4
        || (record.entry_meta & 0b11 == 1 && record.mode_bits != 0)
    {
        return Err(BinaryDbError::corruption(
            "tree entry fixed fields are corrupt",
        ));
    }
    Ok(record)
}

fn require_len(raw: &[u8], expected: usize, label: &str) -> StoreResult<()> {
    if raw.len() == expected {
        Ok(())
    } else {
        Err(BinaryDbError::corruption(format!(
            "{label} record length {}, expected {expected}",
            raw.len()
        )))
    }
}
fn blob_id_from_sha256(value: &[u8; 32]) -> String {
    format!("BLB-{}", hex_lower(&value[..10]))
}
fn hex_lower(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn hex_upper(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02X}")).collect()
}
fn parse_sha256(value: &str) -> StoreResult<[u8; 32]> {
    prefixed_hex_key_array::<32>(value, "")
}
fn tree_entry_kind(value: &str) -> StoreResult<u8> {
    match value.trim() {
        "blob" => Ok(0),
        "tree" => Ok(1),
        other => Err(BinaryDbError::invalid_domain_data(format!(
            "unsupported Binary DB tree entry kind: {other}"
        ))),
    }
}
fn tree_entry_mode_bits(entry_type: &str, mode: &str) -> StoreResult<u16> {
    if entry_type.trim() == "tree" {
        return if mode == "tree" {
            Ok(0)
        } else {
            Err(BinaryDbError::invalid_domain_data(format!(
                "tree entry mode must be exactly `tree`, got {mode:?}"
            )))
        };
    }
    let trimmed = mode.trim();
    let octal = trimmed
        .strip_prefix("0o")
        .or_else(|| trimmed.strip_prefix("0O"))
        .unwrap_or(trimmed);
    u16::from_str_radix(octal, 8).map_err(|error| {
        BinaryDbError::invalid_domain_data(format!(
            "invalid Binary DB tree entry mode `{mode}`: {error}"
        ))
    })
}
fn tree_entry_modes_match(entry_type: &str, left: &str, right: &str) -> StoreResult<bool> {
    Ok(tree_entry_mode_bits(entry_type, left)? == tree_entry_mode_bits(entry_type, right)?)
}
fn prefixed_hash48(value: &str, prefix: &str) -> StoreResult<u64> {
    let bytes = prefixed_hex_key(value, prefix, 6)?;
    Ok(bytes
        .into_iter()
        .fold(0u64, |out, byte| (out << 8) | u64::from(byte)))
}

fn prefixed_hash48_index_key(value: &str, prefix: &str) -> StoreResult<[u8; 8]> {
    Ok(prefixed_hash48(value, prefix)?.to_le_bytes())
}
fn prefixed_hex_key_array<const N: usize>(value: &str, prefix: &str) -> StoreResult<[u8; N]> {
    let bytes = prefixed_hex_key(value, prefix, N)?;
    bytes
        .try_into()
        .map_err(|_| BinaryDbError::invalid_domain_data("hex key length mismatch"))
}
fn prefixed_hex_key(value: &str, prefix: &str, len: usize) -> StoreResult<Vec<u8>> {
    let hex = value.strip_prefix(prefix).ok_or_else(|| {
        BinaryDbError::invalid_domain_data(format!("identity {value} is missing prefix {prefix}"))
    })?;
    if hex.len() != len * 2 {
        return Err(BinaryDbError::invalid_domain_data(format!(
            "identity {value} has wrong length"
        )));
    }
    let mut out = Vec::with_capacity(len);
    for pair in hex.as_bytes().chunks_exact(2) {
        let text = std::str::from_utf8(pair)
            .map_err(|_| BinaryDbError::invalid_domain_data("identity hex is not UTF-8"))?;
        out.push(u8::from_str_radix(text, 16).map_err(|_| {
            BinaryDbError::invalid_domain_data(format!("identity {value} is not hex"))
        })?);
    }
    Ok(out)
}
fn required_text(value: &JsonValue, field: &str) -> StoreResult<String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| BinaryDbError::invalid_domain_data(format!("metadata is missing {field}")))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ServerBinaryTreePayloadValidation {
    StrictPackComparison,
    NormalizedServingAuthority,
}

pub fn validate_server_tree_authority_v0<D>(db: &D) -> StoreResult<()>
where
    D: ServerRemoteBinaryDb + Clone,
{
    validate_server_tree_authority_v0_with_mode(
        db,
        ServerBinaryTreePayloadValidation::StrictPackComparison,
    )
}

pub fn validate_server_tree_serving_authority_v0<D>(db: &D) -> StoreResult<()>
where
    D: ServerRemoteBinaryDb + Clone,
{
    validate_server_tree_authority_v0_with_mode(
        db,
        ServerBinaryTreePayloadValidation::NormalizedServingAuthority,
    )
}

fn validate_server_tree_authority_v0_with_mode<D>(
    db: &D,
    payload_validation: ServerBinaryTreePayloadValidation,
) -> StoreResult<()>
where
    D: ServerRemoteBinaryDb + Clone,
{
    #[cfg(feature = "perfetto-tracing")]
    let _trace = crate::perfetto_trace::PerfettoRange::new(match payload_validation {
        ServerBinaryTreePayloadValidation::StrictPackComparison => {
            "ait.server.content.validate_tree_authority_v0"
        }
        ServerBinaryTreePayloadValidation::NormalizedServingAuthority => {
            "ait.server.content.validate_tree_serving_authority_v0"
        }
    });
    let read = BinaryDbReadTxn::new(db);
    let tree_count = read.record_count(tree_file())?;
    let range_count = read.record_count(tree_entry_range_file())?;
    if tree_count != range_count {
        return Err(BinaryDbError::corruption(format!(
            "tree_entry_range.bin count {range_count} disagrees with tree.bin count {tree_count}"
        )));
    }
    let entry_count = read.record_count(tree_entry_file())?;
    let mut cache = ServerBinaryTreeReadCache::default();
    cache.cache_normalized_tree_authority(&read, tree_count, entry_count)?;
    let tree_records = read
        .read_records(tree_file(), 0, tree_count)?
        .into_iter()
        .map(|raw| decode_tree(&raw))
        .collect::<StoreResult<Vec<_>>>()?;
    if tree_records.len() != tree_count as usize {
        return Err(BinaryDbError::corruption(
            "tree batch read returned a misaligned result",
        ));
    }
    let mut expected_first = 0_u32;
    for (tree_index, record) in tree_records.iter().enumerate() {
        let range = *cache
            .normalized_entry_ranges
            .as_ref()
            .and_then(|ranges| ranges.get(tree_index))
            .ok_or_else(|| BinaryDbError::corruption("cached tree entry range index is missing"))?;
        if range.first_entry_index != expected_first {
            return Err(BinaryDbError::corruption(format!(
                "tree {tree_index} normalized range starts at {}, expected {expected_first}",
                range.first_entry_index
            )));
        }
        expected_first = expected_first
            .checked_add(record.entry_count)
            .ok_or_else(|| BinaryDbError::corruption("tree entry range overflow"))?;
        if expected_first > entry_count {
            return Err(BinaryDbError::corruption(format!(
                "tree {tree_index} normalized range exceeds tree_entry.bin"
            )));
        }
    }
    if expected_first != entry_count {
        return Err(BinaryDbError::corruption(format!(
            "tree_entry.bin has {} orphan committed rows",
            entry_count - expected_first
        )));
    }

    let store = ServerBinaryRepositoryContentStore::new(db.clone());
    let pack_count = read.record_count(tree_pack_file())?;
    let tree_packs = read
        .read_records(tree_pack_file(), 0, pack_count)?
        .into_iter()
        .enumerate()
        .map(|(pack_index, raw)| {
            let pack_index = u32::try_from(pack_index)
                .map_err(|_| BinaryDbError::corruption("tree pack index exceeds u32"))?;
            let record = decode_tree_pack(&raw)?;
            Ok(ServerBinaryTreePackView {
                pack_index,
                pack_id: format!("TPK-{:012X}", record.pack_hash48),
                record,
            })
        })
        .collect::<StoreResult<Vec<_>>>()?;
    for pack in &tree_packs {
        if pack.record.pack_meta & META_TOMBSTONE == 0 && !pack.record.is_ready() {
            return Err(BinaryDbError::corruption(format!(
                "tree pack {} is not ready",
                pack.pack_id
            )));
        }
    }
    #[cfg(feature = "perfetto-tracing")]
    let identity_trace = crate::perfetto_trace::PerfettoRange::new(
        "ait.server.content.validate_identity_indexes_v0",
    );
    cache.cache_blobs(preloaded_blob_views_with_validated_index(&read)?);

    let mut trees = Vec::new();
    let mut tree_locators = BTreeMap::<(u32, u32), Vec<ServerBinaryTreeView>>::new();
    let (all_trees, indexed_trees) =
        preloaded_tree_views_with_validated_index(&read, &tree_records, &tree_packs)?;
    for tree in all_trees {
        tree_locators
            .entry((tree.pack_index, tree.record.pack_entry_ordinal))
            .or_default()
            .push(tree.clone());
        if tree.record.tree_meta & META_TOMBSTONE == 0 {
            trees.push(tree);
        }
    }
    cache.cache_trees(indexed_trees);
    #[cfg(feature = "perfetto-tracing")]
    drop(identity_trace);

    let mut tree_nodes = BTreeMap::new();
    for tree in trees {
        let entries = match payload_validation {
            ServerBinaryTreePayloadValidation::StrictPackComparison => {
                store.tree_entries_for_tree_with_read_cache(&read, &tree, &mut cache)?
            }
            ServerBinaryTreePayloadValidation::NormalizedServingAuthority => {
                normalized_tree_entries(&read, &tree, &mut cache)?
            }
        };
        tree_nodes.insert(
            tree.tree_id.clone(),
            tree_aggregate_node(&tree.tree_id, &entries)?,
        );
    }
    #[cfg(feature = "perfetto-tracing")]
    let _snapshot_trace = crate::perfetto_trace::PerfettoRange::new(
        "ait.server.content.validate_snapshot_aggregates_v0",
    );
    let mut aggregate_cache = BTreeMap::new();
    let snapshot_file = ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::record_file();
    let snapshot_count = read.record_count(snapshot_file.clone())?;
    for snapshot_index in 0..snapshot_count {
        let snapshot = ServerBinarySnapshotCodec::<SERVER_CONTENT_BINARY_LAYOUT_ID>::decode_record(
            &read.read_record(snapshot_file.clone(), snapshot_index)?,
        )?;
        if !snapshot.is_tombstone() {
            let Some(root_tree) =
                snapshot_root_tree_with_validated_locators(&snapshot, &tree_packs, &tree_locators)?
            else {
                continue;
            };
            let mut load_node = |tree_id: &str| {
                if let Some(node) = tree_nodes.get(tree_id) {
                    return Ok(node.clone());
                }
                let tree = tree_with_access(&read, tree_id)?.ok_or_else(|| {
                    BinaryDbError::corruption(format!(
                        "snapshot Tree graph references missing tree {tree_id}"
                    ))
                })?;
                let entries = match payload_validation {
                    ServerBinaryTreePayloadValidation::StrictPackComparison => {
                        store.tree_entries_for_tree_with_read_cache(&read, &tree, &mut cache)?
                    }
                    ServerBinaryTreePayloadValidation::NormalizedServingAuthority => {
                        normalized_tree_entries(&read, &tree, &mut cache)?
                    }
                };
                let node = tree_aggregate_node(&tree.tree_id, &entries)?;
                tree_nodes.insert(tree.tree_id.clone(), node.clone());
                Ok(node)
            };
            let aggregate = fold_snapshot_tree_aggregate(
                &root_tree.tree_id,
                &mut BTreeSet::new(),
                &mut aggregate_cache,
                &mut load_node,
            )?;
            if aggregate.file_count != snapshot.file_count
                || aggregate.total_bytes != snapshot.total_bytes
            {
                return Err(BinaryDbError::corruption(format!(
                    "snapshot Tree aggregates ({}, {}) disagree with fixed record ({}, {})",
                    aggregate.file_count,
                    aggregate.total_bytes,
                    snapshot.file_count,
                    snapshot.total_bytes
                )));
            }
        }
    }
    Ok(())
}
fn timestamp_s(value: Option<&JsonValue>) -> StoreResult<u64> {
    let Some(text) = value.and_then(JsonValue::as_str) else {
        return Ok(0);
    };
    chrono::DateTime::parse_from_rfc3339(text)
        .map_err(|error| {
            BinaryDbError::invalid_domain_data(format!("invalid timestamp {text}: {error}"))
        })
        .and_then(|value| {
            u64::try_from(value.timestamp())
                .map_err(|_| BinaryDbError::invalid_domain_data("timestamp precedes Unix epoch"))
        })
}

#[cfg(test)]
mod mode_tests {
    use super::*;

    #[test]
    fn blob_sha256_kind_zero_is_only_admitted_and_emitted() -> StoreResult<()> {
        let record = ServerBinaryBlobRecord {
            blob_meta: META_HAS_PACK_MEMBER,
            hash_kind: SHA256_HASH_KIND,
            size_bytes: 3,
            pack_member_index_plus1: 1,
            created_at_s: 17,
            pruned_at_s: 0,
            sha256: [7; 32],
        };

        let encoded = encode_blob(&record);
        assert_eq!(encoded[1], SHA256_HASH_KIND);
        assert_eq!(decode_blob(&encoded)?, record);

        let mut invalid = encoded;
        invalid[1] = 1;
        let error = decode_blob(&invalid).expect_err("hash kind one must fail closed");
        assert!(error.to_string().contains("unsupported hash kind 1"));
        Ok(())
    }

    #[test]
    fn blob_mode_comparison_uses_only_existing_accepted_mode_bits() -> StoreResult<()> {
        for equivalent in ["0o600", "0O600", "600", " 0o600 "] {
            assert!(tree_entry_modes_match("blob", "000600", equivalent)?);
        }
        assert!(!tree_entry_modes_match("blob", "000600", "000644")?);
        assert!(tree_entry_modes_match("tree", "tree", "tree")?);
        assert!(tree_entry_modes_match("tree", "tree", " tree").is_err());
        assert!(tree_entry_modes_match("blob", "000600", "not-octal").is_err());
        Ok(())
    }

    #[test]
    fn snapshot_tree_aggregate_memoizes_shared_trees_and_preserves_path_multiplicity(
    ) -> StoreResult<()> {
        let nodes = BTreeMap::from([
            (
                "TRE-ROOT".to_string(),
                ServerBinaryTreeAggregateNode {
                    direct: ServerBinaryTreeAggregate {
                        file_count: 1,
                        total_bytes: 3,
                    },
                    child_tree_ids: vec!["TRE-SHARED".to_string(), "TRE-SHARED".to_string()],
                },
            ),
            (
                "TRE-SHARED".to_string(),
                ServerBinaryTreeAggregateNode {
                    direct: ServerBinaryTreeAggregate {
                        file_count: 2,
                        total_bytes: 7,
                    },
                    child_tree_ids: Vec::new(),
                },
            ),
        ]);
        let mut load_count = BTreeMap::<String, usize>::new();
        let mut load_node = |tree_id: &str| {
            *load_count.entry(tree_id.to_string()).or_default() += 1;
            nodes
                .get(tree_id)
                .cloned()
                .ok_or_else(|| BinaryDbError::corruption("missing test tree"))
        };
        let mut aggregate_cache = BTreeMap::new();
        let aggregate = fold_snapshot_tree_aggregate(
            "TRE-ROOT",
            &mut BTreeSet::new(),
            &mut aggregate_cache,
            &mut load_node,
        )?;
        assert_eq!(
            aggregate,
            ServerBinaryTreeAggregate {
                file_count: 5,
                total_bytes: 17,
            }
        );
        let repeated = fold_snapshot_tree_aggregate(
            "TRE-ROOT",
            &mut BTreeSet::new(),
            &mut aggregate_cache,
            &mut load_node,
        )?;
        assert_eq!(repeated, aggregate);
        drop(load_node);
        assert_eq!(load_count.get("TRE-ROOT"), Some(&1));
        assert_eq!(load_count.get("TRE-SHARED"), Some(&1));
        Ok(())
    }

    #[test]
    fn snapshot_tree_aggregate_rejects_cycles_and_overflow() {
        let cycle_nodes = BTreeMap::from([
            (
                "TRE-A".to_string(),
                ServerBinaryTreeAggregateNode {
                    direct: ServerBinaryTreeAggregate::default(),
                    child_tree_ids: vec!["TRE-B".to_string()],
                },
            ),
            (
                "TRE-B".to_string(),
                ServerBinaryTreeAggregateNode {
                    direct: ServerBinaryTreeAggregate::default(),
                    child_tree_ids: vec!["TRE-A".to_string()],
                },
            ),
        ]);
        let mut load_cycle_node = |tree_id: &str| {
            cycle_nodes
                .get(tree_id)
                .cloned()
                .ok_or_else(|| BinaryDbError::corruption("missing test tree"))
        };
        let cycle_error = fold_snapshot_tree_aggregate(
            "TRE-A",
            &mut BTreeSet::new(),
            &mut BTreeMap::new(),
            &mut load_cycle_node,
        )
        .expect_err("cycle must fail closed");
        assert!(cycle_error.to_string().contains("contains a cycle"));

        let overflow_nodes = BTreeMap::from([
            (
                "TRE-ROOT".to_string(),
                ServerBinaryTreeAggregateNode {
                    direct: ServerBinaryTreeAggregate {
                        file_count: u32::MAX,
                        total_bytes: u64::MAX,
                    },
                    child_tree_ids: vec!["TRE-CHILD".to_string()],
                },
            ),
            (
                "TRE-CHILD".to_string(),
                ServerBinaryTreeAggregateNode {
                    direct: ServerBinaryTreeAggregate {
                        file_count: 1,
                        total_bytes: 1,
                    },
                    child_tree_ids: Vec::new(),
                },
            ),
        ]);
        let mut load_overflow_node = |tree_id: &str| {
            overflow_nodes
                .get(tree_id)
                .cloned()
                .ok_or_else(|| BinaryDbError::corruption("missing test tree"))
        };
        let overflow_error = fold_snapshot_tree_aggregate(
            "TRE-ROOT",
            &mut BTreeSet::new(),
            &mut BTreeMap::new(),
            &mut load_overflow_node,
        )
        .expect_err("aggregate overflow must fail closed");
        assert!(overflow_error
            .to_string()
            .contains("file count exceeds u32"));
    }
}
