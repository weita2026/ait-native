use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ait_agent_core::AgentWorkerRuntimeConfig;
use ait_agent_worker::{
    prepare_worker_run_with_env, AgentEventLoopHostWait, SlackHttpCommandJobExecutor,
    SlackWorkerHttpHandler, WorkerHostEventLoop, WorkerHostRuntime, WorkerHttpHostConfig,
    WorkerHttpHostRuntime, WorkerPathInputs, WorkerRunContext, WorkerRunRequest,
};
use ait_core::json_support::{json, JsonValue};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use ring::digest::{digest, SHA1_FOR_LEGACY_USE_ONLY};
use sha2::Sha256;
use tempfile::tempdir;

#[path = "../../../test_support.rs"]
mod workspace_test_support;

const SIGNING_SECRET: &str = "manifest-slack-secret";
const RAW_COMMAND: &str = "team_id=T1&channel_id=C1&user_id=U1&command=%2Fait&text=hello&response_url=https%3A%2F%2Fhooks.slack.test%2Freply&trigger_id=trig-http-runner";

fn agent_worker_binary() -> std::path::PathBuf {
    workspace_test_support::cargo_binary(
        "ait-agent-worker",
        option_env!("CARGO_BIN_EXE_ait-agent-worker"),
    )
}

#[derive(Clone, Default)]
struct StubCommandJob {
    calls: Arc<Mutex<Vec<JsonValue>>>,
}

impl SlackHttpCommandJobExecutor for StubCommandJob {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.lock().expect("calls").push(request.clone());
        Ok(json!({
            "contract": "ait_agent_core.event_loop.SlackCommandJob.v1",
            "migration_stage": "rust_agent_slack_command_job_transaction",
            "command_job_state": "processed",
            "ok": true,
            "processed": true,
            "duplicate": false,
            "conversation_key": "slack:C1:root",
            "binding_created": false,
            "turn_ok": true,
            "delivery_attempted": true,
            "delivered": true,
            "recorded": true,
            "sequence": 1,
            "error_kind": null,
        }))
    }
}

fn fixture_context() -> (tempfile::TempDir, WorkerRunContext) {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        format!(
            r#"{{"version":1,"workers":{{"slack/main":{{"kind":"slack","name":"main","signing_secret":"{SIGNING_SECRET}","command_path":"/command","ack_text":"queued by rust"}}}}}}"#
        ),
    )
    .expect("manifest");
    let context = prepare_worker_run_with_env(
        &WorkerRunRequest {
            transport: "slack".to_string(),
            worker: "main".to_string(),
            event_loop_backend: "portable_poll".to_string(),
            shard: "0".to_string(),
        },
        &WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: Some(temp.path().to_path_buf()),
            manifest_path_override: None,
        },
        BTreeMap::new(),
    )
    .expect("worker context");
    (temp, context)
}

fn slack_runtime(
    context: &WorkerRunContext,
    executor: StubCommandJob,
) -> WorkerHttpHostRuntime<SlackWorkerHttpHandler<StubCommandJob>> {
    let AgentWorkerRuntimeConfig::Slack(config) = &context.config else {
        panic!("Slack config");
    };
    let handler = SlackWorkerHttpHandler::new(config, executor, 2).expect("Slack handler");
    WorkerHttpHostRuntime::new(
        WorkerHttpHostConfig {
            expected_path: config.command_path.clone(),
            enforce_expected_path: false,
            request_timeout: Duration::from_secs(2),
            ..WorkerHttpHostConfig::default()
        },
        handler,
    )
}

fn signature(raw_payload: &str, timestamp: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(SIGNING_SECRET.as_bytes()).expect("HMAC");
    mac.update(format!("v0:{timestamp}:{raw_payload}").as_bytes());
    format!(
        "v0={}",
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

fn exchange(
    context: &WorkerRunContext,
    runtime: &mut WorkerHttpHostRuntime<SlackWorkerHttpHandler<StubCommandJob>>,
    event_loop: &mut dyn WorkerHostEventLoop,
    path: &str,
    signature: &str,
    timestamp: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut client =
        TcpStream::connect(runtime.local_addr().expect("listener address")).expect("client");
    write!(
        client,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nX-Slack-Signature: {signature}\r\nX-Slack-Request-Timestamp: {timestamp}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .expect("request head");
    client.write_all(body).expect("request body");
    client.set_nonblocking(true).expect("nonblocking client");

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut response = Vec::new();
    let mut chunk = [0u8; 4_096];
    loop {
        let events = event_loop.wait(Duration::from_millis(5)).expect("poll");
        runtime
            .tick(context, event_loop, &events)
            .expect("Slack host tick");
        loop {
            match client.read(&mut chunk) {
                Ok(0) => return response,
                Ok(read) => response.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => panic!("client read failed: {error}"),
            }
        }
        assert!(Instant::now() < deadline, "Slack response timed out");
    }
}

fn assert_status(response: &[u8], status: u16) {
    assert!(
        response.starts_with(format!("HTTP/1.1 {status} ").as_bytes()),
        "{}",
        String::from_utf8_lossy(response)
    );
}

#[test]
fn production_slack_handler_verifies_signature_acks_and_reaps_background_job() {
    let (_temp, context) = fixture_context();
    let executor = StubCommandJob::default();
    let calls = executor.calls.clone();
    let mut runtime = slack_runtime(&context, executor);
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start Slack host");
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs()
        .to_string();
    let signed = signature(RAW_COMMAND, &timestamp);

    let success = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/command",
        &signed,
        &timestamp,
        RAW_COMMAND.as_bytes(),
    );
    assert_status(&success, 200);
    assert!(success.ends_with(br#"{"response_type":"ephemeral","text":"queued by rust"}"#));

    let deadline = Instant::now() + Duration::from_secs(3);
    while runtime.inflight_work_count() != 0 {
        let events = event_loop.wait(Duration::from_millis(5)).expect("poll");
        runtime
            .tick(&context, &mut event_loop, &events)
            .expect("reap Slack job");
        assert!(Instant::now() < deadline, "Slack job did not complete");
    }
    assert_eq!(calls.lock().expect("calls").len(), 1);

    let rejected_path = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/wrong",
        &signed,
        &timestamp,
        RAW_COMMAND.as_bytes(),
    );
    assert_status(&rejected_path, 404);
    assert!(rejected_path.ends_with(b"\r\n\r\n"));

    let bad_signature = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/command",
        "v0=bad",
        &timestamp,
        RAW_COMMAND.as_bytes(),
    );
    assert_status(&bad_signature, 401);
    let public = String::from_utf8_lossy(&bad_signature);
    assert!(!public.contains(SIGNING_SECRET));
    assert!(!public.contains("hooks.slack.test"));

    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop Slack host");
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish Slack host");
}

#[test]
fn compiled_slack_runner_reaches_ready_and_stops_cleanly_cross_platform() {
    let (repo, _context) = fixture_context();
    let reserved = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
    let port = reserved.local_addr().expect("reserved address").port();
    drop(reserved);
    fs::write(
        repo.path().join(".ait/agent-workers.json"),
        r#"{"version":1,"workers":{"slack/main":{"kind":"slack","name":"main","signing_secret":"$SIGNING_SECRET","command_path":"/command","ack_text":"queued by rust","bind_host":"127.0.0.1","bind_port":$PORT}}}"#
            .replace("$SIGNING_SECRET", SIGNING_SECRET)
            .replace("$PORT", &port.to_string()),
    )
    .expect("manifest with listener address");
    let mut child = workspace_test_support::worker_command(agent_worker_binary())
        .current_dir(repo.path())
        .env("AIT_REPO_ROOT", repo.path())
        .args([
            "run",
            "--transport",
            "slack",
            "--worker",
            "main",
            "--event-loop-backend",
            "portable_poll",
            "--shard",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Slack worker");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut probe = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) => {
                assert!(
                    child.try_wait().expect("poll child").is_none(),
                    "Slack worker exited before readiness"
                );
                assert!(Instant::now() < deadline, "Slack worker did not bind");
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    probe
        .write_all(b"POST /probe HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("probe request");
    probe
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("probe timeout");
    let mut response = Vec::new();
    probe.read_to_end(&mut response).expect("probe response");
    assert_status(&response, 404);
    workspace_test_support::request_worker_shutdown(child.id());
    let status = workspace_test_support::wait_for_child_exit(
        &mut child,
        "Slack worker",
        Duration::from_secs(5),
    );
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_string(&mut stderr)
        .expect("read child stderr");

    assert!(status.success(), "{stderr}");
    for state in ["ready", "stopping", "stopped"] {
        assert!(
            stderr.contains(&format!("\"state\":\"{state}\"")),
            "{stderr}"
        );
    }
    assert!(stderr.contains("\"transport\":\"slack\""));
    assert!(stderr.contains("\"python_worker_execution_allowed\":false"));
    assert!(!stderr.contains(SIGNING_SECRET));
}

#[test]
fn compiled_app_token_runner_owns_socket_mode_and_stops_without_python_fallback() {
    const APP_TOKEN: &str = "xapp-compiled-socket-mode-secret";
    let websocket_listener = TcpListener::bind(("127.0.0.1", 0)).expect("websocket listener");
    let websocket_address = websocket_listener.local_addr().expect("websocket address");
    let api_listener = TcpListener::bind(("127.0.0.1", 0)).expect("API listener");
    let api_address = api_listener.local_addr().expect("API address");

    let api_server = thread::spawn(move || {
        let (mut stream, _) = api_listener.accept().expect("API accept");
        let request = read_headers(&mut stream);
        assert!(request.starts_with("POST /apps.connections.open HTTP/1.1"));
        assert!(request.contains(&format!("authorization: Bearer {APP_TOKEN}")));
        let body = format!(
            r#"{{"ok":true,"url":"ws://127.0.0.1:{}/socket"}}"#,
            websocket_address.port()
        );
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .expect("API response");
    });

    let websocket_server = thread::spawn(move || {
        let (mut stream, _) = websocket_listener.accept().expect("websocket accept");
        let request = read_headers(&mut stream);
        let key = request
            .lines()
            .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
            .map(str::trim)
            .expect("websocket key");
        let accept = websocket_accept(key);
        write!(
            stream,
            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
        )
        .expect("websocket response");
        stream
            .write_all(&server_text_frame(
                br#"{"type":"hello","num_connections":1}"#,
            ))
            .expect("hello frame");
        stream.flush().expect("hello flush");
        let (opcode, _) = read_masked_client_frame(&mut stream);
        assert_eq!(opcode, 0x8, "worker should send a graceful close frame");
    });

    let repo = tempdir().expect("tempdir");
    fs::create_dir_all(repo.path().join(".ait/agent-runtime")).expect("runtime dir");
    fs::write(
        repo.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    fs::write(
        repo.path().join(".ait/agent-workers.json"),
        format!(
            r#"{{"version":1,"workers":{{"slack/main":{{"kind":"slack","name":"main","app_token":"{APP_TOKEN}","api_base_url":"http://127.0.0.1:{}"}}}}}}"#,
            api_address.port()
        ),
    )
    .expect("manifest");
    let mut child = workspace_test_support::worker_command(agent_worker_binary())
        .current_dir(repo.path())
        .env("AIT_REPO_ROOT", repo.path())
        .args([
            "run",
            "--transport",
            "slack",
            "--worker",
            "main",
            "--event-loop-backend",
            "portable_poll",
            "--shard",
            "0",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Socket Mode worker");

    api_server.join().expect("API server");
    thread::sleep(Duration::from_millis(350));
    assert!(
        child.try_wait().expect("poll child").is_none(),
        "Socket Mode worker exited before termination request"
    );
    workspace_test_support::request_worker_shutdown(child.id());
    let status = workspace_test_support::wait_for_child_exit(
        &mut child,
        "Socket Mode worker",
        Duration::from_secs(5),
    );
    websocket_server.join().expect("websocket server");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_string(&mut stderr)
        .expect("read child stderr");
    assert!(status.success(), "{stderr}");
    assert!(
        stderr.contains("\"runtime_state\":\"awaiting_hello\""),
        "{stderr}"
    );
    assert!(stderr.contains("\"runtime_state\":\"ready\""), "{stderr}");
    assert!(stderr.contains("\"runtime_state\":\"stopped\""), "{stderr}");
    assert!(stderr.contains("\"python_worker_execution_allowed\":false"));
    assert!(!stderr.contains(APP_TOKEN));
}

fn read_headers(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut chunk = [0u8; 1024];
    while !bytes.windows(4).any(|window| window == b"\r\n\r\n") {
        let count = stream.read(&mut chunk).expect("header read");
        assert!(count > 0, "unexpected header EOF");
        bytes.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(bytes).expect("header UTF-8")
}

fn websocket_accept(key: &str) -> String {
    let input = format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    BASE64_STANDARD.encode(digest(&SHA1_FOR_LEGACY_USE_ONLY, input.as_bytes()).as_ref())
}

fn server_text_frame(payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 126);
    let mut frame = vec![0x81, payload.len() as u8];
    frame.extend_from_slice(payload);
    frame
}

fn read_masked_client_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("frame timeout");
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).expect("frame header");
    assert_ne!(header[1] & 0x80, 0, "client frames must be masked");
    let length = usize::from(header[1] & 0x7f);
    assert!(length < 126);
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).expect("frame mask");
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).expect("frame payload");
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    (header[0] & 0x0f, payload)
}
