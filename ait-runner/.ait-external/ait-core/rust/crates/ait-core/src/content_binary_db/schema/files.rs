pub const BLOB_BIN: &str = "blob.bin";
pub const SNAPSHOT_BIN: &str = "snapshot.bin";
pub const SNAPSHOT_PAYLOAD_BIN: &str = "snapshot_payload.bin";
pub const OBJECT_PACK_BIN: &str = "object_pack.bin";
pub const OBJECT_PACK_MEMBER_BIN: &str = "object_pack_member.bin";
pub const TREE_PACK_BIN: &str = "tree_pack.bin";
pub const TREE_BIN: &str = "tree.bin";

pub const BLOB_ID_IDX: &str = "blob_id.idx";
pub const SNAPSHOT_ID_IDX: &str = "snapshot_id.idx";
pub const OBJECT_PACK_ID_IDX: &str = "object_pack_id.idx";
pub const TREE_ID_IDX: &str = "tree_id.idx";
pub const TREE_PACK_ID_IDX: &str = "tree_pack_id.idx";

pub const BLOB_RECORD_SIZE: u32 = 64;
pub const SNAPSHOT_RECORD_SIZE: u32 = 88;
pub const OBJECT_PACK_RECORD_SIZE: u32 = 32;
pub const OBJECT_PACK_MEMBER_RECORD_SIZE: u32 = 16;
pub const TREE_PACK_RECORD_SIZE: u32 = 32;
pub const TREE_RECORD_SIZE: u32 = 20;

pub(crate) const BLOB_RECORD_SIZE_USIZE: usize = BLOB_RECORD_SIZE as usize;
pub(crate) const SNAPSHOT_RECORD_SIZE_USIZE: usize = SNAPSHOT_RECORD_SIZE as usize;
pub(crate) const OBJECT_PACK_RECORD_SIZE_USIZE: usize = OBJECT_PACK_RECORD_SIZE as usize;
pub(crate) const OBJECT_PACK_MEMBER_RECORD_SIZE_USIZE: usize =
    OBJECT_PACK_MEMBER_RECORD_SIZE as usize;
pub(crate) const TREE_PACK_RECORD_SIZE_USIZE: usize = TREE_PACK_RECORD_SIZE as usize;
pub(crate) const TREE_RECORD_SIZE_USIZE: usize = TREE_RECORD_SIZE as usize;
