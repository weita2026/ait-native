use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATION_STAGE: &str = "rust_agent_slack_command_http_ingress";
const COMMAND_HTTP_INGRESS_CONTRACT: &str = "ait_agent_core.event_loop.SlackCommandHttpIngress.v1";
const DEFAULT_COMMAND_PATH: &str = "/command";
const SLACK_SIGNATURE_VERSION: &str = "v0";
const DEFAULT_TIMESTAMP_TOLERANCE_SECONDS: i64 = 60 * 5;

type HmacSha256 = Hmac<Sha256>;

pub trait SlackCommandHttpIngressPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackCommandHttpIngressPlanner;

impl SlackCommandHttpIngressPlanner for DefaultSlackCommandHttpIngressPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_command_http_ingress_json(request)
    }
}

pub fn agent_slack_command_http_ingress_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_slack_command_http_ingress_planner(&DefaultSlackCommandHttpIngressPlanner, request)
}

pub fn plan_with_slack_command_http_ingress_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: SlackCommandHttpIngressPlanner + ?Sized,
{
    planner.plan_json(request)
}

fn plan_command_http_ingress_json(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let request_path =
        clean_text(object.get("request_path")).or_else(|| clean_text(object.get("path")));
    let command_path = clean_text(object.get("command_path"))
        .map(|value| normalize_command_path(&value))
        .unwrap_or_else(|| DEFAULT_COMMAND_PATH.to_string());

    if let Some(path) = request_path.as_deref() {
        if path != command_path {
            return Ok(base_payload(
                "not_found",
                json!({
                    "ok": false,
                    "accepted": false,
                    "http_status": 404,
                    "write_json_response": false,
                    "response": JsonValue::Null,
                    "error_kind": "not_found",
                    "error": "Slack command endpoint not found.",
                    "command_path": command_path,
                    "request_path": path,
                    "should_handle_command": false,
                    "should_parse_payload": false,
                    "signature_verified": false,
                }),
            ));
        }
    }

    let signing_secret = clean_text(object.get("signing_secret")).unwrap_or_default();
    if signing_secret.is_empty() {
        return Ok(error_payload(
            "missing_signing_secret",
            400,
            "config_error",
            "Missing Slack signing secret for command payload verification.",
        ));
    }

    let raw_payload = required_string(object.get("raw_payload"), "raw_payload")?;
    let signature = clean_text(object.get("signature"));
    let signature_timestamp = clean_text(object.get("signature_timestamp"))
        .or_else(|| clean_text(object.get("timestamp")));
    let now = i64_field(object.get("now_unix_seconds")).unwrap_or_else(current_unix_seconds);
    let tolerance = i64_field(object.get("timestamp_tolerance_seconds"))
        .filter(|value| *value >= 0)
        .unwrap_or(DEFAULT_TIMESTAMP_TOLERANCE_SECONDS);

    if let Err(error) = verify_slack_signature(
        &raw_payload,
        signature.as_deref(),
        signature_timestamp.as_deref(),
        &signing_secret,
        now,
        tolerance,
    ) {
        return Ok(error_payload(
            error.state,
            401,
            "invalid_signature",
            error.message,
        ));
    }

    let parsed_payload = match parse_command_payload(&raw_payload) {
        Ok(payload) => payload,
        Err(error) => {
            return Ok(error_payload(
                error.state,
                400,
                "invalid_payload",
                error.message,
            ));
        }
    };

    Ok(base_payload(
        "command_payload_ready",
        json!({
            "ok": true,
            "accepted": true,
            "http_status": 200,
            "write_json_response": true,
            "response": JsonValue::Null,
            "error_kind": JsonValue::Null,
            "error": JsonValue::Null,
            "command_path": command_path,
            "request_path": request_path,
            "should_handle_command": true,
            "should_parse_payload": true,
            "signature_verified": true,
            "command_payload": parsed_payload,
            "next_ingress_request": {
                "stage": "command",
                "payload": parsed_payload,
            },
        }),
    ))
}

fn verify_slack_signature(
    raw_payload: &str,
    signature: Option<&str>,
    timestamp: Option<&str>,
    signing_secret: &str,
    now_unix_seconds: i64,
    tolerance_seconds: i64,
) -> Result<(), SlackSignatureError> {
    let Some(normalized_signature) = signature.and_then(clean_text_str) else {
        return Err(SlackSignatureError::new(
            "missing_signature",
            "Missing Slack signature header.",
        ));
    };
    let Some(normalized_timestamp) = timestamp.and_then(clean_text_str) else {
        return Err(SlackSignatureError::new(
            "missing_timestamp",
            "Missing Slack timestamp header.",
        ));
    };
    let timestamp_value = normalized_timestamp.parse::<i64>().map_err(|_| {
        SlackSignatureError::new("invalid_timestamp", "Invalid Slack timestamp header.")
    })?;
    if (now_unix_seconds - timestamp_value).abs() > tolerance_seconds {
        return Err(SlackSignatureError::new(
            "timestamp_outside_tolerance",
            "Slack request timestamp is outside the allowed tolerance.",
        ));
    }

    let expected = build_slack_signature(raw_payload, signing_secret, &normalized_timestamp)?;
    if !constant_time_eq(expected.as_bytes(), normalized_signature.as_bytes()) {
        return Err(SlackSignatureError::new(
            "invalid_signature",
            "Invalid Slack request signature.",
        ));
    }
    Ok(())
}

fn build_slack_signature(
    raw_payload: &str,
    signing_secret: &str,
    timestamp: &str,
) -> Result<String, SlackSignatureError> {
    let base_string = format!("{SLACK_SIGNATURE_VERSION}:{timestamp}:{raw_payload}");
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes()).map_err(|_| {
        SlackSignatureError::new("invalid_signing_secret", "Invalid Slack signing secret.")
    })?;
    mac.update(base_string.as_bytes());
    let digest = mac.finalize().into_bytes();
    Ok(format!("{SLACK_SIGNATURE_VERSION}={}", lower_hex(&digest)))
}

fn parse_command_payload(raw_payload: &str) -> Result<JsonValue, CommandPayloadError> {
    if raw_payload.trim().is_empty() {
        return Err(CommandPayloadError::new(
            "empty_payload",
            "No Slack command payload provided.",
        ));
    }
    let mut output = Map::new();
    for pair in raw_payload.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (key_raw, value_raw) = match pair.split_once('=') {
            Some((key, value)) => (key, value),
            None => (pair, ""),
        };
        let key = form_decode_component(key_raw);
        let value = form_decode_component(value_raw);
        output.insert(key, JsonValue::String(value));
    }
    if output.is_empty() {
        return Err(CommandPayloadError::new(
            "invalid_form",
            "Slack command payload must be form-encoded.",
        ));
    }
    Ok(JsonValue::Object(output))
}

fn form_decode_component(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);
                if let (Some(high), Some(low)) = (high, low) {
                    output.push((high << 4) | low);
                    index += 3;
                } else {
                    output.push(bytes[index]);
                    index += 1;
                }
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn base_payload(state: &str, mut payload: JsonValue) -> JsonValue {
    let object = payload
        .as_object_mut()
        .expect("base payload must be backed by an object");
    object.insert(
        "stage".to_string(),
        JsonValue::String("http_command_request".to_string()),
    );
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "slack_command_http_ingress_contract".to_string(),
        JsonValue::String(COMMAND_HTTP_INGRESS_CONTRACT.to_string()),
    );
    object.insert(
        "transport".to_string(),
        JsonValue::String("slack".to_string()),
    );
    object.insert(
        "command_http_ingress_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "python_signature_verification_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_form_parsing_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    payload
}

fn error_payload(state: &str, status: u16, error_kind: &str, error: &str) -> JsonValue {
    base_payload(
        state,
        json!({
            "ok": false,
            "accepted": false,
            "http_status": status,
            "write_json_response": true,
            "response": {
                "ok": false,
                "error": error,
            },
            "error_kind": error_kind,
            "error": error,
            "should_handle_command": false,
            "should_parse_payload": false,
            "signature_verified": false,
            "command_payload": JsonValue::Null,
            "next_ingress_request": JsonValue::Null,
        }),
    )
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "Slack command HTTP ingress request must be an object.".to_string())
}

fn required_string(value: Option<&JsonValue>, field_name: &str) -> Result<String, String> {
    value
        .and_then(|item| item.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("Slack command HTTP ingress request requires `{field_name}`."))
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value.and_then(|item| {
        if let Some(text) = item.as_str() {
            clean_text_str(text)
        } else if item.is_null() {
            None
        } else {
            clean_text_str(&item.to_string())
        }
    })
}

fn clean_text_str(value: &str) -> Option<String> {
    let text = value.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

fn normalize_command_path(value: &str) -> String {
    let text = value.trim();
    if text.is_empty() {
        DEFAULT_COMMAND_PATH.to_string()
    } else if text.starts_with('/') {
        text.to_string()
    } else {
        format!("/{text}")
    }
}

fn i64_field(value: Option<&JsonValue>) -> Option<i64> {
    match value {
        Some(JsonValue::Number(number)) => number.as_i64(),
        Some(JsonValue::String(text)) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (left_byte, right_byte) in left.iter().zip(right.iter()) {
        diff |= left_byte ^ right_byte;
    }
    diff == 0
}

fn lower_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(b"0123456789abcdef"[(byte >> 4) as usize]));
        output.push(char::from(b"0123456789abcdef"[(byte & 0x0f) as usize]));
    }
    output
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
struct SlackSignatureError {
    state: &'static str,
    message: &'static str,
}

impl SlackSignatureError {
    fn new(state: &'static str, message: &'static str) -> Self {
        Self { state, message }
    }
}

#[derive(Debug, Clone, Copy)]
struct CommandPayloadError {
    state: &'static str,
    message: &'static str,
}

impl CommandPayloadError {
    fn new(state: &'static str, message: &'static str) -> Self {
        Self { state, message }
    }
}
