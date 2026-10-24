use super::*;

impl PlanHttpClientManager {
    pub fn list_repo_jobs(
        &mut self,
        repo_name: &str,
        state: Option<&str>,
        limit: i64,
        diagnostics: bool,
        stale_after_seconds: i64,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_list_repo_jobs_request_spec(
            &self.config,
            repo_name,
            state,
            limit,
            diagnostics,
            stale_after_seconds,
        )?;
        parse_any_payload(self.execute_json(spec)?)
    }

    pub fn get_repo_job(&mut self, job_id: i64) -> PlanHttpClientResult<Value> {
        let spec = build_get_repo_job_request_spec(&self.config, job_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn reconcile_repo(&mut self, repo_name: &str, repair: bool) -> PlanHttpClientResult<Value> {
        let spec = build_reconcile_repo_request_spec(&self.config, repo_name, repair)?;
        parse_object_payload(self.execute_json(spec)?)
    }
}

pub fn get_repo_job(config: PlanHttpClientConfig, job_id: i64) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.get_repo_job(job_id)
}

pub fn reconcile_repo(
    config: PlanHttpClientConfig,
    repo_name: &str,
    repair: bool,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.reconcile_repo(repo_name, repair)
}
