use std::io;
use std::time::Duration;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::{
    agent_websocket_shard_event_batch_plan_json, execute_agent_websocket_registration_actions,
    AgentEvent, AgentEventLoopBackend, AgentEventLoopBackendPort, AgentEventLoopPollPort,
    AgentEventLoopRegistrationPort,
};

const MIGRATION_STAGE: &str = "rust_agent_websocket_reactor_tick_execution";
const REACTOR_TICK_CONTRACT: &str = "ait_agent_core.event_loop.WebSocketReactorTick.v1";

pub fn execute_agent_websocket_reactor_tick<E>(
    event_loop: &mut E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: AgentEventLoopBackendPort + AgentEventLoopPollPort + AgentEventLoopRegistrationPort + ?Sized,
{
    let object = request_object(request)?;
    let backend = event_loop.backend();
    if let Some(request_backend) = request_backend(object)? {
        if request_backend != backend {
            return Ok(configuration_error_payload(
                object,
                backend,
                "backend_mismatch",
                &format!(
                    "WebSocket reactor tick request backend `{}` does not match event-loop backend `{}`.",
                    request_backend.label(),
                    backend.label()
                ),
            ));
        }
    }
    let (poll_timeout, poll_timeout_ms) = match poll_timeout(object) {
        Ok(timeout) => timeout,
        Err(reason) => {
            return Ok(configuration_error_payload(
                object,
                backend,
                "configuration_error",
                &reason,
            ));
        }
    };

    let events = match event_loop.poll(poll_timeout) {
        Ok(events) => events,
        Err(err) => return Ok(poll_error_payload(object, backend, poll_timeout_ms, err)),
    };
    let event_values = events_json(&events);
    let shard_request = shard_batch_request(object, backend, event_values.clone());
    let shard_plan = agent_websocket_shard_event_batch_plan_json(&shard_request)?;
    let registration_request = registration_action_request(object, backend, &shard_plan);
    let registration_result =
        execute_agent_websocket_registration_actions(event_loop, &registration_request)?;

    let shard_ok = bool_field(shard_plan.get("ok")).unwrap_or(false);
    let registration_ok = bool_field(registration_result.get("ok")).unwrap_or(false);
    let shard_state = clean_text(shard_plan.get("websocket_shard_event_batch_state"))
        .unwrap_or_else(|| "unknown".to_string());
    let registration_state = clean_text(registration_result.get("websocket_registration_state"))
        .unwrap_or_else(|| "unknown".to_string());
    let tick_state = tick_state(&events, shard_ok, &shard_state, registration_ok);
    let diagnostics = collect_diagnostics(&shard_plan, &registration_result);

    Ok(base_payload(
        object,
        "execute",
        tick_state,
        backend,
        json!({
            "ok": shard_ok && registration_ok,
            "executed": true,
            "poll_timeout_ms": poll_timeout_ms,
            "poll_event_count": events.len(),
            "events": event_values,
            "shard_ok": shard_ok,
            "shard_state": shard_state,
            "known_event_count": clone_field(&shard_plan, "known_event_count"),
            "unknown_event_count": clone_field(&shard_plan, "unknown_event_count"),
            "turn_failure_count": clone_field(&shard_plan, "turn_failure_count"),
            "registration_ok": registration_ok,
            "registration_state": registration_state,
            "registration_operation_count": clone_field(&registration_result, "operation_count"),
            "registration_applied_operation_count": clone_field(&registration_result, "applied_operation_count"),
            "registration_skipped_action_count": clone_field(&registration_result, "skipped_action_count"),
            "shard_batch_plan": shard_plan,
            "registration_result": registration_result,
            "diagnostics": diagnostics,
            "actions": [],
        }),
    ))
}

fn tick_state(
    events: &[AgentEvent],
    shard_ok: bool,
    shard_state: &str,
    registration_ok: bool,
) -> &'static str {
    if !shard_ok && shard_state == "backend_requires_epoll" {
        return "backend_requires_epoll";
    }
    if !shard_ok {
        return "shard_error";
    }
    if !registration_ok {
        return "registration_error";
    }
    if events.is_empty() {
        "idle"
    } else {
        "tick_applied"
    }
}

fn shard_batch_request(
    object: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    events: Vec<JsonValue>,
) -> JsonValue {
    let mut request = object.clone();
    request.insert(
        "stage".to_string(),
        JsonValue::String("event_batch".to_string()),
    );
    request.insert(
        "backend".to_string(),
        JsonValue::String(backend.label().to_string()),
    );
    request.insert("events".to_string(), JsonValue::Array(events));
    JsonValue::Object(request)
}

fn registration_action_request(
    object: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    shard_plan: &JsonValue,
) -> JsonValue {
    let mut request = Map::new();
    request.insert(
        "backend".to_string(),
        JsonValue::String(backend.label().to_string()),
    );
    if let Some(shard_index) = optional_u64(object.get("shard_index"))
        .or_else(|| optional_u64(shard_plan.get("shard_index")))
    {
        request.insert("shard_index".to_string(), JsonValue::from(shard_index));
    }
    request.insert("shard_batch_plan".to_string(), shard_plan.clone());
    JsonValue::Object(request)
}

fn events_json(events: &[AgentEvent]) -> Vec<JsonValue> {
    events
        .iter()
        .map(|event| {
            json!({
                "event_loop_token": event.token,
                "readable": event.readable,
                "writable": event.writable,
                "hangup": event.hangup,
            })
        })
        .collect()
}

fn configuration_error_payload(
    object: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    state: &str,
    reason: &str,
) -> JsonValue {
    base_payload(
        object,
        "execute",
        state,
        backend,
        json!({
            "ok": false,
            "executed": false,
            "poll_timeout_ms": 0,
            "poll_event_count": 0,
            "events": [],
            "shard_ok": false,
            "registration_ok": false,
            "shard_batch_plan": JsonValue::Null,
            "registration_result": JsonValue::Null,
            "error": reason,
            "diagnostics": [reason],
            "actions": [{
                "kind": "diagnose_websocket_reactor_tick_configuration_error",
                "reason": reason,
            }],
        }),
    )
}

fn poll_error_payload(
    object: &Map<String, JsonValue>,
    backend: AgentEventLoopBackend,
    poll_timeout_ms: u64,
    err: io::Error,
) -> JsonValue {
    let reason = format!("WebSocket reactor tick poll failed: {err}");
    base_payload(
        object,
        "execute",
        "poll_error",
        backend,
        json!({
            "ok": false,
            "executed": false,
            "poll_timeout_ms": poll_timeout_ms,
            "poll_event_count": 0,
            "events": [],
            "shard_ok": false,
            "registration_ok": false,
            "shard_batch_plan": JsonValue::Null,
            "registration_result": JsonValue::Null,
            "error": reason,
            "diagnostics": [reason],
            "actions": [{
                "kind": "diagnose_websocket_reactor_tick_poll_error",
                "io_error_kind": format!("{:?}", err.kind()),
            }],
        }),
    )
}

fn base_payload(
    object: &Map<String, JsonValue>,
    stage: &str,
    state: &str,
    backend: AgentEventLoopBackend,
    payload: JsonValue,
) -> JsonValue {
    let mut object_out = payload.as_object().cloned().unwrap_or_default();
    object_out.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object_out.insert(
        "websocket_reactor_tick_contract".to_string(),
        JsonValue::String(REACTOR_TICK_CONTRACT.to_string()),
    );
    object_out.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object_out.insert(
        "websocket_reactor_tick_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object_out.insert(
        "backend".to_string(),
        JsonValue::String(backend.label().to_string()),
    );
    if let Some(shard_index) = optional_u64(object.get("shard_index")) {
        object_out.insert("shard_index".to_string(), JsonValue::from(shard_index));
    }
    object_out.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object_out.insert(
        "python_websocket_reactor_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object_out.insert(
        "python_websocket_event_loop_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object_out.insert(
        "python_websocket_shard_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object_out.insert(
        "python_websocket_registration_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object_out.insert(
        "python_fallback_allowed".to_string(),
        JsonValue::Bool(false),
    );
    JsonValue::Object(object_out)
}

fn collect_diagnostics(shard_plan: &JsonValue, registration_result: &JsonValue) -> JsonValue {
    let mut diagnostics = Vec::new();
    extend_diagnostics(&mut diagnostics, shard_plan.get("diagnostics"));
    extend_diagnostics(&mut diagnostics, registration_result.get("diagnostics"));
    JsonValue::Array(diagnostics)
}

fn extend_diagnostics(target: &mut Vec<JsonValue>, diagnostics: Option<&JsonValue>) {
    let Some(diagnostics) = diagnostics else {
        return;
    };
    if let Some(items) = diagnostics.as_array() {
        target.extend(items.iter().cloned());
    } else if !diagnostics.is_null() {
        target.push(diagnostics.clone());
    }
}

fn request_backend(
    object: &Map<String, JsonValue>,
) -> Result<Option<AgentEventLoopBackend>, String> {
    let Some(raw) =
        clean_text(object.get("backend")).or_else(|| clean_text(object.get("event_loop_backend")))
    else {
        return Ok(None);
    };
    AgentEventLoopBackend::from_label(&raw)
        .map(Some)
        .ok_or_else(|| format!("invalid ait-agent event-loop backend `{raw}`"))
}

fn poll_timeout(object: &Map<String, JsonValue>) -> Result<(Duration, u64), String> {
    for key in ["poll_timeout_ms", "timeout_ms", "timeout_millis"] {
        if let Some(value) = object.get(key) {
            let millis = required_u64(value, key)?;
            return Ok((Duration::from_millis(millis), millis));
        }
    }
    if let Some(value) = object
        .get("poll_timeout_seconds")
        .or_else(|| object.get("timeout_seconds"))
    {
        let seconds = required_u64(value, "poll_timeout_seconds")?;
        let millis = seconds.saturating_mul(1_000);
        return Ok((Duration::from_millis(millis), millis));
    }
    Ok((Duration::from_millis(0), 0))
}

fn required_u64(value: &JsonValue, key: &str) -> Result<u64, String> {
    match value {
        JsonValue::Number(number) => number.as_u64().ok_or_else(|| {
            format!("WebSocket reactor tick field `{key}` must be a non-negative integer.")
        }),
        JsonValue::String(text) => text.trim().parse::<u64>().map_err(|_| {
            format!("WebSocket reactor tick field `{key}` must be a non-negative integer.")
        }),
        JsonValue::Bool(_) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => Err(
            format!("WebSocket reactor tick field `{key}` must be a non-negative integer."),
        ),
    }
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket reactor tick request must be an object.".to_string())
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

fn clone_field(value: &JsonValue, field: &str) -> JsonValue {
    value.get(field).cloned().unwrap_or(JsonValue::Null)
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
