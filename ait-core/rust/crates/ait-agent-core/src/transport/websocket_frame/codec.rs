use crate::json_support::encode_value_to_vec;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_transport_websocket_frame_codec";
const WEBSOCKET_FRAME_CONTRACT: &str = "ait_agent_core.transport.WebSocketFrame.v1";
const DEFAULT_MAX_PAYLOAD_BYTES: usize = 1_048_576;

const OPCODE_CONTINUATION: u8 = 0x0;
const OPCODE_TEXT: u8 = 0x1;
const OPCODE_BINARY: u8 = 0x2;
const OPCODE_CLOSE: u8 = 0x8;
const OPCODE_PING: u8 = 0x9;
const OPCODE_PONG: u8 = 0xA;

pub fn agent_transport_websocket_frame_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "decode".to_string());

    match stage.as_str() {
        "encode" | "encode_frame" => plan_encode(object),
        "decode" | "decode_frame" => plan_decode(object),
        other => Err(format!("unsupported WebSocket frame codec stage: {other}")),
    }
}

fn plan_encode(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let opcode = opcode_from_request(object)?;
    let opcode_name = opcode_name(opcode);
    let client_to_server = optional_bool(object.get("client_to_server")).unwrap_or(true);
    let mask_key = if client_to_server {
        Some(required_mask_key(object)?)
    } else {
        optional_mask_key(object)?
    };
    let payload = encode_payload(object, opcode)?;
    validate_outbound_frame(opcode, &payload)?;
    let frame_bytes = encode_frame(opcode, &payload, mask_key.as_ref());

    Ok(base_payload(
        "encode",
        "encoded",
        json!({
            "ok": true,
            "complete": true,
            "opcode": opcode,
            "opcode_name": opcode_name,
            "fin": true,
            "masked": mask_key.is_some(),
            "mask_key": mask_key
                .as_ref()
                .map(|key| bytes_json(key.as_slice()))
                .unwrap_or(JsonValue::Null),
            "payload_length": payload.len(),
            "payload_bytes": bytes_json(&payload),
            "payload_text": if opcode == OPCODE_TEXT {
                String::from_utf8(payload.clone()).ok().map(JsonValue::from).unwrap_or(JsonValue::Null)
            } else {
                JsonValue::Null
            },
            "frame_bytes": bytes_json(&frame_bytes),
            "frame_hex": bytes_hex(&frame_bytes),
            "actions": [
                {
                    "kind": "write_websocket_frame",
                    "opcode": opcode_name,
                    "frame_bytes": bytes_json(&frame_bytes),
                }
            ],
        }),
    ))
}

fn plan_decode(object: &Map<String, JsonValue>) -> Result<JsonValue, String> {
    let frame_bytes = request_bytes(object, "frame_bytes", "frame_hex")?;
    let max_payload_bytes = optional_usize(object.get("max_payload_bytes"))
        .unwrap_or(DEFAULT_MAX_PAYLOAD_BYTES)
        .max(1);
    let allow_masked = optional_bool(object.get("allow_masked")).unwrap_or(false);

    match decode_frame(&frame_bytes, max_payload_bytes, allow_masked) {
        DecodeOutcome::Frame(frame) => Ok(decoded_payload(frame)),
        DecodeOutcome::Partial { needed_bytes } => Ok(base_payload(
            "decode",
            "partial_frame",
            json!({
                "ok": true,
                "complete": false,
                "needed_bytes": needed_bytes,
                "consumed_bytes": 0,
                "actions": [],
            }),
        )),
        DecodeOutcome::ProtocolError { message } => Ok(protocol_error_payload(&message)),
    }
}

fn decoded_payload(frame: DecodedFrame) -> JsonValue {
    let opcode_name = opcode_name(frame.opcode);
    let payload_text = String::from_utf8(frame.payload.clone())
        .ok()
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);
    let close_detail = if frame.opcode == OPCODE_CLOSE {
        close_detail(&frame.payload)
    } else {
        None
    };
    let actions = decoded_actions(&frame, &payload_text, &close_detail);

    base_payload(
        "decode",
        "decoded",
        json!({
            "ok": true,
            "complete": true,
            "fin": frame.fin,
            "opcode": frame.opcode,
            "opcode_name": opcode_name,
            "masked": frame.masked,
            "payload_length": frame.payload.len(),
            "payload_bytes": bytes_json(&frame.payload),
            "payload_text": payload_text,
            "close_status_code": close_detail
                .as_ref()
                .and_then(|detail| detail.status_code)
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
            "close_reason": close_detail
                .as_ref()
                .and_then(|detail| detail.reason.clone())
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
            "consumed_bytes": frame.consumed_bytes,
            "fragmented": !frame.fin || frame.opcode == OPCODE_CONTINUATION,
            "continuation": frame.opcode == OPCODE_CONTINUATION,
            "should_deliver_text": frame.fin && frame.opcode == OPCODE_TEXT,
            "should_deliver_binary": frame.fin && frame.opcode == OPCODE_BINARY,
            "should_send_pong": frame.opcode == OPCODE_PING,
            "should_mark_pong": frame.opcode == OPCODE_PONG,
            "should_close_websocket": frame.opcode == OPCODE_CLOSE,
            "actions": actions,
        }),
    )
}

fn decoded_actions(
    frame: &DecodedFrame,
    payload_text: &JsonValue,
    close_detail: &Option<CloseDetail>,
) -> JsonValue {
    match frame.opcode {
        OPCODE_TEXT if frame.fin => json!([
            {
                "kind": "deliver_websocket_text",
                "payload_text": payload_text,
            }
        ]),
        OPCODE_BINARY if frame.fin => json!([
            {
                "kind": "deliver_websocket_binary",
                "payload_bytes": bytes_json(&frame.payload),
            }
        ]),
        OPCODE_PING => json!([
            {
                "kind": "send_websocket_pong",
                "payload_bytes": bytes_json(&frame.payload),
                "execute_before_payload_dispatch": true,
            }
        ]),
        OPCODE_PONG => json!([
            {
                "kind": "mark_websocket_pong",
            }
        ]),
        OPCODE_CLOSE => json!([
            {
                "kind": "close_websocket",
                "status_code": close_detail
                    .as_ref()
                    .and_then(|detail| detail.status_code)
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null),
                "reason": close_detail
                    .as_ref()
                    .and_then(|detail| detail.reason.clone())
                    .map(JsonValue::from)
                    .unwrap_or(JsonValue::Null),
            }
        ]),
        _ => json!([]),
    }
}

fn decode_frame(bytes: &[u8], max_payload_bytes: usize, allow_masked: bool) -> DecodeOutcome {
    if bytes.len() < 2 {
        return DecodeOutcome::Partial { needed_bytes: 2 };
    }
    let first = bytes[0];
    let second = bytes[1];
    let fin = first & 0x80 != 0;
    let rsv = first & 0x70;
    let opcode = first & 0x0F;
    if rsv != 0 {
        return protocol_error("WebSocket frame RSV bits are not supported.");
    }
    if !valid_opcode(opcode) {
        return protocol_error("WebSocket frame opcode is invalid.");
    }
    if is_control_opcode(opcode) && !fin {
        return protocol_error("WebSocket control frames must not be fragmented.");
    }

    let masked = second & 0x80 != 0;
    if masked && !allow_masked {
        return protocol_error("WebSocket server-to-client frame must not be masked.");
    }
    let initial_len = (second & 0x7F) as usize;
    let mut cursor = 2usize;
    let payload_len = match initial_len {
        126 => {
            if bytes.len() < cursor + 2 {
                return DecodeOutcome::Partial {
                    needed_bytes: cursor + 2,
                };
            }
            let len = u16::from_be_bytes([bytes[cursor], bytes[cursor + 1]]) as usize;
            cursor += 2;
            len
        }
        127 => {
            if bytes.len() < cursor + 8 {
                return DecodeOutcome::Partial {
                    needed_bytes: cursor + 8,
                };
            }
            let len = u64::from_be_bytes([
                bytes[cursor],
                bytes[cursor + 1],
                bytes[cursor + 2],
                bytes[cursor + 3],
                bytes[cursor + 4],
                bytes[cursor + 5],
                bytes[cursor + 6],
                bytes[cursor + 7],
            ]);
            cursor += 8;
            match usize::try_from(len) {
                Ok(value) => value,
                Err(_) => return protocol_error("WebSocket frame payload length is too large."),
            }
        }
        value => value,
    };
    if payload_len > max_payload_bytes {
        return protocol_error("WebSocket frame payload length exceeds the configured limit.");
    }
    if is_control_opcode(opcode) && payload_len > 125 {
        return protocol_error("WebSocket control frame payload is too large.");
    }
    let mask_key = if masked {
        if bytes.len() < cursor + 4 {
            return DecodeOutcome::Partial {
                needed_bytes: cursor + 4,
            };
        }
        let key = [
            bytes[cursor],
            bytes[cursor + 1],
            bytes[cursor + 2],
            bytes[cursor + 3],
        ];
        cursor += 4;
        Some(key)
    } else {
        None
    };
    let frame_end = cursor + payload_len;
    if bytes.len() < frame_end {
        return DecodeOutcome::Partial {
            needed_bytes: frame_end,
        };
    }
    let mut payload = bytes[cursor..frame_end].to_vec();
    if let Some(mask_key) = mask_key {
        apply_mask(&mut payload, &mask_key);
    }
    if opcode == OPCODE_CLOSE && close_detail(&payload).is_none() && !payload.is_empty() {
        return protocol_error("WebSocket close frame payload is malformed.");
    }

    DecodeOutcome::Frame(DecodedFrame {
        fin,
        opcode,
        masked,
        payload,
        consumed_bytes: frame_end,
    })
}

fn encode_frame(opcode: u8, payload: &[u8], mask_key: Option<&[u8; 4]>) -> Vec<u8> {
    let mut frame = Vec::new();
    frame.push(0x80 | opcode);
    let mask_bit = if mask_key.is_some() { 0x80 } else { 0x00 };
    let len = payload.len();
    if len <= 125 {
        frame.push(mask_bit | len as u8);
    } else if len <= u16::MAX as usize {
        frame.push(mask_bit | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(mask_bit | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    if let Some(mask_key) = mask_key {
        frame.extend_from_slice(mask_key);
        let mut masked = payload.to_vec();
        apply_mask(&mut masked, mask_key);
        frame.extend_from_slice(&masked);
    } else {
        frame.extend_from_slice(payload);
    }
    frame
}

fn encode_payload(object: &Map<String, JsonValue>, opcode: u8) -> Result<Vec<u8>, String> {
    if opcode == OPCODE_CLOSE {
        return close_payload(object);
    }
    if let Some(bytes) = optional_request_bytes(object, "payload_bytes", "payload_hex")? {
        return Ok(bytes);
    }
    if let Some(text) =
        clean_text(object.get("payload_text")).or_else(|| clean_text(object.get("text")))
    {
        return Ok(text.into_bytes());
    }
    if let Some(payload) = object.get("payload") {
        if payload.is_null() {
            return Ok(Vec::new());
        }
        if let Some(text) = payload.as_str() {
            return Ok(text.as_bytes().to_vec());
        }
        return encode_value_to_vec(payload, "failed to encode WebSocket JSON payload");
    }
    Ok(Vec::new())
}

fn close_payload(object: &Map<String, JsonValue>) -> Result<Vec<u8>, String> {
    if let Some(bytes) = optional_request_bytes(object, "payload_bytes", "payload_hex")? {
        return Ok(bytes);
    }
    let status_code = optional_u16(object.get("status_code"))
        .or_else(|| optional_u16(object.get("close_status_code")));
    let reason =
        clean_text(object.get("reason")).or_else(|| clean_text(object.get("close_reason")));
    let mut payload = Vec::new();
    if let Some(status_code) = status_code {
        payload.extend_from_slice(&status_code.to_be_bytes());
    }
    if let Some(reason) = reason {
        payload.extend_from_slice(reason.as_bytes());
    }
    Ok(payload)
}

fn validate_outbound_frame(opcode: u8, payload: &[u8]) -> Result<(), String> {
    if !valid_opcode(opcode) || opcode == OPCODE_CONTINUATION {
        return Err("WebSocket frame opcode is invalid.".to_string());
    }
    if is_control_opcode(opcode) && payload.len() > 125 {
        return Err("WebSocket control frame payload is too large.".to_string());
    }
    if opcode == OPCODE_CLOSE && close_detail(payload).is_none() && !payload.is_empty() {
        return Err("WebSocket close frame payload is malformed.".to_string());
    }
    Ok(())
}

fn opcode_from_request(object: &Map<String, JsonValue>) -> Result<u8, String> {
    if let Some(value) = optional_u8(object.get("opcode")) {
        if valid_opcode(value) {
            return Ok(value);
        }
        return Err("WebSocket frame opcode is invalid.".to_string());
    }
    let raw = clean_text(object.get("opcode"))
        .or_else(|| clean_text(object.get("opcode_name")))
        .unwrap_or_else(|| "text".to_string());
    match raw.trim().to_ascii_lowercase().as_str() {
        "text" => Ok(OPCODE_TEXT),
        "binary" => Ok(OPCODE_BINARY),
        "close" => Ok(OPCODE_CLOSE),
        "ping" => Ok(OPCODE_PING),
        "pong" => Ok(OPCODE_PONG),
        _ => Err("WebSocket frame opcode is invalid.".to_string()),
    }
}

fn required_mask_key(object: &Map<String, JsonValue>) -> Result<[u8; 4], String> {
    optional_mask_key(object)?.ok_or_else(|| {
        "WebSocket client-to-server frames require an explicit 4-byte mask_key.".to_string()
    })
}

fn optional_mask_key(object: &Map<String, JsonValue>) -> Result<Option<[u8; 4]>, String> {
    let Some(value) = object.get("mask_key").or_else(|| object.get("mask")) else {
        return Ok(None);
    };
    let bytes = json_bytes(value).or_else(|| value.as_str().and_then(parse_hex_bytes));
    let Some(bytes) = bytes else {
        return Err("WebSocket mask_key must be 4 bytes.".to_string());
    };
    if bytes.len() != 4 {
        return Err("WebSocket mask_key must be 4 bytes.".to_string());
    }
    Ok(Some([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

fn request_bytes(
    object: &Map<String, JsonValue>,
    bytes_key: &str,
    hex_key: &str,
) -> Result<Vec<u8>, String> {
    optional_request_bytes(object, bytes_key, hex_key)?.ok_or_else(|| {
        format!("WebSocket frame request must include `{bytes_key}` or `{hex_key}`.")
    })
}

fn optional_request_bytes(
    object: &Map<String, JsonValue>,
    bytes_key: &str,
    hex_key: &str,
) -> Result<Option<Vec<u8>>, String> {
    if let Some(value) = object.get(bytes_key) {
        return json_bytes(value)
            .map(Some)
            .ok_or_else(|| format!("WebSocket frame field `{bytes_key}` must be a byte array."));
    }
    if let Some(value) = object.get(hex_key) {
        let Some(raw) = value.as_str() else {
            return Err(format!(
                "WebSocket frame field `{hex_key}` must be a hex string."
            ));
        };
        return parse_hex_bytes(raw).map(Some).ok_or_else(|| {
            format!("WebSocket frame field `{hex_key}` must be a valid hex string.")
        });
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

fn close_detail(payload: &[u8]) -> Option<CloseDetail> {
    if payload.is_empty() {
        return Some(CloseDetail {
            status_code: None,
            reason: None,
        });
    }
    if payload.len() == 1 {
        return None;
    }
    let status_code = u16::from_be_bytes([payload[0], payload[1]]);
    let reason = if payload.len() > 2 {
        match String::from_utf8(payload[2..].to_vec()) {
            Ok(reason) => Some(reason),
            Err(_) => return None,
        }
    } else {
        None
    };
    Some(CloseDetail {
        status_code: Some(status_code),
        reason,
    })
}

fn valid_opcode(opcode: u8) -> bool {
    matches!(
        opcode,
        OPCODE_CONTINUATION
            | OPCODE_TEXT
            | OPCODE_BINARY
            | OPCODE_CLOSE
            | OPCODE_PING
            | OPCODE_PONG
    )
}

fn is_control_opcode(opcode: u8) -> bool {
    matches!(opcode, OPCODE_CLOSE | OPCODE_PING | OPCODE_PONG)
}

fn opcode_name(opcode: u8) -> &'static str {
    match opcode {
        OPCODE_TEXT => "text",
        OPCODE_BINARY => "binary",
        OPCODE_CLOSE => "close",
        OPCODE_PING => "ping",
        OPCODE_PONG => "pong",
        OPCODE_CONTINUATION => "continuation",
        _ => "unknown",
    }
}

fn apply_mask(payload: &mut [u8], mask_key: &[u8; 4]) {
    for (index, byte) in payload.iter_mut().enumerate() {
        *byte ^= mask_key[index % 4];
    }
}

fn protocol_error(message: &str) -> DecodeOutcome {
    DecodeOutcome::ProtocolError {
        message: message.to_string(),
    }
}

fn protocol_error_payload(message: &str) -> JsonValue {
    base_payload(
        "decode",
        "protocol_error",
        json!({
            "ok": false,
            "complete": false,
            "error": message,
            "should_close_websocket": true,
            "actions": [
                {
                    "kind": "close_websocket",
                    "reason": message,
                }
            ],
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
        "websocket_frame_contract".to_string(),
        JsonValue::String(WEBSOCKET_FRAME_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "websocket_frame_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_websocket_frame_codec_allowed".to_string(),
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
        .ok_or_else(|| "WebSocket frame codec request must be an object.".to_string())
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

fn optional_u8(value: Option<&JsonValue>) -> Option<u8> {
    match value? {
        JsonValue::Number(number) => number.as_u64().and_then(|value| u8::try_from(value).ok()),
        JsonValue::String(text) => text.trim().parse::<u8>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
    }
}

fn optional_u16(value: Option<&JsonValue>) -> Option<u16> {
    match value? {
        JsonValue::Number(number) => number.as_u64().and_then(|value| u16::try_from(value).ok()),
        JsonValue::String(text) => text.trim().parse::<u16>().ok(),
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) | JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => {
            None
        }
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

enum DecodeOutcome {
    Frame(DecodedFrame),
    Partial { needed_bytes: usize },
    ProtocolError { message: String },
}

struct DecodedFrame {
    fin: bool,
    opcode: u8,
    masked: bool,
    payload: Vec<u8>,
    consumed_bytes: usize,
}

struct CloseDetail {
    status_code: Option<u16>,
    reason: Option<String>,
}
