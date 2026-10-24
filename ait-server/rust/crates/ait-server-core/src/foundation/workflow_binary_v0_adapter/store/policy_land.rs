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

    pub(super) fn require_finish_evidence<A: ReadV0>(
        &self,
        read: &A,
        index: u32,
        patchset: V0PatchsetRecord,
    ) -> Result<(), String> {
        let task = self
            .read_task(read, patchset.task_index)
            .map_err(|e| e.to_string())?;
        let (_, attestation) = self
            .latest_attestation_for_patchset(read, index)?
            .ok_or("TASK_LAND_NOT_READY: current candidate has no Attestation")?;
        let (_, policy) = self
            .latest_policy_decision(read, index)?
            .ok_or("TASK_LAND_NOT_READY: current candidate has no Policy decision")?;
        if task.task_meta & TASK_META_READY_TO_LAND == 0
            || task.task_meta
                & (TASK_META_REVIEW_PENDING | TASK_META_VALIDATION_PENDING | TASK_META_BLOCKED)
                != 0
            || patchset.patchset_meta & PATCHSET_EVALUATION_PENDING != 0
            || attestation.patch_ordinal != patchset.patch_ordinal
            || attestation.change_ordinal_plus1 != patchset.change_ordinal_plus1
            || attestation.attest_meta & ATTEST_REVOKED != 0
            || attestation.attest_meta & ATTEST_VERIFICATION_MASK != 2
            || policy.patch_ordinal != patchset.patch_ordinal
            || policy.change_ordinal_plus1 != patchset.change_ordinal_plus1
            || policy.policy_meta & POLICY_DECISION_MASK != 1
        {
            return Err("TASK_LAND_NOT_READY: exact current candidate evidence is pending, invalid or blocked".to_string());
        }
        if attestation.attest_meta & ATTEST_REQUIRE_TESTS_PASS != 0
            && (patchset.ci_completed_at_s == 0
                || patchset.ci_run_seq == 0
                || patchset.ci_status(CI_STATUS_OVERALL_SHIFT) != CI_STATUS_PASS
                || patchset.ci_status(CI_STATUS_TESTS_SHIFT) != CI_STATUS_PASS
                || patchset.ci_suite_result_count == 0
                || patchset.ci_blocking_failure_count != 0)
        {
            return Err("TASK_LAND_NOT_READY: exact Patchset CI has not passed".to_string());
        }
        if attestation.attest_meta & ATTEST_REQUIRE_LINT_PASS != 0
            && (patchset.ci_completed_at_s == 0
                || patchset.ci_status(CI_STATUS_LINT_SHIFT) != CI_STATUS_PASS)
        {
            return Err("TASK_LAND_NOT_READY: exact Patchset lint has not passed".to_string());
        }
        if attestation.attest_meta & ATTEST_REQUIRE_HUMAN_REVIEW != 0
            && self.live_human_review_approvals(read, index)? == 0
        {
            return Err("TASK_LAND_NOT_READY: exact Patchset human Review is missing".to_string());
        }
        Ok(())
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
        let (workflow_owner, patchset) =
            self.owner_for_patchset_index(read, record.patchset_index)?;
        if record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal_plus1 != patchset.change_ordinal_plus1
        {
            return Err("Binary DB v0 Attestation ownership ordinals disagree".to_string());
        }
        let patchset_id = self.patchset_public_id(patchset);
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
            "attestation_id": self.attestation_id(workflow_owner.task_index, record.attest_ordinal),
            "repo_id": self.db.repo_id().as_str(),
            "task_id": self.task_id(workflow_owner.task_index),
            "change_id": workflow_owner.change.map(|(_, change)| format!("C-{:02}", change.change_ordinal + 1)),
            "change_ref": workflow_owner.change.map(|(_, change)| self.change_ref(workflow_owner.task_index, change.change_ordinal)),
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
        let (_, patchset) = self.owner_for_patchset_index(read, record.patchset_index)?;
        if record.patch_ordinal != patchset.patch_ordinal
            || record.change_ordinal_plus1 != patchset.change_ordinal_plus1
        {
            return Err("Binary DB v0 Policy ownership ordinals disagree".to_string());
        }
        let patchset_id = self.patchset_public_id(patchset);
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

    fn live_human_review_approvals<A: ReadV0>(
        &self,
        read: &A,
        patchset_index: u32,
    ) -> Result<usize, String> {
        let count = read
            .record_count_v0(WorkflowBinaryV0Codec::review_file())
            .map_err(|error| Self::error("Policy Review read", error))?;
        let mut decisions = BTreeMap::new();
        for index in 0..count {
            let review = WorkflowBinaryV0Codec::decode_review(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::review_file(), index)
                    .map_err(|error| Self::error("Policy Review read", error))?,
            )
            .map_err(|error| Self::error("Policy Review decode", error))?;
            if review.patchset_index == patchset_index && review.review_meta & REVIEW_TOMBSTONE == 0
            {
                let actor_index = review
                    .actor_index_plus1
                    .checked_sub(1)
                    .ok_or_else(|| "Policy Review has no Actor".to_string())?;
                match review.review_meta & REVIEW_ACTION_MASK {
                    2 => {
                        decisions.insert(actor_index, true);
                    }
                    3 | 4 => {
                        decisions.insert(actor_index, false);
                    }
                    _ => {}
                }
            }
        }
        let mut approvals = 0;
        for (actor_index, approved) in decisions {
            if approved && self.actor_at(read, actor_index)?.0.actor_meta & 0b111 == 0 {
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
        let (task_id, _) = submission_id
            .rsplit_once("/L-")
            .ok_or("Invalid Task Land identity")?;
        let task_index = self.task_index_for_id(read, task_id)?;
        let ordinal = Self::parse_owned_ordinal(submission_id, task_id, "L")?;
        let mut cursor = self
            .task_inventory(read, "task_land_index.bin", task_index)?
            .latest_index_plus1
            .checked_sub(1);
        let mut seen = BTreeSet::new();
        while let Some(index) = cursor {
            if !seen.insert(index) {
                return Err("Task Land inventory has a cycle".to_string());
            }
            let land = WorkflowBinaryV0Codec::decode_land(
                &read
                    .read_record_v0(WorkflowBinaryV0Codec::land_file(), index)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if land.task_index != task_index {
                return Err("Task Land owner disagrees".to_string());
            }
            if land.land_meta & LAND_TOMBSTONE == 0 && land.land_ordinal == ordinal {
                return Ok(index);
            }
            cursor = land.previous_task_land_index_plus1.checked_sub(1);
        }
        Err(format!("Unknown Land {submission_id}"))
    }
}

impl<D> ServerWorkflowAttestationStore for BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    fn put_attestation(&self, patchset_id: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error("Attestation write", error))?;
        let index = self.put_attestation_in_write(&mut tx, patchset_id, payload, Self::now_s()?)?;
        tx.commit()
            .map_err(|error| Self::error("Attestation commit", error))?;
        self.attestation_at(&BinaryDbReadTxn::new(&self.db), index)
    }

    fn get_attestation(&self, patchset_id: &str) -> Result<JsonValue, String> {
        let read = BinaryDbReadTxn::new(&self.db);
        let patchset_index = self.patchset_index_for_id(&read, patchset_id)?;
        let (index, _) = self
            .latest_attestation_for_patchset(&read, patchset_index)?
            .ok_or_else(|| {
                crate::foundation::remote_binary_db::binary_db_typed_runtime_error(
                    "Attestation read",
                    BinaryDbError::missing_data(format!(
                        "Unknown attestation for patchset: {patchset_id}"
                    )),
                )
            })?;
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
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerWorkflow)
                .map_err(|error| Self::error("Policy write", error))?;
        let index = self.evaluate_policy_in_write(&mut tx, patchset_id, Self::now_s()?)?;
        tx.commit()
            .map_err(|error| Self::error("Policy commit", error))?;
        self.policy_at(&BinaryDbReadTxn::new(&self.db), index)
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
        let (workflow_owner, mut patchset) = self.owner_for_patchset_index(&tx, patchset_index)?;
        self.require_current_patchset(&tx, patchset_index, patchset)?;
        self.require_active_owner(&tx, workflow_owner)?;
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
        self.update_owner_state(
            &mut tx,
            workflow_owner,
            TASK_META_VALIDATION_PENDING,
            TASK_META_READY_TO_LAND | TASK_META_BLOCKED,
            now,
            None,
            |change| {
                change.change_meta |= CHANGE_META_VALIDATION_PENDING;
                change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
            },
        )?;
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
        let (workflow_owner, mut patchset) = self.owner_for_patchset_index(&tx, patchset_index)?;
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
        if self
            .current_task_patchset(&tx, workflow_owner.task_index)?
            .map(|(index, _)| index)
            == Some(patchset_index)
            && !self
                .read_task(&tx, workflow_owner.task_index)
                .map_err(|e| e.to_string())?
                .is_terminal()
        {
            self.update_owner_state(
                &mut tx,
                workflow_owner,
                TASK_META_VALIDATION_PENDING,
                TASK_META_READY_TO_LAND | TASK_META_BLOCKED,
                now,
                None,
                |change| {
                    change.change_meta |= CHANGE_META_VALIDATION_PENDING;
                    change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
                },
            )?;
        }
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

    fn submit_land(&self, reference: &str, payload: &JsonValue) -> Result<JsonValue, String> {
        self.request_actor(Self::required_object(payload, "Land submit payload")?)?;
        let mut tx = BinaryDbWriteTxn::begin_serving(&self.db, BinaryDbCommandScope::ServerLand)
            .map_err(|error| error.to_string())?;
        let (result, _, changed) = self.submit_land_in_write(&mut tx, reference, payload, false)?;
        if changed {
            tx.commit().map_err(|error| error.to_string())?;
        }
        Ok(result)
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

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn package_attestation_meta(
        payload: &JsonMap<String, JsonValue>,
        author_mode_bits: u8,
        ci_backed: bool,
    ) -> Result<u8, String> {
        if let Some(author_mode) = Self::optional_text(payload, "author_mode") {
            let expected = Self::author_mode_bits(&author_mode)?;
            if author_mode_bits & PATCHSET_AUTHOR_MODE_MASK != expected {
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
        if ci_backed {
            attest_meta |= ATTEST_CI_BACKED;
        }
        Ok(attest_meta)
    }
    pub(super) fn put_attestation_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        patchset_id: &str,
        payload: &JsonValue,
        now: u64,
    ) -> Result<u32, String> {
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
        let patchset_index = self.patchset_index_for_id(tx, patchset_id)?;
        self.require_patchset_governance_authority(
            tx,
            patchset_index,
            patchset_id,
            "Attestation mutation",
        )?;
        let (workflow_owner, mut patchset) = self.owner_for_patchset_index(tx, patchset_index)?;
        self.require_current_patchset(tx, patchset_index, patchset)?;
        self.require_active_owner(tx, workflow_owner)?;
        let attest_meta = Self::package_attestation_meta(
            payload,
            patchset.patchset_meta,
            patchset.ci_completed_at_s != 0,
        )?;
        let mut task_owner = WorkflowBinaryV0Codec::decode_ordinal_index(
            &tx.read_record(
                WorkflowBinaryV0Codec::chain_index_file("task_attest_index.bin"),
                workflow_owner.task_index,
            )
            .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if task_owner.next_ordinal >= 64 {
            return Err(format!(
                "Task {} has exhausted its v0 Attestation ordinals",
                self.task_id(workflow_owner.task_index)
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
            change_ordinal_plus1: patchset.change_ordinal_plus1,
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
            workflow_owner.task_index,
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
                .encode_patchset_replacement(tx, patchset_index, patchset)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        self.update_owner_state(
            tx,
            workflow_owner,
            TASK_META_VALIDATION_PENDING,
            TASK_META_READY_TO_LAND | TASK_META_BLOCKED,
            now,
            None,
            |change| {
                change.change_meta |= CHANGE_META_VALIDATION_PENDING;
                change.change_meta &= !(CHANGE_META_READY_TO_LAND | CHANGE_META_BLOCKED);
            },
        )?;
        Ok(attest_index)
    }
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn evaluate_policy_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        patchset_id: &str,
        now: u64,
    ) -> Result<u32, String> {
        let operation = "ServerWorkflowPolicyStore::evaluate_policy";
        let patchset_index = self.patchset_index_for_id(tx, patchset_id)?;
        self.require_patchset_governance_authority(
            tx,
            patchset_index,
            patchset_id,
            "Policy evaluation",
        )?;
        let (workflow_owner, mut patchset) = self.owner_for_patchset_index(tx, patchset_index)?;
        self.require_current_patchset(tx, patchset_index, patchset)?;
        self.require_active_owner(tx, workflow_owner)?;
        let attestation = self
            .latest_attestation_for_patchset(tx, patchset_index)?
            .map(|(_, record)| record);
        let approvals = self.live_human_review_approvals(tx, patchset_index)?;
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
            check_status: if attestation.is_some_and(|record| {
                record.attest_meta & ATTEST_REVOKED == 0
                    && record.attest_meta & ATTEST_VERIFICATION_MASK == 2
            }) {
                3
            } else {
                2
            },
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
            self.task_inventory_in_write(tx, "task_policy_index.bin", workflow_owner.task_index)?;
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
        self.sync_file(tx, "policy_check.bin")
            .map_err(|error| Self::error(operation, error))?;
        let record = V0PolicyRecord {
            policy_meta: decision_kind,
            policy_ordinal: owner.next_ordinal,
            patch_ordinal: patchset.patch_ordinal,
            change_ordinal_plus1: patchset.change_ordinal_plus1,
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
            workflow_owner.task_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(inventory)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        if decision_kind == 1 {
            patchset.patchset_meta &= !PATCHSET_EVALUATION_PENDING;
        } else {
            patchset.patchset_meta |= PATCHSET_EVALUATION_PENDING;
        }
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            patchset_index,
            &self
                .encode_patchset_replacement(tx, patchset_index, patchset)
                .map_err(|error| Self::error(operation, error))?,
        )
        .map_err(|error| Self::error(operation, error))?;
        let task = self
            .read_task(tx, workflow_owner.task_index)
            .map_err(|e| e.to_string())?;
        let ready = decision_kind == 1 && task.task_meta & TASK_META_REVIEW_PENDING == 0;
        self.update_owner_state(
            tx,
            workflow_owner,
            if ready {
                TASK_META_READY_TO_LAND
            } else if decision_kind == 0 {
                TASK_META_VALIDATION_PENDING
            } else {
                0
            },
            TASK_META_READY_TO_LAND
                | if decision_kind == 1 {
                    TASK_META_VALIDATION_PENDING | TASK_META_BLOCKED
                } else {
                    0
                },
            now,
            None,
            |change| {
                if decision_kind == 1 {
                    change.change_meta &= !(CHANGE_META_VALIDATION_PENDING | CHANGE_META_BLOCKED);
                    if change.change_meta & CHANGE_META_REVIEW_PENDING == 0 {
                        change.change_meta |= CHANGE_META_READY_TO_LAND;
                    }
                } else {
                    change.change_meta |= CHANGE_META_VALIDATION_PENDING;
                    change.change_meta &= !CHANGE_META_READY_TO_LAND;
                }
            },
        )?;
        Ok(policy_index)
    }
}

impl<D> BinaryDbServerWorkflowV0Store<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone + Send + Sync + 'static,
{
    pub(super) fn submit_land_in_write<F: BinaryDbFsyncPolicy>(
        &self,
        tx: &mut BinaryDbWriteTxn<'_, D, F>,
        reference: &str,
        payload: &JsonValue,
        package: bool,
    ) -> Result<(JsonValue, u32, bool), String> {
        let payload = Self::required_object(payload, "Land submit payload")?;
        let contract = Self::optional_text(payload, "contract");
        let atomic = matches!(
            contract.as_deref(),
            Some(super::task_land::ATOMIC_CONTRACT | super::task_land::GUARDED_CONTRACT)
        );
        if contract.is_some() && !atomic {
            return Err("Unsupported Task Land contract".to_string());
        }
        let expected = if contract.as_deref() == Some(super::task_land::GUARDED_CONTRACT) {
            Some(
                payload
                    .get("expected_identity")
                    .ok_or("Guarded atomic Task Land requires expected_identity")?,
            )
        } else {
            None
        };
        let key = if atomic {
            Self::required_text(payload, "idempotency_key")?
        } else {
            String::new()
        };
        let target = Self::required_text(payload, "target_line")?;
        if self.task_prefix.starts_with('R') {
            crate::foundation::workflow_package::require_ordinary_remote_line(&target)?;
        }
        let mode_text = Self::required_text(payload, "mode")?;
        let mode = match mode_text.as_str() {
            "direct" => LAND_MODE_DIRECT,
            "merge" => LAND_MODE_MERGE,
            "ff-only" => LAND_MODE_FF_ONLY,
            _ => return Err(format!("Unsupported Land mode {mode_text}")),
        };
        let requested_patchset = Self::optional_text(payload, "patchset_id");
        let expected_head = Self::optional_text(payload, "expected_head_snapshot_id")
            .or_else(|| Self::optional_text(payload, "expected_target_line_head"));
        let actor_index_plus1 = self.request_actor_in_write(tx, payload)?;
        let requested = self.workflow_owner(tx, reference)?;
        if atomic {
            let path_ref = Self::required_text(payload, "task_or_change_ref")?;
            if self.owner_ref(self.workflow_owner(tx, &path_ref)?) != self.owner_ref(requested) {
                return Err("Atomic Task Land reference changed during admission".to_string());
            }
        }
        let lines =
            ServerBinaryDbLineStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        let (target_index, line) = lines
            .line_by_name_in_write(tx, &target)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!("CANONICAL_LINE_MISSING: target Line {target} does not exist")
            })?;
        // A successful receipt is authoritative even if the target or latest candidate subsequently changes.
        if let Some((land_index, receipt)) =
            self.task_successful_receipt(tx, requested.task_index)?
        {
            let patchset = self
                .read_patchset(tx, receipt.patchset_index)
                .map_err(|e| e.to_string())?;
            self.require_patchset_owner(tx, reference, patchset)?;
            self.validate_expected_patchset_in_read(tx, receipt.patchset_index, expected)?;
            let original_head = receipt
                .pre_land_target_snapshot_index_plus1
                .checked_sub(1)
                .map(|index| self.content_snapshot_id(tx, index))
                .transpose()?;
            if receipt.target_line_index_plus1 != target_index + 1
                || (receipt.land_meta & LAND_MODE_MASK) >> 5 != mode
                || requested_patchset.as_ref().is_some_and(|id| {
                    self.patchset_index_for_id(tx, id).ok() != Some(receipt.patchset_index)
                })
                || expected_head
                    .as_ref()
                    .is_some_and(|head| Some(head.as_str()) != original_head.as_deref())
                || receipt.landed_snapshot_index_plus1 != patchset.revision_snapshot_index + 1
            {
                return Err("TASK_LAND_IDEMPOTENCY_CONFLICT: accepted candidate, target, mode or applied result differs".to_string());
            }
            if Self::optional_text(payload, "submission_id").is_some_and(|id| {
                id != self.land_id(&self.task_id(requested.task_index), receipt.land_ordinal)
            }) {
                return Err(
                    "LAND_IDEMPOTENCY_CONFLICT: accepted submission identity differs".to_string(),
                );
            }
            let completed = self.complete_task_in_write(
                tx,
                requested.task_index,
                receipt.updated_at_s,
                receipt.actor_index_plus1,
            )?;
            let result = if atomic {
                self.atomic_task_land_result_in_write(
                    tx,
                    requested.task_index,
                    receipt.patchset_index,
                    land_index,
                    &key,
                    true,
                    expected,
                )?
            } else {
                self.land_at(tx, land_index)?
            };
            return Ok((result, land_index, completed));
        }
        let (patchset_index, patchset) = self
            .current_task_patchset(tx, requested.task_index)?
            .ok_or("Task has no current Patchset")?;
        let owner = self.require_patchset_owner(tx, reference, patchset)?;
        self.require_current_patchset(tx, patchset_index, patchset)?;
        self.require_active_owner(tx, owner)?;
        self.validate_expected_patchset_in_read(tx, patchset_index, expected)?;
        if requested_patchset
            .as_ref()
            .is_some_and(|id| self.patchset_index_for_id(tx, id).ok() != Some(patchset_index))
        {
            return Err(
                "EXPECTED_CANDIDATE_MISMATCH: requested Patchset is no longer current".to_string(),
            );
        }
        self.require_patchset_governance_authority(
            tx,
            patchset_index,
            &self.patchset_public_id(patchset),
            "Remote acceptance",
        )?;
        if !atomic
            && self
                .history_manifest_for_patchset(tx, patchset_index)?
                .is_some()
        {
            return Err(
                "AGGREGATE_ATOMIC_FINISH_REQUIRED: aggregate acceptance requires atomic Task Land"
                    .to_string(),
            );
        }
        self.require_finish_evidence(tx, patchset_index, patchset)?;
        let head = line.head_snapshot_index();
        let head_id = head
            .map(|index| self.content_snapshot_id(tx, index))
            .transpose()?;
        if expected_head
            .as_deref()
            .is_some_and(|expected| Some(expected) != head_id.as_deref())
        {
            return Err("STALE_TARGET_LINE: expected target head differs".to_string());
        }
        let snapshots =
            ServerBinaryDbSnapshotStore::<D, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(self.db.clone());
        if head == Some(patchset.revision_snapshot_index) {
            return Err("REMOTE_APPLY_RECEIPT_MISSING: target already has the candidate without this Task's receipt".to_string());
        }
        if let Some(head) = head {
            if snapshots
                .snapshot_chain_contains_ancestor_in_write(
                    tx,
                    patchset.revision_snapshot_index,
                    head,
                )
                .map_err(|e| e.to_string())?
            {
                return Err("REMOTE_APPLY_RECEIPT_MISSING: target already contains the candidate without this Task's receipt".to_string());
            }
        }
        if head != Some(patchset.base_snapshot_index) {
            return Err(
                "TASK_LAND_TARGET_LINE_BLOCKED: target no longer equals the exact candidate base"
                    .to_string(),
            );
        }
        if package {
            self.apply_history_receipts_with_package_mode(
                tx,
                patchset_index,
                owner.task_index,
                &target,
                target_index,
                head,
                true,
            )?;
        } else {
            self.apply_history_receipts_in_land_write(
                tx,
                patchset_index,
                owner.task_index,
                &target,
                target_index,
                head,
            )?;
        }
        let mut inventory = self.task_inventory(tx, "task_land_index.bin", owner.task_index)?;
        let mut cursor = inventory.latest_index_plus1.checked_sub(1);
        let mut seen = BTreeSet::new();
        let mut ordinals = BTreeSet::new();
        while let Some(index) = cursor {
            if !seen.insert(index) {
                return Err("Task Land inventory has a cycle".to_string());
            }
            let land = WorkflowBinaryV0Codec::decode_land(
                &tx.read_record(WorkflowBinaryV0Codec::land_file(), index)
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
            if land.task_index != owner.task_index || !ordinals.insert(land.land_ordinal) {
                return Err("Task Land owner/ordinal disagrees".to_string());
            }
            cursor = land.previous_task_land_index_plus1.checked_sub(1);
        }
        if seen.len() != usize::from(inventory.count) {
            return Err("Task Land count disagrees".to_string());
        }
        let ordinal = ordinals.last().map_or(0, |n| n + 1);
        if ordinal >= 64 {
            return Err("Task has exhausted its Land ordinals".to_string());
        }
        if Self::optional_text(payload, "submission_id")
            .is_some_and(|id| id != self.land_id(&self.task_id(owner.task_index), ordinal))
        {
            return Err("LAND_IDEMPOTENCY_CONFLICT: next Task Land identity differs".to_string());
        }
        let mut change_inventory = owner
            .change
            .map(|(index, _)| self.owner_ordinal_index(tx, "change_land_index.bin", index))
            .transpose()?;
        let now = Self::now_s()?;
        lines
            .set_line_head_in_tx(tx, &target, head, patchset.revision_snapshot_index, now)
            .map_err(|e| e.to_string())?;
        let receipt = V0LandRecord {
            actor_index_plus1,
            land_meta: LAND_STATUS_SUCCEEDED
                | LAND_HAS_PRE_TARGET
                | LAND_HAS_LANDED_SNAPSHOT
                | (mode << 5),
            land_ordinal: ordinal,
            change_ordinal_plus1: owner.change_ordinal_plus1(),
            task_index: owner.task_index,
            patchset_index,
            previous_task_land_index_plus1: inventory.latest_index_plus1,
            previous_change_land_index_plus1: change_inventory
                .as_ref()
                .map_or(0, |i| i.latest_index_plus1),
            pre_land_target_snapshot_index_plus1: patchset.base_snapshot_index + 1,
            landed_snapshot_index_plus1: patchset.revision_snapshot_index + 1,
            submitted_at_s: now,
            updated_at_s: now,
            target_line_index_plus1: target_index + 1,
            failure_kind: 0,
        };
        let land_index = tx
            .append_record(
                WorkflowBinaryV0Codec::land_file(),
                &WorkflowBinaryV0Codec::encode_land(receipt).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        inventory.latest_index_plus1 = land_index + 1;
        inventory.count = inventory
            .count
            .checked_add(1)
            .ok_or("Task Land count exceeds u16")?;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::chain_index_file("task_land_index.bin"),
            owner.task_index,
            &WorkflowBinaryV0Codec::encode_inventory_index(inventory).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if let (Some((change_index, _)), Some(index)) = (owner.change, change_inventory.as_mut()) {
            index.latest_index_plus1 = land_index + 1;
            index.count = index
                .count
                .checked_add(1)
                .ok_or("Change Land count exceeds u16")?;
            index.next_ordinal = ordinal + 1;
            tx.overwrite_record(
                WorkflowBinaryV0Codec::chain_index_file("change_land_index.bin"),
                change_index,
                &WorkflowBinaryV0Codec::encode_ordinal_index(*index).map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        self.update_owner_state(
            tx,
            owner,
            0,
            TASK_META_READY_TO_LAND
                | TASK_META_BLOCKED
                | TASK_META_REVIEW_PENDING
                | TASK_META_VALIDATION_PENDING,
            now,
            Some(actor_index_plus1),
            |change| {
                change.change_meta =
                    (change.change_meta & CHANGE_META_HAS_PATCHSETS) | CHANGE_LIFECYCLE_LANDED
            },
        )?;
        self.complete_task_in_write(tx, owner.task_index, now, actor_index_plus1)?;
        let result = if atomic {
            self.atomic_task_land_result_in_write(
                tx,
                owner.task_index,
                patchset_index,
                land_index,
                &key,
                false,
                expected,
            )?
        } else {
            self.land_at(tx, land_index)?
        };
        Ok((result, land_index, true))
    }
}
