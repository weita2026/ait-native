use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::telegram_polling::agent_telegram_update_dispatch_plan_json;

const MIGRATION_STAGE: &str = "rust_agent_telegram_submission_runtime";
const SUBMISSION_RUNTIME_CONTRACT: &str = "ait_agent_core.event_loop.TelegramSubmissionRuntime.v1";

pub trait TelegramSubmissionRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramSubmissionRuntimePlanner;

impl TelegramSubmissionRuntimePlanner for DefaultTelegramSubmissionRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_telegram_submission_runtime_json(request)
    }
}

pub fn agent_telegram_submission_runtime_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_telegram_submission_runtime_planner(&DefaultTelegramSubmissionRuntimePlanner, request)
}

pub fn plan_with_telegram_submission_runtime_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramSubmissionRuntimePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_telegram_submission_runtime_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object
        .get("stage")
        .and_then(JsonValue::as_str)
        .unwrap_or("submit_update");

    match stage {
        "submit_update" => plan_submit_update(object),
        "submit_planned_update" => plan_submit_planned_update(object),
        "submit_background_sync" | "submit_background_sync_for_chat" => {
            plan_submit_background_sync_for_chat(object)
        }
        "submit_reply_serialized" => plan_submit_reply_serialized(object),
        "wait_for_idle" => plan_wait_for_idle(object),
        "forget_future" => plan_forget_future(object),
        other => Err(format!(
            "unsupported Telegram submission runtime stage: {other}"
        )),
    }
}

fn plan_submit_update(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let update = object
        .get("update")
        .cloned()
        .ok_or_else(|| "update is required".to_string())?;
    if !update.is_object() {
        return Err("update must be a JSON object".to_string());
    }
    let fallback_update_key = clean_text(object.get("fallback_update_key"))
        .unwrap_or_else(|| "memory-unknown".to_string());
    let dispatch_item = agent_telegram_update_dispatch_plan_json(&json!({
        "update": update.clone(),
        "fallback_update_key": fallback_update_key,
    }))?;
    let queue_key = required_text_field(
        dispatch_item.as_object(),
        "dispatch_key",
        "dispatch plan dispatch_key is required",
    )?;
    let logical_turn_merge_enabled =
        optional_bool(object.get("logical_turn_merge_enabled")).unwrap_or(false);
    let rejected = rejection_reasons(object);
    if !rejected.is_empty() {
        return Ok(rejected_submission("submit_update", rejected));
    }

    Ok(planned_submission(
        "submit_update",
        queue_key.clone(),
        dispatch_item,
        json!({
            "kind": "submit_serialized",
            "callback": "handle_submitted_update",
            "callback_group": "update",
            "queue_key": queue_key,
            "args": [update.clone()],
        }),
        if logical_turn_merge_enabled {
            vec![json!({
                "kind": "buffer_submitted_text_update",
                "update": update,
            })]
        } else {
            Vec::new()
        },
        json!({
            "logical_turn_merge_enabled": logical_turn_merge_enabled,
            "should_buffer_submitted_text_update": logical_turn_merge_enabled,
        }),
    ))
}

fn plan_submit_planned_update(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let update = object
        .get("update")
        .cloned()
        .ok_or_else(|| "update is required".to_string())?;
    if !update.is_object() {
        return Err("update must be a JSON object".to_string());
    }
    let dispatch_item = object
        .get("dispatch_item")
        .cloned()
        .ok_or_else(|| "dispatch_item is required".to_string())?;
    let queue_key = required_text_field(
        dispatch_item.as_object(),
        "dispatch_key",
        "dispatch_item.dispatch_key is required",
    )?;
    let logical_turn_merge_enabled =
        optional_bool(object.get("logical_turn_merge_enabled")).unwrap_or(false);
    let rejected = rejection_reasons(object);
    if !rejected.is_empty() {
        return Ok(rejected_submission("submit_planned_update", rejected));
    }

    Ok(planned_submission(
        "submit_planned_update",
        queue_key.clone(),
        dispatch_item,
        json!({
            "kind": "submit_serialized",
            "callback": "handle_submitted_update",
            "callback_group": "update",
            "queue_key": queue_key,
            "args": [update.clone()],
        }),
        if logical_turn_merge_enabled {
            vec![json!({
                "kind": "buffer_submitted_text_update",
                "update": update,
            })]
        } else {
            Vec::new()
        },
        json!({
            "logical_turn_merge_enabled": logical_turn_merge_enabled,
            "should_buffer_submitted_text_update": logical_turn_merge_enabled,
        }),
    ))
}

fn plan_submit_background_sync_for_chat(
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let chat_id = object
        .get("chat_id")
        .cloned()
        .ok_or_else(|| "chat_id is required".to_string())?;
    let chat_id_text = pythonish_text(&chat_id);
    if chat_id_text.trim().is_empty() || chat_id_text == "None" {
        return Err("chat_id is required".to_string());
    }
    let queue_key =
        clean_text(object.get("queue_key")).unwrap_or_else(|| format!("chat-{chat_id_text}"));
    let rejected = rejection_reasons(object);
    if !rejected.is_empty() {
        return Ok(rejected_submission(
            "submit_background_sync_for_chat",
            rejected,
        ));
    }

    Ok(planned_submission(
        "submit_background_sync_for_chat",
        queue_key.clone(),
        json!({
            "chat_id": chat_id.clone(),
            "dispatch_key": queue_key.clone(),
        }),
        json!({
            "kind": "submit_serialized",
            "callback": "run_background_sync_for_chat",
            "callback_group": "background_sync",
            "queue_key": queue_key,
            "args": [chat_id_text.clone()],
        }),
        Vec::new(),
        json!({
            "chat_id": chat_id,
            "chat_id_text": chat_id_text,
        }),
    ))
}

fn plan_submit_reply_serialized(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let queue_key =
        clean_text(object.get("queue_key")).ok_or_else(|| "queue_key is required".to_string())?;
    let callback_slot =
        clean_text(object.get("callback_slot")).unwrap_or_else(|| "reply_callback".to_string());
    let args = object
        .get("args")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let rejected = rejection_reasons(object);
    if !rejected.is_empty() {
        return Ok(rejected_submission("submit_reply_serialized", rejected));
    }

    Ok(planned_submission(
        "submit_reply_serialized",
        queue_key.clone(),
        json!({
            "dispatch_key": queue_key.clone(),
            "reply_dispatch": true,
        }),
        json!({
            "kind": "submit_reply_serialized",
            "callback": callback_slot.clone(),
            "callback_group": "reply",
            "queue_key": queue_key,
            "args": args.clone(),
        }),
        Vec::new(),
        json!({
            "callback_slot": callback_slot,
            "reply_dispatch": true,
        }),
    ))
}

fn plan_wait_for_idle(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let service_runtime_idle = optional_bool(object.get("service_runtime_idle")).unwrap_or(false);
    let live_reply_manager_idle =
        optional_bool(object.get("live_reply_manager_idle")).unwrap_or(false);
    let timeout_seconds = object
        .get("timeout_seconds")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let checked_live_reply_manager = service_runtime_idle;
    let idle = service_runtime_idle && live_reply_manager_idle;

    Ok(json!({
        "migration_stage": MIGRATION_STAGE,
        "submission_runtime_contract": SUBMISSION_RUNTIME_CONTRACT,
        "stage": "wait_for_idle",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_submission_allowed": false,
        "service_runtime_dispatch_port_required": true,
        "submission_state": if idle { "idle" } else { "busy" },
        "idle": idle,
        "service_runtime_idle": service_runtime_idle,
        "live_reply_manager_idle": live_reply_manager_idle,
        "checked_service_runtime_first": true,
        "checked_live_reply_manager": checked_live_reply_manager,
        "timeout_seconds": timeout_seconds.clone(),
        "actions": [
            {
                "kind": "wait_service_runtime_idle",
                "timeout_seconds": timeout_seconds.clone(),
            },
            {
                "kind": "wait_live_reply_manager_idle",
                "enabled": checked_live_reply_manager,
                "timeout_seconds": timeout_seconds,
            }
        ],
        "rejection_reasons": [],
    }))
}

fn plan_forget_future(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let future_token = object
        .get("future_token")
        .cloned()
        .unwrap_or_else(|| json!("future"));

    Ok(json!({
        "migration_stage": MIGRATION_STAGE,
        "submission_runtime_contract": SUBMISSION_RUNTIME_CONTRACT,
        "stage": "forget_future",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_submission_allowed": false,
        "service_runtime_dispatch_port_required": true,
        "submission_state": "planned",
        "should_submit": false,
        "future_token": future_token.clone(),
        "actions": [
            {
                "kind": "forget_future",
                "future_token": future_token,
            }
        ],
        "rejection_reasons": [],
    }))
}

fn planned_submission(
    stage: &str,
    queue_key: String,
    dispatch_item: JsonValue,
    submit_action: JsonValue,
    mut pre_submit_actions: Vec<JsonValue>,
    details: JsonValue,
) -> JsonValue {
    pre_submit_actions.push(submit_action.clone());
    json!({
        "migration_stage": MIGRATION_STAGE,
        "submission_runtime_contract": SUBMISSION_RUNTIME_CONTRACT,
        "stage": stage,
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_submission_allowed": false,
        "service_runtime_dispatch_port_required": true,
        "submission_state": "planned",
        "should_submit": true,
        "queue_key": queue_key,
        "dispatch_item": dispatch_item,
        "submit_action": submit_action,
        "actions": pre_submit_actions,
        "details": details,
        "rejection_reasons": [],
    })
}

fn rejected_submission(stage: &str, rejection_reasons: Vec<JsonValue>) -> JsonValue {
    json!({
        "migration_stage": MIGRATION_STAGE,
        "submission_runtime_contract": SUBMISSION_RUNTIME_CONTRACT,
        "stage": stage,
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_submission_allowed": false,
        "service_runtime_dispatch_port_required": true,
        "submission_state": "rejected",
        "should_submit": false,
        "actions": [],
        "rejection_reasons": rejection_reasons,
    })
}

fn rejection_reasons(object: &Map<String, JsonValue>) -> Vec<JsonValue> {
    let accepting = optional_bool(object.get("service_runtime_accepting_submissions"))
        .unwrap_or_else(|| !optional_bool(object.get("service_runtime_stopped")).unwrap_or(false));
    if accepting {
        Vec::new()
    } else {
        vec![json!({
            "kind": "service_runtime_stopped",
            "message": "Telegram service runtime is stopped; submission fallback is rejected.",
        })]
    }
}

fn required_text_field(
    object: Option<&Map<String, JsonValue>>,
    key: &str,
    error: &str,
) -> Result<String, String> {
    clean_text(object.and_then(|object| object.get(key))).ok_or_else(|| error.to_string())
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = value?.as_str()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
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

fn pythonish_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "None".to_string(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => "False".to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::String(text) => text.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
    }
}

#[cfg(test)]
mod tests;
