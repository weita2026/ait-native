pub const SNAPSHOT_PARENT_EXTENSION_VERSION: u8 = 1;
pub use crate::snapshot_store::MAX_SNAPSHOT_PARENT_COUNT;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BinarySnapshotPayload {
    pub line_name: String,
    pub message: Option<String>,
    pub additional_parent_snapshot_indices: Vec<u32>,
}
