use super::*;

pub(super) fn required_object<'a>(
    value: &'a JsonMap<String, JsonValue>,
    key: &str,
) -> Result<&'a JsonMap<String, JsonValue>, String> {
    value
        .get(key)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| format!("Field `{key}` must be a JSON object."))
}

pub(super) fn required_text(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<String, String> {
    optional_text(value, key).ok_or_else(|| format!("Field `{key}` must be a non-empty string."))
}

pub(super) fn optional_text(value: &JsonMap<String, JsonValue>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

pub(super) fn optional_path(value: &JsonMap<String, JsonValue>, key: &str) -> Option<PathBuf> {
    optional_text(value, key).map(PathBuf::from)
}

pub(super) fn optional_bool(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<bool>, String> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(format!("Field `{key}` must be a boolean.")),
    }
}

pub(super) fn optional_i64(value: &JsonMap<String, JsonValue>, key: &str) -> Option<i64> {
    value.get(key).and_then(JsonValue::as_i64)
}

pub(super) fn string_array(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Vec<String>, String> {
    let Some(raw) = value.get(key) else {
        return Ok(Vec::new());
    };
    let values = raw
        .as_array()
        .ok_or_else(|| format!("Field `{key}` must be an array of non-empty strings."))?;
    let mut parsed = Vec::new();
    for item in values {
        let text = item
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Field `{key}` must contain non-empty strings."))?;
        parsed.push(text.to_string());
    }
    Ok(parsed)
}

pub(super) fn optional_json_text(value: Option<&str>) -> JsonValue {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| json!(value))
        .unwrap_or(JsonValue::Null)
}

pub(super) fn path_has_parent_escape(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

pub(super) fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

pub(super) fn duration_seconds(started: Instant) -> f64 {
    let seconds = started.elapsed().as_secs_f64();
    (seconds * 1000.0).round() / 1000.0
}
