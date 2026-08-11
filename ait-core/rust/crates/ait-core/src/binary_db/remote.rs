use super::{
    AuthorityId, BinaryDb, BinaryDbCommandLockSet, BinaryDbCommandScope, BinaryDbFsyncPolicy,
    BinaryDbIndexAppender, BinaryDbReadLockSet, BinaryDbReadScope, BinaryDbReadTxn,
    BinaryDbStoreFsyncPolicy, BinaryDbWriteTxn, BinaryFileId, BinaryIndexId, BinaryIndexKeyRef,
    BinaryPayloadFileId, BinaryRecordBytes, BinaryRecordBytesRef, BinaryWriteContext,
    LocalBinaryDbFs, LocalStateScope, PayloadRange, RemoteBinaryDb, RepoId, RepoName, StorePath,
    StoreResult,
};
use std::path::Path;

/// Filesystem-backed non-authoritative representation of remote Binary DB data.
///
/// This adapter keeps the same byte-level substrate as `LocalBinaryDbFs`, while
/// making remote identity and local storage purpose explicit at the type
/// boundary. It must not point at a deployed `ait-server` authority root;
/// deployed authority mutations go through server HTTP, CAS, pack, or
/// import/export contracts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemoteBinaryDbFsRole {
    LocalMirror,
    TestFixture,
}

impl RemoteBinaryDbFsRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalMirror => "local_mirror",
            Self::TestFixture => "test_fixture",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RemoteBinaryDbFs {
    inner: LocalBinaryDbFs,
    role: RemoteBinaryDbFsRole,
    remote_repo_id: RepoId,
    remote_repo_name: RepoName,
    remote_authority_root: StorePath,
}

impl RemoteBinaryDbFs {
    pub fn local_mirror(
        mirror_root: impl Into<StorePath>,
        remote_repo_id: RepoId,
        remote_repo_name: RepoName,
    ) -> Self {
        Self::with_role(
            RemoteBinaryDbFsRole::LocalMirror,
            mirror_root,
            remote_repo_id,
            remote_repo_name,
        )
    }

    pub fn test_fixture(
        fixture_root: impl Into<StorePath>,
        remote_repo_id: RepoId,
        remote_repo_name: RepoName,
    ) -> Self {
        Self::with_role(
            RemoteBinaryDbFsRole::TestFixture,
            fixture_root,
            remote_repo_id.clone(),
            remote_repo_name,
        )
    }

    fn with_role(
        role: RemoteBinaryDbFsRole,
        authority_root: impl Into<StorePath>,
        remote_repo_id: RepoId,
        remote_repo_name: RepoName,
    ) -> Self {
        let authority_root = authority_root.into();
        let remote_authority_root = authority_root.clone();
        let local_authority_id =
            AuthorityId::new(format!("remote-{}:{}", role.as_str(), remote_repo_id.0));
        Self {
            inner: LocalBinaryDbFs::new(
                authority_root.clone(),
                remote_authority_root.clone(),
                local_authority_id,
                LocalStateScope::RemoteCache,
            ),
            role,
            remote_repo_id,
            remote_repo_name,
            remote_authority_root,
        }
    }

    pub fn role(&self) -> RemoteBinaryDbFsRole {
        self.role
    }

    pub fn begin_read_txn(&self) -> BinaryDbReadTxn<'_, Self> {
        BinaryDbReadTxn::new(self)
    }

    pub fn begin_write_txn(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbWriteTxn<'_, Self, BinaryDbStoreFsyncPolicy<'_, Self>>> {
        BinaryDbWriteTxn::begin(self, command_scope)
    }

    pub fn begin_write_txn_with_fsync_policy<F>(
        &self,
        command_scope: BinaryDbCommandScope,
        fsync_policy: F,
    ) -> StoreResult<BinaryDbWriteTxn<'_, Self, F>>
    where
        F: BinaryDbFsyncPolicy,
    {
        BinaryDbWriteTxn::begin_with_fsync_policy(self, command_scope, fsync_policy)
    }

    pub fn inner(&self) -> &LocalBinaryDbFs {
        &self.inner
    }
}

impl BinaryDb for RemoteBinaryDbFs {
    fn authority_root(&self) -> &StorePath {
        self.inner.authority_root()
    }

    fn acquire_command_lock(
        &self,
        command_scope: BinaryDbCommandScope,
    ) -> StoreResult<BinaryDbCommandLockSet> {
        self.inner.acquire_command_lock(command_scope)
    }

    fn acquire_read_lock(&self) -> StoreResult<BinaryDbReadLockSet> {
        self.inner.acquire_read_lock()
    }

    fn acquire_read_lock_for_scope(
        &self,
        read_scope: BinaryDbReadScope,
    ) -> StoreResult<BinaryDbReadLockSet> {
        self.inner.acquire_read_lock_for_scope(read_scope)
    }

    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        self.inner.sync_file(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.inner.sync_directory(path)
    }

    fn metadata_len(&self, path: &Path) -> StoreResult<Option<u64>> {
        self.inner.metadata_len(path)
    }

    fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.inner.layout_id(file)
    }

    fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.inner.record_count(file)
    }

    fn read_record(&self, file: BinaryFileId, record_index: u32) -> StoreResult<BinaryRecordBytes> {
        self.inner.read_record(file, record_index)
    }

    fn append_record(
        &self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<u32> {
        self.inner.append_record(file, record, write)
    }

    fn append_records(
        &self,
        file: BinaryFileId,
        records: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<(u32, u32)> {
        self.inner.append_records(file, records, write)
    }

    fn overwrite_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
        record: BinaryRecordBytesRef<'_>,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        self.inner
            .overwrite_record(file, record_index, record, write)
    }

    fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.inner.read_payload(file, offset, len)
    }

    fn append_payload(
        &self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<PayloadRange> {
        self.inner.append_payload(file, bytes, write)
    }

    fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        self.inner.lookup_index(index, key)
    }
}

impl crate::binary_db::BinaryDbRecoveryIo for RemoteBinaryDbFs {
    fn recovery_truncate_file(&self, path: &Path, len: u64) -> StoreResult<()> {
        self.inner.recovery_truncate_file(path, len)
    }

    fn recovery_remove_file_if_exists(&self, path: &Path) -> StoreResult<()> {
        self.inner.recovery_remove_file_if_exists(path)
    }
}

impl BinaryDbIndexAppender for RemoteBinaryDbFs {
    fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        self.inner
            .append_index_candidate(index, key, record_index, write)
    }
}

impl RemoteBinaryDb for RemoteBinaryDbFs {
    fn remote_repo_id(&self) -> &RepoId {
        &self.remote_repo_id
    }

    fn remote_repo_name(&self) -> &RepoName {
        &self.remote_repo_name
    }

    fn remote_authority_root(&self) -> &StorePath {
        &self.remote_authority_root
    }
}
