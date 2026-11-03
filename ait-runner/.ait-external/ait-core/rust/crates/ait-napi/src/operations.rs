use super::repository::{init_request, repository, InitOptions};
use crate::JsonInput;
use ait_cli::binding_services as services;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;
use std::path::PathBuf;

#[napi]
pub fn unset_configuration(root: String, key: String) -> Result<JsonValue> {
    ait_cli::config_surface::config_unset_from_key(&repository(&root)?, &key)
        .map_err(Error::from_reason)
}

#[napi]
pub fn task_audit(
    root: String,
    task_id: String,
    options: Option<super::repository::ScopeOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    services::audit_task(
        &repository(&root)?,
        &task_id,
        &services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_abandon(
    root: String,
    task_id: String,
    options: Option<super::repository::ScopeOptions>,
) -> Result<super::workflows::WorkflowOperationResult> {
    let options = options.unwrap_or_default();
    services::abandon_task(
        &repository(&root)?,
        &task_id,
        &services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
    )
    .map(Into::into)
    .map_err(Error::from_reason)
}

#[napi(object)]
pub struct TaskStartRequest {
    pub intent: String,
    pub title: Option<String>,
    pub source: Option<String>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub edit_root: Option<String>,
    pub human_progress: Option<bool>,
}

#[napi(object)]
pub struct TaskQuickRequest {
    pub intent: String,
    pub title: Option<String>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub edit_root: Option<String>,
    pub human_progress: Option<bool>,
}

#[napi(object)]
#[derive(Default)]
pub struct TaskFinishOptions {
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub message: Option<String>,
    pub sync_plan_markdown: Option<bool>,
}

#[napi(object)]
#[derive(Default)]
pub struct SnapshotCaptureOptions {
    pub message: Option<String>,
    pub resume_snapshot_id: Option<String>,
}

#[napi]
pub fn initialize_repository(root: String, options: Option<InitOptions>) -> Result<JsonValue> {
    services::initialize_repository(&init_request(root, options)).map_err(Error::from_reason)
}

#[napi]
pub fn configure_repository(root: String, payload: JsonInput) -> Result<JsonValue> {
    ait_cli::config_surface::config_set_from_payload_with_team_bootstrap(
        &repository(&root)?,
        &payload,
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn configuration_error(payload: JsonInput) -> Option<String> {
    ait_cli::config_surface::team_setup_error(&payload)
}

#[napi]
pub fn task_start(root: String, request: TaskStartRequest) -> Result<JsonValue> {
    let mut progress = services::emit_task_start_progress;
    services::start_task(
        &repository(&root)?,
        &services::TaskStartRequest {
            title: request.title,
            source: request.source,
            intent: request.intent,
            local: request.local.unwrap_or(false),
            remote_name: request.remote_name,
            edit_root: request.edit_root.map(PathBuf::from),
        },
        request
            .human_progress
            .unwrap_or(false)
            .then_some(&mut progress),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_quick(root: String, request: TaskQuickRequest) -> Result<JsonValue> {
    let mut progress = services::emit_task_start_progress;
    services::quick_task(
        &repository(&root)?,
        &services::TaskQuickRequest {
            title: request.title,
            intent: request.intent,
            local: request.local.unwrap_or(false),
            remote_name: request.remote_name,
            edit_root: request.edit_root.map(PathBuf::from),
        },
        request
            .human_progress
            .unwrap_or(false)
            .then_some(&mut progress),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_finish(
    root: String,
    task_id: String,
    options: Option<TaskFinishOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    services::finish_task(
        &repository(&root)?,
        &services::TaskFinishRequest {
            task_id,
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
            message: options.message,
            sync_plan_markdown: options.sync_plan_markdown.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_finish_exit_code(payload: JsonInput) -> u32 {
    u32::from(ait_cli::post_finish_hooks::task_finish_exit_code(&payload))
}

#[napi]
pub fn snapshot_capture(
    root: String,
    reference: String,
    options: Option<SnapshotCaptureOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    services::capture_snapshot(
        &repository(&root)?,
        &services::SnapshotCaptureRequest {
            reference,
            message: options.message,
            resume_snapshot_id: options.resume_snapshot_id,
        },
    )
    .map_err(Error::from_reason)
}
