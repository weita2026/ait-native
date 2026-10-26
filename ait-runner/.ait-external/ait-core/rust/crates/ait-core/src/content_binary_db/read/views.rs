use crate::content_binary_db::{
    BinaryBlobRecord, BinaryObjectPackMemberRecord, BinaryObjectPackRecord, BinarySnapshotPayload,
    BinarySnapshotRecord, BinaryTreePackRecord, BinaryTreeRecord,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryBlobView {
    pub blob_index: u32,
    pub record: BinaryBlobRecord,
    pub blob_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub pack_member_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinarySnapshotView {
    pub snapshot_index: u32,
    pub record: BinarySnapshotRecord,
    pub payload: BinarySnapshotPayload,
    pub snapshot_id: String,
    pub parent_snapshot_ids: Vec<String>,
    pub primary_parent_snapshot_id: Option<String>,
    pub parent_snapshot_id: Option<String>,
    pub root_tree_pack_id: Option<String>,
    pub root_tree_pack_path: Option<String>,
    pub root_tree_id: Option<String>,
    pub root_tree_index: Option<u32>,
    pub root_entry_ordinal: u32,
    pub manifest_hash: String,
    pub snapshot_kind: String,
    pub file_count: u64,
    pub total_bytes: u64,
    pub created_at_s: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryObjectPackView {
    pub pack_index: u32,
    pub record: BinaryObjectPackRecord,
    pub pack_id: String,
    pub pack_path: String,
    pub pack_format: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryObjectPackMemberView {
    pub member_index: u32,
    pub record: BinaryObjectPackMemberRecord,
    pub pack_id: String,
    pub blob_id: String,
    pub entry_name: String,
    pub base_blob_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryTreePackView {
    pub tree_pack_index: u32,
    pub record: BinaryTreePackRecord,
    pub pack_id: String,
    pub pack_path: String,
    pub pack_format: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryTreeView {
    pub tree_index: u32,
    pub record: BinaryTreeRecord,
    pub tree_id: String,
    pub tree_pack_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinaryTreeEntryView {
    pub entry_ordinal: u32,
    pub entry_name: String,
    pub entry_type: String,
    pub target_id: String,
    pub size_bytes: Option<u64>,
    pub mode: Option<String>,
}
