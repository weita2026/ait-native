use super::*;

pub(super) fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(|text| JsonValue::String(text.to_string()))
        .unwrap_or(JsonValue::Null)
}

pub(super) fn string_field(value: &Map<String, JsonValue>, key: &str) -> Result<String, String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(|text| text.to_string())
        .ok_or_else(|| format!("field `{key}` must be a string"))
}

pub(super) fn required_text_field(
    value: &Map<String, JsonValue>,
    key: &str,
) -> Result<String, String> {
    let resolved = string_field(value, key)?;
    if resolved.trim().is_empty() {
        return Err(format!("field `{key}` must be a non-empty string"));
    }
    Ok(resolved)
}
