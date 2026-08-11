use crate::json_support::parse_value;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

pub(super) fn plan_background_sync(
    enabled: bool,
    next_background_sync_at: Option<f64>,
    now_monotonic_seconds: f64,
    interval_seconds: f64,
) -> (Option<f64>, bool) {
    if !enabled {
        return (None, false);
    }
    match next_background_sync_at {
        None => (Some(now_monotonic_seconds + interval_seconds), false),
        Some(deadline) if now_monotonic_seconds < deadline => (Some(deadline), false),
        Some(_) => (Some(now_monotonic_seconds + interval_seconds), true),
    }
}

pub(super) fn plan_poll_timeout_seconds(
    poll_timeout_seconds: i64,
    background_sync_enabled: bool,
    next_background_sync_at: Option<f64>,
    now_monotonic_seconds: f64,
) -> i64 {
    if !background_sync_enabled {
        return poll_timeout_seconds;
    }
    let Some(deadline) = next_background_sync_at else {
        return poll_timeout_seconds;
    };
    let seconds_until_sync = (deadline - now_monotonic_seconds).max(0.0);
    let clamped_to_sync = seconds_until_sync.ceil().max(1.0) as i64;
    poll_timeout_seconds.min(clamped_to_sync)
}

pub(super) fn telegram_update_chat_id(update: &Map<String, JsonValue>) -> Option<JsonValue> {
    update
        .get("message")
        .and_then(JsonValue::as_object)
        .and_then(|message| message.get("chat"))
        .and_then(JsonValue::as_object)
        .and_then(|chat| chat.get("id"))
        .cloned()
}

pub(super) fn telegram_update_message_id(update: &Map<String, JsonValue>) -> Option<i64> {
    update
        .get("message")
        .and_then(JsonValue::as_object)
        .and_then(|message| optional_i64(message.get("message_id")))
}

pub(super) fn fallback_update_key_at(
    values: Option<&Vec<JsonValue>>,
    index: usize,
) -> Option<String> {
    values?
        .get(index)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
}

pub(super) fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())
}

pub(super) fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
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

pub(super) fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                Some(0)
            } else {
                text.parse::<i64>().ok()
            }
        }
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) => Some(0),
        JsonValue::Null => None,
        JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

pub(super) fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                Some(0.0)
            } else {
                text.parse::<f64>().ok()
            }
        }
        JsonValue::Bool(true) => Some(1.0),
        JsonValue::Bool(false) => Some(0.0),
        JsonValue::Null => None,
        JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

pub(super) fn pythonish_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => "None".to_string(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => "False".to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::String(text) => text.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => value.to_string(),
    }
}

pub(super) fn action_kind(action: &Map<String, JsonValue>) -> Result<&str, String> {
    let kind = action
        .get("kind")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "action.kind is required".to_string())?;
    if kind.is_empty() {
        Err("action.kind must not be empty".to_string())
    } else {
        Ok(kind)
    }
}

pub(super) fn callback_poll_request(action: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    if let Some(poll_request) = action.get("poll_request") {
        let request = poll_request
            .as_object()
            .ok_or_else(|| "poll_request must be a JSON object".to_string())?;
        let offset = optional_i64(request.get("offset")).unwrap_or(0);
        let timeout_seconds = optional_i64(request.get("timeout_seconds")).unwrap_or(0);
        return Ok(json!({
            "offset": offset,
            "timeout_seconds": timeout_seconds,
        }));
    }

    Ok(json!({
        "offset": optional_i64(action.get("offset")).unwrap_or(0),
        "timeout_seconds": optional_i64(action.get("timeout_seconds")).unwrap_or(0),
    }))
}

pub(super) fn object_field_or_empty(object: &Map<String, JsonValue>, key: &str) -> JsonValue {
    object_field_from(Some(object), key)
}

pub(super) fn adapter_request_or_empty(object: &Map<String, JsonValue>) -> JsonValue {
    object
        .get("adapter_request")
        .or_else(|| object.get("request"))
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}))
}

pub(super) fn object_field_from(object: Option<&Map<String, JsonValue>>, key: &str) -> JsonValue {
    object
        .and_then(|object| object.get(key))
        .and_then(JsonValue::as_object)
        .map(|value| JsonValue::Object(value.clone()))
        .unwrap_or_else(|| json!({}))
}

pub(super) fn array_field(
    object: Option<&Map<String, JsonValue>>,
    key: &str,
) -> Option<Vec<JsonValue>> {
    object
        .and_then(|object| object.get(key))
        .and_then(JsonValue::as_array)
        .cloned()
}

pub(super) fn text_array_field(object: &Map<String, JsonValue>, key: &str) -> Vec<String> {
    object
        .get(key)
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(|value| clean_text(Some(value)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn clean_text(value: Option<&JsonValue>) -> Option<String> {
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

pub(super) fn normalize_command_trigger_reply_parts(
    payload: &Map<String, JsonValue>,
) -> (bool, String, Vec<JsonValue>) {
    let reply_payload = payload.get("reply").and_then(JsonValue::as_object);
    let top_level_text =
        clean_text(payload.get("reply_text")).or_else(|| clean_text(payload.get("text")));
    let nested_text = clean_text(reply_payload.and_then(|reply| reply.get("text")));
    let attachments_source = reply_payload
        .and_then(|reply| reply.get("attachments"))
        .and_then(JsonValue::as_array)
        .or_else(|| payload.get("attachments").and_then(JsonValue::as_array));
    let attachments = attachments_source
        .map(|values| {
            values
                .iter()
                .filter(|value| value.as_object().is_some())
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let reply_text = nested_text.or(top_level_text).unwrap_or_default();
    let handled = match payload.get("handled") {
        Some(JsonValue::Bool(value)) => *value,
        _ => !reply_text.is_empty() || !attachments.is_empty(),
    };
    (handled, reply_text, attachments)
}

pub(super) fn reply_delivery_execution_request_source(
    object: &Map<String, JsonValue>,
) -> JsonValue {
    object
        .get("execution_request")
        .or_else(|| object.get("callback_execution_request"))
        .or_else(|| object.get("request"))
        .or_else(|| object.get("adapter_request"))
        .and_then(JsonValue::as_object)
        .map(|request| JsonValue::Object(request.clone()))
        .unwrap_or_else(|| json!({}))
}

pub(super) fn command_trigger_execution_request_source(
    object: &Map<String, JsonValue>,
) -> JsonValue {
    object
        .get("execution_request")
        .or_else(|| object.get("callback_execution_request"))
        .or_else(|| object.get("request"))
        .or_else(|| object.get("adapter_request"))
        .and_then(JsonValue::as_object)
        .map(|request| JsonValue::Object(request.clone()))
        .unwrap_or_else(|| json!({}))
}

pub(super) fn non_empty_object_field_from(
    object: Option<&Map<String, JsonValue>>,
    key: &str,
) -> Option<JsonValue> {
    let value = object
        .and_then(|object| object.get(key))
        .and_then(JsonValue::as_object)?;
    if value.is_empty() {
        None
    } else {
        Some(JsonValue::Object(value.clone()))
    }
}

pub(super) fn text_array_field_from(
    object: Option<&Map<String, JsonValue>>,
    key: &str,
) -> Option<Vec<String>> {
    let values = object
        .and_then(|object| object.get(key))
        .and_then(JsonValue::as_array)?;
    Some(
        values
            .iter()
            .filter_map(|value| clean_text(Some(value)))
            .collect::<Vec<_>>(),
    )
}

pub(super) fn command_trigger_env_overrides(repo_root: Option<&str>) -> JsonValue {
    match repo_root {
        Some(repo_root) if !repo_root.is_empty() => json!({
            "AIT_REPO_ROOT": repo_root,
        }),
        _ => json!({}),
    }
}

pub(super) fn command_trigger_repo_src(repo_root: Option<&str>) -> JsonValue {
    match repo_root {
        Some(repo_root) if !repo_root.is_empty() => JsonValue::String(format!("{repo_root}/src")),
        _ => JsonValue::Null,
    }
}

pub(super) fn telegram_assistant_reply_text(assistant_event: &JsonValue) -> String {
    let payload = assistant_event
        .as_object()
        .and_then(|event| event.get("payload"))
        .and_then(JsonValue::as_object);
    let envelope_text = payload
        .and_then(|payload| payload.get("transport_reply_envelope"))
        .and_then(JsonValue::as_object)
        .and_then(|envelope| envelope.get("message"))
        .and_then(JsonValue::as_object)
        .and_then(|message| clean_text(message.get("text")));
    envelope_text
        .or_else(|| payload.and_then(|payload| clean_text(payload.get("text"))))
        .unwrap_or_default()
}

pub(super) fn telegram_assistant_reply_attachments(assistant_event: &JsonValue) -> Vec<JsonValue> {
    assistant_event
        .as_object()
        .and_then(|event| event.get("payload"))
        .and_then(JsonValue::as_object)
        .and_then(|payload| payload.get("transport_reply_envelope"))
        .and_then(JsonValue::as_object)
        .and_then(|envelope| envelope.get("message"))
        .and_then(JsonValue::as_object)
        .and_then(|message| message.get("attachments"))
        .and_then(JsonValue::as_array)
        .map(|attachments| {
            attachments
                .iter()
                .filter(|attachment| attachment.as_object().is_some())
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn telegram_reply_delivery_operations(
    chat_id: &JsonValue,
    reply_text: &str,
    attachments: &[JsonValue],
) -> Vec<JsonValue> {
    let mut operations =
        Vec::with_capacity(usize::from(!reply_text.is_empty()) + attachments.len());
    if !reply_text.is_empty() {
        operations.push(json!({
            "kind": "send_message",
            "method": "sendMessage",
            "chat_id": chat_id,
            "text": reply_text,
        }));
    }
    for (index, attachment) in attachments.iter().enumerate() {
        let (kind, method, file_field) = telegram_reply_attachment_operation(attachment);
        operations.push(json!({
            "kind": kind,
            "method": method,
            "file_field": file_field,
            "chat_id": chat_id,
            "attachment_index": index,
            "attachment": attachment,
        }));
    }
    operations
}

pub(super) fn telegram_reply_attachment_operation(
    attachment: &JsonValue,
) -> (&'static str, &'static str, &'static str) {
    if telegram_attachment_should_send_as_audio(attachment) {
        ("send_audio", "sendAudio", "audio")
    } else if telegram_attachment_should_send_as_photo(attachment) {
        ("send_photo", "sendPhoto", "photo")
    } else {
        ("send_document", "sendDocument", "document")
    }
}

pub(super) fn telegram_attachment_should_send_as_audio(attachment: &JsonValue) -> bool {
    let kind = lower_attachment_text(attachment, "kind");
    if kind == "audio" {
        return true;
    }
    if kind == "document" {
        return false;
    }
    let mime_type = lower_attachment_text(attachment, "mime_type");
    if mime_type.starts_with("audio/") {
        return true;
    }
    let suffix = attachment_filename_suffix(attachment);
    matches!(
        suffix.as_str(),
        ".aac"
            | ".aif"
            | ".aiff"
            | ".alac"
            | ".flac"
            | ".m4a"
            | ".mp3"
            | ".ogg"
            | ".opus"
            | ".wav"
            | ".wma"
    ) && kind != "document"
}

pub(super) fn telegram_attachment_should_send_as_photo(attachment: &JsonValue) -> bool {
    let kind = lower_attachment_text(attachment, "kind");
    if kind == "document" {
        return false;
    }
    if kind == "photo" || kind == "image" {
        return true;
    }
    let mime_type = lower_attachment_text(attachment, "mime_type");
    if mime_type.starts_with("image/") && mime_type != "image/gif" {
        return true;
    }
    matches!(
        attachment_filename_suffix(attachment).as_str(),
        ".jpg" | ".jpeg" | ".png" | ".webp"
    )
}

pub(super) fn lower_attachment_text(attachment: &JsonValue, key: &str) -> String {
    attachment
        .as_object()
        .and_then(|attachment| clean_text(attachment.get(key)))
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub(super) fn attachment_filename_suffix(attachment: &JsonValue) -> String {
    let source = attachment
        .as_object()
        .and_then(|attachment| {
            clean_text(attachment.get("file_name"))
                .or_else(|| clean_text(attachment.get("local_path")))
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    match source.rsplit_once('.') {
        Some((_, suffix)) if !suffix.is_empty() => format!(".{suffix}"),
        _ => String::new(),
    }
}

pub(super) fn callback_error(payload: &Map<String, JsonValue>) -> Option<String> {
    clean_text(payload.get("error")).or_else(|| clean_text(payload.get("exception")))
}

pub(super) fn normalize_command_execution_response(
    payload: &Map<String, JsonValue>,
    returncode: i64,
    stdout: &str,
    stderr: &str,
) -> (JsonValue, Option<String>) {
    if let Some(error) = callback_error(payload) {
        return (
            object_field_or_empty(payload, "handler_response"),
            Some(error),
        );
    }
    if returncode != 0 {
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else if !stdout.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            format!("exit code {returncode}")
        };
        return (json!({}), Some(detail));
    }
    if let Some(response) = payload
        .get("handler_response")
        .or_else(|| payload.get("response"))
        .and_then(JsonValue::as_object)
    {
        return (JsonValue::Object(response.clone()), None);
    }
    let source = if stdout.trim().is_empty() {
        "{}"
    } else {
        stdout.trim()
    };
    match parse_value(
        source,
        "failed to parse operational trigger handler response",
    ) {
        Ok(JsonValue::Object(response)) => (JsonValue::Object(response), None),
        Ok(_) => (
            json!({}),
            Some("Operational trigger handler must return a JSON object.".to_string()),
        ),
        Err(_) => (
            json!({}),
            Some("Operational trigger handler returned invalid JSON.".to_string()),
        ),
    }
}
