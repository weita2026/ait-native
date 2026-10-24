use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const EXECUTION_KIND: &str = "telegram_reply_spool";

pub trait TelegramReplySpoolPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramReplySpoolPlanner;

impl TelegramReplySpoolPlanner for DefaultTelegramReplySpoolPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_telegram_reply_spool_execution_json(request)
    }
}

pub fn agent_telegram_reply_spool_execution_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_telegram_reply_spool_planner(&DefaultTelegramReplySpoolPlanner, request)
}

pub fn plan_with_telegram_reply_spool_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramReplySpoolPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_telegram_reply_spool_execution_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "key".to_string());
    match stage.as_str() {
        "key" => plan_spool_key(object),
        "entries" => plan_spool_entries(object),
        "remember" => plan_remember_spool_entry(object),
        "clear" => plan_clear_spool_entry(object),
        other => Err(format!(
            "unsupported Telegram reply spool execution stage `{other}`"
        )),
    }
}

fn plan_spool_key(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let pending_turn = pending_turn_from(source.as_object(), object);
    let (spool_key, transport_event_id, message_ids) = spool_key(&pending_turn);

    Ok(json!({
        "stage": "key",
        "execution_kind": EXECUTION_KIND,
        "spool_key": spool_key,
        "result": {
            "execution_kind": EXECUTION_KIND,
            "spool_key": spool_key,
            "transport_event_id": transport_event_id,
            "telegram_message_ids": message_ids,
        },
    }))
}

fn plan_spool_entries(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let link = object_field_from(source.as_object(), "link")
        .or_else(|| object_field_from(source.as_object(), "current_link"))
        .or_else(|| object_field_from(Some(object), "link"))
        .or_else(|| object_field_from(Some(object), "current_link"));
    let entries = telegram_reply_spool_entries(link.as_ref());

    Ok(json!({
        "stage": "entries",
        "execution_kind": EXECUTION_KIND,
        "entries": entries,
        "entry_count": entries.len(),
        "result": {
            "execution_kind": EXECUTION_KIND,
            "entries": entries,
            "entry_count": entries.len(),
        },
    }))
}

fn plan_remember_spool_entry(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let source_object = source.as_object();
    let pending_turn = pending_turn_from(source_object, object);
    let current_link = object_field_from(source_object, "current_link")
        .or_else(|| object_field_from(source_object, "link"))
        .or_else(|| object_field_from(Some(object), "current_link"))
        .or_else(|| object_field_from(Some(object), "link"));
    let Some(current_link) = current_link else {
        return Ok(no_patch("remember", "missing_current_link"));
    };
    let current_conversation_key = current_link
        .as_object()
        .and_then(|link| field_truthy_text(link, "conversation_key"))
        .map(|value| value.trim().to_string())
        .unwrap_or_default();
    let pending_conversation_key = pending_conversation_key(&pending_turn);
    if current_conversation_key != pending_conversation_key {
        return Ok(no_patch("remember", "conversation_mismatch"));
    }

    let (spool_key, transport_event_id, message_ids) = spool_key(&pending_turn);
    let existing_entries = telegram_reply_spool_entries(Some(&current_link));
    let existing = existing_entries
        .iter()
        .find(|entry| {
            entry
                .as_object()
                .and_then(|entry| field_truthy_text(entry, "spool_key"))
                .map(|value| value.trim().to_string())
                .unwrap_or_default()
                == spool_key
        })
        .and_then(JsonValue::as_object);
    let status = clean_text_from(source_object, "status")
        .or_else(|| clean_text_from(Some(object), "status"))
        .unwrap_or_default();
    let attempt_increment = optional_bool(field(source_object, "attempt_increment"))
        .or_else(|| optional_bool(object.get("attempt_increment")))
        .unwrap_or(false);
    let now_iso = field_truthy_text_opt(source_object, "now_iso")
        .or_else(|| field_truthy_text_opt(Some(object), "now_iso"))
        .unwrap_or_default();
    let last_error = field_truthy_text_opt(source_object, "last_error")
        .or_else(|| field_truthy_text_opt(Some(object), "last_error"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let user_event = object_field_from(source_object, "user_event")
        .or_else(|| object_field_from(Some(object), "user_event"));
    let assistant_event = object_field_from(source_object, "assistant_event")
        .or_else(|| object_field_from(Some(object), "assistant_event"));
    let ready_reply_text = field_truthy_text_opt(source_object, "ready_reply_text")
        .or_else(|| field_truthy_text_opt(Some(object), "ready_reply_text"))
        .map(|value| value.trim().to_string())
        .or_else(|| {
            existing
                .and_then(|entry| field_truthy_text(entry, "ready_reply_text"))
                .map(|value| value.trim().to_string())
        });
    let provider_thread = object_field_from(source_object, "provider_thread")
        .or_else(|| object_field_from(Some(object), "provider_thread"))
        .or_else(|| existing.and_then(|entry| object_field_from(Some(entry), "provider_thread")));
    let turn_telemetry = object_field_from(source_object, "turn_telemetry")
        .or_else(|| object_field_from(Some(object), "turn_telemetry"))
        .or_else(|| existing.and_then(|entry| object_field_from(Some(entry), "turn_telemetry")));
    let spool_limit = optional_i64(field(source_object, "spool_limit"))
        .or_else(|| optional_i64(object.get("spool_limit")))
        .unwrap_or(100);

    let last_attempt_at = if attempt_increment || matches!(status.as_str(), "attempting" | "failed")
    {
        json!(now_iso)
    } else {
        existing
            .and_then(|entry| entry.get("last_attempt_at"))
            .cloned()
            .unwrap_or(JsonValue::Null)
    };
    let queued_at = existing
        .and_then(|entry| field_truthy_text(entry, "queued_at"))
        .unwrap_or_else(|| now_iso.clone());
    let attempt_count = existing
        .and_then(|entry| optional_i64(entry.get("attempt_count")))
        .unwrap_or(0)
        + i64::from(attempt_increment);
    let mut entry = Map::new();
    entry.insert("spool_key".to_string(), json!(spool_key));
    entry.insert("status".to_string(), json!(status));
    entry.insert(
        "conversation_key".to_string(),
        json!(pending_conversation_key),
    );
    entry.insert("chat_id".to_string(), json!(pending_chat_id(&pending_turn)));
    entry.insert(
        "chat_title".to_string(),
        json!(field_text(&pending_turn, "chat_title")),
    );
    entry.insert(
        "chat_type".to_string(),
        pending_turn
            .get("chat_type")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    entry.insert("text".to_string(), json!(field_text(&pending_turn, "text")));
    entry.insert(
        "actor_identity".to_string(),
        json!(field_text(&pending_turn, "actor_identity")),
    );
    entry.insert(
        "transport_event_id".to_string(),
        transport_event_id
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    entry.insert(
        "telegram_message_id".to_string(),
        pending_turn
            .get("telegram_message_id")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    entry.insert("telegram_message_ids".to_string(), json!(message_ids));
    entry.insert("queued_at".to_string(), json!(queued_at));
    entry.insert("last_attempt_at".to_string(), last_attempt_at);
    entry.insert("attempt_count".to_string(), json!(attempt_count.max(0)));
    entry.insert(
        "last_error".to_string(),
        last_error.map(JsonValue::String).unwrap_or(JsonValue::Null),
    );
    entry.insert(
        "ready_reply_text".to_string(),
        ready_reply_text
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    entry.insert(
        "provider_thread".to_string(),
        provider_thread.unwrap_or(JsonValue::Null),
    );
    entry.insert(
        "turn_telemetry".to_string(),
        turn_telemetry.unwrap_or(JsonValue::Null),
    );
    entry.insert(
        "last_user_sequence".to_string(),
        sequence_or_null(user_event.as_ref()),
    );
    entry.insert(
        "last_assistant_sequence".to_string(),
        sequence_or_null(assistant_event.as_ref()),
    );
    entry.insert(
        "watch_spec".to_string(),
        pending_turn
            .get("watch_spec")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    let entry = JsonValue::Object(entry);
    let next_entries = limited_entries(
        existing_entries
            .into_iter()
            .filter(|item| {
                item.as_object()
                    .and_then(|entry| field_truthy_text(entry, "spool_key"))
                    .map(|value| value.trim().to_string())
                    .unwrap_or_default()
                    != spool_key
            })
            .chain(std::iter::once(entry.clone()))
            .collect(),
        spool_limit,
    );
    let patch_payload = json!({"telegram_reply_spool": next_entries});

    Ok(json!({
        "stage": "remember",
        "execution_kind": EXECUTION_KIND,
        "patch_required": true,
        "spool_key": spool_key,
        "entry": entry,
        "entries": patch_payload["telegram_reply_spool"].clone(),
        "patch_payload": patch_payload,
        "result": {
            "execution_kind": EXECUTION_KIND,
            "patch_required": true,
            "reason": JsonValue::Null,
            "spool_key": spool_key,
            "entry": entry,
            "entries": patch_payload["telegram_reply_spool"].clone(),
            "patch_payload": patch_payload,
        },
    }))
}

fn plan_clear_spool_entry(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let source_object = source.as_object();
    let pending_turn = pending_turn_from(source_object, object);
    let current_link = object_field_from(source_object, "current_link")
        .or_else(|| object_field_from(source_object, "link"))
        .or_else(|| object_field_from(Some(object), "current_link"))
        .or_else(|| object_field_from(Some(object), "link"));
    let Some(current_link) = current_link else {
        return Ok(no_patch("clear", "missing_current_link"));
    };

    let (spool_key, _, _) = spool_key(&pending_turn);
    let next_entries = telegram_reply_spool_entries(Some(&current_link))
        .into_iter()
        .filter(|item| {
            item.as_object()
                .and_then(|entry| field_truthy_text(entry, "spool_key"))
                .map(|value| value.trim().to_string())
                .unwrap_or_default()
                != spool_key
        })
        .collect::<Vec<_>>();
    let patch_payload = json!({"telegram_reply_spool": next_entries});

    Ok(json!({
        "stage": "clear",
        "execution_kind": EXECUTION_KIND,
        "patch_required": true,
        "spool_key": spool_key,
        "entries": patch_payload["telegram_reply_spool"].clone(),
        "patch_payload": patch_payload,
        "result": {
            "execution_kind": EXECUTION_KIND,
            "patch_required": true,
            "reason": JsonValue::Null,
            "spool_key": spool_key,
            "entries": patch_payload["telegram_reply_spool"].clone(),
            "patch_payload": patch_payload,
        },
    }))
}

fn no_patch(stage: &str, reason: &str) -> JsonValue {
    json!({
        "stage": stage,
        "execution_kind": EXECUTION_KIND,
        "patch_required": false,
        "spool_key": JsonValue::Null,
        "entries": [],
        "patch_payload": JsonValue::Null,
        "result": {
            "execution_kind": EXECUTION_KIND,
            "patch_required": false,
            "reason": reason,
            "spool_key": JsonValue::Null,
            "entries": [],
            "patch_payload": JsonValue::Null,
        },
    })
}

fn pending_turn_from(
    source_object: Option<&Map<String, JsonValue>>,
    object: &Map<String, JsonValue>,
) -> Map<String, JsonValue> {
    object_field_from(source_object, "pending_turn")
        .or_else(|| object_field_from(Some(object), "pending_turn"))
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_else(|| source_object.cloned().unwrap_or_else(|| object.clone()))
}

fn spool_key(pending_turn: &Map<String, JsonValue>) -> (String, Option<String>, Vec<i64>) {
    let envelope = object_field_from(Some(pending_turn), "transport_envelope");
    let transport_event_id = envelope
        .as_ref()
        .and_then(JsonValue::as_object)
        .map(|envelope| {
            field_truthy_text(envelope, "event_id")
                .unwrap_or_default()
                .trim()
                .to_string()
        });
    if let Some(event_id) = transport_event_id
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        return (
            event_id.clone(),
            transport_event_id,
            pending_message_ids(pending_turn),
        );
    }

    let message_ids = pending_message_ids(pending_turn);
    if !message_ids.is_empty() {
        let message_id_text = message_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        return (
            format!(
                "telegram:{}:messages:{message_id_text}",
                pending_chat_id(pending_turn)
            ),
            transport_event_id,
            message_ids,
        );
    }

    (
        format!(
            "telegram:{}:conversation:{}:text:{}",
            pending_chat_id(pending_turn),
            pending_conversation_key(pending_turn),
            field_text(pending_turn, "text").trim()
        ),
        transport_event_id,
        message_ids,
    )
}

fn pending_message_ids(pending_turn: &Map<String, JsonValue>) -> Vec<i64> {
    std::iter::once(pending_turn.get("telegram_message_id"))
        .chain(
            pending_turn
                .get("telegram_message_ids")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .map(Some),
        )
        .filter_map(optional_i64)
        .filter(|value| *value > 0)
        .collect()
}

fn telegram_reply_spool_entries(link: Option<&JsonValue>) -> Vec<JsonValue> {
    link.and_then(JsonValue::as_object)
        .and_then(|link| link.get("telegram_reply_spool"))
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|item| item.as_object().map(|item| JsonValue::Object(item.clone())))
                .collect()
        })
        .unwrap_or_default()
}

fn limited_entries(entries: Vec<JsonValue>, spool_limit: i64) -> Vec<JsonValue> {
    if spool_limit <= 0 || entries.len() <= spool_limit as usize {
        return entries;
    }
    entries[entries.len() - spool_limit as usize..].to_vec()
}

fn sequence_or_null(value: Option<&JsonValue>) -> JsonValue {
    value
        .and_then(JsonValue::as_object)
        .map(|value| json!(optional_i64(value.get("sequence")).unwrap_or(0)))
        .unwrap_or(JsonValue::Null)
}

fn request_source(object: &Map<String, JsonValue>) -> JsonValue {
    object
        .get("execution_request")
        .or_else(|| object.get("request"))
        .and_then(JsonValue::as_object)
        .map(|request| JsonValue::Object(request.clone()))
        .unwrap_or_else(|| JsonValue::Object(object.clone()))
}

fn field<'a>(object: Option<&'a Map<String, JsonValue>>, key: &str) -> Option<&'a JsonValue> {
    object.and_then(|object| object.get(key))
}

fn object_field_from(object: Option<&Map<String, JsonValue>>, key: &str) -> Option<JsonValue> {
    object
        .and_then(|object| object.get(key))
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
}

fn clean_text_from(object: Option<&Map<String, JsonValue>>, key: &str) -> Option<String> {
    object.and_then(|object| clean_text(object.get(key)))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    if matches!(value, JsonValue::Null) {
        return None;
    }
    let text = pythonish_text(value).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn field_text(object: &Map<String, JsonValue>, key: &str) -> String {
    object.get(key).map(pythonish_text).unwrap_or_default()
}

fn pending_conversation_key(pending_turn: &Map<String, JsonValue>) -> String {
    field_text(pending_turn, "conversation_key")
}

fn pending_chat_id(pending_turn: &Map<String, JsonValue>) -> String {
    field_text(pending_turn, "chat_id")
}

fn field_truthy_text(object: &Map<String, JsonValue>, key: &str) -> Option<String> {
    object
        .get(key)
        .filter(|value| python_truthy(value))
        .map(pythonish_text)
}

fn field_truthy_text_opt(object: Option<&Map<String, JsonValue>>, key: &str) -> Option<String> {
    object.and_then(|object| field_truthy_text(object, key))
}

fn python_truthy(value: &JsonValue) -> bool {
    match value {
        JsonValue::Null => false,
        JsonValue::Bool(value) => *value,
        JsonValue::Number(number) => number
            .as_i64()
            .map(|value| value != 0)
            .or_else(|| number.as_u64().map(|value| value != 0))
            .or_else(|| number.as_f64().map(|value| value != 0.0))
            .unwrap_or(false),
        JsonValue::String(text) => !text.is_empty(),
        JsonValue::Array(values) => !values.is_empty(),
        JsonValue::Object(values) => !values.is_empty(),
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

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) => Some(0),
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
