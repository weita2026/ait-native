use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const EXECUTION_KIND: &str = "telegram_file_download";
const MISSING_FILE_ID_MESSAGE: &str =
    "That Telegram attachment did not include a downloadable file id.";
const MISSING_FILE_PATH_MESSAGE: &str =
    "Telegram did not return a downloadable file path for that attachment.";

pub trait TelegramFileDownloadPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramFileDownloadPlanner;

impl TelegramFileDownloadPlanner for DefaultTelegramFileDownloadPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_file_download_execution_json(request)
    }
}

pub fn agent_telegram_file_download_execution_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_telegram_file_download_planner(&DefaultTelegramFileDownloadPlanner, request)
}

pub fn plan_with_telegram_file_download_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramFileDownloadPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_file_download_execution_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "request".to_string());
    match stage.as_str() {
        "request" => plan_file_download_request(object),
        "file_info" => plan_file_download_file_info(object),
        "cache" => plan_file_download_cache(object),
        "result" => plan_file_download_result(object),
        other => Err(format!(
            "unsupported Telegram file download execution stage `{other}`"
        )),
    }
}

fn plan_file_download_request(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let source_object = source.as_object();
    let message = object_field_from(source_object, "message")
        .or_else(|| object_field_from(Some(object), "message"))
        .unwrap_or_else(|| json!({}));
    let attachment = object_field_from(source_object, "attachment")
        .or_else(|| object_field_from(Some(object), "attachment"))
        .unwrap_or_else(|| json!({}));
    let cache_root = clean_text_from(source_object, "cache_root")
        .or_else(|| clean_text(object.get("cache_root")))
        .unwrap_or_else(|| "telegram-downloads".to_string());
    let file_id = attachment
        .as_object()
        .and_then(|attachment| clean_text(attachment.get("telegram_file_id")));
    let operations = file_id
        .as_ref()
        .map(|file_id| {
            vec![json!({
                "kind": "get_file",
                "method": "telegram.get_file",
                "telegram_file_id": file_id,
                "file_id": file_id,
            })]
        })
        .unwrap_or_default();
    let ok = file_id.is_some();
    let error = if ok {
        JsonValue::Null
    } else {
        json!(MISSING_FILE_ID_MESSAGE)
    };

    Ok(json!({
        "stage": "request",
        "execution_kind": EXECUTION_KIND,
        "should_execute": ok,
        "expects_result": true,
        "request": {
            "execution_kind": EXECUTION_KIND,
            "callback_group": "telegram_attachment_download",
            "operation": "download_attachment",
            "ok": ok,
            "error": error,
            "user_message": if ok { JsonValue::Null } else { json!(MISSING_FILE_ID_MESSAGE) },
            "message": message,
            "attachment": attachment,
            "cache_root": cache_root,
            "telegram_file_id": file_id,
            "operations": operations,
            "operation_count": operations.len(),
        },
    }))
}

fn plan_file_download_file_info(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let source_object = source.as_object();
    let message = object_field_from(source_object, "message")
        .or_else(|| object_field_from(Some(object), "message"))
        .unwrap_or_else(|| json!({}));
    let attachment = object_field_from(source_object, "attachment")
        .or_else(|| object_field_from(Some(object), "attachment"))
        .unwrap_or_else(|| json!({}));
    let cache_root = clean_text_from(source_object, "cache_root")
        .or_else(|| clean_text(object.get("cache_root")))
        .unwrap_or_else(|| "telegram-downloads".to_string());
    let file_info = object_field_from(Some(object), "file_info")
        .or_else(|| object_field_from(source_object, "file_info"))
        .unwrap_or_else(|| json!({}));
    let telegram_file_path = file_info
        .as_object()
        .and_then(|file_info| clean_text(file_info.get("file_path")));

    let Some(telegram_file_path) = telegram_file_path else {
        return Ok(json!({
            "stage": "file_info",
            "execution_kind": EXECUTION_KIND,
            "should_execute": false,
            "expects_result": true,
            "request": {
                "execution_kind": EXECUTION_KIND,
                "callback_group": "telegram_attachment_download",
                "operation": "resolve_file_info",
                "ok": false,
                "error": MISSING_FILE_PATH_MESSAGE,
                "user_message": MISSING_FILE_PATH_MESSAGE,
                "message": message,
                "attachment": attachment,
                "file_info": file_info,
                "cache_root": cache_root,
                "operations": [],
                "operation_count": 0,
            },
        }));
    };

    let resolved_attachment = resolved_attachment(&attachment, &telegram_file_path);
    let local_path = cache_path(
        &cache_root,
        &message,
        &resolved_attachment,
        &telegram_file_path,
    );
    let operations = vec![json!({
        "kind": "check_cache",
        "method": "telegram.file_download.check_cache",
        "local_path": local_path,
    })];

    Ok(json!({
        "stage": "file_info",
        "execution_kind": EXECUTION_KIND,
        "should_execute": true,
        "expects_result": true,
        "request": {
            "execution_kind": EXECUTION_KIND,
            "callback_group": "telegram_attachment_download",
            "operation": "resolve_file_info",
            "ok": true,
            "error": JsonValue::Null,
            "user_message": JsonValue::Null,
            "message": message,
            "attachment": resolved_attachment,
            "file_info": file_info,
            "cache_root": cache_root,
            "telegram_file_path": telegram_file_path,
            "local_path": local_path,
            "operations": operations,
            "operation_count": operations.len(),
        },
    }))
}

fn plan_file_download_cache(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let source_object = source.as_object();
    let attachment = object_field_from(source_object, "attachment")
        .or_else(|| object_field_from(Some(object), "attachment"))
        .unwrap_or_else(|| json!({}));
    let local_path = clean_text_from(source_object, "local_path")
        .or_else(|| clean_text(object.get("local_path")))
        .unwrap_or_default();
    let telegram_file_path = clean_text_from(source_object, "telegram_file_path")
        .or_else(|| clean_text(object.get("telegram_file_path")))
        .or_else(|| {
            attachment
                .as_object()
                .and_then(|attachment| clean_text(attachment.get("telegram_file_path")))
        })
        .unwrap_or_default();
    let local_path_exists = optional_bool(object.get("local_path_exists"))
        .or_else(|| optional_bool(object.get("cache_hit")))
        .unwrap_or(false);
    let operations = if local_path_exists {
        Vec::new()
    } else {
        vec![json!({
            "kind": "download_file_bytes",
            "method": "telegram.file_download.download_file_bytes",
            "telegram_file_path": telegram_file_path,
            "file_path": telegram_file_path,
            "local_path": local_path,
        })]
    };

    Ok(json!({
        "stage": "cache",
        "execution_kind": EXECUTION_KIND,
        "should_execute": !local_path_exists,
        "expects_result": true,
        "request": {
            "execution_kind": EXECUTION_KIND,
            "callback_group": "telegram_attachment_download",
            "operation": "download_file_bytes",
            "ok": true,
            "error": JsonValue::Null,
            "user_message": JsonValue::Null,
            "attachment": attachment,
            "telegram_file_path": telegram_file_path,
            "local_path": local_path,
            "cache_hit": local_path_exists,
            "operations": operations,
            "operation_count": operations.len(),
        },
    }))
}

fn plan_file_download_result(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let source = request_source(object);
    let source_object = source.as_object();
    let callback_result = object
        .get("callback_result")
        .or_else(|| object.get("result"))
        .and_then(JsonValue::as_object)
        .unwrap_or(object);
    let attachment = object_field_from(Some(callback_result), "attachment")
        .or_else(|| object_field_from(source_object, "attachment"))
        .or_else(|| object_field_from(Some(object), "attachment"))
        .unwrap_or_else(|| json!({}));
    let local_path = clean_text_from(Some(callback_result), "local_path")
        .or_else(|| clean_text_from(source_object, "local_path"))
        .or_else(|| clean_text(object.get("local_path")));
    let operation_results = callback_result
        .get("operation_results")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let failed_operation = operation_results.iter().find_map(|result| {
        let result = result.as_object()?;
        if optional_bool(result.get("ok")).unwrap_or(false) {
            None
        } else {
            clean_text(result.get("error"))
                .or_else(|| Some("Telegram file download failed.".into()))
        }
    });
    let error = clean_text(callback_result.get("error")).or(failed_operation);
    let ok = error.is_none()
        && optional_bool(callback_result.get("ok")).unwrap_or(true)
        && !operation_results.iter().any(|result| {
            result
                .as_object()
                .map(|result| !optional_bool(result.get("ok")).unwrap_or(false))
                .unwrap_or(true)
        });
    let cache_hit = optional_bool(callback_result.get("cache_hit"))
        .or_else(|| source_object.and_then(|source| optional_bool(source.get("cache_hit"))))
        .unwrap_or(false);
    let downloaded = optional_bool(callback_result.get("downloaded")).unwrap_or_else(|| {
        operation_results.iter().any(|result| {
            let Some(result) = result.as_object() else {
                return false;
            };
            optional_bool(result.get("downloaded")).unwrap_or_else(|| {
                optional_bool(result.get("ok")).unwrap_or(false)
                    && matches!(
                        clean_text(result.get("kind")).as_deref(),
                        Some("download_file_bytes")
                    )
            })
        })
    });
    let mut resolved_attachment = attachment.as_object().cloned().unwrap_or_else(Map::new);
    if let Some(local_path) = &local_path {
        resolved_attachment.insert("local_path".to_string(), json!(local_path));
    }

    Ok(json!({
        "stage": "result",
        "execution_kind": EXECUTION_KIND,
        "completed": ok,
        "result": {
            "execution_kind": EXECUTION_KIND,
            "ok": ok,
            "error": error,
            "user_message": if ok {
                JsonValue::Null
            } else {
                json!("Telegram file download failed. Please retry in a moment.")
            },
            "attachment": JsonValue::Object(resolved_attachment),
            "local_path": local_path,
            "cache_hit": cache_hit,
            "downloaded": downloaded,
            "operation_results": operation_results,
            "operation_count": optional_i64(callback_result.get("operation_count"))
                .unwrap_or(operation_results.len() as i64),
        },
    }))
}

fn request_source(object: &Map<String, JsonValue>) -> JsonValue {
    object
        .get("execution_request")
        .or_else(|| object.get("file_info_request"))
        .or_else(|| object.get("cache_request"))
        .or_else(|| object.get("request"))
        .and_then(JsonValue::as_object)
        .map(|request| JsonValue::Object(request.clone()))
        .unwrap_or_else(|| JsonValue::Object(object.clone()))
}

fn resolved_attachment(attachment: &JsonValue, telegram_file_path: &str) -> JsonValue {
    let mut resolved = attachment.as_object().cloned().unwrap_or_else(Map::new);
    resolved.insert(
        "telegram_file_path".to_string(),
        json!(telegram_file_path.to_string()),
    );
    if clean_text(resolved.get("file_name")).is_none() {
        let file_name = path_file_name(telegram_file_path);
        if !file_name.is_empty() {
            resolved.insert("file_name".to_string(), json!(file_name));
        }
    }
    if clean_text(resolved.get("mime_type")).is_none() {
        let file_name =
            clean_text(resolved.get("file_name")).unwrap_or_else(|| telegram_file_path.to_string());
        if let Some(mime_type) =
            guess_mime_type(&file_name).or_else(|| guess_mime_type(telegram_file_path))
        {
            resolved.insert("mime_type".to_string(), json!(mime_type));
        }
    }
    JsonValue::Object(resolved)
}

fn cache_path(
    cache_root: &str,
    message: &JsonValue,
    attachment: &JsonValue,
    telegram_file_path: &str,
) -> String {
    let chat = message
        .as_object()
        .and_then(|message| message.get("chat"))
        .and_then(JsonValue::as_object);
    let chat_id = safe_token(
        chat.and_then(|chat| chat.get("id"))
            .unwrap_or(&JsonValue::Null),
        "chat",
    );
    let message_id = safe_token(
        message
            .as_object()
            .and_then(|message| message.get("message_id"))
            .unwrap_or(&JsonValue::Null),
        "message",
    );
    let attachment_object = attachment.as_object();
    let kind = safe_token(
        attachment_object
            .and_then(|attachment| {
                attachment
                    .get("kind")
                    .or_else(|| attachment.get("media_kind"))
            })
            .unwrap_or(&JsonValue::Null),
        "file",
    );
    let unique_id = safe_token(
        attachment_object
            .and_then(|attachment| {
                attachment
                    .get("telegram_file_unique_id")
                    .or_else(|| attachment.get("telegram_file_id"))
            })
            .unwrap_or(&JsonValue::Null),
        "attachment",
    );
    let mime_type = attachment_object
        .and_then(|attachment| clean_text(attachment.get("mime_type")))
        .unwrap_or_default();
    let suffix = clean_text_from(attachment_object, "file_name")
        .and_then(|file_name| path_suffix(&file_name))
        .or_else(|| path_suffix(telegram_file_path))
        .unwrap_or_else(|| default_attachment_suffix(&kind, &mime_type).to_string());
    let file_name = safe_file_name(
        attachment_object
            .and_then(|attachment| clean_text(attachment.get("file_name")))
            .unwrap_or_else(|| path_file_name(telegram_file_path)),
        &format!("{kind}-{unique_id}"),
        &suffix,
    );
    join_path(
        cache_root,
        &[&chat_id, &message_id, &format!("{unique_id}-{file_name}")],
    )
}

fn safe_token(value: &JsonValue, fallback: &str) -> String {
    let text = pythonish_text(value).trim().to_string();
    if text.is_empty() {
        return fallback.to_string();
    }
    let mut normalized = String::new();
    let mut previous_invalid = false;
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            normalized.push(ch);
            previous_invalid = false;
        } else if !previous_invalid {
            normalized.push('_');
            previous_invalid = true;
        }
    }
    let normalized = normalized.trim_matches(&['.', '_', '-'][..]).to_string();
    if normalized.is_empty() {
        fallback.to_string()
    } else {
        normalized
    }
}

fn safe_file_name(raw_name: String, fallback: &str, suffix: &str) -> String {
    let mut candidate = path_file_name(&raw_name);
    if matches!(candidate.as_str(), "" | "." | "..") {
        candidate = fallback.to_string();
    }
    let (stem, candidate_suffix) = split_stem_suffix(&candidate);
    let safe_stem = safe_token(
        &json!(if stem.is_empty() { fallback } else { &stem }),
        fallback,
    );
    let mut safe_suffix = candidate_suffix.unwrap_or_else(|| suffix.to_string());
    safe_suffix = safe_suffix
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '.')
        .collect::<String>();
    if safe_suffix.is_empty() {
        safe_suffix = suffix.to_string();
    }
    if !safe_suffix.starts_with('.') {
        safe_suffix = format!(".{safe_suffix}");
    }
    format!("{safe_stem}{safe_suffix}")
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
    let (_, suffix) = split_stem_suffix(&path_file_name(value));
    suffix
}

fn split_stem_suffix(value: &str) -> (String, Option<String>) {
    let candidate = path_file_name(value);
    let Some(index) = candidate.rfind('.') else {
        return (candidate, None);
    };
    if index == 0 || index + 1 >= candidate.len() {
        return (candidate, None);
    }
    (
        candidate[..index].to_string(),
        Some(candidate[index..].to_string()),
    )
}

fn default_attachment_suffix(kind: &str, mime_type: &str) -> &'static str {
    if kind == "photo" {
        return ".jpg";
    }
    if kind == "voice" {
        return ".ogg";
    }
    guess_extension(mime_type).unwrap_or(".bin")
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
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        _ => None,
    }
}

fn guess_extension(mime_type: &str) -> Option<&'static str> {
    match mime_type.trim().to_ascii_lowercase().as_str() {
        "image/jpeg" => Some(".jpg"),
        "image/png" => Some(".png"),
        "image/gif" => Some(".gif"),
        "image/webp" => Some(".webp"),
        "audio/ogg" => Some(".ogg"),
        "audio/mpeg" => Some(".mp3"),
        "audio/wav" => Some(".wav"),
        "audio/mp4" => Some(".m4a"),
        "application/pdf" => Some(".pdf"),
        "text/plain" => Some(".txt"),
        _ => None,
    }
}

fn join_path(root: &str, segments: &[&str]) -> String {
    let mut output = root.trim().trim_end_matches('/').to_string();
    if output.is_empty() {
        output.push_str("telegram-downloads");
    }
    for segment in segments {
        if !output.ends_with('/') {
            output.push('/');
        }
        output.push_str(segment.trim_matches('/'));
    }
    output
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
