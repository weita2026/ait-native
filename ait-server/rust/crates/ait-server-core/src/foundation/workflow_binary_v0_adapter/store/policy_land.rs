use super::*;

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn latest_attestation_for_patchset<A: ReadV0>(
        &self,
        read: &A,
        patchset_index: u32,
    ) -> Result<Option<(u32, V0AttestRecord)>, String> {
        let count = read
            .record_count_v0(WorkflowBinaryV0Codec::attest_file())
            .map_err(|error| Self::error("Attestation read", error))?;
        let mut latest = None;
        for index in 0..count {
            let record = WorkflowBinaryV0Codec::decode_attest(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::attest_file(), index)
                    .map_err(|error| Self::error("Attestation read", error))?,
            )
            .map_err(|error| Self::error("Attestation decode", error))?;
            if record.patchset_index == patchset_index && record.attest_meta & ATTEST_TOMBSTONE == 0
            {
                latest = Some((index, record));
            }
        }
        Ok(latest)
    }

    pub(super) fn attestation_at(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        index: u32,
    ) -> Result<JsonValue, String> {
        let record = WorkflowBinaryV0Codec::decode_attest(
            &read
                .read_record(WorkflowBinaryV0Codec::attest_file(), index)
                .map_err(|error| Self::error("Attestation read", error))?,
        )
        .map_err(|error| Self::error("Attestation decode", error))?;
        if record.attest_meta & ATTEST_TOMBSTONE != 0 {
            return Err(format!("Unknown Attestation index {index}"));
        }
        let (_, change, patchset) = self.change_for_patchset_index(read, record.patchset_index)?;
        if record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal != change.change_ordinal
        {
            return Err("Binary DB v0 Attestation ownership ordinals disagree".to_string());
        }
        let patchset_id = self.patchset_id(change, patchset.patch_ordinal);
        let author_mode = match (patchset.patchset_meta & PATCHSET_AUTHOR_MODE_MASK) >> 2 {
            0 => "human_only",
            1 => "human_with_ai_assist",
            2 => "ai_with_human_review",
            3 => "ai_only_experimental",
            4 => "agent",
            5 => "codex",
            6 => "xhigh",
            _ => return Err("Binary DB v0 Patchset author mode is reserved".to_string()),
        };
        let verification_state = match record.attest_meta & ATTEST_VERIFICATION_MASK {
            0 => "unknown",
            1 => "pending",
            2 => "pass",
            3 => "fail",
            _ => unreachable!(),
        };
        Ok(json!({
            "attestation_id": self.attestation_id(change.task_index, record.attest_ordinal),
            "repo_id": self.db.repo_id().as_str(),
            "task_id": self.task_id(change.task_index),
            "change_id": format!("C-{:02}", change.change_ordinal + 1),
            "change_ref": self.change_ref(change.task_index, change.change_ordinal),
            "patchset_id": patchset_id,
            "verification_state": verification_state,
            "revoked": record.attest_meta & ATTEST_REVOKED != 0,
            "require_tests_pass": record.attest_meta & ATTEST_REQUIRE_TESTS_PASS != 0,
            "require_human_review": record.attest_meta & ATTEST_REQUIRE_HUMAN_REVIEW != 0,
            "require_lint_pass": record.attest_meta & ATTEST_REQUIRE_LINT_PASS != 0,
            "ci_backed": record.attest_meta & ATTEST_CI_BACKED != 0,
            "author_mode": author_mode,
            "date": Self::timestamp(record.created_at_s)?,
            "attested_at": Self::timestamp(record.created_at_s)?,
            "created_at": Self::timestamp(record.created_at_s)?,
            "updated_at": Self::timestamp(record.created_at_s)?,
            "evaluation_summary": {
                "tests": ci_status_name(patchset.ci_status(CI_STATUS_TESTS_SHIFT)),
                "lint": ci_status_name(patchset.ci_status(CI_STATUS_LINT_SHIFT)),
            },
        }))
    }

    fn policy_checks_at<A: ReadV0>(
        &self,
        read: &A,
        policy: V0PolicyRecord,
    ) -> Result<Vec<V0PolicyCheckRecord>, String> {
        if policy.check_count == 0 {
            return Ok(Vec::new());
        }
        let first = policy.first_check_index_plus1 - 1;
        let mut checks = Vec::with_capacity(usize::from(policy.check_count));
        for offset in 0..u32::from(policy.check_count) {
            let index = first
                .checked_add(offset)
                .ok_or_else(|| "Binary DB v0 Policy check range overflow".to_string())?;
            checks.push(
                WorkflowBinaryV0Codec::decode_policy_check(
                    &read
                        .read_record_v0(WorkflowBinaryV0Codec::policy_check_file(), index)
                        .map_err(|error| Self::error("Policy Check read", error))?,
                )
                .map_err(|error| Self::error("Policy Check decode", error))?,
            );
        }
        Ok(checks)
    }

    fn policy_check_projection(check: V0PolicyCheckRecord) -> Result<JsonValue, String> {
        let (name, label) = match check.check_kind {
            0 => ("require_attestation", "Attestation"),
            1 => ("ai_provenance", "AI provenance"),
            2 => ("code_review_summary", "Code review summary"),
            3 => ("tests", "Tests"),
            4 => ("lint", "Lint"),
            5 => ("security_scan", "Security scan"),
            6 => ("license_scan", "License scan"),
            7 => ("required_human_review", "Required human review"),
            8 => ("ci_rollout_phase", "CI rollout phase"),
            9 => ("ci_patchset_suite", "CI patchset suite"),
            _ => return Err("Binary DB v0 Policy Check kind is reserved".to_string()),
        };
        let status = match check.check_status {
            0 => "absent",
            1 => "not_required",
            2 => "pending",
            3 => "pass",
            4 => "hard_fail",
            5 => "soft_fail",
            6 => "waived",
            7 => "optional_fail",
            _ => return Err("Binary DB v0 Policy Check status is reserved".to_string()),
        };
        let message = match (check.check_kind, check.check_status) {
            (0, 2) => "Attestation is required before landing",
            (0, 3) => "Attestation policy evaluated",
            (3, 1) => "Passing tests are not required by this Attestation",
            (3, 2) => "Passing runnable Patchset CI evidence is required before landing",
            (3, 3) => "Test policy evaluated",
            (4, 1) => "Passing lint is not required by this Attestation",
            (4, 2) => "Explicit passing lint evidence is required",
            (4, 3) => "Lint policy evaluated",
            (7, 1) => "Human review is not required by this Attestation",
            (7, 2) => "At least one human approval is required",
            (7, 3) => "Required human approval is present",
            _ => "Policy check projected from Binary DB v0 fixed authority",
        };
        Ok(json!({
            "name": name,
            "label": label,
            "status": status,
            "message": message,
            "subject_ordinal": check.subject_ordinal,
            "detail_flags": check.detail_flags,
        }))
    }

    pub(super) fn policy_at(
        &self,
        read: &BinaryDbReadTxn<'_, D>,
        index: u32,
    ) -> Result<JsonValue, String> {
        let record = WorkflowBinaryV0Codec::decode_policy(
            &read
                .read_record(WorkflowBinaryV0Codec::policy_file(), index)
                .map_err(|error| Self::error("Policy read", error))?,
        )
        .map_err(|error| Self::error("Policy decode", error))?;
        if record.policy_meta & POLICY_TOMBSTONE != 0 {
            return Err(format!("Unknown Policy index {index}"));
        }
        let (_, change, patchset) = self.change_for_patchset_index(read, record.patchset_index)?;
        if record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal != change.change_ordinal
        {
            return Err("Binary DB v0 Policy ownership ordinals disagree".to_string());
        }
        let patchset_id = self.patchset_id(change, patchset.patch_ordinal);
        let checks = self
            .policy_checks_at(read, record)?
            .into_iter()
            .map(Self::policy_check_projection)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(json!({
            "policy_decision_id": self.policy_id(&patchset_id, record.policy_ordinal),
            "repo_id": self.db.repo_id().as_str(),
            "patchset_id": patchset_id,
            "decision": policy_decision_name(record.policy_meta & POLICY_DECISION_MASK)?,
            "checks": checks,
            "evaluated_at": Self::timestamp(record.created_at_s)?,
            "policy_id": "prototype",
        }))
    }

    fn latest_live_review_approvals<A: ReadV0>(
        &self,
        read: &A,
        patchset_index: u32,
    ) -> Result<usize, String> {
        let count = read
            .record_count_v0(WorkflowBinaryV0Codec::review_file())
            .map_err(|error| Self::error("Policy Review read", error))?;
        let mut approvals = 0_usize;
        for index in 0..count {
            let review = WorkflowBinaryV0Codec::decode_review(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::review_file(), index)
                    .map_err(|error| Self::error("Policy Review read", error))?,
            )
            .map_err(|error| Self::error("Policy Review decode", error))?;
            if review.patchset_index == patchset_index
                && review.review_meta & REVIEW_TOMBSTONE == 0
                && review.review_meta & REVIEW_ACTION_MASK == 2
            {
                approvals += 1;
            }
        }
        Ok(approvals)
    }

    fn compact_ci_status_code(
        completion: &JsonMap<String, JsonValue>,
        field: &str,
    ) -> Result<u8, String> {
        match completion.get(field).and_then(JsonValue::as_str) {
            Some("none") => Ok(CI_STATUS_NONE),
            Some("pass") => Ok(CI_STATUS_PASS),
            Some("fail") => Ok(CI_STATUS_FAIL),
            Some("error") => Ok(CI_STATUS_ERROR),
            Some(value) => Err(format!(
                "patchset_ci completion {field} has unsupported state {value:?}"
            )),
            None => Err(format!("patchset_ci completion requires {field}")),
        }
    }

    fn compact_ci_count(
        completion: &JsonMap<String, JsonValue>,
        field: &str,
    ) -> Result<u16, String> {
        completion
            .get(field)
            .and_then(JsonValue::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| format!("patchset_ci completion requires u16 {field}"))
    }

    fn land_index_for_id<A: ReadV0>(&self, read: &A, submission_id: &str) -> Result<u32, String> {
        let (change_ref, _) = submission_id
            .rsplit_once("/L-")
            .ok_or_else(|| format!("{submission_id:?} is not a normalized Land identity"))?;
        let change_index = self.change_index_for_ref(read, change_ref)?;
        let ordinal = Self::parse_owned_ordinal(submission_id, change_ref, "L")?;
        let count = read
            .record_count_v0(WorkflowBinaryV0Codec::land_file())
            .map_err(|error| Self::error("Land identity read", error))?;
        let mut found = None;
        for index in 0..count {
            let record = WorkflowBinaryV0Codec::decode_land(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::land_file(), index)
                    .map_err(|error| Self::error("Land identity read", error))?,
            )
            .map_err(|error| Self::error("Land identity decode", error))?;
            if record.land_meta & LAND_TOMBSTONE == 0
                && record.change_index == change_index
                && record.land_ordinal == ordinal
                && found.replace(index).is_some()
            {
                return Err(format!(
                    "Duplicate Binary DB v0 Land identity: {submission_id}"
                ));
            }
        }
        found.ok_or_else(|| format!("Unknown land: {submission_id}"))
    }

    pub(super) fn land_at<A: ReadV0>(&self, read: &A, index: u32) -> Result<JsonValue, String> {
        let record = WorkflowBinaryV0Codec::decode_land(
            &read
                .read_record_v0(WorkflowBinaryV0Codec::land_file(), index)
                .map_err(|error| Self::error("Land read", error))?,
        )
        .map_err(|error| Self::error("Land decode", error))?;
        if record.land_meta & LAND_TOMBSTONE != 0 {
            return Err(format!("Unknown Land index {index}"));
        }
        let change = self
            .read_change(read, record.change_index)
            .map_err(|error| Self::error("Land Change read", error))?;
        let patchset = self
            .read_patchset(read, record.patchset_index)
            .map_err(|error| Self::error("Land Patchset read", error))?;
        if record.change_ordinal != change.change_ordinal
            || patchset.change_index != record.change_index
        {
            return Err("Binary DB v0 Land ownership relation disagrees".to_string());
        }
        let line_raw = read
            .read_record_v0(
                crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::record_file(),
                record.target_line_index_plus1 - 1,
            )
            .map_err(|error| Self::error("Land target Line read", error))?;
        let line_record =
            crate::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::decode_record(
                &line_raw,
            )
            .map_err(|error| Self::error("Land target Line decode", error))?;
        let target_line = self.content_line_name(read, &line_record)?;
        let snapshot_id = |plus1: u32| -> Result<JsonValue, String> {
            plus1
                .checked_sub(1)
                .map(|snapshot_index| {
                    self.content_snapshot_id(read, snapshot_index)
                        .map(JsonValue::String)
                        .map_err(|error| format!("Land Snapshot read failed: {error}"))
                })
                .transpose()
                .map(|value| value.unwrap_or(JsonValue::Null))
        };
        let pre_target = snapshot_id(record.pre_land_target_snapshot_index_plus1)?;
        let landed_snapshot = snapshot_id(record.landed_snapshot_index_plus1)?;
        let base_snapshot = self
            .content_snapshot_id(read, patchset.base_snapshot_index)
            .map_err(|error| format!("Land base Snapshot read failed: {error}"))?;
        let revision_snapshot = self
            .content_snapshot_id(read, patchset.revision_snapshot_index)
            .map_err(|error| format!("Land revision Snapshot read failed: {error}"))?;
        let patchset_summary_raw = read
            .read_payload_v0(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                patchset.summary_offset,
                u32::from(patchset.summary_len),
            )
            .map_err(|error| Self::error("Land Patchset summary read", error))?;
        let patchset_summary = WorkflowBinaryV0Codec::decode_single_text_payload(
            &patchset_summary_raw,
            "Land Patchset summary",
        )
        .map_err(|error| Self::error("Land Patchset summary decode", error))?;
        let (source_kind, governance_authority) =
            history_promotion::source_kind_for_summary(patchset_summary);
        let status = match record.land_meta & LAND_STATUS_MASK {
            LAND_STATUS_QUEUED => "queued",
            LAND_STATUS_RUNNING => "running",
            LAND_STATUS_SUCCEEDED => "succeeded",
            LAND_STATUS_BLOCKED => "blocked",
            LAND_STATUS_FAILED => "failed",
            LAND_STATUS_CANCELED => "canceled",
            LAND_STATUS_UPDATING => "updating",
            _ => return Err("Binary DB v0 Land status is reserved".to_string()),
        };
        let mode = match (record.land_meta & LAND_MODE_MASK) >> 5 {
            LAND_MODE_DIRECT => "direct",
            LAND_MODE_MERGE => "merge",
            LAND_MODE_FF_ONLY => "ff-only",
            _ => return Err("Binary DB v0 Land mode is reserved".to_string()),
        };
        let failure = match record.failure_kind {
            0 => None,
            1 => Some("BASE_STALE"),
            2 => Some("POLICY_BLOCKED"),
            3 => Some("REVIEW_BLOCKED"),
            4 => Some("CI_BLOCKED"),
            5 => Some("CONFLICT"),
            6 => Some("TARGET_UPDATE_FAILED"),
            7 => Some("INTERNAL_ERROR"),
            _ => return Err("Binary DB v0 Land failure kind is reserved".to_string()),
        };
        let line_action = if status != "succeeded" {
            JsonValue::Null
        } else if landed_snapshot == pre_target
            && landed_snapshot.as_str() == Some(revision_snapshot.as_str())
        {
            json!("already_at_selected_patchset_revision")
        } else if landed_snapshot == pre_target {
            json!("already_contains_selected_patchset_revision")
        } else {
            json!("moved")
        };
        let result = if status == "succeeded" {
            json!({
                "landed_snapshot_id": landed_snapshot,
                "selected_revision_snapshot_id": revision_snapshot,
                "target_line": target_line,
                "base_snapshot_id": base_snapshot,
                "target_line_head": pre_target,
                "line_action": line_action,
                "current_head_contains_selected_revision": line_action == json!("already_contains_selected_patchset_revision"),
                "snapshot_action": if line_action == json!("already_contains_selected_patchset_revision") { "selected_patchset_revision_already_contained" } else { "selected_patchset_revision" },
            })
        } else {
            json!({
                "blocker_class": failure,
                "target_line": target_line,
                "target_line_head": pre_target,
                "expected_base_snapshot_id": base_snapshot,
                "selected_revision_snapshot_id": revision_snapshot,
                "current_head_contains_selected_revision": false,
            })
        };
        let change_ref = self.change_ref(change.task_index, change.change_ordinal);
        let patchset_id = self.patchset_id(change, patchset.patch_ordinal);
        Ok(json!({
            "submission_id": self.land_id(&change_ref, record.land_ordinal),
            "repo_name": self.db.repo_name().as_str(),
            "repo_id": self.db.repo_id().as_str(),
            "land_seq": index + 1,
            "change_id": format!("C-{:02}", change.change_ordinal + 1),
            "change_ref": change_ref,
            "patchset_id": patchset_id,
            "source_kind": source_kind,
            "governance_authority": governance_authority,
            "target_line": target_line,
            "mode": mode,
            "status": status,
            "failure_kind": failure,
            "result": result,
            "result_json": serde_json::to_string(&result)
                .map_err(|error| format!("failed to render derived Land result: {error}"))?,
            "landed_snapshot_id": landed_snapshot,
            "line_action": line_action,
            "created_at": Self::timestamp(record.submitted_at_s)?,
            "updated_at": Self::timestamp(record.updated_at_s)?,
        }))
    }
}

impl<D> ServerWorkflowAttestationStore for BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn put_attestation(&self, patchset_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let operation = "ServerWorkflowAttestationStore::put_attestation";
        let payload = Self::required_object(payload, "attestation input")?;
        if payload
            .get("detail")
            .and_then(JsonValue::as_object)
            .is_some_and(|detail| detail.contains_key("patchset_ci"))
        {
            return Err(
                "Attestation input must not contain patchset_ci; CI completion belongs to Patchset"
                    .to_string(),
            );
        }
        let now = Self::now_s()?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let patchset_index = self.patchset_index_for_id(&tx, patchset_id)?;
        self.require_patchset_governance_authority(
            &tx,
            patchset_index,
            patchset_id,
            "Attestation mutation",
        )?;
        let (_, mut change, mut patchset) = self.change_for_patchset_index(&tx, patchset_index)?;
        if change.selected_patchset_index_plus1 != patchset_index + 1 {
            return Err(format!(
                "Patchset {patchset_id} is not the selected Patchset for its Change"
            ));
        }
        if let Some(author_mode) = Self::optional_text(payload, "author_mode") {
            let expected = Self::author_mode_bits(&author_mode)?;
            if patchset.patchset_meta & PATCHSET_AUTHOR_MODE_MASK != expected {
                return Err(format!(
                    "Attestation author_mode {author_mode:?} disagrees with Patchset authority"
                ));
            }
        }
        let verification = match Self::optional_text(payload, "verification_state").as_deref() {
            None | Some("unknown") => 0,
            Some("pending") => 1,
            Some("pass") => 2,
            Some("fail") => 3,
            Some(value) => {
                return Err(format!(
                    "unsupported Binary DB v0 Attestation verification_state {value:?}"
                ))
            }
        };
        let mut attest_meta = verification;
        if Self::optional_bool(payload, "revoked").unwrap_or(false) {
            attest_meta |= ATTEST_REVOKED;
        }
        if Self::optional_bool(payload, "require_tests_pass").unwrap_or(true) {
            attest_meta |= ATTEST_REQUIRE_TESTS_PASS;
        }
        if Self::optional_bool(payload, "require_human_review").unwrap_or(false) {
            attest_meta |= ATTEST_REQUIRE_HUMAN_REVIEW;
        }
        if Self::optional_bool(payload, "require_lint_pass").unwrap_or(false) {
            attest_meta |= ATTEST_REQUIRE_LINT_PASS;
        }
        if patchset.ci_completed_at_s != 0 {
            attest_meta |= ATTEST_CI_BACKED;
        }
        let mut task_owner = WorkflowBinaryV0Codec::decode_ordinal_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("task_attest_index.bin"),
                change.task_index,
            )
            .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if task_owner.next_ordinal >= 64 {
            return Err(format!(
                "Task {} has exhausted its v0 Attestation ordinals",
                self.task_id(change.task_index)
            ));
        }
        let mut patch_inventory = WorkflowBinaryV0Codec::decode_inventory_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("patchset_attest_index.bin"),
                patchset_index,
            )
            .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        let record = V0AttestRecord {
            attest_meta,
            attest_ordinal: task_owner.next_ordinal,
            patch_ordinal: patchset.patch_ordinal,
            change_ordinal: change.change_ordinal,
            patchset_index,
            previous_task_attest_index_plus1: task_owner.latest_index_plus1,
            previous_patchset_attest_index_plus1: patch_inventory.latest_index_plus1,
            created_at_s: now,
        };
        let raw = WorkflowBinaryV0Codec::encode_attest(record)
            .map_err(|error| Self::error(operation, error))?;
        let attest_index = tx
            .append_record(WorkflowBinaryV0Codec::attest_file(), &raw)
            .map_err(|error| Self::error(operation, error))?;
        task_owner.latest_index_plus1 = attest_index + 1;
        task_owner.count = task_owner
            .count
            .checked_add(1)
            .ok_or_else(|| "Task Attestation count exceeds u16".to_string())?;
        task_owner.next_ordinal = task_owner
            .next_ordinal
            .checked_add(1)
            .ok_or_else(|| "Task Attestation ordinal overflow".to_string())?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_attest_index.bin"),
            change.task_index,
            &WorkflowBinaryV0Codec::encode_ordinal_index(task_owner)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        patch_inventory.latest_index_plus1 = attest_index + 1;
        patch_inventory.count = patch_inventory
            .count
            .checked_add(1)
            .ok_or_else(|| "Patchset Attestation count exceeds u16".to_string())?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("patchset_attest_index.bin"),
            patchset_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(patch_inventory)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        patchset.patchset_meta |= PATCHSET_EVALUATION_PENDING;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            patchset_index,
            &self
                .encode_patchset_replacement(&tx, patchset_index, patchset)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        change.change_meta |= CHANGE_META_VALIDATION_PENDING;
        change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
        change.updated_at_s = now;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::change_file(),
            patchset.change_index,
            &WorkflowBinaryV0Codec::encode_change(change)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        tx.commit().map_err(|error| Self::error(operation, error))?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.attestation_at(&read, attest_index)
    }

    fn get_attestation(&self, patchset_id: &str) -> Result<JsonValue, String> {
        let read = BinaryDbReadTxn::new(&self.db);
        let patchset_index = self.patchset_index_for_id(&read, patchset_id)?;
        let (index, _) = self
            .latest_attestation_for_patchset(&read, patchset_index)?
            .ok_or_else(|| format!("Unknown attestation for patchset: {patchset_id}"))?;
        self.attestation_at(&read, index)
    }
}

impl<D> ServerWorkflowPolicyStore for BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn get_policy(&self, patchset_id: &str) -> Result<JsonValue, String> {
        let read = BinaryDbReadTxn::new(&self.db);
        let patchset_index = self.patchset_index_for_id(&read, patchset_id)?;
        match self.latest_policy_decision(&read, patchset_index)? {
            Some((index, _)) => self.policy_at(&read, index),
            None => Ok(json!({
                "patchset_id": patchset_id,
                "decision": "pending",
                "checks": [],
            })),
        }
    }

    fn evaluate_policy(&self, patchset_id: &str) -> Result<JsonValue, String> {
        let operation = "ServerWorkflowPolicyStore::evaluate_policy";
        let now = Self::now_s()?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let patchset_index = self.patchset_index_for_id(&tx, patchset_id)?;
        self.require_patchset_governance_authority(
            &tx,
            patchset_index,
            patchset_id,
            "Policy evaluation",
        )?;
        let (_, mut change, mut patchset) = self.change_for_patchset_index(&tx, patchset_index)?;
        let attestation = self
            .latest_attestation_for_patchset(&tx, patchset_index)?
            .map(|(_, record)| record);
        let approvals = self.latest_live_review_approvals(&tx, patchset_index)?;
        let require_tests = attestation
            .map(|record| record.attest_meta & ATTEST_REQUIRE_TESTS_PASS != 0)
            .unwrap_or(true);
        let require_lint =
            attestation.is_some_and(|record| record.attest_meta & ATTEST_REQUIRE_LINT_PASS != 0);
        let require_human =
            attestation.is_some_and(|record| record.attest_meta & ATTEST_REQUIRE_HUMAN_REVIEW != 0);
        let ci_passes = patchset.ci_completed_at_s != 0
            && patchset.ci_status(CI_STATUS_OVERALL_SHIFT) == CI_STATUS_PASS
            && patchset.ci_status(CI_STATUS_TESTS_SHIFT) == CI_STATUS_PASS
            && patchset.ci_suite_result_count > 0
            && patchset.ci_blocking_failure_count == 0;
        let lint_passes = patchset.ci_completed_at_s != 0
            && patchset.ci_status(CI_STATUS_LINT_SHIFT) == CI_STATUS_PASS;
        let mut checks = vec![V0PolicyCheckRecord {
            check_kind: 0,
            check_status: if attestation.is_some() { 3 } else { 2 },
            subject_ordinal: 0,
            detail_flags: 0,
        }];
        checks.push(V0PolicyCheckRecord {
            check_kind: 3,
            check_status: if !require_tests {
                1
            } else if ci_passes {
                3
            } else {
                2
            },
            subject_ordinal: 0,
            detail_flags: 0,
        });
        checks.push(V0PolicyCheckRecord {
            check_kind: 4,
            check_status: if !require_lint {
                1
            } else if lint_passes {
                3
            } else {
                2
            },
            subject_ordinal: 0,
            detail_flags: 0,
        });
        checks.push(V0PolicyCheckRecord {
            check_kind: 7,
            check_status: if !require_human {
                1
            } else if approvals > 0 {
                3
            } else {
                2
            },
            subject_ordinal: 0,
            detail_flags: 0,
        });
        let decision_kind = if checks
            .iter()
            .all(|check| matches!(check.check_status, 1 | 3))
        {
            1
        } else {
            0
        };
        let mut owner = WorkflowBinaryV0Codec::decode_ordinal_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("patchset_policy_index.bin"),
                patchset_index,
            )
            .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if owner.next_ordinal >= 64 {
            return Err(format!(
                "Patchset {patchset_id} has exhausted its v0 Policy ordinals"
            ));
        }
        let mut inventory =
            self.task_inventory_in_write(&tx, "task_policy_index.bin", change.task_index)?;
        let first_check = tx
            .record_count(WorkflowBinaryV0Codec::policy_check_file())
            .map_err(|error| Self::error(operation, error))?;
        for check in &checks {
            tx.append_record(
                WorkflowBinaryV0Codec::policy_check_file(),
                &WorkflowBinaryV0Codec::encode_policy_check(*check)
                    .map_err(|error| Self::error(operation, error))?,
            )
            .map_err(|error| Self::error(operation, error))?;
        }
        self.sync_file(&tx, "policy_check.bin")
            .map_err(|error| Self::error(operation, error))?;
        let record = V0PolicyRecord {
            policy_meta: decision_kind,
            policy_ordinal: owner.next_ordinal,
            patch_ordinal: patchset.patch_ordinal,
            change_ordinal: change.change_ordinal,
            patchset_index,
            previous_task_policy_index_plus1: inventory.latest_index_plus1,
            previous_patchset_policy_index_plus1: owner.latest_index_plus1,
            first_check_index_plus1: first_check + 1,
            check_count: u16::try_from(checks.len())
                .map_err(|_| "Policy Check count exceeds u16".to_string())?,
            reserved0: 0,
            created_at_s: now,
        };
        let policy_index = tx
            .append_record(
                WorkflowBinaryV0Codec::policy_file(),
                &WorkflowBinaryV0Codec::encode_policy(record)
                    .map_err(|error| Self::error(operation, error))?,
            )
            .map_err(|error| Self::error(operation, error))?;
        owner.latest_index_plus1 = policy_index + 1;
        owner.count = owner
            .count
            .checked_add(1)
            .ok_or_else(|| "Patchset Policy count exceeds u16".to_string())?;
        owner.next_ordinal += 1;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("patchset_policy_index.bin"),
            patchset_index,
            &WorkflowBinaryV0Codec::encode_ordinal_index(owner)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        inventory.latest_index_plus1 = policy_index + 1;
        inventory.count = inventory
            .count
            .checked_add(1)
            .ok_or_else(|| "Task Policy count exceeds u16".to_string())?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_policy_index.bin"),
            change.task_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(inventory)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if decision_kind == 1 {
            patchset.patchset_meta &= !PATCHSET_EVALUATION_PENDING;
            change.change_meta &= !(CHANGE_META_VALIDATION_PENDING | CHANGE_META_BLOCKED);
            if change.change_meta & CHANGE_META_REVIEW_PENDING == 0 {
                change.change_meta |= CHANGE_META_READY_TO_LAND;
            }
        } else {
            patchset.patchset_meta |= PATCHSET_EVALUATION_PENDING;
            change.change_meta |= CHANGE_META_VALIDATION_PENDING;
            change.change_meta &= !CHANGE_META_READY_TO_LAND;
        }
        change.updated_at_s = now;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            patchset_index,
            &self
                .encode_patchset_replacement(&tx, patchset_index, patchset)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::change_file(),
            patchset.change_index,
            &WorkflowBinaryV0Codec::encode_change(change)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        tx.commit().map_err(|error| Self::error(operation, error))?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.policy_at(&read, policy_index)
    }

    fn run_patchset_ci(&self, patchset_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let payload = Self::required_object(payload, "patchset runCi payload")?;
        let requests_new_run = patchset_ci_trigger_requests_new_run(
            Self::optional_text(payload, "trigger").as_deref(),
        );
        let operation = "ServerWorkflowPolicyStore::run_patchset_ci";
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let patchset_index = self.patchset_index_for_id(&tx, patchset_id)?;
        self.require_patchset_governance_authority(
            &tx,
            patchset_index,
            patchset_id,
            "CI execution",
        )?;
        let (_, mut change, mut patchset) = self.change_for_patchset_index(&tx, patchset_index)?;
        if patchset.ci_run_seq > 0 && (patchset.ci_completed_at_s == 0 || !requests_new_run) {
            drop(tx);
            let read = BinaryDbReadTxn::new(&self.db);
            return self.patchset_at(&read, patchset_index);
        }
        patchset.ci_run_seq = patchset
            .ci_run_seq
            .checked_add(1)
            .ok_or_else(|| "Patchset CI run sequence exceeds u32".to_string())?;
        patchset.ci_completed_at_s = 0;
        patchset.ci_selected_suite_count = 0;
        patchset.ci_suite_result_count = 0;
        patchset.ci_blocking_failure_count = 0;
        patchset.ci_status_bits = 0;
        patchset.patchset_meta |= PATCHSET_EVALUATION_PENDING;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            patchset_index,
            &self
                .encode_patchset_replacement(&tx, patchset_index, patchset)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        let now = Self::now_s()?;
        change.change_meta |= CHANGE_META_VALIDATION_PENDING;
        change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
        change.updated_at_s = now;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::change_file(),
            patchset.change_index,
            &WorkflowBinaryV0Codec::encode_change(change)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        tx.commit().map_err(|error| Self::error(operation, error))?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.patchset_at(&read, patchset_index)
    }

    fn complete_patchset_ci(
        &self,
        patchset_id: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        const FIELDS: [&str; 8] = [
            "patchset_id",
            "ci_run_seq",
            "selected_suite_count",
            "suite_result_count",
            "blocking_failure_count",
            "overall_status",
            "tests_status",
            "lint_status",
        ];
        let operation = "ServerWorkflowPolicyStore::complete_patchset_ci";
        let completion = Self::required_object(payload, "patchset CI completion")?;
        if let Some(field) = completion
            .keys()
            .find(|field| !FIELDS.contains(&field.as_str()))
        {
            return Err(format!(
                "patchset_ci completion contains undeclared field {field:?}"
            ));
        }
        if completion.get("patchset_id").and_then(JsonValue::as_str) != Some(patchset_id) {
            return Err(format!(
                "Patchset CI completion identity does not match {patchset_id}"
            ));
        }
        let run_seq = completion
            .get("ci_run_seq")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| "patchset_ci completion requires ci_run_seq".to_string())?;
        let overall = Self::compact_ci_status_code(completion, "overall_status")?;
        if overall == CI_STATUS_NONE {
            return Err("patchset_ci completion requires a terminal overall_status".to_string());
        }
        let tests = Self::compact_ci_status_code(completion, "tests_status")?;
        let lint = Self::compact_ci_status_code(completion, "lint_status")?;
        let selected = Self::compact_ci_count(completion, "selected_suite_count")?;
        let results = Self::compact_ci_count(completion, "suite_result_count")?;
        let blocking = Self::compact_ci_count(completion, "blocking_failure_count")?;
        let completed_at_s = u64::try_from(Utc::now().timestamp())
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                "system time is outside the legacy Binary DB v0 u64 range".to_string()
            })?;
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error(operation, error))?;
        let patchset_index = self.patchset_index_for_id(&tx, patchset_id)?;
        self.require_patchset_governance_authority(
            &tx,
            patchset_index,
            patchset_id,
            "CI completion",
        )?;
        let (_, mut change, mut patchset) = self.change_for_patchset_index(&tx, patchset_index)?;
        if patchset.ci_run_seq != run_seq || patchset.ci_completed_at_s != 0 {
            return Err(format!(
                "stale Patchset CI completion for run {run_seq}; current run is {}",
                patchset.ci_run_seq
            ));
        }
        patchset.ci_completed_at_s = completed_at_s;
        patchset.ci_selected_suite_count = selected;
        patchset.ci_suite_result_count = results;
        patchset.ci_blocking_failure_count = blocking;
        patchset.ci_status_bits = (overall << CI_STATUS_OVERALL_SHIFT)
            | (tests << CI_STATUS_TESTS_SHIFT)
            | (lint << CI_STATUS_LINT_SHIFT);
        patchset.patchset_meta |= PATCHSET_EVALUATION_PENDING;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            patchset_index,
            &self
                .encode_patchset_replacement(&tx, patchset_index, patchset)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        let now = Self::now_s()?;
        change.change_meta |= CHANGE_META_VALIDATION_PENDING;
        change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
        change.updated_at_s = now;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::change_file(),
            patchset.change_index,
            &WorkflowBinaryV0Codec::encode_change(change)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        tx.commit().map_err(|error| Self::error(operation, error))?;
        let read = BinaryDbReadTxn::new(&self.db);
        self.patchset_at(&read, patchset_index)
    }
}

impl<D> ServerWorkflowLandStore for BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn resolve_task_land_change_ref(&self, task_or_change_ref: &str) -> Result<String, String> {
        self.resolve_task_land_change_ref_from_store(task_or_change_ref)
    }

    fn submit_task_land(
        &self,
        task_or_change_ref: &str,
        payload: &JsonValue,
    ) -> Result<JsonValue, String> {
        #[cfg(feature = "perfetto-tracing")]
        let _operation_trace =
            crate::perfetto_trace::PerfettoRange::new("ait.server.task_land.atomic.store");
        let (change_ref, prepared) = {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.task_land.atomic.request_prepare",
            );
            self.prepare_atomic_task_land_payload(task_or_change_ref, payload)?
        };
        self.submit_land(&change_ref, &prepared)
    }

    fn submit_land(&self, change_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let operation = "ServerWorkflowLandStore::submit_land";
        let payload = Self::required_object(payload, "land submit payload")?;
        let atomic_task_land =
            Self::optional_text(payload, "contract").as_deref() == Some("task-land-atomic/v1");
        #[cfg(feature = "perfetto-tracing")]
        let _atomic_submit_trace = atomic_task_land.then(|| {
            crate::perfetto_trace::PerfettoRange::new("ait.server.task_land.atomic.submit")
        });
        let idempotency_key = if atomic_task_land {
            Some(Self::required_text(payload, "idempotency_key")?)
        } else {
            None
        };
        let target_line = Self::required_text(payload, "target_line")?;
        let mode_text = Self::required_text(payload, "mode")?;
        let mode = match mode_text.as_str() {
            "direct" => LAND_MODE_DIRECT,
            "merge" => LAND_MODE_MERGE,
            "ff-only" => LAND_MODE_FF_ONLY,
            _ => return Err(format!("unsupported Binary DB v0 Land mode {mode_text:?}")),
        };
        let requested_patchset = Self::optional_text(payload, "patchset_id");
        let explicit_submission = Self::optional_text(payload, "submission_id");
        let expected_head = Self::optional_text(payload, "expected_head_snapshot_id")
            .or_else(|| Self::optional_text(payload, "expected_target_line_head"));
        if atomic_task_land {
            let requested_ref = Self::required_text(payload, "task_or_change_ref")?;
            self.apply_staged_history_receipts_before_atomic_land(
                change_id,
                &requested_ref,
                &target_line,
                requested_patchset.as_deref(),
                expected_head.as_deref(),
            )?;
        }
        let now = Self::now_s()?;
        #[cfg(feature = "perfetto-tracing")]
        let writer_trace = atomic_task_land.then(|| {
            crate::perfetto_trace::PerfettoRange::new(
                "ait.server.task_land.atomic.writer_critical_section",
            )
        });
        let mut tx = {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = atomic_task_land.then(|| {
                crate::perfetto_trace::PerfettoRange::new_lane(
                    "ait.server.task_land.atomic.writer_admission",
                )
            });
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerLand)
                .map_err(|error| Self::error(operation, error))?
        };
        #[cfg(feature = "perfetto-tracing")]
        let revalidation_trace = atomic_task_land.then(|| {
            crate::perfetto_trace::PerfettoRange::new(
                "ait.server.task_land.atomic.authoritative_revalidation",
            )
        });
        let change_index = self.change_index_for_ref(&tx, change_id)?;
        let mut change = self
            .read_change(&tx, change_index)
            .map_err(|error| Self::error(operation, error))?;
        if atomic_task_land {
            let requested_ref = Self::required_text(payload, "task_or_change_ref")?;
            let resolved_ref = self.resolve_task_land_change_ref_in_read(&tx, &requested_ref)?;
            if resolved_ref != change_id {
                return Err(format!(
                    "Atomic Task Land reference changed during admission: expected {change_id}, resolved {resolved_ref}."
                ));
            }
        }
        let selected_patchset_index = change
            .selected_patchset_index_plus1
            .checked_sub(1)
            .ok_or_else(|| format!("No selected Patchset found for Change {change_id}"))?;
        let patchset_index = match requested_patchset.as_deref() {
            Some(patchset_id) => {
                let requested = self.patchset_index_for_id(&tx, patchset_id)?;
                if requested != selected_patchset_index {
                    return Err(format!(
                        "Patchset {patchset_id} is not the selected Patchset for Change {change_id}"
                    ));
                }
                requested
            }
            None => selected_patchset_index,
        };
        let patchset = self
            .read_patchset(&tx, patchset_index)
            .map_err(|error| Self::error(operation, error))?;
        if patchset.change_index != change_index {
            return Err(format!(
                "accepted Patchset does not belong to Change {change_id}"
            ));
        }
        self.require_patchset_governance_authority(
            &tx,
            patchset_index,
            &self.patchset_id(change, patchset.patch_ordinal),
            "direct Land submission",
        )?;
        if patchset.patchset_meta & 0b11 != 0 {
            return Err(format!(
                "selected Patchset for Change {change_id} is withdrawn or invalidated"
            ));
        }
        let patchset_id = self.patchset_id(change, patchset.patch_ordinal);
        let lines =
            ServerBinaryDbLineStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let snapshots =
            ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let (target_line_index, line) = lines
            .line_by_name_in_write(&tx, &target_line)
            .map_err(|error| Self::error(operation, error))?
            .ok_or_else(|| {
                format!("CANONICAL_LINE_MISSING: target line {target_line} does not exist")
            })?;
        let current_head_index = line.head_snapshot_index();
        let current_head = current_head_index
            .map(|index| {
                snapshots
                    .snapshot_id_at_in_write(&tx, index)
                    .map_err(|error| Self::error(operation, error))
            })
            .transpose()?;
        if expected_head.is_some() && expected_head.as_deref() != current_head.as_deref() {
            return Err(format!(
                "STALE_TARGET_LINE: {}:{} expected {:?}, current {:?}",
                self.db.repo_name().as_str(),
                target_line,
                expected_head.as_deref(),
                current_head.as_deref()
            ));
        }
        let base_snapshot_id = snapshots
            .snapshot_id_at_in_write(&tx, patchset.base_snapshot_index)
            .map_err(|error| Self::error(operation, error))?;
        let revision_snapshot_id = snapshots
            .snapshot_id_at_in_write(&tx, patchset.revision_snapshot_index)
            .map_err(|error| Self::error(operation, error))?;
        let current_contains_revision = match current_head_index {
            Some(head) if head != patchset.revision_snapshot_index => snapshots
                .snapshot_chain_contains_ancestor_in_write(
                    &tx,
                    patchset.revision_snapshot_index,
                    head,
                )
                .map_err(|error| Self::error(operation, error))?,
            _ => false,
        };
        let blocked = current_head.as_deref().is_some_and(|head| {
            head != base_snapshot_id && head != revision_snapshot_id && !current_contains_revision
        });
        if atomic_task_land && change.lifecycle() == CHANGE_LIFECYCLE_LANDED {
            let (existing_index, existing) = self
                .latest_succeeded_land(&tx, change_index)?
                .ok_or_else(|| {
                    format!(
                        "Atomic Task Land found landed Change {change_id} without a successful Land record."
                    )
                })?;
            let existing_mode = (existing.land_meta & LAND_MODE_MASK) >> 5;
            if existing.patchset_index != patchset_index
                || existing.target_line_index_plus1 != target_line_index + 1
                || existing_mode != mode
            {
                return Err(format!(
                    "TASK_LAND_IDEMPOTENCY_CONFLICT: landed Change {change_id} does not match the selected Patchset, target Line, or mode."
                ));
            }
            let task_changed = self.complete_task_in_write(&mut tx, change.task_index, now)?;
            #[cfg(feature = "perfetto-tracing")]
            drop(revalidation_trace);
            let result = {
                #[cfg(feature = "perfetto-tracing")]
                let _trace = crate::perfetto_trace::PerfettoRange::new(
                    "ait.server.task_land.atomic.response_projection",
                );
                self.atomic_task_land_result_in_write(
                    &tx,
                    change_index,
                    patchset_index,
                    existing_index,
                    idempotency_key.as_deref().unwrap_or_default(),
                    true,
                )?
            };
            if task_changed {
                {
                    #[cfg(feature = "perfetto-tracing")]
                    let _trace = crate::perfetto_trace::PerfettoRange::new(
                        "ait.server.task_land.atomic.transaction_commit",
                    );
                    tx.commit().map_err(|error| Self::error(operation, error))?;
                }
            }
            drop(tx);
            #[cfg(feature = "perfetto-tracing")]
            drop(writer_trace);
            return Ok(result);
        }
        if atomic_task_land && change.lifecycle() == CHANGE_LIFECYCLE_ARCHIVED {
            return Err(format!(
                "Change {change_id} is archived and cannot be completed by Atomic Task Land."
            ));
        }
        if atomic_task_land && change.change_meta & CHANGE_META_READY_TO_LAND == 0 {
            return Err(format!(
                "TASK_LAND_NOT_READY: Change {change_id} does not have an already-ready selected Patchset. Run `ait workflow ready {change_id} --apply` before Task Land."
            ));
        }
        if atomic_task_land && blocked {
            return Err(format!(
                "TASK_LAND_TARGET_LINE_BLOCKED: target Line {target_line} no longer contains the selected Patchset base for Change {change_id}."
            ));
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(revalidation_trace);
        if atomic_task_land && change.lifecycle() != CHANGE_LIFECYCLE_LANDED {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = crate::perfetto_trace::PerfettoRange::new(
                "ait.server.task_land.atomic.history_receipt_mutation",
            );
            self.apply_history_receipts_in_land_write(
                &mut tx,
                patchset_index,
                change_index,
                &target_line,
                target_line_index,
                current_head_index,
            )?;
        }
        #[cfg(feature = "perfetto-tracing")]
        let aggregate_mutation_trace = atomic_task_land.then(|| {
            crate::perfetto_trace::PerfettoRange::new(
                "ait.server.task_land.atomic.aggregate_land_mutation",
            )
        });
        let mut owner = WorkflowBinaryV0Codec::decode_ordinal_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("change_land_index.bin"),
                change_index,
            )
            .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        let ordinal = owner.next_ordinal;
        let canonical_id = self.land_id(change_id, ordinal);
        if let Some(explicit) = explicit_submission.as_deref() {
            if let Ok(existing_index) = self.land_index_for_id(&tx, explicit) {
                let existing = self.land_at(&BinaryDbReadTxn::new(&self.db), existing_index)?;
                if existing.get("change_ref").and_then(JsonValue::as_str) == Some(change_id)
                    && existing.get("patchset_id").and_then(JsonValue::as_str)
                        == Some(patchset_id.as_str())
                    && existing.get("target_line").and_then(JsonValue::as_str)
                        == Some(target_line.as_str())
                    && existing.get("mode").and_then(JsonValue::as_str) == Some(mode_text.as_str())
                {
                    return Ok(existing);
                }
                return Err(format!(
                    "LAND_IDEMPOTENCY_CONFLICT: submission {explicit} disagrees with this request"
                ));
            }
            if explicit != canonical_id {
                return Err(format!(
                    "Binary DB v0 Land identity is derived as {canonical_id}, not {explicit}"
                ));
            }
        }
        if matches!(
            change.lifecycle(),
            CHANGE_LIFECYCLE_LANDED | CHANGE_LIFECYCLE_ARCHIVED
        ) {
            return Err(format!(
                "Change {change_id} cannot accept a new Land attempt"
            ));
        }
        if owner.next_ordinal >= 64 {
            return Err(format!(
                "Change {change_id} has exhausted its v0 Land ordinals"
            ));
        }
        let mut inventory =
            self.task_inventory_in_write(&tx, "task_land_index.bin", change.task_index)?;
        let succeeded = !blocked;
        let landed_index = if succeeded {
            if current_head.as_deref() == Some(revision_snapshot_id.as_str())
                || current_contains_revision
            {
                current_head_index.expect("successful contained revision has a line head")
            } else {
                patchset.revision_snapshot_index
            }
        } else {
            0
        };
        let mut land_meta = if succeeded {
            LAND_STATUS_SUCCEEDED | LAND_HAS_LANDED_SNAPSHOT
        } else {
            LAND_STATUS_BLOCKED
        } | (mode << 5);
        if current_head_index.is_some() {
            land_meta |= LAND_HAS_PRE_TARGET;
        }
        if succeeded && landed_index != current_head_index.unwrap_or(u32::MAX) {
            lines
                .set_line_head_in_tx(&mut tx, &target_line, current_head_index, landed_index, now)
                .map_err(|error| Self::error(operation, error))?;
        }
        let record = V0LandRecord {
            land_meta,
            land_ordinal: ordinal,
            change_ordinal: change.change_ordinal,
            failure_kind: if blocked { 1 } else { 0 },
            change_index,
            patchset_index,
            previous_task_land_index_plus1: inventory.latest_index_plus1,
            previous_change_land_index_plus1: owner.latest_index_plus1,
            pre_land_target_snapshot_index_plus1: current_head_index.map_or(0, |index| index + 1),
            landed_snapshot_index_plus1: if succeeded { landed_index + 1 } else { 0 },
            submitted_at_s: now,
            updated_at_s: now,
            target_line_index_plus1: target_line_index + 1,
        };
        let land_index = tx
            .append_record(
                WorkflowBinaryV0Codec::land_file(),
                &WorkflowBinaryV0Codec::encode_land(record)
                    .map_err(|error| Self::error(operation, error))?,
            )
            .map_err(|error| Self::error(operation, error))?;
        owner.latest_index_plus1 = land_index + 1;
        owner.count = owner
            .count
            .checked_add(1)
            .ok_or_else(|| "Change Land count exceeds u16".to_string())?;
        owner.next_ordinal += 1;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("change_land_index.bin"),
            change_index,
            &WorkflowBinaryV0Codec::encode_ordinal_index(owner)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        inventory.latest_index_plus1 = land_index + 1;
        inventory.count = inventory
            .count
            .checked_add(1)
            .ok_or_else(|| "Task Land count exceeds u16".to_string())?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_land_index.bin"),
            change.task_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(inventory)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if succeeded {
            change.change_meta =
                (change.change_meta & CHANGE_META_HAS_PATCHSETS) | CHANGE_LIFECYCLE_LANDED;
        } else {
            change.change_meta |= CHANGE_META_BLOCKED;
            change.change_meta &= !CHANGE_META_READY_TO_LAND;
        }
        change.updated_at_s = now;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::change_file(),
            change_index,
            &WorkflowBinaryV0Codec::encode_change(change)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if atomic_task_land && succeeded {
            self.complete_task_in_write(&mut tx, change.task_index, now)?;
        }
        #[cfg(feature = "perfetto-tracing")]
        drop(aggregate_mutation_trace);
        let atomic_result = if atomic_task_land {
            Some({
                #[cfg(feature = "perfetto-tracing")]
                let _trace = crate::perfetto_trace::PerfettoRange::new(
                    "ait.server.task_land.atomic.response_projection",
                );
                self.atomic_task_land_result_in_write(
                    &tx,
                    change_index,
                    patchset_index,
                    land_index,
                    idempotency_key.as_deref().unwrap_or_default(),
                    false,
                )?
            })
        } else {
            None
        };
        {
            #[cfg(feature = "perfetto-tracing")]
            let _trace = atomic_task_land.then(|| {
                crate::perfetto_trace::PerfettoRange::new(
                    "ait.server.task_land.atomic.transaction_commit",
                )
            });
            tx.commit().map_err(|error| Self::error(operation, error))?;
        }
        drop(tx);
        #[cfg(feature = "perfetto-tracing")]
        drop(writer_trace);
        if let Some(result) = atomic_result {
            return Ok(result);
        }
        let read = BinaryDbReadTxn::new(&self.db);
        self.land_at(&read, land_index)
    }

    fn get_land(&self, repo_name: Option<&str>, submission_id: &str) -> Result<JsonValue, String> {
        if let Some(repo_name) = repo_name {
            self.repo_scope("ServerWorkflowLandStore::get_land", repo_name)?;
        }
        let read = BinaryDbReadTxn::new(&self.db);
        let index = self.land_index_for_id(&read, submission_id)?;
        self.land_at(&read, index)
    }
}
