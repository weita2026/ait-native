use std::collections::HashMap;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::{
    agent_websocket_event_loop_turn_plan_json, AgentEventLoopBackend, AgentEventLoopConfig,
    AgentRuntimeCapacity, DEFAULT_WORKERS_PER_EPOLL_SHARD, DEFAULT_WORKERS_PER_POLL_SHARD,
};

const MIGRATION_STAGE: &str = "rust_agent_websocket_shard_event_batch_boundary";
const WEBSOCKET_SHARD_CONTRACT: &str = "ait_agent_core.event_loop.WebSocketShardEventBatch.v1";

pub fn agent_websocket_shard_event_batch_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "event_batch".to_string());

    match stage.as_str() {
        "event_batch" | "ready_batch" | "readiness_batch" | "websocket_shard_event_batch" => {
            plan_event_batch(object)
        }
        other => Err(format!(
            "unsupported WebSocket shard event-batch stage: {other}"
        )),
    }
}

fn plan_event_batch(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let backend = backend_from_request(object)?;
    let shard_index = optional_usize(object.get("shard_index")).unwrap_or(0);
    let worker_values = match array_field(
        object,
        &[
            "connections",
            "websocket_connections",
            "workers",
            "worker_connections",
        ],
    ) {
        Ok(values) => values,
        Err(reason) => {
            return Ok(batch_payload(
                "event_batch",
                "invalid_batch_payload",
                backend,
                shard_index,
                json!({
                    "ok": false,
                    "launch_allowed": false,
                    "shard_batch_allowed": false,
                    "expected_concurrent_workers": 0,
                    "workers_per_shard": default_workers_per_shard(backend),
                    "high_concurrency": false,
                    "requires_epoll_for_target_scale": false,
                    "connection_count": 0,
                    "event_count": 0,
                    "known_event_count": 0,
                    "unknown_event_count": 0,
                    "turn_failure_count": 0,
                    "turn_results": [],
                    "connection_state_updates": [],
                    "diagnostics": [reason],
                    "actions": [{
                        "kind": "diagnose_invalid_websocket_shard_payload",
                    }],
                }),
            ));
        }
    };
    let event_values = match array_field(object, &["events", "ready_events", "readiness_events"]) {
        Ok(values) => values,
        Err(reason) => {
            return Ok(batch_payload(
                "event_batch",
                "invalid_batch_payload",
                backend,
                shard_index,
                json!({
                    "ok": false,
                    "launch_allowed": false,
                    "shard_batch_allowed": false,
                    "expected_concurrent_workers": worker_values.len(),
                    "workers_per_shard": default_workers_per_shard(backend),
                    "high_concurrency": false,
                    "requires_epoll_for_target_scale": false,
                    "connection_count": worker_values.len(),
                    "event_count": 0,
                    "known_event_count": 0,
                    "unknown_event_count": 0,
                    "turn_failure_count": 0,
                    "turn_results": [],
                    "connection_state_updates": [],
                    "diagnostics": [reason],
                    "actions": [{
                        "kind": "diagnose_invalid_websocket_shard_payload",
                    }],
                }),
            ));
        }
    };

    let expected_concurrent_workers = optional_usize(object.get("expected_concurrent_workers"))
        .or_else(|| optional_usize(object.get("expected_workers")))
        .unwrap_or(worker_values.len())
        .max(worker_values.len());
    let workers_per_shard = optional_usize(object.get("workers_per_shard"))
        .unwrap_or_else(|| default_workers_per_shard(backend))
        .max(1);
    let capacity = AgentRuntimeCapacity::from_config(AgentEventLoopConfig {
        backend,
        workers_per_shard,
        expected_workers: expected_concurrent_workers,
    });
    let declared_high_concurrency = optional_bool(object.get("high_concurrency")).unwrap_or(false);
    let high_concurrency = declared_high_concurrency || capacity.high_concurrency;
    let requires_epoll_for_target_scale = capacity.requires_epoll_for_target_scale
        || (declared_high_concurrency && !backend.is_epoll());

    if requires_epoll_for_target_scale {
        let reason = format!(
            "WebSocket shard event batching for {expected_concurrent_workers} expected workers requires linux_epoll; backend {} is not allowed for high-concurrency ait-agent websocket workers.",
            backend.label()
        );
        return Ok(batch_payload(
            "event_batch",
            "backend_requires_epoll",
            backend,
            shard_index,
            json!({
                "ok": false,
                "launch_allowed": false,
                "shard_batch_allowed": false,
                "expected_concurrent_workers": expected_concurrent_workers,
                "workers_per_shard": workers_per_shard,
                "high_concurrency": high_concurrency,
                "requires_epoll_for_target_scale": true,
                "connection_count": worker_values.len(),
                "event_count": event_values.len(),
                "known_event_count": 0,
                "unknown_event_count": 0,
                "turn_failure_count": 0,
                "turn_results": [],
                "connection_state_updates": [],
                "diagnostics": [reason.clone()],
                "actions": [{
                    "kind": "reject_websocket_shard_backend",
                    "backend": backend.label(),
                    "shard_index": shard_index,
                    "reason": reason,
                }],
            }),
        ));
    }

    let mut diagnostics = Vec::new();
    let mut connections: Vec<ShardConnection> = Vec::new();
    let mut token_to_connection: HashMap<u64, usize> = HashMap::new();
    for (connection_index, value) in worker_values.iter().enumerate() {
        let Some(object) = value.as_object() else {
            diagnostics.push(format!(
                "WebSocket shard connection at index {connection_index} must be an object."
            ));
            continue;
        };
        let Some(token) = event_loop_token(object) else {
            diagnostics.push(format!(
                "WebSocket shard connection at index {connection_index} is missing event_loop_token."
            ));
            continue;
        };
        if let Some(previous_index) = token_to_connection.insert(token, connections.len()) {
            let previous = &connections[previous_index];
            diagnostics.push(format!(
                "WebSocket shard token {token} is assigned to both `{}` and connection index {connection_index}.",
                previous.worker_key
            ));
            continue;
        }
        connections.push(ShardConnection {
            worker_key: worker_key(object, token, connection_index),
            transport: transport_label(object).unwrap_or_else(|| "unknown".to_string()),
            token,
            fd: websocket_fd(object),
            object: object.clone(),
        });
    }

    if !diagnostics.is_empty() {
        return Ok(batch_payload(
            "event_batch",
            "invalid_connection_state",
            backend,
            shard_index,
            json!({
                "ok": false,
                "launch_allowed": false,
                "shard_batch_allowed": false,
                "expected_concurrent_workers": expected_concurrent_workers,
                "workers_per_shard": workers_per_shard,
                "high_concurrency": high_concurrency,
                "requires_epoll_for_target_scale": false,
                "connection_count": worker_values.len(),
                "event_count": event_values.len(),
                "known_event_count": 0,
                "unknown_event_count": 0,
                "turn_failure_count": 0,
                "turn_results": [],
                "connection_state_updates": [],
                "diagnostics": diagnostics,
                "actions": [{
                    "kind": "diagnose_invalid_websocket_shard_state",
                    "backend": backend.label(),
                    "shard_index": shard_index,
                }],
            }),
        ));
    }

    let mut actions = Vec::new();
    let mut turn_results = Vec::new();
    let mut state_updates = Vec::new();
    let mut known_event_count = 0usize;
    let mut unknown_event_count = 0usize;
    let mut turn_failure_count = 0usize;

    for (event_index, value) in event_values.iter().enumerate() {
        let Some(event) = value.as_object() else {
            unknown_event_count += 1;
            let reason = format!("WebSocket shard event at index {event_index} must be an object.");
            diagnostics.push(reason.clone());
            actions.push(json!({
                "kind": "diagnose_malformed_websocket_event",
                "event_index": event_index,
                "reason": reason,
            }));
            continue;
        };
        let Some(token) = event_loop_token(event) else {
            unknown_event_count += 1;
            let reason = format!(
                "WebSocket shard event at index {event_index} is missing event_loop_token."
            );
            diagnostics.push(reason.clone());
            actions.push(json!({
                "kind": "diagnose_malformed_websocket_event",
                "event_index": event_index,
                "reason": reason,
            }));
            continue;
        };
        let Some(connection_index) = token_to_connection.get(&token).copied() else {
            unknown_event_count += 1;
            let reason = format!(
                "WebSocket shard event token {token} has no registered connection in shard {shard_index}."
            );
            diagnostics.push(reason.clone());
            actions.push(json!({
                "kind": "diagnose_unknown_websocket_token",
                "backend": backend.label(),
                "shard_index": shard_index,
                "event_index": event_index,
                "event_loop_token": token,
                "reason": reason,
            }));
            continue;
        };
        let connection = &connections[connection_index];
        let readable = event_bool(event, "readable").unwrap_or(true);
        let hangup = event_bool(event, "hangup")
            .or_else(|| event_bool(event, "hup"))
            .unwrap_or(false);
        let writable = event_bool(event, "writable").unwrap_or(false);
        if !readable && !writable && !hangup {
            actions.push(json!({
                "kind": "skip_websocket_event",
                "backend": backend.label(),
                "shard_index": shard_index,
                "event_index": event_index,
                "event_loop_token": token,
                "worker_key": connection.worker_key,
                "reason": "event was neither readable, writable, nor hangup",
            }));
            continue;
        }

        known_event_count += 1;
        let turn_stage = if readable && writable && !hangup {
            "ready_turn"
        } else if readable || hangup {
            "readable_turn"
        } else {
            "writable_turn"
        };
        let turn_request = turn_request(
            connection,
            event,
            backend,
            shard_index,
            event_index,
            turn_stage,
        );
        let turn_plan = agent_websocket_event_loop_turn_plan_json(&turn_request)?;
        if !bool_field(turn_plan.get("ok")).unwrap_or(false) {
            turn_failure_count += 1;
        }
        append_wrapped_turn_actions(
            &mut actions,
            &turn_plan,
            connection,
            backend,
            shard_index,
            event_index,
        );
        state_updates.push(connection_state_update(
            &turn_plan,
            connection,
            backend,
            shard_index,
            event_index,
        ));
        turn_results.push(json!({
            "event_index": event_index,
            "worker_key": connection.worker_key,
            "transport": connection.transport,
            "event_loop_token": connection.token,
            "websocket_fd": connection.fd,
            "websocket_turn_state": clone_field(&turn_plan, "websocket_turn_state"),
            "ok": bool_field(turn_plan.get("ok")).unwrap_or(false),
            "turn_plan": turn_plan,
        }));
    }

    let ok = unknown_event_count == 0 && turn_failure_count == 0;
    let state = if unknown_event_count > 0 && known_event_count == 0 {
        "unknown_tokens"
    } else if unknown_event_count > 0 || turn_failure_count > 0 {
        "partial_failure"
    } else if known_event_count > 0 {
        "events_planned"
    } else {
        "idle"
    };

    Ok(batch_payload(
        "event_batch",
        state,
        backend,
        shard_index,
        json!({
            "ok": ok,
            "launch_allowed": true,
            "shard_batch_allowed": ok,
            "expected_concurrent_workers": expected_concurrent_workers,
            "workers_per_shard": workers_per_shard,
            "high_concurrency": high_concurrency,
            "requires_epoll_for_target_scale": false,
            "connection_count": connections.len(),
            "event_count": event_values.len(),
            "known_event_count": known_event_count,
            "unknown_event_count": unknown_event_count,
            "turn_failure_count": turn_failure_count,
            "turn_results": turn_results,
            "connection_state_updates": state_updates,
            "diagnostics": diagnostics,
            "actions": actions,
        }),
    ))
}

#[derive(Debug, Clone)]
struct ShardConnection {
    worker_key: String,
    transport: String,
    token: u64,
    fd: Option<u64>,
    object: Map<String, JsonValue>,
}

fn turn_request(
    connection: &ShardConnection,
    event: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    shard_index: usize,
    event_index: usize,
    turn_stage: &str,
) -> JsonValue {
    let mut request = connection.object.clone();
    request.insert(
        "stage".to_string(),
        JsonValue::String(turn_stage.to_string()),
    );
    request.insert(
        "backend".to_string(),
        JsonValue::String(backend.label().to_string()),
    );
    request.insert("shard_index".to_string(), JsonValue::from(shard_index));
    request.insert(
        "event_loop_token".to_string(),
        JsonValue::from(connection.token),
    );
    if let Some(fd) = connection.fd {
        request.insert("websocket_fd".to_string(), JsonValue::from(fd));
    }
    if connection.transport != "unknown" {
        request.insert(
            "transport".to_string(),
            JsonValue::String(connection.transport.clone()),
        );
    }
    request.insert(
        "worker_key".to_string(),
        JsonValue::String(connection.worker_key.clone()),
    );
    request.insert("event_index".to_string(), JsonValue::from(event_index));
    copy_buffer_alias(&mut request, "remaining_buffer_bytes", "buffer_bytes");
    copy_buffer_alias(&mut request, "remaining_buffer_hex", "buffer_hex");
    for key in [
        "read_bytes",
        "chunk_bytes",
        "read_hex",
        "chunk_hex",
        "read_eof",
        "eof",
        "mask_key",
        "control_mask_key",
        "mask",
        "max_payload_bytes",
        "allow_masked",
        "pending_write_bytes",
        "pending_write_hex",
        "remaining_write_bytes",
        "remaining_write_hex",
        "queued_write_bytes",
        "queued_write_hex",
        "write_bytes",
        "write_hex",
        "frame_bytes",
        "frame_hex",
        "max_write_bytes",
        "set_nonblocking",
    ] {
        if let Some(value) = event.get(key) {
            request.insert(key.to_string(), value.clone());
        }
    }
    let readable = event_bool(event, "readable").unwrap_or(true);
    let writable = event_bool(event, "writable").unwrap_or(false);
    let hangup = event_bool(event, "hangup")
        .or_else(|| event_bool(event, "hup"))
        .unwrap_or(false);
    request.insert("readable".to_string(), JsonValue::Bool(readable));
    request.insert("writable".to_string(), JsonValue::Bool(writable));
    request.insert("hangup".to_string(), JsonValue::Bool(hangup));

    let mut event_payload = event.clone();
    event_payload.insert("token".to_string(), JsonValue::from(connection.token));
    event_payload.insert("readable".to_string(), JsonValue::Bool(readable));
    event_payload.insert("writable".to_string(), JsonValue::Bool(writable));
    event_payload.insert("hangup".to_string(), JsonValue::Bool(hangup));
    if let Some(fd) = connection.fd {
        event_payload.insert("fd".to_string(), JsonValue::from(fd));
    }
    request.insert("event".to_string(), JsonValue::Object(event_payload));
    JsonValue::Object(request)
}

fn append_wrapped_turn_actions(
    actions: &mut Vec<JsonValue>,
    turn_plan: &JsonValue,
    connection: &ShardConnection,
    backend: AgentEventLoopBackend,
    shard_index: usize,
    event_index: usize,
) {
    let Some(turn_actions) = turn_plan.get("actions").and_then(JsonValue::as_array) else {
        return;
    };
    for (turn_action_index, action) in turn_actions.iter().enumerate() {
        actions.push(json!({
            "kind": "websocket_shard_worker_action",
            "backend": backend.label(),
            "shard_index": shard_index,
            "event_index": event_index,
            "turn_action_index": turn_action_index,
            "worker_key": connection.worker_key,
            "transport": connection.transport,
            "event_loop_token": connection.token,
            "websocket_fd": connection.fd,
            "source": "websocket_turn",
            "action": action,
        }));
    }
}

fn connection_state_update(
    turn_plan: &JsonValue,
    connection: &ShardConnection,
    backend: AgentEventLoopBackend,
    shard_index: usize,
    event_index: usize,
) -> JsonValue {
    json!({
        "backend": backend.label(),
        "shard_index": shard_index,
        "event_index": event_index,
        "worker_key": connection.worker_key,
        "transport": connection.transport,
        "event_loop_token": connection.token,
        "websocket_fd": connection.fd,
        "websocket_turn_state": clone_field(turn_plan, "websocket_turn_state"),
        "ok": bool_field(turn_plan.get("ok")).unwrap_or(false),
        "remaining_buffer_bytes": clone_field(turn_plan, "remaining_buffer_bytes"),
        "remaining_buffer_hex": clone_field(turn_plan, "remaining_buffer_hex"),
        "pending_write_byte_count": clone_field(turn_plan, "pending_write_byte_count"),
        "pending_write_bytes": clone_field(turn_plan, "pending_write_bytes"),
        "pending_write_hex": clone_field(turn_plan, "pending_write_hex"),
        "remaining_write_byte_count": clone_field(turn_plan, "remaining_write_byte_count"),
        "remaining_write_bytes": clone_field(turn_plan, "remaining_write_bytes"),
        "remaining_write_hex": clone_field(turn_plan, "remaining_write_hex"),
        "should_keep_registered": bool_field(turn_plan.get("should_keep_registered")).unwrap_or(false),
        "should_register_read_write": bool_field(turn_plan.get("should_register_read_write")).unwrap_or(false),
        "should_unregister": bool_field(turn_plan.get("should_unregister")).unwrap_or(false),
        "should_close_websocket": bool_field(turn_plan.get("should_close_websocket")).unwrap_or(false),
        "should_reconnect": bool_field(turn_plan.get("should_reconnect")).unwrap_or(false),
    })
}

fn batch_payload(
    stage: &str,
    state: &str,
    backend: AgentEventLoopBackend,
    shard_index: usize,
    payload: JsonValue,
) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "websocket_shard_event_batch_contract".to_string(),
        JsonValue::String(WEBSOCKET_SHARD_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "websocket_shard_event_batch_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "transport".to_string(),
        JsonValue::String("websocket".to_string()),
    );
    object.insert(
        "backend".to_string(),
        JsonValue::String(backend.label().to_string()),
    );
    object.insert("shard_index".to_string(), JsonValue::from(shard_index));
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_websocket_shard_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_websocket_event_loop_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_websocket_turn_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_fallback_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(object)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket shard event-batch request must be an object.".to_string())
}

fn array_field<'a>(
    object: &'a Map<String, JsonValue>,
    keys: &[&str],
) -> Result<&'a Vec<JsonValue>, String> {
    for key in keys {
        if let Some(value) = object.get(*key) {
            return value.as_array().ok_or_else(|| {
                format!("WebSocket shard event-batch field `{key}` must be an array.")
            });
        }
    }
    Ok(empty_array())
}

fn empty_array() -> &'static Vec<JsonValue> {
    use std::sync::OnceLock;

    static EMPTY: OnceLock<Vec<JsonValue>> = OnceLock::new();
    EMPTY.get_or_init(Vec::new)
}

fn backend_from_request(object: &Map<String, JsonValue>) -> Result<AgentEventLoopBackend, String> {
    match clean_text(object.get("backend")).or_else(|| clean_text(object.get("event_loop_backend")))
    {
        Some(raw) => AgentEventLoopBackend::from_label(&raw)
            .ok_or_else(|| format!("invalid ait-agent event-loop backend `{raw}`")),
        None => Ok(AgentEventLoopBackend::current_platform_default()),
    }
}

fn default_workers_per_shard(backend: AgentEventLoopBackend) -> usize {
    match backend {
        AgentEventLoopBackend::LinuxEpoll => DEFAULT_WORKERS_PER_EPOLL_SHARD,
        AgentEventLoopBackend::PortablePoll => DEFAULT_WORKERS_PER_POLL_SHARD,
    }
}

fn worker_key(object: &Map<String, JsonValue>, token: u64, connection_index: usize) -> String {
    clean_text(object.get("worker_key"))
        .or_else(|| clean_text(object.get("key")))
        .or_else(|| clean_text(object.get("worker_name")))
        .unwrap_or_else(|| format!("websocket/{token}/{connection_index}"))
}

fn transport_label(object: &Map<String, JsonValue>) -> Option<String> {
    let transport = clean_text(object.get("transport"))
        .or_else(|| clean_text(object.get("websocket_transport")))
        .or_else(|| nested_text(object, "worker_lease", "transport"))?;
    match transport
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "slack" | "slack_socket_mode" | "socket_mode" => Some("slack".to_string()),
        "discord" | "discord_gateway" | "gateway" => Some("discord".to_string()),
        other => Some(other.to_string()),
    }
}

fn event_loop_token(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(object.get("event_loop_token"))
        .or_else(|| optional_u64(object.get("token")))
        .or_else(|| nested_u64(object, "worker_lease", "token"))
        .or_else(|| nested_u64(object, "event_loop_registration", "token"))
        .or_else(|| nested_u64(object, "event", "token"))
}

fn websocket_fd(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(object.get("websocket_fd"))
        .or_else(|| optional_u64(object.get("fd")))
        .or_else(|| nested_u64(object, "event_loop_registration", "fd"))
        .or_else(|| nested_u64(object, "event", "fd"))
}

fn copy_buffer_alias(request: &mut Map<String, JsonValue>, from: &str, to: &str) {
    if request.contains_key(to) {
        return;
    }
    if let Some(value) = request.get(from).cloned() {
        request.insert(to.to_string(), value);
    }
}

fn clone_field(value: &JsonValue, field: &str) -> JsonValue {
    value.get(field).cloned().unwrap_or(JsonValue::Null)
}

fn event_bool(object: &Map<String, JsonValue>, key: &str) -> Option<bool> {
    optional_bool(object.get(key)).or_else(|| nested_bool(object, "event", key))
}

fn nested_text(object: &Map<String, JsonValue>, parent: &str, key: &str) -> Option<String> {
    object
        .get(parent)
        .and_then(JsonValue::as_object)
        .and_then(|nested| clean_text(nested.get(key)))
}

fn nested_bool(object: &Map<String, JsonValue>, parent: &str, key: &str) -> Option<bool> {
    object
        .get(parent)
        .and_then(JsonValue::as_object)
        .and_then(|nested| optional_bool(nested.get(key)))
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
    optional_bool(value)
}

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
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

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    optional_u64(value).and_then(|value| usize::try_from(value).ok())
}
