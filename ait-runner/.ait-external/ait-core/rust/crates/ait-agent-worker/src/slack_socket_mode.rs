use std::collections::{HashSet, VecDeque};
use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::thread;
use std::time::{Duration, Instant};

use ait_agent_core::{
    agent_slack_socket_mode_runtime_plan_json, agent_transport_http_execute_json_request_json,
    agent_transport_websocket_fd_io_execute_json, agent_transport_websocket_frame_plan_json,
    agent_transport_websocket_handshake_plan_json, agent_transport_websocket_stream_plan_json,
    agent_transport_websocket_tls_execute_json, close_native_socket, native_socket_is_valid,
    set_native_socket_close_on_exec, tcp_stream_into_native_socket, tcp_stream_native_socket,
    NativeSocket, SlackWorkerConfig, INVALID_NATIVE_SOCKET,
};
use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonValue};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use ring::rand::{SecureRandom, SystemRandom};

use crate::slack_command_once::{command_job_request, validate_command_job_contract};
use crate::slack_runner::{DefaultSlackHttpCommandJobExecutor, SlackHttpCommandJobExecutor};
use crate::{
    run_worker_host, BoundedWorkerJobExecutor, WorkerDiagnostic, WorkerHostEventLoop,
    WorkerHostRuntime, WorkerJobExecutorConfig, WorkerRunContext, EXIT_INVALID_CONFIGURATION,
    EXIT_RUNTIME_UNAVAILABLE,
};

const SLACK_SOCKET_EVENT_LOOP_TOKEN: u64 = 0x0053_4c41_434b;
const SLACK_SOCKET_COMMAND_JOB_KIND: &str = "slack.socket_mode_command_job";
const MAX_HANDSHAKE_BYTES: usize = 64 * 1024;
const MAX_SOCKET_READ_BYTES: usize = 1024 * 1024;
const READ_CHUNK_BYTES: usize = 16 * 1024;
const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
const MAX_RECENT_ENVELOPE_IDS: usize = 256;
const DEFAULT_MAX_INFLIGHT_JOBS: usize = 4;
const DEFAULT_MAX_RECONNECT_ATTEMPTS: usize = 8;
const DEFAULT_RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);
const DEFAULT_PING_INTERVAL: Duration = Duration::from_secs(30);
const DEFAULT_PONG_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(20);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlackSocketModeEvent {
    Text(String),
    Binary(Vec<u8>),
    Pong,
    Closed,
}

pub trait SlackSocketModeConnection {
    fn raw_fd(&self) -> NativeSocket;
    fn has_buffered_input(&self) -> bool {
        false
    }
    fn read_events(&mut self) -> Result<Vec<SlackSocketModeEvent>, WorkerDiagnostic>;
    fn send_json(&mut self, payload: &JsonValue) -> Result<(), WorkerDiagnostic>;
    fn send_ping(&mut self) -> Result<(), WorkerDiagnostic>;
    fn send_close(&mut self, status_code: u16, reason: &str) -> Result<(), WorkerDiagnostic>;
    fn take_close_info(&mut self) -> (Option<u16>, Option<String>) {
        (None, None)
    }
    fn close(&mut self);
}

pub trait SlackSocketModeConnector {
    fn connect(
        &mut self,
        config: &SlackWorkerConfig,
    ) -> Result<Box<dyn SlackSocketModeConnection>, WorkerDiagnostic>;
}

trait SlackSocketModeHttpExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
struct DefaultSlackSocketModeHttpExecutor;

impl SlackSocketModeHttpExecutor for DefaultSlackSocketModeHttpExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_transport_http_execute_json_request_json(request)
    }
}

pub trait SlackSocketModeClock {
    fn now(&self) -> Duration;
}

#[derive(Debug)]
pub struct SystemSlackSocketModeClock {
    started_at: Instant,
}

impl SystemSlackSocketModeClock {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

impl Default for SystemSlackSocketModeClock {
    fn default() -> Self {
        Self::new()
    }
}

impl SlackSocketModeClock for SystemSlackSocketModeClock {
    fn now(&self) -> Duration {
        self.started_at.elapsed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlackSocketModeSettings {
    pub max_inflight_jobs: usize,
    pub max_reconnect_attempts: usize,
    pub reconnect_base_delay: Duration,
    pub ping_interval: Duration,
    pub pong_timeout: Duration,
}

impl Default for SlackSocketModeSettings {
    fn default() -> Self {
        Self {
            max_inflight_jobs: DEFAULT_MAX_INFLIGHT_JOBS,
            max_reconnect_attempts: DEFAULT_MAX_RECONNECT_ATTEMPTS,
            reconnect_base_delay: DEFAULT_RECONNECT_BASE_DELAY,
            ping_interval: DEFAULT_PING_INTERVAL,
            pong_timeout: DEFAULT_PONG_TIMEOUT,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackSocketModeConnector;

impl SlackSocketModeConnector for DefaultSlackSocketModeConnector {
    fn connect(
        &mut self,
        config: &SlackWorkerConfig,
    ) -> Result<Box<dyn SlackSocketModeConnection>, WorkerDiagnostic> {
        let socket_url = acquire_socket_mode_url(config)?;
        NativeSlackSocketModeConnection::connect(&socket_url)
            .map(|connection| Box::new(connection) as Box<dyn SlackSocketModeConnection>)
    }
}

pub struct SlackSocketModeWorkerRuntime<C, E, K> {
    config: SlackWorkerConfig,
    connector: C,
    command_executor: E,
    clock: K,
    settings: SlackSocketModeSettings,
    jobs: BoundedWorkerJobExecutor<()>,
    connection: Option<Box<dyn SlackSocketModeConnection>>,
    reconnect_attempt: usize,
    reconnect_at: Option<Duration>,
    next_ping_at: Duration,
    ping_sent_at: Option<Duration>,
    stopping: bool,
    runtime_state: &'static str,
    health_generation: u64,
    last_diagnostic_code: Option<&'static str>,
    recent_envelope_ids: VecDeque<String>,
    recent_envelope_set: HashSet<String>,
}

impl<C, E, K> SlackSocketModeWorkerRuntime<C, E, K>
where
    C: SlackSocketModeConnector,
    E: SlackHttpCommandJobExecutor,
    K: SlackSocketModeClock,
{
    pub fn new(
        config: SlackWorkerConfig,
        connector: C,
        command_executor: E,
        clock: K,
        settings: SlackSocketModeSettings,
    ) -> Result<Self, WorkerDiagnostic> {
        if settings.max_reconnect_attempts == 0
            || settings.reconnect_base_delay.is_zero()
            || settings.ping_interval.is_zero()
            || settings.pong_timeout.is_zero()
        {
            return Err(WorkerDiagnostic::new(
                "slack_socket_mode_settings_invalid",
                "Rust Slack Socket Mode runtime settings must be positive.",
                EXIT_INVALID_CONFIGURATION,
            ));
        }
        let jobs = BoundedWorkerJobExecutor::new(WorkerJobExecutorConfig {
            max_inflight: settings.max_inflight_jobs,
        })?;
        Ok(Self {
            config,
            connector,
            command_executor,
            clock,
            settings,
            jobs,
            connection: None,
            reconnect_attempt: 0,
            reconnect_at: None,
            next_ping_at: Duration::ZERO,
            ping_sent_at: None,
            stopping: false,
            runtime_state: "starting",
            health_generation: 1,
            last_diagnostic_code: None,
            recent_envelope_ids: VecDeque::new(),
            recent_envelope_set: HashSet::new(),
        })
    }

    pub fn runtime_state(&self) -> &'static str {
        self.runtime_state
    }

    pub fn reconnect_attempt(&self) -> usize {
        self.reconnect_attempt
    }

    pub fn health_generation(&self) -> u64 {
        self.health_generation
    }

    pub fn last_diagnostic_code(&self) -> Option<&'static str> {
        self.last_diagnostic_code
    }

    fn set_health(&mut self, state: &'static str, diagnostic_code: Option<&'static str>) {
        if self.runtime_state != state || self.last_diagnostic_code != diagnostic_code {
            self.runtime_state = state;
            self.last_diagnostic_code = diagnostic_code;
            self.health_generation = self.health_generation.saturating_add(1);
        }
    }

    fn connect_now(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let connection = self.connector.connect(&self.config)?;
        event_loop.register_readable(SLACK_SOCKET_EVENT_LOOP_TOKEN, connection.raw_fd())?;
        let now = self.clock.now();
        self.connection = Some(connection);
        self.reconnect_attempt = 0;
        self.reconnect_at = None;
        self.next_ping_at = now.saturating_add(self.settings.ping_interval);
        self.ping_sent_at = None;
        self.set_health("awaiting_hello", None);
        Ok(())
    }

    fn disconnect(&mut self, event_loop: &mut dyn WorkerHostEventLoop) {
        if let Some(mut connection) = self.connection.take() {
            let _ = event_loop.unregister(SLACK_SOCKET_EVENT_LOOP_TOKEN);
            connection.close();
        }
        self.ping_sent_at = None;
    }

    fn schedule_reconnect(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
        reason: &'static str,
    ) -> Result<(), WorkerDiagnostic> {
        self.disconnect(event_loop);
        self.reconnect_attempt = self.reconnect_attempt.saturating_add(1);
        if self.reconnect_attempt > self.settings.max_reconnect_attempts {
            self.set_health(
                "reconnect_exhausted",
                Some("slack_socket_mode_reconnect_exhausted"),
            );
            return Err(WorkerDiagnostic::new(
                "slack_socket_mode_reconnect_exhausted",
                "Rust Slack Socket Mode exhausted its bounded reconnect attempts.",
                EXIT_RUNTIME_UNAVAILABLE,
            )
            .with_detail(
                "max_reconnect_attempts",
                self.settings.max_reconnect_attempts,
            ));
        }
        let exponent = u32::try_from(self.reconnect_attempt.saturating_sub(1).min(8)).unwrap_or(8);
        let multiplier = 1u32 << exponent;
        let delay = self
            .settings
            .reconnect_base_delay
            .saturating_mul(multiplier)
            .min(Duration::from_secs(60));
        self.reconnect_at = Some(self.clock.now().saturating_add(delay));
        self.set_health("reconnect_wait", Some(reason));
        Ok(())
    }

    fn maybe_reconnect(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let Some(reconnect_at) = self.reconnect_at else {
            return Ok(());
        };
        if self.clock.now() < reconnect_at {
            return Ok(());
        }
        self.set_health("reconnecting", None);
        match self.connector.connect(&self.config) {
            Ok(connection) => {
                event_loop.register_readable(SLACK_SOCKET_EVENT_LOOP_TOKEN, connection.raw_fd())?;
                let now = self.clock.now();
                self.connection = Some(connection);
                self.reconnect_at = None;
                self.next_ping_at = now.saturating_add(self.settings.ping_interval);
                self.ping_sent_at = None;
                self.set_health("awaiting_hello", Some("slack_socket_mode_reconnected"));
                Ok(())
            }
            Err(error) if error.code == "slack_socket_mode_auth_failed" => Err(error),
            Err(_) => self.schedule_reconnect(event_loop, "slack_socket_mode_reconnect_failed"),
        }
    }

    fn handle_heartbeat(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        if self.connection.is_none() {
            return Ok(());
        }
        let now = self.clock.now();
        let plan = agent_slack_socket_mode_runtime_plan_json(&json!({
            "stage": "tick",
            "now_monotonic_seconds": now.as_secs_f64(),
            "next_ping_at": self.next_ping_at.as_secs_f64(),
            "pong_pending": self.ping_sent_at.is_some(),
            "ping_sent_at": self.ping_sent_at.unwrap_or(now).as_secs_f64(),
            "ping_interval_seconds": self.settings.ping_interval.as_secs_f64(),
            "pong_timeout_seconds": self.settings.pong_timeout.as_secs_f64(),
        }))
        .map_err(|_| slack_socket_contract_invalid())?;
        validate_runtime_plan(&plan)?;
        if plan.get("should_reconnect").and_then(JsonValue::as_bool) == Some(true) {
            return self.schedule_reconnect(event_loop, "slack_socket_mode_pong_timeout");
        }
        if plan.get("should_send_ping").and_then(JsonValue::as_bool) == Some(true) {
            self.connection
                .as_mut()
                .ok_or_else(slack_socket_connection_missing)?
                .send_ping()?;
            self.ping_sent_at = Some(now);
            self.next_ping_at = now.saturating_add(self.settings.ping_interval);
            self.set_health("awaiting_pong", None);
        }
        Ok(())
    }

    fn process_text(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
        text: &str,
    ) -> Result<(), WorkerDiagnostic> {
        let payload = JsonCodec::parse_value_with_error_prefix(
            text,
            "Invalid Slack Socket Mode JSON payload",
        )
        .map_err(|_| slack_socket_payload_invalid())?;
        let plan = agent_slack_socket_mode_runtime_plan_json(&json!({
            "stage": "payload",
            "payload": payload,
            "repo_name": self.config.shared.runtime_target.repo_name,
            "defer_replies": true,
        }))
        .map_err(|_| slack_socket_transaction_invalid())?;
        validate_runtime_plan(&plan)?;
        match clean_text(plan.get("socket_mode_runtime_state")).as_deref() {
            Some("ready") => {
                self.reconnect_attempt = 0;
                self.set_health("ready", None);
                return Ok(());
            }
            Some("pong_acknowledged") => {
                self.ping_sent_at = None;
                self.set_health("ready", None);
                return Ok(());
            }
            Some("disconnect_requested") => {
                return self
                    .schedule_reconnect(event_loop, "slack_socket_mode_disconnect_requested");
            }
            _ => {}
        }

        if plan
            .get("should_execute_websocket_ack")
            .and_then(JsonValue::as_bool)
            == Some(true)
        {
            let ack = plan
                .get("websocket_ack_response")
                .filter(|value| value.is_object())
                .ok_or_else(slack_socket_transaction_invalid)?
                .clone();
            let connection = self
                .connection
                .as_mut()
                .ok_or_else(slack_socket_connection_missing)?;
            connection.send_json(&ack)?;
        }

        if plan
            .get("should_handle_command")
            .and_then(JsonValue::as_bool)
            != Some(true)
            || plan.get("should_submit_turn").and_then(JsonValue::as_bool) != Some(true)
        {
            return Ok(());
        }
        let envelope_id =
            clean_text(payload.get("envelope_id")).ok_or_else(slack_socket_transaction_invalid)?;
        if !self.remember_envelope(envelope_id) {
            return Ok(());
        }
        let command_payload = payload
            .get("payload")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(slack_socket_transaction_invalid)?;
        let request = command_job_request(&self.config, command_payload);
        let executor = self.command_executor.clone();
        match self.jobs.submit(SLACK_SOCKET_COMMAND_JOB_KIND, move || {
            let result = executor.execute(&request).map_err(|_| {
                WorkerDiagnostic::new(
                    "slack_socket_mode_command_job_failed",
                    "Rust Slack Socket Mode command execution failed.",
                    EXIT_RUNTIME_UNAVAILABLE,
                )
            })?;
            validate_command_job_contract(&result)?;
            Ok(())
        }) {
            Ok(_) => {}
            Err(error)
                if matches!(
                    error.code,
                    "worker_job_capacity_exhausted" | "worker_job_executor_closed"
                ) =>
            {
                self.set_health(self.runtime_state, Some(error.code));
            }
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn remember_envelope(&mut self, envelope_id: String) -> bool {
        if self.recent_envelope_set.contains(&envelope_id) {
            return false;
        }
        self.recent_envelope_set.insert(envelope_id.clone());
        self.recent_envelope_ids.push_back(envelope_id);
        while self.recent_envelope_ids.len() > MAX_RECENT_ENVELOPE_IDS {
            if let Some(expired) = self.recent_envelope_ids.pop_front() {
                self.recent_envelope_set.remove(&expired);
            }
        }
        true
    }

    fn reap_jobs(&mut self) {
        let failed = self
            .jobs
            .poll_completed()
            .into_iter()
            .any(|completion| completion.result.is_err());
        if failed {
            self.set_health(
                self.runtime_state,
                Some("slack_socket_mode_command_job_failed"),
            );
        }
    }
}

impl<C, E, K> WorkerHostRuntime for SlackSocketModeWorkerRuntime<C, E, K>
where
    C: SlackSocketModeConnector,
    E: SlackHttpCommandJobExecutor,
    K: SlackSocketModeClock,
{
    fn start(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        self.connect_now(event_loop)
    }

    fn tick(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
        events: &[ait_agent_core::AgentEvent],
    ) -> Result<(), WorkerDiagnostic> {
        self.reap_jobs();
        if self.stopping {
            return Ok(());
        }
        self.maybe_reconnect(event_loop)?;
        if self.connection.is_none() {
            return Ok(());
        }
        let socket_event = events
            .iter()
            .find(|event| event.token == SLACK_SOCKET_EVENT_LOOP_TOKEN);
        if socket_event.is_some_and(|event| event.hangup) {
            return self.schedule_reconnect(event_loop, "slack_socket_mode_hangup");
        }
        let has_buffered_input = self
            .connection
            .as_ref()
            .is_some_and(|connection| connection.has_buffered_input());
        if has_buffered_input || socket_event.is_some_and(|event| event.readable) {
            let socket_events = self
                .connection
                .as_mut()
                .ok_or_else(slack_socket_connection_missing)?
                .read_events();
            let socket_events = match socket_events {
                Ok(events) => events,
                Err(_) => {
                    return self.schedule_reconnect(event_loop, "slack_socket_mode_read_failed")
                }
            };
            for event in socket_events {
                match event {
                    SlackSocketModeEvent::Text(text) => {
                        if self.process_text(event_loop, &text).is_err() {
                            return self.schedule_reconnect(
                                event_loop,
                                "slack_socket_mode_payload_invalid",
                            );
                        }
                        if self.connection.is_none() {
                            return Ok(());
                        }
                    }
                    SlackSocketModeEvent::Binary(_) => {
                        return self
                            .schedule_reconnect(event_loop, "slack_socket_mode_binary_unsupported")
                    }
                    SlackSocketModeEvent::Pong => {
                        self.ping_sent_at = None;
                        self.set_health("ready", None);
                    }
                    SlackSocketModeEvent::Closed => {
                        return self.schedule_reconnect(event_loop, "slack_socket_mode_peer_closed")
                    }
                }
            }
        }
        self.handle_heartbeat(event_loop)
    }

    fn request_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
        _signal: i32,
    ) -> Result<(), WorkerDiagnostic> {
        self.stopping = true;
        self.reconnect_at = None;
        self.jobs.close_admission();
        if let Some(connection) = self.connection.as_mut() {
            let _ = connection.send_close(1000, "worker shutdown");
        }
        self.disconnect(event_loop);
        self.set_health("stopping", None);
        Ok(())
    }

    fn inflight_work_count(&self) -> usize {
        self.jobs.inflight_count()
    }

    fn finish_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        _event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        if self.jobs.inflight_count() != 0 {
            return Err(WorkerDiagnostic::new(
                "slack_socket_mode_jobs_still_inflight",
                "Rust Slack Socket Mode jobs remain in flight during graceful shutdown.",
                EXIT_RUNTIME_UNAVAILABLE,
            ));
        }
        self.set_health("stopped", None);
        Ok(())
    }

    fn force_shutdown(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        self.stopping = true;
        self.reconnect_at = None;
        self.jobs.close_admission();
        self.jobs.force_detach();
        self.disconnect(event_loop);
        self.set_health("stopped", Some("slack_socket_mode_forced_shutdown"));
        Ok(())
    }

    fn runtime_health_generation(&self) -> u64 {
        self.health_generation
    }

    fn runtime_health_state(&self) -> Option<&str> {
        Some(self.runtime_state)
    }

    fn runtime_reconnect_attempt(&self) -> Option<usize> {
        Some(self.reconnect_attempt)
    }

    fn runtime_diagnostic_code(&self) -> Option<&str> {
        self.last_diagnostic_code
    }
}

pub fn run_slack_socket_mode_transport(
    context: &WorkerRunContext,
    config: &SlackWorkerConfig,
) -> Result<(), WorkerDiagnostic> {
    if config.app_token.is_none() {
        return Err(WorkerDiagnostic::new(
            "slack_socket_mode_app_token_missing",
            "The Rust Slack Socket Mode worker requires an app token.",
            EXIT_INVALID_CONFIGURATION,
        ));
    }
    let mut runtime = SlackSocketModeWorkerRuntime::new(
        config.clone(),
        DefaultSlackSocketModeConnector,
        DefaultSlackHttpCommandJobExecutor,
        SystemSlackSocketModeClock::new(),
        SlackSocketModeSettings::default(),
    )?;
    run_worker_host(context, &mut runtime)
}

fn acquire_socket_mode_url(config: &SlackWorkerConfig) -> Result<String, WorkerDiagnostic> {
    acquire_socket_mode_url_with_executor(config, &DefaultSlackSocketModeHttpExecutor)
}

fn acquire_socket_mode_url_with_executor<E>(
    config: &SlackWorkerConfig,
    executor: &E,
) -> Result<String, WorkerDiagnostic>
where
    E: SlackSocketModeHttpExecutor + ?Sized,
{
    let app_token = config
        .app_token
        .as_ref()
        .map(|token| token.expose().trim())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| {
            WorkerDiagnostic::new(
                "slack_socket_mode_app_token_missing",
                "The Rust Slack Socket Mode worker requires an app token.",
                EXIT_INVALID_CONFIGURATION,
            )
        })?;
    let planned = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "connection_open_request",
        "app_token": app_token,
        "slack_api_base_url": config.api_base_url,
        "slack_http_user_agent": config.http_user_agent,
        "request_timeout_seconds": config.shared.request_timeout_seconds.map(JsonValue::from).unwrap_or(JsonValue::Null),
    }))
    .map_err(|_| slack_socket_connection_open_failed())?;
    validate_runtime_plan(&planned)?;
    let request = planned
        .get("request")
        .filter(|value| value.is_object())
        .ok_or_else(slack_socket_connection_open_failed)?;
    let executed = executor
        .execute(request)
        .map_err(|_| slack_socket_connection_open_failed())?;
    if executed.get("ok").and_then(JsonValue::as_bool) != Some(true) {
        return Err(slack_socket_connection_open_failed());
    }
    let payload = executed
        .get("payload")
        .and_then(JsonValue::as_object)
        .ok_or_else(slack_socket_connection_open_failed)?;
    if payload.get("ok").and_then(JsonValue::as_bool) != Some(true) {
        let marker = clean_text(payload.get("error")).unwrap_or_default();
        if matches!(
            marker.as_str(),
            "invalid_auth" | "not_authed" | "account_inactive" | "token_revoked"
        ) {
            return Err(WorkerDiagnostic::new(
                "slack_socket_mode_auth_failed",
                "Slack rejected the Socket Mode app-token authentication.",
                EXIT_INVALID_CONFIGURATION,
            )
            .with_detail("slack_error", marker));
        }
        return Err(slack_socket_connection_open_failed()
            .with_detail("slack_error", safe_slack_error_marker(&marker)));
    }
    clean_text(payload.get("url"))
        .filter(|url| url.starts_with("wss://") || url.starts_with("ws://"))
        .ok_or_else(slack_socket_connection_open_failed)
}

struct NativeSlackSocketModeConnection {
    fd: NativeSocket,
    tls_session_id: Option<String>,
    read_buffer: Vec<u8>,
    fragment_opcode: Option<String>,
    fragment_payload: Vec<u8>,
    close_status_code: Option<u16>,
    close_reason: Option<String>,
    closed: bool,
}

pub(crate) fn connect_native_worker_websocket(
    url: &str,
) -> Result<Box<dyn SlackSocketModeConnection>, WorkerDiagnostic> {
    NativeSlackSocketModeConnection::connect(url)
        .map(|connection| Box::new(connection) as Box<dyn SlackSocketModeConnection>)
}

impl NativeSlackSocketModeConnection {
    fn connect(url: &str) -> Result<Self, WorkerDiagnostic> {
        let sec_websocket_key = random_websocket_key()?;
        let handshake = agent_transport_websocket_handshake_plan_json(&json!({
            "stage": "upgrade_request",
            "url": url,
            "sec_websocket_key": sec_websocket_key,
        }))
        .map_err(|_| slack_socket_handshake_failed())?;
        if handshake.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            return Err(slack_socket_handshake_failed());
        }
        let host = clean_text(handshake.get("host")).ok_or_else(slack_socket_handshake_failed)?;
        let port = handshake
            .get("port")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(slack_socket_handshake_failed)?;
        let secure = handshake.get("secure").and_then(JsonValue::as_bool) == Some(true);
        let addresses = (host.as_str(), port)
            .to_socket_addrs()
            .map_err(|_| slack_socket_connect_failed())?;
        let connect_deadline = Instant::now() + CONNECT_TIMEOUT;
        let mut connected = None;
        for address in addresses {
            let remaining = connect_deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            if let Ok(stream) = TcpStream::connect_timeout(&address, remaining) {
                connected = Some(stream);
                break;
            }
        }
        let stream = connected.ok_or_else(slack_socket_connect_failed)?;
        stream
            .set_nodelay(true)
            .map_err(|_| slack_socket_connect_failed())?;
        let (fd, tls_session_id) = if secure {
            let fd = tcp_stream_into_native_socket(stream);
            let session_id = format!("slack-socket-mode-{}-{fd}", std::process::id());
            start_tls(fd, &host, &session_id)?;
            (fd, Some(session_id))
        } else {
            prepare_plain_stream(&stream).map_err(|_| slack_socket_connect_failed())?;
            (tcp_stream_into_native_socket(stream), None)
        };
        let mut connection = Self {
            fd,
            tls_session_id,
            read_buffer: Vec::new(),
            fragment_opcode: None,
            fragment_payload: Vec::new(),
            close_status_code: None,
            close_reason: None,
            closed: false,
        };
        let request_bytes = json_bytes(
            handshake
                .get("request_bytes")
                .ok_or_else(slack_socket_handshake_failed)?,
        )?;
        connection.write_all(&request_bytes)?;
        let response = connection.read_handshake_response()?;
        let header_end =
            find_http_header_end(&response).ok_or_else(slack_socket_handshake_failed)?;
        let header_bytes = &response[..header_end];
        let validated = agent_transport_websocket_handshake_plan_json(&json!({
            "stage": "validate_response",
            "sec_websocket_key": sec_websocket_key,
            "response_bytes": bytes_json(header_bytes),
        }))
        .map_err(|_| slack_socket_handshake_failed())?;
        if validated.get("upgrade_valid").and_then(JsonValue::as_bool) != Some(true) {
            connection.close();
            return Err(slack_socket_handshake_failed());
        }
        connection
            .read_buffer
            .extend_from_slice(&response[header_end..]);
        Ok(connection)
    }

    fn read_handshake_response(&mut self) -> Result<Vec<u8>, WorkerDiagnostic> {
        let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
        let mut response = Vec::new();
        loop {
            let (bytes, eof) = self.read_once()?;
            response.extend_from_slice(&bytes);
            if response.len() > MAX_HANDSHAKE_BYTES {
                return Err(slack_socket_handshake_failed());
            }
            if find_http_header_end(&response).is_some() {
                return Ok(response);
            }
            if eof || Instant::now() >= deadline {
                return Err(slack_socket_handshake_failed());
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn read_once(&self) -> Result<(Vec<u8>, bool), WorkerDiagnostic> {
        let result = if let Some(session_id) = &self.tls_session_id {
            agent_transport_websocket_tls_execute_json(&json!({
                "stage": "read_tls",
                "tls_connection_id": session_id,
                "max_read_bytes": MAX_SOCKET_READ_BYTES,
                "read_chunk_bytes": READ_CHUNK_BYTES,
            }))
        } else {
            agent_transport_websocket_fd_io_execute_json(&json!({
                "stage": "read_ready_fd",
                "websocket_fd": self.fd,
                "max_read_bytes": MAX_SOCKET_READ_BYTES,
                "read_chunk_bytes": READ_CHUNK_BYTES,
            }))
        }
        .map_err(|_| slack_socket_read_failed())?;
        if result.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            return Err(slack_socket_read_failed());
        }
        let bytes = json_bytes(result.get("read_bytes").unwrap_or(&JsonValue::Null))?;
        let eof = result.get("read_eof").and_then(JsonValue::as_bool) == Some(true)
            || clean_text(result.get("websocket_tls_state")).as_deref() == Some("tls_peer_eof")
            || clean_text(result.get("websocket_fd_io_state")).as_deref() == Some("peer_eof");
        Ok((bytes, eof))
    }

    fn write_all(&self, bytes: &[u8]) -> Result<(), WorkerDiagnostic> {
        let deadline = Instant::now() + WRITE_TIMEOUT;
        let mut remaining = bytes.to_vec();
        loop {
            let result = if let Some(session_id) = &self.tls_session_id {
                agent_transport_websocket_tls_execute_json(&json!({
                    "stage": "write_tls",
                    "tls_connection_id": session_id,
                    "write_bytes": bytes_json(&remaining),
                    "max_write_bytes": remaining.len().max(1),
                }))
            } else {
                agent_transport_websocket_fd_io_execute_json(&json!({
                    "stage": "write_frame",
                    "websocket_fd": self.fd,
                    "write_bytes": bytes_json(&remaining),
                    "max_write_bytes": remaining.len().max(1),
                }))
            }
            .map_err(|_| slack_socket_write_failed())?;
            if result.get("ok").and_then(JsonValue::as_bool) != Some(true) {
                return Err(slack_socket_write_failed());
            }
            if result.get("write_complete").and_then(JsonValue::as_bool) == Some(true)
                || result.get("complete").and_then(JsonValue::as_bool) == Some(true)
            {
                return Ok(());
            }
            remaining = json_bytes(
                result
                    .get("remaining_write_bytes")
                    .unwrap_or(&JsonValue::Null),
            )?;
            if Instant::now() >= deadline {
                return Err(slack_socket_write_failed());
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn send_frame(&self, opcode: &str, payload: JsonValue) -> Result<(), WorkerDiagnostic> {
        let frame = agent_transport_websocket_frame_plan_json(&json!({
            "stage": "encode",
            "opcode": opcode,
            "payload": payload,
            "mask_key": bytes_json(&random_mask_key()?),
        }))
        .map_err(|_| slack_socket_write_failed())?;
        if frame.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            return Err(slack_socket_write_failed());
        }
        let bytes = json_bytes(
            frame
                .get("frame_bytes")
                .ok_or_else(slack_socket_write_failed)?,
        )?;
        self.write_all(&bytes)
    }
}

impl SlackSocketModeConnection for NativeSlackSocketModeConnection {
    fn raw_fd(&self) -> NativeSocket {
        self.fd
    }

    fn has_buffered_input(&self) -> bool {
        !self.read_buffer.is_empty()
    }

    fn read_events(&mut self) -> Result<Vec<SlackSocketModeEvent>, WorkerDiagnostic> {
        let (read_bytes, eof) = self.read_once()?;
        let stream = agent_transport_websocket_stream_plan_json(&json!({
            "stage": "consume_read_chunk",
            "buffer_bytes": bytes_json(&self.read_buffer),
            "read_bytes": bytes_json(&read_bytes),
            "read_eof": eof,
            "fragment_opcode": self.fragment_opcode.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
            "fragment_payload_bytes": bytes_json(&self.fragment_payload),
            "max_payload_bytes": MAX_MESSAGE_BYTES,
            "mask_key": bytes_json(&random_mask_key()?),
        }))
        .map_err(|_| slack_socket_frame_invalid())?;
        let mut events = Vec::new();
        for action in stream
            .get("actions")
            .and_then(JsonValue::as_array)
            .into_iter()
            .flatten()
        {
            match clean_text(action.get("kind")).as_deref() {
                Some("write_websocket_frame") => {
                    let bytes = json_bytes(
                        action
                            .get("frame_bytes")
                            .ok_or_else(slack_socket_frame_invalid)?,
                    )?;
                    self.write_all(&bytes)?;
                }
                Some("deliver_websocket_text") => {
                    let text = action
                        .get("payload_text")
                        .and_then(JsonValue::as_str)
                        .map(str::to_string)
                        .ok_or_else(slack_socket_frame_invalid)?;
                    events.push(SlackSocketModeEvent::Text(text));
                }
                Some("deliver_websocket_binary") => {
                    events.push(SlackSocketModeEvent::Binary(json_bytes(
                        action
                            .get("payload_bytes")
                            .ok_or_else(slack_socket_frame_invalid)?,
                    )?));
                }
                Some("mark_websocket_pong") => events.push(SlackSocketModeEvent::Pong),
                Some("close_websocket") => {
                    self.close_status_code = action
                        .get("status_code")
                        .and_then(JsonValue::as_u64)
                        .and_then(|value| u16::try_from(value).ok());
                    self.close_reason = clean_text(action.get("reason"));
                    events.push(SlackSocketModeEvent::Closed);
                }
                _ => {}
            }
        }
        if stream.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            return Err(slack_socket_frame_invalid());
        }
        self.read_buffer = json_bytes(
            stream
                .get("remaining_buffer_bytes")
                .unwrap_or(&JsonValue::Null),
        )?;
        self.fragment_opcode = clean_text(stream.get("fragment_opcode"));
        self.fragment_payload = json_bytes(
            stream
                .get("fragment_payload_bytes")
                .unwrap_or(&JsonValue::Null),
        )?;
        Ok(events)
    }

    fn send_json(&mut self, payload: &JsonValue) -> Result<(), WorkerDiagnostic> {
        let encoded = JsonCodec::encode_value(payload, JsonEncodeOptions::compact())
            .map_err(|_| slack_socket_write_failed())?;
        self.send_frame("text", JsonValue::String(encoded))
    }

    fn send_ping(&mut self) -> Result<(), WorkerDiagnostic> {
        self.send_frame("ping", JsonValue::String(String::new()))
    }

    fn send_close(&mut self, status_code: u16, reason: &str) -> Result<(), WorkerDiagnostic> {
        let frame = agent_transport_websocket_frame_plan_json(&json!({
            "stage": "encode",
            "opcode": "close",
            "status_code": status_code,
            "reason": reason,
            "mask_key": bytes_json(&random_mask_key()?),
        }))
        .map_err(|_| slack_socket_write_failed())?;
        let bytes = json_bytes(
            frame
                .get("frame_bytes")
                .ok_or_else(slack_socket_write_failed)?,
        )?;
        self.write_all(&bytes)
    }

    fn take_close_info(&mut self) -> (Option<u16>, Option<String>) {
        (self.close_status_code.take(), self.close_reason.take())
    }

    fn close(&mut self) {
        if self.closed {
            return;
        }
        if let Some(session_id) = self.tls_session_id.take() {
            let _ = agent_transport_websocket_tls_execute_json(&json!({
                "stage": "close_tls",
                "tls_connection_id": session_id,
                "close_fd": true,
            }));
        } else if native_socket_is_valid(self.fd) {
            let _ = close_native_socket(self.fd);
        }
        self.fd = INVALID_NATIVE_SOCKET;
        self.closed = true;
    }
}

impl Drop for NativeSlackSocketModeConnection {
    fn drop(&mut self) {
        self.close();
    }
}

fn start_tls(fd: NativeSocket, host: &str, session_id: &str) -> Result<(), WorkerDiagnostic> {
    let started = agent_transport_websocket_tls_execute_json(&json!({
        "stage": "start_tls_handshake",
        "websocket_fd": fd,
        "server_name": host,
        "tls_connection_id": session_id,
        "event_loop_token": SLACK_SOCKET_EVENT_LOOP_TOKEN,
    }))
    .map_err(|_| slack_socket_tls_failed())?;
    if started.get("ok").and_then(JsonValue::as_bool) != Some(true) {
        return Err(slack_socket_tls_failed());
    }
    if started.get("tls_established").and_then(JsonValue::as_bool) == Some(true) {
        return Ok(());
    }
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        let resumed = agent_transport_websocket_tls_execute_json(&json!({
            "stage": "resume",
            "tls_connection_id": session_id,
            "event_loop_token": SLACK_SOCKET_EVENT_LOOP_TOKEN,
        }))
        .map_err(|_| slack_socket_tls_failed())?;
        if resumed.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            return Err(slack_socket_tls_failed());
        }
        if resumed.get("tls_established").and_then(JsonValue::as_bool) == Some(true) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(slack_socket_tls_failed());
        }
        thread::sleep(Duration::from_millis(2));
    }
}

fn random_websocket_key() -> Result<String, WorkerDiagnostic> {
    let mut bytes = [0u8; 16];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| slack_socket_random_failed())?;
    Ok(BASE64_STANDARD.encode(bytes))
}

fn random_mask_key() -> Result<[u8; 4], WorkerDiagnostic> {
    let mut bytes = [0u8; 4];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| slack_socket_random_failed())?;
    Ok(bytes)
}

fn prepare_plain_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nonblocking(true)?;
    set_native_socket_close_on_exec(tcp_stream_native_socket(stream))
}

fn find_http_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn json_bytes(value: &JsonValue) -> Result<Vec<u8>, WorkerDiagnostic> {
    let values = value.as_array().ok_or_else(slack_socket_contract_invalid)?;
    values
        .iter()
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(slack_socket_contract_invalid)
        })
        .collect()
}

fn bytes_json(bytes: &[u8]) -> JsonValue {
    JsonValue::Array(bytes.iter().copied().map(JsonValue::from).collect())
}

fn validate_runtime_plan(plan: &JsonValue) -> Result<(), WorkerDiagnostic> {
    if plan
        .get("slack_socket_mode_runtime_contract")
        .and_then(JsonValue::as_str)
        != Some("ait_agent_core.event_loop.SlackSocketModeRuntime.v1")
        || plan
            .get("python_socket_mode_runtime_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || plan
            .get("python_socket_mode_sequencing_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || plan
            .get("python_websocket_event_loop_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || plan
            .get("python_fallback_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(slack_socket_contract_invalid());
    }
    Ok(())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn safe_slack_error_marker(value: &str) -> String {
    let marker = value.trim();
    if !marker.is_empty()
        && marker.len() <= 64
        && marker
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        marker.to_string()
    } else {
        "unknown".to_string()
    }
}

fn slack_socket_connection_open_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_connection_open_failed",
        "Rust Slack Socket Mode could not acquire a connection URL.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_connect_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_connect_failed",
        "Rust Slack Socket Mode could not connect its WebSocket.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_tls_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_tls_failed",
        "Rust Slack Socket Mode TLS negotiation failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_handshake_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_handshake_failed",
        "Rust Slack Socket Mode WebSocket handshake failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_read_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_read_failed",
        "Rust Slack Socket Mode WebSocket read failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_write_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_write_failed",
        "Rust Slack Socket Mode WebSocket write failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_payload_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_payload_invalid",
        "Rust Slack Socket Mode received an invalid payload.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_transaction_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_transaction_invalid",
        "Rust Slack Socket Mode transaction planning failed closed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_connection_missing() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_connection_missing",
        "Rust Slack Socket Mode has no active connection.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_frame_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_frame_invalid",
        "Rust Slack Socket Mode received an invalid WebSocket frame.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_contract_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_contract_invalid",
        "Rust Slack Socket Mode received an invalid native contract.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn slack_socket_random_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "slack_socket_mode_random_failed",
        "Rust Slack Socket Mode could not generate WebSocket masking entropy.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

#[cfg(test)]
mod tests;
