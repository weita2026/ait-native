use crate::json_support::JsonValue;
use crate::line_store::LineStore;
use crate::snapshot_store::SnapshotStore;
use std::collections::BTreeMap;

pub type ContentStoreResult<T> = Result<T, String>;

pub trait ContentStoreBundle {
    type Snapshots: SnapshotStore;
    type Lines: LineStore;
    type Blobs: BlobStore;
    type Trees: TreeStore;
    type ObjectPacks: ObjectPackStore;
    type TreePacks: TreePackStore;

    fn snapshots(&self) -> &Self::Snapshots;
    fn lines(&self) -> &Self::Lines;
    fn blobs(&self) -> &Self::Blobs;
    fn trees(&self) -> &Self::Trees;
    fn object_packs(&self) -> &Self::ObjectPacks;
    fn tree_packs(&self) -> &Self::TreePacks;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPackLocator {
    pub pack_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlobRecord {
    pub blob_id: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub object_pack_locator: Option<ObjectPackLocator>,
    pub pack_entry_name: Option<String>,
    pub base_blob_id: Option<String>,
    pub pack_entry_type: Option<String>,
    pub pack_chain_depth: Option<i64>,
}

pub struct EnsureBlobInput<'a> {
    pub data: &'a [u8],
    pub path_hint: Option<&'a str>,
}

pub trait BlobStore {
    fn get_blob(&self, blob_id: &str) -> ContentStoreResult<Option<BlobRecord>>;

    fn get_blobs(&self, blob_ids: &[String]) -> ContentStoreResult<Vec<BlobRecord>> {
        let mut records = Vec::new();
        for blob_id in blob_ids {
            if let Some(record) = self.get_blob(blob_id)? {
                records.push(record);
            }
        }
        Ok(records)
    }

    fn read_blob_bytes(&self, blob_id: &str) -> ContentStoreResult<Vec<u8>>;

    fn read_blob_bytes_batch(
        &self,
        blob_ids: &[String],
    ) -> ContentStoreResult<BTreeMap<String, Vec<u8>>> {
        let mut bytes_by_blob_id = BTreeMap::new();
        for blob_id in blob_ids {
            if bytes_by_blob_id.contains_key(blob_id) {
                continue;
            }
            bytes_by_blob_id.insert(blob_id.clone(), self.read_blob_bytes(blob_id)?);
        }
        Ok(bytes_by_blob_id)
    }

    fn ensure_blob_bytes(&self, input: EnsureBlobInput<'_>) -> ContentStoreResult<BlobRecord>;
}

pub fn ensure_blob_bytes_with_blob_store<S>(
    store: &S,
    input: EnsureBlobInput<'_>,
) -> ContentStoreResult<BlobRecord>
where
    S: BlobStore + ?Sized,
{
    store.ensure_blob_bytes(input)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPackRecord {
    pub pack_id: String,
    pub pack_path: String,
    pub pack_format: String,
    pub member_count: i64,
    pub total_bytes: i64,
    pub index_entry_name: Option<String>,
    pub index_checksum: Option<String>,
    pub created_at: Option<String>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPackMemberRecord {
    pub pack_id: String,
    pub blob_id: String,
    pub entry_name: String,
    pub base_blob_id: Option<String>,
}

pub struct RecordObjectPackInput<'a> {
    pub pack_id: &'a str,
    pub pack_path: &'a str,
    pub pack_format: &'a str,
}

pub trait ObjectPackStore {
    fn get_object_pack(&self, pack_id: &str) -> ContentStoreResult<Option<ObjectPackRecord>>;
    fn list_object_pack_ids(&self) -> ContentStoreResult<Vec<String>>;
    fn list_referenced_object_pack_ids(&self) -> ContentStoreResult<Vec<String>> {
        self.list_object_pack_ids()
    }
    fn list_object_pack_members(
        &self,
        pack_id: &str,
    ) -> ContentStoreResult<Vec<ObjectPackMemberRecord>>;
    fn record_object_pack(
        &self,
        input: RecordObjectPackInput<'_>,
    ) -> ContentStoreResult<ObjectPackRecord>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepoPath(pub String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeRecord {
    pub tree_id: String,
    pub entry_count: Option<i64>,
    pub tree_pack_id: Option<String>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreeEntryRecord {
    pub path: String,
    pub blob_id: Option<String>,
    pub tree_id: Option<String>,
    pub mode: Option<String>,
    pub size_bytes: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SnapshotPathBlobRecord {
    pub snapshot_index: usize,
    pub blob_id: String,
}

pub struct RecordTreeInput<'a> {
    pub tree_id: &'a str,
}

pub trait TreeStore {
    fn get_tree(&self, tree_id: &str) -> ContentStoreResult<Option<TreeRecord>>;
    fn list_tree_entries(&self, tree_id: &str) -> ContentStoreResult<Vec<TreeEntryRecord>>;
    fn snapshot_root_entries(&self, snapshot_id: &str) -> ContentStoreResult<Vec<TreeEntryRecord>>;
    fn snapshot_path_blob(
        &self,
        snapshot_id: &str,
        path: &RepoPath,
    ) -> ContentStoreResult<Option<BlobRecord>>;
    fn snapshot_path_blobs_for_snapshots(
        &self,
        snapshot_ids: &[String],
        path: &RepoPath,
    ) -> ContentStoreResult<Vec<SnapshotPathBlobRecord>> {
        let mut records = Vec::new();
        for (snapshot_index, snapshot_id) in snapshot_ids.iter().enumerate() {
            if let Some(record) = self.snapshot_path_blob(snapshot_id, path)? {
                records.push(SnapshotPathBlobRecord {
                    snapshot_index,
                    blob_id: record.blob_id,
                });
            }
        }
        Ok(records)
    }
    fn record_tree(&self, input: RecordTreeInput<'_>) -> ContentStoreResult<TreeRecord>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreePackRecord {
    pub pack_id: String,
    pub pack_path: String,
    pub pack_format: String,
    pub entry_count: Option<i64>,
    pub checksum: Option<String>,
    pub status: Option<String>,
}

pub struct RecordTreePackInput<'a> {
    pub pack_id: &'a str,
    pub pack_path: &'a str,
    pub pack_format: &'a str,
}

pub trait TreePackStore {
    fn get_tree_pack(&self, pack_id: &str) -> ContentStoreResult<Option<TreePackRecord>>;
    fn read_tree_payload(&self, tree_id: &str) -> ContentStoreResult<Option<JsonValue>>;
    fn record_tree_pack(
        &self,
        input: RecordTreePackInput<'_>,
    ) -> ContentStoreResult<TreePackRecord>;
}
