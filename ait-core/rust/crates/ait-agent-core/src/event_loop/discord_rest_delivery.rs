use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::file_store::agent_file_store_read_bytes_json;
use crate::transport::{
    agent_transport_config_split_message_chunks, agent_transport_http_execute_json_request_json,
    agent_transport_http_execute_multipart_json_request_json,
};

const MIGRATION_STAGE: &str = "rust_agent_discord_rest_delivery_execution";
const DISCORD_REST_DELIVERY_CONTRACT: &str =
    "ait_agent_core.event_loop.DiscordRestDeliveryExecution.v1";
const DEFAULT_DISCORD_API_BASE_URL: &str = "https://discord.com/api/v10";
const DEFAULT_DISCORD_USER_AGENT: &str = "ait-agent/discord-rest";
const DEFAULT_TIMEOUT_SECONDS: f64 = 20.0;
const DISCORD_MESSAGE_LIMIT: usize = 2_000;
const REDACTED: &str = "[redacted]";
static MULTIPART_BOUNDARY_COUNTER: AtomicU64 = AtomicU64::new(1);

pub trait DiscordRestDeliveryExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn execute_multipart_request(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn read_attachment_bytes(&self, path: &Path) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDiscordRestDeliveryExecutor;

impl DiscordRestDeliveryExecutor for DefaultDiscordRestDeliveryExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_transport_http_execute_json_request_json(request)
    }

    fn execute_multipart_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_transport_http_execute_multipart_json_request_json(request)
    }

    fn read_attachment_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        agent_file_store_read_bytes_json(&json!({"path": path.to_string_lossy()}))
            .map(|(_, payload)| payload)
    }
}

pub fn agent_discord_rest_delivery_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    execute_with_discord_rest_delivery_executor(&DefaultDiscordRestDeliveryExecutor, request)
}

pub fn execute_with_discord_rest_delivery_executor<E>(
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: DiscordRestDeliveryExecutor + ?Sized,
{
    let request = request_object(request)?;
    let operation = operation_object(request);
    let kind = clean_text(operation.get("kind")).unwrap_or_default();
    let secrets = SecretValues::from_request(request, operation);
    let config = match ExecutionConfig::parse(request, operation) {
        Ok(config) => config,
        Err(error) => {
            return Ok(failure_payload(
                "rejected",
                &kind,
                sanitize_text(&error, &secrets),
                json!({}),
            ))
        }
    };

    let result = match kind.as_str() {
        "edit_original_response" | "send_followup" | "send_channel_message" => {
            execute_text_operation(executor, request, operation, &config, &kind)
        }
        "send_followup_attachment" | "send_channel_attachment" => {
            execute_attachment_operation(executor, request, operation, &config, &kind)
        }
        "list_channel_messages" => {
            execute_list_channel_messages(executor, request, operation, &config)
        }
        _ => Err(DeliveryFailure::new(format!(
            "Unsupported Discord REST delivery operation: {}.",
            if kind.is_empty() { "<missing>" } else { &kind }
        ))),
    };

    match result {
        Ok(payload) => Ok(success_payload(&kind, payload)),
        Err(failure) => Ok(failure_payload(
            "delivery_failed",
            &kind,
            sanitize_text(&failure.error, &config.secrets),
            failure.fields,
        )),
    }
}

struct DeliveryFailure {
    error: String,
    fields: JsonValue,
}

impl DeliveryFailure {
    fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            fields: json!({}),
        }
    }

    fn with_fields(error: impl Into<String>, fields: JsonValue) -> Self {
        Self {
            error: error.into(),
            fields,
        }
    }
}

impl From<String> for DeliveryFailure {
    fn from(error: String) -> Self {
        Self::new(error)
    }
}

struct ExecutionConfig {
    api_base_url: String,
    bot_token: Option<String>,
    user_agent: String,
    timeout_seconds: Option<f64>,
    repo_root: Option<PathBuf>,
    secrets: SecretValues,
}

impl ExecutionConfig {
    fn parse(
        request: &Map<String, JsonValue>,
        operation: &Map<String, JsonValue>,
    ) -> Result<Self, String> {
        let api_base_url = clean_text(operation.get("api_base_url"))
            .or_else(|| clean_text(request.get("api_base_url")))
            .unwrap_or_else(|| DEFAULT_DISCORD_API_BASE_URL.to_string());
        let api_base_url = api_base_url.trim_end_matches('/').to_string();
        if !(api_base_url.starts_with("http://") || api_base_url.starts_with("https://")) {
            return Err("Discord REST api_base_url must use HTTP or HTTPS.".to_string());
        }
        let bot_token =
            clean_text(operation.get("bot_token")).or_else(|| clean_text(request.get("bot_token")));
        let user_agent = clean_text(operation.get("http_user_agent"))
            .or_else(|| clean_text(request.get("http_user_agent")))
            .unwrap_or_else(|| DEFAULT_DISCORD_USER_AGENT.to_string());
        if user_agent.contains(['\r', '\n']) {
            return Err("Discord REST http_user_agent must not contain newlines.".to_string());
        }
        let timeout_value = operation
            .get("timeout_seconds")
            .or_else(|| request.get("timeout_seconds"));
        let timeout_seconds = match timeout_value {
            None => Some(DEFAULT_TIMEOUT_SECONDS),
            Some(JsonValue::Null) => None,
            Some(value) => {
                let value = value.as_f64().ok_or_else(|| {
                    "Discord REST timeout_seconds must be a number or null.".to_string()
                })?;
                if !value.is_finite() || value <= 0.0 {
                    return Err(
                        "Discord REST timeout_seconds must be finite and greater than zero."
                            .to_string(),
                    );
                }
                Some(value)
            }
        };
        let repo_root = clean_text(operation.get("repo_root"))
            .or_else(|| clean_text(request.get("repo_root")))
            .map(PathBuf::from);
        let secrets = SecretValues::from_request(request, operation);
        Ok(Self {
            api_base_url,
            bot_token,
            user_agent,
            timeout_seconds,
            repo_root,
            secrets,
        })
    }

    fn headers(&self, bot_authorization: bool) -> Result<JsonValue, String> {
        let mut headers = Map::from_iter([(
            "User-Agent".to_string(),
            JsonValue::String(self.user_agent.clone()),
        )]);
        if bot_authorization {
            let token = self.bot_token.as_deref().ok_or_else(|| {
                "Discord channel REST operation requires a bot token.".to_string()
            })?;
            headers.insert(
                "Authorization".to_string(),
                JsonValue::String(format!("Bot {token}")),
            );
        }
        Ok(JsonValue::Object(headers))
    }
}

#[derive(Default)]
struct SecretValues {
    values: Vec<String>,
}

impl SecretValues {
    fn from_request(request: &Map<String, JsonValue>, operation: &Map<String, JsonValue>) -> Self {
        let mut values = Vec::new();
        for key in ["bot_token", "interaction_token", "local_path"] {
            for source in [operation, request] {
                if let Some(value) = clean_text(source.get(key)) {
                    push_unique(&mut values, value);
                }
            }
        }
        if let Some(attachment) = operation.get("attachment").and_then(JsonValue::as_object) {
            if let Some(value) = clean_text(attachment.get("local_path")) {
                push_unique(&mut values, value);
            }
        }
        Self { values }
    }

    fn with_path(mut self, path: &Path) -> Self {
        push_unique(&mut self.values, path.to_string_lossy().to_string());
        self
    }
}

fn execute_text_operation<E>(
    executor: &E,
    request: &Map<String, JsonValue>,
    operation: &Map<String, JsonValue>,
    config: &ExecutionConfig,
    kind: &str,
) -> Result<JsonValue, DeliveryFailure>
where
    E: DiscordRestDeliveryExecutor + ?Sized,
{
    let text = field_text(operation, request, "text")
        .ok_or_else(|| "Discord text delivery operation requires non-empty text.".to_string())?;
    let chunks = agent_transport_config_split_message_chunks(&text, DISCORD_MESSAGE_LIMIT);
    if chunks.is_empty() {
        return Err(DeliveryFailure::new(
            "Discord text delivery operation produced no message chunks.",
        ));
    }
    let application_id = field_text(operation, request, "application_id");
    let interaction_token = field_text(operation, request, "interaction_token");
    let channel_id = field_text(operation, request, "channel_id");
    let mut operation_results = Vec::new();
    let mut message_ids = Vec::new();

    for (index, chunk) in chunks.iter().enumerate() {
        let target = text_request_target(
            config,
            kind,
            index,
            application_id.as_deref(),
            interaction_token.as_deref(),
            channel_id.as_deref(),
        )?;
        let http_request = json!({
            "method": target.method,
            "url": target.url,
            "payload": {
                "content": chunk,
                "allowed_mentions": {"parse": []},
            },
            "headers": config.headers(target.bot_authorization)?,
            "timeout_seconds": optional_f64_json(config.timeout_seconds),
        });
        let result = execute_json_attempt(executor, &http_request, config)?;
        message_ids.extend(result.message_ids.iter().cloned());
        operation_results.push(json!({
            "index": index,
            "kind": target.result_kind,
            "ok": result.ok,
            "status_code": optional_i64_json(result.status_code),
            "message_ids": result.message_ids,
            "error": optional_string_json(result.error.as_deref()),
        }));
        if !result.ok {
            let error = result
                .error
                .unwrap_or_else(|| "Discord REST text delivery failed.".to_string());
            return Err(DeliveryFailure::with_fields(
                error,
                json!({
                    "chunk_count": chunks.len(),
                    "attempted_chunk_count": operation_results.len(),
                    "delivered_chunk_count": successful_count(&operation_results),
                    "failed_chunk_count": failed_count(&operation_results),
                    "message_ids": message_ids,
                    "messages": [],
                    "operation_results": operation_results,
                }),
            ));
        }
    }

    Ok(json!({
        "delivered": true,
        "completed": true,
        "chunk_count": chunks.len(),
        "attempted_chunk_count": operation_results.len(),
        "delivered_chunk_count": successful_count(&operation_results),
        "failed_chunk_count": failed_count(&operation_results),
        "message_ids": message_ids,
        "messages": [],
        "operation_results": operation_results,
        "error": null,
    }))
}

struct TextRequestTarget {
    method: &'static str,
    url: String,
    result_kind: &'static str,
    bot_authorization: bool,
}

fn text_request_target(
    config: &ExecutionConfig,
    kind: &str,
    chunk_index: usize,
    application_id: Option<&str>,
    interaction_token: Option<&str>,
    channel_id: Option<&str>,
) -> Result<TextRequestTarget, String> {
    match kind {
        "edit_original_response" if chunk_index == 0 => {
            let (application_id, interaction_token) =
                webhook_identity(application_id, interaction_token)?;
            Ok(TextRequestTarget {
                method: "PATCH",
                url: format!(
                    "{}/webhooks/{application_id}/{interaction_token}/messages/@original",
                    config.api_base_url
                ),
                result_kind: "edit_original_response",
                bot_authorization: false,
            })
        }
        "edit_original_response" | "send_followup" => {
            let (application_id, interaction_token) =
                webhook_identity(application_id, interaction_token)?;
            Ok(TextRequestTarget {
                method: "POST",
                url: format!(
                    "{}/webhooks/{application_id}/{interaction_token}?wait=true",
                    config.api_base_url
                ),
                result_kind: "send_followup",
                bot_authorization: false,
            })
        }
        "send_channel_message" => {
            let channel_id = required_segment(channel_id, "channel_id")?;
            Ok(TextRequestTarget {
                method: "POST",
                url: format!("{}/channels/{channel_id}/messages", config.api_base_url),
                result_kind: "send_channel_message",
                bot_authorization: true,
            })
        }
        _ => Err(format!("Unsupported Discord text operation `{kind}`.")),
    }
}

fn execute_attachment_operation<E>(
    executor: &E,
    request: &Map<String, JsonValue>,
    operation: &Map<String, JsonValue>,
    config: &ExecutionConfig,
    kind: &str,
) -> Result<JsonValue, DeliveryFailure>
where
    E: DiscordRestDeliveryExecutor + ?Sized,
{
    let attachment = operation
        .get("attachment")
        .or_else(|| request.get("attachment"))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Discord attachment operation requires attachment metadata.".to_string())?;
    let repo_root = config
        .repo_root
        .as_deref()
        .ok_or_else(|| "Discord attachment operation requires repo_root.".to_string())?;
    let local_path = clean_text(attachment.get("local_path"))
        .ok_or_else(|| "Discord attachment operation requires local_path.".to_string())?;
    let resolved_path = resolve_attachment_path(repo_root, &local_path)?;
    let file_name = attachment_file_name(attachment, &resolved_path);
    let mime_type = attachment_mime_type(attachment);
    let public_attachment = public_attachment(attachment, &file_name, &mime_type);
    let attachment_index = optional_i64(operation.get("attachment_index"))
        .unwrap_or(0)
        .max(0);
    let file_bytes = executor
        .read_attachment_bytes(&resolved_path)
        .map_err(|error| {
            DeliveryFailure::with_fields(
                sanitize_text(
                    &format!("Discord attachment read failed: {error}"),
                    &SecretValues::from_request(request, operation).with_path(&resolved_path),
                ),
                json!({
                    "attachment_index": attachment_index,
                    "attachment": public_attachment.clone(),
                }),
            )
        })?;
    let caption =
        clean_text(attachment.get("caption")).or_else(|| clean_text(attachment.get("description")));
    let (url, bot_authorization) = match kind {
        "send_followup_attachment" => {
            let application_id = field_text(operation, request, "application_id");
            let interaction_token = field_text(operation, request, "interaction_token");
            let (application_id, interaction_token) =
                webhook_identity(application_id.as_deref(), interaction_token.as_deref())?;
            (
                format!(
                    "{}/webhooks/{application_id}/{interaction_token}?wait=true",
                    config.api_base_url
                ),
                false,
            )
        }
        "send_channel_attachment" => {
            let channel_id = field_text(operation, request, "channel_id");
            let channel_id = required_segment(channel_id.as_deref(), "channel_id")?;
            (
                format!("{}/channels/{channel_id}/messages", config.api_base_url),
                true,
            )
        }
        _ => {
            return Err(DeliveryFailure::new(format!(
                "Unsupported Discord attachment operation `{kind}`."
            )))
        }
    };
    let mut attachment_payload = json!({
        "id": 0,
        "filename": file_name,
    });
    if let Some(caption) = &caption {
        attachment_payload["description"] = JsonValue::String(caption.clone());
    }
    let boundary = multipart_boundary();
    let http_request = json!({
        "url": url,
        "boundary": boundary,
        "fields": {
            "payload_json": {
                "attachments": [attachment_payload],
                "allowed_mentions": {"parse": []},
            },
        },
        "file_field": "files[0]",
        "file_name": file_name,
        "file_bytes": file_bytes,
        "mime_type": mime_type,
        "headers": config.headers(bot_authorization)?,
        "timeout_seconds": optional_f64_json(config.timeout_seconds),
    });
    let result = execute_multipart_attempt(executor, &http_request, config)?;
    if !result.ok {
        let error = result
            .error
            .unwrap_or_else(|| "Discord REST attachment delivery failed.".to_string());
        let operation_result = json!({
            "index": 0,
            "kind": kind,
            "ok": false,
            "status_code": optional_i64_json(result.status_code),
            "message_ids": result.message_ids.clone(),
            "error": error.clone(),
        });
        return Err(DeliveryFailure::with_fields(
            error,
            json!({
                "attachment_index": attachment_index,
                "attachment": public_attachment,
                "byte_count": file_bytes.len(),
                "message_ids": result.message_ids,
                "messages": [],
                "operation_results": [operation_result],
            }),
        ));
    }
    Ok(json!({
        "delivered": true,
        "completed": true,
        "attachment_index": attachment_index,
        "attachment": public_attachment,
        "byte_count": file_bytes.len(),
        "message_ids": result.message_ids,
        "messages": [],
        "operation_results": [{
            "index": 0,
            "kind": kind,
            "ok": true,
            "status_code": optional_i64_json(result.status_code),
            "message_ids": result.message_ids,
            "error": null,
        }],
        "error": null,
    }))
}

fn execute_list_channel_messages<E>(
    executor: &E,
    request: &Map<String, JsonValue>,
    operation: &Map<String, JsonValue>,
    config: &ExecutionConfig,
) -> Result<JsonValue, DeliveryFailure>
where
    E: DiscordRestDeliveryExecutor + ?Sized,
{
    let channel_id = field_text(operation, request, "channel_id");
    let channel_id = required_segment(channel_id.as_deref(), "channel_id")?;
    let limit = optional_i64(operation.get("limit").or_else(|| request.get("limit")))
        .unwrap_or(100)
        .clamp(1, 100);
    let mut url = format!(
        "{}/channels/{channel_id}/messages?limit={limit}",
        config.api_base_url
    );
    for name in ["after", "before", "around"] {
        if let Some(value) = field_text(operation, request, name) {
            let value = required_segment(Some(&value), name)?;
            url.push('&');
            url.push_str(name);
            url.push('=');
            url.push_str(value);
        }
    }
    let http_request = json!({
        "method": "GET",
        "url": url,
        "headers": config.headers(true)?,
        "timeout_seconds": optional_f64_json(config.timeout_seconds),
    });
    let raw = executor
        .execute_json_request(&http_request)
        .map_err(|_| "Discord channel-history executor failed.".to_string())?;
    if raw.get("ok").and_then(JsonValue::as_bool) != Some(true) {
        return Err(DeliveryFailure::new(http_failure_message(&raw, config)));
    }
    let messages = raw
        .get("payload")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| {
            "Discord channel-history executor returned a non-array payload.".to_string()
        })?
        .iter()
        .map(|message| sanitize_json(message, &config.secrets))
        .collect::<Vec<_>>();
    let message_ids = collect_message_ids(&JsonValue::Array(messages.clone()));
    Ok(json!({
        "delivered": true,
        "completed": true,
        "limit": limit,
        "message_count": messages.len(),
        "message_ids": message_ids,
        "messages": messages,
        "operation_results": [{
            "index": 0,
            "kind": "list_channel_messages",
            "ok": true,
            "status_code": raw.get("status_code").cloned().unwrap_or(JsonValue::Null),
            "message_ids": message_ids,
            "error": null,
        }],
        "error": null,
    }))
}

struct HttpAttempt {
    ok: bool,
    status_code: Option<i64>,
    message_ids: Vec<JsonValue>,
    error: Option<String>,
}

fn execute_json_attempt<E>(
    executor: &E,
    request: &JsonValue,
    config: &ExecutionConfig,
) -> Result<HttpAttempt, String>
where
    E: DiscordRestDeliveryExecutor + ?Sized,
{
    let raw = executor
        .execute_json_request(request)
        .map_err(|_| "Discord REST JSON executor failed.".to_string())?;
    Ok(http_attempt(&raw, config))
}

fn execute_multipart_attempt<E>(
    executor: &E,
    request: &JsonValue,
    config: &ExecutionConfig,
) -> Result<HttpAttempt, String>
where
    E: DiscordRestDeliveryExecutor + ?Sized,
{
    let raw = executor
        .execute_multipart_request(request)
        .map_err(|_| "Discord REST multipart executor failed.".to_string())?;
    Ok(http_attempt(&raw, config))
}

fn http_attempt(raw: &JsonValue, config: &ExecutionConfig) -> HttpAttempt {
    let ok = raw.get("ok").and_then(JsonValue::as_bool) == Some(true);
    HttpAttempt {
        ok,
        status_code: raw.get("status_code").and_then(JsonValue::as_i64),
        message_ids: raw
            .get("payload")
            .map(collect_message_ids)
            .unwrap_or_default(),
        error: (!ok).then(|| http_failure_message(raw, config)),
    }
}

fn http_failure_message(raw: &JsonValue, config: &ExecutionConfig) -> String {
    let text = ["message", "detail", "reason", "error"]
        .iter()
        .find_map(|key| clean_text(raw.get(*key)))
        .unwrap_or_else(|| "Discord REST request failed.".to_string());
    sanitize_text(&text, &config.secrets)
}

fn resolve_attachment_path(repo_root: &Path, local_path: &str) -> Result<PathBuf, String> {
    let root = repo_root
        .canonicalize()
        .map_err(|_| "Discord attachment repo_root is unavailable.".to_string())?;
    let requested = Path::new(local_path);
    let candidate = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        root.join(requested)
    };
    let candidate = candidate
        .canonicalize()
        .map_err(|_| "Discord attachment file is unavailable.".to_string())?;
    if !candidate.starts_with(&root) || !candidate.is_file() {
        return Err("Discord attachment path must resolve to a file under repo_root.".to_string());
    }
    Ok(candidate)
}

fn webhook_identity<'a>(
    application_id: Option<&'a str>,
    interaction_token: Option<&'a str>,
) -> Result<(&'a str, &'a str), String> {
    Ok((
        required_segment(application_id, "application_id")?,
        required_segment(interaction_token, "interaction_token")?,
    ))
}

fn required_segment<'a>(value: Option<&'a str>, field: &str) -> Result<&'a str, String> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Discord REST operation requires {field}."))?;
    if value.len() > 512
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!("Discord REST operation field {field} is invalid."));
    }
    Ok(value)
}

fn attachment_file_name(attachment: &Map<String, JsonValue>, path: &Path) -> String {
    let raw = clean_text(attachment.get("file_name"))
        .or_else(|| clean_text(attachment.get("name")))
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "attachment.bin".to_string());
    let cleaned = raw
        .chars()
        .map(|ch| {
            if matches!(ch, '\r' | '\n' | '"') {
                '_'
            } else {
                ch
            }
        })
        .take(255)
        .collect::<String>();
    if cleaned.trim().is_empty() {
        "attachment.bin".to_string()
    } else {
        cleaned
    }
}

fn attachment_mime_type(attachment: &Map<String, JsonValue>) -> String {
    clean_text(attachment.get("mime_type"))
        .filter(|value| value.len() <= 127 && !value.contains(['\r', '\n']) && value.contains('/'))
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn public_attachment(
    attachment: &Map<String, JsonValue>,
    file_name: &str,
    mime_type: &str,
) -> JsonValue {
    json!({
        "kind": clean_text(attachment.get("kind")).unwrap_or_else(|| "document".to_string()),
        "file_name": file_name,
        "mime_type": mime_type,
        "caption": clean_text(attachment.get("caption"))
            .or_else(|| clean_text(attachment.get("description")))
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    })
}

fn multipart_boundary() -> String {
    let counter = MULTIPART_BOUNDARY_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("aitdiscord-{nanos:x}-{counter:x}")
}

fn collect_message_ids(payload: &JsonValue) -> Vec<JsonValue> {
    match payload {
        JsonValue::Object(object) => clean_text(object.get("id"))
            .map(JsonValue::String)
            .into_iter()
            .collect(),
        JsonValue::Array(items) => items
            .iter()
            .filter_map(|item| clean_text(item.get("id")).map(JsonValue::String))
            .collect(),
        _ => Vec::new(),
    }
}

fn success_payload(kind: &str, fields: JsonValue) -> JsonValue {
    base_payload("delivered", kind, true, fields)
}

fn failure_payload(state: &str, kind: &str, error: String, fields: JsonValue) -> JsonValue {
    let mut fields = fields.as_object().cloned().unwrap_or_default();
    fields.insert("delivered".to_string(), JsonValue::Bool(false));
    fields.insert("completed".to_string(), JsonValue::Bool(false));
    if !fields.contains_key("message_ids") {
        fields.insert("message_ids".to_string(), JsonValue::Array(Vec::new()));
    }
    if !fields.contains_key("messages") {
        fields.insert("messages".to_string(), JsonValue::Array(Vec::new()));
    }
    if !fields.contains_key("operation_results") {
        fields.insert(
            "operation_results".to_string(),
            JsonValue::Array(Vec::new()),
        );
    }
    fields.insert("error".to_string(), JsonValue::String(error));
    base_payload(state, kind, false, JsonValue::Object(fields))
}

fn base_payload(state: &str, kind: &str, ok: bool, fields: JsonValue) -> JsonValue {
    let mut payload = fields.as_object().cloned().unwrap_or_default();
    payload.insert(
        "contract".to_string(),
        JsonValue::String(DISCORD_REST_DELIVERY_CONTRACT.to_string()),
    );
    payload.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    payload.insert(
        "stage".to_string(),
        JsonValue::String("execute".to_string()),
    );
    payload.insert(
        "transport".to_string(),
        JsonValue::String("discord".to_string()),
    );
    payload.insert("kind".to_string(), JsonValue::String(kind.to_string()));
    payload.insert("ok".to_string(), JsonValue::Bool(ok));
    payload.insert(
        "delivery_execution_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    payload.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    payload.insert(
        "python_discord_api_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_file_read_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(payload)
}

fn operation_object(request: &Map<String, JsonValue>) -> &Map<String, JsonValue> {
    request
        .get("operation")
        .or_else(|| request.get("delivery_operation"))
        .and_then(JsonValue::as_object)
        .unwrap_or(request)
}

fn field_text(
    operation: &Map<String, JsonValue>,
    request: &Map<String, JsonValue>,
    key: &str,
) -> Option<String> {
    clean_text(operation.get(key)).or_else(|| clean_text(request.get(key)))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?;
    let text = match value {
        JsonValue::String(value) => value.trim().to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => return None,
    };
    (!text.is_empty()).then_some(text)
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(value) => value.as_i64(),
        JsonValue::String(value) => value.trim().parse().ok(),
        JsonValue::Bool(_) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}

fn optional_i64_json(value: Option<i64>) -> JsonValue {
    value.map(JsonValue::from).unwrap_or(JsonValue::Null)
}

fn optional_f64_json(value: Option<f64>) -> JsonValue {
    value.map(JsonValue::from).unwrap_or(JsonValue::Null)
}

fn successful_count(results: &[JsonValue]) -> usize {
    results
        .iter()
        .filter(|result| result.get("ok").and_then(JsonValue::as_bool) == Some(true))
        .count()
}

fn failed_count(results: &[JsonValue]) -> usize {
    results.len().saturating_sub(successful_count(results))
}

fn sanitize_text(text: &str, secrets: &SecretValues) -> String {
    secrets
        .values
        .iter()
        .filter(|secret| !secret.is_empty())
        .fold(text.to_string(), |value, secret| {
            value.replace(secret, REDACTED)
        })
}

fn sanitize_json(value: &JsonValue, secrets: &SecretValues) -> JsonValue {
    match value {
        JsonValue::String(value) => JsonValue::String(sanitize_text(value, secrets)),
        JsonValue::Array(values) => JsonValue::Array(
            values
                .iter()
                .map(|value| sanitize_json(value, secrets))
                .collect(),
        ),
        JsonValue::Object(values) => JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(key) {
                        JsonValue::String(REDACTED.to_string())
                    } else {
                        sanitize_json(value, secrets)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization" | "bot_token" | "interaction_token" | "local_path"
    )
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !value.is_empty() && !values.contains(&value) {
        values.push(value);
    }
}

fn request_object(value: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| "Discord REST delivery execution request must be an object.".to_string())
}

#[cfg(test)]
mod tests;
