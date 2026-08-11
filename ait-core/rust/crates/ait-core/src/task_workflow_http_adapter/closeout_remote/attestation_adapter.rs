use super::*;

impl HttpWorkflowCloseoutRemote {
    #[expect(
        clippy::too_many_arguments,
        reason = "arguments mirror the remote attestation HTTP contract"
    )]
    pub fn put_attestation(
        &mut self,
        patchset_id: &str,
        author_mode: &str,
        evaluation_summary: &Value,
        provenance_summary: &Value,
        detail: &Value,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        put_attestation_with_task_workflow_closeout_remote(
            self,
            patchset_id,
            author_mode,
            evaluation_summary,
            provenance_summary,
            detail,
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
        evaluation_summary: &Value,
        provenance_summary: &Value,
        detail: &Value,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_patchset_id = if repo_name.is_some() && !exact_id {
            let patchset = self.manager.get_patchset(patchset_id, repo_name, None)?;
            PatchsetJson::stateless().resolved_patchset_id_from_payload(&patchset, patchset_id)
        } else {
            patchset_id.to_string()
        };
        self.manager.put_attestation(
            &resolved_patchset_id,
            author_mode,
            evaluation_summary,
            provenance_summary,
            detail,
        )
    }
}

impl TaskWorkflowAttestationReader for HttpWorkflowCloseoutRemote {
    fn get_attestation(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_patchset_id = if repo_name.is_some() && !exact_id {
            let patchset = self.manager.get_patchset(patchset_id, repo_name, None)?;
            PatchsetJson::stateless().resolved_patchset_id_from_payload(&patchset, patchset_id)
        } else {
            patchset_id.to_string()
        };
        self.manager.get_attestation(&resolved_patchset_id)
    }
}
