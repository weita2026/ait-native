use super::*;
#[path = "package_plan.rs"]
mod plan;
use crate::foundation::prepared_ci_runtime::PreparedPackageEvidence;
use crate::foundation::server_operational_job_domain::WorkerJobCompactCiEvidence;
use crate::foundation::server_operational_worker_jobs::PreparedCiPublication;
use crate::foundation::server_operational_worker_jobs::WorkerJobEntry;
use crate::foundation::workflow_package::{PackageHistoryEntry, WorkflowPackageManifest};
use crate::foundation::workflow_package_contract::PackageExpectedIdentity;
use crate::foundation::workflow_package_contract::PackageFinishRequest;

pub struct PreparedWorkflowPackageHistory {
    manifest: WorkflowPackageManifest,
    bindings: Vec<HistoryPromotionPlanBinding>,
    fingerprint: String,
    binding_errors: Vec<String>,
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub fn lookup_workflow_package_finish(
        &self,
        job: WorkerJobEntry,
        request: &PackageFinishRequest,
        _source: Option<&WorkflowPackageManifest>,
    ) -> Result<Option<JsonValue>, String> {
        let Some(index) = job.record.patchset_index_plus1.checked_sub(1) else {
            return Ok(None);
        };
        use crate::foundation::operational_binary_v0::{
            WORKER_JOB_KIND_PATCHSET_CI, WORKER_JOB_OUTCOME_COMPLETED, WORKER_JOB_STATE_SUCCEEDED,
        };
        if self.repository_index != Some(request.expected_identity.repository_index)
            || job.key.repository_index != request.expected_identity.repository_index
            || job.key.worker_job_index != request.expected_identity.worker_job_index
            || job.record.job_kind != WORKER_JOB_KIND_PATCHSET_CI
            || job.record.state_kind != WORKER_JOB_STATE_SUCCEEDED
            || job.record.outcome_kind != WORKER_JOB_OUTCOME_COMPLETED
            || job.record.is_tombstoned()
        {
            return Err("Original package Job is not accepted".into());
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let patchset = self
            .read_patchset(&read, index)
            .map_err(|error| error.to_string())?;
        let Some(manifest) = self.history_manifest_for_patchset(&read, index)? else {
            return Err("Bound Job is not a package history publication".into());
        };
        if manifest["idempotency_key"].as_str() != Some(&request.idempotency_key)
            || patchset.ci_worker_job_index_plus1.checked_sub(1) != Some(job.key.worker_job_index)
            || self.content_snapshot_id(&read, patchset.base_snapshot_index)?
                != request.expected_identity.base_snapshot_id
            || self.content_snapshot_id(&read, patchset.revision_snapshot_index)?
                != request.expected_identity.revision_snapshot_id
        {
            return Err("Accepted original Job/Patchset identity differs".into());
        }
        let (_, land) = self
            .task_successful_receipt(&read, patchset.task_index)?
            .ok_or("Bound original Job has no successful Land")?;
        let patchset_id = self.patchset_public_id(patchset);
        let task_id = self.task_id(patchset.task_index);
        if manifest["target_line"].as_str() != Some(request.target_line.as_str())
            || manifest["base_snapshot_id"].as_str()
                != Some(request.expected_identity.base_snapshot_id.as_str())
            || manifest["revision_snapshot_id"].as_str()
                != Some(request.expected_identity.revision_snapshot_id.as_str())
            || manifest["aggregate"]["patchset_index"].as_u64() != Some(u64::from(index))
            || manifest["aggregate"]["patchset_id"].as_str() != Some(patchset_id.as_str())
            || manifest["aggregate"]["task_id"].as_str() != Some(task_id.as_str())
        {
            return Err("Immutable package aggregate identity differs".into());
        }
        let mode = match request.mode.as_str() {
            "direct" => LAND_MODE_DIRECT,
            "merge" => LAND_MODE_MERGE,
            "ff-only" => LAND_MODE_FF_ONLY,
            _ => return Err("Unsupported package finish mode".into()),
        };
        let (line_index, _) = ServerBinaryDbLineStore::<D, 1>::new(self.db.clone())
            .line_by_name(&read, &request.target_line)
            .map_err(|error| error.to_string())?
            .ok_or("Accepted target Line is missing")?;
        if land.patchset_index != index
            || land.target_line_index_plus1 != line_index + 1
            || (land.land_meta & LAND_MODE_MASK) >> 5 != mode
            || land.pre_land_target_snapshot_index_plus1 != patchset.base_snapshot_index + 1
            || land.landed_snapshot_index_plus1 != patchset.revision_snapshot_index + 1
        {
            return Err("Immutable package Land differs from requested acceptance".into());
        }
        // Existing immutable summaries do not contain the complete Change set
        // or Local Change ownership labels. A caller-supplied manifest cannot
        // prove its omission/substitution against the sealed hash after cleanup.
        // Source assertions cannot erase formal acceptance or establish full
        // mapping membership. Preserve accepted IDs without Local writeback.
        Ok(Some(
            json!({"outcome":"incomplete","expected_identity":request.expected_identity,
            "patchset_id":patchset_id,"submission_id":self.land_id(&task_id,land.land_ordinal),"entries":[]}),
        ))
    }

    /// Resolve a prepared candidate only through its immutable original key
    /// and the same complete Job/Patchset/Land proof used by finish replay.
    pub fn lookup_accepted_workflow_package(
        &self,
        job: WorkerJobEntry,
        package_hash: &str,
        actor_key_index: u32,
    ) -> Result<Option<JsonValue>, String> {
        if self.repository_index != Some(job.key.repository_index) {
            return Err("Original package Job belongs to another Repository".into());
        }
        let Some(index) = job.record.patchset_index_plus1.checked_sub(1) else {
            return Ok(None);
        };
        let read = BinaryDbReadTxn::new(&self.db);
        let Some(manifest) = self.history_manifest_for_patchset(&read, index)? else {
            return Ok(None);
        };
        let key = manifest["idempotency_key"]
            .as_str()
            .ok_or("Formal package acceptance key is unavailable")?;
        if !key.starts_with(&format!("workflow-package:{package_hash}:")) {
            return Ok(None);
        }
        let patchset = self
            .read_patchset(&read, index)
            .map_err(|e| e.to_string())?;
        let identity = PackageExpectedIdentity {
            repository_index: job.key.repository_index,
            package_hash: package_hash.to_string(),
            worker_job_index: job.key.worker_job_index,
            base_snapshot_id: self.content_snapshot_id(&read, patchset.base_snapshot_index)?,
            revision_snapshot_id: self
                .content_snapshot_id(&read, patchset.revision_snapshot_index)?,
        };
        if key != identity.idempotency_key() {
            return Err("Accepted original Job/Patchset identity differs".into());
        }
        let (land_index, _) = self
            .task_successful_receipt(&read, patchset.task_index)?
            .ok_or("Bound original Job has no successful Land")?;
        let land = self.land_at(&read, land_index)?;
        let request = PackageFinishRequest {
            contract: crate::foundation::workflow_package_contract::PACKAGE_FINISH_CONTRACT.into(),
            idempotency_key: identity.idempotency_key(),
            actor_key_index,
            target_line: land["target_line"]
                .as_str()
                .ok_or("Accepted target Line is unavailable")?
                .to_string(),
            mode: land["mode"]
                .as_str()
                .ok_or("Accepted Land mode is unavailable")?
                .to_string(),
            expected_identity: identity,
        };
        drop(read);
        self.lookup_workflow_package_finish(job, &request, None)
    }

    pub fn prepare_workflow_package_history(
        &self,
        manifest: &WorkflowPackageManifest,
        identity: &PackageExpectedIdentity,
    ) -> Result<PreparedWorkflowPackageHistory, String> {
        if self.patchset_codec != PatchsetCodecMode::Frozen {
            return Err("Package finish requires the admitted frozen Patchset codec".into());
        }
        manifest.validate()?;
        if manifest.repository_index != identity.repository_index
            || manifest.base_snapshot_id != identity.base_snapshot_id
            || manifest.revision_snapshot_id != identity.revision_snapshot_id
        {
            return Err("Package history differs from selected execution".into());
        }
        if manifest.history_entries.len() > MAX_HISTORY_PROMOTION_ENTRIES {
            return Err("Package history exceeds the existing batch bound".into());
        }
        let mut binding_errors = Vec::new();
        let bindings = manifest
            .history_entries
            .iter()
            .map(
                |entry| match self.resolve_plan_binding_for_create(&entry.task) {
                    Ok(binding) => binding,
                    Err(error) => {
                        binding_errors.push(format!("{} Plan: {error}", entry.local_task_id));
                        (0, 0, 0)
                    }
                },
            )
            .collect();
        // Preserve the existing history request fingerprint representation. It
        // is not a durable authority for the compressed package hash.
        let entries = manifest.history_entries.iter().map(|entry| json!({
            "local_task_id":entry.local_task_id,"local_change_id":null,"local_change_ref":null,
            "expected_remote_task_id":entry.expected_remote_task_id,"expected_remote_change_ref":null,
            "task":entry.task,"change":null,"pre_land_target_snapshot_id":entry.pre_land_target_snapshot_id,
            "landed_snapshot_id":entry.landed_snapshot_id,"landed_at_s":entry.landed_at_s,
            "snapshots":entry.snapshots.iter().map(|snapshot|json!({"snapshot_id":snapshot.snapshot_id,"created_at_s":snapshot.created_at_s})).collect::<Vec<_>>()
        })).collect::<Vec<_>>();
        let fingerprint = request_sha256(
            &json!({"contract":HISTORY_PROMOTION_CONTRACT,"idempotency_key":identity.idempotency_key(),
            "target_line":manifest.target_line,"base_snapshot_id":manifest.base_snapshot_id,"revision_snapshot_id":manifest.revision_snapshot_id,
            "author_mode":manifest.author_mode,"summary":manifest.summary,"entries":entries}),
        )?;
        Ok(PreparedWorkflowPackageHistory {
            manifest: manifest.clone(),
            bindings,
            fingerprint,
            binding_errors,
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn package_reusable_receipt<A: ReadV0>(
        &self,
        tx: &A,
        entry: &PackageHistoryEntry,
        binding: HistoryPromotionPlanBinding,
        target: &str,
        endpoints: (u32, u32),
        task_index: u32,
        selected_links: &BTreeMap<u32, V0SnapshotLinkRecord>,
    ) -> Result<Option<(WorkflowOwner, u32, String, JsonValue)>, String> {
        let mut found = None;
        let inventory = self.task_inventory(tx, "task_patchset_index.bin", task_index)?;
        let mut cursor = inventory.latest_index_plus1.checked_sub(1);
        let mut seen = BTreeSet::new();
        while let Some(index) = cursor {
            if !seen.insert(index) {
                return Err("Source receipt Patchset cycle".into());
            }
            let patchset = self
                .read_patchset(tx, index)
                .map_err(|error| error.to_string())?;
            if patchset.task_index != task_index {
                return Err("Source receipt Task inventory differs".into());
            }
            cursor = patchset.previous_task_patchset_index_plus1.checked_sub(1);
            let Some(value) = parse_summary(
                &self.history_patchset_summary(tx, index)?,
                LOCAL_LAND_RECEIPT_SUMMARY_PREFIX,
            )?
            else {
                continue;
            };
            let mut receipt: PersistedLocalLandReceipt =
                serde_json::from_value(value.clone()).map_err(|error| error.to_string())?;
            if receipt.contract == "ait-local-land-receipt/v1" {
                receipt = self
                    .legacy_local_receipt_view_with_links(
                        tx,
                        index,
                        entry.expected_remote_task_id.as_deref().ok_or(
                            "Legacy source reuse requires its explicit Remote Task mapping",
                        )?,
                        Some(selected_links),
                    )?
                    .ok_or("Legacy receipt is unavailable")?;
            }
            let owner = self.patchset_owner(tx, patchset)?;
            let task = self
                .read_task(tx, owner.task_index)
                .map_err(|error| error.to_string())?;
            let payload = WorkflowBinaryV0Codec::decode_task_payload(
                &tx.read_payload_v0(
                    WorkflowBinaryV0Codec::task_payload_file(),
                    task.payload_offset,
                    u32::from(task.payload_len),
                )
                .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?;
            if receipt.task_id != self.task_id(owner.task_index)
                || entry
                    .expected_remote_task_id
                    .as_ref()
                    .is_some_and(|id| id != &receipt.task_id)
                || receipt.target_line != target
                || receipt.source_kind != "imported_local_land_receipt"
                || receipt.pre_land_target_snapshot_id != entry.pre_land_target_snapshot_id
                || receipt.landed_snapshot_id != entry.landed_snapshot_id
                || receipt.landed_at_s != entry.landed_at_s
                || patchset.base_snapshot_index != endpoints.0
                || patchset.revision_snapshot_index != endpoints.1
                || patchset.created_at_s != entry.landed_at_s
                || patchset.ci_run_seq != 0
                || task.origin_plan_revision_index_plus1 != binding.0
                || task.plan_item_index_plus1 != binding.1
                || payload.title
                    != entry.task["title"]
                        .as_str()
                        .ok_or("Source Task title is missing")?
                        .trim()
                || payload.intent
                    != entry.task["intent"]
                        .as_str()
                        .ok_or("Source Task intent is missing")?
                        .trim()
            {
                return Err(
                    "Package source Task conflicts with its immutable published Local receipt"
                        .into(),
                );
            }
            if found
                .replace((owner, index, self.patchset_public_id(patchset), value))
                .is_some()
            {
                return Err("Package source Task has ambiguous original receipts".into());
            }
        }
        if seen.len() != usize::from(inventory.count) {
            return Err("Source receipt inventory count differs".into());
        }
        Ok(found)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn publish_workflow_package_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        prepared: PreparedWorkflowPackageHistory,
        identity: &PackageExpectedIdentity,
        actor_key_index: u32,
        ci: WorkerJobCompactCiEvidence,
        actions: &[PreparedPackageEvidence],
        mode: &str,
    ) -> Result<PreparedCiPublication<JsonValue>, String> {
        let actor = self.request_actor_in_write(
            tx,
            json!({"actor_key_index":actor_key_index})
                .as_object()
                .expect("Actor input"),
        )?;
        let package = &prepared.manifest;
        if package.repository_index != identity.repository_index
            || package.base_snapshot_id != identity.base_snapshot_id
            || package.revision_snapshot_id != identity.revision_snapshot_id
        {
            return Err("Publication package execution differs".into());
        }
        let now = Self::now_s()?;
        let (_, line) = ServerBinaryDbLineStore::<D, 1>::new(self.db.clone())
            .line_by_name_in_write(tx, &package.target_line)
            .map_err(|error| error.to_string())?
            .ok_or("Package target Line is missing")?;
        let tested_base = self.snapshot_index_in_history_write(
            tx,
            &package.base_snapshot_id,
            "tested package base",
        )?;
        let tested_revision = self.snapshot_index_in_history_write(
            tx,
            &package.revision_snapshot_id,
            "tested package revision",
        )?;
        if line.head_snapshot_index() != Some(tested_base) {
            return Err("STALE_TARGET_LINE: tested package base moved".into());
        }
        let conversion = self.convert_package(tx, &prepared)?;
        let attestations = actions.iter().filter(|a| a.request.operation == crate::foundation::workflow_package_contract::PackageEvidenceOperation::Attestation).count();
        if attestations > conversion.attestation_slots {
            return Err("Attestation64/Task capacity exhausted".into());
        }
        self.prepare_history_promotion_write_set(tx)?;
        tx.prepare_write_set(
            &[ServerBinaryLineCodec::<1>::record_file()],
            &[ServerBinaryLineCodec::<1>::payload_file()],
            &[ServerBinaryLineCodec::<1>::name_index()],
        )
        .map_err(|e| e.to_string())?;
        let lines = ServerBinaryDbLineStore::<D, 1>::new(self.db.clone());
        let mut receipts = Vec::new();
        let mut mappings = Vec::new();
        let mut final_task = None;
        for ((entry, &binding), resolved) in package
            .history_entries
            .iter()
            .zip(&prepared.bindings)
            .zip(conversion.entries)
        {
            let original_base = self.snapshot_index_in_history_write(
                tx,
                &entry.pre_land_target_snapshot_id,
                "original receipt base",
            )?;
            let original_revision = self.snapshot_index_in_history_write(
                tx,
                &entry.landed_snapshot_id,
                "original receipt revision",
            )?;
            let (receipt_owner, receipt_index, receipt_id, local_receipt) = if let Some(receipt) =
                resolved.receipt
            {
                receipt
            } else {
                let (task_index, task_id) = self.append_task_in_write(
                    tx,
                    &entry.task,
                    binding.0,
                    binding.1,
                    binding.2,
                    now,
                    PayloadSyncBoundary::TransactionCommit,
                    0,
                )?;
                if task_index != resolved.task_index {
                    return Err("Symbolic Task allocation changed under publication lock".into());
                }
                let owner = WorkflowOwner {
                    task_index,
                    change: None,
                };
                let local = json!({"contract":"ait-local-land-receipt/v2","local_task_id":entry.local_task_id,"local_change_id":null,"local_change_ref":null,
                    "task_id":task_id,"change_ref":null,"target_line":package.target_line,"pre_land_target_snapshot_id":entry.pre_land_target_snapshot_id,
                    "landed_snapshot_id":entry.landed_snapshot_id,"landed_at_s":entry.landed_at_s,"source_kind":"imported_local_land_receipt"});
                let (index, id) = self.append_history_patchset_in_write(
                    tx,
                    owner,
                    original_base,
                    original_revision,
                    LOCAL_LAND_RECEIPT_SUMMARY_PREFIX,
                    &local,
                    &package.author_mode,
                    entry.landed_at_s,
                    0,
                )?;
                (owner, index, id, local)
            };
            let task_index = receipt_owner.task_index;
            let mut changes = BTreeMap::new();
            let mut change_mappings = Vec::new();
            let mut newly_materialized = BTreeMap::new();
            for (change, resolved) in entry.changes.iter().zip(resolved.changes) {
                let owner = if let Some(owner) = resolved.existing {
                    owner
                } else {
                    let fork = self.snapshot_index_in_history_write(
                        tx,
                        &resolved.fork,
                        "projected Change fork",
                    )?;
                    if lines
                        .line_by_name_in_write(tx, &resolved.base)
                        .map_err(|e| e.to_string())?
                        .is_none()
                    {
                        lines
                            .create_line_in_tx(tx, &resolved.base, fork + 1, now)
                            .map_err(|e| e.to_string())?;
                    }
                    lines
                        .materialize_package_line_in_tx(tx, &resolved.base, fork, now, false)
                        .map_err(|e| e.to_string())?;
                    let mut payload = change.change.clone();
                    payload.insert("base_line".into(), json!(resolved.base));
                    let (index, _) = self.append_change_with_fork_in_write(
                        tx,
                        &payload,
                        task_index,
                        &self.task_id(task_index),
                        now,
                        Some(fork + 1),
                        PayloadSyncBoundary::PackageMaterialization,
                        0,
                    )?;
                    newly_materialized.insert(resolved.base, fork);
                    WorkflowOwner {
                        task_index,
                        change: Some((
                            index,
                            self.read_change(tx, index).map_err(|e| e.to_string())?,
                        )),
                    }
                };
                change_mappings.push(json!({"local_change_ref":change.local_change_ref,"remote_change_ref":self.owner_ref(owner)}));
                changes.insert(change.local_change_ref.as_str(), owner);
            }
            for (name, fork) in newly_materialized {
                lines
                    .materialize_package_line_in_tx(tx, &name, fork, now, true)
                    .map_err(|e| e.to_string())?;
            }
            for snapshot in &entry.snapshots {
                let index = self.snapshot_index_in_history_write(
                    tx,
                    &snapshot.snapshot_id,
                    "source owned Snapshot",
                )?;
                let owner = match snapshot.local_change_ref.as_deref() {
                    None => WorkflowOwner {
                        task_index,
                        change: None,
                    },
                    Some(r) => *changes.get(r).ok_or("Source Snapshot Change missing")?,
                };
                if let Some(existing) = conversion.owners.get(&snapshot.snapshot_id) {
                    if existing.task_index != owner.task_index
                        || existing.change.map(|(i, _)| i) != owner.change.map(|(i, _)| i)
                    {
                        return Err("Source Snapshot exact numeric owner conflicts".into());
                    }
                } else {
                    self.append_numeric_snapshot_link(tx, owner, index, 0)?;
                }
            }
            receipts.push(json!({"local_task_id":entry.local_task_id,"local_change_id":local_receipt["local_change_id"],"local_change_ref":local_receipt["local_change_ref"],
                "task_index":task_index,"task_id":self.task_id(task_index),"change_index":receipt_owner.change.map(|(i,_)|i),"change_ref":receipt_owner.change.map(|(_,r)|self.change_ref(task_index,r.change_ordinal)),
                "receipt_patchset_index":receipt_index,"receipt_patchset_id":receipt_id,"pre_land_target_snapshot_id":entry.pre_land_target_snapshot_id,"landed_snapshot_id":entry.landed_snapshot_id,
                "landed_at_s":entry.landed_at_s,"snapshot_link_count":entry.snapshots.len()}));
            mappings.push(json!({"local_task_id":entry.local_task_id,"remote_task_id":self.task_id(task_index),"changes":change_mappings}));
            final_task = Some(WorkflowOwner {
                task_index,
                change: None,
            });
        }
        let owner = final_task.ok_or("Package has no final source Task")?;
        self.require_active_owner(tx, owner)?;
        let index = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|error| error.to_string())?;
        let ordinal = self.next_task_patchset_ordinal(tx, owner.task_index)?;
        let patchset_id = self.patchset_public_id(V0PatchsetRecord {
            task_index: owner.task_index,
            patch_ordinal: ordinal,
            ..V0PatchsetRecord::default()
        });
        let manifest = json!({"contract":"ait-history-promotion/v3","idempotency_key":identity.idempotency_key(),"request_sha256":prepared.fingerprint,
            "target_line":package.target_line,"base_snapshot_id":package.base_snapshot_id,"revision_snapshot_id":package.revision_snapshot_id,"display_summary":package.summary,"entries":receipts,
            "aggregate":{"task_index":owner.task_index,"task_id":self.task_id(owner.task_index),"change_index":null,"change_ref":null,"patchset_index":index,"patchset_id":patchset_id}});
        let (actual, id) = self.append_history_patchset_in_write(
            tx,
            owner,
            tested_base,
            tested_revision,
            HISTORY_PROMOTION_SUMMARY_PREFIX,
            &manifest,
            &package.author_mode,
            now,
            actor,
        )?;
        if actual != index || id != patchset_id {
            return Err("Package aggregate identity drifted".into());
        }
        let mut patchset = self
            .read_patchset(tx, index)
            .map_err(|error| error.to_string())?;
        patchset.ci_worker_job_index_plus1 = identity
            .worker_job_index
            .checked_add(1)
            .ok_or("Worker Job index exceeds bounds")?;
        patchset.ci_completed_at_s = ci.ci_completed_at_s;
        patchset.ci_run_seq = ci.ci_run_seq;
        patchset.ci_selected_suite_count = ci.selected_suite_count;
        patchset.ci_suite_result_count = ci.suite_result_count;
        patchset.ci_blocking_failure_count = ci.blocking_failure_count;
        patchset.ci_status_bits = ci.status_bits().map_err(|error| error.to_string())?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            index,
            &WorkflowBinaryV0Codec::encode_frozen_patchset(
                Self::frozen_patchset_from_logical(patchset, patchset.ci_worker_job_index_plus1)
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let owner_ref = self.owner_ref(owner);
        self.transfer_package_evidence_in_write(
            tx,
            &owner_ref,
            &patchset_id,
            &package.author_mode,
            actions,
        )?;
        self.evaluate_policy_in_write(tx, &patchset_id, now)?;
        let formal = json!({"contract":"task-land-atomic-guarded/v2","actor_key_index":actor_key_index,"idempotency_key":identity.idempotency_key(),"task_or_change_ref":owner_ref,
            "target_line":package.target_line,"mode":mode,"patchset_id":patchset_id,"expected_head_snapshot_id":package.base_snapshot_id,
            "expected_identity":{"repository_index":identity.repository_index,"patchset_id":patchset_id,"base_snapshot_id":identity.base_snapshot_id,"revision_snapshot_id":identity.revision_snapshot_id}});
        let (_, land_index, _) = self.submit_land_in_write(tx, &owner_ref, &formal, true)?;
        let land = WorkflowBinaryV0Codec::decode_land(
            &tx.read_record(WorkflowBinaryV0Codec::land_file(), land_index)
                .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(PreparedCiPublication {
            result: json!({"outcome":"confirmed","expected_identity":identity,"patchset_id":patchset_id,"submission_id":self.land_id(&self.task_id(owner.task_index),land.land_ordinal),"entries":mappings}),
            patchset_index: index,
            land_index,
        })
    }
}
