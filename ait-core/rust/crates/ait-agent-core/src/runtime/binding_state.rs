use ait_core::json_support::JsonValue;
use ait_core::runtime_binding_state::{
    default_runtime_binding_state_payload_json, normalize_runtime_binding_state_document_json,
    runtime_binding_state_ir_version, runtime_binding_state_schema_json,
};

pub fn agent_runtime_binding_state_ir_version() -> &'static str {
    runtime_binding_state_ir_version()
}

pub fn agent_runtime_binding_state_schema_json() -> JsonValue {
    runtime_binding_state_schema_json()
}

pub fn agent_default_runtime_binding_state_payload_json() -> JsonValue {
    default_runtime_binding_state_payload_json()
}

pub fn agent_normalize_runtime_binding_state_document_json(payload: &JsonValue) -> JsonValue {
    normalize_runtime_binding_state_document_json(payload)
}

#[cfg(test)]
mod tests;
