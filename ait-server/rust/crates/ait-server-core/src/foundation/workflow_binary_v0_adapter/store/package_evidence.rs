use super::*;
use crate::foundation::prepared_ci_runtime::{fold_package_reviews, PreparedPackageEvidence};
use crate::foundation::workflow_package_contract::{
    PackageEvidenceOperation, PackageEvidenceRequest,
};

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    /// Validate attribution without allocating formal evidence or Actors.
    pub fn validate_package_evidence(
        &self,
        request: &PackageEvidenceRequest,
        author_mode: &str,
    ) -> Result<u32, String> {
        let actor = self.validate_workflow_package_actor(request.actor_key_index)?;
        let read = BinaryDbReadTxn::new_bounded_for_scope(
            &self.db,
            crate::foundation::remote_binary_db::BinaryDbReadScope::WORKFLOW,
        );
        let (record, identity) = self.actor_at(&read, actor - 1)?;
        match request.operation {
            PackageEvidenceOperation::Review => {
                let payload = &request.payload;
                let reviewer = Self::required_text(payload, "reviewer")?;
                let kind = match payload.get("reviewer_kind") {
                    None => "user",
                    Some(JsonValue::String(kind)) if matches!(kind.as_str(), "user" | "bot") => {
                        kind.as_str()
                    }
                    _ => return Err("reviewer_kind must be user or bot".into()),
                };
                if kind == "user" && (record.actor_meta & 7 != 0 || identity.user_name != reviewer)
                {
                    return Err("Human package review must identify the live request Actor; an asserted reviewer name is not authorization".into());
                }
                if kind == "bot" {
                    WorkflowBinaryV0Codec::encode_actor_payload(&V0ActorPayload {
                        user_name: reviewer,
                        user_id: String::new(),
                        email: String::new(),
                        memo: String::new(),
                    })
                    .map_err(|error| error.to_string())?;
                }
                let action = Self::required_text(payload, "action")?;
                if action == "request" {
                    return Err("Package review does not allocate group requests".into());
                }
                Self::review_meta_for_action(
                    &action,
                    Self::optional_bool(payload, "blocking").unwrap_or(false),
                )?;
                let bytes = WorkflowBinaryV0Codec::encode_review_payload(
                    &Self::optional_text(payload, "comment").unwrap_or_default(),
                )
                .map_err(|error| error.to_string())?;
                u16::try_from(bytes.len())
                    .map_err(|_| "Review payload exceeds u16 byte capacity")?;
            }
            PackageEvidenceOperation::Attestation => {
                let mode = Self::author_mode_bits(author_mode)?;
                let meta = Self::package_attestation_meta(&request.payload, mode, true)?;
                if mode == Self::author_mode_bits("ai_with_human_review")?
                    && meta & ATTEST_REQUIRE_HUMAN_REVIEW == 0
                {
                    return Err(
                        "Package author mode requires human review in its Attestation".into(),
                    );
                }
            }
        }
        Ok(actor)
    }

    pub fn package_available_attestations(
        &self,
        manifest: &crate::foundation::workflow_package::WorkflowPackageManifest,
    ) -> Result<usize, String> {
        let read = BinaryDbReadTxn::new(&self.db);
        let entry = manifest
            .history_entries
            .last()
            .ok_or("Package final source missing")?;
        let mut task = entry
            .expected_remote_task_id
            .as_deref()
            .map(|id| self.task_index_for_id(&read, id))
            .transpose()?;
        let snapshots = ServerBinaryDbSnapshotStore::<D, 1>::new(self.db.clone());
        let mut indices = BTreeSet::new();
        for snapshot in &entry.snapshots {
            if let Some((index, _)) = snapshots
                .snapshot_by_id(&read, &snapshot.snapshot_id)
                .map_err(|e| e.to_string())?
            {
                indices.insert(index);
            }
        }
        for i in 0..read
            .record_count(WorkflowBinaryV0Codec::snapshot_link_file())
            .map_err(|e| e.to_string())?
        {
            let link = WorkflowBinaryV0Codec::decode_snapshot_link(
                &read
                    .read_record(WorkflowBinaryV0Codec::snapshot_link_file(), i)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if indices.contains(&link.content_snapshot_index) {
                if link.link_meta & SNAPSHOT_LINK_TOMBSTONE != 0
                    || task.is_some_and(|t| t != link.task_index)
                {
                    return Err("Final source owner conflicts before evidence acceptance".into());
                }
                task = Some(link.task_index);
            }
        }
        let Some(task) = task else {
            return Ok(64);
        };
        let allocated = WorkflowBinaryV0Codec::decode_ordinal_index(
            &read
                .read_record(
                    WorkflowBinaryV0Codec::chain_index_file("task_attest_index.bin"),
                    task,
                )
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        64usize
            .checked_sub(usize::from(allocated.next_ordinal))
            .ok_or("Attestation64/Task capacity exhausted".into())
    }

    pub(super) fn transfer_package_evidence_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        owner_ref: &str,
        patchset_id: &str,
        author_mode: &str,
        actions: &[PreparedPackageEvidence],
    ) -> Result<(), String> {
        let mode = Self::author_mode_bits(author_mode)?;
        let mut require_human = mode == Self::author_mode_bits("ai_with_human_review")?;
        for event in actions
            .iter()
            .filter(|event| event.request.operation == PackageEvidenceOperation::Attestation)
        {
            require_human = mode == Self::author_mode_bits("ai_with_human_review")?
                || Self::package_attestation_meta(&event.request.payload, mode, true)?
                    & ATTEST_REQUIRE_HUMAN_REVIEW
                    != 0;
        }
        fold_package_reviews(actions).require_complete(require_human)?;
        for event in actions {
            // Revalidate live attribution inside the same finish transaction.
            let actor_payload = json!({"actor_key_index":event.request.actor_key_index});
            let actor_payload = actor_payload.as_object().expect("Actor object");
            if self.request_actor_in_write(tx, actor_payload)? != event.actor_index_plus1 {
                return Err("Accepted package Actor differs at finish".into());
            }
            match event.request.operation {
                PackageEvidenceOperation::Review => {
                    let payload = &event.request.payload;
                    let reviewer = Self::required_text(payload, "reviewer")?;
                    let user =
                        Self::optional_text(payload, "reviewer_kind").as_deref() != Some("bot");
                    if user {
                        let (actor, identity) = self.actor_at(tx, event.actor_index_plus1 - 1)?;
                        if actor.actor_meta & 7 != 0 || identity.user_name != reviewer {
                            return Err(
                                "Accepted human reviewer no longer matches its Actor".into()
                            );
                        }
                    }
                    self.append_review_in_write(
                        tx,
                        owner_ref,
                        patchset_id,
                        &reviewer,
                        if user { 0 } else { 2 },
                        Self::review_meta_for_action(
                            &Self::required_text(payload, "action")?,
                            Self::optional_bool(payload, "blocking").unwrap_or(false),
                        )?,
                        &Self::optional_text(payload, "comment").unwrap_or_default(),
                        None,
                        Some(actor_payload),
                        user.then_some(event.actor_index_plus1 - 1),
                        event.recorded_at_s,
                    )?;
                }
                PackageEvidenceOperation::Attestation => {
                    let mode = Self::author_mode_bits(author_mode)?;
                    let meta = Self::package_attestation_meta(&event.request.payload, mode, true)?;
                    if mode == Self::author_mode_bits("ai_with_human_review")?
                        && meta & ATTEST_REQUIRE_HUMAN_REVIEW == 0
                    {
                        return Err("Package Attestation cannot waive required human review".into());
                    }
                    self.put_attestation_in_write(
                        tx,
                        patchset_id,
                        &JsonValue::Object(event.request.payload.clone()),
                        event.recorded_at_s,
                    )?;
                }
            }
        }
        // Generic Review actions retain their existing semantics. The package
        // predicate above independently resolves every principal/lane; reconcile
        // only its aggregate's derived pending bit after transferring real events.
        let owner = self.workflow_owner(tx, owner_ref)?;
        let mut task = self
            .read_task(tx, owner.task_index)
            .map_err(|error| error.to_string())?;
        task.task_meta &= !TASK_META_REVIEW_PENDING;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::task_file(),
            owner.task_index,
            &WorkflowBinaryV0Codec::encode_task(task).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    }
}
