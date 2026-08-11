use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::thread;

use ait_agent_core::{
    agent_runtime_admission_plan_json, resolve_agent_worker_config, AgentEvent,
    AgentEventLoopBackend, AgentWorkerConfigInput, AgentWorkerRuntimeConfig, TransportKind,
};
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
    reads: VecDeque<Result<Vec<DiscordGatewayEvent>, &'static str>>,
    writes: Vec<JsonValue>,
    close_frames: Vec<(u16, String)>,
    closed: bool,
}

struct FakeConnection {
    fd: NativeSocket,
    state: Arc<Mutex<FakeConnectionState>>,
}

impl DiscordGatewayConnection for FakeConnection {
    fn raw_fd(&self) -> NativeSocket {
        self.fd
    }

    fn read_events(&mut self) -> Result<Vec<DiscordGatewayEvent>, WorkerDiagnostic> {
        match self
            .state
            .lock()
            .expect("connection state")
            .reads
            .pop_front()
            .unwrap_or_else(|| Ok(Vec::new()))
        {
            Ok(events) => Ok(events),
            Err(_) => Err(discord_gateway_read_failed()),
        }
    }

    fn send_json(&mut self, payload: &JsonValue) -> Result<(), WorkerDiagnostic> {
        self.state
            .lock()
            .expect("connection state")
            .writes
            .push(payload.clone());
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
    Error,
}

#[derive(Clone)]
struct FakeConnector {
    steps: Arc<Mutex<VecDeque<ConnectorStep>>>,
    urls: Arc<Mutex<Vec<String>>>,
}

impl FakeConnector {
    fn new(steps: Vec<ConnectorStep>) -> Self {
        Self {
            steps: Arc::new(Mutex::new(steps.into())),
            urls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl DiscordGatewayConnector for FakeConnector {
    fn connect(
        &mut self,
        gateway_url: &str,
    ) -> Result<Box<dyn DiscordGatewayConnection>, WorkerDiagnostic> {
        self.urls
            .lock()
            .expect("connector urls")
            .push(gateway_url.to_string());
        match self
            .steps
            .lock()
            .expect("connector steps")
            .pop_front()
            .expect("connector step")
        {
            ConnectorStep::Connection(fd, state) => Ok(Box::new(FakeConnection { fd, state })),
            ConnectorStep::Error => Err(discord_gateway_connect_failed()),
        }
    }
}

#[derive(Clone)]
struct FakeHttpExecutor {
    calls: Arc<Mutex<Vec<JsonValue>>>,
    gateway_url: String,
}

impl FakeHttpExecutor {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
            gateway_url: "ws://gateway.discord.test/socket".to_string(),
        }
    }
}

impl DiscordGatewayHttpExecutor for FakeHttpExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.lock().expect("HTTP calls").push(request.clone());
        Ok(json!({
            "ok": true,
            "payload": {"url": self.gateway_url},
        }))
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

impl DiscordGatewayClock for FakeClock {
    fn now(&self) -> Duration {
        *self.now.lock().expect("clock")
    }
}

#[derive(Clone)]
struct FakeInteractionExecutor {
    calls: Arc<Mutex<Vec<JsonValue>>>,
}

impl FakeInteractionExecutor {
    fn new() -> Self {
        Self {
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl DiscordHttpInteractionJobExecutor for FakeInteractionExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls
            .lock()
            .expect("interaction calls")
            .push(request.clone());
        Ok(ignored_job())
    }

    fn execute_delivery(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Err("unexpected delivery".to_string())
    }

    fn execute_state(
        &self,
        _path: &str,
        _operation: &str,
        _request: &JsonValue,
    ) -> Result<JsonValue, String> {
        Err("unexpected state".to_string())
    }

    fn execute_backend(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Err("unexpected backend".to_string())
    }
}

fn ignored_job() -> JsonValue {
    json!({
        "contract": "ait_agent_core.event_loop.DiscordInteractionJob.v1",
        "migration_stage": "rust_agent_discord_interaction_job_transaction",
        "interaction_job_state": "ignored",
        "ok": true,
        "processed": false,
        "duplicate": false,
        "session_id": null,
        "turn_ok": null,
        "recorded": false,
        "sequence": null,
        "response": null,
        "delivery_request": null,
        "recovery_request": null,
        "error_kind": null,
        "error": null,
        "python_interaction_execution_allowed": false,
    })
}

fn fixture() -> (TempDir, DiscordWorkerConfig, WorkerRunContext) {
    let temp = tempdir().expect("tempdir");
    std::fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    std::fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    let config = resolve_agent_worker_config(AgentWorkerConfigInput {
        repo_root: temp.path().to_path_buf(),
        worker_key: "discord/main".to_string(),
        worker: json!({
            "kind": "discord",
            "name": "main",
            "application_id": "123456789012345678",
            "bot_token": "discord-bot-token-secret",
            "api_base_url": "http://discord.test/api/v10",
        }),
        process_env: BTreeMap::new(),
    })
    .expect("Discord config");
    let AgentWorkerRuntimeConfig::Discord(discord) = config.clone() else {
        panic!("Discord config")
    };
    let context = WorkerRunContext {
        paths: ResolvedWorkerPaths {
            repo_root: temp.path().to_path_buf(),
            manifest_path: temp.path().join(".ait/agent-workers.json"),
        },
        transport: TransportKind::Discord,
        worker_key: "discord/main".to_string(),
        worker_name: "main".to_string(),
        event_loop_backend: AgentEventLoopBackend::PortablePoll,
        shard_index: 0,
        runtime_admission_plan: agent_runtime_admission_plan_json(&json!({
            "worker_manifest": {
                "version": 1,
                "workers": {"discord/main": {"kind": "discord", "name": "main"}}
            },
            "backend": "portable_poll",
            "transport_runtime": "rust",
            "allow_python_fallback": false,
            "requested_worker_keys": ["discord/main"],
        }))
        .expect("admission"),
        config,
    };
    (temp, discord, context)
}

fn readable_event() -> AgentEvent {
    AgentEvent {
        token: DISCORD_GATEWAY_EVENT_LOOP_TOKEN,
        readable: true,
        writable: false,
        hangup: false,
    }
}

fn hello(interval_ms: i64) -> String {
    json!({"op": 10, "d": {"heartbeat_interval": interval_ms}}).to_string()
}

fn ready(sequence: i64) -> String {
    json!({
        "op": 0,
        "s": sequence,
        "t": "READY",
        "d": {
            "session_id": "discord-session-1",
            "resume_gateway_url": "ws://resume.discord.test/socket",
            "user": {"id": "discord-bot-user", "bot": true},
        },
    })
    .to_string()
}

fn message(message_id: &str, author_id: &str, bot: bool) -> String {
    json!({
        "op": 0,
        "s": 2,
        "t": "MESSAGE_CREATE",
        "d": {
            "id": message_id,
            "type": 0,
            "channel_id": "discord-channel-1",
            "guild_id": "discord-guild-1",
            "content": "hello from Gateway",
            "author": {
                "id": author_id,
                "username": "weita",
                "bot": bot,
            },
            "attachments": [{
                "id": "attachment-1",
                "filename": "question.txt",
                "content_type": "text/plain",
                "size": 42,
                "url": "https://cdn.discord.test/question.txt",
            }],
        },
    })
    .to_string()
}

type TestRuntime = DiscordGatewayWorkerRuntime<
    FakeConnector,
    FakeInteractionExecutor,
    FakeHttpExecutor,
    FakeClock,
>;

fn make_runtime(
    config: DiscordWorkerConfig,
    connector: FakeConnector,
    executor: FakeInteractionExecutor,
    http: FakeHttpExecutor,
    clock: FakeClock,
    settings: DiscordGatewaySettings,
) -> TestRuntime {
    DiscordGatewayWorkerRuntime::new(config, connector, executor, http, clock, settings)
        .expect("Gateway runtime")
}

#[test]
fn gateway_discovers_endpoint_and_identifies_after_hello() {
    let (_temp, config, context) = fixture();
    let connection = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![DiscordGatewayEvent::Text(hello(1_000))])]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![ConnectorStep::Connection(41, connection.clone())]);
    let connector_probe = connector.clone();
    let http = FakeHttpExecutor::new();
    let http_probe = http.clone();
    let clock = FakeClock::new();
    let mut runtime = make_runtime(
        config,
        connector,
        FakeInteractionExecutor::new(),
        http,
        clock,
        DiscordGatewaySettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();

    runtime.start(&context, &mut event_loop).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();

    assert_eq!(runtime.runtime_state(), "identifying");
    assert_eq!(
        connector_probe.urls.lock().unwrap().as_slice(),
        ["ws://gateway.discord.test/socket?v=10&encoding=json"]
    );
    let calls = http_probe.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]["headers"]["Authorization"],
        "Bot discord-bot-token-secret"
    );
    drop(calls);
    let state = connection.lock().unwrap();
    assert_eq!(state.writes.len(), 1);
    assert_eq!(state.writes[0]["op"], 2);
    assert_eq!(state.writes[0]["d"]["token"], "discord-bot-token-secret");
    assert_eq!(state.writes[0]["d"]["intents"], DEFAULT_GATEWAY_INTENTS);
    assert_eq!(
        event_loop.registered,
        [(DISCORD_GATEWAY_EVENT_LOOP_TOKEN, 41)]
    );
}

#[test]
fn ready_message_dispatch_is_bounded_deduplicated_and_filters_bot_messages() {
    let (_temp, config, context) = fixture();
    let connection = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![
            DiscordGatewayEvent::Text(hello(1_000)),
            DiscordGatewayEvent::Text(ready(1)),
            DiscordGatewayEvent::Text(message("message-1", "discord-user-1", false)),
            DiscordGatewayEvent::Text(message("message-1", "discord-user-1", false)),
            DiscordGatewayEvent::Text(message("message-2", "discord-bot-user", true)),
        ])]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![ConnectorStep::Connection(42, connection)]);
    let executor = FakeInteractionExecutor::new();
    let executor_probe = executor.clone();
    let mut runtime = make_runtime(
        config,
        connector,
        executor,
        FakeHttpExecutor::new(),
        FakeClock::new(),
        DiscordGatewaySettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();

    runtime.start(&context, &mut event_loop).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();
    for _ in 0..100 {
        if executor_probe.calls.lock().unwrap().len() == 1 {
            break;
        }
        thread::sleep(Duration::from_millis(1));
    }

    assert_eq!(runtime.runtime_state(), "ready");
    assert_eq!(runtime.session_id(), Some("discord-session-1"));
    assert_eq!(
        runtime.resume_gateway_url(),
        Some("ws://resume.discord.test/socket")
    );
    assert_eq!(runtime.sequence(), Some(2));
    let calls = executor_probe.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["message_payload"]["id"], "message-1");
    assert_eq!(
        calls[0]["message_payload"]["attachments"][0]["filename"],
        "question.txt"
    );
}

#[test]
fn heartbeat_timeout_reconnects_and_resumes_the_ready_session() {
    let (_temp, config, context) = fixture();
    let first = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![
            DiscordGatewayEvent::Text(hello(100)),
            DiscordGatewayEvent::Text(ready(7)),
        ])]),
        ..FakeConnectionState::default()
    }));
    let second = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![DiscordGatewayEvent::Text(hello(100))])]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![
        ConnectorStep::Connection(43, first.clone()),
        ConnectorStep::Connection(44, second.clone()),
    ]);
    let http = FakeHttpExecutor::new();
    let http_probe = http.clone();
    let clock = FakeClock::new();
    let settings = DiscordGatewaySettings {
        reconnect_base_delay: Duration::from_millis(10),
        ..DiscordGatewaySettings::default()
    };
    let mut runtime = make_runtime(
        config,
        connector,
        FakeInteractionExecutor::new(),
        http,
        clock.clone(),
        settings,
    );
    let mut event_loop = FakeEventLoop::default();

    runtime.start(&context, &mut event_loop).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();
    clock.advance(Duration::from_millis(100));
    runtime.tick(&context, &mut event_loop, &[]).unwrap();
    assert_eq!(first.lock().unwrap().writes.last().unwrap()["op"], 1);
    clock.advance(Duration::from_millis(100));
    runtime.tick(&context, &mut event_loop, &[]).unwrap();
    assert_eq!(runtime.runtime_state(), "reconnect_wait");
    assert_eq!(runtime.reconnect_attempt(), 1);
    clock.advance(Duration::from_millis(10));
    runtime.tick(&context, &mut event_loop, &[]).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();

    let state = second.lock().unwrap();
    assert_eq!(state.writes[0]["op"], 6);
    assert_eq!(state.writes[0]["d"]["session_id"], "discord-session-1");
    assert_eq!(state.writes[0]["d"]["seq"], 7);
    assert_eq!(http_probe.calls.lock().unwrap().len(), 1);
}

#[test]
fn server_heartbeat_request_is_answered_and_ack_restores_ready_health() {
    let (_temp, config, context) = fixture();
    let connection = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([
            Ok(vec![
                DiscordGatewayEvent::Text(hello(1_000)),
                DiscordGatewayEvent::Text(ready(11)),
            ]),
            Ok(vec![DiscordGatewayEvent::Text(
                json!({"op": 1, "d": null}).to_string(),
            )]),
            Ok(vec![DiscordGatewayEvent::Text(
                json!({"op": 11, "d": null}).to_string(),
            )]),
        ]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![ConnectorStep::Connection(51, connection.clone())]);
    let mut runtime = make_runtime(
        config,
        connector,
        FakeInteractionExecutor::new(),
        FakeHttpExecutor::new(),
        FakeClock::new(),
        DiscordGatewaySettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();

    runtime.start(&context, &mut event_loop).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();
    {
        let state = connection.lock().unwrap();
        assert_eq!(state.writes.last().unwrap()["op"], 1);
        assert_eq!(state.writes.last().unwrap()["d"], 11);
    }
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();

    assert_eq!(runtime.runtime_state(), "ready");
    assert_eq!(runtime.reconnect_attempt(), 0);
}

#[test]
fn invalid_session_reset_identifies_on_the_next_connection() {
    let (_temp, config, context) = fixture();
    let first = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([
            Ok(vec![
                DiscordGatewayEvent::Text(hello(1_000)),
                DiscordGatewayEvent::Text(ready(9)),
            ]),
            Ok(vec![DiscordGatewayEvent::Text(
                json!({"op": 9, "d": false}).to_string(),
            )]),
        ]),
        ..FakeConnectionState::default()
    }));
    let second = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![DiscordGatewayEvent::Text(hello(1_000))])]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![
        ConnectorStep::Connection(45, first),
        ConnectorStep::Connection(46, second.clone()),
    ]);
    let clock = FakeClock::new();
    let mut runtime = make_runtime(
        config,
        connector,
        FakeInteractionExecutor::new(),
        FakeHttpExecutor::new(),
        clock.clone(),
        DiscordGatewaySettings {
            reconnect_base_delay: Duration::from_millis(5),
            ..DiscordGatewaySettings::default()
        },
    );
    let mut event_loop = FakeEventLoop::default();

    runtime.start(&context, &mut event_loop).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();
    assert_eq!(runtime.session_id(), None);
    clock.advance(Duration::from_millis(5));
    runtime.tick(&context, &mut event_loop, &[]).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();

    assert_eq!(second.lock().unwrap().writes[0]["op"], 2);
}

#[test]
fn close_codes_fail_closed_or_drop_only_message_content_intent() {
    let (_temp, config, context) = fixture();
    let disallowed = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![
            DiscordGatewayEvent::Text(hello(1_000)),
            DiscordGatewayEvent::Closed {
                status_code: Some(4014),
                reason: Some("Disallowed intent(s)".to_string()),
            },
        ])]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![ConnectorStep::Connection(47, disallowed)]);
    let mut runtime = make_runtime(
        config.clone(),
        connector,
        FakeInteractionExecutor::new(),
        FakeHttpExecutor::new(),
        FakeClock::new(),
        DiscordGatewaySettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();
    assert_eq!(
        runtime.gateway_intents() & DISCORD_MESSAGE_CONTENT_INTENT,
        0
    );
    assert_eq!(runtime.runtime_state(), "reconnect_wait");

    let fatal = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![
            DiscordGatewayEvent::Text(hello(1_000)),
            DiscordGatewayEvent::Closed {
                status_code: Some(4004),
                reason: Some("authentication failed".to_string()),
            },
        ])]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![ConnectorStep::Connection(48, fatal)]);
    let mut runtime = make_runtime(
        config,
        connector,
        FakeInteractionExecutor::new(),
        FakeHttpExecutor::new(),
        FakeClock::new(),
        DiscordGatewaySettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).unwrap();
    let error = runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap_err();
    assert_eq!(error.code, "discord_gateway_close_fatal");
    assert_eq!(error.exit_code, EXIT_INVALID_CONFIGURATION);
}

#[test]
fn malformed_payload_exhausts_bounded_retries_and_shutdown_sends_close() {
    let (_temp, config, context) = fixture();
    let malformed = Arc::new(Mutex::new(FakeConnectionState {
        reads: VecDeque::from([Ok(vec![DiscordGatewayEvent::Text("{".to_string())])]),
        ..FakeConnectionState::default()
    }));
    let connector = FakeConnector::new(vec![
        ConnectorStep::Connection(49, malformed),
        ConnectorStep::Error,
        ConnectorStep::Error,
    ]);
    let clock = FakeClock::new();
    let mut runtime = make_runtime(
        config.clone(),
        connector,
        FakeInteractionExecutor::new(),
        FakeHttpExecutor::new(),
        clock.clone(),
        DiscordGatewaySettings {
            max_reconnect_attempts: 2,
            reconnect_base_delay: Duration::from_millis(5),
            ..DiscordGatewaySettings::default()
        },
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).unwrap();
    runtime
        .tick(&context, &mut event_loop, &[readable_event()])
        .unwrap();
    clock.advance(Duration::from_millis(5));
    runtime.tick(&context, &mut event_loop, &[]).unwrap();
    clock.advance(Duration::from_millis(10));
    let error = runtime.tick(&context, &mut event_loop, &[]).unwrap_err();
    assert_eq!(error.code, "discord_gateway_reconnect_exhausted");

    let shutdown_connection = Arc::new(Mutex::new(FakeConnectionState::default()));
    let connector = FakeConnector::new(vec![ConnectorStep::Connection(
        50,
        shutdown_connection.clone(),
    )]);
    let mut runtime = make_runtime(
        config,
        connector,
        FakeInteractionExecutor::new(),
        FakeHttpExecutor::new(),
        FakeClock::new(),
        DiscordGatewaySettings::default(),
    );
    let mut event_loop = FakeEventLoop::default();
    runtime.start(&context, &mut event_loop).unwrap();
    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .unwrap();
    runtime.finish_shutdown(&context, &mut event_loop).unwrap();
    let state = shutdown_connection.lock().unwrap();
    assert_eq!(state.close_frames, [(1000, "worker shutdown".to_string())]);
    assert!(state.closed);
    assert_eq!(runtime.runtime_state(), "stopped");
}
