use ait_core::json_support::JsonValue;
use ait_core::transport_envelope::{
    build_transport_binding_metadata_json, build_transport_event_envelope_json,
    build_transport_reply_envelope_json, compact_transport_event_envelope_json,
    compact_transport_reply_envelope_json, transport_envelope_ir_version,
    transport_envelope_schema_json,
};

pub fn agent_transport_envelope_ir_version() -> &'static str {
    transport_envelope_ir_version()
}

pub fn agent_transport_envelope_schema_json() -> JsonValue {
    transport_envelope_schema_json()
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the shared transport binding metadata contract"
)]
pub fn agent_transport_binding_metadata_json(
    transport: &str,
    surface_id: &str,
    surface_title: Option<&str>,
    surface_kind: Option<&str>,
    thread_id: Option<&str>,
    conversation_key: &str,
    reply_target: Option<&JsonValue>,
    metadata_extra: Option<&JsonValue>,
) -> JsonValue {
    build_transport_binding_metadata_json(
        transport,
        surface_id,
        surface_title,
        surface_kind,
        thread_id,
        conversation_key,
        reply_target,
        metadata_extra,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn agent_transport_event_envelope_json(
    transport: &str,
    actor_identity: &str,
    channel_id: &str,
    text: &str,
    actor_transport_id: Option<&str>,
    actor_username: Option<&str>,
    actor_display_name: Option<&str>,
    actor_is_bot: Option<bool>,
    channel_title: Option<&str>,
    channel_kind: Option<&str>,
    thread_id: Option<&str>,
    message_id: Option<&JsonValue>,
    message_ids: Option<&JsonValue>,
    occurred_at: Option<&str>,
    event_id: Option<&str>,
    dedupe_key: Option<&str>,
    attachments: Option<&JsonValue>,
    metadata: Option<&JsonValue>,
) -> JsonValue {
    build_transport_event_envelope_json(
        transport,
        actor_identity,
        channel_id,
        text,
        actor_transport_id,
        actor_username,
        actor_display_name,
        actor_is_bot,
        channel_title,
        channel_kind,
        thread_id,
        message_id,
        message_ids,
        occurred_at,
        event_id,
        dedupe_key,
        attachments,
        metadata,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn agent_transport_reply_envelope_json(
    transport: &str,
    channel_id: &str,
    text: &str,
    channel_title: Option<&str>,
    channel_kind: Option<&str>,
    thread_id: Option<&str>,
    delivery_kind: Option<&str>,
    reply_to_event_id: Option<&str>,
    reply_to_message_id: Option<&JsonValue>,
    reply_to_message_ids: Option<&JsonValue>,
    attachments: Option<&JsonValue>,
    metadata: Option<&JsonValue>,
) -> JsonValue {
    build_transport_reply_envelope_json(
        transport,
        channel_id,
        text,
        channel_title,
        channel_kind,
        thread_id,
        delivery_kind,
        reply_to_event_id,
        reply_to_message_id,
        reply_to_message_ids,
        attachments,
        metadata,
    )
}

pub fn agent_compact_transport_event_envelope_json(envelope: &JsonValue) -> JsonValue {
    compact_transport_event_envelope_json(envelope)
}

pub fn agent_compact_transport_reply_envelope_json(envelope: &JsonValue) -> JsonValue {
    compact_transport_reply_envelope_json(envelope)
}

#[cfg(test)]
mod tests;
