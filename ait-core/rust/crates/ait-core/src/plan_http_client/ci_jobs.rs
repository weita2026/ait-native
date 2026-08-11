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

    #[allow(clippy::too_many_arguments)]
    pub fn run_repo_ci(
        &mut self,
        repo_name: &str,
        suite_ids: &[String],
        plane: Option<&str>,
        target_line: &str,
        trigger: &str,
        selector: Option<&str>,
        task_ids: &[String],
        curated_corpus: Option<&str>,
        count: Option<i64>,
        window_days: Option<i64>,
        dependency_evidence: &[String],
        compliance_evidence: &[String],
    ) -> PlanHttpClientResult<Value> {
        let spec = build_run_repo_ci_request_spec(
            &self.config,
            repo_name,
            suite_ids,
            plane,
            target_line,
            trigger,
            selector,
            task_ids,
            curated_corpus,
            count,
            window_days,
            dependency_evidence,
            compliance_evidence,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn read_repository_ci_runs(
        &mut self,
        repo_name: &str,
        limit: i64,
        plane: Option<&str>,
        suite_id: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_read_repository_ci_runs_request_spec(
            &self.config,
            repo_name,
            limit,
            plane,
            suite_id,
        )?;
        parse_any_payload(self.execute_json(spec)?)
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

#[allow(clippy::too_many_arguments)]
pub fn run_repo_ci(
    config: PlanHttpClientConfig,
    repo_name: &str,
    suite_ids: &[String],
    plane: Option<&str>,
    target_line: &str,
    trigger: &str,
    selector: Option<&str>,
    task_ids: &[String],
    curated_corpus: Option<&str>,
    count: Option<i64>,
    window_days: Option<i64>,
    dependency_evidence: &[String],
    compliance_evidence: &[String],
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.run_repo_ci(
        repo_name,
        suite_ids,
        plane,
        target_line,
        trigger,
        selector,
        task_ids,
        curated_corpus,
        count,
        window_days,
        dependency_evidence,
        compliance_evidence,
    )
}

pub fn read_repository_ci_runs(
    config: PlanHttpClientConfig,
    repo_name: &str,
    limit: i64,
    plane: Option<&str>,
    suite_id: Option<&str>,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.read_repository_ci_runs(repo_name, limit, plane, suite_id)
}
