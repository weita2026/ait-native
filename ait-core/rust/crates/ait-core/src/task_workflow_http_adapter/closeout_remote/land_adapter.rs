use super::*;

#[derive(Clone, Copy)]
struct AtomicTaskLandRequest<'a> {
    task_or_change_ref: &'a str,
    target_line: Option<&'a str>,
    mode: &'a str,
    idempotency_key: &'a str,
    repo_name: Option<&'a str>,
    response_deadline_ms: Option<u64>,
    expected: Option<&'a crate::workflow_expected_identity::WorkflowExpectedIdentity>,
}

impl HttpWorkflowCloseoutRemote {
    pub fn submit_task_land_guarded(
        &mut self,
        task_or_change_ref: &str,
        target_line: Option<&str>,
        mode: &str,
        idempotency_key: &str,
        repo_name: Option<&str>,
        expected: Option<&crate::workflow_expected_identity::WorkflowExpectedIdentity>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let response_deadline_ms = remote_task_land_response_deadline_timeout_ms()
            .map(|timeout_ms| timeout_ms.min(self.manager.config.default_timeout_ms));
        let request = AtomicTaskLandRequest {
            task_or_change_ref,
            target_line,
            mode,
            idempotency_key,
            repo_name,
            response_deadline_ms,
            expected,
        };
        match self.submit_task_land_once(request, response_deadline_ms) {
            Ok(result) => Ok(result),
            Err(error) if error.is_retryable_busy() || is_remote_mutation_timeout(&error) => {
                self.resume_task_land_after_retryable_response(request, error)
            }
            Err(error) => Err(error),
        }
    }
    pub fn submit_task_land(
        &mut self,
        task_or_change_ref: &str,
        target_line: Option<&str>,
        mode: &str,
        idempotency_key: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        submit_task_land_with_task_workflow_closeout_remote(
            self,
            task_or_change_ref,
            target_line,
            mode,
            idempotency_key,
            repo_name,
        )
    }

    pub fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        submit_land_with_task_workflow_closeout_remote(
            self,
            change_id,
            patchset_id,
            target_line,
            mode,
            repo_name,
        )
    }

    pub fn get_land(
        &mut self,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_land_with_task_workflow_closeout_remote(self, submission_id, repo_name)
    }

    pub fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        retry_land_with_task_workflow_closeout_remote(self, submission_id, reason, repo_name)
    }

    pub fn close_task(
        &mut self,
        task_id: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        close_task_with_task_workflow_closeout_remote(self, task_id, status, repo_name)
    }

    pub(super) fn resolve_change_row(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let wire_change_id = self.wire_change_id(change_id)?;
        match self.manager.get_change(&wire_change_id, repo_name) {
            Ok(change) => {
                let expected_task_id = self.expected_task_id_for_change(change_id);
                self.normalize_change(&change, expected_task_id.as_deref())
            }
            Err(err) => self.recover_change_via_repo_listing(change_id, repo_name, err),
        }
    }

    fn recover_change_via_repo_listing(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        original_error: TaskWorkflowHttpClientError,
    ) -> TaskWorkflowHttpClientResult<Value> {
        if !change_read_error_allows_listing_recovery(&original_error) {
            return Err(original_error);
        }
        let Some(repo_name) = repo_name else {
            return Err(original_error);
        };
        let canonical = ChangeJson::stateless()
            .canonical_change_id(change_id)
            .map_err(PlanHttpClientError::Invalid)?;
        let expected_task_id = self.expected_task_id_for_change(change_id);
        let rows = self.manager.list_changes(repo_name)?;
        rows.into_iter()
            .filter_map(|row| self.normalize_change(&row, None).ok())
            .find(|row| {
                change_matches_reference(row, &canonical)
                    && expected_task_id.as_deref().is_none_or(|task_id| {
                        row.get("task_id").and_then(Value::as_str) == Some(task_id)
                    })
            })
            .ok_or(original_error)
    }

    pub(super) fn recover_remote_published_patchset(
        &mut self,
        change_id: &str,
        repo_name: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
    ) -> TaskWorkflowHttpClientResult<Option<Value>> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(recovered) = self.matching_current_candidate(
                change_id,
                Some(repo_name),
                base_snapshot_id,
                revision_snapshot_id,
            )? {
                return Ok(Some(recovered));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            sleep(Duration::from_millis(250));
        }
    }

    pub(super) fn with_temporary_default_timeout<T, F>(
        &mut self,
        timeout_ms: Option<u64>,
        operation: F,
    ) -> TaskWorkflowHttpClientResult<T>
    where
        F: FnOnce(&mut Self) -> TaskWorkflowHttpClientResult<T>,
    {
        let Some(timeout_ms) = timeout_ms else {
            return operation(self);
        };
        let original_timeout_ms = self.manager.config.default_timeout_ms;
        self.manager.config.default_timeout_ms = timeout_ms;
        let result = operation(self);
        self.manager.config.default_timeout_ms = original_timeout_ms;
        result
    }

    fn submit_land_once(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
        timeout_ms: Option<u64>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let change_ref = self.wire_change_id(change_id)?;
        let result = self.with_temporary_default_timeout(timeout_ms, |remote| {
            remote
                .manager
                .submit_land(&change_ref, patchset_id, target_line, mode, repo_name)
        })?;
        self.normalize_change_identity_payload(result, change_id)
    }

    fn atomic_task_land_wire_reference(
        &self,
        task_or_change_ref: &str,
    ) -> TaskWorkflowHttpClientResult<String> {
        let requested = task_or_change_ref.trim();
        if requested.is_empty() {
            return Err(PlanHttpClientError::Invalid(
                "Atomic Task Land task_or_change_ref must not be empty.".to_string(),
            ));
        }
        if requested.contains("/C-") {
            return Ok(requested.to_string());
        }
        if self.bound_change_id.as_deref() == Some(requested) {
            return self.bound_change_ref.clone().ok_or_else(|| {
                PlanHttpClientError::Invalid(format!(
                    "Atomic Task Land cannot derive an exact Change reference for `{requested}`."
                ))
            });
        }
        Ok(requested.to_string())
    }

    fn submit_task_land_once(
        &mut self,
        request: AtomicTaskLandRequest<'_>,
        timeout_ms: Option<u64>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let wire_reference = self.atomic_task_land_wire_reference(request.task_or_change_ref)?;
        let result = self.with_temporary_default_timeout(timeout_ms, |remote| {
            remote.manager.submit_task_land_guarded(
                &wire_reference,
                request.target_line,
                request.mode,
                request.idempotency_key,
                request.repo_name,
                request.expected,
            )
        })?;
        let result = self.normalize_atomic_task_land_payload(
            result,
            &wire_reference,
            request.idempotency_key,
            request.repo_name,
            request.expected,
        )?;
        if request
            .target_line
            .is_some_and(|line| result["target_line"].as_str() != Some(line))
            || result["land"]["mode"].as_str() != Some(request.mode)
        {
            return Err(PlanHttpClientError::Invalid(
                "Atomic response target Line or mode differs from request.".to_string(),
            ));
        }
        Ok(result)
    }

    fn resume_task_land_after_retryable_response(
        &mut self,
        request: AtomicTaskLandRequest<'_>,
        mut last_error: TaskWorkflowHttpClientError,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let in_flight_request_budget = Duration::from_millis(
            request
                .response_deadline_ms
                .unwrap_or(self.manager.config.default_timeout_ms),
        );
        let deadline = Instant::now()
            + remote_mutation_settle_window().saturating_add(in_flight_request_budget);
        let poll = remote_mutation_settle_poll();
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err(last_error);
            }
            sleep(poll.min(deadline.saturating_duration_since(now)));
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(last_error);
            }
            let Some(remaining_ms) = duration_to_timeout_ms(remaining) else {
                return Err(last_error);
            };
            let attempt_timeout_ms = Some(
                request
                    .response_deadline_ms
                    .map(|timeout_ms| timeout_ms.min(remaining_ms))
                    .unwrap_or(remaining_ms),
            );
            match self.submit_task_land_once(request, attempt_timeout_ms) {
                Ok(result) => return Ok(result),
                Err(error) if error.is_retryable_busy() || is_remote_mutation_timeout(&error) => {
                    last_error = error
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn normalize_atomic_task_land_payload(
        &self,
        payload: Value,
        task_or_change_ref: &str,
        idempotency_key: &str,
        repo_name: Option<&str>,
        expected: Option<&crate::workflow_expected_identity::WorkflowExpectedIdentity>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let object = payload.as_object().ok_or_else(|| {
            PlanHttpClientError::Invalid("Atomic Task Land response must be an object.".to_string())
        })?;
        let expected_contract = if expected.is_some() {
            crate::workflow_expected_identity::GUARDED_TASK_LAND_CONTRACT
        } else {
            "task-land-atomic/v2"
        };
        require_atomic_task_land_text(object, "contract", "response")?
            .eq(expected_contract)
            .then_some(())
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(format!(
                    "Atomic Task Land response contract must be `{expected_contract}`."
                ))
            })?;
        let response_key = require_atomic_task_land_text(object, "idempotency_key", "response")?;
        if response_key != idempotency_key {
            return Err(PlanHttpClientError::Invalid(format!(
                "Atomic Task Land response idempotency_key `{response_key}` does not match request `{idempotency_key}`."
            )));
        }
        object
            .get("replayed")
            .and_then(Value::as_bool)
            .ok_or_else(|| {
                PlanHttpClientError::Invalid(
                    "Atomic Task Land response is missing boolean replayed.".to_string(),
                )
            })?;

        let task = require_atomic_task_land_object(object, "task")?;
        let change = object
            .get("change")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_object().ok_or_else(|| {
                    PlanHttpClientError::Invalid(
                        "Atomic Task finish Change must be an object or null".into(),
                    )
                })
            })
            .transpose()?;
        let patchset = require_atomic_task_land_object(object, "patchset")?;
        let land = require_atomic_task_land_object(object, "land")?;
        let task_id = require_atomic_task_land_text(task, "task_id", "task")?;
        let change_ref = change
            .map(|row| require_atomic_task_land_text(row, "change_ref", "change"))
            .transpose()?;
        let patchset_id = require_atomic_task_land_text(patchset, "patchset_id", "patchset")?;
        let target_line = require_atomic_task_land_text(land, "target_line", "land")?;
        let landed_snapshot_id = require_atomic_task_land_text(land, "landed_snapshot_id", "land")?;
        let stored = crate::land_json::LandJson::stateless()
            .task_successful_receipt(
                &payload["task"],
                &task_id,
                self.manager
                    .config
                    .repository_index
                    .map(|index| index.get()),
            )
            .map_err(PlanHttpClientError::Invalid)?
            .ok_or_else(|| {
                PlanHttpClientError::Invalid("Atomic Task finish has no actual Task receipt".into())
            })?;
        for field in [
            "submission_id",
            "task_id",
            "patchset_id",
            "target_line",
            "landed_snapshot_id",
            "mode",
        ] {
            if stored[field] != payload["land"][field] {
                return Err(PlanHttpClientError::Invalid(format!(
                    "Atomic Task finish differs from its stored receipt at {field}"
                )));
            }
        }
        for field in ["base_snapshot_id", "revision_snapshot_id"] {
            if stored[field] != payload["patchset"][field] {
                return Err(PlanHttpClientError::Invalid(format!(
                    "Accepted Patchset differs from the stored receipt at {field}"
                )));
            }
        }

        require_atomic_task_land_status(object, "status", &["succeeded", "landed"], "response")?;
        require_atomic_task_land_status(task, "status", &["completed"], "task")?;
        if let Some(change) = change {
            require_atomic_task_land_status(change, "status", &["landed"], "change")?;
        }
        require_atomic_task_land_status(land, "status", &["succeeded", "landed"], "land")?;

        for (field, expected) in [
            ("task_id", task_id.as_str()),
            ("patchset_id", patchset_id.as_str()),
            ("target_line", target_line.as_str()),
            ("landed_snapshot_id", landed_snapshot_id.as_str()),
        ] {
            let actual = require_atomic_task_land_text(object, field, "response")?;
            if actual != expected {
                return Err(PlanHttpClientError::Invalid(format!(
                    "Atomic Task Land response {field} `{actual}` disagrees with nested `{expected}`."
                )));
            }
        }
        let root_task_status = require_atomic_task_land_text(object, "task_status", "response")?;
        if root_task_status != "completed" {
            return Err(PlanHttpClientError::Invalid(format!(
                "Atomic Task Land response task_status must be `completed`, got `{root_task_status}`."
            )));
        }
        let optional_text = |map: &crate::json_support::JsonMap<String, Value>,
                             field: &str|
         -> TaskWorkflowHttpClientResult<Option<String>> {
            match map.get(field) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(text)) if !text.is_empty() && text.trim() == text => {
                    Ok(Some(text.clone()))
                }
                _ => Err(PlanHttpClientError::Invalid(format!(
                    "Atomic Task finish {field} must be exact text or null"
                ))),
            }
        };
        if optional_text(object, "change_ref")? != change_ref
            || optional_text(object, "change_status")?.as_deref() != change.map(|_| "landed")
        {
            return Err(PlanHttpClientError::Invalid(
                "Atomic Task finish optional Change differs between receipt and response".into(),
            ));
        }
        crate::workflow_expected_identity::validate_patchset_reference_owner(
            &patchset_id,
            &task_id,
            change_ref.as_deref(),
        )
        .map_err(PlanHttpClientError::Invalid)?;
        for projection in [patchset, land] {
            if require_atomic_task_land_text(projection, "task_id", "accepted evidence")? != task_id
                || optional_text(projection, "change_ref")? != change_ref
            {
                return Err(PlanHttpClientError::Invalid(
                    "Atomic Task finish accepted evidence has a foreign Task or Change".into(),
                ));
            }
        }
        if require_atomic_task_land_text(land, "patchset_id", "receipt")? != patchset_id {
            return Err(PlanHttpClientError::Invalid(
                "Atomic Task receipt accepted a different Patchset".into(),
            ));
        }
        if task_or_change_ref.contains("/C-") {
            if change_ref.as_deref() != Some(task_or_change_ref) {
                return Err(PlanHttpClientError::Invalid(format!(
                    "Atomic Task Land returned Change `{change_ref:?}`, not requested `{task_or_change_ref}`."
                )));
            }
        } else if task_id != task_or_change_ref {
            return Err(PlanHttpClientError::Invalid(format!(
                "Atomic Task Land returned Task `{task_id}`, not requested `{task_or_change_ref}`."
            )));
        }
        if let Some(expected_repo_name) = repo_name {
            let response_repo_name =
                require_atomic_task_land_text(object, "repo_name", "response")?;
            if response_repo_name != expected_repo_name {
                return Err(PlanHttpClientError::Invalid(format!(
                    "Atomic Task Land response repository `{response_repo_name}` does not match `{expected_repo_name}`."
                )));
            }
        }
        if let Some(expected) = expected {
            if object.get("repository_index").and_then(Value::as_u64)
                != Some(u64::from(expected.repository_index))
            {
                return Err(PlanHttpClientError::Invalid(
                    "Guarded atomic response Repository identity differs.".to_string(),
                ));
            }
            expected
                .validate_patchset(expected.repository_index, &payload["patchset"])
                .map_err(PlanHttpClientError::Invalid)?;
            let echoed = crate::workflow_expected_identity::WorkflowExpectedIdentity::from_value(
                &payload["expected_identity"],
            )
            .map_err(PlanHttpClientError::Invalid)?;
            if echoed != *expected {
                return Err(PlanHttpClientError::Invalid(
                    "Guarded atomic response did not commit the expected identity.".to_string(),
                ));
            }
        }
        Ok(payload)
    }

    fn close_task_once(
        &mut self,
        task_id: &str,
        status: &str,
        timeout_ms: Option<u64>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.with_temporary_default_timeout(timeout_ms, |remote| {
            remote.manager.close_task(task_id, status)
        })
    }

    fn landed_change_for_task(
        &mut self,
        task_id: &str,
        repo_name: &str,
        accept_landing_evidence: bool,
    ) -> TaskWorkflowHttpClientResult<Option<String>> {
        let rows = self.manager.list_changes(repo_name)?;
        Ok(rows.into_iter().find_map(|row| {
            let row = self.normalize_change(&row, Some(task_id)).ok()?;
            let row_task_id = normalize_optional_text(row.get("task_id"))?;
            if row_task_id != task_id {
                return None;
            }
            let change_id = normalize_optional_text(row.get("change_id"))?;
            if change_has_landed_status(&row) {
                return Some(change_id);
            }
            if accept_landing_evidence && change_has_landing_evidence(&row) {
                return Some(change_id);
            }
            let wire_change_id = self.wire_change_id(&change_id).ok()?;
            match self
                .manager
                .get_change_detail(&wire_change_id, Some(repo_name))
            {
                Ok(detail) => {
                    let detail = self.normalize_change_detail(&detail, Some(task_id)).ok()?;
                    if change_has_landed_status(&detail)
                        || (accept_landing_evidence && change_has_landing_evidence(&detail))
                    {
                        Some(change_id)
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        }))
    }

    fn wait_for_landed_change_status(
        &mut self,
        task_id: &str,
        repo_name: &str,
    ) -> TaskWorkflowHttpClientResult<Option<String>> {
        let deadline = Instant::now() + remote_mutation_settle_window();
        let poll = remote_mutation_settle_poll();
        loop {
            if let Some(change_id) = self.landed_change_for_task(task_id, repo_name, false)? {
                return Ok(Some(change_id));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            sleep(poll);
        }
    }

    fn recover_remote_land_submission(
        &mut self,
        change_id: &str,
        repo_name: &str,
    ) -> TaskWorkflowHttpClientResult<Option<Value>> {
        let deadline = Instant::now() + remote_mutation_settle_window();
        let poll = remote_mutation_settle_poll();
        loop {
            let change = self.resolve_change_row(change_id, Some(repo_name))?;
            if let Some(recovered) = recover_land_submission_from_change_state(&change, change_id) {
                return Ok(Some(recovered));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            sleep(poll);
        }
    }

    fn recover_remote_closed_task(
        &mut self,
        task_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Option<Value>> {
        let deadline = Instant::now() + remote_mutation_settle_window();
        let poll = remote_mutation_settle_poll();
        loop {
            let task = self.manager.get_task(task_id, repo_name)?;
            if let Some(recovered) = recover_closed_task_from_state(&task, task_id) {
                return Ok(Some(recovered));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            sleep(poll);
        }
    }
}

impl TaskWorkflowAtomicTaskLandSubmitter for HttpWorkflowCloseoutRemote {
    fn submit_task_land(
        &mut self,
        task_or_change_ref: &str,
        target_line: Option<&str>,
        mode: &str,
        idempotency_key: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.submit_task_land_guarded(
            task_or_change_ref,
            target_line,
            mode,
            idempotency_key,
            repo_name,
            None,
        )
    }
}

impl TaskWorkflowLandSubmitter for HttpWorkflowCloseoutRemote {
    fn submit_land(
        &mut self,
        change_id: &str,
        patchset_id: Option<&str>,
        target_line: &str,
        mode: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let response_deadline_ms = remote_mutation_response_deadline_timeout_ms();
        match self.submit_land_once(
            change_id,
            patchset_id,
            target_line,
            mode,
            repo_name,
            response_deadline_ms,
        ) {
            Ok(result) => Ok(result),
            Err(err) if is_remote_mutation_timeout(&err) => {
                if let Some(repo_name) = repo_name {
                    if let Some(recovered) =
                        self.recover_remote_land_submission(change_id, repo_name)?
                    {
                        return Ok(recovered);
                    }
                }
                let retry = self.submit_land_once(
                    change_id,
                    patchset_id,
                    target_line,
                    mode,
                    repo_name,
                    response_deadline_ms,
                );
                match retry {
                    Ok(result) => Ok(result),
                    Err(retry_err) if is_remote_mutation_timeout(&retry_err) => {
                        if let Some(repo_name) = repo_name {
                            if let Some(recovered) =
                                self.recover_remote_land_submission(change_id, repo_name)?
                            {
                                return Ok(recovered);
                            }
                        }
                        Err(retry_err)
                    }
                    Err(retry_err) => Err(retry_err),
                }
            }
            Err(err) => Err(err),
        }
    }
}

fn require_atomic_task_land_object<'a>(
    object: &'a crate::json_support::JsonMap<String, Value>,
    field: &str,
) -> TaskWorkflowHttpClientResult<&'a crate::json_support::JsonMap<String, Value>> {
    object.get(field).and_then(Value::as_object).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!(
            "Atomic Task Land response is missing object {field}."
        ))
    })
}

fn require_atomic_task_land_text(
    object: &crate::json_support::JsonMap<String, Value>,
    field: &str,
    context: &str,
) -> TaskWorkflowHttpClientResult<String> {
    normalize_optional_text(object.get(field)).ok_or_else(|| {
        PlanHttpClientError::Invalid(format!(
            "Atomic Task Land {context} is missing non-empty {field}."
        ))
    })
}

fn require_atomic_task_land_status(
    object: &crate::json_support::JsonMap<String, Value>,
    field: &str,
    accepted: &[&str],
    context: &str,
) -> TaskWorkflowHttpClientResult<()> {
    let status = require_atomic_task_land_text(object, field, context)?;
    if accepted.contains(&status.as_str()) {
        Ok(())
    } else {
        Err(PlanHttpClientError::Invalid(format!(
            "Atomic Task Land {context} {field} `{status}` is not successful."
        )))
    }
}

impl TaskWorkflowLandReader for HttpWorkflowCloseoutRemote {
    fn get_land(
        &mut self,
        submission_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.get_land(submission_id, repo_name)
    }
}

impl TaskWorkflowLandRetryer for HttpWorkflowCloseoutRemote {
    fn retry_land(
        &mut self,
        submission_id: &str,
        reason: Option<&str>,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager.retry_land(submission_id, reason, repo_name)
    }
}

impl TaskWorkflowRemoteTaskCloser for HttpWorkflowCloseoutRemote {
    fn close_task(
        &mut self,
        task_id: &str,
        status: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_task_id = if repo_name.is_some() {
            let task = self.manager.get_task(task_id, repo_name)?;
            TaskJson::stateless().resolved_task_id_from_task_payload(&task, task_id)
        } else {
            task_id.to_string()
        };
        let normalized_status = status.trim().to_ascii_lowercase();
        if repo_name.is_some() && normalized_status != "completed" {
            if let Some(repo_name) = repo_name {
                if let Some(change_id) =
                    self.landed_change_for_task(&resolved_task_id, repo_name, true)?
                {
                    return Err(TaskWorkflowHttpClientError::Invalid(format!(
                        "Remote task {resolved_task_id} cannot be closed as `{normalized_status}` after linked change {change_id} landed; complete the task instead."
                    )));
                }
            }
        }
        let response_deadline_ms = remote_mutation_response_deadline_timeout_ms();
        match self.close_task_once(&resolved_task_id, status, response_deadline_ms) {
            Ok(result) => Ok(result),
            Err(err)
                if normalized_status == "completed"
                    && repo_name.is_some()
                    && task_close_error_needs_landed_settle(&err) =>
            {
                if let Some(repo_name) = repo_name {
                    if self
                        .wait_for_landed_change_status(&resolved_task_id, repo_name)?
                        .is_some()
                    {
                        return self.close_task_once(
                            &resolved_task_id,
                            status,
                            response_deadline_ms,
                        );
                    }
                }
                Err(err)
            }
            Err(err) if is_remote_mutation_timeout(&err) => {
                if let Some(recovered) =
                    self.recover_remote_closed_task(&resolved_task_id, repo_name)?
                {
                    return Ok(recovered);
                }
                let retry = self.close_task_once(&resolved_task_id, status, response_deadline_ms);
                match retry {
                    Ok(result) => Ok(result),
                    Err(retry_err) if is_remote_mutation_timeout(&retry_err) => {
                        if let Some(recovered) =
                            self.recover_remote_closed_task(&resolved_task_id, repo_name)?
                        {
                            return Ok(recovered);
                        }
                        Err(retry_err)
                    }
                    Err(retry_err) => Err(retry_err),
                }
            }
            Err(err) => Err(err),
        }
    }
}
