use super::*;

pub(super) fn workflow_land_step(
    code: &str,
    label: &str,
    status: &str,
    detail: &str,
    command: Option<String>,
) -> JsonValue {
    let mut payload = Map::new();
    payload.insert("code".to_string(), JsonValue::String(code.to_string()));
    payload.insert("label".to_string(), JsonValue::String(label.to_string()));
    payload.insert("status".to_string(), JsonValue::String(status.to_string()));
    payload.insert("detail".to_string(), JsonValue::String(detail.to_string()));
    if let Some(command) = command {
        payload.insert("command".to_string(), JsonValue::String(command));
    }
    JsonValue::Object(payload)
}

pub(super) fn unique_command_values(values: Vec<Option<String>>) -> Vec<JsonValue> {
    let mut output = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for value in values.into_iter().flatten() {
        if seen.insert(value.clone()) {
            output.push(JsonValue::String(value));
        }
    }
    output
}

pub(super) fn nested_step_or_default(
    full_steps: &JsonValue,
    key: &str,
    default: JsonValue,
) -> JsonValue {
    full_steps
        .as_object()
        .and_then(|value| value.get(key))
        .cloned()
        .unwrap_or(default)
}
