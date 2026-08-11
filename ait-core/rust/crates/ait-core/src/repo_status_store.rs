use crate::{
    binary_db::{BinaryDb, BinaryDbErrorKind, BinaryDbReadScope, BinaryDbReadTxn, BinaryFileId},
    content_binary_db::{BinaryDbObjectPackStore, BinaryDbSnapshotStore},
};

pub type RepoStatusStoreResult<T> = Result<T, String>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoStatusStorageCounts {
    pub snapshot_count: i64,
    pub pack_count: i64,
    pub packed_blob_count: i64,
}

pub trait RepoStatusStore {
    fn storage_counts(&self) -> RepoStatusStoreResult<RepoStatusStorageCounts>;
}

pub fn storage_counts_with_repo_status_store<S>(
    store: &S,
) -> RepoStatusStoreResult<RepoStatusStorageCounts>
where
    S: RepoStatusStore + ?Sized,
{
    store.storage_counts()
}

#[derive(Clone, Debug)]
pub struct BinaryDbRepoStatusStore<B, const LAYOUT: u32> {
    db: B,
}

impl<B, const LAYOUT: u32> BinaryDbRepoStatusStore<B, LAYOUT> {
    pub fn new(db: B) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &B {
        &self.db
    }
}

impl<B, const LAYOUT: u32> RepoStatusStore for BinaryDbRepoStatusStore<B, LAYOUT>
where
    B: BinaryDb,
{
    fn storage_counts(&self) -> RepoStatusStoreResult<RepoStatusStorageCounts> {
        let read = BinaryDbReadTxn::new_for_scope(&self.db, BinaryDbReadScope::Content);
        let snapshot_count =
            optional_record_count(&read, BinaryDbSnapshotStore::<B, LAYOUT>::snapshot_file())?;
        let pack_count = optional_record_count(
            &read,
            BinaryDbObjectPackStore::<B, LAYOUT>::object_pack_file(),
        )?;
        let packed_blob_count = optional_record_count(
            &read,
            BinaryDbObjectPackStore::<B, LAYOUT>::object_pack_member_file(),
        )?;
        Ok(RepoStatusStorageCounts {
            snapshot_count,
            pack_count,
            packed_blob_count,
        })
    }
}

fn optional_record_count<B: BinaryDb>(
    read: &BinaryDbReadTxn<'_, B>,
    file: BinaryFileId,
) -> RepoStatusStoreResult<i64> {
    match read.record_count(file) {
        Ok(count) => Ok(i64::from(count)),
        Err(error) if error.kind() == BinaryDbErrorKind::MissingData => Ok(0),
        Err(error) => Err(error.to_string()),
    }
}

#[cfg(test)]
mod tests;
