use super::*;

impl PlanHttpClientManager {
    pub fn get_server_handshake(&mut self) -> PlanHttpClientResult<Value> {
        let spec = build_get_server_handshake_request_spec(&self.config)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_server_health(&mut self) -> PlanHttpClientResult<Value> {
        let spec = build_get_server_health_request_spec(&self.config)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_server_metrics(
        &mut self,
        recent_jobs_limit: i64,
        stale_after_seconds: i64,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_server_metrics_request_spec(
            &self.config,
            recent_jobs_limit,
            stale_after_seconds,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn get_server_readiness(
        &mut self,
        recent_jobs_limit: i64,
        stale_after_seconds: i64,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_server_readiness_request_spec(
            &self.config,
            recent_jobs_limit,
            stale_after_seconds,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }
}

pub fn get_server_health(config: PlanHttpClientConfig) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.get_server_health()
}

pub fn get_server_metrics(
    config: PlanHttpClientConfig,
    recent_jobs_limit: i64,
    stale_after_seconds: i64,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.get_server_metrics(recent_jobs_limit, stale_after_seconds)
}

pub fn get_server_readiness(
    config: PlanHttpClientConfig,
    recent_jobs_limit: i64,
    stale_after_seconds: i64,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.get_server_readiness(recent_jobs_limit, stale_after_seconds)
}
