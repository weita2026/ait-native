use super::*;

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

pub(super) fn parse_json_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<JsonValue> {
    match obj.get(field) {
        Some(JsonValue::String(text)) => serde_json::from_str(text).ok(),
        Some(JsonValue::Object(_)) | Some(JsonValue::Array(_)) => obj.get(field).cloned(),
        _ => None,
    }
}

pub(super) fn patchset_number(row: &JsonMap<String, JsonValue>) -> i64 {
    row.get("patchset_number").and_then(int_value).unwrap_or(0)
}

pub(super) fn string_list(value: Option<&JsonValue>) -> Vec<String> {
    match value {
        Some(JsonValue::Array(items)) => items.iter().filter_map(json_value_to_text).collect(),
        Some(JsonValue::String(text)) if text.trim().is_empty() => Vec::new(),
        Some(JsonValue::String(text)) => text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
        Some(other) => json_value_to_text(other).into_iter().collect(),
        None => Vec::new(),
    }
}

pub(super) fn insert_string(value: &mut JsonValue, field: &str, text: &str) {
    value
        .as_object_mut()
        .expect("authority docs are objects")
        .insert(field.to_string(), json!(text));
}

pub(super) fn filename(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

pub(super) fn filename_stem(path: &str) -> String {
    filename(path)
        .trim_end_matches(".md")
        .replace('_', " ")
        .trim()
        .to_string()
}
