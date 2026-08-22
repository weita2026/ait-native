use super::json_helpers::print_json;
use super::*;

pub(super) fn normalize_async_job_command(
    job_type: &str,
    payload_json: &str,
) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let payload = payload_value
        .as_object()
        .ok_or_else(|| format!("{job_type} payload must be a JSON object."))?;
    let normalized = normalize_async_job_payload(job_type, payload)?;
    print_json(&JsonValue::Object(normalized))
}

pub(super) fn normalize_agent_server_job_command(payload_json: &str) -> Result<(), String> {
    print_json(&normalize_agent_server_job_json(payload_json)?)
}

pub(super) fn identity_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&identity_json(operation, &payload_value)?)
}

pub(super) fn land_request_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&land_request_json(operation, &payload_value)?)
}

pub(super) fn plan_revision_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&plan_revision_json(operation, &payload_value)?)
}

pub(super) fn workflow_async_runtime_command(
    operation: &str,
    payload_json: &str,
) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&workflow_async_runtime_json(operation, &payload_value)?)
}

pub(super) fn workflow_artifacts_command(
    operation: &str,
    payload_json: &str,
) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&workflow_artifacts_json(operation, &payload_value)?)
}

pub(super) fn policy_gate_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&policy_gate_json(operation, &payload_value)?)
}
