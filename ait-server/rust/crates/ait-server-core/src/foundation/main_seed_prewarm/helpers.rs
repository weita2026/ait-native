use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub(super) fn optional_object<'a>(
    value: &'a JsonMap<String, JsonValue>,
    key: &str,
) -> Option<&'a JsonMap<String, JsonValue>> {
    value.get(key).and_then(JsonValue::as_object)
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

pub(super) fn positive_usize(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<usize>, String> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => {
            let value = number
                .as_u64()
                .ok_or_else(|| format!("Field `{key}` must be a positive integer."))?;
            if value < 1 {
                Err(format!("Field `{key}` must be a positive integer."))
            } else {
                Ok(Some(value as usize))
            }
        }
        Some(_) => Err(format!("Field `{key}` must be a positive integer.")),
    }
}

pub(super) fn positive_u64(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<u64>, String> {
    match value.get(key) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(number)) => {
            let value = number
                .as_u64()
                .ok_or_else(|| format!("Field `{key}` must be a positive integer."))?;
            if value < 1 {
                Err(format!("Field `{key}` must be a positive integer."))
            } else {
                Ok(Some(value))
            }
        }
        Some(_) => Err(format!("Field `{key}` must be a positive integer.")),
    }
}

pub(super) fn optional_string_array(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    let Some(values) = value.get(key) else {
        return Ok(None);
    };
    let values = values
        .as_array()
        .ok_or_else(|| format!("Field `{key}` must be an array of non-empty strings."))?;
    let mut parsed = Vec::new();
    for value in values {
        let item = value
            .as_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| format!("Field `{key}` must contain non-empty strings."))?;
        parsed.push(item.to_string());
    }
    Ok(Some(parsed))
}

pub(super) fn optional_string_map(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<BTreeMap<String, String>>, String> {
    let Some(values) = value.get(key) else {
        return Ok(None);
    };
    let values = values
        .as_object()
        .ok_or_else(|| format!("Field `{key}` must be an object with string values."))?;
    let mut parsed = BTreeMap::new();
    for (map_key, map_value) in values {
        let item = map_value
            .as_str()
            .ok_or_else(|| format!("Field `{key}.{map_key}` must be a string."))?;
        parsed.insert(map_key.clone(), item.to_string());
    }
    Ok(Some(parsed))
}

pub(super) fn safe_path_segment(value: &str) -> String {
    let segment = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if segment.is_empty() || segment == "." || segment == ".." {
        "segment".to_string()
    } else {
        segment
    }
}

pub(super) fn duration_seconds(started: Instant) -> f64 {
    let millis = started.elapsed().as_millis() as f64;
    (millis / 1000.0 * 1000.0).round() / 1000.0
}

pub(super) fn now_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

pub(super) fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
