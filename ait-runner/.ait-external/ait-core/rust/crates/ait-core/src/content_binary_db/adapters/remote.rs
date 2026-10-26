use crate::binary_db::{RemoteBinaryDb, RemoteBinaryDbFs, StorePath};
use crate::content_binary_db::{
    BinaryDbBlobStore, BinaryDbObjectPackStore, BinaryDbSnapshotStore, BinaryDbTreePackStore,
    BinaryDbTreeStore,
};

/// Remote content Binary DB adapter.
///
/// Remote sync policy and leases live above this adapter. This type only makes
/// the remote authority byte substrate explicit for content metadata reads and
/// transaction-scoped writes.
#[derive(Clone, Debug)]
pub struct RemoteContentBinaryDb<B, const WRITE_LAYOUT: u32>
where
    B: RemoteBinaryDb + Clone,
{
    db: B,
    repo_root: StorePath,
    blobs: BinaryDbBlobStore<B, WRITE_LAYOUT>,
    snapshots: BinaryDbSnapshotStore<B, WRITE_LAYOUT>,
    object_packs: BinaryDbObjectPackStore<B, WRITE_LAYOUT>,
    tree_packs: BinaryDbTreePackStore<B, WRITE_LAYOUT>,
    trees: BinaryDbTreeStore<B, WRITE_LAYOUT>,
}

pub type RemoteFsContentBinaryDb<const WRITE_LAYOUT: u32> =
    RemoteContentBinaryDb<RemoteBinaryDbFs, WRITE_LAYOUT>;

impl<B, const WRITE_LAYOUT: u32> RemoteContentBinaryDb<B, WRITE_LAYOUT>
where
    B: RemoteBinaryDb + Clone,
{
    pub fn from_db(db: B, repo_root: impl Into<StorePath>) -> Self {
        let repo_root = repo_root.into();
        Self {
            blobs: BinaryDbBlobStore::new(db.clone(), repo_root.clone()),
            snapshots: BinaryDbSnapshotStore::new(db.clone(), repo_root.clone()),
            object_packs: BinaryDbObjectPackStore::new(db.clone(), repo_root.clone()),
            tree_packs: BinaryDbTreePackStore::new(db.clone(), repo_root.clone()),
            trees: BinaryDbTreeStore::new(db.clone(), repo_root.clone()),
            db,
            repo_root,
        }
    }

    pub fn db(&self) -> &B {
        &self.db
    }

    pub fn repo_root(&self) -> &StorePath {
        &self.repo_root
    }

    pub fn blobs(&self) -> &BinaryDbBlobStore<B, WRITE_LAYOUT> {
        &self.blobs
    }

    pub fn snapshots(&self) -> &BinaryDbSnapshotStore<B, WRITE_LAYOUT> {
        &self.snapshots
    }

    pub fn object_packs(&self) -> &BinaryDbObjectPackStore<B, WRITE_LAYOUT> {
        &self.object_packs
    }

    pub fn tree_packs(&self) -> &BinaryDbTreePackStore<B, WRITE_LAYOUT> {
        &self.tree_packs
    }

    pub fn trees(&self) -> &BinaryDbTreeStore<B, WRITE_LAYOUT> {
        &self.trees
    }
}
