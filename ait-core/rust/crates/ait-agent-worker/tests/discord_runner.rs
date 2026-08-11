use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ait_agent_core::AgentWorkerRuntimeConfig;
use ait_agent_worker::{
    prepare_worker_run_with_env, AgentEventLoopHostWait, DefaultDiscordHttpInteractionJobExecutor,
    DiscordHttpInteractionJobExecutor, DiscordWorkerHttpHandler, WorkerHostEventLoop,
    WorkerHostRuntime, WorkerHttpHostConfig, WorkerHttpHostRuntime, WorkerPathInputs,
    WorkerRunContext, WorkerRunRequest,
};
use ait_core::json_support::{json, JsonValue};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ring::digest::{digest, SHA1_FOR_LEGACY_USE_ONLY};
use ring::signature::{Ed25519KeyPair, KeyPair};
use tempfile::tempdir;

#[path = "../../../test_support.rs"]
mod workspace_test_support;

const PUBLIC_KEY: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const APPLICATION_ID: &str = "123456789012345678";

fn agent_worker_binary() -> std::path::PathBuf {
    workspace_test_support::cargo_binary(
        "ait-agent-worker",
        option_env!("CARGO_BIN_EXE_ait-agent-worker"),
    )
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
            r#"{{"version":1,"workers":{{"discord/main":{{"kind":"discord","name":"main","application_id":"{APPLICATION_ID}","public_key":"{PUBLIC_KEY}"}}}}}}"#
        ),
    )
    .expect("manifest");
    let context = prepare_worker_run_with_env(
        &WorkerRunRequest {
            transport: "discord".to_string(),
            worker: "main".to_string(),
            event_loop_backend: "portable_poll".to_string(),
            shard: "0".to_string(),
        },
        &WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: Some(temp.path().to_path_buf()),
            manifest_path_override: None,
        },
        BTreeMap::from([(
            "AIT_DISCORD_INTERACTION_PATH".to_string(),
            "/interactions".to_string(),
        )]),
    )
    .expect("worker context");
    (temp, context)
}

fn discord_runtime(
    context: &WorkerRunContext,
) -> WorkerHttpHostRuntime<DiscordWorkerHttpHandler<DefaultDiscordHttpInteractionJobExecutor>> {
    discord_runtime_with_executor(context, DefaultDiscordHttpInteractionJobExecutor)
}

fn discord_runtime_with_executor<E>(
    context: &WorkerRunContext,
    executor: E,
) -> WorkerHttpHostRuntime<DiscordWorkerHttpHandler<E>>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let AgentWorkerRuntimeConfig::Discord(config) = &context.config else {
        panic!("Discord config");
    };
    let handler = DiscordWorkerHttpHandler::new(config, executor, 2).expect("Discord handler");
    WorkerHttpHostRuntime::new(
        WorkerHttpHostConfig {
            expected_path: config.interaction_path.clone(),
            enforce_expected_path: false,
            request_timeout: Duration::from_secs(2),
            ..WorkerHttpHostConfig::default()
        },
        handler,
    )
}

fn signature(raw_payload: &[u8], timestamp: &str) -> String {
    let seed = (0u8..32).collect::<Vec<_>>();
    let pair = Ed25519KeyPair::from_seed_unchecked(&seed).expect("Ed25519 seed");
    assert_eq!(hex(pair.public_key().as_ref()), PUBLIC_KEY);
    let mut message = timestamp.as_bytes().to_vec();
    message.extend_from_slice(raw_payload);
    hex(pair.sign(&message).as_ref())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn exchange<E>(
    context: &WorkerRunContext,
    runtime: &mut WorkerHttpHostRuntime<DiscordWorkerHttpHandler<E>>,
    event_loop: &mut dyn WorkerHostEventLoop,
    path: &str,
    signature: &str,
    timestamp: &str,
    body: &[u8],
) -> Vec<u8>
where
    E: DiscordHttpInteractionJobExecutor,
{
    let mut client =
        TcpStream::connect(runtime.local_addr().expect("listener address")).expect("client");
    write!(
        client,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nX-Signature-Ed25519: {signature}\r\nX-Signature-Timestamp: {timestamp}\r\nContent-Length: {}\r\n\r\n",
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
            .expect("Discord host tick");
        loop {
            match client.read(&mut chunk) {
                Ok(0) => return response,
                Ok(read) => response.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => panic!("client read failed: {error}"),
            }
        }
        assert!(Instant::now() < deadline, "Discord response timed out");
    }
}

fn assert_status(response: &[u8], status: u16) {
    assert!(
        response.starts_with(format!("HTTP/1.1 {status} ").as_bytes()),
        "{}",
        String::from_utf8_lossy(response)
    );
}

#[derive(Clone, Default)]
struct BlockingDiscordExecutor {
    gate: Arc<(Mutex<bool>, Condvar)>,
    job_calls: Arc<Mutex<Vec<JsonValue>>>,
    delivery_calls: Arc<Mutex<Vec<JsonValue>>>,
}

impl BlockingDiscordExecutor {
    fn release(&self) {
        let (lock, ready) = &*self.gate;
        *lock.lock().expect("gate lock") = true;
        ready.notify_all();
    }
}

impl DiscordHttpInteractionJobExecutor for BlockingDiscordExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.job_calls
            .lock()
            .expect("job calls")
            .push(request.clone());
        let (lock, ready) = &*self.gate;
        let released = lock.lock().expect("gate lock");
        drop(
            ready
                .wait_while(released, |released| !*released)
                .expect("gate wait"),
        );
        Ok(json!({
            "contract": "ait_agent_core.event_loop.DiscordInteractionJob.v1",
            "migration_stage": "rust_agent_discord_interaction_job_transaction",
            "interaction_job_state": "processed",
            "ok": true,
            "processed": true,
            "duplicate": false,
            "conversation_key": "discord:loopback-channel",
            "binding_created": false,
            "turn_ok": true,
            "recorded": true,
            "sequence": 7,
            "response": {"type": 4, "data": {"content": "loopback reply"}},
            "delivery_request": {
                "reply_mode": "interaction",
                "operations": [{
                    "kind": "edit_original_response",
                    "application_id": APPLICATION_ID,
                    "interaction_token": "loopback-interaction-token",
                    "text": "loopback reply",
                }],
            },
            "recovery_request": null,
            "error_kind": null,
        }))
    }

    fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.delivery_calls
            .lock()
            .expect("delivery calls")
            .push(request.clone());
        Ok(json!({
            "contract": "ait_agent_core.event_loop.DiscordRestDeliveryExecution.v1",
            "migration_stage": "rust_agent_discord_rest_delivery_execution",
            "transport": "discord",
            "kind": request["operation"]["kind"].clone(),
            "ok": true,
            "delivered": true,
            "completed": true,
            "delivery_execution_state": "delivered",
            "python_discord_api_allowed": false,
            "python_file_read_allowed": false,
        }))
    }

    fn execute_state(
        &self,
        _path: &str,
        operation: &str,
        _request: &JsonValue,
    ) -> Result<JsonValue, String> {
        match operation {
            "has_recent_value" => Ok(JsonValue::Bool(false)),
            "list_bindings" => Ok(JsonValue::Array(Vec::new())),
            _ => Ok(json!({"transport": "discord", "surface_id": "fixture"})),
        }
    }

    fn execute_backend(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Ok(json!({"ok": true, "payload": []}))
    }
}

#[test]
fn production_discord_handler_verifies_signature_and_returns_pong_on_loopback() {
    let (_temp, context) = fixture_context();
    let mut runtime = discord_runtime(&context);
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start Discord host");
    let body = br#"{"type":1}"#;
    let timestamp = "1714990000";
    let signed = signature(body, timestamp);

    let success = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/interactions",
        &signed,
        timestamp,
        body,
    );
    assert_status(&success, 200);
    assert!(success.ends_with(br#"{"type":1}"#));
    assert_eq!(runtime.inflight_work_count(), 0);

    let rejected_path = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/wrong",
        &signed,
        timestamp,
        body,
    );
    assert_status(&rejected_path, 404);
    assert!(rejected_path.ends_with(b"\r\n\r\n"));

    let bad_signature = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/interactions",
        &"00".repeat(64),
        timestamp,
        body,
    );
    assert_status(&bad_signature, 401);
    let public = String::from_utf8_lossy(&bad_signature);
    assert!(!public.contains(PUBLIC_KEY));
    assert!(!public.contains(APPLICATION_ID));

    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop Discord host");
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish Discord host");
}

#[test]
fn loopback_command_acknowledges_before_blocked_background_delivery_completes() {
    let (_temp, context) = fixture_context();
    let executor = BlockingDiscordExecutor::default();
    let observed = executor.clone();
    let mut runtime = discord_runtime_with_executor(&context, executor);
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start Discord host");
    let body = br#"{"id":"112233445566778899","type":2,"token":"loopback-interaction-token","application_id":"123456789012345678","channel_id":"998877665544332211","data":{"name":"ask","options":[{"name":"text","type":3,"value":"hello from loopback"}]},"member":{"user":{"id":"discord-user-1","username":"weita"}}}"#;
    let timestamp = "1714990001";
    let signed = signature(body, timestamp);

    let acknowledgement = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/interactions",
        &signed,
        timestamp,
        body,
    );
    assert_status(&acknowledgement, 200);
    assert!(acknowledgement.ends_with(br#"{"type":5}"#));
    assert_eq!(runtime.inflight_work_count(), 1);
    assert!(observed
        .delivery_calls
        .lock()
        .expect("delivery calls")
        .is_empty());

    observed.release();
    let deadline = Instant::now() + Duration::from_secs(3);
    while runtime.inflight_work_count() != 0 {
        let events = event_loop.wait(Duration::from_millis(5)).expect("poll");
        runtime
            .tick(&context, &mut event_loop, &events)
            .expect("reap background delivery");
        assert!(Instant::now() < deadline, "background delivery timed out");
    }
    assert_eq!(observed.job_calls.lock().expect("job calls").len(), 1);
    let deliveries = observed.delivery_calls.lock().expect("delivery calls");
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0]["operation"]["kind"], "edit_original_response");
    assert_eq!(deliveries[0]["operation"]["text"], "loopback reply");
    drop(deliveries);

    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop Discord host");
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish Discord host");
}

#[test]
fn compiled_discord_runner_reaches_ready_and_stops_cleanly_cross_platform() {
    let (repo, _context) = fixture_context();
    let reserved = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
    let port = reserved.local_addr().expect("reserved address").port();
    drop(reserved);
    let mut child = Command::new(agent_worker_binary())
        .current_dir(repo.path())
        .env("AIT_REPO_ROOT", repo.path())
        .env("AIT_DISCORD_BIND_HOST", "127.0.0.1")
        .env("AIT_DISCORD_BIND_PORT", port.to_string())
        .args([
            "run",
            "--transport",
            "discord",
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
        .expect("spawn Discord worker");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut probe = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) => {
                assert!(
                    child.try_wait().expect("poll child").is_none(),
                    "Discord worker exited before readiness"
                );
                assert!(Instant::now() < deadline, "Discord worker did not bind");
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
    workspace_test_support::request_worker_shutdown(repo.path(), "discord", "main", child.id());
    let status = workspace_test_support::wait_for_child_exit(
        &mut child,
        "Discord worker",
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
    assert!(stderr.contains("\"transport\":\"discord\""));
    assert!(stderr.contains("\"python_worker_execution_allowed\":false"));
    assert!(!stderr.contains(PUBLIC_KEY));
    assert!(!stderr.contains(APPLICATION_ID));
}

#[test]
fn compiled_bot_token_runner_owns_gateway_and_stops_without_python_fallback() {
    const BOT_TOKEN: &str = "discord-compiled-gateway-secret";
    let websocket_listener = TcpListener::bind(("127.0.0.1", 0)).expect("websocket listener");
    let websocket_address = websocket_listener.local_addr().expect("websocket address");
    let api_listener = TcpListener::bind(("127.0.0.1", 0)).expect("API listener");
    let api_address = api_listener.local_addr().expect("API address");

    let api_server = thread::spawn(move || {
        let (mut stream, _) = api_listener.accept().expect("API accept");
        let request = read_headers(&mut stream);
        assert!(
            request.starts_with("GET /gateway/bot HTTP/1.1"),
            "{request}"
        );
        assert!(
            request.contains(&format!("authorization: Bot {BOT_TOKEN}")),
            "{request}"
        );
        let body = format!(
            r#"{{"url":"ws://127.0.0.1:{}/gateway"}}"#,
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
        assert!(
            request.starts_with("GET /gateway?v=10&encoding=json HTTP/1.1"),
            "{request}"
        );
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
            .write_all(&server_websocket_frame(
                false,
                0x1,
                br#"{"op":10,"d":{"heartbeat_interval":60000"#,
            ))
            .expect("Hello fragment start");
        stream
            .write_all(&server_websocket_frame(true, 0x0, br#"}}"#))
            .expect("Hello fragment end");
        stream
            .write_all(&server_websocket_frame(true, 0x9, b"gateway-probe"))
            .expect("Gateway ping");
        stream.flush().expect("Gateway fixture flush");

        let mut identify = None;
        let mut saw_pong = false;
        while identify.is_none() || !saw_pong {
            let (opcode, payload) = read_masked_websocket_frame(&mut stream);
            match opcode {
                0x1 => identify = Some(payload),
                0xA => {
                    assert_eq!(payload, b"gateway-probe");
                    saw_pong = true;
                }
                other => panic!("unexpected pre-ready opcode {other}"),
            }
        }
        let identify = JsonValue::from(
            String::from_utf8(identify.expect("Identify payload")).expect("Identify UTF-8"),
        );
        let identify = ait_core::json_support::JsonCodec::parse_value_with_error_prefix(
            identify.as_str().expect("Identify text"),
            "Identify JSON",
        )
        .expect("Identify JSON");
        assert_eq!(identify["op"], 2);
        assert_eq!(identify["d"]["token"], BOT_TOKEN);
        assert_eq!(identify["d"]["intents"], (1 << 9) | (1 << 12) | (1 << 15));
        let ready = json!({
            "op": 0,
            "s": 1,
            "t": "READY",
            "d": {
                "session_id": "compiled-discord-session",
                "resume_gateway_url": format!(
                    "ws://127.0.0.1:{}",
                    websocket_address.port()
                ),
                "user": {"id": "compiled-discord-bot", "bot": true},
            },
        })
        .to_string();
        stream
            .write_all(&server_websocket_frame(true, 0x1, ready.as_bytes()))
            .expect("READY frame");
        stream.flush().expect("READY flush");

        let (opcode, _) = read_masked_websocket_frame(&mut stream);
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
            r#"{{"version":1,"workers":{{"discord/main":{{"kind":"discord","name":"main","application_id":"{APPLICATION_ID}","bot_token":"{BOT_TOKEN}"}}}}}}"#
        ),
    )
    .expect("manifest");
    let mut child = Command::new(agent_worker_binary())
        .current_dir(repo.path())
        .env("AIT_REPO_ROOT", repo.path())
        .env(
            "AIT_DISCORD_API_BASE_URL",
            format!("http://127.0.0.1:{}", api_address.port()),
        )
        .args([
            "run",
            "--transport",
            "discord",
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
        .expect("spawn Discord Gateway worker");

    api_server.join().expect("API server");
    thread::sleep(Duration::from_millis(350));
    assert!(
        child.try_wait().expect("poll child").is_none(),
        "Discord Gateway worker exited before termination request"
    );
    workspace_test_support::request_worker_shutdown(repo.path(), "discord", "main", child.id());
    let status = workspace_test_support::wait_for_child_exit(
        &mut child,
        "Discord Gateway worker",
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
    for state in ["awaiting_hello", "identifying", "ready", "stopped"] {
        assert!(
            stderr.contains(&format!("\"runtime_state\":\"{state}\"")),
            "{stderr}"
        );
    }
    assert!(stderr.contains("\"transport\":\"discord\""));
    assert!(stderr.contains("\"python_worker_execution_allowed\":false"));
    assert!(!stderr.contains(BOT_TOKEN));
    assert!(!stderr.contains(APPLICATION_ID));
}

fn read_headers(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
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

fn server_websocket_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![(if fin { 0x80 } else { 0 }) | opcode];
    match payload.len() {
        length @ 0..=125 => frame.push(length as u8),
        length @ 126..=65_535 => {
            frame.push(126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            frame.push(127);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    frame.extend_from_slice(payload);
    frame
}

fn read_masked_websocket_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("frame timeout");
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).expect("frame header");
    assert_ne!(header[1] & 0x80, 0, "client frames must be masked");
    let length = match header[1] & 0x7f {
        126 => {
            let mut extended = [0u8; 2];
            stream.read_exact(&mut extended).expect("16-bit length");
            usize::from(u16::from_be_bytes(extended))
        }
        127 => {
            let mut extended = [0u8; 8];
            stream.read_exact(&mut extended).expect("64-bit length");
            usize::try_from(u64::from_be_bytes(extended)).expect("frame length")
        }
        length => usize::from(length),
    };
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).expect("frame mask");
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).expect("frame payload");
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    (header[0] & 0x0f, payload)
}
