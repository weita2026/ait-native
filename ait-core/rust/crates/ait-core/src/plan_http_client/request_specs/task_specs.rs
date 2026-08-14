use super::*;

pub fn build_list_tasks_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    TaskJson::stateless().build_list_tasks_request_spec(config, repo_name)
}

pub fn build_get_task_request_spec(
    config: &PlanHttpClientConfig,
    task_id: &str,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    TaskJson::stateless().build_get_task_request_spec(config, task_id, repo_name)
}

pub fn build_read_task_audit_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    task_id: &str,
    target_line: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    TaskJson::stateless().build_read_task_audit_request_spec(
        config,
        repo_name,
        task_id,
        target_line,
    )
}

pub fn build_read_task_queue_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    status: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    TaskJson::stateless().build_read_task_queue_request_spec(config, repo_name, status)
}

pub fn build_read_reviewer_inbox_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    TaskJson::stateless().build_read_reviewer_inbox_request_spec(config, repo_name)
}

pub fn build_read_queue_summary_bundle_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    status: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    TaskJson::stateless().build_read_queue_summary_bundle_request_spec(config, repo_name, status)
}

pub fn build_close_line_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    line_name: &str,
    status: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    let line_name = require_non_empty_text(line_name, "line_name")?;
    let status = require_non_empty_text(status, "status")?;
    let mut body = Map::new();
    body.insert("status".to_string(), Value::String(status));
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!(
            "/v1/native/repository-authorities/{repository_index}/lines/{}:close",
            encode_path_segment(&line_name)
        ),
        Vec::new(),
        Some(Value::Object(body)),
    )
}

pub fn build_start_plan_bound_task_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    payload: &Value,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let repository_index = configured_repository_authority_path_segment(config)?;
    let body = payload.as_object().ok_or_else(|| {
        PlanHttpClientError::Invalid(
            "Plan HTTP atomic task-start payload must be an object.".to_string(),
        )
    })?;
    if body.get("contract").and_then(Value::as_str) != Some("task-start-atomic/v1") {
        return Err(PlanHttpClientError::Invalid(
            "Plan HTTP atomic task-start contract must be `task-start-atomic/v1`.".to_string(),
        ));
    }
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &format!("/v1/native/repository-authorities/{repository_index}/task-start"),
        Vec::new(),
        Some(Value::Object(body.clone())),
    )
}

pub fn build_prepare_history_promotion_request_spec(
    config: &PlanHttpClientConfig,
    _repo_name: &str,
    payload: &Value,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    let body = payload.as_object().ok_or_else(|| {
        PlanHttpClientError::Invalid(
            "Plan HTTP history promotion payload must be an object.".to_string(),
        )
    })?;
    if body.get("contract").and_then(Value::as_str) != Some("history-promotion-prepare/v1") {
        return Err(PlanHttpClientError::Invalid(
            "Plan HTTP history promotion contract must be `history-promotion-prepare/v1`."
                .to_string(),
        ));
    }
    let idempotency_key = body
        .get("idempotency_key")
        .and_then(Value::as_str)
        .and_then(|value| normalize_optional_text(Some(value)))
        .ok_or_else(|| {
            PlanHttpClientError::Invalid(
                "Plan HTTP history promotion idempotency_key must be non-empty.".to_string(),
            )
        })?;
    if idempotency_key.len() > 256 {
        return Err(PlanHttpClientError::Invalid(
            "Plan HTTP history promotion idempotency_key must not exceed 256 bytes.".to_string(),
        ));
    }
    let repository_index = configured_repository_authority_path_segment(config)?;
    let path =
        format!("/v1/native/repository-authorities/{repository_index}/history-promotion:prepare");
    plan_http_transport::build_request_spec(
        config,
        Method::POST,
        &path,
        Vec::new(),
        Some(Value::Object(body.clone())),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_create_task_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    title: &str,
    intent: &str,
    task_id: Option<&str>,
    plan_id: Option<&str>,
    origin_plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    TaskJson::stateless().build_create_task_request_spec(
        config,
        repo_name,
        title,
        intent,
        task_id,
        plan_id,
        origin_plan_revision_id,
        plan_item_ref,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn build_create_change_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
    task_id: &str,
    title: &str,
    base_line: &str,
    change_id: Option<&str>,
    fork_snapshot_id: Option<&str>,
    forked_from_line: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    ChangeJson::stateless().build_create_change_request_spec(
        config,
        repo_name,
        task_id,
        title,
        base_line,
        change_id,
        fork_snapshot_id,
        forked_from_line,
    )
}

pub fn build_list_changes_request_spec(
    config: &PlanHttpClientConfig,
    repo_name: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    ChangeJson::stateless().build_list_changes_request_spec(config, repo_name)
}

pub fn build_get_change_detail_request_spec(
    config: &PlanHttpClientConfig,
    change_ref: &str,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    ChangeJson::stateless().build_get_change_detail_request_spec(config, change_ref, repo_name)
}

pub fn build_get_change_request_spec(
    config: &PlanHttpClientConfig,
    change_ref: &str,
    repo_name: Option<&str>,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    ChangeJson::stateless().build_get_change_request_spec(config, change_ref, repo_name)
}

pub fn build_close_change_request_spec(
    config: &PlanHttpClientConfig,
    change_id: &str,
    status: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    ChangeJson::stateless().build_close_change_request_spec(config, change_id, status)
}

pub fn build_close_task_request_spec(
    config: &PlanHttpClientConfig,
    task_id: &str,
    status: &str,
) -> PlanHttpClientResult<PlanHttpRequestSpec> {
    TaskJson::stateless().build_close_task_request_spec(config, task_id, status)
}
