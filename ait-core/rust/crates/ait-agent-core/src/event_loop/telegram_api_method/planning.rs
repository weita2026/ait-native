use crate::json_support::encode_value_or;
use ait_core::json_support::{json, JsonMap as Map, JsonNumber as Number, JsonValue};

const EXECUTION_KIND: &str = "telegram_api_method";
const DEFAULT_BASE_URL_PREFIX: &str = "https://api.telegram.org/bot";
const DEFAULT_FILE_BASE_URL_PREFIX: &str = "https://api.telegram.org/file/bot";

pub trait TelegramApiMethodPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramApiMethodPlanner;

impl TelegramApiMethodPlanner for DefaultTelegramApiMethodPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_api_method_execution_json(request)
    }
}

pub fn agent_telegram_api_method_execution_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_telegram_api_method_planner(&DefaultTelegramApiMethodPlanner, request)
}

pub fn plan_with_telegram_api_method_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramApiMethodPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_api_method_execution_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "request".to_string());
    match stage.as_str() {
        "request" => plan_api_method_request(object),
        "result" => plan_api_method_result(object),
        other => Err(format!(
            "unsupported Telegram API method execution stage `{other}`"
        )),
    }
}

fn plan_api_method_request(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let source_object = source.as_object();
    let operation = clean_text_from(source_object, "operation")
        .or_else(|| clean_text_from(source_object, "method_kind"))
        .or_else(|| clean_text_from(source_object, "kind"))
        .or_else(|| clean_text_from(Some(object), "operation"))
        .or_else(|| clean_text_from(Some(object), "method_kind"))
        .unwrap_or_else(|| "unknown".to_string());
    let normalized = normalize_operation(&operation);

    match normalized.as_str() {
        "get_updates" => plan_get_updates(source_object),
        "get_file" => plan_get_file(source_object),
        "download_file" => plan_download_file(source_object),
        "send_message" => plan_send_message(source_object),
        "send_attachment" => plan_send_attachment(source_object),
        other => Ok(failed_request(
            "request",
            other,
            format!("unsupported Telegram API method operation `{operation}`"),
        )),
    }
}

fn plan_get_updates(source_object: Option<&Map<String, JsonValue>>) -> Result<JsonValue, String> {
    let Some(base_url) = telegram_base_url(source_object) else {
        return Ok(failed_request(
            "request",
            "get_updates",
            "Telegram API method planning requires bot_token or base_url.",
        ));
    };
    let offset = optional_i64(field(source_object, "offset")).unwrap_or(0);
    let poll_timeout_seconds = optional_i64(field(source_object, "timeout_seconds"))
        .or_else(|| optional_i64(field(source_object, "poll_timeout_seconds")))
        .unwrap_or(0);
    let timeout_minimum = poll_timeout_seconds + 10;
    let request_timeout = optional_f64(field(source_object, "request_timeout_seconds"));
    let timeout = request_timeout
        .map(|value| value.max(timeout_minimum as f64))
        .unwrap_or(timeout_minimum as f64);
    let url = format!(
        "{}/getUpdates?offset={offset}&timeout={poll_timeout_seconds}&allowed_updates=%5B%22message%22%5D",
        base_url.trim_end_matches('/')
    );

    Ok(success_request(json!({
        "operation": "get_updates",
        "telegram_method": "getUpdates",
        "transport": "json",
        "retry_family": "poll",
        "result_kind": "updates",
        "method": "GET",
        "url": url,
        "payload": JsonValue::Null,
        "timeout": number_or_null(timeout),
        "timeout_minimum_seconds": timeout_minimum,
        "offset": offset,
        "poll_timeout_seconds": poll_timeout_seconds,
    })))
}

fn plan_get_file(source_object: Option<&Map<String, JsonValue>>) -> Result<JsonValue, String> {
    let Some(base_url) = telegram_base_url(source_object) else {
        return Ok(failed_request(
            "request",
            "get_file",
            "Telegram API method planning requires bot_token or base_url.",
        ));
    };
    let file_id = clean_text_from(source_object, "file_id")
        .or_else(|| clean_text_from(source_object, "telegram_file_id"));
    let Some(file_id) = file_id else {
        return Ok(failed_request(
            "request",
            "get_file",
            "Telegram getFile requires a file_id.",
        ));
    };

    Ok(success_request(json!({
        "operation": "get_file",
        "telegram_method": "getFile",
        "transport": "json",
        "retry_family": "delivery",
        "result_kind": "file_info",
        "method": "POST",
        "url": join_url(&base_url, "getFile"),
        "payload": {"file_id": file_id},
        "timeout": field(source_object, "request_timeout_seconds").cloned().unwrap_or(JsonValue::Null),
        "file_id": file_id,
        "telegram_file_id": file_id,
    })))
}

fn plan_download_file(source_object: Option<&Map<String, JsonValue>>) -> Result<JsonValue, String> {
    let Some(file_base_url) = telegram_file_base_url(source_object) else {
        return Ok(failed_request(
            "request",
            "download_file",
            "Telegram API file download planning requires bot_token or file_base_url.",
        ));
    };
    let file_path = clean_text_from(source_object, "file_path")
        .or_else(|| clean_text_from(source_object, "telegram_file_path"))
        .map(|path| path.trim_start_matches('/').to_string())
        .filter(|path| !path.is_empty());
    let Some(file_path) = file_path else {
        return Ok(failed_request(
            "request",
            "download_file",
            "Telegram file download requires a file_path.",
        ));
    };

    Ok(success_request(json!({
        "operation": "download_file",
        "telegram_method": "downloadFile",
        "transport": "bytes",
        "retry_family": "delivery",
        "result_kind": "bytes",
        "method": "GET",
        "url": join_url(&file_base_url, &file_path),
        "payload": JsonValue::Null,
        "timeout": field(source_object, "request_timeout_seconds").cloned().unwrap_or(JsonValue::Null),
        "file_path": file_path,
        "telegram_file_path": file_path,
    })))
}

fn plan_send_message(source_object: Option<&Map<String, JsonValue>>) -> Result<JsonValue, String> {
    let Some(base_url) = telegram_base_url(source_object) else {
        return Ok(failed_request(
            "request",
            "send_message",
            "Telegram API method planning requires bot_token or base_url.",
        ));
    };
    let chat_id = field(source_object, "chat_id")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let text = clean_text_from(source_object, "text").unwrap_or_default();
    let mut payload = Map::new();
    payload.insert("chat_id".to_string(), chat_id.clone());
    payload.insert("text".to_string(), json!(text));
    payload.insert("disable_web_page_preview".to_string(), json!(true));
    if let Some(parse_mode) = clean_text_from(source_object, "parse_mode") {
        payload.insert("parse_mode".to_string(), json!(parse_mode));
    }

    Ok(success_request(json!({
        "operation": "send_message",
        "telegram_method": "sendMessage",
        "transport": "json",
        "retry_family": "delivery",
        "result_kind": "unit",
        "method": "POST",
        "url": join_url(&base_url, "sendMessage"),
        "payload": JsonValue::Object(payload),
        "timeout": field(source_object, "request_timeout_seconds").cloned().unwrap_or(JsonValue::Null),
        "chat_id": chat_id,
    })))
}

fn plan_send_attachment(
    source_object: Option<&Map<String, JsonValue>>,
) -> Result<JsonValue, String> {
    let Some(base_url) = telegram_base_url(source_object) else {
        return Ok(failed_request(
            "request",
            "send_attachment",
            "Telegram API method planning requires bot_token or base_url.",
        ));
    };
    let method_name = clean_text_from(source_object, "method_name")
        .or_else(|| clean_text_from(source_object, "telegram_method"))
        .unwrap_or_else(|| "sendDocument".to_string());
    let file_field = clean_text_from(source_object, "file_field")
        .unwrap_or_else(|| default_file_field(&method_name).to_string());
    let chat_id = field(source_object, "chat_id")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let attachment = object_field_from(source_object, "attachment").unwrap_or_else(|| json!({}));
    let attachment_object = attachment.as_object();
    let extra_fields =
        object_field_from(source_object, "extra_fields").unwrap_or_else(|| json!({}));

    let mut fields = extra_fields.as_object().cloned().unwrap_or_else(Map::new);
    fields.insert("chat_id".to_string(), chat_id.clone());
    for key in ["caption", "title", "performer"] {
        if let Some(value) =
            attachment_object.and_then(|attachment| clean_text(attachment.get(key)))
        {
            fields.insert(key.to_string(), json!(value));
        }
    }
    if let Some(duration) = attachment_object
        .and_then(|attachment| {
            attachment
                .get("duration_seconds")
                .or_else(|| attachment.get("duration"))
        })
        .and_then(optional_positive_i64)
    {
        fields.insert("duration".to_string(), json!(duration));
    }

    let source = attachment_object
        .and_then(|attachment| clean_text(attachment.get("telegram_file_id")))
        .or_else(|| attachment_object.and_then(|attachment| clean_text(attachment.get("url"))));
    if let Some(source) = source {
        let mut payload = fields.clone();
        payload.insert(file_field.clone(), json!(source));
        return Ok(success_request(json!({
            "operation": "send_attachment",
            "telegram_method": method_name,
            "transport": "json",
            "retry_family": "delivery",
            "result_kind": "unit",
            "method": "POST",
            "url": join_url(&base_url, &method_name),
            "payload": JsonValue::Object(payload),
            "timeout": field(source_object, "request_timeout_seconds").cloned().unwrap_or(JsonValue::Null),
            "chat_id": chat_id,
            "file_field": file_field,
            "attachment": attachment,
        })));
    }

    let local_path = attachment_object
        .and_then(|attachment| clean_text(attachment.get("local_path")))
        .or_else(|| clean_text_from(source_object, "local_path"));
    let Some(local_path) = local_path else {
        return Ok(failed_request(
            "request",
            "send_attachment",
            format!(
                "Telegram {method_name} attachment requires one of telegram_file_id, url, or local_path."
            ),
        ));
    };
    let file_name = attachment_object
        .and_then(|attachment| clean_text(attachment.get("file_name")))
        .unwrap_or_else(|| path_file_name(&local_path));
    let mime_type = attachment_object
        .and_then(|attachment| clean_text(attachment.get("mime_type")))
        .or_else(|| guess_mime_type(&file_name).map(str::to_string))
        .unwrap_or_else(|| "application/octet-stream".to_string());

    Ok(success_request(json!({
        "operation": "send_attachment",
        "telegram_method": method_name,
        "transport": "multipart",
        "retry_family": "delivery",
        "result_kind": "unit",
        "method": "POST",
        "url": join_url(&base_url, &method_name),
        "fields": JsonValue::Object(fields),
        "file_field": file_field,
        "file_name": file_name,
        "mime_type": mime_type,
        "local_path": local_path,
        "timeout": field(source_object, "request_timeout_seconds").cloned().unwrap_or(JsonValue::Null),
        "chat_id": chat_id,
        "attachment": attachment,
    })))
}

fn plan_api_method_result(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let source_object = source.as_object();
    let operation = clean_text_from(source_object, "operation")
        .or_else(|| clean_text_from(source_object, "method_kind"))
        .or_else(|| clean_text_from(source_object, "kind"))
        .or_else(|| clean_text_from(Some(object), "operation"))
        .or_else(|| clean_text_from(Some(object), "method_kind"))
        .unwrap_or_else(|| "unknown".to_string());
    let operation = normalize_operation(&operation);
    let telegram_method = clean_text_from(source_object, "telegram_method")
        .or_else(|| telegram_method_for_operation(&operation).map(str::to_string))
        .unwrap_or_else(|| operation.clone());
    let payload = object
        .get("callback_result")
        .or_else(|| object.get("payload"))
        .or_else(|| object.get("response"))
        .or_else(|| object.get("result"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let payload_object = payload.as_object();
    let ok = payload_object
        .and_then(|payload| optional_bool(payload.get("ok")))
        .unwrap_or(false);

    let (normalized_ok, value, error) = match operation.as_str() {
        "get_updates" if ok => (
            true,
            payload_object
                .and_then(|payload| payload.get("result"))
                .and_then(JsonValue::as_array)
                .map(|updates| JsonValue::Array(updates.clone()))
                .unwrap_or_else(|| json!([])),
            JsonValue::Null,
        ),
        "get_file" if ok => {
            let result = payload_object
                .and_then(|payload| payload.get("result"))
                .and_then(JsonValue::as_object);
            if let Some(result) = result {
                (true, JsonValue::Object(result.clone()), JsonValue::Null)
            } else {
                (
                    false,
                    JsonValue::Null,
                    json!(format!(
                        "Telegram {telegram_method} failed: {}",
                        compact_json(&payload)
                    )),
                )
            }
        }
        "send_message" | "send_attachment" if ok => (true, JsonValue::Null, JsonValue::Null),
        _ if ok => (true, JsonValue::Null, JsonValue::Null),
        _ => (
            false,
            JsonValue::Null,
            json!(format!(
                "Telegram {telegram_method} failed: {}",
                compact_json(&payload)
            )),
        ),
    };

    Ok(json!({
        "stage": "result",
        "execution_kind": EXECUTION_KIND,
        "completed": normalized_ok,
        "result": {
            "execution_kind": EXECUTION_KIND,
            "operation": operation,
            "telegram_method": telegram_method,
            "ok": normalized_ok,
            "error": error,
            "value": value,
            "updates": if normalized_ok && operation == "get_updates" { value.clone() } else { JsonValue::Null },
            "file_info": if normalized_ok && operation == "get_file" { value.clone() } else { JsonValue::Null },
            "payload": payload,
        },
    }))
}

fn success_request(mut request: JsonValue) -> JsonValue {
    let request_object = request.as_object_mut().expect("request object");
    request_object.insert("execution_kind".to_string(), json!(EXECUTION_KIND));
    request_object.insert("ok".to_string(), json!(true));
    request_object.insert("error".to_string(), JsonValue::Null);
    request_object.insert("user_message".to_string(), JsonValue::Null);
    json!({
        "stage": "request",
        "execution_kind": EXECUTION_KIND,
        "should_execute": true,
        "expects_result": true,
        "request": request,
    })
}

fn failed_request(stage: &str, operation: &str, error: impl Into<String>) -> JsonValue {
    let error = error.into();
    json!({
        "stage": stage,
        "execution_kind": EXECUTION_KIND,
        "should_execute": false,
        "expects_result": false,
        "request": {
            "execution_kind": EXECUTION_KIND,
            "operation": operation,
            "ok": false,
            "error": error,
            "user_message": error,
            "transport": JsonValue::Null,
            "operations": [],
            "operation_count": 0,
        },
    })
}

fn request_source(object: &Map<String, JsonValue>) -> JsonValue {
    object
        .get("execution_request")
        .or_else(|| object.get("request"))
        .and_then(JsonValue::as_object)
        .map(|request| JsonValue::Object(request.clone()))
        .unwrap_or_else(|| JsonValue::Object(object.clone()))
}

fn normalize_operation(value: &str) -> String {
    match value.trim() {
        "getUpdates" | "telegram.get_updates" => "get_updates".to_string(),
        "getFile" | "telegram.get_file" => "get_file".to_string(),
        "downloadFile" | "download_file_bytes" | "telegram.download_file" => {
            "download_file".to_string()
        }
        "sendMessage" | "telegram.send_message" => "send_message".to_string(),
        "sendAudio" | "sendPhoto" | "sendDocument" | "telegram.send_attachment" => {
            "send_attachment".to_string()
        }
        other => other.to_string(),
    }
}

fn telegram_method_for_operation(operation: &str) -> Option<&'static str> {
    match operation {
        "get_updates" => Some("getUpdates"),
        "get_file" => Some("getFile"),
        "send_message" => Some("sendMessage"),
        _ => None,
    }
}

fn telegram_base_url(source_object: Option<&Map<String, JsonValue>>) -> Option<String> {
    clean_text_from(source_object, "base_url").or_else(|| {
        clean_text_from(source_object, "bot_token")
            .or_else(|| clean_text_from(source_object, "token"))
            .map(|token| format!("{DEFAULT_BASE_URL_PREFIX}{token}"))
    })
}

fn telegram_file_base_url(source_object: Option<&Map<String, JsonValue>>) -> Option<String> {
    clean_text_from(source_object, "file_base_url").or_else(|| {
        clean_text_from(source_object, "bot_token")
            .or_else(|| clean_text_from(source_object, "token"))
            .map(|token| format!("{DEFAULT_FILE_BASE_URL_PREFIX}{token}"))
    })
}

fn join_url(base_url: &str, tail: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        tail.trim_start_matches('/')
    )
}

fn default_file_field(method_name: &str) -> &'static str {
    match method_name {
        "sendAudio" => "audio",
        "sendPhoto" => "photo",
        _ => "document",
    }
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
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) => Some(0),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_positive_i64(value: &JsonValue) -> Option<i64> {
    optional_i64(Some(value)).filter(|value| *value > 0)
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

fn number_or_null(value: f64) -> JsonValue {
    Number::from_f64(value)
        .map(JsonValue::Number)
        .unwrap_or(JsonValue::Null)
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

fn path_file_name(value: &str) -> String {
    value
        .trim()
        .trim_end_matches(&['/', '\\'][..])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .to_string()
}

fn path_suffix(value: &str) -> Option<String> {
    let candidate = path_file_name(value);
    let index = candidate.rfind('.')?;
    if index == 0 || index + 1 >= candidate.len() {
        return None;
    }
    Some(candidate[index..].to_string())
}

fn guess_mime_type(path: &str) -> Option<&'static str> {
    match path_suffix(path)?
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "ogg" | "oga" | "opus" => Some("audio/ogg"),
        "mp3" => Some("audio/mpeg"),
        "wav" => Some("audio/wav"),
        "m4a" => Some("audio/mp4"),
        "flac" => Some("audio/flac"),
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        _ => None,
    }
}

fn compact_json(value: &JsonValue) -> String {
    encode_value_or(value, "null")
}

#[cfg(test)]
mod tests;
