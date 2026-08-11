use serde_json::{Map as JsonMap, Value as JsonValue};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::time::Instant;

pub(super) fn validate_relative_path(path: &Path, field: &str) -> Result<(), String> {
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("Field `{field}` must be a relative path."));
    }
    Ok(())
}
pub(super) fn path_field(value: &JsonMap<String, JsonValue>, key: &str) -> Result<PathBuf, String> {
    optional_path(value, key).ok_or_else(|| format!("Field `{key}` must be a non-empty path."))
}

pub(super) fn optional_path(value: &JsonMap<String, JsonValue>, key: &str) -> Option<PathBuf> {
    optional_text(value, key).map(PathBuf::from)
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

pub(super) fn optional_positive_i64(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<Option<i64>, String> {
    let Some(raw) = value.get(key) else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let Some(parsed) = raw.as_i64() else {
        return Err(format!("Field `{key}` must be a positive integer."));
    };
    if parsed < 1 {
        return Err(format!("Field `{key}` must be a positive integer."));
    }
    Ok(Some(parsed))
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

pub(super) fn string_set(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<BTreeSet<String>, String> {
    Ok(string_array(value, key)?.into_iter().collect())
}

pub(super) fn string_map(
    value: &JsonMap<String, JsonValue>,
    key: &str,
) -> Result<BTreeMap<String, String>, String> {
    let Some(raw) = value.get(key) else {
        return Ok(BTreeMap::new());
    };
    let object = raw
        .as_object()
        .ok_or_else(|| format!("Field `{key}` must be an object of string values."))?;
    let mut parsed = BTreeMap::new();
    for (entry_key, entry_value) in object {
        let text = entry_value
            .as_str()
            .ok_or_else(|| format!("Field `{key}.{entry_key}` must be a string."))?;
        parsed.insert(entry_key.clone(), text.to_string());
    }
    Ok(parsed)
}

pub(super) fn duration_seconds(started: Instant) -> f64 {
    let millis = started.elapsed().as_millis() as f64;
    (millis / 1000.0 * 1000.0).round() / 1000.0
}

pub(super) fn command_line(program: &str, args: &[String]) -> String {
    std::iter::once(program.to_string())
        .chain(args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn relative_path_string(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .to_string()
}

pub(super) fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}
