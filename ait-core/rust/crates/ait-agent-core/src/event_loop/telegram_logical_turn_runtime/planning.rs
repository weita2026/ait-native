use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_telegram_logical_turn_runtime";
const LOGICAL_TURN_RUNTIME_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramLogicalTurnRuntime.v1";

pub trait TelegramLogicalTurnRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramLogicalTurnRuntimePlanner;

impl TelegramLogicalTurnRuntimePlanner for DefaultTelegramLogicalTurnRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_logical_turn_runtime_json(request)
    }
}

pub fn agent_telegram_logical_turn_runtime_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_telegram_logical_turn_runtime_planner(
        &DefaultTelegramLogicalTurnRuntimePlanner,
        request,
    )
}

pub fn plan_with_telegram_logical_turn_runtime_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramLogicalTurnRuntimePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_logical_turn_runtime_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = object
        .get("stage")
        .and_then(JsonValue::as_str)
        .unwrap_or("claim_logical_turn");

    match stage {
        "merge_enabled" | "logical_turn_merge_enabled" => plan_merge_enabled(object),
        "candidate_metadata" | "classify_pending_text_update" => plan_candidate_metadata(object),
        "buffer_submitted_text_update" => plan_buffer_submitted_text_update(object),
        "discard_buffered_text_update" => plan_discard_buffered_text_update(object),
        "claim_logical_turn" => plan_claim_logical_turn(object),
        other => Err(format!(
            "unsupported Telegram logical-turn runtime stage: {other}"
        )),
    }
}

fn plan_merge_enabled(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let merge_window_seconds = optional_f64(object.get("merge_window_seconds")).unwrap_or(0.0);
    let max_messages = optional_i64(object.get("max_messages")).unwrap_or(0);
    let enabled = merge_window_seconds > 0.0 && max_messages > 1;
    Ok(json!({
        "migration_stage": MIGRATION_STAGE,
        "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
        "stage": "merge_enabled",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_logical_turn_allowed": false,
        "logical_turn_state": if enabled { "enabled" } else { "disabled" },
        "merge_enabled": enabled,
        "merge_window_seconds": merge_window_seconds,
        "max_messages": max_messages,
        "actions": [],
    }))
}

fn plan_candidate_metadata(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let requested_stage = object
        .get("stage")
        .and_then(JsonValue::as_str)
        .unwrap_or("candidate_metadata");
    let update = object
        .get("update")
        .cloned()
        .ok_or_else(|| "update is required".to_string())?;
    if !update.is_object() {
        return Err("update must be a JSON object".to_string());
    }
    let message = update.get("message").and_then(JsonValue::as_object);
    let text = message
        .and_then(|message| message.get("text"))
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let chat_id = message
        .and_then(|message| message.get("chat"))
        .and_then(JsonValue::as_object)
        .and_then(|chat| chat.get("id"))
        .cloned();
    let telegram_message_id = message
        .and_then(|message| message.get("message_id"))
        .and_then(json_to_i64);
    let is_text_candidate = !text.trim().is_empty() && chat_id.is_some();
    let chat_id_text = chat_id.as_ref().map(pythonish_text);
    let classification_requested = requested_stage == "classify_pending_text_update"
        || object.contains_key("update_key")
        || object.contains_key("chat_key")
        || object.contains_key("normalized_text")
        || object.contains_key("actor_identity")
        || object.contains_key("command")
        || object.contains_key("command_present")
        || object.contains_key("workflow_query")
        || object.contains_key("workflow_query_present");
    let mut command_present = false;
    let mut workflow_query_present = false;
    let mut mergeable = false;
    let candidate = if is_text_candidate && classification_requested {
        let update_key = required_request_text(
            object,
            "update_key",
            "update_key is required for logical-turn candidate classification",
        )?;
        let chat_key = required_request_text(
            object,
            "chat_key",
            "chat_key is required for logical-turn candidate classification",
        )?;
        let normalized_text = clean_text(object.get("normalized_text")).unwrap_or_default();
        let actor_identity = clean_text(object.get("actor_identity")).unwrap_or_default();
        let received_at = optional_f64(object.get("received_at")).unwrap_or(0.0);
        command_present = optional_bool(object.get("command_present")).unwrap_or(false)
            || value_is_present(object.get("command"));
        workflow_query_present = optional_bool(object.get("workflow_query_present"))
            .unwrap_or(false)
            || value_is_present(object.get("workflow_query"));
        mergeable = !normalized_text.is_empty() && !command_present && !workflow_query_present;
        json!({
            "update_key": update_key,
            "chat_key": chat_key,
            "normalized_text": normalized_text,
            "mergeable": mergeable,
            "actor_identity": actor_identity,
            "received_at": received_at,
            "telegram_message_id": telegram_message_id,
        })
    } else {
        JsonValue::Null
    };

    Ok(json!({
        "migration_stage": MIGRATION_STAGE,
        "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
        "stage": requested_stage,
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_logical_turn_allowed": false,
        "logical_turn_state": if is_text_candidate {
            if classification_requested { "classified_candidate" } else { "candidate" }
        } else {
            "not_candidate"
        },
        "is_text_candidate": is_text_candidate,
        "raw_text": if is_text_candidate { JsonValue::String(text.to_string()) } else { JsonValue::Null },
        "chat_id": chat_id.unwrap_or(JsonValue::Null),
        "chat_id_text": chat_id_text.unwrap_or_default(),
        "telegram_message_id": telegram_message_id,
        "candidate": candidate,
        "mergeable": mergeable,
        "command_present": command_present,
        "workflow_query_present": workflow_query_present,
        "actions": [
            {
                "kind": "classify_pending_text_update",
                "is_text_candidate": is_text_candidate,
            }
        ],
    }))
}

fn plan_buffer_submitted_text_update(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let Some(candidate) = optional_object(object.get("candidate")) else {
        return Ok(json!({
            "migration_stage": MIGRATION_STAGE,
            "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
            "stage": "buffer_submitted_text_update",
            "transport": "telegram",
            "rust_event_loop_required": true,
            "python_logical_turn_allowed": false,
            "logical_turn_state": "ignored",
            "should_append": false,
            "duplicate": false,
            "actions": [],
        }));
    };
    let update_key = required_text(candidate, "update_key", "candidate.update_key is required")?;
    let chat_key = required_text(candidate, "chat_key", "candidate.chat_key is required")?;
    let queue = object
        .get("queue")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let duplicate = queue.iter().any(|entry| {
        entry
            .get("update_key")
            .and_then(JsonValue::as_str)
            .map(|value| value == update_key)
            .unwrap_or(false)
    });
    let should_append = !duplicate;

    Ok(json!({
        "migration_stage": MIGRATION_STAGE,
        "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
        "stage": "buffer_submitted_text_update",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_logical_turn_allowed": false,
        "logical_turn_state": if should_append { "append" } else { "duplicate" },
        "should_append": should_append,
        "duplicate": duplicate,
        "chat_key": chat_key,
        "update_key": update_key,
        "queue_len": queue.len(),
        "actions": if should_append {
            vec![json!({
                "kind": "append_pending_text_update",
                "chat_key": chat_key,
                "update_key": update_key,
            })]
        } else {
            Vec::new()
        },
    }))
}

fn plan_discard_buffered_text_update(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let candidate = optional_object(object.get("candidate"))
        .ok_or_else(|| "candidate is required for buffered update discard".to_string())?;
    let update_key = required_text(candidate, "update_key", "candidate.update_key is required")?;
    let chat_key = required_text(candidate, "chat_key", "candidate.chat_key is required")?;
    let queue = object
        .get("queue")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let current_index = queue.iter().position(|entry| {
        entry
            .get("update_key")
            .and_then(JsonValue::as_str)
            .is_some_and(|value| value == update_key)
    });
    let should_remove = current_index.is_some();

    Ok(json!({
        "migration_stage": MIGRATION_STAGE,
        "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
        "stage": "discard_buffered_text_update",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_logical_turn_allowed": false,
        "logical_turn_state": if should_remove { "discard" } else { "missing" },
        "should_remove": should_remove,
        "chat_key": chat_key,
        "update_key": update_key,
        "current_index": current_index,
        "actions": if should_remove {
            vec![json!({
                "kind": "discard_pending_text_update",
                "chat_key": chat_key,
                "update_key": update_key,
                "current_index": current_index,
            })]
        } else {
            Vec::new()
        },
    }))
}

fn plan_claim_logical_turn(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let Some(candidate) = optional_object(object.get("candidate")) else {
        return Ok(no_claim_result("not_candidate", "none", Vec::new()));
    };
    let update_key = required_text(candidate, "update_key", "candidate.update_key is required")?;
    let chat_key = required_text(candidate, "chat_key", "candidate.chat_key is required")?;
    let queue = object
        .get("queue")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let Some(current_index) = queue.iter().position(|entry| {
        entry
            .get("update_key")
            .and_then(JsonValue::as_str)
            .map(|value| value == update_key)
            .unwrap_or(false)
    }) else {
        return Ok(no_claim_result(
            "missing",
            "skip",
            vec![json!({
                "kind": "skip_missing_logical_turn",
                "chat_key": chat_key,
                "update_key": update_key,
            })],
        ));
    };

    let first = entry_object(&queue[current_index], current_index)?;
    if !optional_bool(first.get("mergeable")).unwrap_or(false) {
        return Ok(json!({
            "migration_stage": MIGRATION_STAGE,
            "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
            "stage": "claim_logical_turn",
            "transport": "telegram",
            "rust_event_loop_required": true,
            "python_logical_turn_allowed": false,
            "logical_turn_state": "non_mergeable",
            "return_kind": "pass_through",
            "chat_key": chat_key,
            "update_key": update_key,
            "current_index": current_index,
            "should_remove": true,
            "should_emit": false,
            "should_wait": false,
            "actions": [
                {
                    "kind": "remove_pending_text_update",
                    "chat_key": chat_key,
                    "update_key": update_key,
                }
            ],
        }));
    }

    let merge_window_seconds = optional_f64(object.get("merge_window_seconds")).unwrap_or(0.0);
    let poll_interval_seconds = optional_f64(object.get("poll_interval_seconds")).unwrap_or(0.0);
    let max_messages = optional_usize(object.get("max_messages"))
        .unwrap_or(1)
        .max(1);
    let now_monotonic_seconds = optional_f64(object.get("now_monotonic_seconds")).unwrap_or(0.0);
    let selected = select_mergeable_entries(&queue, current_index, max_messages)?;
    let latest_received_at = selected
        .iter()
        .filter_map(|entry| optional_f64(entry.get("received_at")))
        .fold(0.0, f64::max);
    let quiet_elapsed_seconds = (now_monotonic_seconds - latest_received_at).max(0.0);
    let reached_limit = selected.len() >= max_messages;
    let boundary_seen = selected_boundary_seen(&queue, current_index, selected.len())?;

    if boundary_seen || reached_limit || quiet_elapsed_seconds >= merge_window_seconds {
        return Ok(emit_logical_turn_result(
            chat_key,
            update_key,
            current_index,
            &selected,
            boundary_seen,
            reached_limit,
            quiet_elapsed_seconds,
        ));
    }

    let sleep_for_seconds = (merge_window_seconds - quiet_elapsed_seconds)
        .max(0.0)
        .min(poll_interval_seconds);
    Ok(json!({
        "migration_stage": MIGRATION_STAGE,
        "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
        "stage": "claim_logical_turn",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_logical_turn_allowed": false,
        "logical_turn_state": "wait",
        "return_kind": "wait",
        "chat_key": chat_key,
        "update_key": update_key,
        "current_index": current_index,
        "selected_count": selected.len(),
        "boundary_seen": boundary_seen,
        "reached_limit": reached_limit,
        "quiet_elapsed_seconds": quiet_elapsed_seconds,
        "sleep_for_seconds": sleep_for_seconds,
        "should_remove": false,
        "should_emit": false,
        "should_wait": true,
        "actions": [
            {
                "kind": "wait_for_quiet_window",
                "chat_key": chat_key,
                "update_key": update_key,
                "sleep_for_seconds": sleep_for_seconds,
            }
        ],
    }))
}

fn select_mergeable_entries(
    queue: &[JsonValue],
    current_index: usize,
    max_messages: usize,
) -> Result<Vec<Map<String, JsonValue>>, String> {
    let first = entry_object(&queue[current_index], current_index)?;
    let first_actor = first
        .get("actor_identity")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();
    let mut selected = Vec::new();
    for (relative_index, item) in queue[current_index..].iter().enumerate() {
        let index = current_index + relative_index;
        let entry = entry_object(item, index)?;
        if !optional_bool(entry.get("mergeable")).unwrap_or(false) {
            break;
        }
        let actor = entry
            .get("actor_identity")
            .and_then(JsonValue::as_str)
            .unwrap_or("");
        if actor != first_actor {
            break;
        }
        selected.push(entry.clone());
        if selected.len() >= max_messages {
            break;
        }
    }
    Ok(selected)
}

fn selected_boundary_seen(
    queue: &[JsonValue],
    current_index: usize,
    selected_count: usize,
) -> Result<bool, String> {
    let Some(next) = queue.get(current_index + selected_count) else {
        return Ok(false);
    };
    let first = entry_object(&queue[current_index], current_index)?;
    let first_actor = first
        .get("actor_identity")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    let next = entry_object(next, current_index + selected_count)?;
    let next_mergeable = optional_bool(next.get("mergeable")).unwrap_or(false);
    let next_actor = next
        .get("actor_identity")
        .and_then(JsonValue::as_str)
        .unwrap_or("");
    Ok(!next_mergeable || next_actor != first_actor)
}

fn emit_logical_turn_result(
    chat_key: &str,
    update_key: &str,
    current_index: usize,
    selected: &[Map<String, JsonValue>],
    boundary_seen: bool,
    reached_limit: bool,
    quiet_elapsed_seconds: f64,
) -> JsonValue {
    let texts: Vec<String> = selected
        .iter()
        .filter_map(|entry| clean_text(entry.get("normalized_text")))
        .collect();
    let message_ids: Vec<i64> = selected
        .iter()
        .filter_map(|entry| entry.get("telegram_message_id").and_then(json_to_i64))
        .collect();
    let telegram_message_id = message_ids.last().cloned();
    let text = texts.join("\n\n").trim().to_string();
    let actor_identity = selected
        .first()
        .and_then(|entry| entry.get("actor_identity"))
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .to_string();

    json!({
        "migration_stage": MIGRATION_STAGE,
        "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
        "stage": "claim_logical_turn",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_logical_turn_allowed": false,
        "logical_turn_state": "emit",
        "return_kind": "logical_turn",
        "chat_key": chat_key,
        "update_key": update_key,
        "current_index": current_index,
        "consume_count": selected.len(),
        "selected_count": selected.len(),
        "boundary_seen": boundary_seen,
        "reached_limit": reached_limit,
        "quiet_elapsed_seconds": quiet_elapsed_seconds,
        "should_remove": true,
        "should_emit": true,
        "should_wait": false,
        "logical_turn": {
            "text": text,
            "actor_identity": actor_identity,
            "telegram_message_id": telegram_message_id,
            "telegram_message_ids": message_ids,
        },
        "actions": [
            {
                "kind": "consume_logical_turn",
                "chat_key": chat_key,
                "update_key": update_key,
                "start_index": current_index,
                "consume_count": selected.len(),
            },
            {
                "kind": "build_logical_turn",
                "chat_key": chat_key,
                "update_key": update_key,
                "text": text,
                "actor_identity": actor_identity,
                "telegram_message_id": telegram_message_id,
                "telegram_message_ids": message_ids,
            }
        ],
    })
}

fn no_claim_result(state: &str, return_kind: &str, actions: Vec<JsonValue>) -> JsonValue {
    json!({
        "migration_stage": MIGRATION_STAGE,
        "logical_turn_runtime_contract": LOGICAL_TURN_RUNTIME_CONTRACT,
        "stage": "claim_logical_turn",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_logical_turn_allowed": false,
        "logical_turn_state": state,
        "return_kind": return_kind,
        "should_remove": false,
        "should_emit": false,
        "should_wait": false,
        "actions": actions,
    })
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())
}

fn optional_object(value: Option<&JsonValue>) -> Option<&Map<String, JsonValue>> {
    value.and_then(JsonValue::as_object)
}

fn entry_object(value: &JsonValue, index: usize) -> Result<&Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("queue entry at index {index} must be an object"))
}

fn required_text<'a>(
    object: &'a Map<String, JsonValue>,
    key: &str,
    error: &str,
) -> Result<&'a str, String> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error.to_string())
}

fn required_request_text(
    object: &Map<String, JsonValue>,
    key: &str,
    error: &str,
) -> Result<String, String> {
    clean_text(object.get(key)).ok_or_else(|| error.to_string())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = value?.as_str()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn value_is_present(value: Option<&JsonValue>) -> bool {
    match value {
        Some(JsonValue::Null) | None => false,
        Some(JsonValue::Bool(value)) => *value,
        Some(JsonValue::Number(_)) => true,
        Some(JsonValue::String(text)) => !text.trim().is_empty(),
        Some(JsonValue::Array(values)) => !values.is_empty(),
        Some(JsonValue::Object(values)) => !values.is_empty(),
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

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => text.trim().parse::<f64>().ok(),
        JsonValue::Bool(true) => Some(1.0),
        JsonValue::Bool(false) => Some(0.0),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => json_to_i64(&JsonValue::Number(number.clone())),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) => Some(0),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    optional_i64(value).and_then(|value| usize::try_from(value).ok())
}

fn json_to_i64(value: &JsonValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
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
