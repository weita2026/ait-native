use super::*;

impl HttpWorkflowCloseoutRemote {
    pub fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        run_patchset_ci_with_task_workflow_closeout_remote(
            self,
            patchset_id,
            trigger,
            execution_profile,
            repo_name,
            exact_id,
        )
    }

    pub fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        read_patchset_ci_status_with_task_workflow_closeout_remote(
            self,
            patchset_id,
            recent_limit,
            repo_name,
            exact_id,
        )
    }

    pub fn read_patchset_ci_readiness(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
        repo_name: Option<&str>,
        exact_id: bool,
    ) -> TaskWorkflowHttpClientResult<Value> {
        read_patchset_ci_readiness_with_task_workflow_closeout_remote(
            self,
            patchset_id,
            recent_limit,
            repo_name,
            exact_id,
        )
    }

    pub fn list_repo_jobs(
        &mut self,
        repo_name: &str,
        state: Option<&str>,
        limit: i64,
        diagnostics: bool,
        stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<Value> {
        list_repo_jobs_with_task_workflow_closeout_remote(
            self,
            repo_name,
            state,
            limit,
            diagnostics,
            stale_after_seconds,
        )
    }
}

impl TaskWorkflowPatchsetCiRunner for HttpWorkflowCloseoutRemote {
    fn run_patchset_ci(
        &mut self,
        patchset_id: &str,
        trigger: &str,
        execution_profile: Option<&str>,
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
            .run_patchset_ci(&resolved_patchset_id, trigger, execution_profile)
    }
}

impl TaskWorkflowPatchsetCiStatusReader for HttpWorkflowCloseoutRemote {
    fn read_patchset_ci_status(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
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
            .read_patchset_ci_status(&resolved_patchset_id, recent_limit)
    }

    fn read_patchset_ci_readiness(
        &mut self,
        patchset_id: &str,
        recent_limit: i64,
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
            .read_patchset_ci_readiness(&resolved_patchset_id, recent_limit)
    }
}

impl TaskWorkflowRepoJobLister for HttpWorkflowCloseoutRemote {
    fn list_repo_jobs(
        &mut self,
        repo_name: &str,
        state: Option<&str>,
        limit: i64,
        diagnostics: bool,
        stale_after_seconds: i64,
    ) -> TaskWorkflowHttpClientResult<Value> {
        self.manager
            .list_repo_jobs(repo_name, state, limit, diagnostics, stale_after_seconds)
    }
}
