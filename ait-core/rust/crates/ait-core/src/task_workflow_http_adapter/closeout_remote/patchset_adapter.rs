use super::*;

impl HttpWorkflowCloseoutRemote {
    pub fn list_patchsets(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<Value>> {
        list_patchsets_with_task_workflow_closeout_remote(self, change_id, repo_name)
    }

    pub fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_patchset_with_task_workflow_closeout_remote(self, patchset_id, repo_name, change_ref)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "arguments mirror the remote patchset publication contract"
    )]
    pub fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        publish_patchset_with_task_workflow_closeout_remote(
            self,
            change_id,
            base_snapshot_id,
            revision_snapshot_id,
            summary,
            author_mode,
            repo_name,
            exact_id,
        )
    }

    pub fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        select_patchset_with_task_workflow_closeout_remote(
            self,
            change_id,
            patchset_id,
            repo_name,
            exact_id,
        )
    }
}

impl TaskWorkflowPatchsetLister for HttpWorkflowCloseoutRemote {
    fn list_patchsets(
        &mut self,
        change_id: &str,
        repo_name: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Vec<Value>> {
        let change_ref = self.wire_change_id(change_id)?;
        self.manager
            .list_patchsets(&change_ref, repo_name)?
            .into_iter()
            .map(|row| {
                let task = self
                    .expected_task_id_for_change(&change_ref)
                    .ok_or_else(|| {
                        PlanHttpClientError::Invalid(
                            "Patchset inventory requires exact Task ownership".into(),
                        )
                    })?;
                let association =
                    crate::workflow_expected_identity::validate_patchset_owner_fields(&row, &task)
                        .map_err(PlanHttpClientError::Invalid)?;
                if !crate::public_references::is_task_id(&change_ref)
                    && association.as_deref() != Some(change_ref.as_str())
                {
                    return Err(PlanHttpClientError::Invalid(
                        "Change inventory returned another Patchset association".into(),
                    ));
                }
                self.normalize_change_identity_payload(row, change_id)
            })
            .collect()
    }
}

impl TaskWorkflowPatchsetReader for HttpWorkflowCloseoutRemote {
    fn get_patchset(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        change_ref: Option<&str>,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let requested_change_ref = change_ref.map(str::to_string);
        let change_ref = change_ref
            .map(|value| self.wire_change_id(value))
            .transpose()?;
        let patchset = self
            .manager
            .get_patchset(patchset_id, repo_name, change_ref.as_deref())?;
        let task = patchset["task_id"].as_str().ok_or_else(|| {
            PlanHttpClientError::Invalid("Patchset requires Task ownership".into())
        })?;
        crate::workflow_expected_identity::validate_patchset_owner_fields(&patchset, task)
            .map_err(PlanHttpClientError::Invalid)?;
        crate::workflow_expected_identity::WorkflowExpectedIdentity {
            repository_index: 0,
            patchset_id: patchset_id.into(),
            base_snapshot_id: patchset["base_snapshot_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
            revision_snapshot_id: patchset["revision_snapshot_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
        }
        .validate_patchset_reference(&patchset)
        .map_err(PlanHttpClientError::Invalid)?;
        match requested_change_ref {
            Some(requested) => self.normalize_change_identity_payload(patchset, &requested),
            None => Ok(patchset),
        }
    }
}

impl TaskWorkflowPatchsetPublisher for HttpWorkflowCloseoutRemote {
    fn publish_patchset(
        &mut self,
        change_id: &str,
        base_snapshot_id: &str,
        revision_snapshot_id: &str,
        summary: &str,
        author_mode: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_change_ref = self.resolved_change_ref(change_id, repo_name, exact_id)?;
        if let Some(current) = self.matching_current_candidate(
            &resolved_change_ref,
            repo_name,
            base_snapshot_id,
            revision_snapshot_id,
        )? {
            return self.normalize_change_identity_payload(current, change_id);
        }
        match self.manager.publish_patchset(
            &resolved_change_ref,
            base_snapshot_id,
            revision_snapshot_id,
            summary,
            author_mode,
        ) {
            Ok(patchset) => {
                let task = self
                    .expected_task_id_for_change(&resolved_change_ref)
                    .expect("preflight checked Task");
                let association =
                    crate::workflow_expected_identity::validate_patchset_owner_fields(
                        &patchset, &task,
                    )
                    .map_err(PlanHttpClientError::Invalid)?;
                if association.as_deref()
                    != (!crate::public_references::is_task_id(&resolved_change_ref))
                        .then_some(resolved_change_ref.as_str())
                    || patchset["base_snapshot_id"].as_str() != Some(base_snapshot_id)
                    || patchset["revision_snapshot_id"].as_str() != Some(revision_snapshot_id)
                {
                    return Err(PlanHttpClientError::Invalid(
                        "Published candidate differs from captured owner/base/revision".into(),
                    ));
                }
                self.normalize_change_identity_payload(patchset, change_id)
            }
            Err(err) if err.is_retryable_busy() || is_remote_mutation_timeout(&err) => {
                let Some(repo_name) = repo_name else {
                    return Err(err);
                };
                if let Some(recovered) = self.recover_remote_published_patchset(
                    &resolved_change_ref,
                    repo_name,
                    base_snapshot_id,
                    revision_snapshot_id,
                )? {
                    return self.normalize_change_identity_payload(recovered, change_id);
                }
                Err(err)
            }
            Err(err) => Err(err),
        }
    }
}

impl TaskWorkflowPatchsetSelector for HttpWorkflowCloseoutRemote {
    fn select_patchset(
        &mut self,
        change_id: &str,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_change_ref = self.resolved_change_ref(change_id, repo_name, exact_id)?;
        let (task_id, _task, current) =
            self.current_task_candidate(&resolved_change_ref, repo_name)?;
        let current = current
            .ok_or_else(|| PlanHttpClientError::Invalid("Task has no current Patchset".into()))?;
        let change =
            crate::workflow_expected_identity::validate_patchset_owner_fields(&current, &task_id)
                .map_err(PlanHttpClientError::Invalid)?;
        if !crate::public_references::is_task_id(&resolved_change_ref)
            && change.as_deref() != Some(resolved_change_ref.as_str())
        {
            return Err(PlanHttpClientError::Invalid(
                "Current Patchset belongs to a different Change".into(),
            ));
        }
        let index = self
            .manager
            .config
            .repository_index
            .map(|index| index.get())
            .unwrap_or_default();
        let expected = crate::workflow_expected_identity::WorkflowExpectedIdentity {
            repository_index: index,
            patchset_id: patchset_id.into(),
            base_snapshot_id: current["base_snapshot_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
            revision_snapshot_id: current["revision_snapshot_id"]
                .as_str()
                .unwrap_or_default()
                .into(),
        };
        expected
            .validate_patchset(index, &current)
            .map_err(PlanHttpClientError::Invalid)?;
        let patchset = self
            .manager
            .select_patchset(&resolved_change_ref, patchset_id)?;
        if patchset["selection_changed"].as_bool() != Some(false) {
            return Err(PlanHttpClientError::Invalid(
                "Select must return a read-only current-candidate check".into(),
            ));
        }
        crate::workflow_expected_identity::validate_patchset_owner_fields(&patchset, &task_id)
            .map_err(PlanHttpClientError::Invalid)?;
        expected
            .validate_patchset_reference(&patchset)
            .map_err(PlanHttpClientError::Invalid)?;
        let projection = &patchset["patchset"];
        let returned_change =
            crate::workflow_expected_identity::validate_patchset_owner_fields(projection, &task_id)
                .map_err(PlanHttpClientError::Invalid)?;
        expected
            .validate_patchset(index, projection)
            .map_err(PlanHttpClientError::Invalid)?;
        let mut returned_current = patchset.clone();
        returned_current["patchset_id"] = patchset["current_patchset_id"].clone();
        expected
            .validate_patchset_reference(&returned_current)
            .map_err(PlanHttpClientError::Invalid)?;
        if returned_change != change || patchset["change_ref"] != projection["change_ref"] {
            return Err(PlanHttpClientError::Invalid(
                "Select current candidate drifted from preflight".into(),
            ));
        }
        self.normalize_change_identity_payload(patchset, change_id)
    }
}
