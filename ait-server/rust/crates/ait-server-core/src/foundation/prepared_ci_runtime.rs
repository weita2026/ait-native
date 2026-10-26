//! Temporary execution state. No data in this module is Binary DB authority.
use crate::foundation::remote_binary_db::{BinaryDbError, StoreResult};
use crate::foundation::server_operational_job_domain::WorkerJobCompactCiEvidence;
use crate::foundation::server_operational_worker_jobs::WorkerJobKey;
use crate::foundation::workflow_binary_v0::{
    CI_STATUS_ERROR, CI_STATUS_FAIL, CI_STATUS_NONE, CI_STATUS_PASS,
};
use crate::foundation::workflow_package_contract::{
    PackageAbandonRequest, PackageEvidenceOperation, PackageEvidenceRequest,
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

mod governance;
pub(crate) use governance::fold_package_reviews;
mod empty_cleanup;
#[cfg(unix)]
pub use empty_cleanup::open_owned_managed_runtime_root;
pub use empty_cleanup::remove_empty_managed_runtime_container;
#[cfg(windows)]
mod windows_fs;
#[cfg(windows)]
pub use windows_fs::{verify_managed_runtime_source, WindowsManagedSource};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PreparedCiTarget {
    pub target_line: String,
    pub base_snapshot_id: String,
    pub revision_snapshot_id: String,
}

pub struct ManagedCiPackagePaths {
    pub managed_root: PathBuf,
    pub worktree: PathBuf,
    pub sources: PathBuf,
    pub package: PathBuf,
}

#[derive(Clone, Debug)]
pub struct ManagedCiPackage {
    repository_index: u32,
    worktree: PathBuf,
    sources: PathBuf,
    package: PathBuf,
    sha256: [u8; 32],
    target: PreparedCiTarget,
    downloads: Arc<PackageDownloads>,
}

#[derive(Debug, Default)]
struct PackageDownloads {
    readers: AtomicUsize,
    cleanup_requested: AtomicBool,
    child_removed: Mutex<bool>,
}

impl PartialEq for ManagedCiPackage {
    fn eq(&self, other: &Self) -> bool {
        self.repository_index == other.repository_index
            && self.worktree == other.worktree
            && self.sources == other.sources
            && self.package == other.package
            && self.sha256 == other.sha256
            && self.target == other.target
    }
}
impl Eq for ManagedCiPackage {}

pub(crate) struct ManagedPackageDownload(ManagedCiPackage);

impl ManagedPackageDownload {
    pub(crate) fn read(&self) -> StoreResult<Option<Vec<u8>>> {
        let source = &self.0;
        if !source
            .package
            .try_exists()
            .map_err(|error| BinaryDbError::io("inspect managed package availability", error))?
            || !source
                .sources
                .try_exists()
                .map_err(|error| BinaryDbError::io("inspect managed source availability", error))?
        {
            return Ok(None);
        }
        source.verify()?;
        let bytes = fs::read(&source.package)
            .map_err(|error| BinaryDbError::io("read original managed package", error))?;
        if <[u8; 32]>::from(Sha256::digest(&bytes)) != source.sha256 {
            return Err(invalid("Managed package changed during download"));
        }
        Ok(Some(bytes))
    }
}

impl Drop for ManagedPackageDownload {
    fn drop(&mut self) {
        if self.0.downloads.readers.fetch_sub(1, Ordering::SeqCst) == 1
            && self.0.downloads.cleanup_requested.load(Ordering::SeqCst)
        {
            if let Err(error) = self.0.request_cleanup() {
                eprintln!("warning: deferred package cleanup failed: {error}");
            }
        }
    }
}

impl ManagedCiPackage {
    pub fn open(
        repository_index: u32,
        paths: ManagedCiPackagePaths,
        sha256: [u8; 32],
        target: PreparedCiTarget,
    ) -> StoreResult<Self> {
        let managed_root = real_directory(&paths.managed_root)?;
        let worktree = real_directory(&paths.worktree)?;
        let sources = real_directory(&paths.sources)?;
        let package = real_file(&paths.package)?;
        if worktree == managed_root
            || !worktree.starts_with(&managed_root)
            || sources == worktree
            || !sources.starts_with(&worktree)
            || !package.starts_with(&worktree)
            || package.starts_with(&sources)
        {
            return Err(invalid("prepared CI package and source paths must be separate children of a managed worktree"));
        }
        if target.target_line.is_empty()
            || target.base_snapshot_id.is_empty()
            || target.revision_snapshot_id.is_empty()
        {
            return Err(invalid("prepared CI target identity is incomplete"));
        }
        let source = Self {
            repository_index,
            worktree,
            sources,
            package,
            sha256,
            target,
            downloads: Arc::new(PackageDownloads::default()),
        };
        source.verify()?;
        Ok(source)
    }

    pub fn repository_index(&self) -> u32 {
        self.repository_index
    }
    pub fn worktree(&self) -> &Path {
        &self.worktree
    }
    pub fn sources(&self) -> &Path {
        &self.sources
    }
    pub fn package(&self) -> &Path {
        &self.package
    }
    pub fn sha256(&self) -> [u8; 32] {
        self.sha256
    }
    pub fn target(&self) -> &PreparedCiTarget {
        &self.target
    }

    /// Called under Queue ownership before releasing its short validation lock.
    pub(crate) fn pin_download(&self) -> StoreResult<ManagedPackageDownload> {
        if self.downloads.cleanup_requested.load(Ordering::SeqCst) {
            return Err(invalid("Package source cleanup has started"));
        }
        self.downloads.readers.fetch_add(1, Ordering::SeqCst);
        Ok(ManagedPackageDownload(self.clone()))
    }

    /// The caller fences new execution/downloads first. Active copies defer
    /// deletion without holding Queue, runtime, or lease locks during file I/O.
    pub fn request_cleanup(&self) -> StoreResult<bool> {
        self.downloads
            .cleanup_requested
            .store(true, Ordering::SeqCst);
        if self.downloads.readers.load(Ordering::SeqCst) != 0 {
            return Ok(false);
        }
        // Serialize the final reader's deferred deletion with a cleanup retry.
        // Removing the old child never grants ownership of a recreated path.
        let mut removed = self
            .downloads
            .child_removed
            .lock()
            .map_err(|_| invalid("Package child cleanup lock is poisoned"))?;
        match fs::symlink_metadata(&self.worktree) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                *removed = true;
                Ok(true)
            }
            Err(error) => Err(BinaryDbError::io("inspect package cleanup root", error)),
            Ok(_) if *removed => Err(invalid(
                "Package Job path was recreated after owned child cleanup",
            )),
            Ok(metadata)
                if metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && real_directory(&self.worktree)? == self.worktree =>
            {
                match fs::remove_dir_all(&self.worktree) {
                    Ok(()) => {
                        *removed = true;
                        Ok(true)
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        *removed = true;
                        Ok(true)
                    }
                    Err(error) => Err(BinaryDbError::io("remove managed package source", error)),
                }
            }
            _ => Err(invalid(
                "Package cleanup root is not its original real directory",
            )),
        }
    }

    pub fn expected_identity(
        &self,
        key: WorkerJobKey,
    ) -> Result<crate::foundation::workflow_package_contract::PackageExpectedIdentity, String> {
        if key.repository_index != self.repository_index {
            return Err("Package Job belongs to another Repository".to_string());
        }
        let identity = crate::foundation::workflow_package_contract::PackageExpectedIdentity {
            repository_index: key.repository_index,
            package_hash: self
                .sha256
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            worker_job_index: key.worker_job_index,
            base_snapshot_id: self.target.base_snapshot_id.clone(),
            revision_snapshot_id: self.target.revision_snapshot_id.clone(),
        };
        identity.validate()?;
        Ok(identity)
    }

    pub fn verify(&self) -> StoreResult<()> {
        if real_directory(&self.worktree)? != self.worktree
            || real_directory(&self.sources)? != self.sources
            || real_file(&self.package)? != self.package
        {
            return Err(invalid("prepared CI source paths changed"));
        }
        let mut input = fs::File::open(&self.package)
            .map_err(|error| BinaryDbError::io("open managed CI package", error))?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = input
                .read(&mut buffer)
                .map_err(|error| BinaryDbError::io("read managed CI package", error))?;
            if count == 0 {
                break;
            }
            digest.update(&buffer[..count]);
        }
        if <[u8; 32]>::from(digest.finalize()) != self.sha256 {
            return Err(invalid("managed CI package hash changed"));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct PreparedCiExecution {
    source: ManagedCiPackage,
    evidence: Option<WorkerJobCompactCiEvidence>,
    success_order: Option<u64>,
    released: bool,
    actions: Vec<PreparedPackageEvidence>,
    abandonment: Option<PreparedAbandonment>,
    committed: bool,
    committed_result: Option<serde_json::Value>,
    cleanup_complete: bool,
    reprepare_origins: BTreeSet<WorkerJobKey>,
}

#[derive(Clone)]
struct PreparedAbandonment {
    request: PackageAbandonRequest,
    stopped: bool,
    cleaned: bool,
}

#[derive(Clone, Copy, Debug, serde::Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PackageAbandonState {
    Abandoning,
    Abandoned,
    Unknown,
}

#[derive(Clone, Debug)]
pub struct PreparedPackageEvidence {
    pub request: PackageEvidenceRequest,
    pub actor_index_plus1: u32,
    pub recorded_at_s: u64,
}

pub fn prepared_ci_passed(evidence: WorkerJobCompactCiEvidence) -> bool {
    evidence.overall_status == CI_STATUS_PASS
        && evidence.tests_status == CI_STATUS_PASS
        && evidence.selected_suite_count > 0
        && evidence.suite_result_count > 0
        && evidence.blocking_failure_count == 0
}

pub fn prepared_ci_state(evidence: Option<WorkerJobCompactCiEvidence>) -> &'static str {
    let Some(evidence) = evidence else {
        return "unknown";
    };
    if matches!(evidence.overall_status, CI_STATUS_FAIL | CI_STATUS_ERROR)
        || matches!(evidence.tests_status, CI_STATUS_FAIL | CI_STATUS_ERROR)
        || evidence.blocking_failure_count > 0
    {
        return "failed";
    }
    if evidence.ci_completed_at_s == 0
        || evidence.ci_run_seq == 0
        || evidence.selected_suite_count == 0
        || evidence.suite_result_count == 0
        || evidence.overall_status == CI_STATUS_NONE
        || evidence.tests_status == CI_STATUS_NONE
    {
        return "unknown";
    }
    if prepared_ci_passed(evidence) {
        "passed"
    } else {
        "failed"
    }
}

impl PreparedCiExecution {
    fn review_released(&self) -> bool {
        fold_package_reviews(&self.actions).has_objection()
    }
}

#[derive(Default)]
struct PreparedCiState {
    executions: BTreeMap<WorkerJobKey, PreparedCiExecution>,
    selected: BTreeMap<(u32, [u8; 32]), WorkerJobKey>,
    next_success_order: u64,
}

#[derive(Default)]
pub struct PreparedCiRuntime {
    state: Mutex<PreparedCiState>,
}

impl PreparedCiRuntime {
    pub fn cancel_requested(&self, key: WorkerJobKey) -> StoreResult<bool> {
        Ok(self
            .lock()?
            .executions
            .get(&key)
            .is_some_and(|execution| execution.released))
    }

    pub(crate) fn request_abandonment(
        &self,
        key: WorkerJobKey,
        request: &PackageAbandonRequest,
        actor_index_plus1: u32,
    ) -> StoreResult<PackageAbandonState> {
        if request.expected_identity.repository_index != key.repository_index
            || request.expected_identity.worker_job_index != key.worker_job_index
            || request.actor_key_index.checked_add(1) != Some(actor_index_plus1)
        {
            return Err(invalid("Package abandonment path or Actor differs"));
        }
        let mut state = self.lock()?;
        let Some(execution) = state.executions.get(&key) else {
            return Ok(PackageAbandonState::Unknown);
        };
        if let Some(previous) = &execution.abandonment {
            if previous.request != *request {
                return Err(invalid("Package abandonment request changed during retry"));
            }
            return Ok(if previous.cleaned {
                PackageAbandonState::Abandoned
            } else {
                PackageAbandonState::Abandoning
            });
        }
        if execution
            .source
            .expected_identity(key)
            .map_err(|error| invalid(&error))?
            != request.expected_identity
            || state
                .selected
                .get(&(key.repository_index, execution.source.sha256))
                != Some(&key)
        {
            return Err(invalid(
                "Package abandonment differs from the exact selected Job",
            ));
        }
        execution.source.verify()?;
        let execution = state
            .executions
            .get_mut(&key)
            .expect("execution stays locked");
        execution.released = true;
        execution.abandonment = Some(PreparedAbandonment {
            request: request.clone(),
            stopped: false,
            cleaned: false,
        });
        Ok(PackageAbandonState::Abandoning)
    }

    pub(crate) fn confirm_abandoned_stop(&self, key: WorkerJobKey) -> StoreResult<()> {
        if let Some(abandonment) = self
            .lock()?
            .executions
            .get_mut(&key)
            .and_then(|execution| execution.abandonment.as_mut())
        {
            abandonment.stopped = true;
        }
        Ok(())
    }

    pub(crate) fn cleanup_abandoned_source(
        &self,
        key: WorkerJobKey,
        cleanup: impl FnOnce(&ManagedCiPackage) -> bool,
    ) -> StoreResult<PackageAbandonState> {
        let mut state = self.lock()?;
        let Some(execution) = state.executions.get_mut(&key) else {
            return Ok(PackageAbandonState::Unknown);
        };
        let Some(abandonment) = execution.abandonment.as_mut() else {
            return Ok(PackageAbandonState::Unknown);
        };
        if abandonment.cleaned {
            return Ok(PackageAbandonState::Abandoned);
        }
        if abandonment.stopped && cleanup(&execution.source) {
            abandonment.cleaned = true;
        }
        Ok(if abandonment.cleaned {
            PackageAbandonState::Abandoned
        } else {
            PackageAbandonState::Abandoning
        })
    }

    pub fn selected_job(
        &self,
        repository_index: u32,
        hash: [u8; 32],
    ) -> StoreResult<Option<WorkerJobKey>> {
        Ok(self
            .lock()?
            .selected
            .get(&(repository_index, hash))
            .copied())
    }

    pub(crate) fn register(&self, key: WorkerJobKey, source: ManagedCiPackage) -> StoreResult<()> {
        if key.repository_index != source.repository_index {
            return Err(invalid("prepared CI Repository differs"));
        }
        let mut state = self.lock()?;
        if state.executions.contains_key(&key) {
            return Err(invalid("prepared CI Job is already associated"));
        }
        state
            .selected
            .insert((key.repository_index, source.sha256), key);
        state.executions.insert(
            key,
            PreparedCiExecution {
                source,
                evidence: None,
                success_order: None,
                released: false,
                actions: Vec::new(),
                abandonment: None,
                committed: false,
                committed_result: None,
                cleanup_complete: false,
                reprepare_origins: BTreeSet::new(),
            },
        );
        Ok(())
    }

    pub fn source(&self, key: WorkerJobKey) -> StoreResult<ManagedCiPackage> {
        self.lock()?
            .executions
            .get(&key)
            .map(|execution| execution.source.clone())
            .ok_or_else(|| {
                invalid("prepared CI Job has no recoverable managed package association")
            })
    }

    /// Retry context established only after Server verifies the original retained
    /// source and its replacement. This dies with the existing execution RAM.
    pub fn remember_reprepare_origin(
        &self,
        key: WorkerJobKey,
        origin: WorkerJobKey,
    ) -> StoreResult<()> {
        let mut state = self.lock()?;
        let execution = state
            .executions
            .get(&key)
            .ok_or_else(|| invalid("Replacement execution is unavailable"))?;
        if key == origin
            || key.repository_index != origin.repository_index
            || execution.released
            || state
                .selected
                .get(&(key.repository_index, execution.source.sha256))
                != Some(&key)
        {
            return Err(invalid(
                "Replacement execution is not the unique selected source",
            ));
        }
        state
            .executions
            .get_mut(&key)
            .expect("checked replacement")
            .reprepare_origins
            .insert(origin);
        Ok(())
    }

    pub fn is_reprepared_from(&self, key: WorkerJobKey, origin: WorkerJobKey) -> StoreResult<bool> {
        Ok(self
            .lock()?
            .executions
            .get(&key)
            .is_some_and(|execution| execution.reprepare_origins.contains(&origin)))
    }

    pub(crate) fn require_selected(&self, key: WorkerJobKey) -> StoreResult<ManagedCiPackage> {
        let state = self.lock()?;
        let execution = state
            .executions
            .get(&key)
            .ok_or_else(|| invalid("prepared CI Job has no managed package association"))?;
        if state
            .selected
            .get(&(key.repository_index, execution.source.sha256))
            != Some(&key)
            || execution.released
        {
            return Err(invalid(
                "prepared CI Job is superseded, rejected or abandoned",
            ));
        }
        Ok(execution.source.clone())
    }

    pub(crate) fn record_result(
        &self,
        key: WorkerJobKey,
        evidence: WorkerJobCompactCiEvidence,
    ) -> StoreResult<()> {
        evidence.status_bits()?;
        let mut state = self.lock()?;
        let execution = state
            .executions
            .get(&key)
            .ok_or_else(|| invalid("prepared CI completion has no managed source"))?;
        if state
            .selected
            .get(&(key.repository_index, execution.source.sha256))
            != Some(&key)
            || execution.released
        {
            return Err(invalid(
                "prepared CI completion belongs to a superseded Job",
            ));
        }
        if let Some(previous) = execution.evidence {
            // A retry after the domain callback but before the Queue update
            // keeps the original server completion time and success order.
            return if previous
                == (WorkerJobCompactCiEvidence {
                    ci_completed_at_s: previous.ci_completed_at_s,
                    ..evidence
                }) {
                Ok(())
            } else {
                Err(invalid("prepared CI result is immutable"))
            };
        }
        let success_order = if prepared_ci_passed(evidence) {
            state.next_success_order = state
                .next_success_order
                .checked_add(1)
                .ok_or_else(|| invalid("prepared CI success order exhausted"))?;
            Some(state.next_success_order)
        } else {
            None
        };
        let execution = state
            .executions
            .get_mut(&key)
            .expect("selected execution remains present while locked");
        execution.evidence = Some(evidence);
        execution.success_order = success_order;
        Ok(())
    }

    pub fn evidence(&self, key: WorkerJobKey) -> StoreResult<Option<WorkerJobCompactCiEvidence>> {
        self.lock()?
            .executions
            .get(&key)
            .map(|execution| execution.evidence)
            .ok_or_else(|| invalid("prepared CI Job is unknown"))
    }

    pub(crate) fn execution_hashes(
        &self,
        repository_index: u32,
    ) -> StoreResult<Vec<(WorkerJobKey, [u8; 32])>> {
        Ok(self
            .lock()?
            .executions
            .iter()
            .filter(|(key, _)| key.repository_index == repository_index)
            .map(|(key, execution)| (*key, execution.source.sha256))
            .collect())
    }

    pub fn recoverable_selection(&self, key: WorkerJobKey) -> bool {
        self.require_selected(key).is_ok()
    }

    /// The caller validates the live Actor and existing review/attestation rules
    /// before invoking this transient, timestamp-preserving acceptance path.
    pub fn accept_evidence(
        &self,
        key: WorkerJobKey,
        request: PackageEvidenceRequest,
        actor_index_plus1: u32,
        now_s: u64,
    ) -> StoreResult<PreparedPackageEvidence> {
        self.accept_evidence_with_capacity(key, request, actor_index_plus1, now_s, || Ok(64))
    }

    pub fn accept_evidence_with_capacity(
        &self,
        key: WorkerJobKey,
        request: PackageEvidenceRequest,
        actor_index_plus1: u32,
        now_s: u64,
        available_attestations: impl FnOnce() -> StoreResult<usize>,
    ) -> StoreResult<PreparedPackageEvidence> {
        if request.expected_identity.repository_index != key.repository_index
            || request.expected_identity.worker_job_index != key.worker_job_index
            || actor_index_plus1
                != request
                    .actor_key_index
                    .checked_add(1)
                    .ok_or_else(|| invalid("Actor index exceeds plus-one bounds"))?
            || now_s == 0
        {
            return Err(invalid(
                "Package evidence path, Actor or acceptance time differs",
            ));
        }
        let mut state = self.lock()?;
        let execution = state
            .executions
            .get(&key)
            .ok_or_else(|| invalid("Package evidence has no managed execution"))?;
        if state
            .selected
            .get(&(key.repository_index, execution.source.sha256))
            != Some(&key)
            || execution.released
            || !execution.evidence.is_some_and(prepared_ci_passed)
            || execution
                .source
                .expected_identity(key)
                .map_err(|error| invalid(&error))?
                != request.expected_identity
        {
            return Err(invalid(
                "Package evidence requires the exact selected passed execution",
            ));
        }
        execution.source.verify()?;
        for previous in &execution.actions {
            if previous.request.operation == request.operation
                && previous.actor_index_plus1 == actor_index_plus1
                && previous.request.idempotency_key == request.idempotency_key
            {
                return if previous.request == request {
                    Ok(previous.clone())
                } else {
                    Err(invalid(
                        "Package evidence idempotency key was reused with a different body",
                    ))
                };
            }
        }
        let capacity = match request.operation {
            PackageEvidenceOperation::Review => 64,
            PackageEvidenceOperation::Attestation => available_attestations()?.min(64),
        };
        if execution
            .actions
            .iter()
            .filter(|event| event.request.operation == request.operation)
            .count()
            >= capacity
        {
            return Err(invalid(
                "Package evidence exceeds its existing 64-event formal capacity",
            ));
        }
        let event = PreparedPackageEvidence {
            request,
            actor_index_plus1,
            recorded_at_s: now_s,
        };
        state
            .executions
            .get_mut(&key)
            .expect("execution remains locked")
            .actions
            .push(event.clone());
        Ok(event)
    }

    /// Rejection and stopped abandonment release priority without changing head.
    pub fn release_priority(&self, key: WorkerJobKey, hash: [u8; 32]) -> StoreResult<()> {
        let mut state = self.lock()?;
        let execution = state
            .executions
            .get_mut(&key)
            .ok_or_else(|| invalid("prepared CI Job is unknown"))?;
        if execution.source.sha256 != hash {
            return Err(invalid("prepared CI approval hash differs"));
        }
        execution.released = true;
        Ok(())
    }

    pub fn winner(
        &self,
        repository_index: u32,
        target_line: &str,
        base_snapshot_id: &str,
    ) -> StoreResult<Option<WorkerJobKey>> {
        let state = self.lock()?;
        Ok(state
            .executions
            .iter()
            .filter(|(key, execution)| {
                key.repository_index == repository_index
                    && !execution.released
                    && !execution.review_released()
                    && execution.source.target.target_line == target_line
                    && execution.source.target.base_snapshot_id == base_snapshot_id
                    && execution.success_order.is_some()
                    && state
                        .selected
                        .get(&(key.repository_index, execution.source.sha256))
                        == Some(key)
            })
            .min_by_key(|(_, execution)| execution.success_order)
            .map(|(key, _)| *key))
    }

    pub(crate) fn with_finish_candidate<T>(
        &self,
        key: WorkerJobKey,
        hash: [u8; 32],
        operation: impl FnOnce(
            &ManagedCiPackage,
            WorkerJobCompactCiEvidence,
            &[PreparedPackageEvidence],
        ) -> StoreResult<T>,
    ) -> StoreResult<T> {
        // Hold this lock through the domain commit so rejection, abandonment,
        // rerun selection and CI-success priority cannot race acceptance.
        let mut state = self.lock()?;
        let execution = state
            .executions
            .get(&key)
            .ok_or_else(|| invalid("prepared finish has no recoverable execution"))?;
        let source = execution.source.clone();
        if execution.review_released() {
            return Err(invalid("PACKAGE_REVIEW_BLOCKED"));
        }
        if source.sha256 != hash
            || execution.released
            || state.selected.get(&(key.repository_index, hash)) != Some(&key)
            || execution.success_order.is_none()
        {
            return Err(invalid(
                "prepared finish identity is not the selected passed Job",
            ));
        }
        let winner = state
            .executions
            .iter()
            .filter(|(candidate, execution)| {
                candidate.repository_index == key.repository_index
                    && !execution.released
                    && !execution.review_released()
                    && execution.source.target.target_line == source.target.target_line
                    && execution.source.target.base_snapshot_id == source.target.base_snapshot_id
                    && execution.success_order.is_some()
                    && state
                        .selected
                        .get(&(candidate.repository_index, execution.source.sha256))
                        == Some(candidate)
            })
            .min_by_key(|(_, execution)| execution.success_order)
            .map(|(key, _)| *key);
        if winner != Some(key) {
            return Err(invalid(
                "prepared finish is waiting for the earlier passed candidate",
            ));
        }
        let evidence = execution
            .evidence
            .ok_or_else(|| invalid("prepared finish has no CI result"))?;
        let result = operation(&source, evidence, &execution.actions)?;
        // Only committed acceptance releases peers on this base. CI pass alone
        // leaves them eligible, and a failed operation changes no priority.
        for (candidate, execution) in &mut state.executions {
            if candidate.repository_index == key.repository_index
                && execution.source.target.target_line == source.target.target_line
                && execution.source.target.base_snapshot_id == source.target.base_snapshot_id
            {
                execution.released = true;
            }
        }
        state
            .executions
            .get_mut(&key)
            .expect("committed execution")
            .committed = true;
        state.selected.remove(&(key.repository_index, hash));
        Ok(result)
    }

    /// Observe only this process's committed owner. Retained files alone never
    /// reconstruct authority after RAM loss, nor grant broad orphan deletion.
    pub fn committed_source(&self, key: WorkerJobKey) -> StoreResult<Option<ManagedCiPackage>> {
        Ok(self.lock()?.executions.get(&key).and_then(|execution| {
            (execution.committed && execution.released).then(|| execution.source.clone())
        }))
    }

    /// Keep only the original native response in the existing volatile owner.
    /// Formal identity must be established independently before replaying it.
    pub fn retain_committed_result(
        &self,
        key: WorkerJobKey,
        result: serde_json::Value,
    ) -> StoreResult<()> {
        let mut state = self.lock()?;
        let execution = state
            .executions
            .get_mut(&key)
            .filter(|execution| execution.committed && execution.released)
            .ok_or_else(|| invalid("Publication has no committed cleanup owner"))?;
        if result["expected_identity"]
            != serde_json::to_value(
                execution
                    .source
                    .expected_identity(key)
                    .map_err(|error| invalid(&error))?,
            )
            .map_err(|error| invalid(&error.to_string()))?
            || execution
                .committed_result
                .as_ref()
                .is_some_and(|previous| previous != &result)
        {
            return Err(invalid("Committed publication response identity changed"));
        }
        execution.committed_result = Some(result);
        Ok(())
    }

    pub fn committed_result(&self, key: WorkerJobKey) -> StoreResult<Option<serde_json::Value>> {
        Ok(self.lock()?.executions.get(&key).and_then(|execution| {
            execution
                .committed
                .then(|| execution.committed_result.clone())
                .flatten()
        }))
    }

    pub fn committed_cleanup_complete(&self, key: WorkerJobKey) -> StoreResult<bool> {
        Ok(self
            .lock()?
            .executions
            .get(&key)
            .is_some_and(|execution| execution.committed && execution.cleanup_complete))
    }

    /// Only strict finish releases the retained response after complete cleanup.
    /// Opportunistic metadata observation must leave it available to that call.
    pub fn release_completed_result(&self, key: WorkerJobKey) -> StoreResult<()> {
        let mut state = self.lock()?;
        if let Some(execution) = state.executions.get(&key) {
            if !execution.committed || !execution.cleanup_complete {
                return Err(invalid("Committed publication cleanup is incomplete"));
            }
            state.executions.remove(&key);
        }
        Ok(())
    }

    pub fn cleanup_committed(&self, key: WorkerJobKey) -> StoreResult<bool> {
        self.cleanup_committed_with(key, ManagedCiPackage::request_cleanup)
    }

    /// The serving adapter also finalizes the known hash parent under its
    /// package-operation lock. Keep the owner when either cleanup phase fails.
    pub fn cleanup_committed_with(
        &self,
        key: WorkerJobKey,
        cleanup: impl FnOnce(&ManagedCiPackage) -> StoreResult<bool>,
    ) -> StoreResult<bool> {
        let Some(source) = self.committed_source(key)? else {
            return Ok(false);
        };
        if !cleanup(&source)? {
            return Ok(false);
        }
        let mut state = self.lock()?;
        if state
            .executions
            .get(&key)
            .is_some_and(|execution| execution.committed && execution.source == source)
        {
            let execution = state.executions.get_mut(&key).expect("committed owner");
            execution.cleanup_complete = true;
            if execution.committed_result.is_none() {
                state.executions.remove(&key);
            }
        }
        Ok(true)
    }

    fn lock(&self) -> StoreResult<std::sync::MutexGuard<'_, PreparedCiState>> {
        self.state
            .lock()
            .map_err(|_| invalid("prepared CI runtime lock is poisoned"))
    }
}

fn real_directory(path: &Path) -> StoreResult<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| BinaryDbError::io("inspect managed CI directory", error))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(invalid("managed CI path is not a real directory"));
    }
    fs::canonicalize(path).map_err(|error| BinaryDbError::io("resolve managed CI directory", error))
}

fn real_file(path: &Path) -> StoreResult<PathBuf> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| BinaryDbError::io("inspect managed CI package", error))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid("managed CI package is not a regular file"));
    }
    fs::canonicalize(path).map_err(|error| BinaryDbError::io("resolve managed CI package", error))
}

fn invalid(message: &str) -> BinaryDbError {
    BinaryDbError::invalid_domain_data(message)
}

#[cfg(test)]
#[path = "prepared_ci_runtime/tests.rs"]
mod tests;
