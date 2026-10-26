use crate::json_support::{JsonMap as Map, JsonValue};
use crate::land_json::LandJson;
use crate::policy_json::PolicyJson;

pub(crate) trait JsonLookup {
    fn get_json_value(&self, key: &str) -> JsonValue;
}

impl JsonLookup for JsonValue {
    fn get_json_value(&self, key: &str) -> JsonValue {
        if key.is_empty() {
            return self.clone();
        }
        self.get(key).cloned().unwrap_or(JsonValue::Null)
    }
}

impl JsonLookup for Map<String, JsonValue> {
    fn get_json_value(&self, key: &str) -> JsonValue {
        if key.is_empty() {
            return JsonValue::Object(self.clone());
        }
        self.get(key).cloned().unwrap_or(JsonValue::Null)
    }
}

pub(crate) fn command_hint(commands: &JsonValue, key: &str) -> Option<String> {
    optional_string_field(commands, key)
}

pub(crate) fn command_hint_json(commands: &JsonValue, key: &str) -> JsonValue {
    command_hint(commands, key)
        .map(JsonValue::String)
        .unwrap_or(JsonValue::Null)
}

pub(crate) fn workflow_land_policy_has_checks(policy: Option<&Map<String, JsonValue>>) -> bool {
    PolicyJson::stateless().workflow_land_policy_has_checks(policy)
}

pub(crate) fn workflow_land_change_effectively_landed(
    change: &Map<String, JsonValue>,
    landing_summary: Option<&Map<String, JsonValue>>,
) -> bool {
    LandJson::stateless().change_effectively_landed(change, landing_summary)
}

pub(crate) fn workflow_land_submission_status(
    landing_summary: Option<&Map<String, JsonValue>>,
) -> String {
    LandJson::stateless().landing_summary_status(landing_summary)
}

pub(crate) fn workflow_land_submission_id(
    landing_summary: Option<&Map<String, JsonValue>>,
) -> Option<String> {
    LandJson::stateless().landing_summary_submission_id(landing_summary)
}

pub(crate) fn workflow_land_result(
    landing_summary: Option<&Map<String, JsonValue>>,
) -> Map<String, JsonValue> {
    LandJson::stateless().landing_summary_result(landing_summary)
}

pub(crate) fn workflow_land_result_blocker_class(result: &Map<String, JsonValue>) -> String {
    LandJson::stateless().landing_result_blocker_class(result)
}

pub(crate) fn workflow_land_stale_policy_blocker_cleared(
    landing_status: &str,
    landing_blocker_class: &str,
    policy_decision: &str,
) -> bool {
    LandJson::stateless().stale_policy_blocker_cleared(
        landing_status,
        landing_blocker_class,
        policy_decision,
    )
}

pub(crate) fn external_readiness_is_ready(readiness: Option<&Map<String, JsonValue>>) -> bool {
    readiness
        .and_then(|value| value.get("ready"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(true)
}

pub(crate) fn external_readiness_blocker_detail(
    readiness: Option<&Map<String, JsonValue>>,
) -> String {
    let Some(readiness) = readiness else {
        return "External readiness is not recorded.".to_string();
    };
    let blockers = readiness
        .get("blockers")
        .and_then(JsonValue::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let row = row.as_object()?;
                    let code = optional_string_field(row, "code").unwrap_or_default();
                    let name =
                        optional_string_field(row, "name").unwrap_or_else(|| "-".to_string());
                    let path =
                        optional_string_field(row, "path").unwrap_or_else(|| "-".to_string());
                    let message = optional_string_field(row, "message").unwrap_or_default();
                    if code.is_empty() && message.is_empty() {
                        None
                    } else {
                        Some(format!("{code} {name} {path}: {message}"))
                    }
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if blockers.is_empty() {
        "External readiness is blocked.".to_string()
    } else {
        format!(
            "External readiness is blocked: {}.",
            blockers.into_iter().take(3).collect::<Vec<_>>().join("; ")
        )
    }
}

pub(crate) fn workflow_land_policy_blocker_detail(
    policy: Option<&Map<String, JsonValue>>,
    landing_submission_id: Option<&str>,
    fallback_decision: Option<&str>,
) -> String {
    PolicyJson::stateless().workflow_land_policy_blocker_detail(
        policy,
        landing_submission_id,
        fallback_decision,
    )
}

pub(crate) fn field_obj(value: &JsonValue, key: &str) -> Map<String, JsonValue> {
    if key.is_empty() {
        return value.as_object().cloned().unwrap_or_default();
    }
    value
        .get(key)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn field_obj_value<T: JsonLookup>(value: &T, key: &str) -> JsonValue {
    value.get_json_value(key)
}

pub(crate) fn clone_field<T: JsonLookup>(value: &T, key: &str) -> JsonValue {
    value.get_json_value(key)
}

pub(crate) fn clone_obj_field(value: &JsonValue, key: &str) -> Map<String, JsonValue> {
    value
        .get(key)
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn insert_json(target: &mut Map<String, JsonValue>, key: &str, value: JsonValue) {
    target.insert(key.to_string(), value);
}

pub(crate) fn string_field<T: JsonLookup>(value: &T, key: &str) -> String {
    optional_string_field(value, key).unwrap_or_default()
}

pub(crate) fn optional_string_field<T: JsonLookup>(value: &T, key: &str) -> Option<String> {
    let field = value.get_json_value(key);
    field
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

pub(crate) fn optional_nonempty_string<T: JsonLookup>(value: &T, key: &str) -> Option<String> {
    optional_string_field(value, key)
}

pub(crate) fn bool_field<T: JsonLookup>(value: &T, key: &str) -> bool {
    value.get_json_value(key).as_bool().unwrap_or(false)
}

pub(crate) fn optional_bool_field<T: JsonLookup>(value: &T, key: &str) -> Option<bool> {
    value.get_json_value(key).as_bool()
}

pub(crate) fn int_field<T: JsonLookup>(value: &T, key: &str) -> i64 {
    value.get_json_value(key).as_i64().unwrap_or(0)
}

pub(crate) fn optional_obj_field<T: JsonLookup>(
    value: &T,
    key: &str,
) -> Option<Map<String, JsonValue>> {
    value.get_json_value(key).as_object().cloned()
}
