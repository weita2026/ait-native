use crate::foundation::workflow_async_runtime::normalize_patchset_ci_execution_profile;
use serde_json::{Map as JsonMap, Value as JsonValue};

pub(super) const TG1_REQUIRED_SUITE_ID: &str = "tg1_required";
pub(super) const STATUS_MAX_DIAGNOSTIC_CHARS: usize = 4096;
pub(super) const STATUS_MAX_ID_CHARS: usize = 256;
pub(super) const STATUS_MAX_LIST_ITEMS: usize = 64;
pub(super) const STATUS_MAX_RECENT_JOBS: usize = 20;

pub(super) fn int_value(value: Option<&JsonValue>) -> i64 {
    match value {
        Some(JsonValue::Number(number)) => number.as_i64().unwrap_or_default(),
        Some(JsonValue::String(text)) => text.trim().parse::<i64>().unwrap_or_default(),
        _ => 0,
    }
}

pub(super) fn normalize_execution_profile(value: Option<&JsonValue>) -> Result<String, String> {
    normalize_patchset_ci_execution_profile(value.and_then(optional_text))
}

pub(super) fn optional_int(value: Option<&JsonValue>) -> Option<i64> {
    match value {
        Some(JsonValue::Number(number)) => number.as_i64(),
        Some(JsonValue::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

pub(super) fn optional_text(value: &JsonValue) -> Option<String> {
    let text = value.as_str()?.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub(super) fn required_text(
    payload: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<String, String> {
    payload
        .get(field)
        .and_then(optional_text)
        .ok_or_else(|| format!("patchset-ci payload field `{field}` must be non-empty."))
}

pub(super) fn required_text_from_object(
    payload: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<String, String> {
    payload
        .get(field)
        .and_then(optional_text)
        .ok_or_else(|| format!("patchset-ci object field `{field}` must be non-empty."))
}

pub(super) fn truncate_chars(value: &str, limit: usize) -> String {
    let total = value.chars().count();
    if total <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}

pub(super) fn unique_strs(values: Option<&Vec<JsonValue>>) -> Vec<String> {
    values
        .map(|items| unique_strs_from_values(items))
        .unwrap_or_default()
}

pub(super) fn unique_strs_from_values(values: &[JsonValue]) -> Vec<String> {
    bounded_unique_strs_from_values(values, usize::MAX, usize::MAX)
}

pub(super) fn bounded_unique_strs(
    values: Option<&Vec<JsonValue>>,
    item_limit: usize,
    char_limit: usize,
) -> Vec<String> {
    values
        .map(|items| bounded_unique_strs_from_values(items, item_limit, char_limit))
        .unwrap_or_default()
}

pub(super) fn bounded_unique_strs_from_values(
    values: &[JsonValue],
    item_limit: usize,
    char_limit: usize,
) -> Vec<String> {
    let mut seen = Vec::<String>::new();
    for value in values {
        let Some(text) = optional_text(value) else {
            continue;
        };
        let text = truncate_chars(&text, char_limit);
        if seen.iter().any(|item| item == &text) {
            continue;
        }
        seen.push(text);
        if seen.len() >= item_limit {
            break;
        }
    }
    seen
}
