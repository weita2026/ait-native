use crate::{repository::repository, workflows::WorkflowOperationResult, JsonInput};
use ait_cli::{binding_external as service, external_surface as external};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct ExternalUpdateOptions {
    pub name: Option<String>,
    pub snapshot: Option<String>,
    pub latest: Option<bool>,
    pub locked: Option<bool>,
    pub validate: Option<bool>,
    pub no_recursive: Option<bool>,
}
#[napi]
pub fn external_status(root: String) -> Result<JsonValue> {
    external::external_status(&repository(&root)?).map_err(Error::from_reason)
}
#[napi]
pub fn external_doctor(
    root: String,
    fail_on_blocking: Option<bool>,
) -> Result<WorkflowOperationResult> {
    service::diagnose_external(&repository(&root)?, fail_on_blocking.unwrap_or(false))
        .map(Into::into)
        .map_err(Error::from_reason)
}
#[napi]
pub fn external_update(root: String, options: Option<ExternalUpdateOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::update_external(
        &repository(&root)?,
        &service::ExternalUpdateRequest {
            name: options.name,
            snapshot: options.snapshot,
            latest: options.latest.unwrap_or(false),
            locked: options.locked.unwrap_or(false),
            validate: options.validate.unwrap_or(false),
            no_recursive: options.no_recursive.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn external_link(root: String, name: String, path: String) -> Result<JsonValue> {
    external::external_link(&repository(&root)?, &name, &path).map_err(Error::from_reason)
}
#[napi]
pub fn external_unlink(root: String, name: String) -> Result<JsonValue> {
    external::external_unlink(&repository(&root)?, &name).map_err(Error::from_reason)
}
#[napi]
pub fn format_external_status(payload: JsonInput) -> Result<String> {
    external::render_external_status_text(&payload).map_err(Error::from_reason)
}
#[napi]
pub fn format_external_doctor(payload: JsonInput) -> Result<String> {
    external::render_external_doctor_text(&payload).map_err(Error::from_reason)
}
#[napi]
pub fn format_external_update(payload: JsonInput) -> Result<String> {
    external::render_external_update_text(&payload).map_err(Error::from_reason)
}
#[napi]
pub fn format_external_link(payload: JsonInput) -> Result<String> {
    external::render_external_link_text(&payload).map_err(Error::from_reason)
}
#[napi]
pub fn format_external_unlink(payload: JsonInput) -> Result<String> {
    external::render_external_unlink_text(&payload).map_err(Error::from_reason)
}
