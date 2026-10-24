use std::collections::{BTreeMap, VecDeque};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ait_agent_core::{
    agent_runtime_admission_plan_json, resolve_agent_worker_config, AgentEvent,
    AgentEventLoopBackend, AgentWorkerConfigInput, AgentWorkerRuntimeConfig, TransportKind,
};
use ait_core::json_support::{json, JsonValue};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ring::digest::{digest, SHA1_FOR_LEGACY_USE_ONLY};
use tempfile::{tempdir, TempDir};

use super::*;
use crate::paths::ResolvedWorkerPaths;

#[derive(Default)]
struct FakeEventLoop {
    registered: Vec<(u64, NativeSocket)>,
    unregistered: Vec<u64>,
}

impl WorkerHostEventLoop for FakeEventLoop {
    fn register_readable(&mut self, token: u64, fd: NativeSocket) -> Result<(), WorkerDiagnostic> {
        self.registered.push((token, fd));
        Ok(())
    }

    fn register_read_write(
        &mut self,
        token: u64,
        fd: NativeSocket,
    ) -> Result<(), WorkerDiagnostic> {
        self.registered.push((token, fd));
        Ok(())
    }

    fn unregister(&mut self, token: u64) -> Result<(), WorkerDiagnostic> {
        self.unregistered.push(token);
        Ok(())
    }

    fn wait(&mut self, _timeout: Duration) -> Result<Vec<AgentEvent>, WorkerDiagnostic> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct FakeConnectionState {
    reads: VecDeque<Result<Vec<SlackSocketModeEvent>, &'static str>>,
    writes: Vec<JsonValue>,
    ping_count: usize,
    close_frames: Vec<(u16, String)>,
    closed: bool,
    timeline: Vec<&'static str>,
    shared_timeline: Option<Arc<Mutex<Vec<&'static str>>>>,
}

struct FakeConnection {
    fd: NativeSocket,
    state: Arc<Mutex<FakeConnectionState>>,
}

impl SlackSocketModeConnection for FakeConnection {
    fn raw_fd(&self) -> NativeSocket {
        self.fd
    }

    fn read_events(&mut self) -> Result<Vec<SlackSocketModeEvent>, WorkerDiagnostic> {
        match self
            .state
            .lock()
            .expect("connection state")
            .reads
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()))
        {
            Ok(events) => Ok(events),
            Err(_) => Err(slack_socket_read_failed()),
        }
    }

    fn send_json(&mut self, payload: &JsonValue) -> Result<(), WorkerDiagnostic> {
        let mut state = self.state.lock().expect("connection state");
        state.timeline.push("ack");
        if let Some(timeline) = &state.shared_timeline {
            timeline.lock().expect("shared timeline").push("ack");
        }
        state.writes.push(payload.clone());
        Ok(())
    }

    fn send_ping(&mut self) -> Result<(), WorkerDiagnostic> {
        self.state.lock().expect("connection state").ping_count += 1;
        Ok(())
    }

    fn send_close(&mut self, status_code: u16, reason: &str) -> Result<(), WorkerDiagnostic> {
        self.state
            .lock()
            .expect("connection state")
            .close_frames
            .push((status_code, reason.to_string()));
        Ok(())
    }

    fn close(&mut self) {
        self.state.lock().expect("connection state").closed = true;
    }
}

enum ConnectorStep {
    Connection(NativeSocket, Arc<Mutex<FakeConnectionState>>),
    Error(&'static str),
}

#[derive(Clone)]
struct FakeConnector {
    steps: Arc<Mutex<VecDeque<ConnectorStep>>>,
    calls: Arc<Mutex<usize>>,
}

impl FakeConnector {
    fn new(steps: Vec<ConnectorStep>) -> Self {
        Self {
            steps: Arc::new(Mutex::new(steps.into())),
            calls: Arc::new(Mutex::new(0)),
        }
    }
}

impl SlackSocketModeConnector for FakeConnector {
    fn connect(
        &mut self,
        _config: &SlackWorkerConfig,
    ) -> Result<Box<dyn SlackSocketModeConnection>, WorkerDiagnostic> {
        *self.calls.lock().expect("connector calls") += 1;
        match self
            .steps
            .lock()
            .expect("connector steps")
            .pop_front()
            .expect("connector step")
        {
            ConnectorStep::Connection(fd, state) => Ok(Box::new(FakeConnection { fd, state })),
            ConnectorStep::Error("auth") => Err(WorkerDiagnostic::new(
                "slack_socket_mode_auth_failed",
                "fixture auth failure",
                EXIT_INVALID_CONFIGURATION,
            )),
            ConnectorStep::Error(_) => Err(slack_socket_connect_failed()),
        }
    }
}

#[derive(Clone)]
struct FakeClock {
    now: Arc<Mutex<Duration>>,
}

impl FakeClock {
    fn new() -> Self {
        Self {
            now: Arc::new(Mutex::new(Duration::ZERO)),
        }
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.now.lock().expect("clock");
        *now = now.saturating_add(duration);
    }
}

impl SlackSocketModeClock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().expect("clock")
    }
}

#[derive(Clone)]
struct FakeCommandExecutor {
    calls: Arc<Mutex<Vec<JsonValue>>>,
    timeline: Arc<Mutex<Vec<&'static str>>>,
}

impl SlackHttpCommandJobExecutor for FakeCommandExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.timeline.lock().expect("timeline").push("command");
        self.calls
            .lock()
            .expect("command calls")
            .push(request.clone());
        Ok(processed_job())
    }
}

fn processed_job() -> JsonValue {
    json!({
        "contract": "ait_agent_core.event_loop.SlackCommandJob.v1",
        "migration_stage": "rust_agent_slack_command_job_transaction",
        "command_job_state": "processed",
        "ok": true,
        "processed": true,
        "duplicate": false,
        "turn_ok": true,
        "delivery_attempted": true,
        "delivered": true,
        "recorded": true,
        "sequence": 7,
        "error_kind": null,
    })
}

fn fixture() -> (TempDir, SlackWorkerConfig, WorkerRunContext) {
    let temp = tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    std::fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    let config = resolve_agent_worker_config(AgentWorkerConfigInput {
        repo_root: temp.path().to_path_buf(),
        worker_key: "slack/main".to_string(),
        worker: json!({
            "kind": "slack",
            "name": "main",
            "app_token": "xapp-fixture-secret",
        }),
        process_env: BTreeMap::new(),
    })
    .expect("Slack config");
    let AgentWorkerRuntimeConfig::Slack(slack) = config.clone() else {
        panic!("Slack config")
    };
    let context = WorkerRunContext {
        paths: ResolvedWorkerPaths {
            repo_root: temp.path().to_path_buf(),
            manifest_path: temp.path().join(".ait/agent-workers.json"),
        },
        transport: TransportKind::Slack,
        worker_key: "slack/main".to_string(),
        worker_name: "main".to_string(),
        event_loop_backend: AgentEventLoopBackend::PortablePoll,
        shard_index: 0,
        runtime_admission_plan: agent_runtime_admission_plan_json(&json!({
            "worker_manifest": {
                "version": 1,
                "workers": {"slack/main": {"kind": "slack", "name": "main"}}
            },
            "backend": "portable_poll",
            "transport_runtime": "rust",
            "allow_python_fallback": false,
            "requested_worker_keys": ["slack/main"],
        }))
        .expect("admission"),
        config,
    };
    (temp, slack, context)
}

fn readable_event() -> AgentEvent {
    AgentEvent {
        token: SLACK_SOCKET_EVENT_LOOP_TOKEN,
        readable: true,
        writable: false,
        hangup: false,
    }
}

fn slash_command(envelope_id: &str) -> String {
    json!({
        "envelope_id": envelope_id,
        "type": "slash_commands",
        "accepts_response_payload": true,
        "payload": {
            "team_id": "T1",
            "channel_id": "C1",
            "channel_name": "ops",
            "user_id": "U1",
            "user_name": "alice",
            "command": "/ait",
            "text": "hello world",
            "response_url": "https://hooks.slack.test/secret-response",
            "trigger_id": "trigger-1"
        }
    })
    .to_string()
}

fn runtime(
    config: SlackWorkerConfig,
    connector: FakeConnector,
    clock: FakeClock,
    command: FakeCommandExecutor,
    settings: SlackSocketModeSettings,
) -> SlackSocketModeWorkerRuntime<FakeConnector, FakeCommandExecutor, FakeClock> {
    SlackSocketModeWorkerRuntime::new(config, connector, command, clock, settings)
        .expect("Socket Mode runtime")
}

#[test]
fn socket_mode_acknowledges_before_command_and_deduplicates_envelopes() {
    let (_temp, config, context) = fixture();
    let command_calls = Arc::new(Mutex::new(Vec::new()));
    let command_timeline = Arc::new(Mutex::new(Vec::new()));
    let connection = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([
            Ok(vec![SlackSocketModeEvent::Text(
                json!({"type": "hello", "num_connections": 1}).to_string(),
            )]),
            Ok(vec![
                SlackSocketModeEvent::Text(slash_command("env-1")),
                SlackSocketModeEvent::Text(slash_command("env-1")),
            ]),
        ]),
        shared_timeline: Some(command_timeline.clone()),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![ConnectorStep::Connection(41, connection.clone())]);
    let clock = FakeClock::new();
    let command = FakeCommandExecutor {
        calls: command_calls.clone(),
        timeline: command_timeline.clone(),
    };
    let mut runtime = runtime(
        config,
        connector,
        clock,
        command,
        SlackSocketModeSettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).expect("start");
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("hello");
    assert_eq!(runtime.runtime_state(), "ready");
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("commands");

    let deadline = Instant::now() + Duration::from_secs(3);
    while command_calls.lock().expect("calls").is_empty() {
        runtime.tick(&context, &mut event_loop, &[]).expect("reap");
        assert!(Instant::now() < deadline, "command job timeout");
        thread::yield_now();
    }
    runtime.tick(&context, &mut event_loop, &[]).expect("reap");
    let state = connection.lock().expect("connection");
    assert_eq!(state.writes.len(), 2, "duplicates are acknowledged");
    assert_eq!(state.writes[0]["envelope_id"], "env-1");
    assert_eq!(state.timeline, vec!["ack", "ack"]);
    assert_eq!(command_calls.lock().expect("calls").len(), 1);
    assert_eq!(
        command_calls.lock().expect("calls")[0]["command_payload"]["text"],
        "hello world"
    );
    let timeline = command_timeline.lock().expect("timeline");
    assert_eq!(
        timeline.iter().filter(|event| **event == "command").count(),
        1
    );
    assert_eq!(
        timeline.first(),
        Some(&"ack"),
        "ACK must precede command effects"
    );
}

#[test]
fn socket_mode_capacity_pressure_acks_and_keeps_the_connection_open() {
    let (_temp, config, context) = fixture();
    let command_calls = Arc::new(Mutex::new(Vec::new()));
    let connection = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([
            Ok(vec![SlackSocketModeEvent::Text(
                json!({"type": "hello"}).to_string(),
            )]),
            Ok(vec![
                SlackSocketModeEvent::Text(slash_command("env-capacity-1")),
                SlackSocketModeEvent::Text(slash_command("env-capacity-2")),
            ]),
        ]),
        ..FakeConnectionState::default()
    }));
    let mut runtime = runtime(
        config,
        FakeConnector::new(vec![ConnectorStep::Connection(42, connection.clone())]),
        FakeClock::new(),
        FakeCommandExecutor {
            calls: command_calls.clone(),
            timeline: Arc::default(),
        },
        SlackSocketModeSettings {
            max_inflight_jobs: 1,
            ..SlackSocketModeSettings::default()
        },
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).expect("start");
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("hello");
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("capacity pressure must not reconnect");

    assert_eq!(runtime.runtime_state(), "ready");
    assert_eq!(
        runtime.last_diagnostic_code(),
        Some("worker_job_capacity_exhausted")
    );
    assert_eq!(connection.lock().expect("connection").writes.len(), 2);
    assert!(!connection.lock().expect("connection").closed);
    assert!(event_loop.unregistered.is_empty());

    let deadline = Instant::now() + Duration::from_secs(3);
    while command_calls.lock().expect("calls").is_empty() {
        assert!(Instant::now() < deadline, "command job timeout");
        thread::yield_now();
    }
    runtime.tick(&context, &mut event_loop, &[]).expect("reap");
    assert_eq!(command_calls.lock().expect("calls").len(), 1);
}

#[test]
fn socket_mode_heartbeat_reconnects_and_resumes_after_transient_disconnect() {
    let (_temp, config, context) = fixture();
    let first = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([
            Ok(vec![SlackSocketModeEvent::Pong]),
            Ok(vec![SlackSocketModeEvent::Closed]),
        ]),
        ..FakeConnectionState::default()
    }));
    let second = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![SlackSocketModeEvent::Text(
            json!({"type": "hello"}).to_string(),
        )])]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![
        ConnectorStep::Connection(51, first.clone()),
        ConnectorStep::Connection(52, second.clone()),
    ]);
    let clock = FakeClock::new();
    let mut runtime = runtime(
        config,
        connector,
        clock.clone(),
        FakeCommandExecutor {
            calls: Arc::default(),
            timeline: Arc::default(),
        },
        SlackSocketModeSettings {
            ping_interval: Duration::from_secs(2),
            pong_timeout: Duration::from_secs(1),
            reconnect_base_delay: Duration::from_secs(1),
            ..SlackSocketModeSettings::default()
        },
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).expect("start");
    clock.advance(Duration::from_secs(2));
    runtime.tick(&context, &mut event_loop, &[]).expect("ping");
    assert_eq!(first.lock().expect("first").ping_count, 1);
    assert_eq!(runtime.runtime_state(), "awaiting_pong");
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("pong");
    assert_eq!(runtime.runtime_state(), "ready");
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("close");
    assert_eq!(runtime.runtime_state(), "reconnect_wait");
    assert_eq!(runtime.reconnect_attempt(), 1);
    clock.advance(Duration::from_secs(1));
    runtime
        .tick(&context, &mut event_loop, &[])
        .expect("reconnect");
    assert_eq!(runtime.runtime_state(), "awaiting_hello");
    assert_eq!(runtime.reconnect_attempt(), 1);
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("resumed hello");
    assert_eq!(runtime.runtime_state(), "ready");
    assert_eq!(runtime.reconnect_attempt(), 0);
    assert_eq!(
        event_loop.registered,
        vec![
            (SLACK_SOCKET_EVENT_LOOP_TOKEN, 51),
            (SLACK_SOCKET_EVENT_LOOP_TOKEN, 52)
        ]
    );
    assert_eq!(event_loop.unregistered, vec![SLACK_SOCKET_EVENT_LOOP_TOKEN]);
}

#[test]
fn socket_mode_disconnect_stops_processing_the_retired_connection_batch() {
    let (_temp, config, context) = fixture();
    let command_calls = Arc::new(Mutex::new(Vec::new()));
    let connection = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![
            SlackSocketModeEvent::Text(
                json!({"type": "disconnect", "reason": "refresh_requested"}).to_string(),
            ),
            SlackSocketModeEvent::Text(slash_command("env-after-disconnect")),
        ])]),
        ..FakeConnectionState::default()
    }));
    let mut runtime = runtime(
        config,
        FakeConnector::new(vec![ConnectorStep::Connection(53, connection.clone())]),
        FakeClock::new(),
        FakeCommandExecutor {
            calls: command_calls.clone(),
            timeline: Arc::default(),
        },
        SlackSocketModeSettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).expect("start");
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("disconnect");

    assert_eq!(runtime.runtime_state(), "reconnect_wait");
    assert_eq!(runtime.reconnect_attempt(), 1);
    assert!(command_calls.lock().expect("calls").is_empty());
    assert!(connection.lock().expect("connection").writes.is_empty());
    assert_eq!(event_loop.unregistered, vec![SLACK_SOCKET_EVENT_LOOP_TOKEN]);
}

#[test]
fn socket_mode_auth_failure_and_reconnect_exhaustion_fail_closed() {
    let (_temp, config, context) = fixture();
    let mut auth = runtime(
        config.clone(),
        FakeConnector::new(vec![ConnectorStep::Error("auth")]),
        FakeClock::new(),
        FakeCommandExecutor {
            calls: Arc::default(),
            timeline: Arc::default(),
        },
        SlackSocketModeSettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();
    let error = auth
        .start(&context, &mut event_loop)
        .expect_err("auth failure");
    assert_eq!(error.code, "slack_socket_mode_auth_failed");
    assert!(!error.render_json().contains("xapp-fixture-secret"));

    let connection = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![SlackSocketModeEvent::Closed])]),
        ..FakeConnectionState::default()
    }));
    let clock = FakeClock::new();
    let mut exhausted = runtime(
        config,
        FakeConnector::new(vec![
            ConnectorStep::Connection(61, connection),
            ConnectorStep::Error("transient"),
        ]),
        clock.clone(),
        FakeCommandExecutor {
            calls: Arc::default(),
            timeline: Arc::default(),
        },
        SlackSocketModeSettings {
            max_reconnect_attempts: 1,
            reconnect_base_delay: Duration::from_millis(1),
            ..SlackSocketModeSettings::default()
        },
    );
    exhausted.start(&context, &mut event_loop).expect("start");
    exhausted
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("schedule reconnect");
    clock.advance(Duration::from_millis(1));
    let error = exhausted
        .tick(&context, &mut event_loop, &[])
        .expect_err("attempts exhausted");
    assert_eq!(error.code, "slack_socket_mode_reconnect_exhausted");
    assert_eq!(exhausted.runtime_state(), "reconnect_exhausted");
}

#[test]
fn socket_mode_malformed_read_reconnects_and_shutdown_closes_cleanly() {
    let (_temp, config, context) = fixture();
    let connection = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Err("malformed")]),
        ..FakeConnectionState::default()
    }));
    let mut runtime = runtime(
        config,
        FakeConnector::new(vec![ConnectorStep::Connection(71, connection.clone())]),
        FakeClock::new(),
        FakeCommandExecutor {
            calls: Arc::default(),
            timeline: Arc::default(),
        },
        SlackSocketModeSettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).expect("start");
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .expect("reconnect after malformed frame");
    assert_eq!(
        runtime.last_diagnostic_code(),
        Some("slack_socket_mode_read_failed")
    );
    assert!(connection.lock().expect("connection").closed);
    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("shutdown");
    assert_eq!(runtime.runtime_state(), "stopping");
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish");
    assert_eq!(runtime.runtime_state(), "stopped");
}

#[derive(Clone)]
struct StubHttpExecutor {
    calls: Arc<Mutex<Vec<JsonValue>>>,
    result: JsonValue,
}

impl SlackSocketModeHttpExecutor for StubHttpExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.lock().expect("HTTP calls").push(request.clone());
        Ok(self.result.clone())
    }
}

#[test]
fn connection_url_acquisition_uses_native_http_contract_and_sanitizes_auth_errors() {
    let (_temp, config, _context) = fixture();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let success = StubHttpExecutor {
        calls: calls.clone(),
        result: json!({
            "ok": true,
            "status_code": 200,
            "payload": {"ok": true, "url": "wss://wss-primary.slack.test/link"}
        }),
    };
    let url = acquire_socket_mode_url_with_executor(&config, &success).expect("socket URL");
    assert_eq!(url, "wss://wss-primary.slack.test/link");
    let request = calls.lock().expect("HTTP calls").pop().expect("request");
    assert_eq!(
        request["url"],
        "https://slack.com/api/apps.connections.open"
    );
    assert_eq!(request["method"], "POST");
    assert_eq!(
        request["headers"]["Authorization"],
        "Bearer xapp-fixture-secret"
    );

    let rejected = StubHttpExecutor {
        calls: Arc::default(),
        result: json!({
            "ok": true,
            "status_code": 200,
            "payload": {"ok": false, "error": "invalid_auth"}
        }),
    };
    let error =
        acquire_socket_mode_url_with_executor(&config, &rejected).expect_err("auth rejection");
    assert_eq!(error.code, "slack_socket_mode_auth_failed");
    assert!(!error.render_json().contains("xapp-fixture-secret"));
}

#[test]
fn native_websocket_fixture_connects_and_assembles_fragmented_hello_with_ping_pong() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
    let address = listener.local_addr().expect("address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("accept");
        let request = read_http_headers(&mut stream);
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
        .expect("handshake response");
        stream
            .write_all(&server_frame(false, 0x1, br#"{"type":"hel"#))
            .expect("fragment start");
        stream
            .write_all(&server_frame(true, 0x0, br#"lo"}"#))
            .expect("fragment end");
        stream
            .write_all(&server_frame(true, 0x9, b"probe"))
            .expect("ping");
        stream.flush().expect("flush");
        let frame = read_client_frame(&mut stream);
        assert_eq!(frame.0, 0xA);
        assert_eq!(frame.1, b"probe");
    });

    let mut connection = NativeSlackSocketModeConnection::connect(&format!(
        "ws://127.0.0.1:{}/socket",
        address.port()
    ))
    .expect("native connection");
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut events = Vec::new();
    while events.is_empty() {
        events.extend(connection.read_events().expect("events"));
        assert!(Instant::now() < deadline, "fixture event timeout");
        thread::sleep(Duration::from_millis(1));
    }
    assert!(events.contains(&SlackSocketModeEvent::Text(
        json!({"type": "hello"}).to_string()
    )));
    connection.close();
    server.join().expect("server");
}

fn read_http_headers(stream: &mut TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("read timeout");
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 1024];
    while find_http_header_end(&bytes).is_none() {
        let count = stream.read(&mut buffer).expect("handshake request");
        assert!(count > 0, "handshake request EOF");
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8(bytes).expect("handshake UTF-8")
}

fn websocket_accept(key: &str) -> String {
    let input = format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    BASE64_STANDARD.encode(digest(&SHA1_FOR_LEGACY_USE_ONLY, input.as_bytes()).as_ref())
}

fn server_frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() < 126);
    let mut frame = vec![(if fin { 0x80 } else { 0 }) | opcode, payload.len() as u8];
    frame.extend_from_slice(payload);
    frame
}

fn read_client_frame(stream: &mut TcpStream) -> (u8, Vec<u8>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("read timeout");
    let mut header = [0u8; 2];
    stream.read_exact(&mut header).expect("frame header");
    assert_ne!(header[1] & 0x80, 0, "client frame must be masked");
    let length = usize::from(header[1] & 0x7f);
    assert!(length < 126);
    let mut mask = [0u8; 4];
    stream.read_exact(&mut mask).expect("mask");
    let mut payload = vec![0u8; length];
    stream.read_exact(&mut payload).expect("payload");
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask[index % 4];
    }
    (header[0] & 0x0f, payload)
}
