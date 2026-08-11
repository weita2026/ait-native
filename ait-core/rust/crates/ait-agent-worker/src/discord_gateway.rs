use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

use ait_agent_core::{
    agent_discord_gateway_runtime_plan_json, agent_discord_ingress_runtime_plan_json,
    agent_transport_http_execute_json_request_json, DiscordWorkerConfig, NativeSocket,
};
use ait_core::json_support::{json, JsonCodec, JsonValue};

use crate::discord_interaction_once::message_job_request;
use crate::discord_runner::{
    execute_discord_background_message, DefaultDiscordHttpInteractionJobExecutor,
    DiscordHttpInteractionJobExecutor,
};
use crate::slack_socket_mode::{
    connect_native_worker_websocket, SlackSocketModeConnection, SlackSocketModeEvent,
};
use crate::{
    run_worker_host, BoundedWorkerJobExecutor, WorkerDiagnostic, WorkerHostEventLoop,
    WorkerHostRuntime, WorkerJobExecutorConfig, WorkerRunContext, EXIT_INVALID_CONFIGURATION,
    EXIT_RUNTIME_UNAVAILABLE,
};

const DISCORD_GATEWAY_EVENT_LOOP_TOKEN: u64 = 0x0044_4953_434f;
const DISCORD_GATEWAY_MESSAGE_JOB_KIND: &str = "discord.gateway_message_job";
const DEFAULT_MAX_INFLIGHT_JOBS: usize = 4;
const DEFAULT_MAX_RECONNECT_ATTEMPTS: usize = 8;
const DEFAULT_RECONNECT_BASE_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(60);
const MAX_RECENT_MESSAGE_IDS: usize = 256;
const DISCORD_GUILD_MESSAGES_INTENT: i64 = 1 << 9;
const DISCORD_DIRECT_MESSAGES_INTENT: i64 = 1 << 12;
const DISCORD_MESSAGE_CONTENT_INTENT: i64 = 1 << 15;
const DEFAULT_GATEWAY_INTENTS: i64 =
    DISCORD_GUILD_MESSAGES_INTENT | DISCORD_DIRECT_MESSAGES_INTENT | DISCORD_MESSAGE_CONTENT_INTENT;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscordGatewayEvent {
    Text(String),
    Binary(Vec<u8>),
    Pong,
    Closed {
        status_code: Option<u16>,
        reason: Option<String>,
    },
}

pub trait DiscordGatewayConnection {
    fn raw_fd(&self) -> NativeSocket;
    fn has_buffered_input(&self) -> bool {
        false
    }
    fn read_events(&mut self) -> Result<Vec<DiscordGatewayEvent>, WorkerDiagnostic>;
    fn send_json(&mut self, payload: &JsonValue) -> Result<(), WorkerDiagnostic>;
    fn send_close(&mut self, status_code: u16, reason: &str) -> Result<(), WorkerDiagnostic>;
    fn close(&mut self);
}

pub trait DiscordGatewayConnector {
    fn connect(
        &mut self,
        gateway_url: &str,
    ) -> Result<Box<dyn DiscordGatewayConnection>, WorkerDiagnostic>;
}

pub trait DiscordGatewayHttpExecutor: Clone + Send + Sync + 'static {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDiscordGatewayHttpExecutor;

impl DiscordGatewayHttpExecutor for DefaultDiscordGatewayHttpExecutor {
    fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_transport_http_execute_json_request_json(request)
    }
}

pub trait DiscordGatewayClock {
    fn now(&self) -> Duration;
}

#[derive(Debug)]
pub struct SystemDiscordGatewayClock {
    started_at: Instant,
}

impl SystemDiscordGatewayClock {
    pub fn new() -> Self {
        Self {
            started_at: Instant::now(),
        }
    }
}

impl Default for SystemDiscordGatewayClock {
    fn default() -> Self {
        Self::new()
    }
}

impl DiscordGatewayClock for SystemDiscordGatewayClock {
    fn now(&self) -> Duration {
        self.started_at.elapsed()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiscordGatewaySettings {
    pub max_inflight_jobs: usize,
    pub max_reconnect_attempts: usize,
    pub reconnect_base_delay: Duration,
    pub gateway_intents: i64,
}

impl Default for DiscordGatewaySettings {
    fn default() -> Self {
        Self {
            max_inflight_jobs: DEFAULT_MAX_INFLIGHT_JOBS,
            max_reconnect_attempts: DEFAULT_MAX_RECONNECT_ATTEMPTS,
            reconnect_base_delay: DEFAULT_RECONNECT_BASE_DELAY,
            gateway_intents: DEFAULT_GATEWAY_INTENTS,
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDiscordGatewayConnector;

struct NativeDiscordGatewayConnection {
    inner: Box<dyn SlackSocketModeConnection>,
}

impl DiscordGatewayConnector for DefaultDiscordGatewayConnector {
    fn connect(
        &mut self,
        gateway_url: &str,
    ) -> Result<Box<dyn DiscordGatewayConnection>, WorkerDiagnostic> {
        let inner = connect_native_worker_websocket(gateway_url)
            .map_err(|_| discord_gateway_connect_failed())?;
        Ok(Box::new(NativeDiscordGatewayConnection { inner }))
    }
}

impl DiscordGatewayConnection for NativeDiscordGatewayConnection {
    fn raw_fd(&self) -> NativeSocket {
        self.inner.raw_fd()
    }

    fn has_buffered_input(&self) -> bool {
        self.inner.has_buffered_input()
    }

    fn read_events(&mut self) -> Result<Vec<DiscordGatewayEvent>, WorkerDiagnostic> {
        self.inner
            .read_events()
            .map_err(|_| discord_gateway_read_failed())?
            .into_iter()
            .map(|event| {
                Ok(match event {
                    SlackSocketModeEvent::Text(text) => DiscordGatewayEvent::Text(text),
                    SlackSocketModeEvent::Binary(bytes) => DiscordGatewayEvent::Binary(bytes),
                    SlackSocketModeEvent::Pong => DiscordGatewayEvent::Pong,
                    SlackSocketModeEvent::Closed => {
                        let (status_code, reason) = self.inner.take_close_info();
                        DiscordGatewayEvent::Closed {
                            status_code,
                            reason,
                        }
                    }
                })
            })
            .collect()
    }

    fn send_json(&mut self, payload: &JsonValue) -> Result<(), WorkerDiagnostic> {
        self.inner
            .send_json(payload)
            .map_err(|_| discord_gateway_write_failed())
    }

    fn send_close(&mut self, status_code: u16, reason: &str) -> Result<(), WorkerDiagnostic> {
        self.inner
            .send_close(status_code, reason)
            .map_err(|_| discord_gateway_write_failed())
    }

    fn close(&mut self) {
        self.inner.close();
    }
}

pub struct DiscordGatewayWorkerRuntime<C, E, H, K> {
    config: DiscordWorkerConfig,
    connector: C,
    interaction_executor: E,
    http_executor: H,
    clock: K,
    settings: DiscordGatewaySettings,
    jobs: BoundedWorkerJobExecutor<()>,
    connection: Option<Box<dyn DiscordGatewayConnection>>,
    reconnect_attempt: usize,
    reconnect_at: Option<Duration>,
    session_id: Option<String>,
    resume_gateway_url: Option<String>,
    sequence: Option<i64>,
    heartbeat_interval: Option<Duration>,
    next_heartbeat_at: Option<Duration>,
    heartbeat_acknowledged: bool,
    gateway_intents: i64,
    bot_user_id: Option<String>,
    stopping: bool,
    runtime_state: &'static str,
    health_generation: u64,
    last_diagnostic_code: Option<&'static str>,
    recent_message_ids: VecDeque<String>,
    recent_message_set: HashSet<String>,
}

impl<C, E, H, K> DiscordGatewayWorkerRuntime<C, E, H, K>
where
    C: DiscordGatewayConnector,
    E: DiscordHttpInteractionJobExecutor,
    H: DiscordGatewayHttpExecutor,
    K: DiscordGatewayClock,
{
    pub fn new(
        config: DiscordWorkerConfig,
        connector: C,
        interaction_executor: E,
        http_executor: H,
        clock: K,
        settings: DiscordGatewaySettings,
    ) -> Result<Self, WorkerDiagnostic> {
        if config
            .bot_token
            .as_ref()
            .map(|token| token.expose().trim())
            .is_none_or(str::is_empty)
        {
            return Err(discord_bot_token_missing());
        }
        if settings.max_inflight_jobs == 0
            || settings.max_reconnect_attempts == 0
            || settings.reconnect_base_delay.is_zero()
            || settings.gateway_intents < 0
        {
            return Err(discord_gateway_settings_invalid());
        }
        let jobs = BoundedWorkerJobExecutor::new(WorkerJobExecutorConfig {
            max_inflight: settings.max_inflight_jobs,
        })?;
        Ok(Self {
            config,
            connector,
            interaction_executor,
            http_executor,
            clock,
            settings,
            jobs,
            connection: None,
            reconnect_attempt: 0,
            reconnect_at: None,
            session_id: None,
            resume_gateway_url: None,
            sequence: None,
            heartbeat_interval: None,
            next_heartbeat_at: None,
            heartbeat_acknowledged: true,
            gateway_intents: settings.gateway_intents,
            bot_user_id: None,
            stopping: false,
            runtime_state: "starting",
            health_generation: 1,
            last_diagnostic_code: None,
            recent_message_ids: VecDeque::new(),
            recent_message_set: HashSet::new(),
        })
    }

    pub fn runtime_state(&self) -> &'static str {
        self.runtime_state
    }

    pub fn reconnect_attempt(&self) -> usize {
        self.reconnect_attempt
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn resume_gateway_url(&self) -> Option<&str> {
        self.resume_gateway_url.as_deref()
    }

    pub fn sequence(&self) -> Option<i64> {
        self.sequence
    }

    pub fn gateway_intents(&self) -> i64 {
        self.gateway_intents
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
        let gateway_url = acquire_gateway_socket_url(
            &self.config,
            &self.http_executor,
            self.session_id.as_deref(),
            self.resume_gateway_url.as_deref(),
        )?;
        let connection = self.connector.connect(&gateway_url)?;
        event_loop.register_readable(DISCORD_GATEWAY_EVENT_LOOP_TOKEN, connection.raw_fd())?;
        self.connection = Some(connection);
        self.reconnect_at = None;
        self.heartbeat_interval = None;
        self.next_heartbeat_at = None;
        self.heartbeat_acknowledged = true;
        self.set_health("awaiting_hello", None);
        Ok(())
    }

    fn disconnect(&mut self, event_loop: &mut dyn WorkerHostEventLoop) {
        if let Some(mut connection) = self.connection.take() {
            let _ = event_loop.unregister(DISCORD_GATEWAY_EVENT_LOOP_TOKEN);
            connection.close();
        }
        self.heartbeat_interval = None;
        self.next_heartbeat_at = None;
        self.heartbeat_acknowledged = true;
    }

    fn clear_resume_state(&mut self) {
        self.session_id = None;
        self.resume_gateway_url = None;
        self.sequence = None;
        self.bot_user_id = None;
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
                Some("discord_gateway_reconnect_exhausted"),
            );
            return Err(WorkerDiagnostic::new(
                "discord_gateway_reconnect_exhausted",
                "Rust Discord Gateway exhausted its bounded reconnect attempts.",
                EXIT_RUNTIME_UNAVAILABLE,
            )
            .with_detail(
                "max_reconnect_attempts",
                self.settings.max_reconnect_attempts,
            ));
        }
        let exponent = u32::try_from(self.reconnect_attempt.saturating_sub(1).min(8)).unwrap_or(8);
        let delay = self
            .settings
            .reconnect_base_delay
            .saturating_mul(1u32 << exponent)
            .min(MAX_RECONNECT_DELAY);
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
        match self.connect_now(event_loop) {
            Ok(()) => {
                self.set_health("awaiting_hello", Some("discord_gateway_reconnected"));
                Ok(())
            }
            Err(error) if error.exit_code == EXIT_INVALID_CONFIGURATION => Err(error),
            Err(_) => self.schedule_reconnect(event_loop, "discord_gateway_reconnect_failed"),
        }
    }

    fn process_text(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
        text: &str,
    ) -> Result<(), WorkerDiagnostic> {
        let payload =
            JsonCodec::parse_value_with_error_prefix(text, "Invalid Discord Gateway JSON payload")
                .map_err(|_| discord_gateway_payload_invalid())?;
        if !payload.is_object() {
            return Err(discord_gateway_payload_invalid());
        }
        if self.heartbeat_interval.is_none() {
            return self.process_hello(&payload);
        }
        let now = self.clock.now();
        let plan = agent_discord_gateway_runtime_plan_json(&json!({
            "stage": "payload",
            "gateway_payload": payload,
            "session_id": optional_string_json(self.session_id.as_deref()),
            "resume_gateway_url": optional_string_json(self.resume_gateway_url.as_deref()),
            "sequence": self.sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "heartbeat_interval_seconds": self
                .heartbeat_interval
                .map(|interval| interval.as_secs_f64())
                .unwrap_or(1.0),
            "now_monotonic_seconds": now.as_secs_f64(),
        }))
        .map_err(|_| discord_gateway_contract_invalid())?;
        validate_gateway_plan(&plan)?;
        if let Some(sequence) = plan.get("sequence").and_then(JsonValue::as_i64) {
            self.sequence = Some(sequence);
        }
        if plan
            .get("should_send_heartbeat")
            .and_then(JsonValue::as_bool)
            == Some(true)
        {
            let outbound = plan
                .get("outbound_payload")
                .filter(|value| value.is_object())
                .ok_or_else(discord_gateway_contract_invalid)?
                .clone();
            self.connection_mut()?.send_json(&outbound)?;
            self.heartbeat_acknowledged = false;
            self.next_heartbeat_at = self
                .heartbeat_interval
                .map(|interval| now.saturating_add(interval));
        }
        if clean_text(plan.get("gateway_runtime_state")).as_deref()
            == Some("heartbeat_acknowledged")
        {
            self.heartbeat_acknowledged = true;
            self.set_health("ready", None);
        }
        if plan.get("should_reconnect").and_then(JsonValue::as_bool) == Some(true) {
            if plan.get("can_resume").and_then(JsonValue::as_bool) == Some(false) {
                self.clear_resume_state();
            }
            return self.schedule_reconnect(event_loop, "discord_gateway_reconnect_requested");
        }
        match clean_text(plan.get("dispatch_event_name")).as_deref() {
            Some("READY") => self.record_ready(&payload, &plan)?,
            Some("RESUMED") => {
                self.reconnect_attempt = 0;
                self.set_health("ready", None);
            }
            Some("MESSAGE_CREATE") => self.submit_message(&payload)?,
            _ => {}
        }
        Ok(())
    }

    fn process_hello(&mut self, payload: &JsonValue) -> Result<(), WorkerDiagnostic> {
        let bot_token = self.bot_token()?;
        let now = self.clock.now();
        let plan = agent_discord_gateway_runtime_plan_json(&json!({
            "stage": "handshake",
            "hello_payload": payload,
            "bot_token": bot_token,
            "session_id": optional_string_json(self.session_id.as_deref()),
            "sequence": self.sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "gateway_intents": self.gateway_intents,
            "platform": std::env::consts::OS,
            "now_monotonic_seconds": now.as_secs_f64(),
        }))
        .map_err(|_| discord_gateway_hello_invalid())?;
        validate_gateway_plan(&plan)?;
        let interval_seconds = plan
            .get("heartbeat_interval_seconds")
            .and_then(JsonValue::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
            .ok_or_else(discord_gateway_contract_invalid)?;
        let interval = Duration::from_secs_f64(interval_seconds);
        let outbound = plan
            .get("outbound_payload")
            .filter(|value| value.is_object())
            .ok_or_else(discord_gateway_contract_invalid)?
            .clone();
        self.connection_mut()?.send_json(&outbound)?;
        self.heartbeat_interval = Some(interval);
        self.next_heartbeat_at = Some(now.saturating_add(interval));
        self.heartbeat_acknowledged = true;
        if plan.get("should_resume").and_then(JsonValue::as_bool) == Some(true) {
            self.set_health("resuming", None);
        } else {
            self.set_health("identifying", None);
        }
        Ok(())
    }

    fn record_ready(
        &mut self,
        payload: &JsonValue,
        plan: &JsonValue,
    ) -> Result<(), WorkerDiagnostic> {
        self.session_id = clean_text(plan.get("session_id"));
        self.resume_gateway_url = clean_text(plan.get("resume_gateway_url"));
        if self.session_id.is_none() || self.resume_gateway_url.is_none() {
            return Err(discord_gateway_contract_invalid());
        }
        self.bot_user_id = payload
            .get("d")
            .and_then(|data| data.get("user"))
            .and_then(|user| clean_text(user.get("id")));
        self.reconnect_attempt = 0;
        self.set_health("ready", None);
        Ok(())
    }

    fn submit_message(&mut self, gateway_payload: &JsonValue) -> Result<(), WorkerDiagnostic> {
        let message = gateway_payload
            .get("d")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(discord_gateway_contract_invalid)?;
        if self.bot_user_id.as_deref().is_some_and(|bot_user_id| {
            clean_text(message.get("author").and_then(|author| author.get("id"))).as_deref()
                == Some(bot_user_id)
        }) {
            return Ok(());
        }
        let ingress = agent_discord_ingress_runtime_plan_json(&json!({
            "stage": "message",
            "payload": message,
            "config_application_id": self.config.application_id.expose(),
        }))
        .map_err(|_| discord_gateway_message_invalid())?;
        if clean_text(ingress.get("migration_stage")).as_deref()
            != Some("rust_agent_discord_ingress_runtime")
            || clean_text(ingress.get("stage")).as_deref() != Some("message")
        {
            return Err(discord_gateway_message_invalid());
        }
        let should_execute = ingress
            .get("should_submit_turn")
            .and_then(JsonValue::as_bool)
            == Some(true)
            || ingress.get("fresh_topic").and_then(JsonValue::as_bool) == Some(true);
        if !should_execute {
            return Ok(());
        }
        let message_id =
            clean_text(ingress.get("event_id")).ok_or_else(discord_gateway_message_invalid)?;
        if self.recent_message_set.contains(&message_id) {
            return Ok(());
        }
        let job_request = message_job_request(&self.config, message.clone());
        let config = self.config.clone();
        let executor = self.interaction_executor.clone();
        match self.jobs.submit(DISCORD_GATEWAY_MESSAGE_JOB_KIND, move || {
            execute_discord_background_message(&executor, &config, &message, &job_request)
        }) {
            Ok(_) => self.remember_message(message_id),
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

    fn remember_message(&mut self, message_id: String) {
        self.recent_message_set.insert(message_id.clone());
        self.recent_message_ids.push_back(message_id);
        while self.recent_message_ids.len() > MAX_RECENT_MESSAGE_IDS {
            if let Some(expired) = self.recent_message_ids.pop_front() {
                self.recent_message_set.remove(&expired);
            }
        }
    }

    fn handle_heartbeat(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        let (Some(interval), Some(next_heartbeat_at)) =
            (self.heartbeat_interval, self.next_heartbeat_at)
        else {
            return Ok(());
        };
        let now = self.clock.now();
        let plan = agent_discord_gateway_runtime_plan_json(&json!({
            "stage": "tick",
            "now_monotonic_seconds": now.as_secs_f64(),
            "next_heartbeat_at": next_heartbeat_at.as_secs_f64(),
            "heartbeat_interval_seconds": interval.as_secs_f64(),
            "heartbeat_acknowledged": self.heartbeat_acknowledged,
            "sequence": self.sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
        }))
        .map_err(|_| discord_gateway_contract_invalid())?;
        validate_gateway_plan(&plan)?;
        if plan.get("should_reconnect").and_then(JsonValue::as_bool) == Some(true) {
            return self.schedule_reconnect(event_loop, "discord_gateway_heartbeat_ack_timeout");
        }
        if plan
            .get("should_send_heartbeat")
            .and_then(JsonValue::as_bool)
            == Some(true)
        {
            let outbound = plan
                .get("outbound_payload")
                .filter(|value| value.is_object())
                .ok_or_else(discord_gateway_contract_invalid)?
                .clone();
            self.connection_mut()?.send_json(&outbound)?;
            self.heartbeat_acknowledged = false;
            self.next_heartbeat_at = Some(now.saturating_add(interval));
            self.set_health("awaiting_heartbeat_ack", None);
        }
        Ok(())
    }

    fn handle_close(
        &mut self,
        event_loop: &mut dyn WorkerHostEventLoop,
        status_code: Option<u16>,
    ) -> Result<(), WorkerDiagnostic> {
        match status_code {
            Some(4004 | 4010 | 4011 | 4012 | 4013) => {
                self.disconnect(event_loop);
                self.set_health("failed", Some("discord_gateway_close_fatal"));
                Err(WorkerDiagnostic::new(
                    "discord_gateway_close_fatal",
                    "Discord closed the Gateway with a non-recoverable configuration error.",
                    EXIT_INVALID_CONFIGURATION,
                )
                .with_detail("status_code", status_code.unwrap_or_default()))
            }
            Some(4014) => {
                let recovery = agent_discord_gateway_runtime_plan_json(&json!({
                    "stage": "error_recovery",
                    "error_message": "Discord close 4014: disallowed intent",
                    "gateway_intents": self.gateway_intents,
                    "session_id": optional_string_json(self.session_id.as_deref()),
                    "resume_gateway_url": optional_string_json(self.resume_gateway_url.as_deref()),
                    "sequence": self.sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
                }))
                .map_err(|_| discord_gateway_contract_invalid())?;
                validate_gateway_plan(&recovery)?;
                if recovery
                    .get("should_drop_message_content_intent")
                    .and_then(JsonValue::as_bool)
                    != Some(true)
                {
                    self.disconnect(event_loop);
                    return Err(WorkerDiagnostic::new(
                        "discord_gateway_intents_disallowed",
                        "Discord rejected the configured Gateway intents.",
                        EXIT_INVALID_CONFIGURATION,
                    ));
                }
                self.gateway_intents = recovery
                    .get("new_gateway_intents")
                    .and_then(JsonValue::as_i64)
                    .ok_or_else(discord_gateway_contract_invalid)?;
                self.clear_resume_state();
                self.schedule_reconnect(
                    event_loop,
                    "discord_gateway_message_content_intent_dropped",
                )
            }
            Some(4007 | 4009) => {
                self.clear_resume_state();
                self.schedule_reconnect(event_loop, "discord_gateway_resume_state_rejected")
            }
            _ => self.schedule_reconnect(event_loop, "discord_gateway_peer_closed"),
        }
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
                Some("discord_gateway_message_job_failed"),
            );
        }
    }

    fn connection_mut(
        &mut self,
    ) -> Result<&mut (dyn DiscordGatewayConnection + 'static), WorkerDiagnostic> {
        self.connection
            .as_deref_mut()
            .ok_or_else(discord_gateway_connection_missing)
    }

    fn bot_token(&self) -> Result<&str, WorkerDiagnostic> {
        self.config
            .bot_token
            .as_ref()
            .map(|token| token.expose().trim())
            .filter(|token| !token.is_empty())
            .ok_or_else(discord_bot_token_missing)
    }
}

impl<C, E, H, K> WorkerHostRuntime for DiscordGatewayWorkerRuntime<C, E, H, K>
where
    C: DiscordGatewayConnector,
    E: DiscordHttpInteractionJobExecutor,
    H: DiscordGatewayHttpExecutor,
    K: DiscordGatewayClock,
{
    fn start(
        &mut self,
        _context: &WorkerRunContext,
        event_loop: &mut dyn WorkerHostEventLoop,
    ) -> Result<(), WorkerDiagnostic> {
        match self.connect_now(event_loop) {
            Ok(()) => Ok(()),
            Err(error) if error.exit_code == EXIT_INVALID_CONFIGURATION => Err(error),
            Err(_) => self.schedule_reconnect(event_loop, "discord_gateway_initial_connect_failed"),
        }
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
            .find(|event| event.token == DISCORD_GATEWAY_EVENT_LOOP_TOKEN);
        let has_buffered_input = self
            .connection
            .as_ref()
            .is_some_and(|connection| connection.has_buffered_input());
        if has_buffered_input || socket_event.is_some_and(|event| event.readable) {
            let socket_events = match self.connection_mut()?.read_events() {
                Ok(events) => events,
                Err(_) => {
                    return self.schedule_reconnect(event_loop, "discord_gateway_read_failed")
                }
            };
            for event in socket_events {
                match event {
                    DiscordGatewayEvent::Text(text) => {
                        if self.process_text(event_loop, &text).is_err() {
                            return self
                                .schedule_reconnect(event_loop, "discord_gateway_payload_invalid");
                        }
                        if self.connection.is_none() {
                            return Ok(());
                        }
                    }
                    DiscordGatewayEvent::Binary(_) => {
                        return self
                            .schedule_reconnect(event_loop, "discord_gateway_binary_unsupported")
                    }
                    DiscordGatewayEvent::Pong => {}
                    DiscordGatewayEvent::Closed { status_code, .. } => {
                        return self.handle_close(event_loop, status_code)
                    }
                }
            }
        }
        if self.connection.is_some() && socket_event.is_some_and(|event| event.hangup) {
            return self.schedule_reconnect(event_loop, "discord_gateway_hangup");
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
                "discord_gateway_jobs_still_inflight",
                "Rust Discord Gateway jobs remain in flight during graceful shutdown.",
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
        self.set_health("stopped", Some("discord_gateway_forced_shutdown"));
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

pub fn run_discord_gateway_transport(
    context: &WorkerRunContext,
    config: &DiscordWorkerConfig,
) -> Result<(), WorkerDiagnostic> {
    let mut runtime = DiscordGatewayWorkerRuntime::new(
        config.clone(),
        DefaultDiscordGatewayConnector,
        DefaultDiscordHttpInteractionJobExecutor,
        DefaultDiscordGatewayHttpExecutor,
        SystemDiscordGatewayClock::new(),
        DiscordGatewaySettings::default(),
    )?;
    run_worker_host(context, &mut runtime)
}

fn acquire_gateway_socket_url<H>(
    config: &DiscordWorkerConfig,
    executor: &H,
    session_id: Option<&str>,
    resume_gateway_url: Option<&str>,
) -> Result<String, WorkerDiagnostic>
where
    H: DiscordGatewayHttpExecutor,
{
    if session_id.is_some() && resume_gateway_url.is_some() {
        let plan = agent_discord_gateway_runtime_plan_json(&json!({
            "stage": "gateway_url",
            "session_id": optional_string_json(session_id),
            "resume_gateway_url": optional_string_json(resume_gateway_url),
        }))
        .map_err(|_| discord_gateway_contract_invalid())?;
        validate_gateway_plan(&plan)?;
        return gateway_url_from_plan(&plan);
    }
    let token = config
        .bot_token
        .as_ref()
        .map(|token| token.expose().trim())
        .filter(|token| !token.is_empty())
        .ok_or_else(discord_bot_token_missing)?;
    let request_plan = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "gateway_info_request",
        "bot_token": token,
        "discord_api_base_url": config.api_base_url,
        "discord_http_user_agent": config.http_user_agent,
        "request_timeout_seconds": config
            .shared
            .request_timeout_seconds
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    }))
    .map_err(|_| discord_gateway_discovery_failed())?;
    validate_gateway_plan(&request_plan)?;
    let request = request_plan
        .get("request")
        .filter(|value| value.is_object())
        .ok_or_else(discord_gateway_contract_invalid)?;
    let executed = executor
        .execute(request)
        .map_err(|_| discord_gateway_discovery_failed())?;
    if executed.get("ok").and_then(JsonValue::as_bool) != Some(true) {
        return Err(discord_gateway_discovery_failed());
    }
    let gateway_info = executed
        .get("payload")
        .filter(|value| value.is_object())
        .ok_or_else(discord_gateway_discovery_failed)?;
    let plan = agent_discord_gateway_runtime_plan_json(&json!({
        "stage": "gateway_url",
        "gateway_info": gateway_info,
    }))
    .map_err(|_| discord_gateway_contract_invalid())?;
    validate_gateway_plan(&plan)?;
    gateway_url_from_plan(&plan)
}

fn gateway_url_from_plan(plan: &JsonValue) -> Result<String, WorkerDiagnostic> {
    if plan.get("should_connect").and_then(JsonValue::as_bool) != Some(true) {
        return Err(discord_gateway_discovery_failed());
    }
    clean_text(plan.get("gateway_socket_url"))
        .filter(|url| url.starts_with("wss://") || url.starts_with("ws://"))
        .ok_or_else(discord_gateway_discovery_failed)
}

fn validate_gateway_plan(plan: &JsonValue) -> Result<(), WorkerDiagnostic> {
    if clean_text(plan.get("migration_stage")).as_deref()
        != Some("rust_agent_discord_gateway_runtime")
        || clean_text(plan.get("gateway_runtime_contract")).as_deref()
            != Some("ait_agent_core.event_loop.DiscordGatewayRuntime.v1")
        || plan
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || plan
            .get("python_gateway_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(discord_gateway_contract_invalid());
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

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null)
}

fn discord_bot_token_missing() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_bot_token_missing",
        "The Rust Discord Gateway worker requires a bot token.",
        EXIT_INVALID_CONFIGURATION,
    )
}

fn discord_gateway_settings_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_settings_invalid",
        "Rust Discord Gateway runtime settings must be positive and bounded.",
        EXIT_INVALID_CONFIGURATION,
    )
}

fn discord_gateway_discovery_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_discovery_failed",
        "Rust Discord Gateway discovery failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_gateway_connect_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_connect_failed",
        "Rust Discord Gateway could not connect its WebSocket.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_gateway_read_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_read_failed",
        "Rust Discord Gateway WebSocket read failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_gateway_write_failed() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_write_failed",
        "Rust Discord Gateway WebSocket write failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_gateway_payload_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_payload_invalid",
        "Rust Discord Gateway received an invalid payload.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_gateway_hello_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_hello_invalid",
        "Rust Discord Gateway did not receive a valid Hello payload.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_gateway_message_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_message_invalid",
        "Rust Discord Gateway received an invalid message dispatch.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_gateway_contract_invalid() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_contract_invalid",
        "Rust Discord Gateway planning contract validation failed.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

fn discord_gateway_connection_missing() -> WorkerDiagnostic {
    WorkerDiagnostic::new(
        "discord_gateway_connection_missing",
        "Rust Discord Gateway connection is unavailable.",
        EXIT_RUNTIME_UNAVAILABLE,
    )
}

#[cfg(test)]
mod tests;
