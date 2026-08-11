use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::{
    execute_agent_websocket_reactor_tick, AgentEventLoopBackend, AgentEventLoopBackendPort,
    AgentEventLoopPollPort, AgentEventLoopRegistrationPort,
};

const MIGRATION_STAGE: &str = "rust_agent_websocket_reactor_run_execution";
const REACTOR_RUN_CONTRACT: &str = "ait_agent_core.event_loop.WebSocketReactorRun.v1";
const DEFAULT_MAX_TICKS: u64 = 1;
const DEFAULT_MAX_IDLE_TICKS: u64 = 1;
const HARD_MAX_TICKS: u64 = 1024;

#[derive(Debug, Clone)]
struct RunConfig {
    max_ticks: u64,
    max_idle_ticks: u64,
}

#[derive(Debug, Clone)]
struct RunCounters {
    tick_count: u64,
    eventful_tick_count: u64,
    idle_tick_count: u64,
    poll_event_count: u64,
    known_event_count: u64,
    unknown_event_count: u64,
    turn_failure_count: u64,
    registration_operation_count: u64,
    registration_applied_operation_count: u64,
    merged_connection_update_count: u64,
}

impl RunCounters {
    fn new() -> Self {
        Self {
            tick_count: 0,
            eventful_tick_count: 0,
            idle_tick_count: 0,
            poll_event_count: 0,
            known_event_count: 0,
            unknown_event_count: 0,
            turn_failure_count: 0,
            registration_operation_count: 0,
            registration_applied_operation_count: 0,
            merged_connection_update_count: 0,
        }
    }
}

pub fn execute_agent_websocket_reactor_run<E>(
    event_loop: &mut E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: AgentEventLoopBackendPort + AgentEventLoopPollPort + AgentEventLoopRegistrationPort + ?Sized,
{
    let object = request_object(request)?;
    let backend = event_loop.backend();
    let config = match run_config(object) {
        Ok(config) => config,
        Err(reason) => {
            return Ok(configuration_error_payload(
                object,
                backend,
                "configuration_error",
                &reason,
            ));
        }
    };
    let mut tick_request = object.clone();
    let mut connections = match initial_connections(object) {
        Ok(connections) => connections,
        Err(reason) => {
            return Ok(configuration_error_payload(
                object,
                backend,
                "configuration_error",
                &reason,
            ));
        }
    };
    normalize_connections(&mut tick_request, &connections);

    if connections.is_empty() {
        return Ok(run_payload(
            object,
            backend,
            &config,
            "no_connections",
            true,
            false,
            "no_connections",
            RunCounters::new(),
            Vec::new(),
            Vec::new(),
            JsonValue::Null,
            connections,
        ));
    }

    let mut counters = RunCounters::new();
    let mut tick_history = Vec::new();
    let mut diagnostics = Vec::new();
    let mut last_tick = JsonValue::Null;
    let mut consecutive_idle_ticks = 0u64;
    let mut stop_reason = "max_ticks_reached".to_string();
    let mut run_state = "max_ticks_reached".to_string();
    let mut ok = true;

    for tick_index in 0..config.max_ticks {
        normalize_connections(&mut tick_request, &connections);
        let tick = match execute_agent_websocket_reactor_tick(
            event_loop,
            &JsonValue::Object(tick_request.clone()),
        ) {
            Ok(tick) => tick,
            Err(err) => {
                ok = false;
                run_state = "failed_closed".to_string();
                stop_reason = "tick_execution_error".to_string();
                diagnostics.push(format!(
                    "WebSocket reactor run tick execution failed: {err}"
                ));
                tick_history.push(json!({
                    "tick_index": tick_index,
                    "ok": false,
                    "websocket_reactor_tick_state": "tick_execution_error",
                    "poll_event_count": 0,
                    "known_event_count": 0,
                    "unknown_event_count": 0,
                    "turn_failure_count": 0,
                    "registration_operation_count": 0,
                    "registration_applied_operation_count": 0,
                    "merged_connection_update_count": 0,
                    "error": err,
                }));
                counters.tick_count += 1;
                break;
            }
        };

        counters.tick_count += 1;
        last_tick = tick.clone();
        accumulate_tick_counters(&mut counters, &tick);
        extend_diagnostics(&mut diagnostics, tick.get("diagnostics"));
        let merged = merge_connection_state_updates(&mut connections, &tick);
        counters.merged_connection_update_count += merged as u64;

        let poll_event_count = u64_field(&tick, "poll_event_count").unwrap_or(0);
        let tick_state = clean_text(tick.get("websocket_reactor_tick_state"))
            .unwrap_or_else(|| "unknown".to_string());
        let tick_ok = bool_field(tick.get("ok")).unwrap_or(false);
        if poll_event_count > 0 {
            counters.eventful_tick_count += 1;
            consecutive_idle_ticks = 0;
        } else {
            counters.idle_tick_count += 1;
            consecutive_idle_ticks += 1;
        }

        tick_history.push(tick_summary(tick_index, &tick, merged));

        if !tick_ok {
            ok = false;
            run_state = "failed_closed".to_string();
            stop_reason = tick_state;
            break;
        }
        if consecutive_idle_ticks >= config.max_idle_ticks {
            run_state = "idle_limit_reached".to_string();
            stop_reason = "idle_limit_reached".to_string();
            break;
        }
        if tick_index + 1 == config.max_ticks {
            run_state = "max_ticks_reached".to_string();
            stop_reason = "max_ticks_reached".to_string();
        }
    }

    Ok(run_payload(
        object,
        backend,
        &config,
        &run_state,
        ok,
        true,
        &stop_reason,
        counters,
        tick_history,
        diagnostics,
        last_tick,
        connections,
    ))
}

fn run_config(object: &Map<String, JsonValue>) -> Result<RunConfig, String> {
    let max_ticks = optional_u64_from_keys(
        object,
        &[
            "max_ticks",
            "tick_limit",
            "max_reactor_ticks",
            "max_run_ticks",
        ],
    )?
    .unwrap_or(DEFAULT_MAX_TICKS);
    if max_ticks == 0 {
        return Err("WebSocket reactor run max_ticks must be greater than zero.".to_string());
    }
    if max_ticks > HARD_MAX_TICKS {
        return Err(format!(
            "WebSocket reactor run max_ticks must be <= {HARD_MAX_TICKS}."
        ));
    }
    let max_idle_ticks = optional_u64_from_keys(
        object,
        &[
            "max_idle_ticks",
            "idle_tick_limit",
            "max_consecutive_idle_ticks",
        ],
    )?
    .unwrap_or(DEFAULT_MAX_IDLE_TICKS);
    if max_idle_ticks == 0 {
        return Err("WebSocket reactor run max_idle_ticks must be greater than zero.".to_string());
    }
    Ok(RunConfig {
        max_ticks,
        max_idle_ticks,
    })
}

fn initial_connections(object: &Map<String, JsonValue>) -> Result<Vec<JsonValue>, String> {
    for key in [
        "connections",
        "websocket_connections",
        "workers",
        "worker_connections",
    ] {
        if let Some(value) = object.get(key) {
            let Some(values) = value.as_array() else {
                return Err(format!(
                    "WebSocket reactor run field `{key}` must be an array."
                ));
            };
            return Ok(values.clone());
        }
    }
    Ok(Vec::new())
}

fn normalize_connections(request: &mut Map<String, JsonValue>, connections: &[JsonValue]) {
    request.insert(
        "connections".to_string(),
        JsonValue::Array(connections.to_vec()),
    );
}

fn merge_connection_state_updates(connections: &mut [JsonValue], tick: &JsonValue) -> usize {
    let Some(updates) = tick
        .get("shard_batch_plan")
        .and_then(|plan| plan.get("connection_state_updates"))
        .and_then(JsonValue::as_array)
    else {
        return 0;
    };
    let mut merged = 0usize;
    for update in updates {
        let Some(update_object) = update.as_object() else {
            continue;
        };
        let Some(token) = event_loop_token(update_object) else {
            continue;
        };
        let Some(connection) = connections
            .iter_mut()
            .filter_map(JsonValue::as_object_mut)
            .find(|connection| event_loop_token(connection) == Some(token))
        else {
            continue;
        };
        apply_connection_update(connection, update_object);
        merged += 1;
    }
    merged
}

fn apply_connection_update(
    connection: &mut Map<String, JsonValue>,
    update: &Map<String, JsonValue>,
) {
    clear_one_shot_turn_inputs(connection);
    copy_fields(
        connection,
        update,
        &[
            "backend",
            "shard_index",
            "event_index",
            "worker_key",
            "transport",
            "event_loop_token",
            "websocket_fd",
            "websocket_turn_state",
            "ok",
            "pending_write_byte_count",
            "remaining_write_byte_count",
            "should_keep_registered",
            "should_register_read_write",
            "should_unregister",
            "should_close_websocket",
            "should_reconnect",
        ],
    );
    if let Some(value) = update.get("websocket_turn_state") {
        connection.insert("last_websocket_turn_state".to_string(), value.clone());
    }
    copy_state_aliases(
        connection,
        update,
        "remaining_buffer_bytes",
        &[
            "remaining_buffer_bytes",
            "receive_buffer_bytes",
            "buffer_bytes",
        ],
    );
    copy_state_aliases(
        connection,
        update,
        "remaining_buffer_hex",
        &["remaining_buffer_hex", "receive_buffer_hex", "buffer_hex"],
    );
    copy_state_aliases(
        connection,
        update,
        "remaining_write_bytes",
        &["remaining_write_bytes", "pending_write_bytes"],
    );
    copy_state_aliases(
        connection,
        update,
        "remaining_write_hex",
        &["remaining_write_hex", "pending_write_hex"],
    );
    let interest = if bool_field(update.get("should_unregister")).unwrap_or(false) {
        "unregistered"
    } else if bool_field(update.get("should_register_read_write")).unwrap_or(false) {
        "read_write"
    } else if bool_field(update.get("should_keep_registered")).unwrap_or(false) {
        "readable"
    } else {
        "unknown"
    };
    connection.insert(
        "websocket_registration_interest".to_string(),
        JsonValue::String(interest.to_string()),
    );
}

fn clear_one_shot_turn_inputs(connection: &mut Map<String, JsonValue>) {
    for key in [
        "read_bytes",
        "chunk_bytes",
        "read_hex",
        "chunk_hex",
        "read_eof",
        "eof",
        "write_bytes",
        "write_hex",
        "frame_bytes",
        "frame_hex",
        "queued_write_bytes",
        "queued_write_hex",
    ] {
        connection.remove(key);
    }
}

fn copy_fields(
    target: &mut Map<String, JsonValue>,
    source: &Map<String, JsonValue>,
    fields: &[&str],
) {
    for field in fields {
        if let Some(value) = source.get(*field) {
            target.insert((*field).to_string(), value.clone());
        }
    }
}

fn copy_state_aliases(
    target: &mut Map<String, JsonValue>,
    source: &Map<String, JsonValue>,
    source_key: &str,
    target_keys: &[&str],
) {
    let Some(value) = source.get(source_key) else {
        return;
    };
    for target_key in target_keys {
        target.insert((*target_key).to_string(), value.clone());
    }
}

fn accumulate_tick_counters(counters: &mut RunCounters, tick: &JsonValue) {
    counters.poll_event_count += u64_field(tick, "poll_event_count").unwrap_or(0);
    counters.known_event_count += u64_field(tick, "known_event_count").unwrap_or(0);
    counters.unknown_event_count += u64_field(tick, "unknown_event_count").unwrap_or(0);
    counters.turn_failure_count += u64_field(tick, "turn_failure_count").unwrap_or(0);
    counters.registration_operation_count +=
        u64_field(tick, "registration_operation_count").unwrap_or(0);
    counters.registration_applied_operation_count +=
        u64_field(tick, "registration_applied_operation_count").unwrap_or(0);
}

fn tick_summary(tick_index: u64, tick: &JsonValue, merged_update_count: usize) -> JsonValue {
    json!({
        "tick_index": tick_index,
        "ok": bool_field(tick.get("ok")).unwrap_or(false),
        "websocket_reactor_tick_state": clone_field(tick, "websocket_reactor_tick_state"),
        "poll_timeout_ms": clone_field(tick, "poll_timeout_ms"),
        "poll_event_count": clone_field(tick, "poll_event_count"),
        "known_event_count": clone_field(tick, "known_event_count"),
        "unknown_event_count": clone_field(tick, "unknown_event_count"),
        "turn_failure_count": clone_field(tick, "turn_failure_count"),
        "registration_state": clone_field(tick, "registration_state"),
        "registration_operation_count": clone_field(tick, "registration_operation_count"),
        "registration_applied_operation_count": clone_field(tick, "registration_applied_operation_count"),
        "merged_connection_update_count": merged_update_count,
        "diagnostics": clone_field(tick, "diagnostics"),
    })
}

#[allow(clippy::too_many_arguments)]
fn run_payload(
    object: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    config: &RunConfig,
    state: &str,
    ok: bool,
    executed: bool,
    stop_reason: &str,
    counters: RunCounters,
    tick_history: Vec<JsonValue>,
    diagnostics: Vec<String>,
    last_tick: JsonValue,
    final_connections: Vec<JsonValue>,
) -> JsonValue {
    let mut payload = Map::new();
    payload.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    payload.insert(
        "websocket_reactor_run_contract".to_string(),
        JsonValue::String(REACTOR_RUN_CONTRACT.to_string()),
    );
    payload.insert(
        "stage".to_string(),
        JsonValue::String("execute".to_string()),
    );
    payload.insert(
        "websocket_reactor_run_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    payload.insert("ok".to_string(), JsonValue::Bool(ok));
    payload.insert("executed".to_string(), JsonValue::Bool(executed));
    payload.insert(
        "stop_reason".to_string(),
        JsonValue::String(stop_reason.to_string()),
    );
    payload.insert(
        "backend".to_string(),
        JsonValue::String(backend.label().to_string()),
    );
    if let Some(shard_index) = optional_u64(object.get("shard_index")) {
        payload.insert("shard_index".to_string(), JsonValue::from(shard_index));
    }
    payload.insert("max_ticks".to_string(), JsonValue::from(config.max_ticks));
    payload.insert(
        "max_idle_ticks".to_string(),
        JsonValue::from(config.max_idle_ticks),
    );
    payload.insert(
        "tick_count".to_string(),
        JsonValue::from(counters.tick_count),
    );
    payload.insert(
        "eventful_tick_count".to_string(),
        JsonValue::from(counters.eventful_tick_count),
    );
    payload.insert(
        "idle_tick_count".to_string(),
        JsonValue::from(counters.idle_tick_count),
    );
    payload.insert(
        "poll_event_count".to_string(),
        JsonValue::from(counters.poll_event_count),
    );
    payload.insert(
        "known_event_count".to_string(),
        JsonValue::from(counters.known_event_count),
    );
    payload.insert(
        "unknown_event_count".to_string(),
        JsonValue::from(counters.unknown_event_count),
    );
    payload.insert(
        "turn_failure_count".to_string(),
        JsonValue::from(counters.turn_failure_count),
    );
    payload.insert(
        "registration_operation_count".to_string(),
        JsonValue::from(counters.registration_operation_count),
    );
    payload.insert(
        "registration_applied_operation_count".to_string(),
        JsonValue::from(counters.registration_applied_operation_count),
    );
    payload.insert(
        "merged_connection_update_count".to_string(),
        JsonValue::from(counters.merged_connection_update_count),
    );
    payload.insert(
        "final_connection_count".to_string(),
        JsonValue::from(final_connections.len()),
    );
    payload.insert(
        "final_connections".to_string(),
        JsonValue::Array(final_connections),
    );
    payload.insert("tick_history".to_string(), JsonValue::Array(tick_history));
    payload.insert("last_tick".to_string(), last_tick);
    payload.insert(
        "diagnostics".to_string(),
        JsonValue::Array(diagnostics.into_iter().map(JsonValue::String).collect()),
    );
    payload.insert("actions".to_string(), JsonValue::Array(Vec::new()));
    payload.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    payload.insert(
        "python_websocket_reactor_run_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_websocket_reactor_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_websocket_event_loop_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_websocket_shard_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_websocket_registration_allowed".to_string(),
        JsonValue::Bool(false),
    );
    payload.insert(
        "python_fallback_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(payload)
}

fn configuration_error_payload(
    object: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    state: &str,
    reason: &str,
) -> JsonValue {
    let config = RunConfig {
        max_ticks: DEFAULT_MAX_TICKS,
        max_idle_ticks: DEFAULT_MAX_IDLE_TICKS,
    };
    let mut payload = run_payload(
        object,
        backend,
        &config,
        state,
        false,
        false,
        state,
        RunCounters::new(),
        Vec::new(),
        vec![reason.to_string()],
        JsonValue::Null,
        Vec::new(),
    );
    if let Some(object) = payload.as_object_mut() {
        object.insert("error".to_string(), JsonValue::String(reason.to_string()));
        object.insert(
            "actions".to_string(),
            json!([{
                "kind": "diagnose_websocket_reactor_run_configuration_error",
                "reason": reason,
            }]),
        );
    }
    payload
}

fn optional_u64_from_keys(
    object: &Map<String, JsonValue>,
    keys: &[&str],
) -> Result<Option<u64>, String> {
    for key in keys {
        if let Some(value) = object.get(*key) {
            return required_u64(value, key).map(Some);
        }
    }
    Ok(None)
}

fn required_u64(value: &JsonValue, key: &str) -> Result<u64, String> {
    match value {
        JsonValue::Number(number) => number.as_u64().ok_or_else(|| {
            format!("WebSocket reactor run field `{key}` must be a non-negative integer.")
        }),
        JsonValue::String(text) => text.trim().parse::<u64>().map_err(|_| {
            format!("WebSocket reactor run field `{key}` must be a non-negative integer.")
        }),
        JsonValue::Bool(_) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => Err(
            format!("WebSocket reactor run field `{key}` must be a non-negative integer."),
        ),
    }
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket reactor run request must be an object.".to_string())
}

fn event_loop_token(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(object.get("event_loop_token"))
        .or_else(|| optional_u64(object.get("token")))
        .or_else(|| nested_u64(object, "worker_lease", "token"))
        .or_else(|| nested_u64(object, "event_loop_registration", "token"))
}

fn nested_u64(object: &Map<String, JsonValue>, parent: &str, key: &str) -> Option<u64> {
    object
        .get(parent)
        .and_then(JsonValue::as_object)
        .and_then(|nested| optional_u64(nested.get(key)))
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

fn u64_field(object: &JsonValue, key: &str) -> Option<u64> {
    optional_u64(object.get(key))
}

fn clone_field(object: &JsonValue, key: &str) -> JsonValue {
    object.get(key).cloned().unwrap_or(JsonValue::Null)
}

fn extend_diagnostics(target: &mut Vec<String>, diagnostics: Option<&JsonValue>) {
    let Some(diagnostics) = diagnostics else {
        return;
    };
    if let Some(items) = diagnostics.as_array() {
        for item in items {
            if let Some(text) = clean_text(Some(item)) {
                target.push(text);
            } else if !item.is_null() {
                target.push(item.to_string());
            }
        }
    } else if let Some(text) = clean_text(Some(diagnostics)) {
        target.push(text);
    } else if !diagnostics.is_null() {
        target.push(diagnostics.to_string());
    }
}
