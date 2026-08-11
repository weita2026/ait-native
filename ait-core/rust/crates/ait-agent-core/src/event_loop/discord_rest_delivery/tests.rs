use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use ait_core::json_support::{json, JsonValue};
use tempfile::tempdir;

use super::*;

const BOT_TOKEN: &str = "discord-bot-secret";
const INTERACTION_TOKEN: &str = "discord-interaction-secret.token";
const APPLICATION_ID: &str = "123456789012345678";
const CHANNEL_ID: &str = "998877665544332211";

#[derive(Default)]
struct StubExecutor {
    json_calls: RefCell<Vec<JsonValue>>,
    multipart_calls: RefCell<Vec<JsonValue>>,
    read_paths: RefCell<Vec<PathBuf>>,
    json_results: RefCell<VecDeque<Result<JsonValue, String>>>,
    multipart_results: RefCell<VecDeque<Result<JsonValue, String>>>,
    file_result: RefCell<Option<Result<Vec<u8>, String>>>,
}

impl StubExecutor {
    fn with_json_results(results: Vec<Result<JsonValue, String>>) -> Self {
        Self {
            json_results: RefCell::new(results.into()),
            ..Self::default()
        }
    }

    fn with_multipart_result(result: Result<JsonValue, String>) -> Self {
        Self {
            multipart_results: RefCell::new(VecDeque::from([result])),
            file_result: RefCell::new(Some(Ok(b"attachment-bytes".to_vec()))),
            ..Self::default()
        }
    }
}

impl DiscordRestDeliveryExecutor for StubExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.json_calls.borrow_mut().push(request.clone());
        self.json_results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Ok(http_success("message-default")))
    }

    fn execute_multipart_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.multipart_calls.borrow_mut().push(request.clone());
        self.multipart_results
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Ok(http_success("attachment-default")))
    }

    fn read_attachment_bytes(&self, path: &Path) -> Result<Vec<u8>, String> {
        self.read_paths.borrow_mut().push(path.to_path_buf());
        self.file_result
            .borrow_mut()
            .take()
            .unwrap_or_else(|| Ok(b"attachment-bytes".to_vec()))
    }
}

fn http_success(message_id: &str) -> JsonValue {
    json!({
        "ok": true,
        "status_code": 200,
        "response_kind": "json",
        "payload": {"id": message_id},
    })
}

fn execute(executor: &StubExecutor, operation: JsonValue) -> JsonValue {
    execute_with_discord_rest_delivery_executor(
        executor,
        &json!({
            "operation": operation,
            "bot_token": BOT_TOKEN,
            "api_base_url": "https://discord.example.test/api/v10/",
            "http_user_agent": "ait-agent-test",
            "timeout_seconds": 12.5,
        }),
    )
    .expect("Discord REST execution")
}

#[test]
fn edit_original_chunks_first_patch_then_followup_without_bot_authorization() {
    let executor = StubExecutor::with_json_results(vec![
        Ok(http_success("message-original")),
        Ok(http_success("message-followup")),
    ]);
    let text = "x".repeat(DISCORD_MESSAGE_LIMIT + 1);

    let result = execute(
        &executor,
        json!({
            "kind": "edit_original_response",
            "application_id": APPLICATION_ID,
            "interaction_token": INTERACTION_TOKEN,
            "text": text,
        }),
    );

    assert_eq!(result["delivery_execution_state"], "delivered");
    assert_eq!(result["chunk_count"], 2);
    assert_eq!(
        result["message_ids"],
        json!(["message-original", "message-followup"])
    );
    assert_eq!(result["python_discord_api_allowed"], false);
    let calls = executor.json_calls.borrow();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["method"], "PATCH");
    assert_eq!(
        calls[0]["url"],
        format!(
            "https://discord.example.test/api/v10/webhooks/{APPLICATION_ID}/{INTERACTION_TOKEN}/messages/@original"
        )
    );
    assert_eq!(
        calls[0]["payload"]["content"].as_str().unwrap().len(),
        2_000
    );
    assert!(calls[0]["headers"].get("Authorization").is_none());
    assert_eq!(calls[1]["method"], "POST");
    assert_eq!(
        calls[1]["url"],
        format!(
            "https://discord.example.test/api/v10/webhooks/{APPLICATION_ID}/{INTERACTION_TOKEN}?wait=true"
        )
    );
    assert_eq!(calls[1]["payload"]["content"], "x");
    assert!(!result.to_string().contains(INTERACTION_TOKEN));
    assert!(!result.to_string().contains(BOT_TOKEN));
}

#[test]
fn followup_and_channel_messages_use_distinct_authorization_contracts() {
    let followup = StubExecutor::default();
    let followup_result = execute(
        &followup,
        json!({
            "kind": "send_followup",
            "application_id": APPLICATION_ID,
            "interaction_token": INTERACTION_TOKEN,
            "text": "followup",
        }),
    );
    assert_eq!(followup_result["ok"], true);
    let followup_calls = followup.json_calls.borrow();
    assert!(followup_calls[0]["headers"].get("Authorization").is_none());
    assert_eq!(followup_calls[0]["headers"]["User-Agent"], "ait-agent-test");

    let channel = StubExecutor::default();
    let channel_result = execute(
        &channel,
        json!({
            "kind": "send_channel_message",
            "channel_id": CHANNEL_ID,
            "text": "channel reply",
        }),
    );
    assert_eq!(channel_result["ok"], true);
    let channel_calls = channel.json_calls.borrow();
    assert_eq!(
        channel_calls[0]["url"],
        format!("https://discord.example.test/api/v10/channels/{CHANNEL_ID}/messages")
    );
    assert_eq!(
        channel_calls[0]["headers"]["Authorization"],
        format!("Bot {BOT_TOKEN}")
    );
    assert_eq!(
        channel_calls[0]["payload"]["allowed_mentions"]["parse"],
        json!([])
    );
    assert!(!channel_result.to_string().contains(BOT_TOKEN));
}

#[test]
fn interaction_and_channel_attachments_read_repo_files_and_build_multipart_payloads() {
    let repo = tempdir().expect("repo");
    fs::create_dir(repo.path().join("artifacts")).expect("artifact dir");
    fs::write(repo.path().join("artifacts/report.md"), b"disk-bytes").expect("artifact");
    let interaction = StubExecutor::with_multipart_result(Ok(http_success("attachment-1")));
    let interaction_result = execute_with_discord_rest_delivery_executor(
        &interaction,
        &json!({
            "repo_root": repo.path(),
            "api_base_url": "https://discord.example.test/api/v10",
            "operation": {
                "kind": "send_followup_attachment",
                "application_id": APPLICATION_ID,
                "interaction_token": INTERACTION_TOKEN,
                "attachment_index": 3,
                "attachment": {
                    "kind": "document",
                    "local_path": "artifacts/report.md",
                    "file_name": "report.md",
                    "mime_type": "text/markdown",
                    "caption": "Build report",
                },
            },
        }),
    )
    .unwrap();

    assert_eq!(interaction_result["ok"], true);
    assert_eq!(interaction_result["attachment_index"], 3);
    assert_eq!(interaction_result["attachment"]["file_name"], "report.md");
    assert_eq!(interaction_result["attachment"]["caption"], "Build report");
    assert_eq!(interaction_result["byte_count"], 16);
    let calls = interaction.multipart_calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["file_field"], "files[0]");
    assert_eq!(calls[0]["file_name"], "report.md");
    assert_eq!(calls[0]["mime_type"], "text/markdown");
    assert_eq!(calls[0]["file_bytes"], json!(b"attachment-bytes"));
    assert_eq!(
        calls[0]["fields"]["payload_json"]["attachments"][0],
        json!({"id": 0, "filename": "report.md", "description": "Build report"})
    );
    assert!(calls[0]["headers"].get("Authorization").is_none());
    assert!(interaction.read_paths.borrow()[0].ends_with("artifacts/report.md"));
    let public = interaction_result.to_string();
    assert!(!public.contains(INTERACTION_TOKEN));
    assert!(!public.contains("artifacts/report.md"));

    let channel = StubExecutor::with_multipart_result(Ok(http_success("attachment-2")));
    let channel_result = execute_with_discord_rest_delivery_executor(
        &channel,
        &json!({
            "repo_root": repo.path(),
            "bot_token": BOT_TOKEN,
            "operation": {
                "kind": "send_channel_attachment",
                "channel_id": CHANNEL_ID,
                "attachment": {"local_path": "artifacts/report.md"},
            },
        }),
    )
    .unwrap();
    assert_eq!(channel_result["ok"], true);
    assert_eq!(
        channel.multipart_calls.borrow()[0]["headers"]["Authorization"],
        format!("Bot {BOT_TOKEN}")
    );
    assert!(!channel_result.to_string().contains(BOT_TOKEN));
}

#[test]
fn failed_delivery_preserves_safe_partial_progress_and_attachment_metadata() {
    let text_executor = StubExecutor::with_json_results(vec![
        Ok(http_success("message-original")),
        Ok(json!({
            "ok": false,
            "status_code": 429,
            "message": format!("retry interaction {INTERACTION_TOKEN}"),
        })),
    ]);
    let text_result = execute(
        &text_executor,
        json!({
            "kind": "edit_original_response",
            "application_id": APPLICATION_ID,
            "interaction_token": INTERACTION_TOKEN,
            "text": "x".repeat(DISCORD_MESSAGE_LIMIT + 1),
        }),
    );

    assert_eq!(text_result["ok"], false);
    assert_eq!(text_result["chunk_count"], 2);
    assert_eq!(text_result["attempted_chunk_count"], 2);
    assert_eq!(text_result["delivered_chunk_count"], 1);
    assert_eq!(text_result["failed_chunk_count"], 1);
    assert_eq!(text_result["message_ids"], json!(["message-original"]));
    assert_eq!(
        text_result["operation_results"].as_array().unwrap().len(),
        2
    );
    assert_eq!(text_result["operation_results"][0]["ok"], true);
    assert_eq!(text_result["operation_results"][1]["status_code"], 429);
    assert!(!text_result.to_string().contains(INTERACTION_TOKEN));

    let repo = tempdir().expect("repo");
    fs::create_dir(repo.path().join("artifacts")).expect("artifact dir");
    let local_path = repo.path().join("artifacts/private-report.md");
    fs::write(&local_path, b"disk-bytes").expect("artifact");
    let attachment_executor = StubExecutor::with_multipart_result(Ok(json!({
        "ok": false,
        "status_code": 413,
        "message": format!(
            "rejected token {INTERACTION_TOKEN} path {}",
            local_path.display()
        ),
    })));
    let attachment_result = execute_with_discord_rest_delivery_executor(
        &attachment_executor,
        &json!({
            "repo_root": repo.path(),
            "operation": {
                "kind": "send_followup_attachment",
                "application_id": APPLICATION_ID,
                "interaction_token": INTERACTION_TOKEN,
                "attachment_index": 7,
                "attachment": {
                    "local_path": "artifacts/private-report.md",
                    "file_name": "public-report.md",
                    "mime_type": "text/markdown",
                },
            },
        }),
    )
    .unwrap();

    assert_eq!(attachment_result["ok"], false);
    assert_eq!(attachment_result["attachment_index"], 7);
    assert_eq!(
        attachment_result["attachment"]["file_name"],
        "public-report.md"
    );
    assert_eq!(attachment_result["byte_count"], 16);
    assert_eq!(attachment_result["operation_results"][0]["ok"], false);
    assert_eq!(
        attachment_result["operation_results"][0]["status_code"],
        413
    );
    let public = attachment_result.to_string();
    assert!(!public.contains(INTERACTION_TOKEN));
    assert!(!public.contains(local_path.to_string_lossy().as_ref()));
    assert!(!public.contains("artifacts/private-report.md"));
}

#[test]
fn channel_history_preserves_messages_and_builds_bounded_query() {
    let executor = StubExecutor::with_json_results(vec![Ok(json!({
        "ok": true,
        "status_code": 200,
        "payload": [
            {"id": "m-2", "content": "new"},
            {"id": "m-1", "content": "old"},
        ],
    }))]);

    let result = execute(
        &executor,
        json!({
            "kind": "list_channel_messages",
            "channel_id": CHANNEL_ID,
            "limit": 999,
            "after": "112233445566778899",
        }),
    );

    assert_eq!(result["ok"], true);
    assert_eq!(result["message_count"], 2);
    assert_eq!(result["message_ids"], json!(["m-2", "m-1"]));
    assert_eq!(result["messages"][0]["content"], "new");
    let calls = executor.json_calls.borrow();
    assert_eq!(calls[0]["method"], "GET");
    assert_eq!(
        calls[0]["url"],
        format!(
            "https://discord.example.test/api/v10/channels/{CHANNEL_ID}/messages?limit=100&after=112233445566778899"
        )
    );
    assert_eq!(
        calls[0]["headers"]["Authorization"],
        format!("Bot {BOT_TOKEN}")
    );
}

#[test]
fn invalid_operations_credentials_paths_and_executor_errors_fail_closed_without_secrets() {
    let missing_token = execute_with_discord_rest_delivery_executor(
        &StubExecutor::default(),
        &json!({
            "operation": {
                "kind": "send_channel_message",
                "channel_id": CHANNEL_ID,
                "text": "hello",
            },
        }),
    )
    .unwrap();
    assert_eq!(missing_token["ok"], false);
    assert!(missing_token["error"]
        .as_str()
        .unwrap()
        .contains("requires a bot token"));

    let missing_identity = execute(
        &StubExecutor::default(),
        json!({"kind": "send_followup", "text": "hello"}),
    );
    assert_eq!(missing_identity["ok"], false);
    assert!(missing_identity["error"]
        .as_str()
        .unwrap()
        .contains("application_id"));

    let unsupported = execute(&StubExecutor::default(), json!({"kind": "unknown"}));
    assert_eq!(unsupported["delivery_execution_state"], "delivery_failed");

    let repo = tempdir().expect("repo");
    let outside = tempdir().expect("outside");
    fs::write(outside.path().join("secret.txt"), b"secret").expect("outside file");
    let traversal = StubExecutor::default();
    let traversed = execute_with_discord_rest_delivery_executor(
        &traversal,
        &json!({
            "repo_root": repo.path(),
            "bot_token": BOT_TOKEN,
            "operation": {
                "kind": "send_channel_attachment",
                "channel_id": CHANNEL_ID,
                "attachment": {"local_path": outside.path().join("secret.txt")},
            },
        }),
    )
    .unwrap();
    assert_eq!(traversed["ok"], false);
    assert!(traversal.read_paths.borrow().is_empty());
    assert!(!traversed
        .to_string()
        .contains(outside.path().to_string_lossy().as_ref()));

    let failing = StubExecutor::with_json_results(vec![Err(format!(
        "executor leaked {BOT_TOKEN} {INTERACTION_TOKEN}"
    ))]);
    let failed = execute(
        &failing,
        json!({
            "kind": "send_followup",
            "application_id": APPLICATION_ID,
            "interaction_token": INTERACTION_TOKEN,
            "text": "hello",
        }),
    );
    assert_eq!(failed["ok"], false);
    let public = failed.to_string();
    assert!(!public.contains(BOT_TOKEN));
    assert!(!public.contains(INTERACTION_TOKEN));

    let malformed_history = StubExecutor::with_json_results(vec![Ok(http_success("not-array"))]);
    let malformed = execute(
        &malformed_history,
        json!({"kind": "list_channel_messages", "channel_id": CHANNEL_ID}),
    );
    assert_eq!(malformed["ok"], false);
    assert!(malformed["error"]
        .as_str()
        .unwrap()
        .contains("non-array payload"));
}

#[test]
fn default_executor_sends_a_real_discord_shaped_json_request_on_loopback() {
    let (api_base_url, request_rx, handle) = serve_once("200 OK", r#"{"id":"m-loopback"}"#);
    let result = agent_discord_rest_delivery_execute_json(&json!({
        "api_base_url": api_base_url,
        "bot_token": BOT_TOKEN,
        "http_user_agent": "ait-agent-loopback",
        "timeout_seconds": 5.0,
        "operation": {
            "kind": "send_channel_message",
            "channel_id": CHANNEL_ID,
            "text": "hello loopback",
        },
    }))
    .expect("loopback execution");

    assert_eq!(result["ok"], true);
    assert_eq!(result["message_ids"], json!(["m-loopback"]));
    let raw = request_rx.recv().expect("request capture");
    assert!(raw.starts_with(&format!("POST /channels/{CHANNEL_ID}/messages HTTP/1.1")));
    let lower = raw.to_ascii_lowercase();
    assert!(lower.contains(&format!("authorization: bot {}", BOT_TOKEN).to_ascii_lowercase()));
    assert!(lower.contains("user-agent: ait-agent-loopback"));
    assert!(raw.ends_with(r#"{"allowed_mentions":{"parse":[]},"content":"hello loopback"}"#));
    assert!(!result.to_string().contains(BOT_TOKEN));
    handle.join().expect("server thread");
}

fn serve_once(
    status: &str,
    response_body: &str,
) -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback");
    let address = listener.local_addr().expect("loopback address");
    let (request_tx, request_rx) = mpsc::channel();
    let status = status.to_string();
    let response_body = response_body.to_string();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept request");
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .expect("read timeout");
        let mut raw = Vec::new();
        let mut buffer = [0u8; 4_096];
        let header_end = loop {
            let read = stream.read(&mut buffer).expect("read request");
            assert!(read > 0, "request ended before headers");
            raw.extend_from_slice(&buffer[..read]);
            if let Some(index) = raw.windows(4).position(|window| window == b"\r\n\r\n") {
                break index + 4;
            }
        };
        let headers = String::from_utf8_lossy(&raw[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.split_once(':').and_then(|(name, value)| {
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
            })
            .unwrap_or(0);
        while raw.len() < header_end + content_length {
            let read = stream.read(&mut buffer).expect("read body");
            assert!(read > 0, "request ended before body");
            raw.extend_from_slice(&buffer[..read]);
        }
        request_tx
            .send(String::from_utf8_lossy(&raw).to_string())
            .expect("capture request");
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}",
            response_body.len()
        )
        .expect("write response");
    });
    (format!("http://{address}"), request_rx, handle)
}
