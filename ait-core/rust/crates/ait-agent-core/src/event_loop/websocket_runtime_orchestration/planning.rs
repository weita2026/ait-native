use std::collections::HashSet;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_websocket_runtime_reconnect_orchestration";
const ORCHESTRATION_CONTRACT: &str =
    "ait_agent_core.event_loop.WebSocketRuntimeReconnectOrchestration.v1";
const DEFAULT_RECONNECT_BASE_DELAY_SECONDS: f64 = 1.0;
const DEFAULT_RECONNECT_MAX_DELAY_SECONDS: f64 = 60.0;
const DEFAULT_MAX_RECONNECT_ATTEMPTS: u64 = 8;

pub trait WebSocketRuntimeOrchestrationPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultWebSocketRuntimeOrchestrationPlanner;

impl WebSocketRuntimeOrchestrationPlanner for DefaultWebSocketRuntimeOrchestrationPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_runtime_orchestration_json(request)
    }
}

pub fn agent_websocket_runtime_orchestration_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_websocket_runtime_orchestration_planner(
        &DefaultWebSocketRuntimeOrchestrationPlanner,
        request,
    )
}

pub fn plan_with_websocket_runtime_orchestration_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: WebSocketRuntimeOrchestrationPlanner + ?Sized,
{
    planner.plan_json(request)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum RuntimeRequestKind {
    Reconnect,
    Stop,
}

impl RuntimeRequestKind {
    fn label(self) -> &'static str {
        match self {
            Self::Reconnect => "reconnect",
            Self::Stop => "stop_runtime",
        }
    }
}

#[derive(Debug, Clone)]
struct RuntimeRequest {
    kind: RuntimeRequestKind,
    transport: String,
    worker_key: Option<String>,
    token: Option<u64>,
    shard_index: Option<u64>,
    reason: Option<String>,
    source_action_kind: String,
    retry_attempt: Option<u64>,
    delay_seconds: Option<f64>,
    session_id: Option<String>,
    resume_gateway_url: Option<String>,
    sequence: Option<i64>,
    gateway_info: Option<JsonValue>,
    socket_url: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RequestContext {
    transport: Option<String>,
    worker_key: Option<String>,
    token: Option<u64>,
    shard_index: Option<u64>,
    retry_attempt: Option<u64>,
    delay_seconds: Option<f64>,
    session_id: Option<String>,
    resume_gateway_url: Option<String>,
    sequence: Option<i64>,
    gateway_info: Option<JsonValue>,
    socket_url: Option<String>,
}

#[derive(Debug, Clone)]
struct ParsedRuntimeRequests {
    requests: Vec<RuntimeRequest>,
    diagnostics: Vec<String>,
    skipped_request_count: usize,
}

impl ParsedRuntimeRequests {
    fn new() -> Self {
        Self {
            requests: Vec::new(),
            diagnostics: Vec::new(),
            skipped_request_count: 0,
        }
    }

    fn push(&mut self, request: RuntimeRequest) {
        self.requests.push(request);
    }

    fn into_result(mut self) -> Result<(Vec<RuntimeRequest>, usize, usize), Vec<String>> {
        if !self.diagnostics.is_empty() {
            return Err(self.diagnostics);
        }
        let before = self.requests.len();
        let mut seen = HashSet::new();
        self.requests
            .retain(|request| seen.insert(request_dedup_key(request)));
        let duplicate_count = before.saturating_sub(self.requests.len());
        Ok((self.requests, self.skipped_request_count, duplicate_count))
    }
}

#[derive(Debug, Clone)]
struct OrchestrationConfig {
    now_monotonic_seconds: Option<f64>,
    reconnect_base_delay_seconds: f64,
    reconnect_max_delay_seconds: f64,
    max_reconnect_attempts: u64,
}

pub fn plan_runtime_orchestration_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "orchestrate".to_string());
    match stage.as_str() {
        "orchestrate" | "lifecycle_result" | "runtime_orchestration" => {
            Ok(plan_orchestration(object))
        }
        other => Err(format!(
            "unsupported WebSocket runtime orchestration stage: {other}"
        )),
    }
}

fn plan_orchestration(object: &Map<String, JsonValue>) -> JsonValue {
    let config = orchestration_config(object);
    let parsed = parse_runtime_request_sources(object);
    let (requests, skipped_request_count, duplicate_request_count) = match parsed.into_result() {
        Ok(parsed) => parsed,
        Err(diagnostics) => {
            return configuration_error_payload(object, &config, diagnostics);
        }
    };
    let stop_requests = requests
        .iter()
        .filter(|request| request.kind == RuntimeRequestKind::Stop)
        .cloned()
        .collect::<Vec<_>>();

    let mut reconnect_schedules = Vec::new();
    let mut stop_schedules = Vec::new();
    let mut runtime_requests = Vec::new();
    let mut actions = Vec::new();
    let mut diagnostics = Vec::new();
    let mut suppressed_reconnect_count = 0_usize;
    let mut exhausted_reconnect_count = 0_usize;

    for request in &requests {
        match request.kind {
            RuntimeRequestKind::Stop => {
                let schedule = stop_schedule_json(request);
                actions.push(stop_action_json(request, &schedule));
                runtime_requests.push(stop_runtime_request_json(request));
                stop_schedules.push(schedule);
            }
            RuntimeRequestKind::Reconnect => {
                if let Some(stop) = matching_stop_request(&stop_requests, request) {
                    suppressed_reconnect_count += 1;
                    diagnostics.push(format!(
                        "WebSocket reconnect for {} suppressed by stop request for {}.",
                        runtime_identity_label(request),
                        runtime_identity_label(stop)
                    ));
                    continue;
                }
                let retry_attempt = request.retry_attempt.unwrap_or(0);
                if retry_attempt >= config.max_reconnect_attempts {
                    exhausted_reconnect_count += 1;
                    let diagnostic = format!(
                        "WebSocket reconnect for {} exhausted attempt limit {}.",
                        runtime_identity_label(request),
                        config.max_reconnect_attempts
                    );
                    diagnostics.push(diagnostic.clone());
                    actions.push(json!({
                        "kind": "diagnose_websocket_reconnect_attempt_exhausted",
                        "transport": request.transport,
                        "worker_key": optional_string_json(request.worker_key.as_deref()),
                        "event_loop_token": request.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
                        "retry_attempt": retry_attempt,
                        "max_reconnect_attempts": config.max_reconnect_attempts,
                        "reason": optional_string_json(request.reason.as_deref()),
                        "diagnostic": diagnostic,
                    }));
                    continue;
                }
                let schedule = reconnect_schedule_json(request, &config, retry_attempt);
                let runtime_request = reconnect_runtime_request_json(request, &schedule);
                actions.push(reconnect_action_json(request, &schedule, &runtime_request));
                runtime_requests.push(runtime_request);
                reconnect_schedules.push(schedule);
            }
        }
    }

    let ok = exhausted_reconnect_count == 0;
    let state = if requests.is_empty() {
        "idle"
    } else if exhausted_reconnect_count > 0 {
        "reconnect_exhausted"
    } else if suppressed_reconnect_count > 0 {
        "stop_precedence_applied"
    } else if !stop_schedules.is_empty() && reconnect_schedules.is_empty() {
        "stop_scheduled"
    } else {
        "reconnect_scheduled"
    };

    base_payload(
        object,
        &config,
        state,
        json!({
            "ok": ok,
            "executed": false,
            "should_wait": reconnect_schedules.iter().any(|schedule| {
                bool_field(schedule.get("should_wait")).unwrap_or(false)
            }),
            "request_count": requests.len(),
            "reconnect_request_count": count_requests(&requests, RuntimeRequestKind::Reconnect),
            "stop_request_count": count_requests(&requests, RuntimeRequestKind::Stop),
            "scheduled_reconnect_count": reconnect_schedules.len(),
            "scheduled_stop_count": stop_schedules.len(),
            "suppressed_reconnect_count": suppressed_reconnect_count,
            "exhausted_reconnect_count": exhausted_reconnect_count,
            "duplicate_request_count": duplicate_request_count,
            "skipped_request_count": skipped_request_count,
            "requests": requests_json(&requests),
            "reconnect_schedules": reconnect_schedules,
            "stop_schedules": stop_schedules,
            "runtime_requests": runtime_requests,
            "diagnostics": diagnostics,
            "actions": actions,
        }),
    )
}

fn parse_runtime_request_sources(object: &Map<String, JsonValue>) -> ParsedRuntimeRequests {
    let mut parsed = ParsedRuntimeRequests::new();
    parse_request_source(
        &JsonValue::Object(object.clone()),
        &RequestContext::default(),
        &mut parsed,
    );
    parsed
}

fn parse_request_source(
    value: &JsonValue,
    context: &RequestContext,
    parsed: &mut ParsedRuntimeRequests,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    let source_context = context.with_object(object);
    if clean_text(object.get("kind")).is_some() {
        parse_action_or_request(value, &source_context, parsed, 0);
        return;
    }
    if let Some(reconnect_requests) = object
        .get("reconnect_requests")
        .or_else(|| object.get("websocket_reconnect_requests"))
        .and_then(JsonValue::as_array)
    {
        for (request_index, request) in reconnect_requests.iter().enumerate() {
            parse_typed_runtime_request(
                request,
                RuntimeRequestKind::Reconnect,
                &source_context,
                parsed,
                request_index,
            );
        }
    }
    if let Some(stop_requests) = object
        .get("stop_requests")
        .or_else(|| object.get("websocket_stop_requests"))
        .and_then(JsonValue::as_array)
    {
        for (request_index, request) in stop_requests.iter().enumerate() {
            parse_typed_runtime_request(
                request,
                RuntimeRequestKind::Stop,
                &source_context,
                parsed,
                request_index,
            );
        }
    }
    if let Some(action) = object.get("action") {
        parse_action_or_request(action, &source_context, parsed, 0);
    }
    if let Some(actions) = object.get("actions").and_then(JsonValue::as_array) {
        for (action_index, action) in actions.iter().enumerate() {
            parse_action_or_request(action, &source_context, parsed, action_index);
        }
    }
    for key in [
        "lifecycle_result",
        "websocket_lifecycle_result",
        "websocket_lifecycle",
        "runtime_orchestration_input",
        "reactor_run_result",
        "run_result",
        "last_tick",
        "reactor_tick_result",
    ] {
        if let Some(nested) = object.get(key) {
            parse_request_source(nested, &source_context, parsed);
        }
    }
}

fn parse_typed_runtime_request(
    value: &JsonValue,
    kind: RuntimeRequestKind,
    context: &RequestContext,
    parsed: &mut ParsedRuntimeRequests,
    request_index: usize,
) {
    let Some(object) = value.as_object() else {
        parsed.diagnostics.push(format!(
            "WebSocket runtime {} request at index {request_index} must be an object.",
            kind.label()
        ));
        return;
    };
    let source_action_kind = clean_text(object.get("source_action_kind"))
        .or_else(|| clean_text(object.get("kind")))
        .unwrap_or_else(|| match kind {
            RuntimeRequestKind::Reconnect => "reconnect_websocket".to_string(),
            RuntimeRequestKind::Stop => "stop_websocket_runtime".to_string(),
        });
    push_runtime_request(
        object,
        kind,
        source_action_kind,
        context,
        parsed,
        request_index,
    );
}

fn parse_action_or_request(
    value: &JsonValue,
    context: &RequestContext,
    parsed: &mut ParsedRuntimeRequests,
    action_index: usize,
) {
    let Some(object) = value.as_object() else {
        parsed.diagnostics.push(format!(
            "WebSocket runtime orchestration action at index {action_index} must be an object."
        ));
        return;
    };
    let Some(kind) = clean_text(object.get("kind")) else {
        parsed.diagnostics.push(format!(
            "WebSocket runtime orchestration action at index {action_index} is missing kind."
        ));
        return;
    };
    let request_kind = match kind.as_str() {
        "reconnect_socket_mode" | "reconnect_gateway" | "reconnect_websocket" => {
            RuntimeRequestKind::Reconnect
        }
        "stop_socket_mode_runtime" | "stop_gateway_runtime" | "stop_websocket_runtime" => {
            RuntimeRequestKind::Stop
        }
        _ => {
            parsed.skipped_request_count += 1;
            return;
        }
    };
    push_runtime_request(object, request_kind, kind, context, parsed, action_index);
}

fn push_runtime_request(
    object: &Map<String, JsonValue>,
    kind: RuntimeRequestKind,
    source_action_kind: String,
    context: &RequestContext,
    parsed: &mut ParsedRuntimeRequests,
    request_index: usize,
) {
    let request_context = context.with_object(object);
    let transport = request_transport(&source_action_kind, object, &request_context);
    if transport.is_none() {
        parsed.diagnostics.push(format!(
            "WebSocket runtime {} request at index {request_index} is missing transport.",
            kind.label()
        ));
        return;
    }
    parsed.push(RuntimeRequest {
        kind,
        transport: transport.unwrap(),
        worker_key: request_context.worker_key,
        token: request_context.token,
        shard_index: request_context.shard_index,
        reason: request_reason(object),
        source_action_kind,
        retry_attempt: request_context.retry_attempt,
        delay_seconds: request_context.delay_seconds,
        session_id: request_context.session_id,
        resume_gateway_url: request_context.resume_gateway_url,
        sequence: request_context.sequence,
        gateway_info: request_context.gateway_info,
        socket_url: request_context.socket_url,
    });
}

impl RequestContext {
    fn with_object(&self, object: &Map<String, JsonValue>) -> Self {
        Self {
            transport: raw_transport(object).or_else(|| self.transport.clone()),
            worker_key: request_worker_key(object).or_else(|| self.worker_key.clone()),
            token: request_token(object).or(self.token),
            shard_index: request_shard_index(object).or(self.shard_index),
            retry_attempt: optional_u64(object.get("retry_attempt"))
                .or_else(|| optional_u64(object.get("attempt")))
                .or(self.retry_attempt),
            delay_seconds: optional_f64(object.get("delay_seconds"))
                .or_else(|| optional_f64(object.get("reconnect_delay_seconds")))
                .or(self.delay_seconds),
            session_id: clean_text(object.get("session_id")).or_else(|| self.session_id.clone()),
            resume_gateway_url: clean_text(object.get("resume_gateway_url"))
                .or_else(|| self.resume_gateway_url.clone()),
            sequence: optional_i64(object.get("sequence")).or(self.sequence),
            gateway_info: object
                .get("gateway_info")
                .cloned()
                .or_else(|| self.gateway_info.clone()),
            socket_url: clean_text(object.get("socket_url"))
                .or_else(|| clean_text(object.get("websocket_url")))
                .or_else(|| self.socket_url.clone()),
        }
    }
}

fn request_transport(
    source_action_kind: &str,
    object: &Map<String, JsonValue>,
    context: &RequestContext,
) -> Option<String> {
    raw_transport(object)
        .or_else(|| context.transport.clone())
        .or_else(|| match source_action_kind {
            "reconnect_socket_mode" | "stop_socket_mode_runtime" => Some("slack".to_string()),
            "reconnect_gateway" | "stop_gateway_runtime" => Some("discord".to_string()),
            "reconnect_websocket" | "stop_websocket_runtime" => Some("websocket".to_string()),
            _ => None,
        })
        .and_then(|transport| normalize_transport(&transport))
}

fn raw_transport(object: &Map<String, JsonValue>) -> Option<String> {
    clean_text(object.get("transport"))
        .or_else(|| clean_text(object.get("websocket_transport")))
        .or_else(|| nested_text(object, "registration", "transport"))
        .or_else(|| nested_text(object, "event_loop_registration", "transport"))
        .or_else(|| nested_text(object, "worker_lease", "transport"))
}

fn normalize_transport(transport: &str) -> Option<String> {
    match transport.trim().to_ascii_lowercase().as_str() {
        "" => None,
        "slack" | "socket_mode" | "slack_socket_mode" => Some("slack".to_string()),
        "discord" | "gateway" | "discord_gateway" => Some("discord".to_string()),
        "websocket" | "generic_websocket" => Some("websocket".to_string()),
        other => Some(other.to_string()),
    }
}

fn request_worker_key(object: &Map<String, JsonValue>) -> Option<String> {
    clean_text(object.get("worker_key"))
        .or_else(|| clean_text(object.get("key")))
        .or_else(|| nested_text(object, "worker_lease", "worker_key"))
        .or_else(|| nested_text(object, "registration", "worker_key"))
}

fn request_token(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(object.get("event_loop_token"))
        .or_else(|| optional_u64(object.get("token")))
        .or_else(|| nested_u64(object, "registration", "token"))
        .or_else(|| nested_u64(object, "event_loop_registration", "token"))
        .or_else(|| nested_u64(object, "worker_lease", "token"))
}

fn request_shard_index(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(object.get("shard_index"))
        .or_else(|| nested_u64(object, "registration", "shard_index"))
        .or_else(|| nested_u64(object, "event_loop_registration", "shard_index"))
        .or_else(|| nested_u64(object, "worker_lease", "shard_index"))
}

fn request_reason(object: &Map<String, JsonValue>) -> Option<String> {
    clean_text(object.get("reason"))
        .or_else(|| clean_text(object.get("reconnect_reason")))
        .or_else(|| clean_text(object.get("error")))
}

fn orchestration_config(object: &Map<String, JsonValue>) -> OrchestrationConfig {
    let reconnect_max_delay_seconds = optional_f64(object.get("reconnect_max_delay_seconds"))
        .or_else(|| optional_f64(object.get("max_delay_seconds")))
        .unwrap_or(DEFAULT_RECONNECT_MAX_DELAY_SECONDS)
        .max(0.0);
    OrchestrationConfig {
        now_monotonic_seconds: optional_f64(object.get("now_monotonic_seconds")),
        reconnect_base_delay_seconds: optional_f64(object.get("reconnect_base_delay_seconds"))
            .or_else(|| optional_f64(object.get("retry_base_delay_seconds")))
            .unwrap_or(DEFAULT_RECONNECT_BASE_DELAY_SECONDS)
            .max(0.0),
        reconnect_max_delay_seconds,
        max_reconnect_attempts: optional_u64(object.get("max_reconnect_attempts"))
            .unwrap_or(DEFAULT_MAX_RECONNECT_ATTEMPTS),
    }
}

fn reconnect_schedule_json(
    request: &RuntimeRequest,
    config: &OrchestrationConfig,
    retry_attempt: u64,
) -> JsonValue {
    let delay_seconds = reconnect_delay_seconds(request, config, retry_attempt);
    json!({
        "kind": "websocket_reconnect_schedule",
        "transport": request.transport,
        "worker_key": optional_string_json(request.worker_key.as_deref()),
        "event_loop_token": request.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "shard_index": request.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": optional_string_json(request.reason.as_deref()),
        "source_action_kind": request.source_action_kind,
        "retry_attempt": retry_attempt,
        "max_reconnect_attempts": config.max_reconnect_attempts,
        "delay_seconds": delay_seconds,
        "should_wait": delay_seconds > 0.0,
        "wait_seconds": delay_seconds,
        "scheduled_at_monotonic_seconds": config.now_monotonic_seconds
            .map(|now| JsonValue::from(now + delay_seconds))
            .unwrap_or(JsonValue::Null),
        "execute_connect": false,
    })
}

fn reconnect_delay_seconds(
    request: &RuntimeRequest,
    config: &OrchestrationConfig,
    retry_attempt: u64,
) -> f64 {
    let raw_delay = request.delay_seconds.unwrap_or_else(|| {
        let exponent = retry_attempt.min(8) as i32;
        config.reconnect_base_delay_seconds * 2_f64.powi(exponent)
    });
    raw_delay.clamp(0.0, config.reconnect_max_delay_seconds)
}

fn stop_schedule_json(request: &RuntimeRequest) -> JsonValue {
    json!({
        "kind": "websocket_stop_schedule",
        "transport": request.transport,
        "worker_key": optional_string_json(request.worker_key.as_deref()),
        "event_loop_token": request.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "shard_index": request.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": optional_string_json(request.reason.as_deref()),
        "source_action_kind": request.source_action_kind,
        "execute_stop": false,
    })
}

fn reconnect_runtime_request_json(request: &RuntimeRequest, schedule: &JsonValue) -> JsonValue {
    let stage = match request.transport.as_str() {
        "slack" if request.socket_url.is_some() => "connect",
        "slack" => "connection_open_request",
        "discord" if request.resume_gateway_url.is_some() || request.gateway_info.is_some() => {
            "gateway_url"
        }
        "discord" => "gateway_info_request",
        _ => "reconnect_websocket",
    };
    let mut runtime_request = Map::from_iter([
        ("stage".to_string(), JsonValue::String(stage.to_string())),
        (
            "transport".to_string(),
            JsonValue::String(request.transport.clone()),
        ),
        (
            "worker_key".to_string(),
            optional_string_json(request.worker_key.as_deref()),
        ),
        (
            "event_loop_token".to_string(),
            request
                .token
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "shard_index".to_string(),
            request
                .shard_index
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "reason".to_string(),
            optional_string_json(request.reason.as_deref()),
        ),
        (
            "source_action_kind".to_string(),
            JsonValue::String(request.source_action_kind.clone()),
        ),
        ("reconnect_schedule".to_string(), schedule.clone()),
        ("execute_connect".to_string(), JsonValue::Bool(false)),
        (
            "python_websocket_runtime_allowed".to_string(),
            JsonValue::Bool(false),
        ),
    ]);
    if let Some(socket_url) = request.socket_url.as_deref() {
        runtime_request.insert(
            "socket_url".to_string(),
            JsonValue::String(socket_url.to_string()),
        );
    }
    if let Some(session_id) = request.session_id.as_deref() {
        runtime_request.insert(
            "session_id".to_string(),
            JsonValue::String(session_id.to_string()),
        );
    }
    if let Some(resume_gateway_url) = request.resume_gateway_url.as_deref() {
        runtime_request.insert(
            "resume_gateway_url".to_string(),
            JsonValue::String(resume_gateway_url.to_string()),
        );
    }
    if let Some(sequence) = request.sequence {
        runtime_request.insert("sequence".to_string(), JsonValue::from(sequence));
    }
    if let Some(gateway_info) = request.gateway_info.as_ref() {
        runtime_request.insert("gateway_info".to_string(), gateway_info.clone());
    }
    JsonValue::Object(runtime_request)
}

fn stop_runtime_request_json(request: &RuntimeRequest) -> JsonValue {
    let stage = match request.transport.as_str() {
        "slack" => "stop_socket_mode_runtime",
        "discord" => "stop_gateway_runtime",
        _ => "stop_websocket_runtime",
    };
    json!({
        "stage": stage,
        "transport": request.transport,
        "worker_key": optional_string_json(request.worker_key.as_deref()),
        "event_loop_token": request.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "shard_index": request.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "reason": optional_string_json(request.reason.as_deref()),
        "source_action_kind": request.source_action_kind,
        "execute_stop": false,
        "python_websocket_runtime_allowed": false,
    })
}

fn reconnect_action_json(
    request: &RuntimeRequest,
    schedule: &JsonValue,
    runtime_request: &JsonValue,
) -> JsonValue {
    let kind = match request.transport.as_str() {
        "slack" => "schedule_socket_mode_reconnect",
        "discord" => "schedule_gateway_reconnect",
        _ => "schedule_websocket_reconnect",
    };
    json!({
        "kind": kind,
        "transport": request.transport,
        "worker_key": optional_string_json(request.worker_key.as_deref()),
        "reason": optional_string_json(request.reason.as_deref()),
        "schedule": schedule,
        "runtime_request": runtime_request,
        "execute_connect": false,
    })
}

fn stop_action_json(request: &RuntimeRequest, schedule: &JsonValue) -> JsonValue {
    let kind = match request.transport.as_str() {
        "slack" => "schedule_socket_mode_runtime_stop",
        "discord" => "schedule_gateway_runtime_stop",
        _ => "schedule_websocket_runtime_stop",
    };
    json!({
        "kind": kind,
        "transport": request.transport,
        "worker_key": optional_string_json(request.worker_key.as_deref()),
        "reason": optional_string_json(request.reason.as_deref()),
        "schedule": schedule,
        "execute_stop": false,
    })
}

fn matching_stop_request<'a>(
    stop_requests: &'a [RuntimeRequest],
    reconnect_request: &RuntimeRequest,
) -> Option<&'a RuntimeRequest> {
    stop_requests
        .iter()
        .find(|stop| stop_matches_reconnect(stop, reconnect_request))
}

fn stop_matches_reconnect(stop: &RuntimeRequest, reconnect: &RuntimeRequest) -> bool {
    if stop.transport != reconnect.transport && stop.transport != "websocket" {
        return false;
    }
    let worker_matches = stop
        .worker_key
        .as_deref()
        .is_none_or(|worker| reconnect.worker_key.as_deref() == Some(worker));
    let token_matches = stop
        .token
        .is_none_or(|token| reconnect.token == Some(token));
    worker_matches && token_matches
}

fn request_dedup_key(request: &RuntimeRequest) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}",
        request.kind.label(),
        request.transport,
        request.worker_key.as_deref().unwrap_or(""),
        request
            .token
            .map(|token| token.to_string())
            .unwrap_or_default(),
        request.source_action_kind,
        request.reason.as_deref().unwrap_or("")
    )
}

fn runtime_identity_label(request: &RuntimeRequest) -> String {
    format!(
        "{}:{}:{}",
        request.transport,
        request.worker_key.as_deref().unwrap_or("runtime"),
        request
            .token
            .map(|token| token.to_string())
            .unwrap_or_else(|| "no-token".to_string())
    )
}

fn requests_json(requests: &[RuntimeRequest]) -> JsonValue {
    JsonValue::Array(
        requests
            .iter()
            .enumerate()
            .map(|(request_index, request)| {
                json!({
                    "request_index": request_index,
                    "kind": request.kind.label(),
                    "transport": request.transport,
                    "worker_key": optional_string_json(request.worker_key.as_deref()),
                    "event_loop_token": request.token.map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "shard_index": request.shard_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "reason": optional_string_json(request.reason.as_deref()),
                    "source_action_kind": request.source_action_kind,
                    "retry_attempt": request.retry_attempt.map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "delay_seconds": request.delay_seconds.map(JsonValue::from).unwrap_or(JsonValue::Null),
                })
            })
            .collect(),
    )
}

fn configuration_error_payload(
    object: &Map<String, JsonValue>,
    config: &OrchestrationConfig,
    diagnostics: Vec<String>,
) -> JsonValue {
    base_payload(
        object,
        config,
        "configuration_error",
        json!({
            "ok": false,
            "executed": false,
            "should_wait": false,
            "request_count": 0,
            "reconnect_request_count": 0,
            "stop_request_count": 0,
            "scheduled_reconnect_count": 0,
            "scheduled_stop_count": 0,
            "suppressed_reconnect_count": 0,
            "exhausted_reconnect_count": 0,
            "duplicate_request_count": 0,
            "skipped_request_count": 0,
            "requests": [],
            "reconnect_schedules": [],
            "stop_schedules": [],
            "runtime_requests": [],
            "diagnostics": diagnostics,
            "error": diagnostics.first().cloned().unwrap_or_else(|| "WebSocket runtime orchestration configuration error.".to_string()),
            "actions": [{
                "kind": "diagnose_websocket_runtime_orchestration_configuration_error",
            }],
        }),
    )
}

fn base_payload(
    object: &Map<String, JsonValue>,
    config: &OrchestrationConfig,
    state: &str,
    payload: JsonValue,
) -> JsonValue {
    let mut output = payload.as_object().cloned().unwrap_or_default();
    output.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    output.insert(
        "websocket_runtime_orchestration_contract".to_string(),
        JsonValue::String(ORCHESTRATION_CONTRACT.to_string()),
    );
    output.insert(
        "stage".to_string(),
        clean_text(object.get("stage"))
            .map(JsonValue::String)
            .unwrap_or_else(|| JsonValue::String("orchestrate".to_string())),
    );
    output.insert(
        "websocket_runtime_orchestration_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    output.insert(
        "now_monotonic_seconds".to_string(),
        config
            .now_monotonic_seconds
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    );
    output.insert(
        "reconnect_base_delay_seconds".to_string(),
        JsonValue::from(config.reconnect_base_delay_seconds),
    );
    output.insert(
        "reconnect_max_delay_seconds".to_string(),
        JsonValue::from(config.reconnect_max_delay_seconds),
    );
    output.insert(
        "max_reconnect_attempts".to_string(),
        JsonValue::from(config.max_reconnect_attempts),
    );
    output.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    output.insert(
        "python_websocket_runtime_allowed".to_string(),
        JsonValue::Bool(false),
    );
    output.insert(
        "python_websocket_reconnect_allowed".to_string(),
        JsonValue::Bool(false),
    );
    output.insert(
        "python_websocket_shutdown_allowed".to_string(),
        JsonValue::Bool(false),
    );
    output.insert(
        "python_fallback_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(output)
}

fn count_requests(requests: &[RuntimeRequest], kind: RuntimeRequestKind) -> usize {
    requests
        .iter()
        .filter(|request| request.kind == kind)
        .count()
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket runtime orchestration request must be an object.".to_string())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = match value? {
        JsonValue::String(text) => text.trim().to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => return None,
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn bool_field(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        },
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_u64(value: Option<&JsonValue>) -> Option<u64> {
    match value? {
        JsonValue::Number(number) => number.as_u64(),
        JsonValue::String(text) => text.trim().parse::<u64>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(number) => number.as_f64().filter(|value| value.is_finite()),
        JsonValue::String(text) => text
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite()),
        JsonValue::Bool(true) => Some(1.0),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn nested_text(object: &Map<String, JsonValue>, parent: &str, key: &str) -> Option<String> {
    object
        .get(parent)
        .and_then(JsonValue::as_object)
        .and_then(|nested| clean_text(nested.get(key)))
}

fn nested_u64(object: &Map<String, JsonValue>, parent: &str, key: &str) -> Option<u64> {
    object
        .get(parent)
        .and_then(JsonValue::as_object)
        .and_then(|nested| optional_u64(nested.get(key)))
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}
