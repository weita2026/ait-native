use super::*;

pub(super) const TASK_STATUS_ACTIVE: &str = "active";
pub(super) const CHANGE_STATUS_LANDED: &str = "landed";
pub(super) const CHANGE_STATUS_ARCHIVED: &str = "archived";
pub(super) const REVIEWABLE_CHANGE_STATES: &[&str] =
    &["review", "gated", "approved", "landable", "blocked"];

pub(super) fn optional_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    optional_text_field(obj, field)
}

pub(super) fn value_to_text(value: &JsonValue) -> Option<String> {
    json_value_to_text(value)
}

pub(super) fn object_text(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    object_text_field(obj, field)
}

pub(super) fn value_text(value: &JsonValue, field: &str) -> Option<String> {
    value.as_object().and_then(|obj| object_text(obj, field))
}

pub(super) fn value_text_path(value: &JsonValue, path: &[&str]) -> Option<String> {
    let mut current = value;
    for field in path {
        current = current.as_object()?.get(*field)?;
    }
    value_to_text(current)
}

pub(super) fn parse_json_field(row: &JsonMap<String, JsonValue>, field: &str) -> Option<JsonValue> {
    match row.get(field) {
        Some(JsonValue::String(text)) => serde_json::from_str(text).ok(),
        Some(value @ JsonValue::Object(_)) | Some(value @ JsonValue::Array(_)) => {
            Some(value.clone())
        }
        _ => None,
    }
}

pub(super) fn increment_summary(summary: &mut JsonValue, field: &str) {
    let Some(obj) = summary.as_object_mut() else {
        return;
    };
    if let Some(value) = obj.get_mut(field) {
        let next = value.as_i64().unwrap_or(0) + 1;
        *value = json!(next);
    }
}

pub(super) fn workflow_priority(state: Option<&str>) -> i64 {
    match state.unwrap_or_default() {
        "attention_required" => 0,
        "ready_to_land" => 1,
        "ready_to_complete" => 2,
        "in_review" => 3,
        "in_progress" => 4,
        "planning" => 5,
        "completed" => 6,
        "abandoned" | TASK_STATUS_LATER_PROMOTION_EXCLUDED | TASK_STATUS_LEGACY_CANCELED => 7,
        _ => 99,
    }
}
