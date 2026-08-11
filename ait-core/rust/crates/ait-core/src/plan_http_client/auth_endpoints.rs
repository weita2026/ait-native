use super::*;

impl PlanHttpClientManager {
    pub fn auth_whoami(&mut self, repo_name: Option<&str>) -> PlanHttpClientResult<Value> {
        let spec = build_auth_whoami_request_spec(&self.config, repo_name)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn grant_role_bindings(
        &mut self,
        repo_name: &str,
        actor_identity: &str,
        roles: &[String],
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_grant_role_bindings_request_spec(&self.config, repo_name, actor_identity, roles)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_role_bindings(&mut self, repo_name: &str) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_role_bindings_request_spec(&self.config, repo_name)?;
        parse_list_payload(self.execute_json(spec)?)
    }
}

pub fn auth_whoami(
    config: PlanHttpClientConfig,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.auth_whoami(repo_name)
}

pub fn grant_role_bindings(
    config: PlanHttpClientConfig,
    repo_name: &str,
    actor_identity: &str,
    roles: &[String],
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.grant_role_bindings(repo_name, actor_identity, roles)
}

pub fn list_role_bindings(
    config: PlanHttpClientConfig,
    repo_name: &str,
) -> PlanHttpClientResult<Vec<Value>> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.list_role_bindings(repo_name)
}
