use super::*;

impl HttpWorkflowCloseoutRemote {
    pub fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        statement: &crate::attest_json::RemoteAttestationStatement,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        put_attestation_with_task_workflow_closeout_remote(
            self,
            patchset_id,
            author_mode,
            statement,
            repo_name,
            exact_id,
        )
    }

    pub fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_attestation_with_task_workflow_closeout_remote(self, patchset_id, repo_name, exact_id)
    }
}

impl TaskWorkflowAttestationWriter for HttpWorkflowCloseoutRemote {
    fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        statement: &crate::attest_json::RemoteAttestationStatement,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        use crate::attest_json::RemoteAttestationStatement;
        let _ = exact_id;
        let patchset = self.manager.get_patchset(patchset_id, repo_name, None)?;
        let task_id = patchset["task_id"].as_str().ok_or_else(|| {
            PlanHttpClientError::Invalid("Attestation requires exact Task ownership".into())
        })?;
        let (_, task, current) = self.current_task_candidate(task_id, repo_name)?;
        let current = current
            .ok_or_else(|| PlanHttpClientError::Invalid("Task has no current Patchset".into()))?;
        let identity = crate::workflow_expected_identity::WorkflowExpectedIdentity {
            repository_index: self
                .manager
                .config
                .repository_index
                .map(|index| index.get())
                .unwrap_or_default(),
            patchset_id: patchset_id.into(),
            base_snapshot_id: patchset["base_snapshot_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
            revision_snapshot_id: patchset["revision_snapshot_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
        };
        identity
            .validate_patchset(identity.repository_index, &current)
            .map_err(PlanHttpClientError::Invalid)?;
        if task["status"] != "active"
            || !task["successful_land"].is_null()
            || current["invalidated"] == true
            || current["author_mode"].as_str() != Some(author_mode)
        {
            return Err(PlanHttpClientError::Invalid(
                "Attestation requires the exact live current Patchset and immutable author_mode"
                    .into(),
            ));
        }
        let resolved_patchset_id = current["patchset_id"].as_str().unwrap();
        let mut wanted = statement.clone();
        match self.manager.get_attestation(resolved_patchset_id) {
            Ok(value) => {
                let previous = RemoteAttestationStatement::validate_for_patchset(&value, &current)
                    .map_err(PlanHttpClientError::Invalid)?;
                wanted.retain_requirements_from(&previous);
                if wanted == previous {
                    return Ok(value);
                }
            }
            Err(error) if error.remote_status() == Some(404) => {}
            Err(error) => return Err(error),
        }
        let value = match self
            .manager
            .put_attestation(resolved_patchset_id, author_mode, &wanted)
        {
            Ok(value) => value,
            Err(error) if error.is_transport_timeout() => {
                // A timed-out append may have committed. Recover by exact readback, never repeat PUT.
                self.manager
                    .get_attestation(resolved_patchset_id)
                    .map_err(|_| error)?
            }
            Err(error) => return Err(error),
        };
        let actual = RemoteAttestationStatement::validate_for_patchset(&value, &current)
            .map_err(PlanHttpClientError::Invalid)?;
        if actual != wanted {
            return Err(PlanHttpClientError::Invalid("Remote Attestation readback differs from the requested statement; stopped without another PUT".into()));
        }
        Ok(value)
    }
}

impl TaskWorkflowAttestationReader for HttpWorkflowCloseoutRemote {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let _ = exact_id;
        let patchset = self.manager.get_patchset(patchset_id, repo_name, None)?;
        let resolved_patchset_id =
            PatchsetJson::stateless().resolved_patchset_id_from_payload(&patchset, patchset_id);
        let value = self.manager.get_attestation(&resolved_patchset_id)?;
        crate::attest_json::RemoteAttestationStatement::validate_for_patchset(&value, &patchset)
            .map_err(PlanHttpClientError::Invalid)?;
        Ok(value)
    }
}
