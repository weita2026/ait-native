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
            .map(|row| self.normalize_change_identity_payload(row, change_id))
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
        let (resolved_change_ref, prior_patchset_number) = if repo_name.is_some() && !exact_id {
            let change = self.resolve_change_row(change_id, repo_name)?;
            (
                change
                    .get("change_ref")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        PlanHttpClientError::Invalid(
                            "Normalized remote Change is missing change_ref.".to_string(),
                        )
                    })?
                    .to_string(),
                change
                    .get("current_patchset_number")
                    .and_then(Value::as_i64)
                    .unwrap_or_default(),
            )
        } else {
            (self.wire_change_id(change_id)?, 0)
        };
        match self.manager.publish_patchset(
            &resolved_change_ref,
            base_snapshot_id,
            revision_snapshot_id,
            summary,
            author_mode,
        ) {
            Ok(patchset) => self.normalize_change_identity_payload(patchset, change_id),
            Err(err) => {
                let Some(repo_name) = repo_name else {
                    return Err(err);
                };
                if let Some(recovered) = self.recover_remote_published_patchset(
                    &resolved_change_ref,
                    repo_name,
                    base_snapshot_id,
                    revision_snapshot_id,
                    prior_patchset_number,
                )? {
                    return self.normalize_change_identity_payload(recovered, change_id);
                }
                Err(err)
            }
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
        let patchset = self
            .manager
            .select_patchset(&resolved_change_ref, patchset_id)?;
        self.normalize_change_identity_payload(patchset, change_id)
    }
}
