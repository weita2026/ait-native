//! Retained pure runtime/schema/manifest/transport and usage SDK functions.
//! Chat execution remains retired; these exports perform no chat delivery.
use crate::JsonInput;
use ait_core::{
    benchmark_usage, json_support::JsonValue, runtime_binding_state as state,
    transport_envelope as envelope, worker_manifest as manifest,
};
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn extract_codex_usage_jsonl(
    usage_jsonl_path: String,
    usage_scope: String,
) -> Result<JsonValue> {
    benchmark_usage::extract_codex_usage_jsonl(
        std::path::Path::new(&usage_jsonl_path),
        &usage_scope,
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct UsageBundleOptions {
    pub usage_roles: Option<Vec<String>>,
    pub usage_scope: Option<String>,
}
#[napi]
pub fn extract_codex_usage_bundle_jsonl(
    usage_jsonl_paths: Vec<String>,
    options: Option<UsageBundleOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    benchmark_usage::extract_codex_usage_bundle_jsonl(
        &usage_jsonl_paths,
        options.usage_roles.as_deref(),
        options.usage_scope.as_deref().unwrap_or("total"),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn project_repo_retire_runtime_blockers(payload: JsonInput) -> Result<JsonValue> {
    ait_core::server_repo_retire::project_repo_retire_runtime_blockers(&payload)
        .map_err(Error::from_reason)
}
#[napi]
pub fn language_binding_info() -> JsonValue {
    ait_agent_core::language_binding_info_json()
}
#[napi]
pub fn transport_envelope_ir_version() -> String {
    envelope::transport_envelope_ir_version().to_string()
}
#[napi]
pub fn transport_envelope_schema() -> JsonValue {
    envelope::transport_envelope_schema_json()
}
#[napi]
pub fn runtime_binding_state_ir_version() -> String {
    state::runtime_binding_state_ir_version().to_string()
}
#[napi]
pub fn runtime_binding_state_schema() -> JsonValue {
    state::runtime_binding_state_schema_json()
}
#[napi]
pub fn runtime_binding_state_default_payload() -> JsonValue {
    state::default_runtime_binding_state_payload_json()
}
#[napi]
pub fn runtime_binding_state_normalize_document(payload: JsonInput) -> JsonValue {
    state::normalize_runtime_binding_state_document_json(&payload)
}
#[napi]
pub fn worker_manifest_ir_version() -> String {
    manifest::worker_manifest_ir_version().to_string()
}
#[napi]
pub fn worker_manifest_schema() -> JsonValue {
    manifest::worker_manifest_schema_json()
}
#[napi]
pub fn worker_manifest_default_config() -> JsonValue {
    manifest::default_worker_manifest_config_json()
}
#[napi]
pub fn worker_manifest_normalize_document(payload: JsonInput, path: Option<String>) -> JsonValue {
    manifest::normalize_worker_manifest_document_json(&payload, path.as_deref())
}
#[napi]
pub fn worker_manifest_select_telegram_worker(
    config: JsonInput,
    requested_name: Option<String>,
) -> JsonValue {
    manifest::select_telegram_worker_json(&config, requested_name.as_deref())
}

#[napi(object)]
pub struct TransportBindingMetadataInput {
    pub transport: String,
    pub surface_id: String,
    pub conversation_key: String,
    pub surface_title: Option<String>,
    pub surface_kind: Option<String>,
    pub thread_id: Option<String>,
    pub reply_target: Option<JsonInput>,
    pub metadata_extra: Option<JsonInput>,
}
#[napi]
pub fn transport_envelope_build_binding_metadata(
    input: TransportBindingMetadataInput,
) -> JsonValue {
    envelope::build_transport_binding_metadata_json(
        &input.transport,
        &input.surface_id,
        input.surface_title.as_deref(),
        input.surface_kind.as_deref(),
        input.thread_id.as_deref(),
        &input.conversation_key,
        input.reply_target.as_deref(),
        input.metadata_extra.as_deref(),
    )
}
#[napi(object)]
pub struct TransportEventInput {
    pub transport: String,
    pub actor_identity: String,
    pub channel_id: String,
    pub text: String,
    pub actor_transport_id: Option<String>,
    pub actor_username: Option<String>,
    pub actor_display_name: Option<String>,
    pub actor_is_bot: Option<bool>,
    pub channel_title: Option<String>,
    pub channel_kind: Option<String>,
    pub thread_id: Option<String>,
    pub message_id: Option<JsonInput>,
    pub message_ids: Option<JsonInput>,
    pub occurred_at: Option<String>,
    pub event_id: Option<String>,
    pub dedupe_key: Option<String>,
    pub attachments: Option<JsonInput>,
    pub metadata: Option<JsonInput>,
}
#[napi]
pub fn transport_envelope_build_event_envelope(input: TransportEventInput) -> JsonValue {
    envelope::build_transport_event_envelope_json(
        &input.transport,
        &input.actor_identity,
        &input.channel_id,
        &input.text,
        input.actor_transport_id.as_deref(),
        input.actor_username.as_deref(),
        input.actor_display_name.as_deref(),
        input.actor_is_bot,
        input.channel_title.as_deref(),
        input.channel_kind.as_deref(),
        input.thread_id.as_deref(),
        input.message_id.as_deref(),
        input.message_ids.as_deref(),
        input.occurred_at.as_deref(),
        input.event_id.as_deref(),
        input.dedupe_key.as_deref(),
        input.attachments.as_deref(),
        input.metadata.as_deref(),
    )
}
#[napi(object)]
pub struct TransportReplyInput {
    pub transport: String,
    pub channel_id: String,
    pub text: String,
    pub channel_title: Option<String>,
    pub channel_kind: Option<String>,
    pub thread_id: Option<String>,
    pub delivery_kind: Option<String>,
    pub reply_to_event_id: Option<String>,
    pub reply_to_message_id: Option<JsonInput>,
    pub reply_to_message_ids: Option<JsonInput>,
    pub attachments: Option<JsonInput>,
    pub metadata: Option<JsonInput>,
}
#[napi]
pub fn transport_envelope_build_reply_envelope(input: TransportReplyInput) -> JsonValue {
    envelope::build_transport_reply_envelope_json(
        &input.transport,
        &input.channel_id,
        &input.text,
        input.channel_title.as_deref(),
        input.channel_kind.as_deref(),
        input.thread_id.as_deref(),
        Some(input.delivery_kind.as_deref().unwrap_or("chat_reply")),
        input.reply_to_event_id.as_deref(),
        input.reply_to_message_id.as_deref(),
        input.reply_to_message_ids.as_deref(),
        input.attachments.as_deref(),
        input.metadata.as_deref(),
    )
}
#[napi]
pub fn transport_envelope_compact_event_envelope(envelope: JsonInput) -> JsonValue {
    envelope::compact_transport_event_envelope_json(&envelope)
}
#[napi]
pub fn transport_envelope_compact_reply_envelope(envelope: JsonInput) -> JsonValue {
    envelope::compact_transport_reply_envelope_json(&envelope)
}
