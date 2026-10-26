use std::path::Path;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_telegram_turn_input";
const TURN_INPUT_CONTRACT: &str = "ait_agent_core.event_loop.TelegramTurnInput.v1";

const MUSIC_DOCUMENT_EXTENSIONS: &[&str] = &[
    "aac", "aif", "aiff", "alac", "flac", "m4a", "mp3", "ogg", "opus", "wav", "wma",
];
const PHOTO_DOCUMENT_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

pub trait TelegramTurnInputPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramTurnInputPlanner;

impl TelegramTurnInputPlanner for DefaultTelegramTurnInputPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_telegram_turn_input_json(request)
    }
}

pub fn agent_telegram_turn_input_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_telegram_turn_input_planner(&DefaultTelegramTurnInputPlanner, request)
}

pub fn plan_with_telegram_turn_input_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramTurnInputPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_telegram_turn_input_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request
        .as_object()
        .ok_or_else(|| "request must be a JSON object".to_string())?;
    let kind = clean_text(object.get("kind"))
        .or_else(|| clean_text(object.get("stage")))
        .unwrap_or_else(|| "normalize_user_text".to_string());

    match kind.as_str() {
        "strip_leading_bot_mention" => Ok(base_result(
            &kind,
            json!({
                "text": strip_leading_bot_mention(
                    clean_text(object.get("text")).as_deref().unwrap_or(""),
                    clean_text(object.get("username")).as_deref().unwrap_or(""),
                ),
            }),
        )),
        "normalize_user_text" => Ok(base_result(
            &kind,
            json!({
                "text": normalize_user_text(
                    clean_text(object.get("text")).as_deref().unwrap_or(""),
                    clean_text(object.get("username")).as_deref().unwrap_or(""),
                ),
            }),
        )),
        "speech_attachments_from_message" => {
            let attachments = speech_attachments_from_message(
                object.get("message").and_then(JsonValue::as_object),
                bool_field(object, "include_audio_uploads"),
            );
            Ok(base_result(&kind, json!({ "attachments": attachments })))
        }
        "music_attachments_from_message" => {
            let attachments = music_attachments_from_message(
                object.get("message").and_then(JsonValue::as_object),
            );
            Ok(base_result(&kind, json!({ "attachments": attachments })))
        }
        "file_attachments_from_message" => {
            let attachments = file_attachments_from_message(
                object.get("message").and_then(JsonValue::as_object),
                bool_field(object, "include_speech_uploads"),
            );
            Ok(base_result(&kind, json!({ "attachments": attachments })))
        }
        "attachment_summary" => {
            let attachments = object
                .get("attachments")
                .and_then(JsonValue::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(JsonValue::as_object)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(base_result(
                &kind,
                json!({
                    "text": attachment_summary(&attachments),
                }),
            ))
        }
        "normalized_turn_text" => {
            let attachments = object
                .get("attachments")
                .and_then(JsonValue::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(JsonValue::as_object)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(base_result(
                &kind,
                json!({
                    "text": normalized_turn_text(
                        clean_text(object.get("raw_text")).as_deref(),
                        clean_text(object.get("username")).as_deref().unwrap_or(""),
                        &attachments,
                    ),
                }),
            ))
        }
        "speech_turn_text" => Ok(base_result(
            &kind,
            json!({
                "text": speech_turn_text(
                    clean_text(object.get("caption")).as_deref(),
                    clean_text(object.get("transcript")).as_deref().unwrap_or(""),
                ),
            }),
        )),
        "transport_reply_attachments" => {
            let attachments = transport_reply_attachments(object.get("assistant_event"));
            Ok(base_result(&kind, json!({ "attachments": attachments })))
        }
        "transport_reply_text" => Ok(base_result(
            &kind,
            json!({
                "text": transport_reply_text(object.get("assistant_event")),
            }),
        )),
        "attachment_send_kind" => {
            let attachment = object.get("attachment").and_then(JsonValue::as_object);
            Ok(base_result(
                &kind,
                json!({
                    "send_as_audio": attachment.is_some_and(attachment_should_send_as_audio),
                    "send_as_photo": attachment.is_some_and(attachment_should_send_as_photo),
                }),
            ))
        }
        other => Err(format!(
            "unsupported Telegram turn input plan kind `{other}`"
        )),
    }
}

fn base_result(kind: &str, mut fields: JsonValue) -> JsonValue {
    let mut base = json!({
        "migration_stage": MIGRATION_STAGE,
        "turn_input_contract": TURN_INPUT_CONTRACT,
        "kind": kind,
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_turn_input_allowed": false,
    });
    if let (Some(base), Some(fields)) = (base.as_object_mut(), fields.as_object_mut()) {
        for (key, value) in std::mem::take(fields) {
            base.insert(key, value);
        }
    }
    base
}

fn bool_field(object: &Map<String, JsonValue>, key: &str) -> bool {
    object
        .get(key)
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    let text = match value {
        JsonValue::String(text) => text.trim().to_string(),
        JsonValue::Null => return None,
        other => other.to_string().trim().to_string(),
    };
    (!text.is_empty()).then_some(text)
}

fn positive_i64(value: Option<&JsonValue>) -> Option<i64> {
    let parsed = match value? {
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().map(|value| value as i64)),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }?;
    (parsed > 0).then_some(parsed)
}

fn strip_leading_bot_mention(text: &str, username: &str) -> String {
    let trimmed = text.trim();
    let username = username.trim();
    if username.is_empty() {
        return trimmed.to_string();
    }
    let mention = format!("@{}", username);
    if !trimmed
        .get(..mention.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(&mention))
    {
        return trimmed.to_string();
    }
    let suffix = &trimmed[mention.len()..];
    let mut chars = suffix.char_indices();
    let Some((_, first)) = chars.next() else {
        return trimmed.to_string();
    };
    if first.is_whitespace() {
        let end = suffix
            .char_indices()
            .find(|(_, ch)| !ch.is_whitespace())
            .map(|(idx, _)| idx)
            .unwrap_or(suffix.len());
        return suffix[end..].to_string();
    }
    if is_mention_separator(first) {
        let end = suffix
            .char_indices()
            .find(|(_, ch)| !is_mention_separator(*ch))
            .map(|(idx, _)| idx)
            .unwrap_or(suffix.len());
        return suffix[end..].to_string();
    }
    trimmed.to_string()
}

fn is_mention_separator(ch: char) -> bool {
    matches!(ch, ':' | ',' | '-' | '，' | '：')
}

fn normalize_user_text(text: &str, username: &str) -> String {
    let without_mention = strip_leading_bot_mention(text, username)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    collapse_newlines(&collapse_spaces_and_tabs(&without_mention))
        .trim()
        .to_string()
}

fn collapse_spaces_and_tabs(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut in_space = false;
    for ch in text.chars() {
        if matches!(ch, ' ' | '\t') {
            if !in_space {
                output.push(' ');
                in_space = true;
            }
        } else {
            output.push(ch);
            in_space = false;
        }
    }
    output
}

fn collapse_newlines(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut newline_count = 0usize;
    for ch in text.chars() {
        if ch == '\n' {
            newline_count += 1;
            if newline_count <= 2 {
                output.push(ch);
            }
        } else {
            newline_count = 0;
            output.push(ch);
        }
    }
    output
}

fn supported_music_document(document: &Map<String, JsonValue>) -> bool {
    clean_text(document.get("mime_type"))
        .is_some_and(|mime_type| mime_type.to_ascii_lowercase().starts_with("audio/"))
        || extension_in(
            clean_text(document.get("file_name"))
                .as_deref()
                .unwrap_or(""),
            MUSIC_DOCUMENT_EXTENSIONS,
        )
}

fn extension_in(value: &str, candidates: &[&str]) -> bool {
    Path::new(value)
        .extension()
        .and_then(|value| value.to_str())
        .map(|extension| {
            candidates
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
        .unwrap_or(false)
}

fn speech_attachments_from_message(
    message: Option<&Map<String, JsonValue>>,
    include_audio_uploads: bool,
) -> Vec<JsonValue> {
    let Some(message) = message else {
        return Vec::new();
    };
    let mut attachments = Vec::new();
    let caption = clean_text(message.get("caption"));
    if let Some(voice) = message.get("voice").and_then(JsonValue::as_object) {
        attachments.push(json!({
            "kind": "voice",
            "media_kind": "speech",
            "telegram_file_id": clean_text(voice.get("file_id")),
            "telegram_file_unique_id": clean_text(voice.get("file_unique_id")),
            "mime_type": clean_text(voice.get("mime_type")),
            "caption": caption,
            "duration_seconds": positive_i64(voice.get("duration")),
            "file_size_bytes": positive_i64(voice.get("file_size")),
        }));
    }
    if include_audio_uploads {
        if let Some(audio) = message.get("audio").and_then(JsonValue::as_object) {
            attachments.push(json!({
                "kind": "audio",
                "media_kind": "speech",
                "telegram_file_id": clean_text(audio.get("file_id")),
                "telegram_file_unique_id": clean_text(audio.get("file_unique_id")),
                "file_name": clean_text(audio.get("file_name")),
                "mime_type": clean_text(audio.get("mime_type")),
                "caption": caption,
                "title": clean_text(audio.get("title")),
                "performer": clean_text(audio.get("performer")),
                "duration_seconds": positive_i64(audio.get("duration")),
                "file_size_bytes": positive_i64(audio.get("file_size")),
            }));
        }
        if let Some(document) = message.get("document").and_then(JsonValue::as_object) {
            if supported_music_document(document) {
                attachments.push(json!({
                    "kind": "document",
                    "media_kind": "speech",
                    "telegram_file_id": clean_text(document.get("file_id")),
                    "telegram_file_unique_id": clean_text(document.get("file_unique_id")),
                    "file_name": clean_text(document.get("file_name")),
                    "mime_type": clean_text(document.get("mime_type")),
                    "caption": caption,
                    "file_size_bytes": positive_i64(document.get("file_size")),
                }));
            }
        }
    }
    filter_attachments_with_file_id(attachments)
}

fn music_attachments_from_message(message: Option<&Map<String, JsonValue>>) -> Vec<JsonValue> {
    let Some(message) = message else {
        return Vec::new();
    };
    let mut attachments = Vec::new();
    let caption = clean_text(message.get("caption"));
    if let Some(audio) = message.get("audio").and_then(JsonValue::as_object) {
        attachments.push(json!({
            "kind": "audio",
            "media_kind": "music",
            "telegram_file_id": clean_text(audio.get("file_id")),
            "telegram_file_unique_id": clean_text(audio.get("file_unique_id")),
            "file_name": clean_text(audio.get("file_name")),
            "mime_type": clean_text(audio.get("mime_type")),
            "caption": caption,
            "title": clean_text(audio.get("title")),
            "performer": clean_text(audio.get("performer")),
            "duration_seconds": positive_i64(audio.get("duration")),
            "file_size_bytes": positive_i64(audio.get("file_size")),
        }));
    }
    if let Some(document) = message.get("document").and_then(JsonValue::as_object) {
        if supported_music_document(document) {
            attachments.push(json!({
                "kind": "document",
                "media_kind": "music",
                "telegram_file_id": clean_text(document.get("file_id")),
                "telegram_file_unique_id": clean_text(document.get("file_unique_id")),
                "file_name": clean_text(document.get("file_name")),
                "mime_type": clean_text(document.get("mime_type")),
                "caption": caption,
                "file_size_bytes": positive_i64(document.get("file_size")),
            }));
        }
    }
    filter_attachments_with_file_id(attachments)
}

fn file_attachments_from_message(
    message: Option<&Map<String, JsonValue>>,
    include_speech_uploads: bool,
) -> Vec<JsonValue> {
    let Some(message) = message else {
        return Vec::new();
    };
    let mut attachments = Vec::new();
    if include_speech_uploads {
        attachments.extend(speech_attachments_from_message(Some(message), false));
    }
    let caption = clean_text(message.get("caption"));
    if let Some(document) = message.get("document").and_then(JsonValue::as_object) {
        if !supported_music_document(document) {
            attachments.push(json!({
                "kind": "document",
                "media_kind": "file",
                "telegram_file_id": clean_text(document.get("file_id")),
                "telegram_file_unique_id": clean_text(document.get("file_unique_id")),
                "file_name": clean_text(document.get("file_name")),
                "mime_type": clean_text(document.get("mime_type")),
                "caption": caption,
                "file_size_bytes": positive_i64(document.get("file_size")),
            }));
        }
    }
    if let Some(photo) = best_photo_variant(message) {
        attachments.push(json!({
            "kind": "photo",
            "media_kind": "image",
            "telegram_file_id": clean_text(photo.get("file_id")),
            "telegram_file_unique_id": clean_text(photo.get("file_unique_id")),
            "mime_type": "image/jpeg",
            "caption": caption,
            "file_size_bytes": positive_i64(photo.get("file_size")),
        }));
    }
    filter_attachments_with_file_id(attachments)
}

fn filter_attachments_with_file_id(attachments: Vec<JsonValue>) -> Vec<JsonValue> {
    attachments
        .into_iter()
        .filter(|attachment| {
            attachment
                .get("telegram_file_id")
                .and_then(JsonValue::as_str)
                .is_some_and(|value| !value.trim().is_empty())
        })
        .collect()
}

fn best_photo_variant(message: &Map<String, JsonValue>) -> Option<&Map<String, JsonValue>> {
    message
        .get("photo")
        .and_then(JsonValue::as_array)?
        .iter()
        .filter_map(JsonValue::as_object)
        .max_by_key(|item| {
            (
                positive_i64(item.get("file_size")).unwrap_or(0),
                positive_i64(item.get("width")).unwrap_or(0),
                positive_i64(item.get("height")).unwrap_or(0),
            )
        })
}

fn attachment_summary(attachments: &[&Map<String, JsonValue>]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let first_media_kind = clean_text(attachments[0].get("media_kind"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if first_media_kind != "music" {
        let mut lines = vec![if attachments.len() > 1 {
            "Telegram attachment uploads:".to_string()
        } else {
            "Telegram attachment upload:".to_string()
        }];
        for attachment in attachments {
            let label = clean_text(attachment.get("file_name"))
                .or_else(|| {
                    clean_text(attachment.get("local_path")).map(|path| {
                        Path::new(&path)
                            .file_name()
                            .and_then(|value| value.to_str())
                            .unwrap_or(&path)
                            .to_string()
                    })
                })
                .or_else(|| clean_text(attachment.get("telegram_file_id")))
                .unwrap_or_else(|| "uploaded-file".to_string());
            let mut details = Vec::new();
            if let Some(kind) = clean_text(attachment.get("kind"))
                .or_else(|| clean_text(attachment.get("media_kind")))
                .map(|value| value.to_ascii_lowercase())
                .filter(|value| !value.is_empty())
            {
                details.push(format!("kind={kind}"));
            }
            if let Some(mime_type) = clean_text(attachment.get("mime_type")) {
                details.push(mime_type);
            }
            if let Some(duration) = positive_i64(attachment.get("duration_seconds")) {
                details.push(format!("{duration}s"));
            }
            if let Some(size_label) = format_file_size(attachment.get("file_size_bytes")) {
                details.push(size_label);
            }
            if let Some(local_path) = clean_text(attachment.get("local_path")) {
                details.push(format!("local_path={local_path}"));
            }
            let suffix = if details.is_empty() {
                String::new()
            } else {
                format!(" ({})", details.join(", "))
            };
            lines.push(format!("- {label}{suffix}"));
        }
        return lines.join("\n");
    }

    let mut lines = vec!["Telegram music upload:".to_string()];
    for attachment in attachments {
        let label = clean_text(attachment.get("file_name"))
            .or_else(|| clean_text(attachment.get("title")))
            .or_else(|| clean_text(attachment.get("performer")))
            .or_else(|| clean_text(attachment.get("telegram_file_id")))
            .unwrap_or_else(|| "uploaded-audio".to_string());
        let mut details = Vec::new();
        if let Some(title) = clean_text(attachment.get("title")).filter(|title| title != &label) {
            details.push(format!("title={title}"));
        }
        if let Some(performer) = clean_text(attachment.get("performer")) {
            details.push(format!("performer={performer}"));
        }
        if let Some(mime_type) = clean_text(attachment.get("mime_type")) {
            details.push(mime_type);
        }
        if let Some(duration) = positive_i64(attachment.get("duration_seconds")) {
            details.push(format!("{duration}s"));
        }
        if let Some(size_label) = format_file_size(attachment.get("file_size_bytes")) {
            details.push(size_label);
        }
        let suffix = if details.is_empty() {
            String::new()
        } else {
            format!(" ({})", details.join(", "))
        };
        lines.push(format!("- {label}{suffix}"));
    }
    lines.join("\n")
}

fn format_file_size(value: Option<&JsonValue>) -> Option<String> {
    let size = positive_i64(value)?;
    if size < 1024 {
        return Some(format!("{size} B"));
    }
    if size < 1024 * 1024 {
        return Some(format!("{:.1} KB", size as f64 / 1024.0));
    }
    Some(format!("{:.1} MB", size as f64 / (1024.0 * 1024.0)))
}

fn normalized_turn_text(
    raw_text: Option<&str>,
    username: &str,
    attachments: &[&Map<String, JsonValue>],
) -> String {
    let normalized = raw_text
        .filter(|value| !value.is_empty())
        .map(|value| normalize_user_text(value, username))
        .unwrap_or_default();
    let attachment_summary = attachment_summary(attachments);
    if !attachment_summary.is_empty() {
        if normalized.is_empty() {
            attachment_summary
        } else {
            format!("{normalized}\n\n{attachment_summary}")
                .trim()
                .to_string()
        }
    } else {
        normalized
    }
}

fn normalize_speech_transcript(text: &str) -> String {
    collapse_newlines(&collapse_spaces_and_tabs(
        &text.replace("\r\n", "\n").replace('\r', "\n"),
    ))
    .trim()
    .to_string()
}

fn speech_turn_text(caption: Option<&str>, transcript: &str) -> String {
    let normalized_transcript = normalize_speech_transcript(transcript);
    let caption = caption.unwrap_or("").trim();
    if !caption.is_empty() && !normalized_transcript.is_empty() {
        return format!("{caption}\n\n[local speech transcript]\n{normalized_transcript}")
            .trim()
            .to_string();
    }
    if caption.is_empty() {
        normalized_transcript
    } else {
        caption.to_string()
    }
}

fn transport_reply_attachments(assistant_event: Option<&JsonValue>) -> Vec<JsonValue> {
    let Some(payload) = assistant_event
        .and_then(JsonValue::as_object)
        .and_then(|event| event.get("payload"))
        .and_then(JsonValue::as_object)
    else {
        return Vec::new();
    };
    let Some(message) = payload
        .get("transport_reply_envelope")
        .and_then(JsonValue::as_object)
        .and_then(|envelope| envelope.get("message"))
        .and_then(JsonValue::as_object)
    else {
        return Vec::new();
    };
    message
        .get("attachments")
        .and_then(JsonValue::as_array)
        .map(|attachments| {
            attachments
                .iter()
                .filter(|item| item.as_object().is_some())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn transport_reply_text(assistant_event: Option<&JsonValue>) -> String {
    let Some(payload) = assistant_event
        .and_then(JsonValue::as_object)
        .and_then(|event| event.get("payload"))
        .and_then(JsonValue::as_object)
    else {
        return String::new();
    };
    if let Some(text) = payload
        .get("transport_reply_envelope")
        .and_then(JsonValue::as_object)
        .and_then(|envelope| envelope.get("message"))
        .and_then(JsonValue::as_object)
        .and_then(|message| clean_text(message.get("text")))
    {
        return text;
    }
    clean_text(payload.get("text")).unwrap_or_default()
}

fn attachment_should_send_as_audio(attachment: &Map<String, JsonValue>) -> bool {
    let kind = clean_text(attachment.get("kind"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if kind == "audio" {
        return true;
    }
    if kind == "document" {
        return false;
    }
    clean_text(attachment.get("mime_type"))
        .is_some_and(|mime_type| mime_type.to_ascii_lowercase().starts_with("audio/"))
        || extension_in(
            clean_text(attachment.get("file_name"))
                .or_else(|| clean_text(attachment.get("local_path")))
                .as_deref()
                .unwrap_or(""),
            MUSIC_DOCUMENT_EXTENSIONS,
        )
}

fn attachment_should_send_as_photo(attachment: &Map<String, JsonValue>) -> bool {
    let kind = clean_text(attachment.get("kind"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    if kind == "document" {
        return false;
    }
    if matches!(kind.as_str(), "photo" | "image") {
        return true;
    }
    clean_text(attachment.get("mime_type"))
        .map(|mime_type| mime_type.to_ascii_lowercase())
        .is_some_and(|mime_type| mime_type.starts_with("image/") && mime_type != "image/gif")
        || extension_in(
            clean_text(attachment.get("file_name"))
                .or_else(|| clean_text(attachment.get("local_path")))
                .as_deref()
                .unwrap_or(""),
            PHOTO_DOCUMENT_EXTENSIONS,
        )
}

#[cfg(test)]
mod tests;
