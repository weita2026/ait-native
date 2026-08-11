use std::collections::HashSet;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_websocket_runtime_timer_scheduler";
const TIMER_SCHEDULER_CONTRACT: &str =
    "ait_agent_core.event_loop.WebSocketRuntimeTimerScheduler.v1";
const DEFAULT_IDLE_POLL_TIMEOUT_SECONDS: f64 = 30.0;
const DEFAULT_MAX_POLL_TIMEOUT_SECONDS: f64 = 60.0;

pub trait WebSocketRuntimeTimerScheduler {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultWebSocketRuntimeTimerScheduler;

impl WebSocketRuntimeTimerScheduler for DefaultWebSocketRuntimeTimerScheduler {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_runtime_timer_scheduler_json(request)
    }
}

pub fn agent_websocket_runtime_timer_scheduler_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_websocket_runtime_timer_scheduler(&DefaultWebSocketRuntimeTimerScheduler, request)
}

pub fn plan_with_websocket_runtime_timer_scheduler<P>(
    scheduler: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: WebSocketRuntimeTimerScheduler + ?Sized,
{
    scheduler.plan_json(request)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TimerKind {
    Reconnect,
    Stop,
}

impl TimerKind {
    fn label(self) -> &'static str {
        match self {
            Self::Reconnect => "reconnect",
            Self::Stop => "stop_runtime",
        }
    }
}

#[derive(Debug, Clone)]
struct RuntimeTimer {
    kind: TimerKind,
    transport: String,
    worker_key: Option<String>,
    token: Option<u64>,
    shard_index: Option<u64>,
    reason: Option<String>,
    source_action_kind: String,
    scheduled_at_monotonic_seconds: Option<f64>,
    delay_seconds: Option<f64>,
    runtime_request: Option<JsonValue>,
}

#[derive(Debug, Clone, Default)]
struct TimerContext {
    transport: Option<String>,
    worker_key: Option<String>,
    token: Option<u64>,
    shard_index: Option<u64>,
    reason: Option<String>,
    source_action_kind: Option<String>,
    runtime_request: Option<JsonValue>,
}

impl TimerContext {
    fn with_object(&self, object: &Map<String, JsonValue>) -> Self {
        Self {
            transport: clean_text(object.get("transport"))
                .and_then(|transport| normalize_transport_label(&transport))
                .or_else(|| self.transport.clone()),
            worker_key: clean_text(object.get("worker_key"))
                .or_else(|| clean_text(object.get("runtime_worker_key")))
                .or_else(|| self.worker_key.clone()),
            token: optional_u64(object.get("event_loop_token"))
                .or_else(|| optional_u64(object.get("token")))
                .or(self.token),
            shard_index: optional_u64(object.get("shard_index")).or(self.shard_index),
            reason: clean_text(object.get("reason"))
                .or_else(|| clean_text(object.get("reconnect_reason")))
                .or_else(|| self.reason.clone()),
            source_action_kind: clean_text(object.get("source_action_kind"))
                .or_else(|| clean_text(object.get("kind")))
                .or_else(|| self.source_action_kind.clone()),
            runtime_request: self.runtime_request.clone(),
        }
    }

    fn with_source_action(&self, source_action_kind: String) -> Self {
        let mut context = self.clone();
        context.source_action_kind = Some(source_action_kind);
        context
    }

    fn with_runtime_request(&self, runtime_request: JsonValue) -> Self {
        let context = runtime_request
            .as_object()
            .map(|object| self.with_object(object))
            .unwrap_or_else(|| self.clone());
        Self {
            runtime_request: Some(runtime_request),
            ..context
        }
    }
}

#[derive(Debug, Clone)]
struct ParsedRuntimeTimers {
    timers: Vec<RuntimeTimer>,
    diagnostics: Vec<String>,
    skipped_timer_count: usize,
}

impl ParsedRuntimeTimers {
    fn new() -> Self {
        Self {
            timers: Vec::new(),
            diagnostics: Vec::new(),
            skipped_timer_count: 0,
        }
    }

    fn push(&mut self, timer: RuntimeTimer) {
        self.timers.push(timer);
    }

    fn into_result(mut self) -> Result<(Vec<RuntimeTimer>, usize, usize), Vec<String>> {
        if !self.diagnostics.is_empty() {
            return Err(self.diagnostics);
        }
        let before = self.timers.len();
        let mut seen = HashSet::new();
        self.timers
            .retain(|timer| seen.insert(timer_dedup_key(timer)));
        let duplicate_count = before.saturating_sub(self.timers.len());
        Ok((self.timers, self.skipped_timer_count, duplicate_count))
    }
}

#[derive(Debug, Clone)]
struct TimerSchedulerConfig {
    now_monotonic_seconds: Option<f64>,
    default_idle_timeout_seconds: f64,
    max_poll_timeout_seconds: f64,
}

pub fn plan_runtime_timer_scheduler_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "schedule".to_string());
    match stage.as_str() {
        "schedule" | "timer_scheduler" | "runtime_timer_scheduler" => {
            Ok(plan_timer_scheduler(object))
        }
        other => Err(format!(
            "unsupported WebSocket runtime timer scheduler stage: {other}"
        )),
    }
}

fn plan_timer_scheduler(object: &Map<String, JsonValue>) -> JsonValue {
    let config = scheduler_config(object);
    let parsed = parse_timer_sources(object);
    let (timers, skipped_timer_count, duplicate_timer_count) = match parsed.into_result() {
        Ok(parsed) => parsed,
        Err(diagnostics) => {
            return configuration_error_payload(object, &config, diagnostics);
        }
    };

    let reconnect_timer_count = timers
        .iter()
        .filter(|timer| timer.kind == TimerKind::Reconnect)
        .count();
    if reconnect_timer_count > 0 && config.now_monotonic_seconds.is_none() {
        return configuration_error_payload(
            object,
            &config,
            vec![
                "WebSocket runtime timer scheduling requires now_monotonic_seconds when reconnect timers are present.".to_string(),
            ],
        );
    }
    let now = config.now_monotonic_seconds.unwrap_or(0.0);
    let stop_timers = timers
        .iter()
        .filter(|timer| timer.kind == TimerKind::Stop)
        .cloned()
        .collect::<Vec<_>>();

    let mut due_timers = Vec::new();
    let mut pending_timers = Vec::new();
    let mut next_timers = Vec::new();
    let mut canceled_timers = Vec::new();
    let mut due_runtime_requests = Vec::new();
    let mut actions = Vec::new();
    let mut diagnostics = Vec::new();
    let mut configuration_diagnostics = Vec::new();

    for timer in timers
        .iter()
        .filter(|timer| timer.kind == TimerKind::Reconnect)
    {
        let scheduled_at = match materialized_scheduled_at(timer, &config) {
            Ok(scheduled_at) => scheduled_at,
            Err(diagnostic) => {
                configuration_diagnostics.push(diagnostic);
                continue;
            }
        };
        if let Some(stop_timer) = matching_stop_timer(&stop_timers, timer) {
            let canceled_timer = timer_json(timer, scheduled_at, now, "canceled");
            diagnostics.push(format!(
                "WebSocket reconnect timer for {} canceled by stop request for {}.",
                runtime_identity_label(timer),
                runtime_identity_label(stop_timer)
            ));
            actions.push(cancel_timer_action_json(timer, &canceled_timer, stop_timer));
            canceled_timers.push(canceled_timer);
            continue;
        }
        if scheduled_at <= now {
            let due_timer = timer_json(timer, scheduled_at, now, "due");
            let runtime_request = due_runtime_request_json(timer, scheduled_at, &due_timer);
            actions.push(release_timer_action_json(
                timer,
                &due_timer,
                &runtime_request,
            ));
            due_runtime_requests.push(runtime_request);
            due_timers.push(due_timer);
        } else {
            let pending_timer = timer_json(timer, scheduled_at, now, "pending");
            actions.push(pending_timer_action_json(timer, &pending_timer));
            next_timers.push(pending_timer.clone());
            pending_timers.push(pending_timer);
        }
    }

    if !configuration_diagnostics.is_empty() {
        return configuration_error_payload(object, &config, configuration_diagnostics);
    }

    let next_poll_timeout_seconds =
        next_poll_timeout_seconds(&config, now, &pending_timers, !due_timers.is_empty());
    let state = if due_timers.is_empty() && pending_timers.is_empty() && canceled_timers.is_empty()
    {
        "idle"
    } else if !due_timers.is_empty() && !pending_timers.is_empty() {
        "timers_due_and_pending"
    } else if !due_timers.is_empty() {
        "timers_due"
    } else if !pending_timers.is_empty() {
        "timers_pending"
    } else {
        "stops_applied"
    };

    base_payload(
        object,
        &config,
        state,
        json!({
            "ok": true,
            "executed": false,
            "timer_count": timers.len(),
            "reconnect_timer_count": reconnect_timer_count,
            "stop_timer_count": stop_timers.len(),
            "due_timer_count": due_timers.len(),
            "pending_timer_count": pending_timers.len(),
            "next_timer_count": next_timers.len(),
            "canceled_timer_count": canceled_timers.len(),
            "duplicate_timer_count": duplicate_timer_count,
            "skipped_timer_count": skipped_timer_count,
            "should_poll": true,
            "should_wait": !pending_timers.is_empty() && due_timers.is_empty(),
            "next_poll_timeout_seconds": next_poll_timeout_seconds,
            "next_poll_timeout_milliseconds": seconds_to_milliseconds(next_poll_timeout_seconds),
            "due_timers": due_timers,
            "pending_timers": pending_timers,
            "next_timers": next_timers,
            "canceled_timers": canceled_timers,
            "stop_timers": timers_json(&stop_timers),
            "due_runtime_requests": due_runtime_requests.clone(),
            "runtime_requests": due_runtime_requests,
            "diagnostics": diagnostics,
            "actions": actions,
        }),
    )
}

fn parse_timer_sources(object: &Map<String, JsonValue>) -> ParsedRuntimeTimers {
    let mut parsed = ParsedRuntimeTimers::new();
    parse_timer_source(
        &JsonValue::Object(object.clone()),
        &TimerContext::default(),
        &mut parsed,
    );
    parsed
}

fn parse_timer_source(value: &JsonValue, context: &TimerContext, parsed: &mut ParsedRuntimeTimers) {
    let Some(object) = value.as_object() else {
        return;
    };
    let source_context = context.with_object(object);
    if clean_text(object.get("kind")).is_some() {
        parse_action_or_timer(value, &source_context, parsed, 0);
        return;
    }

    for key in [
        "timers",
        "websocket_runtime_timers",
        "pending_timers",
        "next_timers",
        "reconnect_timers",
    ] {
        if let Some(timers) = object.get(key).and_then(JsonValue::as_array) {
            for (timer_index, timer) in timers.iter().enumerate() {
                parse_action_or_timer(timer, &source_context, parsed, timer_index);
            }
        }
    }

    if let Some(reconnect_schedules) = object
        .get("reconnect_schedules")
        .or_else(|| object.get("websocket_reconnect_schedules"))
        .and_then(JsonValue::as_array)
    {
        for (timer_index, schedule) in reconnect_schedules.iter().enumerate() {
            parse_typed_timer(
                schedule,
                TimerKind::Reconnect,
                &source_context,
                parsed,
                timer_index,
            );
        }
    }
    if let Some(stop_schedules) = object
        .get("stop_schedules")
        .or_else(|| object.get("websocket_stop_schedules"))
        .and_then(JsonValue::as_array)
    {
        for (timer_index, schedule) in stop_schedules.iter().enumerate() {
            parse_typed_timer(
                schedule,
                TimerKind::Stop,
                &source_context,
                parsed,
                timer_index,
            );
        }
    }
    if let Some(runtime_requests) = object.get("runtime_requests").and_then(JsonValue::as_array) {
        for (request_index, runtime_request) in runtime_requests.iter().enumerate() {
            parse_runtime_request_timer(runtime_request, &source_context, parsed, request_index);
        }
    }
    if let Some(runtime_requests) = object
        .get("due_runtime_requests")
        .and_then(JsonValue::as_array)
    {
        for (request_index, runtime_request) in runtime_requests.iter().enumerate() {
            parse_runtime_request_timer(runtime_request, &source_context, parsed, request_index);
        }
    }
    if let Some(action) = object.get("action") {
        parse_action_or_timer(action, &source_context, parsed, 0);
    }
    if let Some(actions) = object.get("actions").and_then(JsonValue::as_array) {
        for (action_index, action) in actions.iter().enumerate() {
            parse_action_or_timer(action, &source_context, parsed, action_index);
        }
    }

    for key in [
        "runtime_orchestration",
        "runtime_orchestration_result",
        "websocket_runtime_orchestration",
        "websocket_runtime_orchestration_result",
        "timer_scheduler_input",
        "websocket_runtime_timer_scheduler",
    ] {
        if let Some(nested) = object.get(key) {
            parse_timer_source(nested, &source_context, parsed);
        }
    }
}

fn parse_action_or_timer(
    value: &JsonValue,
    context: &TimerContext,
    parsed: &mut ParsedRuntimeTimers,
    timer_index: usize,
) {
    let Some(object) = value.as_object() else {
        parsed.skipped_timer_count += 1;
        return;
    };
    if object.get("reconnect_schedule").is_some() || clean_text(object.get("stage")).is_some() {
        parse_runtime_request_timer(value, context, parsed, timer_index);
        return;
    }
    let Some(kind) = clean_text(object.get("kind")) else {
        parsed.skipped_timer_count += 1;
        return;
    };
    let action_context = context.with_object(object).with_source_action(kind.clone());
    match kind.as_str() {
        "websocket_reconnect_schedule" | "websocket_reconnect_timer" => {
            parse_typed_timer(
                value,
                TimerKind::Reconnect,
                &action_context,
                parsed,
                timer_index,
            );
        }
        "websocket_stop_schedule" | "websocket_stop_timer" => {
            parse_typed_timer(value, TimerKind::Stop, &action_context, parsed, timer_index);
        }
        "schedule_socket_mode_reconnect"
        | "schedule_gateway_reconnect"
        | "schedule_websocket_reconnect"
        | "keep_socket_mode_reconnect_timer_pending"
        | "keep_gateway_reconnect_timer_pending"
        | "keep_websocket_reconnect_timer_pending"
        | "release_socket_mode_reconnect"
        | "release_gateway_reconnect"
        | "release_websocket_reconnect" => {
            let runtime_context = object
                .get("runtime_request")
                .map(|runtime_request| action_context.with_runtime_request(runtime_request.clone()))
                .unwrap_or_else(|| action_context.clone());
            if let Some(schedule) = object.get("timer").or_else(|| object.get("schedule")) {
                parse_typed_timer(
                    schedule,
                    TimerKind::Reconnect,
                    &runtime_context,
                    parsed,
                    timer_index,
                );
            } else {
                parse_typed_timer(
                    value,
                    TimerKind::Reconnect,
                    &runtime_context,
                    parsed,
                    timer_index,
                );
            }
        }
        "schedule_socket_mode_runtime_stop"
        | "schedule_gateway_runtime_stop"
        | "schedule_websocket_runtime_stop" => {
            if let Some(schedule) = object.get("timer").or_else(|| object.get("schedule")) {
                parse_typed_timer(
                    schedule,
                    TimerKind::Stop,
                    &action_context,
                    parsed,
                    timer_index,
                );
            } else {
                parse_typed_timer(value, TimerKind::Stop, &action_context, parsed, timer_index);
            }
        }
        "cancel_socket_mode_reconnect_timer"
        | "cancel_gateway_reconnect_timer"
        | "cancel_websocket_reconnect_timer" => {
            parsed.skipped_timer_count += 1;
        }
        _ => {
            parsed.skipped_timer_count += 1;
        }
    }
}

fn parse_runtime_request_timer(
    value: &JsonValue,
    context: &TimerContext,
    parsed: &mut ParsedRuntimeTimers,
    timer_index: usize,
) {
    let Some(object) = value.as_object() else {
        parsed.skipped_timer_count += 1;
        return;
    };
    let runtime_context = context.with_runtime_request(value.clone());
    if let Some(schedule) = object.get("reconnect_schedule") {
        parse_typed_timer(
            schedule,
            TimerKind::Reconnect,
            &runtime_context,
            parsed,
            timer_index,
        );
        return;
    }
    let stage = clean_text(object.get("stage"));
    if matches!(
        stage.as_deref(),
        Some("connect")
            | Some("connection_open_request")
            | Some("gateway_url")
            | Some("gateway_info_request")
            | Some("reconnect_websocket")
    ) {
        parse_typed_timer(
            value,
            TimerKind::Reconnect,
            &runtime_context,
            parsed,
            timer_index,
        );
    } else {
        parsed.skipped_timer_count += 1;
    }
}

fn parse_typed_timer(
    value: &JsonValue,
    kind: TimerKind,
    context: &TimerContext,
    parsed: &mut ParsedRuntimeTimers,
    timer_index: usize,
) {
    let Some(object) = value.as_object() else {
        parsed.skipped_timer_count += 1;
        return;
    };
    let timer_context = context.with_object(object);
    let Some(transport) = timer_context.transport.clone() else {
        parsed.diagnostics.push(format!(
            "WebSocket runtime timer {timer_index} is missing transport."
        ));
        return;
    };
    let scheduled_at_monotonic_seconds = optional_f64(object.get("scheduled_at_monotonic_seconds"))
        .or_else(|| optional_f64(object.get("due_at_monotonic_seconds")))
        .or_else(|| optional_f64(object.get("deadline_monotonic_seconds")));
    let delay_seconds = optional_f64(object.get("delay_seconds"))
        .or_else(|| optional_f64(object.get("wait_seconds")))
        .or_else(|| optional_f64(object.get("reconnect_delay_seconds")));
    if kind == TimerKind::Reconnect
        && scheduled_at_monotonic_seconds.is_none()
        && delay_seconds.is_none()
    {
        parsed.diagnostics.push(format!(
            "WebSocket reconnect timer for {} is missing scheduled_at_monotonic_seconds or delay_seconds.",
            runtime_identity_parts(
                &transport,
                timer_context.worker_key.as_deref(),
                timer_context.token
            )
        ));
        return;
    }
    parsed.push(RuntimeTimer {
        kind,
        transport,
        worker_key: timer_context.worker_key,
        token: timer_context.token,
        shard_index: timer_context.shard_index,
        reason: timer_context.reason,
        source_action_kind: timer_context
            .source_action_kind
            .unwrap_or_else(|| kind.label().to_string()),
        scheduled_at_monotonic_seconds,
        delay_seconds,
        runtime_request: timer_context.runtime_request,
    });
}

fn scheduler_config(object: &Map<String, JsonValue>) -> TimerSchedulerConfig {
    let max_poll_timeout_seconds = optional_f64(object.get("max_poll_timeout_seconds"))
        .or_else(|| optional_f64(object.get("max_timeout_seconds")))
        .unwrap_or(DEFAULT_MAX_POLL_TIMEOUT_SECONDS)
        .max(0.0);
    TimerSchedulerConfig {
        now_monotonic_seconds: optional_f64(object.get("now_monotonic_seconds")),
        default_idle_timeout_seconds: optional_f64(object.get("default_idle_timeout_seconds"))
            .or_else(|| optional_f64(object.get("idle_timeout_seconds")))
            .unwrap_or(DEFAULT_IDLE_POLL_TIMEOUT_SECONDS)
            .max(0.0),
        max_poll_timeout_seconds,
    }
}

fn materialized_scheduled_at(
    timer: &RuntimeTimer,
    config: &TimerSchedulerConfig,
) -> Result<f64, String> {
    if let Some(scheduled_at) = timer.scheduled_at_monotonic_seconds {
        return Ok(scheduled_at.max(0.0));
    }
    let Some(now) = config.now_monotonic_seconds else {
        return Err(format!(
            "WebSocket reconnect timer for {} requires now_monotonic_seconds to materialize delay_seconds.",
            runtime_identity_label(timer)
        ));
    };
    timer.delay_seconds
        .map(|delay| now + delay.max(0.0))
        .ok_or_else(|| {
            format!(
                "WebSocket reconnect timer for {} is missing scheduled_at_monotonic_seconds or delay_seconds.",
                runtime_identity_label(timer)
            )
        })
}

fn next_poll_timeout_seconds(
    config: &TimerSchedulerConfig,
    now: f64,
    pending_timers: &[JsonValue],
    has_due_timers: bool,
) -> f64 {
    if has_due_timers {
        return 0.0;
    }
    let pending_timeout = pending_timers
        .iter()
        .filter_map(|timer| optional_f64(timer.get("scheduled_at_monotonic_seconds")))
        .map(|scheduled_at| (scheduled_at - now).max(0.0))
        .min_by(|left, right| left.total_cmp(right));
    pending_timeout
        .unwrap_or(config.default_idle_timeout_seconds)
        .clamp(0.0, config.max_poll_timeout_seconds)
}

fn timer_json(timer: &RuntimeTimer, scheduled_at: f64, now: f64, timer_state: &str) -> JsonValue {
    json!({
        "kind": "websocket_reconnect_timer",
        "timer_state": timer_state,
        "transport": timer.transport,
        "worker_key": optional_string_json(timer.worker_key.as_deref()),
        "event_loop_token": timer.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "shard_index": timer.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": optional_string_json(timer.reason.as_deref()),
        "source_action_kind": timer.source_action_kind,
        "scheduled_at_monotonic_seconds": scheduled_at,
        "due_in_seconds": (scheduled_at - now).max(0.0),
        "delay_seconds": timer.delay_seconds.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "execute_sleep": false,
        "execute_connect": false,
    })
}

fn timers_json(timers: &[RuntimeTimer]) -> Vec<JsonValue> {
    timers
        .iter()
        .map(|timer| {
            json!({
                "kind": match timer.kind {
                    TimerKind::Reconnect => "websocket_reconnect_timer",
                    TimerKind::Stop => "websocket_stop_timer",
                },
                "transport": timer.transport,
                "worker_key": optional_string_json(timer.worker_key.as_deref()),
                "event_loop_token": timer.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "shard_index": timer.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "reason": optional_string_json(timer.reason.as_deref()),
                "source_action_kind": timer.source_action_kind,
                "execute_sleep": false,
            })
        })
        .collect()
}

fn due_runtime_request_json(
    timer: &RuntimeTimer,
    scheduled_at: f64,
    due_timer: &JsonValue,
) -> JsonValue {
    let mut runtime_request = timer
        .runtime_request
        .as_ref()
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_else(|| {
            Map::from_iter([
                (
                    "stage".to_string(),
                    JsonValue::String(reconnect_stage_for_transport(&timer.transport).to_string()),
                ),
                (
                    "transport".to_string(),
                    JsonValue::String(timer.transport.clone()),
                ),
                (
                    "worker_key".to_string(),
                    optional_string_json(timer.worker_key.as_deref()),
                ),
                (
                    "event_loop_token".to_string(),
                    timer.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
                ),
                (
                    "shard_index".to_string(),
                    timer
                        .shard_index
                        .map(JsonValue::from)
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "reason".to_string(),
                    optional_string_json(timer.reason.as_deref()),
                ),
                (
                    "source_action_kind".to_string(),
                    JsonValue::String(timer.source_action_kind.clone()),
                ),
            ])
        });
    runtime_request
        .entry("stage".to_string())
        .or_insert_with(|| {
            JsonValue::String(reconnect_stage_for_transport(&timer.transport).to_string())
        });
    runtime_request.insert(
        "transport".to_string(),
        JsonValue::String(timer.transport.clone()),
    );
    runtime_request.insert(
        "worker_key".to_string(),
        optional_string_json(timer.worker_key.as_deref()),
    );
    runtime_request.insert(
        "event_loop_token".to_string(),
        timer.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
    );
    runtime_request.insert(
        "shard_index".to_string(),
        timer
            .shard_index
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    );
    runtime_request.insert(
        "reason".to_string(),
        optional_string_json(timer.reason.as_deref()),
    );
    runtime_request.insert(
        "source_action_kind".to_string(),
        JsonValue::String(timer.source_action_kind.clone()),
    );
    runtime_request.insert("timer_due".to_string(), JsonValue::Bool(true));
    runtime_request.insert(
        "timer_due_at_monotonic_seconds".to_string(),
        JsonValue::from(scheduled_at),
    );
    runtime_request.insert("timer".to_string(), due_timer.clone());
    runtime_request.insert("execute_sleep".to_string(), JsonValue::Bool(false));
    runtime_request.insert("execute_connect".to_string(), JsonValue::Bool(false));
    runtime_request.insert(
        "python_websocket_runtime_allowed".to_string(),
        JsonValue::Bool(false),
    );
    runtime_request.insert(
        "python_websocket_timer_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(runtime_request)
}

fn release_timer_action_json(
    timer: &RuntimeTimer,
    due_timer: &JsonValue,
    runtime_request: &JsonValue,
) -> JsonValue {
    let kind = match timer.transport.as_str() {
        "slack" => "release_socket_mode_reconnect",
        "discord" => "release_gateway_reconnect",
        _ => "release_websocket_reconnect",
    };
    json!({
        "kind": kind,
        "transport": timer.transport,
        "worker_key": optional_string_json(timer.worker_key.as_deref()),
        "event_loop_token": timer.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": optional_string_json(timer.reason.as_deref()),
        "timer": due_timer,
        "runtime_request": runtime_request,
        "execute_sleep": false,
        "execute_connect": false,
    })
}

fn pending_timer_action_json(timer: &RuntimeTimer, pending_timer: &JsonValue) -> JsonValue {
    let kind = match timer.transport.as_str() {
        "slack" => "keep_socket_mode_reconnect_timer_pending",
        "discord" => "keep_gateway_reconnect_timer_pending",
        _ => "keep_websocket_reconnect_timer_pending",
    };
    json!({
        "kind": kind,
        "transport": timer.transport,
        "worker_key": optional_string_json(timer.worker_key.as_deref()),
        "event_loop_token": timer.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": optional_string_json(timer.reason.as_deref()),
        "timer": pending_timer,
        "execute_sleep": false,
        "execute_connect": false,
    })
}

fn cancel_timer_action_json(
    timer: &RuntimeTimer,
    canceled_timer: &JsonValue,
    stop_timer: &RuntimeTimer,
) -> JsonValue {
    let kind = match timer.transport.as_str() {
        "slack" => "cancel_socket_mode_reconnect_timer",
        "discord" => "cancel_gateway_reconnect_timer",
        _ => "cancel_websocket_reconnect_timer",
    };
    json!({
        "kind": kind,
        "transport": timer.transport,
        "worker_key": optional_string_json(timer.worker_key.as_deref()),
        "event_loop_token": timer.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": optional_string_json(timer.reason.as_deref()),
        "timer": canceled_timer,
        "stop_timer": timers_json(std::slice::from_ref(stop_timer)).into_iter().next().unwrap_or(JsonValue::Null),
        "execute_sleep": false,
        "execute_connect": false,
    })
}

fn matching_stop_timer<'a>(
    stop_timers: &'a [RuntimeTimer],
    reconnect_timer: &RuntimeTimer,
) -> Option<&'a RuntimeTimer> {
    stop_timers
        .iter()
        .find(|stop_timer| stop_matches_reconnect(stop_timer, reconnect_timer))
}

fn stop_matches_reconnect(stop_timer: &RuntimeTimer, reconnect_timer: &RuntimeTimer) -> bool {
    if stop_timer.transport != reconnect_timer.transport && stop_timer.transport != "websocket" {
        return false;
    }
    let worker_matches = stop_timer
        .worker_key
        .as_deref()
        .is_none_or(|worker| reconnect_timer.worker_key.as_deref() == Some(worker));
    let token_matches = stop_timer
        .token
        .is_none_or(|token| reconnect_timer.token == Some(token));
    worker_matches && token_matches
}

fn reconnect_stage_for_transport(transport: &str) -> &'static str {
    match transport {
        "slack" => "connection_open_request",
        "discord" => "gateway_info_request",
        _ => "reconnect_websocket",
    }
}

fn base_payload(
    object: &Map<String, JsonValue>,
    config: &TimerSchedulerConfig,
    state: &str,
    extra: JsonValue,
) -> JsonValue {
    let mut payload = Map::new();
    payload.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    payload.insert(
        "websocket_runtime_timer_scheduler_contract".to_string(),
        JsonValue::String(TIMER_SCHEDULER_CONTRACT.to_string()),
    );
    payload.insert(
        "websocket_runtime_timer_scheduler_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    payload.insert(
        "stage".to_string(),
        JsonValue::String(
            clean_text(object.get("stage")).unwrap_or_else(|| "schedule".to_string()),
        ),
    );
    payload.insert(
        "now_monotonic_seconds".to_string(),
        config
            .now_monotonic_seconds
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    );
    payload.insert(
        "default_idle_timeout_seconds".to_string(),
        JsonValue::from(config.default_idle_timeout_seconds),
    );
    payload.insert(
        "max_poll_timeout_seconds".to_string(),
        JsonValue::from(config.max_poll_timeout_seconds),
    );
    payload.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    payload.insert(
        "python_websocket_runtime_timer_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_websocket_timer_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_websocket_sleep_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_websocket_runtime_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_fallback_allowed".to_string(),
        JsonValue::Bool(false),
    );
    if let Some(extra) = extra.as_object() {
        for (key, value) in extra {
            payload.insert(key.clone(), value.clone());
        }
    }
    JsonValue::Object(payload)
}

fn configuration_error_payload(
    object: &Map<String, JsonValue>,
    config: &TimerSchedulerConfig,
    diagnostics: Vec<String>,
) -> JsonValue {
    base_payload(
        object,
        config,
        "configuration_error",
        json!({
            "ok": false,
            "executed": false,
            "timer_count": 0,
            "reconnect_timer_count": 0,
            "stop_timer_count": 0,
            "due_timer_count": 0,
            "pending_timer_count": 0,
            "next_timer_count": 0,
            "canceled_timer_count": 0,
            "duplicate_timer_count": 0,
            "skipped_timer_count": 0,
            "should_poll": false,
            "should_wait": false,
            "next_poll_timeout_seconds": JsonValue::Null,
            "next_poll_timeout_milliseconds": JsonValue::Null,
            "due_timers": [],
            "pending_timers": [],
            "next_timers": [],
            "canceled_timers": [],
            "stop_timers": [],
            "due_runtime_requests": [],
            "runtime_requests": [],
            "diagnostics": diagnostics,
            "actions": [{
                "kind": "diagnose_websocket_runtime_timer_scheduler_configuration_error",
                "execute_sleep": false,
                "python_fallback_allowed": false,
            }],
        }),
    )
}

fn timer_dedup_key(timer: &RuntimeTimer) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        timer.kind.label(),
        timer.transport,
        timer.worker_key.as_deref().unwrap_or(""),
        timer
            .token
            .map(|token| token.to_string())
            .unwrap_or_default(),
        timer.source_action_kind,
        timer.reason.as_deref().unwrap_or(""),
        timer_time_key(timer)
    )
}

fn timer_time_key(timer: &RuntimeTimer) -> String {
    if let Some(scheduled_at) = timer.scheduled_at_monotonic_seconds {
        format!("{scheduled_at:.6}")
    } else if let Some(delay_seconds) = timer.delay_seconds {
        format!("delay:{delay_seconds:.6}")
    } else {
        String::new()
    }
}

fn runtime_identity_label(timer: &RuntimeTimer) -> String {
    runtime_identity_parts(&timer.transport, timer.worker_key.as_deref(), timer.token)
}

fn runtime_identity_parts(transport: &str, worker_key: Option<&str>, token: Option<u64>) -> String {
    let worker = worker_key.unwrap_or("<unknown-worker>");
    let token = token
        .map(|token| token.to_string())
        .unwrap_or_else(|| "<no-token>".to_string());
    format!("{transport}/{worker}/{token}")
}

fn seconds_to_milliseconds(seconds: f64) -> u64 {
    let milliseconds = (seconds.max(0.0) * 1000.0).ceil();
    if milliseconds.is_finite() {
        milliseconds as u64
    } else {
        0
    }
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request.as_object().ok_or_else(|| {
        "WebSocket runtime timer scheduler request must be a JSON object".to_string()
    })
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}

fn optional_u64(value: Option<&JsonValue>) -> Option<u64> {
    match value {
        Some(JsonValue::Number(number)) => number.as_u64(),
        Some(JsonValue::String(value)) => value.trim().parse::<u64>().ok(),
        _ => None,
    }
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value {
        Some(JsonValue::Number(number)) => number.as_f64().filter(|value| value.is_finite()),
        Some(JsonValue::String(value)) => value
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite()),
        _ => None,
    }
}

fn normalize_transport_label(raw: &str) -> Option<String> {
    let normalized = raw.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    match normalized.as_str() {
        "" => None,
        "slack" | "socket_mode" | "slack_socket_mode" => Some("slack".to_string()),
        "discord" | "gateway" | "discord_gateway" => Some("discord".to_string()),
        "websocket" | "generic_websocket" => Some("websocket".to_string()),
        other => Some(other.to_string()),
    }
}
