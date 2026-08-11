use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use ait_core::json_support::{json, JsonValue};

use super::*;

struct StubPlanner {
    result: Result<JsonValue, String>,
    requests: RefCell<Vec<JsonValue>>,
}

impl StubPlanner {
    fn returning(result: JsonValue) -> Self {
        Self {
            result: Ok(result),
            requests: RefCell::new(Vec::new()),
        }
    }

    fn failing(error: &str) -> Self {
        Self {
            result: Err(error.to_string()),
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramMessageFormattingPlanner for StubPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.borrow_mut().push(request.clone());
        self.result.clone()
    }
}

struct StubApiPort {
    results: RefCell<VecDeque<Result<JsonValue, String>>>,
    requests: RefCell<Vec<JsonValue>>,
}

impl StubApiPort {
    fn new(results: Vec<Result<JsonValue, String>>) -> Self {
        Self {
            results: RefCell::new(results.into()),
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramMessageDeliveryApiPort for StubApiPort {
    fn execute_send_message(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.borrow_mut().push(request.clone());
        self.results.borrow_mut().pop_front().unwrap_or_else(|| {
            Err("unexpected API call with queue-secret and 123:bot-secret".to_string())
        })
    }
}

fn planned_chunks(chunks: Vec<JsonValue>) -> JsonValue {
    json!({
        "migration_stage": "rust_agent_telegram_message_formatting",
        "message_format_contract": FORMAT_CONTRACT,
        "kind": "message_chunks",
        "transport": "telegram",
        "rust_event_loop_required": true,
        "python_message_formatting_allowed": false,
        "chunks": chunks,
    })
}

fn plain_chunk(text: &str) -> JsonValue {
    json!({
        "text": text,
        "plain_text": text,
        "parse_mode": JsonValue::Null,
    })
}

fn formatted_chunk(text: &str, plain_text: &str) -> JsonValue {
    json!({
        "text": text,
        "plain_text": plain_text,
        "parse_mode": "HTML",
    })
}

fn api_outcome(
    ok: bool,
    state: &str,
    error_kind: Option<&str>,
    telegram_parse_error: bool,
    attempts: u64,
    status_code: Option<i64>,
) -> Result<JsonValue, String> {
    Ok(json!({
        "contract": API_CONTRACT,
        "migration_stage": API_MIGRATION_STAGE,
        "stage": "execute",
        "telegram_api_state": state,
        "operation": "send_message",
        "telegram_method": "sendMessage",
        "transport": "json",
        "downloaded": false,
        "downloaded_bytes_exposed": false,
        "token_bearing_url_exposed": false,
        "ok": ok,
        "completed": ok,
        "sent": ok,
        "telegram_parse_error": telegram_parse_error,
        "attempts": attempts,
        "http_status_code": status_code.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "error_kind": error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
    }))
}

fn api_success(attempts: u64) -> Result<JsonValue, String> {
    api_outcome(true, "completed", None, false, attempts, Some(200))
}

fn api_failure(
    state: &str,
    error_kind: &str,
    telegram_parse_error: bool,
    attempts: u64,
    status_code: Option<i64>,
) -> Result<JsonValue, String> {
    api_outcome(
        false,
        state,
        Some(error_kind),
        telegram_parse_error,
        attempts,
        status_code,
    )
}

fn request(markdown: bool) -> JsonValue {
    json!({
        "chat_id": "private-chat-998877",
        "text": "private-original-input",
        "reply_markdown_enabled": markdown,
        "bot_token": "123:bot-secret",
        "base_url": "https://telegram-secret.example/bot123:bot-secret",
        "request_timeout_seconds": 7.0,
        "untrusted_secret": "must-not-reach-api-port",
    })
}

fn execute<P>(planner: &P, api: &StubApiPort, request: &JsonValue) -> JsonValue
where
    P: TelegramMessageFormattingPlanner + ?Sized,
{
    execute_with_telegram_message_delivery_ports(planner, api, request)
        .expect("Telegram message delivery execution")
}

fn assert_public_outcome_is_safe(outcome: &JsonValue) {
    for rendered in [outcome.to_string(), format!("{outcome:?}")] {
        for secret in [
            "123:bot-secret",
            "private-chat-998877",
            "private-original-input",
            "private-rendered",
            "private-plain",
            "telegram-secret.example",
            "must-not-reach-api-port",
            "downstream-secret",
            "queue-secret",
        ] {
            assert!(!rendered.contains(secret), "public outcome leaked {secret}");
        }
    }
    assert_eq!(outcome["raw_api_result_exposed"], false);
    assert_eq!(outcome["telegram_description_exposed"], false);
    assert_eq!(outcome["token_bearing_url_exposed"], false);
    assert_eq!(outcome["chat_id_exposed"], false);
    assert_eq!(outcome["formatted_text_exposed"], false);
    assert_eq!(outcome["plain_text_exposed"], false);
    assert_eq!(outcome["python_message_delivery_allowed"], false);
    assert_eq!(outcome["python_message_formatting_allowed"], false);
}

#[test]
fn sends_plain_chunks_in_order_and_preserves_per_chunk_attempt_counts() {
    let planner = StubPlanner::returning(planned_chunks(vec![
        plain_chunk("first-private"),
        plain_chunk("second-private"),
        plain_chunk("third-private"),
    ]));
    let api = StubApiPort::new(vec![api_success(2), api_success(1), api_success(3)]);

    let outcome = execute(&planner, &api, &request(false));

    assert_eq!(outcome["contract"], CONTRACT);
    assert_eq!(outcome["message_delivery_state"], "completed");
    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["chunk_count"], 3);
    assert_eq!(outcome["completed_chunk_count"], 3);
    assert_eq!(outcome["fallback_count"], 0);
    assert_eq!(outcome["api_call_count"], 3);
    assert!(outcome["failed_chunk_index"].is_null());
    assert_eq!(outcome["chunk_results"][0]["attempt_count"], 2);
    assert_eq!(outcome["chunk_results"][1]["attempt_count"], 1);
    assert_eq!(outcome["chunk_results"][2]["attempt_count"], 3);

    let requests = api.requests.borrow();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["text"], "first-private");
    assert_eq!(requests[1]["text"], "second-private");
    assert_eq!(requests[2]["text"], "third-private");
    assert!(requests
        .iter()
        .all(|request| request.get("parse_mode").is_none()));
    assert!(requests
        .iter()
        .all(|request| request["operation"] == "send_message"));
    assert!(requests
        .iter()
        .all(|request| request["chat_id"] == "private-chat-998877"));
    assert!(requests
        .iter()
        .all(|request| request["bot_token"] == "123:bot-secret"));
    assert!(requests
        .iter()
        .all(|request| request.get("untrusted_secret").is_none()));
    assert_eq!(planner.requests.borrow()[0]["limit"], 3_800);
    assert_public_outcome_is_safe(&outcome);
}

#[test]
fn retries_only_parse_rejected_formatted_chunk_as_plain_then_continues() {
    let planner = StubPlanner::returning(planned_chunks(vec![
        formatted_chunk("<b>private-rendered-one</b>", "private-plain-one"),
        formatted_chunk("<i>private-rendered-two</i>", "private-plain-two"),
    ]));
    let api = StubApiPort::new(vec![
        api_failure("telegram_api_failed", "telegram_api", true, 1, Some(400)),
        api_success(2),
        api_success(1),
    ]);

    let outcome = execute(&planner, &api, &request(true));

    assert_eq!(outcome["message_delivery_state"], "completed");
    assert_eq!(outcome["completed_chunk_count"], 2);
    assert_eq!(outcome["fallback_count"], 1);
    assert_eq!(outcome["api_call_count"], 3);
    assert_eq!(outcome["chunk_results"][0]["fallback_used"], true);
    assert_eq!(outcome["chunk_results"][0]["api_call_count"], 2);
    assert_eq!(outcome["chunk_results"][0]["attempt_count"], 3);
    assert_eq!(outcome["chunk_results"][1]["fallback_used"], false);

    let requests = api.requests.borrow();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["text"], "<b>private-rendered-one</b>");
    assert_eq!(requests[0]["parse_mode"], "HTML");
    assert_eq!(requests[1]["text"], "private-plain-one");
    assert!(requests[1].get("parse_mode").is_none());
    assert_eq!(requests[2]["text"], "<i>private-rendered-two</i>");
    assert_eq!(requests[2]["parse_mode"], "HTML");
    assert_public_outcome_is_safe(&outcome);
}

#[test]
fn non_parse_failure_never_falls_back_and_stops_later_chunks() {
    let planner = StubPlanner::returning(planned_chunks(vec![
        plain_chunk("first-private"),
        plain_chunk("second-private"),
        plain_chunk("third-private"),
    ]));
    let api = StubApiPort::new(vec![
        api_success(1),
        api_failure("http_failed", "http", false, 3, Some(503)),
        api_success(1),
    ]);

    let outcome = execute(&planner, &api, &request(false));

    assert_eq!(outcome["message_delivery_state"], "delivery_failed");
    assert_eq!(outcome["ok"], false);
    assert_eq!(outcome["completed_chunk_count"], 1);
    assert_eq!(outcome["failed_chunk_index"], 1);
    assert_eq!(outcome["fallback_count"], 0);
    assert_eq!(outcome["api_call_count"], 2);
    assert_eq!(outcome["chunk_results"].as_array().unwrap().len(), 2);
    assert_eq!(outcome["chunk_results"][1]["attempt_count"], 3);
    assert_eq!(outcome["chunk_results"][1]["http_status_code"], 503);
    assert_eq!(api.requests.borrow().len(), 2);
    assert_public_outcome_is_safe(&outcome);
}

#[test]
fn failed_plain_fallback_stops_before_the_next_formatted_chunk() {
    let planner = StubPlanner::returning(planned_chunks(vec![
        formatted_chunk("<b>private-rendered-one</b>", "private-plain-one"),
        formatted_chunk("<b>private-rendered-two</b>", "private-plain-two"),
    ]));
    let api = StubApiPort::new(vec![
        api_failure("telegram_api_failed", "telegram_api", true, 1, Some(400)),
        api_failure("http_failed", "transport", false, 2, None),
        api_success(1),
    ]);

    let outcome = execute(&planner, &api, &request(true));

    assert_eq!(outcome["message_delivery_state"], "delivery_failed");
    assert_eq!(outcome["completed_chunk_count"], 0);
    assert_eq!(outcome["failed_chunk_index"], 0);
    assert_eq!(outcome["fallback_count"], 1);
    assert_eq!(outcome["api_call_count"], 2);
    assert_eq!(outcome["chunk_results"][0]["fallback_used"], true);
    assert_eq!(outcome["chunk_results"][0]["attempt_count"], 3);
    assert_eq!(api.requests.borrow().len(), 2);
    assert!(api.requests.borrow()[1].get("parse_mode").is_none());
    assert_public_outcome_is_safe(&outcome);
}

#[test]
fn parse_signal_on_plain_chunk_does_not_enable_fallback() {
    let planner = StubPlanner::returning(planned_chunks(vec![
        plain_chunk("first-private"),
        plain_chunk("second-private"),
    ]));
    let api = StubApiPort::new(vec![
        api_failure("telegram_api_failed", "telegram_api", true, 1, Some(400)),
        api_success(1),
    ]);

    let outcome = execute(&planner, &api, &request(false));

    assert_eq!(outcome["message_delivery_state"], "delivery_failed");
    assert_eq!(outcome["fallback_count"], 0);
    assert_eq!(outcome["api_call_count"], 1);
    assert_eq!(api.requests.borrow().len(), 1);
    assert_public_outcome_is_safe(&outcome);
}

#[test]
fn planner_identity_and_planner_errors_fail_closed_before_api_execution() {
    let malformed = StubPlanner::returning(json!({
        "migration_stage": "python-secret-stage",
        "chunks": [{"text": "private-rendered", "plain_text": "private-plain"}],
        "raw": "123:bot-secret",
    }));
    let api = StubApiPort::new(Vec::new());
    let malformed_outcome = execute(&malformed, &api, &request(false));

    assert_eq!(
        malformed_outcome["message_delivery_state"],
        "planner_contract_failed"
    );
    assert_eq!(api.requests.borrow().len(), 0);
    assert_public_outcome_is_safe(&malformed_outcome);

    let failing = StubPlanner::failing("downstream-secret 123:bot-secret private-rendered");
    let failing_outcome = execute(&failing, &api, &request(false));
    assert_eq!(failing_outcome["message_delivery_state"], "planner_failed");
    assert_eq!(api.requests.borrow().len(), 0);
    assert_public_outcome_is_safe(&failing_outcome);
}

#[test]
fn api_executor_and_api_contract_errors_are_generic_and_secret_safe() {
    let planner = StubPlanner::returning(planned_chunks(vec![plain_chunk("private-plain")]));
    let executor_error = StubApiPort::new(vec![Err(
        "downstream-secret 123:bot-secret private-plain".to_string(),
    )]);

    let executor_outcome = execute(&planner, &executor_error, &request(false));

    assert_eq!(
        executor_outcome["message_delivery_state"],
        "api_executor_failed"
    );
    assert_eq!(executor_outcome["error_kind"], "executor");
    assert_public_outcome_is_safe(&executor_outcome);

    let malformed = StubApiPort::new(vec![Ok(json!({
        "contract": API_CONTRACT,
        "raw": "downstream-secret 123:bot-secret private-plain",
    }))]);
    let contract_outcome = execute(&planner, &malformed, &request(false));
    assert_eq!(
        contract_outcome["message_delivery_state"],
        "api_contract_failed"
    );
    assert_eq!(contract_outcome["error_kind"], "contract");
    assert_public_outcome_is_safe(&contract_outcome);
}

#[test]
fn inconsistent_api_parse_signal_and_status_type_fail_closed() {
    let planner = StubPlanner::returning(planned_chunks(vec![formatted_chunk(
        "<b>private-rendered</b>",
        "private-plain",
    )]));
    let inconsistent =
        StubApiPort::new(vec![api_failure("http_failed", "http", true, 1, Some(400))]);

    let inconsistent_outcome = execute(&planner, &inconsistent, &request(true));

    assert_eq!(
        inconsistent_outcome["message_delivery_state"],
        "api_contract_failed"
    );
    assert_eq!(inconsistent_outcome["fallback_count"], 0);
    assert_eq!(inconsistent.requests.borrow().len(), 1);
    assert_public_outcome_is_safe(&inconsistent_outcome);

    let mut invalid_status = api_success(1).expect("stub API success metadata");
    invalid_status["http_status_code"] = json!("200");
    let invalid_status_port = StubApiPort::new(vec![Ok(invalid_status)]);
    let invalid_status_outcome = execute(&planner, &invalid_status_port, &request(true));

    assert_eq!(
        invalid_status_outcome["message_delivery_state"],
        "api_contract_failed"
    );
    assert_eq!(invalid_status_port.requests.borrow().len(), 1);
    assert_public_outcome_is_safe(&invalid_status_outcome);
}

#[test]
fn invalid_chunk_contracts_fail_closed_before_api_execution() {
    let cases = [
        (
            planned_chunks(vec![formatted_chunk("private-rendered", "private-plain")]),
            false,
        ),
        (
            planned_chunks(vec![json!({
                "text": "private-rendered",
                "plain_text": "different-private-plain",
                "parse_mode": JsonValue::Null,
            })]),
            false,
        ),
        (
            planned_chunks(vec![json!({
                "text": "private-rendered",
                "plain_text": "private-plain",
                "parse_mode": "MarkdownV2",
            })]),
            true,
        ),
    ];

    for (planned, markdown_enabled) in cases {
        let planner = StubPlanner::returning(planned);
        let api = StubApiPort::new(Vec::new());
        let outcome = execute(&planner, &api, &request(markdown_enabled));
        assert_eq!(outcome["message_delivery_state"], "planner_contract_failed");
        assert_eq!(api.requests.borrow().len(), 0);
        assert_public_outcome_is_safe(&outcome);
    }
}

#[test]
fn invalid_request_is_rejected_without_invoking_planner_or_api() {
    let planner = StubPlanner::returning(planned_chunks(vec![plain_chunk("private-plain")]));
    let api = StubApiPort::new(Vec::new());

    let outcome = execute(
        &planner,
        &api,
        &json!({
            "chat_id": "\n",
            "text": "private-original-input",
            "bot_token": "123:bot-secret",
        }),
    );

    assert_eq!(outcome["message_delivery_state"], "invalid_request");
    assert_eq!(planner.requests.borrow().len(), 0);
    assert_eq!(api.requests.borrow().len(), 0);
    assert_public_outcome_is_safe(&outcome);
}

#[test]
fn default_markdown_planner_integrates_with_ordered_delivery() {
    let api = StubApiPort::new(vec![api_success(1)]);

    let outcome = execute(
        &DefaultTelegramMessageFormattingPlanner,
        &api,
        &json!({
            "chat_id": "private-chat-998877",
            "text": "# private-original-input",
            "reply_markdown_enabled": true,
            "bot_token": "123:bot-secret",
        }),
    );

    assert_eq!(outcome["message_delivery_state"], "completed");
    assert_eq!(outcome["chunk_count"], 1);
    assert_eq!(api.requests.borrow()[0]["parse_mode"], "HTML");
    assert!(api.requests.borrow()[0]["text"]
        .as_str()
        .unwrap_or_default()
        .contains("<b>private-original-input</b>"));
    assert_public_outcome_is_safe(&outcome);
}

#[test]
fn native_delivery_falls_back_after_real_loopback_http_400_parse_error() {
    let (base_url, requests, server) = serve_http_sequence(vec![
        (
            "400 Bad Request",
            "{\"ok\":false,\"error_code\":400,\"description\":\"Bad Request: can't parse entities near private-original-input 123:bot-secret\"}",
        ),
        (
            "200 OK",
            "{\"ok\":true,\"result\":{\"message_id\":17,\"text\":\"private-original-input\"}}",
        ),
    ]);

    let outcome = agent_telegram_message_delivery_execute_json(&json!({
        "chat_id": "private-chat-998877",
        "text": "# private-original-input",
        "reply_markdown_enabled": true,
        "bot_token": "123:bot-secret",
        "base_url": base_url,
        "request_timeout_seconds": 3.0,
    }))
    .expect("native Telegram message delivery execution");

    assert_eq!(outcome["message_delivery_state"], "completed");
    assert_eq!(outcome["fallback_count"], 1);
    assert_eq!(outcome["api_call_count"], 2);
    assert_eq!(outcome["chunk_results"][0]["fallback_used"], true);
    let first_request = requests.recv().expect("first loopback request");
    let fallback_request = requests.recv().expect("fallback loopback request");
    assert!(first_request.contains("<b>private-original-input</b>"));
    assert!(first_request.contains("\"parse_mode\":\"HTML\""));
    assert!(fallback_request.contains("private-original-input"));
    assert!(!fallback_request.contains("parse_mode"));
    server.join().expect("loopback Telegram server");
    assert_public_outcome_is_safe(&outcome);
}

fn serve_http_sequence(
    responses: Vec<(&'static str, &'static str)>,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback server should bind");
    let base_url = format!(
        "http://{}",
        listener.local_addr().expect("loopback address")
    );
    let (request_tx, request_rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        for (status, body) in responses {
            let (mut stream, _) = listener.accept().expect("loopback request");
            let request = read_http_request(&mut stream);
            request_tx.send(request).expect("capture loopback request");
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
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
