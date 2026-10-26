use super::helpers::*;
use crate::transport::agent_transport_reply_envelope_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

pub fn agent_telegram_polling_cycle_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let last_update_id = optional_i64(object.get("last_update_id")).unwrap_or(0);
    let poll_timeout_seconds = optional_i64(object.get("poll_timeout_seconds")).unwrap_or(0);
    let background_sync_enabled =
        optional_bool(object.get("background_sync_enabled")).unwrap_or(false);
    let background_sync_interval_seconds =
        optional_f64(object.get("background_sync_interval_seconds")).unwrap_or(0.0);
    let now_monotonic_seconds = optional_f64(object.get("now_monotonic_seconds")).unwrap_or(0.0);
    let next_background_sync_at = optional_f64(object.get("next_background_sync_at"));

    let (planned_next_background_sync_at, background_sync_due) = plan_background_sync(
        background_sync_enabled,
        next_background_sync_at,
        now_monotonic_seconds,
        background_sync_interval_seconds,
    );
    let planned_poll_timeout_seconds = plan_poll_timeout_seconds(
        poll_timeout_seconds,
        background_sync_enabled,
        planned_next_background_sync_at,
        now_monotonic_seconds,
    );

    Ok(json!({
        "offset": last_update_id + 1,
        "poll_timeout_seconds": planned_poll_timeout_seconds,
        "background_sync_enabled": background_sync_enabled,
        "background_sync_due": background_sync_due,
        "should_run_background_sync_once": background_sync_due,
        "next_background_sync_at": planned_next_background_sync_at,
    }))
}

pub fn agent_telegram_update_dispatch_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let update = object
        .get("update")
        .ok_or_else(|| "update is required".to_string())?;
    let fallback_update_key = object
        .get("fallback_update_key")
        .and_then(JsonValue::as_str)
        .unwrap_or("memory-unknown");
    let planned = plan_telegram_update_dispatch(update, fallback_update_key)?;

    Ok(planned.into_json())
}

pub fn agent_telegram_update_batch_dispatch_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let updates = object
        .get("updates")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "updates must be a JSON array".to_string())?;
    let fallback_update_keys = object
        .get("fallback_update_keys")
        .and_then(JsonValue::as_array);
    let (dispatch_items, last_update_id) =
        plan_telegram_update_batch_dispatch(updates, fallback_update_keys)?;

    Ok(json!({
        "dispatch_items": dispatch_items,
        "last_update_id": last_update_id,
        "should_update_last_update_id": last_update_id != 0,
        "update_count": updates.len(),
    }))
}

pub fn agent_telegram_service_runtime_shell_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("updates") {
            "updates"
        } else {
            "poll"
        },
    );

    match stage {
        "poll" => plan_telegram_service_poll_shell(object),
        "updates" => plan_telegram_service_updates_shell(object),
        other => Err(format!(
            "unsupported Telegram service runtime shell stage: {other}"
        )),
    }
}

pub fn agent_telegram_callback_action_boundary_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result") {
            "result"
        } else {
            "request"
        },
    );
    let action = object
        .get("action")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "action must be a JSON object".to_string())?;

    match stage {
        "request" => plan_telegram_callback_action_request(action, object),
        "result" => plan_telegram_callback_action_result(action, object),
        other => Err(format!(
            "unsupported Telegram callback action boundary stage: {other}"
        )),
    }
}

pub fn agent_telegram_service_shell_callback_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result") {
            "result"
        } else {
            "request"
        },
    );
    let action = object
        .get("action")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "action must be a JSON object".to_string())?;

    match stage {
        "request" => plan_telegram_service_shell_callback_request(action, object),
        "result" => plan_telegram_service_shell_callback_result(action, object),
        other => Err(format!(
            "unsupported Telegram service shell callback stage: {other}"
        )),
    }
}

pub fn agent_telegram_callback_side_effect_adapter_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result") {
            "result"
        } else {
            "request"
        },
    );
    let adapter_kind = object
        .get("adapter_kind")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "adapter_kind is required".to_string())?;
    if adapter_kind.is_empty() {
        return Err("adapter_kind must not be empty".to_string());
    }

    match stage {
        "request" => plan_telegram_side_effect_adapter_request(adapter_kind, object),
        "result" => plan_telegram_side_effect_adapter_result(adapter_kind, object),
        other => Err(format!(
            "unsupported Telegram callback side-effect adapter stage: {other}"
        )),
    }
}

pub fn agent_telegram_callback_execution_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result") {
            "result"
        } else {
            "request"
        },
    );
    let execution_kind = object
        .get("execution_kind")
        .or_else(|| object.get("adapter_kind"))
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "execution_kind is required".to_string())?;
    if execution_kind.is_empty() {
        return Err("execution_kind must not be empty".to_string());
    }

    match stage {
        "request" => plan_telegram_callback_execution_request(execution_kind, object),
        "result" => plan_telegram_callback_execution_result(execution_kind, object),
        other => Err(format!(
            "unsupported Telegram callback execution boundary stage: {other}"
        )),
    }
}

pub fn agent_telegram_reply_delivery_execution_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result") || object.contains_key("operation_results") {
            "result"
        } else {
            "request"
        },
    );

    match stage {
        "request" => plan_telegram_reply_delivery_execution_request(object),
        "result" => plan_telegram_reply_delivery_execution_result(object),
        other => Err(format!(
            "unsupported Telegram reply delivery execution stage: {other}"
        )),
    }
}

pub fn agent_telegram_reply_turn_delivery_callback_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result")
            || object.contains_key("operation_results")
            || object.contains_key("error")
        {
            "result"
        } else {
            "request"
        },
    );

    match stage {
        "request" => plan_telegram_reply_turn_delivery_callback_request(object),
        "result" => plan_telegram_reply_turn_delivery_callback_result(object),
        other => Err(format!(
            "unsupported Telegram reply turn delivery callback stage: {other}"
        )),
    }
}

pub fn agent_telegram_live_reply_delivery_callback_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result")
            || object.contains_key("operation_results")
            || object.contains_key("delivery_operation_results")
            || object.contains_key("error")
        {
            "result"
        } else {
            "request"
        },
    );

    match stage {
        "request" => plan_telegram_live_reply_delivery_callback_request(object),
        "result" => plan_telegram_live_reply_delivery_callback_result(object),
        other => Err(format!(
            "unsupported Telegram live reply delivery callback stage: {other}"
        )),
    }
}

pub fn agent_telegram_command_trigger_execution_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result")
            || object.contains_key("returncode")
            || object.contains_key("stdout")
            || object.contains_key("stderr")
            || object.contains_key("handler_response")
            || object.contains_key("response")
        {
            "result"
        } else {
            "request"
        },
    );

    match stage {
        "request" => plan_telegram_command_trigger_execution_request(object),
        "result" => plan_telegram_command_trigger_execution_result(object),
        other => Err(format!(
            "unsupported Telegram command trigger execution stage: {other}"
        )),
    }
}

pub fn agent_telegram_operational_trigger_callback_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object.get("stage").and_then(JsonValue::as_str).unwrap_or(
        if object.contains_key("callback_result")
            || object.contains_key("operation_results")
            || object.contains_key("handler_response")
        {
            "result"
        } else {
            "request"
        },
    );

    match stage {
        "request" => plan_telegram_operational_trigger_callback_request(object),
        "result" => plan_telegram_operational_trigger_callback_result(object),
        other => Err(format!(
            "unsupported Telegram operational trigger callback stage: {other}"
        )),
    }
}

fn plan_telegram_reply_turn_delivery_callback_request(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let assistant_event = object_field_or_empty(object, "assistant_event");
    let assistant_sequence = optional_i64(object.get("assistant_sequence"))
        .or_else(|| {
            assistant_event
                .as_object()
                .and_then(|event| optional_i64(event.get("sequence")))
        })
        .unwrap_or(0);
    let through_sequence =
        optional_i64(object.get("through_sequence")).unwrap_or(assistant_sequence);
    let execution_request = json!({
        "execution_kind": "reply_delivery",
        "callback_group": "reply_delivery",
        "operation": "send_assistant_event_reply",
        "chat_id": object.get("chat_id").cloned().unwrap_or(JsonValue::Null),
        "assistant_event": assistant_event,
        "assistant_sequence": assistant_sequence,
        "through_sequence": through_sequence,
    });
    let execution_input = json!({
        "stage": "request",
        "execution_request": execution_request,
        "reply_text": clean_text(object.get("reply_text")).unwrap_or_default(),
    });
    let execution_plan = plan_telegram_reply_delivery_execution_request(
        execution_input
            .as_object()
            .expect("reply delivery execution input is an object"),
    )?;
    let request = execution_plan
        .get("request")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));
    let should_execute = optional_bool(execution_plan.get("should_execute")).unwrap_or(false);

    Ok(json!({
        "stage": "request",
        "execution_kind": "reply_turn_delivery_callback",
        "delivery_kind": "telegram_assistant_reply",
        "callback_group": "reply_delivery",
        "callback_kind": "send_assistant_event_reply",
        "trigger_kind": "telegram_reply_turn",
        "should_execute": should_execute,
        "expects_result": true,
        "completed": false,
        "request": request,
        "result": JsonValue::Null,
        "reply_delivery_execution": execution_plan,
    }))
}

fn plan_telegram_reply_turn_delivery_callback_result(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let callback_result = object
        .get("callback_result")
        .cloned()
        .unwrap_or_else(|| build_telegram_reply_turn_delivery_callback_result(object));
    let execution_input = json!({
        "stage": "result",
        "callback_result": callback_result,
    });
    let execution_plan = plan_telegram_reply_delivery_execution_result(
        execution_input
            .as_object()
            .expect("reply delivery execution result input is an object"),
    )?;
    let result = execution_plan
        .get("result")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));
    let completed = optional_bool(execution_plan.get("completed")).unwrap_or(false);

    Ok(json!({
        "stage": "result",
        "execution_kind": "reply_turn_delivery_callback",
        "delivery_kind": "telegram_assistant_reply",
        "callback_group": "reply_delivery",
        "callback_kind": "send_assistant_event_reply",
        "trigger_kind": "telegram_reply_turn",
        "should_execute": false,
        "expects_result": false,
        "completed": completed,
        "request": JsonValue::Null,
        "result": result,
        "reply_delivery_execution": execution_plan,
    }))
}

fn build_telegram_reply_turn_delivery_callback_result(
    object: &Map<String, JsonValue>,
) -> JsonValue {
    let request = object.get("request").and_then(JsonValue::as_object);
    let operation_results = object
        .get("operation_results")
        .or_else(|| object.get("delivery_operation_results"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let operation_count = optional_i64(object.get("operation_count"))
        .or_else(|| request.and_then(|request| optional_i64(request.get("operation_count"))))
        .unwrap_or(operation_results.len() as i64);
    let assistant_sequence = optional_i64(object.get("assistant_sequence"))
        .or_else(|| request.and_then(|request| optional_i64(request.get("assistant_sequence"))))
        .unwrap_or(0);
    let through_sequence = optional_i64(object.get("through_sequence"))
        .or_else(|| request.and_then(|request| optional_i64(request.get("through_sequence"))))
        .unwrap_or(assistant_sequence);

    json!({
        "assistant_sequence": assistant_sequence,
        "through_sequence": through_sequence,
        "operation_count": operation_count,
        "operation_results": operation_results,
        "error": clean_text(object.get("error")),
    })
}

fn plan_telegram_live_reply_delivery_callback_request(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let adapter_plan = plan_telegram_side_effect_adapter_request("reply_delivery", object)?;
    let adapter_request = adapter_plan
        .get("request")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));
    let adapter_should_execute = optional_bool(adapter_plan.get("should_execute")).unwrap_or(true);
    let callback_input = json!({
        "adapter_request": adapter_request.clone(),
        "should_execute": adapter_should_execute,
    });
    let callback_plan = plan_telegram_callback_execution_request(
        "reply_delivery",
        callback_input
            .as_object()
            .expect("callback input is an object"),
    )?;
    let callback_request = callback_plan
        .get("request")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));
    let callback_request_object = callback_request.as_object();
    let should_execute = optional_bool(callback_plan.get("should_execute")).unwrap_or(false);
    let chat_id = callback_request_object
        .and_then(|request| request.get("chat_id"))
        .cloned()
        .unwrap_or_else(|| object.get("chat_id").cloned().unwrap_or(JsonValue::Null));
    let assistant_event = object_field_from(callback_request_object, "assistant_event");
    let assistant_sequence = optional_i64(
        callback_request_object
            .and_then(|request| request.get("assistant_sequence"))
            .or_else(|| {
                assistant_event
                    .as_object()
                    .and_then(|event| event.get("sequence"))
            }),
    )
    .unwrap_or(0);
    let through_sequence = optional_i64(
        callback_request_object
            .and_then(|request| request.get("through_sequence"))
            .or_else(|| object.get("through_sequence")),
    )
    .unwrap_or(assistant_sequence);

    Ok(json!({
        "stage": "request",
        "execution_kind": "live_reply_delivery_callback",
        "callback_group": "reply_delivery",
        "trigger_kind": "telegram_live_reply",
        "should_execute": should_execute,
        "expects_result": true,
        "completed": false,
        "request": {
            "execution_kind": "live_reply_delivery_callback",
            "callback_group": "reply_delivery",
            "trigger_kind": "telegram_live_reply",
            "adapter_request": adapter_request,
            "callback_execution_request": callback_request,
            "chat_id": chat_id,
            "assistant_event": assistant_event,
            "assistant_sequence": assistant_sequence,
            "through_sequence": through_sequence,
        },
    }))
}

fn plan_telegram_live_reply_delivery_callback_result(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let callback_result = object
        .get("callback_result")
        .cloned()
        .unwrap_or_else(|| build_telegram_live_reply_delivery_callback_result(object));
    let callback_input = json!({
        "callback_result": callback_result,
    });
    let callback_plan = plan_telegram_callback_execution_result(
        "reply_delivery",
        callback_input
            .as_object()
            .expect("live reply callback input is an object"),
    )?;
    let callback_result = callback_plan
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "live reply delivery callback result must be an object".to_string())?;
    let callback_error = clean_text(callback_result.get("error"));
    let side_effect_input = json!({
        "callback_result": JsonValue::Object(callback_result.clone()),
    });
    let side_effect_plan = plan_telegram_side_effect_adapter_result(
        "reply_delivery",
        side_effect_input
            .as_object()
            .expect("side-effect input is an object"),
    )?;
    let side_effect_result = side_effect_plan
        .get("result")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let delivered = optional_bool(side_effect_result.get("delivered")).unwrap_or(false);
    let assistant_sequence =
        optional_i64(side_effect_result.get("assistant_sequence")).unwrap_or(0);
    let through_sequence =
        optional_i64(side_effect_result.get("through_sequence")).unwrap_or(assistant_sequence);
    let ok = delivered && callback_error.is_none();

    Ok(json!({
        "stage": "result",
        "execution_kind": "live_reply_delivery_callback",
        "callback_group": "reply_delivery",
        "trigger_kind": "telegram_live_reply",
        "should_execute": false,
        "expects_result": false,
        "completed": ok,
        "result": {
            "execution_kind": "live_reply_delivery_callback",
            "callback_group": "reply_delivery",
            "trigger_kind": "telegram_live_reply",
            "ok": ok,
            "error": callback_error,
            "delivered": delivered,
            "assistant_sequence": assistant_sequence,
            "through_sequence": through_sequence,
            "callback_execution_result": callback_result,
            "side_effect_result": side_effect_result,
        },
    }))
}

fn build_telegram_live_reply_delivery_callback_result(
    object: &Map<String, JsonValue>,
) -> JsonValue {
    let request = object.get("request").and_then(JsonValue::as_object);
    let operation_results = object
        .get("operation_results")
        .or_else(|| object.get("delivery_operation_results"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let operation_count = optional_i64(object.get("operation_count"))
        .or_else(|| request.and_then(|request| optional_i64(request.get("operation_count"))))
        .unwrap_or(operation_results.len() as i64);
    let assistant_sequence = optional_i64(object.get("assistant_sequence"))
        .or_else(|| request.and_then(|request| optional_i64(request.get("assistant_sequence"))))
        .unwrap_or(0);
    let through_sequence = optional_i64(object.get("through_sequence"))
        .or_else(|| request.and_then(|request| optional_i64(request.get("through_sequence"))))
        .unwrap_or(assistant_sequence);
    let failed_operation_count = operation_results
        .iter()
        .filter(|result| {
            result
                .as_object()
                .map(|result| !optional_bool(result.get("ok")).unwrap_or(false))
                .unwrap_or(true)
        })
        .count() as i64;
    let delivered = optional_bool(object.get("delivered")).unwrap_or_else(|| {
        clean_text(object.get("error")).is_none()
            && operation_count > 0
            && failed_operation_count == 0
    });

    json!({
        "delivered": delivered,
        "assistant_sequence": assistant_sequence,
        "through_sequence": through_sequence,
        "operation_count": operation_count,
        "operation_results": operation_results,
        "error": clean_text(object.get("error")),
    })
}

fn plan_telegram_operational_trigger_callback_request(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let trigger = object_field_or_empty(object, "trigger");
    let trigger_object = trigger.as_object();
    let mut handler_command = text_array_field(object, "handler_command");
    if handler_command.is_empty() {
        handler_command =
            text_array_field_from(trigger_object, "handler_command").unwrap_or_default();
    }

    let adapter_plan = plan_telegram_side_effect_adapter_request("command_trigger", object)?;
    let adapter_request = adapter_plan
        .get("request")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));

    let callback_input = json!({
        "adapter_request": adapter_request,
        "handler_command": handler_command,
        "repo_root": clean_text(object.get("repo_root")).unwrap_or_default(),
        "trigger_id": clean_text(trigger_object.and_then(|trigger| trigger.get("trigger_id"))),
        "reply_to_message_id": optional_i64(object.get("reply_to_message_id")),
    });
    let callback_plan = plan_telegram_callback_execution_request(
        "command_trigger",
        callback_input
            .as_object()
            .expect("callback input is an object"),
    )?;
    let callback_request = callback_plan
        .get("request")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));

    let command_input = json!({
        "execution_request": callback_request,
    });
    let command_plan = plan_telegram_command_trigger_execution_request(
        command_input
            .as_object()
            .expect("command input is an object"),
    )?;
    let command_request = command_plan
        .get("request")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));
    let command_request_object = command_request.as_object();
    let operations = array_field(command_request_object, "operations").unwrap_or_default();
    let should_execute = optional_bool(command_plan.get("should_execute")).unwrap_or(false);
    let error = clean_text(command_request_object.and_then(|request| request.get("error")));

    Ok(json!({
        "stage": "request",
        "execution_kind": "operational_trigger_callback",
        "callback_group": "command_trigger",
        "trigger_kind": "telegram_operational_trigger",
        "should_execute": should_execute,
        "expects_result": true,
        "completed": false,
        "request": {
            "execution_kind": "operational_trigger_callback",
            "callback_group": "command_trigger",
            "trigger_kind": "telegram_operational_trigger",
            "ok": error.is_none(),
            "error": error,
            "adapter_request": adapter_request,
            "callback_execution_request": callback_request,
            "command_request": command_request,
            "operation": command_request_object
                .and_then(|request| request.get("operation"))
                .cloned()
                .unwrap_or(JsonValue::Null),
            "operations": operations,
            "operation_count": operations.len(),
        },
    }))
}

fn plan_telegram_operational_trigger_callback_result(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let command_result_plan = plan_telegram_command_trigger_execution_result(object)?;
    let command_result = command_result_plan
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "command trigger result must be an object".to_string())?;
    let ok = optional_bool(command_result.get("ok")).unwrap_or(false);
    let error = clean_text(command_result.get("error"));
    let handler_response = object_field_from(Some(command_result), "handler_response");
    let side_effect_input = json!({
        "callback_result": handler_response,
    });
    let side_effect_result = plan_telegram_side_effect_adapter_result(
        "command_trigger",
        side_effect_input
            .as_object()
            .expect("side-effect input is an object"),
    )?;
    let reply_result = side_effect_result
        .get("result")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let handled = ok && optional_bool(reply_result.get("handled")).unwrap_or(false);
    let reply_text = clean_text(reply_result.get("reply_text")).unwrap_or_default();
    let attachments = array_field(Some(&reply_result), "attachments").unwrap_or_default();
    let assistant_event = if handled && (!reply_text.is_empty() || !attachments.is_empty()) {
        build_operational_trigger_assistant_event(object, &reply_text, &attachments)
    } else {
        JsonValue::Null
    };

    Ok(json!({
        "stage": "result",
        "execution_kind": "operational_trigger_callback",
        "callback_group": "command_trigger",
        "trigger_kind": "telegram_operational_trigger",
        "should_execute": false,
        "expects_result": false,
        "completed": ok,
        "result": {
            "execution_kind": "operational_trigger_callback",
            "callback_group": "command_trigger",
            "trigger_kind": "telegram_operational_trigger",
            "ok": ok,
            "error": error,
            "handled": handled,
            "reply_text": reply_text,
            "attachments": attachments,
            "attachment_count": attachments.len(),
            "assistant_event": assistant_event,
            "should_send_assistant_event": !assistant_event.is_null(),
            "command_result": command_result,
            "side_effect_result": reply_result,
        },
    }))
}

fn build_operational_trigger_assistant_event(
    object: &Map<String, JsonValue>,
    reply_text: &str,
    attachments: &[JsonValue],
) -> JsonValue {
    let trigger = object_field_or_empty(object, "trigger");
    let trigger_object = trigger.as_object();
    let context = object_field_or_empty(object, "context");
    let context_object = context.as_object();
    let chat = object_field_or_empty(object, "chat");
    let chat_object = chat.as_object();
    let trigger_id = clean_text(trigger_object.and_then(|trigger| trigger.get("trigger_id")))
        .or_else(|| clean_text(trigger_object.and_then(|trigger| trigger.get("id"))))
        .unwrap_or_default();
    let trigger_source_path =
        clean_text(trigger_object.and_then(|trigger| trigger.get("source_path")))
            .unwrap_or_default();
    let chat_id = object
        .get("chat_id")
        .map(pythonish_text)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default();
    let chat_title = clean_text(object.get("chat_title")).unwrap_or_default();
    let chat_kind = clean_text(chat_object.and_then(|chat| chat.get("type")));
    let telegram_message_id = context_object.and_then(|context| context.get("telegram_message_id"));
    let telegram_message_ids = context_object
        .and_then(|context| context.get("telegram_message_ids"))
        .cloned()
        .unwrap_or_else(|| json!([]));
    let attachments_value = JsonValue::Array(attachments.to_vec());
    let metadata = json!({
        "delivered_via": "telegram_operational_trigger",
        "trigger_id": trigger_id,
        "trigger_source_path": trigger_source_path,
    });
    let envelope = agent_transport_reply_envelope_json(
        "telegram",
        &chat_id,
        reply_text,
        Some(chat_title.as_str()),
        chat_kind.as_deref(),
        None,
        Some("chat_reply"),
        None,
        telegram_message_id,
        Some(&telegram_message_ids),
        Some(&attachments_value),
        Some(&metadata),
    );

    json!({
        "event_type": "assistant.reply",
        "payload": {
            "text": reply_text,
            "transport_reply_envelope": envelope,
        },
    })
}

struct TelegramUpdateDispatchPlan {
    chat_id: JsonValue,
    dispatch_key: String,
    update_id: i64,
    message_id: i64,
    should_update_last_update_id: bool,
    update_key: String,
}

impl TelegramUpdateDispatchPlan {
    fn into_json(self) -> JsonValue {
        json!({
            "chat_id": self.chat_id,
            "dispatch_key": self.dispatch_key,
            "update_id": self.update_id,
            "message_id": self.message_id,
            "should_update_last_update_id": self.should_update_last_update_id,
            "update_key": self.update_key,
        })
    }

    fn into_indexed_json(self, index: usize) -> JsonValue {
        json!({
            "index": index,
            "chat_id": self.chat_id,
            "dispatch_key": self.dispatch_key,
            "update_id": self.update_id,
            "message_id": self.message_id,
            "should_update_last_update_id": self.should_update_last_update_id,
            "update_key": self.update_key,
        })
    }
}

fn plan_telegram_update_dispatch(
    update: &JsonValue,
    fallback_update_key: &str,
) -> Result<TelegramUpdateDispatchPlan, String> {
    let update_object = update
        .as_object()
        .ok_or_else(|| "update must be a JSON object".to_string())?;
    let update_id = optional_i64(update_object.get("update_id")).unwrap_or(0);
    let chat_id = telegram_update_chat_id(update_object);
    let dispatch_key = match &chat_id {
        Some(value) => format!("chat-{}", pythonish_text(value)),
        None if update_id != 0 => format!("update-{update_id}"),
        None => "update-unknown".to_string(),
    };
    let message_id = telegram_update_message_id(update_object).unwrap_or(0);
    let update_key = if update_id != 0 {
        format!("update-{update_id}")
    } else if message_id != 0 {
        format!("message-{message_id}")
    } else {
        fallback_update_key.to_string()
    };

    Ok(TelegramUpdateDispatchPlan {
        chat_id: chat_id.unwrap_or(JsonValue::Null),
        dispatch_key,
        update_id,
        message_id,
        should_update_last_update_id: update_id != 0,
        update_key,
    })
}

fn plan_telegram_service_poll_shell(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let last_update_id = optional_i64(object.get("last_update_id")).unwrap_or(0);
    let poll_timeout_seconds = optional_i64(object.get("poll_timeout_seconds")).unwrap_or(0);
    let background_sync_enabled =
        optional_bool(object.get("background_sync_enabled")).unwrap_or(false);
    let background_sync_interval_seconds =
        optional_f64(object.get("background_sync_interval_seconds")).unwrap_or(0.0);
    let now_monotonic_seconds = optional_f64(object.get("now_monotonic_seconds")).unwrap_or(0.0);
    let next_background_sync_at = optional_f64(object.get("next_background_sync_at"));

    let (planned_next_background_sync_at, background_sync_due) = plan_background_sync(
        background_sync_enabled,
        next_background_sync_at,
        now_monotonic_seconds,
        background_sync_interval_seconds,
    );
    let planned_poll_timeout_seconds = plan_poll_timeout_seconds(
        poll_timeout_seconds,
        background_sync_enabled,
        planned_next_background_sync_at,
        now_monotonic_seconds,
    );
    let offset = last_update_id + 1;
    let poll_request = json!({
        "offset": offset,
        "timeout_seconds": planned_poll_timeout_seconds,
    });
    let mut actions = Vec::new();
    if background_sync_due {
        actions.push(json!({
            "kind": "run_background_sync_once",
        }));
    }
    actions.push(json!({
        "kind": "poll_updates",
        "poll_request": poll_request.clone(),
        "offset": offset,
        "timeout_seconds": planned_poll_timeout_seconds,
    }));

    Ok(json!({
        "stage": "poll",
        "actions": actions,
        "poll_request": poll_request,
        "background_sync": {
            "enabled": background_sync_enabled,
            "due": background_sync_due,
            "should_run_background_sync_once": background_sync_due,
            "next_background_sync_at": planned_next_background_sync_at,
        },
        "next_background_sync_at": planned_next_background_sync_at,
    }))
}

fn plan_telegram_service_updates_shell(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let updates = object
        .get("updates")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "updates must be a JSON array".to_string())?;
    let fallback_update_keys = object
        .get("fallback_update_keys")
        .and_then(JsonValue::as_array);
    let (dispatch_items, last_update_id) =
        plan_telegram_update_batch_dispatch(updates, fallback_update_keys)?;
    let mut actions = Vec::with_capacity(dispatch_items.len() + usize::from(last_update_id != 0));

    for dispatch_item in dispatch_items.iter() {
        let index = dispatch_item
            .get("index")
            .cloned()
            .unwrap_or_else(|| json!(0));
        actions.push(json!({
            "kind": "dispatch_update",
            "index": index,
            "dispatch_item": dispatch_item,
        }));
    }
    if last_update_id != 0 {
        actions.push(json!({
            "kind": "update_last_update_id",
            "last_update_id": last_update_id,
        }));
    }

    Ok(json!({
        "stage": "updates",
        "actions": actions,
        "dispatch_plan": {
            "dispatch_items": dispatch_items,
            "last_update_id": last_update_id,
            "should_update_last_update_id": last_update_id != 0,
            "update_count": updates.len(),
        },
        "last_update_id": last_update_id,
        "should_update_last_update_id": last_update_id != 0,
        "update_count": updates.len(),
        "has_updates": !updates.is_empty(),
        "should_continue_without_callbacks": updates.is_empty(),
    }))
}

fn plan_telegram_callback_action_request(
    action: &Map<String, JsonValue>,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let kind = action_kind(action)?;
    match kind {
        "run_background_sync_once" => Ok(json!({
            "stage": "request",
            "action_kind": kind,
            "should_execute": true,
            "expects_result": true,
            "request": {
                "callback_kind": "run_background_sync_once",
                "callback_group": "background_sync",
            },
        })),
        "poll_updates" => {
            let poll_request = callback_poll_request(action)?;
            Ok(json!({
                "stage": "request",
                "action_kind": kind,
                "should_execute": true,
                "expects_result": true,
                "request": {
                    "callback_kind": "poll_updates",
                    "callback_group": "telegram_api",
                    "poll_request": poll_request,
                },
            }))
        }
        "dispatch_update" => {
            let dispatch_item = action
                .get("dispatch_item")
                .cloned()
                .ok_or_else(|| "dispatch_update action requires dispatch_item".to_string())?;
            let index = optional_i64(action.get("index")).unwrap_or_else(|| {
                dispatch_item
                    .as_object()
                    .and_then(|item| optional_i64(item.get("index")))
                    .unwrap_or(0)
            });
            let update = object
                .get("updates")
                .and_then(JsonValue::as_array)
                .and_then(|updates| usize::try_from(index).ok().and_then(|idx| updates.get(idx)))
                .cloned()
                .unwrap_or(JsonValue::Null);
            let dispatch_key = dispatch_item
                .as_object()
                .and_then(|item| item.get("dispatch_key"))
                .and_then(JsonValue::as_str)
                .unwrap_or("update-unknown");
            let update_key = dispatch_item
                .as_object()
                .and_then(|item| item.get("update_key"))
                .and_then(JsonValue::as_str)
                .unwrap_or("memory-unknown");
            Ok(json!({
                "stage": "request",
                "action_kind": kind,
                "should_execute": true,
                "expects_result": true,
                "request": {
                    "callback_kind": "dispatch_update",
                    "callback_group": "dispatch",
                    "index": index,
                    "queue_key": dispatch_key,
                    "update_key": update_key,
                    "dispatch_item": dispatch_item,
                    "update": update,
                },
            }))
        }
        "update_last_update_id" => {
            let last_update_id = optional_i64(action.get("last_update_id")).unwrap_or(0);
            Ok(json!({
                "stage": "request",
                "action_kind": kind,
                "should_execute": last_update_id != 0,
                "expects_result": true,
                "request": {
                    "callback_kind": "update_last_update_id",
                    "callback_group": "state",
                    "last_update_id": last_update_id,
                },
            }))
        }
        other => Err(format!(
            "unsupported Telegram callback action kind for request boundary: {other}"
        )),
    }
}

fn plan_telegram_callback_action_result(
    action: &Map<String, JsonValue>,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let kind = action_kind(action)?;
    let callback_result = match object.get("callback_result") {
        Some(value) => value
            .as_object()
            .ok_or_else(|| "callback_result must be a JSON object".to_string())?,
        None => object,
    };
    match kind {
        "run_background_sync_once" => {
            let future_count = optional_i64(callback_result.get("future_count")).unwrap_or(0);
            Ok(json!({
                "stage": "result",
                "action_kind": kind,
                "completed": true,
                "result": {
                    "callback_kind": "run_background_sync_once",
                    "future_count": future_count,
                },
            }))
        }
        "poll_updates" => {
            let updates = callback_result
                .get("updates")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            Ok(json!({
                "stage": "result",
                "action_kind": kind,
                "completed": true,
                "result": {
                    "callback_kind": "poll_updates",
                    "updates": updates,
                    "update_count": updates.len(),
                    "has_updates": !updates.is_empty(),
                },
            }))
        }
        "dispatch_update" => {
            let index = optional_i64(action.get("index")).unwrap_or(0);
            let submitted = optional_bool(callback_result.get("submitted")).unwrap_or(false);
            let dispatch_item = action
                .get("dispatch_item")
                .cloned()
                .unwrap_or(JsonValue::Null);
            Ok(json!({
                "stage": "result",
                "action_kind": kind,
                "completed": submitted,
                "result": {
                    "callback_kind": "dispatch_update",
                    "submitted": submitted,
                    "index": index,
                    "dispatch_item": dispatch_item,
                },
            }))
        }
        "update_last_update_id" => {
            let planned_last_update_id = optional_i64(action.get("last_update_id")).unwrap_or(0);
            let written_last_update_id = optional_i64(callback_result.get("last_update_id"))
                .unwrap_or(planned_last_update_id);
            Ok(json!({
                "stage": "result",
                "action_kind": kind,
                "completed": written_last_update_id == planned_last_update_id,
                "result": {
                    "callback_kind": "update_last_update_id",
                    "last_update_id": written_last_update_id,
                    "planned_last_update_id": planned_last_update_id,
                },
            }))
        }
        other => Err(format!(
            "unsupported Telegram callback action kind for result boundary: {other}"
        )),
    }
}

fn plan_telegram_service_shell_callback_request(
    action: &Map<String, JsonValue>,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let boundary = plan_telegram_callback_action_request(action, object)?;
    let action_kind = clean_text(boundary.get("action_kind")).unwrap_or_default();
    let request = boundary
        .get("request")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));
    let callback_group = clean_text(
        request
            .as_object()
            .and_then(|value| value.get("callback_group")),
    )
    .unwrap_or_default();
    let callback_kind = clean_text(
        request
            .as_object()
            .and_then(|value| value.get("callback_kind")),
    )
    .unwrap_or_else(|| action_kind.clone());
    let should_execute = optional_bool(boundary.get("should_execute")).unwrap_or(false);
    let expects_result = optional_bool(boundary.get("expects_result")).unwrap_or(true);

    Ok(json!({
        "stage": "request",
        "execution_kind": "telegram_service_shell_callback",
        "callback_group": callback_group,
        "callback_kind": callback_kind,
        "action_kind": action_kind,
        "should_execute": should_execute,
        "expects_result": expects_result,
        "completed": false,
        "request": request,
        "callback_action_boundary": boundary,
    }))
}

fn plan_telegram_service_shell_callback_result(
    action: &Map<String, JsonValue>,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let boundary = plan_telegram_callback_action_result(action, object)?;
    let action_kind = clean_text(boundary.get("action_kind")).unwrap_or_default();
    let result = boundary
        .get("result")
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}));
    let callback_kind = clean_text(
        result
            .as_object()
            .and_then(|value| value.get("callback_kind")),
    )
    .unwrap_or_else(|| action_kind.clone());
    let callback_group = match callback_kind.as_str() {
        "run_background_sync_once" => "background_sync",
        "poll_updates" => "telegram_api",
        "dispatch_update" => "dispatch",
        "update_last_update_id" => "state",
        _ => "",
    };
    let completed = optional_bool(boundary.get("completed")).unwrap_or(false);

    Ok(json!({
        "stage": "result",
        "execution_kind": "telegram_service_shell_callback",
        "callback_group": callback_group,
        "callback_kind": callback_kind,
        "action_kind": action_kind,
        "should_execute": false,
        "expects_result": false,
        "completed": completed,
        "result": result,
        "callback_action_boundary": boundary,
    }))
}

fn plan_telegram_side_effect_adapter_request(
    adapter_kind: &str,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    match adapter_kind {
        "background_sync_pass" => {
            let chat_id = object.get("chat_id").cloned().unwrap_or(JsonValue::Null);
            let binding = object_field_or_empty(object, "binding");
            let binding_object = binding.as_object();
            let workflow_notifications_enabled = optional_bool(
                binding_object.and_then(|binding| binding.get("workflow_notifications_enabled")),
            )
            .unwrap_or(false);
            let should_execute = workflow_notifications_enabled;

            Ok(json!({
                "stage": "request",
                "adapter_kind": adapter_kind,
                "should_execute": should_execute,
                "expects_result": true,
                "request": {
                    "adapter_kind": adapter_kind,
                    "callback_group": "background_sync",
                    "chat_id": chat_id,
                    "binding": binding,
                    "workflow_notifications_enabled": workflow_notifications_enabled,
                },
            }))
        }
        "command_trigger" => {
            let trigger = object_field_or_empty(object, "trigger");
            let match_payload = object_field_or_empty(object, "match_payload");
            let chat_id = object.get("chat_id").cloned().unwrap_or(JsonValue::Null);
            let chat = object_field_or_empty(object, "chat");
            let from_user = object_field_or_empty(object, "from_user");
            let chat_title = clean_text(object.get("chat_title")).unwrap_or_default();
            let context = object_field_or_empty(object, "context");
            let context_object = context.as_object();
            let repo_name = clean_text(object.get("repo_name"));
            let repo_root = clean_text(object.get("repo_root")).unwrap_or_default();
            let reply_to_message_id = optional_i64(object.get("reply_to_message_id"));
            let command = context_object.and_then(|context| context.get("command"));
            let command_name = command
                .and_then(JsonValue::as_array)
                .and_then(|values| values.first())
                .and_then(|value| clean_text(Some(value)));
            let command_args = command
                .and_then(JsonValue::as_array)
                .and_then(|values| values.get(1))
                .and_then(|value| clean_text(Some(value)));
            let chat_type = clean_text(chat.as_object().and_then(|chat| chat.get("type")));
            let trigger_id = clean_text(
                trigger
                    .as_object()
                    .and_then(|trigger| trigger.get("trigger_id")),
            )
            .or_else(|| clean_text(trigger.as_object().and_then(|trigger| trigger.get("id"))));
            let display_trigger = clean_text(
                trigger
                    .as_object()
                    .and_then(|trigger| trigger.get("display_trigger")),
            );
            let source_path = clean_text(
                trigger
                    .as_object()
                    .and_then(|trigger| trigger.get("source_path")),
            );
            let telegram_message_ids =
                array_field(context_object, "telegram_message_ids").unwrap_or_default();
            let attachments = array_field(context_object, "attachments").unwrap_or_default();
            let reply_to_message = object_field_from(context_object, "reply_to_message");
            let message_payload = object_field_from(context_object, "message");
            let binding = object.get("binding").cloned().unwrap_or(JsonValue::Null);
            let raw_text = clean_text(context_object.and_then(|context| context.get("raw_text")))
                .unwrap_or_default();
            let normalized_text =
                clean_text(context_object.and_then(|context| context.get("normalized_text")))
                    .unwrap_or_default();
            let telegram_message_id =
                optional_i64(context_object.and_then(|context| context.get("telegram_message_id")));
            let actor_identity =
                clean_text(context_object.and_then(|context| context.get("actor_identity")));

            let handler_payload = json!({
                "schema_version": 1,
                "transport": "telegram",
                "repo_name": repo_name,
                "repo_root": repo_root,
                "trigger": {
                    "id": trigger_id,
                    "display_trigger": display_trigger,
                    "source_path": source_path,
                    "match": match_payload,
                },
                "chat": {
                    "chat_id": pythonish_text(&chat_id),
                    "chat_title": chat_title,
                    "chat_type": chat_type,
                    "payload": chat,
                },
                "actor": {
                    "actor_identity": actor_identity,
                    "from_user": from_user,
                },
                "message": {
                    "raw_text": raw_text,
                    "normalized_text": normalized_text,
                    "command_name": command_name,
                    "command_args": command_args,
                    "telegram_message_id": telegram_message_id,
                    "telegram_message_ids": telegram_message_ids,
                    "reply_to_message_id": reply_to_message_id,
                    "reply_to_message": reply_to_message,
                    "attachments": attachments,
                    "payload": message_payload,
                },
                "binding": binding,
            });

            Ok(json!({
                "stage": "request",
                "adapter_kind": adapter_kind,
                "should_execute": true,
                "expects_result": true,
                "request": {
                    "adapter_kind": adapter_kind,
                    "callback_group": "command_trigger",
                    "trigger_id": trigger_id,
                    "reply_to_message_id": reply_to_message_id,
                    "handler_payload": handler_payload,
                },
            }))
        }
        "reply_delivery" => {
            let chat_id = object.get("chat_id").cloned().unwrap_or(JsonValue::Null);
            let assistant_event = object_field_or_empty(object, "assistant_event");
            let assistant_sequence = optional_i64(
                assistant_event
                    .as_object()
                    .and_then(|event| event.get("sequence")),
            )
            .unwrap_or(0);
            let through_sequence =
                optional_i64(object.get("through_sequence")).unwrap_or(assistant_sequence);
            let payload = assistant_event
                .as_object()
                .and_then(|event| event.get("payload"))
                .cloned()
                .unwrap_or_else(|| json!({}));

            Ok(json!({
                "stage": "request",
                "adapter_kind": adapter_kind,
                "should_execute": true,
                "expects_result": true,
                "request": {
                    "adapter_kind": adapter_kind,
                    "callback_group": "reply_delivery",
                    "chat_id": chat_id,
                    "assistant_event": assistant_event,
                    "assistant_sequence": assistant_sequence,
                    "through_sequence": through_sequence,
                    "payload": payload,
                },
            }))
        }
        other => Err(format!(
            "unsupported Telegram callback side-effect adapter kind for request boundary: {other}"
        )),
    }
}

fn plan_telegram_side_effect_adapter_result(
    adapter_kind: &str,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let callback_result = object
        .get("callback_result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "callback_result must be a JSON object".to_string())?;
    match adapter_kind {
        "background_sync_pass" => {
            let sent_any = optional_bool(callback_result.get("sent_any")).unwrap_or(false);
            let workflow_notification_sent =
                optional_bool(callback_result.get("workflow_notification_sent")).unwrap_or(false);
            Ok(json!({
                "stage": "result",
                "adapter_kind": adapter_kind,
                "completed": true,
                "result": {
                    "adapter_kind": adapter_kind,
                    "sent_any": sent_any,
                    "workflow_notification_sent": workflow_notification_sent,
                },
            }))
        }
        "command_trigger" => {
            let (handled, reply_text, attachments) =
                normalize_command_trigger_reply_parts(callback_result);
            Ok(json!({
                "stage": "result",
                "adapter_kind": adapter_kind,
                "completed": true,
                "result": {
                    "adapter_kind": adapter_kind,
                    "handled": handled,
                    "reply_text": reply_text,
                    "attachments": attachments,
                    "attachment_count": attachments.len(),
                },
            }))
        }
        "reply_delivery" => {
            let delivered = optional_bool(callback_result.get("delivered")).unwrap_or(false);
            let assistant_sequence =
                optional_i64(callback_result.get("assistant_sequence")).unwrap_or(0);
            let through_sequence =
                optional_i64(callback_result.get("through_sequence")).unwrap_or(assistant_sequence);
            Ok(json!({
                "stage": "result",
                "adapter_kind": adapter_kind,
                "completed": delivered,
                "result": {
                    "adapter_kind": adapter_kind,
                    "delivered": delivered,
                    "assistant_sequence": assistant_sequence,
                    "through_sequence": through_sequence,
                },
            }))
        }
        other => Err(format!(
            "unsupported Telegram callback side-effect adapter kind for result boundary: {other}"
        )),
    }
}

fn plan_telegram_callback_execution_request(
    execution_kind: &str,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let adapter_request = adapter_request_or_empty(object);
    let adapter_object = adapter_request.as_object();
    match execution_kind {
        "background_sync_pass" => {
            let chat_id = adapter_object
                .and_then(|request| request.get("chat_id"))
                .or_else(|| object.get("chat_id"))
                .cloned()
                .unwrap_or(JsonValue::Null);
            let binding = object_field_from(adapter_object, "binding");
            let workflow_notifications_enabled = optional_bool(
                adapter_object.and_then(|request| request.get("workflow_notifications_enabled")),
            )
            .unwrap_or(false);
            let should_execute = optional_bool(object.get("should_execute"))
                .unwrap_or(workflow_notifications_enabled);

            Ok(json!({
                "stage": "request",
                "execution_kind": execution_kind,
                "should_execute": should_execute,
                "expects_result": true,
                "request": {
                    "execution_kind": execution_kind,
                    "callback_group": "background_sync",
                    "operation": "run_background_sync_pass",
                    "adapter_request": adapter_request,
                    "chat_id": chat_id,
                    "binding": binding,
                    "workflow_notifications_enabled": workflow_notifications_enabled,
                },
            }))
        }
        "command_trigger" => {
            let handler_payload = object_field_from(adapter_object, "handler_payload");
            let handler_command = text_array_field(object, "handler_command");
            let repo_root = clean_text(object.get("repo_root"))
                .or_else(|| {
                    clean_text(
                        handler_payload
                            .as_object()
                            .and_then(|payload| payload.get("repo_root")),
                    )
                })
                .unwrap_or_default();
            let trigger_id = clean_text(
                adapter_object
                    .and_then(|request| request.get("trigger_id"))
                    .or_else(|| object.get("trigger_id")),
            );
            let reply_to_message_id = optional_i64(
                adapter_object
                    .and_then(|request| request.get("reply_to_message_id"))
                    .or_else(|| object.get("reply_to_message_id")),
            );
            let should_execute =
                optional_bool(object.get("should_execute")).unwrap_or(!handler_command.is_empty());

            Ok(json!({
                "stage": "request",
                "execution_kind": execution_kind,
                "should_execute": should_execute,
                "expects_result": true,
                "request": {
                    "execution_kind": execution_kind,
                    "callback_group": "command_trigger",
                    "operation": "run_handler",
                    "adapter_request": adapter_request,
                    "trigger_id": trigger_id,
                    "reply_to_message_id": reply_to_message_id,
                    "handler_command": handler_command,
                    "cwd": repo_root,
                    "handler_payload": handler_payload,
                    "stdin_json": handler_payload,
                },
            }))
        }
        "reply_delivery" => {
            let chat_id = adapter_object
                .and_then(|request| request.get("chat_id"))
                .or_else(|| object.get("chat_id"))
                .cloned()
                .unwrap_or(JsonValue::Null);
            let assistant_event = object_field_from(adapter_object, "assistant_event");
            let assistant_sequence = optional_i64(
                adapter_object
                    .and_then(|request| request.get("assistant_sequence"))
                    .or_else(|| {
                        assistant_event
                            .as_object()
                            .and_then(|event| event.get("sequence"))
                    }),
            )
            .unwrap_or(0);
            let through_sequence = optional_i64(
                adapter_object
                    .and_then(|request| request.get("through_sequence"))
                    .or_else(|| object.get("through_sequence")),
            )
            .unwrap_or(assistant_sequence);
            let should_execute = optional_bool(object.get("should_execute")).unwrap_or(true);

            Ok(json!({
                "stage": "request",
                "execution_kind": execution_kind,
                "should_execute": should_execute,
                "expects_result": true,
                "request": {
                    "execution_kind": execution_kind,
                    "callback_group": "reply_delivery",
                    "operation": "send_assistant_event_reply",
                    "adapter_request": adapter_request,
                    "chat_id": chat_id,
                    "assistant_event": assistant_event,
                    "assistant_sequence": assistant_sequence,
                    "through_sequence": through_sequence,
                },
            }))
        }
        other => Err(format!(
            "unsupported Telegram callback execution kind for request boundary: {other}"
        )),
    }
}

fn plan_telegram_callback_execution_result(
    execution_kind: &str,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let callback_result = object
        .get("callback_result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "callback_result must be a JSON object".to_string())?;
    match execution_kind {
        "background_sync_pass" => {
            let sent_any = optional_bool(callback_result.get("sent_any")).unwrap_or(false);
            let workflow_notification_sent =
                optional_bool(callback_result.get("workflow_notification_sent")).unwrap_or(false);
            let error = callback_error(callback_result);
            let ok = error.is_none();
            Ok(json!({
                "stage": "result",
                "execution_kind": execution_kind,
                "completed": ok,
                "result": {
                    "execution_kind": execution_kind,
                    "ok": ok,
                    "error": error,
                    "sent_any": sent_any,
                    "workflow_notification_sent": workflow_notification_sent,
                },
            }))
        }
        "command_trigger" => {
            let returncode = optional_i64(callback_result.get("returncode")).unwrap_or(0);
            let stdout = clean_text(callback_result.get("stdout")).unwrap_or_default();
            let stderr = clean_text(callback_result.get("stderr")).unwrap_or_default();
            let (handler_response, error) =
                normalize_command_execution_response(callback_result, returncode, &stdout, &stderr);
            let response_object = handler_response.as_object();
            let (handled, reply_text, attachments) = response_object
                .map(normalize_command_trigger_reply_parts)
                .unwrap_or((false, String::new(), Vec::new()));
            let ok = error.is_none();

            Ok(json!({
                "stage": "result",
                "execution_kind": execution_kind,
                "completed": ok,
                "result": {
                    "execution_kind": execution_kind,
                    "ok": ok,
                    "error": error,
                    "returncode": returncode,
                    "stdout": stdout,
                    "stderr": stderr,
                    "handler_response": handler_response,
                    "handled": handled,
                    "reply_text": reply_text,
                    "attachments": attachments,
                    "attachment_count": attachments.len(),
                },
            }))
        }
        "reply_delivery" => {
            let delivered = optional_bool(callback_result.get("delivered")).unwrap_or(false);
            let assistant_sequence =
                optional_i64(callback_result.get("assistant_sequence")).unwrap_or(0);
            let through_sequence =
                optional_i64(callback_result.get("through_sequence")).unwrap_or(assistant_sequence);
            let error = callback_error(callback_result);
            let ok = error.is_none() && delivered;
            Ok(json!({
                "stage": "result",
                "execution_kind": execution_kind,
                "completed": ok,
                "result": {
                    "execution_kind": execution_kind,
                    "ok": ok,
                    "error": error,
                    "delivered": delivered,
                    "assistant_sequence": assistant_sequence,
                    "through_sequence": through_sequence,
                },
            }))
        }
        other => Err(format!(
            "unsupported Telegram callback execution kind for result boundary: {other}"
        )),
    }
}

fn plan_telegram_reply_delivery_execution_request(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let execution_request = reply_delivery_execution_request_source(object);
    let execution_object = execution_request.as_object();
    let chat_id = execution_object
        .and_then(|request| request.get("chat_id"))
        .or_else(|| object.get("chat_id"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let execution_assistant_event = object_field_from(execution_object, "assistant_event");
    let assistant_event = if execution_assistant_event
        .as_object()
        .map(|event| event.is_empty())
        .unwrap_or(true)
    {
        object_field_or_empty(object, "assistant_event")
    } else {
        execution_assistant_event
    };
    let assistant_sequence = optional_i64(
        execution_object
            .and_then(|request| request.get("assistant_sequence"))
            .or_else(|| object.get("assistant_sequence"))
            .or_else(|| {
                assistant_event
                    .as_object()
                    .and_then(|event| event.get("sequence"))
            }),
    )
    .unwrap_or(0);
    let through_sequence = optional_i64(
        execution_object
            .and_then(|request| request.get("through_sequence"))
            .or_else(|| object.get("through_sequence")),
    )
    .unwrap_or(assistant_sequence);
    let requested_should_execute =
        optional_bool(object.get("should_execute")).unwrap_or_else(|| {
            execution_object
                .and_then(|request| optional_bool(request.get("should_execute")))
                .unwrap_or(true)
        });
    let reply_text = clean_text(object.get("reply_text"))
        .unwrap_or_else(|| telegram_assistant_reply_text(&assistant_event));
    let attachments = telegram_assistant_reply_attachments(&assistant_event);
    let operations = telegram_reply_delivery_operations(&chat_id, &reply_text, &attachments);
    let error = if requested_should_execute && operations.is_empty() {
        Some("ait-server returned an empty Telegram reply.".to_string())
    } else {
        None
    };
    let should_execute = requested_should_execute && error.is_none();

    Ok(json!({
        "stage": "request",
        "execution_kind": "reply_delivery",
        "delivery_kind": "telegram_assistant_reply",
        "should_execute": should_execute,
        "expects_result": true,
        "request": {
            "execution_kind": "reply_delivery",
            "delivery_kind": "telegram_assistant_reply",
            "callback_group": "reply_delivery",
            "operation": "deliver_assistant_reply",
            "ok": error.is_none(),
            "error": error,
            "execution_request": execution_request,
            "chat_id": chat_id,
            "assistant_event": assistant_event,
            "assistant_sequence": assistant_sequence,
            "through_sequence": through_sequence,
            "reply_text": reply_text,
            "attachments": attachments,
            "attachment_count": attachments.len(),
            "operations": operations,
            "operation_count": operations.len(),
        },
    }))
}

fn plan_telegram_reply_delivery_execution_result(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let callback_result = object
        .get("callback_result")
        .or_else(|| object.get("result"))
        .and_then(JsonValue::as_object)
        .unwrap_or(object);
    let operation_results = callback_result
        .get("operation_results")
        .or_else(|| callback_result.get("delivery_operation_results"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let operation_count = optional_i64(callback_result.get("operation_count"))
        .unwrap_or(operation_results.len() as i64);
    let assistant_sequence = optional_i64(callback_result.get("assistant_sequence")).unwrap_or(0);
    let through_sequence =
        optional_i64(callback_result.get("through_sequence")).unwrap_or(assistant_sequence);
    let delivered_operation_count = operation_results
        .iter()
        .filter(|result| {
            result
                .as_object()
                .and_then(|result| optional_bool(result.get("ok")))
                .unwrap_or(false)
        })
        .count() as i64;
    let failed_operation_count = operation_results
        .iter()
        .filter(|result| {
            result
                .as_object()
                .map(|result| !optional_bool(result.get("ok")).unwrap_or(false))
                .unwrap_or(true)
        })
        .count() as i64;
    let operation_error = operation_results.iter().find_map(|result| {
        let result = result.as_object()?;
        if optional_bool(result.get("ok")).unwrap_or(false) {
            None
        } else {
            clean_text(result.get("error"))
        }
    });
    let error = callback_error(callback_result).or_else(|| {
        if failed_operation_count > 0 {
            Some(
                operation_error
                    .unwrap_or_else(|| "Telegram reply delivery operation failed.".to_string()),
            )
        } else {
            None
        }
    });
    let delivered = optional_bool(callback_result.get("delivered"))
        .unwrap_or(error.is_none() && operation_count > 0 && failed_operation_count == 0);
    let ok = error.is_none() && delivered;

    Ok(json!({
        "stage": "result",
        "execution_kind": "reply_delivery",
        "delivery_kind": "telegram_assistant_reply",
        "completed": ok,
        "result": {
            "execution_kind": "reply_delivery",
            "delivery_kind": "telegram_assistant_reply",
            "ok": ok,
            "error": error,
            "delivered": delivered,
            "assistant_sequence": assistant_sequence,
            "through_sequence": through_sequence,
            "operation_results": operation_results,
            "operation_count": operation_count,
            "delivered_operation_count": delivered_operation_count,
            "failed_operation_count": failed_operation_count,
        },
    }))
}

fn plan_telegram_command_trigger_execution_request(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let execution_request = command_trigger_execution_request_source(object);
    let execution_object = execution_request.as_object();
    let adapter_request = object_field_from(execution_object, "adapter_request");
    let adapter_object = adapter_request.as_object();
    let handler_payload = non_empty_object_field_from(execution_object, "handler_payload")
        .or_else(|| non_empty_object_field_from(execution_object, "stdin_json"))
        .or_else(|| non_empty_object_field_from(adapter_object, "handler_payload"))
        .unwrap_or_else(|| json!({}));
    let handler_command = text_array_field_from(execution_object, "handler_command")
        .unwrap_or_else(|| text_array_field(object, "handler_command"));
    let repo_root = clean_text(
        execution_object
            .and_then(|request| request.get("repo_root"))
            .or_else(|| object.get("repo_root"))
            .or_else(|| {
                handler_payload
                    .as_object()
                    .and_then(|payload| payload.get("repo_root"))
            }),
    );
    let cwd = clean_text(
        execution_object
            .and_then(|request| request.get("cwd"))
            .or_else(|| object.get("cwd")),
    )
    .or_else(|| repo_root.clone())
    .unwrap_or_default();
    let trigger_id = clean_text(
        execution_object
            .and_then(|request| request.get("trigger_id"))
            .or_else(|| adapter_object.and_then(|request| request.get("trigger_id")))
            .or_else(|| object.get("trigger_id")),
    );
    let reply_to_message_id = optional_i64(
        execution_object
            .and_then(|request| request.get("reply_to_message_id"))
            .or_else(|| adapter_object.and_then(|request| request.get("reply_to_message_id")))
            .or_else(|| object.get("reply_to_message_id")),
    );
    let requested_should_execute =
        optional_bool(object.get("should_execute")).unwrap_or_else(|| {
            execution_object
                .and_then(|request| optional_bool(request.get("should_execute")))
                .unwrap_or(true)
        });
    let error = if requested_should_execute && handler_command.is_empty() {
        Some("Operational trigger handler command is empty.".to_string())
    } else {
        None
    };
    let should_execute = requested_should_execute && error.is_none();
    let operation = if should_execute {
        json!({
            "kind": "run_handler",
            "method": "subprocess.run",
            "trigger_id": trigger_id,
            "reply_to_message_id": reply_to_message_id,
            "handler_command": handler_command,
            "cwd": cwd,
            "repo_root": repo_root,
            "stdin_json": handler_payload,
            "env_overrides": command_trigger_env_overrides(repo_root.as_deref()),
            "pythonpath_repo_src": command_trigger_repo_src(repo_root.as_deref()),
        })
    } else {
        JsonValue::Null
    };
    let operations = if operation.is_null() {
        Vec::new()
    } else {
        vec![operation]
    };

    Ok(json!({
        "stage": "request",
        "execution_kind": "command_trigger",
        "trigger_kind": "telegram_operational_trigger",
        "should_execute": should_execute,
        "expects_result": true,
        "request": {
            "execution_kind": "command_trigger",
            "trigger_kind": "telegram_operational_trigger",
            "callback_group": "command_trigger",
            "operation": "run_handler",
            "ok": error.is_none(),
            "error": error,
            "execution_request": execution_request,
            "adapter_request": adapter_request,
            "trigger_id": trigger_id,
            "reply_to_message_id": reply_to_message_id,
            "handler_command": handler_command,
            "cwd": cwd,
            "repo_root": repo_root,
            "handler_payload": handler_payload,
            "stdin_json": handler_payload,
            "env_overrides": command_trigger_env_overrides(repo_root.as_deref()),
            "pythonpath_repo_src": command_trigger_repo_src(repo_root.as_deref()),
            "operations": operations,
            "operation_count": operations.len(),
        },
    }))
}

fn plan_telegram_command_trigger_execution_result(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let callback_result = object
        .get("callback_result")
        .or_else(|| object.get("result"))
        .and_then(JsonValue::as_object)
        .unwrap_or(object);
    let operation_results = callback_result
        .get("operation_results")
        .or_else(|| callback_result.get("handler_operation_results"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let operation_source = operation_results
        .iter()
        .find_map(JsonValue::as_object)
        .unwrap_or(callback_result);
    let operation_count = optional_i64(callback_result.get("operation_count"))
        .unwrap_or(operation_results.len() as i64);
    let returncode = optional_i64(
        callback_result
            .get("returncode")
            .or_else(|| operation_source.get("returncode")),
    )
    .unwrap_or(0);
    let stdout = clean_text(
        callback_result
            .get("stdout")
            .or_else(|| operation_source.get("stdout")),
    )
    .unwrap_or_default();
    let stderr = clean_text(
        callback_result
            .get("stderr")
            .or_else(|| operation_source.get("stderr")),
    )
    .unwrap_or_default();
    let mut response_payload = callback_result.clone();
    if !response_payload.contains_key("handler_response") {
        if let Some(response) = operation_source
            .get("handler_response")
            .or_else(|| operation_source.get("response"))
        {
            response_payload.insert("handler_response".to_string(), response.clone());
        }
    }
    if !response_payload.contains_key("response") {
        if let Some(response) = operation_source.get("response") {
            response_payload.insert("response".to_string(), response.clone());
        }
    }
    let (handler_response, mut error) =
        normalize_command_execution_response(&response_payload, returncode, &stdout, &stderr);
    if error.is_none() {
        error = operation_results.iter().find_map(|result| {
            let result = result.as_object()?;
            if optional_bool(result.get("ok")).unwrap_or(false) {
                None
            } else {
                clean_text(result.get("error")).or_else(|| {
                    Some("Telegram command trigger handler operation failed.".to_string())
                })
            }
        });
    }
    let response_object = handler_response.as_object();
    let (handled, reply_text, attachments) = response_object
        .map(normalize_command_trigger_reply_parts)
        .unwrap_or((false, String::new(), Vec::new()));
    let ok = error.is_none();
    let completed_operation_count = if operation_results.is_empty() {
        i64::from(ok)
    } else {
        operation_results
            .iter()
            .filter(|result| {
                result
                    .as_object()
                    .and_then(|result| optional_bool(result.get("ok")))
                    .unwrap_or(false)
            })
            .count() as i64
    };
    let failed_operation_count = if operation_results.is_empty() {
        i64::from(!ok)
    } else {
        operation_results
            .iter()
            .filter(|result| {
                result
                    .as_object()
                    .map(|result| !optional_bool(result.get("ok")).unwrap_or(false))
                    .unwrap_or(true)
            })
            .count() as i64
    };

    Ok(json!({
        "stage": "result",
        "execution_kind": "command_trigger",
        "trigger_kind": "telegram_operational_trigger",
        "completed": ok,
        "result": {
            "execution_kind": "command_trigger",
            "trigger_kind": "telegram_operational_trigger",
            "ok": ok,
            "error": error,
            "returncode": returncode,
            "stdout": stdout,
            "stderr": stderr,
            "handler_response": handler_response,
            "handled": handled,
            "reply_text": reply_text,
            "attachments": attachments,
            "attachment_count": attachments.len(),
            "operation_results": operation_results,
            "operation_count": operation_count,
            "completed_operation_count": completed_operation_count,
            "failed_operation_count": failed_operation_count,
        },
    }))
}

pub(in crate::event_loop) fn plan_telegram_update_batch_dispatch(
    updates: &[JsonValue],
    fallback_update_keys: Option<&Vec<JsonValue>>,
) -> Result<(Vec<JsonValue>, i64), String> {
    let mut last_update_id = 0_i64;
    let mut dispatch_items = Vec::with_capacity(updates.len());

    for (index, update) in updates.iter().enumerate() {
        let fallback_update_key = fallback_update_key_at(fallback_update_keys, index)
            .unwrap_or_else(|| format!("memory-unknown-{index}"));
        let planned = plan_telegram_update_dispatch(update, &fallback_update_key)?;
        if planned.should_update_last_update_id {
            last_update_id = planned.update_id;
        }
        dispatch_items.push(planned.into_indexed_json(index));
    }

    Ok((dispatch_items, last_update_id))
}
