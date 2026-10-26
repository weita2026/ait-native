use super::*;

impl HttpWorkflowCloseoutRemote {
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
