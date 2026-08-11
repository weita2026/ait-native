use crate::platform::{
    close_native_socket, native_socket_from_u64, native_socket_take_error,
    tcp_stream_into_native_socket, NativeSocket,
};
use crate::transport::websocket_handshake::agent_transport_websocket_handshake_plan_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use socket2::{Domain, Protocol, SockAddr, Socket, Type};
use std::io;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::str::FromStr;

const MIGRATION_STAGE: &str = "rust_agent_transport_websocket_tcp_connect_boundary";
const WEBSOCKET_CONNECT_CONTRACT: &str = "ait_agent_core.transport.WebSocketTcpConnect.v1";

pub trait WebSocketConnectStartExecutor {
    fn start_tcp_connect(&self, address: SocketAddr) -> TcpConnectOutcome;
}

pub trait WebSocketConnectFinishExecutor {
    fn finish_tcp_connect(&self, fd: NativeSocket) -> TcpConnectOutcome;
}

pub trait WebSocketConnectExecutor:
    WebSocketConnectStartExecutor + WebSocketConnectFinishExecutor
{
}

impl<E> WebSocketConnectExecutor for E where
    E: WebSocketConnectStartExecutor + WebSocketConnectFinishExecutor + ?Sized
{
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultWebSocketConnectExecutor;

impl WebSocketConnectStartExecutor for DefaultWebSocketConnectExecutor {
    fn start_tcp_connect(&self, address: SocketAddr) -> TcpConnectOutcome {
        start_nonblocking_tcp_connect(address)
    }
}

impl WebSocketConnectFinishExecutor for DefaultWebSocketConnectExecutor {
    fn finish_tcp_connect(&self, fd: NativeSocket) -> TcpConnectOutcome {
        finish_nonblocking_tcp_connect(fd)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpConnectStatus {
    Connected,
    InProgress,
    Failed,
}

impl TcpConnectStatus {
    fn state(self, secure: bool) -> &'static str {
        match self {
            Self::Connected if secure => "tcp_connected_tls_required",
            Self::Connected => "tcp_connected",
            Self::InProgress => "tcp_connecting",
            Self::Failed => "tcp_connect_error",
        }
    }

    fn connected(self) -> bool {
        matches!(self, Self::Connected)
    }

    fn in_progress(self) -> bool {
        matches!(self, Self::InProgress)
    }
}

#[derive(Debug, Clone)]
pub struct TcpConnectOutcome {
    pub status: TcpConnectStatus,
    pub fd: Option<NativeSocket>,
    pub errno: Option<i32>,
    pub error: Option<String>,
}

impl TcpConnectOutcome {
    pub fn connected(fd: NativeSocket) -> Self {
        Self {
            status: TcpConnectStatus::Connected,
            fd: Some(fd),
            errno: None,
            error: None,
        }
    }

    pub fn in_progress(fd: NativeSocket, errno: i32, error: impl Into<String>) -> Self {
        Self {
            status: TcpConnectStatus::InProgress,
            fd: Some(fd),
            errno: Some(errno),
            error: Some(error.into()),
        }
    }

    pub fn failed(fd: Option<NativeSocket>, errno: Option<i32>, error: impl Into<String>) -> Self {
        Self {
            status: TcpConnectStatus::Failed,
            fd,
            errno,
            error: Some(error.into()),
        }
    }
}

pub fn agent_transport_websocket_connect_execute_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_websocket_connect_executor(&DefaultWebSocketConnectExecutor, request)
}

pub fn plan_with_websocket_connect_executor<E>(
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: WebSocketConnectExecutor + ?Sized,
{
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "plan".to_string());

    match stage.as_str() {
        "plan" | "connection_plan" | "connect_plan" => Ok(plan_connection(object)),
        "start" | "start_tcp_connect" | "tcp_connect" | "open_tcp_fd" => {
            Ok(start_tcp_connect(executor, object))
        }
        "finish" | "finish_tcp_connect" | "check_tcp_connect" | "connect_ready" => {
            Ok(finish_tcp_connect(executor, object))
        }
        other => Err(format!("unsupported WebSocket TCP connect stage: {other}")),
    }
}

fn plan_connection(object: &Map<String, JsonValue>) -> JsonValue {
    let target = match required_target(object, "plan") {
        Ok(target) => target,
        Err(message) => return configuration_error_payload("plan", &message),
    };
    let mut actions = vec![
        json!({
            "kind": "resolve_websocket_host",
            "host": target.host,
            "port": target.port,
            "secure": target.secure,
            "execute_dns": false,
            "blocking_dns_allowed": false,
            "python_dns_allowed": false,
        }),
        json!({
            "kind": "open_websocket_tcp_connect",
            "host": target.host,
            "port": target.port,
            "secure": target.secure,
            "requires_resolved_address": true,
            "execute_connect": false,
        }),
    ];
    if target.secure {
        actions.push(tls_action(None, &target));
    } else {
        actions.push(json!({
            "kind": "plan_websocket_upgrade_request",
            "url": target.url,
            "execute_upgrade_write": false,
        }));
    }

    base_payload(
        "plan",
        "connect_planned",
        json!({
            "ok": true,
            "executed": false,
            "target": target_json(&target),
            "url": target.url,
            "scheme": target.scheme,
            "secure": target.secure,
            "host": target.host,
            "port": target.port,
            "host_header": target.host_header,
            "path_and_query": target.path_and_query,
            "tls_required": target.secure,
            "requires_resolved_address": true,
            "execute_dns": false,
            "execute_connect": false,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "actions": actions,
        }),
    )
}

fn start_tcp_connect<E>(executor: &E, object: &Map<String, JsonValue>) -> JsonValue
where
    E: WebSocketConnectStartExecutor + ?Sized,
{
    let target = match optional_target(object) {
        Ok(target) => target,
        Err(message) => return configuration_error_payload("start_tcp_connect", &message),
    };
    let address = match socket_address_from_request(object) {
        Ok(address) => address,
        Err(message) => return configuration_error_payload("start_tcp_connect", &message),
    };
    let outcome = executor.start_tcp_connect(address);
    connect_outcome_payload(
        "start_tcp_connect",
        object,
        target.as_ref(),
        Some(address),
        outcome,
    )
}

fn finish_tcp_connect<E>(executor: &E, object: &Map<String, JsonValue>) -> JsonValue
where
    E: WebSocketConnectFinishExecutor + ?Sized,
{
    let fd = match required_fd(object) {
        Ok(fd) => fd,
        Err(message) => return configuration_error_payload("finish_tcp_connect", &message),
    };
    let target = match optional_target(object) {
        Ok(target) => target,
        Err(message) => return configuration_error_payload("finish_tcp_connect", &message),
    };
    let address = socket_address_from_request(object).ok();
    let outcome = executor.finish_tcp_connect(fd);
    if outcome.status == TcpConnectStatus::Failed
        && optional_bool(object.get("close_on_error")).unwrap_or(true)
    {
        let _ = close_native_socket(fd);
    }
    connect_outcome_payload(
        "finish_tcp_connect",
        object,
        target.as_ref(),
        address,
        outcome,
    )
}

fn connect_outcome_payload(
    stage: &str,
    object: &Map<String, JsonValue>,
    target: Option<&WebSocketTarget>,
    address: Option<SocketAddr>,
    outcome: TcpConnectOutcome,
) -> JsonValue {
    let secure = target
        .map(|target| target.secure)
        .or_else(|| optional_bool(object.get("secure")))
        .unwrap_or(false);
    let state = outcome.status.state(secure);
    let fd_json = outcome.fd.map(JsonValue::from).unwrap_or(JsonValue::Null);
    let mut actions = Vec::new();

    if outcome.status.in_progress() {
        if let Some(action) = writable_registration_action(object, outcome.fd) {
            actions.push(action);
        } else {
            actions.push(json!({
                "kind": "await_websocket_connect_writable_registration",
                "websocket_fd": fd_json,
                "reason": "missing_event_loop_token",
            }));
        }
    } else if outcome.status.connected() {
        actions.extend(connected_next_step_actions(object, target, outcome.fd));
    } else {
        actions.push(json!({
            "kind": "diagnose_websocket_tcp_connect_error",
            "websocket_fd": fd_json,
            "socket_address": address.map(socket_address_json).unwrap_or(JsonValue::Null),
            "errno": outcome.errno.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "error": outcome.error.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        }));
    }

    base_payload(
        stage,
        state,
        json!({
            "ok": outcome.status != TcpConnectStatus::Failed,
            "executed": true,
            "connected": outcome.status.connected(),
            "connect_in_progress": outcome.status.in_progress(),
            "connect_failed": outcome.status == TcpConnectStatus::Failed,
            "socket_address": address.map(socket_address_json).unwrap_or(JsonValue::Null),
            "target": target.map(target_json).unwrap_or(JsonValue::Null),
            "url": target.map(|target| JsonValue::from(target.url.clone())).unwrap_or(JsonValue::Null),
            "scheme": target.map(|target| JsonValue::from(target.scheme.clone())).unwrap_or(JsonValue::Null),
            "secure": secure,
            "tls_required": secure,
            "websocket_fd": fd_json,
            "errno": outcome.errno.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "error": outcome.error.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "should_register_writable": outcome.status.in_progress()
                && event_loop_token(object).is_some()
                && outcome.fd.is_some(),
            "should_start_tls": outcome.status.connected() && secure,
            "should_write_upgrade_request": outcome.status.connected() && !secure,
            "execute_dns": false,
            "execute_connect": true,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "actions": actions,
        }),
    )
}

fn connected_next_step_actions(
    object: &Map<String, JsonValue>,
    target: Option<&WebSocketTarget>,
    fd: Option<NativeSocket>,
) -> Vec<JsonValue> {
    let Some(target) = target else {
        return vec![json!({
            "kind": "plan_websocket_upgrade_or_tls_next_step",
            "websocket_fd": fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "reason": "missing_websocket_url",
            "execute_tls": false,
            "execute_upgrade_write": false,
        })];
    };
    if target.secure {
        return vec![tls_action(fd, target)];
    }
    match upgrade_request_action(object, target, fd) {
        Some(action) => action,
        None => vec![json!({
            "kind": "plan_websocket_upgrade_request",
            "websocket_fd": fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "url": target.url,
            "execute_upgrade_write": false,
            "reason": "missing_sec_websocket_key",
        })],
    }
}

fn upgrade_request_action(
    object: &Map<String, JsonValue>,
    target: &WebSocketTarget,
    fd: Option<NativeSocket>,
) -> Option<Vec<JsonValue>> {
    let key = clean_text(
        object
            .get("sec_websocket_key")
            .or_else(|| object.get("websocket_key"))
            .or_else(|| object.get("key")),
    )?;
    let mut request = json!({
        "stage": "upgrade_request",
        "url": target.url,
        "sec_websocket_key": key,
    });
    if let Some(value) = object
        .get("subprotocols")
        .or_else(|| object.get("protocols"))
        .cloned()
    {
        request["subprotocols"] = value;
    }
    if let Some(value) = object
        .get("additional_headers")
        .or_else(|| object.get("extra_headers"))
        .cloned()
    {
        request["additional_headers"] = value;
    }
    let handshake = match agent_transport_websocket_handshake_plan_json(&request) {
        Ok(handshake) if handshake["ok"] == true => handshake,
        Ok(handshake) => {
            return Some(vec![json!({
                "kind": "diagnose_websocket_upgrade_request_configuration_error",
                "websocket_fd": fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "handshake_result": handshake,
            })]);
        }
        Err(error) => {
            return Some(vec![json!({
                "kind": "diagnose_websocket_upgrade_request_configuration_error",
                "websocket_fd": fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
                "error": error,
            })]);
        }
    };
    Some(vec![
        json!({
            "kind": "write_websocket_upgrade_request",
            "websocket_fd": fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "url": target.url,
            "request_bytes": handshake["request_bytes"].clone(),
            "request_hex": handshake["request_hex"].clone(),
            "expected_sec_websocket_accept": handshake["expected_sec_websocket_accept"].clone(),
            "execute_write": false,
        }),
        json!({
            "kind": "await_websocket_upgrade_response",
            "websocket_fd": fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "expected_status_code": 101,
            "expected_sec_websocket_accept": handshake["expected_sec_websocket_accept"].clone(),
        }),
    ])
}

fn tls_action(fd: Option<NativeSocket>, target: &WebSocketTarget) -> JsonValue {
    json!({
        "kind": "start_websocket_tls_handshake",
        "websocket_fd": fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "host": target.host,
        "port": target.port,
        "server_name": target.host.trim_matches(&['[', ']'][..]),
        "execute_tls": false,
        "python_tls_allowed": false,
        "reason": "rust_tls_stream_boundary_required",
    })
}

fn writable_registration_action(
    object: &Map<String, JsonValue>,
    fd: Option<NativeSocket>,
) -> Option<JsonValue> {
    let fd = fd?;
    let token = event_loop_token(object)?;
    Some(json!({
        "kind": "register_websocket_read_write",
        "event_loop_token": token,
        "websocket_fd": fd,
        "interest": "read_write",
        "reason": "tcp_connect_in_progress",
        "execute_registration": false,
        "worker_key": clean_text(object.get("worker_key"))
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
        "shard_index": optional_u64(object.get("shard_index"))
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
    }))
}

fn start_nonblocking_tcp_connect(address: SocketAddr) -> TcpConnectOutcome {
    let domain = match address {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };
    let socket = match Socket::new(domain, Type::STREAM, Some(Protocol::TCP)) {
        Ok(socket) => socket,
        Err(err) => {
            return TcpConnectOutcome::failed(None, err.raw_os_error(), err.to_string());
        }
    };
    if let Err(err) = socket.set_nonblocking(true) {
        let errno = err.raw_os_error();
        return TcpConnectOutcome::failed(None, errno, err.to_string());
    }

    match socket.connect(&SockAddr::from(address)) {
        Ok(()) => {
            let stream: TcpStream = socket.into();
            let fd = tcp_stream_into_native_socket(stream);
            TcpConnectOutcome::connected(fd)
        }
        Err(err) if is_connect_in_progress(&err) => {
            let errno = err.raw_os_error().unwrap_or(-1);
            let stream: TcpStream = socket.into();
            let fd = tcp_stream_into_native_socket(stream);
            TcpConnectOutcome::in_progress(fd, errno, err.to_string())
        }
        Err(err) => TcpConnectOutcome::failed(None, err.raw_os_error(), err.to_string()),
    }
}

fn finish_nonblocking_tcp_connect(fd: NativeSocket) -> TcpConnectOutcome {
    match native_socket_take_error(fd) {
        Ok(None) => TcpConnectOutcome::connected(fd),
        Ok(Some(err)) => TcpConnectOutcome::failed(Some(fd), err.raw_os_error(), err.to_string()),
        Err(err) => TcpConnectOutcome::failed(Some(fd), err.raw_os_error(), err.to_string()),
    }
}

fn is_connect_in_progress(error: &io::Error) -> bool {
    if error.kind() == io::ErrorKind::WouldBlock {
        return true;
    }
    let Some(errno) = error.raw_os_error() else {
        return false;
    };
    #[cfg(unix)]
    {
        errno == libc::EINPROGRESS || errno == libc::EALREADY || errno == libc::EWOULDBLOCK
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Networking::WinSock::{
            WSAEALREADY, WSAEINPROGRESS, WSAEWOULDBLOCK,
        };
        errno == WSAEINPROGRESS || errno == WSAEALREADY || errno == WSAEWOULDBLOCK
    }
}

fn required_target(
    object: &Map<String, JsonValue>,
    stage: &str,
) -> Result<WebSocketTarget, String> {
    optional_target(object)?
        .ok_or_else(|| format!("WebSocket TCP connect {stage} stage must include a websocket URL."))
}

fn optional_target(object: &Map<String, JsonValue>) -> Result<Option<WebSocketTarget>, String> {
    let Some(url) = clean_text(
        object
            .get("websocket_url")
            .or_else(|| object.get("url"))
            .or_else(|| object.get("gateway_url"))
            .or_else(|| object.get("socket_url")),
    ) else {
        return Ok(None);
    };
    parse_websocket_url(&url).map(Some)
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
    let host_header = if explicit_port || port != default_port {
        format!("{host}:{port}")
    } else {
        host.clone()
    };
    Ok(WebSocketTarget {
        url: raw.to_string(),
        scheme,
        secure,
        host,
        port,
        host_header,
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

fn socket_address_from_request(object: &Map<String, JsonValue>) -> Result<SocketAddr, String> {
    if let Some(address) = clean_text(
        object
            .get("socket_address")
            .or_else(|| object.get("resolved_address"))
            .or_else(|| object.get("address")),
    ) {
        return parse_socket_address(&address);
    }
    if let Some(object) = object.get("socket_address").and_then(JsonValue::as_object) {
        return socket_address_from_object(object);
    }
    if let Some(object) = object
        .get("resolved_address")
        .and_then(JsonValue::as_object)
    {
        return socket_address_from_object(object);
    }
    if let Some(addresses) = object
        .get("resolved_addresses")
        .or_else(|| object.get("socket_addresses"))
        .and_then(JsonValue::as_array)
    {
        let Some(first) = addresses.first() else {
            return Err(
                "WebSocket TCP connect requires at least one resolved address.".to_string(),
            );
        };
        if let Some(address) = clean_text(Some(first)) {
            return parse_socket_address(&address);
        }
        if let Some(object) = first.as_object() {
            return socket_address_from_object(object);
        }
    }
    Err("WebSocket TCP connect requires a resolved socket address.".to_string())
}

fn socket_address_from_object(object: &Map<String, JsonValue>) -> Result<SocketAddr, String> {
    let ip = clean_text(object.get("ip").or_else(|| object.get("host")))
        .ok_or_else(|| "WebSocket resolved address object must include `ip`.".to_string())?;
    let port = optional_u64(object.get("port"))
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| {
            "WebSocket resolved address object must include numeric `port`.".to_string()
        })?;
    let ip = IpAddr::from_str(&ip)
        .map_err(|_| "WebSocket resolved address IP must be numeric.".to_string())?;
    Ok(SocketAddr::new(ip, port))
}

fn parse_socket_address(raw: &str) -> Result<SocketAddr, String> {
    raw.parse::<SocketAddr>()
        .map_err(|_| "WebSocket TCP connect socket address must be numeric host:port.".to_string())
}

fn required_fd(object: &Map<String, JsonValue>) -> Result<NativeSocket, String> {
    let raw = optional_u64(
        object
            .get("websocket_fd")
            .or_else(|| object.get("fd"))
            .or_else(|| object.get("socket_fd")),
    )
    .ok_or_else(|| "WebSocket TCP connect finish stage requires websocket_fd.".to_string())?;
    native_socket_from_u64(raw)
        .map_err(|_| "WebSocket TCP connect fd is outside native socket range.".to_string())
}

fn target_json(target: &WebSocketTarget) -> JsonValue {
    json!({
        "url": target.url,
        "scheme": target.scheme,
        "secure": target.secure,
        "host": target.host,
        "port": target.port,
        "host_header": target.host_header,
        "path_and_query": target.path_and_query,
    })
}

fn socket_address_json(address: SocketAddr) -> JsonValue {
    json!({
        "ip": address.ip().to_string(),
        "port": address.port(),
        "address": address.to_string(),
    })
}

fn configuration_error_payload(stage: &str, message: &str) -> JsonValue {
    base_payload(
        stage,
        "configuration_error",
        json!({
            "ok": false,
            "executed": false,
            "connected": false,
            "connect_in_progress": false,
            "connect_failed": false,
            "error": message,
            "should_register_writable": false,
            "should_start_tls": false,
            "should_write_upgrade_request": false,
            "execute_dns": false,
            "execute_connect": false,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "actions": [
                {
                    "kind": "diagnose_websocket_tcp_connect_configuration_error",
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
        "websocket_connect_contract".to_string(),
        JsonValue::String(WEBSOCKET_CONNECT_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "websocket_connect_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_websocket_connect_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_socket_connect_allowed".to_string(),
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
        .ok_or_else(|| "WebSocket TCP connect request must be an object.".to_string())
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
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn optional_u64(value: Option<&JsonValue>) -> Option<u64> {
    match value? {
        JsonValue::Number(number) => number.as_u64(),
        JsonValue::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

fn event_loop_token(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(
        object
            .get("event_loop_token")
            .or_else(|| object.get("token"))
            .or_else(|| object.get("registration_token")),
    )
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use ait_core::json_support::json;

    struct StartOnlyConnectExecutor;

    impl WebSocketConnectStartExecutor for StartOnlyConnectExecutor {
        fn start_tcp_connect(&self, address: SocketAddr) -> TcpConnectOutcome {
            assert_eq!(address.port(), 12345);
            TcpConnectOutcome::in_progress(99 as NativeSocket, 1, "operation in progress")
        }
    }

    struct FinishOnlyConnectExecutor;

    impl WebSocketConnectFinishExecutor for FinishOnlyConnectExecutor {
        fn finish_tcp_connect(&self, fd: NativeSocket) -> TcpConnectOutcome {
            TcpConnectOutcome::connected(fd)
        }
    }

    #[test]
    fn websocket_connect_stage_helpers_accept_single_capability_executors() {
        let start_request = json!({
            "stage": "start_tcp_connect",
            "socket_address": "127.0.0.1:12345",
            "event_loop_token": 12,
        });
        let started = start_tcp_connect(
            &StartOnlyConnectExecutor,
            start_request.as_object().unwrap(),
        );
        assert_eq!(started["websocket_connect_state"], "tcp_connecting");
        assert_eq!(started["websocket_fd"], 99);
        assert_eq!(
            started["actions"][0]["kind"],
            "register_websocket_read_write"
        );

        let finish_request = json!({
            "stage": "finish_tcp_connect",
            "websocket_fd": 99,
        });
        let finished = finish_tcp_connect(
            &FinishOnlyConnectExecutor,
            finish_request.as_object().unwrap(),
        );
        assert_eq!(finished["websocket_connect_state"], "tcp_connected");
        assert_eq!(finished["websocket_fd"], 99);
    }
}

#[derive(Debug, Clone)]
struct WebSocketTarget {
    url: String,
    scheme: String,
    secure: bool,
    host: String,
    port: u16,
    host_header: String,
    path_and_query: String,
}
