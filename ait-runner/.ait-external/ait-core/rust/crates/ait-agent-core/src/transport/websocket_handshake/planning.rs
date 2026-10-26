use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use sha1::{Digest, Sha1};
use std::collections::BTreeMap;

const MIGRATION_STAGE: &str = "rust_agent_transport_websocket_handshake_boundary";
const WEBSOCKET_HANDSHAKE_CONTRACT: &str = "ait_agent_core.transport.WebSocketHandshake.v1";
const WEBSOCKET_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub trait WebSocketHandshakePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultWebSocketHandshakePlanner;

impl WebSocketHandshakePlanner for DefaultWebSocketHandshakePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        plan_default(request)
    }
}

pub fn agent_transport_websocket_handshake_plan_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_websocket_handshake_planner(&DefaultWebSocketHandshakePlanner, request)
}

pub fn plan_with_websocket_handshake_planner<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: WebSocketHandshakePlanner,
{
    planner.plan_json(request)
}

fn plan_default(request: &JsonValue) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "upgrade_request".to_string());

    match stage.as_str() {
        "request" | "plan_request" | "upgrade_request" | "client_request" => {
            Ok(plan_upgrade_request(object))
        }
        "response" | "validate_response" | "upgrade_response" => Ok(plan_upgrade_response(object)),
        other => Err(format!("unsupported WebSocket handshake stage: {other}")),
    }
}

fn plan_upgrade_request(object: &Map<String, JsonValue>) -> JsonValue {
    let url = match clean_text(
        object
            .get("websocket_url")
            .or_else(|| object.get("url"))
            .or_else(|| object.get("gateway_url")),
    ) {
        Some(url) => url,
        None => {
            return configuration_error_payload(
                "upgrade_request",
                "WebSocket handshake request must include a websocket URL.",
            );
        }
    };
    let target = match parse_websocket_url(&url) {
        Ok(target) => target,
        Err(message) => return configuration_error_payload("upgrade_request", &message),
    };
    let sec_websocket_key = match sec_websocket_key(object) {
        Ok(key) => key,
        Err(message) => return configuration_error_payload("upgrade_request", &message),
    };
    let expected_accept = websocket_accept(&sec_websocket_key);
    let subprotocols = match subprotocols(object) {
        Ok(subprotocols) => subprotocols,
        Err(message) => return configuration_error_payload("upgrade_request", &message),
    };
    let additional_headers = match additional_headers(object) {
        Ok(headers) => headers,
        Err(message) => return configuration_error_payload("upgrade_request", &message),
    };

    let mut headers = vec![
        Header::new("Host", target.host_header.clone()),
        Header::new("Upgrade", "websocket"),
        Header::new("Connection", "Upgrade"),
        Header::new("Sec-WebSocket-Key", sec_websocket_key.clone()),
        Header::new("Sec-WebSocket-Version", "13"),
    ];
    if !subprotocols.is_empty() {
        headers.push(Header::new(
            "Sec-WebSocket-Protocol",
            subprotocols.join(", "),
        ));
    }
    headers.extend(additional_headers);

    let request_line = format!("GET {} HTTP/1.1", target.path_and_query);
    let request_text = http_request_text(&request_line, &headers);
    let request_bytes = request_text.as_bytes().to_vec();

    base_payload(
        "upgrade_request",
        "upgrade_request_planned",
        json!({
            "ok": true,
            "complete": true,
            "url": url,
            "scheme": target.scheme,
            "secure": target.secure,
            "host": target.host,
            "port": target.port,
            "explicit_port": target.explicit_port,
            "authority": target.authority,
            "host_header": target.host_header,
            "path": target.path,
            "query": target.query,
            "path_and_query": target.path_and_query,
            "request_line": request_line,
            "request_headers": headers_json(&headers),
            "request_text": request_text,
            "request_bytes": bytes_json(&request_bytes),
            "request_hex": bytes_hex(&request_bytes),
            "sec_websocket_key": sec_websocket_key,
            "expected_sec_websocket_accept": expected_accept,
            "subprotocols": subprotocols,
            "execute_connect": false,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "should_validate_upgrade_response": true,
            "should_register_websocket": false,
            "actions": [
                {
                    "kind": "write_websocket_upgrade_request",
                    "request_bytes": bytes_json(&request_bytes),
                    "request_hex": bytes_hex(&request_bytes),
                    "execute_write": false,
                },
                {
                    "kind": "await_websocket_upgrade_response",
                    "expected_status_code": 101,
                    "expected_sec_websocket_accept": expected_accept,
                }
            ],
        }),
    )
}

fn plan_upgrade_response(object: &Map<String, JsonValue>) -> JsonValue {
    let sec_websocket_key = match sec_websocket_key(object) {
        Ok(key) => key,
        Err(message) => return configuration_error_payload("validate_response", &message),
    };
    let expected_accept = websocket_accept(&sec_websocket_key);
    let response_bytes = match response_bytes(object) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return configuration_error_payload(
                "validate_response",
                "WebSocket handshake response validation must include response bytes or text.",
            );
        }
        Err(message) => return configuration_error_payload("validate_response", &message),
    };
    let response_text = match String::from_utf8(response_bytes.clone()) {
        Ok(text) => text,
        Err(_) => {
            return rejected_response_payload(
                &sec_websocket_key,
                &expected_accept,
                None,
                vec!["WebSocket handshake response must be valid UTF-8 HTTP headers.".to_string()],
                object,
            );
        }
    };
    let parsed = match parse_response(&response_text) {
        Ok(parsed) => parsed,
        Err(message) => {
            return rejected_response_payload(
                &sec_websocket_key,
                &expected_accept,
                None,
                vec![message],
                object,
            );
        }
    };

    let mut errors = Vec::new();
    if parsed.status_code != 101 {
        errors.push(format!(
            "WebSocket handshake response status must be 101, got {}.",
            parsed.status_code
        ));
    }
    if !header_contains_token(&parsed.headers, "upgrade", "websocket") {
        errors.push("WebSocket handshake response must include `Upgrade: websocket`.".to_string());
    }
    if !header_contains_token(&parsed.headers, "connection", "upgrade") {
        errors.push(
            "WebSocket handshake response must include `Connection` token `upgrade`.".to_string(),
        );
    }
    let actual_accept = header_values(&parsed.headers, "sec-websocket-accept")
        .first()
        .map(|value| value.trim().to_string());
    match actual_accept.as_deref() {
        Some(actual) if actual == expected_accept => {}
        Some(actual) => errors.push(format!(
            "WebSocket handshake response `Sec-WebSocket-Accept` mismatch: expected `{expected_accept}`, got `{actual}`."
        )),
        None => errors.push(
            "WebSocket handshake response must include `Sec-WebSocket-Accept`.".to_string(),
        ),
    }

    if !errors.is_empty() {
        return rejected_response_payload(
            &sec_websocket_key,
            &expected_accept,
            Some(&parsed),
            errors,
            object,
        );
    }

    let registration_action = registration_action(object);
    let should_register = registration_action.is_some();
    let mut actions = vec![json!({
        "kind": "complete_websocket_upgrade",
        "status_code": parsed.status_code,
        "expected_sec_websocket_accept": expected_accept,
        "actual_sec_websocket_accept": actual_accept,
        "execute_registration": false,
    })];
    if let Some(action) = registration_action.clone() {
        actions.push(action);
    }

    base_payload(
        "validate_response",
        "upgrade_accepted",
        json!({
            "ok": true,
            "complete": true,
            "status_code": parsed.status_code,
            "reason": parsed.reason,
            "response_headers": response_headers_json(&parsed.headers),
            "response_text": response_text,
            "response_bytes": bytes_json(&response_bytes),
            "response_hex": bytes_hex(&response_bytes),
            "sec_websocket_key": sec_websocket_key,
            "expected_sec_websocket_accept": expected_accept,
            "actual_sec_websocket_accept": actual_accept,
            "upgrade_valid": true,
            "registration_ready": true,
            "should_register_websocket": should_register,
            "registration_action": registration_action.unwrap_or(JsonValue::Null),
            "should_close_websocket": false,
            "execute_connect": false,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "execute_registration": false,
            "actions": actions,
        }),
    )
}

fn rejected_response_payload(
    sec_websocket_key: &str,
    expected_accept: &str,
    parsed: Option<&ParsedResponse>,
    errors: Vec<String>,
    object: &Map<String, JsonValue>,
) -> JsonValue {
    let error = errors
        .first()
        .cloned()
        .unwrap_or_else(|| "WebSocket handshake response was rejected.".to_string());
    let status_code = parsed
        .map(|response| JsonValue::from(response.status_code))
        .unwrap_or(JsonValue::Null);
    let reason = parsed
        .map(|response| JsonValue::from(response.reason.clone()))
        .unwrap_or(JsonValue::Null);
    let response_headers = parsed
        .map(|response| response_headers_json(&response.headers))
        .unwrap_or_else(|| json!([]));
    let actual_accept = parsed
        .and_then(|response| {
            header_values(&response.headers, "sec-websocket-accept")
                .first()
                .map(|value| value.trim().to_string())
        })
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);

    let close_reason = clean_text(object.get("close_reason")).unwrap_or_else(|| error.clone());

    base_payload(
        "validate_response",
        "upgrade_rejected",
        json!({
            "ok": false,
            "complete": false,
            "status_code": status_code,
            "reason": reason,
            "response_headers": response_headers,
            "sec_websocket_key": sec_websocket_key,
            "expected_sec_websocket_accept": expected_accept,
            "actual_sec_websocket_accept": actual_accept,
            "upgrade_valid": false,
            "validation_errors": errors,
            "error": error,
            "registration_ready": false,
            "should_register_websocket": false,
            "should_close_websocket": true,
            "execute_connect": false,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "execute_registration": false,
            "actions": [
                {
                    "kind": "diagnose_websocket_handshake_rejection",
                    "error": error,
                },
                {
                    "kind": "close_websocket",
                    "reason": close_reason,
                }
            ],
        }),
    )
}

fn registration_action(object: &Map<String, JsonValue>) -> Option<JsonValue> {
    let fd = optional_i64(
        object
            .get("websocket_fd")
            .or_else(|| object.get("fd"))
            .or_else(|| object.get("socket_fd")),
    )?;
    let token = clean_text(
        object
            .get("event_loop_token")
            .or_else(|| object.get("token"))
            .or_else(|| object.get("registration_token")),
    )?;
    let worker_key = clean_text(object.get("worker_key"))
        .or_else(|| clean_text(object.get("worker_id")))
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);
    Some(json!({
        "kind": "register_websocket_after_upgrade",
        "websocket_fd": fd,
        "event_loop_token": token,
        "worker_key": worker_key,
        "interest": "readable",
        "execute_registration": false,
    }))
}

fn parse_websocket_url(raw: &str) -> Result<WebSocketTarget, String> {
    let raw = raw.trim();
    if raw.contains('#') {
        return Err("WebSocket URL fragments are not supported.".to_string());
    }
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| "WebSocket URL must include `ws://` or `wss://` scheme.".to_string())?;
    let scheme = scheme.to_ascii_lowercase();
    let secure = match scheme.as_str() {
        "ws" => false,
        "wss" => true,
        _ => return Err(format!("unsupported WebSocket URL scheme `{scheme}`.")),
    };
    let default_port = if secure { 443 } else { 80 };
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let suffix = &rest[authority_end..];
    if authority.trim().is_empty() {
        return Err("WebSocket URL must include a host.".to_string());
    }
    let (host, port, explicit_port) = parse_authority(authority, default_port)?;
    let path_and_query = if suffix.is_empty() {
        "/".to_string()
    } else if suffix.starts_with('?') {
        format!("/{suffix}")
    } else {
        suffix.to_string()
    };
    if path_and_query.is_empty() {
        return Err("WebSocket URL path is invalid.".to_string());
    }
    let (path, query) = match path_and_query.split_once('?') {
        Some((path, query)) => (path.to_string(), Some(query.to_string())),
        None => (path_and_query.clone(), None),
    };
    if path.is_empty() || !path.starts_with('/') {
        return Err("WebSocket URL path must start with `/`.".to_string());
    }
    let host_header = if explicit_port || port != default_port {
        format!("{host}:{port}")
    } else {
        host.clone()
    };
    let authority = if explicit_port {
        format!("{host}:{port}")
    } else {
        host.clone()
    };

    Ok(WebSocketTarget {
        scheme,
        secure,
        host,
        port,
        explicit_port,
        authority,
        host_header,
        path,
        query,
        path_and_query,
    })
}

fn parse_authority(authority: &str, default_port: u16) -> Result<(String, u16, bool), String> {
    if authority.contains('@') {
        return Err("WebSocket URL userinfo is not supported.".to_string());
    }
    if authority.starts_with('[') {
        let end = authority
            .find(']')
            .ok_or_else(|| "WebSocket IPv6 host must be bracketed.".to_string())?;
        let host = authority[..=end].to_string();
        let remainder = &authority[end + 1..];
        if remainder.is_empty() {
            return Ok((host, default_port, false));
        }
        let Some(port) = remainder.strip_prefix(':') else {
            return Err("WebSocket URL authority is invalid.".to_string());
        };
        return Ok((host, parse_port(port)?, true));
    }

    let colon_count = authority.chars().filter(|ch| *ch == ':').count();
    if colon_count > 1 {
        return Err("WebSocket IPv6 hosts must be bracketed.".to_string());
    }
    if let Some((host, port)) = authority.rsplit_once(':') {
        if host.trim().is_empty() {
            return Err("WebSocket URL must include a host.".to_string());
        }
        if port.chars().all(|ch| ch.is_ascii_digit()) {
            return Ok((host.to_string(), parse_port(port)?, true));
        }
        return Err("WebSocket URL port must be numeric.".to_string());
    }
    Ok((authority.to_string(), default_port, false))
}

fn parse_port(raw: &str) -> Result<u16, String> {
    if raw.is_empty() {
        return Err("WebSocket URL port must not be empty.".to_string());
    }
    let port = raw
        .parse::<u16>()
        .map_err(|_| "WebSocket URL port must be between 1 and 65535.".to_string())?;
    if port == 0 {
        return Err("WebSocket URL port must be between 1 and 65535.".to_string());
    }
    Ok(port)
}

fn sec_websocket_key(object: &Map<String, JsonValue>) -> Result<String, String> {
    let key = clean_text(
        object
            .get("sec_websocket_key")
            .or_else(|| object.get("websocket_key"))
            .or_else(|| object.get("key")),
    )
    .ok_or_else(|| "WebSocket handshake requires `sec_websocket_key`.".to_string())?;
    let decoded = BASE64_STANDARD
        .decode(key.as_bytes())
        .map_err(|_| "WebSocket `sec_websocket_key` must be valid base64.".to_string())?;
    if decoded.len() != 16 {
        return Err("WebSocket `sec_websocket_key` must decode to 16 bytes.".to_string());
    }
    Ok(key)
}

fn websocket_accept(sec_websocket_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(sec_websocket_key.as_bytes());
    hasher.update(WEBSOCKET_GUID.as_bytes());
    BASE64_STANDARD.encode(hasher.finalize())
}

fn subprotocols(object: &Map<String, JsonValue>) -> Result<Vec<String>, String> {
    let Some(value) = object
        .get("subprotocols")
        .or_else(|| object.get("protocols"))
        .or_else(|| object.get("sec_websocket_protocols"))
    else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    if let Some(text) = value.as_str() {
        return Ok(text
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect());
    }
    let Some(items) = value.as_array() else {
        return Err("WebSocket subprotocols must be a string or string array.".to_string());
    };
    items
        .iter()
        .map(|item| {
            clean_text(Some(item))
                .filter(|text| !text.is_empty())
                .ok_or_else(|| "WebSocket subprotocols must be non-empty strings.".to_string())
        })
        .collect()
}

fn additional_headers(object: &Map<String, JsonValue>) -> Result<Vec<Header>, String> {
    let Some(value) = object
        .get("additional_headers")
        .or_else(|| object.get("extra_headers"))
    else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let mut headers = Vec::new();
    if let Some(map) = value.as_object() {
        for (name, value) in map {
            headers.push(validated_header(name, value)?);
        }
        return Ok(headers);
    }
    let Some(items) = value.as_array() else {
        return Err("WebSocket additional headers must be an object or array.".to_string());
    };
    for item in items {
        let Some(item) = item.as_object() else {
            return Err("WebSocket additional header entries must be objects.".to_string());
        };
        let name = clean_text(item.get("name").or_else(|| item.get("header")))
            .ok_or_else(|| "WebSocket additional header entry must include `name`.".to_string())?;
        let value = item
            .get("value")
            .ok_or_else(|| "WebSocket additional header entry must include `value`.".to_string())?;
        headers.push(validated_header(&name, value)?);
    }
    Ok(headers)
}

fn validated_header(name: &str, value: &JsonValue) -> Result<Header, String> {
    let name = validate_header_name(name)?;
    let value = validate_header_value(value)?;
    if is_core_request_header(&name) {
        return Err(format!(
            "WebSocket additional header `{name}` must not override core upgrade headers."
        ));
    }
    Ok(Header::new(name, value))
}

fn validate_header_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() || name.contains(':') || name.contains('\r') || name.contains('\n') {
        return Err("WebSocket header names must be non-empty tokens.".to_string());
    }
    Ok(name.to_string())
}

fn validate_header_value(value: &JsonValue) -> Result<String, String> {
    let value = clean_text(Some(value))
        .ok_or_else(|| "WebSocket header values must be scalar values.".to_string())?;
    if value.contains('\r') || value.contains('\n') {
        return Err("WebSocket header values must not contain newlines.".to_string());
    }
    Ok(value)
}

fn is_core_request_header(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "host"
            | "upgrade"
            | "connection"
            | "sec-websocket-key"
            | "sec-websocket-version"
            | "sec-websocket-protocol"
            | "sec-websocket-accept"
    )
}

fn http_request_text(request_line: &str, headers: &[Header]) -> String {
    let mut text = format!("{request_line}\r\n");
    for header in headers {
        text.push_str(&header.name);
        text.push_str(": ");
        text.push_str(&header.value);
        text.push_str("\r\n");
    }
    text.push_str("\r\n");
    text
}

fn response_bytes(object: &Map<String, JsonValue>) -> Result<Option<Vec<u8>>, String> {
    if let Some(value) = object.get("response_text") {
        let Some(text) = value.as_str() else {
            return Err("WebSocket response_text must be a string.".to_string());
        };
        return Ok(Some(text.as_bytes().to_vec()));
    }
    if let Some(value) = object.get("response_bytes") {
        return json_bytes(value)
            .map(Some)
            .ok_or_else(|| "WebSocket response_bytes must be a byte array.".to_string());
    }
    if let Some(value) = object.get("response_hex") {
        let Some(raw) = value.as_str() else {
            return Err("WebSocket response_hex must be a hex string.".to_string());
        };
        return parse_hex_bytes(raw)
            .map(Some)
            .ok_or_else(|| "WebSocket response_hex must be a valid hex string.".to_string());
    }
    Ok(None)
}

fn parse_response(response_text: &str) -> Result<ParsedResponse, String> {
    let normalized = response_text.replace("\r\n", "\n");
    let header_text = normalized
        .split("\n\n")
        .next()
        .ok_or_else(|| "WebSocket handshake response is empty.".to_string())?;
    let mut lines = header_text.lines();
    let status_line = lines
        .next()
        .ok_or_else(|| "WebSocket handshake response is empty.".to_string())?;
    let mut status_parts = status_line.split_whitespace();
    let version = status_parts
        .next()
        .ok_or_else(|| "WebSocket handshake response status line is malformed.".to_string())?;
    if !version.starts_with("HTTP/") {
        return Err("WebSocket handshake response status line must start with HTTP/.".to_string());
    }
    let status_code = status_parts
        .next()
        .ok_or_else(|| "WebSocket handshake response status line is missing a code.".to_string())?
        .parse::<u16>()
        .map_err(|_| "WebSocket handshake response status code is invalid.".to_string())?;
    let reason = status_parts.collect::<Vec<_>>().join(" ");
    let mut headers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in lines {
        if line.trim().is_empty() {
            break;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| "WebSocket handshake response header is malformed.".to_string())?;
        let name = validate_header_name(name)?.to_ascii_lowercase();
        let value = value.trim().to_string();
        headers.entry(name).or_default().push(value);
    }
    Ok(ParsedResponse {
        status_code,
        reason,
        headers,
    })
}

fn header_contains_token(
    headers: &BTreeMap<String, Vec<String>>,
    name: &str,
    expected: &str,
) -> bool {
    header_values(headers, name).iter().any(|value| {
        value
            .split(',')
            .map(str::trim)
            .any(|token| token.eq_ignore_ascii_case(expected))
    })
}

fn header_values(headers: &BTreeMap<String, Vec<String>>, name: &str) -> Vec<String> {
    headers
        .get(&name.to_ascii_lowercase())
        .cloned()
        .unwrap_or_default()
}

fn configuration_error_payload(stage: &str, message: &str) -> JsonValue {
    base_payload(
        stage,
        "configuration_error",
        json!({
            "ok": false,
            "complete": false,
            "error": message,
            "upgrade_valid": false,
            "registration_ready": false,
            "should_register_websocket": false,
            "should_close_websocket": true,
            "execute_connect": false,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "execute_registration": false,
            "actions": [
                {
                    "kind": "diagnose_websocket_handshake_configuration_error",
                    "error": message,
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
        "websocket_handshake_contract".to_string(),
        JsonValue::String(WEBSOCKET_HANDSHAKE_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "websocket_handshake_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_websocket_handshake_allowed".to_string(),
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

fn headers_json(headers: &[Header]) -> JsonValue {
    JsonValue::Array(
        headers
            .iter()
            .map(|header| {
                json!({
                    "name": header.name,
                    "value": header.value,
                })
            })
            .collect(),
    )
}

fn response_headers_json(headers: &BTreeMap<String, Vec<String>>) -> JsonValue {
    let mut entries = Vec::new();
    for (name, values) in headers {
        for value in values {
            entries.push(json!({
                "name": name,
                "value": value,
            }));
        }
    }
    JsonValue::Array(entries)
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

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "WebSocket handshake request must be an object.".to_string())
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

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

#[derive(Debug, Clone)]
struct Header {
    name: String,
    value: String,
}

impl Header {
    fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

#[derive(Debug, Clone)]
struct WebSocketTarget {
    scheme: String,
    secure: bool,
    host: String,
    port: u16,
    explicit_port: bool,
    authority: String,
    host_header: String,
    path: String,
    query: Option<String>,
    path_and_query: String,
}

#[derive(Debug, Clone)]
struct ParsedResponse {
    status_code: u16,
    reason: String,
    headers: BTreeMap<String, Vec<String>>,
}
