use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::agent_slack_ingress_runtime_plan_json;

const MIGRATION_STAGE: &str = "rust_agent_slack_socket_mode_transaction";
const SOCKET_MODE_TRANSACTION_CONTRACT: &str =
    "ait_agent_core.event_loop.SlackSocketModeTransaction.v1";

pub trait SlackSocketModeTransactionPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackSocketModeTransactionPlanner;

impl SlackSocketModeTransactionPlanner for DefaultSlackSocketModeTransactionPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_socket_mode_transaction_json(request)
    }
}

pub fn agent_slack_socket_mode_transaction_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_slack_socket_mode_transaction_planner(
        &DefaultSlackSocketModeTransactionPlanner,
        request,
    )
}

pub fn plan_with_slack_socket_mode_transaction_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: SlackSocketModeTransactionPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_socket_mode_transaction_json(request: &JsonValue) -> Result<JsonValue, String> {
    request_object(request)?;
    let ingress_request = build_ingress_request(request)?;
    let ingress_plan = match agent_slack_ingress_runtime_plan_json(&ingress_request) {
        Ok(plan) => plan,
        Err(error) => {
            let envelope_id = envelope_id_from_request(request);
            let ack_response = envelope_id
                .as_ref()
                .map(|id| json!({ "envelope_id": id }))
                .unwrap_or(JsonValue::Null);
            let should_ack_socket_envelope = envelope_id.is_some();
            return Ok(base_payload(
                "invalid_socket_envelope",
                json!({
                    "ok": false,
                    "accepted": false,
                    "error": error,
                    "socket_ingress_state": JsonValue::Null,
                    "ingress_request": ingress_request,
                    "ingress_plan": JsonValue::Null,
                    "command_plan": JsonValue::Null,
                    "envelope_id": optional_string_json(envelope_id.as_deref()),
                    "envelope_type": optional_string_json(envelope_type_from_request(request).as_deref()),
                    "ack_response": ack_response,
                    "websocket_ack_response": ack_response,
                    "should_ack_socket_envelope": should_ack_socket_envelope,
                    "should_execute_websocket_ack": should_ack_socket_envelope,
                    "should_handle_command": false,
                    "should_plan_ingress": true,
                    "should_submit_turn": false,
                    "should_create_turn": false,
                    "should_start_background_reply": false,
                    "should_execute_inline_reply": false,
                    "actions": transaction_actions(
                        should_ack_socket_envelope,
                        false,
                        envelope_id.as_deref(),
                        &ack_response,
                        &JsonValue::Null,
                    ),
                }),
            ));
        }
    };

    let socket_ingress_state =
        clean_text(ingress_plan.get("ingress_runtime_state")).unwrap_or_else(|| "planned".into());
    let command_plan = clone_field(&ingress_plan, "command_plan");
    let envelope_id =
        clean_text(ingress_plan.get("envelope_id")).or_else(|| envelope_id_from_request(request));
    let envelope_type = clean_text(ingress_plan.get("envelope_type"))
        .or_else(|| envelope_type_from_request(request));
    let ack_response = clone_field(&ingress_plan, "response");
    let should_ack_socket_envelope = envelope_id.is_some() && ack_response.is_object();
    let should_handle_command =
        bool_field(ingress_plan.get("should_handle_command")).unwrap_or(false);
    let state = if should_handle_command {
        "command_ack_planned"
    } else {
        "ignored_envelope_ack_planned"
    };

    Ok(base_payload(
        state,
        json!({
            "ok": clone_field(&ingress_plan, "ok"),
            "accepted": clone_field(&ingress_plan, "accepted"),
            "error": JsonValue::Null,
            "socket_ingress_state": socket_ingress_state,
            "ingress_request": ingress_request,
            "ingress_plan": ingress_plan,
            "command_plan": command_plan,
            "envelope_id": optional_string_json(envelope_id.as_deref()),
            "envelope_type": optional_string_json(envelope_type.as_deref()),
            "ack_response": ack_response,
            "websocket_ack_response": ack_response,
            "should_ack_socket_envelope": should_ack_socket_envelope,
            "should_execute_websocket_ack": should_ack_socket_envelope,
            "should_handle_command": should_handle_command,
            "should_plan_ingress": true,
            "should_submit_turn": bool_field(command_plan.get("should_submit_turn")).unwrap_or(false),
            "should_create_turn": bool_field(command_plan.get("should_create_turn")).unwrap_or(false),
            "should_start_background_reply": bool_field(
                command_plan.get("should_start_background_reply")
            )
            .unwrap_or(false),
            "should_execute_inline_reply": bool_field(
                command_plan.get("should_execute_inline_reply")
            )
            .unwrap_or(false),
            "pending_reply": clone_field(&command_plan, "pending_reply"),
            "transport_envelope": clone_field(&command_plan, "transport_envelope"),
            "actions": transaction_actions(
                should_ack_socket_envelope,
                should_handle_command,
                envelope_id.as_deref(),
                &ack_response,
                &command_plan,
            ),
        }),
    ))
}

fn build_ingress_request(request: &JsonValue) -> Result<JsonValue, String> {
    let mut object = request_object(request)?.clone();
    object.insert(
        "stage".to_string(),
        JsonValue::String("socket_envelope".to_string()),
    );
    Ok(JsonValue::Object(object))
}

fn transaction_actions(
    should_ack_socket_envelope: bool,
    should_handle_command: bool,
    envelope_id: Option<&str>,
    ack_response: &JsonValue,
    command_plan: &JsonValue,
) -> JsonValue {
    let mut actions = Vec::new();
    if should_ack_socket_envelope {
        actions.push(json!({
            "kind": "ack_socket_envelope",
            "envelope_id": optional_string_json(envelope_id),
            "response": ack_response,
            "execute_before_command_side_effects": true,
        }));
    }
    if should_handle_command {
        actions.push(json!({
            "kind": "dispatch_slash_command_plan",
            "command_plan": command_plan,
            "should_submit_turn": bool_field(command_plan.get("should_submit_turn")).unwrap_or(false),
            "should_start_background_reply": bool_field(
                command_plan.get("should_start_background_reply")
            )
            .unwrap_or(false),
            "should_execute_inline_reply": bool_field(
                command_plan.get("should_execute_inline_reply")
            )
            .unwrap_or(false),
        }));
    }
    JsonValue::Array(actions)
}

fn base_payload(state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "stage".to_string(),
        JsonValue::String("socket_mode_transaction".to_string()),
    );
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "slack_socket_mode_transaction_contract".to_string(),
        JsonValue::String(SOCKET_MODE_TRANSACTION_CONTRACT.to_string()),
    );
    object.insert(
        "transport".to_string(),
        JsonValue::String("slack".to_string()),
    );
    object.insert(
        "socket_mode_transaction_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_socket_mode_sequencing_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_socket_mode_ack_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert("python_ingress_allowed".to_string(), JsonValue::Bool(false));
    object.insert(
        "python_websocket_event_loop_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(object)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Slack Socket Mode transaction request must be an object.".to_string())
}

fn envelope_id_from_request(request: &JsonValue) -> Option<String> {
    envelope_text_field(request, "envelope_id")
}

fn envelope_type_from_request(request: &JsonValue) -> Option<String> {
    envelope_text_field(request, "type")
}

fn envelope_text_field(request: &JsonValue, key: &str) -> Option<String> {
    let object = request.as_object()?;
    object
        .get("envelope")
        .or_else(|| object.get("payload"))
        .and_then(JsonValue::as_object)
        .and_then(|envelope| clean_text(envelope.get(key)))
        .or_else(|| clean_text(object.get(key)))
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

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .filter(|text| !text.trim().is_empty())
        .map(|text| JsonValue::String(text.to_string()))
        .unwrap_or(JsonValue::Null)
}
