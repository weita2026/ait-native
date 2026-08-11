use super::inputs::OperatorMetricsInput;
use super::*;

const DEFAULT_CACHE_TTL_SECONDS: f64 = 5.0;

pub(super) fn annotate_operator_read_payload(
    input: &OperatorMetricsInput,
    mut payload: JsonValue,
) -> JsonValue {
    let Some(obj) = payload.as_object_mut() else {
        return payload;
    };
    obj.insert(
        "cache_state".to_string(),
        json!(input.cache_state.as_deref().unwrap_or("computed")),
    );
    obj.insert(
        "cache_age_seconds".to_string(),
        json!(round_f64(input.cache_age_seconds.max(0.0), 3)),
    );
    obj.insert(
        "cache_ttl_seconds".to_string(),
        json!(input
            .cache_ttl_seconds
            .unwrap_or(DEFAULT_CACHE_TTL_SECONDS)
            .max(0.0)),
    );
    let cached_at = input
        .cached_at
        .clone()
        .or_else(|| input.snapshot_at.clone())
        .unwrap_or_else(|| "unknown".to_string());
    obj.insert("cached_at".to_string(), json!(cached_at));
    payload
}

pub(super) fn readiness_check(
    name: &str,
    status: &str,
    summary: impl Into<String>,
    detail: Option<String>,
    recommended_action: &str,
) -> JsonValue {
    json!({
        "name": name,
        "status": status,
        "summary": summary.into(),
        "detail": detail,
        "recommended_action": recommended_action,
    })
}

pub(super) fn ranked_operator_action(actions: &[String]) -> String {
    actions
        .iter()
        .map(|action| action.as_str())
        .max_by_key(|action| (operator_action_priority(action), *action))
        .unwrap_or("none")
        .to_string()
}

fn operator_action_priority(action: &str) -> i64 {
    match action {
        "migrate_to_postgres" => 60,
        "reclaim_stale" => 50,
        "inspect_failed" | "exhausted_failed" => 40,
        "wait_for_retry" => 30,
        "monitor_workers" => 20,
        "inspect" | "optimize" | "repack" | "pack" | "gc" | "inspect_storage"
        | "inspect_postgres" | "configure_postgres" | "run_core_build" => 10,
        "none" => 0,
        _ => 1,
    }
}

pub(super) fn count_rows(
    rows: &[JsonMap<String, JsonValue>],
    field: &str,
) -> BTreeMap<String, i64> {
    let mut counts = BTreeMap::new();
    for row in rows {
        let key = object_text(row, field).unwrap_or_else(|| "unknown".to_string());
        increment_count(&mut counts, &key, 1);
    }
    counts
}

pub(super) fn count_value(map: &BTreeMap<String, i64>, key: &str) -> i64 {
    map.get(key).copied().unwrap_or(0)
}

pub(super) fn merge_count_summary(
    target: &mut BTreeMap<String, i64>,
    source: JsonMap<String, JsonValue>,
) {
    for (key, value) in source {
        increment_count(target, &key, int_value(&value).unwrap_or(0));
    }
}

pub(super) fn increment_count(target: &mut BTreeMap<String, i64>, key: &str, value: i64) {
    target
        .entry(key.to_string())
        .and_modify(|count| *count += value)
        .or_insert(value);
}

pub(super) fn add_metric(value: &mut JsonValue, field: &str, amount: i64) {
    let next = value.get(field).and_then(int_value).unwrap_or(0) + amount;
    set_metric(value, field, next);
}

pub(super) fn set_metric(value: &mut JsonValue, field: &str, amount: i64) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(field.to_string(), json!(amount));
    }
}

pub(super) fn object_value(value: &JsonValue, field: &str) -> JsonMap<String, JsonValue> {
    value
        .get(field)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

pub(super) fn object_field(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> JsonMap<String, JsonValue> {
    obj.get(field)
        .and_then(|value| {
            if let Some(map) = value.as_object() {
                Some(map.clone())
            } else if let Some(text) = value.as_str() {
                serde_json::from_str::<JsonValue>(text)
                    .ok()
                    .and_then(|parsed| parsed.as_object().cloned())
            } else {
                None
            }
        })
        .unwrap_or_default()
}

pub(super) fn object_array(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Vec<JsonMap<String, JsonValue>> {
    obj.get(field)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_object)
        .cloned()
        .collect()
}

pub(super) fn nested_object_rows(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Vec<JsonMap<String, JsonValue>> {
    object_array(obj, field)
}

pub(super) fn json_map(value: JsonValue) -> JsonMap<String, JsonValue> {
    value.as_object().cloned().unwrap_or_default()
}

pub(super) fn oldest_text(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (None, None) => None,
        (Some(value), None) | (None, Some(value)) => Some(value),
        (Some(left), Some(right)) => Some(if left <= right { left } else { right }),
    }
}

pub(super) fn latest_text(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (None, None) => None,
        (Some(value), None) | (None, Some(value)) => Some(value),
        (Some(left), Some(right)) => Some(if left >= right { left } else { right }),
    }
}

pub(super) fn first_present_value(
    sources: &[&JsonMap<String, JsonValue>],
    fields: &[&str],
) -> Option<JsonValue> {
    for source in sources {
        for field in fields {
            if let Some(value) = source.get(*field) {
                if !value.is_null() {
                    return Some(value.clone());
                }
            }
        }
    }
    None
}

pub(super) fn first_present_i64(
    sources: &[&JsonMap<String, JsonValue>],
    fields: &[&str],
) -> Option<i64> {
    first_present_value(sources, fields).and_then(|value| int_value(&value))
}

pub(super) fn first_present_f64(
    sources: &[&JsonMap<String, JsonValue>],
    fields: &[&str],
) -> Option<f64> {
    first_present_value(sources, fields).and_then(|value| f64_value(&value))
}

pub(super) fn optional_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    optional_text_field(obj, field)
}

pub(super) fn object_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    object_text_field(obj, field)
}

pub(super) fn value_text(value: &JsonValue, field: &str) -> Option<String> {
    value.as_object().and_then(|obj| object_text(obj, field))
}

pub(super) fn value_int(value: &JsonValue, field: &str) -> i64 {
    value.get(field).and_then(int_value).unwrap_or(0)
}

pub(super) fn int_field(obj: &JsonMap<String, JsonValue>, field: &str) -> i64 {
    obj.get(field).and_then(int_value).unwrap_or(0)
}

pub(super) fn int_value(value: &JsonValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
        .or_else(|| value.as_str()?.trim().parse::<i64>().ok())
}

pub(super) fn optional_i64(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<i64>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => int_value(value)
            .ok_or_else(|| format!("`{field}` must be an integer when present."))
            .map(Some),
    }
}

pub(super) fn optional_f64(
    obj: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<f64>, String> {
    match obj.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => f64_value(value)
            .ok_or_else(|| format!("`{field}` must be a number when present."))
            .map(Some),
    }
}

pub(super) fn optional_f64_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<f64> {
    obj.get(field).and_then(f64_value)
}

pub(super) fn f64_value(value: &JsonValue) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_i64().map(|value| value as f64))
        .or_else(|| value.as_u64().map(|value| value as f64))
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())
}

pub(super) fn optional_bool(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<bool> {
    obj.get(field).and_then(bool_value)
}

pub(super) fn object_bool(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<bool> {
    obj.get(field).and_then(bool_value)
}

pub(super) fn bool_value(value: &JsonValue) -> Option<bool> {
    value.as_bool().or_else(
        || match value.as_str()?.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
    )
}

pub(super) fn round_f64(value: f64, digits: i32) -> f64 {
    let factor = 10_f64.powi(digits);
    (value * factor).round() / factor
}
