use super::*;

pub(super) fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn json_string_or_null(value: Option<String>) -> JsonValue {
    value.map(JsonValue::String).unwrap_or(JsonValue::Null)
}

pub(super) fn optional_text_field(
    payload: &JsonMap<String, JsonValue>,
    field: &str,
) -> Option<String> {
    payload.get(field).and_then(|value| {
        let normalized = value_to_string(value);
        if normalized.is_empty() {
            None
        } else {
            Some(normalized)
        }
    })
}

pub(super) fn required_text_field(
    payload: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<String, String> {
    optional_text_field(payload, field)
        .ok_or_else(|| format!("land-request payload requires text field `{field}`."))
}

pub(super) fn value_to_string(value: &JsonValue) -> String {
    if value.is_null() {
        String::new()
    } else {
        match value {
            JsonValue::String(text) => text.trim().to_string(),
            _ => value.to_string().trim().to_string(),
        }
    }
}
