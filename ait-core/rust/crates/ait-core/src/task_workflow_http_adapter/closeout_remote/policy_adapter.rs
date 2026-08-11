use super::*;

impl HttpWorkflowCloseoutRemote {
    pub fn evaluate_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        evaluate_policy_with_task_workflow_closeout_remote(self, patchset_id, repo_name, exact_id)
    }

    pub fn get_policy(
        &mut self,
        patchset_id: &str,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        get_policy_with_task_workflow_closeout_remote(self, patchset_id, repo_name, exact_id)
    }

    pub fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        create_waiver_with_task_workflow_closeout_remote(
            self,
            patchset_id,
            rule_name,
            reason,
            expires_at,
            repo_name,
            exact_id,
        )
    }
}

impl TaskWorkflowPolicyEvaluator for HttpWorkflowCloseoutRemote {
    fn evaluate_policy(
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
        self.manager.evaluate_policy(&resolved_patchset_id)
    }
}

impl TaskWorkflowPolicyReader for HttpWorkflowCloseoutRemote {
    fn get_policy(
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
        self.manager.get_policy(&resolved_patchset_id)
    }
}

impl TaskWorkflowPolicyWaiverCreator for HttpWorkflowCloseoutRemote {
    fn create_waiver(
        &mut self,
        patchset_id: &str,
        rule_name: &str,
        reason: &str,
        expires_at: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        let resolved_patchset_id = if repo_name.is_some() && !exact_id {
            let patchset = self.manager.get_patchset(patchset_id, repo_name, None)?;
            PatchsetJson::stateless().resolved_patchset_id_from_payload(&patchset, patchset_id)
        } else {
            patchset_id.to_string()
        };
        self.manager
            .create_waiver(&resolved_patchset_id, rule_name, reason, expires_at)
    }
}
