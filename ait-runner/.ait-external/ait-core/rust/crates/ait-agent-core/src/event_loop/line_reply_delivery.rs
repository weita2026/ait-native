use std::collections::VecDeque;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::transport::{
    agent_transport_config_split_message_chunks, agent_transport_http_execute_json_request_json,
};

const MIGRATION_STAGE: &str = "rust_agent_line_reply_delivery_execution";
const LINE_REPLY_DELIVERY_CONTRACT: &str =
    "ait_agent_core.event_loop.LineReplyDeliveryExecution.v1";
const DEFAULT_LINE_API_BASE_URL: &str = "https://api.line.me";
const DEFAULT_MESSAGE_LIMIT: usize = 5_000;
const MAX_MESSAGES_PER_REQUEST: usize = 5;
const DEFAULT_TIMEOUT_SECONDS: f64 = 20.0;
const REDACTED: &str = "[redacted]";

pub trait LineReplyDeliveryExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultLineReplyDeliveryExecutor;

impl LineReplyDeliveryExecutor for DefaultLineReplyDeliveryExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_transport_http_execute_json_request_json(request)
    }
}

pub fn agent_line_reply_delivery_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    execute_with_line_reply_delivery_executor(&DefaultLineReplyDeliveryExecutor, request)
}

pub fn execute_with_line_reply_delivery_executor<E>(
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: LineReplyDeliveryExecutor + ?Sized,
{
    execute_line_reply_delivery(executor, request)
}

fn execute_line_reply_delivery<E>(executor: &E, request: &JsonValue) -> Result<JsonValue, String>
where
    E: LineReplyDeliveryExecutor + ?Sized,
{
    let request = request_object(request)?;
    let channel_id = required_text(request.get("channel_id"), "channel_id")?;
    let channel_access_token =
        required_text(request.get("channel_access_token"), "channel_access_token")?;
    let text = required_string(request.get("text"), "text")?;
    let reply_token = clean_text(request.get("reply_token"));
    let api_base_url = normalized_api_base_url(request.get("api_base_url"))?;
    let timeout_seconds = timeout_seconds(request.get("timeout_seconds"))?;
    let chunks = agent_transport_config_split_message_chunks(&text, DEFAULT_MESSAGE_LIMIT);
    let batches = delivery_batches(&chunks, reply_token.is_some());
    let mut operation_results = Vec::with_capacity(batches.len());

    for (index, batch) in batches.iter().enumerate() {
        let (path, payload) = match batch.kind {
            DeliveryKind::Reply => (
                "/v2/bot/message/reply",
                json!({
                    "replyToken": reply_token,
                    "messages": batch.messages,
                }),
            ),
            DeliveryKind::Push => (
                "/v2/bot/message/push",
                json!({
                    "to": channel_id,
                    "messages": batch.messages,
                }),
            ),
        };
        let url = format!("{api_base_url}{path}");
        let http_request = json!({
            "method": "POST",
            "url": url,
            "payload": payload,
            "headers": {
                "Authorization": format!("Bearer {channel_access_token}"),
                "Content-Type": "application/json",
            },
            "timeout_seconds": timeout_seconds,
        });
        let execution = execute_batch(
            executor,
            &http_request,
            &channel_access_token,
            reply_token.as_deref(),
        );
        operation_results.push(json!({
            "index": index,
            "kind": batch.kind.as_str(),
            "ok": execution.ok,
            "message_count": batch.messages.len(),
            "endpoint": url,
            "http_result": execution.public_result,
        }));
        if !execution.ok {
            return Ok(delivery_payload(
                "delivery_failed",
                json!({
                    "ok": false,
                    "delivered": false,
                    "channel_id": channel_id,
                    "reply_token_present": reply_token.is_some(),
                    "message_limit": DEFAULT_MESSAGE_LIMIT,
                    "max_messages_per_request": MAX_MESSAGES_PER_REQUEST,
                    "chunk_count": chunks.len(),
                    "batch_count": batches.len(),
                    "attempted_batch_count": operation_results.len(),
                    "delivered_batch_count": successful_count(&operation_results),
                    "failed_batch_count": failed_count(&operation_results),
                    "operation_results": operation_results,
                    "error": execution.error,
                }),
            ));
        }
    }

    Ok(delivery_payload(
        "delivered",
        json!({
            "ok": true,
            "delivered": true,
            "channel_id": channel_id,
            "reply_token_present": reply_token.is_some(),
            "message_limit": DEFAULT_MESSAGE_LIMIT,
            "max_messages_per_request": MAX_MESSAGES_PER_REQUEST,
            "chunk_count": chunks.len(),
            "batch_count": batches.len(),
            "attempted_batch_count": operation_results.len(),
            "delivered_batch_count": successful_count(&operation_results),
            "failed_batch_count": failed_count(&operation_results),
            "operation_results": operation_results,
            "error": JsonValue::Null,
        }),
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeliveryKind {
    Reply,
    Push,
}

impl DeliveryKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Push => "push",
        }
    }
}

struct DeliveryBatch {
    kind: DeliveryKind,
    messages: Vec<JsonValue>,
}

fn delivery_batches(chunks: &[String], use_reply_token: bool) -> Vec<DeliveryBatch> {
    let mut chunks = VecDeque::from(chunks.to_vec());
    let mut batches = Vec::new();
    if use_reply_token && !chunks.is_empty() {
        batches.push(DeliveryBatch {
            kind: DeliveryKind::Reply,
            messages: take_messages(&mut chunks),
        });
    }
    while !chunks.is_empty() {
        batches.push(DeliveryBatch {
            kind: DeliveryKind::Push,
            messages: take_messages(&mut chunks),
        });
    }
    batches
}

fn take_messages(chunks: &mut VecDeque<String>) -> Vec<JsonValue> {
    (0..MAX_MESSAGES_PER_REQUEST)
        .filter_map(|_| chunks.pop_front())
        .map(|text| json!({"type": "text", "text": text}))
        .collect()
}

struct BatchExecution {
    ok: bool,
    public_result: JsonValue,
    error: JsonValue,
}

fn execute_batch<E>(
    executor: &E,
    request: &JsonValue,
    access_token: &str,
    reply_token: Option<&str>,
) -> BatchExecution
where
    E: LineReplyDeliveryExecutor + ?Sized,
{
    let result = match executor.execute_json_request(request) {
        Ok(result) => result,
        Err(_) => {
            return BatchExecution {
                ok: false,
                public_result: json!({
                    "ok": false,
                    "error_kind": "executor",
                    "message": "LINE Messaging API executor failed.",
                }),
                error: JsonValue::String("LINE Messaging API executor failed.".to_string()),
            }
        }
    };
    let Some(ok) = result.get("ok").and_then(JsonValue::as_bool) else {
        return BatchExecution {
            ok: false,
            public_result: json!({
                "ok": false,
                "error_kind": "contract",
                "message": "LINE Messaging API executor returned an invalid status.",
            }),
            error: JsonValue::String(
                "LINE Messaging API executor returned an invalid status.".to_string(),
            ),
        };
    };
    let public_result = public_http_result(&result, access_token, reply_token);
    let error = if ok {
        JsonValue::Null
    } else {
        JsonValue::String(
            first_error_text(&public_result)
                .unwrap_or_else(|| "LINE Messaging API request failed.".to_string()),
        )
    };
    BatchExecution {
        ok,
        public_result,
        error,
    }
}

fn public_http_result(
    result: &JsonValue,
    access_token: &str,
    reply_token: Option<&str>,
) -> JsonValue {
    let mut public = Map::new();
    for key in [
        "ok",
        "error_kind",
        "status_code",
        "response_kind",
        "reason",
        "message",
        "detail",
        "payload",
    ] {
        if let Some(value) = result.get(key) {
            public.insert(
                key.to_string(),
                sanitize_json(value, access_token, reply_token),
            );
        }
    }
    JsonValue::Object(public)
}

fn sanitize_json(value: &JsonValue, access_token: &str, reply_token: Option<&str>) -> JsonValue {
    match value {
        JsonValue::String(value) => {
            JsonValue::String(sanitize_text(value, access_token, reply_token))
        }
        JsonValue::Array(values) => JsonValue::Array(
            values
                .iter()
                .map(|value| sanitize_json(value, access_token, reply_token))
                .collect(),
        ),
        JsonValue::Object(values) => JsonValue::Object(
            values
                .iter()
                .map(|(key, value)| {
                    let value = if sensitive_key(key) {
                        JsonValue::String(REDACTED.to_string())
                    } else {
                        sanitize_json(value, access_token, reply_token)
                    };
                    (key.clone(), value)
                })
                .collect(),
        ),
        value => value.clone(),
    }
}

fn sanitize_text(value: &str, access_token: &str, reply_token: Option<&str>) -> String {
    let mut sanitized = value.replace(access_token, REDACTED);
    if let Some(reply_token) = reply_token {
        sanitized = sanitized.replace(reply_token, REDACTED);
    }
    sanitized
}

fn sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization" | "channel_access_token" | "access_token" | "replytoken" | "reply_token"
    )
}

fn first_error_text(result: &JsonValue) -> Option<String> {
    ["message", "detail", "reason"]
        .iter()
        .find_map(|key| clean_text(result.get(*key)))
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

fn delivery_payload(state: &str, fields: JsonValue) -> JsonValue {
    let mut payload = fields.as_object().cloned().unwrap_or_default();
    payload.insert(
        "contract".to_string(),
        JsonValue::String(LINE_REPLY_DELIVERY_CONTRACT.to_string()),
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
        "delivery_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    payload.insert(
        "python_line_api_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(payload)
}

fn request_object(value: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| "LINE reply delivery request must be an object.".to_string())
}

fn required_text(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    clean_text(value)
        .ok_or_else(|| format!("LINE reply delivery request requires non-empty `{field}`."))
}

fn required_string(value: Option<&JsonValue>, field: &str) -> Result<String, String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("LINE reply delivery request requires string `{field}`."))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalized_api_base_url(value: Option<&JsonValue>) -> Result<String, String> {
    let value = match value {
        None | Some(JsonValue::Null) => DEFAULT_LINE_API_BASE_URL.to_string(),
        Some(JsonValue::String(value)) => value.trim().trim_end_matches('/').to_string(),
        Some(_) => {
            return Err(
                "LINE reply delivery request field `api_base_url` must be a string or null."
                    .to_string(),
            )
        }
    };
    if !(value.starts_with("http://") || value.starts_with("https://")) {
        return Err(
            "LINE reply delivery request field `api_base_url` must use HTTP or HTTPS.".to_string(),
        );
    }
    Ok(value)
}

fn timeout_seconds(value: Option<&JsonValue>) -> Result<Option<f64>, String> {
    match value {
        None => Ok(Some(DEFAULT_TIMEOUT_SECONDS)),
        Some(JsonValue::Null) => Ok(None),
        Some(value) => {
            let timeout = value.as_f64().ok_or_else(|| {
                "LINE reply delivery request field `timeout_seconds` must be a number or null."
                    .to_string()
            })?;
            if !timeout.is_finite() || timeout <= 0.0 {
                return Err(
                    "LINE reply delivery request field `timeout_seconds` must be greater than zero."
                        .to_string(),
                );
            }
            Ok(Some(timeout))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::VecDeque;

    use super::*;

    #[derive(Default)]
    struct StubExecutor {
        calls: RefCell<Vec<JsonValue>>,
        responses: RefCell<VecDeque<Result<JsonValue, String>>>,
    }

    impl StubExecutor {
        fn with_responses(responses: Vec<Result<JsonValue, String>>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(VecDeque::from(responses)),
            }
        }
    }

    impl LineReplyDeliveryExecutor for StubExecutor {
        fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
            self.calls.borrow_mut().push(request.clone());
            self.responses
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Ok(json!({"ok": true, "payload": {}})))
        }
    }

    fn request(text: &str, reply_token: Option<&str>) -> JsonValue {
        json!({
            "channel_id": "U-line-channel",
            "reply_token": reply_token,
            "text": text,
            "channel_access_token": "line-access-secret",
            "api_base_url": "https://line.example.test/",
            "timeout_seconds": 12.5,
        })
    }

    #[test]
    fn reply_token_uses_reply_then_push_batches_with_exact_headers_and_redaction() {
        let executor = StubExecutor::default();
        let text = "x".repeat(DEFAULT_MESSAGE_LIMIT * MAX_MESSAGES_PER_REQUEST + 1);

        let result = execute_with_line_reply_delivery_executor(
            &executor,
            &request(&text, Some("line-reply-secret")),
        )
        .unwrap();

        assert_eq!(result["delivery_state"], "delivered");
        assert_eq!(result["chunk_count"], 6);
        assert_eq!(result["batch_count"], 2);
        assert_eq!(result["delivered_batch_count"], 2);
        let calls = executor.calls.borrow();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            calls[0]["url"],
            "https://line.example.test/v2/bot/message/reply"
        );
        assert_eq!(calls[0]["payload"]["replyToken"], "line-reply-secret");
        assert_eq!(calls[0]["payload"]["messages"].as_array().unwrap().len(), 5);
        assert_eq!(
            calls[0]["headers"]["Authorization"],
            "Bearer line-access-secret"
        );
        assert_eq!(calls[0]["timeout_seconds"], 12.5);
        assert_eq!(
            calls[1]["url"],
            "https://line.example.test/v2/bot/message/push"
        );
        assert_eq!(calls[1]["payload"]["to"], "U-line-channel");
        assert_eq!(calls[1]["payload"]["messages"].as_array().unwrap().len(), 1);
        let rendered = result.to_string();
        assert!(!rendered.contains("line-access-secret"));
        assert!(!rendered.contains("line-reply-secret"));
        assert_eq!(result["python_line_api_allowed"], false);
    }

    #[test]
    fn push_only_and_empty_text_preserve_splitter_and_unicode_character_semantics() {
        let executor = StubExecutor::default();
        let unicode = "你".repeat(DEFAULT_MESSAGE_LIMIT + 1);
        let unicode_result =
            execute_with_line_reply_delivery_executor(&executor, &request(&unicode, None)).unwrap();
        assert_eq!(unicode_result["chunk_count"], 2);
        let calls = executor.calls.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(
            calls[0]["url"],
            "https://line.example.test/v2/bot/message/push"
        );
        let messages = calls[0]["payload"]["messages"].as_array().unwrap();
        assert_eq!(
            messages[0]["text"].as_str().unwrap().chars().count(),
            DEFAULT_MESSAGE_LIMIT
        );
        assert_eq!(messages[1]["text"].as_str().unwrap(), "你");
        drop(calls);

        let empty_executor = StubExecutor::default();
        let empty =
            execute_with_line_reply_delivery_executor(&empty_executor, &request("   ", None))
                .unwrap();
        assert_eq!(empty["chunk_count"], 1);
        assert_eq!(
            empty_executor.calls.borrow()[0]["payload"]["messages"][0]["text"],
            "(empty)"
        );
    }

    #[test]
    fn http_failure_stops_early_and_sanitizes_nested_secret_values() {
        let executor = StubExecutor::with_responses(vec![
            Ok(json!({"ok": true, "payload": {}})),
            Ok(json!({
                "ok": false,
                "error_kind": "http",
                "status_code": 429,
                "message": "line-access-secret line-reply-secret",
                "detail": {
                    "Authorization": "Bearer line-access-secret",
                    "replyToken": "line-reply-secret",
                }
            })),
            Ok(json!({"ok": true})),
        ]);
        let text = "y".repeat(DEFAULT_MESSAGE_LIMIT * MAX_MESSAGES_PER_REQUEST * 2 + 1);

        let result = execute_with_line_reply_delivery_executor(
            &executor,
            &request(&text, Some("line-reply-secret")),
        )
        .unwrap();

        assert_eq!(result["delivery_state"], "delivery_failed");
        assert_eq!(result["batch_count"], 3);
        assert_eq!(result["attempted_batch_count"], 2);
        assert_eq!(result["delivered_batch_count"], 1);
        assert_eq!(result["failed_batch_count"], 1);
        assert_eq!(executor.calls.borrow().len(), 2);
        let rendered = result.to_string();
        assert!(!rendered.contains("line-access-secret"));
        assert!(!rendered.contains("line-reply-secret"));
        assert!(rendered.contains(REDACTED));
    }

    #[test]
    fn executor_and_malformed_envelope_failures_are_stable_and_secret_safe() {
        let executor = StubExecutor::with_responses(vec![Err(
            "failed with line-access-secret and line-reply-secret".to_string(),
        )]);
        let failed = execute_with_line_reply_delivery_executor(
            &executor,
            &request("hello", Some("line-reply-secret")),
        )
        .unwrap();
        assert_eq!(failed["delivery_state"], "delivery_failed");
        assert_eq!(failed["error"], "LINE Messaging API executor failed.");
        assert!(!failed.to_string().contains("line-access-secret"));

        let malformed = StubExecutor::with_responses(vec![Ok(json!({
            "status": "ok",
            "Authorization": "Bearer line-access-secret",
        }))]);
        let malformed_result = execute_with_line_reply_delivery_executor(
            &malformed,
            &request("hello", Some("line-reply-secret")),
        )
        .unwrap();
        assert_eq!(
            malformed_result["error"],
            "LINE Messaging API executor returned an invalid status."
        );
        assert!(!malformed_result.to_string().contains("line-access-secret"));
    }

    #[test]
    fn invalid_request_shape_and_fields_fail_before_http_execution() {
        let executor = StubExecutor::default();
        assert!(execute_with_line_reply_delivery_executor(&executor, &json!([])).is_err());
        for invalid in [
            json!({"channel_id": "", "text": "hello", "channel_access_token": "token"}),
            json!({"channel_id": "U1", "text": 1, "channel_access_token": "token"}),
            json!({"channel_id": "U1", "text": "hello", "channel_access_token": ""}),
            json!({
                "channel_id": "U1",
                "text": "hello",
                "channel_access_token": "token",
                "api_base_url": "file:///tmp/line"
            }),
            json!({
                "channel_id": "U1",
                "text": "hello",
                "channel_access_token": "token",
                "timeout_seconds": 0
            }),
        ] {
            assert!(
                execute_with_line_reply_delivery_executor(&executor, &invalid).is_err(),
                "{invalid}"
            );
        }
        assert!(executor.calls.borrow().is_empty());
    }
}
