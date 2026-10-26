use crate::repository::repository;
use crate::workspace::native_count;
use crate::JsonInput;
use ait_cli::{binding_output as output, binding_services as services};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;
use std::path::PathBuf;

#[napi(object)]
#[derive(Default)]
pub struct WorkflowReadyOptions {
    pub apply: Option<bool>,
    pub remote_name: Option<String>,
    pub expected_identity_path: Option<String>,
    pub snapshot_message: Option<String>,
    pub summary: Option<String>,
    pub tests: Option<String>,
    pub lint: Option<String>,
    pub security: Option<String>,
    pub license: Option<String>,
    pub author_mode: Option<String>,
    pub model: Option<String>,
    pub run_ci_once: Option<bool>,
}

#[napi(object)]
#[derive(Default)]
pub struct WorkflowFinishOptions {
    pub apply: Option<bool>,
    pub remote_name: Option<String>,
    pub expected_identity_path: Option<String>,
    pub reviewer: Option<String>,
    pub review_message: Option<String>,
}

#[napi(object)]
pub struct WorkflowOperationResult {
    pub payload: JsonValue,
    pub exit_code: u32,
}

impl From<services::WorkflowOperationResult> for WorkflowOperationResult {
    fn from(value: services::WorkflowOperationResult) -> Self {
        Self {
            payload: value.payload,
            exit_code: u32::from(value.exit_code),
        }
    }
}

#[napi]
pub fn workflow_ready(
    root: String,
    task_id: String,
    options: Option<WorkflowReadyOptions>,
) -> Result<WorkflowOperationResult> {
    let options = options.unwrap_or_default();
    let request = services::WorkflowReadyRequest {
        task_id,
        apply: options.apply.unwrap_or(false),
        remote_name: options.remote_name,
        expected_identity: options.expected_identity_path.map(PathBuf::from),
        snapshot_message: options.snapshot_message,
        summary: options.summary,
        tests: options.tests,
        lint: options.lint,
        security: options.security,
        license: options.license,
        author_mode: options.author_mode,
        model: options.model,
        run_ci_once: options.run_ci_once.unwrap_or(false),
    };
    services::prepare_workflow(&repository(&root)?, &request)
        .map(Into::into)
        .map_err(Error::from_reason)
}

#[napi]
pub fn workflow_finish(
    root: String,
    task_id: String,
    options: Option<WorkflowFinishOptions>,
) -> Result<WorkflowOperationResult> {
    let options = options.unwrap_or_default();
    let request = services::WorkflowFinishRequest {
        task_id,
        apply: options.apply.unwrap_or(false),
        remote_name: options.remote_name,
        expected_identity: options.expected_identity_path.map(PathBuf::from),
        reviewer: options.reviewer,
        review_message: options.review_message,
    };
    services::finish_workflow(&repository(&root)?, &request)
        .map(Into::into)
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorkflowReconcileOptions {
    pub remote_name: Option<String>,
    pub task_id: Option<String>,
    pub apply: Option<bool>,
    pub safe_only: Option<bool>,
    pub scheduled: Option<bool>,
    pub limit: Option<JsonInput>,
}

#[napi]
pub fn workflow_reconcile(
    root: String,
    options: Option<WorkflowReconcileOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let request = services::WorkflowReconcileRequest {
        remote_name: options.remote_name,
        task_id: options.task_id,
        apply: options.apply.unwrap_or(false),
        safe_only: options.safe_only.unwrap_or(false),
        scheduled: options.scheduled.unwrap_or(false),
        limit: native_count(options.limit, 100, "limit")?,
    };
    services::reconcile_workflow(&repository(&root)?, &request).map_err(Error::from_reason)
}

#[napi]
pub fn workflow_guide(topic: Option<String>) -> Result<JsonValue> {
    services::workflow_guide(topic.as_deref()).map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorkflowOutputOptions {
    pub remote_name: Option<String>,
    pub full: Option<bool>,
}

#[napi]
pub fn project_workflow_ready(
    root: String,
    payload: JsonInput,
    task_id: String,
    options: Option<WorkflowOutputOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    Ok(output::project_workflow_ready(
        &repository(&root)?,
        &payload,
        &task_id,
        options.remote_name.as_deref(),
        options.full.unwrap_or(false),
    ))
}

#[napi]
pub fn project_workflow_finish(
    root: String,
    payload: JsonInput,
    task_id: String,
    options: Option<WorkflowOutputOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    Ok(output::project_workflow_finish(
        &repository(&root)?,
        &payload,
        &task_id,
        options.remote_name.as_deref(),
        options.full.unwrap_or(false),
    ))
}

#[napi]
pub fn format_workflow_ready(payload: JsonInput, task_id: String) -> Result<String> {
    output::format_workflow_ready(&payload, &task_id).map_err(Error::from_reason)
}

#[napi]
pub fn format_workflow_finish(payload: JsonInput, task_id: String) -> Result<String> {
    output::format_workflow_finish(&payload, &task_id).map_err(Error::from_reason)
}

#[napi]
pub fn format_workflow_reconcile(payload: JsonInput) -> Result<String> {
    output::format_workflow_reconcile(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_workflow_guide(payload: JsonInput) -> Result<String> {
    output::format_workflow_guide(&payload).map_err(Error::from_reason)
}
