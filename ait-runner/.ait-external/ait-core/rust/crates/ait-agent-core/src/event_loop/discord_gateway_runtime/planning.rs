use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_discord_gateway_runtime";
const GATEWAY_RUNTIME_CONTRACT: &str = "ait_agent_core.event_loop.DiscordGatewayRuntime.v1";
const DEFAULT_DISCORD_API_BASE_URL: &str = "https://discord.com/api/v10";
const DEFAULT_DISCORD_HTTP_USER_AGENT: &str = "curl/8.7.1";
const DISCORD_GUILD_MESSAGES_INTENT: i64 = 1 << 9;
const DISCORD_DIRECT_MESSAGES_INTENT: i64 = 1 << 12;
const DISCORD_MESSAGE_CONTENT_INTENT: i64 = 1 << 15;
const DEFAULT_DISCORD_GATEWAY_INTENTS: i64 =
    DISCORD_GUILD_MESSAGES_INTENT | DISCORD_DIRECT_MESSAGES_INTENT | DISCORD_MESSAGE_CONTENT_INTENT;

pub trait DiscordGatewayRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDiscordGatewayRuntimePlanner;

impl DiscordGatewayRuntimePlanner for DefaultDiscordGatewayRuntimePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_gateway_runtime_json(request)
    }
}

pub fn agent_discord_gateway_runtime_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    plan_with_discord_gateway_runtime_planner(&DefaultDiscordGatewayRuntimePlanner, request)
}

pub fn plan_with_discord_gateway_runtime_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: DiscordGatewayRuntimePlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_gateway_runtime_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "gateway_url".to_string());

    match stage.as_str() {
        "gateway_info_request" => plan_gateway_info_request(object),
        "gateway_url" | "socket_url" => Ok(plan_gateway_url(object)),
        "handshake" | "hello" => plan_handshake(object),
        "tick" | "heartbeat_tick" => Ok(plan_heartbeat_tick(object)),
        "payload" | "gateway_payload" => plan_gateway_payload(object),
        "error_recovery" | "gateway_error" => Ok(plan_error_recovery(object)),
        other => Err(format!(
            "unsupported Discord gateway runtime stage: {other}"
        )),
    }
}

fn plan_gateway_info_request(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let token = required_text(
        object,
        &["bot_token", "token", "discord_bot_token"],
        "Missing Discord bot token. Set AIT_DISCORD_BOT_TOKEN or DISCORD_BOT_TOKEN.",
    )?;
    let api_base_url = clean_text(object.get("discord_api_base_url"))
        .or_else(|| clean_text(object.get("api_base_url")))
        .unwrap_or_else(|| DEFAULT_DISCORD_API_BASE_URL.to_string());
    let user_agent = clean_text(object.get("discord_http_user_agent"))
        .or_else(|| clean_text(object.get("user_agent")))
        .unwrap_or_else(|| DEFAULT_DISCORD_HTTP_USER_AGENT.to_string());
    let timeout_seconds = optional_f64(object.get("request_timeout_seconds"))
        .or_else(|| optional_f64(object.get("timeout_seconds")));
    let url = format!("{}/gateway/bot", api_base_url.trim_end_matches('/'));
    let request = json!({
        "method": "GET",
        "url": url,
        "payload": JsonValue::Null,
        "headers": {
            "Authorization": format!("Bot {token}"),
            "User-Agent": user_agent,
        },
        "timeout_seconds": timeout_seconds.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "allow_retry": true,
        "retry_attempts": 4,
        "retry_base_delay_seconds": 0.75,
    });

    Ok(base_payload(
        "gateway_info_request",
        "request_planned",
        json!({
            "ok": true,
            "should_execute": true,
            "request": request,
            "method": "GET",
            "url": url,
            "headers": request["headers"].clone(),
            "actions": [
                {
                    "kind": "fetch_gateway_info",
                    "request": request,
                }
            ],
        }),
    ))
}

fn plan_gateway_url(object: &Map<String, JsonValue>) -> JsonValue {
    let session_id = clean_text(object.get("session_id"));
    let resume_gateway_url = clean_text(object.get("resume_gateway_url"));

    if let (Some(session_id), Some(gateway_base_url)) =
        (session_id.as_deref(), resume_gateway_url.as_deref())
    {
        return base_payload(
            "gateway_url",
            "planned",
            json!({
                "ok": true,
                "should_connect": true,
                "should_fetch_gateway_info": false,
                "gateway_source": "resume_gateway_url",
                "session_id": session_id,
                "gateway_base_url": gateway_base_url,
                "gateway_socket_url": discord_gateway_socket_url(gateway_base_url),
                "actions": [
                    {
                        "kind": "use_resume_gateway_url",
                        "gateway_base_url": gateway_base_url,
                    },
                    {
                        "kind": "connect_gateway",
                        "gateway_socket_url": discord_gateway_socket_url(gateway_base_url),
                    }
                ],
            }),
        );
    }

    let gateway_base_url = gateway_info_url(object);
    let should_connect = gateway_base_url.is_some();
    let actions = if let Some(gateway_base_url) = gateway_base_url.as_deref() {
        json!([
            {
                "kind": "fetch_gateway_info",
                "reason": "missing_resume_gateway_url",
            },
            {
                "kind": "connect_gateway",
                "gateway_socket_url": discord_gateway_socket_url(gateway_base_url),
            }
        ])
    } else {
        json!([
            {
                "kind": "fetch_gateway_info",
                "reason": "missing_resume_gateway_url",
            }
        ])
    };

    base_payload(
        "gateway_url",
        if should_connect {
            "planned"
        } else {
            "awaiting_gateway_info"
        },
        json!({
            "ok": should_connect,
            "should_connect": should_connect,
            "should_fetch_gateway_info": true,
            "gateway_source": if should_connect { "gateway_info" } else { "gateway_info_required" },
            "session_id": session_id,
            "gateway_base_url": gateway_base_url.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
            "gateway_socket_url": gateway_base_url
                .as_deref()
                .map(discord_gateway_socket_url)
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
            "error": if should_connect {
                JsonValue::Null
            } else {
                JsonValue::from("Discord gateway info did not include a gateway URL.")
            },
            "actions": actions,
        }),
    )
}

fn plan_handshake(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let payload = gateway_payload_object(object)?;
    let hello_op = optional_i64(payload.get("op")).unwrap_or(0);
    if hello_op != 10 {
        return Err(format!(
            "Discord gateway did not start with Hello (received op={hello_op})."
        ));
    }
    let hello_data = payload
        .get("d")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Discord Hello payload is missing heartbeat data.".to_string())?;
    let heartbeat_interval_ms = optional_i64(hello_data.get("heartbeat_interval"))
        .filter(|value| *value > 0)
        .ok_or_else(|| "Discord Hello payload did not include a heartbeat interval.".to_string())?;
    let heartbeat_interval_seconds = heartbeat_interval_ms as f64 / 1000.0;
    let token = required_text(
        object,
        &["bot_token", "token", "discord_bot_token"],
        "Missing Discord bot token. Set AIT_DISCORD_BOT_TOKEN or DISCORD_BOT_TOKEN.",
    )?;
    let session_id = clean_text(object.get("session_id"));
    let sequence = optional_i64(object.get("sequence"));
    let now_monotonic_seconds = optional_f64(object.get("now_monotonic_seconds"));
    let next_heartbeat_at = now_monotonic_seconds.map(|now| now + heartbeat_interval_seconds);

    let (operation_kind, outbound_payload) =
        if let (Some(session_id), Some(sequence)) = (session_id.as_ref(), sequence) {
            (
                "send_gateway_resume",
                json!({
                    "op": 6,
                    "d": {
                        "token": token,
                        "session_id": session_id,
                        "seq": sequence,
                    }
                }),
            )
        } else {
            let gateway_intents = optional_i64(object.get("gateway_intents"))
                .unwrap_or(DEFAULT_DISCORD_GATEWAY_INTENTS)
                .max(0);
            let platform = clean_text(object.get("platform"))
                .or_else(|| clean_text(object.get("os")))
                .unwrap_or_else(|| "unknown".to_string());
            (
                "send_gateway_identify",
                json!({
                    "op": 2,
                    "d": {
                        "token": token,
                        "intents": gateway_intents,
                        "properties": {
                            "os": platform,
                            "browser": "ait-agent",
                            "device": "ait-agent",
                        }
                    }
                }),
            )
        };

    Ok(base_payload(
        "handshake",
        "ready_for_gateway_loop",
        json!({
            "ok": true,
            "hello_op": hello_op,
            "heartbeat_interval_ms": heartbeat_interval_ms,
            "heartbeat_interval_seconds": heartbeat_interval_seconds,
            "heartbeat_acknowledged": true,
            "next_heartbeat_after_seconds": heartbeat_interval_seconds,
            "next_heartbeat_at": next_heartbeat_at.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "should_resume": operation_kind == "send_gateway_resume",
            "should_identify": operation_kind == "send_gateway_identify",
            "outbound_payload": outbound_payload,
            "actions": [
                {
                    "kind": operation_kind,
                    "payload": outbound_payload,
                }
            ],
        }),
    ))
}

fn plan_heartbeat_tick(object: &Map<String, JsonValue>) -> JsonValue {
    let now = optional_f64(object.get("now_monotonic_seconds")).unwrap_or(0.0);
    let next_heartbeat_at = optional_f64(object.get("next_heartbeat_at")).unwrap_or(now);
    let heartbeat_interval_seconds = optional_f64(object.get("heartbeat_interval_seconds"))
        .or_else(|| optional_f64(object.get("interval_seconds")))
        .unwrap_or(1.0)
        .max(0.001);
    let heartbeat_acknowledged =
        optional_bool(object.get("heartbeat_acknowledged")).unwrap_or(true);
    let sequence = optional_i64(object.get("sequence"));

    if now < next_heartbeat_at {
        let wait_seconds = (next_heartbeat_at - now).clamp(0.0, 1.0);
        return base_payload(
            "tick",
            "waiting",
            json!({
                "ok": true,
                "should_wait": true,
                "wait_seconds": wait_seconds,
                "should_send_heartbeat": false,
                "should_reconnect": false,
                "heartbeat_acknowledged": heartbeat_acknowledged,
                "next_heartbeat_at": next_heartbeat_at,
                "actions": [],
            }),
        );
    }

    if !heartbeat_acknowledged {
        return base_payload(
            "tick",
            "heartbeat_ack_timeout",
            json!({
                "ok": false,
                "should_wait": false,
                "should_send_heartbeat": false,
                "should_reconnect": true,
                "reconnect_reason": "heartbeat_ack_timeout",
                "error": "Discord gateway heartbeat ACK timed out.",
                "actions": [
                    {
                        "kind": "reconnect_gateway",
                        "reason": "heartbeat_ack_timeout",
                    }
                ],
            }),
        );
    }

    let outbound_payload = json!({
        "op": 1,
        "d": sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
    });
    base_payload(
        "tick",
        "heartbeat_planned",
        json!({
            "ok": true,
            "should_wait": false,
            "should_send_heartbeat": true,
            "should_reconnect": false,
            "heartbeat_acknowledged": false,
            "next_heartbeat_at": now + heartbeat_interval_seconds,
            "outbound_payload": outbound_payload,
            "actions": [
                {
                    "kind": "send_gateway_heartbeat",
                    "payload": outbound_payload,
                }
            ],
        }),
    )
}

fn plan_gateway_payload(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let payload = gateway_payload_object(object)?;
    let mut sequence = optional_i64(object.get("sequence"));
    if let Some(next_sequence) = optional_i64(payload.get("s")).filter(|value| *value > 0) {
        sequence = Some(next_sequence);
    }
    let op = optional_i64(payload.get("op")).unwrap_or(0);
    let heartbeat_interval_seconds = optional_f64(object.get("heartbeat_interval_seconds"))
        .or_else(|| optional_f64(object.get("interval_seconds")))
        .unwrap_or(1.0)
        .max(0.001);
    let now = optional_f64(object.get("now_monotonic_seconds")).unwrap_or(0.0);

    match op {
        0 => Ok(plan_dispatch_payload(object, payload, sequence)),
        1 => {
            let outbound_payload = json!({
                "op": 1,
                "d": sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
            });
            Ok(base_payload(
                "payload",
                "heartbeat_requested",
                json!({
                    "ok": true,
                    "gateway_op": 1,
                    "sequence": sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "heartbeat_acknowledged": false,
                    "next_heartbeat_at": now + heartbeat_interval_seconds,
                    "should_send_heartbeat": true,
                    "should_reconnect": false,
                    "outbound_payload": outbound_payload,
                    "actions": [
                        {
                            "kind": "send_gateway_heartbeat",
                            "payload": outbound_payload,
                        }
                    ],
                }),
            ))
        }
        7 => Ok(base_payload(
            "payload",
            "reconnect_requested",
            json!({
                "ok": true,
                "gateway_op": 7,
                "sequence": sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "should_reconnect": true,
                "reconnect_reason": "gateway_reconnect_requested",
                "actions": [
                    {
                        "kind": "reconnect_gateway",
                        "reason": "gateway_reconnect_requested",
                    }
                ],
            }),
        )),
        9 => {
            let can_resume = optional_bool(payload.get("d")).unwrap_or(false);
            Ok(base_payload(
                "payload",
                "invalid_session",
                json!({
                    "ok": true,
                    "gateway_op": 9,
                    "can_resume": can_resume,
                    "should_reconnect": true,
                    "reconnect_reason": if can_resume { "invalid_session_resumable" } else { "invalid_session_reset" },
                    "session_id": if can_resume {
                        clean_text(object.get("session_id")).map(JsonValue::from).unwrap_or(JsonValue::Null)
                    } else {
                        JsonValue::Null
                    },
                    "resume_gateway_url": if can_resume {
                        clean_text(object.get("resume_gateway_url")).map(JsonValue::from).unwrap_or(JsonValue::Null)
                    } else {
                        JsonValue::Null
                    },
                    "sequence": if can_resume {
                        sequence.map(JsonValue::from).unwrap_or(JsonValue::Null)
                    } else {
                        JsonValue::Null
                    },
                    "actions": [
                        {
                            "kind": "reconnect_gateway",
                            "reason": if can_resume { "invalid_session_resumable" } else { "invalid_session_reset" },
                            "reset_resume_state": !can_resume,
                        }
                    ],
                }),
            ))
        }
        11 => Ok(base_payload(
            "payload",
            "heartbeat_acknowledged",
            json!({
                "ok": true,
                "gateway_op": 11,
                "heartbeat_acknowledged": true,
                "sequence": sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "should_reconnect": false,
                "actions": [
                    {
                        "kind": "mark_heartbeat_acknowledged",
                    }
                ],
            }),
        )),
        other => Ok(base_payload(
            "payload",
            "ignored",
            json!({
                "ok": true,
                "gateway_op": other,
                "sequence": sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "should_reconnect": false,
                "actions": [],
            }),
        )),
    }
}

fn plan_dispatch_payload(
    object: &Map<String, JsonValue>,
    payload: &Map<String, JsonValue>,
    sequence: Option<i64>,
) -> JsonValue {
    let event_name = clean_text(payload.get("t")).unwrap_or_default();
    let data = payload.get("d").cloned().unwrap_or(JsonValue::Null);
    let data_object = data.as_object();
    let action = match event_name.as_str() {
        "READY" => json!({
            "kind": "record_gateway_ready",
            "session_id": data_object
                .and_then(|data| clean_text(data.get("session_id")))
                .or_else(|| clean_text(object.get("session_id")))
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
            "resume_gateway_url": data_object
                .and_then(|data| clean_text(data.get("resume_gateway_url")))
                .or_else(|| clean_text(object.get("resume_gateway_url")))
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
        }),
        "INTERACTION_CREATE" => json!({
            "kind": "handle_interaction_create",
            "payload": data,
        }),
        "MESSAGE_CREATE" => json!({
            "kind": "handle_message_create",
            "payload": data,
        }),
        _ => json!({
            "kind": "ignore_dispatch_event",
            "event_name": event_name,
        }),
    };

    let state = match event_name.as_str() {
        "READY" => "ready_received",
        "INTERACTION_CREATE" => "interaction_dispatch_planned",
        "MESSAGE_CREATE" => "message_dispatch_planned",
        _ => "dispatch_ignored",
    };
    let mut actions = vec![action];
    if event_name == "INTERACTION_CREATE" {
        actions.push(json!({
            "kind": "create_initial_interaction_response",
            "interaction_id": data_object
                .and_then(|data| clean_text(data.get("id")))
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
            "interaction_token": data_object
                .and_then(|data| clean_text(data.get("token")))
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
        }));
    }

    base_payload(
        "payload",
        state,
        json!({
            "ok": true,
            "gateway_op": 0,
            "dispatch_event_name": event_name,
            "sequence": sequence.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "session_id": if event_name == "READY" {
                data_object
                    .and_then(|data| clean_text(data.get("session_id")))
                    .or_else(|| clean_text(object.get("session_id")))
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null)
            } else {
                clean_text(object.get("session_id")).map(JsonValue::from).unwrap_or(JsonValue::Null)
            },
            "resume_gateway_url": if event_name == "READY" {
                data_object
                    .and_then(|data| clean_text(data.get("resume_gateway_url")))
                    .or_else(|| clean_text(object.get("resume_gateway_url")))
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null)
            } else {
                clean_text(object.get("resume_gateway_url")).map(JsonValue::from).unwrap_or(JsonValue::Null)
            },
            "actions": actions,
        }),
    )
}

fn plan_error_recovery(object: &Map<String, JsonValue>) -> JsonValue {
    let gateway_intents = optional_i64(object.get("gateway_intents"))
        .unwrap_or(DEFAULT_DISCORD_GATEWAY_INTENTS)
        .max(0);
    let error_text = clean_text(object.get("error_message"))
        .or_else(|| clean_text(object.get("error")))
        .or_else(|| clean_text(object.get("exception")))
        .unwrap_or_default();
    let should_drop = should_drop_message_content_intent(&error_text, gateway_intents);
    let new_gateway_intents = if should_drop {
        gateway_intents & !DISCORD_MESSAGE_CONTENT_INTENT
    } else {
        gateway_intents
    };

    base_payload(
        "error_recovery",
        if should_drop {
            "message_content_intent_dropped"
        } else {
            "unchanged"
        },
        json!({
            "ok": true,
            "should_drop_message_content_intent": should_drop,
            "should_continue_immediately": should_drop,
            "gateway_intents": gateway_intents,
            "new_gateway_intents": new_gateway_intents,
            "session_id": if should_drop {
                JsonValue::Null
            } else {
                clean_text(object.get("session_id")).map(JsonValue::from).unwrap_or(JsonValue::Null)
            },
            "resume_gateway_url": if should_drop {
                JsonValue::Null
            } else {
                clean_text(object.get("resume_gateway_url")).map(JsonValue::from).unwrap_or(JsonValue::Null)
            },
            "sequence": if should_drop {
                JsonValue::Null
            } else {
                optional_i64(object.get("sequence")).map(JsonValue::from).unwrap_or(JsonValue::Null)
            },
            "actions": if should_drop {
                json!([
                    {
                        "kind": "drop_message_content_intent",
                        "gateway_intents": gateway_intents,
                        "new_gateway_intents": new_gateway_intents,
                        "reset_resume_state": true,
                    }
                ])
            } else {
                json!([])
            },
        }),
    )
}

fn base_payload(stage: &str, state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "gateway_runtime_contract".to_string(),
        JsonValue::String(GATEWAY_RUNTIME_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "transport".to_string(),
        JsonValue::String("discord".to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert("python_gateway_allowed".to_string(), JsonValue::Bool(false));
    object.insert(
        "gateway_runtime_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    JsonValue::Object(object)
}

fn gateway_payload_object(
    object: &Map<String, JsonValue>,
) -> Result<&Map<String, JsonValue>, String> {
    object
        .get("gateway_payload")
        .or_else(|| object.get("hello_payload"))
        .or_else(|| object.get("payload"))
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Discord gateway payload must be a JSON object.".to_string())
}

fn gateway_info_url(object: &Map<String, JsonValue>) -> Option<String> {
    object
        .get("gateway_info")
        .and_then(JsonValue::as_object)
        .and_then(|info| clean_text(info.get("url")))
        .or_else(|| clean_text(object.get("gateway_info_url")))
        .or_else(|| clean_text(object.get("gateway_base_url")))
}

fn discord_gateway_socket_url(base_url: &str) -> String {
    let normalized = base_url.trim().trim_end_matches('/');
    let separator = if normalized.contains('?') { '&' } else { '?' };
    format!("{normalized}{separator}v=10&encoding=json")
}

fn should_drop_message_content_intent(error_text: &str, gateway_intents: i64) -> bool {
    if (gateway_intents & DISCORD_MESSAGE_CONTENT_INTENT) == 0 {
        return false;
    }
    let lowered = error_text.to_ascii_lowercase();
    lowered.contains("4014") && lowered.contains("disallowed intent")
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Discord gateway runtime request must be an object.".to_string())
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

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok())),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
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
