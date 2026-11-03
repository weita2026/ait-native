//! Retained Plan request builders and direct native HTTP operations.
use crate::{workspace::native_count, JsonInput};
use ait_core::{json_support::JsonValue, plan_http_client as http, plan_http_sdk as sdk};
use napi::{Error, Result};
use napi_derive::napi;
use std::collections::BTreeMap;

#[napi]
pub fn join_path(base_url: String, path: String) -> String {
    sdk::join_path(&base_url, &path)
}
#[napi]
pub fn build_query_string(params: JsonInput) -> Result<Option<String>> {
    let rows = params
        .as_object()
        .ok_or_else(|| Error::from_reason("query parameters must be an object."))?;
    let params = rows
        .iter()
        .map(|(key, value)| {
            value
                .as_str()
                .map(|text| (key.clone(), text.to_string()))
                .ok_or_else(|| Error::from_reason("query parameter values must be strings."))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    Ok(sdk::build_query_string(&params))
}
#[napi]
pub fn normalize_remote_text(value: Option<String>) -> Option<String> {
    sdk::normalize_remote_text(value.as_deref())
}

#[napi(object)]
pub struct CreatePlanInput {
    pub repo_name: String,
    pub title: String,
    pub artifact_path: String,
    pub artifact_selector: Option<String>,
    pub artifact_heading: String,
    pub items: JsonInput,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub plan_id: Option<String>,
    pub source_kind: Option<String>,
    pub artifact_body: Option<String>,
}
#[napi(object)]
pub struct RevisePlanInput {
    pub plan_id: String,
    pub artifact_path: String,
    pub artifact_selector: Option<String>,
    pub artifact_heading: String,
    pub items: JsonInput,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub source_kind: Option<String>,
    pub artifact_body: Option<String>,
    pub expected_head_revision_id: Option<String>,
}

#[napi(object)]
#[derive(Default)]
pub struct PlanHttpOptions {
    pub headers_json: Option<String>,
    pub timeout_ms: Option<JsonInput>,
    pub retry_attempts: Option<JsonInput>,
    pub retry_backoff_ms: Option<JsonInput>,
    pub pool_max_idle_per_host: Option<JsonInput>,
    pub repository_index: Option<JsonInput>,
}
fn u64_input(value: Option<JsonInput>, name: &str) -> Result<Option<u64>> {
    value
        .map(|v| {
            v.as_u64()
                .ok_or_else(|| Error::from_reason(format!("{name} must fit in u64.")))
        })
        .transpose()
}
pub(crate) fn client_config(
    base_url: &str,
    options: Option<PlanHttpOptions>,
) -> Result<http::PlanHttpClientConfig> {
    let options = options.unwrap_or_default();
    let repository_index = u64_input(options.repository_index, "repository_index")?
        .map(|value| {
            u32::try_from(value)
                .map_err(|_| Error::from_reason("repository_index must fit in u32."))
        })
        .transpose()?;
    sdk::client_config(
        base_url,
        options.headers_json.as_deref(),
        u64_input(options.timeout_ms, "timeout_ms")?,
        native_count(options.retry_attempts, 0, "retry_attempts")?,
        u64_input(options.retry_backoff_ms, "retry_backoff_ms")?.unwrap_or(0),
        native_count(options.pool_max_idle_per_host, 1, "pool_max_idle_per_host")?,
        repository_index,
    )
    .map_err(Error::from_reason)
}
fn array<'a>(value: &'a JsonValue, field: &str) -> Result<&'a [JsonValue]> {
    value
        .as_array()
        .map(Vec::as_slice)
        .ok_or_else(|| Error::from_reason(format!("{field} must be a list.")))
}
fn i64_input(value: Option<JsonInput>, default: i64, name: &str) -> Result<i64> {
    value
        .map(|v| {
            v.as_i64()
                .ok_or_else(|| Error::from_reason(format!("{name} must fit in i64.")))
        })
        .transpose()
        .map(|v| v.unwrap_or(default))
}
pub(crate) fn http_error(error: http::PlanHttpClientError) -> Error {
    Error::from_reason(error.to_string())
}

#[napi]
pub fn build_list_plans_request(
    base_url: String,
    repo_name: String,
    artifact_path: Option<String>,
) -> Result<JsonValue> {
    sdk::build_list_plans_request(&base_url, &repo_name, artifact_path.as_deref())
        .map_err(Error::from_reason)
}

#[napi]
pub fn build_get_plan_request(base_url: String, plan_id: String) -> Result<JsonValue> {
    sdk::build_get_plan_request(&base_url, &plan_id).map_err(Error::from_reason)
}

#[napi]
pub fn build_list_plan_revisions_request(base_url: String, plan_id: String) -> Result<JsonValue> {
    sdk::build_list_plan_revisions_request(&base_url, &plan_id).map_err(Error::from_reason)
}

#[napi]
pub fn build_get_plan_revision_request(
    base_url: String,
    plan_id: String,
    plan_revision_id: String,
) -> Result<JsonValue> {
    sdk::build_get_plan_revision_request(&base_url, &plan_id, &plan_revision_id)
        .map_err(Error::from_reason)
}

#[napi]
pub fn build_create_plan_request(base_url: String, input: CreatePlanInput) -> Result<JsonValue> {
    sdk::build_create_plan_request(
        &base_url,
        &input.repo_name,
        &input.title,
        &input.artifact_path,
        input.artifact_selector.as_deref(),
        &input.artifact_heading,
        &input.items,
        input.summary.as_deref(),
        input.status.as_deref().unwrap_or("draft"),
        input.plan_id.as_deref(),
        input.source_kind.as_deref(),
        input.artifact_body.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn build_revise_plan_request(base_url: String, input: RevisePlanInput) -> Result<JsonValue> {
    sdk::build_revise_plan_request(
        &base_url,
        &input.plan_id,
        &input.artifact_path,
        input.artifact_selector.as_deref(),
        &input.artifact_heading,
        &input.items,
        input.title.as_deref(),
        input.summary.as_deref(),
        input.source_kind.as_deref(),
        input.artifact_body.as_deref(),
        input.expected_head_revision_id.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn build_update_plan_status_request(
    base_url: String,
    plan_id: String,
    status: String,
) -> Result<JsonValue> {
    sdk::build_update_plan_status_request(&base_url, &plan_id, &status).map_err(Error::from_reason)
}

#[napi]
pub fn build_put_plan_revision_artifacts_request(
    base_url: String,
    plan_id: String,
    plan_revision_id: String,
    artifacts: JsonInput,
) -> Result<JsonValue> {
    sdk::build_put_plan_revision_artifacts_request(
        &base_url,
        &plan_id,
        &plan_revision_id,
        &artifacts,
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskPlanLinkageOptions {
    pub plan_id: Option<String>,
    pub origin_plan_revision_id: Option<String>,
    pub plan_item_ref: Option<String>,
}
#[napi(object)]
#[derive(Default)]
pub struct CreatePlanningSessionOptions {
    pub title: Option<String>,
    pub mode: Option<String>,
    pub preferred_agent: Option<String>,
    pub resume_if_active: Option<bool>,
    pub planning_session_id: Option<String>,
}
#[napi(object)]
#[derive(Default)]
pub struct PlanningSessionEventsOptions {
    pub after_sequence: Option<JsonInput>,
    pub limit: Option<JsonInput>,
}
#[napi(object)]
#[derive(Default)]
pub struct JoinPlanningSessionOptions {
    pub surface: Option<String>,
    pub title: Option<String>,
    pub model_name: Option<String>,
    pub resume_if_active: Option<bool>,
}
#[napi(object)]
pub struct PromotePlanningSessionInput {
    pub artifact_path: String,
    pub artifact_selector: String,
    pub artifact_heading: String,
    pub items: JsonInput,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub artifact_body: Option<String>,
}

#[napi]
pub fn list_plans(
    base_url: String,
    repo_name: String,
    artifact_path: Option<String>,
    options: Option<PlanHttpOptions>,
) -> Result<Vec<JsonValue>> {
    http::list_plans(
        client_config(&base_url, options)?,
        &repo_name,
        artifact_path.as_deref(),
    )
    .map_err(http_error)
}

#[napi]
pub fn get_plan(
    base_url: String,
    plan_id: String,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::get_plan(client_config(&base_url, options)?, &plan_id).map_err(http_error)
}

#[napi]
pub fn list_plan_revisions(
    base_url: String,
    plan_id: String,
    options: Option<PlanHttpOptions>,
) -> Result<Vec<JsonValue>> {
    http::list_plan_revisions(client_config(&base_url, options)?, &plan_id).map_err(http_error)
}

#[napi]
pub fn get_plan_revision(
    base_url: String,
    plan_id: String,
    plan_revision_id: String,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::get_plan_revision(
        client_config(&base_url, options)?,
        &plan_id,
        &plan_revision_id,
    )
    .map_err(http_error)
}

#[napi]
pub fn resolve_task_plan_linkage(
    base_url: String,
    repo_name: String,
    linkage: Option<TaskPlanLinkageOptions>,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    let linkage = linkage.unwrap_or_default();
    http::resolve_task_plan_linkage(
        client_config(&base_url, options)?,
        &repo_name,
        linkage.plan_id.as_deref(),
        linkage.origin_plan_revision_id.as_deref(),
        linkage.plan_item_ref.as_deref(),
    )
    .map_err(http_error)
}

#[napi]
pub fn list_plan_ids_matching_contains(
    base_url: String,
    repo_name: String,
    contains_terms: Vec<String>,
    options: Option<PlanHttpOptions>,
) -> Result<Vec<JsonValue>> {
    http::list_plan_ids_matching_contains(
        client_config(&base_url, options)?,
        &repo_name,
        &contains_terms,
    )
    .map_err(http_error)
}

#[napi]
pub fn create_plan(
    base_url: String,
    input: CreatePlanInput,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::create_plan(
        client_config(&base_url, options)?,
        &input.repo_name,
        &input.title,
        &input.artifact_path,
        input.artifact_selector.as_deref(),
        &input.artifact_heading,
        array(&input.items, "items")?,
        input.summary.as_deref(),
        input.status.as_deref().unwrap_or("draft"),
        input.plan_id.as_deref(),
        input.source_kind.as_deref().unwrap_or("manual_edit"),
        input.artifact_body.as_deref(),
    )
    .map_err(http_error)
}

#[napi]
pub fn revise_plan(
    base_url: String,
    input: RevisePlanInput,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::revise_plan(
        client_config(&base_url, options)?,
        &input.plan_id,
        &input.artifact_path,
        input.artifact_selector.as_deref(),
        &input.artifact_heading,
        array(&input.items, "items")?,
        input.title.as_deref(),
        input.summary.as_deref(),
        input.source_kind.as_deref().unwrap_or("manual_edit"),
        input.artifact_body.as_deref(),
        input.expected_head_revision_id.as_deref(),
    )
    .map_err(http_error)
}

#[napi]
pub fn update_plan_status(
    base_url: String,
    plan_id: String,
    status: String,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::update_plan_status(client_config(&base_url, options)?, &plan_id, &status)
        .map_err(http_error)
}

#[napi]
pub fn put_plan_revision_artifacts(
    base_url: String,
    plan_id: String,
    plan_revision_id: String,
    artifacts: JsonInput,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::put_plan_revision_artifacts(
        client_config(&base_url, options)?,
        &plan_id,
        &plan_revision_id,
        array(&artifacts, "artifacts")?,
    )
    .map_err(http_error)
}

#[napi]
pub fn create_planning_session(
    base_url: String,
    plan_id: String,
    input: Option<CreatePlanningSessionOptions>,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    let input = input.unwrap_or_default();
    http::create_planning_session(
        client_config(&base_url, options)?,
        &plan_id,
        input.title.as_deref(),
        input.mode.as_deref().unwrap_or("connected_local"),
        input.preferred_agent.as_deref(),
        input.resume_if_active.unwrap_or(true),
        input.planning_session_id.as_deref(),
    )
    .map_err(http_error)
}

#[napi]
pub fn list_planning_sessions(
    base_url: String,
    plan_id: String,
    status: Option<String>,
    options: Option<PlanHttpOptions>,
) -> Result<Vec<JsonValue>> {
    http::list_planning_sessions(
        client_config(&base_url, options)?,
        &plan_id,
        status.as_deref(),
    )
    .map_err(http_error)
}

#[napi]
pub fn get_planning_session(
    base_url: String,
    planning_session_id: String,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::get_planning_session(client_config(&base_url, options)?, &planning_session_id)
        .map_err(http_error)
}

#[napi]
pub fn append_planning_session_event(
    base_url: String,
    planning_session_id: String,
    event_type: String,
    payload: Option<JsonInput>,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    let payload = payload
        .map(|value| value.0)
        .unwrap_or_else(|| ait_core::json_support::json!({}));
    http::append_planning_session_event(
        client_config(&base_url, options)?,
        &planning_session_id,
        &event_type,
        &payload,
    )
    .map_err(http_error)
}

#[napi]
pub fn list_planning_session_events(
    base_url: String,
    planning_session_id: String,
    input: Option<PlanningSessionEventsOptions>,
    options: Option<PlanHttpOptions>,
) -> Result<Vec<JsonValue>> {
    let input = input.unwrap_or_default();
    http::list_planning_session_events(
        client_config(&base_url, options)?,
        &planning_session_id,
        i64_input(input.after_sequence, 0, "after_sequence")?,
        i64_input(input.limit, 200, "limit")?,
    )
    .map_err(http_error)
}

#[napi]
pub fn join_planning_session(
    base_url: String,
    planning_session_id: String,
    input: Option<JoinPlanningSessionOptions>,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    let input = input.unwrap_or_default();
    http::join_planning_session(
        client_config(&base_url, options)?,
        &planning_session_id,
        input.surface.as_deref().unwrap_or("cli"),
        input.title.as_deref(),
        input.model_name.as_deref(),
        input.resume_if_active.unwrap_or(true),
    )
    .map_err(http_error)
}

#[napi]
pub fn promote_planning_session(
    base_url: String,
    planning_session_id: String,
    input: PromotePlanningSessionInput,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::promote_planning_session(
        client_config(&base_url, options)?,
        &planning_session_id,
        &input.artifact_path,
        &input.artifact_selector,
        &input.artifact_heading,
        array(&input.items, "items")?,
        input.title.as_deref(),
        input.summary.as_deref(),
        input.artifact_body.as_deref(),
    )
    .map_err(http_error)
}

#[napi]
pub fn close_planning_session(
    base_url: String,
    planning_session_id: String,
    status: Option<String>,
    options: Option<PlanHttpOptions>,
) -> Result<JsonValue> {
    http::close_planning_session(
        client_config(&base_url, options)?,
        &planning_session_id,
        status.as_deref().unwrap_or("closed"),
    )
    .map_err(http_error)
}

#[napi]
pub struct PlanRemoteTransportConfig {
    request_timeout_seconds: f64,
    long_running_request_timeout_seconds: f64,
}
#[napi]
impl PlanRemoteTransportConfig {
    #[napi(constructor)]
    pub fn new(
        request_timeout_seconds: Option<f64>,
        long_running_request_timeout_seconds: Option<f64>,
    ) -> Result<Self> {
        Ok(Self {
            request_timeout_seconds: sdk::remote_timeout("request", request_timeout_seconds)
                .map_err(Error::from_reason)?,
            long_running_request_timeout_seconds: sdk::remote_timeout(
                "long_running_request",
                long_running_request_timeout_seconds,
            )
            .map_err(Error::from_reason)?,
        })
    }
    #[napi]
    pub fn request_timeout_seconds(&self, long_running: bool) -> f64 {
        if long_running {
            self.long_running_request_timeout_seconds
        } else {
            self.request_timeout_seconds
        }
    }
}

#[napi]
pub struct PlanHttpClientManager {
    manager: http::PlanHttpClientManager,
}
#[napi]
impl PlanHttpClientManager {
    #[napi(constructor)]
    pub fn new(base_url: String, options: Option<PlanHttpOptions>) -> Result<Self> {
        Ok(Self {
            manager: http::PlanHttpClientManager::new(client_config(&base_url, options)?)
                .map_err(http_error)?,
        })
    }
    #[napi]
    pub fn inspect(&self) -> JsonValue {
        sdk::client_stats_payload(self.manager.inspect())
    }
    #[napi]
    pub fn close(&mut self) -> JsonValue {
        sdk::client_stats_payload(self.manager.close())
    }
    #[napi]
    pub fn list_plans(
        &mut self,
        repo_name: String,
        artifact_path: Option<String>,
    ) -> Result<Vec<JsonValue>> {
        self.manager
            .list_plans(&repo_name, artifact_path.as_deref())
            .map_err(http_error)
    }
    #[napi]
    pub fn get_plan(&mut self, plan_id: String) -> Result<JsonValue> {
        self.manager.get_plan(&plan_id).map_err(http_error)
    }
    #[napi]
    pub fn list_plan_revisions(&mut self, plan_id: String) -> Result<Vec<JsonValue>> {
        self.manager
            .list_plan_revisions(&plan_id)
            .map_err(http_error)
    }
    #[napi]
    pub fn get_plan_revision(
        &mut self,
        plan_id: String,
        plan_revision_id: String,
    ) -> Result<JsonValue> {
        self.manager
            .get_plan_revision(&plan_id, &plan_revision_id)
            .map_err(http_error)
    }
    #[napi]
    pub fn create_plan(&mut self, input: CreatePlanInput) -> Result<JsonValue> {
        self.manager
            .create_plan(
                &input.repo_name,
                &input.title,
                &input.artifact_path,
                input.artifact_selector.as_deref(),
                &input.artifact_heading,
                array(&input.items, "items")?,
                input.summary.as_deref(),
                input.status.as_deref().unwrap_or("draft"),
                input.plan_id.as_deref(),
                input.source_kind.as_deref().unwrap_or("manual_edit"),
                input.artifact_body.as_deref(),
            )
            .map_err(http_error)
    }
    #[napi]
    pub fn revise_plan(&mut self, input: RevisePlanInput) -> Result<JsonValue> {
        self.manager
            .revise_plan(
                &input.plan_id,
                &input.artifact_path,
                input.artifact_selector.as_deref(),
                &input.artifact_heading,
                array(&input.items, "items")?,
                input.title.as_deref(),
                input.summary.as_deref(),
                input.source_kind.as_deref().unwrap_or("manual_edit"),
                input.artifact_body.as_deref(),
                input.expected_head_revision_id.as_deref(),
            )
            .map_err(http_error)
    }
    #[napi]
    pub fn update_plan_status(&mut self, plan_id: String, status: String) -> Result<JsonValue> {
        self.manager
            .update_plan_status(&plan_id, &status)
            .map_err(http_error)
    }
    #[napi]
    pub fn put_plan_revision_artifacts(
        &mut self,
        plan_id: String,
        plan_revision_id: String,
        artifacts: JsonInput,
    ) -> Result<JsonValue> {
        self.manager
            .put_plan_revision_artifacts(
                &plan_id,
                &plan_revision_id,
                array(&artifacts, "artifacts")?,
            )
            .map_err(http_error)
    }
    #[napi]
    pub fn create_planning_session(
        &mut self,
        plan_id: String,
        input: Option<CreatePlanningSessionOptions>,
    ) -> Result<JsonValue> {
        let input = input.unwrap_or_default();
        self.manager
            .create_planning_session(
                &plan_id,
                input.title.as_deref(),
                input.mode.as_deref().unwrap_or("connected_local"),
                input.preferred_agent.as_deref(),
                input.resume_if_active.unwrap_or(true),
                input.planning_session_id.as_deref(),
            )
            .map_err(http_error)
    }
    #[napi]
    pub fn list_planning_sessions(
        &mut self,
        plan_id: String,
        status: Option<String>,
    ) -> Result<Vec<JsonValue>> {
        self.manager
            .list_planning_sessions(&plan_id, status.as_deref())
            .map_err(http_error)
    }
    #[napi]
    pub fn get_planning_session(&mut self, planning_session_id: String) -> Result<JsonValue> {
        self.manager
            .get_planning_session(&planning_session_id)
            .map_err(http_error)
    }
    #[napi]
    pub fn append_planning_session_event(
        &mut self,
        planning_session_id: String,
        event_type: String,
        payload: Option<JsonInput>,
    ) -> Result<JsonValue> {
        let payload = payload
            .map(|value| value.0)
            .unwrap_or_else(|| ait_core::json_support::json!({}));
        self.manager
            .append_planning_session_event(&planning_session_id, &event_type, &payload)
            .map_err(http_error)
    }
    #[napi]
    pub fn list_planning_session_events(
        &mut self,
        planning_session_id: String,
        input: Option<PlanningSessionEventsOptions>,
    ) -> Result<Vec<JsonValue>> {
        let input = input.unwrap_or_default();
        self.manager
            .list_planning_session_events(
                &planning_session_id,
                i64_input(input.after_sequence, 0, "after_sequence")?,
                i64_input(input.limit, 200, "limit")?,
            )
            .map_err(http_error)
    }
    #[napi]
    pub fn join_planning_session(
        &mut self,
        planning_session_id: String,
        input: Option<JoinPlanningSessionOptions>,
    ) -> Result<JsonValue> {
        let input = input.unwrap_or_default();
        self.manager
            .join_planning_session(
                &planning_session_id,
                input.surface.as_deref().unwrap_or("cli"),
                input.title.as_deref(),
                input.model_name.as_deref(),
                input.resume_if_active.unwrap_or(true),
            )
            .map_err(http_error)
    }
    #[napi]
    pub fn promote_planning_session(
        &mut self,
        planning_session_id: String,
        input: PromotePlanningSessionInput,
    ) -> Result<JsonValue> {
        self.manager
            .promote_planning_session(
                &planning_session_id,
                &input.artifact_path,
                &input.artifact_selector,
                &input.artifact_heading,
                array(&input.items, "items")?,
                input.title.as_deref(),
                input.summary.as_deref(),
                input.artifact_body.as_deref(),
            )
            .map_err(http_error)
    }
    #[napi]
    pub fn close_planning_session(
        &mut self,
        planning_session_id: String,
        status: Option<String>,
    ) -> Result<JsonValue> {
        self.manager
            .close_planning_session(&planning_session_id, status.as_deref().unwrap_or("closed"))
            .map_err(http_error)
    }
}

#[napi]
pub struct TaskWorkflowHttpClientManager {
    manager: ait_core::task_workflow_http_adapter::TaskWorkflowHttpClientManager,
}
#[napi]
impl TaskWorkflowHttpClientManager {
    #[napi(constructor)]
    pub fn new(base_url: String, options: Option<PlanHttpOptions>) -> Result<Self> {
        Ok(Self {
            manager: ait_core::task_workflow_http_adapter::TaskWorkflowHttpClientManager::new(
                client_config(&base_url, options)?,
            )
            .map_err(http_error)?,
        })
    }
    #[napi]
    pub fn inspect(&self) -> JsonValue {
        sdk::client_stats_payload(self.manager.inspect())
    }
    #[napi]
    pub fn close(&mut self) -> JsonValue {
        sdk::client_stats_payload(self.manager.close())
    }
}

#[napi]
pub const DEFAULT_REQUEST_TIMEOUT_SECONDS: f64 = sdk::DEFAULT_REQUEST_TIMEOUT_SECONDS;
#[napi]
pub const LONG_RUNNING_REQUEST_TIMEOUT_SECONDS: f64 = sdk::LONG_RUNNING_REQUEST_TIMEOUT_SECONDS;
