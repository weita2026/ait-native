use crate::json_support::{json, JsonMap, JsonValue};

use crate::config_runtime::build_plan_runtime_selection_facts_json;
use crate::time_identity::{
    build_plan_sequence_identity_payload_json, build_plan_timestamp_payload_json,
    build_plan_workflow_id_payload_json,
};

pub fn task_workflow_timestamp_facts(now: Option<&str>) -> Result<JsonValue, String> {
    build_plan_timestamp_payload_json(&json!({ "now": now }).to_string())
}

pub fn task_workflow_sequence_identity_facts(
    family: &str,
    number: i64,
    namespace_prefix: Option<&str>,
    width: usize,
) -> Result<JsonValue, String> {
    build_plan_sequence_identity_payload_json(
        &json!({
            "family": family,
            "number": number,
            "namespace_prefix": namespace_prefix,
            "width": width,
        })
        .to_string(),
    )
}

pub fn task_workflow_workflow_id_facts(
    family: &str,
    namespace_prefix: Option<&str>,
    timestamp_ms: Option<i64>,
    randomness_hex: Option<&str>,
) -> Result<JsonValue, String> {
    build_plan_workflow_id_payload_json(
        &json!({
            "family": family,
            "namespace_prefix": namespace_prefix,
            "timestamp_ms": timestamp_ms,
            "randomness_hex": randomness_hex,
        })
        .to_string(),
    )
}

pub fn task_workflow_runtime_selection_facts(
    overrides: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    let request = JsonMap::from_iter([(
        "overrides".to_string(),
        overrides
            .cloned()
            .unwrap_or(JsonValue::Object(JsonMap::new())),
    )]);
    build_plan_runtime_selection_facts_json(&JsonValue::Object(request).to_string())
}
