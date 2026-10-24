//! Plan-command projection stays concrete to the `ait plan ...` surface.
//! It intentionally wraps plan service payloads instead of claiming a shared
//! command abstraction.

use crate::json_support::{JsonMap, JsonValue};
use crate::plan_application::{
    build_plan_candidates_service_payload_map, build_plan_inspect_service_payload_map,
    build_plan_items_service_payload_map, build_plan_list_service_payload_map,
    build_plan_revisions_service_payload_map, build_plan_show_service_payload_map,
    build_plan_sync_service_payload_map, normalize_plan_candidates_service_request_payload_map,
    normalize_plan_inspect_service_request_payload_map,
    normalize_plan_items_service_request_payload_map,
    normalize_plan_list_service_request_payload_map,
    normalize_plan_revisions_service_request_payload_map,
    normalize_plan_show_service_request_payload_map,
    normalize_plan_sync_service_request_payload_map,
};
use crate::plan_workflow_json::PlanWorkflowJson;

pub fn normalize_plan_list_command_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_list_command_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_list_command_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_list_service_request_payload_map(payload)
}

pub fn build_plan_list_command_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_list_command_payload_json(payload_json)
}

pub(crate) fn build_plan_list_command_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let payload = require_object(
        build_plan_list_service_payload_map(payload)?,
        "plan list service payload",
    )?;
    match payload.get("plans") {
        Some(JsonValue::Array(rows)) => Ok(JsonValue::Array(rows.clone())),
        _ => Err("Plan list service payload must include plans as a list.".to_string()),
    }
}

pub fn normalize_plan_show_command_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_show_command_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_show_command_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_show_service_request_payload_map(payload)
}

pub fn build_plan_show_command_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_show_command_payload_json(payload_json)
}

pub(crate) fn build_plan_show_command_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let payload = require_object(
        build_plan_show_service_payload_map(payload)?,
        "plan show service payload",
    )?;
    let plan = payload
        .get("plan")
        .cloned()
        .ok_or_else(|| "Plan show service payload must include plan.".to_string())?;
    match payload.get("revision") {
        None | Some(JsonValue::Null) => Ok(plan),
        Some(revision) => Ok(JsonValue::Object(JsonMap::from_iter([
            ("plan".to_string(), plan),
            ("revision".to_string(), revision.clone()),
        ]))),
    }
}

pub fn normalize_plan_revisions_command_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless()
        .normalize_plan_revisions_command_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_revisions_command_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_revisions_service_request_payload_map(payload)
}

pub fn build_plan_revisions_command_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_revisions_command_payload_json(payload_json)
}

pub(crate) fn build_plan_revisions_command_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let payload = require_object(
        build_plan_revisions_service_payload_map(payload)?,
        "plan revisions service payload",
    )?;
    match payload.get("revisions") {
        Some(JsonValue::Array(rows)) => Ok(JsonValue::Array(rows.clone())),
        _ => Err("Plan revisions service payload must include revisions as a list.".to_string()),
    }
}

pub fn normalize_plan_items_command_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_items_command_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_items_command_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_items_service_request_payload_map(payload)
}

pub fn build_plan_items_command_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_items_command_payload_json(payload_json)
}

pub(crate) fn build_plan_items_command_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    let payload = require_object(
        build_plan_items_service_payload_map(payload)?,
        "plan items service payload",
    )?;
    payload
        .get("plan")
        .cloned()
        .ok_or_else(|| "Plan items service payload must include plan.".to_string())
}

pub fn normalize_plan_candidates_command_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless()
        .normalize_plan_candidates_command_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_candidates_command_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_candidates_service_request_payload_map(payload)
}

pub fn build_plan_candidates_command_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_candidates_command_payload_json(payload_json)
}

pub(crate) fn build_plan_candidates_command_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    build_plan_candidates_service_payload_map(payload)
}

pub fn normalize_plan_inspect_command_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_inspect_command_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_inspect_command_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_inspect_service_request_payload_map(payload)
}

pub fn build_plan_inspect_command_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_inspect_command_payload_json(payload_json)
}

pub(crate) fn build_plan_inspect_command_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    build_plan_inspect_service_payload_map(payload)
}

pub fn normalize_plan_sync_command_request_payload_json(
    payload_json: &str,
) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().normalize_plan_sync_command_request_payload_json(payload_json)
}

pub(crate) fn normalize_plan_sync_command_request_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    normalize_plan_sync_service_request_payload_map(payload)
}

pub fn build_plan_sync_command_payload_json(payload_json: &str) -> Result<JsonValue, String> {
    PlanWorkflowJson::stateless().build_plan_sync_command_payload_json(payload_json)
}

pub(crate) fn build_plan_sync_command_payload_map(
    payload: JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    build_plan_sync_service_payload_map(payload)
}

fn require_object(value: JsonValue, label: &str) -> Result<JsonMap<String, JsonValue>, String> {
    match value {
        JsonValue::Object(object) => Ok(object),
        _ => Err(format!("{label} must be an object.")),
    }
}

#[cfg(test)]
mod tests;
