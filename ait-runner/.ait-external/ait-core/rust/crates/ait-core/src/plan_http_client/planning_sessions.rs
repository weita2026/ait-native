use super::*;

impl PlanHttpClientManager {
    pub fn create_planning_session(
        &mut self,
        plan_id: &str,
        title: Option<&str>,
        mode: &str,
        preferred_agent: Option<&str>,
        resume_if_active: bool,
        planning_session_id: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_create_planning_session_request_spec(
            &self.config,
            plan_id,
            title,
            mode,
            preferred_agent,
            resume_if_active,
            planning_session_id,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_planning_sessions(
        &mut self,
        plan_id: &str,
        status: Option<&str>,
    ) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_planning_sessions_request_spec(&self.config, plan_id, status)?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn get_planning_session(
        &mut self,
        planning_session_id: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_get_planning_session_request_spec(&self.config, planning_session_id)?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn append_planning_session_event(
        &mut self,
        planning_session_id: &str,
        event_type: &str,
        payload: &Value,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_append_planning_session_event_request_spec(
            &self.config,
            planning_session_id,
            event_type,
            payload,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn list_planning_session_events(
        &mut self,
        planning_session_id: &str,
        after_sequence: i64,
        limit: i64,
    ) -> PlanHttpClientResult<Vec<Value>> {
        let spec = build_list_planning_session_events_request_spec(
            &self.config,
            planning_session_id,
            after_sequence,
            limit,
        )?;
        parse_list_payload(self.execute_json(spec)?)
    }

    pub fn join_planning_session(
        &mut self,
        planning_session_id: &str,
        surface: &str,
        title: Option<&str>,
        model_name: Option<&str>,
        resume_if_active: bool,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_join_planning_session_request_spec(
            &self.config,
            planning_session_id,
            surface,
            title,
            model_name,
            resume_if_active,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn promote_planning_session(
        &mut self,
        planning_session_id: &str,
        artifact_path: &str,
        artifact_selector: &str,
        artifact_heading: &str,
        items: &[Value],
        title: Option<&str>,
        summary: Option<&str>,
        artifact_body: Option<&str>,
    ) -> PlanHttpClientResult<Value> {
        let spec = build_promote_planning_session_request_spec(
            &self.config,
            planning_session_id,
            artifact_path,
            artifact_selector,
            artifact_heading,
            items,
            title,
            summary,
            artifact_body,
        )?;
        parse_object_payload(self.execute_json(spec)?)
    }

    pub fn close_planning_session(
        &mut self,
        planning_session_id: &str,
        status: &str,
    ) -> PlanHttpClientResult<Value> {
        let spec =
            build_close_planning_session_request_spec(&self.config, planning_session_id, status)?;
        parse_object_payload(self.execute_json(spec)?)
    }
}

pub fn create_planning_session(
    config: PlanHttpClientConfig,
    plan_id: &str,
    title: Option<&str>,
    mode: &str,
    preferred_agent: Option<&str>,
    resume_if_active: bool,
    planning_session_id: Option<&str>,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.create_planning_session(
        plan_id,
        title,
        mode,
        preferred_agent,
        resume_if_active,
        planning_session_id,
    )
}

pub fn list_planning_sessions(
    config: PlanHttpClientConfig,
    plan_id: &str,
    status: Option<&str>,
) -> PlanHttpClientResult<Vec<Value>> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.list_planning_sessions(plan_id, status)
}

pub fn get_planning_session(
    config: PlanHttpClientConfig,
    planning_session_id: &str,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.get_planning_session(planning_session_id)
}

pub fn append_planning_session_event(
    config: PlanHttpClientConfig,
    planning_session_id: &str,
    event_type: &str,
    payload: &Value,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.append_planning_session_event(planning_session_id, event_type, payload)
}

pub fn list_planning_session_events(
    config: PlanHttpClientConfig,
    planning_session_id: &str,
    after_sequence: i64,
    limit: i64,
) -> PlanHttpClientResult<Vec<Value>> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.list_planning_session_events(planning_session_id, after_sequence, limit)
}

pub fn join_planning_session(
    config: PlanHttpClientConfig,
    planning_session_id: &str,
    surface: &str,
    title: Option<&str>,
    model_name: Option<&str>,
    resume_if_active: bool,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.join_planning_session(
        planning_session_id,
        surface,
        title,
        model_name,
        resume_if_active,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn promote_planning_session(
    config: PlanHttpClientConfig,
    planning_session_id: &str,
    artifact_path: &str,
    artifact_selector: &str,
    artifact_heading: &str,
    items: &[Value],
    title: Option<&str>,
    summary: Option<&str>,
    artifact_body: Option<&str>,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.promote_planning_session(
        planning_session_id,
        artifact_path,
        artifact_selector,
        artifact_heading,
        items,
        title,
        summary,
        artifact_body,
    )
}

pub fn close_planning_session(
    config: PlanHttpClientConfig,
    planning_session_id: &str,
    status: &str,
) -> PlanHttpClientResult<Value> {
    let mut manager = PlanHttpClientManager::new(config)?;
    manager.close_planning_session(planning_session_id, status)
}
