use super::*;

pub(crate) mod private {
    use super::*;

    /// Raw journal and recovery mutation kept inside server authority.
    pub trait BinaryDbJournalIo {
        fn journal_append_bytes(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64>;

        fn journal_overwrite_range(
            &self,
            path: &Path,
            offset: u64,
            bytes: &[u8],
        ) -> StoreResult<()>;

        fn journal_truncate_file(&self, path: &Path, len: u64) -> StoreResult<()>;

        fn journal_remove_file_if_exists(&self, path: &Path) -> StoreResult<()>;
    }
}

pub trait BinaryDb: private::BinaryDbJournalIo {
    fn authority_root(&self) -> &StorePath;

    /// Optionally orders writers that share one in-process repository runtime.
    /// Filesystem process locks remain authoritative for cross-process safety.
    fn acquire_in_process_write_admission(
        &self,
        _command_scope: BinaryDbCommandScope,
        _max_wait: Option<Duration>,
    ) -> StoreResult<Option<BinaryDbWriterAdmissionGuard>> {
        Ok(None)
    }

    fn acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::acquire(self.authority_root(), command_scope)
    }

    fn try_acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::try_acquire(self.authority_root(), command_scope)
    }

    fn try_acquire_command_scope_union(
        &self,
        command_scope: BinaryDbCommandScope,
        scopes: &[BinaryDbCommandScope],
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::try_acquire_scope_union(
            self.authority_root(),
            command_scope,
            scopes,
        )
    }

    fn try_acquire_recovery_admission_lock(&self) -> StoreResult<BinaryDbRecoveryAdmissionLock> {
        BinaryDbRecoveryAdmissionLock::try_acquire(self.authority_root())
    }

    fn acquire_queued_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
        max_wait: Duration,
        retry_interval: Duration,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        BinaryDbCommandLockSet::acquire_queued(
            self.authority_root(),
            command_scope,
            max_wait,
            retry_interval,
        )
    }

    fn acquire_read_lock(&self) -> StoreResult<BinaryDbReadLockSet> {
        self.acquire_read_lock_for_scope(BinaryDbReadScope::ALL)
    }

    fn acquire_read_lock_for_scope(
        &self,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<BinaryDbReadLockSet> {
        BinaryDbReadLockSet::try_acquire_for_scope(self.authority_root(), read_scope)
    }

    fn path_exists(&self, path: &Path) -> bool {
        ServerBinaryDbFilesystemStore.path_exists(path)
    }

    fn read_to_string(&self, path: &Path) -> StoreResult<String> {
        ServerBinaryDbFilesystemStore.read_to_string(path)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        ServerBinaryDbFilesystemStore.metadata_len(path)
    }

    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_file(path)
    }

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_file_data(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_directory(path)
    }

    fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32>;

    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32>;

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<BinaryRecordBytes>;

    /// Reads one contiguous fixed-record range.
    ///
    /// Backends should override this to avoid opening and validating the same
    /// record file once per row. Results are ordered from `first_record_index`.
    fn read_records(
        &self,
        file: BinaryFileId,
        first_record_index: u32,
        record_count: u32,
    ) -> StoreResult<Vec<BinaryRecordBytes>> {
        let end = first_record_index
            .checked_add(record_count)
            .ok_or_else(|| BinaryDbError::invalid_domain_data("record range index overflow"))?;
        (first_record_index..end)
            .map(|record_index| self.read_record(file.clone(), record_index))
            .collect()
    }

    fn append_record(
        &self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32>;

    /// Appends one contiguous fixed-record batch and returns its aligned
    /// record indexes. Backends should override this to avoid reopening,
    /// restating, and revalidating the same file once per record.
    fn append_records(
        &self,
        file: BinaryFileId,
        records: &[BinaryRecordBytes],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<Vec<u32>> {
        records
            .iter()
            .map(|record| self.append_record(file.clone(), record, write))
            .collect()
    }

    fn overwrite_record(
        &self,
        _file: BinaryFileId,
        _record_index: u32,
        _record: BinaryRecordBytesRef<'_>,
        _write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        Err(BinaryDbError::unsupported(
            "Binary DB backend does not support fixed-record overwrite",
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

    /// Looks up several keys from one logical index snapshot.
    ///
    /// Backends should override this when a single-key lookup has a fixed
    /// setup cost, such as reading or mapping the complete index. The result
    /// vector is aligned with `keys`, including duplicate keys.
    fn lookup_index_many(
        &self,
        index: BinaryIndexId,
        keys: &[BinaryIndexKeyRef<'_>],
    ) -> StoreResult<Vec<Vec<u32>>> {
        keys.iter()
            .map(|key| self.lookup_index(index.clone(), key))
            .collect()
    }
}

pub trait RemoteBinaryDb: BinaryDb {
    fn remote_repo_id(&self) -> &RepoId;

    fn remote_repo_name(&self) -> &RepoName;

    fn remote_authority_root(&self) -> &StorePath;
}

pub trait ServerRemoteBinaryDb: RemoteBinaryDb {
    fn repo_id(&self) -> &RepoId;
    fn repo_name(&self) -> &RepoName;
    fn authority_root(&self) -> &StorePath;
    fn storage_generation(&self) -> StoreGeneration;
    fn authority_mode(&self) -> ServerBinaryDbAuthorityMode;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServerBinaryDbAuthorityContractRow {
    pub invariant: &'static str,
    pub guarantee: &'static str,
}

pub const SERVER_BINARY_DB_AUTHORITY_CONTRACT: &[ServerBinaryDbAuthorityContractRow] = &[
    ServerBinaryDbAuthorityContractRow {
        invariant: "deployed authority writer",
        guarantee: "ait-server serving authority is the sole filesystem writer for a deployed server Binary DB root",
    },
    ServerBinaryDbAuthorityContractRow {
        invariant: "client synchronization",
        guarantee: "clients synchronize through HTTP, compare-and-swap, pack, and import/export contracts and never mutate the deployed authority root directly",
    },
    ServerBinaryDbAuthorityContractRow {
        invariant: "serving transaction ownership",
        guarantee: "serving writes use authority recovery admission, server domain locks, queued acquisition where required, and persistent rollback journals",
    },
    ServerBinaryDbAuthorityContractRow {
        invariant: "network disconnect",
        guarantee: "client disconnect does not transfer or cancel server filesystem transaction ownership; server process crash recovery uses the persistent journal",
    },
    ServerBinaryDbAuthorityContractRow {
        invariant: "independent local locks",
        guarantee: "CLI-local lock names may differ because local and deployed server authority roots have different writers",
    },
];
