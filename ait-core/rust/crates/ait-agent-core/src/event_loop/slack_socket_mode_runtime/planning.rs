use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::super::agent_slack_socket_mode_transaction_plan_json;

const MIGRATION_STAGE: &str = "rust_agent_slack_socket_mode_runtime";
const SOCKET_MODE_RUNTIME_CONTRACT: &str = "ait_agent_core.event_loop.SlackSocketModeRuntime.v1";
const DEFAULT_SLACK_API_BASE_URL: &str = "https://slack.com/api";
const DEFAULT_SLACK_HTTP_USER_AGENT: &str = "curl/8.7.1";
const DEFAULT_PING_INTERVAL_SECONDS: f64 = 30.0;
const DEFAULT_PONG_TIMEOUT_SECONDS: f64 = 10.0;

pub trait SlackSocketModeRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackSocketModeRuntimePlanner;

impl SlackSocketModeRuntimePlanner for DefaultSlackSocketModeRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_socket_mode_runtime_json(request)
    }
}

pub fn agent_slack_socket_mode_runtime_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_slack_socket_mode_runtime_planner(&DefaultSlackSocketModeRuntimePlanner, request)
}

pub fn plan_with_slack_socket_mode_runtime_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: SlackSocketModeRuntimePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_socket_mode_runtime_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "payload".to_string());

    match stage.as_str() {
        "connection_open_request" | "open_connection_request" | "apps_connections_open" => {
            plan_connection_open_request(object)
        }
        "connect" | "socket_url" | "connection_ready" => Ok(plan_connect(object)),
        "tick" | "heartbeat_tick" => Ok(plan_tick(object)),
        "payload" | "socket_payload" | "message" => plan_payload(object),
        "error_recovery" | "error" | "disconnect" => Ok(plan_error_recovery(object)),
        other => Err(format!(
            "unsupported Slack Socket Mode runtime stage: {other}"
        )),
    }
}

fn plan_connection_open_request(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let app_token = required_text(
        object,
        &["app_token", "slack_app_token", "socket_mode_app_token"],
        "Missing Slack app token. Set AIT_SLACK_APP_TOKEN or SLACK_APP_TOKEN.",
    )?;
    let api_base_url = clean_text(object.get("slack_api_base_url"))
        .or_else(|| clean_text(object.get("api_base_url")))
        .unwrap_or_else(|| DEFAULT_SLACK_API_BASE_URL.to_string());
    let user_agent = clean_text(object.get("slack_http_user_agent"))
        .or_else(|| clean_text(object.get("user_agent")))
        .unwrap_or_else(|| DEFAULT_SLACK_HTTP_USER_AGENT.to_string());
    let timeout_seconds = optional_f64(object.get("request_timeout_seconds"))
        .or_else(|| optional_f64(object.get("timeout_seconds")));
    let url = format!(
        "{}/apps.connections.open",
        api_base_url.trim_end_matches('/')
    );
    let request = json!({
        "method": "POST",
        "url": url,
        "payload": {},
        "headers": {
            "Authorization": format!("Bearer {app_token}"),
            "Content-Type": "application/json; charset=utf-8",
            "User-Agent": user_agent,
        },
        "timeout_seconds": timeout_seconds.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "allow_retry": true,
        "retry_attempts": 4,
        "retry_base_delay_seconds": 0.75,
    });

    Ok(base_payload(
        "connection_open_request",
        "request_planned",
        json!({
            "ok": true,
            "should_execute": true,
            "should_open_connection": true,
            "request": request,
            "method": "POST",
            "url": url,
            "headers": request["headers"].clone(),
            "actions": [
                {
                    "kind": "open_socket_mode_connection",
                    "request": request,
                }
            ],
        }),
    ))
}

fn plan_connect(object: &Map<String, JsonValue>) -> JsonValue {
    let backend = runtime_backend(object);
    let shard_index = optional_usize(object.get("shard_index"))
        .or_else(|| nested_usize(object, "worker_lease", "shard_index"))
        .unwrap_or(0);
    let token = optional_u64(object.get("event_loop_token"))
        .or_else(|| optional_u64(object.get("token")))
        .or_else(|| nested_u64(object, "worker_lease", "token"));
    let websocket_fd =
        optional_u64(object.get("websocket_fd")).or_else(|| optional_u64(object.get("fd")));
    let Some(socket_url) = socket_url_from_object(object) else {
        return base_payload(
            "connect",
            "awaiting_socket_url",
            json!({
                "ok": false,
                "should_open_connection": true,
                "should_connect_websocket": false,
                "should_register_event_loop": false,
                "backend": backend,
                "shard_index": shard_index,
                "event_loop_token": token.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "websocket_fd": websocket_fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "socket_url": JsonValue::Null,
                "event_loop_registration": JsonValue::Null,
                "error": "Slack Socket Mode connection info did not include a websocket URL.",
                "actions": [
                    {
                        "kind": "open_socket_mode_connection",
                        "reason": "missing_socket_url",
                    }
                ],
            }),
        );
    };

    let event_loop_registration =
        event_loop_registration(&backend, shard_index, token, websocket_fd);
    let should_register_event_loop = event_loop_registration.is_object();
    let mut actions = vec![json!({
        "kind": "connect_socket_mode_websocket",
        "socket_url": socket_url.clone(),
    })];
    if should_register_event_loop {
        actions.push(json!({
            "kind": "register_websocket_readable",
            "registration": event_loop_registration,
        }));
    }

    base_payload(
        "connect",
        "websocket_ready",
        json!({
            "ok": true,
            "should_open_connection": false,
            "should_connect_websocket": true,
            "should_register_event_loop": should_register_event_loop,
            "backend": backend,
            "shard_index": shard_index,
            "event_loop_token": token.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "websocket_fd": websocket_fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "socket_url": socket_url,
            "event_loop_registration": event_loop_registration,
            "actions": actions,
        }),
    )
}

fn plan_tick(object: &Map<String, JsonValue>) -> JsonValue {
    let now = optional_f64(object.get("now_monotonic_seconds")).unwrap_or(0.0);
    let ping_interval_seconds = optional_f64(object.get("ping_interval_seconds"))
        .or_else(|| optional_f64(object.get("heartbeat_interval_seconds")))
        .unwrap_or(DEFAULT_PING_INTERVAL_SECONDS)
        .max(0.001);
    let pong_timeout_seconds = optional_f64(object.get("pong_timeout_seconds"))
        .unwrap_or(DEFAULT_PONG_TIMEOUT_SECONDS)
        .max(0.001);
    let next_ping_at = optional_f64(object.get("next_ping_at")).unwrap_or(now);
    let pong_pending = optional_bool(object.get("pong_pending"))
        .or_else(|| optional_bool(object.get("awaiting_pong")))
        .unwrap_or(false);
    let ping_sent_at = optional_f64(object.get("ping_sent_at")).unwrap_or(now);
    let ping_id = clean_text(object.get("ping_id"));

    if pong_pending {
        let elapsed = (now - ping_sent_at).max(0.0);
        if elapsed >= pong_timeout_seconds {
            return base_payload(
                "tick",
                "pong_timeout",
                json!({
                    "ok": false,
                    "should_wait": false,
                    "should_send_ping": false,
                    "should_reconnect": true,
                    "pong_pending": true,
                    "ping_sent_at": ping_sent_at,
                    "pong_timeout_seconds": pong_timeout_seconds,
                    "reconnect_reason": "socket_mode_pong_timeout",
                    "error": "Slack Socket Mode websocket pong timed out.",
                    "actions": [
                        {
                            "kind": "reconnect_socket_mode",
                            "reason": "socket_mode_pong_timeout",
                        }
                    ],
                }),
            );
        }
        return base_payload(
            "tick",
            "waiting_for_pong",
            json!({
                "ok": true,
                "should_wait": true,
                "wait_seconds": (pong_timeout_seconds - elapsed).clamp(0.0, 1.0),
                "should_send_ping": false,
                "should_reconnect": false,
                "pong_pending": true,
                "ping_sent_at": ping_sent_at,
                "next_ping_at": next_ping_at,
                "actions": [],
            }),
        );
    }

    if now < next_ping_at {
        return base_payload(
            "tick",
            "waiting",
            json!({
                "ok": true,
                "should_wait": true,
                "wait_seconds": (next_ping_at - now).clamp(0.0, 1.0),
                "should_send_ping": false,
                "should_reconnect": false,
                "pong_pending": false,
                "next_ping_at": next_ping_at,
                "actions": [],
            }),
        );
    }

    let websocket_ping = json!({
        "kind": "websocket_ping",
        "ping_id": optional_string_json(ping_id.as_deref()),
    });
    base_payload(
        "tick",
        "ping_planned",
        json!({
            "ok": true,
            "should_wait": false,
            "should_send_ping": true,
            "should_reconnect": false,
            "pong_pending": true,
            "ping_sent_at": now,
            "next_ping_at": now + ping_interval_seconds,
            "ping_interval_seconds": ping_interval_seconds,
            "pong_timeout_seconds": pong_timeout_seconds,
            "websocket_ping": websocket_ping,
            "actions": [
                {
                    "kind": "send_websocket_ping",
                    "websocket_ping": websocket_ping,
                }
            ],
        }),
    )
}

fn plan_payload(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let payload = runtime_payload_object(object)?;
    let payload_type = clean_text(payload.get("type")).unwrap_or_default();

    match payload_type.as_str() {
        "hello" => Ok(base_payload(
            "payload",
            "ready",
            json!({
                "ok": true,
                "payload_type": "hello",
                "should_mark_ready": true,
                "should_plan_transaction": false,
                "num_connections": optional_i64(payload.get("num_connections")).map(JsonValue::from).unwrap_or(JsonValue::Null),
                "actions": [
                    {
                        "kind": "mark_socket_mode_ready",
                        "payload": JsonValue::Object(payload.clone()),
                    }
                ],
            }),
        )),
        "disconnect" => Ok(base_payload(
            "payload",
            "disconnect_requested",
            json!({
                "ok": true,
                "payload_type": "disconnect",
                "should_plan_transaction": false,
                "should_reconnect": true,
                "reconnect_reason": clean_text(payload.get("reason")).unwrap_or_else(|| "socket_mode_disconnect".to_string()),
                "actions": [
                    {
                        "kind": "reconnect_socket_mode",
                        "reason": clean_text(payload.get("reason")).unwrap_or_else(|| "socket_mode_disconnect".to_string()),
                    }
                ],
            }),
        )),
        "pong" => Ok(base_payload(
            "payload",
            "pong_acknowledged",
            json!({
                "ok": true,
                "payload_type": "pong",
                "should_plan_transaction": false,
                "pong_pending": false,
                "actions": [
                    {
                        "kind": "mark_socket_mode_pong",
                    }
                ],
            }),
        )),
        _ if payload.contains_key("envelope_id") => plan_transaction_payload(object, payload),
        _ => Ok(base_payload(
            "payload",
            "ignored_payload",
            json!({
                "ok": true,
                "payload_type": optional_string_json(Some(&payload_type)),
                "should_plan_transaction": false,
                "should_ack_socket_envelope": false,
                "should_reconnect": false,
                "actions": [],
            }),
        )),
    }
}

fn plan_transaction_payload(
    object: &Map<String, JsonValue>,
    payload: &Map<String, JsonValue>,
) -> Result<JsonValue, String> {
    let transaction_request = transaction_request(object, payload);
    let transaction_plan = agent_slack_socket_mode_transaction_plan_json(&transaction_request)
        .map_err(|err| format!("Slack Socket Mode runtime transaction planning failed: {err}"))?;
    let should_ack_socket_envelope =
        bool_field(transaction_plan.get("should_ack_socket_envelope")).unwrap_or(false);
    let should_handle_command =
        bool_field(transaction_plan.get("should_handle_command")).unwrap_or(false);
    let should_submit_turn =
        bool_field(transaction_plan.get("should_submit_turn")).unwrap_or(false);
    let actions = runtime_transaction_actions(&transaction_plan);

    Ok(base_payload(
        "payload",
        "transaction_planned",
        json!({
            "ok": clone_field(&transaction_plan, "ok"),
            "accepted": clone_field(&transaction_plan, "accepted"),
            "payload_type": clean_text(payload.get("type")).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "envelope_id": clean_text(payload.get("envelope_id")).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "should_plan_transaction": true,
            "transaction_request": transaction_request,
            "transaction_plan": transaction_plan,
            "ack_response": clone_field(&transaction_plan, "ack_response"),
            "websocket_ack_response": clone_field(&transaction_plan, "websocket_ack_response"),
            "should_ack_socket_envelope": should_ack_socket_envelope,
            "should_execute_websocket_ack": should_ack_socket_envelope,
            "should_handle_command": should_handle_command,
            "should_submit_turn": should_submit_turn,
            "should_reconnect": false,
            "actions": actions,
        }),
    ))
}

fn plan_error_recovery(object: &Map<String, JsonValue>) -> JsonValue {
    let error_text = clean_text(object.get("error_message"))
        .or_else(|| clean_text(object.get("error")))
        .or_else(|| clean_text(object.get("exception")))
        .unwrap_or_default();
    let retry_attempt = optional_usize(object.get("retry_attempt")).unwrap_or(0);
    let retry_base_delay_seconds = optional_f64(object.get("retry_base_delay_seconds"))
        .unwrap_or(1.0)
        .max(0.001);
    let reconnect_delay_seconds =
        (retry_base_delay_seconds * 2_f64.powi(retry_attempt.min(8) as i32)).min(60.0);
    let auth_failure = is_auth_failure(&error_text);

    base_payload(
        "error_recovery",
        if auth_failure {
            "fatal_auth_error"
        } else {
            "reconnect_planned"
        },
        json!({
            "ok": !auth_failure,
            "error": error_text,
            "retry_attempt": retry_attempt,
            "reconnect_delay_seconds": if auth_failure {
                JsonValue::Null
            } else {
                JsonValue::from(reconnect_delay_seconds)
            },
            "should_reconnect": !auth_failure,
            "should_stop_runtime": auth_failure,
            "actions": if auth_failure {
                json!([
                    {
                        "kind": "stop_socket_mode_runtime",
                        "reason": "auth_failure",
                    }
                ])
            } else {
                json!([
                    {
                        "kind": "reconnect_socket_mode",
                        "reason": "runtime_error",
                        "delay_seconds": reconnect_delay_seconds,
                    }
                ])
            },
        }),
    )
}

fn runtime_transaction_actions(transaction_plan: &JsonValue) -> JsonValue {
    let mut actions = Vec::new();
    if bool_field(transaction_plan.get("should_execute_websocket_ack")).unwrap_or(false) {
        actions.push(json!({
            "kind": "execute_websocket_ack",
            "response": clone_field(transaction_plan, "websocket_ack_response"),
            "execute_before_command_side_effects": true,
            "transaction_action": first_transaction_action(transaction_plan, "ack_socket_envelope"),
        }));
    }
    if bool_field(transaction_plan.get("should_handle_command")).unwrap_or(false) {
        actions.push(json!({
            "kind": "dispatch_socket_mode_command",
            "transaction_plan": transaction_plan,
            "should_submit_turn": bool_field(transaction_plan.get("should_submit_turn")).unwrap_or(false),
            "should_start_background_reply": bool_field(transaction_plan.get("should_start_background_reply")).unwrap_or(false),
            "should_execute_inline_reply": bool_field(transaction_plan.get("should_execute_inline_reply")).unwrap_or(false),
        }));
    }
    JsonValue::Array(actions)
}

fn first_transaction_action(transaction_plan: &JsonValue, kind: &str) -> JsonValue {
    transaction_plan
        .get("actions")
        .and_then(JsonValue::as_array)
        .and_then(|actions| {
            actions
                .iter()
                .find(|action| clean_text(action.get("kind")).as_deref() == Some(kind))
        })
        .cloned()
        .unwrap_or(JsonValue::Null)
}

fn transaction_request(
    object: &Map<String, JsonValue>,
    payload: &Map<String, JsonValue>,
) -> JsonValue {
    let mut request = object.clone();
    request.insert("envelope".to_string(), JsonValue::Object(payload.clone()));
    request.remove("payload");
    JsonValue::Object(request)
}

fn event_loop_registration(
    backend: &str,
    shard_index: usize,
    token: Option<u64>,
    websocket_fd: Option<u64>,
) -> JsonValue {
    match (token, websocket_fd) {
        (Some(token), Some(fd)) => json!({
            "backend": backend,
            "shard_index": shard_index,
            "token": token,
            "fd": fd,
            "interest": "readable",
        }),
        _ => JsonValue::Null,
    }
}

fn socket_url_from_object(object: &Map<String, JsonValue>) -> Option<String> {
    clean_text(object.get("socket_url"))
        .or_else(|| clean_text(object.get("websocket_url")))
        .or_else(|| clean_text(object.get("url")))
        .or_else(|| nested_text(object, "connection_info", "url"))
        .or_else(|| nested_text(object, "connection", "url"))
        .filter(|url| url.starts_with("wss://") || url.starts_with("ws://"))
}

fn runtime_payload_object(
    object: &Map<String, JsonValue>,
) -> Result<&Map<String, JsonValue>, String> {
    object
        .get("payload")
        .and_then(JsonValue::as_object)
        .or_else(|| {
            if object.contains_key("type") || object.contains_key("envelope_id") {
                Some(object)
            } else {
                None
            }
        })
        .ok_or_else(|| "Slack Socket Mode runtime payload must be a JSON object.".to_string())
}

fn runtime_backend(object: &Map<String, JsonValue>) -> String {
    clean_text(object.get("backend"))
        .or_else(|| nested_text(object, "worker_lease", "backend"))
        .or_else(|| nested_text(object, "admission_plan", "backend"))
        .unwrap_or_else(|| "portable_poll".to_string())
}

fn is_auth_failure(error_text: &str) -> bool {
    let lower = error_text.to_ascii_lowercase();
    [
        "invalid_auth",
        "not_authed",
        "account_inactive",
        "token_revoked",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn base_payload(stage: &str, state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "slack_socket_mode_runtime_contract".to_string(),
        JsonValue::String(SOCKET_MODE_RUNTIME_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "transport".to_string(),
        JsonValue::String("slack".to_string()),
    );
    object.insert(
        "socket_mode_runtime_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_socket_mode_runtime_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_socket_mode_sequencing_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_websocket_event_loop_allowed".to_string(),
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
        .ok_or_else(|| "Slack Socket Mode runtime request must be an object.".to_string())
}

fn required_text(
    object: &Map<String, JsonValue>,
    keys: &[&str],
    error: &str,
) -> Result<String, String> {
    keys.iter()
        .find_map(|key| clean_text(object.get(*key)))
        .ok_or_else(|| error.to_string())
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

fn nested_usize(object: &Map<String, JsonValue>, parent: &str, key: &str) -> Option<usize> {
    object
        .get(parent)
        .and_then(JsonValue::as_object)
        .and_then(|nested| optional_usize(nested.get(key)))
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

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    optional_u64(value).and_then(|value| usize::try_from(value).ok())
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => text.trim().parse::<f64>().ok(),
        JsonValue::Bool(true) => Some(1.0),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn optional_string_json(value: Option<&str>) -> JsonValue {
    value
        .filter(|text| !text.trim().is_empty())
        .map(|text| JsonValue::String(text.to_string()))
        .unwrap_or(JsonValue::Null)
}

fn bool_field(value: Option<&JsonValue>) -> Option<bool> {
    optional_bool(value)
}

fn clone_field(object: &JsonValue, key: &str) -> JsonValue {
    object.get(key).cloned().unwrap_or(JsonValue::Null)
}
