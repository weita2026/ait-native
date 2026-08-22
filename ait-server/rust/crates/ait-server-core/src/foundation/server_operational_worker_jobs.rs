use crate::foundation::operational_binary_v0::{
    ServerOperationalBinaryV0Codec, ServerWorkerJobRecord, ServerWorkerReadyIndexRecord,
    ServerWorkerStateIndexRecord, OPERATIONAL_BIN_HEADER_SIZE, OPERATIONAL_V0_LAYOUT_ID,
    SERVER_WORKER_JOB_RECORD_SIZE, SERVER_WORKER_READY_INDEX_RECORD_SIZE,
    SERVER_WORKER_STATE_INDEX_RECORD_SIZE, WORKER_JOB_ERROR_LEASE_EXPIRED, WORKER_JOB_ERROR_NONE,
    WORKER_JOB_ERROR_RETRYABLE_EXECUTION, WORKER_JOB_ERROR_TERMINAL_EXECUTION,
    WORKER_JOB_KIND_PATCHSET_CI, WORKER_JOB_META_TOMBSTONED, WORKER_JOB_OUTCOME_ATTACHED,
    WORKER_JOB_OUTCOME_COMPLETED, WORKER_JOB_OUTCOME_FAILED, WORKER_JOB_OUTCOME_NONE,
    WORKER_JOB_OUTCOME_SKIPPED, WORKER_JOB_OUTCOME_SUPERSEDED, WORKER_JOB_STATE_FAILED,
    WORKER_JOB_STATE_QUEUED, WORKER_JOB_STATE_RUNNING, WORKER_JOB_STATE_SUCCEEDED,
};
use crate::foundation::remote_binary_db::{
    BinaryDbError, BoxedServerBinaryDbProcessLockGuard, ServerBinaryDbByteStore,
    ServerBinaryDbDurabilityStore, ServerBinaryDbFilesystemStore, ServerBinaryDbLockMode,
    ServerBinaryDbLockStore, ServerBinaryDbLockWait, StoreResult,
};
use crate::foundation::server_operational_repository_registry::{
    canonical_repository_directory_name, parse_repository_directory_name,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub const WORKER_QUEUE_LOCK_FILE_NAME: &str = "worker-queue.lock";

const WORKER_JOB_FILE_NAME: &str = "worker_job.bin";
const WORKER_READY_INDEX_FILE_NAME: &str = "worker_ready.idx";
const WORKER_STATE_INDEX_FILE_NAME: &str = "worker_state.idx";
const WORKER_JOB_REWRITE_FILE_NAME: &str = ".worker_job.bin.rewrite";
const WORKER_JOB_UPDATE_JOURNAL_FILE_NAME: &str = ".worker_job.bin.update-journal";
const WORKER_JOB_UPDATE_STAGING_FILE_NAME: &str = ".worker_job.bin.update-staging";
const WORKER_READY_REBUILD_FILE_NAME: &str = ".worker_ready.idx.rebuild";
const WORKER_STATE_REBUILD_FILE_NAME: &str = ".worker_state.idx.rebuild";
const WORKER_JOB_UPDATE_JOURNAL_MAGIC: &[u8; 8] = b"AITWJUP1";
const WORKER_JOB_UPDATE_JOURNAL_BODY_SIZE: usize =
    8 + 4 + 2 * SERVER_WORKER_JOB_RECORD_SIZE as usize;
const WORKER_JOB_UPDATE_JOURNAL_SIZE: usize = WORKER_JOB_UPDATE_JOURNAL_BODY_SIZE + 32;

const FORBIDDEN_WORKER_PAYLOAD_FILES: [&str; 7] = [
    "worker_job_payload.bin",
    "worker_job_input_payload.bin",
    "worker_job_request_payload.bin",
    "worker_job_result_payload.bin",
    "worker_job_error_payload.bin",
    "worker_job_lease_owner_payload.bin",
    "job.bin",
];

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct WorkerJobKey {
    pub repository_index: u32,
    pub worker_job_index: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerJobCreateSpec {
    pub job_kind: u8,
    pub max_attempts: u16,
    pub patchset_index_plus1: u32,
    pub snapshot_index_plus1: u32,
    pub available_at_s: u64,
    pub created_at_s: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerJobEnqueueDisposition {
    Queued,
    Attached,
    Skipped,
    Superseded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerJobEntry {
    pub key: WorkerJobKey,
    pub record: ServerWorkerJobRecord,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WorkerJobRetirementBlockers {
    pub queued: u32,
    pub running: u32,
}

impl WorkerJobRetirementBlockers {
    pub const fn is_drained(self) -> bool {
        self.queued == 0 && self.running == 0
    }
}

pub trait WorkerJobDomainAuthority: Send + Sync {
    fn validate_patchset_index(&self, patchset_index: u32) -> StoreResult<()>;

    fn validate_snapshot_index(&self, snapshot_index: u32) -> StoreResult<()>;
}

#[derive(Clone)]
pub struct ServerOperationalWorkerJobStore {
    repository_index: u32,
    authority_root: PathBuf,
    domain: Arc<dyn WorkerJobDomainAuthority>,
    files: ServerBinaryDbFilesystemStore,
}

pub(crate) struct WorkerJobQueueWrite<'a> {
    store: &'a ServerOperationalWorkerJobStore,
    entries: BTreeMap<u32, WorkerJobEntry>,
}

impl WorkerJobQueueWrite<'_> {
    pub(crate) fn entry(&mut self, worker_job_index: u32) -> StoreResult<WorkerJobEntry> {
        if let Some(entry) = self.entries.get(&worker_job_index) {
            return Ok(*entry);
        }
        let entry = self.store.read_job_at_unlocked(worker_job_index)?;
        self.store.validate_domain_references(entry.record)?;
        self.entries.insert(worker_job_index, entry);
        Ok(entry)
    }

    pub(crate) fn replace(
        &mut self,
        worker_job_index: u32,
        next: ServerWorkerJobRecord,
    ) -> StoreResult<WorkerJobEntry> {
        let current = self.entry(worker_job_index)?;
        if next.job_kind != current.record.job_kind
            || next.patchset_index_plus1 != current.record.patchset_index_plus1
            || next.snapshot_index_plus1 != current.record.snapshot_index_plus1
            || next.max_attempts != current.record.max_attempts
            || next.created_at_s != current.record.created_at_s
            || next.available_at_s == 0
            || next.updated_at_s < current.record.updated_at_s
        {
            return Err(invalid(
                "Worker Job replacement changed immutable fields or moved time backwards",
            ));
        }
        let ready_index_changed =
            ready_index_projection(current.record) != ready_index_projection(next);
        let state_index_changed =
            state_index_projection(current.record) != state_index_projection(next);
        self.store.validate_domain_references(next)?;
        self.store.persist_job_replacement(
            current,
            next,
            ready_index_changed,
            state_index_changed,
        )?;
        let entry = WorkerJobEntry {
            key: current.key,
            record: next,
        };
        self.entries.insert(worker_job_index, entry);
        Ok(entry)
    }
}

impl std::fmt::Debug for ServerOperationalWorkerJobStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerOperationalWorkerJobStore")
            .field("repository_index", &self.repository_index)
            .field("authority_root", &self.authority_root)
            .finish_non_exhaustive()
    }
}

impl ServerOperationalWorkerJobStore {
    pub fn new(
        repository_index: u32,
        authority_root: impl Into<PathBuf>,
        domain: Arc<dyn WorkerJobDomainAuthority>,
    ) -> StoreResult<Self> {
        let authority_root = absolute_path(authority_root.into())?;
        let basename = authority_root
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| invalid("Repository authority root has no UTF-8 basename"))?;
        let parsed = parse_repository_directory_name(basename)?;
        if parsed != repository_index
            || canonical_repository_directory_name(repository_index) != basename
        {
            return Err(invalid(format!(
                "Repository authority root basename {basename:?} does not match index {repository_index}"
            )));
        }
        require_real_directory(&authority_root)?;
        Ok(Self {
            repository_index,
            authority_root,
            domain,
            files: ServerBinaryDbFilesystemStore,
        })
    }

    pub fn repository_index(&self) -> u32 {
        self.repository_index
    }

    pub fn authority_root(&self) -> &Path {
        &self.authority_root
    }

    pub fn initialize(&self) -> StoreResult<()> {
        self.validate_root_paths()?;
        let mut lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.recover_exclusive_state()?;
        let job_path = self.worker_job_path();
        if job_path.exists() {
            let authority = self.read_authority()?;
            self.rebuild_indexes(&authority.entries)?;
            lock.clear_contents_and_flush()?;
            return Ok(());
        }
        self.write_new_header_file(&job_path)?;
        self.rebuild_indexes(&[])?;
        lock.clear_contents_and_flush()?;
        Ok(())
    }

    pub fn enqueue(
        &self,
        spec: WorkerJobCreateSpec,
        disposition: WorkerJobEnqueueDisposition,
    ) -> StoreResult<WorkerJobEntry> {
        self.enqueue_with_relationship_commit(spec, disposition, |_| Ok(()))
    }

    pub(crate) fn enqueue_with_relationship_commit(
        &self,
        spec: WorkerJobCreateSpec,
        disposition: WorkerJobEnqueueDisposition,
        relationship_commit: impl FnOnce(WorkerJobEntry) -> StoreResult<()>,
    ) -> StoreResult<WorkerJobEntry> {
        self.validate_root_paths()?;
        let mut lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.recover_exclusive_state()?;
        let worker_job_index = self.worker_job_count_unlocked()?;
        if worker_job_index == u32::MAX {
            return Err(invalid(
                "Repository-local Worker Job authority exhausted v0 index capacity",
            ));
        }

        let (state_kind, outcome_kind) = match disposition {
            WorkerJobEnqueueDisposition::Queued => {
                (WORKER_JOB_STATE_QUEUED, WORKER_JOB_OUTCOME_NONE)
            }
            WorkerJobEnqueueDisposition::Attached => {
                (WORKER_JOB_STATE_SUCCEEDED, WORKER_JOB_OUTCOME_ATTACHED)
            }
            WorkerJobEnqueueDisposition::Skipped => {
                (WORKER_JOB_STATE_SUCCEEDED, WORKER_JOB_OUTCOME_SKIPPED)
            }
            WorkerJobEnqueueDisposition::Superseded => {
                (WORKER_JOB_STATE_SUCCEEDED, WORKER_JOB_OUTCOME_SUPERSEDED)
            }
        };
        let record = ServerWorkerJobRecord {
            job_meta: 0,
            job_kind: spec.job_kind,
            state_kind,
            outcome_kind,
            attempt_count: 0,
            max_attempts: spec.max_attempts,
            error_kind: WORKER_JOB_ERROR_NONE,
            reserved0: 0,
            patchset_index_plus1: spec.patchset_index_plus1,
            snapshot_index_plus1: spec.snapshot_index_plus1,
            available_at_s: spec.available_at_s,
            locked_at_s: 0,
            created_at_s: spec.created_at_s,
            updated_at_s: spec.created_at_s,
        };
        self.validate_domain_references(record)?;
        let entry = WorkerJobEntry {
            key: WorkerJobKey {
                repository_index: self.repository_index,
                worker_job_index,
            },
            record,
        };
        let ready_index_bytes =
            self.updated_ready_index_bytes(None, Some(entry), worker_job_index)?;
        let state_index_bytes =
            self.updated_state_index_bytes(None, Some(entry), worker_job_index)?;
        let raw = ServerOperationalBinaryV0Codec::encode_worker_job(record)?;
        let expected_offset = OPERATIONAL_BIN_HEADER_SIZE
            .checked_add(
                u64::from(worker_job_index)
                    .checked_mul(u64::from(SERVER_WORKER_JOB_RECORD_SIZE))
                    .ok_or_else(|| corrupt("Worker Job append offset overflow"))?,
            )
            .ok_or_else(|| corrupt("Worker Job append offset overflow"))?;
        let actual_offset = self.append_and_sync(&self.worker_job_path(), &raw)?;
        if actual_offset != expected_offset {
            return Err(corrupt(format!(
                "Worker Job append offset changed: expected {expected_offset}, got {actual_offset}"
            )));
        }

        let relationship_result = relationship_commit(entry);
        let index_result = self.replace_prepared_indexes(ready_index_bytes, state_index_bytes);
        if let Err(error) = index_result {
            let repair_result = self
                .read_authority_without_domain_validation()
                .and_then(|authority| self.rebuild_indexes(&authority.entries));
            return Err(combine_recovery_error(
                "persist appended Worker Job indexes",
                error,
                repair_result,
            ));
        }
        lock.clear_contents_and_flush()?;
        relationship_result?;
        Ok(entry)
    }

    /// Complete administrative inventory. Latency-sensitive callers must use
    /// `get`, `list_recent`, or `ready_candidates` instead.
    pub fn list(&self) -> StoreResult<Vec<WorkerJobEntry>> {
        self.validate_root_paths()?;
        let _lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Shared)?;
        self.require_clean_read_state()?;
        let authority = self.read_authority()?;
        Ok(authority.entries)
    }

    pub fn list_recent(
        &self,
        state_kind: Option<u8>,
        limit: usize,
    ) -> StoreResult<Vec<WorkerJobEntry>> {
        self.validate_root_paths()?;
        let _lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Shared)?;
        self.require_clean_read_state()?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut entries = match state_kind {
            Some(state_kind) => self.state_index_entries_unlocked(state_kind, limit, true)?,
            None => self.recent_job_entries_unlocked(limit)?,
        };
        for entry in &entries {
            self.validate_domain_references(entry.record)?;
        }
        entries.truncate(limit);
        Ok(entries)
    }

    pub fn patchset_ci_locators_for_activation_repair(&self) -> StoreResult<BTreeMap<u32, u32>> {
        self.validate_root_paths()?;
        let _lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Shared)?;
        self.require_clean_read_state()?;
        let authority = self.read_authority_without_domain_validation()?;
        let mut locators = BTreeMap::new();
        for entry in authority.entries {
            if entry.record.is_tombstoned() || entry.record.job_kind != WORKER_JOB_KIND_PATCHSET_CI
            {
                continue;
            }
            let patchset_index = entry
                .record
                .patchset_index_plus1
                .checked_sub(1)
                .ok_or_else(|| corrupt("patchset.ci Worker Job lacks its Patchset reference"))?;
            let worker_job_index_plus1 = entry
                .key
                .worker_job_index
                .checked_add(1)
                .ok_or_else(|| corrupt("Worker Job plus-one index overflow"))?;
            locators.insert(patchset_index, worker_job_index_plus1);
        }
        Ok(locators)
    }

    pub fn get(&self, worker_job_index: u32) -> StoreResult<WorkerJobEntry> {
        self.validate_root_paths()?;
        let _lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Shared)?;
        self.require_clean_read_state()?;
        let entry = self.read_job_at_unlocked(worker_job_index)?;
        self.validate_domain_references(entry.record)?;
        Ok(entry)
    }

    pub fn begin_attempt(&self, worker_job_index: u32, now_s: u64) -> StoreResult<WorkerJobEntry> {
        self.replace_job(worker_job_index, |current| {
            if current.state_kind != WORKER_JOB_STATE_QUEUED || current.is_tombstoned() {
                return Err(invalid(
                    "only a live queued Worker Job can begin an attempt",
                ));
            }
            if current.available_at_s > now_s {
                return Err(invalid("Worker Job is not available yet"));
            }
            if current.attempt_count >= current.max_attempts {
                return Err(invalid("Worker Job attempt budget is exhausted"));
            }
            let attempt_count = current
                .attempt_count
                .checked_add(1)
                .ok_or_else(|| corrupt("Worker Job attempt count overflow"))?;
            Ok(ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_RUNNING,
                attempt_count,
                error_kind: WORKER_JOB_ERROR_NONE,
                locked_at_s: now_s,
                updated_at_s: now_s,
                ..current
            })
        })
    }

    pub fn requeue(
        &self,
        worker_job_index: u32,
        expected_attempt_count: u16,
        error_kind: u16,
        available_at_s: u64,
        updated_at_s: u64,
    ) -> StoreResult<WorkerJobEntry> {
        if !matches!(
            error_kind,
            WORKER_JOB_ERROR_RETRYABLE_EXECUTION | WORKER_JOB_ERROR_LEASE_EXPIRED
        ) {
            return Err(invalid("requeued Worker Job error kind is not retryable"));
        }
        self.replace_job(worker_job_index, |current| {
            require_running_attempt(current, expected_attempt_count)?;
            if current.attempt_count >= current.max_attempts {
                return Err(invalid(
                    "exhausted Worker Job cannot return to queued state",
                ));
            }
            Ok(ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_QUEUED,
                error_kind,
                available_at_s,
                locked_at_s: 0,
                updated_at_s,
                ..current
            })
        })
    }

    pub fn complete(
        &self,
        worker_job_index: u32,
        expected_attempt_count: u16,
        outcome_kind: u8,
        updated_at_s: u64,
    ) -> StoreResult<WorkerJobEntry> {
        if !matches!(
            outcome_kind,
            WORKER_JOB_OUTCOME_COMPLETED
                | WORKER_JOB_OUTCOME_SKIPPED
                | WORKER_JOB_OUTCOME_ATTACHED
                | WORKER_JOB_OUTCOME_SUPERSEDED
        ) {
            return Err(invalid("successful Worker Job outcome kind is invalid"));
        }
        self.replace_job(worker_job_index, |current| {
            require_running_attempt(current, expected_attempt_count)?;
            Ok(ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_SUCCEEDED,
                outcome_kind,
                error_kind: WORKER_JOB_ERROR_NONE,
                locked_at_s: 0,
                updated_at_s,
                ..current
            })
        })
    }

    pub fn fail(
        &self,
        worker_job_index: u32,
        expected_attempt_count: u16,
        error_kind: u16,
        updated_at_s: u64,
    ) -> StoreResult<WorkerJobEntry> {
        if !matches!(
            error_kind,
            WORKER_JOB_ERROR_TERMINAL_EXECUTION | WORKER_JOB_ERROR_LEASE_EXPIRED
        ) {
            return Err(invalid("failed Worker Job error kind is invalid"));
        }
        self.replace_job(worker_job_index, |current| {
            require_running_attempt(current, expected_attempt_count)?;
            Ok(ServerWorkerJobRecord {
                state_kind: WORKER_JOB_STATE_FAILED,
                outcome_kind: WORKER_JOB_OUTCOME_FAILED,
                error_kind,
                locked_at_s: 0,
                updated_at_s,
                ..current
            })
        })
    }

    pub fn find_equivalent(&self, spec: WorkerJobCreateSpec) -> StoreResult<Vec<WorkerJobEntry>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|entry| {
                !entry.record.is_tombstoned()
                    && entry.record.job_kind == spec.job_kind
                    && entry.record.patchset_index_plus1 == spec.patchset_index_plus1
                    && entry.record.snapshot_index_plus1 == spec.snapshot_index_plus1
            })
            .collect())
    }

    pub fn ready_candidates(&self, now_s: u64, limit: usize) -> StoreResult<Vec<WorkerJobEntry>> {
        self.validate_root_paths()?;
        let _lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Shared)?;
        self.require_clean_read_state()?;
        let candidates = self.ready_index_entries_unlocked(now_s, limit)?;
        for entry in &candidates {
            self.validate_domain_references(entry.record)?;
        }
        Ok(candidates)
    }

    pub(crate) fn lease_reconciliation_entries(&self) -> StoreResult<Vec<WorkerJobEntry>> {
        self.validate_root_paths()?;
        let _lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Shared)?;
        self.require_clean_read_state()?;
        Ok(self.read_authority_without_domain_validation()?.entries)
    }

    pub fn validate(&self) -> StoreResult<Vec<WorkerJobEntry>> {
        self.validate_root_paths()?;
        let _lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Shared)?;
        self.require_clean_read_state()?;
        let authority = self.read_authority()?;
        self.validate_indexes_if_present(&authority.entries)?;
        Ok(authority.entries)
    }

    pub fn recover(&self) -> StoreResult<Vec<WorkerJobEntry>> {
        self.validate_root_paths()?;
        let mut lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.recover_exclusive_state()?;
        let authority = self.read_authority()?;
        self.repair_indexes(&authority.entries)?;
        lock.clear_contents_and_flush()?;
        Ok(authority.entries)
    }

    pub fn retirement_blockers(&self) -> StoreResult<WorkerJobRetirementBlockers> {
        self.validate_root_paths()?;
        let _lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Shared)?;
        self.require_clean_read_state()?;
        let queued =
            self.state_index_entries_unlocked(WORKER_JOB_STATE_QUEUED, usize::MAX, false)?;
        let running =
            self.state_index_entries_unlocked(WORKER_JOB_STATE_RUNNING, usize::MAX, false)?;
        Ok(WorkerJobRetirementBlockers {
            queued: u32::try_from(queued.len())
                .map_err(|_| corrupt("queued Worker Job count exceeds u32"))?,
            running: u32::try_from(running.len())
                .map_err(|_| corrupt("running Worker Job count exceeds u32"))?,
        })
    }

    /// Makes imported or purged Jobs permanently non-runnable while retaining
    /// their fixed records as historical authority.
    pub fn tombstone_all(&self, updated_at_s: u64) -> StoreResult<u32> {
        self.validate_root_paths()?;
        let mut lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.recover_exclusive_state()?;
        let mut authority = self.read_authority_without_domain_validation()?;
        let mut changed = 0_u32;
        for entry in &mut authority.entries {
            if entry.record.is_tombstoned() {
                continue;
            }
            entry.record.job_meta |= WORKER_JOB_META_TOMBSTONED;
            entry.record.updated_at_s = entry.record.updated_at_s.max(updated_at_s);
            changed = changed
                .checked_add(1)
                .ok_or_else(|| corrupt("tombstoned Worker Job count exceeds u32"))?;
        }
        if changed != 0 {
            let mut bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
            for entry in &authority.entries {
                bytes.extend_from_slice(&ServerOperationalBinaryV0Codec::encode_worker_job(
                    entry.record,
                )?);
            }
            self.atomic_replace(
                &self.worker_job_path(),
                &self.authority_root.join(WORKER_JOB_REWRITE_FILE_NAME),
                &bytes,
            )?;
        }
        self.rebuild_indexes(&authority.entries)?;
        lock.clear_contents_and_flush()?;
        Ok(changed)
    }

    fn replace_job(
        &self,
        worker_job_index: u32,
        replacement: impl FnOnce(ServerWorkerJobRecord) -> StoreResult<ServerWorkerJobRecord>,
    ) -> StoreResult<WorkerJobEntry> {
        self.with_exclusive_queue(|queue| {
            let current = queue.entry(worker_job_index)?;
            queue.replace(worker_job_index, replacement(current.record)?)
        })
    }

    pub(crate) fn with_exclusive_queue<T>(
        &self,
        operation: impl FnOnce(&mut WorkerJobQueueWrite<'_>) -> StoreResult<T>,
    ) -> StoreResult<T> {
        self.validate_root_paths()?;
        let mut lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.recover_exclusive_state()?;
        let mut queue = WorkerJobQueueWrite {
            store: self,
            entries: BTreeMap::new(),
        };
        let result = operation(&mut queue)?;
        lock.clear_contents_and_flush()?;
        Ok(result)
    }

    fn validate_domain_references(&self, record: ServerWorkerJobRecord) -> StoreResult<()> {
        if record.is_tombstoned() {
            return Ok(());
        }
        if let Some(index) = record.patchset_index_plus1.checked_sub(1) {
            self.domain.validate_patchset_index(index)?;
        }
        if let Some(index) = record.snapshot_index_plus1.checked_sub(1) {
            self.domain.validate_snapshot_index(index)?;
        }
        Ok(())
    }

    fn worker_job_count_unlocked(&self) -> StoreResult<u32> {
        self.fixed_record_count_unlocked(
            &self.worker_job_path(),
            SERVER_WORKER_JOB_RECORD_SIZE,
            WORKER_JOB_FILE_NAME,
        )
    }

    fn fixed_record_count_unlocked(
        &self,
        path: &Path,
        record_size: u32,
        label: &str,
    ) -> StoreResult<u32> {
        reject_symlink_or_hardlink_if_present(path)?;
        let len = self
            .files
            .metadata_len(path)?
            .ok_or_else(|| BinaryDbError::missing_data(format!("{label} is missing")))?;
        if len < OPERATIONAL_BIN_HEADER_SIZE {
            return Err(corrupt(format!("{label} is missing its layout header")));
        }
        let payload_len = len - OPERATIONAL_BIN_HEADER_SIZE;
        if !payload_len.is_multiple_of(u64::from(record_size)) {
            return Err(corrupt(format!(
                "{label} has an incomplete trailing {record_size}-byte record"
            )));
        }
        let header = self
            .files
            .read_range(path, 0, OPERATIONAL_BIN_HEADER_SIZE as u32)?;
        validate_layout_header(&header, label)?;
        u32::try_from(payload_len / u64::from(record_size))
            .map_err(|_| corrupt(format!("{label} record count exceeds u32")))
    }

    fn read_job_at_unlocked(&self, worker_job_index: u32) -> StoreResult<WorkerJobEntry> {
        let worker_job_count = self.worker_job_count_unlocked()?;
        self.read_job_at_known_count_unlocked(worker_job_index, worker_job_count)
    }

    fn read_job_at_known_count_unlocked(
        &self,
        worker_job_index: u32,
        worker_job_count: u32,
    ) -> StoreResult<WorkerJobEntry> {
        if worker_job_index >= worker_job_count {
            return Err(invalid(format!(
                "Worker Job index {worker_job_index} is out of range"
            )));
        }
        let offset = fixed_record_offset(
            worker_job_index,
            SERVER_WORKER_JOB_RECORD_SIZE,
            WORKER_JOB_FILE_NAME,
        )?;
        let raw = self.files.read_range(
            &self.worker_job_path(),
            offset,
            SERVER_WORKER_JOB_RECORD_SIZE,
        )?;
        Ok(WorkerJobEntry {
            key: WorkerJobKey {
                repository_index: self.repository_index,
                worker_job_index,
            },
            record: ServerOperationalBinaryV0Codec::decode_worker_job(&raw)?,
        })
    }

    fn recent_job_entries_unlocked(&self, limit: usize) -> StoreResult<Vec<WorkerJobEntry>> {
        let worker_job_count = self.worker_job_count_unlocked()?;
        let take = usize::try_from(worker_job_count)
            .unwrap_or(usize::MAX)
            .min(limit);
        let mut entries = Vec::with_capacity(take);
        for offset_from_end in 0..take {
            let offset_from_end = u32::try_from(offset_from_end)
                .map_err(|_| corrupt("Worker Job recent-list offset exceeds u32"))?;
            let worker_job_index = worker_job_count
                .checked_sub(offset_from_end + 1)
                .ok_or_else(|| corrupt("Worker Job recent-list offset underflow"))?;
            entries
                .push(self.read_job_at_known_count_unlocked(worker_job_index, worker_job_count)?);
        }
        Ok(entries)
    }

    fn read_ready_index_row_unlocked(
        &self,
        row_index: u32,
        row_count: u32,
    ) -> StoreResult<ServerWorkerReadyIndexRecord> {
        if row_index >= row_count {
            return Err(corrupt("Worker ready index row is out of range"));
        }
        let offset = fixed_record_offset(
            row_index,
            SERVER_WORKER_READY_INDEX_RECORD_SIZE,
            WORKER_READY_INDEX_FILE_NAME,
        )?;
        let raw = self.files.read_range(
            &self.worker_ready_index_path(),
            offset,
            SERVER_WORKER_READY_INDEX_RECORD_SIZE,
        )?;
        ServerOperationalBinaryV0Codec::decode_worker_ready_index(&raw)
    }

    fn read_state_index_row_unlocked(
        &self,
        row_index: u32,
        row_count: u32,
    ) -> StoreResult<ServerWorkerStateIndexRecord> {
        if row_index >= row_count {
            return Err(corrupt("Worker state index row is out of range"));
        }
        let offset = fixed_record_offset(
            row_index,
            SERVER_WORKER_STATE_INDEX_RECORD_SIZE,
            WORKER_STATE_INDEX_FILE_NAME,
        )?;
        let raw = self.files.read_range(
            &self.worker_state_index_path(),
            offset,
            SERVER_WORKER_STATE_INDEX_RECORD_SIZE,
        )?;
        ServerOperationalBinaryV0Codec::decode_worker_state_index(&raw)
    }

    fn ready_index_entries_unlocked(
        &self,
        now_s: u64,
        limit: usize,
    ) -> StoreResult<Vec<WorkerJobEntry>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let row_count = self.fixed_record_count_unlocked(
            &self.worker_ready_index_path(),
            SERVER_WORKER_READY_INDEX_RECORD_SIZE,
            WORKER_READY_INDEX_FILE_NAME,
        )?;
        let worker_job_count = self.worker_job_count_unlocked()?;
        let mut previous_key = None;
        let mut seen_job_indexes = BTreeSet::new();
        let mut entries = Vec::with_capacity(limit.min(row_count as usize));
        for row_index in 0..row_count {
            let row = self.read_ready_index_row_unlocked(row_index, row_count)?;
            let key = (row.available_at_s, row.worker_job_index_plus1);
            if previous_key.is_some_and(|previous| previous >= key) {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} is not strictly sorted"
                )));
            }
            previous_key = Some(key);
            let worker_job_index = row
                .worker_job_index_plus1
                .checked_sub(1)
                .ok_or_else(|| corrupt("Worker ready index contains a zero Job locator"))?;
            if worker_job_index >= worker_job_count {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} contains an out-of-range Job locator"
                )));
            }
            if !seen_job_indexes.insert(worker_job_index) {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} contains a duplicate Job locator"
                )));
            }
            if row.available_at_s > now_s {
                break;
            }
            let entry =
                self.read_job_at_known_count_unlocked(worker_job_index, worker_job_count)?;
            if entry.record.is_tombstoned()
                || entry.record.state_kind != WORKER_JOB_STATE_QUEUED
                || entry.record.available_at_s != row.available_at_s
            {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} is stale for Worker Job {worker_job_index}"
                )));
            }
            if entry.record.attempt_count < entry.record.max_attempts {
                entries.push(entry);
                if entries.len() == limit {
                    break;
                }
            }
        }
        Ok(entries)
    }

    fn state_index_entries_unlocked(
        &self,
        state_kind: u8,
        limit: usize,
        newest_first: bool,
    ) -> StoreResult<Vec<WorkerJobEntry>> {
        require_worker_job_state_kind(state_kind)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let row_count = self.fixed_record_count_unlocked(
            &self.worker_state_index_path(),
            SERVER_WORKER_STATE_INDEX_RECORD_SIZE,
            WORKER_STATE_INDEX_FILE_NAME,
        )?;
        let worker_job_count = self.worker_job_count_unlocked()?;
        let start = self.state_index_bound_unlocked(row_count, state_kind, false)?;
        let end = self.state_index_bound_unlocked(row_count, state_kind, true)?;
        if start > end {
            return Err(corrupt(format!(
                "{WORKER_STATE_INDEX_FILE_NAME} has invalid state boundaries"
            )));
        }
        if start > 0 {
            let previous = self.read_state_index_row_unlocked(start - 1, row_count)?;
            if previous.state_kind >= state_kind {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} is not sorted by state"
                )));
            }
        }
        if end < row_count {
            let next = self.read_state_index_row_unlocked(end, row_count)?;
            if next.state_kind <= state_kind {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} is not sorted by state"
                )));
            }
        }

        let available = usize::try_from(end - start)
            .map_err(|_| corrupt("Worker state index projection exceeds usize"))?;
        let take = available.min(limit);
        let selected_start = end
            .checked_sub(
                u32::try_from(take)
                    .map_err(|_| corrupt("Worker state index selection exceeds u32"))?,
            )
            .ok_or_else(|| corrupt("Worker state index selection underflow"))?;
        let mut rows = Vec::with_capacity(take);
        let mut previous_job_index_plus1 = None;
        let mut seen_job_indexes = BTreeSet::new();
        for row_index in selected_start..end {
            let row = self.read_state_index_row_unlocked(row_index, row_count)?;
            if row.state_kind != state_kind
                || previous_job_index_plus1
                    .is_some_and(|previous| previous >= row.worker_job_index_plus1)
            {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} is not strictly sorted"
                )));
            }
            previous_job_index_plus1 = Some(row.worker_job_index_plus1);
            let worker_job_index = row
                .worker_job_index_plus1
                .checked_sub(1)
                .ok_or_else(|| corrupt("Worker state index contains a zero Job locator"))?;
            if worker_job_index >= worker_job_count {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} contains an out-of-range Job locator"
                )));
            }
            if !seen_job_indexes.insert(worker_job_index) {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} contains a duplicate Job locator"
                )));
            }
            let entry =
                self.read_job_at_known_count_unlocked(worker_job_index, worker_job_count)?;
            if entry.record.is_tombstoned() || entry.record.state_kind != state_kind {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} is stale for Worker Job {worker_job_index}"
                )));
            }
            rows.push(entry);
        }
        if newest_first {
            rows.reverse();
        }
        Ok(rows)
    }

    fn state_index_bound_unlocked(
        &self,
        row_count: u32,
        state_kind: u8,
        upper: bool,
    ) -> StoreResult<u32> {
        let mut low = 0_u32;
        let mut high = row_count;
        while low < high {
            let middle = low + (high - low) / 2;
            let row = self.read_state_index_row_unlocked(middle, row_count)?;
            let advances = if upper {
                row.state_kind <= state_kind
            } else {
                row.state_kind < state_kind
            };
            if advances {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        Ok(low)
    }

    fn read_authority(&self) -> StoreResult<WorkerJobAuthority> {
        self.read_authority_with_domain_validation(true)
    }

    fn read_authority_without_domain_validation(&self) -> StoreResult<WorkerJobAuthority> {
        self.read_authority_with_domain_validation(false)
    }

    fn read_authority_with_domain_validation(
        &self,
        validate_domain: bool,
    ) -> StoreResult<WorkerJobAuthority> {
        let worker_job_bytes = read_regular_file(&self.worker_job_path())?;
        validate_header_and_alignment(
            &worker_job_bytes,
            SERVER_WORKER_JOB_RECORD_SIZE,
            WORKER_JOB_FILE_NAME,
        )?;
        let mut entries = Vec::new();
        for (index, raw) in worker_job_bytes[4..]
            .chunks_exact(SERVER_WORKER_JOB_RECORD_SIZE as usize)
            .enumerate()
        {
            let record = ServerOperationalBinaryV0Codec::decode_worker_job(raw)?;
            if validate_domain {
                self.validate_domain_references(record)?;
            }
            entries.push(WorkerJobEntry {
                key: WorkerJobKey {
                    repository_index: self.repository_index,
                    worker_job_index: u32::try_from(index)
                        .map_err(|_| corrupt("Worker Job index exceeds u32"))?,
                },
                record,
            });
        }
        Ok(WorkerJobAuthority { entries })
    }

    fn read_ready_index_rows_unlocked(
        &self,
        worker_job_count: u32,
    ) -> StoreResult<Vec<ServerWorkerReadyIndexRecord>> {
        let bytes = read_regular_file(&self.worker_ready_index_path())?;
        validate_header_and_alignment(
            &bytes,
            SERVER_WORKER_READY_INDEX_RECORD_SIZE,
            WORKER_READY_INDEX_FILE_NAME,
        )?;
        let mut rows = Vec::new();
        let mut previous_key = None;
        let mut seen_job_indexes = BTreeSet::new();
        for raw in bytes[4..].chunks_exact(SERVER_WORKER_READY_INDEX_RECORD_SIZE as usize) {
            let row = ServerOperationalBinaryV0Codec::decode_worker_ready_index(raw)?;
            let key = (row.available_at_s, row.worker_job_index_plus1);
            if previous_key.is_some_and(|previous| previous >= key) {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} is not strictly sorted"
                )));
            }
            previous_key = Some(key);
            let worker_job_index = row
                .worker_job_index_plus1
                .checked_sub(1)
                .ok_or_else(|| corrupt("Worker ready index contains a zero Job locator"))?;
            if worker_job_index >= worker_job_count {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} contains an out-of-range Job locator"
                )));
            }
            if !seen_job_indexes.insert(worker_job_index) {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} contains a duplicate Job locator"
                )));
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn read_state_index_rows_unlocked(
        &self,
        worker_job_count: u32,
    ) -> StoreResult<Vec<ServerWorkerStateIndexRecord>> {
        let bytes = read_regular_file(&self.worker_state_index_path())?;
        validate_header_and_alignment(
            &bytes,
            SERVER_WORKER_STATE_INDEX_RECORD_SIZE,
            WORKER_STATE_INDEX_FILE_NAME,
        )?;
        let mut rows = Vec::new();
        let mut previous_key = None;
        let mut seen_job_indexes = BTreeSet::new();
        for raw in bytes[4..].chunks_exact(SERVER_WORKER_STATE_INDEX_RECORD_SIZE as usize) {
            let row = ServerOperationalBinaryV0Codec::decode_worker_state_index(raw)?;
            let key = (row.state_kind, row.worker_job_index_plus1);
            if previous_key.is_some_and(|previous| previous >= key) {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} is not strictly sorted"
                )));
            }
            previous_key = Some(key);
            let worker_job_index = row
                .worker_job_index_plus1
                .checked_sub(1)
                .ok_or_else(|| corrupt("Worker state index contains a zero Job locator"))?;
            if worker_job_index >= worker_job_count {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} contains an out-of-range Job locator"
                )));
            }
            if !seen_job_indexes.insert(worker_job_index) {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} contains a duplicate Job locator"
                )));
            }
            rows.push(row);
        }
        Ok(rows)
    }

    fn updated_ready_index_bytes(
        &self,
        current: Option<WorkerJobEntry>,
        next: Option<WorkerJobEntry>,
        existing_worker_job_count: u32,
    ) -> StoreResult<Option<Vec<u8>>> {
        let current_row = current.and_then(ready_index_row);
        let next_row = next.and_then(ready_index_row);
        if current_row == next_row {
            return Ok(None);
        }
        let worker_job_index_plus1 = current
            .or(next)
            .ok_or_else(|| invalid("Worker ready index update has no Job"))?
            .key
            .worker_job_index
            .checked_add(1)
            .ok_or_else(|| corrupt("Worker Job plus-one index overflow"))?;
        let mut rows = self.read_ready_index_rows_unlocked(existing_worker_job_count)?;
        let matching = rows
            .iter()
            .position(|row| row.worker_job_index_plus1 == worker_job_index_plus1);
        match (current_row, matching) {
            (Some(expected), Some(position)) if rows[position] == expected => {
                rows.remove(position);
            }
            (Some(_), _) => {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} is stale for Worker Job {}",
                    worker_job_index_plus1 - 1
                )))
            }
            (None, Some(_)) => {
                return Err(corrupt(format!(
                    "{WORKER_READY_INDEX_FILE_NAME} has an unexpected Worker Job projection"
                )))
            }
            (None, None) => {}
        }
        if let Some(row) = next_row {
            rows.push(row);
        }
        rows.sort_by_key(|row| (row.available_at_s, row.worker_job_index_plus1));
        encode_ready_index_rows(&rows).map(Some)
    }

    fn updated_state_index_bytes(
        &self,
        current: Option<WorkerJobEntry>,
        next: Option<WorkerJobEntry>,
        existing_worker_job_count: u32,
    ) -> StoreResult<Option<Vec<u8>>> {
        let current_row = current.and_then(state_index_row);
        let next_row = next.and_then(state_index_row);
        if current_row == next_row {
            return Ok(None);
        }
        let worker_job_index_plus1 = current
            .or(next)
            .ok_or_else(|| invalid("Worker state index update has no Job"))?
            .key
            .worker_job_index
            .checked_add(1)
            .ok_or_else(|| corrupt("Worker Job plus-one index overflow"))?;
        let mut rows = self.read_state_index_rows_unlocked(existing_worker_job_count)?;
        let matching = rows
            .iter()
            .position(|row| row.worker_job_index_plus1 == worker_job_index_plus1);
        match (current_row, matching) {
            (Some(expected), Some(position)) if rows[position] == expected => {
                rows.remove(position);
            }
            (Some(_), _) => {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} is stale for Worker Job {}",
                    worker_job_index_plus1 - 1
                )))
            }
            (None, Some(_)) => {
                return Err(corrupt(format!(
                    "{WORKER_STATE_INDEX_FILE_NAME} has an unexpected Worker Job projection"
                )))
            }
            (None, None) => {}
        }
        if let Some(row) = next_row {
            rows.push(row);
        }
        rows.sort_by_key(|row| (row.state_kind, row.worker_job_index_plus1));
        encode_state_index_rows(&rows).map(Some)
    }

    fn replace_prepared_indexes(
        &self,
        ready_index_bytes: Option<Vec<u8>>,
        state_index_bytes: Option<Vec<u8>>,
    ) -> StoreResult<()> {
        if let Some(bytes) = ready_index_bytes {
            self.atomic_replace(
                &self.worker_ready_index_path(),
                &self.authority_root.join(WORKER_READY_REBUILD_FILE_NAME),
                &bytes,
            )?;
        }
        if let Some(bytes) = state_index_bytes {
            self.atomic_replace(
                &self.worker_state_index_path(),
                &self.authority_root.join(WORKER_STATE_REBUILD_FILE_NAME),
                &bytes,
            )?;
        }
        Ok(())
    }

    fn persist_job_replacement(
        &self,
        current: WorkerJobEntry,
        next: ServerWorkerJobRecord,
        ready_index_changed: bool,
        state_index_changed: bool,
    ) -> StoreResult<()> {
        let worker_job_count = self.worker_job_count_unlocked()?;
        if current.key.worker_job_index >= worker_job_count {
            return Err(corrupt("Worker Job replacement index is out of range"));
        }
        let next_entry = WorkerJobEntry {
            key: current.key,
            record: next,
        };
        let ready_index_bytes = if ready_index_changed {
            self.updated_ready_index_bytes(Some(current), Some(next_entry), worker_job_count)?
        } else {
            None
        };
        let state_index_bytes = if state_index_changed {
            self.updated_state_index_bytes(Some(current), Some(next_entry), worker_job_count)?
        } else {
            None
        };
        let before_raw = ServerOperationalBinaryV0Codec::encode_worker_job(current.record)?;
        let after_raw = ServerOperationalBinaryV0Codec::encode_worker_job(next)?;
        let journal = encode_worker_job_update_journal(
            current.key.worker_job_index,
            &before_raw,
            &after_raw,
        )?;
        let journal_path = self.worker_job_update_journal_path();
        self.atomic_replace(
            &journal_path,
            &self.worker_job_update_staging_path(),
            &journal,
        )?;

        let persist_result = (|| {
            let offset = fixed_record_offset(
                current.key.worker_job_index,
                SERVER_WORKER_JOB_RECORD_SIZE,
                WORKER_JOB_FILE_NAME,
            )?;
            self.files
                .overwrite_range(&self.worker_job_path(), offset, &after_raw)?;
            self.files.sync_file(&self.worker_job_path())?;
            self.replace_prepared_indexes(ready_index_bytes, state_index_bytes)?;
            self.files.remove_file_if_exists(&journal_path)?;
            self.files.sync_directory(&self.authority_root)
        })();
        if let Err(error) = persist_result {
            let recovery_result = (|| {
                let offset = fixed_record_offset(
                    current.key.worker_job_index,
                    SERVER_WORKER_JOB_RECORD_SIZE,
                    WORKER_JOB_FILE_NAME,
                )?;
                self.files
                    .overwrite_range(&self.worker_job_path(), offset, &before_raw)?;
                self.files.sync_file(&self.worker_job_path())?;
                let authority = self.read_authority_without_domain_validation()?;
                self.rebuild_indexes(&authority.entries)?;
                self.files.remove_file_if_exists(&journal_path)?;
                self.files.sync_directory(&self.authority_root)
            })();
            return Err(combine_recovery_error(
                "persist Worker Job replacement",
                error,
                recovery_result,
            ));
        }
        Ok(())
    }

    fn expected_ready_index(&self, entries: &[WorkerJobEntry]) -> StoreResult<Vec<u8>> {
        let mut rows = entries
            .iter()
            .filter(|entry| {
                !entry.record.is_tombstoned() && entry.record.state_kind == WORKER_JOB_STATE_QUEUED
            })
            .map(|entry| {
                Ok(ServerWorkerReadyIndexRecord {
                    available_at_s: entry.record.available_at_s,
                    worker_job_index_plus1: entry
                        .key
                        .worker_job_index
                        .checked_add(1)
                        .ok_or_else(|| corrupt("Worker Job plus-one index overflow"))?,
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;
        rows.sort_by_key(|row| (row.available_at_s, row.worker_job_index_plus1));
        let mut bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
        for row in rows {
            bytes.extend_from_slice(&ServerOperationalBinaryV0Codec::encode_worker_ready_index(
                row,
            )?);
        }
        Ok(bytes)
    }

    fn expected_state_index(&self, entries: &[WorkerJobEntry]) -> StoreResult<Vec<u8>> {
        let mut rows = entries
            .iter()
            .filter(|entry| !entry.record.is_tombstoned())
            .map(|entry| {
                Ok(ServerWorkerStateIndexRecord {
                    state_kind: entry.record.state_kind,
                    reserved0: 0,
                    reserved1: 0,
                    worker_job_index_plus1: entry
                        .key
                        .worker_job_index
                        .checked_add(1)
                        .ok_or_else(|| corrupt("Worker Job plus-one index overflow"))?,
                })
            })
            .collect::<StoreResult<Vec<_>>>()?;
        rows.sort_by_key(|row| (row.state_kind, row.worker_job_index_plus1));
        let mut bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
        for row in rows {
            bytes.extend_from_slice(&ServerOperationalBinaryV0Codec::encode_worker_state_index(
                row,
            )?);
        }
        Ok(bytes)
    }

    fn rebuild_indexes(&self, entries: &[WorkerJobEntry]) -> StoreResult<()> {
        self.rebuild_ready_index(entries)?;
        self.rebuild_state_index(entries)
    }

    fn repair_indexes(&self, entries: &[WorkerJobEntry]) -> StoreResult<()> {
        self.repair_index_if_needed(
            &self.worker_ready_index_path(),
            &self.authority_root.join(WORKER_READY_REBUILD_FILE_NAME),
            &self.expected_ready_index(entries)?,
        )?;
        self.repair_index_if_needed(
            &self.worker_state_index_path(),
            &self.authority_root.join(WORKER_STATE_REBUILD_FILE_NAME),
            &self.expected_state_index(entries)?,
        )
    }

    fn repair_index_if_needed(
        &self,
        target: &Path,
        temporary: &Path,
        expected: &[u8],
    ) -> StoreResult<()> {
        reject_symlink_or_hardlink_if_present(target)?;
        if target.exists() && read_regular_file(target)? == expected {
            return Ok(());
        }
        self.atomic_replace(target, temporary, expected)
    }

    fn rebuild_ready_index(&self, entries: &[WorkerJobEntry]) -> StoreResult<()> {
        self.atomic_replace(
            &self.worker_ready_index_path(),
            &self.authority_root.join(WORKER_READY_REBUILD_FILE_NAME),
            &self.expected_ready_index(entries)?,
        )
    }

    fn rebuild_state_index(&self, entries: &[WorkerJobEntry]) -> StoreResult<()> {
        self.atomic_replace(
            &self.worker_state_index_path(),
            &self.authority_root.join(WORKER_STATE_REBUILD_FILE_NAME),
            &self.expected_state_index(entries)?,
        )
    }

    fn validate_indexes_if_present(&self, entries: &[WorkerJobEntry]) -> StoreResult<()> {
        let expected = [
            (
                self.worker_ready_index_path(),
                self.expected_ready_index(entries)?,
                SERVER_WORKER_READY_INDEX_RECORD_SIZE,
                WORKER_READY_INDEX_FILE_NAME,
            ),
            (
                self.worker_state_index_path(),
                self.expected_state_index(entries)?,
                SERVER_WORKER_STATE_INDEX_RECORD_SIZE,
                WORKER_STATE_INDEX_FILE_NAME,
            ),
        ];
        for (path, expected_bytes, record_size, label) in expected {
            if !path.exists() {
                continue;
            }
            let actual = read_regular_file(&path)?;
            validate_header_and_alignment(&actual, record_size, label)?;
            if actual != expected_bytes {
                return Err(corrupt(format!("{label} is stale")));
            }
        }
        Ok(())
    }

    fn validate_root_paths(&self) -> StoreResult<()> {
        require_real_directory(&self.authority_root)?;
        let basename = self
            .authority_root
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| invalid("Repository authority root has no UTF-8 basename"))?;
        if parse_repository_directory_name(basename)? != self.repository_index {
            return Err(invalid("Repository authority root index changed"));
        }
        for name in FORBIDDEN_WORKER_PAYLOAD_FILES {
            if self.authority_root.join(name).exists() {
                return Err(invalid(format!(
                    "Repository authority contains forbidden Worker Job path {name}"
                )));
            }
        }
        Ok(())
    }

    fn acquire_queue_lock(
        &self,
        mode: ServerBinaryDbLockMode,
    ) -> StoreResult<BoxedServerBinaryDbProcessLockGuard> {
        let path = self.authority_root.join(WORKER_QUEUE_LOCK_FILE_NAME);
        reject_symlink_or_hardlink_if_present(&path)?;
        let mut lock = self
            .files
            .acquire_process_lock(&path, mode, ServerBinaryDbLockWait::Blocking)?
            .ok_or_else(|| {
                BinaryDbError::retryable_busy(format!(
                    "Worker Job queue lock is busy at {}",
                    path.display()
                ))
            })?;
        if matches!(mode, ServerBinaryDbLockMode::Exclusive) {
            lock.replace_contents_and_flush(
                format!(
                    "repository_index={}\nroot={}\n",
                    self.repository_index,
                    self.authority_root.display()
                )
                .as_bytes(),
            )?;
        }
        Ok(lock)
    }

    fn worker_job_path(&self) -> PathBuf {
        self.authority_root.join(WORKER_JOB_FILE_NAME)
    }

    fn worker_ready_index_path(&self) -> PathBuf {
        self.authority_root.join(WORKER_READY_INDEX_FILE_NAME)
    }

    fn worker_state_index_path(&self) -> PathBuf {
        self.authority_root.join(WORKER_STATE_INDEX_FILE_NAME)
    }

    fn worker_job_update_journal_path(&self) -> PathBuf {
        self.authority_root
            .join(WORKER_JOB_UPDATE_JOURNAL_FILE_NAME)
    }

    fn worker_job_update_staging_path(&self) -> PathBuf {
        self.authority_root
            .join(WORKER_JOB_UPDATE_STAGING_FILE_NAME)
    }

    fn require_clean_read_state(&self) -> StoreResult<()> {
        for name in [
            WORKER_JOB_REWRITE_FILE_NAME,
            WORKER_JOB_UPDATE_JOURNAL_FILE_NAME,
            WORKER_JOB_UPDATE_STAGING_FILE_NAME,
            WORKER_READY_REBUILD_FILE_NAME,
            WORKER_STATE_REBUILD_FILE_NAME,
        ] {
            let path = self.authority_root.join(name);
            if path.exists() {
                reject_symlink_or_hardlink_if_present(&path)?;
                return Err(corrupt(format!(
                    "Worker Job authority has interrupted update state at {}",
                    path.display()
                )));
            }
        }
        Ok(())
    }

    fn recover_exclusive_state(&self) -> StoreResult<()> {
        self.remove_rebuild_temps()?;
        let journal_path = self.worker_job_update_journal_path();
        if !journal_path.exists() {
            return Ok(());
        }
        let journal = decode_worker_job_update_journal(&read_regular_file(&journal_path)?)?;
        let worker_job_count = self.worker_job_count_unlocked()?;
        if journal.worker_job_index >= worker_job_count {
            return Err(corrupt(
                "Worker Job update journal contains an out-of-range Job locator",
            ));
        }
        let offset = fixed_record_offset(
            journal.worker_job_index,
            SERVER_WORKER_JOB_RECORD_SIZE,
            WORKER_JOB_FILE_NAME,
        )?;
        self.files
            .overwrite_range(&self.worker_job_path(), offset, &journal.before_raw)?;
        self.files.sync_file(&self.worker_job_path())?;
        let authority = self.read_authority_without_domain_validation()?;
        self.rebuild_indexes(&authority.entries)?;
        self.files.remove_file_if_exists(&journal_path)?;
        self.files.sync_directory(&self.authority_root)
    }

    fn write_new_header_file(&self, path: &Path) -> StoreResult<()> {
        reject_symlink_or_hardlink_if_present(path)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| {
                BinaryDbError::io(
                    format!("create Worker Job Binary DB file {}", path.display()),
                    error,
                )
            })?;
        file.write_all(&OPERATIONAL_V0_LAYOUT_ID.to_le_bytes())
            .map_err(|error| {
                BinaryDbError::io(
                    format!("write Worker Job Binary DB header {}", path.display()),
                    error,
                )
            })?;
        file.sync_all().map_err(|error| {
            BinaryDbError::io(
                format!("sync Worker Job Binary DB file {}", path.display()),
                error,
            )
        })?;
        self.files.sync_directory(&self.authority_root)
    }

    fn append_and_sync(&self, path: &Path, bytes: &[u8]) -> StoreResult<u64> {
        reject_symlink_or_hardlink_if_present(path)?;
        let offset = self.files.append_bytes(path, bytes)?;
        self.files.sync_file(path)?;
        Ok(offset)
    }

    fn atomic_replace(&self, target: &Path, temporary: &Path, bytes: &[u8]) -> StoreResult<()> {
        reject_symlink_or_hardlink_if_present(target)?;
        if temporary.exists() {
            reject_symlink_or_hardlink_if_present(temporary)?;
            fs::remove_file(temporary).map_err(|error| {
                BinaryDbError::io(
                    format!("remove stale Worker Job rebuild {}", temporary.display()),
                    error,
                )
            })?;
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(temporary)
            .map_err(|error| {
                BinaryDbError::io(
                    format!("create Worker Job rebuild {}", temporary.display()),
                    error,
                )
            })?;
        file.write_all(bytes).map_err(|error| {
            BinaryDbError::io(
                format!("write Worker Job rebuild {}", temporary.display()),
                error,
            )
        })?;
        file.sync_all().map_err(|error| {
            BinaryDbError::io(
                format!("sync Worker Job rebuild {}", temporary.display()),
                error,
            )
        })?;
        fs::rename(temporary, target).map_err(|error| {
            BinaryDbError::io(
                format!(
                    "activate Worker Job rebuild {} as {}",
                    temporary.display(),
                    target.display()
                ),
                error,
            )
        })?;
        self.files.sync_directory(&self.authority_root)
    }

    fn remove_rebuild_temps(&self) -> StoreResult<()> {
        let mut removed = false;
        for name in [
            WORKER_JOB_REWRITE_FILE_NAME,
            WORKER_JOB_UPDATE_STAGING_FILE_NAME,
            WORKER_READY_REBUILD_FILE_NAME,
            WORKER_STATE_REBUILD_FILE_NAME,
        ] {
            let path = self.authority_root.join(name);
            if path.exists() {
                reject_symlink_or_hardlink_if_present(&path)?;
                fs::remove_file(&path).map_err(|error| {
                    BinaryDbError::io(
                        format!("remove interrupted Worker Job rebuild {}", path.display()),
                        error,
                    )
                })?;
                removed = true;
            }
        }
        if removed {
            self.files.sync_directory(&self.authority_root)?;
        }
        Ok(())
    }
}

struct WorkerJobAuthority {
    entries: Vec<WorkerJobEntry>,
}

struct WorkerJobUpdateJournal {
    worker_job_index: u32,
    before_raw: [u8; SERVER_WORKER_JOB_RECORD_SIZE as usize],
}

fn ready_index_row(entry: WorkerJobEntry) -> Option<ServerWorkerReadyIndexRecord> {
    ready_index_projection(entry.record).map(|available_at_s| ServerWorkerReadyIndexRecord {
        available_at_s,
        worker_job_index_plus1: entry
            .key
            .worker_job_index
            .checked_add(1)
            .expect("stored Worker Job index always has a plus-one locator"),
    })
}

fn state_index_row(entry: WorkerJobEntry) -> Option<ServerWorkerStateIndexRecord> {
    state_index_projection(entry.record).map(|state_kind| ServerWorkerStateIndexRecord {
        state_kind,
        reserved0: 0,
        reserved1: 0,
        worker_job_index_plus1: entry
            .key
            .worker_job_index
            .checked_add(1)
            .expect("stored Worker Job index always has a plus-one locator"),
    })
}

fn encode_ready_index_rows(rows: &[ServerWorkerReadyIndexRecord]) -> StoreResult<Vec<u8>> {
    let mut bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
    for row in rows {
        bytes.extend_from_slice(&ServerOperationalBinaryV0Codec::encode_worker_ready_index(
            *row,
        )?);
    }
    Ok(bytes)
}

fn encode_state_index_rows(rows: &[ServerWorkerStateIndexRecord]) -> StoreResult<Vec<u8>> {
    let mut bytes = OPERATIONAL_V0_LAYOUT_ID.to_le_bytes().to_vec();
    for row in rows {
        bytes.extend_from_slice(&ServerOperationalBinaryV0Codec::encode_worker_state_index(
            *row,
        )?);
    }
    Ok(bytes)
}

fn fixed_record_offset(index: u32, record_size: u32, label: &str) -> StoreResult<u64> {
    OPERATIONAL_BIN_HEADER_SIZE
        .checked_add(
            u64::from(index)
                .checked_mul(u64::from(record_size))
                .ok_or_else(|| corrupt(format!("{label} record offset overflow")))?,
        )
        .ok_or_else(|| corrupt(format!("{label} record offset overflow")))
}

fn encode_worker_job_update_journal(
    worker_job_index: u32,
    before_raw: &[u8],
    after_raw: &[u8],
) -> StoreResult<Vec<u8>> {
    if before_raw.len() != SERVER_WORKER_JOB_RECORD_SIZE as usize
        || after_raw.len() != SERVER_WORKER_JOB_RECORD_SIZE as usize
    {
        return Err(corrupt("Worker Job update journal record width changed"));
    }
    let before = ServerOperationalBinaryV0Codec::decode_worker_job(before_raw)?;
    let after = ServerOperationalBinaryV0Codec::decode_worker_job(after_raw)?;
    validate_journal_record_pair(before, after)?;
    let mut bytes = Vec::with_capacity(WORKER_JOB_UPDATE_JOURNAL_SIZE);
    bytes.extend_from_slice(WORKER_JOB_UPDATE_JOURNAL_MAGIC);
    bytes.extend_from_slice(&worker_job_index.to_le_bytes());
    bytes.extend_from_slice(before_raw);
    bytes.extend_from_slice(after_raw);
    debug_assert_eq!(bytes.len(), WORKER_JOB_UPDATE_JOURNAL_BODY_SIZE);
    bytes.extend_from_slice(&Sha256::digest(&bytes));
    Ok(bytes)
}

fn decode_worker_job_update_journal(raw: &[u8]) -> StoreResult<WorkerJobUpdateJournal> {
    if raw.len() != WORKER_JOB_UPDATE_JOURNAL_SIZE
        || raw.get(..8) != Some(WORKER_JOB_UPDATE_JOURNAL_MAGIC.as_slice())
    {
        return Err(corrupt(
            "Worker Job update journal header or width is invalid",
        ));
    }
    let (body, recorded_digest) = raw.split_at(WORKER_JOB_UPDATE_JOURNAL_BODY_SIZE);
    if Sha256::digest(body).as_slice() != recorded_digest {
        return Err(corrupt("Worker Job update journal digest does not match"));
    }
    let worker_job_index = u32::from_le_bytes(
        body[8..12]
            .try_into()
            .expect("Worker Job journal index width"),
    );
    let before_start = 12;
    let after_start = before_start + SERVER_WORKER_JOB_RECORD_SIZE as usize;
    let mut before_raw = [0_u8; SERVER_WORKER_JOB_RECORD_SIZE as usize];
    before_raw.copy_from_slice(&body[before_start..after_start]);
    let after_raw = &body[after_start..WORKER_JOB_UPDATE_JOURNAL_BODY_SIZE];
    let before = ServerOperationalBinaryV0Codec::decode_worker_job(&before_raw)?;
    let after = ServerOperationalBinaryV0Codec::decode_worker_job(after_raw)?;
    validate_journal_record_pair(before, after)?;
    Ok(WorkerJobUpdateJournal {
        worker_job_index,
        before_raw,
    })
}

fn validate_journal_record_pair(
    before: ServerWorkerJobRecord,
    after: ServerWorkerJobRecord,
) -> StoreResult<()> {
    if after.job_kind != before.job_kind
        || after.patchset_index_plus1 != before.patchset_index_plus1
        || after.snapshot_index_plus1 != before.snapshot_index_plus1
        || after.max_attempts != before.max_attempts
        || after.created_at_s != before.created_at_s
        || after.updated_at_s < before.updated_at_s
    {
        return Err(corrupt(
            "Worker Job update journal changes immutable fields or moves time backwards",
        ));
    }
    Ok(())
}

fn combine_recovery_error(
    context: &str,
    original: BinaryDbError,
    recovery: StoreResult<()>,
) -> BinaryDbError {
    match recovery {
        Ok(()) => original,
        Err(recovery) => BinaryDbError::other(format!(
            "{context} failed: {original}; recovery also failed: {recovery}"
        )),
    }
}

fn ready_index_projection(record: ServerWorkerJobRecord) -> Option<u64> {
    (!record.is_tombstoned() && record.state_kind == WORKER_JOB_STATE_QUEUED)
        .then_some(record.available_at_s)
}

fn state_index_projection(record: ServerWorkerJobRecord) -> Option<u8> {
    (!record.is_tombstoned()).then_some(record.state_kind)
}

fn require_worker_job_state_kind(state_kind: u8) -> StoreResult<()> {
    if matches!(
        state_kind,
        WORKER_JOB_STATE_QUEUED
            | WORKER_JOB_STATE_RUNNING
            | WORKER_JOB_STATE_SUCCEEDED
            | WORKER_JOB_STATE_FAILED
    ) {
        Ok(())
    } else {
        Err(invalid("Worker Job state kind is reserved"))
    }
}

pub fn merge_ready_candidates(
    candidates: impl IntoIterator<Item = WorkerJobEntry>,
    limit: usize,
) -> Vec<WorkerJobEntry> {
    let mut candidates = candidates.into_iter().collect::<Vec<_>>();
    candidates.sort_by_key(|entry| {
        (
            entry.record.available_at_s,
            entry.key.repository_index,
            entry.key.worker_job_index,
        )
    });
    candidates.truncate(limit);
    candidates
}

fn require_running_attempt(
    current: ServerWorkerJobRecord,
    expected_attempt_count: u16,
) -> StoreResult<()> {
    if current.is_tombstoned()
        || current.state_kind != WORKER_JOB_STATE_RUNNING
        || current.attempt_count != expected_attempt_count
    {
        return Err(invalid(
            "Worker Job is not the expected live running attempt",
        ));
    }
    Ok(())
}

fn absolute_path(path: PathBuf) -> StoreResult<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| BinaryDbError::io("resolve current directory", error))
    }
}

fn require_real_directory(path: &Path) -> StoreResult<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| BinaryDbError::io(format!("inspect {}", path.display()), error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid(format!(
            "{} is not a real directory",
            path.display()
        )));
    }
    Ok(())
}

fn reject_symlink_or_hardlink_if_present(path: &Path) -> StoreResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(BinaryDbError::io(
                format!("inspect {}", path.display()),
                error,
            ))
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid(format!(
            "{} is not a real regular file",
            path.display()
        )));
    }
    #[cfg(unix)]
    if metadata.nlink() != 1 {
        return Err(invalid(format!(
            "{} is shared through a hard link",
            path.display()
        )));
    }
    Ok(())
}

fn read_regular_file(path: &Path) -> StoreResult<Vec<u8>> {
    reject_symlink_or_hardlink_if_present(path)?;
    fs::read(path).map_err(|error| BinaryDbError::io(format!("read {}", path.display()), error))
}

fn validate_header_and_alignment(bytes: &[u8], record_size: u32, label: &str) -> StoreResult<()> {
    let header = bytes
        .get(..4)
        .ok_or_else(|| corrupt(format!("{label} is missing its layout header")))?;
    validate_layout_header(header, label)?;
    if !(bytes.len() - 4).is_multiple_of(record_size as usize) {
        return Err(corrupt(format!(
            "{label} has an incomplete trailing {record_size}-byte record"
        )));
    }
    Ok(())
}

fn validate_layout_header(header: &[u8], label: &str) -> StoreResult<()> {
    let header: [u8; 4] = header
        .try_into()
        .map_err(|_| corrupt(format!("{label} is missing its layout header")))?;
    let layout_id = u32::from_le_bytes(header);
    if layout_id != OPERATIONAL_V0_LAYOUT_ID {
        return Err(BinaryDbError::layout_mismatch(format!(
            "{label} layout is {layout_id}, expected {OPERATIONAL_V0_LAYOUT_ID}"
        )));
    }
    Ok(())
}

fn invalid(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::invalid_domain_data(message)
}

fn corrupt(message: impl Into<String>) -> BinaryDbError {
    BinaryDbError::corruption(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::operational_binary_v0::{
        WORKER_JOB_KIND_CONTENT_GC, WORKER_JOB_KIND_PATCHSET_CI, WORKER_JOB_KIND_REPO_CI,
    };
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Default)]
    struct TestDomainAuthority {
        patchsets: BTreeSet<u32>,
        snapshots: BTreeSet<u32>,
    }

    impl WorkerJobDomainAuthority for TestDomainAuthority {
        fn validate_patchset_index(&self, patchset_index: u32) -> StoreResult<()> {
            if self.patchsets.contains(&patchset_index) {
                Ok(())
            } else {
                Err(invalid(format!(
                    "unknown test Patchset index {patchset_index}"
                )))
            }
        }

        fn validate_snapshot_index(&self, snapshot_index: u32) -> StoreResult<()> {
            if self.snapshots.contains(&snapshot_index) {
                Ok(())
            } else {
                Err(invalid(format!(
                    "unknown test Snapshot index {snapshot_index}"
                )))
            }
        }
    }

    #[derive(Debug)]
    struct CountingDomainAuthority {
        validation_count: Arc<AtomicUsize>,
    }

    impl WorkerJobDomainAuthority for CountingDomainAuthority {
        fn validate_patchset_index(&self, _patchset_index: u32) -> StoreResult<()> {
            self.validation_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn validate_snapshot_index(&self, _snapshot_index: u32) -> StoreResult<()> {
            self.validation_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    fn fixture(name: &str, repository_index: u32) -> (PathBuf, ServerOperationalWorkerJobStore) {
        fixture_with_domain(
            name,
            repository_index,
            Arc::new(TestDomainAuthority {
                patchsets: [2, 4].into_iter().collect(),
                snapshots: [7, 9].into_iter().collect(),
            }),
        )
    }

    fn fixture_with_domain(
        name: &str,
        repository_index: u32,
        domain: Arc<dyn WorkerJobDomainAuthority>,
    ) -> (PathBuf, ServerOperationalWorkerJobStore) {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ait-server-worker-jobs-{name}-{}-{nonce}",
            std::process::id()
        ));
        let authority_root = root.join(repository_index.to_string());
        fs::create_dir_all(&authority_root).unwrap();
        let store =
            ServerOperationalWorkerJobStore::new(repository_index, authority_root, domain).unwrap();
        (root, store)
    }

    fn cleanup(root: &Path) {
        let _ = fs::remove_dir_all(root);
    }

    fn patchset_ci_spec(available_at_s: u64) -> WorkerJobCreateSpec {
        WorkerJobCreateSpec {
            job_kind: WORKER_JOB_KIND_PATCHSET_CI,
            max_attempts: 3,
            patchset_index_plus1: 3,
            snapshot_index_plus1: 0,
            available_at_s,
            created_at_s: 100,
        }
    }

    #[test]
    fn fresh_authority_is_header_only_with_empty_rebuildable_indexes() {
        let (root, store) = fixture("fresh", 1);
        store.initialize().unwrap();

        for path in [
            store.worker_job_path(),
            store.worker_ready_index_path(),
            store.worker_state_index_path(),
        ] {
            assert_eq!(fs::read(path).unwrap(), 1_u32.to_le_bytes());
        }
        assert!(store.validate().unwrap().is_empty());
        cleanup(&root);
    }

    #[test]
    fn append_allocates_physical_keys_and_validates_fixed_domain_references() {
        let (root, store) = fixture("append", 6);
        store.initialize().unwrap();

        let first = store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();
        let second = store
            .enqueue(
                WorkerJobCreateSpec {
                    job_kind: WORKER_JOB_KIND_REPO_CI,
                    max_attempts: 1,
                    patchset_index_plus1: 0,
                    snapshot_index_plus1: 8,
                    available_at_s: 101,
                    created_at_s: 101,
                },
                WorkerJobEnqueueDisposition::Queued,
            )
            .unwrap();
        assert_eq!(
            (first.key, second.key),
            (
                WorkerJobKey {
                    repository_index: 6,
                    worker_job_index: 0,
                },
                WorkerJobKey {
                    repository_index: 6,
                    worker_job_index: 1,
                },
            )
        );
        assert_eq!(
            fs::metadata(store.worker_job_path()).unwrap().len(),
            OPERATIONAL_BIN_HEADER_SIZE + 2 * u64::from(SERVER_WORKER_JOB_RECORD_SIZE)
        );

        let before = fs::read(store.worker_job_path()).unwrap();
        let mut wrong_shape = patchset_ci_spec(102);
        wrong_shape.patchset_index_plus1 = 0;
        assert!(store
            .enqueue(wrong_shape, WorkerJobEnqueueDisposition::Queued)
            .is_err());
        let mut missing_target = patchset_ci_spec(102);
        missing_target.patchset_index_plus1 = 10;
        assert!(store
            .enqueue(missing_target, WorkerJobEnqueueDisposition::Queued)
            .is_err());
        assert_eq!(fs::read(store.worker_job_path()).unwrap(), before);
        cleanup(&root);
    }

    #[test]
    fn state_transitions_are_attempt_bound_and_rebuild_local_indexes() {
        let (root, store) = fixture("transitions", 2);
        store.initialize().unwrap();
        store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();

        let running = store.begin_attempt(0, 101).unwrap();
        assert_eq!(running.record.state_kind, WORKER_JOB_STATE_RUNNING);
        assert_eq!(running.record.attempt_count, 1);
        assert_eq!(
            fs::read(store.worker_ready_index_path()).unwrap(),
            1_u32.to_le_bytes()
        );
        assert!(store
            .complete(0, 2, WORKER_JOB_OUTCOME_COMPLETED, 102)
            .is_err());

        let queued = store
            .requeue(0, 1, WORKER_JOB_ERROR_RETRYABLE_EXECUTION, 105, 102)
            .unwrap();
        assert_eq!(queued.record.state_kind, WORKER_JOB_STATE_QUEUED);
        let ready = fs::read(store.worker_ready_index_path()).unwrap();
        assert_eq!(
            &ready[4..],
            &ServerOperationalBinaryV0Codec::encode_worker_ready_index(
                ServerWorkerReadyIndexRecord {
                    available_at_s: 105,
                    worker_job_index_plus1: 1,
                }
            )
            .unwrap()
        );

        assert!(store.begin_attempt(0, 104).is_err());
        store.begin_attempt(0, 105).unwrap();
        let completed = store
            .complete(0, 2, WORKER_JOB_OUTCOME_COMPLETED, 106)
            .unwrap();
        assert_eq!(completed.record.state_kind, WORKER_JOB_STATE_SUCCEEDED);
        assert_eq!(completed.record.outcome_kind, WORKER_JOB_OUTCOME_COMPLETED);
        assert!(store.begin_attempt(0, 107).is_err());
        assert!(store.ready_candidates(107, 10).unwrap().is_empty());

        let state = fs::read(store.worker_state_index_path()).unwrap();
        assert_eq!(
            &state[4..],
            &ServerOperationalBinaryV0Codec::encode_worker_state_index(
                ServerWorkerStateIndexRecord {
                    state_kind: WORKER_JOB_STATE_SUCCEEDED,
                    reserved0: 0,
                    reserved1: 0,
                    worker_job_index_plus1: 1,
                }
            )
            .unwrap()
        );
        cleanup(&root);
    }

    #[test]
    fn hot_path_reads_validate_only_relevant_jobs_and_reconciliation_skips_history() {
        let validation_count = Arc::new(AtomicUsize::new(0));
        let (root, store) = fixture_with_domain(
            "bounded-validation",
            12,
            Arc::new(CountingDomainAuthority {
                validation_count: validation_count.clone(),
            }),
        );
        store.initialize().unwrap();
        for _ in 0..3 {
            store
                .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
                .unwrap();
        }
        for worker_job_index in 0..2 {
            store.begin_attempt(worker_job_index, 101).unwrap();
            store
                .complete(worker_job_index, 1, WORKER_JOB_OUTCOME_COMPLETED, 102)
                .unwrap();
        }

        validation_count.store(0, Ordering::SeqCst);
        let ready = store.ready_candidates(103, 10).unwrap();
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].key.worker_job_index, 2);
        assert_eq!(validation_count.load(Ordering::SeqCst), 1);

        validation_count.store(0, Ordering::SeqCst);
        assert_eq!(store.get(2).unwrap().key.worker_job_index, 2);
        assert_eq!(validation_count.load(Ordering::SeqCst), 1);

        validation_count.store(0, Ordering::SeqCst);
        assert_eq!(store.lease_reconciliation_entries().unwrap().len(), 3);
        assert_eq!(validation_count.load(Ordering::SeqCst), 0);

        validation_count.store(0, Ordering::SeqCst);
        store.begin_attempt(2, 103).unwrap();
        assert_eq!(validation_count.load(Ordering::SeqCst), 2);

        validation_count.store(0, Ordering::SeqCst);
        store
            .with_exclusive_queue(|queue| {
                let current = queue.entry(2)?;
                queue.replace(
                    2,
                    ServerWorkerJobRecord {
                        locked_at_s: 104,
                        updated_at_s: 104,
                        ..current.record
                    },
                )
            })
            .unwrap();
        assert_eq!(validation_count.load(Ordering::SeqCst), 2);
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn state_neutral_mutation_does_not_replace_unchanged_indexes() {
        let (root, store) = fixture("state-neutral-indexes", 13);
        store.initialize().unwrap();
        store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();
        store.begin_attempt(0, 101).unwrap();

        let worker_inode = fs::metadata(store.worker_job_path()).unwrap().ino();
        let ready_inode = fs::metadata(store.worker_ready_index_path()).unwrap().ino();
        let state_inode = fs::metadata(store.worker_state_index_path()).unwrap().ino();
        store
            .with_exclusive_queue(|queue| {
                let current = queue.entry(0)?;
                queue.replace(
                    0,
                    ServerWorkerJobRecord {
                        locked_at_s: 102,
                        updated_at_s: 102,
                        ..current.record
                    },
                )
            })
            .unwrap();

        assert_eq!(
            fs::metadata(store.worker_job_path()).unwrap().ino(),
            worker_inode
        );
        assert_eq!(
            fs::metadata(store.worker_ready_index_path()).unwrap().ino(),
            ready_inode
        );
        assert_eq!(
            fs::metadata(store.worker_state_index_path()).unwrap().ino(),
            state_inode
        );
        assert!(!store.worker_job_update_journal_path().exists());
        assert!(!store.worker_job_update_staging_path().exists());
        cleanup(&root);
    }

    #[test]
    fn terminal_history_is_not_decoded_by_point_ready_state_or_update_paths() {
        let (root, store) = fixture("bounded-terminal-history", 14);
        store.initialize().unwrap();
        for created_at_s in 100..228 {
            store
                .enqueue(
                    WorkerJobCreateSpec {
                        created_at_s,
                        available_at_s: created_at_s,
                        ..patchset_ci_spec(created_at_s)
                    },
                    WorkerJobEnqueueDisposition::Skipped,
                )
                .unwrap();
        }
        let live = store
            .enqueue(
                WorkerJobCreateSpec {
                    created_at_s: 228,
                    available_at_s: 228,
                    ..patchset_ci_spec(228)
                },
                WorkerJobEnqueueDisposition::Queued,
            )
            .unwrap();

        let first_record_offset = OPERATIONAL_BIN_HEADER_SIZE;
        store
            .files
            .overwrite_range(&store.worker_job_path(), first_record_offset, &[0xff])
            .unwrap();
        store.files.sync_file(&store.worker_job_path()).unwrap();

        assert_eq!(store.get(live.key.worker_job_index).unwrap(), live);
        assert_eq!(store.ready_candidates(228, 1).unwrap(), vec![live]);
        assert_eq!(
            store.list_recent(Some(WORKER_JOB_STATE_QUEUED), 1).unwrap(),
            vec![live]
        );
        let running = store.begin_attempt(live.key.worker_job_index, 229).unwrap();
        assert_eq!(running.record.state_kind, WORKER_JOB_STATE_RUNNING);
        assert!(store.validate().is_err());
        cleanup(&root);
    }

    #[test]
    fn bounded_indexes_reject_stale_duplicate_and_out_of_range_rows() {
        let (root, store) = fixture("bounded-index-corruption", 15);
        store.initialize().unwrap();
        store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();

        let stale_ready = encode_ready_index_rows(&[ServerWorkerReadyIndexRecord {
            available_at_s: 99,
            worker_job_index_plus1: 1,
        }])
        .unwrap();
        fs::write(store.worker_ready_index_path(), stale_ready).unwrap();
        assert!(store
            .ready_candidates(100, 1)
            .unwrap_err()
            .contains("stale"));
        store.recover().unwrap();

        let duplicate_ready = encode_ready_index_rows(&[
            ServerWorkerReadyIndexRecord {
                available_at_s: 100,
                worker_job_index_plus1: 1,
            },
            ServerWorkerReadyIndexRecord {
                available_at_s: 101,
                worker_job_index_plus1: 1,
            },
        ])
        .unwrap();
        fs::write(store.worker_ready_index_path(), duplicate_ready).unwrap();
        assert!(store
            .ready_candidates(101, 2)
            .unwrap_err()
            .contains("duplicate"));
        store.recover().unwrap();

        let out_of_range_state = encode_state_index_rows(&[ServerWorkerStateIndexRecord {
            state_kind: WORKER_JOB_STATE_QUEUED,
            reserved0: 0,
            reserved1: 0,
            worker_job_index_plus1: 2,
        }])
        .unwrap();
        fs::write(store.worker_state_index_path(), out_of_range_state).unwrap();
        assert!(store
            .list_recent(Some(WORKER_JOB_STATE_QUEUED), 1)
            .unwrap_err()
            .contains("out-of-range"));
        store.recover().unwrap();

        let stale_state = encode_state_index_rows(&[ServerWorkerStateIndexRecord {
            state_kind: WORKER_JOB_STATE_SUCCEEDED,
            reserved0: 0,
            reserved1: 0,
            worker_job_index_plus1: 1,
        }])
        .unwrap();
        fs::write(store.worker_state_index_path(), stale_state).unwrap();
        assert!(store
            .list_recent(Some(WORKER_JOB_STATE_SUCCEEDED), 1)
            .unwrap_err()
            .contains("stale"));
        cleanup(&root);
    }

    #[test]
    fn interrupted_positional_update_rolls_back_from_durable_journal() {
        let (root, store) = fixture("positional-update-recovery", 16);
        store.initialize().unwrap();
        let queued = store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();
        let expected_ready = fs::read(store.worker_ready_index_path()).unwrap();
        let expected_state = fs::read(store.worker_state_index_path()).unwrap();
        let running = ServerWorkerJobRecord {
            state_kind: WORKER_JOB_STATE_RUNNING,
            attempt_count: 1,
            locked_at_s: 101,
            updated_at_s: 101,
            ..queued.record
        };
        let before_raw = ServerOperationalBinaryV0Codec::encode_worker_job(queued.record).unwrap();
        let after_raw = ServerOperationalBinaryV0Codec::encode_worker_job(running).unwrap();
        let journal = encode_worker_job_update_journal(0, &before_raw, &after_raw).unwrap();
        fs::write(store.worker_job_update_journal_path(), journal).unwrap();
        store
            .files
            .overwrite_range(
                &store.worker_job_path(),
                OPERATIONAL_BIN_HEADER_SIZE,
                &after_raw,
            )
            .unwrap();
        fs::write(
            store.worker_ready_index_path(),
            OPERATIONAL_V0_LAYOUT_ID.to_le_bytes(),
        )
        .unwrap();

        assert!(store.get(0).unwrap_err().contains("interrupted update"));
        assert_eq!(store.recover().unwrap(), vec![queued]);
        assert_eq!(
            fs::read(store.worker_ready_index_path()).unwrap(),
            expected_ready
        );
        assert_eq!(
            fs::read(store.worker_state_index_path()).unwrap(),
            expected_state
        );
        assert!(!store.worker_job_update_journal_path().exists());
        assert!(!store.worker_job_update_staging_path().exists());
        cleanup(&root);
    }

    #[test]
    fn retirement_blockers_and_bulk_tombstone_remove_every_runnable_projection() {
        let (root, store) = fixture("retirement", 2);
        store.initialize().unwrap();
        store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();
        store
            .enqueue(
                WorkerJobCreateSpec {
                    job_kind: WORKER_JOB_KIND_CONTENT_GC,
                    max_attempts: 1,
                    patchset_index_plus1: 0,
                    snapshot_index_plus1: 0,
                    available_at_s: 100,
                    created_at_s: 100,
                },
                WorkerJobEnqueueDisposition::Skipped,
            )
            .unwrap();
        assert_eq!(
            store.retirement_blockers().unwrap(),
            WorkerJobRetirementBlockers {
                queued: 1,
                running: 0,
            }
        );
        assert_eq!(store.tombstone_all(110).unwrap(), 2);
        assert!(store.retirement_blockers().unwrap().is_drained());
        assert!(store
            .validate()
            .unwrap()
            .iter()
            .all(|entry| { entry.record.is_tombstoned() && entry.record.updated_at_s >= 110 }));
        assert_eq!(
            fs::read(store.worker_ready_index_path()).unwrap(),
            1_u32.to_le_bytes()
        );
        assert_eq!(
            fs::read(store.worker_state_index_path()).unwrap(),
            1_u32.to_le_bytes()
        );
        assert_eq!(store.tombstone_all(111).unwrap(), 0);
        cleanup(&root);
    }

    #[test]
    fn deduplication_outcomes_keep_no_related_job_identity() {
        let (root, store) = fixture("dedup", 3);
        store.initialize().unwrap();
        let dispositions = [
            WorkerJobEnqueueDisposition::Queued,
            WorkerJobEnqueueDisposition::Attached,
            WorkerJobEnqueueDisposition::Skipped,
            WorkerJobEnqueueDisposition::Superseded,
        ];
        for disposition in dispositions {
            store.enqueue(patchset_ci_spec(100), disposition).unwrap();
        }

        let equivalent = store.find_equivalent(patchset_ci_spec(100)).unwrap();
        assert_eq!(
            equivalent
                .iter()
                .map(|entry| entry.key.worker_job_index)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
        assert_eq!(
            equivalent
                .iter()
                .map(|entry| entry.record.outcome_kind)
                .collect::<Vec<_>>(),
            [
                WORKER_JOB_OUTCOME_NONE,
                WORKER_JOB_OUTCOME_ATTACHED,
                WORKER_JOB_OUTCOME_SKIPPED,
                WORKER_JOB_OUTCOME_SUPERSEDED,
            ]
        );
        assert_eq!(
            fs::metadata(store.worker_job_path()).unwrap().len(),
            OPERATIONAL_BIN_HEADER_SIZE + 4 * u64::from(SERVER_WORKER_JOB_RECORD_SIZE)
        );
        assert!(!store
            .authority_root()
            .join("worker_job_payload.bin")
            .exists());
        cleanup(&root);
    }

    #[test]
    fn recovery_rebuilds_indexes_but_never_repairs_fixed_authority() {
        let (root, store) = fixture("recovery", 4);
        store.initialize().unwrap();
        store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();

        let expected_ready = fs::read(store.worker_ready_index_path()).unwrap();
        let expected_state = fs::read(store.worker_state_index_path()).unwrap();

        fs::write(
            store.worker_ready_index_path(),
            [1, 0, 0, 0, 0xff, 0xff, 0xff, 0xff],
        )
        .unwrap();
        fs::remove_file(store.worker_state_index_path()).unwrap();
        assert!(store.validate().is_err());
        assert_eq!(store.recover().unwrap().len(), 1);
        assert!(store.validate().is_ok());
        assert_eq!(
            fs::read(store.worker_ready_index_path()).unwrap(),
            expected_ready
        );
        assert_eq!(
            fs::read(store.worker_state_index_path()).unwrap(),
            expected_state
        );

        let mut authority = fs::read(store.worker_job_path()).unwrap();
        authority[5] = 1;
        fs::write(store.worker_job_path(), &authority).unwrap();
        let before = fs::read(store.worker_job_path()).unwrap();
        assert!(store.recover().is_err());
        assert_eq!(fs::read(store.worker_job_path()).unwrap(), before);
        cleanup(&root);
    }

    #[cfg(unix)]
    #[test]
    fn clean_recovery_preserves_exact_index_inodes() {
        let (root, store) = fixture("clean-recovery", 4);
        store.initialize().unwrap();
        store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();

        let worker_inode = fs::metadata(store.worker_job_path()).unwrap().ino();
        let ready_inode = fs::metadata(store.worker_ready_index_path()).unwrap().ino();
        let state_inode = fs::metadata(store.worker_state_index_path()).unwrap().ino();

        assert_eq!(store.recover().unwrap().len(), 1);
        assert_eq!(
            fs::metadata(store.worker_job_path()).unwrap().ino(),
            worker_inode
        );
        assert_eq!(
            fs::metadata(store.worker_ready_index_path()).unwrap().ino(),
            ready_inode
        );
        assert_eq!(
            fs::metadata(store.worker_state_index_path()).unwrap().ino(),
            state_inode
        );
        assert!(!store
            .authority_root()
            .join(WORKER_READY_REBUILD_FILE_NAME)
            .exists());
        assert!(!store
            .authority_root()
            .join(WORKER_STATE_REBUILD_FILE_NAME)
            .exists());
        cleanup(&root);
    }

    #[test]
    fn local_ready_selection_and_installation_merge_have_exact_order() {
        let (first_root, first_store) = fixture("merge-a", 7);
        let (second_root, second_store) = fixture("merge-b", 2);
        first_store.initialize().unwrap();
        second_store.initialize().unwrap();
        first_store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();
        first_store
            .enqueue(patchset_ci_spec(99), WorkerJobEnqueueDisposition::Queued)
            .unwrap();
        second_store
            .enqueue(patchset_ci_spec(100), WorkerJobEnqueueDisposition::Queued)
            .unwrap();

        let merged = merge_ready_candidates(
            first_store
                .ready_candidates(100, 10)
                .unwrap()
                .into_iter()
                .chain(second_store.ready_candidates(100, 10).unwrap()),
            10,
        );
        assert_eq!(
            merged
                .iter()
                .map(|entry| (
                    entry.record.available_at_s,
                    entry.key.repository_index,
                    entry.key.worker_job_index,
                ))
                .collect::<Vec<_>>(),
            [(99, 7, 1), (100, 2, 0), (100, 7, 0)]
        );
        cleanup(&first_root);
        cleanup(&second_root);
    }

    #[test]
    fn queue_lock_serializes_concurrent_physical_index_allocation() {
        let (root, store) = fixture("concurrent", 8);
        store.initialize().unwrap();
        let first_store = store.clone();
        let second_store = store.clone();
        let first = std::thread::spawn(move || {
            first_store
                .enqueue(
                    WorkerJobCreateSpec {
                        job_kind: WORKER_JOB_KIND_CONTENT_GC,
                        max_attempts: 1,
                        patchset_index_plus1: 0,
                        snapshot_index_plus1: 0,
                        available_at_s: 100,
                        created_at_s: 100,
                    },
                    WorkerJobEnqueueDisposition::Queued,
                )
                .unwrap()
        });
        let second = std::thread::spawn(move || {
            second_store
                .enqueue(
                    WorkerJobCreateSpec {
                        job_kind: WORKER_JOB_KIND_CONTENT_GC,
                        max_attempts: 1,
                        patchset_index_plus1: 0,
                        snapshot_index_plus1: 0,
                        available_at_s: 101,
                        created_at_s: 101,
                    },
                    WorkerJobEnqueueDisposition::Queued,
                )
                .unwrap()
        });
        let mut allocated = [
            first.join().unwrap().key.worker_job_index,
            second.join().unwrap().key.worker_job_index,
        ];
        allocated.sort();
        assert_eq!(allocated, [0, 1]);
        assert_eq!(store.validate().unwrap().len(), 2);
        cleanup(&root);
    }

    #[test]
    fn numeric_root_and_no_payload_boundaries_fail_closed() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ait-server-worker-jobs-boundary-{}-{nonce}",
            std::process::id()
        ));
        let alias_root = root.join("01");
        fs::create_dir_all(&alias_root).unwrap();
        assert!(ServerOperationalWorkerJobStore::new(
            1,
            &alias_root,
            Arc::new(TestDomainAuthority::default()),
        )
        .is_err());
        assert!(ServerOperationalWorkerJobStore::new(
            2,
            root.join("01"),
            Arc::new(TestDomainAuthority::default()),
        )
        .is_err());

        let (fixture_root, store) = fixture("forbidden-payload", 9);
        fs::write(
            store.authority_root().join("worker_job_result_payload.bin"),
            [],
        )
        .unwrap();
        assert!(store.initialize().is_err());
        cleanup(&root);
        cleanup(&fixture_root);
    }
}
