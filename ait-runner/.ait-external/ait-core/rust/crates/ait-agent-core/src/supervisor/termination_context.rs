use std::fs;
use std::path::Path;

use ait_core::json_support::{json, JsonMap, JsonValue};

use crate::json_support::parse_value;

pub const AGENT_SUPERVISOR_TERMINATION_CONTEXT_CONTRACT: &str =
    "ait.agent.supervisor.termination_context.v1";

pub fn consume_worker_termination_context_json(request: &JsonValue) -> Result<JsonValue, String> {
    let request = request
        .as_object()
        .ok_or_else(|| "termination context consume request must be an object".to_string())?;
    let path = optional_text(request.get("path"), "path")?;
    let expected_pid = required_positive_i64(request.get("expected_pid"), "expected_pid")?;
    let signal = required_positive_i64(request.get("signal"), "signal")?;
    let include_issuer_details = optional_bool(
        request.get("include_issuer_details"),
        "include_issuer_details",
    )?
    .unwrap_or(false);
    let Some(path) = path else {
        return Ok(non_consumed_result("missing_path"));
    };
    let path = Path::new(&path);
    let text = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(non_consumed_result("not_found"));
        }
        Err(_) => return Ok(non_consumed_result("unreadable")),
    };
    let payload = match parse_value(&text, "Invalid termination context JSON") {
        Ok(value) => value,
        Err(_) => return Ok(non_consumed_result("invalid_json")),
    };
    let Some(payload_object) = payload.as_object() else {
        return Ok(non_consumed_result("invalid_payload"));
    };
    let Some(context_pid) = json_i64(payload_object.get("pid")) else {
        return Ok(non_consumed_result("invalid_pid"));
    };
    if context_pid != expected_pid {
        return Ok(non_consumed_result("pid_mismatch"));
    }

    let removed = match fs::remove_file(path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => false,
    };
    let suffix = termination_context_suffix(signal, payload_object, include_issuer_details);
    Ok(json!({
        "contract": AGENT_SUPERVISOR_TERMINATION_CONTEXT_CONTRACT,
        "ok": true,
        "consumed": true,
        "status": "consumed",
        "removed": removed,
        "suffix": suffix,
        "payload": payload,
    }))
}

fn non_consumed_result(status: &str) -> JsonValue {
    json!({
        "contract": AGENT_SUPERVISOR_TERMINATION_CONTEXT_CONTRACT,
        "ok": true,
        "consumed": false,
        "status": status,
        "removed": false,
        "suffix": "",
    })
}

fn termination_context_suffix(
    signal: i64,
    payload: &JsonMap<String, JsonValue>,
    include_issuer_details: bool,
) -> String {
    let mut details = vec![format!("signal={signal}")];
    push_text_detail(&mut details, "reason", payload.get("reason"));
    push_text_detail(&mut details, "worker", payload.get("worker_name"));
    if include_issuer_details {
        push_text_detail(&mut details, "issued_at", payload.get("issued_at"));
        push_scalar_detail(&mut details, "issued_by_pid", payload.get("issued_by_pid"));
    }
    format!(" ({})", details.join(", "))
}

fn push_text_detail(details: &mut Vec<String>, label: &str, value: Option<&JsonValue>) {
    let Some(value) = value.and_then(JsonValue::as_str).map(str::trim) else {
        return;
    };
    if !value.is_empty() {
        details.push(format!("{label}={value}"));
    }
}

fn push_scalar_detail(details: &mut Vec<String>, label: &str, value: Option<&JsonValue>) {
    let Some(value) = value else {
        return;
    };
    let text = match value {
        JsonValue::Null => return,
        JsonValue::String(value) => value.trim().to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        _ => return,
    };
    if !text.is_empty() {
        details.push(format!("{label}={text}"));
    }
}

fn json_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        JsonValue::Number(value) => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok())),
        _ => None,
    }
}

fn required_positive_i64(value: Option<&JsonValue>, field: &str) -> Result<i64, String> {
    let value = json_i64(value).filter(|value| *value > 0).ok_or_else(|| {
        format!("termination context consume request field `{field}` must be a positive integer")
    })?;
    Ok(value)
}

fn optional_text(value: Option<&JsonValue>, field: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => {
            let value = value.trim();
            Ok((!value.is_empty()).then(|| value.to_string()))
        }
        Some(_) => Err(format!(
            "termination context consume request field `{field}` must be a string or null"
        )),
    }
}

fn optional_bool(value: Option<&JsonValue>, field: &str) -> Result<Option<bool>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!(
            "termination context consume request field `{field}` must be a boolean or null"
        )),
    }
}

#[cfg(test)]
mod tests;
