use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ait_core::json_support::{json, JsonValue};

use super::*;

#[derive(Default)]
struct StubExecutor {
    json_results: RefCell<VecDeque<Result<JsonValue, String>>>,
    multipart_results: RefCell<VecDeque<Result<JsonValue, String>>>,
    bytes_results: RefCell<VecDeque<Result<AgentTransportHttpBytesExecution, String>>>,
    file_results: RefCell<VecDeque<Result<Vec<u8>, String>>>,
    json_requests: RefCell<Vec<JsonValue>>,
    multipart_requests: RefCell<Vec<JsonValue>>,
    multipart_payloads: RefCell<Vec<Vec<u8>>>,
    bytes_requests: RefCell<Vec<JsonValue>>,
    file_paths: RefCell<Vec<PathBuf>>,
}

impl TelegramApiTransportExecutor for StubExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.json_requests.borrow_mut().push(request.clone());
        self.json_results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected-json-executor-secret".to_string()))
    }

    fn execute_multipart_request(
        &self,
        request: &JsonValue,
        file_bytes: &[u8],
    ) -> Result<JsonValue, String> {
        self.multipart_requests.borrow_mut().push(request.clone());
        self.multipart_payloads
            .borrow_mut()
            .push(file_bytes.to_vec());
        self.multipart_results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected-multipart-executor-secret".to_string()))
    }

    fn execute_bytes_request(
        &self,
        request: &JsonValue,
    ) -> Result<AgentTransportHttpBytesExecution, String> {
        self.bytes_requests.borrow_mut().push(request.clone());
        self.bytes_results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected-bytes-executor-secret".to_string()))
    }

    fn read_attachment_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        self.file_paths.borrow_mut().push(path.to_path_buf());
        self.file_results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected-file-reader-secret".to_string()))
    }
}

#[derive(Default)]
struct StubSleeper {
    results: RefCell<VecDeque<Result<(), String>>>,
    sleeps: RefCell<Vec<f64>>,
}

impl TelegramApiRetrySleeper for StubSleeper {
    fn sleep_seconds(&self, seconds: f64) -> Result<(), String> {
        self.sleeps.borrow_mut().push(seconds);
        self.results.borrow_mut().pop_front().unwrap_or(Ok(()))
    }
}

fn execute(
    executor: &StubExecutor,
    sleeper: &StubSleeper,
    request: JsonValue,
) -> TelegramApiTransportExecution {
    execute_with_telegram_api_transport_ports(executor, sleeper, &request)
        .expect("Telegram transport execution")
}

fn download_request() -> JsonValue {
    json!({
        "operation": "download_file",
        "bot_token": "123:bot-secret",
        "file_path": "voice/private.ogg",
        "request_timeout_seconds": 9.0,
    })
}

fn local_attachment_request(method_name: &str, local_path: &str) -> JsonValue {
    json!({
        "operation": "send_attachment",
        "bot_token": "123:bot-secret",
        "chat_id": 42,
        "method_name": method_name,
        "attachment": {
            "local_path": local_path,
            "file_name": "private-name.bin",
            "mime_type": "application/octet-stream",
            "caption": "private caption",
        },
        "extra_fields": {"protect_content": true},
        "request_timeout_seconds": 11.0,
    })
}

fn bytes_success(payload: Vec<u8>) -> AgentTransportHttpBytesExecution {
    AgentTransportHttpBytesExecution::Success {
        method: "GET".to_string(),
        url: "https://api.telegram.org/file/bot123:bot-secret/voice/private.ogg".to_string(),
        status_code: 200,
        payload,
    }
}

fn http_failure(kind: &str, secret: &str) -> JsonValue {
    json!({
        "ok": false,
        "error_kind": kind,
        "status_code": if kind == "http" { json!(503) } else { JsonValue::Null },
        "url": "https://api.telegram.org/bot123:bot-secret/private",
        "message": secret,
        "detail": {"local_path": "/private/secret.bin"},
    })
}

fn telegram_success() -> JsonValue {
    json!({
        "ok": true,
        "status_code": 200,
        "response_kind": "json",
        "payload": {
            "ok": true,
            "result": {
                "message_id": 9,
                "caption": "private caption",
                "document": {"file_id": "private-file-id"},
            },
        },
        "url": "https://api.telegram.org/bot123:bot-secret/sendDocument",
    })
}

fn assert_secret_safe(metadata: &JsonValue) {
    let rendered = metadata.to_string();
    for secret in [
        "123:bot-secret",
        "voice/private.ogg",
        "/private/secret.bin",
        "/private/local.bin",
        "private-name.bin",
        "private caption",
        "private-file-id",
        "executor-secret",
        "reader-secret",
        "telegram-secret",
    ] {
        assert!(!rendered.contains(secret), "metadata leaked {secret}");
    }
    assert_eq!(metadata["downloaded_bytes_exposed"], false);
    assert_eq!(metadata["token_bearing_url_exposed"], false);
    assert_eq!(metadata["local_path_exposed"], false);
    assert_eq!(metadata["multipart_fields_exposed"], false);
    assert_eq!(metadata["file_name_exposed"], false);
}

#[test]
fn download_retains_exact_bytes_out_of_band_with_count_only_metadata() {
    let executor = StubExecutor::default();
    executor
        .bytes_results
        .borrow_mut()
        .push_back(Ok(bytes_success(vec![0, 255, 1, 2, 3])));
    let sleeper = StubSleeper::default();

    let outcome = execute(&executor, &sleeper, download_request());

    assert_eq!(outcome.downloaded_bytes(), Some(&[0, 255, 1, 2, 3][..]));
    let metadata = outcome.metadata();
    assert_eq!(metadata["contract"], CONTRACT);
    assert_eq!(metadata["telegram_api_state"], "completed");
    assert_eq!(metadata["operation"], "download_file");
    assert_eq!(metadata["telegram_method"], "downloadFile");
    assert_eq!(metadata["transport"], "bytes");
    assert_eq!(metadata["downloaded"], true);
    assert_eq!(metadata["byte_count"], 5);
    assert_eq!(metadata["attempts"], 1);
    assert_eq!(executor.bytes_requests.borrow()[0]["method"], "GET");
    assert_eq!(executor.bytes_requests.borrow()[0]["timeout_seconds"], 9.0);
    assert!(executor.bytes_requests.borrow()[0]["url"]
        .as_str()
        .unwrap_or_default()
        .contains("123:bot-secret"));
    assert!(executor.json_requests.borrow().is_empty());
    assert!(executor.multipart_requests.borrow().is_empty());
    assert_secret_safe(metadata);
    let debug = format!("{outcome:?}");
    assert!(debug.contains("downloaded_byte_count: Some(5)"));
    assert!(!debug.contains("[0, 255, 1, 2, 3]"));
}

#[test]
fn bytes_transport_retries_timeout_and_transport_then_returns_exact_payload() {
    let executor = StubExecutor::default();
    executor.bytes_results.borrow_mut().extend([
        Ok(AgentTransportHttpBytesExecution::Error(http_failure(
            "timeout",
            "timeout executor-secret",
        ))),
        Ok(AgentTransportHttpBytesExecution::Error(http_failure(
            "transport",
            "transport executor-secret",
        ))),
        Ok(bytes_success(vec![7, 8, 9])),
    ]);
    let sleeper = StubSleeper::default();

    let outcome = execute(&executor, &sleeper, download_request());

    assert_eq!(outcome.downloaded_bytes(), Some(&[7, 8, 9][..]));
    assert_eq!(outcome.metadata()["attempts"], 3);
    assert_eq!(outcome.metadata()["retry_count"], 2);
    assert_eq!(
        outcome.metadata()["retry_delays_seconds"],
        json!([1.0, 2.0])
    );
    assert_eq!(&*sleeper.sleeps.borrow(), &[1.0, 2.0]);
    assert_eq!(executor.bytes_requests.borrow().len(), 3);
    assert_secret_safe(outcome.metadata());
}

#[test]
fn bytes_retry_exhaustion_nonretryable_errors_and_sleep_failure_are_stable() {
    let exhausted_executor = StubExecutor::default();
    exhausted_executor
        .bytes_results
        .borrow_mut()
        .extend((0..3).map(|_| {
            Ok(AgentTransportHttpBytesExecution::Error(http_failure(
                "transport",
                "executor-secret",
            )))
        }));
    let sleeper = StubSleeper::default();
    let exhausted = execute(&exhausted_executor, &sleeper, download_request());
    assert_eq!(
        exhausted.metadata()["telegram_api_state"],
        "retry_exhausted"
    );
    assert_eq!(exhausted.metadata()["attempts"], 3);
    assert_eq!(exhausted.metadata()["retry_exhausted"], true);
    assert!(exhausted.downloaded_bytes().is_none());
    assert_secret_safe(exhausted.metadata());

    let http_executor = StubExecutor::default();
    http_executor.bytes_results.borrow_mut().push_back(Ok(
        AgentTransportHttpBytesExecution::Error(http_failure("http", "executor-secret")),
    ));
    let http = execute(&http_executor, &StubSleeper::default(), download_request());
    assert_eq!(http.metadata()["telegram_api_state"], "http_failed");
    assert_eq!(http.metadata()["error_kind"], "http");
    assert_eq!(http.metadata()["attempts"], 1);

    let sleep_executor = StubExecutor::default();
    sleep_executor.bytes_results.borrow_mut().push_back(Ok(
        AgentTransportHttpBytesExecution::Error(http_failure("timeout", "executor-secret")),
    ));
    let sleep = StubSleeper::default();
    sleep
        .results
        .borrow_mut()
        .push_back(Err("sleep executor-secret".to_string()));
    let sleep_failed = execute(&sleep_executor, &sleep, download_request());
    assert_eq!(
        sleep_failed.metadata()["telegram_api_state"],
        "retry_sleep_failed"
    );
    assert_eq!(sleep_failed.metadata()["attempts"], 1);
    assert_eq!(sleep_failed.metadata()["retry_count"], 0);
    assert_secret_safe(sleep_failed.metadata());
}

#[test]
fn malformed_and_failed_bytes_executors_fail_closed_without_payloads() {
    let malformed_executor = StubExecutor::default();
    malformed_executor.bytes_results.borrow_mut().push_back(Ok(
        AgentTransportHttpBytesExecution::Error(json!({
            "secret": "executor-secret",
        })),
    ));
    let malformed = execute(
        &malformed_executor,
        &StubSleeper::default(),
        download_request(),
    );
    assert_eq!(
        malformed.metadata()["telegram_api_state"],
        "http_contract_failed"
    );
    assert!(malformed.downloaded_bytes().is_none());
    assert_secret_safe(malformed.metadata());

    let failed_executor = StubExecutor::default();
    failed_executor
        .bytes_results
        .borrow_mut()
        .push_back(Err("bytes executor-secret".to_string()));
    let failed = execute(
        &failed_executor,
        &StubSleeper::default(),
        download_request(),
    );
    assert_eq!(failed.metadata()["telegram_api_state"], "executor_failed");
    assert!(failed.downloaded_bytes().is_none());
    assert_secret_safe(failed.metadata());
}

#[test]
fn local_attachment_reads_once_and_executes_bounded_multipart_request() {
    let executor = StubExecutor::default();
    executor
        .file_results
        .borrow_mut()
        .push_back(Ok(vec![37, 80, 68, 70, 0, 255]));
    executor
        .multipart_results
        .borrow_mut()
        .push_back(Ok(telegram_success()));
    let sleeper = StubSleeper::default();

    let outcome = execute(
        &executor,
        &sleeper,
        local_attachment_request("sendDocument", "/private/local.bin"),
    );

    let metadata = outcome.metadata();
    assert_eq!(metadata["telegram_api_state"], "completed");
    assert_eq!(metadata["operation"], "send_attachment");
    assert_eq!(metadata["telegram_method"], "sendDocument");
    assert_eq!(metadata["transport"], "multipart");
    assert_eq!(metadata["sent"], true);
    assert_eq!(metadata["byte_count"], 6);
    assert!(outcome.downloaded_bytes().is_none());
    assert_eq!(
        &*executor.file_paths.borrow(),
        &[PathBuf::from("/private/local.bin")]
    );
    assert_eq!(executor.multipart_requests.borrow().len(), 1);
    let requests = executor.multipart_requests.borrow();
    let request = &requests[0];
    assert_eq!(request["file_field"], "document");
    assert_eq!(request["file_name"], "private-name.bin");
    assert_eq!(request["mime_type"], "application/octet-stream");
    assert_eq!(request["fields"]["chat_id"], 42);
    assert_eq!(request["fields"]["caption"], "private caption");
    assert_eq!(request["fields"]["protect_content"], true);
    assert!(request.get("file_bytes").is_none());
    assert_eq!(
        &*executor.multipart_payloads.borrow(),
        &[vec![37, 80, 68, 70, 0, 255]]
    );
    assert_eq!(request["timeout_seconds"], 11.0);
    assert!(request["boundary"]
        .as_str()
        .unwrap_or_default()
        .starts_with("aittelegram-"));
    assert!(request["url"]
        .as_str()
        .unwrap_or_default()
        .contains("123:bot-secret/sendDocument"));
    drop(requests);
    assert_secret_safe(metadata);
}

#[test]
fn all_supported_local_attachment_methods_use_their_exact_file_fields() {
    for (method, expected_field) in [
        ("sendAudio", "audio"),
        ("sendPhoto", "photo"),
        ("sendDocument", "document"),
    ] {
        let executor = StubExecutor::default();
        executor.file_results.borrow_mut().push_back(Ok(vec![1]));
        executor
            .multipart_results
            .borrow_mut()
            .push_back(Ok(telegram_success()));
        let outcome = execute(
            &executor,
            &StubSleeper::default(),
            local_attachment_request(method, "/private/local.bin"),
        );
        assert_eq!(outcome.metadata()["ok"], true, "method={method}");
        assert_eq!(outcome.metadata()["telegram_method"], method);
        assert_eq!(
            executor.multipart_requests.borrow()[0]["file_field"],
            expected_field
        );
    }
}

#[test]
fn telegram_hosted_attachment_uses_json_through_the_typed_surface() {
    let executor = StubExecutor::default();
    executor
        .json_results
        .borrow_mut()
        .push_back(Ok(telegram_success()));
    let outcome = execute(
        &executor,
        &StubSleeper::default(),
        json!({
            "operation": "send_attachment",
            "bot_token": "123:bot-secret",
            "chat_id": 42,
            "method_name": "sendDocument",
            "attachment": {
                "telegram_file_id": "private-file-id",
                "caption": "private caption",
            },
        }),
    );

    assert_eq!(outcome.metadata()["contract"], CONTRACT);
    assert_eq!(outcome.metadata()["transport"], "json");
    assert_eq!(outcome.metadata()["sent"], true);
    assert!(outcome.downloaded_bytes().is_none());
    assert_eq!(executor.json_requests.borrow().len(), 1);
    assert_eq!(
        executor.json_requests.borrow()[0]["payload"]["document"],
        "private-file-id"
    );
    assert!(executor.multipart_requests.borrow().is_empty());
    assert!(executor.file_paths.borrow().is_empty());
    assert_secret_safe(outcome.metadata());
}

#[test]
fn multipart_transport_retries_then_succeeds_without_rereading_the_file() {
    let executor = StubExecutor::default();
    executor
        .file_results
        .borrow_mut()
        .push_back(Ok(vec![1, 2, 3]));
    executor.multipart_results.borrow_mut().extend([
        Ok(http_failure("timeout", "executor-secret")),
        Ok(http_failure("transport", "executor-secret")),
        Ok(telegram_success()),
    ]);
    let sleeper = StubSleeper::default();

    let outcome = execute(
        &executor,
        &sleeper,
        local_attachment_request("sendDocument", "/private/local.bin"),
    );

    assert_eq!(outcome.metadata()["ok"], true);
    assert_eq!(outcome.metadata()["attempts"], 3);
    assert_eq!(
        outcome.metadata()["retry_delays_seconds"],
        json!([1.0, 2.0])
    );
    assert_eq!(executor.file_paths.borrow().len(), 1);
    assert_eq!(executor.multipart_requests.borrow().len(), 3);
    assert_eq!(&*sleeper.sleeps.borrow(), &[1.0, 2.0]);
    assert_secret_safe(outcome.metadata());
}

#[test]
fn file_read_capacity_and_planner_contract_failures_stop_before_http() {
    let read_executor = StubExecutor::default();
    read_executor
        .file_results
        .borrow_mut()
        .push_back(Err("reader-secret /private/local.bin".to_string()));
    let read_failed = execute(
        &read_executor,
        &StubSleeper::default(),
        local_attachment_request("sendDocument", "/private/local.bin"),
    );
    assert_eq!(
        read_failed.metadata()["telegram_api_state"],
        "file_read_failed"
    );
    assert!(read_executor.multipart_requests.borrow().is_empty());
    assert_secret_safe(read_failed.metadata());

    let capacity_executor = StubExecutor::default();
    capacity_executor
        .file_results
        .borrow_mut()
        .push_back(Ok(vec![0; MAX_ATTACHMENT_BYTES + 1]));
    let too_large = execute(
        &capacity_executor,
        &StubSleeper::default(),
        local_attachment_request("sendDocument", "/private/local.bin"),
    );
    assert_eq!(
        too_large.metadata()["telegram_api_state"],
        "attachment_too_large"
    );
    assert!(capacity_executor.multipart_requests.borrow().is_empty());

    let unsupported_executor = StubExecutor::default();
    let unsupported = execute(
        &unsupported_executor,
        &StubSleeper::default(),
        local_attachment_request("sendVideo", "/private/local.bin"),
    );
    assert_eq!(
        unsupported.metadata()["telegram_api_state"],
        "planning_contract_failed"
    );
    assert!(unsupported_executor.file_paths.borrow().is_empty());
    assert!(unsupported_executor.multipart_requests.borrow().is_empty());

    let invalid_plan = json!({
        "execution_kind": API_METHOD_EXECUTION_KIND,
        "ok": true,
        "operation": "download_file",
        "transport": "bytes",
        "method": "POST",
        "telegram_method": "downloadFile",
        "result_kind": "bytes",
        "retry_family": "delivery",
        "url": "https://example.invalid/private",
    });
    assert!(BytesPlan::parse(invalid_plan.as_object().unwrap_or(&Map::new())).is_err());
}

#[test]
fn multipart_http_contract_telegram_and_executor_failures_are_secret_safe() {
    let telegram_executor = StubExecutor::default();
    telegram_executor
        .file_results
        .borrow_mut()
        .push_back(Ok(vec![1]));
    telegram_executor
        .multipart_results
        .borrow_mut()
        .push_back(Ok(json!({
            "ok": true,
            "status_code": 200,
            "response_kind": "json",
            "payload": {
                "ok": false,
                "description": "telegram-secret 123:bot-secret",
            },
        })));
    let telegram_failed = execute(
        &telegram_executor,
        &StubSleeper::default(),
        local_attachment_request("sendDocument", "/private/local.bin"),
    );
    assert_eq!(
        telegram_failed.metadata()["telegram_api_state"],
        "telegram_api_failed"
    );
    assert_secret_safe(telegram_failed.metadata());

    let contract_executor = StubExecutor::default();
    contract_executor
        .file_results
        .borrow_mut()
        .push_back(Ok(vec![1]));
    contract_executor
        .multipart_results
        .borrow_mut()
        .push_back(Ok(json!({"secret": "executor-secret"})));
    let contract_failed = execute(
        &contract_executor,
        &StubSleeper::default(),
        local_attachment_request("sendDocument", "/private/local.bin"),
    );
    assert_eq!(
        contract_failed.metadata()["telegram_api_state"],
        "http_contract_failed"
    );
    assert_secret_safe(contract_failed.metadata());

    let failed_executor = StubExecutor::default();
    failed_executor
        .file_results
        .borrow_mut()
        .push_back(Ok(vec![1]));
    failed_executor
        .multipart_results
        .borrow_mut()
        .push_back(Err("multipart executor-secret".to_string()));
    let executor_failed = execute(
        &failed_executor,
        &StubSleeper::default(),
        local_attachment_request("sendDocument", "/private/local.bin"),
    );
    assert_eq!(
        executor_failed.metadata()["telegram_api_state"],
        "executor_failed"
    );
    assert_secret_safe(executor_failed.metadata());
}

#[test]
fn multipart_retry_exhaustion_and_sleep_failure_are_bounded() {
    let exhausted_executor = StubExecutor::default();
    exhausted_executor
        .file_results
        .borrow_mut()
        .push_back(Ok(vec![1]));
    exhausted_executor
        .multipart_results
        .borrow_mut()
        .extend((0..3).map(|_| Ok(http_failure("transport", "executor-secret"))));
    let exhausted = execute(
        &exhausted_executor,
        &StubSleeper::default(),
        local_attachment_request("sendDocument", "/private/local.bin"),
    );
    assert_eq!(
        exhausted.metadata()["telegram_api_state"],
        "retry_exhausted"
    );
    assert_eq!(exhausted.metadata()["attempts"], 3);
    assert_eq!(exhausted_executor.file_paths.borrow().len(), 1);
    assert_eq!(exhausted_executor.multipart_requests.borrow().len(), 3);
    assert_secret_safe(exhausted.metadata());

    let sleep_executor = StubExecutor::default();
    sleep_executor
        .file_results
        .borrow_mut()
        .push_back(Ok(vec![1]));
    sleep_executor
        .multipart_results
        .borrow_mut()
        .push_back(Ok(http_failure("timeout", "executor-secret")));
    let sleeper = StubSleeper::default();
    sleeper
        .results
        .borrow_mut()
        .push_back(Err("sleep executor-secret".to_string()));
    let sleep_failed = execute(
        &sleep_executor,
        &sleeper,
        local_attachment_request("sendDocument", "/private/local.bin"),
    );
    assert_eq!(
        sleep_failed.metadata()["telegram_api_state"],
        "retry_sleep_failed"
    );
    assert_eq!(sleep_executor.multipart_requests.borrow().len(), 1);
    assert_secret_safe(sleep_failed.metadata());
}

#[test]
fn typed_surface_preserves_existing_json_get_file_behavior() {
    let executor = StubExecutor::default();
    executor.json_results.borrow_mut().push_back(Ok(json!({
        "ok": true,
        "status_code": 200,
        "response_kind": "json",
        "payload": {
            "ok": true,
            "result": {
                "file_id": "f-1",
                "file_path": "remote/path.ogg",
                "unexpected": "private-file-id",
            },
        },
    })));
    let outcome = execute(
        &executor,
        &StubSleeper::default(),
        json!({
            "operation": "get_file",
            "bot_token": "123:bot-secret",
            "file_id": "f-1",
        }),
    );

    assert_eq!(outcome.metadata()["ok"], true);
    assert_eq!(outcome.metadata()["transport"], "json");
    assert_eq!(outcome.metadata()["file_info"]["file_id"], "f-1");
    assert!(outcome.metadata()["file_info"].get("unexpected").is_none());
    assert!(outcome.downloaded_bytes().is_none());
    assert_secret_safe(outcome.metadata());
}

#[test]
fn native_executor_downloads_exact_loopback_bytes() {
    let (base_url, request_rx, handle) = serve_once("application/octet-stream", vec![0, 255, 1, 2]);

    let outcome = agent_telegram_api_execute(&json!({
        "operation": "download_file",
        "file_base_url": base_url,
        "file_path": "voice/native.ogg",
        "request_timeout_seconds": 3.0,
    }))
    .expect("native Telegram bytes execution");

    assert_eq!(outcome.downloaded_bytes(), Some(&[0, 255, 1, 2][..]));
    assert_eq!(outcome.metadata()["byte_count"], 4);
    let raw = request_rx.recv().expect("captured bytes request");
    assert!(String::from_utf8_lossy(&raw).starts_with("GET /voice/native.ogg HTTP/1.1"));
    handle.join().expect("bytes server thread");
}

#[test]
fn native_executor_reads_and_delivers_loopback_multipart() {
    let response = br#"{"ok":true,"result":{"message_id":17}}"#.to_vec();
    let (base_url, request_rx, handle) = serve_once("application/json", response);
    let temp = tempfile::tempdir().expect("temporary attachment directory");
    let path = temp.path().join("native.bin");
    std::fs::write(&path, [0, 1, 2, 255]).expect("write temporary attachment");

    let outcome = agent_telegram_api_execute(&json!({
        "operation": "send_attachment",
        "base_url": base_url,
        "chat_id": 42,
        "method_name": "sendDocument",
        "attachment": {
            "local_path": path.to_string_lossy(),
            "file_name": "native.bin",
            "mime_type": "application/octet-stream",
        },
        "request_timeout_seconds": 3.0,
    }))
    .expect("native Telegram multipart execution");

    assert_eq!(outcome.metadata()["ok"], true);
    assert_eq!(outcome.metadata()["sent"], true);
    assert_eq!(outcome.metadata()["byte_count"], 4);
    let raw = request_rx.recv().expect("captured multipart request");
    let rendered = String::from_utf8_lossy(&raw);
    assert!(rendered.starts_with("POST /sendDocument HTTP/1.1"));
    assert!(rendered.contains("multipart/form-data; boundary=aittelegram-"));
    assert!(rendered.contains("name=\"document\"; filename=\"native.bin\""));
    assert!(rendered.contains("name=\"chat_id\"\r\n\r\n42"));
    handle.join().expect("multipart server thread");
}

fn serve_once(
    content_type: &'static str,
    response_body: Vec<u8>,
) -> (String, mpsc::Receiver<Vec<u8>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind Telegram loopback");
    let address = listener.local_addr().expect("Telegram loopback address");
    let (request_tx, request_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept Telegram request");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("set Telegram request timeout");
        let raw = read_http_request(&mut stream);
        request_tx.send(raw).expect("capture Telegram request");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response_body.len()
        )
        .expect("write Telegram response headers");
        stream
            .write_all(&response_body)
            .expect("write Telegram response body");
    });
    (format!("http://{address}"), request_rx, handle)
}

fn read_http_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
    let mut raw = Vec::new();
    let mut chunk = [0u8; 4_096];
    loop {
        let read = stream.read(&mut chunk).expect("read Telegram request");
        assert!(read > 0, "Telegram request ended before completion");
        raw.extend_from_slice(&chunk[..read]);
        let Some(header_index) = raw.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let header_end = header_index + 4;
        let headers = String::from_utf8_lossy(&raw[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        if raw.len() >= header_end + content_length {
            return raw;
        }
    }
}
