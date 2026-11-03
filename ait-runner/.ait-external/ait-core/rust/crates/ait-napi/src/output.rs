use crate::JsonInput;
use ait_core::json_support::{JsonCodec, JsonEncodeOptions, JsonValue};
use napi::{Error, Result};
use napi_derive::napi;

macro_rules! projection {
    ($(($name:ident, $target:path)),* $(,)?) => {$(
        #[napi]
        pub fn $name(payload: JsonInput, full: Option<bool>) -> JsonValue {
            $target(&payload, full.unwrap_or(false))
        }
    )*};
}

projection! {
    (project_status, ait_cli::binding_output::project_status),
    (project_task_start, ait_cli::binding_output::project_task_start),
    (project_task_finish, ait_cli::binding_output::project_task_finish),
    (project_snapshot_capture, ait_cli::binding_output::project_snapshot_capture),
}

macro_rules! text_output {
    ($(($name:ident, $target:path)),* $(,)?) => {$(
        #[napi]
        pub fn $name(payload: JsonInput) -> Result<String> {
            $target(&payload).map_err(Error::from_reason)
        }
    )*};
}

text_output! {
    (format_task_show, ait_cli::binding_output::format_task_show),
    (format_remote_add, ait_cli::binding_output::format_remote_add),
    (format_remote_list, ait_cli::binding_output::format_remote_list),
    (format_queue_summary, ait_cli::binding_output::format_queue_summary),
    (format_doctor_memory_root, ait_cli::binding_output::format_doctor_memory_root),
    (format_doctor_runtime_root, ait_cli::binding_output::format_doctor_runtime_root),
    (format_doctor_plan_authority, ait_cli::binding_output::format_doctor_plan_authority),
    (format_configuration_show, ait_cli::binding_output::format_configuration_show),
    (format_status, ait_cli::binding_output::format_status),
    (format_task_start, ait_cli::binding_output::format_task_start),
    (format_task_quick, ait_cli::binding_output::format_task_quick),
    (format_snapshot_capture, ait_cli::binding_output::format_snapshot_capture),
}

#[napi]
pub fn project_task_list(payload: JsonInput, all: Option<bool>) -> JsonValue {
    ait_cli::binding_output::project_task_list(&payload, all.unwrap_or(false))
}

#[napi(object)]
#[derive(Default)]
pub struct TaskListOutputOptions {
    pub all: Option<bool>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn format_task_list(
    payload: JsonInput,
    options: Option<TaskListOutputOptions>,
) -> Result<String> {
    let options = options.unwrap_or_default();
    ait_cli::binding_output::format_task_list(
        &payload,
        options.all.unwrap_or(false),
        &ait_cli::binding_services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn format_initialize_repository(payload: JsonInput) -> String {
    ait_cli::binding_output::format_initialize_repository(&payload)
}

#[napi]
pub fn format_configuration(payload: JsonInput, request: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_configuration(&payload, &request).map_err(Error::from_reason)
}

#[napi]
pub fn format_configuration_unset(payload: JsonInput, key: String) -> Result<String> {
    ait_cli::binding_output::format_configuration_unset(&payload, &key).map_err(Error::from_reason)
}

#[napi]
pub fn format_task_audit(payload: JsonInput, task_id: String) -> Result<String> {
    ait_cli::binding_output::format_task_audit(&payload, &task_id).map_err(Error::from_reason)
}

#[napi]
pub fn format_task_abandon(payload: JsonInput, task_id: String) -> Result<String> {
    ait_cli::binding_output::format_task_abandon(&payload, &task_id).map_err(Error::from_reason)
}

#[napi]
pub fn format_task_finish(payload: JsonInput, requested: String) -> Result<String> {
    ait_cli::binding_output::format_task_finish(&payload, &requested).map_err(Error::from_reason)
}

#[napi]
pub fn format_domain_error(message: String) -> String {
    ait_cli::render::domain_error_text(&message)
}

#[napi]
pub fn domain_error_exit_code() -> u32 {
    u32::from(ait_cli::render::DOMAIN_ERROR_EXIT_CODE)
}

#[napi]
pub fn encode_json(value: JsonInput, pretty: Option<bool>) -> Result<String> {
    let options = if pretty.unwrap_or(false) {
        JsonEncodeOptions::pretty()
    } else {
        JsonEncodeOptions::compact()
    };
    JsonCodec::encode_value(&value, options).map_err(|error| Error::from_reason(error.to_string()))
}

#[napi]
pub fn decode_json(text: String) -> Result<JsonValue> {
    JsonCodec::parse_value_with_error_prefix(&text, "Invalid JSON")
        .map_err(|error| Error::from_reason(error.to_string()))
}
