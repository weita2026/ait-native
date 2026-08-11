use std::cell::RefCell;
use std::collections::VecDeque;

use ait_core::json_support::{json, JsonValue};

use super::*;

struct StubHttpExecutor {
    results: RefCell<VecDeque<Result<JsonValue, String>>>,
    requests: RefCell<Vec<JsonValue>>,
}

impl StubHttpExecutor {
    fn new(results: Vec<Result<JsonValue, String>>) -> Self {
        Self {
            results: RefCell::new(results.into()),
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramApiJsonHttpExecutor for StubHttpExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.borrow_mut().push(request.clone());
        self.results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected-http-executor-secret".to_string()))
    }
}

struct StubSleeper {
    results: RefCell<VecDeque<Result<(), String>>>,
    sleeps: RefCell<Vec<f64>>,
}

impl StubSleeper {
    fn new() -> Self {
        Self {
            results: RefCell::new(VecDeque::new()),
            sleeps: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramApiRetrySleeper for StubSleeper {
    fn sleep_seconds(&self, seconds: f64) -> Result<(), String> {
        self.sleeps.borrow_mut().push(seconds);
        self.results.borrow_mut().pop_front().unwrap_or(Ok(()))
    }
}

fn http_success(payload: JsonValue) -> Result<JsonValue, String> {
    Ok(json!({
        "ok": true,
        "status_code": 200,
        "response_kind": "json",
        "payload": payload,
        "url": "https://api.telegram.org/bot123:bot-secret/secret-method",
    }))
}

fn http_failure(kind: &str, status_code: Option<i64>, secret: &str) -> Result<JsonValue, String> {
    Ok(json!({
        "ok": false,
        "error_kind": kind,
        "status_code": status_code.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "url": "https://api.telegram.org/bot123:bot-secret/getUpdates",
        "message": secret,
        "detail": {"bot_token": "123:bot-secret"},
    }))
}

fn get_updates_request() -> JsonValue {
    json!({
        "operation": "get_updates",
        "bot_token": "123:bot-secret",
        "offset": 7,
        "timeout_seconds": 5,
        "request_timeout_seconds": 3.0,
    })
}

fn execute(executor: &StubHttpExecutor, sleeper: &StubSleeper, request: &JsonValue) -> JsonValue {
    execute_with_telegram_api_json_ports(executor, sleeper, request)
        .expect("Telegram JSON API execution")
}

#[test]
fn get_updates_executes_exact_planned_request_and_returns_only_updates() {
    let executor = StubHttpExecutor::new(vec![http_success(json!({
        "ok": true,
        "result": [{
            "update_id": 7,
            "message": {"text": "hello 123:bot-secret", "token": "123:bot-secret"},
        }],
    }))]);
    let sleeper = StubSleeper::new();

    let outcome = execute(&executor, &sleeper, &get_updates_request());

    assert_eq!(outcome["contract"], CONTRACT);
    assert_eq!(outcome["telegram_api_state"], "completed");
    assert_eq!(outcome["operation"], "get_updates");
    assert_eq!(outcome["telegram_method"], "getUpdates");
    assert_eq!(outcome["attempts"], 1);
    assert_eq!(outcome["updates"][0]["update_id"], 7);
    assert_eq!(outcome["updates"][0]["message"]["text"], "hello [redacted]");
    assert_eq!(outcome["updates"][0]["message"]["token"], REDACTED);
    assert_eq!(outcome["python_telegram_api_allowed"], false);
    assert_eq!(executor.requests.borrow()[0]["method"], "GET");
    assert_eq!(executor.requests.borrow()[0]["timeout_seconds"], 15.0);
    assert!(executor.requests.borrow()[0]["url"]
        .as_str()
        .unwrap_or_default()
        .contains("getUpdates?offset=7&timeout=5"));
    assert!(executor.requests.borrow()[0]["url"]
        .as_str()
        .unwrap_or_default()
        .contains("123:bot-secret"));
    assert!(!outcome.to_string().contains("123:bot-secret"));
    assert!(!outcome.to_string().contains("api.telegram.org"));
    assert!(sleeper.sleeps.borrow().is_empty());
}

#[test]
fn get_file_returns_metadata_without_raw_telegram_payload() {
    let executor = StubHttpExecutor::new(vec![http_success(json!({
        "ok": true,
        "result": {
            "file_id": "f-1",
            "file_path": "voice/file.ogg",
            "bot_token": "123:bot-secret",
            "unexpected": "raw-result-secret",
        },
        "secret_extra": "raw-response-secret",
    }))]);
    let sleeper = StubSleeper::new();
    let outcome = execute(
        &executor,
        &sleeper,
        &json!({
            "operation": "get_file",
            "bot_token": "123:bot-secret",
            "file_id": "f-1",
            "request_timeout_seconds": 9.0,
        }),
    );

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["file_info"]["file_path"], "voice/file.ogg");
    assert!(outcome["file_info"].get("bot_token").is_none());
    assert!(outcome["file_info"].get("unexpected").is_none());
    assert_eq!(executor.requests.borrow()[0]["method"], "POST");
    assert_eq!(
        executor.requests.borrow()[0]["payload"],
        json!({"file_id": "f-1"})
    );
    assert_eq!(executor.requests.borrow()[0]["timeout_seconds"], 9.0);
    assert!(!outcome.to_string().contains("raw-response-secret"));
    assert!(!outcome.to_string().contains("raw-result-secret"));
    assert!(!outcome.to_string().contains("123:bot-secret"));
}

#[test]
fn send_message_succeeds_without_echoing_text_or_response() {
    let executor = StubHttpExecutor::new(vec![http_success(json!({
        "ok": true,
        "result": {"message_id": 9, "text": "private message text"},
    }))]);
    let sleeper = StubSleeper::new();
    let outcome = execute(
        &executor,
        &sleeper,
        &json!({
            "operation": "send_message",
            "bot_token": "123:bot-secret",
            "chat_id": 42,
            "text": "private message text",
            "parse_mode": "MarkdownV2",
        }),
    );

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["sent"], true);
    assert_eq!(outcome["updates"], JsonValue::Null);
    assert_eq!(outcome["file_info"], JsonValue::Null);
    assert_eq!(executor.requests.borrow()[0]["payload"]["chat_id"], 42);
    assert_eq!(
        executor.requests.borrow()[0]["payload"]["text"],
        "private message text"
    );
    assert!(!outcome.to_string().contains("private message text"));
    assert!(!outcome.to_string().contains("123:bot-secret"));
}

#[test]
fn telegram_hosted_attachment_uses_json_without_echoing_reference_or_response() {
    let executor = StubHttpExecutor::new(vec![http_success(json!({
        "ok": true,
        "result": {"message_id": 10, "document": {"file_id": "remote-secret"}},
    }))]);
    let sleeper = StubSleeper::new();
    let outcome = execute(
        &executor,
        &sleeper,
        &json!({
            "operation": "send_attachment",
            "bot_token": "123:bot-secret",
            "chat_id": 42,
            "method_name": "sendDocument",
            "attachment": {
                "telegram_file_id": "remote-secret",
                "caption": "private caption",
            },
        }),
    );

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["sent"], true);
    assert_eq!(outcome["telegram_method"], "sendDocument");
    assert_eq!(executor.requests.borrow()[0]["method"], "POST");
    assert_eq!(
        executor.requests.borrow()[0]["payload"]["document"],
        "remote-secret"
    );
    assert!(!outcome.to_string().contains("remote-secret"));
    assert!(!outcome.to_string().contains("private caption"));
    assert!(!outcome.to_string().contains("123:bot-secret"));
}

#[test]
fn polling_retries_timeout_and_transport_failures_in_exact_order_then_recovers() {
    let executor = StubHttpExecutor::new(vec![
        http_failure("timeout", None, "timeout with 123:bot-secret"),
        http_failure("transport", None, "transport with 123:bot-secret"),
        http_success(json!({"ok": true, "result": []})),
    ]);
    let sleeper = StubSleeper::new();

    let outcome = execute(&executor, &sleeper, &get_updates_request());

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["attempts"], 3);
    assert_eq!(outcome["retry_count"], 2);
    assert_eq!(outcome["retry_delays_seconds"], json!([1.0, 2.0]));
    assert_eq!(&*sleeper.sleeps.borrow(), &[1.0, 2.0]);
    assert_eq!(executor.requests.borrow().len(), 3);
    assert!(!outcome.to_string().contains("123:bot-secret"));
}

#[test]
fn polling_retry_exhaustion_stops_after_four_attempts_without_secrets() {
    let executor = StubHttpExecutor::new(
        (0..4)
            .map(|_| http_failure("transport", None, "network 123:bot-secret"))
            .collect(),
    );
    let sleeper = StubSleeper::new();

    let outcome = execute(&executor, &sleeper, &get_updates_request());

    assert_eq!(outcome["telegram_api_state"], "retry_exhausted");
    assert_eq!(outcome["ok"], false);
    assert_eq!(outcome["attempts"], 4);
    assert_eq!(outcome["max_attempts"], 4);
    assert_eq!(outcome["retry_exhausted"], true);
    assert_eq!(outcome["retry_delays_seconds"], json!([1.0, 2.0, 4.0]));
    assert_eq!(executor.requests.borrow().len(), 4);
    assert!(!outcome.to_string().contains("123:bot-secret"));
}

#[test]
fn delivery_retry_exhaustion_stops_after_three_attempts() {
    let executor = StubHttpExecutor::new(
        (0..3)
            .map(|_| http_failure("timeout", None, "delivery 123:bot-secret"))
            .collect(),
    );
    let sleeper = StubSleeper::new();

    let outcome = execute(
        &executor,
        &sleeper,
        &json!({
            "operation": "get_file",
            "bot_token": "123:bot-secret",
            "file_id": "f-1",
        }),
    );

    assert_eq!(outcome["telegram_api_state"], "retry_exhausted");
    assert_eq!(outcome["attempts"], 3);
    assert_eq!(outcome["max_attempts"], 3);
    assert_eq!(outcome["retry_delays_seconds"], json!([1.0, 2.0]));
    assert_eq!(executor.requests.borrow().len(), 3);
    assert_eq!(&*sleeper.sleeps.borrow(), &[1.0, 2.0]);
}

#[test]
fn non_retryable_http_failure_stops_after_one_attempt() {
    let executor = StubHttpExecutor::new(vec![http_failure(
        "http",
        Some(503),
        "HTTP detail 123:bot-secret",
    )]);
    let sleeper = StubSleeper::new();

    let outcome = execute(&executor, &sleeper, &get_updates_request());

    assert_eq!(outcome["telegram_api_state"], "http_failed");
    assert_eq!(outcome["error_kind"], "http");
    assert_eq!(outcome["http_status_code"], 503);
    assert_eq!(outcome["attempts"], 1);
    assert_eq!(outcome["retry_exhausted"], false);
    assert!(sleeper.sleeps.borrow().is_empty());
    assert!(!outcome.to_string().contains("HTTP detail"));
}

#[test]
fn telegram_api_rejection_is_generic_and_not_retried() {
    let executor = StubHttpExecutor::new(vec![http_success(json!({
        "ok": false,
        "error_code": 400,
        "description": "Bad Request with 123:bot-secret and private text",
    }))]);
    let sleeper = StubSleeper::new();

    let outcome = execute(&executor, &sleeper, &get_updates_request());

    assert_eq!(outcome["telegram_api_state"], "telegram_api_failed");
    assert_eq!(outcome["error_kind"], "telegram_api");
    assert_eq!(outcome["attempts"], 1);
    assert!(sleeper.sleeps.borrow().is_empty());
    assert!(!outcome.to_string().contains("Bad Request"));
    assert!(!outcome.to_string().contains("123:bot-secret"));
    assert!(!outcome.to_string().contains("private text"));
}

#[test]
fn send_message_classifies_entity_parse_rejection_without_exposing_description() {
    let executor = StubHttpExecutor::new(vec![http_success(json!({
        "ok": false,
        "error_code": 400,
        "description": "Bad Request: can't parse entities near private text 123:bot-secret",
    }))]);
    let sleeper = StubSleeper::new();

    let outcome = execute(
        &executor,
        &sleeper,
        &json!({
            "operation": "send_message",
            "bot_token": "123:bot-secret",
            "chat_id": 42,
            "text": "<b>private text</b>",
            "parse_mode": "HTML",
        }),
    );

    assert_eq!(outcome["telegram_api_state"], "telegram_api_failed");
    assert_eq!(outcome["telegram_parse_error"], true);
    assert_eq!(outcome["attempts"], 1);
    assert!(!outcome.to_string().contains("can't parse entities"));
    assert!(!outcome.to_string().contains("private text"));
    assert!(!outcome.to_string().contains("123:bot-secret"));
}

#[test]
fn send_message_classifies_real_http_400_entity_parse_response() {
    let executor = StubHttpExecutor::new(vec![Ok(json!({
        "ok": false,
        "error_kind": "http",
        "status_code": 400,
        "url": "https://api.telegram.org/bot123:bot-secret/sendMessage",
        "message": "POST failed with private text 123:bot-secret",
        "detail": "{\"ok\":false,\"error_code\":400,\"description\":\"Bad Request: can't parse entities near private text 123:bot-secret\"}",
    }))]);
    let sleeper = StubSleeper::new();

    let outcome = execute(
        &executor,
        &sleeper,
        &json!({
            "operation": "send_message",
            "bot_token": "123:bot-secret",
            "chat_id": 42,
            "text": "<b>private text</b>",
            "parse_mode": "HTML",
        }),
    );

    assert_eq!(outcome["telegram_api_state"], "telegram_api_failed");
    assert_eq!(outcome["telegram_parse_error"], true);
    assert_eq!(outcome["http_status_code"], 400);
    assert_eq!(outcome["attempts"], 1);
    assert!(!outcome.to_string().contains("can't parse entities"));
    assert!(!outcome.to_string().contains("private text"));
    assert!(!outcome.to_string().contains("123:bot-secret"));
    assert!(!outcome.to_string().contains("api.telegram.org"));
}

#[test]
fn invalid_successful_telegram_envelope_fails_closed_without_raw_payload() {
    let executor = StubHttpExecutor::new(vec![http_success(json!({
        "ok": true,
        "result": "invalid result with 123:bot-secret",
    }))]);
    let sleeper = StubSleeper::new();

    let outcome = execute(&executor, &sleeper, &get_updates_request());

    assert_eq!(outcome["telegram_api_state"], "result_contract_failed");
    assert_eq!(outcome["error_kind"], "response");
    assert_eq!(outcome["attempts"], 1);
    assert!(!outcome.to_string().contains("invalid result"));
    assert!(!outcome.to_string().contains("123:bot-secret"));
    assert!(sleeper.sleeps.borrow().is_empty());
}

#[test]
fn planning_rejection_does_not_execute_http() {
    let executor = StubHttpExecutor::new(Vec::new());
    let sleeper = StubSleeper::new();

    let outcome = execute(
        &executor,
        &sleeper,
        &json!({"operation": "get_updates", "offset": 1, "timeout_seconds": 2}),
    );

    assert_eq!(outcome["telegram_api_state"], "planning_rejected");
    assert_eq!(outcome["operation"], "get_updates");
    assert_eq!(outcome["attempts"], 0);
    assert!(executor.requests.borrow().is_empty());
}

#[test]
fn bytes_and_multipart_operations_are_rejected_without_fallback() {
    let executor = StubHttpExecutor::new(Vec::new());
    let sleeper = StubSleeper::new();
    let bytes = execute(
        &executor,
        &sleeper,
        &json!({
            "operation": "download_file",
            "bot_token": "123:bot-secret",
            "file_path": "voice/a.ogg",
        }),
    );
    let multipart = execute(
        &executor,
        &sleeper,
        &json!({
            "operation": "send_attachment",
            "bot_token": "123:bot-secret",
            "chat_id": 1,
            "method_name": "sendDocument",
            "attachment": {"local_path": "/tmp/private.pdf"},
        }),
    );

    assert_eq!(
        bytes["telegram_api_state"],
        "unsupported_operation_or_transport"
    );
    assert_eq!(bytes["operation"], "download_file");
    assert_eq!(
        multipart["telegram_api_state"],
        "unsupported_operation_or_transport"
    );
    assert_eq!(multipart["operation"], "send_attachment");
    assert_eq!(bytes["python_telegram_api_allowed"], false);
    assert_eq!(multipart["python_http_execution_allowed"], false);
    assert!(executor.requests.borrow().is_empty());
}

#[test]
fn executor_and_http_contract_failures_are_secret_safe() {
    let executor = StubHttpExecutor::new(vec![Err("executor 123:bot-secret".to_string())]);
    let sleeper = StubSleeper::new();
    let executor_failure = execute(&executor, &sleeper, &get_updates_request());
    assert_eq!(executor_failure["telegram_api_state"], "executor_failed");
    assert_eq!(executor_failure["error_kind"], "executor");
    assert!(!executor_failure.to_string().contains("123:bot-secret"));

    let invalid_executor = StubHttpExecutor::new(vec![Ok(json!({
        "secret": "invalid 123:bot-secret",
    }))]);
    let invalid = execute(&invalid_executor, &sleeper, &get_updates_request());
    assert_eq!(invalid["telegram_api_state"], "http_contract_failed");
    assert_eq!(invalid["error_kind"], "contract");
    assert!(!invalid.to_string().contains("123:bot-secret"));
}

#[test]
fn retry_sleep_failure_stops_before_another_http_attempt() {
    let executor = StubHttpExecutor::new(vec![http_failure(
        "timeout",
        None,
        "timeout 123:bot-secret",
    )]);
    let sleeper = StubSleeper::new();
    sleeper
        .results
        .borrow_mut()
        .push_back(Err("sleep 123:bot-secret".to_string()));

    let outcome = execute(&executor, &sleeper, &get_updates_request());

    assert_eq!(outcome["telegram_api_state"], "retry_sleep_failed");
    assert_eq!(outcome["error_kind"], "sleep");
    assert_eq!(outcome["attempts"], 1);
    assert_eq!(outcome["retry_count"], 0);
    assert_eq!(&*sleeper.sleeps.borrow(), &[1.0]);
    assert_eq!(executor.requests.borrow().len(), 1);
    assert!(!outcome.to_string().contains("123:bot-secret"));
}
