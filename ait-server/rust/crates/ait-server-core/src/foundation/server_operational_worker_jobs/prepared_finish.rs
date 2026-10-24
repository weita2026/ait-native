use super::*;
use crate::foundation::remote_binary_db::{
    BinaryDbCommandScope, BinaryDbCommitOutcome, BinaryDbIndexAppender, BinaryDbStoreFsyncPolicy,
    BinaryDbWriteTxn, ServerRemoteBinaryDb,
};
use crate::foundation::server_content_binary_db::{
    ServerBinaryDbLineStore, ServerBinaryDbSnapshotStore, SERVER_CONTENT_BINARY_LAYOUT_ID,
};
use crate::foundation::workflow_binary_v0::{
    WorkflowBinaryV0Codec, LAND_STATUS_MASK, LAND_STATUS_SUCCEEDED,
};

pub struct PreparedCiFinishContext {
    pub job: WorkerJobEntry,
    pub source: ManagedCiPackage,
    pub evidence: WorkerJobCompactCiEvidence,
    pub actions: Vec<crate::foundation::prepared_ci_runtime::PreparedPackageEvidence>,
}

pub struct PreparedCiPublication<T> {
    pub result: T,
    pub patchset_index: u32,
    pub land_index: u32,
}

/// The journal commit has succeeded even when lock cleanup reports a warning.
/// The caller may clean this source only after receiving this value.
pub struct PreparedCiCommitted<T> {
    pub result: T,
    pub commit: BinaryDbCommitOutcome,
    pub queue_lock_cleanup_warning: Option<BinaryDbError>,
    pub source: ManagedCiPackage,
}

impl ServerOperationalWorkerJobStore {
    /// Recover the existing composite journal before a request reads either
    /// side of a possibly interrupted Job/Patchset binding.
    pub fn recover_prepared_ci_publication<D>(&self, db: &D) -> StoreResult<()>
    where
        D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
    {
        let journal = self.authority_root.join(LAND_PUBLICATION_JOURNAL_FILE_NAME);
        if !journal
            .try_exists()
            .map_err(|error| BinaryDbError::io("inspect pending package publication", error))?
        {
            return Ok(());
        }
        self.validate_root_paths()?;
        if fs::canonicalize(ServerRemoteBinaryDb::authority_root(db).as_path())
            .map_err(|error| BinaryDbError::io("resolve recovery DB", error))?
            != fs::canonicalize(&self.authority_root)
                .map_err(|error| BinaryDbError::io("resolve recovery Job authority", error))?
        {
            return Err(invalid("Publication recovery authorities differ"));
        }
        let mut lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.recover_job_replacement()?;
        let mut tx = BinaryDbWriteTxn::begin_serving(db, BinaryDbCommandScope::ServerLand)?;
        tx.abort()?;
        lock.clear_contents_and_flush()?;
        Ok(())
    }

    /// Stage the complete publication and bind its original CI Job under the
    /// existing Land journal. The callback must stage writes without committing.
    pub fn finish_prepared_ci<'a, D, T>(
        &self,
        db: &'a D,
        worker_job_index: u32,
        hash: [u8; 32],
        publish: impl FnOnce(
            &mut BinaryDbWriteTxn<'a, D, BinaryDbStoreFsyncPolicy<'a, D>>,
            &PreparedCiFinishContext,
        ) -> StoreResult<PreparedCiPublication<T>>,
    ) -> StoreResult<PreparedCiCommitted<T>>
    where
        D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
    {
        self.validate_root_paths()?;
        if fs::canonicalize(ServerRemoteBinaryDb::authority_root(db).as_path())
            .map_err(|error| BinaryDbError::io("resolve finish DB authority", error))?
            != fs::canonicalize(&self.authority_root)
                .map_err(|error| BinaryDbError::io("resolve finish Job authority", error))?
        {
            return Err(invalid("prepared finish DB and Job authorities differ"));
        }
        let mut queue_lock = self.acquire_queue_lock(ServerBinaryDbLockMode::Exclusive)?;
        self.recover_job_replacement()?;
        // Beginning this scope also recovers any interrupted publication before
        // either the Job or its possibly-uncommitted Patchset is observed.
        let mut tx = BinaryDbWriteTxn::begin_serving(db, BinaryDbCommandScope::ServerLand)?;
        for (key, hash) in self.prepared_ci.execution_hashes(self.repository_index)? {
            let actual = ServerOperationalBinaryV0Codec::decode_worker_job(&tx.read_record(
                ServerOperationalBinaryV0Codec::worker_job_file(),
                key.worker_job_index,
            )?)?;
            if actual.is_tombstoned() || actual.state_kind == WORKER_JOB_STATE_FAILED {
                self.prepared_ci.release_priority(key, hash)?;
            }
        }

        let entry = WorkerJobEntry {
            key: WorkerJobKey {
                repository_index: self.repository_index,
                worker_job_index,
            },
            record: ServerOperationalBinaryV0Codec::decode_worker_job(&tx.read_record(
                ServerOperationalBinaryV0Codec::worker_job_file(),
                worker_job_index,
            )?)?,
        };
        if entry.record.is_tombstoned()
            || entry.record.job_kind != WORKER_JOB_KIND_PATCHSET_CI
            || entry.record.patchset_index_plus1 != 0
            || entry.record.state_kind != WORKER_JOB_STATE_SUCCEEDED
            || entry.record.outcome_kind != WORKER_JOB_OUTCOME_COMPLETED
        {
            return Err(invalid(
                "prepared finish requires its successful original unbound kind7 Job",
            ));
        }
        self.prepared_ci.with_finish_candidate(entry.key, hash, |source, evidence, actions| {
            source.verify()?;
            let context = PreparedCiFinishContext { job: entry, source: source.clone(), evidence, actions: actions.to_vec() };
            let lines = ServerBinaryDbLineStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
            let snapshots = ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(db.clone());
            let (target_index, line) = lines.line_by_name_in_write(&tx, &source.target().target_line)?
                .ok_or_else(|| invalid("prepared finish target Line is missing"))?;
            let base = line.head_snapshot_index().ok_or_else(|| invalid("prepared finish target has no base"))?;
            if snapshots.snapshot_id_at_in_write(&tx, base)? != source.target().base_snapshot_id {
                return Err(invalid("prepared finish target no longer equals the tested base"));
            }
            let land_count = tx.record_count(WorkflowBinaryV0Codec::land_file())?;
            let publication = publish(&mut tx, &context)?;
            let patchset = WorkflowBinaryV0Codec::decode_frozen_patchset(
                &tx.read_record(WorkflowBinaryV0Codec::patchset_file(), publication.patchset_index)?,
            )?;
            if patchset.ci_worker_job_index_plus1 != worker_job_index.checked_add(1).ok_or_else(|| invalid("CI Job index exceeds plus-one bounds"))?
                || patchset.ci_completed_at_s != evidence.ci_completed_at_s
                || patchset.ci_run_seq != evidence.ci_run_seq
                || patchset.ci_selected_suite_count != evidence.selected_suite_count
                || patchset.ci_suite_result_count != evidence.suite_result_count
                || patchset.ci_blocking_failure_count != evidence.blocking_failure_count
                || patchset.ci_status_bits != evidence.status_bits()?
                || snapshots.snapshot_id_at_in_write(&tx, patchset.base_snapshot_index)? != source.target().base_snapshot_id
                || snapshots.snapshot_id_at_in_write(&tx, patchset.revision_snapshot_index)? != source.target().revision_snapshot_id
            {
                return Err(invalid("prepared finish Patchset differs from the original tested execution"));
            }
            let land = WorkflowBinaryV0Codec::decode_land(
                &tx.read_record(WorkflowBinaryV0Codec::land_file(), publication.land_index)?,
            )?;
            let (_, final_line) = lines.line_by_name_in_write(&tx, &source.target().target_line)?
                .ok_or_else(|| invalid("prepared finish target Line disappeared"))?;
            if publication.land_index != land_count
                || tx.record_count(WorkflowBinaryV0Codec::land_file())? != land_count.checked_add(1).ok_or_else(|| invalid("Land inventory exhausted"))?
                || land.land_meta & LAND_STATUS_MASK != LAND_STATUS_SUCCEEDED
                || land.task_index != patchset.task_index
                || land.patchset_index != publication.patchset_index
                || land.target_line_index_plus1 != target_index + 1
                || land.pre_land_target_snapshot_index_plus1 != base + 1
                || land.landed_snapshot_index_plus1 != patchset.revision_snapshot_index + 1
                || final_line.head_snapshot_index() != Some(patchset.revision_snapshot_index)
            {
                return Err(invalid("prepared finish must publish exactly one real successful Land and its target head"));
            }
            let current = ServerOperationalBinaryV0Codec::decode_worker_job(
                &tx.read_record(ServerOperationalBinaryV0Codec::worker_job_file(), worker_job_index)?,
            )?;
            if current != entry.record { return Err(invalid("prepared finish changed the original CI Job")); }
            let bound = ServerWorkerJobRecord {
                patchset_index_plus1: publication.patchset_index.checked_add(1).ok_or_else(|| invalid("Patchset index exceeds plus-one bounds"))?,
                ..current
            };
            tx.overwrite_record(ServerOperationalBinaryV0Codec::worker_job_file(), worker_job_index,
                &ServerOperationalBinaryV0Codec::encode_worker_job(bound)?)?;
            let commit = tx.commit()?;
            // No ordinary error may be returned after the durable commit point.
            let queue_lock_cleanup_warning = queue_lock.clear_contents_and_flush().err();
            Ok(PreparedCiCommitted { result: publication.result, commit,
                queue_lock_cleanup_warning, source: source.clone() })
        })
    }
}
