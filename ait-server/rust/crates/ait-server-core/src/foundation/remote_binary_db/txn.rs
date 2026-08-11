use super::*;

pub const BINARY_DB_READ_MAX_WAIT: Duration = Duration::from_millis(100);
pub const BINARY_DB_READ_RETRY_INTERVAL: Duration = Duration::from_millis(5);
pub const BINARY_DB_SERVING_WRITE_MAX_WAIT: Duration = Duration::from_secs(5);
pub const BINARY_DB_SERVING_WORKFLOW_WRITE_MAX_WAIT: Duration = Duration::from_secs(20);
pub const BINARY_DB_SERVING_WRITE_RETRY_INTERVAL: Duration = Duration::from_millis(5);

pub const fn binary_db_serving_write_max_wait(command_scope: BinaryDbCommandScope) -> Duration {
    match command_scope {
        BinaryDbCommandScope::ServerWorkflow
        | BinaryDbCommandScope::ServerTaskStart
        | BinaryDbCommandScope::ServerLand => BINARY_DB_SERVING_WORKFLOW_WRITE_MAX_WAIT,
        _ => BINARY_DB_SERVING_WRITE_MAX_WAIT,
    }
}

/// Acquires the serving lock used only for publishing immutable raw repository
/// pack paths. Raw pack bytes are not Binary DB records: callers must prepare
/// and fsync a unique temporary file before taking this lock, hold it only for
/// the final-path idempotency check and atomic rename, then release it before
/// syncing the parent directory. Consequently this lock deliberately does not
/// create or recover a rollback journal.
pub fn acquire_serving_repository_pack_lock<B>(db: &B) -> StoreResult<BinaryDbCommandLockSet>
where
    B: BinaryDb + ?Sized,
{
    let command_scope = BinaryDbCommandScope::ServerRepositoryPack;
    db.acquire_queued_command_lock(
        command_scope,
        binary_db_serving_write_max_wait(command_scope),
        BINARY_DB_SERVING_WRITE_RETRY_INTERVAL,
    )
}

pub trait BinaryDbFsyncPolicy {
    fn sync_file(&self, path: &Path) -> StoreResult<()>;

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        self.sync_file(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()>;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BinaryDbNoopFsyncPolicy;

impl BinaryDbFsyncPolicy for BinaryDbNoopFsyncPolicy {
    fn sync_file(&self, _path: &Path) -> StoreResult<()> {
        Ok(())
    }

    fn sync_directory(&self, _path: &Path) -> StoreResult<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BinaryDbStdFsyncPolicy;

impl BinaryDbFsyncPolicy for BinaryDbStdFsyncPolicy {
    fn sync_file(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_file(path)
    }

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_file_data(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        ServerBinaryDbFilesystemStore.sync_directory(path)
    }
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

    fn sync_file_data(&self, path: &Path) -> StoreResult<()> {
        self.db.sync_file_data(path)
    }

    fn sync_directory(&self, path: &Path) -> StoreResult<()> {
        self.db.sync_directory(path)
    }
}

pub struct BinaryDbReadTxn<'a, B: BinaryDb + ?Sized> {
    db: &'a B,
    read_lock: StoreResult<BinaryDbReadLockSet>,
}

#[derive(Clone, Copy, Debug)]
enum BinaryDbWriteBeginMode {
    Blocking,
    Nonblocking,
    Queued {
        started: Instant,
        max_wait: Duration,
        retry_interval: Duration,
    },
}

impl<'a, B> BinaryDbReadTxn<'a, B>
where
    B: BinaryDb + ?Sized,
{
    pub fn new(db: &'a B) -> Self {
        Self::new_for_scope(db, BinaryDbReadScope::ALL)
    }

    pub fn new_for_scope(db: &'a B, read_scope: BinaryDbReadScope) -> Self {
        Self {
            db,
            read_lock: db.acquire_read_lock_for_scope(read_scope),
        }
    }

    pub fn new_queued_for_scope(
        db: &'a B,
        read_scope: BinaryDbReadScope,
        max_wait: Duration,
        retry_interval: Duration,
    ) -> Self {
        let started = Instant::now();
        let retry_interval = retry_interval.max(Duration::from_millis(1));
        let read_lock = loop {
            match db.acquire_read_lock_for_scope(read_scope) {
                Err(err) if err.is_retryable_busy() && started.elapsed() < max_wait => {
                    thread::sleep(retry_interval);
                }
                result => break result,
            }
        };
        Self { db, read_lock }
    }

    pub fn new_bounded_for_scope(db: &'a B, read_scope: BinaryDbReadScope) -> Self {
        Self::new_queued_for_scope(
            db,
            read_scope,
            BINARY_DB_READ_MAX_WAIT,
            BINARY_DB_READ_RETRY_INTERVAL,
        )
    }

    pub fn db(&self) -> &'a B {
        self.db
    }

    pub fn read_lock_paths(&self) -> StoreResult<&[PathBuf]> {
        Ok(self.read_guard()?.paths())
    }

    fn read_guard(&self) -> StoreResult<&BinaryDbReadLockSet> {
        self.read_lock.as_ref().map_err(Clone::clone)
    }

    pub fn layout_id(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.read_guard()?;
        self.db.layout_id(file)
    }

    pub fn record_count(&self, file: BinaryFileId) -> StoreResult<u32> {
        self.read_guard()?;
        self.db.record_count(file)
    }

    pub fn read_record(
        &self,
        file: BinaryFileId,
        record_index: u32,
    ) -> StoreResult<BinaryRecordBytes> {
        self.read_guard()?;
        self.db.read_record(file, record_index)
    }

    pub fn read_records(
        &self,
        file: BinaryFileId,
        first_record_index: u32,
        record_count: u32,
    ) -> StoreResult<Vec<BinaryRecordBytes>> {
        self.read_guard()?;
        self.db.read_records(file, first_record_index, record_count)
    }

    pub fn read_payload(
        &self,
        file: BinaryPayloadFileId,
        offset: u64,
        len: u32,
    ) -> StoreResult<Vec<u8>> {
        self.read_guard()?;
        self.db.read_payload(file, offset, len)
    }

    pub fn lookup_index(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
    ) -> StoreResult<Vec<u32>> {
        self.read_guard()?;
        self.db.lookup_index(index, key)
    }

    pub fn lookup_index_many(
        &self,
        index: BinaryIndexId,
        keys: &[BinaryIndexKeyRef<'_>],
    ) -> StoreResult<Vec<Vec<u32>>> {
        self.read_guard()?;
        self.db.lookup_index_many(index, keys)
    }
}

/// Result of crossing the durable server Binary DB commit point.
///
/// The rollback journal has already been removed and its directory synced
/// before this outcome is created. Lock cleanup warnings are observable but
/// cannot turn that durable commit into a retryable failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BinaryDbCommitOutcome {
    lock_cleanup_warning: Option<BinaryDbError>,
    admission_wait_duration: Duration,
    lock_hold_duration: Duration,
}

impl BinaryDbCommitOutcome {
    fn new(
        lock_cleanup_warning: Option<BinaryDbError>,
        admission_wait_duration: Duration,
        lock_hold_duration: Duration,
    ) -> Self {
        Self {
            lock_cleanup_warning,
            admission_wait_duration,
            lock_hold_duration,
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

    pub fn admission_wait_duration(&self) -> Duration {
        self.admission_wait_duration
    }

    pub fn lock_hold_duration(&self) -> Duration {
        self.lock_hold_duration
    }
}

pub struct BinaryDbWriteTxn<'a, B: BinaryDb + ?Sized, F: BinaryDbFsyncPolicy> {
    db: &'a B,
    command_scope: BinaryDbCommandScope,
    write_context: BinaryWriteContext,
    fsync_policy: F,
    touched_files: Vec<PathBuf>,
    touched_directories: Vec<PathBuf>,
    journal: BinaryDbTxnJournal,
    write_lock: BinaryDbWriteLock,
    in_process_admission: Option<BinaryDbWriterAdmissionGuard>,
    admission_wait_duration: Duration,
    commit_outcome: Option<BinaryDbCommitOutcome>,
    finished: bool,
    #[cfg(feature = "perfetto-tracing")]
    _perfetto_range: Option<crate::perfetto_trace::PerfettoRange>,
}

impl<'a, B> BinaryDbWriteTxn<'a, B, BinaryDbStoreFsyncPolicy<'a, B>>
where
    B: BinaryDb + ?Sized,
{
    pub fn begin(db: &'a B, command_scope: BinaryDbCommandScope) -> StoreResult<Self> {
        Self::begin_with_fsync_policy(db, command_scope, BinaryDbStoreFsyncPolicy::new(db))
    }

    pub fn try_begin(db: &'a B, command_scope: BinaryDbCommandScope) -> StoreResult<Self> {
        Self::try_begin_with_fsync_policy(db, command_scope, BinaryDbStoreFsyncPolicy::new(db))
    }

    pub fn begin_queued(
        db: &'a B,
        command_scope: BinaryDbCommandScope,
        max_wait: Duration,
        retry_interval: Duration,
    ) -> StoreResult<Self> {
        Self::begin_queued_with_fsync_policy(
            db,
            command_scope,
            max_wait,
            retry_interval,
            BinaryDbStoreFsyncPolicy::new(db),
        )
    }

    /// Bounded admission for normal serving mutations. Offline recovery and
    /// deterministic tests may still opt into the lower-level begin modes.
    pub fn begin_serving(db: &'a B, command_scope: BinaryDbCommandScope) -> StoreResult<Self> {
        ensure_serving_command_scope(command_scope)?;
        Self::begin_queued(
            db,
            command_scope,
            binary_db_serving_write_max_wait(command_scope),
            BINARY_DB_SERVING_WRITE_RETRY_INTERVAL,
        )
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
        Self::begin_with_mode(
            db,
            command_scope,
            fsync_policy,
            BinaryDbWriteBeginMode::Blocking,
        )
    }

    pub fn try_begin_with_fsync_policy(
        db: &'a B,
        command_scope: BinaryDbCommandScope,
        fsync_policy: F,
    ) -> StoreResult<Self> {
        Self::begin_with_mode(
            db,
            command_scope,
            fsync_policy,
            BinaryDbWriteBeginMode::Nonblocking,
        )
    }

    pub fn begin_queued_with_fsync_policy(
        db: &'a B,
        command_scope: BinaryDbCommandScope,
        max_wait: Duration,
        retry_interval: Duration,
        fsync_policy: F,
    ) -> StoreResult<Self> {
        Self::begin_with_mode(
            db,
            command_scope,
            fsync_policy,
            BinaryDbWriteBeginMode::Queued {
                started: Instant::now(),
                max_wait,
                retry_interval: retry_interval.max(Duration::from_millis(1)),
            },
        )
    }

    pub fn begin_serving_with_fsync_policy(
        db: &'a B,
        command_scope: BinaryDbCommandScope,
        fsync_policy: F,
    ) -> StoreResult<Self> {
        ensure_serving_command_scope(command_scope)?;
        Self::begin_queued_with_fsync_policy(
            db,
            command_scope,
            binary_db_serving_write_max_wait(command_scope),
            BINARY_DB_SERVING_WRITE_RETRY_INTERVAL,
            fsync_policy,
        )
    }

    fn begin_with_mode(
        db: &'a B,
        command_scope: BinaryDbCommandScope,
        fsync_policy: F,
        mode: BinaryDbWriteBeginMode,
    ) -> StoreResult<Self> {
        #[cfg(feature = "perfetto-tracing")]
        let perfetto_range = Some(crate::perfetto_trace::PerfettoRange::new(
            binary_db_write_txn_trace_name(command_scope),
        ));
        let admission_started = Instant::now();
        let in_process_admission = match mode {
            // Offline recovery and explicit nonblocking diagnostics retain the
            // process-lock-only primitive. In particular, simulated process
            // death can intentionally forget such a transaction; a serving
            // admission ticket must never survive that test-only boundary.
            BinaryDbWriteBeginMode::Blocking | BinaryDbWriteBeginMode::Nonblocking => None,
            BinaryDbWriteBeginMode::Queued {
                started, max_wait, ..
            } => db.acquire_in_process_write_admission(
                command_scope,
                Some(max_wait.saturating_sub(started.elapsed())),
            )?,
        };
        loop {
            match Self::try_begin_with_recovery_admission(db, command_scope, &fsync_policy) {
                Ok((journal, write_lock)) => {
                    let admission_wait_duration = admission_started
                        .elapsed()
                        .saturating_sub(write_lock.held_duration());
                    return Ok(Self {
                        db,
                        command_scope,
                        write_context: BinaryWriteContext::new(command_scope),
                        fsync_policy,
                        touched_files: Vec::new(),
                        touched_directories: Vec::new(),
                        journal,
                        write_lock,
                        in_process_admission,
                        admission_wait_duration,
                        commit_outcome: None,
                        finished: false,
                        #[cfg(feature = "perfetto-tracing")]
                        _perfetto_range: perfetto_range,
                    });
                }
                Err(err) if err.is_retryable_busy() => match mode {
                    BinaryDbWriteBeginMode::Blocking => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    BinaryDbWriteBeginMode::Nonblocking => return Err(err),
                    BinaryDbWriteBeginMode::Queued {
                        started,
                        max_wait,
                        retry_interval,
                    } if started.elapsed() < max_wait => {
                        thread::sleep(retry_interval);
                    }
                    BinaryDbWriteBeginMode::Queued {
                        started, max_wait, ..
                    } => {
                        return Err(BinaryDbError::retryable_busy(format!(
                            "timed out waiting for Binary DB {:?} recovery admission; waited_ms={} max_wait_ms={}: {}",
                            command_scope,
                            started.elapsed().as_millis(),
                            max_wait.as_millis(),
                            err,
                        )));
                    }
                },
                Err(err) => return Err(err),
            }
        }
    }

    fn try_begin_with_recovery_admission(
        db: &'a B,
        command_scope: BinaryDbCommandScope,
        fsync_policy: &F,
    ) -> StoreResult<(BinaryDbTxnJournal, BinaryDbWriteLock)> {
        // Do not make every waiter fight over the recovery gate while the
        // requested family is visibly owned by an active writer. A shared,
        // non-mutating probe rejects that common busy case first. Correctness
        // still comes from the exclusive union acquired below while recovery
        // admission is held; this probe is only contention control.
        let preflight = db.acquire_read_lock_for_scope(command_scope.write_scope())?;
        drop(preflight);
        let _admission = db.try_acquire_recovery_admission_lock()?;
        let recovery_scopes = BinaryDbTxnJournal::overlapping_recovery_scopes(db, command_scope);
        let write_lock =
            BinaryDbWriteLock::try_acquire_scope_union(db, command_scope, &recovery_scopes)?;
        let confirmed_scopes = BinaryDbTxnJournal::overlapping_recovery_scopes(db, command_scope);
        if confirmed_scopes
            .iter()
            .any(|scope| !recovery_scopes.contains(scope))
        {
            return Err(BinaryDbError::retryable_busy(
                "Binary DB stale journal set changed during recovery admission; retry begin",
            ));
        }
        for scope in confirmed_scopes {
            BinaryDbTxnJournal::recover_existing(db, scope, fsync_policy)?;
        }
        let journal = BinaryDbTxnJournal::create_new(db, command_scope, fsync_policy)?;
        Ok((journal, write_lock))
    }

    pub fn db(&self) -> &'a B {
        self.db
    }

    pub fn command_scope(&self) -> BinaryDbCommandScope {
        self.command_scope
    }

    #[cfg(test)]
    pub(crate) fn write_lock_paths(&self) -> &[PathBuf] {
        self.write_lock.paths()
    }

    #[cfg(test)]
    pub(crate) fn write_context(&mut self) -> &mut BinaryWriteContext {
        &mut self.write_context
    }

    pub fn fsync_policy(&self) -> &F {
        &self.fsync_policy
    }

    pub fn admission_wait_duration(&self) -> Duration {
        self.admission_wait_duration
    }

    pub fn lock_hold_duration(&self) -> Duration {
        self.write_lock.held_duration()
    }

    pub fn append_record(
        &mut self,
        file: BinaryFileId,
        record: BinaryRecordBytesRef<'_>,
    ) -> StoreResult<u32> {
        self.ensure_write_open()?;
        self.write_context.ensure_authorized_family(file.family())?;
        self.track_record_file_for_write(file.clone())?;
        self.db.append_record(file, record, &mut self.write_context)
    }

    pub fn append_records(
        &mut self,
        file: BinaryFileId,
        records: &[BinaryRecordBytes],
    ) -> StoreResult<Vec<u32>> {
        self.ensure_write_open()?;
        self.write_context.ensure_authorized_family(file.family())?;
        if records.is_empty() {
            return Ok(Vec::new());
        }
        self.track_record_file_for_write(file.clone())?;
        #[cfg(feature = "perfetto-tracing")]
        let _trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.binary_db.write.append_records");
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
        self.write_context.ensure_authorized_family(file.family())?;
        let before_image = self.db.read_record(file.clone(), record_index)?;
        self.track_record_file_for_write(file.clone())?;
        let offset = 4_u64
            .checked_add(u64::from(record_index) * u64::from(file.record_size()))
            .ok_or_else(|| BinaryDbError::invalid_domain_data("record offset overflow"))?;
        if self
            .journal
            .requires_before_image(file.relative_path(), offset)?
        {
            self.journal.track_before_image(
                self.db,
                file.relative_path(),
                offset,
                &before_image,
                &self.fsync_policy,
            )?;
        }
        self.db
            .overwrite_record(file, record_index, record, &mut self.write_context)
    }

    pub fn overwrite_records(
        &mut self,
        file: BinaryFileId,
        records: &[(u32, BinaryRecordBytes)],
    ) -> StoreResult<()> {
        self.ensure_write_open()?;
        self.write_context.ensure_authorized_family(file.family())?;
        if records.is_empty() {
            return Ok(());
        }
        let expected_len = usize::try_from(file.record_size())
            .map_err(|_| BinaryDbError::invalid_domain_data("record size does not fit usize"))?;
        if let Some((_, record)) = records
            .iter()
            .find(|(_, record)| record.len() != expected_len)
        {
            return Err(BinaryDbError::invalid_domain_data(format!(
                "record body length {} does not match configured size {} for '{}'",
                record.len(),
                expected_len,
                file.as_str()
            )));
        }
        let mut seen = BTreeSet::new();
        let count = self.db.record_count(file.clone())?;
        let mut before_images = Vec::with_capacity(records.len());
        for (record_index, _) in records {
            if !seen.insert(*record_index) {
                return Err(BinaryDbError::invalid_domain_data(format!(
                    "record batch repeats index {record_index} for '{}'",
                    file.as_str()
                )));
            }
            if *record_index >= count {
                return Err(BinaryDbError::missing_data(format!(
                    "record index {record_index} is out of range for '{}'",
                    file.as_str()
                )));
            }
            let before_image = self.db.read_record(file.clone(), *record_index)?;
            let offset = 4_u64
                .checked_add(u64::from(*record_index) * u64::from(file.record_size()))
                .ok_or_else(|| BinaryDbError::invalid_domain_data("record offset overflow"))?;
            before_images.push((offset, before_image));
        }
        self.track_record_file_for_write(file.clone())?;
        let mut protected_before_images = Vec::with_capacity(before_images.len());
        for (offset, bytes) in before_images {
            if self
                .journal
                .requires_before_image(file.relative_path(), offset)?
            {
                protected_before_images.push((offset, bytes));
            }
        }
        self.journal.track_before_images(
            self.db,
            file.relative_path(),
            &protected_before_images,
            &self.fsync_policy,
        )?;
        for (record_index, record) in records {
            self.db.overwrite_record(
                file.clone(),
                *record_index,
                record,
                &mut self.write_context,
            )?;
        }
        Ok(())
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

    pub fn read_records(
        &self,
        file: BinaryFileId,
        first_record_index: u32,
        record_count: u32,
    ) -> StoreResult<Vec<BinaryRecordBytes>> {
        self.db.read_records(file, first_record_index, record_count)
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

    pub fn lookup_index_many(
        &self,
        index: BinaryIndexId,
        keys: &[BinaryIndexKeyRef<'_>],
    ) -> StoreResult<Vec<Vec<u32>>> {
        self.db.lookup_index_many(index, keys)
    }

    pub fn append_payload(
        &mut self,
        file: BinaryPayloadFileId,
        bytes: &[u8],
    ) -> StoreResult<PayloadRange> {
        self.ensure_write_open()?;
        self.write_context.ensure_authorized_family(file.family())?;
        self.track_payload_file_for_write(file.clone())?;
        self.db.append_payload(file, bytes, &mut self.write_context)
    }

    pub(crate) fn prepare_write_set(
        &mut self,
        record_files: &[BinaryFileId],
        payload_files: &[BinaryPayloadFileId],
        index_files: &[BinaryIndexId],
    ) -> StoreResult<()> {
        self.ensure_write_open()?;
        let mut relatives = Vec::with_capacity(
            record_files
                .len()
                .saturating_add(payload_files.len())
                .saturating_add(index_files.len()),
        );
        for file in record_files {
            self.write_context.ensure_authorized_family(file.family())?;
            if !relatives.contains(file.relative_path()) {
                relatives.push(file.relative_path().clone());
            }
        }
        for file in payload_files {
            self.write_context.ensure_authorized_family(file.family())?;
            if !relatives.contains(file.relative_path()) {
                relatives.push(file.relative_path().clone());
            }
        }
        for index in index_files {
            self.write_context
                .ensure_authorized_family(index.family())?;
            if !relatives.contains(index.relative_path()) {
                relatives.push(index.relative_path().clone());
            }
        }
        self.track_relative_paths_for_write(&relatives)
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
        self.journal.commit(self.db, &self.fsync_policy)?;
        self.write_context.finish();
        self.finished = true;
        let lock_cleanup_warning = self.write_lock.release().err();
        if let Some(mut admission) = self.in_process_admission.take() {
            admission.release();
        }
        let outcome = BinaryDbCommitOutcome::new(
            lock_cleanup_warning,
            self.admission_wait_duration,
            self.write_lock.held_duration(),
        );
        self.commit_outcome = Some(outcome.clone());
        Ok(outcome)
    }

    pub fn abort(&mut self) -> StoreResult<()> {
        if self.finished {
            return Ok(());
        }
        self.touched_files.clear();
        self.touched_directories.clear();
        self.journal.abort(self.db, &self.fsync_policy)?;
        self.write_context.finish();
        self.finished = true;
        let lock_result = self.write_lock.release();
        if let Some(mut admission) = self.in_process_admission.take() {
            admission.release();
        }
        lock_result
    }

    pub fn is_finished(&self) -> bool {
        self.finished
    }

    pub fn touched_files(&self) -> &[PathBuf] {
        &self.touched_files
    }

    pub fn touched_directories(&self) -> &[PathBuf] {
        &self.touched_directories
    }

    fn ensure_write_open(&self) -> StoreResult<()> {
        if self.finished {
            return Err(BinaryDbError::invalid_domain_data(
                "Binary DB write transaction is already finished",
            ));
        }
        self.write_context.ensure_active()
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

    fn track_relative_path_for_write(&mut self, relative: &StorePath) -> StoreResult<()> {
        self.track_relative_paths_for_write(std::slice::from_ref(relative))
    }

    fn track_relative_paths_for_write(&mut self, relatives: &[StorePath]) -> StoreResult<()> {
        let existed = self
            .journal
            .track_relative_paths(self.db, relatives, &self.fsync_policy)?;
        for (relative, existed) in relatives.iter().zip(existed) {
            let path = store_path_for(self.db.authority_root(), relative)?;
            if !self.touched_files.contains(&path) {
                self.touched_files.push(path.clone());
            }
            if !existed {
                if let Some(parent) = path.parent() {
                    let parent = parent.to_path_buf();
                    if !self.touched_directories.contains(&parent) {
                        self.touched_directories.push(parent);
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(feature = "perfetto-tracing")]
const fn binary_db_write_txn_trace_name(command_scope: BinaryDbCommandScope) -> &'static str {
    match command_scope {
        BinaryDbCommandScope::General => "ait.server.binary_db.write.general",
        BinaryDbCommandScope::ServerWorkflow => "ait.server.binary_db.write.server_workflow",
        BinaryDbCommandScope::ServerPlan => "ait.server.binary_db.write.server_plan",
        BinaryDbCommandScope::ServerQueue => "ait.server.binary_db.write.server_queue",
        BinaryDbCommandScope::ServerRepositoryPack => {
            "ait.server.binary_db.write.server_repository_pack"
        }
        BinaryDbCommandScope::ServerContent => "ait.server.binary_db.write.server_content",
        BinaryDbCommandScope::ServerTaskStart => "ait.server.binary_db.write.server_task_start",
        BinaryDbCommandScope::ServerLand => "ait.server.binary_db.write.server_land",
        BinaryDbCommandScope::ServerRemoteSyncCommit => {
            "ait.server.binary_db.write.server_remote_sync_commit"
        }
    }
}

fn ensure_serving_command_scope(command_scope: BinaryDbCommandScope) -> StoreResult<()> {
    if command_scope == BinaryDbCommandScope::General {
        return Err(BinaryDbError::invalid_domain_data(
            "General is an offline whole-authority Binary DB scope and cannot be used by a serving writer",
        ));
    }
    Ok(())
}

impl<'a, B, F> Drop for BinaryDbWriteTxn<'a, B, F>
where
    B: BinaryDb + ?Sized,
    F: BinaryDbFsyncPolicy,
{
    fn drop(&mut self) {
        if self.finished {
            let _ = self.write_lock.release();
        } else {
            let _ = self.abort();
        }
    }
}

pub trait BinaryDbIndexAppender {
    fn append_index_candidate(
        &self,
        index: BinaryIndexId,
        key: BinaryIndexKeyRef<'_>,
        record_index: u32,
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()>;

    fn append_index_candidates(
        &self,
        index: BinaryIndexId,
        candidates: &[(Vec<u8>, u32)],
        write: &mut BinaryWriteContext,
    ) -> StoreResult<()> {
        for (key, record_index) in candidates {
            self.append_index_candidate(index.clone(), key, *record_index, write)?;
        }
        Ok(())
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
        self.write_context
            .ensure_authorized_family(index.family())?;
        self.track_index_file_for_write(index.clone())?;
        self.db
            .append_index_candidate(index, key, record_index, &mut self.write_context)
    }

    pub fn append_index_candidates(
        &mut self,
        index: BinaryIndexId,
        candidates: &[(Vec<u8>, u32)],
    ) -> StoreResult<()> {
        self.ensure_write_open()?;
        self.write_context
            .ensure_authorized_family(index.family())?;
        if candidates.is_empty() {
            return Ok(());
        }
        self.track_index_file_for_write(index.clone())?;
        #[cfg(feature = "perfetto-tracing")]
        let _trace = crate::perfetto_trace::PerfettoRange::new(
            "ait.server.binary_db.write.append_index_candidates",
        );
        self.db
            .append_index_candidates(index, candidates, &mut self.write_context)
    }
}
