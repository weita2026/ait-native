use super::{
    storage_counts_with_repo_status_store, BinaryDbRepoStatusStore, RepoStatusStorageCounts,
    RepoStatusStore, RepoStatusStoreResult,
};
use crate::binary_db::{
    AuthorityId, BinaryDbCommandLockSet, BinaryDbCommandScope, LocalBinaryDbFs, LocalStateScope,
    StorePath, REPOSITORY_BINARY_DB_BIN_PATHS,
};
use std::fs;
use tempfile::TempDir;

struct FakeRepoStatusStore;

impl RepoStatusStore for FakeRepoStatusStore {
    fn storage_counts(&self) -> RepoStatusStoreResult<RepoStatusStorageCounts> {
        Ok(RepoStatusStorageCounts {
            snapshot_count: 1,
            pack_count: 2,
            packed_blob_count: 3,
        })
    }
}

#[test]
fn repo_status_store_helper_accepts_trait_object() {
    let store = FakeRepoStatusStore;
    let repo_status_store: &dyn RepoStatusStore = &store;

    assert_eq!(
        storage_counts_with_repo_status_store(repo_status_store).unwrap(),
        RepoStatusStorageCounts {
            snapshot_count: 1,
            pack_count: 2,
            packed_blob_count: 3,
        }
    );
}

#[test]
fn binary_repo_status_reads_three_fixed_counts_and_releases_content_lock() {
    let temp = TempDir::new().unwrap();
    let authority = temp.path().join("binary-db");
    fs::create_dir_all(&authority).unwrap();
    write_fixed_file(&authority.join("snapshot.bin"), 88, 2);
    write_fixed_file(&authority.join("object_pack.bin"), 32, 3);
    write_fixed_file(&authority.join("object_pack_member.bin"), 16, 5);
    let db = LocalBinaryDbFs::new(
        authority.clone(),
        temp.path().to_path_buf(),
        AuthorityId::new("status-test"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(crate::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS);
    let store = BinaryDbRepoStatusStore::<_, 1>::new(db);

    assert_eq!(
        store.storage_counts().unwrap(),
        RepoStatusStorageCounts {
            snapshot_count: 2,
            pack_count: 3,
            packed_blob_count: 5,
        }
    );
    assert!(BinaryDbCommandLockSet::try_acquire(
        &StorePath::from(authority),
        BinaryDbCommandScope::ContentWrite,
    )
    .unwrap()
    .is_some());
}

#[test]
fn binary_repo_status_treats_absent_optional_tables_as_zero() {
    let temp = TempDir::new().unwrap();
    let authority = temp.path().join("binary-db");
    fs::create_dir_all(&authority).unwrap();
    let db = LocalBinaryDbFs::new(
        authority,
        temp.path().to_path_buf(),
        AuthorityId::new("status-empty-test"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(REPOSITORY_BINARY_DB_BIN_PATHS)
    .with_declared_index_paths(crate::binary_db::REPOSITORY_BINARY_DB_INDEX_PATHS);

    assert_eq!(
        BinaryDbRepoStatusStore::<_, 1>::new(db)
            .storage_counts()
            .unwrap(),
        RepoStatusStorageCounts {
            snapshot_count: 0,
            pack_count: 0,
            packed_blob_count: 0,
        }
    );
}

fn write_fixed_file(path: &std::path::Path, record_size: usize, count: usize) {
    let mut bytes = 1_u32.to_le_bytes().to_vec();
    bytes.resize(4 + record_size * count, 0);
    fs::write(path, bytes).unwrap();
}
