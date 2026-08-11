use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use super::*;

struct StubPort {
    message_results: RefCell<VecDeque<Result<JsonValue, String>>>,
    attachment_results: RefCell<VecDeque<Result<JsonValue, String>>>,
    calls: RefCell<Vec<String>>,
    message_requests: RefCell<Vec<JsonValue>>,
    attachment_requests: RefCell<Vec<JsonValue>>,
}

impl StubPort {
    fn new(
        message_results: Vec<Result<JsonValue, String>>,
        attachment_results: Vec<Result<JsonValue, String>>,
    ) -> Self {
        Self {
            message_results: RefCell::new(message_results.into()),
            attachment_results: RefCell::new(attachment_results.into()),
            calls: RefCell::new(Vec::new()),
            message_requests: RefCell::new(Vec::new()),
            attachment_requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramReplyDeliveryPort for StubPort {
    fn execute_message(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.borrow_mut().push("send_message".to_string());
        self.message_requests.borrow_mut().push(request.clone());
        self.message_results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected message executor-secret".to_string()))
    }

    fn execute_attachment(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.borrow_mut().push(
            request
                .get("method_name")
                .and_then(JsonValue::as_str)
                .unwrap_or("unknown")
                .to_string(),
        );
        self.attachment_requests.borrow_mut().push(request.clone());
        self.attachment_results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Err("unexpected attachment executor-secret".to_string()))
    }
}

struct TestPlanner {
    fail_request: bool,
    fail_result: bool,
    request_mutator: Option<fn(&mut JsonValue)>,
    result_mutator: Option<fn(&mut JsonValue)>,
    calls: RefCell<Vec<String>>,
}

impl Default for TestPlanner {
    fn default() -> Self {
        Self {
            fail_request: false,
            fail_result: false,
            request_mutator: None,
            result_mutator: None,
            calls: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramReplyDeliveryPlanner for TestPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let stage = request
            .get("stage")
            .and_then(JsonValue::as_str)
            .unwrap_or("request");
        self.calls.borrow_mut().push(stage.to_string());
        if (stage == "request" && self.fail_request) || (stage == "result" && self.fail_result) {
            return Err("planner-secret 123:bot-secret".to_string());
        }
        let mut planned = agent_telegram_reply_delivery_execution_plan_json(request)?;
        match stage {
            "request" => {
                if let Some(mutator) = self.request_mutator {
                    mutator(&mut planned);
                }
            }
            "result" => {
                if let Some(mutator) = self.result_mutator {
                    mutator(&mut planned);
                }
            }
            _ => {}
        }
        Ok(planned)
    }
}

fn message_success() -> Result<JsonValue, String> {
    Ok(json!({
        "contract": MESSAGE_CONTRACT,
        "migration_stage": MESSAGE_MIGRATION_STAGE,
        "stage": "execute",
        "message_delivery_state": "completed",
        "ok": true,
        "completed": true,
        "chunk_count": 1,
        "completed_chunk_count": 1,
        "failed_chunk_index": JsonValue::Null,
        "fallback_count": 0,
        "api_call_count": 1,
        "chunk_results": [],
        "error_kind": JsonValue::Null,
        "error": JsonValue::Null,
        "python_message_delivery_allowed": false,
        "python_message_formatting_allowed": false,
        "raw_api_result_exposed": false,
        "telegram_description_exposed": false,
        "token_bearing_url_exposed": false,
        "chat_id_exposed": false,
        "formatted_text_exposed": false,
        "plain_text_exposed": false,
    }))
}

fn message_failure() -> Result<JsonValue, String> {
    Ok(json!({
        "contract": MESSAGE_CONTRACT,
        "migration_stage": MESSAGE_MIGRATION_STAGE,
        "stage": "execute",
        "message_delivery_state": "delivery_failed",
        "ok": false,
        "completed": false,
        "chunk_count": 1,
        "completed_chunk_count": 0,
        "failed_chunk_index": 0,
        "fallback_count": 0,
        "api_call_count": 1,
        "chunk_results": [],
        "error_kind": "telegram_api",
        "error": "downstream-secret",
        "python_message_delivery_allowed": false,
        "python_message_formatting_allowed": false,
        "raw_api_result_exposed": false,
        "telegram_description_exposed": false,
        "token_bearing_url_exposed": false,
        "chat_id_exposed": false,
        "formatted_text_exposed": false,
        "plain_text_exposed": false,
    }))
}

fn attachment_success(method: &str, transport: &str) -> Result<JsonValue, String> {
    Ok(attachment_outcome(
        method,
        transport,
        true,
        "completed",
        None,
    ))
}

fn attachment_failure(method: &str) -> Result<JsonValue, String> {
    Ok(attachment_outcome(
        method,
        "json",
        false,
        "telegram_api_failed",
        Some("telegram_api"),
    ))
}

fn attachment_outcome(
    method: &str,
    transport: &str,
    ok: bool,
    state: &str,
    error_kind: Option<&str>,
) -> JsonValue {
    json!({
        "contract": API_CONTRACT,
        "migration_stage": API_MIGRATION_STAGE,
        "stage": "execute",
        "telegram_api_state": state,
        "operation": "send_attachment",
        "telegram_method": method,
        "transport": transport,
        "attempts": 1,
        "ok": ok,
        "completed": ok,
        "downloaded": false,
        "sent": ok,
        "error_kind": error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "error": if ok { JsonValue::Null } else { json!("downstream-secret") },
        "python_telegram_api_allowed": false,
        "python_http_execution_allowed": false,
        "python_retry_allowed": false,
        "raw_telegram_payload_exposed": false,
        "token_bearing_url_exposed": false,
        "downloaded_bytes_exposed": false,
        "local_path_exposed": false,
        "multipart_fields_exposed": false,
        "file_name_exposed": false,
    })
}

fn request_with(text: Option<&str>, attachments: Vec<JsonValue>) -> JsonValue {
    let mut message = json!({
        "attachments": attachments,
    });
    if let Some(text) = text {
        message["text"] = json!(text);
    }
    json!({
        "chat_id": "private-chat-998877",
        "assistant_event": {
            "sequence": 8,
            "event_type": "assistant.reply",
            "payload": {
                "transport_reply_envelope": {
                    "message": message,
                },
            },
        },
        "through_sequence": 9,
        "bot_token": "123:bot-secret",
        "base_url": "https://telegram-secret.example/bot123:bot-secret",
        "request_timeout_seconds": 7.0,
        "reply_markdown_enabled": true,
    })
}

fn execute(
    planner: &impl TelegramReplyDeliveryPlanner,
    port: &StubPort,
    request: &JsonValue,
) -> Result<JsonValue, TelegramReplyDeliveryExecutionError> {
    execute_with_telegram_reply_delivery_ports(planner, port, request)
}

fn assert_success(outcome: &JsonValue, operation_count: usize) {
    assert_eq!(outcome["contract"], CONTRACT);
    assert_eq!(outcome["migration_stage"], MIGRATION_STAGE);
    assert_eq!(outcome["reply_delivery_state"], "completed");
    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["operation_count"], operation_count);
    assert_eq!(outcome["attempted_operation_count"], operation_count);
    assert_eq!(outcome["delivered_operation_count"], operation_count);
    for flag in [
        "python_reply_delivery_allowed",
        "python_message_delivery_allowed",
        "python_attachment_delivery_allowed",
        "raw_planner_result_exposed",
        "raw_executor_result_exposed",
        "bot_token_exposed",
        "chat_id_exposed",
        "reply_text_exposed",
        "attachment_exposed",
        "telegram_description_exposed",
        "local_path_exposed",
    ] {
        assert_eq!(outcome[flag], false, "unsafe outcome flag {flag}");
    }
}

#[test]
fn text_only_delivery_uses_the_rust_message_executor_contract() {
    let port = StubPort::new(vec![message_success()], vec![]);
    let outcome = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &port,
        &request_with(Some("private reply"), vec![]),
    )
    .expect("text reply delivery");

    assert_success(&outcome, 1);
    assert_eq!(port.calls.borrow().as_slice(), ["send_message"]);
    let requests = port.message_requests.borrow();
    assert_eq!(requests[0]["chat_id"], "private-chat-998877");
    assert_eq!(requests[0]["text"], "private reply");
    assert_eq!(requests[0]["reply_markdown_enabled"], true);
    assert_eq!(requests[0]["bot_token"], "123:bot-secret");
    assert!(requests[0].get("assistant_event").is_none());
}

#[test]
fn attachment_only_delivery_preserves_local_multipart_routing() {
    let port = StubPort::new(
        vec![],
        vec![attachment_success("sendDocument", "multipart")],
    );
    let outcome = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &port,
        &request_with(
            None,
            vec![json!({
                "kind": "document",
                "local_path": "/private/report.pdf",
                "file_name": "report.pdf",
                "mime_type": "application/pdf",
            })],
        ),
    )
    .expect("attachment-only delivery");

    assert_success(&outcome, 1);
    assert_eq!(port.calls.borrow().as_slice(), ["sendDocument"]);
    let requests = port.attachment_requests.borrow();
    assert_eq!(requests[0]["operation"], "send_attachment");
    assert_eq!(requests[0]["method_name"], "sendDocument");
    assert_eq!(requests[0]["file_field"], "document");
    assert_eq!(
        requests[0]["attachment"]["local_path"],
        "/private/report.pdf"
    );
    assert_eq!(requests[0]["bot_token"], "123:bot-secret");
}

#[test]
fn mixed_delivery_selects_audio_photo_and_document_in_order() {
    let port = StubPort::new(
        vec![message_success()],
        vec![
            attachment_success("sendAudio", "json"),
            attachment_success("sendPhoto", "json"),
            attachment_success("sendDocument", "json"),
        ],
    );
    let request = request_with(
        Some("ordered reply"),
        vec![
            json!({"file_name": "voice.mp3", "telegram_file_id": "audio-file-id"}),
            json!({"mime_type": "image/png", "telegram_file_id": "photo-file-id"}),
            json!({"kind": "document", "file_name": "image.png", "url": "https://private.example/report"}),
        ],
    );
    let outcome = execute(&DefaultTelegramReplyDeliveryPlanner, &port, &request)
        .expect("mixed reply delivery");

    assert_success(&outcome, 4);
    assert_eq!(
        port.calls.borrow().as_slice(),
        ["send_message", "sendAudio", "sendPhoto", "sendDocument"]
    );
    let requests = port.attachment_requests.borrow();
    assert_eq!(requests[0]["file_field"], "audio");
    assert_eq!(requests[1]["file_field"], "photo");
    assert_eq!(requests[2]["file_field"], "document");
}

#[test]
fn explicit_reply_text_override_is_trimmed_and_delivered() {
    let port = StubPort::new(vec![message_success()], vec![]);
    let mut request = request_with(Some("event reply"), vec![]);
    request["reply_text"] = json!("  override reply  ");
    let outcome = execute(&DefaultTelegramReplyDeliveryPlanner, &port, &request)
        .expect("reply text override");

    assert_success(&outcome, 1);
    assert_eq!(port.message_requests.borrow()[0]["text"], "override reply");
}

#[test]
fn empty_reply_fails_closed_and_explicit_skip_has_no_side_effects() {
    let empty_port = StubPort::new(vec![], vec![]);
    let empty = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &empty_port,
        &request_with(None, vec![]),
    )
    .unwrap_err();
    assert_eq!(empty.kind(), EmptyReply);
    assert!(empty_port.calls.borrow().is_empty());

    let skipped_port = StubPort::new(vec![], vec![]);
    let mut skipped_request = request_with(Some("must not send"), vec![]);
    skipped_request["should_execute"] = json!(false);
    let skipped = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &skipped_port,
        &skipped_request,
    )
    .expect("explicitly skipped reply");
    assert_eq!(skipped["reply_delivery_state"], "skipped");
    assert_eq!(skipped["attempted_operation_count"], 0);
    assert!(skipped_port.calls.borrow().is_empty());
}

#[test]
fn message_failures_are_typed_and_stop_before_attachments() {
    let cases = [
        (message_failure(), Message),
        (
            Err("message executor-secret 123:bot-secret".to_string()),
            Message,
        ),
        (
            Ok(json!({"ok": true, "secret": "downstream-secret"})),
            MessageContract,
        ),
    ];
    for (message_result, expected_kind) in cases {
        let port = StubPort::new(
            vec![message_result],
            vec![attachment_success("sendDocument", "json")],
        );
        let failure = execute(
            &DefaultTelegramReplyDeliveryPlanner,
            &port,
            &request_with(
                Some("private reply"),
                vec![json!({"kind": "document", "telegram_file_id": "private-file-id"})],
            ),
        )
        .unwrap_err();
        assert_eq!(failure.kind(), expected_kind);
        assert_eq!(failure.operation_index(), Some(0));
        assert_eq!(
            failure.operation_kind(),
            Some(TelegramReplyDeliveryOperationKind::Message)
        );
        assert_eq!(port.calls.borrow().as_slice(), ["send_message"]);
    }
}

#[test]
fn attachment_failures_are_typed_and_stop_later_operations() {
    let cases = [
        (attachment_failure("sendAudio"), Attachment),
        (
            Err("attachment executor-secret 123:bot-secret".to_string()),
            Attachment,
        ),
        (
            Ok(json!({"ok": true, "secret": "downstream-secret"})),
            AttachmentContract,
        ),
    ];
    for (attachment_result, expected_kind) in cases {
        let port = StubPort::new(
            vec![],
            vec![
                attachment_result,
                attachment_success("sendDocument", "json"),
            ],
        );
        let failure = execute(
            &DefaultTelegramReplyDeliveryPlanner,
            &port,
            &request_with(
                None,
                vec![
                    json!({"kind": "audio", "telegram_file_id": "private-audio-id"}),
                    json!({"kind": "document", "telegram_file_id": "private-document-id"}),
                ],
            ),
        )
        .unwrap_err();
        assert_eq!(failure.kind(), expected_kind);
        assert_eq!(failure.operation_index(), Some(0));
        assert_eq!(
            failure.operation_kind(),
            Some(TelegramReplyDeliveryOperationKind::Audio)
        );
        assert_eq!(port.calls.borrow().as_slice(), ["sendAudio"]);
    }
}

#[test]
fn transport_failures_are_explicitly_retryable_but_terminal_failures_are_not() {
    let mut retryable_message = message_failure().unwrap();
    retryable_message["error_kind"] = json!("transport");
    let retryable = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &StubPort::new(vec![Ok(retryable_message)], vec![]),
        &request_with(Some("private reply"), vec![]),
    )
    .unwrap_err();
    assert_eq!(retryable.kind(), Message);
    assert!(retryable.is_retryable());

    let terminal = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &StubPort::new(vec![message_failure()], vec![]),
        &request_with(Some("private reply"), vec![]),
    )
    .unwrap_err();
    assert_eq!(terminal.kind(), Message);
    assert!(!terminal.is_retryable());

    let mut retryable_attachment = attachment_failure("sendDocument").unwrap();
    retryable_attachment["error_kind"] = json!("timeout");
    let retryable = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &StubPort::new(vec![], vec![Ok(retryable_attachment)]),
        &request_with(
            None,
            vec![json!({"kind": "document", "telegram_file_id": "private-file-id"})],
        ),
    )
    .unwrap_err();
    assert_eq!(retryable.kind(), Attachment);
    assert!(retryable.is_retryable());
}

fn corrupt_request_identity(plan: &mut JsonValue) {
    plan["delivery_kind"] = json!("private-corrupt-kind");
}

fn corrupt_request_count(plan: &mut JsonValue) {
    plan["request"]["operation_count"] = json!(999);
}

fn corrupt_request_chat(plan: &mut JsonValue) {
    plan["request"]["operations"][0]["chat_id"] = json!("private-other-chat");
}

fn corrupt_request_order(plan: &mut JsonValue) {
    plan["request"]["operations"]
        .as_array_mut()
        .expect("operations")
        .swap(0, 1);
}

fn corrupt_attachment_method(plan: &mut JsonValue) {
    plan["request"]["operations"][1]["method"] = json!("sendPhoto");
    plan["request"]["operations"][1]["file_field"] = json!("photo");
}

#[test]
fn request_planner_errors_and_corrupt_contracts_fail_before_side_effects() {
    let request = request_with(
        Some("private reply"),
        vec![json!({"kind": "document", "telegram_file_id": "private-file-id"})],
    );
    let failing = TestPlanner {
        fail_request: true,
        ..TestPlanner::default()
    };
    let port = StubPort::new(vec![message_success()], vec![]);
    assert_eq!(
        execute(&failing, &port, &request).unwrap_err().kind(),
        Planner
    );
    assert!(port.calls.borrow().is_empty());

    for mutator in [
        corrupt_request_identity as fn(&mut JsonValue),
        corrupt_request_count,
        corrupt_request_chat,
        corrupt_request_order,
        corrupt_attachment_method,
    ] {
        let planner = TestPlanner {
            request_mutator: Some(mutator),
            ..TestPlanner::default()
        };
        let port = StubPort::new(vec![message_success()], vec![]);
        assert_eq!(
            execute(&planner, &port, &request).unwrap_err().kind(),
            PlannerContract
        );
        assert!(port.calls.borrow().is_empty());
    }
}

fn corrupt_result_identity(plan: &mut JsonValue) {
    plan["result"]["delivery_kind"] = json!("private-corrupt-kind");
}

fn corrupt_result_count(plan: &mut JsonValue) {
    plan["result"]["delivered_operation_count"] = json!(999);
}

fn corrupt_result_operations(plan: &mut JsonValue) {
    plan["result"]["operation_results"][0]["kind"] = json!("send_document");
}

#[test]
fn result_planner_errors_and_corruption_fail_after_delivery() {
    let request = request_with(Some("private reply"), vec![]);
    let failing = TestPlanner {
        fail_result: true,
        ..TestPlanner::default()
    };
    let port = StubPort::new(vec![message_success()], vec![]);
    assert_eq!(
        execute(&failing, &port, &request).unwrap_err().kind(),
        ResultPlanner
    );
    assert_eq!(port.calls.borrow().as_slice(), ["send_message"]);

    for mutator in [
        corrupt_result_identity as fn(&mut JsonValue),
        corrupt_result_count,
        corrupt_result_operations,
    ] {
        let planner = TestPlanner {
            result_mutator: Some(mutator),
            ..TestPlanner::default()
        };
        let port = StubPort::new(vec![message_success()], vec![]);
        assert_eq!(
            execute(&planner, &port, &request).unwrap_err().kind(),
            ResultContract
        );
        assert_eq!(port.calls.borrow().as_slice(), ["send_message"]);
    }
}

#[test]
fn invalid_requests_are_rejected_before_planning_or_delivery() {
    for request in [
        JsonValue::Null,
        json!({"chat_id": "", "reply_text": "private reply"}),
        json!({"chat_id": {"private": true}, "reply_text": "private reply"}),
        json!({"chat_id": "private-chat", "reply_text": "bad\u{0}reply"}),
    ] {
        let planner = TestPlanner::default();
        let port = StubPort::new(vec![message_success()], vec![]);
        let failure = execute(&planner, &port, &request).unwrap_err();
        assert_eq!(failure.kind(), InvalidRequest);
        assert!(planner.calls.borrow().is_empty());
        assert!(port.calls.borrow().is_empty());
    }
}

#[test]
fn public_outcomes_and_errors_never_expose_request_or_downstream_secrets() {
    let request = request_with(
        Some("private-original-input"),
        vec![json!({
            "kind": "document",
            "local_path": "/private/local-secret.pdf",
            "file_name": "private-name.pdf",
        })],
    );
    let failure_port = StubPort::new(
        vec![Err(
            "downstream-secret private-chat-998877 123:bot-secret".to_string()
        )],
        vec![],
    );
    let failure = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &failure_port,
        &request,
    )
    .unwrap_err();
    assert_secret_safe(&format!("{failure:?} {failure}"));

    let planner = TestPlanner {
        fail_request: true,
        ..TestPlanner::default()
    };
    let planner_failure = execute(&planner, &StubPort::new(vec![], vec![]), &request).unwrap_err();
    assert_secret_safe(&format!("{planner_failure:?} {planner_failure}"));

    let success_port = StubPort::new(
        vec![message_success()],
        vec![attachment_success("sendDocument", "multipart")],
    );
    let outcome = execute(
        &DefaultTelegramReplyDeliveryPlanner,
        &success_port,
        &request,
    )
    .expect("secret-safe success");
    assert_secret_safe(&format!("{outcome:?} {outcome}"));
}

fn assert_secret_safe(rendered: &str) {
    for secret in [
        "123:bot-secret",
        "private-chat-998877",
        "private-original-input",
        "private/local-secret.pdf",
        "private-name.pdf",
        "telegram-secret.example",
        "downstream-secret",
        "planner-secret",
        "private-file-id",
    ] {
        assert!(!rendered.contains(secret), "public value leaked {secret}");
    }
}

#[test]
fn native_port_delivers_text_then_telegram_hosted_attachment_over_loopback() {
    let (base_url, requests, server) = serve_http_successes(2);
    let mut request = request_with(
        Some("native private reply"),
        vec![json!({
            "kind": "document",
            "telegram_file_id": "hosted-private-file-id",
        })],
    );
    request["base_url"] = json!(base_url);
    request["reply_markdown_enabled"] = json!(false);

    let outcome = agent_telegram_reply_delivery_execute_json(&request)
        .expect("native reply delivery execution");
    assert_success(&outcome, 2);
    let first = requests.recv().expect("message request");
    let second = requests.recv().expect("attachment request");
    assert!(first.starts_with("POST /sendMessage HTTP/1.1"));
    assert!(first.contains("native private reply"));
    assert!(second.starts_with("POST /sendDocument HTTP/1.1"));
    assert!(second.contains("hosted-private-file-id"));
    server.join().expect("loopback Telegram server");
}

fn serve_http_successes(count: usize) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback server should bind");
    let base_url = format!(
        "http://{}",
        listener.local_addr().expect("loopback address")
    );
    let (request_tx, request_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        for index in 0..count {
            let (mut stream, _) = listener.accept().expect("loopback request");
            let request = read_http_request(&mut stream);
            request_tx.send(request).expect("capture loopback request");
            let body = format!(r#"{{"ok":true,"result":{{"message_id":{}}}}}"#, index + 1);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write loopback response");
        }
    });
    (base_url, request_rx, handle)
}

fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 4_096];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&bytes);
        let Some(headers_end) = text.find("\r\n\r\n") else {
            continue;
        };
        let content_length = text[..headers_end]
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
            })
            .unwrap_or(0);
        if bytes.len() >= headers_end + 4 + content_length {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}
