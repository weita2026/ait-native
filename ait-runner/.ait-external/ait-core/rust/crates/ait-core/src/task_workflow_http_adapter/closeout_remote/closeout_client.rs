use super::*;

impl HttpWorkflowCloseoutRemote {
    #[cfg(test)]
    pub(crate) fn with_resolved_test_actor(mut self, actor: u32) -> Self {
        self.manager.actor_context = self
            .manager
            .config
            .repository_index
            .map(|index| (index, actor));
        self
    }
    pub fn set_actor_registration(
        &mut self,
        repo_name: &str,
        identity: Result<crate::plan_http_client::RemoteActorIdentity, String>,
        namespace: Option<&str>,
    ) {
        self.manager
            .set_actor_registration(repo_name, identity, namespace);
    }
    pub(super) fn current_task_candidate(
        &mut self,
        owner: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<(String, Value, Option<Value>)> {
        let task_id = self.expected_task_id_for_change(owner).ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "Candidate operation requires an exact Task or Change reference".into(),
            )
        })?;
        let task = self.manager.get_task(&task_id, repo_name)?;
        crate::land_json::LandJson::stateless()
            .task_successful_receipt(
                &task,
                &task_id,
                self.manager
                    .config
                    .repository_index
                    .map(|index| index.get()),
            )
            .map_err(PlanHttpClientError::Invalid)?;
        let current = match task.get("current_patchset_id") {
            Some(Value::Null) => None,
            Some(Value::String(id)) if !id.is_empty() && id.trim() == id => Some(id.clone()),
            _ => {
                return Err(PlanHttpClientError::Invalid(
                    "Task requires nullable current_patchset_id".into(),
                ))
            }
        };
        let patchset = current
            .as_deref()
            .map(|id| self.manager.get_patchset(id, repo_name, None))
            .transpose()?;
        if let Some(row) = patchset.as_ref() {
            crate::workflow_expected_identity::validate_patchset_owner_fields(row, &task_id)
                .map_err(PlanHttpClientError::Invalid)?;
            let identity = crate::workflow_expected_identity::WorkflowExpectedIdentity {
                repository_index: self
                    .manager
                    .config
                    .repository_index
                    .map(|index| index.get())
                    .unwrap_or_default(),
                patchset_id: current.clone().expect("candidate has current reference"),
                base_snapshot_id: row["base_snapshot_id"].as_str().unwrap_or_default().into(),
                revision_snapshot_id: row["revision_snapshot_id"]
                    .as_str()
                    .unwrap_or_default()
                    .into(),
            };
            identity
                .validate()
                .and_then(|()| identity.validate_patchset_reference(row))
                .map_err(PlanHttpClientError::Invalid)?;
        }
        Ok((task_id, task, patchset))
    }

    pub(super) fn matching_current_candidate(
        &mut self,
        owner: &str,
        repo_name: Option<&str>,
        base: &str,
        revision: &str,
    ) -> TaskWorkflowHttpClientResult<Option<Value>> {
        let (_, task, current) = self.current_task_candidate(owner, repo_name)?;
        if task["status"].as_str() != Some("active") || !task["successful_land"].is_null() {
            return Err(PlanHttpClientError::Invalid(
                "Publication requires an active Task without a successful receipt".into(),
            ));
        }
        let Some(row) = current else {
            return Ok(None);
        };
        let expected_change = (!crate::public_references::is_task_id(owner)).then_some(owner);
        if row["change_ref"].as_str() != expected_change
            || row["base_snapshot_id"].as_str() != Some(base)
            || row["revision_snapshot_id"].as_str() != Some(revision)
        {
            return Ok(None);
        }
        let index = self
            .manager
            .config
            .repository_index
            .map(|index| index.get())
            .unwrap_or_default();
        crate::workflow_expected_identity::WorkflowExpectedIdentity {
            repository_index: index,
            patchset_id: task["current_patchset_id"].as_str().unwrap().into(),
            base_snapshot_id: base.into(),
            revision_snapshot_id: revision.into(),
        }
        .validate_patchset(index, &row)
        .map_err(PlanHttpClientError::Invalid)?;
        Ok(Some(row))
    }
    pub fn new(config: TaskWorkflowHttpClientConfig) -> TaskWorkflowHttpClientResult<Self> {
        Ok(Self {
            manager: TaskWorkflowHttpClientManager::new(config)?,
            bound_task_id: None,
            bound_change_id: None,
            bound_change_ref: None,
        })
    }

    pub fn set_bound_change_context(
        &mut self,
        task_id: Option<&str>,
        change_id: Option<&str>,
    ) -> Result<(), String> {
        self.set_bound_change_identity_context(task_id, change_id, None)
    }

    pub fn set_bound_change_identity_context(
        &mut self,
        task_id: Option<&str>,
        change_id: Option<&str>,
        change_ref: Option<&str>,
    ) -> Result<(), String> {
        let bound_task_id = task_id
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let bound_change_id = change_id
            .map(|value| ChangeJson::stateless().canonical_change_id(value))
            .transpose()?;
        let derived_change_ref = bound_change_id
            .as_deref()
            .map(|value| {
                ChangeJson::stateless().rolling_server_change_id(bound_task_id.as_deref(), value)
            })
            .transpose()?;
        let provided_change_ref = change_ref
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        if let (Some(provided), Some(derived)) = (
            provided_change_ref.as_deref(),
            derived_change_ref.as_deref(),
        ) {
            if provided != derived {
                return Err(format!(
                    "Bound change_ref `{provided}` does not match derived `{derived}`."
                ));
            }
        }
        self.bound_task_id = bound_task_id;
        self.bound_change_id = bound_change_id;
        self.bound_change_ref = provided_change_ref.or(derived_change_ref);
        Ok(())
    }

    pub(super) fn wire_change_id(&self, change_id: &str) -> TaskWorkflowHttpClientResult<String> {
        let requested = change_id.trim();
        if crate::public_references::is_task_id(requested) {
            return Ok(requested.to_string());
        }
        let canonical = ChangeJson::stateless()
            .canonical_change_id(requested)
            .map_err(PlanHttpClientError::Invalid)?;
        if requested
            .rsplit_once('/')
            .is_some_and(|(_, child)| child == canonical)
        {
            return Ok(requested.to_string());
        }
        let task_id = if self.bound_change_id.as_deref() == Some(canonical.as_str()) {
            if let Some(change_ref) = self.bound_change_ref.as_deref() {
                return Ok(change_ref.to_string());
            }
            self.bound_task_id.as_deref()
        } else {
            None
        };
        ChangeJson::stateless()
            .rolling_server_change_id(task_id, &canonical)
            .map_err(PlanHttpClientError::Invalid)
    }

    pub(super) fn expected_task_id_for_change(&self, change_id: &str) -> Option<String> {
        let requested = change_id.trim();
        if crate::public_references::is_task_id(requested) {
            return Some(requested.to_string());
        }
        if let Some((task_id, child)) = requested.rsplit_once('/') {
            if ChangeJson::stateless()
                .canonical_change_id(requested)
                .ok()
                .as_deref()
                == Some(child)
            {
                return Some(task_id.to_string());
            }
        }
        let canonical = ChangeJson::stateless()
            .canonical_change_id(requested)
            .ok()?;
        (self.bound_change_id.as_deref() == Some(canonical.as_str()))
            .then(|| self.bound_task_id.clone())
            .flatten()
    }

    pub(super) fn resolved_change_ref(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<String> {
        if crate::public_references::is_task_id(change_id) {
            return self.wire_change_id(change_id);
        }
        if repo_name.is_some() && !exact_id {
            let change = self.resolve_change_row(change_id, repo_name)?;
            return change
                .get("change_ref")
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| {
                    PlanHttpClientError::Invalid(
                        "Normalized remote Change is missing change_ref.".to_string(),
                    )
                });
        }
        self.wire_change_id(change_id)
    }

    pub(super) fn normalize_change(
        &self,
        change: &Value,
        expected_task_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        ChangeJson::stateless()
            .normalize_remote_change_payload(change, expected_task_id)
            .map_err(PlanHttpClientError::Invalid)
    }

    pub(super) fn normalize_change_detail(
        &self,
        detail: &Value,
        expected_task_id: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        ChangeJson::stateless()
            .normalize_remote_change_detail_payload(detail, expected_task_id)
            .map_err(PlanHttpClientError::Invalid)
    }

    pub(super) fn normalize_change_identity_payload(
        &self,
        payload: Value,
        change_id: &str,
    ) -> TaskWorkflowHttpClientResult<Value> {
        if crate::public_references::is_task_id(change_id) {
            if payload["task_id"].as_str() != Some(change_id) {
                return Err(PlanHttpClientError::Invalid(
                    "Task operation returned another owner".into(),
                ));
            }
            if let Some(id) = payload["patchset_id"].as_str() {
                crate::workflow_expected_identity::validate_patchset_reference_owner(
                    id,
                    change_id,
                    payload["change_ref"].as_str(),
                )
                .map_err(PlanHttpClientError::Invalid)?;
            }
            return Ok(payload);
        }
        if payload.get("change_id").is_none() {
            return Ok(payload);
        }
        let expected_task_id = self.expected_task_id_for_change(change_id);
        self.normalize_change(&payload, expected_task_id.as_deref())
    }

    pub fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        inspect_client_with_task_workflow_closeout_remote(self)
    }

    pub fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        close_client_with_task_workflow_closeout_remote(self)
    }

    pub fn mutation_receipt(
        &self,
        action: &str,
        source_action: &str,
        delivery: &str,
        response_recovery: Option<&Value>,
        result: Option<&Value>,
    ) -> Result<Value, String> {
        mutation_receipt_with_task_workflow_closeout_remote(
            self,
            action,
            source_action,
            delivery,
            response_recovery,
            result,
        )
    }

    pub fn action_mutation_receipts(&self, code: &str, result: &Value) -> Result<Value, String> {
        action_mutation_receipts_with_task_workflow_closeout_remote(self, code, result)
    }
}

impl TaskWorkflowHttpClientInspector for HttpWorkflowCloseoutRemote {
    fn inspect_client(&self) -> TaskWorkflowHttpClientStats {
        self.manager.inspect()
    }
}

impl TaskWorkflowHttpClientCloser for HttpWorkflowCloseoutRemote {
    fn close_client(&mut self) -> TaskWorkflowHttpClientStats {
        self.manager.close()
    }
}

impl TaskWorkflowMutationReceiptBuilder for HttpWorkflowCloseoutRemote {
    fn mutation_receipt(
        &self,
        action: &str,
        source_action: &str,
        delivery: &str,
        response_recovery: Option<&Value>,
        result: Option<&Value>,
    ) -> Result<Value, String> {
        workflow_closeout_remote::workflow_remote_mutation_receipt(
            action,
            source_action,
            delivery,
            response_recovery,
            result,
        )
    }
}

impl TaskWorkflowActionMutationReceiptsBuilder for HttpWorkflowCloseoutRemote {
    fn action_mutation_receipts(&self, code: &str, result: &Value) -> Result<Value, String> {
        workflow_closeout_remote::workflow_remote_action_mutation_receipts(code, result)
    }
}
