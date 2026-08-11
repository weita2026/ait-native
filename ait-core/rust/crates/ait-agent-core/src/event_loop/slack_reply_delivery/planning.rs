use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_slack_reply_delivery";
const REPLY_DELIVERY_CONTRACT: &str = "ait_agent_core.event_loop.SlackReplyDelivery.v1";
const DEFAULT_RESPONSE_TYPE: &str = "in_channel";
const DEFAULT_RECENT_COMMAND_LIMIT: usize = 64;
const INVALID_TURN_PAYLOAD_ERROR: &str = "AIT gateway returned an invalid Slack turn payload.";

pub trait SlackReplyDeliveryPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackReplyDeliveryPlanner;

impl SlackReplyDeliveryPlanner for DefaultSlackReplyDeliveryPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_reply_delivery_json(request)
    }
}

pub fn agent_slack_reply_delivery_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_slack_reply_delivery_planner(&DefaultSlackReplyDeliveryPlanner, request)
}

pub fn plan_with_slack_reply_delivery_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: SlackReplyDeliveryPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_reply_delivery_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "turn_result".to_string());

    match stage.as_str() {
        "turn_result" | "execute_turn_result" | "reply_turn_result" => {
            plan_turn_result(object, DeliveryMode::Background)
        }
        "background_result" | "background_reply" | "run_pending_reply_safe" => {
            plan_background_result(object)
        }
        "inline_response" | "inline_reply" => plan_turn_result(object, DeliveryMode::Inline),
        other => Err(format!("unsupported Slack reply delivery stage: {other}")),
    }
}

fn plan_background_result(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let pending = pending_reply_object(object)?;
    if let Some(error) = background_error_text(object) {
        let response_url = clean_text(pending.get("response_url")).unwrap_or_default();
        let error_text = format!("ait Slack bot error: {error}");
        let delivery_operation = send_response_operation(&response_url, &error_text, "ephemeral");

        return Ok(base_payload(
            "background_result",
            "background_error_delivery_planned",
            json!({
                "ok": true,
                "turn_ok": false,
                "reply_text": error_text,
                "error_text": error_text,
                "should_deliver_response": true,
                "should_send_response": true,
                "response_type": "ephemeral",
                "delivery_operation": delivery_operation,
                "state_patch": JsonValue::Null,
                "remember_command_patch": JsonValue::Null,
                "actions": [
                    {
                        "kind": "send_error_response",
                        "operation": delivery_operation,
                    }
                ],
            }),
        ));
    }

    plan_turn_result(object, DeliveryMode::Background)
}

fn plan_turn_result(
    object: &Map<String, JsonValue>,
    delivery_mode: DeliveryMode,
) -> Result<JsonValue, String> {
    let pending = pending_reply_object(object)?;
    let turn = turn_object(object)?;
    let turn_ok =
        optional_bool(turn.get("ok")).ok_or_else(|| INVALID_TURN_PAYLOAD_ERROR.to_string())?;
    let response_type = clean_text(object.get("response_type"))
        .unwrap_or_else(|| DEFAULT_RESPONSE_TYPE.to_string());

    if turn_ok {
        plan_successful_turn(object, pending, turn, &response_type, delivery_mode)
    } else {
        plan_failed_turn(object, pending, turn, &response_type, delivery_mode)
    }
}

fn plan_successful_turn(
    object: &Map<String, JsonValue>,
    pending: &Map<String, JsonValue>,
    turn: &Map<String, JsonValue>,
    response_type: &str,
    delivery_mode: DeliveryMode,
) -> Result<JsonValue, String> {
    let reply_text = clean_text(turn.get("reply_text")).unwrap_or_default();
    let last_synced_sequence = next_delivery_sequence(object, pending);
    let state_patch = delivered_command_patch(object, pending, turn, last_synced_sequence);
    let should_deliver_response = !reply_text.is_empty();

    let mut actions = vec![json!({
        "kind": "record_delivered_command",
        "patch": state_patch,
    })];
    let (delivery_operation, response) = delivery_outputs(
        pending,
        &reply_text,
        response_type,
        should_deliver_response,
        delivery_mode,
        &mut actions,
    );

    Ok(base_payload(
        delivery_mode.stage_name(),
        if should_deliver_response {
            delivery_mode.success_state()
        } else {
            "turn_completed_without_reply"
        },
        json!({
            "ok": true,
            "turn_ok": true,
            "reply_text": reply_text,
            "last_synced_sequence": last_synced_sequence,
            "should_deliver_response": should_deliver_response,
            "should_send_response": should_deliver_response && delivery_mode == DeliveryMode::Background,
            "should_return_inline_response": should_deliver_response && delivery_mode == DeliveryMode::Inline,
            "response_type": response_type,
            "delivery_operation": delivery_operation,
            "response": response,
            "state_patch": state_patch,
            "remember_command_patch": state_patch,
            "actions": actions,
        }),
    ))
}

fn plan_failed_turn(
    object: &Map<String, JsonValue>,
    pending: &Map<String, JsonValue>,
    turn: &Map<String, JsonValue>,
    response_type: &str,
    delivery_mode: DeliveryMode,
) -> Result<JsonValue, String> {
    let error_text =
        clean_text(turn.get("error")).unwrap_or_else(|| "Unknown backend reply error.".to_string());
    let reply_text = format!("The AI reply failed.\n{error_text}");
    let last_synced_sequence = next_delivery_sequence(object, pending);
    let state_patch = delivered_command_patch(object, pending, turn, last_synced_sequence);

    let mut actions = vec![json!({
        "kind": "record_delivered_command",
        "patch": state_patch,
    })];
    let (delivery_operation, response) = delivery_outputs(
        pending,
        &reply_text,
        response_type,
        true,
        delivery_mode,
        &mut actions,
    );

    Ok(base_payload(
        delivery_mode.stage_name(),
        "turn_failed_delivery_planned",
        json!({
            "ok": true,
            "turn_ok": false,
            "reply_text": reply_text,
            "error_text": error_text,
            "last_synced_sequence": last_synced_sequence,
            "should_deliver_response": true,
            "should_send_response": delivery_mode == DeliveryMode::Background,
            "should_return_inline_response": delivery_mode == DeliveryMode::Inline,
            "response_type": response_type,
            "delivery_operation": delivery_operation,
            "response": response,
            "state_patch": state_patch,
            "remember_command_patch": state_patch,
            "actions": actions,
        }),
    ))
}

fn delivery_outputs(
    pending: &Map<String, JsonValue>,
    reply_text: &str,
    response_type: &str,
    should_deliver_response: bool,
    delivery_mode: DeliveryMode,
    actions: &mut Vec<JsonValue>,
) -> (JsonValue, JsonValue) {
    if !should_deliver_response {
        return (JsonValue::Null, JsonValue::Null);
    }

    match delivery_mode {
        DeliveryMode::Background => {
            let response_url = clean_text(pending.get("response_url")).unwrap_or_default();
            let operation = send_response_operation(&response_url, reply_text, response_type);
            actions.push(json!({
                "kind": "send_response",
                "operation": operation,
            }));
            (operation, JsonValue::Null)
        }
        DeliveryMode::Inline => {
            let response = command_message_response(reply_text, response_type);
            actions.push(json!({
                "kind": "return_inline_response",
                "response": response,
            }));
            (JsonValue::Null, response)
        }
    }
}

fn delivered_command_patch(
    object: &Map<String, JsonValue>,
    pending: &Map<String, JsonValue>,
    turn: &Map<String, JsonValue>,
    last_synced_sequence: i64,
) -> JsonValue {
    let request_id = clean_text(pending.get("request_id")).unwrap_or_default();
    let source_user_id = clean_text(pending.get("source_user_id"));
    let team_id = clean_text(pending.get("team_id"));
    let command_name = clean_text(pending.get("command_name"));
    let thread_id = clean_text(pending.get("thread_id"));
    let recent_ids = bounded_recent_request_ids(
        &existing_recent_request_ids(object, pending),
        &request_id,
        recent_command_limit(object),
    );

    json!({
        "slack_recent_request_ids": recent_ids,
        "slack_last_request_id": optional_string_json(clean_text(pending.get("request_id")).as_deref()),
        "slack_last_source_user_id": optional_string_json(source_user_id.as_deref()),
        "slack_last_team_id": optional_string_json(team_id.as_deref()),
        "slack_last_command_name": optional_string_json(command_name.as_deref()),
        "slack_last_thread_id": optional_string_json(thread_id.as_deref()),
        "codex_thread_binding": turn
            .get("provider_thread")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or(JsonValue::Null),
        "last_synced_sequence": last_synced_sequence,
    })
}

fn next_delivery_sequence(
    object: &Map<String, JsonValue>,
    pending: &Map<String, JsonValue>,
) -> i64 {
    object
        .get("last_synced_sequence")
        .or_else(|| pending.get("last_synced_sequence"))
        .and_then(|value| sequence_i64(Some(value)))
        .unwrap_or(0_i64)
        .saturating_add(1)
}

fn existing_recent_request_ids(
    object: &Map<String, JsonValue>,
    pending: &Map<String, JsonValue>,
) -> Vec<String> {
    object
        .get("existing_recent_request_ids")
        .or_else(|| object.get("slack_recent_request_ids"))
        .or_else(|| pending.get("existing_recent_request_ids"))
        .or_else(|| pending.get("slack_recent_request_ids"))
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| clean_text(Some(value)))
                .collect()
        })
        .unwrap_or_default()
}

fn bounded_recent_request_ids(
    existing_recent_request_ids: &[String],
    request_id: &str,
    limit: usize,
) -> Vec<String> {
    let mut recent = existing_recent_request_ids
        .iter()
        .filter(|item| item.as_str() != request_id)
        .cloned()
        .collect::<Vec<_>>();
    if !request_id.is_empty() {
        recent.push(request_id.to_string());
    }
    while recent.len() > limit {
        recent.remove(0);
    }
    recent
}

fn send_response_operation(response_url: &str, text: &str, response_type: &str) -> JsonValue {
    json!({
        "kind": "send_response",
        "response_url": response_url,
        "response_type": response_type,
        "text": text,
    })
}

fn command_message_response(text: &str, response_type: &str) -> JsonValue {
    json!({
        "response_type": response_type,
        "text": text,
    })
}

fn base_payload(stage: &str, state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "slack_reply_delivery_contract".to_string(),
        JsonValue::String(REPLY_DELIVERY_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "transport".to_string(),
        JsonValue::String("slack".to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_reply_delivery_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "reply_delivery_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    JsonValue::Object(object)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Slack reply delivery request must be an object.".to_string())
}

fn pending_reply_object(
    object: &Map<String, JsonValue>,
) -> Result<&Map<String, JsonValue>, String> {
    object
        .get("pending_reply")
        .or_else(|| object.get("pending"))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Slack reply delivery pending_reply must be a JSON object.".to_string())
}

fn turn_object(object: &Map<String, JsonValue>) -> Result<&Map<String, JsonValue>, String> {
    object
        .get("turn")
        .or_else(|| object.get("turn_result"))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Slack reply delivery turn result must be a JSON object.".to_string())
}

fn background_error_text(object: &Map<String, JsonValue>) -> Option<String> {
    clean_text(object.get("error"))
        .or_else(|| clean_text(object.get("execution_error")))
        .or_else(|| clean_text(object.get("exception")))
}

fn recent_command_limit(object: &Map<String, JsonValue>) -> usize {
    object
        .get("recent_command_limit")
        .and_then(usize_value)
        .unwrap_or(DEFAULT_RECENT_COMMAND_LIMIT)
}

fn usize_value(value: &JsonValue) -> Option<usize> {
    match value {
        JsonValue::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok()),
        JsonValue::String(text) => text.trim().parse::<usize>().ok(),
        _ => None,
    }
}

fn sequence_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        },
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
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

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .filter(|text| !text.is_empty())
        .map(|text| JsonValue::String(text.to_string()))
        .unwrap_or(JsonValue::Null)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryMode {
    Background,
    Inline,
}

impl DeliveryMode {
    fn stage_name(self) -> &'static str {
        match self {
            DeliveryMode::Background => "turn_result",
            DeliveryMode::Inline => "inline_response",
        }
    }

    fn success_state(self) -> &'static str {
        match self {
            DeliveryMode::Background => "response_delivery_planned",
            DeliveryMode::Inline => "inline_response_planned",
        }
    }
}
