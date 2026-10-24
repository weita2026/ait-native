use super::*;

#[derive(Clone, Debug)]
pub(crate) struct SnapshotFileEntry {
    pub(crate) path: String,
    pub(crate) blob_id: String,
    pub(crate) size_bytes: i64,
    pub(crate) mode: String,
    pub(crate) sha256: String,
    pub(crate) data: Vec<u8>,
    /// True when `data` was intentionally omitted because a validated
    /// workspace cache proved that the authoritative blob already exists.
    pub(crate) data_reused: bool,
    /// The post-read filesystem identity used to publish the next derived
    /// workspace hash cache after the Snapshot metadata transaction commits.
    pub(crate) cache_fingerprint: Option<crate::workspace_hash_cache::WorkspaceFileFingerprint>,
}

#[derive(Clone, Debug)]
pub(crate) struct TreeRow {
    pub(crate) tree_id: String,
    pub(crate) entry_count: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct TreeEntryRow {
    pub(crate) tree_id: String,
    pub(crate) entry_name: String,
    pub(crate) entry_type: String,
    pub(crate) target_id: String,
    pub(crate) mode: String,
}

#[derive(Clone, Debug)]
pub(in crate::local_snapshot) enum TreeNode {
    Blob {
        blob_id: String,
        size_bytes: i64,
        mode: String,
    },
    Tree {
        children: BTreeMap<String, TreeNode>,
    },
}
