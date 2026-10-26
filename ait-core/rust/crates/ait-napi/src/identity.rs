//! Retained workflow identity, status and result SDK helpers.
use crate::{workspace::optional_count, JsonInput};
use ait_core::{
    json_support::{json, JsonValue},
    workflow_primitives as workflow,
};
use napi::{Error, Result, Status};
use napi_derive::napi;

fn optional_string(value: Option<String>) -> JsonValue {
    value.map(JsonValue::String).unwrap_or(JsonValue::Null)
}

fn signed_i64(value: JsonInput, name: &str) -> Result<i64> {
    value.as_i64().ok_or_else(|| {
        Error::new(
            Status::InvalidArg,
            format!("{name} must be a signed 64-bit integer"),
        )
    })
}

#[napi]
pub fn normalize_task_status(value: Option<String>) -> Result<JsonValue> {
    workflow::task_status_value(value.as_deref())
        .map(optional_string)
        .map_err(Error::from_reason)
}
#[napi]
pub fn normalize_optional_text(value: Option<String>) -> JsonValue {
    optional_string(ait_core::text_normalization::normalize_optional_text(
        value.as_deref(),
    ))
}
#[napi]
pub fn utc_now() -> Result<String> {
    ait_core::time_identity::build_plan_timestamp_payload_json("{}")
        .map_err(Error::from_reason)?
        .get("timestamp")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::from_reason("Rust utc_now did not return a timestamp."))
}
#[napi]
pub fn generate_namespaced_workflow_id(
    family: String,
    namespace_prefix: Option<String>,
) -> Result<String> {
    let payload = ait_core::time_identity::build_plan_workflow_id_payload_json(&json!({"family":family, "namespace_prefix":namespace_prefix, "timestamp_ms":null, "randomness_hex":null}).to_string()).map_err(Error::from_reason)?;
    payload
        .get("generated_id")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            Error::from_reason("Rust generate_namespaced_workflow_id did not return an id.")
        })
}
#[napi]
pub fn normalize_id_namespace_prefix(
    value: Option<String>,
    default_prefix: Option<String>,
) -> Result<String> {
    workflow::normalize_id_namespace_prefix(
        value.as_deref(),
        Some(
            default_prefix
                .as_deref()
                .unwrap_or(workflow::DEFAULT_ID_NAMESPACE_PREFIX),
        ),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorkflowIdTokensOptions {
    pub namespace_prefix: Option<String>,
    pub include_legacy: Option<bool>,
}
#[napi]
pub fn workflow_id_tokens(
    family: String,
    options: Option<WorkflowIdTokensOptions>,
) -> Result<Vec<String>> {
    let options = options.unwrap_or_default();
    workflow::workflow_id_tokens(
        &family,
        options.namespace_prefix.as_deref(),
        options.include_legacy.unwrap_or(true),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_origin_namespace_prefix(
    origin_prefix: String,
    namespace_prefix: Option<String>,
) -> Result<String> {
    workflow::workflow_origin_namespace_prefix(&origin_prefix, namespace_prefix.as_deref())
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorkflowNamespaceOptions {
    pub namespace_prefix: Option<String>,
    pub include_legacy: Option<bool>,
    pub include_task_change_origins: Option<bool>,
}
#[napi]
pub fn workflow_id_namespace_prefix_candidates(
    options: Option<WorkflowNamespaceOptions>,
) -> Result<Vec<String>> {
    let options = options.unwrap_or_default();
    workflow::workflow_id_namespace_prefix_candidates(
        options.namespace_prefix.as_deref(),
        options.include_legacy.unwrap_or(true),
        options.include_task_change_origins.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_id_namespace_prefix_for_value(
    family: String,
    value: Option<String>,
    options: Option<WorkflowNamespaceOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    workflow::workflow_id_namespace_prefix_for_value(
        value.as_deref(),
        &family,
        options.namespace_prefix.as_deref(),
        options.include_legacy.unwrap_or(true),
        options.include_task_change_origins.unwrap_or(false),
    )
    .map(optional_string)
    .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_id_matches_any_namespace_prefix(
    family: String,
    value: Option<String>,
    options: Option<WorkflowNamespaceOptions>,
) -> Result<bool> {
    let options = options.unwrap_or_default();
    workflow::workflow_id_matches_any_namespace_prefix(
        value.as_deref(),
        &family,
        options.namespace_prefix.as_deref(),
        options.include_legacy.unwrap_or(true),
        options.include_task_change_origins.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn derive_patchset_id(
    change_id: String,
    patchset_number: JsonInput,
    namespace_prefix: Option<String>,
) -> Result<String> {
    workflow::derive_patchset_id(
        &change_id,
        signed_i64(patchset_number, "patchsetNumber")?,
        namespace_prefix.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn generate_workflow_id(prefix: String) -> Result<String> {
    workflow::generate_workflow_id(&prefix).map_err(Error::from_reason)
}
#[napi]
pub fn task_status_details(value: Option<String>) -> Result<JsonValue> {
    let details = workflow::task_status_details(value.as_deref()).map_err(Error::from_reason)?;
    Ok(
        json!({"normalized_status":details.normalized_status,"display_label":details.display_label,"closed":details.closed}),
    )
}
#[napi]
pub fn normalize_publication_state(value: Option<String>) -> Result<JsonValue> {
    workflow::publication_state_value(value.as_deref())
        .map(optional_string)
        .map_err(Error::from_reason)
}
#[napi]
pub fn publication_state_has_unpublished_head(value: Option<String>) -> Result<bool> {
    workflow::publication_state_has_unpublished_head(value.as_deref()).map_err(Error::from_reason)
}
#[napi]
pub fn workflow_id_token(family: String, namespace_prefix: Option<String>) -> Result<String> {
    workflow::workflow_id_token(&family, namespace_prefix.as_deref()).map_err(Error::from_reason)
}
#[napi]
pub fn workflow_id_matches(
    family: String,
    value: Option<String>,
    options: Option<WorkflowIdTokensOptions>,
) -> Result<bool> {
    let options = options.unwrap_or_default();
    workflow::workflow_id_matches(
        value.as_deref(),
        &family,
        options.namespace_prefix.as_deref(),
        options.include_legacy.unwrap_or(true),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorkflowSequenceOptions {
    pub namespace_prefix: Option<String>,
    pub width: Option<JsonInput>,
}
#[napi]
pub fn generate_namespaced_sequence_id(
    family: String,
    number: JsonInput,
    options: Option<WorkflowSequenceOptions>,
) -> Result<String> {
    let options = options.unwrap_or_default();
    workflow::generate_namespaced_sequence_id(
        &family,
        signed_i64(number, "number")?,
        options.namespace_prefix.as_deref(),
        optional_count(options.width, "width")?.unwrap_or(4),
    )
    .map_err(Error::from_reason)
}

fn result_envelope(envelope: workflow::WorkflowResultEnvelope) -> JsonValue {
    let error = envelope
        .error
        .map(|error| json!({"code":error.code,"message":error.message,"detail":error.detail}))
        .unwrap_or(JsonValue::Null);
    json!({"ok":envelope.ok,"kind":envelope.kind,"value":envelope.value,"error":error})
}
#[napi]
pub fn workflow_success_envelope(kind: String, value: Option<String>) -> Result<JsonValue> {
    workflow::workflow_success_envelope(&kind, value.as_deref())
        .map(result_envelope)
        .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_error_envelope(
    kind: String,
    code: String,
    message: String,
    detail: Option<String>,
) -> Result<JsonValue> {
    workflow::workflow_error_envelope(&kind, &code, &message, detail.as_deref())
        .map(result_envelope)
        .map_err(Error::from_reason)
}
