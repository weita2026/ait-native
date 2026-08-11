use crate::json_support::{json, JsonMap as Map, JsonNumber as Number, JsonValue};

use crate::json_support::{required_array_value, required_object_value};

pub const TRANSPORT_ENVELOPE_IR_VERSION: &str = "ait.transport_envelope.v2";

fn clean_optional_str(value: Option<&str>) -> Option<String> {
    let text = value.unwrap_or("").trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn compact_json_value(value: JsonValue) -> Option<JsonValue> {
    match value {
        JsonValue::Null => None,
        JsonValue::Object(map) => {
            let mut compact = Map::new();
            for (key, entry) in map {
                if let Some(compacted) = compact_json_value(entry) {
                    compact.insert(key, compacted);
                }
            }
            if compact.is_empty() {
                None
            } else {
                Some(JsonValue::Object(compact))
            }
        }
        JsonValue::Array(values) => {
            if values.is_empty() {
                None
            } else {
                Some(JsonValue::Array(values))
            }
        }
        other => Some(other),
    }
}

fn compact_object(map: Map<String, JsonValue>) -> Map<String, JsonValue> {
    match compact_json_value(JsonValue::Object(map)) {
        Some(JsonValue::Object(compact)) => compact,
        _ => Map::new(),
    }
}

fn json_string(value: String) -> JsonValue {
    JsonValue::String(value)
}

fn json_optional_string(value: Option<String>) -> JsonValue {
    match value {
        Some(text) => JsonValue::String(text),
        None => JsonValue::Null,
    }
}

fn json_optional_i64(value: Option<i64>) -> JsonValue {
    match value {
        Some(number) => JsonValue::Number(Number::from(number)),
        None => JsonValue::Null,
    }
}

fn json_optional_bool(value: Option<bool>) -> JsonValue {
    match value {
        Some(flag) => JsonValue::Bool(flag),
        None => JsonValue::Null,
    }
}

fn json_i64(value: i64) -> JsonValue {
    JsonValue::Number(Number::from(value))
}

fn py_style_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::Null => None,
        JsonValue::String(text) => clean_optional_str(Some(text)),
        JsonValue::Bool(true) => Some("True".to_string()),
        JsonValue::Bool(false) => Some("False".to_string()),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Array(_) | JsonValue::Object(_) => Some(value.to_string()),
    }
}

fn normalize_positive_int(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => {
            if let Some(parsed) = number.as_i64() {
                return (parsed > 0).then_some(parsed);
            }
            if let Some(parsed) = number.as_u64() {
                return i64::try_from(parsed).ok().filter(|value| *value > 0);
            }
            None
        }
        JsonValue::String(text) => text.trim().parse::<i64>().ok().filter(|value| *value > 0),
        _ => None,
    }
}

fn normalize_message_ids_value(value: Option<&JsonValue>) -> Vec<i64> {
    let mut normalized = Vec::new();
    if let Some(values) = value.and_then(|value| required_array_value(value, "message_ids").ok()) {
        for entry in values {
            if let Some(parsed) = normalize_positive_int(entry) {
                if !normalized.contains(&parsed) {
                    normalized.push(parsed);
                }
            }
        }
    }
    normalized
}

fn normalize_attachment_from_value(value: &JsonValue) -> Option<Map<String, JsonValue>> {
    let source = required_object_value(value, "attachment").ok()?;
    let mut payload = Map::new();
    payload.insert(
        "kind".to_string(),
        json_optional_string(py_style_string(
            source.get("kind").unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "media_kind".to_string(),
        json_optional_string(py_style_string(
            source.get("media_kind").unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "telegram_file_id".to_string(),
        json_optional_string(py_style_string(
            source
                .get("telegram_file_id")
                .or_else(|| source.get("file_id"))
                .unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "telegram_file_unique_id".to_string(),
        json_optional_string(py_style_string(
            source
                .get("telegram_file_unique_id")
                .or_else(|| source.get("file_unique_id"))
                .unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "file_name".to_string(),
        json_optional_string(py_style_string(
            source.get("file_name").unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "mime_type".to_string(),
        json_optional_string(py_style_string(
            source.get("mime_type").unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "caption".to_string(),
        json_optional_string(py_style_string(
            source.get("caption").unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "title".to_string(),
        json_optional_string(py_style_string(
            source.get("title").unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "performer".to_string(),
        json_optional_string(py_style_string(
            source.get("performer").unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "duration_seconds".to_string(),
        json_optional_i64(
            source
                .get("duration_seconds")
                .and_then(normalize_positive_int)
                .or_else(|| source.get("duration").and_then(normalize_positive_int)),
        ),
    );
    payload.insert(
        "file_size_bytes".to_string(),
        json_optional_i64(
            source
                .get("file_size_bytes")
                .and_then(normalize_positive_int)
                .or_else(|| source.get("file_size").and_then(normalize_positive_int)),
        ),
    );
    payload.insert(
        "telegram_file_path".to_string(),
        json_optional_string(py_style_string(
            source.get("telegram_file_path").unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "local_path".to_string(),
        json_optional_string(py_style_string(
            source
                .get("local_path")
                .or_else(|| source.get("path"))
                .unwrap_or(&JsonValue::Null),
        )),
    );
    payload.insert(
        "url".to_string(),
        json_optional_string(py_style_string(
            source.get("url").unwrap_or(&JsonValue::Null),
        )),
    );
    let mut compact = compact_object(payload);
    if compact.is_empty() {
        return None;
    }
    let normalized_kind = compact
        .get("kind")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            compact
                .get("media_kind")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or("file")
        .to_lowercase();
    compact.insert("kind".to_string(), json_string(normalized_kind));
    Some(compact)
}

fn normalize_attachments_value(value: Option<&JsonValue>) -> Vec<JsonValue> {
    let mut normalized = Vec::new();
    if let Some(values) = value.and_then(|value| required_array_value(value, "attachments").ok()) {
        for entry in values {
            if let Some(attachment) = normalize_attachment_from_value(entry) {
                normalized.push(JsonValue::Object(attachment));
            }
        }
    }
    normalized
}

fn object_or_empty(value: Option<&JsonValue>) -> Map<String, JsonValue> {
    value
        .and_then(|value| required_object_value(value, "transport envelope object").ok())
        .cloned()
        .unwrap_or_default()
}

fn message_label(message_ids: &[i64], fallback: &str) -> String {
    if message_ids.is_empty() {
        fallback.to_string()
    } else {
        message_ids
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join("-")
    }
}

pub struct TransportEnvelopeJson<S> {
    _store: S,
}

impl<S> TransportEnvelopeJson<S> {
    pub fn new(store: S) -> Self {
        Self { _store: store }
    }

    pub fn ir_version(&self) -> &'static str {
        TRANSPORT_ENVELOPE_IR_VERSION
    }

    pub fn schema_json(&self) -> JsonValue {
        transport_envelope_schema_json_value()
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "arguments mirror the transport binding metadata schema"
    )]
    pub fn binding_metadata_json(
        &self,
        transport: &str,
        surface_id: &str,
        surface_title: Option<&str>,
        surface_kind: Option<&str>,
        thread_id: Option<&str>,
        conversation_key: &str,
        reply_target: Option<&JsonValue>,
        metadata_extra: Option<&JsonValue>,
    ) -> JsonValue {
        build_transport_binding_metadata_json_value(
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
    pub fn event_envelope_json(
        &self,
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
        build_transport_event_envelope_json_value(
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
    pub fn reply_envelope_json(
        &self,
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
        build_transport_reply_envelope_json_value(
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

    pub fn compact_event_envelope_json(&self, envelope: &JsonValue) -> JsonValue {
        compact_transport_event_envelope_json_value(envelope)
    }

    pub fn compact_reply_envelope_json(&self, envelope: &JsonValue) -> JsonValue {
        compact_transport_reply_envelope_json_value(envelope)
    }
}

impl TransportEnvelopeJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

pub fn transport_envelope_ir_version() -> &'static str {
    TransportEnvelopeJson::stateless().ir_version()
}

pub fn transport_envelope_schema_json() -> JsonValue {
    TransportEnvelopeJson::stateless().schema_json()
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the transport binding metadata schema"
)]
pub fn build_transport_binding_metadata_json(
    transport: &str,
    surface_id: &str,
    surface_title: Option<&str>,
    surface_kind: Option<&str>,
    thread_id: Option<&str>,
    conversation_key: &str,
    reply_target: Option<&JsonValue>,
    metadata_extra: Option<&JsonValue>,
) -> JsonValue {
    TransportEnvelopeJson::stateless().binding_metadata_json(
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
pub fn build_transport_event_envelope_json(
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
    TransportEnvelopeJson::stateless().event_envelope_json(
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
pub fn build_transport_reply_envelope_json(
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
    TransportEnvelopeJson::stateless().reply_envelope_json(
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

pub fn compact_transport_event_envelope_json(envelope: &JsonValue) -> JsonValue {
    TransportEnvelopeJson::stateless().compact_event_envelope_json(envelope)
}

pub fn compact_transport_reply_envelope_json(envelope: &JsonValue) -> JsonValue {
    TransportEnvelopeJson::stateless().compact_reply_envelope_json(envelope)
}

fn transport_envelope_schema_json_value() -> JsonValue {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://ait.dev/schema/ait.transport_envelope.v2.schema.json",
        "title": "AitTransportEnvelopeSchema",
        "type": "object",
        "additionalProperties": false,
        "required": ["ir_version", "binding_metadata", "event_envelope", "reply_envelope"],
        "properties": {
            "ir_version": {"const": TRANSPORT_ENVELOPE_IR_VERSION},
            "binding_metadata": {"$ref": "#/$defs/bindingMetadata"},
            "event_envelope": {"$ref": "#/$defs/eventEnvelope"},
            "reply_envelope": {"$ref": "#/$defs/replyEnvelope"}
        },
        "$defs": {
            "attachment": {
                "type": "object",
                "additionalProperties": false,
                "required": ["kind"],
                "properties": {
                    "kind": {"type": "string", "minLength": 1},
                    "media_kind": {"type": "string"},
                    "telegram_file_id": {"type": "string"},
                    "telegram_file_unique_id": {"type": "string"},
                    "file_name": {"type": "string"},
                    "mime_type": {"type": "string"},
                    "caption": {"type": "string"},
                    "title": {"type": "string"},
                    "performer": {"type": "string"},
                    "duration_seconds": {"type": "integer", "minimum": 1},
                    "file_size_bytes": {"type": "integer", "minimum": 1},
                    "telegram_file_path": {"type": "string"},
                    "local_path": {"type": "string"},
                    "url": {"type": "string"}
                }
            },
            "bindingMetadata": {
                "type": "object",
                "required": ["transport", "surface_id", "conversation_key"],
                "properties": {
                    "transport": {"type": "string", "minLength": 1},
                    "surface_id": {"type": "string", "minLength": 1},
                    "surface_title": {"type": "string"},
                    "surface_kind": {"type": "string"},
                    "thread_id": {"type": "string"},
                    "conversation_key": {"type": "string", "minLength": 1},
                    "transport_reply_target": {"type": "object"}
                },
                "additionalProperties": true
            },
            "eventEnvelope": {
                "type": "object",
                "additionalProperties": false,
                "required": ["schema_version", "transport", "event_kind", "event_id", "dedupe_key", "actor", "channel", "message"],
                "properties": {
                    "schema_version": {"const": 1},
                    "transport": {"type": "string", "minLength": 1},
                    "event_kind": {"type": "string", "minLength": 1},
                    "event_id": {"type": "string", "minLength": 1},
                    "dedupe_key": {"type": "string", "minLength": 1},
                    "occurred_at": {"type": "string"},
                    "actor": {
                        "type": "object",
                        "required": ["actor_identity"],
                        "properties": {
                            "actor_identity": {"type": "string", "minLength": 1},
                            "transport_user_id": {"type": "string"},
                            "username": {"type": "string"},
                            "display_name": {"type": "string"},
                            "is_bot": {"type": "boolean"}
                        },
                        "additionalProperties": false
                    },
                    "channel": {
                        "type": "object",
                        "required": ["channel_id"],
                        "properties": {
                            "channel_id": {"type": "string", "minLength": 1},
                            "channel_title": {"type": "string"},
                            "channel_kind": {"type": "string"},
                            "thread_id": {"type": "string"}
                        },
                        "additionalProperties": false
                    },
                    "message": {
                        "type": "object",
                        "required": ["text", "logical_turn_message_count"],
                        "properties": {
                            "text": {"type": "string"},
                            "message_id": {"type": "integer", "minimum": 1},
                            "message_ids": {"type": "array", "items": {"type": "integer", "minimum": 1}},
                            "logical_turn_message_count": {"type": "integer", "minimum": 1},
                            "attachments": {"type": "array", "items": {"$ref": "#/$defs/attachment"}}
                        },
                        "additionalProperties": false
                    },
                    "metadata": {"type": "object"}
                }
            },
            "replyEnvelope": {
                "type": "object",
                "additionalProperties": false,
                "required": ["schema_version", "transport", "delivery_kind", "target", "reply_to", "message"],
                "properties": {
                    "schema_version": {"const": 1},
                    "transport": {"type": "string", "minLength": 1},
                    "delivery_kind": {"type": "string", "minLength": 1},
                    "target": {
                        "type": "object",
                        "required": ["channel_id"],
                        "properties": {
                            "channel_id": {"type": "string", "minLength": 1},
                            "channel_title": {"type": "string"},
                            "channel_kind": {"type": "string"},
                            "thread_id": {"type": "string"}
                        },
                        "additionalProperties": false
                    },
                    "reply_to": {
                        "type": "object",
                        "required": ["event_id", "message_id", "message_ids", "logical_turn_message_count"],
                        "properties": {
                            "event_id": {"type": "string"},
                            "message_id": {"type": "integer", "minimum": 1},
                            "message_ids": {"type": "array", "items": {"type": "integer", "minimum": 1}},
                            "logical_turn_message_count": {"type": "integer", "minimum": 1}
                        },
                        "additionalProperties": false
                    },
                    "message": {
                        "type": "object",
                        "required": ["text"],
                        "properties": {
                            "text": {"type": "string"},
                            "attachments": {"type": "array", "items": {"$ref": "#/$defs/attachment"}}
                        },
                        "additionalProperties": false
                    },
                    "metadata": {"type": "object"}
                }
            }
        }
    })
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the transport binding metadata schema"
)]
fn build_transport_binding_metadata_json_value(
    transport: &str,
    surface_id: &str,
    surface_title: Option<&str>,
    surface_kind: Option<&str>,
    thread_id: Option<&str>,
    conversation_key: &str,
    reply_target: Option<&JsonValue>,
    metadata_extra: Option<&JsonValue>,
) -> JsonValue {
    let normalized_transport =
        clean_optional_str(Some(transport)).unwrap_or_else(|| "unknown".to_string());
    let normalized_surface_id =
        clean_optional_str(Some(surface_id)).unwrap_or_else(|| "unknown".to_string());
    let normalized_conversation_key = clean_optional_str(Some(conversation_key))
        .unwrap_or_else(|| format!("{normalized_transport}:{normalized_surface_id}"));
    let mut payload = object_or_empty(metadata_extra);
    payload.insert("transport".to_string(), json_string(normalized_transport));
    payload.insert("surface_id".to_string(), json_string(normalized_surface_id));
    payload.insert(
        "surface_title".to_string(),
        json_optional_string(clean_optional_str(surface_title)),
    );
    payload.insert(
        "surface_kind".to_string(),
        json_optional_string(clean_optional_str(surface_kind)),
    );
    payload.insert(
        "thread_id".to_string(),
        json_optional_string(clean_optional_str(thread_id)),
    );
    payload.insert(
        "conversation_key".to_string(),
        json_string(normalized_conversation_key),
    );
    let compact_reply_target = compact_object(object_or_empty(reply_target));
    if !compact_reply_target.is_empty() {
        payload.insert(
            "transport_reply_target".to_string(),
            JsonValue::Object(compact_reply_target),
        );
    }
    JsonValue::Object(compact_object(payload))
}

#[allow(clippy::too_many_arguments)]
fn build_transport_event_envelope_json_value(
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
    let normalized_transport =
        clean_optional_str(Some(transport)).unwrap_or_else(|| "unknown".to_string());
    let normalized_message_id = message_id.and_then(normalize_positive_int);
    let mut normalized_message_ids = normalize_message_ids_value(message_ids);
    if let Some(value) = normalized_message_id {
        if !normalized_message_ids.contains(&value) {
            normalized_message_ids.push(value);
        }
    }
    let primary_message_id =
        normalized_message_id.or_else(|| normalized_message_ids.last().copied());
    let normalized_attachments = normalize_attachments_value(attachments);
    let label = message_label(&normalized_message_ids, "event");
    let channel_key = channel_id.to_string();

    let mut actor = Map::new();
    actor.insert(
        "actor_identity".to_string(),
        json_optional_string(clean_optional_str(Some(actor_identity))),
    );
    actor.insert(
        "transport_user_id".to_string(),
        json_optional_string(clean_optional_str(actor_transport_id)),
    );
    actor.insert(
        "username".to_string(),
        json_optional_string(clean_optional_str(actor_username)),
    );
    actor.insert(
        "display_name".to_string(),
        json_optional_string(clean_optional_str(actor_display_name)),
    );
    actor.insert("is_bot".to_string(), json_optional_bool(actor_is_bot));

    let mut channel = Map::new();
    channel.insert("channel_id".to_string(), json_string(channel_key.clone()));
    channel.insert(
        "channel_title".to_string(),
        json_optional_string(clean_optional_str(channel_title)),
    );
    channel.insert(
        "channel_kind".to_string(),
        json_optional_string(clean_optional_str(channel_kind)),
    );
    channel.insert(
        "thread_id".to_string(),
        json_optional_string(clean_optional_str(thread_id)),
    );

    let mut message = Map::new();
    message.insert("text".to_string(), json_string(text.to_string()));
    message.insert(
        "message_id".to_string(),
        json_optional_i64(primary_message_id),
    );
    if !normalized_message_ids.is_empty() {
        message.insert(
            "message_ids".to_string(),
            JsonValue::Array(
                normalized_message_ids
                    .iter()
                    .copied()
                    .map(json_i64)
                    .collect(),
            ),
        );
    }
    message.insert(
        "logical_turn_message_count".to_string(),
        json_i64(if normalized_message_ids.is_empty() {
            1
        } else {
            normalized_message_ids.len() as i64
        }),
    );
    if !normalized_attachments.is_empty() {
        message.insert(
            "attachments".to_string(),
            JsonValue::Array(normalized_attachments),
        );
    }

    let mut payload = Map::new();
    payload.insert("schema_version".to_string(), json_i64(1));
    payload.insert(
        "transport".to_string(),
        json_string(normalized_transport.clone()),
    );
    payload.insert("event_kind".to_string(), json_string("message".to_string()));
    payload.insert(
        "event_id".to_string(),
        json_string(
            clean_optional_str(event_id)
                .unwrap_or_else(|| format!("{normalized_transport}:{channel_key}:message:{label}")),
        ),
    );
    payload.insert(
        "dedupe_key".to_string(),
        json_string(
            clean_optional_str(dedupe_key)
                .unwrap_or_else(|| format!("{normalized_transport}:{channel_key}:message:{label}")),
        ),
    );
    payload.insert(
        "occurred_at".to_string(),
        json_optional_string(clean_optional_str(occurred_at)),
    );
    payload.insert("actor".to_string(), JsonValue::Object(actor));
    payload.insert("channel".to_string(), JsonValue::Object(channel));
    payload.insert("message".to_string(), JsonValue::Object(message));
    let compact_metadata = compact_object(object_or_empty(metadata));
    if !compact_metadata.is_empty() {
        payload.insert("metadata".to_string(), JsonValue::Object(compact_metadata));
    }
    JsonValue::Object(compact_object(payload))
}

#[allow(clippy::too_many_arguments)]
fn build_transport_reply_envelope_json_value(
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
    let normalized_transport =
        clean_optional_str(Some(transport)).unwrap_or_else(|| "unknown".to_string());
    let normalized_reply_to_message_id = reply_to_message_id.and_then(normalize_positive_int);
    let mut normalized_reply_to_message_ids = normalize_message_ids_value(reply_to_message_ids);
    if let Some(value) = normalized_reply_to_message_id {
        if !normalized_reply_to_message_ids.contains(&value) {
            normalized_reply_to_message_ids.push(value);
        }
    }
    let normalized_attachments = normalize_attachments_value(attachments);

    let mut target = Map::new();
    target.insert(
        "channel_id".to_string(),
        json_string(channel_id.to_string()),
    );
    target.insert(
        "channel_title".to_string(),
        json_optional_string(clean_optional_str(channel_title)),
    );
    target.insert(
        "channel_kind".to_string(),
        json_optional_string(clean_optional_str(channel_kind)),
    );
    target.insert(
        "thread_id".to_string(),
        json_optional_string(clean_optional_str(thread_id)),
    );

    let mut reply_to = Map::new();
    reply_to.insert(
        "event_id".to_string(),
        json_optional_string(clean_optional_str(reply_to_event_id)),
    );
    reply_to.insert(
        "message_id".to_string(),
        json_optional_i64(normalized_reply_to_message_id),
    );
    if !normalized_reply_to_message_ids.is_empty() {
        reply_to.insert(
            "message_ids".to_string(),
            JsonValue::Array(
                normalized_reply_to_message_ids
                    .iter()
                    .copied()
                    .map(json_i64)
                    .collect(),
            ),
        );
    }
    reply_to.insert(
        "logical_turn_message_count".to_string(),
        json_optional_i64(
            (!normalized_reply_to_message_ids.is_empty())
                .then_some(normalized_reply_to_message_ids.len() as i64),
        ),
    );

    let mut message = Map::new();
    message.insert("text".to_string(), json_string(text.to_string()));
    if !normalized_attachments.is_empty() {
        message.insert(
            "attachments".to_string(),
            JsonValue::Array(normalized_attachments),
        );
    }

    let mut payload = Map::new();
    payload.insert("schema_version".to_string(), json_i64(1));
    payload.insert("transport".to_string(), json_string(normalized_transport));
    payload.insert(
        "delivery_kind".to_string(),
        json_string(clean_optional_str(delivery_kind).unwrap_or_else(|| "chat_reply".to_string())),
    );
    payload.insert("target".to_string(), JsonValue::Object(target));
    payload.insert("reply_to".to_string(), JsonValue::Object(reply_to));
    payload.insert("message".to_string(), JsonValue::Object(message));
    let compact_metadata = compact_object(object_or_empty(metadata));
    if !compact_metadata.is_empty() {
        payload.insert("metadata".to_string(), JsonValue::Object(compact_metadata));
    }
    JsonValue::Object(compact_object(payload))
}

fn compact_transport_event_envelope_json_value(envelope: &JsonValue) -> JsonValue {
    let JsonValue::Object(source) = envelope else {
        return JsonValue::Null;
    };
    let null = JsonValue::Null;
    let channel = object_or_empty(source.get("channel"));
    let message = object_or_empty(source.get("message"));
    let message_id = message.get("message_id").and_then(normalize_positive_int);
    let mut message_ids = normalize_message_ids_value(message.get("message_ids"));
    if let Some(value) = message_id {
        if !message_ids.contains(&value) {
            message_ids.push(value);
        }
    }
    let logical_turn_message_count = if message_ids.is_empty() {
        message
            .get("logical_turn_message_count")
            .and_then(normalize_positive_int)
    } else {
        Some(message_ids.len() as i64)
    };
    let attachments = normalize_attachments_value(message.get("attachments"));

    let mut channel_payload = Map::new();
    channel_payload.insert(
        "channel_id".to_string(),
        json_optional_string(py_style_string(channel.get("channel_id").unwrap_or(&null))),
    );
    channel_payload.insert(
        "thread_id".to_string(),
        json_optional_string(py_style_string(channel.get("thread_id").unwrap_or(&null))),
    );

    let mut message_payload = Map::new();
    message_payload.insert("message_id".to_string(), json_optional_i64(message_id));
    if !message_ids.is_empty() {
        message_payload.insert(
            "message_ids".to_string(),
            JsonValue::Array(message_ids.iter().copied().map(json_i64).collect()),
        );
    }
    message_payload.insert(
        "logical_turn_message_count".to_string(),
        json_optional_i64(logical_turn_message_count),
    );
    if !attachments.is_empty() {
        message_payload.insert("attachments".to_string(), JsonValue::Array(attachments));
    }

    let mut payload = Map::new();
    payload.insert(
        "transport".to_string(),
        json_optional_string(py_style_string(source.get("transport").unwrap_or(&null))),
    );
    payload.insert(
        "event_kind".to_string(),
        json_string(
            py_style_string(source.get("event_kind").unwrap_or(&null))
                .unwrap_or_else(|| "message".to_string()),
        ),
    );
    payload.insert(
        "event_id".to_string(),
        json_optional_string(py_style_string(source.get("event_id").unwrap_or(&null))),
    );
    payload.insert("channel".to_string(), JsonValue::Object(channel_payload));
    payload.insert("message".to_string(), JsonValue::Object(message_payload));
    JsonValue::Object(compact_object(payload))
}

fn compact_transport_reply_envelope_json_value(envelope: &JsonValue) -> JsonValue {
    let JsonValue::Object(source) = envelope else {
        return JsonValue::Null;
    };
    let null = JsonValue::Null;
    let target = object_or_empty(source.get("target"));
    let reply_to = object_or_empty(source.get("reply_to"));
    let message = object_or_empty(source.get("message"));
    let reply_to_message_id = reply_to.get("message_id").and_then(normalize_positive_int);
    let mut reply_to_message_ids = normalize_message_ids_value(reply_to.get("message_ids"));
    if let Some(value) = reply_to_message_id {
        if !reply_to_message_ids.contains(&value) {
            reply_to_message_ids.push(value);
        }
    }
    let attachments = normalize_attachments_value(message.get("attachments"));

    let mut target_payload = Map::new();
    target_payload.insert(
        "channel_id".to_string(),
        json_optional_string(py_style_string(target.get("channel_id").unwrap_or(&null))),
    );
    target_payload.insert(
        "thread_id".to_string(),
        json_optional_string(py_style_string(target.get("thread_id").unwrap_or(&null))),
    );

    let mut reply_to_payload = Map::new();
    reply_to_payload.insert(
        "event_id".to_string(),
        json_optional_string(py_style_string(reply_to.get("event_id").unwrap_or(&null))),
    );
    reply_to_payload.insert(
        "message_id".to_string(),
        json_optional_i64(reply_to_message_id),
    );
    if !reply_to_message_ids.is_empty() {
        reply_to_payload.insert(
            "message_ids".to_string(),
            JsonValue::Array(reply_to_message_ids.iter().copied().map(json_i64).collect()),
        );
    }

    let mut message_payload = Map::new();
    if !attachments.is_empty() {
        message_payload.insert("attachments".to_string(), JsonValue::Array(attachments));
    }

    let mut payload = Map::new();
    payload.insert(
        "transport".to_string(),
        json_optional_string(py_style_string(source.get("transport").unwrap_or(&null))),
    );
    payload.insert("target".to_string(), JsonValue::Object(target_payload));
    payload.insert("reply_to".to_string(), JsonValue::Object(reply_to_payload));
    payload.insert("message".to_string(), JsonValue::Object(message_payload));
    JsonValue::Object(compact_object(payload))
}

#[cfg(test)]
mod tests;
