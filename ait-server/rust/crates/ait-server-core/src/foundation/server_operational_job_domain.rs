use crate::foundation::operational_binary_v0::{
    ServerWorkerJobRecord, WORKER_JOB_KIND_CONTENT_GC, WORKER_JOB_KIND_CONTENT_OPTIMIZE,
    WORKER_JOB_KIND_CONTENT_PACK, WORKER_JOB_KIND_LAND_PROCESS, WORKER_JOB_KIND_MAIN_SEED_REFRESH,
    WORKER_JOB_KIND_PATCHSET_CI, WORKER_JOB_KIND_PATCHSET_CI_AGGREGATE,
    WORKER_JOB_KIND_POLICY_EVALUATE, WORKER_JOB_KIND_RECONCILE_REPO, WORKER_JOB_KIND_REPO_CI,
    WORKER_JOB_OUTCOME_COMPLETED, WORKER_JOB_OUTCOME_SKIPPED,
};
use crate::foundation::remote_binary_db::{
    BinaryDbCommandScope, BinaryDbError, BinaryDbReadTxn, BinaryDbWriteTxn, ServerRemoteBinaryDb,
    StoreResult,
};
use crate::foundation::server_content_binary_db::{
    ServerBinaryDbSnapshotStore, SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use crate::foundation::server_operational_runtime_leases::{
    RuntimeLeaseToken, ServerOperationalRuntimeLeases,
};
use crate::foundation::server_operational_worker_jobs::{
    ServerOperationalWorkerJobStore, WorkerJobCreateSpec, WorkerJobDomainAuthority,
    WorkerJobEnqueueDisposition, WorkerJobEntry, WorkerJobKey,
};
use crate::foundation::workflow_binary_v0::{
    V0ChangeRecord, V0FrozenPatchsetRecord, WorkflowBinaryV0Codec, CHANGE_META_BLOCKED,
    CHANGE_META_READY_TO_LAND, CHANGE_META_VALIDATION_PENDING, CI_STATUS_ERROR, CI_STATUS_FAIL,
    CI_STATUS_NONE, CI_STATUS_PASS, PATCHSET_EVALUATION_PENDING,
};
use std::path::PathBuf;
use std::sync::Arc;

pub const WORKER_JOB_LOGICAL_MAIN: &str = "main";
pub const WORKER_JOB_LAND_MODE_DIRECT: &str = "direct";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum WorkerJobKind {
    ContentGc = WORKER_JOB_KIND_CONTENT_GC,
    ContentOptimize = WORKER_JOB_KIND_CONTENT_OPTIMIZE,
    ContentPack = WORKER_JOB_KIND_CONTENT_PACK,
    LandProcess = WORKER_JOB_KIND_LAND_PROCESS,
    MainSeedRefresh = WORKER_JOB_KIND_MAIN_SEED_REFRESH,
    PatchsetCi = WORKER_JOB_KIND_PATCHSET_CI,
    PatchsetCiAggregate = WORKER_JOB_KIND_PATCHSET_CI_AGGREGATE,
    PolicyEvaluate = WORKER_JOB_KIND_POLICY_EVALUATE,
    ReconcileRepo = WORKER_JOB_KIND_RECONCILE_REPO,
    RepoCi = WORKER_JOB_KIND_REPO_CI,
}

impl WorkerJobKind {
    pub const ALL: [Self; 10] = [
        Self::ContentGc,
        Self::ContentOptimize,
        Self::ContentPack,
        Self::LandProcess,
        Self::MainSeedRefresh,
        Self::PatchsetCi,
        Self::PatchsetCiAggregate,
        Self::PolicyEvaluate,
        Self::ReconcileRepo,
        Self::RepoCi,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ContentGc => "content.gc",
            Self::ContentOptimize => "content.optimize",
            Self::ContentPack => "content.pack",
            Self::LandProcess => "land.process",
            Self::MainSeedRefresh => "main-seed.refresh",
            Self::PatchsetCi => "patchset.ci",
            Self::PatchsetCiAggregate => "patchset.ci.aggregate",
            Self::PolicyEvaluate => "policy.evaluate",
            Self::ReconcileRepo => "reconcile.repo",
            Self::RepoCi => "repo.ci",
        }
    }
}

impl TryFrom<u8> for WorkerJobKind {
    type Error = crate::foundation::remote_binary_db::BinaryDbError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            WORKER_JOB_KIND_CONTENT_GC => Ok(Self::ContentGc),
            WORKER_JOB_KIND_CONTENT_OPTIMIZE => Ok(Self::ContentOptimize),
            WORKER_JOB_KIND_CONTENT_PACK => Ok(Self::ContentPack),
            WORKER_JOB_KIND_LAND_PROCESS => Ok(Self::LandProcess),
            WORKER_JOB_KIND_MAIN_SEED_REFRESH => Ok(Self::MainSeedRefresh),
            WORKER_JOB_KIND_PATCHSET_CI => Ok(Self::PatchsetCi),
            WORKER_JOB_KIND_PATCHSET_CI_AGGREGATE => Ok(Self::PatchsetCiAggregate),
            WORKER_JOB_KIND_POLICY_EVALUATE => Ok(Self::PolicyEvaluate),
            WORKER_JOB_KIND_RECONCILE_REPO => Ok(Self::ReconcileRepo),
            WORKER_JOB_KIND_REPO_CI => Ok(Self::RepoCi),
            _ => Err(invalid("Worker Job kind is unassigned or reserved")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerJobPatchsetClosure {
    pub patchset_index: u32,
    pub change_index: u32,
    pub patch_ordinal: u8,
    pub change_ordinal: u8,
    pub base_snapshot_index: u32,
    pub revision_snapshot_index: u32,
    pub ci_run_seq: u32,
    pub selected_ci_worker_job_index: Option<u32>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerJobExecutionSelector {
    ContentGc,
    ContentOptimize,
    ContentPack,
    LandProcess {
        patchset: WorkerJobPatchsetClosure,
        target_line: &'static str,
        mode: &'static str,
    },
    MainSeedRefresh {
        patchset: WorkerJobPatchsetClosure,
        prior_snapshot_index: Option<u32>,
        target_line: &'static str,
    },
    PatchsetCi {
        patchset: WorkerJobPatchsetClosure,
    },
    PatchsetCiAggregate {
        patchset: WorkerJobPatchsetClosure,
    },
    PolicyEvaluate {
        patchset: WorkerJobPatchsetClosure,
    },
    ReconcileRepo,
    RepoCi {
        snapshot_index: u32,
        target_line: &'static str,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerJobCompactCiEvidence {
    pub ci_completed_at_s: u64,
    pub ci_run_seq: u32,
    pub selected_suite_count: u16,
    pub suite_result_count: u16,
    pub blocking_failure_count: u16,
    pub overall_status: u8,
    pub tests_status: u8,
    pub lint_status: u8,
}

impl WorkerJobCompactCiEvidence {
    fn status_bits(self) -> StoreResult<u8> {
        if self.ci_completed_at_s == 0
            || self.ci_run_seq == 0
            || self.overall_status == CI_STATUS_NONE
            || !valid_ci_status(self.overall_status)
            || !valid_ci_status(self.tests_status)
            || !valid_ci_status(self.lint_status)
            || self.suite_result_count > self.selected_suite_count
            || self.blocking_failure_count > self.suite_result_count
        {
            return Err(invalid("Patchset CI compact evidence is invalid"));
        }
        Ok(self.overall_status | (self.tests_status << 2) | (self.lint_status << 4))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerJobDomainCompletion {
    Completed,
    Skipped,
    PatchsetCi(WorkerJobCompactCiEvidence),
    PatchsetCiAggregate {
        selected_ci_worker_job_index: u32,
        evidence: WorkerJobCompactCiEvidence,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerJobEnqueueRequest {
    pub job_kind: WorkerJobKind,
    pub max_attempts: u16,
    pub patchset_index: Option<u32>,
    pub snapshot_index: Option<u32>,
    pub available_at_s: u64,
    pub created_at_s: u64,
    pub disposition: WorkerJobEnqueueDisposition,
}

impl WorkerJobEnqueueRequest {
    fn fixed_spec(self) -> StoreResult<WorkerJobCreateSpec> {
        Ok(WorkerJobCreateSpec {
            job_kind: self.job_kind as u8,
            max_attempts: self.max_attempts,
            patchset_index_plus1: plus_one(self.patchset_index, "Patchset")?,
            snapshot_index_plus1: plus_one(self.snapshot_index, "Snapshot")?,
            available_at_s: self.available_at_s,
            created_at_s: self.created_at_s,
        })
    }
}

pub trait WorkerJobExecutionAuthority: WorkerJobDomainAuthority {
    fn patchset_closure(&self, patchset_index: u32) -> StoreResult<WorkerJobPatchsetClosure>;

    fn select_patchset_ci_job(
        &self,
        patchset_index: u32,
        worker_job_index: u32,
        starts_new_run: bool,
        now_s: u64,
    ) -> StoreResult<()>;

    fn commit_patchset_ci_evidence(
        &self,
        patchset_index: u32,
        selected_ci_worker_job_index: u32,
        evidence: WorkerJobCompactCiEvidence,
    ) -> StoreResult<()>;
}

/// Commits the successful result into the already-existing owning domain.
///
/// A successful return is the domain commit point, not a projection or log
/// acknowledgement. The Job service calls these methods while the Job remains
/// `running`, and only writes its `succeeded` marker after the method returns.
/// Implementations must be idempotent because process death may occur between
/// the domain commit and the later fixed Job-record replacement.
pub trait WorkerJobDomainActions: Send + Sync {
    fn content_gc(&self, key: WorkerJobKey) -> StoreResult<()>;

    fn content_optimize(&self, key: WorkerJobKey) -> StoreResult<()>;

    fn content_pack(&self, key: WorkerJobKey) -> StoreResult<()>;

    fn land_process(
        &self,
        key: WorkerJobKey,
        patchset: WorkerJobPatchsetClosure,
        target_line: &str,
        mode: &str,
    ) -> StoreResult<()>;

    fn main_seed_refresh(
        &self,
        key: WorkerJobKey,
        patchset: WorkerJobPatchsetClosure,
        prior_snapshot_index: Option<u32>,
        target_line: &str,
    ) -> StoreResult<()>;

    fn patchset_ci(
        &self,
        key: WorkerJobKey,
        patchset: WorkerJobPatchsetClosure,
        evidence: WorkerJobCompactCiEvidence,
    ) -> StoreResult<()>;

    fn patchset_ci_aggregate(
        &self,
        key: WorkerJobKey,
        patchset: WorkerJobPatchsetClosure,
        selected_ci_worker_job_index: u32,
        evidence: WorkerJobCompactCiEvidence,
    ) -> StoreResult<()>;

    fn policy_evaluate(
        &self,
        key: WorkerJobKey,
        patchset: WorkerJobPatchsetClosure,
    ) -> StoreResult<()>;

    fn reconcile_repo(&self, key: WorkerJobKey) -> StoreResult<()>;

    fn repo_ci(&self, key: WorkerJobKey, snapshot_index: u32, target_line: &str)
        -> StoreResult<()>;
}

#[derive(Clone)]
pub struct ServerOperationalWorkerJobDomainService<A, E>
where
    A: WorkerJobExecutionAuthority + 'static,
    E: WorkerJobDomainActions + 'static,
{
    jobs: ServerOperationalWorkerJobStore,
    leases: ServerOperationalRuntimeLeases,
    authority: Arc<A>,
    actions: Arc<E>,
}

impl<A, E> ServerOperationalWorkerJobDomainService<A, E>
where
    A: WorkerJobExecutionAuthority + 'static,
    E: WorkerJobDomainActions + 'static,
{
    pub fn new(
        repository_index: u32,
        authority_root: impl Into<PathBuf>,
        leases: ServerOperationalRuntimeLeases,
        authority: Arc<A>,
        actions: Arc<E>,
    ) -> StoreResult<Self> {
        let validation_authority: Arc<dyn WorkerJobDomainAuthority> = authority.clone();
        let jobs = ServerOperationalWorkerJobStore::new(
            repository_index,
            authority_root,
            validation_authority,
        )?;
        Ok(Self {
            jobs,
            leases,
            authority,
            actions,
        })
    }

    pub fn jobs(&self) -> &ServerOperationalWorkerJobStore {
        &self.jobs
    }

    pub fn leases(&self) -> &ServerOperationalRuntimeLeases {
        &self.leases
    }

    pub fn initialize(&self) -> StoreResult<()> {
        self.jobs.initialize()
    }

    pub fn enqueue(&self, request: WorkerJobEnqueueRequest) -> StoreResult<WorkerJobEntry> {
        let spec = request.fixed_spec()?;
        self.resolve_spec(spec)?;
        let authority = self.authority.clone();
        self.jobs
            .enqueue_with_relationship_commit(spec, request.disposition, move |entry| {
                if request.job_kind == WorkerJobKind::PatchsetCi {
                    let patchset_index = request
                        .patchset_index
                        .ok_or_else(|| invalid("patchset.ci has no Patchset"))?;
                    authority.select_patchset_ci_job(
                        patchset_index,
                        entry.key.worker_job_index,
                        request.disposition == WorkerJobEnqueueDisposition::Queued,
                        request.created_at_s,
                    )?;
                }
                Ok(())
            })
    }

    pub fn execution_selector(
        &self,
        worker_job_index: u32,
    ) -> StoreResult<WorkerJobExecutionSelector> {
        self.resolve_record(self.jobs.get(worker_job_index)?.record)
    }

    pub fn complete(
        &self,
        worker_job_index: u32,
        attempt_count: u16,
        lease_token: RuntimeLeaseToken,
        completion: WorkerJobDomainCompletion,
        now_s: u64,
    ) -> StoreResult<WorkerJobEntry> {
        let outcome_kind = match completion {
            WorkerJobDomainCompletion::Skipped => WORKER_JOB_OUTCOME_SKIPPED,
            _ => WORKER_JOB_OUTCOME_COMPLETED,
        };
        self.leases
            .complete_with_domain_commit(
                &self.jobs,
                worker_job_index,
                attempt_count,
                lease_token,
                outcome_kind,
                now_s,
                |entry| self.commit_domain_result(entry, completion),
            )
            .map(|(worker_job, ())| worker_job)
    }

    fn resolve_spec(&self, spec: WorkerJobCreateSpec) -> StoreResult<WorkerJobExecutionSelector> {
        self.resolve_record(ServerWorkerJobRecord {
            job_kind: spec.job_kind,
            max_attempts: spec.max_attempts,
            patchset_index_plus1: spec.patchset_index_plus1,
            snapshot_index_plus1: spec.snapshot_index_plus1,
            available_at_s: spec.available_at_s,
            created_at_s: spec.created_at_s,
            updated_at_s: spec.created_at_s,
            ..ServerWorkerJobRecord::default()
        })
    }

    fn resolve_record(
        &self,
        record: ServerWorkerJobRecord,
    ) -> StoreResult<WorkerJobExecutionSelector> {
        let job_kind = WorkerJobKind::try_from(record.job_kind)?;
        let patchset_index = minus_one(record.patchset_index_plus1);
        let snapshot_index = minus_one(record.snapshot_index_plus1);
        match job_kind {
            WorkerJobKind::ContentGc if patchset_index.is_none() && snapshot_index.is_none() => {
                Ok(WorkerJobExecutionSelector::ContentGc)
            }
            WorkerJobKind::ContentOptimize
                if patchset_index.is_none() && snapshot_index.is_none() =>
            {
                Ok(WorkerJobExecutionSelector::ContentOptimize)
            }
            WorkerJobKind::ContentPack if patchset_index.is_none() && snapshot_index.is_none() => {
                Ok(WorkerJobExecutionSelector::ContentPack)
            }
            WorkerJobKind::LandProcess if snapshot_index.is_none() => {
                Ok(WorkerJobExecutionSelector::LandProcess {
                    patchset: self.required_patchset_closure(patchset_index, job_kind)?,
                    target_line: WORKER_JOB_LOGICAL_MAIN,
                    mode: WORKER_JOB_LAND_MODE_DIRECT,
                })
            }
            WorkerJobKind::MainSeedRefresh => {
                if let Some(snapshot_index) = snapshot_index {
                    self.authority.validate_snapshot_index(snapshot_index)?;
                }
                Ok(WorkerJobExecutionSelector::MainSeedRefresh {
                    patchset: self.required_patchset_closure(patchset_index, job_kind)?,
                    prior_snapshot_index: snapshot_index,
                    target_line: WORKER_JOB_LOGICAL_MAIN,
                })
            }
            WorkerJobKind::PatchsetCi if snapshot_index.is_none() => {
                Ok(WorkerJobExecutionSelector::PatchsetCi {
                    patchset: self.required_patchset_closure(patchset_index, job_kind)?,
                })
            }
            WorkerJobKind::PatchsetCiAggregate if snapshot_index.is_none() => {
                Ok(WorkerJobExecutionSelector::PatchsetCiAggregate {
                    patchset: self.required_patchset_closure(patchset_index, job_kind)?,
                })
            }
            WorkerJobKind::PolicyEvaluate if snapshot_index.is_none() => {
                Ok(WorkerJobExecutionSelector::PolicyEvaluate {
                    patchset: self.required_patchset_closure(patchset_index, job_kind)?,
                })
            }
            WorkerJobKind::ReconcileRepo
                if patchset_index.is_none() && snapshot_index.is_none() =>
            {
                Ok(WorkerJobExecutionSelector::ReconcileRepo)
            }
            WorkerJobKind::RepoCi if patchset_index.is_none() => {
                let snapshot_index =
                    snapshot_index.ok_or_else(|| invalid("repo.ci has no selected Snapshot"))?;
                self.authority.validate_snapshot_index(snapshot_index)?;
                Ok(WorkerJobExecutionSelector::RepoCi {
                    snapshot_index,
                    target_line: WORKER_JOB_LOGICAL_MAIN,
                })
            }
            _ => Err(invalid(
                "Worker Job cannot be reconstructed from its fixed domain references",
            )),
        }
    }

    fn required_patchset_closure(
        &self,
        patchset_index: Option<u32>,
        job_kind: WorkerJobKind,
    ) -> StoreResult<WorkerJobPatchsetClosure> {
        self.authority.patchset_closure(
            patchset_index
                .ok_or_else(|| invalid(format!("{} has no Patchset", job_kind.as_str())))?,
        )
    }

    fn commit_domain_result(
        &self,
        entry: WorkerJobEntry,
        completion: WorkerJobDomainCompletion,
    ) -> StoreResult<()> {
        let selector = self.resolve_record(entry.record)?;
        if completion == WorkerJobDomainCompletion::Skipped {
            return Ok(());
        }
        match (selector, completion) {
            (WorkerJobExecutionSelector::ContentGc, WorkerJobDomainCompletion::Completed) => {
                self.actions.content_gc(entry.key)
            }
            (WorkerJobExecutionSelector::ContentOptimize, WorkerJobDomainCompletion::Completed) => {
                self.actions.content_optimize(entry.key)
            }
            (WorkerJobExecutionSelector::ContentPack, WorkerJobDomainCompletion::Completed) => {
                self.actions.content_pack(entry.key)
            }
            (
                WorkerJobExecutionSelector::LandProcess {
                    patchset,
                    target_line,
                    mode,
                },
                WorkerJobDomainCompletion::Completed,
            ) => self
                .actions
                .land_process(entry.key, patchset, target_line, mode),
            (
                WorkerJobExecutionSelector::MainSeedRefresh {
                    patchset,
                    prior_snapshot_index,
                    target_line,
                },
                WorkerJobDomainCompletion::Completed,
            ) => self.actions.main_seed_refresh(
                entry.key,
                patchset,
                prior_snapshot_index,
                target_line,
            ),
            (
                WorkerJobExecutionSelector::PatchsetCi { patchset },
                WorkerJobDomainCompletion::PatchsetCi(evidence),
            ) => {
                self.actions.patchset_ci(entry.key, patchset, evidence)?;
                self.authority.commit_patchset_ci_evidence(
                    patchset.patchset_index,
                    entry.key.worker_job_index,
                    evidence,
                )
            }
            (
                WorkerJobExecutionSelector::PatchsetCiAggregate { patchset },
                WorkerJobDomainCompletion::PatchsetCiAggregate {
                    selected_ci_worker_job_index,
                    evidence,
                },
            ) => {
                self.actions.patchset_ci_aggregate(
                    entry.key,
                    patchset,
                    selected_ci_worker_job_index,
                    evidence,
                )?;
                self.authority.commit_patchset_ci_evidence(
                    patchset.patchset_index,
                    selected_ci_worker_job_index,
                    evidence,
                )
            }
            (
                WorkerJobExecutionSelector::PolicyEvaluate { patchset },
                WorkerJobDomainCompletion::Completed,
            ) => self.actions.policy_evaluate(entry.key, patchset),
            (WorkerJobExecutionSelector::ReconcileRepo, WorkerJobDomainCompletion::Completed) => {
                self.actions.reconcile_repo(entry.key)
            }
            (
                WorkerJobExecutionSelector::RepoCi {
                    snapshot_index,
                    target_line,
                },
                WorkerJobDomainCompletion::Completed,
            ) => self.actions.repo_ci(entry.key, snapshot_index, target_line),
            _ => Err(invalid(
                "Worker Job completion does not match its fixed execution selector",
            )),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FrozenBinaryV0WorkerJobAuthority<D>
where
    D: ServerRemoteBinaryDb + Clone,
{
    db: D,
}

impl<D> FrozenBinaryV0WorkerJobAuthority<D>
where
    D: ServerRemoteBinaryDb + Clone,
{
    pub fn new(db: D) -> Self {
        Self { db }
    }

    pub fn db(&self) -> &D {
        &self.db
    }

    pub fn snapshot_id_at(&self, snapshot_index: u32) -> StoreResult<String> {
        let read = BinaryDbReadTxn::new(&self.db);
        ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone())
            .snapshot_id_at(&read, snapshot_index)
    }

    pub fn snapshot_index_for_id(&self, snapshot_id: &str) -> StoreResult<u32> {
        let read = BinaryDbReadTxn::new(&self.db);
        ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone())
            .snapshot_by_id(&read, snapshot_id)?
            .map(|(index, _)| index)
            .ok_or_else(|| BinaryDbError::missing_data(format!("unknown Snapshot {snapshot_id}")))
    }

    pub fn frozen_patchset_at(&self, patchset_index: u32) -> StoreResult<V0FrozenPatchsetRecord> {
        let read = BinaryDbReadTxn::new(&self.db);
        self.read_patchset(&read, patchset_index)
    }

    /// Clears only the mutable Patchset-to-Job locator. Compact CI evidence
    /// remains byte-for-byte authoritative.
    pub fn clear_all_patchset_ci_job_locators(&self) -> StoreResult<u32> {
        let mut tx = BinaryDbWriteTxn::begin(&self.db, BinaryDbCommandScope::General)?;
        let file = WorkflowBinaryV0Codec::patchset_file();
        let count = tx.record_count(file.clone())?;
        let mut replacements = Vec::new();
        for patchset_index in 0..count {
            let mut patchset = WorkflowBinaryV0Codec::decode_frozen_patchset(
                &tx.read_record(file.clone(), patchset_index)?,
            )?;
            if patchset.ci_worker_job_index_plus1 == 0 {
                continue;
            }
            patchset.ci_worker_job_index_plus1 = 0;
            replacements.push((
                patchset_index,
                WorkflowBinaryV0Codec::encode_frozen_patchset(patchset)?,
            ));
        }
        tx.overwrite_records(file, &replacements)?;
        tx.commit()?;
        u32::try_from(replacements.len())
            .map_err(|_| corrupt("cleared Patchset CI locator count exceeds u32"))
    }

    fn read_patchset(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        patchset_index: u32,
    ) -> StoreResult<V0FrozenPatchsetRecord> {
        WorkflowBinaryV0Codec::decode_frozen_patchset(
            &read.read_record(WorkflowBinaryV0Codec::patchset_file(), patchset_index)?,
        )
    }

    fn read_change(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        change_index: u32,
    ) -> StoreResult<V0ChangeRecord> {
        WorkflowBinaryV0Codec::decode_change(
            &read.read_record(WorkflowBinaryV0Codec::change_file(), change_index)?,
        )
    }

    fn validate_snapshot_with_read(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        snapshot_index: u32,
    ) -> StoreResult<()> {
        ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone())
            .snapshot_id_at(read, snapshot_index)
            .map(|_| ())
    }

    fn validate_patchset_relation(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        patchset_index: u32,
        patchset: V0FrozenPatchsetRecord,
    ) -> StoreResult<V0ChangeRecord> {
        let change = self.read_change(read, patchset.change_index)?;
        if patchset.change_ordinal != change.change_ordinal {
            return Err(corrupt(format!(
                "Patchset {patchset_index} Change ordinal disagrees"
            )));
        }
        self.validate_snapshot_with_read(read, patchset.base_snapshot_index)?;
        self.validate_snapshot_with_read(read, patchset.revision_snapshot_index)?;
        Ok(change)
    }

    fn read_frozen_patchset_in_write<F>(
        tx: &BinaryDbWriteTxn<'_, D, F>,
        patchset_index: u32,
    ) -> StoreResult<V0FrozenPatchsetRecord>
    where
        F: crate::foundation::remote_binary_db::BinaryDbFsyncPolicy,
    {
        WorkflowBinaryV0Codec::decode_frozen_patchset(
            &tx.read_record(WorkflowBinaryV0Codec::patchset_file(), patchset_index)?,
        )
    }

    fn read_change_in_write<F>(
        tx: &BinaryDbWriteTxn<'_, D, F>,
        change_index: u32,
    ) -> StoreResult<V0ChangeRecord>
    where
        F: crate::foundation::remote_binary_db::BinaryDbFsyncPolicy,
    {
        WorkflowBinaryV0Codec::decode_change(
            &tx.read_record(WorkflowBinaryV0Codec::change_file(), change_index)?,
        )
    }

    fn mark_change_ci_pending<F>(
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        patchset: V0FrozenPatchsetRecord,
        now_s: u64,
    ) -> StoreResult<()>
    where
        F: crate::foundation::remote_binary_db::BinaryDbFsyncPolicy,
    {
        let mut change = Self::read_change_in_write(tx, patchset.change_index)?;
        if change.change_ordinal != patchset.change_ordinal {
            return Err(corrupt("Patchset Change ordinal relation disagrees"));
        }
        if now_s < change.updated_at_s {
            return Err(invalid("Patchset CI mutation moved Change time backwards"));
        }
        change.change_meta |= CHANGE_META_VALIDATION_PENDING;
        change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
        change.updated_at_s = now_s;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::change_file(),
            patchset.change_index,
            &WorkflowBinaryV0Codec::encode_change(change)?,
        )
    }
}

impl<D> FrozenBinaryV0WorkerJobAuthority<D>
where
    D: ServerRemoteBinaryDb + Clone + Send + Sync,
{
    pub fn enqueue_patchset_ci_job(
        &self,
        jobs: &ServerOperationalWorkerJobStore,
        patchset_index: u32,
        max_attempts: u16,
        available_at_s: u64,
        created_at_s: u64,
        starts_new_run: bool,
    ) -> StoreResult<WorkerJobEntry> {
        self.patchset_closure(patchset_index)?;
        let authority = self.clone();
        jobs.enqueue_with_relationship_commit(
            WorkerJobCreateSpec {
                job_kind: WORKER_JOB_KIND_PATCHSET_CI,
                max_attempts,
                patchset_index_plus1: plus_one(Some(patchset_index), "Patchset")?,
                snapshot_index_plus1: 0,
                available_at_s,
                created_at_s,
            },
            WorkerJobEnqueueDisposition::Queued,
            move |entry| {
                authority.select_patchset_ci_job(
                    patchset_index,
                    entry.key.worker_job_index,
                    starts_new_run,
                    created_at_s,
                )
            },
        )
    }

    pub fn enqueue_repo_ci_job(
        &self,
        jobs: &ServerOperationalWorkerJobStore,
        snapshot_index: u32,
        max_attempts: u16,
        available_at_s: u64,
        created_at_s: u64,
    ) -> StoreResult<WorkerJobEntry> {
        self.validate_snapshot_index(snapshot_index)?;
        jobs.enqueue(
            WorkerJobCreateSpec {
                job_kind: WORKER_JOB_KIND_REPO_CI,
                max_attempts,
                patchset_index_plus1: 0,
                snapshot_index_plus1: plus_one(Some(snapshot_index), "Snapshot")?,
                available_at_s,
                created_at_s,
            },
            WorkerJobEnqueueDisposition::Queued,
        )
    }
}

impl<D> WorkerJobDomainAuthority for FrozenBinaryV0WorkerJobAuthority<D>
where
    D: ServerRemoteBinaryDb + Clone + Send + Sync,
{
    fn validate_patchset_index(&self, patchset_index: u32) -> StoreResult<()> {
        self.patchset_closure(patchset_index).map(|_| ())
    }

    fn validate_snapshot_index(&self, snapshot_index: u32) -> StoreResult<()> {
        let read = BinaryDbReadTxn::new(&self.db);
        self.validate_snapshot_with_read(&read, snapshot_index)
    }
}

impl<D> WorkerJobExecutionAuthority for FrozenBinaryV0WorkerJobAuthority<D>
where
    D: ServerRemoteBinaryDb + Clone + Send + Sync,
{
    fn patchset_closure(&self, patchset_index: u32) -> StoreResult<WorkerJobPatchsetClosure> {
        let read = BinaryDbReadTxn::new(&self.db);
        let patchset = self.read_patchset(&read, patchset_index)?;
        let change = self.validate_patchset_relation(&read, patchset_index, patchset)?;
        Ok(WorkerJobPatchsetClosure {
            patchset_index,
            change_index: patchset.change_index,
            patch_ordinal: patchset.patch_ordinal,
            change_ordinal: change.change_ordinal,
            base_snapshot_index: patchset.base_snapshot_index,
            revision_snapshot_index: patchset.revision_snapshot_index,
            ci_run_seq: patchset.ci_run_seq,
            selected_ci_worker_job_index: minus_one(patchset.ci_worker_job_index_plus1),
        })
    }

    fn select_patchset_ci_job(
        &self,
        patchset_index: u32,
        worker_job_index: u32,
        starts_new_run: bool,
        now_s: u64,
    ) -> StoreResult<()> {
        if now_s == 0 {
            return Err(invalid("Patchset CI selection time is required"));
        }
        let worker_job_index_plus1 = worker_job_index
            .checked_add(1)
            .ok_or_else(|| invalid("Worker Job plus-one index overflow"))?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)?;
        let mut patchset = Self::read_frozen_patchset_in_write(&tx, patchset_index)?;
        if patchset.ci_worker_job_index_plus1 == worker_job_index_plus1 {
            return Ok(());
        }
        if patchset.ci_worker_job_index_plus1 > worker_job_index_plus1 {
            return Err(invalid(
                "older patchset.ci Job cannot replace the selected Job locator",
            ));
        }
        if now_s < patchset.created_at_s {
            return Err(invalid("Patchset CI selection predates its Patchset"));
        }
        patchset.ci_worker_job_index_plus1 = worker_job_index_plus1;
        if starts_new_run {
            patchset.ci_run_seq = patchset
                .ci_run_seq
                .checked_add(1)
                .ok_or_else(|| invalid("Patchset CI run sequence exceeds u32"))?;
            patchset.ci_completed_at_s = 0;
            patchset.ci_selected_suite_count = 0;
            patchset.ci_suite_result_count = 0;
            patchset.ci_blocking_failure_count = 0;
            patchset.ci_status_bits = 0;
            patchset.patchset_meta |= PATCHSET_EVALUATION_PENDING;
            Self::mark_change_ci_pending(&mut tx, patchset, now_s)?;
        }
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            patchset_index,
            &WorkflowBinaryV0Codec::encode_frozen_patchset(patchset)?,
        )?;
        tx.commit().map(|_| ())
    }

    fn commit_patchset_ci_evidence(
        &self,
        patchset_index: u32,
        selected_ci_worker_job_index: u32,
        evidence: WorkerJobCompactCiEvidence,
    ) -> StoreResult<()> {
        let status_bits = evidence.status_bits()?;
        let selected_plus1 = selected_ci_worker_job_index
            .checked_add(1)
            .ok_or_else(|| invalid("selected CI Worker Job plus-one index overflow"))?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)?;
        let mut patchset = Self::read_frozen_patchset_in_write(&tx, patchset_index)?;
        if patchset.ci_worker_job_index_plus1 != selected_plus1 {
            return Err(invalid(
                "Patchset CI completion is not for the selected Worker Job",
            ));
        }
        if patchset.ci_run_seq != evidence.ci_run_seq {
            return Err(invalid("Patchset CI completion has a stale run sequence"));
        }
        if patchset.ci_completed_at_s != 0 {
            let exact_replay = patchset.ci_completed_at_s == evidence.ci_completed_at_s
                && patchset.ci_selected_suite_count == evidence.selected_suite_count
                && patchset.ci_suite_result_count == evidence.suite_result_count
                && patchset.ci_blocking_failure_count == evidence.blocking_failure_count
                && patchset.ci_status_bits == status_bits;
            return if exact_replay {
                Ok(())
            } else {
                Err(invalid(
                    "Patchset CI completion conflicts with committed compact evidence",
                ))
            };
        }
        if evidence.ci_completed_at_s < patchset.created_at_s {
            return Err(invalid("Patchset CI completion predates its Patchset"));
        }
        patchset.ci_completed_at_s = evidence.ci_completed_at_s;
        patchset.ci_selected_suite_count = evidence.selected_suite_count;
        patchset.ci_suite_result_count = evidence.suite_result_count;
        patchset.ci_blocking_failure_count = evidence.blocking_failure_count;
        patchset.ci_status_bits = status_bits;
        patchset.patchset_meta |= PATCHSET_EVALUATION_PENDING;
        Self::mark_change_ci_pending(&mut tx, patchset, evidence.ci_completed_at_s)?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            patchset_index,
            &WorkflowBinaryV0Codec::encode_frozen_patchset(patchset)?,
        )?;
        tx.commit().map(|_| ())
    }
}

fn valid_ci_status(value: u8) -> bool {
    matches!(
        value,
        CI_STATUS_NONE | CI_STATUS_PASS | CI_STATUS_FAIL | CI_STATUS_ERROR
    )
}

fn plus_one(index: Option<u32>, label: &str) -> StoreResult<u32> {
    index
        .map(|value| {
            value
                .checked_add(1)
                .ok_or_else(|| invalid(format!("{label} plus-one index overflow")))
        })
        .transpose()
        .map(|value| value.unwrap_or(0))
}

fn minus_one(index_plus1: u32) -> Option<u32> {
    index_plus1.checked_sub(1)
}

fn invalid(message: impl Into<String>) -> crate::foundation::remote_binary_db::BinaryDbError {
    crate::foundation::remote_binary_db::BinaryDbError::invalid_domain_data(message)
}

fn corrupt(message: impl Into<String>) -> crate::foundation::remote_binary_db::BinaryDbError {
    crate::foundation::remote_binary_db::BinaryDbError::corruption(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::foundation::operational_binary_v0::WORKER_JOB_STATE_RUNNING;
    use crate::foundation::operational_binary_v0::WORKER_JOB_STATE_SUCCEEDED;
    use crate::foundation::remote_binary_db::{
        FilesystemServerRemoteBinaryDb, RepoId, RepoName, StoreGeneration, StorePath,
    };
    use crate::foundation::server_binary_db_schema_registry::{
        SERVER_BINARY_DB_BIN_SCHEMAS, SERVER_BINARY_DB_INDEX_SCHEMAS, SERVER_BINARY_DB_LAYOUT_ID,
    };
    use crate::foundation::server_content_binary_db::{
        server_snapshot_hash48_from_id, ServerBinaryDbLineStore, ServerBinarySnapshotPayload,
        ServerBinarySnapshotRecord,
    };
    use crate::foundation::workflow_binary_v0::{
        V0ChangeRecord, V0FrozenPatchsetRecord, CHANGE_LIFECYCLE_ACTIVE, CHANGE_META_HAS_PATCHSETS,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Debug)]
    struct TestAuthority {
        patchsets: Mutex<BTreeMap<u32, WorkerJobPatchsetClosure>>,
        snapshots: BTreeSet<u32>,
        events: Mutex<Vec<String>>,
    }

    impl TestAuthority {
        fn events(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }

    impl WorkerJobDomainAuthority for TestAuthority {
        fn validate_patchset_index(&self, patchset_index: u32) -> StoreResult<()> {
            self.patchset_closure(patchset_index).map(|_| ())
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

    impl WorkerJobExecutionAuthority for TestAuthority {
        fn patchset_closure(&self, patchset_index: u32) -> StoreResult<WorkerJobPatchsetClosure> {
            self.patchsets
                .lock()
                .unwrap()
                .get(&patchset_index)
                .copied()
                .ok_or_else(|| invalid(format!("unknown test Patchset index {patchset_index}")))
        }

        fn select_patchset_ci_job(
            &self,
            patchset_index: u32,
            worker_job_index: u32,
            starts_new_run: bool,
            _now_s: u64,
        ) -> StoreResult<()> {
            let mut patchsets = self.patchsets.lock().unwrap();
            let patchset = patchsets
                .get_mut(&patchset_index)
                .ok_or_else(|| invalid("unknown test Patchset"))?;
            if starts_new_run {
                patchset.ci_run_seq += 1;
            }
            patchset.selected_ci_worker_job_index = Some(worker_job_index);
            self.events.lock().unwrap().push(format!(
                "select:{patchset_index}:{worker_job_index}:{}",
                patchset.ci_run_seq
            ));
            Ok(())
        }

        fn commit_patchset_ci_evidence(
            &self,
            patchset_index: u32,
            selected_ci_worker_job_index: u32,
            evidence: WorkerJobCompactCiEvidence,
        ) -> StoreResult<()> {
            evidence.status_bits()?;
            let patchset = self.patchset_closure(patchset_index)?;
            if patchset.selected_ci_worker_job_index != Some(selected_ci_worker_job_index)
                || patchset.ci_run_seq != evidence.ci_run_seq
            {
                return Err(invalid("test compact CI selection disagrees"));
            }
            self.events.lock().unwrap().push(format!(
                "compact:{patchset_index}:{selected_ci_worker_job_index}"
            ));
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TestActions {
        events: Arc<Mutex<Vec<String>>>,
        fail_kind: Mutex<Option<WorkerJobKind>>,
    }

    impl TestActions {
        fn record(&self, kind: WorkerJobKind, detail: impl Into<String>) -> StoreResult<()> {
            if self.fail_kind.lock().unwrap().as_ref() == Some(&kind) {
                return Err(invalid(format!("injected {} failure", kind.as_str())));
            }
            self.events
                .lock()
                .unwrap()
                .push(format!("action:{}:{}", kind.as_str(), detail.into()));
            Ok(())
        }
    }

    impl WorkerJobDomainActions for TestActions {
        fn content_gc(&self, key: WorkerJobKey) -> StoreResult<()> {
            self.record(WorkerJobKind::ContentGc, key.worker_job_index.to_string())
        }

        fn content_optimize(&self, key: WorkerJobKey) -> StoreResult<()> {
            self.record(
                WorkerJobKind::ContentOptimize,
                key.worker_job_index.to_string(),
            )
        }

        fn content_pack(&self, key: WorkerJobKey) -> StoreResult<()> {
            self.record(WorkerJobKind::ContentPack, key.worker_job_index.to_string())
        }

        fn land_process(
            &self,
            key: WorkerJobKey,
            patchset: WorkerJobPatchsetClosure,
            target_line: &str,
            mode: &str,
        ) -> StoreResult<()> {
            self.record(
                WorkerJobKind::LandProcess,
                format!(
                    "{}:{}:{target_line}:{mode}",
                    key.worker_job_index, patchset.patchset_index
                ),
            )
        }

        fn main_seed_refresh(
            &self,
            key: WorkerJobKey,
            patchset: WorkerJobPatchsetClosure,
            prior_snapshot_index: Option<u32>,
            target_line: &str,
        ) -> StoreResult<()> {
            self.record(
                WorkerJobKind::MainSeedRefresh,
                format!(
                    "{}:{}:{prior_snapshot_index:?}:{target_line}",
                    key.worker_job_index, patchset.patchset_index
                ),
            )
        }

        fn patchset_ci(
            &self,
            key: WorkerJobKey,
            patchset: WorkerJobPatchsetClosure,
            evidence: WorkerJobCompactCiEvidence,
        ) -> StoreResult<()> {
            self.record(
                WorkerJobKind::PatchsetCi,
                format!(
                    "{}:{}:{}",
                    key.worker_job_index, patchset.patchset_index, evidence.ci_run_seq
                ),
            )
        }

        fn patchset_ci_aggregate(
            &self,
            key: WorkerJobKey,
            patchset: WorkerJobPatchsetClosure,
            selected_ci_worker_job_index: u32,
            evidence: WorkerJobCompactCiEvidence,
        ) -> StoreResult<()> {
            self.record(
                WorkerJobKind::PatchsetCiAggregate,
                format!(
                    "{}:{}:{selected_ci_worker_job_index}:{}",
                    key.worker_job_index, patchset.patchset_index, evidence.ci_run_seq
                ),
            )
        }

        fn policy_evaluate(
            &self,
            key: WorkerJobKey,
            patchset: WorkerJobPatchsetClosure,
        ) -> StoreResult<()> {
            self.record(
                WorkerJobKind::PolicyEvaluate,
                format!("{}:{}", key.worker_job_index, patchset.patchset_index),
            )
        }

        fn reconcile_repo(&self, key: WorkerJobKey) -> StoreResult<()> {
            self.record(
                WorkerJobKind::ReconcileRepo,
                key.worker_job_index.to_string(),
            )
        }

        fn repo_ci(
            &self,
            key: WorkerJobKey,
            snapshot_index: u32,
            target_line: &str,
        ) -> StoreResult<()> {
            self.record(
                WorkerJobKind::RepoCi,
                format!("{}:{snapshot_index}:{target_line}", key.worker_job_index),
            )
        }
    }

    struct Fixture {
        root: PathBuf,
        authority: Arc<TestAuthority>,
        actions: Arc<TestActions>,
        service: ServerOperationalWorkerJobDomainService<TestAuthority, TestActions>,
    }

    impl Fixture {
        fn new(label: &str) -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "ait-server-worker-job-domain-{label}-{}-{nonce}",
                std::process::id()
            ));
            let authority_root = root.join("7");
            fs::create_dir_all(&authority_root).unwrap();
            let events = Arc::new(Mutex::new(Vec::new()));
            let authority = Arc::new(TestAuthority {
                patchsets: Mutex::new(BTreeMap::from([(
                    2,
                    WorkerJobPatchsetClosure {
                        patchset_index: 2,
                        change_index: 4,
                        patch_ordinal: 0,
                        change_ordinal: 0,
                        base_snapshot_index: 20,
                        revision_snapshot_index: 21,
                        ci_run_seq: 0,
                        selected_ci_worker_job_index: None,
                    },
                )])),
                snapshots: [7, 9, 20, 21].into_iter().collect(),
                events: Mutex::new(Vec::new()),
            });
            let actions = Arc::new(TestActions {
                events,
                fail_kind: Mutex::new(None),
            });
            let (leases, _) = ServerOperationalRuntimeLeases::open(
                root.join("runtime").join("leases.bin"),
                [authority_root.clone()],
            )
            .unwrap();
            let service = ServerOperationalWorkerJobDomainService::new(
                7,
                authority_root,
                leases,
                authority.clone(),
                actions.clone(),
            )
            .unwrap();
            service.initialize().unwrap();
            Self {
                root,
                authority,
                actions,
                service,
            }
        }

        fn enqueue(
            &self,
            kind: WorkerJobKind,
            patchset_index: Option<u32>,
            snapshot_index: Option<u32>,
        ) -> WorkerJobEntry {
            self.service
                .enqueue(WorkerJobEnqueueRequest {
                    job_kind: kind,
                    max_attempts: 3,
                    patchset_index,
                    snapshot_index,
                    available_at_s: 100,
                    created_at_s: 100,
                    disposition: WorkerJobEnqueueDisposition::Queued,
                })
                .unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn passing_evidence(run: u32) -> WorkerJobCompactCiEvidence {
        WorkerJobCompactCiEvidence {
            ci_completed_at_s: 250,
            ci_run_seq: run,
            selected_suite_count: 2,
            suite_result_count: 2,
            blocking_failure_count: 0,
            overall_status: CI_STATUS_PASS,
            tests_status: CI_STATUS_PASS,
            lint_status: CI_STATUS_PASS,
        }
    }

    #[test]
    fn all_ten_kinds_dispatch_from_fixed_references_and_commit_domain_first() {
        let fixture = Fixture::new("all-kinds");
        let requests = [
            (WorkerJobKind::ContentGc, None, None),
            (WorkerJobKind::ContentOptimize, None, None),
            (WorkerJobKind::ContentPack, None, None),
            (WorkerJobKind::LandProcess, Some(2), None),
            (WorkerJobKind::MainSeedRefresh, Some(2), Some(9)),
            (WorkerJobKind::PatchsetCi, Some(2), None),
            (WorkerJobKind::PatchsetCiAggregate, Some(2), None),
            (WorkerJobKind::PolicyEvaluate, Some(2), None),
            (WorkerJobKind::ReconcileRepo, None, None),
            (WorkerJobKind::RepoCi, None, Some(7)),
        ];
        let entries = requests
            .into_iter()
            .map(|(kind, patchset, snapshot)| fixture.enqueue(kind, patchset, snapshot))
            .collect::<Vec<_>>();

        assert_eq!(fixture.actions.events.lock().unwrap().len(), 0);
        assert_eq!(fixture.authority.events(), vec!["select:2:5:1".to_string()]);
        assert_eq!(
            fixture.service.execution_selector(3).unwrap(),
            WorkerJobExecutionSelector::LandProcess {
                patchset: fixture.authority.patchset_closure(2).unwrap(),
                target_line: "main",
                mode: "direct",
            }
        );
        assert_eq!(
            fixture.service.execution_selector(9).unwrap(),
            WorkerJobExecutionSelector::RepoCi {
                snapshot_index: 7,
                target_line: "main",
            }
        );

        for entry in entries {
            let now = 200_u64 + u64::from(entry.key.worker_job_index);
            let grant = fixture
                .service
                .leases()
                .claim(fixture.service.jobs(), entry.key.worker_job_index, now, 30)
                .unwrap();
            let completion = match WorkerJobKind::try_from(entry.record.job_kind).unwrap() {
                WorkerJobKind::PatchsetCi => {
                    WorkerJobDomainCompletion::PatchsetCi(passing_evidence(1))
                }
                WorkerJobKind::PatchsetCiAggregate => {
                    WorkerJobDomainCompletion::PatchsetCiAggregate {
                        selected_ci_worker_job_index: 5,
                        evidence: passing_evidence(1),
                    }
                }
                _ => WorkerJobDomainCompletion::Completed,
            };
            let completed = fixture
                .service
                .complete(
                    entry.key.worker_job_index,
                    grant.attempt_count,
                    grant.lease_token,
                    completion,
                    now + 1,
                )
                .unwrap();
            assert_eq!(completed.record.state_kind, WORKER_JOB_STATE_SUCCEEDED);
        }

        let actions = fixture.actions.events.lock().unwrap().clone();
        assert_eq!(actions.len(), 10);
        assert!(actions.contains(&"action:land.process:3:2:main:direct".to_string()));
        assert!(actions.contains(&"action:main-seed.refresh:4:2:Some(9):main".to_string()));
        assert!(actions.contains(&"action:repo.ci:9:7:main".to_string()));
        assert_eq!(
            fixture.authority.events(),
            vec![
                "select:2:5:1".to_string(),
                "compact:2:5".to_string(),
                "compact:2:5".to_string(),
            ]
        );
    }

    #[test]
    fn unreconstructible_input_fails_before_allocating_a_physical_job_index() {
        let fixture = Fixture::new("fail-closed");
        let error = fixture
            .service
            .enqueue(WorkerJobEnqueueRequest {
                job_kind: WorkerJobKind::RepoCi,
                max_attempts: 3,
                patchset_index: None,
                snapshot_index: None,
                available_at_s: 100,
                created_at_s: 100,
                disposition: WorkerJobEnqueueDisposition::Queued,
            })
            .expect_err("repo.ci without its frozen Snapshot must fail");
        assert!(error.to_string().contains("selected Snapshot"));
        assert!(fixture.service.jobs().list().unwrap().is_empty());

        let error = fixture
            .service
            .enqueue(WorkerJobEnqueueRequest {
                job_kind: WorkerJobKind::PatchsetCi,
                max_attempts: 3,
                patchset_index: Some(99),
                snapshot_index: None,
                available_at_s: 100,
                created_at_s: 100,
                disposition: WorkerJobEnqueueDisposition::Queued,
            })
            .expect_err("missing Patchset must fail before append");
        assert!(error.to_string().contains("unknown test Patchset"));
        assert!(fixture.service.jobs().list().unwrap().is_empty());
    }

    #[test]
    fn failed_domain_commit_leaves_the_job_running_and_lease_valid() {
        let fixture = Fixture::new("domain-failure");
        let job = fixture.enqueue(WorkerJobKind::ContentGc, None, None);
        *fixture.actions.fail_kind.lock().unwrap() = Some(WorkerJobKind::ContentGc);
        let grant = fixture
            .service
            .leases()
            .claim(fixture.service.jobs(), job.key.worker_job_index, 200, 30)
            .unwrap();
        let error = fixture
            .service
            .complete(
                job.key.worker_job_index,
                grant.attempt_count,
                grant.lease_token,
                WorkerJobDomainCompletion::Completed,
                201,
            )
            .expect_err("domain failure must precede the Job success marker");
        assert!(error.to_string().contains("injected content.gc failure"));
        assert_eq!(
            fixture
                .service
                .jobs()
                .get(job.key.worker_job_index)
                .unwrap()
                .record
                .state_kind,
            WORKER_JOB_STATE_RUNNING
        );
        fixture
            .service
            .leases()
            .validate_presented(
                fixture.service.jobs(),
                job.key.worker_job_index,
                grant.attempt_count,
                grant.lease_token,
                202,
            )
            .expect("failed domain commit must retain its live retry lease");
    }

    #[test]
    fn patchset_ci_requires_typed_compact_evidence_before_success() {
        let fixture = Fixture::new("ci-evidence");
        let job = fixture.enqueue(WorkerJobKind::PatchsetCi, Some(2), None);
        let grant = fixture
            .service
            .leases()
            .claim(fixture.service.jobs(), job.key.worker_job_index, 200, 30)
            .unwrap();
        let error = fixture
            .service
            .complete(
                job.key.worker_job_index,
                grant.attempt_count,
                grant.lease_token,
                WorkerJobDomainCompletion::Completed,
                201,
            )
            .expect_err("untyped CI completion must fail");
        assert!(error.to_string().contains("does not match"));
        assert_eq!(
            fixture
                .service
                .jobs()
                .get(job.key.worker_job_index)
                .unwrap()
                .record
                .state_kind,
            WORKER_JOB_STATE_RUNNING
        );
        fixture
            .service
            .complete(
                job.key.worker_job_index,
                grant.attempt_count,
                grant.lease_token,
                WorkerJobDomainCompletion::PatchsetCi(passing_evidence(1)),
                202,
            )
            .unwrap();
    }

    #[test]
    fn disposable_lease_replica_stays_outside_the_repository_authority() {
        let fixture = Fixture::new("replica");
        assert!(!fixture
            .service
            .leases()
            .replica_path()
            .starts_with(fixture.service.jobs().authority_root()));
        for forbidden in [
            "worker_job_payload.bin",
            "worker_job_input_payload.bin",
            "worker_job_result_payload.bin",
        ] {
            assert!(!Path::new(fixture.service.jobs().authority_root())
                .join(forbidden)
                .exists());
        }
    }

    #[test]
    fn frozen_binary_authority_commits_ci_locator_and_compact_evidence_idempotently() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ait-server-frozen-job-domain-{}-{nonce}",
            std::process::id()
        ));
        let authority_root = root.join("1");
        fs::create_dir_all(&authority_root).unwrap();
        for path in SERVER_BINARY_DB_BIN_SCHEMAS
            .iter()
            .map(|schema| schema.path)
            .chain(
                SERVER_BINARY_DB_INDEX_SCHEMAS
                    .iter()
                    .map(|schema| schema.path),
            )
        {
            fs::write(
                authority_root.join(path),
                SERVER_BINARY_DB_LAYOUT_ID.to_le_bytes(),
            )
            .unwrap();
        }
        let db = FilesystemServerRemoteBinaryDb::test_fixture(
            RepoId::new("REPO-FROZEN-DOMAIN"),
            RepoName::new("ait-server"),
            StorePath::new(authority_root.clone()),
            StoreGeneration::new(1),
        );
        let lines = ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
        let snapshots =
            ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
        let line_index = lines.create_line("main", 0, 90).unwrap();
        let payload = ServerBinarySnapshotPayload {
            line_name: "main".to_string(),
            message: Some("Worker Job domain fixture".to_string()),
        };
        let snapshot_record =
            |snapshot_id: &str, parent_snapshot_index_plus1: u32| ServerBinarySnapshotRecord {
                snapshot_meta: 0,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: server_snapshot_hash48_from_id(snapshot_id).unwrap(),
                parent_snapshot_index_plus1,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: line_index + 1,
                manifest_hash: [0; 32],
                file_count: 0,
                total_bytes: 0,
                created_at_s: 90,
            };
        let base = snapshots
            .append_snapshot(
                "SNP-0000000000A1",
                snapshot_record("SNP-0000000000A1", 0),
                &payload,
            )
            .unwrap();
        let revision = snapshots
            .append_snapshot(
                "SNP-0000000000A2",
                snapshot_record("SNP-0000000000A2", base + 1),
                &payload,
            )
            .unwrap();
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
        let change = V0ChangeRecord {
            change_meta: CHANGE_LIFECYCLE_ACTIVE | CHANGE_META_HAS_PATCHSETS,
            payload_len: 1,
            selected_patchset_index_plus1: 1,
            base_line_index_plus1: line_index + 1,
            created_at_s: 100,
            updated_at_s: 100,
            ..V0ChangeRecord::default()
        };
        assert_eq!(
            tx.append_record(
                WorkflowBinaryV0Codec::change_file(),
                &WorkflowBinaryV0Codec::encode_change(change).unwrap(),
            )
            .unwrap(),
            0
        );
        let patchset = V0FrozenPatchsetRecord {
            patchset_meta: 0,
            change_index: 0,
            base_snapshot_index: base,
            revision_snapshot_index: revision,
            created_at_s: 100,
            summary_offset: 4,
            summary_len: 1,
            ..V0FrozenPatchsetRecord::default()
        };
        assert_eq!(
            tx.append_record(
                WorkflowBinaryV0Codec::patchset_file(),
                &WorkflowBinaryV0Codec::encode_frozen_patchset(patchset).unwrap(),
            )
            .unwrap(),
            0
        );
        tx.commit().unwrap();

        let authority = FrozenBinaryV0WorkerJobAuthority::new(db.clone());
        let original = authority.patchset_closure(0).unwrap();
        assert_eq!(
            (
                original.base_snapshot_index,
                original.revision_snapshot_index,
                original.ci_run_seq,
                original.selected_ci_worker_job_index,
            ),
            (base, revision, 0, None)
        );
        authority.select_patchset_ci_job(0, 4, true, 101).unwrap();
        let selected = authority.patchset_closure(0).unwrap();
        assert_eq!(selected.ci_run_seq, 1);
        assert_eq!(selected.selected_ci_worker_job_index, Some(4));
        authority
            .select_patchset_ci_job(0, 4, true, 101)
            .expect("exact relationship replay must not allocate another CI run");
        assert_eq!(authority.patchset_closure(0).unwrap().ci_run_seq, 1);
        assert!(authority
            .select_patchset_ci_job(0, 3, true, 102)
            .expect_err("an older Job cannot replace the locator")
            .to_string()
            .contains("older patchset.ci"));

        let evidence = WorkerJobCompactCiEvidence {
            ci_completed_at_s: 102,
            ci_run_seq: 1,
            selected_suite_count: 3,
            suite_result_count: 3,
            blocking_failure_count: 0,
            overall_status: CI_STATUS_PASS,
            tests_status: CI_STATUS_PASS,
            lint_status: CI_STATUS_PASS,
        };
        authority
            .commit_patchset_ci_evidence(0, 4, evidence)
            .unwrap();
        authority
            .commit_patchset_ci_evidence(0, 4, evidence)
            .expect("domain-first retry must accept exact committed evidence");
        let read = BinaryDbReadTxn::new(&db);
        let committed = WorkflowBinaryV0Codec::decode_frozen_patchset(
            &read
                .read_record(WorkflowBinaryV0Codec::patchset_file(), 0)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(committed.ci_worker_job_index_plus1, 5);
        assert_eq!(committed.ci_completed_at_s, 102);
        assert_eq!(committed.ci_selected_suite_count, 3);
        assert_eq!(committed.ci_status_bits, 0b01_01_01);
        drop(read);
        assert_eq!(authority.clear_all_patchset_ci_job_locators().unwrap(), 1);
        let cleared = authority.frozen_patchset_at(0).unwrap();
        assert_eq!(cleared.ci_worker_job_index_plus1, 0);
        assert_eq!(cleared.ci_completed_at_s, 102);
        assert_eq!(cleared.ci_selected_suite_count, 3);
        assert_eq!(cleared.ci_status_bits, 0b01_01_01);
        assert_eq!(authority.clear_all_patchset_ci_job_locators().unwrap(), 0);
        fs::remove_dir_all(root).unwrap();
    }
}
