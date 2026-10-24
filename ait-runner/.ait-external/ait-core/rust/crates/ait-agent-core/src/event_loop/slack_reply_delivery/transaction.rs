use super::{
    execute_with_slack_response_url_delivery_executor, DefaultSlackReplyDeliveryPlanner,
    DefaultSlackResponseUrlDeliveryExecutor, SlackReplyDeliveryPlanner,
    SlackResponseUrlDeliveryExecutor,
};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_slack_background_reply_transaction";
const BACKGROUND_REPLY_TRANSACTION_CONTRACT: &str =
    "ait_agent_core.event_loop.SlackBackgroundReplyTransaction.v1";

pub fn agent_slack_background_reply_transaction_execute_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    execute_with_slack_background_reply_transaction(
        &DefaultSlackReplyDeliveryPlanner,
        &DefaultSlackResponseUrlDeliveryExecutor,
        request,
    )
}

pub fn execute_with_slack_background_reply_transaction<P, E>(
    planner: &P,
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: SlackReplyDeliveryPlanner + ?Sized,
    E: SlackResponseUrlDeliveryExecutor + ?Sized,
{
    execute_background_reply_transaction_json(planner, executor, request)
}

fn execute_background_reply_transaction_json<P, E>(
    planner: &P,
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: SlackReplyDeliveryPlanner + ?Sized,
    E: SlackResponseUrlDeliveryExecutor + ?Sized,
{
    let object = request_object(request)?;
    let reply_request = reply_request(object);
    let reply_plan = planner.plan_json(&reply_request)?;
    let reply_delivery_state =
        clean_text(reply_plan.get("reply_delivery_state")).unwrap_or_else(|| "planned".to_string());
    let should_send_response =
        optional_bool(reply_plan.get("should_send_response")).unwrap_or(false);
    let delivery_operation = reply_plan
        .get("delivery_operation")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let state_patch = reply_plan
        .get("state_patch")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let remember_command_patch = reply_plan
        .get("remember_command_patch")
        .cloned()
        .unwrap_or(JsonValue::Null);

    if should_send_response && !delivery_operation.is_object() {
        return Ok(base_payload(
            "invalid_delivery_plan",
            json!({
                "ok": false,
                "completed": false,
                "reply_plan": reply_plan,
                "delivery_result": JsonValue::Null,
                "delivery_operation": delivery_operation,
                "reply_delivery_state": reply_delivery_state,
                "delivery_execution_state": JsonValue::Null,
                "should_send_response": should_send_response,
                "should_execute_response_url_delivery": false,
                "delivery_ok": false,
                "state_patch": state_patch,
                "remember_command_patch": remember_command_patch,
                "should_apply_state_patch": false,
                "state_patch_application_state": state_patch_application_state(&state_patch, false),
                "error": "Slack background reply transaction expected a delivery operation.",
            }),
        ));
    }

    let delivery_result = if should_send_response {
        let delivery_request = delivery_execution_request(object, &delivery_operation);
        execute_with_slack_response_url_delivery_executor(executor, &delivery_request)?
    } else {
        JsonValue::Null
    };
    let delivery_ok = if should_send_response {
        optional_bool(delivery_result.get("ok")).unwrap_or(false)
    } else {
        true
    };
    let should_apply_state_patch = should_apply_state_patch(&state_patch, delivery_ok);
    let state = transaction_state(should_send_response, delivery_ok, &state_patch);
    let error = if should_send_response && !delivery_ok {
        first_error_text(&delivery_result)
            .unwrap_or_else(|| "Slack background reply response delivery failed.".to_string())
    } else {
        String::new()
    };

    Ok(base_payload(
        state,
        json!({
            "ok": delivery_ok,
            "completed": delivery_ok,
            "reply_plan": reply_plan,
            "delivery_result": delivery_result,
            "delivery_operation": delivery_operation,
            "reply_delivery_state": reply_delivery_state,
            "delivery_execution_state": clean_text(delivery_result.get("delivery_execution_state")),
            "reply_text": reply_plan.get("reply_text").cloned().unwrap_or(JsonValue::Null),
            "response_type": reply_plan.get("response_type").cloned().unwrap_or(JsonValue::Null),
            "should_send_response": should_send_response,
            "should_execute_response_url_delivery": should_send_response,
            "delivery_ok": delivery_ok,
            "state_patch": state_patch,
            "remember_command_patch": remember_command_patch,
            "should_apply_state_patch": should_apply_state_patch,
            "state_patch_application_state": state_patch_application_state(
                reply_plan.get("state_patch").unwrap_or(&JsonValue::Null),
                delivery_ok,
            ),
            "error": if error.is_empty() { JsonValue::Null } else { JsonValue::String(error) },
        }),
    ))
}

fn reply_request(object: &Map<String, JsonValue>) -> JsonValue {
    object
        .get("reply_request")
        .or_else(|| object.get("reply_delivery_request"))
        .cloned()
        .unwrap_or_else(|| JsonValue::Object(object.clone()))
}

fn delivery_execution_request(
    object: &Map<String, JsonValue>,
    delivery_operation: &JsonValue,
) -> JsonValue {
    let mut request = Map::new();
    request.insert("operation".to_string(), delivery_operation.clone());

    for key in [
        "message_limit",
        "slack_message_limit",
        "timeout_seconds",
        "delivery_timeout_seconds",
        "headers",
        "delivery_headers",
        "replace_original",
    ] {
        if let Some(value) = object.get(key) {
            request.insert(
                normalized_delivery_option_key(key).to_string(),
                value.clone(),
            );
        }
    }

    if let Some(options) = object
        .get("delivery_options")
        .and_then(JsonValue::as_object)
    {
        for (key, value) in options {
            request.insert(
                normalized_delivery_option_key(key).to_string(),
                value.clone(),
            );
        }
    }

    JsonValue::Object(request)
}

fn normalized_delivery_option_key(key: &str) -> &str {
    match key {
        "slack_message_limit" => "message_limit",
        "delivery_timeout_seconds" => "timeout_seconds",
        "delivery_headers" => "headers",
        other => other,
    }
}

fn transaction_state(
    should_send_response: bool,
    delivery_ok: bool,
    state_patch: &JsonValue,
) -> &'static str {
    if should_send_response {
        if delivery_ok {
            "completed"
        } else {
            "delivery_failed"
        }
    } else if state_patch.is_null() {
        "completed_without_work"
    } else {
        "completed_without_response_delivery"
    }
}

fn should_apply_state_patch(state_patch: &JsonValue, delivery_ok: bool) -> bool {
    !state_patch.is_null() && delivery_ok
}

fn state_patch_application_state(state_patch: &JsonValue, delivery_ok: bool) -> &'static str {
    if state_patch.is_null() {
        "not_required"
    } else if delivery_ok {
        "ready"
    } else {
        "blocked_by_delivery_failure"
    }
}

fn first_error_text(value: &JsonValue) -> Option<String> {
    clean_text(value.get("error"))
        .or_else(|| clean_text(value.get("message")))
        .or_else(|| clean_text(value.get("detail")))
}

fn base_payload(state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "slack_background_reply_transaction_contract".to_string(),
        JsonValue::String(BACKGROUND_REPLY_TRANSACTION_CONTRACT.to_string()),
    );
    object.insert(
        "stage".to_string(),
        JsonValue::String("background_reply_transaction".to_string()),
    );
    object.insert(
        "transport".to_string(),
        JsonValue::String("slack".to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_background_reply_transaction_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_reply_delivery_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_response_url_delivery_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "background_reply_transaction_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    JsonValue::Object(object)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Slack background reply transaction request must be an object.".to_string())
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

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        },
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}
