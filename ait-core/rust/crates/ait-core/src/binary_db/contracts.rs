//! Public storage contracts shared by local and remote Binary DB adapters.

use super::*;
use std::collections::HashMap;

pub trait BinaryDbIndexAppender {
    fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()>;
}

/// Byte addressing of a record-like storage file.
pub const BIN_FILE_HEADER_BYTES: u32 = 4;

/// Immutable file bytes reused only while one read transaction owns its lock set.
///
/// The cache is intentionally transaction-local: dropping the transaction drops
/// every entry, and a later transaction reads and validates current filesystem
/// state again.
#[derive(Debug, Default)]
pub struct BinaryDbReadCache {
    pub(crate) layout_ids: HashMap<BinaryFileId, u32>,
    pub(crate) record_files: HashMap<BinaryFileId, Option<Vec<u8>>>,
    pub(crate) index_files: HashMap<BinaryIndexId, Option<Vec<u8>>>,
    pub(crate) parsed_index_candidates: HashMap<BinaryIndexId, HashMap<Vec<u8>, Vec<u32>>>,
}

pub(crate) mod private {
    use super::*;

    /// Raw file mutation used only to restore a transaction before-image.
    ///
    /// Keeping this contract in a crate-visible module seals it from
    /// downstream callers while still allowing in-crate adapters and test
    /// doubles to preserve their injected I/O authority.
    pub trait BinaryDbRecoveryIo {
        fn recovery_truncate_file(&self, path: &Path, len: u64) -> StoreResult<()>;

        fn recovery_remove_file_if_exists(&self, path: &Path) -> StoreResult<()>;
    }
}

/// Base substrate of Binary DB storage.
pub trait BinaryDb: private::BinaryDbRecoveryIo {
    fn authority_root(&self) -> &StorePath;

    fn acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::acquire(self.authority_root(), command_scope)
    }

    fn acquire_read_lock(&self) -> StoreResult<BinaryDbReadLockSet> {
        self.acquire_read_lock_for_scope(BinaryDbReadScope::All)
    }

    fn acquire_read_lock_for_scope(
        &self,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<BinaryDbReadLockSet> {
        BinaryDbReadLockSet::try_acquire_for_scope(self.authority_root(), read_scope)
    }

    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        FilesystemFileIoStore.sync_file(path).map_err(|e| {
            file_io_error_to_binary(format!("sync Binary DB file {}", path.display()), e)
        })
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        FilesystemFileIoStore.sync_dir(path).map_err(|e| {
            file_io_error_to_binary(format!("sync Binary DB directory {}", path.display()), e)
        })
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        FilesystemFileIoStore.metadata_len(path).map_err(|e| {
            file_io_error_to_binary(format!("read Binary DB metadata {}", path.display()), e)
        })
    }

    /// Replaces one complete authority file through a same-filesystem staging
    /// directory outside the active Binary DB root.
    ///
    /// Success means the atomic rename crossed. The caller still owns syncing
    /// the returned staging directory and the target parent directory. An
    /// error guarantees that the target was not replaced.
    fn replace_file_atomically(
        &self,
        path: &Path,
        bytes: &[u8],
        publish_label: &str,
    ) -> StoreResult<PathBuf> {
        let staging_directory = binary_db_atomic_staging_directory(self.authority_root())?;
        FilesystemFileIoStore
            .write_bytes_atomically_from_directory(path, &staging_directory, bytes, publish_label)
            .map_err(|e| {
                file_io_error_to_binary(
                    format!("atomically replace Binary DB file {}", path.display()),
                    e,
                )
            })?;
        Ok(staging_directory)
    }

    fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32>;

    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32>;

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<BinaryRecordBytes>;

    /// Reads a record through one read transaction's immutable file cache.
    ///
    /// Non-filesystem adapters retain their existing behavior through this
    /// default. Filesystem adapters may reuse validated bytes while the caller
    /// holds the read transaction lock set.
    fn read_record_in_read_txn(
        &self,
        file: BinaryFileId,
        record_index: u32,
        _cache: &mut BinaryDbReadCache,
    ) -> StoreResult<BinaryRecordBytes> {
        self.read_record(file, record_index)
    }

    fn append_record(
        &self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32>;

    /// Appends a contiguous batch of fixed-size records.
    ///
    /// Adapters may override this to publish the batch with one physical
    /// write. The default keeps existing adapters source-compatible while
    /// enforcing dense indexes and the same fixed-record contract.
    fn append_records(
        &self,
        file: BinaryFileId,
        records: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<(u32, u32)> {
        write.ensure_authorized_path(file.relative_path())?;
        let record_size = usize::try_from(file.record_size())
            .map_err(|_| format!("record size overflow: {}", file.record_size()))?;
        if record_size == 0 {
            return Err(BinaryDbError::invalid_domain_data(
                "fixed record batch has a zero record size",
            ));
        }
        if !records.len().is_multiple_of(record_size) {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "fixed record batch length {} is not aligned to record size {}",
                records.len(),
                file.record_size()
            )));
        }

        let batch_count = u32::try_from(records.len() / record_size).map_err(|_| {
            BinaryDbError::invalid_domain_data("fixed record batch count exceeds u32::MAX")
        })?;
        let start_index = self.record_count(file.clone())?;
        start_index.checked_add(batch_count).ok_or_else(|| {
            BinaryDbError::invalid_domain_data("fixed record batch count overflows u32")
        })?;

        for (offset, record) in records.chunks_exact(record_size).enumerate() {
            let offset = u32::try_from(offset).map_err(|_| {
                BinaryDbError::invalid_domain_data("fixed record batch offset exceeds u32::MAX")
            })?;
            let expected_index = start_index.checked_add(offset).ok_or_else(|| {
                BinaryDbError::invalid_domain_data("fixed record batch index overflows u32")
            })?;
            let actual_index = self.append_record(file.clone(), record, write)?;
            if actual_index != expected_index {
                return Err(BinaryDbError::corruption(format!(
                    "fixed record batch index changed: expected {expected_index}, got {actual_index}"
                )));
            }
        }
        Ok((start_index, batch_count))
    }

    fn overwrite_record(
        &self,
        _file: BinaryFileId,
        _record_index: u32,
        _record: BinaryRecordBytesRef<'_>,
        _write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        Err(BinaryDbError::unsupported(
            "fixed-record overwrite is not supported by this Binary DB adapter",
        ))
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>>;

    fn append_payload(
        &self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<PayloadRange>;

    fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>>;

    /// Looks up an index key through one read transaction's immutable file cache.
    fn lookup_index_in_read_txn(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        _cache: &mut BinaryDbReadCache,
    ) -> StoreResult<Vec<u32>> {
        self.lookup_index(index, key)
    }
}

fn binary_db_atomic_staging_directory(authority_root: &StorePath) -> StoreResult<PathBuf> {
    let authority_parent = authority_root.as_path().parent().ok_or_else(|| {
        BinaryDbError::invalid_domain_data(format!(
            "Binary DB authority root has no staging parent: {}",
            authority_root.as_path().display()
        ))
    })?;
    Ok(authority_parent.join("binary-db-staging"))
}

#[derive(Clone, Copy, Debug)]
pub struct BinaryDbStoreFsyncPolicy<'a, B: BinaryDb + ?Sized> {
    db: &'a B,
}

impl<'a, B> BinaryDbStoreFsyncPolicy<'a, B>
where
    B: BinaryDb + ?Sized,
{
    pub fn new(db: &'a B) -> Self {
        Self { db }
    }
}

impl<'a, B> BinaryDbFsyncPolicy for BinaryDbStoreFsyncPolicy<'a, B>
where
    B: BinaryDb + ?Sized,
{
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.db.sync_file(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.db.sync_directory(path)
    }
}

pub trait LocalBinaryDb: BinaryDb {
    fn local_repo_root(&self) -> &StorePath;

    fn local_authority_id(&self) -> &AuthorityId;

    fn current_line_state_scope(&self) -> LocalStateScope;
}

pub trait RemoteBinaryDb: BinaryDb {
    fn remote_repo_id(&self) -> &RepoId;

    fn remote_repo_name(&self) -> &RepoName;

    fn remote_authority_root(&self) -> &StorePath;
}
