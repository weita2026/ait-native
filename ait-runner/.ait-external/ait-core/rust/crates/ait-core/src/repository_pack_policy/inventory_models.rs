use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryObjectPackInventoryRow {
    pub pack_id: String,
    pub repo_name: Option<String>,
    pub repo_id: Option<String>,
    pub status: String,
    pub pack_format: PackFormatKind,
    pub member_count: i64,
    pub total_bytes: i64,
    pub pack_path: String,
    pub pack_index_entry_name: String,
    pub pack_index_checksum: String,
    pub created_at: String,
    pub embedded_index: ObjectPackIndexInventory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPackIndexInventory {
    pub pack_id: String,
    pub pack_format: PackFormatKind,
    pub member_count: i64,
    pub total_bytes: i64,
    pub entries: Vec<ObjectPackIndexEntryInventory>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectPackIndexEntryInventory {
    pub entry_name: String,
    pub blob_id: String,
    pub entry_type: String,
    pub checksum: String,
    pub base_blob_id: Option<String>,
    pub chain_depth: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryTreePackInventoryRow {
    pub pack_id: String,
    pub repo_name: Option<String>,
    pub repo_id: Option<String>,
    pub status: String,
    pub pack_format: TreePackFormatKind,
    pub tree_count: i64,
    pub total_bytes: i64,
    pub pack_path: String,
    pub pack_index_entry_name: String,
    pub pack_index_checksum: String,
    pub created_at: String,
    pub embedded_index: TreePackIndexInventory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreePackIndexInventory {
    pub pack_id: String,
    pub pack_format: TreePackFormatKind,
    pub tree_count: i64,
    pub total_bytes: i64,
    pub trees: Vec<TreePackIndexEntryInventory>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreePackIndexEntryInventory {
    pub tree_id: String,
    pub entry_ordinal: i64,
    pub entry_count: i64,
    pub checksum: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryBlobLocatorInventoryRow {
    pub blob_id: String,
    pub sha256: String,
    pub size_bytes: i64,
    pub pack_id: String,
    pub pack_entry_name: String,
    pub pack_entry_type: String,
    pub pack_base_blob_id: Option<String>,
    pub pack_chain_depth: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryTreeLocatorInventoryRow {
    pub tree_id: String,
    pub entry_count: i64,
    pub tree_pack_id: String,
    pub tree_pack_checksum: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositorySnapshotInventoryRow {
    pub snapshot_id: String,
    pub parent_snapshot_ids: Vec<String>,
    pub primary_parent_snapshot_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub root_tree_pack_id: String,
    pub root_entry_ordinal: i64,
    pub manifest_hash: String,
    pub message: Option<String>,
    pub line_name: Option<String>,
    pub snapshot_kind: Option<String>,
    pub file_count: i64,
    pub total_bytes: i64,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositoryLineHeadInventoryRow {
    pub line_name: String,
    pub head_snapshot_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepositoryPackInventorySource {
    LocalBinaryDb { repo_root: String },
    ServerPostgres { server_url: String },
    ConvertedZstd { source_path: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvertedZstdInventory {
    pub inventory: RepositoryPackInventory,
    pub source: RepositoryPackInventorySource,
    pub snapshot_conversion_order: Vec<String>,
    pub snapshot_path_blobs: Vec<RepositorySnapshotPathBlobInventoryRow>,
    pub source_packed_blob_ids: Vec<String>,
    pub orphan_object_pack_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositorySnapshotPathBlobInventoryRow {
    pub snapshot_id: String,
    pub path: String,
    pub blob_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConvertedZstdVerificationSummary {
    pub snapshot_order: Vec<String>,
    pub source_packed_blob_count: usize,
    pub unreachable_packed_blob_count: usize,
    pub orphan_pack_count: usize,
}
