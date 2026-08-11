use crate::json_support::parse_value;
use crate::platform::native_socket_from_u64;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::{
    event_loop::{
        agent_discord_gateway_runtime_plan_json, agent_slack_socket_mode_runtime_plan_json,
    },
    transport::{
        agent_transport_websocket_fd_io_execute_json,
        websocket_frame::agent_transport_websocket_frame_plan_json,
        websocket_stream::agent_transport_websocket_stream_plan_json,
    },
};

const MIGRATION_STAGE: &str = "rust_agent_websocket_event_loop_turn_boundary";
const WEBSOCKET_TURN_CONTRACT: &str = "ait_agent_core.event_loop.WebSocketTurn.v1";

pub fn agent_websocket_event_loop_turn_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "readable_turn".to_string());

    match stage.as_str() {
        "readable" | "readable_turn" | "socket_readable" | "fd_readable" => {
            plan_readable_turn(object)
        }
        "writable" | "writable_turn" | "socket_writable" | "fd_writable" => {
            plan_writable_turn(object)
        }
        "ready" | "ready_turn" | "readable_writable_turn" | "fd_ready" => plan_ready_turn(object),
        other => Err(format!(
            "unsupported WebSocket event-loop turn stage: {other}"
        )),
    }
}

fn plan_readable_turn(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let metadata = turn_metadata(object);
    let transport = match websocket_transport(object) {
        Ok(transport) => transport,
        Err(reason) => {
            let mut actions = Vec::new();
            append_transport_context_close(&mut actions, &metadata, &reason);
            return Ok(turn_payload(
                "readable_turn",
                "transport_context_error",
                "unknown",
                &metadata,
                json!({
                    "ok": false,
                    "readable": event_bool(object, "readable").unwrap_or(true),
                    "hangup": event_bool(object, "hangup").unwrap_or(false),
                    "read_eof": event_bool(object, "read_eof").or_else(|| event_bool(object, "eof")).unwrap_or(false),
                    "stream_plan": JsonValue::Null,
                    "runtime_plans": [],
                    "remaining_buffer_bytes": [],
                    "remaining_buffer_hex": "",
                    "should_keep_registered": false,
                    "should_unregister": metadata.token.is_some(),
                    "should_close_websocket": true,
                    "should_reconnect": false,
                    "error": reason,
                    "actions": actions,
                }),
            ));
        }
    };
    let hangup = event_bool(object, "hangup")
        .or_else(|| event_bool(object, "hup"))
        .unwrap_or(false);
    let readable = event_bool(object, "readable").unwrap_or(true);
    let read_eof = event_bool(object, "read_eof")
        .or_else(|| event_bool(object, "eof"))
        .unwrap_or(false);

    if hangup {
        let mut actions = Vec::new();
        append_unregister_close_reconnect(
            &mut actions,
            &transport,
            &metadata,
            "websocket_event_loop_hangup",
            "WebSocket fd reported hangup.",
            false,
        );
        return Ok(turn_payload(
            "readable_turn",
            "hangup_reconnect",
            &transport,
            &metadata,
            json!({
                "ok": false,
                "readable": readable,
                "hangup": true,
                "read_eof": read_eof,
                "stream_plan": JsonValue::Null,
                "runtime_plans": [],
                "remaining_buffer_bytes": [],
                "remaining_buffer_hex": "",
                "should_keep_registered": false,
                "should_unregister": metadata.token.is_some(),
                "should_close_websocket": true,
                "should_reconnect": true,
                "error": "WebSocket fd reported hangup.",
                "actions": actions,
            }),
        ));
    }

    let readable_input = match prepare_readable_stream_input(object, &metadata, &transport) {
        Ok(input) => input,
        Err(reason) => {
            return Ok(readable_turn_configuration_error(
                &transport,
                &metadata,
                &reason,
                JsonValue::Null,
                Vec::new(),
            ));
        }
    };
    if !bool_field(readable_input.fd_io_result.get("ok")).unwrap_or(true) {
        let reason = clean_text(readable_input.fd_io_result.get("error"))
            .unwrap_or_else(|| "WebSocket fd read failed.".to_string());
        return Ok(readable_turn_configuration_error(
            &transport,
            &metadata,
            &reason,
            readable_input.fd_io_result,
            readable_input.fd_io_actions,
        ));
    }

    let stream_plan = agent_transport_websocket_stream_plan_json(&readable_input.stream_request)?;
    let mut actions = readable_input.fd_io_actions.clone();
    let mut runtime_plans = Vec::new();
    let mut fatal_reason: Option<String> = None;
    let mut should_close_websocket =
        bool_field(stream_plan.get("should_close_websocket")).unwrap_or(false);
    let mut should_reconnect = false;
    let mut text_payload_count = 0usize;

    if let Some(stream_actions) = stream_plan.get("actions").and_then(JsonValue::as_array) {
        for stream_action in stream_actions {
            let kind = clean_text(stream_action.get("kind")).unwrap_or_default();
            match kind.as_str() {
                "deliver_websocket_text" => {
                    text_payload_count += 1;
                    let payload_text =
                        clean_text(stream_action.get("payload_text")).unwrap_or_default();
                    match dispatch_text_payload(&transport, object, &metadata, &payload_text) {
                        Ok(dispatched) => {
                            actions.push(json!({
                                "kind": match transport.as_str() {
                                    "slack" => "dispatch_slack_socket_mode_payload",
                                    "discord" => "dispatch_discord_gateway_payload",
                                    _ => "dispatch_websocket_text_payload",
                                },
                                "transport": transport,
                                "payload": dispatched.payload,
                                "runtime_plan": dispatched.runtime_plan,
                                "runtime_action_count": dispatched.runtime_actions.len(),
                                "execute_after_control_writes": true,
                            }));
                            for runtime_action in dispatched.runtime_actions {
                                match encode_runtime_outbound_action(
                                    &runtime_action,
                                    object,
                                    &transport,
                                ) {
                                    Ok(Some(encoded)) => actions.push(encoded),
                                    Ok(None) => {}
                                    Err(reason) => {
                                        fatal_reason = Some(reason);
                                        should_close_websocket = true;
                                        break;
                                    }
                                }
                                if is_reconnect_action(&runtime_action) {
                                    should_reconnect = true;
                                }
                                actions.push(runtime_action);
                            }
                            runtime_plans.push(dispatched.runtime_plan);
                            if fatal_reason.is_some() {
                                break;
                            }
                        }
                        Err(reason) => {
                            fatal_reason = Some(reason);
                            should_close_websocket = true;
                            break;
                        }
                    }
                }
                "deliver_websocket_binary" => {
                    fatal_reason =
                        Some("WebSocket binary payloads are not supported by ait-agent Slack Socket Mode or Discord gateway turn planners.".to_string());
                    should_close_websocket = true;
                    break;
                }
                "close_websocket" => {
                    should_close_websocket = true;
                    actions.push(stream_action.clone());
                }
                "write_websocket_frame" | "mark_websocket_pong" => {
                    actions.push(stream_action.clone());
                }
                _ => actions.push(stream_action.clone()),
            }
        }
    }

    let stream_ok = bool_field(stream_plan.get("ok")).unwrap_or(false);
    if fatal_reason.is_none() && !stream_ok {
        fatal_reason = clean_text(stream_plan.get("error"))
            .or_else(|| clean_text(stream_plan.get("websocket_stream_state")))
            .or_else(|| Some("WebSocket stream planning failed.".to_string()));
        should_close_websocket = true;
    }

    if let Some(reason) = fatal_reason.as_deref() {
        append_unregister_close_reconnect(
            &mut actions,
            &transport,
            &metadata,
            "websocket_turn_failed_closed",
            reason,
            true,
        );
        should_reconnect = true;
    } else if should_close_websocket {
        append_unregister_close_reconnect(
            &mut actions,
            &transport,
            &metadata,
            "websocket_closed",
            "WebSocket stream requested close.",
            true,
        );
        should_reconnect = true;
    } else {
        append_keep_registered(&mut actions, &metadata);
    }

    let fd_read_would_block = readable_input.used_fd_io
        && bool_field(readable_input.fd_io_result.get("would_block")).unwrap_or(false)
        && usize_field(&readable_input.fd_io_result, "read_byte_count").unwrap_or(0) == 0
        && stream_state(&stream_plan) == Some("idle");
    let read_would_block =
        bool_field(readable_input.fd_io_result.get("would_block")).unwrap_or(false);
    let read_limit_reached =
        bool_field(readable_input.fd_io_result.get("read_limit_reached")).unwrap_or(false);
    let turn_state = if fatal_reason.is_some() {
        "failed_closed"
    } else if should_close_websocket {
        "closing"
    } else if fd_read_would_block {
        "read_would_block"
    } else if stream_state(&stream_plan) == Some("partial_frame") {
        "partial_frame"
    } else if text_payload_count > 0 {
        "payloads_dispatched"
    } else if usize_field(&stream_plan, "processed_frame_count").unwrap_or(0) > 0 {
        "control_frames_processed"
    } else {
        "idle"
    };

    Ok(turn_payload(
        "readable_turn",
        turn_state,
        &transport,
        &metadata,
        json!({
            "ok": fatal_reason.is_none() && stream_ok,
            "readable": readable,
            "hangup": false,
            "read_eof": readable_input.read_eof,
            "read_source": readable_input.read_source,
            "read_byte_count": clone_field(&stream_plan, "read_byte_count"),
            "read_bytes": clone_field(&readable_input.stream_request, "read_bytes"),
            "read_hex": clone_field(&readable_input.stream_request, "read_hex"),
            "read_would_block": read_would_block,
            "read_limit_reached": read_limit_reached,
            "fd_io_result": readable_input.fd_io_result,
            "stream_plan": stream_plan,
            "runtime_plans": runtime_plans,
            "processed_text_payload_count": text_payload_count,
            "remaining_buffer_bytes": clone_field(&stream_plan, "remaining_buffer_bytes"),
            "remaining_buffer_hex": clone_field(&stream_plan, "remaining_buffer_hex"),
            "needed_bytes": clone_field(&stream_plan, "needed_bytes"),
            "needed_additional_bytes": clone_field(&stream_plan, "needed_additional_bytes"),
            "should_keep_registered": !should_close_websocket && fatal_reason.is_none(),
            "should_unregister": should_close_websocket || fatal_reason.is_some(),
            "should_close_websocket": should_close_websocket,
            "should_reconnect": should_reconnect,
            "error": fatal_reason.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "actions": actions,
        }),
    ))
}

fn plan_writable_turn(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let metadata = turn_metadata(object);
    let transport = match websocket_transport(object) {
        Ok(transport) => transport,
        Err(reason) => {
            let mut actions = Vec::new();
            append_transport_context_close(&mut actions, &metadata, &reason);
            return Ok(turn_payload(
                "writable_turn",
                "transport_context_error",
                "unknown",
                &metadata,
                json!({
                    "ok": false,
                    "writable": event_bool(object, "writable").unwrap_or(true),
                    "hangup": event_bool(object, "hangup").or_else(|| event_bool(object, "hup")).unwrap_or(false),
                    "fd_io_result": JsonValue::Null,
                    "pending_write_byte_count": 0,
                    "pending_write_bytes": [],
                    "pending_write_hex": "",
                    "remaining_write_byte_count": 0,
                    "remaining_write_bytes": [],
                    "remaining_write_hex": "",
                    "bytes_written": 0,
                    "write_complete": false,
                    "would_block": false,
                    "should_keep_registered": false,
                    "should_register_read_write": false,
                    "should_unregister": metadata.token.is_some(),
                    "should_close_websocket": true,
                    "should_reconnect": false,
                    "error": reason,
                    "actions": actions,
                }),
            ));
        }
    };
    let hangup = event_bool(object, "hangup")
        .or_else(|| event_bool(object, "hup"))
        .unwrap_or(false);
    let writable = event_bool(object, "writable").unwrap_or(true);

    if hangup {
        let mut actions = Vec::new();
        append_unregister_close_reconnect(
            &mut actions,
            &transport,
            &metadata,
            "websocket_event_loop_hangup",
            "WebSocket fd reported hangup.",
            false,
        );
        return Ok(turn_payload(
            "writable_turn",
            "hangup_reconnect",
            &transport,
            &metadata,
            json!({
                "ok": false,
                "writable": writable,
                "hangup": true,
                "fd_io_result": JsonValue::Null,
                "pending_write_byte_count": 0,
                "pending_write_bytes": [],
                "pending_write_hex": "",
                "remaining_write_byte_count": 0,
                "remaining_write_bytes": [],
                "remaining_write_hex": "",
                "bytes_written": 0,
                "write_complete": false,
                "would_block": false,
                "should_keep_registered": false,
                "should_register_read_write": false,
                "should_unregister": metadata.token.is_some(),
                "should_close_websocket": true,
                "should_reconnect": true,
                "error": "WebSocket fd reported hangup.",
                "actions": actions,
            }),
        ));
    }

    let pending_write_bytes = match pending_write_bytes(object) {
        Ok(bytes) => bytes,
        Err(reason) => {
            return Ok(writable_turn_configuration_error(
                &transport, &metadata, &reason,
            ));
        }
    };
    if pending_write_bytes.is_empty() {
        let mut actions = Vec::new();
        append_keep_registered(&mut actions, &metadata);
        return Ok(turn_payload(
            "writable_turn",
            "write_queue_empty",
            &transport,
            &metadata,
            json!({
                "ok": true,
                "writable": writable,
                "hangup": false,
                "fd_io_result": JsonValue::Null,
                "pending_write_byte_count": 0,
                "pending_write_bytes": [],
                "pending_write_hex": "",
                "remaining_write_byte_count": 0,
                "remaining_write_bytes": [],
                "remaining_write_hex": "",
                "bytes_written": 0,
                "write_complete": true,
                "would_block": false,
                "should_keep_registered": metadata.token.is_some() && metadata.fd.is_some(),
                "should_register_read_write": false,
                "should_unregister": false,
                "should_close_websocket": false,
                "should_reconnect": false,
                "error": JsonValue::Null,
                "actions": actions,
            }),
        ));
    }

    if !writable {
        let mut actions = Vec::new();
        append_keep_read_write_registered(&mut actions, &metadata, &pending_write_bytes);
        return Ok(turn_payload(
            "writable_turn",
            "not_writable",
            &transport,
            &metadata,
            json!({
                "ok": true,
                "writable": false,
                "hangup": false,
                "fd_io_result": JsonValue::Null,
                "pending_write_byte_count": pending_write_bytes.len(),
                "pending_write_bytes": bytes_json(&pending_write_bytes),
                "pending_write_hex": bytes_hex(&pending_write_bytes),
                "remaining_write_byte_count": pending_write_bytes.len(),
                "remaining_write_bytes": bytes_json(&pending_write_bytes),
                "remaining_write_hex": bytes_hex(&pending_write_bytes),
                "bytes_written": 0,
                "write_complete": false,
                "would_block": false,
                "should_keep_registered": metadata.token.is_some() && metadata.fd.is_some(),
                "should_register_read_write": metadata.token.is_some() && metadata.fd.is_some(),
                "should_unregister": false,
                "should_close_websocket": false,
                "should_reconnect": false,
                "error": JsonValue::Null,
                "actions": actions,
            }),
        ));
    }

    let Some(fd) = metadata.fd else {
        return Ok(writable_turn_configuration_error(
            &transport,
            &metadata,
            "WebSocket writable turn requires websocket_fd.",
        ));
    };
    if native_socket_from_u64(fd).is_err() {
        return Ok(writable_turn_configuration_error(
            &transport,
            &metadata,
            "WebSocket writable turn websocket_fd must fit NativeSocket.",
        ));
    }

    let fd_io_request = writable_fd_io_request(object, &metadata, fd, &pending_write_bytes);
    let fd_io_result = agent_transport_websocket_fd_io_execute_json(&fd_io_request)?;
    let fd_io_ok = bool_field(fd_io_result.get("ok")).unwrap_or(false);
    let mut remaining_write_bytes = remaining_write_bytes_from_fd_io_result(&fd_io_result)
        .unwrap_or_else(|_| pending_write_bytes.clone());
    if bool_field(fd_io_result.get("write_complete")).unwrap_or(false) {
        remaining_write_bytes.clear();
    }
    let bytes_written = usize_field(&fd_io_result, "bytes_written").unwrap_or(0);
    let write_complete = remaining_write_bytes.is_empty();
    let would_block = bool_field(fd_io_result.get("would_block")).unwrap_or(false);
    let mut actions = Vec::new();
    append_fd_io_result_actions(&mut actions, &fd_io_result);

    if !fd_io_ok {
        let reason = clean_text(fd_io_result.get("error"))
            .unwrap_or_else(|| "WebSocket fd write failed.".to_string());
        append_unregister_close_reconnect(
            &mut actions,
            &transport,
            &metadata,
            "websocket_fd_write_failed",
            &reason,
            false,
        );
        return Ok(turn_payload(
            "writable_turn",
            "fd_write_failed",
            &transport,
            &metadata,
            json!({
                "ok": false,
                "writable": true,
                "hangup": false,
                "fd_io_result": fd_io_result,
                "pending_write_byte_count": pending_write_bytes.len(),
                "pending_write_bytes": bytes_json(&pending_write_bytes),
                "pending_write_hex": bytes_hex(&pending_write_bytes),
                "remaining_write_byte_count": remaining_write_bytes.len(),
                "remaining_write_bytes": bytes_json(&remaining_write_bytes),
                "remaining_write_hex": bytes_hex(&remaining_write_bytes),
                "bytes_written": bytes_written,
                "write_complete": false,
                "would_block": would_block,
                "should_keep_registered": false,
                "should_register_read_write": false,
                "should_unregister": metadata.token.is_some(),
                "should_close_websocket": true,
                "should_reconnect": true,
                "error": reason,
                "actions": actions,
            }),
        ));
    }

    if write_complete {
        actions.push(json!({
            "kind": "clear_websocket_pending_write",
            "websocket_fd": metadata.fd,
            "event_loop_token": metadata.token,
            "bytes_written": bytes_written,
        }));
        append_keep_registered(&mut actions, &metadata);
    } else {
        actions.push(json!({
            "kind": "carry_websocket_pending_write",
            "websocket_fd": metadata.fd,
            "event_loop_token": metadata.token,
            "remaining_write_byte_count": remaining_write_bytes.len(),
            "remaining_write_bytes": bytes_json(&remaining_write_bytes),
            "remaining_write_hex": bytes_hex(&remaining_write_bytes),
        }));
        append_keep_read_write_registered(&mut actions, &metadata, &remaining_write_bytes);
    }

    let state = if write_complete {
        "write_complete"
    } else if would_block && bytes_written == 0 {
        "would_block"
    } else {
        "partial_write"
    };

    Ok(turn_payload(
        "writable_turn",
        state,
        &transport,
        &metadata,
        json!({
            "ok": true,
            "writable": true,
            "hangup": false,
            "fd_io_result": fd_io_result,
            "pending_write_byte_count": pending_write_bytes.len(),
            "pending_write_bytes": bytes_json(&pending_write_bytes),
            "pending_write_hex": bytes_hex(&pending_write_bytes),
            "remaining_write_byte_count": remaining_write_bytes.len(),
            "remaining_write_bytes": bytes_json(&remaining_write_bytes),
            "remaining_write_hex": bytes_hex(&remaining_write_bytes),
            "bytes_written": bytes_written,
            "write_complete": write_complete,
            "would_block": would_block,
            "should_keep_registered": metadata.token.is_some() && metadata.fd.is_some(),
            "should_register_read_write": !write_complete && metadata.token.is_some() && metadata.fd.is_some(),
            "should_unregister": false,
            "should_close_websocket": false,
            "should_reconnect": false,
            "error": JsonValue::Null,
            "actions": actions,
        }),
    ))
}

fn plan_ready_turn(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let metadata = turn_metadata(object);
    let readable = event_bool(object, "readable").unwrap_or(true);
    let writable = event_bool(object, "writable").unwrap_or(false);
    let hangup = event_bool(object, "hangup")
        .or_else(|| event_bool(object, "hup"))
        .unwrap_or(false);

    let read_plan = if readable || hangup {
        Some(plan_readable_turn(object)?)
    } else {
        None
    };
    if let Some(read_plan) = read_plan.as_ref() {
        if hangup || turn_plan_closes_connection(read_plan) {
            return Ok(read_plan.clone());
        }
    }

    let write_plan = if writable {
        Some(plan_writable_turn(object)?)
    } else {
        None
    };

    match (read_plan, write_plan) {
        (Some(read_plan), Some(write_plan)) => Ok(combine_ready_turns(
            &metadata,
            readable,
            writable,
            &read_plan,
            &write_plan,
        )),
        (Some(read_plan), None) => Ok(read_plan),
        (None, Some(write_plan)) => Ok(write_plan),
        (None, None) => {
            let transport = websocket_transport(object).unwrap_or_else(|_| "unknown".to_string());
            let mut actions = Vec::new();
            append_keep_registered(&mut actions, &metadata);
            Ok(turn_payload(
                "ready_turn",
                "idle",
                &transport,
                &metadata,
                json!({
                    "ok": true,
                    "readable": false,
                    "writable": false,
                    "hangup": false,
                    "read_turn_plan": JsonValue::Null,
                    "write_turn_plan": JsonValue::Null,
                    "stream_plan": JsonValue::Null,
                    "runtime_plans": [],
                    "remaining_buffer_bytes": [],
                    "remaining_buffer_hex": "",
                    "pending_write_byte_count": 0,
                    "pending_write_bytes": [],
                    "pending_write_hex": "",
                    "remaining_write_byte_count": 0,
                    "remaining_write_bytes": [],
                    "remaining_write_hex": "",
                    "bytes_written": 0,
                    "write_complete": true,
                    "would_block": false,
                    "should_keep_registered": metadata.token.is_some() && metadata.fd.is_some(),
                    "should_register_read_write": false,
                    "should_unregister": false,
                    "should_close_websocket": false,
                    "should_reconnect": false,
                    "error": JsonValue::Null,
                    "actions": actions,
                }),
            ))
        }
    }
}

fn combine_ready_turns(
    metadata: &TurnMetadata,
    readable: bool,
    writable: bool,
    read_plan: &JsonValue,
    write_plan: &JsonValue,
) -> JsonValue {
    let transport = clean_text(read_plan.get("transport"))
        .or_else(|| clean_text(write_plan.get("transport")))
        .unwrap_or_else(|| "websocket".to_string());
    let read_ok = bool_field(read_plan.get("ok")).unwrap_or(false);
    let write_ok = bool_field(write_plan.get("ok")).unwrap_or(false);
    let should_register_read_write =
        bool_field(write_plan.get("should_register_read_write")).unwrap_or(false);
    let mut actions = Vec::new();
    append_turn_plan_actions(&mut actions, read_plan, true);
    append_turn_plan_actions(&mut actions, write_plan, false);
    let state = if read_ok && write_ok {
        "readable_writable_turns"
    } else if !read_ok && !write_ok {
        "readable_writable_failed"
    } else if !read_ok {
        "readable_failed"
    } else {
        "writable_failed"
    };
    let should_close_websocket = bool_field(read_plan.get("should_close_websocket"))
        .unwrap_or(false)
        || bool_field(write_plan.get("should_close_websocket")).unwrap_or(false);
    let should_unregister = bool_field(read_plan.get("should_unregister")).unwrap_or(false)
        || bool_field(write_plan.get("should_unregister")).unwrap_or(false);
    let should_reconnect = bool_field(read_plan.get("should_reconnect")).unwrap_or(false)
        || bool_field(write_plan.get("should_reconnect")).unwrap_or(false);
    let error = clean_text(read_plan.get("error"))
        .or_else(|| clean_text(write_plan.get("error")))
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);

    turn_payload(
        "ready_turn",
        state,
        &transport,
        metadata,
        json!({
            "ok": read_ok && write_ok,
            "readable": readable,
            "writable": writable,
            "hangup": false,
            "read_turn_plan": read_plan,
            "write_turn_plan": write_plan,
            "stream_plan": clone_field(read_plan, "stream_plan"),
            "runtime_plans": clone_field(read_plan, "runtime_plans"),
            "processed_text_payload_count": clone_field(read_plan, "processed_text_payload_count"),
            "remaining_buffer_bytes": clone_field(read_plan, "remaining_buffer_bytes"),
            "remaining_buffer_hex": clone_field(read_plan, "remaining_buffer_hex"),
            "needed_bytes": clone_field(read_plan, "needed_bytes"),
            "needed_additional_bytes": clone_field(read_plan, "needed_additional_bytes"),
            "fd_io_result": clone_field(write_plan, "fd_io_result"),
            "pending_write_byte_count": clone_field(write_plan, "pending_write_byte_count"),
            "pending_write_bytes": clone_field(write_plan, "pending_write_bytes"),
            "pending_write_hex": clone_field(write_plan, "pending_write_hex"),
            "remaining_write_byte_count": clone_field(write_plan, "remaining_write_byte_count"),
            "remaining_write_bytes": clone_field(write_plan, "remaining_write_bytes"),
            "remaining_write_hex": clone_field(write_plan, "remaining_write_hex"),
            "bytes_written": clone_field(write_plan, "bytes_written"),
            "write_complete": clone_field(write_plan, "write_complete"),
            "would_block": clone_field(write_plan, "would_block"),
            "should_keep_registered": !should_unregister && !should_close_websocket,
            "should_register_read_write": should_register_read_write,
            "should_unregister": should_unregister,
            "should_close_websocket": should_close_websocket,
            "should_reconnect": should_reconnect,
            "error": error,
            "actions": actions,
        }),
    )
}

struct DispatchedTextPayload {
    payload: JsonValue,
    runtime_plan: JsonValue,
    runtime_actions: Vec<JsonValue>,
}

fn dispatch_text_payload(
    transport: &str,
    object: &Map<String, JsonValue>,
    metadata: &TurnMetadata,
    payload_text: &str,
) -> Result<DispatchedTextPayload, String> {
    let payload = parse_value(payload_text, "WebSocket text payload was not valid JSON")?;
    if !payload.is_object() {
        return Err("WebSocket text payload must be a JSON object.".to_string());
    }
    let runtime_request = runtime_request(transport, object, metadata, payload.clone());
    let runtime_plan = match transport {
        "slack" => agent_slack_socket_mode_runtime_plan_json(&runtime_request)
            .map_err(|err| format!("Slack Socket Mode payload planning failed: {err}"))?,
        "discord" => agent_discord_gateway_runtime_plan_json(&runtime_request)
            .map_err(|err| format!("Discord gateway payload planning failed: {err}"))?,
        _ => return Err(format!("unsupported WebSocket transport `{transport}`")),
    };
    let runtime_actions = runtime_plan
        .get("actions")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();

    Ok(DispatchedTextPayload {
        payload,
        runtime_plan,
        runtime_actions,
    })
}

fn runtime_request(
    transport: &str,
    object: &Map<String, JsonValue>,
    metadata: &TurnMetadata,
    payload: JsonValue,
) -> JsonValue {
    let mut request = object.clone();
    request.remove("buffer_bytes");
    request.remove("receive_buffer_bytes");
    request.remove("buffer_hex");
    request.remove("receive_buffer_hex");
    request.remove("read_bytes");
    request.remove("chunk_bytes");
    request.remove("read_hex");
    request.remove("chunk_hex");
    request.insert(
        "backend".to_string(),
        JsonValue::String(metadata.backend.clone()),
    );
    request.insert(
        "shard_index".to_string(),
        JsonValue::from(metadata.shard_index),
    );
    if let Some(token) = metadata.token {
        request.insert("event_loop_token".to_string(), JsonValue::from(token));
    }
    if let Some(fd) = metadata.fd {
        request.insert("websocket_fd".to_string(), JsonValue::from(fd));
    }
    match transport {
        "slack" => {
            request.insert(
                "stage".to_string(),
                JsonValue::String("payload".to_string()),
            );
            request.insert("payload".to_string(), payload);
        }
        "discord" => {
            let stage = if optional_i64(payload.get("op")) == Some(10) {
                "handshake"
            } else {
                "payload"
            };
            request.insert("stage".to_string(), JsonValue::String(stage.to_string()));
            request.insert("gateway_payload".to_string(), payload.clone());
            request.insert("payload".to_string(), payload);
        }
        _ => {}
    }
    JsonValue::Object(request)
}

fn encode_runtime_outbound_action(
    runtime_action: &JsonValue,
    object: &Map<String, JsonValue>,
    transport: &str,
) -> Result<Option<JsonValue>, String> {
    let kind = clean_text(runtime_action.get("kind")).unwrap_or_default();
    let Some(payload) = runtime_outbound_payload(&kind, runtime_action) else {
        return Ok(None);
    };
    if payload.is_null() {
        return Ok(None);
    }
    let Some(mask_key) = object
        .get("mask_key")
        .or_else(|| object.get("control_mask_key"))
        .or_else(|| object.get("mask"))
    else {
        return Err(format!(
            "WebSocket runtime action `{kind}` requires an explicit 4-byte mask_key for outbound frame planning."
        ));
    };
    let encoded = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "encode",
        "opcode": "text",
        "payload": payload,
        "mask_key": mask_key,
    }))?;
    Ok(Some(json!({
        "kind": "write_websocket_frame",
        "opcode": "text",
        "transport": transport,
        "frame_bytes": clone_field(&encoded, "frame_bytes"),
        "frame_hex": clone_field(&encoded, "frame_hex"),
        "payload": payload,
        "source": "websocket_runtime_payload",
        "runtime_action_kind": kind,
        "execute_before_command_side_effects": bool_field(runtime_action.get("execute_before_command_side_effects")).unwrap_or(false),
    })))
}

fn runtime_outbound_payload(kind: &str, runtime_action: &JsonValue) -> Option<JsonValue> {
    match kind {
        "send_gateway_heartbeat" | "send_gateway_identify" | "send_gateway_resume" => {
            Some(clone_field(runtime_action, "payload"))
        }
        "execute_websocket_ack" => Some(clone_field(runtime_action, "response")),
        _ => None,
    }
}

fn stream_request(object: &Map<String, JsonValue>) -> JsonValue {
    let mut request = Map::new();
    request.insert(
        "stage".to_string(),
        JsonValue::String("consume_read_chunk".to_string()),
    );
    for key in [
        "buffer_bytes",
        "receive_buffer_bytes",
        "buffer_hex",
        "receive_buffer_hex",
        "read_bytes",
        "chunk_bytes",
        "read_hex",
        "chunk_hex",
        "max_payload_bytes",
        "allow_masked",
        "read_eof",
        "eof",
        "mask_key",
        "control_mask_key",
        "mask",
    ] {
        if let Some(value) = object.get(key) {
            request.insert(key.to_string(), value.clone());
        }
    }
    JsonValue::Object(request)
}

#[derive(Debug, Clone)]
struct ReadableStreamInput {
    stream_request: JsonValue,
    fd_io_result: JsonValue,
    fd_io_actions: Vec<JsonValue>,
    read_eof: bool,
    read_source: &'static str,
    used_fd_io: bool,
}

fn prepare_readable_stream_input(
    object: &Map<String, JsonValue>,
    metadata: &TurnMetadata,
    transport: &str,
) -> Result<ReadableStreamInput, String> {
    let mut request = stream_request(object);
    if explicit_read_input_present(object) {
        return Ok(ReadableStreamInput {
            stream_request: request,
            fd_io_result: JsonValue::Null,
            fd_io_actions: Vec::new(),
            read_eof: event_bool(object, "read_eof")
                .or_else(|| event_bool(object, "eof"))
                .unwrap_or(false),
            read_source: "request",
            used_fd_io: false,
        });
    }

    let Some(fd) = metadata.fd else {
        return Err(
            "WebSocket readable turn requires websocket_fd when no read bytes are supplied."
                .to_string(),
        );
    };
    if native_socket_from_u64(fd).is_err() {
        return Err("WebSocket readable turn websocket_fd must fit NativeSocket.".to_string());
    }

    let fd_io_request = readable_fd_io_request(object, metadata, transport, fd);
    let fd_io_result = agent_transport_websocket_fd_io_execute_json(&fd_io_request)
        .map_err(|err| format!("WebSocket fd read execution failed: {err}"))?;
    if let Some(request_object) = request.as_object_mut() {
        copy_result_field(&fd_io_result, request_object, "read_bytes", "read_bytes");
        copy_result_field(&fd_io_result, request_object, "read_hex", "read_hex");
        copy_result_field(&fd_io_result, request_object, "read_eof", "read_eof");
    }
    let mut fd_io_actions = Vec::new();
    append_fd_io_result_actions(&mut fd_io_actions, &fd_io_result);
    let read_eof = bool_field(fd_io_result.get("read_eof")).unwrap_or(false);

    Ok(ReadableStreamInput {
        stream_request: request,
        fd_io_result,
        fd_io_actions,
        read_eof,
        read_source: "fd_io",
        used_fd_io: true,
    })
}

fn explicit_read_input_present(object: &Map<String, JsonValue>) -> bool {
    [
        "read_bytes",
        "chunk_bytes",
        "read_hex",
        "chunk_hex",
        "read_eof",
        "eof",
    ]
    .iter()
    .any(|key| object.contains_key(*key))
}

fn readable_fd_io_request(
    object: &Map<String, JsonValue>,
    metadata: &TurnMetadata,
    transport: &str,
    fd: u64,
) -> JsonValue {
    let mut request = Map::new();
    request.insert(
        "stage".to_string(),
        JsonValue::String("read_ready_fd".to_string()),
    );
    request.insert(
        "backend".to_string(),
        JsonValue::String(metadata.backend.clone()),
    );
    request.insert(
        "shard_index".to_string(),
        JsonValue::from(metadata.shard_index),
    );
    request.insert("websocket_fd".to_string(), JsonValue::from(fd));
    request.insert(
        "transport".to_string(),
        JsonValue::String(transport.to_string()),
    );
    if let Some(token) = metadata.token {
        request.insert("event_loop_token".to_string(), JsonValue::from(token));
    }
    for key in [
        "worker_key",
        "set_nonblocking",
        "max_read_bytes",
        "read_chunk_bytes",
    ] {
        if let Some(value) = object.get(key) {
            request.insert(key.to_string(), value.clone());
        }
    }
    JsonValue::Object(request)
}

fn copy_result_field(
    result: &JsonValue,
    request: &mut Map<String, JsonValue>,
    result_key: &str,
    request_key: &str,
) {
    if let Some(value) = result.get(result_key) {
        request.insert(request_key.to_string(), value.clone());
    }
}

fn writable_fd_io_request(
    object: &Map<String, JsonValue>,
    metadata: &TurnMetadata,
    fd: u64,
    pending_write_bytes: &[u8],
) -> JsonValue {
    let mut request = Map::new();
    request.insert(
        "stage".to_string(),
        JsonValue::String("write_frame".to_string()),
    );
    request.insert(
        "backend".to_string(),
        JsonValue::String(metadata.backend.clone()),
    );
    request.insert(
        "shard_index".to_string(),
        JsonValue::from(metadata.shard_index),
    );
    request.insert("websocket_fd".to_string(), JsonValue::from(fd));
    request.insert("write_bytes".to_string(), bytes_json(pending_write_bytes));
    if let Some(token) = metadata.token {
        request.insert("event_loop_token".to_string(), JsonValue::from(token));
    }
    for key in [
        "transport",
        "worker_key",
        "set_nonblocking",
        "max_write_bytes",
    ] {
        if let Some(value) = object.get(key) {
            request.insert(key.to_string(), value.clone());
        }
    }
    JsonValue::Object(request)
}

fn append_fd_io_result_actions(actions: &mut Vec<JsonValue>, fd_io_result: &JsonValue) {
    let Some(fd_io_actions) = fd_io_result.get("actions").and_then(JsonValue::as_array) else {
        return;
    };
    actions.extend(fd_io_actions.iter().cloned());
}

fn append_turn_plan_actions(
    actions: &mut Vec<JsonValue>,
    turn_plan: &JsonValue,
    skip_readable_registration: bool,
) {
    let Some(turn_actions) = turn_plan.get("actions").and_then(JsonValue::as_array) else {
        return;
    };
    for action in turn_actions {
        if skip_readable_registration && is_keep_readable_registration_action(action) {
            continue;
        }
        actions.push(action.clone());
    }
}

fn turn_plan_closes_connection(turn_plan: &JsonValue) -> bool {
    bool_field(turn_plan.get("should_close_websocket")).unwrap_or(false)
        || bool_field(turn_plan.get("should_unregister")).unwrap_or(false)
        || bool_field(turn_plan.get("should_reconnect")).unwrap_or(false)
}

fn is_keep_readable_registration_action(action: &JsonValue) -> bool {
    clean_text(action.get("kind")).as_deref() == Some("keep_websocket_readable_registered")
}

fn remaining_write_bytes_from_fd_io_result(fd_io_result: &JsonValue) -> Result<Vec<u8>, String> {
    let Some(object) = fd_io_result.as_object() else {
        return Err("WebSocket fd I/O result must be an object.".to_string());
    };
    request_optional_bytes(
        object,
        &["remaining_write_bytes"],
        &["remaining_write_hex"],
        &[],
    )
    .map(|bytes| bytes.unwrap_or_default())
}

#[derive(Debug, Clone)]
struct TurnMetadata {
    backend: String,
    shard_index: usize,
    token: Option<u64>,
    fd: Option<u64>,
}

fn turn_metadata(object: &Map<String, JsonValue>) -> TurnMetadata {
    TurnMetadata {
        backend: clean_text(object.get("backend"))
            .or_else(|| nested_text(object, "worker_lease", "backend"))
            .or_else(|| nested_text(object, "event_loop_registration", "backend"))
            .unwrap_or_else(|| "portable_poll".to_string()),
        shard_index: optional_usize(object.get("shard_index"))
            .or_else(|| nested_usize(object, "worker_lease", "shard_index"))
            .or_else(|| nested_usize(object, "event_loop_registration", "shard_index"))
            .unwrap_or(0),
        token: optional_u64(object.get("event_loop_token"))
            .or_else(|| optional_u64(object.get("token")))
            .or_else(|| nested_u64(object, "worker_lease", "token"))
            .or_else(|| nested_u64(object, "event_loop_registration", "token"))
            .or_else(|| nested_u64(object, "event", "token")),
        fd: optional_u64(object.get("websocket_fd"))
            .or_else(|| optional_u64(object.get("fd")))
            .or_else(|| nested_u64(object, "event_loop_registration", "fd"))
            .or_else(|| nested_u64(object, "event", "fd")),
    }
}

fn append_keep_registered(actions: &mut Vec<JsonValue>, metadata: &TurnMetadata) {
    if let (Some(token), Some(fd)) = (metadata.token, metadata.fd) {
        actions.push(json!({
            "kind": "keep_websocket_readable_registered",
            "registration": {
                "backend": metadata.backend,
                "shard_index": metadata.shard_index,
                "token": token,
                "fd": fd,
                "interest": "readable",
            },
        }));
    }
}

fn append_keep_read_write_registered(
    actions: &mut Vec<JsonValue>,
    metadata: &TurnMetadata,
    pending_write_bytes: &[u8],
) {
    if let (Some(token), Some(fd)) = (metadata.token, metadata.fd) {
        actions.push(json!({
            "kind": "keep_websocket_read_write_registered",
            "registration": {
                "backend": metadata.backend,
                "shard_index": metadata.shard_index,
                "token": token,
                "fd": fd,
                "interest": "read_write",
                "pending_write_byte_count": pending_write_bytes.len(),
                "pending_write_bytes": bytes_json(pending_write_bytes),
                "pending_write_hex": bytes_hex(pending_write_bytes),
            },
        }));
    }
}

fn append_transport_context_close(
    actions: &mut Vec<JsonValue>,
    metadata: &TurnMetadata,
    reason: &str,
) {
    actions.push(json!({
        "kind": "close_websocket",
        "reason": reason,
    }));
    if let Some(token) = metadata.token {
        actions.push(json!({
            "kind": "unregister_websocket_readable",
            "backend": metadata.backend,
            "shard_index": metadata.shard_index,
            "token": token,
        }));
    }
}

fn append_unregister_close_reconnect(
    actions: &mut Vec<JsonValue>,
    transport: &str,
    metadata: &TurnMetadata,
    reason_code: &str,
    reason: &str,
    preserve_existing_close: bool,
) {
    if !preserve_existing_close || !actions.iter().any(is_close_action) {
        actions.push(json!({
            "kind": "close_websocket",
            "reason": reason,
        }));
    }
    if let Some(token) = metadata.token {
        actions.push(json!({
            "kind": "unregister_websocket_readable",
            "backend": metadata.backend,
            "shard_index": metadata.shard_index,
            "token": token,
        }));
    }
    actions.push(match transport {
        "slack" => json!({
            "kind": "reconnect_socket_mode",
            "reason": reason_code,
        }),
        "discord" => json!({
            "kind": "reconnect_gateway",
            "reason": reason_code,
        }),
        _ => json!({
            "kind": "reconnect_websocket",
            "transport": transport,
            "reason": reason_code,
        }),
    });
}

fn readable_turn_configuration_error(
    transport: &str,
    metadata: &TurnMetadata,
    reason: &str,
    fd_io_result: JsonValue,
    mut actions: Vec<JsonValue>,
) -> JsonValue {
    actions.push(json!({
        "kind": "diagnose_websocket_readable_turn_configuration_error",
        "reason": reason,
    }));
    append_unregister_close_reconnect(
        &mut actions,
        transport,
        metadata,
        "websocket_readable_turn_configuration_error",
        reason,
        false,
    );
    turn_payload(
        "readable_turn",
        "configuration_error",
        transport,
        metadata,
        json!({
            "ok": false,
            "readable": true,
            "hangup": false,
            "read_eof": false,
            "read_source": "fd_io",
            "read_byte_count": 0,
            "read_bytes": [],
            "read_hex": "",
            "read_would_block": false,
            "read_limit_reached": false,
            "fd_io_result": fd_io_result,
            "stream_plan": JsonValue::Null,
            "runtime_plans": [],
            "processed_text_payload_count": 0,
            "remaining_buffer_bytes": [],
            "remaining_buffer_hex": "",
            "should_keep_registered": false,
            "should_unregister": metadata.token.is_some(),
            "should_close_websocket": true,
            "should_reconnect": true,
            "error": reason,
            "actions": actions,
        }),
    )
}

fn writable_turn_configuration_error(
    transport: &str,
    metadata: &TurnMetadata,
    reason: &str,
) -> JsonValue {
    let mut actions = Vec::new();
    actions.push(json!({
        "kind": "diagnose_websocket_writable_turn_configuration_error",
        "reason": reason,
    }));
    append_unregister_close_reconnect(
        &mut actions,
        transport,
        metadata,
        "websocket_writable_turn_configuration_error",
        reason,
        false,
    );
    turn_payload(
        "writable_turn",
        "configuration_error",
        transport,
        metadata,
        json!({
            "ok": false,
            "writable": true,
            "hangup": false,
            "fd_io_result": JsonValue::Null,
            "pending_write_byte_count": 0,
            "pending_write_bytes": [],
            "pending_write_hex": "",
            "remaining_write_byte_count": 0,
            "remaining_write_bytes": [],
            "remaining_write_hex": "",
            "bytes_written": 0,
            "write_complete": false,
            "would_block": false,
            "should_keep_registered": false,
            "should_register_read_write": false,
            "should_unregister": metadata.token.is_some(),
            "should_close_websocket": true,
            "should_reconnect": true,
            "error": reason,
            "actions": actions,
        }),
    )
}

fn is_close_action(action: &JsonValue) -> bool {
    clean_text(action.get("kind")).as_deref() == Some("close_websocket")
}

fn is_reconnect_action(action: &JsonValue) -> bool {
    matches!(
        clean_text(action.get("kind")).as_deref(),
        Some("reconnect_socket_mode" | "reconnect_gateway" | "reconnect_websocket")
    )
}

fn websocket_transport(object: &Map<String, JsonValue>) -> Result<String, String> {
    let transport = clean_text(object.get("transport"))
        .or_else(|| clean_text(object.get("websocket_transport")))
        .or_else(|| nested_text(object, "worker_lease", "transport"))
        .ok_or_else(|| {
            "WebSocket event-loop turn requires transport `slack` or `discord`.".to_string()
        })?;
    match transport
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "slack" | "slack_socket_mode" | "socket_mode" => Ok("slack".to_string()),
        "discord" | "discord_gateway" | "gateway" => Ok("discord".to_string()),
        other => Err(format!(
            "unsupported WebSocket event-loop transport `{other}`"
        )),
    }
}

fn turn_payload(
    stage: &str,
    state: &str,
    transport: &str,
    metadata: &TurnMetadata,
    payload: JsonValue,
) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "websocket_turn_contract".to_string(),
        JsonValue::String(WEBSOCKET_TURN_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "websocket_turn_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "transport".to_string(),
        JsonValue::String(transport.to_string()),
    );
    object.insert(
        "backend".to_string(),
        JsonValue::String(metadata.backend.clone()),
    );
    object.insert(
        "shard_index".to_string(),
        JsonValue::from(metadata.shard_index),
    );
    object.insert(
        "event_loop_token".to_string(),
        metadata
            .token
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    );
    object.insert(
        "websocket_fd".to_string(),
        metadata.fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
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

fn pending_write_bytes(object: &Map<String, JsonValue>) -> Result<Vec<u8>, String> {
    request_optional_bytes(
        object,
        &[
            "pending_write_bytes",
            "remaining_write_bytes",
            "queued_write_bytes",
            "write_bytes",
            "frame_bytes",
        ],
        &[
            "pending_write_hex",
            "remaining_write_hex",
            "queued_write_hex",
            "write_hex",
            "frame_hex",
        ],
        &["pending_write_text", "write_text"],
    )
    .map(|bytes| bytes.unwrap_or_default())
}

fn request_optional_bytes(
    object: &Map<String, JsonValue>,
    byte_keys: &[&str],
    hex_keys: &[&str],
    text_keys: &[&str],
) -> Result<Option<Vec<u8>>, String> {
    for key in byte_keys {
        if let Some(value) = object.get(*key) {
            if let Some(bytes) = json_bytes(value) {
                return Ok(Some(bytes));
            }
            if let Some(text) = value.as_str() {
                return Ok(Some(text.as_bytes().to_vec()));
            }
            return Err(format!(
                "WebSocket writable turn field `{key}` must be a byte array."
            ));
        }
    }
    for key in hex_keys {
        if let Some(value) = object.get(*key) {
            let Some(raw) = value.as_str() else {
                return Err(format!(
                    "WebSocket writable turn field `{key}` must be a hex string."
                ));
            };
            return parse_hex_bytes(raw).map(Some).ok_or_else(|| {
                format!("WebSocket writable turn field `{key}` must be a valid hex string.")
            });
        }
    }
    for key in text_keys {
        if let Some(value) = object.get(*key) {
            let Some(raw) = value.as_str() else {
                return Err(format!(
                    "WebSocket writable turn field `{key}` must be a string."
                ));
            };
            return Ok(Some(raw.as_bytes().to_vec()));
        }
    }
    Ok(None)
}

fn json_bytes(value: &JsonValue) -> Option<Vec<u8>> {
    value.as_array().map(|items| {
        items
            .iter()
            .map(|item| item.as_u64().and_then(|value| u8::try_from(value).ok()))
            .collect::<Option<Vec<_>>>()
    })?
}

fn parse_hex_bytes(raw: &str) -> Option<Vec<u8>> {
    let trimmed = raw.trim();
    let normalized = trimmed
        .strip_prefix("0x")
        .unwrap_or(trimmed)
        .chars()
        .filter(|ch| !ch.is_whitespace() && *ch != '_' && *ch != ':')
        .collect::<String>();
    if normalized.len() % 2 != 0 {
        return None;
    }
    (0..normalized.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&normalized[index..index + 2], 16).ok())
        .collect()
}

fn bytes_json(bytes: &[u8]) -> JsonValue {
    JsonValue::Array(bytes.iter().map(|byte| JsonValue::from(*byte)).collect())
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket event-loop turn request must be an object.".to_string())
}

fn stream_state(stream_plan: &JsonValue) -> Option<&str> {
    stream_plan
        .get("websocket_stream_state")
        .and_then(JsonValue::as_str)
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

fn usize_field(value: &JsonValue, field: &str) -> Option<usize> {
    optional_usize(value.get(field))
}
