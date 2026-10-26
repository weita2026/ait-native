use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{agent_slack_command_http_ingress_plan_json, agent_slack_ingress_runtime_plan_json};

const MIGRATION_STAGE: &str = "rust_agent_slack_command_http_transaction";
const COMMAND_HTTP_TRANSACTION_CONTRACT: &str =
    "ait_agent_core.event_loop.SlackCommandHttpTransaction.v1";

pub trait SlackCommandHttpTransactionPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackCommandHttpTransactionPlanner;

impl SlackCommandHttpTransactionPlanner for DefaultSlackCommandHttpTransactionPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_command_http_transaction_json(request)
    }
}

pub fn agent_slack_command_http_transaction_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_slack_command_http_transaction_planner(
        &DefaultSlackCommandHttpTransactionPlanner,
        request,
    )
}

pub fn plan_with_slack_command_http_transaction_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: SlackCommandHttpTransactionPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_command_http_transaction_json(request: &JsonValue) -> Result<JsonValue, String> {
    request_object(request)?;
    let http_plan = agent_slack_command_http_ingress_plan_json(request)?;
    let command_http_ingress_state = clean_text(http_plan.get("command_http_ingress_state"))
        .unwrap_or_else(|| {
            if bool_field(http_plan.get("should_handle_command")).unwrap_or(false) {
                "command_payload_ready".to_string()
            } else {
                "request_rejected".to_string()
            }
        });

    if !bool_field(http_plan.get("should_handle_command")).unwrap_or(false) {
        return Ok(base_payload(
            &command_http_ingress_state,
            json!({
                "ok": clone_field(&http_plan, "ok"),
                "accepted": clone_field(&http_plan, "accepted"),
                "http_status": clone_field(&http_plan, "http_status"),
                "write_json_response": clone_field(&http_plan, "write_json_response"),
                "response": clone_field(&http_plan, "response"),
                "error_kind": clone_field(&http_plan, "error_kind"),
                "error": clone_field(&http_plan, "error"),
                "command_http_ingress_state": command_http_ingress_state,
                "ingress_runtime_state": JsonValue::Null,
                "http_ingress_plan": http_plan,
                "ingress_request": JsonValue::Null,
                "ingress_plan": JsonValue::Null,
                "should_handle_command": false,
                "should_plan_ingress": false,
                "should_submit_turn": false,
                "should_create_turn": false,
                "should_start_background_reply": false,
                "should_execute_inline_reply": false,
                "actions": [],
            }),
        ));
    }

    let ingress_request = build_ingress_request(request, &http_plan)?;
    let ingress_plan = match agent_slack_ingress_runtime_plan_json(&ingress_request) {
        Ok(plan) => plan,
        Err(error) => {
            return Ok(base_payload(
                "ingress_error",
                json!({
                    "ok": false,
                    "accepted": false,
                    "http_status": 400,
                    "write_json_response": true,
                    "response": {"ok": false, "error": error},
                    "error_kind": "invalid_command_payload",
                    "error": error,
                    "command_http_ingress_state": command_http_ingress_state,
                    "ingress_runtime_state": JsonValue::Null,
                    "http_ingress_plan": http_plan,
                    "ingress_request": ingress_request,
                    "ingress_plan": JsonValue::Null,
                    "should_handle_command": true,
                    "should_plan_ingress": true,
                    "should_submit_turn": false,
                    "should_create_turn": false,
                    "should_start_background_reply": false,
                    "should_execute_inline_reply": false,
                    "actions": [],
                }),
            ));
        }
    };
    let ingress_runtime_state = clean_text(ingress_plan.get("ingress_runtime_state"))
        .unwrap_or_else(|| "planned".to_string());

    Ok(base_payload(
        "command_http_response_planned",
        json!({
            "ok": clone_field(&ingress_plan, "ok"),
            "accepted": clone_field(&ingress_plan, "accepted"),
            "http_status": 200,
            "write_json_response": true,
            "response": clone_field(&ingress_plan, "response"),
            "error_kind": JsonValue::Null,
            "error": JsonValue::Null,
            "command_http_ingress_state": command_http_ingress_state,
            "ingress_runtime_state": ingress_runtime_state,
            "http_ingress_plan": http_plan,
            "ingress_request": ingress_request,
            "ingress_plan": ingress_plan,
            "pending_reply": clone_field(&ingress_plan, "pending_reply"),
            "transport_envelope": clone_field(&ingress_plan, "transport_envelope"),
            "should_handle_command": true,
            "should_plan_ingress": true,
            "should_submit_turn": bool_field(ingress_plan.get("should_submit_turn")).unwrap_or(false),
            "should_create_turn": bool_field(ingress_plan.get("should_create_turn")).unwrap_or(false),
            "should_start_background_reply": bool_field(
                ingress_plan.get("should_start_background_reply")
            )
            .unwrap_or(false),
            "should_execute_inline_reply": bool_field(
                ingress_plan.get("should_execute_inline_reply")
            )
            .unwrap_or(false),
            "actions": clone_field(&ingress_plan, "actions"),
        }),
    ))
}

fn build_ingress_request(request: &JsonValue, http_plan: &JsonValue) -> Result<JsonValue, String> {
    let mut object = request_object(request)?.clone();
    for key in [
        "raw_payload",
        "signature",
        "signature_timestamp",
        "timestamp",
        "signing_secret",
        "now_unix_seconds",
        "timestamp_tolerance_seconds",
        "request_path",
        "path",
        "command_path",
    ] {
        object.remove(key);
    }

    let next_ingress = http_plan
        .get("next_ingress_request")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "Slack command HTTP ingress plan did not include next_ingress_request.".to_string()
        })?;
    for (key, value) in next_ingress {
        object.insert(key.clone(), value.clone());
    }
    object.insert("command_http_ingress_plan".to_string(), http_plan.clone());
    Ok(JsonValue::Object(object))
}

fn base_payload(state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "stage".to_string(),
        JsonValue::String("http_command_transaction".to_string()),
    );
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "slack_command_http_transaction_contract".to_string(),
        JsonValue::String(COMMAND_HTTP_TRANSACTION_CONTRACT.to_string()),
    );
    object.insert(
        "transport".to_string(),
        JsonValue::String("slack".to_string()),
    );
    object.insert(
        "command_http_transaction_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_http_sequencing_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_signature_verification_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_form_parsing_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert("python_ingress_allowed".to_string(), JsonValue::Bool(false));
    JsonValue::Object(object)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Slack command HTTP transaction request must be an object.".to_string())
}

fn clone_field(object: &JsonValue, key: &str) -> JsonValue {
    object.get(key).cloned().unwrap_or(JsonValue::Null)
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = match value? {
        JsonValue::String(text) => text.trim().to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => return None,
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn bool_field(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        },
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}
