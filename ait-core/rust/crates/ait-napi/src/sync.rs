use crate::{repository::repository, JsonInput};
use ait_cli::{binding_services as service, primitives};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct RemoteSyncOptions {
    pub remote_name: Option<String>,
    pub line_name: Option<String>,
}

#[napi(object)]
#[derive(Default)]
pub struct PullOptions {
    pub remote_name: Option<String>,
    pub line_name: Option<String>,
    pub merge: Option<bool>,
    pub restore: Option<bool>,
    pub force: Option<bool>,
}

#[napi(object)]
#[derive(Default)]
pub struct SnapshotUploadOptions {
    pub remote_name: Option<String>,
    pub line_name: Option<String>,
    pub reason: Option<String>,
}

#[napi]
pub fn pull_repository(root: String, options: Option<PullOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::pull_repository(
        &repository(&root)?,
        &service::PullRequest {
            remote_name: options.remote_name,
            line_name: options.line_name,
            merge: options.merge.unwrap_or(false),
            restore: options.restore.unwrap_or(false),
            force: options.force.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn push_repository(root: String, options: Option<RemoteSyncOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::push_repository(
        &repository(&root)?,
        &service::RemoteSyncRequest {
            remote_name: options.remote_name,
            line_name: options.line_name,
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_pull(root: String, options: Option<PullOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::pull(
        &repository(&root)?,
        options.remote_name.as_deref(),
        options.line_name.as_deref(),
        options.merge.unwrap_or(false),
        options.restore.unwrap_or(false),
        options.force.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_push(root: String, options: Option<RemoteSyncOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::push(
        &repository(&root)?,
        options.remote_name.as_deref(),
        options.line_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_upload_snapshot_chain(
    root: String,
    snapshot_id: String,
    options: Option<SnapshotUploadOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::upload_snapshot_chain(
        &repository(&root)?,
        options.remote_name.as_deref(),
        &snapshot_id,
        options.line_name.as_deref(),
        options.reason.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn project_remote_sync(payload: JsonInput) -> JsonValue {
    ait_cli::binding_output::project_remote_sync(&payload)
}

#[napi]
pub fn format_pull(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_pull(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_push(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_push(&payload).map_err(Error::from_reason)
}
