use super::*;

impl PlanHttpClientManager {
    pub fn ensure_repository(
        &mut self,
        repo_name: &str,
        default_line: &str,
        policy: Option<&Value>,
        id_namespace_prefix: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_ensure_repository_request_spec(
            &self.config,
            repo_name,
            default_line,
            policy,
            id_namespace_prefix,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_repository(&mut self, repo_name: &str) -> PlanHttpClientResult<Value> {
        let _ = repo_name;
        let repository_index = self.config.repository_index.ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "Plan HTTP repository_index is required for repository-authority operations."
                    .to_string(),
            )
        })?;
        self.get_repository_authority_by_index(repository_index)
    }

    pub fn get_repository_storage(&mut self, repo_name: &str) -> PlanHttpClientResult<Value> {
        let spec = build_get_repository_storage_request_spec(&self.config, repo_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn pack_repo(
        &mut self,
        repo_name: &str,
        repack: bool,
        max_members: Option<i64>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_pack_repo_request_spec(&self.config, repo_name, repack, max_members)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn optimize_repo(&mut self, repo_name: &str, repair: bool) -> PlanHttpClientResult<Value> {
        let spec = build_optimize_repo_request_spec(&self.config, repo_name, repair)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn gc_repo(
        &mut self,
        repo_name: &str,
        prune_unreferenced: bool,
        prune_orphan_packs: bool,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_gc_repo_request_spec(
            &self.config,
            repo_name,
            prune_unreferenced,
            prune_orphan_packs,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn retire_repo(
        &mut self,
        repo_name: &str,
        expected_repository_identity: &str,
        require_verified_export: bool,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_retire_repo_request_spec(
            &self.config,
            repo_name,
            expected_repository_identity,
            require_verified_export,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }
}

pub fn get_repository_storage(
    config: PlanHttpClientConfig,
    repo_name: &str,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.get_repository_storage(repo_name)
}

pub fn pack_repo(
    config: PlanHttpClientConfig,
    repo_name: &str,
    repack: bool,
    max_members: Option<i64>,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.pack_repo(repo_name, repack, max_members)
}

pub fn optimize_repo(
    config: PlanHttpClientConfig,
    repo_name: &str,
    repair: bool,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.optimize_repo(repo_name, repair)
}

pub fn gc_repo(
    config: PlanHttpClientConfig,
    repo_name: &str,
    prune_unreferenced: bool,
    prune_orphan_packs: bool,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.gc_repo(repo_name, prune_unreferenced, prune_orphan_packs)
}

pub fn retire_repo(
    config: PlanHttpClientConfig,
    repo_name: &str,
    expected_repository_identity: &str,
    require_verified_export: bool,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.retire_repo(
        repo_name,
        expected_repository_identity,
        require_verified_export,
    )
}

pub fn list_repo_jobs(
    config: PlanHttpClientConfig,
    repo_name: &str,
    state: Option<&str>,
    limit: i64,
    diagnostics: bool,
    stale_after_seconds: i64,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.list_repo_jobs(repo_name, state, limit, diagnostics, stale_after_seconds)
}
