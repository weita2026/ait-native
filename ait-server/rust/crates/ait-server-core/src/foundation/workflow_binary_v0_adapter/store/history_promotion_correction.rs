use super::*;

pub(super) const CORRECTION_PREPARE_CONTRACT: &str = "history-promotion-correction-prepare/v2";
const CORRECTION_MANIFEST_CONTRACT: &str = "ait-history-promotion-correction/v2";

fn receipt_entries_sha256(entries: &JsonValue) -> Result<String, String> {
    let bytes = serde_json::to_vec(entries).map_err(|e| e.to_string())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CorrectionRequest {
    contract: String,
    idempotency_key: String,
    expected_identity: JsonValue,
    expected_receipts_sha256: String,
    target_line: String,
    revision_snapshot_id: String,
    author_mode: String,
    summary: String,
    // Complete cumulative DAG difference from the original receipt terminal.
    snapshots: Vec<HistoryPromotionSnapshot>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(super) struct HistoryCorrection {
    original_patchset_id: String,
    prior_patchset_id: String,
    receipt_revision_snapshot_id: String,
    snapshots: Vec<HistoryPromotionSnapshot>,
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(in super::super) fn latest_history_manifest_for_task<A: ReadV0>(
        &self,
        read: &A,
        task_index: u32,
    ) -> Result<Option<JsonValue>, String> {
        let mut cursor = self
            .task_inventory(read, "task_patchset_index.bin", task_index)?
            .latest_index_plus1
            .checked_sub(1);
        let mut visited = BTreeSet::new();
        while let Some(index) = cursor {
            if !visited.insert(index) {
                return Err("Aggregate Task Patchset lineage has a cycle".to_string());
            }
            let patchset = self.read_patchset(read, index).map_err(|e| e.to_string())?;
            if patchset.task_index != task_index {
                return Err("Aggregate Task lineage has a foreign owner".to_string());
            }
            let summary = self.history_patchset_summary(read, index)?;
            for prefix in [
                HISTORY_PROMOTION_SUMMARY_PREFIX,
                HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX,
                LOCAL_LAND_RECEIPT_SUMMARY_PREFIX,
            ] {
                if let Some(value) = parse_summary(&summary, prefix)? {
                    return Ok(Some(value));
                }
            }
            cursor = patchset.previous_task_patchset_index_plus1.checked_sub(1);
        }
        Ok(None)
    }

    fn original_correction_manifest<A: ReadV0>(
        &self,
        read: &A,
        manifest: &PersistedHistoryManifest,
    ) -> Result<(JsonValue, PersistedHistoryManifest), String> {
        let original_id = manifest
            .correction
            .as_ref()
            .ok_or_else(|| {
                "Aggregate correction manifest has no correction authority.".to_string()
            })?
            .original_patchset_id
            .as_str();
        let index = self.patchset_index_for_id(read, original_id)?;
        let value = self
            .history_manifest_for_patchset(read, index)?
            .ok_or_else(|| "Aggregate correction original manifest is missing.".to_string())?;
        let original: PersistedHistoryManifest = serde_json::from_value(value.clone())
            .map_err(|e| format!("Invalid original aggregate manifest: {e}"))?;
        let patchset = self
            .read_patchset(read, index)
            .map_err(|e| Self::error("original aggregate Patchset", e))?;
        let owner = self.patchset_owner(read, patchset)?;
        if original.contract != "ait-history-promotion/v3"
            || original.correction.is_some()
            || original.aggregate.patchset_index != index
            || original.aggregate.patchset_id != original_id
            || self.patchset_public_id(patchset) != original_id
            || owner.task_index != manifest.aggregate.task_index
            || owner.change.map(|(index, _)| index) != original.aggregate.change_index
            || original.aggregate.change_ref
                != owner
                    .change
                    .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal))
            || original.aggregate.task_index != manifest.aggregate.task_index
            || original.aggregate.task_id != manifest.aggregate.task_id
            || original.target_line != manifest.target_line
            || original.base_snapshot_id != manifest.base_snapshot_id
            || self.content_snapshot_id(read, patchset.base_snapshot_index)?
                != original.base_snapshot_id
            || self.content_snapshot_id(read, patchset.revision_snapshot_index)?
                != original.revision_snapshot_id
        {
            return Err(
                "Aggregate correction does not preserve its original authority.".to_string(),
            );
        }
        Ok((value, original))
    }

    fn correction_owner<A: ReadV0>(
        &self,
        read: &A,
        prior: V0PatchsetRecord,
        revision: u32,
    ) -> Result<WorkflowOwner, String> {
        let old = self.patchset_owner(read, prior)?;
        let owner = self
            .numeric_snapshot_owner_with_access(read, revision)?
            .ok_or("Correction revision lacks durable capture ownership through an exact numeric Snapshot Link")?;
        if owner.task_index != old.task_index {
            return Err("Correction revision has foreign Task ownership".to_string());
        }
        self.require_active_owner(read, owner)?;
        if owner.change.map(|(index, _)| index) != old.change.map(|(index, _)| index) {
            let Some((_, change)) = owner.change else {
                return Err(
                    "Correction cannot discard an existing real Change association".to_string(),
                );
            };
            if change.fork_snapshot_index_plus1 != prior.revision_snapshot_index + 1 {
                return Err(
                    "New correction Change must fork at the exact reviewed prior candidate"
                        .to_string(),
                );
            }
        }
        Ok(owner)
    }

    fn correction_snapshot_links<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &BinaryDbWriteTxn<'_, D, F>,
        dag: &HistorySnapshotDag,
        manifest_value: &JsonValue,
        manifest: &PersistedHistoryManifest,
    ) -> Result<Vec<(u32, HistoryPromotionSnapshot)>, String> {
        let correction = manifest
            .correction
            .as_ref()
            .ok_or_else(|| "Aggregate correction range is missing.".to_string())?;
        let (original_value, original) = self.original_correction_manifest(tx, manifest)?;
        if manifest_value.get("entries") != original_value.get("entries")
            || correction.receipt_revision_snapshot_id != original.revision_snapshot_id
        {
            return Err("Aggregate correction rewrites immutable history receipts.".to_string());
        }
        let base = self.snapshot_index_in_history_write(
            tx,
            &original.revision_snapshot_id,
            "correction base",
        )?;
        let revision = self.snapshot_index_in_history_write(
            tx,
            &manifest.revision_snapshot_id,
            "correction revision",
        )?;
        let prior_index = self.patchset_index_for_id(tx, &correction.prior_patchset_id)?;
        let prior = self
            .read_patchset(tx, prior_index)
            .map_err(|e| e.to_string())?;
        let owner = self.correction_owner(tx, prior, revision)?;
        if owner.task_index != manifest.aggregate.task_index
            || owner.change.map(|(index, _)| index) != manifest.aggregate.change_index
            || owner
                .change
                .map(|(_, change)| self.change_ref(owner.task_index, change.change_ordinal))
                != manifest.aggregate.change_ref
        {
            return Err(
                "Correction aggregate optional Change differs from captured revision ownership"
                    .to_string(),
            );
        }
        let fork_scope = owner
            .change
            .map(|(_, change)| {
                let fork = change
                    .fork_snapshot_index_plus1
                    .checked_sub(1)
                    .ok_or("Correction Change has no reviewed fork")?;
                let scope = self.history_snapshot_ancestor_closure(tx, dag, fork)?;
                if !self
                    .history_snapshot_ancestor_closure(tx, dag, revision)?
                    .contains(&fork)
                {
                    return Err("Correction revision does not contain its Change fork".to_string());
                }
                Ok::<_, String>(scope)
            })
            .transpose()?;
        let resolved = self.validate_history_snapshot_links_range(
            tx,
            dag,
            HistorySnapshotRange {
                owner_ref: manifest
                    .aggregate
                    .change_ref
                    .as_deref()
                    .unwrap_or(&manifest.aggregate.task_id),
                base_snapshot_id: &original.revision_snapshot_id,
                revision_snapshot_id: &manifest.revision_snapshot_id,
                snapshots: &correction.snapshots,
            },
            base,
            revision,
            &mut BTreeSet::new(),
        )?;
        let wanted = resolved
            .iter()
            .map(|(index, _)| *index)
            .collect::<BTreeSet<_>>();
        let mut linked = BTreeSet::new();
        let count = tx
            .record_count(WorkflowBinaryV0Codec::snapshot_link_file())
            .map_err(|e| Self::error("correction Snapshot Link inventory", e))?;
        for index in 0..count {
            let link = WorkflowBinaryV0Codec::decode_snapshot_link(
                &tx.read_record(WorkflowBinaryV0Codec::snapshot_link_file(), index)
                    .map_err(|e| Self::error("correction Snapshot Link", e))?,
            )
            .map_err(|e| Self::error("correction Snapshot Link", e))?;
            if !wanted.contains(&link.content_snapshot_index) {
                continue;
            }
            let expected = resolved
                .iter()
                .find(|(i, _)| *i == link.content_snapshot_index)
                .unwrap()
                .1;
            if self
                .capture_binding_with_access(tx, link.content_snapshot_index)?
                .is_none()
            {
                return Err(
                    "Aggregate correction Snapshot is missing durable capture ownership."
                        .to_string(),
                );
            }
            if link.link_meta & SNAPSHOT_LINK_TOMBSTONE != 0
                || link.task_index != manifest.aggregate.task_index
                || !fork_scope
                    .as_ref()
                    .is_some_and(|scope| scope.contains(&link.content_snapshot_index))
                    && link.change_index_plus1
                        != manifest.aggregate.change_index.map_or(0, |index| index + 1)
                || link.created_at_s != expected.created_at_s
                || !linked.insert(link.content_snapshot_index)
            {
                return Err(
                    "Aggregate correction Snapshot has foreign, duplicate or retired ownership."
                        .to_string(),
                );
            }
        }
        for (index, snapshot) in &resolved {
            if dag.snapshot_records[index].created_at_s != snapshot.created_at_s {
                return Err(
                    "Aggregate correction Snapshot creation time differs from content authority."
                        .to_string(),
                );
            }
        }
        if linked.len() != wanted.len() {
            return Err("Aggregate correction requires existing durable capture ownership for every Snapshot.".to_string());
        }
        Ok(resolved
            .into_iter()
            .filter(|(index, _)| !linked.contains(index))
            .map(|(index, snapshot)| (index, snapshot.clone()))
            .collect())
    }

    pub(super) fn apply_history_correction_receipts<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        manifest_value: &JsonValue,
        manifest: &PersistedHistoryManifest,
        _target_line_index: u32,
    ) -> Result<(), String> {
        let dag = self.load_history_snapshot_dag(tx)?;
        let missing = self.correction_snapshot_links(tx, &dag, manifest_value, manifest)?;
        if !missing.is_empty() {
            return Err(
                "Aggregate correction Snapshot links are incomplete at finish.".to_string(),
            );
        }
        let correction = manifest.correction.as_ref().unwrap();
        self.validate_history_receipt_batch(
            tx,
            &manifest.entries,
            &manifest.base_snapshot_id,
            &correction.receipt_revision_snapshot_id,
        )?;
        Ok(())
    }

    pub(super) fn prepare_history_correction_from_payload(
        &self,
        repo_name: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.repo_scope("prepare aggregate correction", repo_name)?;
        let request: CorrectionRequest =
            serde_json::from_value(super::super::actors::domain_request(payload)?)
                .map_err(|e| format!("Invalid aggregate correction request: {e}"))?;
        if request.contract != CORRECTION_PREPARE_CONTRACT {
            return Err("Unsupported aggregate correction contract.".to_string());
        }
        for (text, label) in [
            (&request.idempotency_key, "idempotency key"),
            (&request.target_line, "target Line"),
            (&request.revision_snapshot_id, "revision"),
            (&request.summary, "summary"),
        ] {
            nonempty(text, label)?;
        }
        if request.idempotency_key.len() > 256 {
            return Err("Aggregate correction idempotency key exceeds 256 bytes.".to_string());
        }
        Self::author_mode_bits(&request.author_mode)?;
        let expected_repository = request.expected_identity["repository_index"]
            .as_u64()
            .ok_or_else(|| {
                "Aggregate correction requires numeric Repository identity.".to_string()
            })?;
        if Some(expected_repository) != self.repository_index.map(u64::from) {
            return Err(
                "EXPECTED_CANDIDATE_MISMATCH: numeric Repository authority differs.".to_string(),
            );
        }
        let prior_id = request
            .expected_identity
            .get("patchset_id")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                "Aggregate correction requires expected Patchset identity.".to_string()
            })?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|e| Self::error("aggregate correction writer", e))?;
        let actor_index_plus1 = self.request_actor_in_write(
            &tx,
            Self::required_object(payload, "history promotion payload")?,
        )?;
        let fingerprint = self.history_request_fingerprint(&tx, payload)?;
        // Replay returns the committed receipt without changing current candidate.
        if let Some(existing) =
            self.existing_history_manifest_in_write(&tx, &request.idempotency_key)?
        {
            if existing["request_sha256"].as_str() != Some(fingerprint.as_str())
                || existing["contract"].as_str() != Some(CORRECTION_MANIFEST_CONTRACT)
            {
                return Err(
                    "HISTORY_PROMOTION_IDEMPOTENCY_CONFLICT: aggregate correction request changed."
                        .to_string(),
                );
            }
            drop(tx);
            return self.history_correction_result(&existing, true);
        }
        let prior_index = self.patchset_index_for_id(&tx, prior_id)?;
        let prior_patchset = self
            .read_patchset(&tx, prior_index)
            .map_err(|e| Self::error("prior aggregate Patchset", e))?;
        let workflow_owner = self.patchset_owner(&tx, prior_patchset)?;
        self.require_current_patchset(&tx, prior_index, prior_patchset)?;
        self.validate_expected_identity_in_read(
            &tx,
            workflow_owner.task_index,
            Some(&request.expected_identity),
        )?;
        self.require_active_owner(&tx, workflow_owner)?;
        let prior_value = self
            .history_manifest_for_patchset(&tx, prior_index)?
            .ok_or_else(|| {
                "AGGREGATE_HISTORY_MANIFEST_REQUIRED: current candidate is not an aggregate."
                    .to_string()
            })?;
        if request.expected_receipts_sha256 != receipt_entries_sha256(&prior_value["entries"])? {
            return Err("Aggregate correction immutable receipt digest changed.".to_string());
        }
        let prior: PersistedHistoryManifest = serde_json::from_value(prior_value.clone())
            .map_err(|e| format!("Invalid selected aggregate manifest: {e}"))?;
        if prior.stage_ordinal.is_some()
            || !matches!(
                prior.contract.as_str(),
                "ait-history-promotion/v3" | CORRECTION_MANIFEST_CONTRACT
            )
        {
            return Err(
                "AGGREGATE_CORRECTION_UNSUPPORTED: staged history correction is not supported."
                    .to_string(),
            );
        }
        let canonical_prior_id = self.patchset_public_id(prior_patchset);
        let change_ref = workflow_owner
            .change
            .map(|(_, change)| self.change_ref(workflow_owner.task_index, change.change_ordinal));
        if prior.aggregate.patchset_index != prior_index
            || prior.aggregate.patchset_id != canonical_prior_id
            || prior.stage_ordinal.is_some()
            || prior.aggregate.task_index != workflow_owner.task_index
            || prior.aggregate.task_id != self.task_id(workflow_owner.task_index)
            || prior.aggregate.change_index != workflow_owner.change.map(|(index, _)| index)
            || prior.aggregate.change_ref != change_ref
            || prior.target_line != request.target_line
            || self.content_snapshot_id(&tx, prior_patchset.base_snapshot_index)?
                != prior.base_snapshot_id
            || self.content_snapshot_id(&tx, prior_patchset.revision_snapshot_index)?
                != prior.revision_snapshot_id
            || request.revision_snapshot_id == prior.revision_snapshot_id
        {
            return Err(
                "Aggregate correction prior authority does not match its current candidate."
                    .to_string(),
            );
        }
        let lines =
            ServerBinaryDbLineStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let (_, line) = lines
            .line_by_name_in_write(&tx, &request.target_line)
            .map_err(|e| Self::error("aggregate correction target Line", e))?
            .ok_or_else(|| "Aggregate correction target Line is missing.".to_string())?;
        if line.head_snapshot_index() != Some(prior_patchset.base_snapshot_index) {
            return Err(
                "STALE_HISTORY_PROMOTION_BASE: aggregate correction target moved.".to_string(),
            );
        }
        let dag = self.load_history_snapshot_dag(&tx)?;
        let revision_index = self.snapshot_index_in_history_write(
            &tx,
            &request.revision_snapshot_id,
            "correction revision",
        )?;
        if !self
            .history_snapshot_ancestor_closure(&tx, &dag, revision_index)?
            .contains(&prior_patchset.revision_snapshot_index)
        {
            return Err(
                "Aggregate correction revision does not contain the prior reviewed revision."
                    .to_string(),
            );
        }
        let correction_owner = self.correction_owner(&tx, prior_patchset, revision_index)?;
        if let Some(correction) = &prior.correction {
            if !request.snapshots.starts_with(&correction.snapshots) {
                return Err(
                    "Aggregate correction must preserve its complete prior Snapshot tail."
                        .to_string(),
                );
            }
        }
        let correction = HistoryCorrection {
            original_patchset_id: prior
                .correction
                .as_ref()
                .map(|c| c.original_patchset_id.clone())
                .unwrap_or_else(|| canonical_prior_id.clone()),
            prior_patchset_id: canonical_prior_id,
            receipt_revision_snapshot_id: prior
                .correction
                .as_ref()
                .map(|c| c.receipt_revision_snapshot_id.clone())
                .unwrap_or_else(|| prior.revision_snapshot_id.clone()),
            snapshots: request.snapshots,
        };
        let now = Self::now_s()?;
        let next_index = tx
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .map_err(|e| Self::error("correction Patchset inventory", e))?;
        let ordinal = self.next_task_patchset_ordinal(&tx, correction_owner.task_index)?;
        let next_id = self.patchset_public_id(V0PatchsetRecord {
            task_index: workflow_owner.task_index,
            change_ordinal_plus1: correction_owner.change_ordinal_plus1(),
            patch_ordinal: ordinal,
            ..V0PatchsetRecord::default()
        });
        let mut value = prior_value;
        value["contract"] = json!(CORRECTION_MANIFEST_CONTRACT);
        value["idempotency_key"] = json!(request.idempotency_key);
        value["request_sha256"] = json!(fingerprint);
        value["revision_snapshot_id"] = json!(request.revision_snapshot_id);
        value["display_summary"] = json!(request.summary);
        value["correction"] = serde_json::to_value(correction).map_err(|e| e.to_string())?;
        value["aggregate"]["change_index"] = json!(correction_owner.change.map(|(index, _)| index));
        value["aggregate"]["change_ref"] =
            json!(correction_owner
                .change
                .map(|(_, change)| self
                    .change_ref(correction_owner.task_index, change.change_ordinal)));
        value["aggregate"]["patchset_index"] = json!(next_index);
        value["aggregate"]["patchset_id"] = json!(next_id);
        let manifest: PersistedHistoryManifest =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        self.correction_snapshot_links(&tx, &dag, &value, &manifest)?;
        self.prepare_history_promotion_write_set(&mut tx)?;
        let (actual_index, actual_id) = self.append_history_patchset_in_write(
            &mut tx,
            correction_owner,
            prior_patchset.base_snapshot_index,
            revision_index,
            HISTORY_PROMOTION_SUMMARY_PREFIX,
            &value,
            &request.author_mode,
            now,
            actor_index_plus1,
        )?;
        if actual_index != next_index || actual_id != next_id {
            return Err("Aggregate correction Patchset identity drifted.".to_string());
        }
        tx.commit()
            .map_err(|e| Self::error("aggregate correction commit", e))?;
        drop(tx);
        self.history_correction_result(&value, false)
    }

    fn history_correction_result(
        &self,
        manifest: &JsonValue,
        replayed: bool,
    ) -> Result<JsonValue, String> {
        let mut result = self.history_promotion_result(manifest, replayed)?;
        result["contract"] = json!(CORRECTION_PREPARE_CONTRACT);
        result["correction"] = manifest["correction"].clone();
        Ok(result)
    }
}

#[cfg(test)]
#[path = "history_promotion_correction_tests.rs"]
mod tests;
