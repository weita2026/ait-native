use super::*;

pub fn build_create_planning_session_request_spec(
    config: &PlanHttpClientConfig,
    plan_id: &str,
    title: Option<&str>,
    mode: &str,
    preferred_agent: Option<&str>,
    resume_if_active: bool,
    planning_session_id: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let plan_id = require_non_empty_text(plan_id, "plan_id")?;
    let mode = require_non_empty_text(mode, "mode")?;
    let mut body = Map::new();
    body.insert("mode".to_string(), Value::String(mode));
    body.insert(
        "resume_if_active".to_string(),
        Value::Bool(resume_if_active),
    );
    insert_optional_string(&mut body, "title", title);
    insert_optional_string(&mut body, "preferred_agent", preferred_agent);
    insert_optional_string(&mut body, "planning_session_id", planning_session_id);
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("/v1/native/sprints/{plan_id}/planning-sessions"),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_list_planning_sessions_request_spec(
    config: &PlanHttpClientConfig,
    plan_id: &str,
    status: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let plan_id = require_non_empty_text(plan_id, "plan_id")?;
    let mut query_pairs = Vec::new();
    if let Some(status) = normalize_optional_text(status) {
        query_pairs.push(("status".to_string(), status));
    }
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!("/v1/native/sprints/{plan_id}/planning-sessions"),
        query_pairs,
        None,
    )
}

pub fn build_get_planning_session_request_spec(
    config: &PlanHttpClientConfig,
    planning_session_id: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let planning_session_id = require_non_empty_text(planning_session_id, "planning_session_id")?;
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!("/v1/native/planning-sessions/{planning_session_id}"),
        Vec::new(),
        None,
    )
}

pub fn build_append_planning_session_event_request_spec(
    config: &PlanHttpClientConfig,
    planning_session_id: &str,
    event_type: &str,
    payload: &Value,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let planning_session_id = require_non_empty_text(planning_session_id, "planning_session_id")?;
    let event_type = require_non_empty_text(event_type, "event_type")?;
    let payload = if payload.is_null() {
        Value::Object(Map::new())
    } else {
        payload.clone()
    };
    if !matches!(payload, Value::Object(_)) {
        return Err(PlanHttpClientError::Invalid(
            "Planning session event payload must be an object.".to_string(),
        ));
    }
    let mut body = Map::new();
    body.insert("event_type".to_string(), Value::String(event_type));
    body.insert("payload".to_string(), payload);
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("/v1/native/planning-sessions/{planning_session_id}/events"),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_list_planning_session_events_request_spec(
    config: &PlanHttpClientConfig,
    planning_session_id: &str,
    after_sequence: i64,
    limit: i64,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let planning_session_id = require_non_empty_text(planning_session_id, "planning_session_id")?;
    let mut query_pairs = Vec::new();
    query_pairs.push(("after_sequence".to_string(), after_sequence.to_string()));
    query_pairs.push(("limit".to_string(), limit.to_string()));
    plan_http_transport::build_request_spec(
        config,
        Method::GET,
        &format!("/v1/native/planning-sessions/{planning_session_id}/events"),
        query_pairs,
        None,
    )
}

pub fn build_join_planning_session_request_spec(
    config: &PlanHttpClientConfig,
    planning_session_id: &str,
    surface: &str,
    title: Option<&str>,
    model_name: Option<&str>,
    resume_if_active: bool,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let planning_session_id = require_non_empty_text(planning_session_id, "planning_session_id")?;
    let surface = require_non_empty_text(surface, "surface")?;
    let mut body = Map::new();
    body.insert("surface".to_string(), Value::String(surface));
    body.insert(
        "resume_if_active".to_string(),
        Value::Bool(resume_if_active),
    );
    insert_optional_string(&mut body, "title", title);
    insert_optional_string(&mut body, "model_name", model_name);
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("/v1/native/planning-sessions/{planning_session_id}:join"),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_promote_planning_session_request_spec(
    config: &PlanHttpClientConfig,
    planning_session_id: &str,
    artifact_path: &str,
    artifact_selector: &str,
    artifact_heading: &str,
    items: &[Value],
    title: Option<&str>,
    summary: Option<&str>,
    artifact_body: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let planning_session_id = require_non_empty_text(planning_session_id, "planning_session_id")?;
    let artifact_path = require_non_empty_text(artifact_path, "artifact_path")?;
    let artifact_selector = require_non_empty_text(artifact_selector, "artifact_selector")?;
    let artifact_heading = require_non_empty_text(artifact_heading, "artifact_heading")?;
    let mut body = Map::new();
    body.insert("artifact_path".to_string(), Value::String(artifact_path));
    body.insert(
        "artifact_selector".to_string(),
        Value::String(artifact_selector),
    );
    body.insert(
        "artifact_heading".to_string(),
        Value::String(artifact_heading),
    );
    body.insert("items".to_string(), Value::Array(items.to_vec()));
    insert_optional_string(&mut body, "title", title);
    insert_optional_string(&mut body, "summary", summary);
    insert_optional_exact_string(&mut body, "artifact_body", artifact_body);
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("/v1/native/planning-sessions/{planning_session_id}:promote"),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_close_planning_session_request_spec(
    config: &PlanHttpClientConfig,
    planning_session_id: &str,
    status: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let planning_session_id = require_non_empty_text(planning_session_id, "planning_session_id")?;
    let status = require_non_empty_text(status, "status")?;
    let mut body = Map::new();
    body.insert("status".to_string(), Value::String(status));
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("/v1/native/planning-sessions/{planning_session_id}:close"),
        Vec::new(),
        Some(Value::Object(body)),
    )
}
