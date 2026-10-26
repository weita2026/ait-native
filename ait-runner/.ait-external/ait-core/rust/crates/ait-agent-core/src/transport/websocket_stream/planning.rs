use crate::transport::websocket_frame::agent_transport_websocket_frame_plan_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_transport_websocket_stream_boundary";
const WEBSOCKET_STREAM_CONTRACT: &str = "ait_agent_core.transport.WebSocketStream.v1";
const DEFAULT_MAX_PAYLOAD_BYTES: usize = 1_048_576;
const PROTOCOL_ERROR_CLOSE_STATUS: u16 = 1002;

pub fn agent_transport_websocket_stream_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "consume_read_chunk".to_string());

    match stage.as_str() {
        "consume" | "consume_read_chunk" | "read_chunk" => plan_consume_read_chunk(object),
        other => Err(format!("unsupported WebSocket stream stage: {other}")),
    }
}

fn plan_consume_read_chunk(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let mut stream_bytes = request_optional_bytes(
        object,
        &["buffer_bytes", "receive_buffer_bytes"],
        &["buffer_hex", "receive_buffer_hex"],
    )?
    .unwrap_or_default();
    let prior_buffer_length = stream_bytes.len();
    let read_bytes = request_optional_bytes(
        object,
        &["read_bytes", "chunk_bytes"],
        &["read_hex", "chunk_hex"],
    )?
    .unwrap_or_default();
    let read_byte_count = read_bytes.len();
    stream_bytes.extend_from_slice(&read_bytes);

    let max_payload_bytes = optional_usize(object.get("max_payload_bytes"))
        .unwrap_or(DEFAULT_MAX_PAYLOAD_BYTES)
        .max(1);
    let allow_masked = optional_bool(object.get("allow_masked")).unwrap_or(false);
    let read_eof =
        optional_bool(object.get("read_eof").or_else(|| object.get("eof"))).unwrap_or(false);
    let mask_key = optional_mask_key(object)?;
    let fragment_opcode_value = object
        .get("fragment_opcode")
        .or_else(|| object.get("fragment_kind"));
    let fragment_opcode_candidate = clean_text(fragment_opcode_value);
    let fragment_opcode_explicit = fragment_opcode_value.is_some_and(|value| !value.is_null());
    if fragment_opcode_explicit
        && !fragment_opcode_candidate
            .as_deref()
            .is_some_and(|value| matches!(value, "text" | "binary"))
    {
        return Ok(configuration_error_payload(
            "WebSocket fragmented-message carry requires opcode `text` or `binary`.",
            prior_buffer_length,
            read_byte_count,
            0,
            stream_bytes.len(),
        ));
    }
    let mut fragment_opcode = fragment_opcode_candidate;
    let mut fragment_payload = request_optional_bytes(
        object,
        &["fragment_payload_bytes", "fragment_bytes"],
        &["fragment_payload_hex", "fragment_hex"],
    )?
    .unwrap_or_default();
    let fragment_field_present = object.contains_key("fragment_opcode")
        || object.contains_key("fragment_kind")
        || object.contains_key("fragment_payload_bytes")
        || object.contains_key("fragment_bytes")
        || object.contains_key("fragment_payload_hex")
        || object.contains_key("fragment_hex");
    if fragment_field_present && fragment_opcode.is_none() && !fragment_payload.is_empty() {
        return Ok(configuration_error_payload(
            "WebSocket fragmented-message carry requires opcode `text` or `binary`.",
            prior_buffer_length,
            read_byte_count,
            0,
            stream_bytes.len(),
        ));
    }
    if fragment_payload.len() > max_payload_bytes {
        return Ok(protocol_error_payload(
            "WebSocket fragmented message payload exceeds the configured limit.",
            prior_buffer_length,
            read_byte_count,
            0,
            stream_bytes.len(),
            mask_key.as_ref(),
        ));
    }

    if stream_bytes.is_empty() {
        if read_eof {
            if fragment_opcode.is_some() {
                return Ok(protocol_error_payload(
                    "WebSocket peer closed with an incomplete fragmented message.",
                    prior_buffer_length,
                    read_byte_count,
                    0,
                    0,
                    mask_key.as_ref(),
                ));
            }
            return Ok(peer_closed_payload(
                prior_buffer_length,
                read_byte_count,
                0,
                Vec::new(),
                Vec::new(),
                "peer closed the WebSocket stream.",
            ));
        }
        return Ok(stream_payload(
            "consume_read_chunk",
            if fragment_opcode.is_some() {
                "fragment_in_progress"
            } else {
                "idle"
            },
            json!({
                "ok": true,
                "complete": fragment_opcode.is_none(),
                "prior_buffer_length": prior_buffer_length,
                "read_byte_count": read_byte_count,
                "input_byte_count": 0,
                "consumed_bytes": 0,
                "processed_frame_count": 0,
                "remaining_buffer_length": 0,
                "remaining_buffer_bytes": [],
                "remaining_buffer_hex": "",
                "fragment_in_progress": fragment_opcode.is_some(),
                "fragment_opcode": fragment_opcode.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
                "fragment_payload_length": fragment_payload.len(),
                "fragment_payload_bytes": bytes_json(&fragment_payload),
                "fragment_payload_hex": bytes_hex(&fragment_payload),
                "should_close_websocket": false,
                "should_keep_reading": true,
                "frames": [],
                "actions": [],
            }),
        ));
    }

    let mut cursor = 0usize;
    let mut frames = Vec::new();
    let mut actions = Vec::new();
    let mut stream_state = "frames_dispatched";
    let mut partial_needed: Option<usize> = None;
    let mut should_close_websocket = false;

    while cursor < stream_bytes.len() {
        match decode_one_frame(&stream_bytes[cursor..], max_payload_bytes, allow_masked)? {
            FrameStreamDecode::Decoded(frame) => {
                let consumed_bytes = usize_field(&frame, "consumed_bytes").unwrap_or(0);
                if consumed_bytes == 0 {
                    return Ok(protocol_error_payload(
                        "WebSocket frame decoder consumed zero bytes.",
                        prior_buffer_length,
                        read_byte_count,
                        cursor,
                        stream_bytes.len().saturating_sub(cursor),
                        mask_key.as_ref(),
                    ));
                }
                let opcode_name = clean_text(frame.get("opcode_name")).unwrap_or_default();
                let fin = optional_bool(frame.get("fin")).unwrap_or(false);
                let payload_bytes =
                    json_bytes(frame.get("payload_bytes").unwrap_or(&JsonValue::Null))
                        .unwrap_or_default();
                if opcode_name == "text"
                    && fin
                    && frame
                        .get("payload_text")
                        .and_then(JsonValue::as_str)
                        .is_none()
                {
                    return Ok(protocol_error_payload(
                        "WebSocket text message must be valid UTF-8.",
                        prior_buffer_length,
                        read_byte_count,
                        cursor,
                        stream_bytes.len().saturating_sub(cursor),
                        mask_key.as_ref(),
                    ));
                }
                let frame_actions = match opcode_name.as_str() {
                    "text" | "binary" if fragment_opcode.is_some() => {
                        return Ok(protocol_error_payload(
                            "WebSocket received a new data frame before the fragmented message completed.",
                            prior_buffer_length,
                            read_byte_count,
                            cursor,
                            stream_bytes.len().saturating_sub(cursor),
                            mask_key.as_ref(),
                        ));
                    }
                    "text" | "binary" if !fin => {
                        fragment_opcode = Some(opcode_name.clone());
                        fragment_payload = payload_bytes;
                        stream_state = "fragment_in_progress";
                        Vec::new()
                    }
                    "continuation" => {
                        let Some(active_opcode) = fragment_opcode.clone() else {
                            return Ok(protocol_error_payload(
                                "WebSocket continuation frame arrived without an active fragmented message.",
                                prior_buffer_length,
                                read_byte_count,
                                cursor,
                                stream_bytes.len().saturating_sub(cursor),
                                mask_key.as_ref(),
                            ));
                        };
                        if fragment_payload.len().saturating_add(payload_bytes.len())
                            > max_payload_bytes
                        {
                            return Ok(protocol_error_payload(
                                "WebSocket fragmented message payload exceeds the configured limit.",
                                prior_buffer_length,
                                read_byte_count,
                                cursor,
                                stream_bytes.len().saturating_sub(cursor),
                                mask_key.as_ref(),
                            ));
                        }
                        fragment_payload.extend_from_slice(&payload_bytes);
                        if fin {
                            let assembled = match assembled_fragment_action(
                                &active_opcode,
                                &fragment_payload,
                            ) {
                                Ok(action) => action,
                                Err(message) => {
                                    return Ok(protocol_error_payload(
                                        &message,
                                        prior_buffer_length,
                                        read_byte_count,
                                        cursor,
                                        stream_bytes.len().saturating_sub(cursor),
                                        mask_key.as_ref(),
                                    ));
                                }
                            };
                            fragment_opcode = None;
                            fragment_payload.clear();
                            vec![assembled]
                        } else {
                            stream_state = "fragment_in_progress";
                            Vec::new()
                        }
                    }
                    _ => match stream_actions_for_frame(&frame, mask_key.as_ref()) {
                        Ok(frame_actions) => frame_actions,
                        Err(message) => {
                            return Ok(configuration_error_payload(
                                &message,
                                prior_buffer_length,
                                read_byte_count,
                                cursor,
                                stream_bytes.len().saturating_sub(cursor),
                            ));
                        }
                    },
                };
                actions.extend(frame_actions);
                frames.push(frame);
                cursor += consumed_bytes;
                if opcode_name == "close" {
                    stream_state = "closing";
                    should_close_websocket = true;
                    break;
                }
            }
            FrameStreamDecode::Partial { needed_bytes } => {
                partial_needed = Some(needed_bytes);
                stream_state = if frames.is_empty() {
                    "partial_frame"
                } else {
                    "frames_dispatched_with_partial"
                };
                break;
            }
            FrameStreamDecode::ProtocolError { message } => {
                return Ok(protocol_error_payload(
                    &message,
                    prior_buffer_length,
                    read_byte_count,
                    cursor,
                    stream_bytes.len().saturating_sub(cursor),
                    mask_key.as_ref(),
                ));
            }
        }
    }

    let remaining = stream_bytes[cursor..].to_vec();
    if read_eof && !remaining.is_empty() {
        return Ok(protocol_error_payload(
            "WebSocket peer closed with an incomplete frame buffered.",
            prior_buffer_length,
            read_byte_count,
            cursor,
            remaining.len(),
            mask_key.as_ref(),
        ));
    }
    if read_eof && fragment_opcode.is_some() {
        return Ok(protocol_error_payload(
            "WebSocket peer closed with an incomplete fragmented message.",
            prior_buffer_length,
            read_byte_count,
            cursor,
            remaining.len(),
            mask_key.as_ref(),
        ));
    }
    if read_eof && !should_close_websocket {
        actions.push(json!({
            "kind": "close_websocket",
            "reason": "peer closed the WebSocket stream.",
        }));
        stream_state = if frames.is_empty() {
            "peer_closed"
        } else {
            "frames_dispatched_then_peer_closed"
        };
        should_close_websocket = true;
    }

    let needed_additional_bytes = partial_needed
        .map(|needed| needed.saturating_sub(remaining.len()))
        .unwrap_or(0);
    let complete = remaining.is_empty() && partial_needed.is_none() && fragment_opcode.is_none();
    let processed_frame_count = frames.len();
    if fragment_opcode.is_some() && partial_needed.is_some() {
        stream_state = "fragment_in_progress_with_partial_frame";
    } else if fragment_opcode.is_some() {
        stream_state = "fragment_in_progress";
    }

    Ok(stream_payload(
        "consume_read_chunk",
        stream_state,
        json!({
            "ok": true,
            "complete": complete,
            "prior_buffer_length": prior_buffer_length,
            "read_byte_count": read_byte_count,
            "input_byte_count": stream_bytes.len(),
            "consumed_bytes": cursor,
            "processed_frame_count": processed_frame_count,
            "remaining_buffer_length": remaining.len(),
            "remaining_buffer_bytes": bytes_json(&remaining),
            "remaining_buffer_hex": bytes_hex(&remaining),
            "needed_bytes": partial_needed.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "needed_additional_bytes": if partial_needed.is_some() {
                JsonValue::from(needed_additional_bytes)
            } else {
                JsonValue::Null
            },
            "fragment_in_progress": fragment_opcode.is_some(),
            "fragment_opcode": fragment_opcode.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
            "fragment_payload_length": fragment_payload.len(),
            "fragment_payload_bytes": bytes_json(&fragment_payload),
            "fragment_payload_hex": bytes_hex(&fragment_payload),
            "should_close_websocket": should_close_websocket,
            "should_keep_reading": !should_close_websocket,
            "frames": frames,
            "actions": actions,
        }),
    ))
}

fn assembled_fragment_action(opcode: &str, payload: &[u8]) -> Result<JsonValue, String> {
    match opcode {
        "text" => String::from_utf8(payload.to_vec())
            .map(|payload_text| {
                json!({
                    "kind": "deliver_websocket_text",
                    "payload_text": payload_text,
                    "fragmented": true,
                })
            })
            .map_err(|_| "WebSocket fragmented text message must be valid UTF-8.".to_string()),
        "binary" => Ok(json!({
            "kind": "deliver_websocket_binary",
            "payload_bytes": bytes_json(payload),
            "fragmented": true,
        })),
        _ => Err("WebSocket fragmented-message opcode is invalid.".to_string()),
    }
}

fn stream_actions_for_frame(
    frame: &JsonValue,
    mask_key: Option<&[u8; 4]>,
) -> Result<Vec<JsonValue>, String> {
    let opcode_name = clean_text(frame.get("opcode_name")).unwrap_or_default();
    let payload_bytes =
        json_bytes(frame.get("payload_bytes").unwrap_or(&JsonValue::Null)).unwrap_or_default();
    let mut actions = Vec::new();
    match opcode_name.as_str() {
        "text" => actions.push(json!({
            "kind": "deliver_websocket_text",
            "payload_text": frame.get("payload_text").cloned().unwrap_or(JsonValue::Null),
            "payload_bytes": bytes_json(&payload_bytes),
        })),
        "binary" => actions.push(json!({
            "kind": "deliver_websocket_binary",
            "payload_bytes": bytes_json(&payload_bytes),
        })),
        "ping" => {
            let frame = encode_control_frame("pong", &payload_bytes, mask_key)?;
            actions.push(json!({
                "kind": "write_websocket_frame",
                "opcode": "pong",
                "payload_bytes": bytes_json(&payload_bytes),
                "frame_bytes": frame["frame_bytes"].clone(),
                "frame_hex": frame["frame_hex"].clone(),
                "execute_before_payload_dispatch": true,
                "source": "websocket_ping",
            }));
        }
        "pong" => actions.push(json!({
            "kind": "mark_websocket_pong",
        })),
        "close" => {
            let close_frame = encode_control_frame("close", &payload_bytes, mask_key)?;
            actions.push(json!({
                "kind": "write_websocket_frame",
                "opcode": "close",
                "payload_bytes": bytes_json(&payload_bytes),
                "frame_bytes": close_frame["frame_bytes"].clone(),
                "frame_hex": close_frame["frame_hex"].clone(),
                "source": "websocket_close",
            }));
            actions.push(json!({
                "kind": "close_websocket",
                "status_code": frame.get("close_status_code").cloned().unwrap_or(JsonValue::Null),
                "reason": frame.get("close_reason").cloned().unwrap_or(JsonValue::Null),
            }));
        }
        _ => {}
    }
    Ok(actions)
}

fn encode_control_frame(
    opcode: &str,
    payload_bytes: &[u8],
    mask_key: Option<&[u8; 4]>,
) -> Result<JsonValue, String> {
    let Some(mask_key) = mask_key else {
        return Err(
            "WebSocket stream control response requires an explicit 4-byte mask_key.".to_string(),
        );
    };
    agent_transport_websocket_frame_plan_json(&json!({
        "stage": "encode",
        "opcode": opcode,
        "payload_bytes": bytes_json(payload_bytes),
        "mask_key": bytes_json(mask_key),
    }))
}

fn encode_protocol_close_frame(mask_key: &[u8; 4]) -> Option<JsonValue> {
    agent_transport_websocket_frame_plan_json(&json!({
        "stage": "encode",
        "opcode": "close",
        "status_code": PROTOCOL_ERROR_CLOSE_STATUS,
        "reason": "protocol error",
        "mask_key": bytes_json(mask_key),
    }))
    .ok()
}

fn decode_one_frame(
    bytes: &[u8],
    max_payload_bytes: usize,
    allow_masked: bool,
) -> Result<FrameStreamDecode, String> {
    let decoded = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_bytes": bytes_json(bytes),
        "max_payload_bytes": max_payload_bytes,
        "allow_masked": allow_masked,
    }))?;
    match clean_text(decoded.get("websocket_frame_state")).as_deref() {
        Some("decoded") => Ok(FrameStreamDecode::Decoded(decoded)),
        Some("partial_frame") => Ok(FrameStreamDecode::Partial {
            needed_bytes: usize_field(&decoded, "needed_bytes").unwrap_or(bytes.len() + 1),
        }),
        Some("protocol_error") => Ok(FrameStreamDecode::ProtocolError {
            message: clean_text(decoded.get("error"))
                .unwrap_or_else(|| "WebSocket frame protocol error.".to_string()),
        }),
        _ => Err("WebSocket frame decoder returned an unsupported state.".to_string()),
    }
}

fn protocol_error_payload(
    message: &str,
    prior_buffer_length: usize,
    read_byte_count: usize,
    consumed_bytes: usize,
    discarded_buffer_bytes: usize,
    mask_key: Option<&[u8; 4]>,
) -> JsonValue {
    let mut actions = Vec::new();
    if let Some(mask_key) = mask_key {
        if let Some(close_frame) = encode_protocol_close_frame(mask_key) {
            actions.push(json!({
                "kind": "write_websocket_frame",
                "opcode": "close",
                "status_code": PROTOCOL_ERROR_CLOSE_STATUS,
                "reason": "protocol error",
                "frame_bytes": close_frame["frame_bytes"].clone(),
                "frame_hex": close_frame["frame_hex"].clone(),
                "source": "websocket_protocol_error",
            }));
        }
    }
    actions.push(json!({
        "kind": "close_websocket",
        "reason": message,
    }));

    stream_payload(
        "consume_read_chunk",
        "protocol_error",
        json!({
            "ok": false,
            "complete": false,
            "error": message,
            "prior_buffer_length": prior_buffer_length,
            "read_byte_count": read_byte_count,
            "consumed_bytes": consumed_bytes,
            "discarded_buffer_bytes": discarded_buffer_bytes,
            "processed_frame_count": 0,
            "remaining_buffer_length": 0,
            "remaining_buffer_bytes": [],
            "remaining_buffer_hex": "",
            "should_close_websocket": true,
            "should_keep_reading": false,
            "frames": [],
            "actions": actions,
        }),
    )
}

fn configuration_error_payload(
    message: &str,
    prior_buffer_length: usize,
    read_byte_count: usize,
    consumed_bytes: usize,
    discarded_buffer_bytes: usize,
) -> JsonValue {
    stream_payload(
        "consume_read_chunk",
        "configuration_error",
        json!({
            "ok": false,
            "complete": false,
            "error": message,
            "prior_buffer_length": prior_buffer_length,
            "read_byte_count": read_byte_count,
            "consumed_bytes": consumed_bytes,
            "discarded_buffer_bytes": discarded_buffer_bytes,
            "processed_frame_count": 0,
            "remaining_buffer_length": 0,
            "remaining_buffer_bytes": [],
            "remaining_buffer_hex": "",
            "should_close_websocket": true,
            "should_keep_reading": false,
            "frames": [],
            "actions": [
                {
                    "kind": "close_websocket",
                    "reason": message,
                }
            ],
        }),
    )
}

fn peer_closed_payload(
    prior_buffer_length: usize,
    read_byte_count: usize,
    consumed_bytes: usize,
    frames: Vec<JsonValue>,
    mut actions: Vec<JsonValue>,
    reason: &str,
) -> JsonValue {
    actions.push(json!({
        "kind": "close_websocket",
        "reason": reason,
    }));
    stream_payload(
        "consume_read_chunk",
        "peer_closed",
        json!({
            "ok": true,
            "complete": true,
            "prior_buffer_length": prior_buffer_length,
            "read_byte_count": read_byte_count,
            "input_byte_count": prior_buffer_length + read_byte_count,
            "consumed_bytes": consumed_bytes,
            "processed_frame_count": frames.len(),
            "remaining_buffer_length": 0,
            "remaining_buffer_bytes": [],
            "remaining_buffer_hex": "",
            "should_close_websocket": true,
            "should_keep_reading": false,
            "frames": frames,
            "actions": actions,
        }),
    )
}

fn stream_payload(stage: &str, state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "websocket_stream_contract".to_string(),
        JsonValue::String(WEBSOCKET_STREAM_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "websocket_stream_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_websocket_stream_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_websocket_event_loop_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object
        .entry("transport".to_string())
        .or_insert_with(|| JsonValue::String("websocket".to_string()));
    JsonValue::Object(object)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket stream request must be an object.".to_string())
}

fn request_optional_bytes(
    object: &Map<String, JsonValue>,
    byte_keys: &[&str],
    hex_keys: &[&str],
) -> Result<Option<Vec<u8>>, String> {
    for key in byte_keys {
        if let Some(value) = object.get(*key) {
            return json_bytes(value)
                .map(Some)
                .ok_or_else(|| format!("WebSocket stream field `{key}` must be a byte array."));
        }
    }
    for key in hex_keys {
        if let Some(value) = object.get(*key) {
            let Some(raw) = value.as_str() else {
                return Err(format!(
                    "WebSocket stream field `{key}` must be a hex string."
                ));
            };
            return parse_hex_bytes(raw).map(Some).ok_or_else(|| {
                format!("WebSocket stream field `{key}` must be a valid hex string.")
            });
        }
    }
    Ok(None)
}

fn optional_mask_key(object: &Map<String, JsonValue>) -> Result<Option<[u8; 4]>, String> {
    let Some(value) = object
        .get("mask_key")
        .or_else(|| object.get("control_mask_key"))
        .or_else(|| object.get("mask"))
    else {
        return Ok(None);
    };
    let bytes = json_bytes(value).or_else(|| value.as_str().and_then(parse_hex_bytes));
    let Some(bytes) = bytes else {
        return Err("WebSocket stream mask_key must be 4 bytes.".to_string());
    };
    if bytes.len() != 4 {
        return Err("WebSocket stream mask_key must be 4 bytes.".to_string());
    }
    Ok(Some([bytes[0], bytes[1], bytes[2], bytes[3]]))
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
    let normalized = raw
        .trim()
        .strip_prefix("0x")
        .unwrap_or(raw.trim())
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

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    match value? {
        JsonValue::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok()),
        JsonValue::String(text) => text.trim().parse::<usize>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn usize_field(object: &JsonValue, key: &str) -> Option<usize> {
    object
        .get(key)
        .and_then(|value| value.as_u64())
        .and_then(|value| usize::try_from(value).ok())
}

enum FrameStreamDecode {
    Decoded(JsonValue),
    Partial { needed_bytes: usize },
    ProtocolError { message: String },
}
