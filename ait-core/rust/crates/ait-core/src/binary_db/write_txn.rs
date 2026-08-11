//! Binary DB write transaction lifecycle and rollback tracking.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
struct BinaryDbRollbackFile {
    path: PathBuf,
    existed: bool,
    len: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BinaryDbRollbackRecord {
    file: BinaryFileId,
    record_index: u32,
    bytes: BinaryRecordBytes,
}

/// Result of crossing the durable Binary DB commit point.
///
/// Lock metadata cleanup happens after data durability. A cleanup warning is
/// observable here, but it never changes a durable commit into a retryable
/// failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbCommitOutcome {
    lock_cleanup_warning: Option<BinaryDbError>,
}

impl BinaryDbCommitOutcome {
    fn new(lock_cleanup_warning: Option<BinaryDbError>) -> Self {
        Self {
            lock_cleanup_warning,
        }
    }

    pub fn committed_cleanly(&self) -> bool {
        self.lock_cleanup_warning.is_none()
    }

    pub fn lock_cleanup_warning(&self) -> Option<&BinaryDbError> {
        self.lock_cleanup_warning.as_ref()
    }

    pub fn into_lock_cleanup_warning(self) -> Option<BinaryDbError> {
        self.lock_cleanup_warning
    }
}

/// API-level write transaction surface for Binary DB access.
pub struct BinaryDbWriteTxn<'a, B: BinaryDb + ?Sized, F: BinaryDbFsyncPolicy> {
    db: &'a B,
    lock: BinaryDbCommandLockSet,
    write_context: BinaryWriteContext,
    fsync_policy: F,
    touched_files: Vec<PathBuf>,
    touched_directories: Vec<PathBuf>,
    rollback_files: Vec<BinaryDbRollbackFile>,
    rollback_records: Vec<BinaryDbRollbackRecord>,
    commit_outcome: Option<BinaryDbCommitOutcome>,
    finished: bool,
}

impl<'a, B> BinaryDbWriteTxn<'a, B, BinaryDbStoreFsyncPolicy<'a, B>>
where
    B: BinaryDb + ?Sized,
{
    pub fn begin(db: &'a B, command_scope: BinaryDbCommandScope) -> StoreResult<Self> {
        Self::begin_with_fsync_policy(db, command_scope, BinaryDbStoreFsyncPolicy::new(db))
    }
}

impl<'a, B, F> BinaryDbWriteTxn<'a, B, F>
where
    B: BinaryDb + ?Sized,
    F: BinaryDbFsyncPolicy,
{
    pub fn begin_with_fsync_policy(
        db: &'a B,
        command_scope: BinaryDbCommandScope,
        fsync_policy: F,
    ) -> StoreResult<Self> {
        let lock = db.acquire_command_lock(command_scope)?;
        Ok(Self {
            db,
            lock,
            write_context: BinaryWriteContext::new(command_scope),
            fsync_policy,
            touched_files: Vec::new(),
            touched_directories: Vec::new(),
            rollback_files: Vec::new(),
            rollback_records: Vec::new(),
            commit_outcome: None,
            finished: false,
        })
    }

    pub fn command_scope(&self) -> BinaryDbCommandScope {
        self.lock.command_scope()
    }

    pub fn lock_paths(&self) -> Vec<PathBuf> {
        self.lock.paths().to_vec()
    }

    #[cfg(test)]
    pub(crate) fn write_context(&mut self) -> &mut BinaryWriteContext {
        &mut self.write_context
    }

    pub fn db(&self) -> &'a B {
        self.db
    }

    pub fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.db.record_count(file)
    }

    pub fn read_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
    ) -> StoreResult<BinaryRecordBytes> {
        self.db.read_record(file, record_index)
    }

    pub fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.db.read_payload(file, offset, len)
    }

    pub fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        self.db.lookup_index(index, key)
    }

    pub fn append_record(
        &mut self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
    ) -> StoreResult<u32> {
        self.ensure_write_open()?;
        self.track_record_file_for_write(file.clone())?;
        let index = self
            .db
            .append_record(file, record, &mut self.write_context)?;
        Ok(index)
    }

    pub fn append_records(
        &mut self,
        file: BinaryFileId,
        records: &[u8],
    ) -> StoreResult<(u32, u32)> {
        self.ensure_write_open()?;
        self.track_record_file_for_write(file.clone())?;
        self.db
            .append_records(file, records, &mut self.write_context)
    }

    pub fn overwrite_record(
        &mut self,
        file: BinaryFileId,
        record_index: u32,
        record: BinaryRecordBytesRef<'_>,
    ) -> StoreResult<()> {
        self.ensure_write_open()?;
        self.track_record_file_for_write(file.clone())?;
        if !self
            .rollback_records
            .iter()
            .any(|entry| entry.file == file && entry.record_index == record_index)
        {
            let bytes = self.db.read_record(file.clone(), record_index)?;
            self.rollback_records.push(BinaryDbRollbackRecord {
                file: file.clone(),
                record_index,
                bytes,
            });
        }
        self.db
            .overwrite_record(file, record_index, record, &mut self.write_context)
    }

    pub fn append_payload(
        &mut self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
    ) -> StoreResult<PayloadRange> {
        self.ensure_write_open()?;
        self.track_payload_file_for_write(file.clone())?;
        let range = self
            .db
            .append_payload(file, bytes, &mut self.write_context)?;
        Ok(range)
    }

    fn track_record_file_for_write(&mut self, file: BinaryFileId) -> StoreResult<()> {
        self.track_relative_path_for_write(file.relative_path())
    }

    fn track_payload_file_for_write(&mut self, file: BinaryPayloadFileId) -> StoreResult<()> {
        self.track_relative_path_for_write(file.relative_path())
    }

    fn track_index_file_for_write(&mut self, index: BinaryIndexId) -> StoreResult<()> {
        self.track_relative_path_for_write(index.relative_path())
    }

    pub fn track_record_file(&mut self, file: BinaryFileId) -> StoreResult<()> {
        self.track_relative_path(file.relative_path())
    }

    pub fn track_payload_file(&mut self, file: BinaryPayloadFileId) -> StoreResult<()> {
        self.track_relative_path(file.relative_path())
    }

    pub fn track_index_file(&mut self, index: BinaryIndexId) -> StoreResult<()> {
        self.track_relative_path(index.relative_path())
    }

    pub fn track_relative_path(&mut self, path: &StorePath) -> StoreResult<()> {
        let path = store_path_for(self.db.authority_root(), path)?;
        self.track_absolute_file(path)
    }

    pub fn track_absolute_file(&mut self, path: PathBuf) -> StoreResult<()> {
        if !self.touched_files.contains(&path) {
            self.touched_files.push(path.clone());
        }
        if let Some(parent) = path.parent() {
            let parent = parent.to_path_buf();
            if !self.touched_directories.contains(&parent) {
                self.touched_directories.push(parent);
            }
        }
        Ok(())
    }

    fn track_relative_path_for_write(&mut self, path: &StorePath) -> StoreResult<()> {
        let path = store_path_for(self.db.authority_root(), path)?;
        self.track_absolute_file_for_write(path)
    }

    fn track_absolute_file_for_write(&mut self, path: PathBuf) -> StoreResult<()> {
        if !self.rollback_files.iter().any(|entry| entry.path == path) {
            let state = match self.db.metadata_len(&path)? {
                Some(len) => BinaryDbRollbackFile {
                    path: path.clone(),
                    existed: true,
                    len,
                },
                None => BinaryDbRollbackFile {
                    path: path.clone(),
                    existed: false,
                    len: 0,
                },
            };
            self.rollback_files.push(state);
        }
        self.track_absolute_file(path)
    }

    pub fn touched_files(&self) -> &[PathBuf] {
        &self.touched_files
    }

    pub fn touched_directories(&self) -> &[PathBuf] {
        &self.touched_directories
    }

    pub fn commit(&mut self) -> StoreResult<BinaryDbCommitOutcome> {
        if let Some(outcome) = &self.commit_outcome {
            return Ok(outcome.clone());
        }
        if self.finished {
            return Err(BinaryDbError::invalid_domain_data(
                "Binary DB write transaction was aborted before commit",
            ));
        }
        for path in &self.touched_files {
            self.fsync_policy.sync_file(path)?;
        }
        for path in &self.touched_directories {
            self.fsync_policy.sync_directory(path)?;
        }
        self.rollback_files.clear();
        self.rollback_records.clear();
        self.write_context.finish();
        self.finished = true;
        let outcome = BinaryDbCommitOutcome::new(self.lock.release().err());
        self.commit_outcome = Some(outcome.clone());
        Ok(outcome)
    }

    pub fn abort(&mut self) -> StoreResult<()> {
        if self.finished {
            return Ok(());
        }
        self.rollback_uncommitted_files()?;
        self.touched_files.clear();
        self.touched_directories.clear();
        self.write_context.finish();
        self.lock.release()?;
        self.finished = true;
        Ok(())
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    fn ensure_write_open(&self) -> StoreResult<()> {
        if self.finished {
            return Err(BinaryDbError::invalid_domain_data(
                "Binary DB write transaction is already finished",
            ));
        }
        self.write_context.ensure_active()
    }

    fn rollback_uncommitted_files(&mut self) -> StoreResult<()> {
        for state in self.rollback_records.iter().rev() {
            self.db.overwrite_record(
                state.file.clone(),
                state.record_index,
                &state.bytes,
                &mut self.write_context,
            )?;
        }
        self.rollback_records.clear();
        for state in self.rollback_files.iter().rev() {
            if state.existed {
                self.db.recovery_truncate_file(&state.path, state.len)?;
            } else {
                self.db.recovery_remove_file_if_exists(&state.path)?;
            }
        }
        self.rollback_files.clear();
        Ok(())
    }
}

impl<'a, B, F> Drop for BinaryDbWriteTxn<'a, B, F>
where
    B: BinaryDb + ?Sized,
    F: BinaryDbFsyncPolicy,
{
    fn drop(&mut self) {
        if !self.finished {
            let _ = self.rollback_uncommitted_files();
            self.write_context.finish();
            let _ = self.lock.release();
            self.finished = true;
        }
    }
}

impl<'a, B, F> BinaryDbWriteTxn<'a, B, F>
where
    B: BinaryDb + BinaryDbIndexAppender + ?Sized,
    F: BinaryDbFsyncPolicy,
{
    pub fn append_index_candidate(
        &mut self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
    ) -> StoreResult<()> {
        self.ensure_write_open()?;
        self.track_index_file_for_write(index.clone())?;
        self.db
            .append_index_candidate(index, key, record_index, &mut self.write_context)
    }
}
