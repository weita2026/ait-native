use crate::json_support::{json, JsonMap as Map, JsonValue};

pub const RUNTIME_BINDING_STATE_IR_VERSION: &str = "ait.runtime_binding_state.v2";
pub const DEFAULT_RUNTIME_BINDING_STATE_VERSION: i64 = 4;

const TELEGRAM_REPLY_SPOOL_FIELD: &str = "telegram_reply_spool";

const RETIRED_SESSION_FIELDS: &[&str] = &[
    "session_id",
    "canonical_session_id",
    "branch_session_id",
    "active_session_id",
    "previous_session_id",
    "shared_session_canonical_session_id",
    "shared_session_branch_session_id",
    "binding_role",
    "last_relink_skipped_from_session_id",
    "last_sync_at",
];

pub struct RuntimeBindingStateJson<S> {
    _store: S,
}

impl<S> RuntimeBindingStateJson<S> {
    pub fn new(store: S) -> Self {
        Self { _store: store }
    }

    pub fn ir_version(&self) -> &'static str {
        RUNTIME_BINDING_STATE_IR_VERSION
    }

    pub fn schema_json(&self) -> JsonValue {
        runtime_binding_state_schema_json_value()
    }

    pub fn default_payload_json(&self) -> JsonValue {
        default_runtime_binding_state_payload_json_value()
    }

    pub fn normalize_document_json(&self, payload: &JsonValue) -> JsonValue {
        normalize_runtime_binding_state_document_json_value(payload)
    }
}

impl RuntimeBindingStateJson<()> {
    pub fn stateless() -> Self {
        Self::new(())
    }
}

pub fn runtime_binding_state_ir_version() -> &'static str {
    RuntimeBindingStateJson::stateless().ir_version()
}

pub fn runtime_binding_state_schema_json() -> JsonValue {
    RuntimeBindingStateJson::stateless().schema_json()
}

pub fn default_runtime_binding_state_payload_json() -> JsonValue {
    RuntimeBindingStateJson::stateless().default_payload_json()
}

pub fn normalize_runtime_binding_state_document_json(payload: &JsonValue) -> JsonValue {
    RuntimeBindingStateJson::stateless().normalize_document_json(payload)
}

fn runtime_binding_state_schema_json_value() -> JsonValue {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://ait.dev/schema/ait.runtime_binding_state.v2.schema.json",
        "title": "AitRuntimeBindingStateSchema",
        "type": "object",
        "additionalProperties": false,
        "required": ["ir_version", "state"],
        "properties": {
            "ir_version": {"const": RUNTIME_BINDING_STATE_IR_VERSION},
            "state": {
                "type": "object",
                "additionalProperties": false,
                "required": ["version", "last_update_id", "surface_bindings", "telegram_bootstrap_auth"],
                "properties": {
                    "version": {"type": "integer", "minimum": DEFAULT_RUNTIME_BINDING_STATE_VERSION},
                    "last_update_id": {"type": "integer", "minimum": 0},
                    "surface_bindings": {"type": "object", "additionalProperties": {"type": "object"}},
                    "telegram_bootstrap_auth": {"type": "object"}
                }
            }
        }
    })
}

fn default_runtime_binding_state_payload_json_value() -> JsonValue {
    json!({
        "version": DEFAULT_RUNTIME_BINDING_STATE_VERSION,
        "last_update_id": 0,
        "surface_bindings": {},
        "telegram_bootstrap_auth": {}
    })
}

fn normalize_runtime_binding_state_document_json_value(payload: &JsonValue) -> JsonValue {
    let state = payload
        .as_object()
        .and_then(|object| object.get("state"))
        .unwrap_or(payload);
    let source = state.as_object();
    let version = source
        .and_then(|object| object.get("version"))
        .and_then(non_negative_i64)
        .unwrap_or(DEFAULT_RUNTIME_BINDING_STATE_VERSION)
        .max(DEFAULT_RUNTIME_BINDING_STATE_VERSION);
    let last_update_id = source
        .and_then(|object| object.get("last_update_id"))
        .and_then(non_negative_i64)
        .unwrap_or(0);
    let mut bindings = Map::new();
    if let Some(entries) = source
        .and_then(|object| object.get("surface_bindings"))
        .and_then(JsonValue::as_object)
    {
        for (binding_id, value) in entries {
            let Some(binding) = value.as_object() else {
                continue;
            };
            let mut binding = binding.clone();
            for field in RETIRED_SESSION_FIELDS {
                binding.remove(*field);
            }
            normalize_telegram_reply_spool(&mut binding);
            bindings.insert(binding_id.clone(), JsonValue::Object(binding));
        }
    }
    let bootstrap_auth = source
        .and_then(|object| object.get("telegram_bootstrap_auth"))
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    json!({
        "ir_version": RUNTIME_BINDING_STATE_IR_VERSION,
        "state": {
            "version": version,
            "last_update_id": last_update_id,
            "surface_bindings": bindings,
            "telegram_bootstrap_auth": bootstrap_auth
        }
    })
}

fn normalize_telegram_reply_spool(binding: &mut Map<String, JsonValue>) {
    let Some(raw_entries) = binding
        .get(TELEGRAM_REPLY_SPOOL_FIELD)
        .and_then(JsonValue::as_array)
    else {
        binding.remove(TELEGRAM_REPLY_SPOOL_FIELD);
        return;
    };
    let Some(conversation_key) = binding
        .get("conversation_key")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        binding.remove(TELEGRAM_REPLY_SPOOL_FIELD);
        return;
    };
    let entries = raw_entries
        .iter()
        .filter_map(JsonValue::as_object)
        .filter(|entry| {
            entry
                .get("conversation_key")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .is_some_and(|value| value == conversation_key)
                && RETIRED_SESSION_FIELDS
                    .iter()
                    .all(|field| !entry.contains_key(*field))
        })
        .cloned()
        .map(JsonValue::Object)
        .collect::<Vec<_>>();
    if entries.is_empty() {
        binding.remove(TELEGRAM_REPLY_SPOOL_FIELD);
    } else {
        binding.insert(
            TELEGRAM_REPLY_SPOOL_FIELD.to_string(),
            JsonValue::Array(entries),
        );
    }
}

fn non_negative_i64(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => number
            .as_i64()
            .map(|value| value.max(0))
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        JsonValue::String(value) => value.trim().parse::<i64>().ok().map(|value| value.max(0)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
