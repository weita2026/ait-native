use crate::foundation::operational_binary_v0::{
    ServerWorkerJobRecord, WORKER_JOB_ERROR_LEASE_EXPIRED, WORKER_JOB_ERROR_NONE,
    WORKER_JOB_ERROR_RETRYABLE_EXECUTION, WORKER_JOB_ERROR_TERMINAL_EXECUTION,
    WORKER_JOB_OUTCOME_COMPLETED, WORKER_JOB_OUTCOME_FAILED, WORKER_JOB_OUTCOME_NONE,
    WORKER_JOB_OUTCOME_SKIPPED, WORKER_JOB_STATE_FAILED, WORKER_JOB_STATE_QUEUED,
    WORKER_JOB_STATE_RUNNING, WORKER_JOB_STATE_SUCCEEDED,
};
use crate::foundation::remote_binary_db::{
    BinaryDbError, ServerBinaryDbDurabilityStore, ServerBinaryDbFilesystemStore, StoreResult,
};
use crate::foundation::server_operational_worker_jobs::{
    ServerOperationalWorkerJobStore, WorkerJobEntry, WorkerJobKey,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

const LEASE_REPLICA_MAGIC: [u8; 8] = *b"AITLSE02";
const LEASE_REPLICA_FORMAT_VERSION: u32 = 2;
const LEASE_REPLICA_HEADER_SIZE: usize = 24;
const LEASE_REPLICA_RECORD_PREFIX_SIZE: usize = 44;
const LEASE_REPLICA_DIGEST_SIZE: usize = 32;
const LEASE_REPLICA_RECORD_SIZE: usize =
    LEASE_REPLICA_RECORD_PREFIX_SIZE + LEASE_REPLICA_DIGEST_SIZE;
const LEASE_REPLICA_ENTRY_DOMAIN: &[u8] = b"ait.runtime-lease-replica.entry.v2\0";
const LEASE_TOKEN_SIZE: usize = 16;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RuntimeLeaseAttemptKey {
    pub repository_index: u32,
    pub worker_job_index: u32,
    pub attempt_count: u16,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub struct RuntimeLeaseToken([u8; LEASE_TOKEN_SIZE]);

impl std::fmt::Debug for RuntimeLeaseToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("RuntimeLeaseToken([REDACTED])")
    }
}

impl RuntimeLeaseToken {
    pub fn from_bytes(bytes: [u8; LEASE_TOKEN_SIZE]) -> StoreResult<Self> {
        if bytes.iter().all(|byte| *byte == 0) {
            return Err(invalid("runtime lease token cannot be all zero"));
        }
        Ok(Self(bytes))
    }

    pub fn parse_hex(value: &str) -> StoreResult<Self> {
        if value.len() != LEASE_TOKEN_SIZE * 2 {
            return Err(invalid(
                "runtime lease token must contain 32 hexadecimal characters",
            ));
        }
        let mut bytes = [0_u8; LEASE_TOKEN_SIZE];
        for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = decode_hex_nibble(pair[0])
                .ok_or_else(|| invalid("runtime lease token contains non-hexadecimal bytes"))?;
            let low = decode_hex_nibble(pair[1])
                .ok_or_else(|| invalid("runtime lease token contains non-hexadecimal bytes"))?;
            bytes[index] = (high << 4) | low;
        }
        Self::from_bytes(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; LEASE_TOKEN_SIZE] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(LEASE_TOKEN_SIZE * 2);
        for byte in self.0 {
            out.push(hex_nibble(byte >> 4));
            out.push(hex_nibble(byte & 0x0f));
        }
        out
    }

    fn exact_matches(self, other: Self) -> bool {
        let mut difference = 0_u8;
        for (left, right) in self.0.into_iter().zip(other.0) {
            difference |= left ^ right;
        }
        difference == 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLeaseEntry {
    pub key: RuntimeLeaseAttemptKey,
    pub token: RuntimeLeaseToken,
    pub heartbeat_at_s: u64,
    pub expires_at_s: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeLeaseGrant {
    pub worker_job: WorkerJobEntry,
    pub attempt_count: u16,
    pub lease_token: RuntimeLeaseToken,
    pub heartbeat_at_s: u64,
    pub expires_at_s: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeLeaseReplicaOpenReport {
    pub replica_existed: bool,
    pub replica_discarded: bool,
    pub loaded_entry_count: u32,
    pub discarded_entry_count: u32,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RuntimeLeaseReconciliationReport {
    pub valid_entry_count: u32,
    pub discarded_entry_count: u32,
    pub requeued_job_count: u32,
    pub failed_job_count: u32,
}

struct RuntimeLeaseState {
    replica_path: PathBuf,
    replica_parent: PathBuf,
    activated_roots: Vec<PathBuf>,
    entries: Mutex<BTreeMap<RuntimeLeaseAttemptKey, RuntimeLeaseEntry>>,
    files: ServerBinaryDbFilesystemStore,
}

#[derive(Clone)]
pub struct ServerOperationalRuntimeLeases {
    state: Arc<RuntimeLeaseState>,
}

impl std::fmt::Debug for ServerOperationalRuntimeLeases {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerOperationalRuntimeLeases")
            .field("replica_path", &self.state.replica_path)
            .field("activated_roots", &self.state.activated_roots)
            .finish_non_exhaustive()
    }
}

impl ServerOperationalRuntimeLeases {
    pub fn open(
        replica_path: impl Into<PathBuf>,
        activated_roots: impl IntoIterator<Item = PathBuf>,
    ) -> StoreResult<(Self, RuntimeLeaseReplicaOpenReport)> {
        let (replica_path, replica_parent, activated_roots) =
            validate_replica_location(replica_path.into(), activated_roots)?;
        let replica_existed = replica_path.exists();
        let mut report = RuntimeLeaseReplicaOpenReport {
            replica_existed,
            ..RuntimeLeaseReplicaOpenReport::default()
        };
        let entries = if replica_existed {
            let raw = read_regular_file(&replica_path)?;
            match decode_replica(&raw) {
                Ok(decoded) => {
                    report.loaded_entry_count = u32::try_from(decoded.entries.len())
                        .map_err(|_| corrupt("runtime lease replica entry count exceeds u32"))?;
                    report.discarded_entry_count = decoded.discarded_entry_count;
                    decoded.entries
                }
                Err(_) => {
                    report.replica_discarded = true;
                    BTreeMap::new()
                }
            }
        } else {
            BTreeMap::new()
        };
        let leases = Self {
            state: Arc::new(RuntimeLeaseState {
                replica_path,
                replica_parent,
                activated_roots,
                entries: Mutex::new(entries),
                files: ServerBinaryDbFilesystemStore,
            }),
        };
        leases.persist_current_replica()?;
        Ok((leases, report))
    }

    pub fn replica_path(&self) -> &Path {
        &self.state.replica_path
    }

    pub fn activated_roots(&self) -> &[PathBuf] {
        &self.state.activated_roots
    }

    pub fn entries(&self) -> StoreResult<Vec<RuntimeLeaseEntry>> {
        Ok(self.lock_entries()?.values().copied().collect())
    }

    pub fn claim(
        &self,
        store: &ServerOperationalWorkerJobStore,
        worker_job_index: u32,
        now_s: u64,
        lease_duration_s: u32,
    ) -> StoreResult<RuntimeLeaseGrant> {
        let expires_at_s = lease_expiry(now_s, lease_duration_s)?;
        let token = random_token()?;
        store.with_exclusive_queue(|queue| {
            let current = queue.entry(worker_job_index)?;
            require_claimable(current.record, now_s)?;
            let attempt_count = current
                .record
                .attempt_count
                .checked_add(1)
                .ok_or_else(|| corrupt("Worker Job attempt count overflow"))?;
            let key = RuntimeLeaseAttemptKey {
                repository_index: store.repository_index(),
                worker_job_index,
                attempt_count,
            };
            let lease = RuntimeLeaseEntry {
                key,
                token,
                heartbeat_at_s: now_s,
                expires_at_s,
            };

            self.replace_job_lease(lease)?;
            let worker_job = queue.replace(
                worker_job_index,
                ServerWorkerJobRecord {
                    state_kind: WORKER_JOB_STATE_RUNNING,
                    outcome_kind: WORKER_JOB_OUTCOME_NONE,
                    attempt_count,
                    error_kind: WORKER_JOB_ERROR_NONE,
                    locked_at_s: now_s,
                    updated_at_s: now_s,
                    ..current.record
                },
            )?;
            Ok(RuntimeLeaseGrant {
                worker_job,
                attempt_count,
                lease_token: token,
                heartbeat_at_s: now_s,
                expires_at_s,
            })
        })
    }

    pub fn heartbeat(
        &self,
        store: &ServerOperationalWorkerJobStore,
        worker_job_index: u32,
        attempt_count: u16,
        token: RuntimeLeaseToken,
        now_s: u64,
        lease_duration_s: u32,
    ) -> StoreResult<RuntimeLeaseGrant> {
        let expires_at_s = lease_expiry(now_s, lease_duration_s)?;
        store.with_exclusive_queue(|queue| {
            let current = queue.entry(worker_job_index)?;
            require_running_attempt(current.record, attempt_count)?;
            if now_s < current.record.updated_at_s {
                return Err(invalid("runtime lease heartbeat moved time backwards"));
            }
            let key = RuntimeLeaseAttemptKey {
                repository_index: store.repository_index(),
                worker_job_index,
                attempt_count,
            };
            let lease = self.update_heartbeat(key, token, now_s, expires_at_s)?;
            let worker_job = queue.replace(
                worker_job_index,
                ServerWorkerJobRecord {
                    locked_at_s: now_s,
                    updated_at_s: now_s,
                    ..current.record
                },
            )?;
            Ok(RuntimeLeaseGrant {
                worker_job,
                attempt_count,
                lease_token: token,
                heartbeat_at_s: lease.heartbeat_at_s,
                expires_at_s: lease.expires_at_s,
            })
        })
    }

    pub fn validate_presented(
        &self,
        store: &ServerOperationalWorkerJobStore,
        worker_job_index: u32,
        attempt_count: u16,
        token: RuntimeLeaseToken,
        now_s: u64,
    ) -> StoreResult<RuntimeLeaseEntry> {
        if now_s == 0 {
            return Err(invalid("runtime lease validation time is required"));
        }
        store.with_exclusive_queue(|queue| {
            let current = queue.entry(worker_job_index)?;
            require_running_attempt(current.record, attempt_count)?;
            self.valid_entry(
                RuntimeLeaseAttemptKey {
                    repository_index: store.repository_index(),
                    worker_job_index,
                    attempt_count,
                },
                token,
                now_s,
            )
        })
    }

    pub fn complete_after_domain_commit(
        &self,
        store: &ServerOperationalWorkerJobStore,
        worker_job_index: u32,
        attempt_count: u16,
        token: RuntimeLeaseToken,
        outcome_kind: u8,
        now_s: u64,
    ) -> StoreResult<WorkerJobEntry> {
        self.complete_with_domain_commit(
            store,
            worker_job_index,
            attempt_count,
            token,
            outcome_kind,
            now_s,
            |_| Ok(()),
        )
        .map(|(worker_job, ())| worker_job)
    }

    pub fn complete_with_domain_commit<T>(
        &self,
        store: &ServerOperationalWorkerJobStore,
        worker_job_index: u32,
        attempt_count: u16,
        token: RuntimeLeaseToken,
        outcome_kind: u8,
        now_s: u64,
        domain_commit: impl FnOnce(WorkerJobEntry) -> StoreResult<T>,
    ) -> StoreResult<(WorkerJobEntry, T)> {
        if !matches!(
            outcome_kind,
            WORKER_JOB_OUTCOME_COMPLETED | WORKER_JOB_OUTCOME_SKIPPED
        ) {
            return Err(invalid(
                "lease-backed independent completion outcome is invalid",
            ));
        }
        store.with_exclusive_queue(|queue| {
            let current = queue.entry(worker_job_index)?;
            require_running_attempt(current.record, attempt_count)?;
            if now_s < current.record.updated_at_s {
                return Err(invalid("Worker Job completion moved time backwards"));
            }
            let key = RuntimeLeaseAttemptKey {
                repository_index: store.repository_index(),
                worker_job_index,
                attempt_count,
            };
            self.valid_entry(key, token, now_s)?;
            let domain_result = domain_commit(current)?;
            let worker_job = queue.replace(
                worker_job_index,
                ServerWorkerJobRecord {
                    state_kind: WORKER_JOB_STATE_SUCCEEDED,
                    outcome_kind,
                    error_kind: WORKER_JOB_ERROR_NONE,
                    locked_at_s: 0,
                    updated_at_s: now_s,
                    ..current.record
                },
            )?;
            self.remove_exact_lease(key, token)?;
            Ok((worker_job, domain_result))
        })
    }

    pub fn fail_attempt(
        &self,
        store: &ServerOperationalWorkerJobStore,
        worker_job_index: u32,
        attempt_count: u16,
        token: RuntimeLeaseToken,
        error_kind: u16,
        retry_available_at_s: Option<u64>,
        now_s: u64,
    ) -> StoreResult<WorkerJobEntry> {
        store.with_exclusive_queue(|queue| {
            let current = queue.entry(worker_job_index)?;
            require_running_attempt(current.record, attempt_count)?;
            if now_s < current.record.updated_at_s {
                return Err(invalid("Worker Job failure moved time backwards"));
            }
            let key = RuntimeLeaseAttemptKey {
                repository_index: store.repository_index(),
                worker_job_index,
                attempt_count,
            };
            self.valid_entry(key, token, now_s)?;
            let next = match retry_available_at_s {
                Some(available_at_s) => {
                    if current.record.attempt_count >= current.record.max_attempts {
                        return Err(invalid(
                            "exhausted Worker Job cannot return to queued state",
                        ));
                    }
                    if !matches!(
                        error_kind,
                        WORKER_JOB_ERROR_RETRYABLE_EXECUTION | WORKER_JOB_ERROR_LEASE_EXPIRED
                    ) {
                        return Err(invalid("requeued Worker Job error kind is invalid"));
                    }
                    ServerWorkerJobRecord {
                        state_kind: WORKER_JOB_STATE_QUEUED,
                        outcome_kind: WORKER_JOB_OUTCOME_NONE,
                        error_kind,
                        available_at_s,
                        locked_at_s: 0,
                        updated_at_s: now_s,
                        ..current.record
                    }
                }
                None => {
                    if !matches!(
                        error_kind,
                        WORKER_JOB_ERROR_TERMINAL_EXECUTION | WORKER_JOB_ERROR_LEASE_EXPIRED
                    ) {
                        return Err(invalid("terminal Worker Job error kind is invalid"));
                    }
                    ServerWorkerJobRecord {
                        state_kind: WORKER_JOB_STATE_FAILED,
                        outcome_kind: WORKER_JOB_OUTCOME_FAILED,
                        error_kind,
                        locked_at_s: 0,
                        updated_at_s: now_s,
                        ..current.record
                    }
                }
            };
            let worker_job = queue.replace(worker_job_index, next)?;
            self.remove_exact_lease(key, token)?;
            Ok(worker_job)
        })
    }

    pub fn reconcile(
        &self,
        stores: &[ServerOperationalWorkerJobStore],
        now_s: u64,
        retry_delay_s: u32,
    ) -> StoreResult<RuntimeLeaseReconciliationReport> {
        if now_s == 0 {
            return Err(invalid("runtime lease reconciliation time is required"));
        }
        let mut store_by_repository = BTreeMap::new();
        let mut jobs_by_key = BTreeMap::new();
        for store in stores {
            if store_by_repository
                .insert(store.repository_index(), store)
                .is_some()
            {
                return Err(invalid(
                    "runtime lease reconciliation has duplicate Repository stores",
                ));
            }
            for entry in store.lease_reconciliation_entries()? {
                jobs_by_key.insert(entry.key, entry);
            }
        }

        let snapshot = self
            .lock_entries()?
            .iter()
            .map(|(key, entry)| (*key, *entry))
            .collect::<BTreeMap<_, _>>();
        let mut valid_keys = BTreeSet::new();
        let mut invalid_entries = Vec::new();
        for (key, lease) in &snapshot {
            let job_key = WorkerJobKey {
                repository_index: key.repository_index,
                worker_job_index: key.worker_job_index,
            };
            let valid = jobs_by_key.get(&job_key).is_some_and(|job| {
                !job.record.is_tombstoned()
                    && job.record.state_kind == WORKER_JOB_STATE_RUNNING
                    && job.record.attempt_count == key.attempt_count
                    && lease_is_live(*lease, now_s)
            });
            if valid {
                valid_keys.insert(*key);
            } else {
                invalid_entries.push((*key, *lease));
            }
        }
        self.remove_snapshot_entries(&invalid_entries)?;

        let mut report = RuntimeLeaseReconciliationReport {
            valid_entry_count: u32::try_from(valid_keys.len())
                .map_err(|_| corrupt("runtime lease valid-entry count exceeds u32"))?,
            discarded_entry_count: u32::try_from(invalid_entries.len())
                .map_err(|_| corrupt("runtime lease discarded-entry count exceeds u32"))?,
            ..RuntimeLeaseReconciliationReport::default()
        };
        for job in jobs_by_key.values().filter(|job| {
            !job.record.is_tombstoned() && job.record.state_kind == WORKER_JOB_STATE_RUNNING
        }) {
            let key = RuntimeLeaseAttemptKey {
                repository_index: job.key.repository_index,
                worker_job_index: job.key.worker_job_index,
                attempt_count: job.record.attempt_count,
            };
            if valid_keys.contains(&key) {
                continue;
            }
            let store = store_by_repository
                .get(&job.key.repository_index)
                .ok_or_else(|| corrupt("running Worker Job lost its Repository store"))?;
            match self.reconcile_lease_lost(
                store,
                job.key.worker_job_index,
                job.record.attempt_count,
                now_s,
                retry_delay_s,
            )? {
                LeaseLostDisposition::StillValid => {
                    report.valid_entry_count = report
                        .valid_entry_count
                        .checked_add(1)
                        .ok_or_else(|| corrupt("runtime lease report count overflow"))?;
                }
                LeaseLostDisposition::Requeued => {
                    report.requeued_job_count = report
                        .requeued_job_count
                        .checked_add(1)
                        .ok_or_else(|| corrupt("runtime lease report count overflow"))?;
                }
                LeaseLostDisposition::Failed => {
                    report.failed_job_count = report
                        .failed_job_count
                        .checked_add(1)
                        .ok_or_else(|| corrupt("runtime lease report count overflow"))?;
                }
                LeaseLostDisposition::NoLongerRunning => {}
            }
        }
        Ok(report)
    }

    fn reconcile_lease_lost(
        &self,
        store: &ServerOperationalWorkerJobStore,
        worker_job_index: u32,
        expected_attempt_count: u16,
        now_s: u64,
        retry_delay_s: u32,
    ) -> StoreResult<LeaseLostDisposition> {
        store.with_exclusive_queue(|queue| {
            let current = queue.entry(worker_job_index)?;
            if current.record.is_tombstoned()
                || current.record.state_kind != WORKER_JOB_STATE_RUNNING
                || current.record.attempt_count != expected_attempt_count
            {
                return Ok(LeaseLostDisposition::NoLongerRunning);
            }
            let key = RuntimeLeaseAttemptKey {
                repository_index: store.repository_index(),
                worker_job_index,
                attempt_count: expected_attempt_count,
            };
            if self.has_live_entry(key, now_s)? {
                return Ok(LeaseLostDisposition::StillValid);
            }
            let updated_at_s = now_s.max(current.record.updated_at_s);
            let (next, disposition) = if current.record.attempt_count < current.record.max_attempts
            {
                let available_at_s = updated_at_s
                    .checked_add(u64::from(retry_delay_s))
                    .ok_or_else(|| invalid("lease-lost retry time exceeds u64"))?;
                (
                    ServerWorkerJobRecord {
                        state_kind: WORKER_JOB_STATE_QUEUED,
                        outcome_kind: WORKER_JOB_OUTCOME_NONE,
                        error_kind: WORKER_JOB_ERROR_LEASE_EXPIRED,
                        available_at_s,
                        locked_at_s: 0,
                        updated_at_s,
                        ..current.record
                    },
                    LeaseLostDisposition::Requeued,
                )
            } else {
                (
                    ServerWorkerJobRecord {
                        state_kind: WORKER_JOB_STATE_FAILED,
                        outcome_kind: WORKER_JOB_OUTCOME_FAILED,
                        error_kind: WORKER_JOB_ERROR_LEASE_EXPIRED,
                        locked_at_s: 0,
                        updated_at_s,
                        ..current.record
                    },
                    LeaseLostDisposition::Failed,
                )
            };
            queue.replace(worker_job_index, next)?;
            self.remove_job_leases(store.repository_index(), worker_job_index)?;
            Ok(disposition)
        })
    }

    fn valid_entry(
        &self,
        key: RuntimeLeaseAttemptKey,
        token: RuntimeLeaseToken,
        now_s: u64,
    ) -> StoreResult<RuntimeLeaseEntry> {
        let entries = self.lock_entries()?;
        let entry = entries
            .get(&key)
            .copied()
            .ok_or_else(|| invalid("runtime lease is absent or attempt-mismatched"))?;
        if !entry.token.exact_matches(token) || !lease_is_live(entry, now_s) {
            return Err(invalid("runtime lease token is invalid or expired"));
        }
        Ok(entry)
    }

    fn has_live_entry(&self, key: RuntimeLeaseAttemptKey, now_s: u64) -> StoreResult<bool> {
        Ok(self
            .lock_entries()?
            .get(&key)
            .copied()
            .is_some_and(|entry| lease_is_live(entry, now_s)))
    }

    fn replace_job_lease(&self, lease: RuntimeLeaseEntry) -> StoreResult<()> {
        self.mutate_entries(|entries| {
            entries.retain(|key, _| {
                key.repository_index != lease.key.repository_index
                    || key.worker_job_index != lease.key.worker_job_index
            });
            entries.insert(lease.key, lease);
            Ok(())
        })
    }

    fn update_heartbeat(
        &self,
        key: RuntimeLeaseAttemptKey,
        token: RuntimeLeaseToken,
        now_s: u64,
        expires_at_s: u64,
    ) -> StoreResult<RuntimeLeaseEntry> {
        self.mutate_entries(|entries| {
            let current = entries
                .get(&key)
                .copied()
                .ok_or_else(|| invalid("runtime lease is absent or attempt-mismatched"))?;
            if !current.token.exact_matches(token)
                || !lease_is_live(current, now_s)
                || now_s < current.heartbeat_at_s
            {
                return Err(invalid("runtime lease token is invalid, expired, or stale"));
            }
            let next = RuntimeLeaseEntry {
                heartbeat_at_s: now_s,
                expires_at_s,
                ..current
            };
            entries.insert(key, next);
            Ok(next)
        })
    }

    fn remove_exact_lease(
        &self,
        key: RuntimeLeaseAttemptKey,
        token: RuntimeLeaseToken,
    ) -> StoreResult<()> {
        self.mutate_entries(|entries| {
            if entries
                .get(&key)
                .is_some_and(|entry| entry.token.exact_matches(token))
            {
                entries.remove(&key);
            }
            Ok(())
        })
    }

    fn remove_job_leases(&self, repository_index: u32, worker_job_index: u32) -> StoreResult<()> {
        self.mutate_entries(|entries| {
            entries.retain(|key, _| {
                key.repository_index != repository_index || key.worker_job_index != worker_job_index
            });
            Ok(())
        })
    }

    fn remove_snapshot_entries(
        &self,
        invalid_entries: &[(RuntimeLeaseAttemptKey, RuntimeLeaseEntry)],
    ) -> StoreResult<()> {
        if invalid_entries.is_empty() {
            return Ok(());
        }
        self.mutate_entries(|entries| {
            for (key, stale) in invalid_entries {
                if entries.get(key) == Some(stale) {
                    entries.remove(key);
                }
            }
            Ok(())
        })
    }

    fn mutate_entries<T>(
        &self,
        mutation: impl FnOnce(
            &mut BTreeMap<RuntimeLeaseAttemptKey, RuntimeLeaseEntry>,
        ) -> StoreResult<T>,
    ) -> StoreResult<T> {
        let mut entries = self.lock_entries()?;
        let before = entries.clone();
        let result = mutation(&mut entries)?;
        if let Err(error) = self.persist_entries(&entries) {
            *entries = before;
            return Err(error);
        }
        Ok(result)
    }

    fn persist_current_replica(&self) -> StoreResult<()> {
        let entries = self.lock_entries()?;
        self.persist_entries(&entries)
    }

    fn persist_entries(
        &self,
        entries: &BTreeMap<RuntimeLeaseAttemptKey, RuntimeLeaseEntry>,
    ) -> StoreResult<()> {
        validate_replica_still_outside_authority(
            &self.state.replica_path,
            &self.state.activated_roots,
        )?;
        let bytes = encode_replica(entries)?;
        let file_name = self
            .state
            .replica_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| invalid("runtime lease replica filename is not UTF-8"))?;
        let temporary = self
            .state
            .replica_parent
            .join(format!(".{file_name}.rewrite"));
        reject_symlink_or_hardlink_if_present(&self.state.replica_path)?;
        if temporary.exists() {
            reject_symlink_or_hardlink_if_present(&temporary)?;
            fs::remove_file(&temporary).map_err(|error| {
                BinaryDbError::io(
                    format!("remove stale runtime lease replica {}", temporary.display()),
                    error,
                )
            })?;
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| {
                BinaryDbError::io(
                    format!("create runtime lease replica {}", temporary.display()),
                    error,
                )
            })?;
        file.write_all(&bytes).map_err(|error| {
            BinaryDbError::io(
                format!("write runtime lease replica {}", temporary.display()),
                error,
            )
        })?;
        file.sync_all().map_err(|error| {
            BinaryDbError::io(
                format!("sync runtime lease replica {}", temporary.display()),
                error,
            )
        })?;
        fs::rename(&temporary, &self.state.replica_path).map_err(|error| {
            BinaryDbError::io(
                format!(
                    "activate runtime lease replica {} as {}",
                    temporary.display(),
                    self.state.replica_path.display()
                ),
                error,
            )
        })?;
        self.state.files.sync_directory(&self.state.replica_parent)
    }

    fn lock_entries(
        &self,
    ) -> StoreResult<MutexGuard<'_, BTreeMap<RuntimeLeaseAttemptKey, RuntimeLeaseEntry>>> {
        self.state
            .entries
            .lock()
            .map_err(|_| BinaryDbError::other("runtime lease memory lock is poisoned"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LeaseLostDisposition {
    StillValid,
    Requeued,
    Failed,
    NoLongerRunning,
}

struct DecodedLeaseReplica {
    entries: BTreeMap<RuntimeLeaseAttemptKey, RuntimeLeaseEntry>,
    discarded_entry_count: u32,
}

fn require_claimable(record: ServerWorkerJobRecord, now_s: u64) -> StoreResult<()> {
    if record.is_tombstoned() || record.state_kind != WORKER_JOB_STATE_QUEUED {
        return Err(invalid("only a live queued Worker Job can be claimed"));
    }
    if now_s == 0
        || now_s < record.updated_at_s
        || record.available_at_s > now_s
        || record.attempt_count >= record.max_attempts
    {
        return Err(invalid(
            "Worker Job is unavailable, time-invalid, or attempt-exhausted",
        ));
    }
    Ok(())
}

fn require_running_attempt(
    record: ServerWorkerJobRecord,
    expected_attempt_count: u16,
) -> StoreResult<()> {
    if record.is_tombstoned()
        || record.state_kind != WORKER_JOB_STATE_RUNNING
        || record.attempt_count != expected_attempt_count
    {
        return Err(invalid(
            "Worker Job is not the expected live running attempt",
        ));
    }
    Ok(())
}

fn lease_expiry(now_s: u64, lease_duration_s: u32) -> StoreResult<u64> {
    if now_s == 0 || lease_duration_s == 0 {
        return Err(invalid("runtime lease time and duration are required"));
    }
    now_s
        .checked_add(u64::from(lease_duration_s))
        .ok_or_else(|| invalid("runtime lease expiry exceeds u64"))
}

fn lease_is_live(entry: RuntimeLeaseEntry, now_s: u64) -> bool {
    entry.key.attempt_count != 0
        && entry.heartbeat_at_s != 0
        && entry.heartbeat_at_s <= now_s
        && entry.expires_at_s > entry.heartbeat_at_s
        && entry.expires_at_s > now_s
        && entry.token.0.iter().any(|byte| *byte != 0)
}

fn random_token() -> StoreResult<RuntimeLeaseToken> {
    loop {
        let mut bytes = [0_u8; LEASE_TOKEN_SIZE];
        getrandom::fill(&mut bytes).map_err(|error| {
            BinaryDbError::other(format!("generate runtime lease token: {error}"))
        })?;
        if let Ok(token) = RuntimeLeaseToken::from_bytes(bytes) {
            return Ok(token);
        }
    }
}

fn encode_replica(
    entries: &BTreeMap<RuntimeLeaseAttemptKey, RuntimeLeaseEntry>,
) -> StoreResult<Vec<u8>> {
    let count = u32::try_from(entries.len())
        .map_err(|_| invalid("runtime lease replica entry count exceeds u32"))?;
    let body_size = entries
        .len()
        .checked_mul(LEASE_REPLICA_RECORD_SIZE)
        .ok_or_else(|| invalid("runtime lease replica size overflow"))?;
    let mut out = Vec::with_capacity(
        LEASE_REPLICA_HEADER_SIZE
            .checked_add(body_size)
            .ok_or_else(|| invalid("runtime lease replica size overflow"))?,
    );
    out.extend_from_slice(&LEASE_REPLICA_MAGIC);
    out.extend_from_slice(&LEASE_REPLICA_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(LEASE_REPLICA_RECORD_SIZE)
            .expect("runtime lease record size fits u32")
            .to_le_bytes(),
    );
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes());
    for entry in entries.values() {
        validate_lease_entry(*entry)?;
        let prefix = encode_entry_prefix(*entry);
        let digest = entry_digest(&prefix);
        out.extend_from_slice(&prefix);
        out.extend_from_slice(&digest);
    }
    Ok(out)
}

fn decode_replica(raw: &[u8]) -> StoreResult<DecodedLeaseReplica> {
    if raw.len() < LEASE_REPLICA_HEADER_SIZE {
        return Err(corrupt("runtime lease replica header is incomplete"));
    }
    if raw[..8] != LEASE_REPLICA_MAGIC {
        return Err(corrupt("runtime lease replica magic is invalid"));
    }
    let version = read_u32(raw, 8)?;
    let record_size = read_u32(raw, 12)?;
    let count = read_u32(raw, 16)?;
    let reserved = read_u32(raw, 20)?;
    if version != LEASE_REPLICA_FORMAT_VERSION
        || record_size != LEASE_REPLICA_RECORD_SIZE as u32
        || reserved != 0
    {
        return Err(corrupt(
            "runtime lease replica format or reserved header is invalid",
        ));
    }
    let expected_body = usize::try_from(count)
        .map_err(|_| corrupt("runtime lease replica count exceeds usize"))?
        .checked_mul(LEASE_REPLICA_RECORD_SIZE)
        .ok_or_else(|| corrupt("runtime lease replica length overflow"))?;
    if raw.len() != LEASE_REPLICA_HEADER_SIZE + expected_body {
        return Err(corrupt(
            "runtime lease replica count or trailing record is invalid",
        ));
    }
    let mut entries = BTreeMap::new();
    let mut discarded_entry_count = 0_u32;
    for record in raw[LEASE_REPLICA_HEADER_SIZE..].chunks_exact(LEASE_REPLICA_RECORD_SIZE) {
        let prefix = &record[..LEASE_REPLICA_RECORD_PREFIX_SIZE];
        let digest = &record[LEASE_REPLICA_RECORD_PREFIX_SIZE..];
        if !constant_time_bytes_equal(digest, &entry_digest(prefix)) {
            discarded_entry_count = discarded_entry_count
                .checked_add(1)
                .ok_or_else(|| corrupt("runtime lease discard count overflow"))?;
            continue;
        }
        let entry = decode_entry_prefix(prefix)?;
        if entries.insert(entry.key, entry).is_some() {
            return Err(corrupt("runtime lease replica contains a duplicate key"));
        }
    }
    Ok(DecodedLeaseReplica {
        entries,
        discarded_entry_count,
    })
}

fn encode_entry_prefix(entry: RuntimeLeaseEntry) -> [u8; LEASE_REPLICA_RECORD_PREFIX_SIZE] {
    let mut raw = [0_u8; LEASE_REPLICA_RECORD_PREFIX_SIZE];
    raw[0..4].copy_from_slice(&entry.key.repository_index.to_le_bytes());
    raw[4..8].copy_from_slice(&entry.key.worker_job_index.to_le_bytes());
    raw[8..10].copy_from_slice(&entry.key.attempt_count.to_le_bytes());
    raw[10..12].copy_from_slice(&0_u16.to_le_bytes());
    raw[12..28].copy_from_slice(entry.token.as_bytes());
    raw[28..36].copy_from_slice(&entry.heartbeat_at_s.to_le_bytes());
    raw[36..44].copy_from_slice(&entry.expires_at_s.to_le_bytes());
    raw
}

fn decode_entry_prefix(raw: &[u8]) -> StoreResult<RuntimeLeaseEntry> {
    if raw.len() != LEASE_REPLICA_RECORD_PREFIX_SIZE || read_u16(raw, 10)? != 0 {
        return Err(corrupt(
            "runtime lease replica entry width or reserved field is invalid",
        ));
    }
    let token: [u8; LEASE_TOKEN_SIZE] = raw[12..28]
        .try_into()
        .expect("validated runtime lease token width");
    let entry = RuntimeLeaseEntry {
        key: RuntimeLeaseAttemptKey {
            repository_index: read_u32(raw, 0)?,
            worker_job_index: read_u32(raw, 4)?,
            attempt_count: read_u16(raw, 8)?,
        },
        token: RuntimeLeaseToken::from_bytes(token)?,
        heartbeat_at_s: read_u64(raw, 28)?,
        expires_at_s: read_u64(raw, 36)?,
    };
    validate_lease_entry(entry)?;
    Ok(entry)
}

fn validate_lease_entry(entry: RuntimeLeaseEntry) -> StoreResult<()> {
    if entry.key.attempt_count == 0
        || entry.heartbeat_at_s == 0
        || entry.expires_at_s <= entry.heartbeat_at_s
    {
        return Err(invalid("runtime lease replica entry is invalid"));
    }
    RuntimeLeaseToken::from_bytes(entry.token.0)?;
    Ok(())
}

fn entry_digest(prefix: &[u8]) -> [u8; LEASE_REPLICA_DIGEST_SIZE] {
    let mut digest = Sha256::new();
    digest.update(LEASE_REPLICA_ENTRY_DOMAIN);
    digest.update(prefix);
    digest.finalize().into()
}

fn validate_replica_location(
    replica_path: PathBuf,
    activated_roots: impl IntoIterator<Item = PathBuf>,
) -> StoreResult<(PathBuf, PathBuf, Vec<PathBuf>)> {
    let replica_path = absolute_path(replica_path)?;
    let file_name = replica_path
        .file_name()
        .ok_or_else(|| invalid("runtime lease replica has no filename"))?
        .to_os_string();
    let parent = replica_path
        .parent()
        .ok_or_else(|| invalid("runtime lease replica has no parent"))?;
    fs::create_dir_all(parent).map_err(|error| {
        BinaryDbError::io(
            format!("create runtime lease replica parent {}", parent.display()),
            error,
        )
    })?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        BinaryDbError::io(
            format!("canonicalize runtime lease parent {}", parent.display()),
            error,
        )
    })?;
    require_real_directory(&canonical_parent)?;
    let canonical_path = canonical_parent.join(file_name);
    reject_symlink_or_hardlink_if_present(&canonical_path)?;

    let mut canonical_roots = Vec::new();
    for root in activated_roots {
        let root = fs::canonicalize(absolute_path(root)?).map_err(|error| {
            BinaryDbError::io("canonicalize activated Binary authority root", error)
        })?;
        require_real_directory(&root)?;
        if canonical_path.starts_with(&root) {
            return Err(invalid(format!(
                "runtime lease replica {} is inside activated Binary authority root {}",
                canonical_path.display(),
                root.display()
            )));
        }
        canonical_roots.push(root);
    }
    canonical_roots.sort();
    canonical_roots.dedup();
    if canonical_roots.is_empty() {
        return Err(invalid(
            "runtime lease replica requires at least one activated authority exclusion root",
        ));
    }
    Ok((canonical_path, canonical_parent, canonical_roots))
}

fn validate_replica_still_outside_authority(
    replica_path: &Path,
    activated_roots: &[PathBuf],
) -> StoreResult<()> {
    let parent = replica_path
        .parent()
        .ok_or_else(|| invalid("runtime lease replica has no parent"))?;
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        BinaryDbError::io(
            format!("canonicalize runtime lease parent {}", parent.display()),
            error,
        )
    })?;
    let current = canonical_parent.join(
        replica_path
            .file_name()
            .ok_or_else(|| invalid("runtime lease replica has no filename"))?,
    );
    if activated_roots.iter().any(|root| current.starts_with(root)) {
        return Err(invalid(
            "runtime lease replica moved inside an activated Binary authority root",
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

fn read_u16(raw: &[u8], offset: usize) -> StoreResult<u16> {
    let bytes: [u8; 2] = raw
        .get(offset..offset + 2)
        .ok_or_else(|| corrupt("runtime lease replica u16 field is truncated"))?
        .try_into()
        .expect("validated u16 width");
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32(raw: &[u8], offset: usize) -> StoreResult<u32> {
    let bytes: [u8; 4] = raw
        .get(offset..offset + 4)
        .ok_or_else(|| corrupt("runtime lease replica u32 field is truncated"))?
        .try_into()
        .expect("validated u32 width");
    Ok(u32::from_le_bytes(bytes))
}

fn read_u64(raw: &[u8], offset: usize) -> StoreResult<u64> {
    let bytes: [u8; 8] = raw
        .get(offset..offset + 8)
        .ok_or_else(|| corrupt("runtime lease replica u64 field is truncated"))?
        .try_into()
        .expect("validated u64 width");
    Ok(u64::from_le_bytes(bytes))
}

fn constant_time_bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn hex_nibble(value: u8) -> char {
    match value {
        0..=9 => char::from(b'0' + value),
        10..=15 => char::from(b'a' + (value - 10)),
        _ => unreachable!("hex nibble is masked"),
    }
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
        WORKER_JOB_KIND_CONTENT_GC, WORKER_JOB_STATE_QUEUED,
    };
    use crate::foundation::server_operational_worker_jobs::{
        WorkerJobCreateSpec, WorkerJobDomainAuthority, WorkerJobEnqueueDisposition,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug, Default)]
    struct EmptyDomainAuthority;

    impl WorkerJobDomainAuthority for EmptyDomainAuthority {
        fn validate_patchset_index(&self, patchset_index: u32) -> StoreResult<()> {
            Err(invalid(format!(
                "unexpected test Patchset index {patchset_index}"
            )))
        }

        fn validate_snapshot_index(&self, snapshot_index: u32) -> StoreResult<()> {
            Err(invalid(format!(
                "unexpected test Snapshot index {snapshot_index}"
            )))
        }
    }

    struct LeaseFixture {
        root: PathBuf,
        global_root: PathBuf,
        repository_parent: PathBuf,
        replica_path: PathBuf,
        store: ServerOperationalWorkerJobStore,
        leases: ServerOperationalRuntimeLeases,
    }

    impl LeaseFixture {
        fn new(name: &str, repository_index: u32) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "ait-server-runtime-leases-{name}-{}-{nonce}",
                std::process::id()
            ));
            let global_root = root.join("authority-global");
            let repository_parent = root.join("authority-repositories");
            let authority_root = repository_parent.join(repository_index.to_string());
            let runtime_root = root.join("runtime");
            fs::create_dir_all(&global_root).unwrap();
            fs::create_dir_all(&authority_root).unwrap();
            fs::create_dir_all(&runtime_root).unwrap();
            let store = ServerOperationalWorkerJobStore::new(
                repository_index,
                authority_root,
                Arc::new(EmptyDomainAuthority),
            )
            .unwrap();
            store.initialize().unwrap();
            let replica_path = runtime_root.join("worker-leases.tmp.bin");
            let (leases, report) = ServerOperationalRuntimeLeases::open(
                &replica_path,
                [global_root.clone(), repository_parent.clone()],
            )
            .unwrap();
            assert_eq!(report, RuntimeLeaseReplicaOpenReport::default());
            Self {
                root,
                global_root,
                repository_parent,
                replica_path,
                store,
                leases,
            }
        }

        fn enqueue(&self, max_attempts: u16, available_at_s: u64) -> WorkerJobEntry {
            self.store
                .enqueue(
                    WorkerJobCreateSpec {
                        job_kind: WORKER_JOB_KIND_CONTENT_GC,
                        max_attempts,
                        patchset_index_plus1: 0,
                        snapshot_index_plus1: 0,
                        available_at_s,
                        created_at_s: available_at_s,
                    },
                    WorkerJobEnqueueDisposition::Queued,
                )
                .unwrap()
        }

        fn roots(&self) -> [PathBuf; 2] {
            [self.global_root.clone(), self.repository_parent.clone()]
        }
    }

    impl Drop for LeaseFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn claim_returns_random_attempt_token_and_restart_loads_only_the_replica() {
        let fixture = LeaseFixture::new("claim", 1);
        fixture.enqueue(3, 100);

        let grant = fixture.leases.claim(&fixture.store, 0, 100, 30).unwrap();
        assert_eq!(grant.worker_job.record.state_kind, WORKER_JOB_STATE_RUNNING);
        assert_eq!(grant.attempt_count, 1);
        assert_eq!(grant.heartbeat_at_s, 100);
        assert_eq!(grant.expires_at_s, 130);
        assert_eq!(grant.lease_token.to_hex().len(), 32);
        assert_eq!(
            RuntimeLeaseToken::parse_hex(&grant.lease_token.to_hex()).unwrap(),
            grant.lease_token
        );
        assert_eq!(
            format!("{:?}", grant.lease_token),
            "RuntimeLeaseToken([REDACTED])"
        );

        let raw = fs::read(&fixture.replica_path).unwrap();
        assert_eq!(&raw[..8], &LEASE_REPLICA_MAGIC);
        assert_ne!(&raw[..4], &1_u32.to_le_bytes());
        let (reopened, report) =
            ServerOperationalRuntimeLeases::open(&fixture.replica_path, fixture.roots()).unwrap();
        assert_eq!(report.loaded_entry_count, 1);
        assert!(!report.replica_discarded);
        assert_eq!(
            reopened
                .validate_presented(&fixture.store, 0, 1, grant.lease_token, 110)
                .unwrap()
                .expires_at_s,
            130
        );
    }

    #[test]
    fn heartbeat_exact_checks_token_and_persists_the_later_lock_time() {
        let fixture = LeaseFixture::new("heartbeat", 2);
        fixture.enqueue(2, 100);
        let grant = fixture.leases.claim(&fixture.store, 0, 100, 20).unwrap();
        let wrong = RuntimeLeaseToken::from_bytes([0xa5; LEASE_TOKEN_SIZE]).unwrap();

        assert!(fixture
            .leases
            .heartbeat(&fixture.store, 0, 1, wrong, 110, 20)
            .is_err());
        assert_eq!(fixture.store.get(0).unwrap().record.locked_at_s, 100);
        let heartbeat = fixture
            .leases
            .heartbeat(&fixture.store, 0, 1, grant.lease_token, 110, 20)
            .unwrap();
        assert_eq!(heartbeat.worker_job.record.locked_at_s, 110);
        assert_eq!(heartbeat.worker_job.record.updated_at_s, 110);
        assert_eq!(heartbeat.expires_at_s, 130);
        assert!(fixture
            .leases
            .heartbeat(&fixture.store, 0, 2, grant.lease_token, 111, 20)
            .is_err());

        let (reopened, _) =
            ServerOperationalRuntimeLeases::open(&fixture.replica_path, fixture.roots()).unwrap();
        assert_eq!(reopened.entries().unwrap()[0].heartbeat_at_s, 110);
    }

    #[cfg(unix)]
    #[test]
    fn live_reconciliation_does_not_run_recovery_grade_index_rebuilds() {
        use std::os::unix::fs::MetadataExt;

        let fixture = LeaseFixture::new("steady-reconcile", 14);
        fixture.enqueue(2, 100);
        fixture.leases.claim(&fixture.store, 0, 100, 30).unwrap();
        let ready_path = fixture.store.authority_root().join("worker_ready.idx");
        let state_path = fixture.store.authority_root().join("worker_state.idx");
        let ready_inode = fs::metadata(&ready_path).unwrap().ino();
        let state_inode = fs::metadata(&state_path).unwrap().ino();

        let report = fixture
            .leases
            .reconcile(std::slice::from_ref(&fixture.store), 101, 10)
            .unwrap();
        assert_eq!(report.valid_entry_count, 1);
        assert_eq!(report.discarded_entry_count, 0);
        assert_eq!(report.requeued_job_count, 0);
        assert_eq!(report.failed_job_count, 0);
        assert_eq!(fs::metadata(ready_path).unwrap().ino(), ready_inode);
        assert_eq!(fs::metadata(state_path).unwrap().ino(), state_inode);
    }

    #[test]
    fn domain_first_completion_invalidates_lease_and_stale_token_cannot_revive_job() {
        let fixture = LeaseFixture::new("complete", 3);
        fixture.enqueue(2, 100);
        let grant = fixture.leases.claim(&fixture.store, 0, 100, 30).unwrap();
        let wrong = RuntimeLeaseToken::from_bytes([0x5a; LEASE_TOKEN_SIZE]).unwrap();
        assert!(fixture
            .leases
            .complete_after_domain_commit(
                &fixture.store,
                0,
                1,
                wrong,
                WORKER_JOB_OUTCOME_COMPLETED,
                105,
            )
            .is_err());

        let completed = fixture
            .leases
            .complete_after_domain_commit(
                &fixture.store,
                0,
                1,
                grant.lease_token,
                WORKER_JOB_OUTCOME_COMPLETED,
                105,
            )
            .unwrap();
        assert_eq!(completed.record.state_kind, WORKER_JOB_STATE_SUCCEEDED);
        assert_eq!(completed.record.locked_at_s, 0);
        assert!(fixture.leases.entries().unwrap().is_empty());
        assert!(fixture
            .leases
            .validate_presented(&fixture.store, 0, 1, grant.lease_token, 106)
            .is_err());
        let (reopened, report) =
            ServerOperationalRuntimeLeases::open(&fixture.replica_path, fixture.roots()).unwrap();
        assert_eq!(report.loaded_entry_count, 0);
        assert!(reopened.entries().unwrap().is_empty());
    }

    #[test]
    fn explicit_failure_can_retry_then_terminally_fail_with_attempt_binding() {
        let fixture = LeaseFixture::new("fail", 4);
        fixture.enqueue(2, 100);
        let first = fixture.leases.claim(&fixture.store, 0, 100, 30).unwrap();
        let retried = fixture
            .leases
            .fail_attempt(
                &fixture.store,
                0,
                1,
                first.lease_token,
                WORKER_JOB_ERROR_RETRYABLE_EXECUTION,
                Some(110),
                105,
            )
            .unwrap();
        assert_eq!(retried.record.state_kind, WORKER_JOB_STATE_QUEUED);
        assert_eq!(retried.record.available_at_s, 110);
        assert!(fixture.leases.entries().unwrap().is_empty());

        let second = fixture.leases.claim(&fixture.store, 0, 110, 30).unwrap();
        assert_eq!(second.attempt_count, 2);
        let failed = fixture
            .leases
            .fail_attempt(
                &fixture.store,
                0,
                2,
                second.lease_token,
                WORKER_JOB_ERROR_TERMINAL_EXECUTION,
                None,
                111,
            )
            .unwrap();
        assert_eq!(failed.record.state_kind, WORKER_JOB_STATE_FAILED);
        assert_eq!(failed.record.outcome_kind, WORKER_JOB_OUTCOME_FAILED);
        assert_eq!(failed.record.locked_at_s, 0);
    }

    #[test]
    fn expiry_requeues_remaining_budget_and_fails_exhausted_attempts() {
        let fixture = LeaseFixture::new("expiry", 5);
        fixture.enqueue(2, 100);
        fixture.enqueue(1, 100);
        fixture.leases.claim(&fixture.store, 0, 100, 5).unwrap();
        fixture.leases.claim(&fixture.store, 1, 100, 5).unwrap();

        let report = fixture
            .leases
            .reconcile(std::slice::from_ref(&fixture.store), 106, 10)
            .unwrap();
        assert_eq!(report.valid_entry_count, 0);
        assert_eq!(report.discarded_entry_count, 2);
        assert_eq!(report.requeued_job_count, 1);
        assert_eq!(report.failed_job_count, 1);
        let requeued = fixture.store.get(0).unwrap();
        assert_eq!(requeued.record.state_kind, WORKER_JOB_STATE_QUEUED);
        assert_eq!(requeued.record.error_kind, WORKER_JOB_ERROR_LEASE_EXPIRED);
        assert_eq!(requeued.record.available_at_s, 116);
        let failed = fixture.store.get(1).unwrap();
        assert_eq!(failed.record.state_kind, WORKER_JOB_STATE_FAILED);
        assert_eq!(failed.record.error_kind, WORKER_JOB_ERROR_LEASE_EXPIRED);
        assert!(fixture.leases.entries().unwrap().is_empty());
    }

    #[test]
    fn missing_or_torn_restart_replica_cannot_preserve_a_running_job() {
        let fixture = LeaseFixture::new("restart-loss", 6);
        fixture.enqueue(2, 100);
        fixture.leases.claim(&fixture.store, 0, 100, 30).unwrap();
        fs::remove_file(&fixture.replica_path).unwrap();
        let (reopened, report) =
            ServerOperationalRuntimeLeases::open(&fixture.replica_path, fixture.roots()).unwrap();
        assert!(!report.replica_existed);
        let recovery = reopened
            .reconcile(std::slice::from_ref(&fixture.store), 105, 5)
            .unwrap();
        assert_eq!(recovery.requeued_job_count, 1);
        assert_eq!(fixture.store.get(0).unwrap().record.locked_at_s, 0);

        let second = reopened.claim(&fixture.store, 0, 110, 30).unwrap();
        let mut raw = fs::read(&fixture.replica_path).unwrap();
        *raw.last_mut().unwrap() ^= 0xff;
        fs::write(&fixture.replica_path, raw).unwrap();
        let (torn, torn_report) =
            ServerOperationalRuntimeLeases::open(&fixture.replica_path, fixture.roots()).unwrap();
        assert_eq!(torn_report.discarded_entry_count, 1);
        assert!(torn
            .validate_presented(&fixture.store, 0, 2, second.lease_token, 111)
            .is_err());
        assert_eq!(
            torn.reconcile(std::slice::from_ref(&fixture.store), 111, 5)
                .unwrap()
                .failed_job_count,
            1
        );
    }

    #[test]
    fn precommit_or_mismatched_replica_entry_never_makes_a_queued_job_running() {
        let fixture = LeaseFixture::new("precommit", 7);
        fixture.enqueue(2, 100);
        let stale = RuntimeLeaseEntry {
            key: RuntimeLeaseAttemptKey {
                repository_index: 7,
                worker_job_index: 0,
                attempt_count: 1,
            },
            token: RuntimeLeaseToken::from_bytes([7; LEASE_TOKEN_SIZE]).unwrap(),
            heartbeat_at_s: 100,
            expires_at_s: 130,
        };
        let mut entries = BTreeMap::new();
        entries.insert(stale.key, stale);
        fs::write(&fixture.replica_path, encode_replica(&entries).unwrap()).unwrap();
        let (reopened, report) =
            ServerOperationalRuntimeLeases::open(&fixture.replica_path, fixture.roots()).unwrap();
        assert_eq!(report.loaded_entry_count, 1);
        let recovery = reopened
            .reconcile(std::slice::from_ref(&fixture.store), 105, 5)
            .unwrap();
        assert_eq!(recovery.discarded_entry_count, 1);
        assert_eq!(recovery.requeued_job_count, 0);
        assert_eq!(
            fixture.store.get(0).unwrap().record.state_kind,
            WORKER_JOB_STATE_QUEUED
        );
        assert_eq!(fixture.store.get(0).unwrap().record.attempt_count, 0);
    }

    #[test]
    fn shared_replica_serializes_claims_without_a_global_job_identity() {
        let first = LeaseFixture::new("cross-repo-base", 8);
        first.enqueue(2, 100);
        let second_root = first.repository_parent.join("9");
        fs::create_dir_all(&second_root).unwrap();
        let second_store =
            ServerOperationalWorkerJobStore::new(9, second_root, Arc::new(EmptyDomainAuthority))
                .unwrap();
        second_store.initialize().unwrap();
        second_store
            .enqueue(
                WorkerJobCreateSpec {
                    job_kind: WORKER_JOB_KIND_CONTENT_GC,
                    max_attempts: 2,
                    patchset_index_plus1: 0,
                    snapshot_index_plus1: 0,
                    available_at_s: 100,
                    created_at_s: 100,
                },
                WorkerJobEnqueueDisposition::Queued,
            )
            .unwrap();
        let first_leases = first.leases.clone();
        let first_store = first.store.clone();
        let second_leases = first.leases.clone();
        let second_store_for_thread = second_store.clone();
        let first_claim =
            std::thread::spawn(move || first_leases.claim(&first_store, 0, 100, 30).unwrap());
        let second_claim = std::thread::spawn(move || {
            second_leases
                .claim(&second_store_for_thread, 0, 100, 30)
                .unwrap()
        });
        let first_grant = first_claim.join().unwrap();
        let second_grant = second_claim.join().unwrap();
        assert_eq!(first_grant.worker_job.key.repository_index, 8);
        assert_eq!(second_grant.worker_job.key.repository_index, 9);
        assert_eq!(first.leases.entries().unwrap().len(), 2);
        assert_ne!(first_grant.lease_token, second_grant.lease_token);
    }

    #[test]
    fn replica_location_is_fail_closed_outside_all_authority_roots() {
        let fixture = LeaseFixture::new("boundary", 10);
        let forbidden = fixture.store.authority_root().join("lease-replica.bin");
        assert!(ServerOperationalRuntimeLeases::open(&forbidden, fixture.roots()).is_err());
        assert!(!forbidden.exists());
        assert!(ServerOperationalRuntimeLeases::open(
            fixture.root.join("runtime/no-roots.bin"),
            Vec::<PathBuf>::new(),
        )
        .is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;

            let target = fixture.root.join("runtime/real.bin");
            fs::write(&target, []).unwrap();
            let alias = fixture.root.join("runtime/alias.bin");
            symlink(&target, &alias).unwrap();
            assert!(ServerOperationalRuntimeLeases::open(&alias, fixture.roots()).is_err());
        }
    }
}
