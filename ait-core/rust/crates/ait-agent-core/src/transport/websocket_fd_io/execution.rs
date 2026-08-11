use crate::platform::{
    native_socket_from_u64, read_native_socket, set_native_socket_nonblocking, write_native_socket,
    NativeSocket,
};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use std::io;

const MIGRATION_STAGE: &str = "rust_agent_transport_websocket_fd_io_boundary";
const WEBSOCKET_FD_IO_CONTRACT: &str = "ait_agent_core.transport.WebSocketFdIo.v1";
const DEFAULT_MAX_READ_BYTES: usize = 65_536;
const DEFAULT_READ_CHUNK_BYTES: usize = 16_384;

pub fn agent_transport_websocket_fd_io_execute_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "read_ready_fd".to_string());

    match stage.as_str() {
        "read" | "read_ready" | "read_ready_fd" | "readable_fd" => {
            Ok(execute_read_ready_fd(object, "read_ready_fd"))
        }
        "write" | "write_bytes" | "write_frame" => Ok(execute_write_frame(object, "write_frame")),
        other => Err(format!("unsupported WebSocket fd I/O stage: {other}")),
    }
}

fn execute_read_ready_fd(object: &Map<String, JsonValue>, stage: &str) -> JsonValue {
    let fd = match required_fd(object) {
        Ok(fd) => fd,
        Err(message) => return configuration_error_payload(object, stage, &message),
    };
    let set_nonblocking = optional_bool(object.get("set_nonblocking")).unwrap_or(true);
    if set_nonblocking {
        if let Err(err) = set_native_socket_nonblocking(fd, true) {
            return io_error_payload(object, stage, "set_nonblocking", fd, err, json!({}));
        }
    }

    let max_read_bytes = optional_usize(object.get("max_read_bytes"))
        .unwrap_or(DEFAULT_MAX_READ_BYTES)
        .max(1);
    let read_chunk_bytes = optional_usize(object.get("read_chunk_bytes"))
        .unwrap_or(DEFAULT_READ_CHUNK_BYTES)
        .max(1)
        .min(max_read_bytes);
    let mut read_bytes = Vec::new();
    let mut read_attempt_count = 0usize;
    let mut would_block = false;
    let mut read_eof = false;
    let mut read_limit_reached = false;

    while read_bytes.len() < max_read_bytes {
        let remaining = max_read_bytes.saturating_sub(read_bytes.len());
        let chunk_len = read_chunk_bytes.min(remaining);
        let mut buffer = vec![0u8; chunk_len];
        read_attempt_count += 1;
        match read_native_socket(fd, &mut buffer) {
            Ok(received) if received > 0 => {
                read_bytes.extend_from_slice(&buffer[..received]);
                if read_bytes.len() >= max_read_bytes {
                    read_limit_reached = true;
                    break;
                }
                continue;
            }
            Ok(_) => {
                read_eof = true;
                break;
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                would_block = true;
                break;
            }
            Err(err) => {
                return io_error_payload(
                    object,
                    stage,
                    "read",
                    fd,
                    err,
                    json!({
                        "bytes_read": read_bytes.len(),
                        "read_byte_count": read_bytes.len(),
                        "read_bytes": bytes_json(&read_bytes),
                        "read_hex": bytes_hex(&read_bytes),
                        "read_attempt_count": read_attempt_count,
                    }),
                );
            }
        }
    }

    let state = if read_bytes.is_empty() {
        if read_eof {
            "peer_eof"
        } else if would_block {
            "would_block"
        } else if read_limit_reached {
            "read_limit_reached"
        } else {
            "idle"
        }
    } else if read_eof {
        "read_chunk_peer_eof"
    } else if read_limit_reached {
        "read_limit_reached"
    } else {
        "read_chunk"
    };
    let mut actions = Vec::new();
    if !read_bytes.is_empty() {
        actions.push(json!({
            "kind": "deliver_websocket_fd_read_chunk",
            "websocket_fd": fd,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "read_byte_count": read_bytes.len(),
            "read_bytes": bytes_json(&read_bytes),
            "read_hex": bytes_hex(&read_bytes),
        }));
    }
    if would_block {
        actions.push(json!({
            "kind": "mark_websocket_fd_would_block",
            "websocket_fd": fd,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
        }));
    }
    if read_eof {
        actions.push(json!({
            "kind": "mark_websocket_fd_peer_eof",
            "websocket_fd": fd,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
        }));
    }
    if read_limit_reached {
        actions.push(json!({
            "kind": "mark_websocket_fd_read_limit_reached",
            "websocket_fd": fd,
            "max_read_bytes": max_read_bytes,
        }));
    }

    base_payload(
        object,
        stage,
        state,
        json!({
            "ok": true,
            "complete": read_eof || read_limit_reached,
            "nonblocking_requested": set_nonblocking,
            "max_read_bytes": max_read_bytes,
            "read_chunk_bytes": read_chunk_bytes,
            "bytes_read": read_bytes.len(),
            "read_byte_count": read_bytes.len(),
            "read_bytes": bytes_json(&read_bytes),
            "read_hex": bytes_hex(&read_bytes),
            "read_attempt_count": read_attempt_count,
            "read_eof": read_eof,
            "would_block": would_block,
            "read_limit_reached": read_limit_reached,
            "diagnostics": [],
            "actions": actions,
        }),
    )
}

fn execute_write_frame(object: &Map<String, JsonValue>, stage: &str) -> JsonValue {
    let fd = match required_fd(object) {
        Ok(fd) => fd,
        Err(message) => return configuration_error_payload(object, stage, &message),
    };
    let write_bytes = match request_optional_bytes(
        object,
        &["write_bytes", "frame_bytes", "bytes"],
        &["write_hex", "frame_hex", "hex"],
        &["write_text", "text"],
    ) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return configuration_error_payload(
                object,
                stage,
                "WebSocket fd I/O write stage requires write bytes or hex.",
            );
        }
        Err(message) => return configuration_error_payload(object, stage, &message),
    };
    let set_nonblocking = optional_bool(object.get("set_nonblocking")).unwrap_or(true);
    if set_nonblocking {
        if let Err(err) = set_native_socket_nonblocking(fd, true) {
            return io_error_payload(object, stage, "set_nonblocking", fd, err, json!({}));
        }
    }

    let max_write_bytes =
        optional_usize(object.get("max_write_bytes")).unwrap_or(write_bytes.len());
    let write_limit = write_bytes.len().min(max_write_bytes);
    let mut written = 0usize;
    let mut write_attempt_count = 0usize;
    let mut would_block = false;

    while written < write_limit {
        write_attempt_count += 1;
        match write_native_socket(fd, &write_bytes[written..write_limit]) {
            Ok(count) if count > 0 => {
                written += count;
                continue;
            }
            Ok(_) => {
                would_block = true;
                break;
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                would_block = true;
                break;
            }
            Err(err) => {
                return io_error_payload(
                    object,
                    stage,
                    "write",
                    fd,
                    err,
                    json!({
                        "requested_write_byte_count": write_bytes.len(),
                        "bytes_written": written,
                        "written_bytes": bytes_json(&write_bytes[..written]),
                        "written_hex": bytes_hex(&write_bytes[..written]),
                        "remaining_write_bytes": bytes_json(&write_bytes[written..]),
                        "remaining_write_hex": bytes_hex(&write_bytes[written..]),
                        "write_attempt_count": write_attempt_count,
                    }),
                );
            }
        }
    }

    let remaining = &write_bytes[written..];
    let write_complete = remaining.is_empty();
    let write_limited =
        !write_complete && written >= write_limit && max_write_bytes < write_bytes.len();
    let state = if write_complete {
        "write_complete"
    } else if would_block && written == 0 {
        "would_block"
    } else {
        "partial_write"
    };
    let actions = if write_complete {
        vec![json!({
            "kind": "mark_websocket_fd_write_complete",
            "websocket_fd": fd,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "bytes_written": written,
        })]
    } else if would_block && written == 0 {
        vec![json!({
            "kind": "retry_websocket_fd_write_when_writable",
            "websocket_fd": fd,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "remaining_write_bytes": bytes_json(remaining),
            "remaining_write_hex": bytes_hex(remaining),
        })]
    } else {
        vec![json!({
            "kind": "queue_websocket_fd_write_retry",
            "websocket_fd": fd,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "bytes_written": written,
            "remaining_write_bytes": bytes_json(remaining),
            "remaining_write_hex": bytes_hex(remaining),
        })]
    };

    base_payload(
        object,
        stage,
        state,
        json!({
            "ok": true,
            "complete": write_complete,
            "nonblocking_requested": set_nonblocking,
            "requested_write_byte_count": write_bytes.len(),
            "max_write_bytes": max_write_bytes,
            "bytes_written": written,
            "written_bytes": bytes_json(&write_bytes[..written]),
            "written_hex": bytes_hex(&write_bytes[..written]),
            "write_complete": write_complete,
            "would_block": would_block,
            "write_limit_reached": write_limited,
            "remaining_write_byte_count": remaining.len(),
            "remaining_write_bytes": bytes_json(remaining),
            "remaining_write_hex": bytes_hex(remaining),
            "write_attempt_count": write_attempt_count,
            "diagnostics": [],
            "actions": actions,
        }),
    )
}

fn configuration_error_payload(
    object: &Map<String, JsonValue>,
    stage: &str,
    message: &str,
) -> JsonValue {
    base_payload(
        object,
        stage,
        "configuration_error",
        json!({
            "ok": false,
            "complete": false,
            "error": message,
            "diagnostics": [
                {
                    "kind": "websocket_fd_io_configuration_error",
                    "message": message,
                }
            ],
            "actions": [
                {
                    "kind": "diagnose_websocket_fd_io_configuration_error",
                    "message": message,
                }
            ],
        }),
    )
}

fn io_error_payload(
    object: &Map<String, JsonValue>,
    stage: &str,
    syscall: &str,
    fd: NativeSocket,
    err: io::Error,
    extra: JsonValue,
) -> JsonValue {
    let mut payload = extra.as_object().cloned().unwrap_or_default();
    let message = format!("WebSocket fd I/O {syscall} failed: {err}");
    payload.insert("ok".to_string(), JsonValue::Bool(false));
    payload.insert("complete".to_string(), JsonValue::Bool(false));
    payload.insert("error".to_string(), JsonValue::String(message.clone()));
    payload.insert(
        "syscall".to_string(),
        JsonValue::String(syscall.to_string()),
    );
    payload.insert("errno".to_string(), errno_json(&err));
    payload.insert(
        "io_error_kind".to_string(),
        JsonValue::String(format!("{:?}", err.kind())),
    );
    payload.insert(
        "diagnostics".to_string(),
        json!([
            {
                "kind": "websocket_fd_io_transport_error",
                "syscall": syscall,
                "websocket_fd": fd,
                "errno": err.raw_os_error(),
                "message": message,
            }
        ]),
    );
    payload.insert(
        "actions".to_string(),
        json!([
            {
                "kind": "mark_websocket_fd_transport_error",
                "syscall": syscall,
                "websocket_fd": fd,
                "errno": err.raw_os_error(),
                "message": message,
            }
        ]),
    );
    base_payload(object, stage, "io_error", JsonValue::Object(payload))
}

fn base_payload(
    object: &Map<String, JsonValue>,
    stage: &str,
    state: &str,
    payload: JsonValue,
) -> JsonValue {
    let mut object_out = payload.as_object().cloned().unwrap_or_default();
    object_out.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object_out.insert(
        "websocket_fd_io_contract".to_string(),
        JsonValue::String(WEBSOCKET_FD_IO_CONTRACT.to_string()),
    );
    object_out.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object_out.insert(
        "websocket_fd_io_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object_out.insert(
        "rust_transport_io_required".to_string(),
        JsonValue::Bool(true),
    );
    object_out.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object_out.insert(
        "python_websocket_fd_io_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object_out.insert(
        "python_socket_io_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object_out.insert(
        "python_fallback_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object_out.insert(
        "backend".to_string(),
        clean_text(object.get("backend"))
            .map(JsonValue::String)
            .unwrap_or_else(|| JsonValue::String("unspecified".to_string())),
    );
    object_out.insert(
        "shard_index".to_string(),
        optional_u64(object.get("shard_index"))
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    );
    object_out.insert(
        "event_loop_token".to_string(),
        event_loop_token(object)
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    );
    object_out.insert(
        "websocket_fd".to_string(),
        optional_u64(object.get("websocket_fd").or_else(|| object.get("fd")))
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    );
    object_out
        .entry("transport".to_string())
        .or_insert_with(|| {
            clean_text(object.get("transport"))
                .map(JsonValue::String)
                .unwrap_or_else(|| JsonValue::String("websocket".to_string()))
        });
    if let Some(worker_key) = clean_text(object.get("worker_key")) {
        object_out.insert("worker_key".to_string(), JsonValue::String(worker_key));
    }
    JsonValue::Object(object_out)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket fd I/O request must be an object.".to_string())
}

fn required_fd(object: &Map<String, JsonValue>) -> Result<NativeSocket, String> {
    let Some(raw) = optional_u64(object.get("websocket_fd").or_else(|| object.get("fd"))) else {
        return Err("WebSocket fd I/O request requires websocket_fd.".to_string());
    };
    native_socket_from_u64(raw)
        .map_err(|_| "WebSocket fd I/O websocket_fd is outside native socket range.".to_string())
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
                "WebSocket fd I/O field `{key}` must be a byte array."
            ));
        }
    }
    for key in hex_keys {
        if let Some(value) = object.get(*key) {
            let Some(raw) = value.as_str() else {
                return Err(format!(
                    "WebSocket fd I/O field `{key}` must be a hex string."
                ));
            };
            return parse_hex_bytes(raw).map(Some).ok_or_else(|| {
                format!("WebSocket fd I/O field `{key}` must be a valid hex string.")
            });
        }
    }
    for key in text_keys {
        if let Some(value) = object.get(*key) {
            let Some(raw) = value.as_str() else {
                return Err(format!("WebSocket fd I/O field `{key}` must be a string."));
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

fn event_loop_token(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(
        object
            .get("event_loop_token")
            .or_else(|| object.get("token")),
    )
}

fn errno_json(err: &io::Error) -> JsonValue {
    err.raw_os_error()
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null)
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
