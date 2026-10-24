use crate::json_support::{JsonMap as Map, JsonValue as Value};

use crate::json_support::JsonCodec;

pub fn validate_planning_session_join_payload_json(payload_json: &str) -> Result<Value, String> {
    let payload = parse_object(payload_json, "Planning session join payload")?;
    let planning_session = object_field(
        &payload,
        "planning_session",
        "Planning session join payload",
    )?;
    let attachment = object_field(&payload, "attachment", "Planning session join payload")?;

    let normalized_planning_session = validate_planning_session_payload(planning_session)?;
    let normalized_attachment = validate_join_attachment_payload(attachment)?;
    Ok(Value::Object(Map::from_iter([
        (
            "planning_session".to_string(),
            Value::Object(normalized_planning_session),
        ),
        (
            "attachment".to_string(),
            Value::Object(normalized_attachment),
        ),
    ])))
}

fn validate_planning_session_payload(
    payload: Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let mut normalized = payload;
    let planning_session_id = require_text_field(
        &normalized,
        "planning_session_id",
        "Planning session payload",
    )?;
    let plan_id = require_text_field(&normalized, "plan_id", "Planning session payload")?;
    let status = require_text_field(&normalized, "status", "Planning session payload")?;
    normalized.insert(
        "planning_session_id".to_string(),
        Value::String(planning_session_id),
    );
    normalized.insert("plan_id".to_string(), Value::String(plan_id));
    normalized.insert("status".to_string(), Value::String(status));
    normalize_optional_text_field(&mut normalized, "mode")?;
    normalize_optional_text_field(&mut normalized, "artifact_status")?;
    normalize_optional_text_field(&mut normalized, "preferred_agent")?;
    Ok(normalized)
}

fn validate_join_attachment_payload(
    payload: Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    let planning_session_id = require_text_field(
        &payload,
        "planning_session_id",
        "Planning session join attachment",
    )?;
    let plan_id = require_text_field(&payload, "plan_id", "Planning session join attachment")?;
    let surface = require_text_field(&payload, "surface", "Planning session join attachment")?;
    let mut normalized = Map::from_iter([
        (
            "planning_session_id".to_string(),
            Value::String(planning_session_id),
        ),
        ("plan_id".to_string(), Value::String(plan_id)),
        ("surface".to_string(), Value::String(surface)),
    ]);
    copy_optional_text_field(&mut normalized, "preferred_agent", &payload)?;
    copy_optional_text_field(&mut normalized, "title", &payload)?;
    copy_optional_text_field(&mut normalized, "model_name", &payload)?;
    copy_optional_text_field(&mut normalized, "status", &payload)?;
    Ok(normalized)
}

fn copy_optional_text_field(
    payload: &mut Map<String, Value>,
    key: &str,
    source: &Map<String, Value>,
) -> Result<(), String> {
    if let Some(value) = optional_text_field(source, key)? {
        payload.insert(key.to_string(), Value::String(value));
    }
    Ok(())
}

fn parse_object(payload_json: &str, label: &str) -> Result<Map<String, Value>, String> {
    JsonCodec::parse_object_with_error_prefix(
        payload_json,
        &format!("{label} must be valid JSON"),
        &format!("{label} must be an object."),
    )
    .map_err(String::from)
}

fn object_field(
    payload: &Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<Map<String, Value>, String> {
    match payload.get(key) {
        Some(Value::Object(map)) => Ok(map.clone()),
        _ => Err(format!("{label} must include {key}.")),
    }
}

fn require_text_field(
    payload: &Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<String, String> {
    optional_text_field(payload, key)?.ok_or_else(|| format!("{label} must include {key}."))
}

fn normalize_optional_text_field(
    payload: &mut Map<String, Value>,
    key: &str,
) -> Result<(), String> {
    if let Some(value) = optional_text_field(payload, key)? {
        payload.insert(key.to_string(), Value::String(value));
    }
    Ok(())
}

fn optional_text_field(payload: &Map<String, Value>, key: &str) -> Result<Option<String>, String> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => {
            let value = raw.trim();
            Ok((!value.is_empty()).then(|| value.to_string()))
        }
        _ => Err(format!("{key} must be a string or null.")),
    }
}

#[cfg(test)]
mod tests;
