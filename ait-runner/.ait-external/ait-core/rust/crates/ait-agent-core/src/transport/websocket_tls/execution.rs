use crate::platform::{
    native_socket_from_u64, set_native_socket_close_on_exec, tcp_stream_from_native_socket,
    tcp_stream_native_socket, NativeSocket,
};
use crate::transport::websocket_handshake::agent_transport_websocket_handshake_plan_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore};
use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, OnceLock};

const MIGRATION_STAGE: &str = "rust_agent_transport_websocket_tls_stream_boundary";
const WEBSOCKET_TLS_CONTRACT: &str = "ait_agent_core.transport.WebSocketTlsStream.v1";
const DEFAULT_ALPN_PROTOCOL: &str = "http/1.1";
const DEFAULT_MAX_READ_BYTES: usize = 65_536;
const DEFAULT_READ_CHUNK_BYTES: usize = 16_384;

pub trait WebSocketTlsStartHandshakeExecutor {
    fn start_tls_handshake(&self, request: TlsStartRequest) -> TlsHandshakeOutcome;
}

pub trait WebSocketTlsDriveHandshakeExecutor {
    fn drive_tls_handshake(&self, session_id: &str) -> TlsHandshakeOutcome;
}

pub trait WebSocketTlsCloseSessionExecutor {
    fn close_tls_session(&self, session_id: &str, close_fd: bool) -> TlsHandshakeOutcome;
}

pub trait WebSocketTlsPlaintextReadExecutor {
    fn read_tls_plaintext(&self, request: TlsReadRequest) -> TlsIoOutcome {
        TlsIoOutcome::failed(
            None,
            Some(request.session_id),
            None,
            "unsupported_executor",
            "WebSocket TLS executor does not support plaintext reads.",
        )
    }
}

pub trait WebSocketTlsPlaintextWriteExecutor {
    fn write_tls_plaintext(&self, request: TlsWriteRequest) -> TlsIoOutcome {
        TlsIoOutcome::failed(
            None,
            Some(request.session_id),
            None,
            "unsupported_executor",
            "WebSocket TLS executor does not support plaintext writes.",
        )
    }
}

pub trait WebSocketTlsExecutor:
    WebSocketTlsStartHandshakeExecutor
    + WebSocketTlsDriveHandshakeExecutor
    + WebSocketTlsCloseSessionExecutor
    + WebSocketTlsPlaintextReadExecutor
    + WebSocketTlsPlaintextWriteExecutor
{
}

impl<E> WebSocketTlsExecutor for E where
    E: WebSocketTlsStartHandshakeExecutor
        + WebSocketTlsDriveHandshakeExecutor
        + WebSocketTlsCloseSessionExecutor
        + WebSocketTlsPlaintextReadExecutor
        + WebSocketTlsPlaintextWriteExecutor
        + ?Sized
{
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultWebSocketTlsExecutor;

impl WebSocketTlsStartHandshakeExecutor for DefaultWebSocketTlsExecutor {
    fn start_tls_handshake(&self, request: TlsStartRequest) -> TlsHandshakeOutcome {
        default_start_tls_handshake(request)
    }
}

impl WebSocketTlsDriveHandshakeExecutor for DefaultWebSocketTlsExecutor {
    fn drive_tls_handshake(&self, session_id: &str) -> TlsHandshakeOutcome {
        default_drive_tls_handshake(session_id)
    }
}

impl WebSocketTlsCloseSessionExecutor for DefaultWebSocketTlsExecutor {
    fn close_tls_session(&self, session_id: &str, close_fd: bool) -> TlsHandshakeOutcome {
        default_close_tls_session(session_id, close_fd)
    }
}

impl WebSocketTlsPlaintextReadExecutor for DefaultWebSocketTlsExecutor {
    fn read_tls_plaintext(&self, request: TlsReadRequest) -> TlsIoOutcome {
        default_read_tls_plaintext(request)
    }
}

impl WebSocketTlsPlaintextWriteExecutor for DefaultWebSocketTlsExecutor {
    fn write_tls_plaintext(&self, request: TlsWriteRequest) -> TlsIoOutcome {
        default_write_tls_plaintext(request)
    }
}

#[derive(Debug, Clone)]
pub struct TlsStartRequest {
    pub fd: NativeSocket,
    pub session_id: String,
    pub server_name: String,
    pub alpn_protocols: Vec<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct TlsReadRequest {
    pub session_id: String,
    pub max_read_bytes: usize,
    pub read_chunk_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct TlsWriteRequest {
    pub session_id: String,
    pub write_bytes: Vec<u8>,
    pub max_write_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsHandshakeStatus {
    Handshaking,
    Established,
    Failed,
    Closed,
}

#[derive(Debug, Clone)]
pub struct TlsHandshakeOutcome {
    pub status: TlsHandshakeStatus,
    pub fd: Option<NativeSocket>,
    pub session_id: Option<String>,
    pub server_name: Option<String>,
    pub bytes_read: usize,
    pub bytes_written: usize,
    pub wants_read: bool,
    pub wants_write: bool,
    pub error_kind: Option<String>,
    pub error: Option<String>,
}

impl TlsHandshakeOutcome {
    pub fn handshaking(
        fd: NativeSocket,
        session_id: impl Into<String>,
        server_name: impl Into<String>,
        wants_read: bool,
        wants_write: bool,
    ) -> Self {
        Self {
            status: TlsHandshakeStatus::Handshaking,
            fd: Some(fd),
            session_id: Some(session_id.into()),
            server_name: Some(server_name.into()),
            bytes_read: 0,
            bytes_written: 0,
            wants_read,
            wants_write,
            error_kind: None,
            error: None,
        }
    }

    pub fn established(
        fd: NativeSocket,
        session_id: impl Into<String>,
        server_name: impl Into<String>,
    ) -> Self {
        Self {
            status: TlsHandshakeStatus::Established,
            fd: Some(fd),
            session_id: Some(session_id.into()),
            server_name: Some(server_name.into()),
            bytes_read: 0,
            bytes_written: 0,
            wants_read: false,
            wants_write: false,
            error_kind: None,
            error: None,
        }
    }

    pub fn failed(
        fd: Option<NativeSocket>,
        session_id: Option<String>,
        server_name: Option<String>,
        error_kind: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            status: TlsHandshakeStatus::Failed,
            fd,
            session_id,
            server_name,
            bytes_read: 0,
            bytes_written: 0,
            wants_read: false,
            wants_write: false,
            error_kind: Some(error_kind.into()),
            error: Some(error.into()),
        }
    }

    pub fn closed(session_id: impl Into<String>) -> Self {
        Self {
            status: TlsHandshakeStatus::Closed,
            fd: None,
            session_id: Some(session_id.into()),
            server_name: None,
            bytes_read: 0,
            bytes_written: 0,
            wants_read: false,
            wants_write: false,
            error_kind: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TlsIoStatus {
    ReadChunk,
    WouldBlock,
    PeerEof,
    WriteComplete,
    WritePending,
    PartialWrite,
    Failed,
}

#[derive(Debug, Clone)]
pub struct TlsIoOutcome {
    pub status: TlsIoStatus,
    pub fd: Option<NativeSocket>,
    pub session_id: Option<String>,
    pub server_name: Option<String>,
    pub bytes_read: usize,
    pub bytes_written: usize,
    pub read_bytes: Vec<u8>,
    pub written_bytes: Vec<u8>,
    pub remaining_write_bytes: Vec<u8>,
    pub tls_wire_bytes_read: usize,
    pub tls_wire_bytes_written: usize,
    pub wants_read: bool,
    pub wants_write: bool,
    pub error_kind: Option<String>,
    pub error: Option<String>,
}

impl TlsIoOutcome {
    pub fn read_chunk(
        fd: NativeSocket,
        session_id: impl Into<String>,
        server_name: impl Into<String>,
        read_bytes: Vec<u8>,
        wants_read: bool,
        wants_write: bool,
    ) -> Self {
        Self {
            status: TlsIoStatus::ReadChunk,
            fd: Some(fd),
            session_id: Some(session_id.into()),
            server_name: Some(server_name.into()),
            bytes_read: read_bytes.len(),
            bytes_written: 0,
            read_bytes,
            written_bytes: Vec::new(),
            remaining_write_bytes: Vec::new(),
            tls_wire_bytes_read: 0,
            tls_wire_bytes_written: 0,
            wants_read,
            wants_write,
            error_kind: None,
            error: None,
        }
    }

    pub fn would_block(
        fd: NativeSocket,
        session_id: impl Into<String>,
        server_name: impl Into<String>,
        wants_read: bool,
        wants_write: bool,
    ) -> Self {
        Self {
            status: TlsIoStatus::WouldBlock,
            fd: Some(fd),
            session_id: Some(session_id.into()),
            server_name: Some(server_name.into()),
            bytes_read: 0,
            bytes_written: 0,
            read_bytes: Vec::new(),
            written_bytes: Vec::new(),
            remaining_write_bytes: Vec::new(),
            tls_wire_bytes_read: 0,
            tls_wire_bytes_written: 0,
            wants_read,
            wants_write,
            error_kind: Some("would_block".to_string()),
            error: None,
        }
    }

    pub fn write_complete(
        fd: NativeSocket,
        session_id: impl Into<String>,
        server_name: impl Into<String>,
        written_bytes: Vec<u8>,
        tls_wire_bytes_read: usize,
        tls_wire_bytes_written: usize,
        wants_read: bool,
    ) -> Self {
        Self {
            status: TlsIoStatus::WriteComplete,
            fd: Some(fd),
            session_id: Some(session_id.into()),
            server_name: Some(server_name.into()),
            bytes_read: 0,
            bytes_written: written_bytes.len(),
            read_bytes: Vec::new(),
            written_bytes,
            remaining_write_bytes: Vec::new(),
            tls_wire_bytes_read,
            tls_wire_bytes_written,
            wants_read,
            wants_write: false,
            error_kind: None,
            error: None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn write_pending(
        fd: NativeSocket,
        session_id: impl Into<String>,
        server_name: impl Into<String>,
        written_bytes: Vec<u8>,
        remaining_write_bytes: Vec<u8>,
        tls_wire_bytes_read: usize,
        tls_wire_bytes_written: usize,
        wants_read: bool,
        wants_write: bool,
    ) -> Self {
        let status = if written_bytes.is_empty() && !remaining_write_bytes.is_empty() {
            TlsIoStatus::PartialWrite
        } else {
            TlsIoStatus::WritePending
        };
        Self {
            status,
            fd: Some(fd),
            session_id: Some(session_id.into()),
            server_name: Some(server_name.into()),
            bytes_read: 0,
            bytes_written: written_bytes.len(),
            read_bytes: Vec::new(),
            written_bytes,
            remaining_write_bytes,
            tls_wire_bytes_read,
            tls_wire_bytes_written,
            wants_read,
            wants_write,
            error_kind: None,
            error: None,
        }
    }

    pub fn peer_eof(
        fd: NativeSocket,
        session_id: impl Into<String>,
        server_name: impl Into<String>,
    ) -> Self {
        Self {
            status: TlsIoStatus::PeerEof,
            fd: Some(fd),
            session_id: Some(session_id.into()),
            server_name: Some(server_name.into()),
            bytes_read: 0,
            bytes_written: 0,
            read_bytes: Vec::new(),
            written_bytes: Vec::new(),
            remaining_write_bytes: Vec::new(),
            tls_wire_bytes_read: 0,
            tls_wire_bytes_written: 0,
            wants_read: false,
            wants_write: false,
            error_kind: None,
            error: None,
        }
    }

    pub fn failed(
        fd: Option<NativeSocket>,
        session_id: Option<String>,
        server_name: Option<String>,
        error_kind: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            status: TlsIoStatus::Failed,
            fd,
            session_id,
            server_name,
            bytes_read: 0,
            bytes_written: 0,
            read_bytes: Vec::new(),
            written_bytes: Vec::new(),
            remaining_write_bytes: Vec::new(),
            tls_wire_bytes_read: 0,
            tls_wire_bytes_written: 0,
            wants_read: false,
            wants_write: false,
            error_kind: Some(error_kind.into()),
            error: Some(error.into()),
        }
    }
}

pub fn agent_transport_websocket_tls_execute_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    plan_with_websocket_tls_executor(&DefaultWebSocketTlsExecutor, request)
}

pub fn plan_with_websocket_tls_executor<E>(
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: WebSocketTlsExecutor + ?Sized,
{
    let object = request_object(request)?;
    let stage = clean_text(object.get("stage")).unwrap_or_else(|| "plan".to_string());

    match stage.as_str() {
        "plan" | "tls_plan" | "handshake_plan" => Ok(plan_tls_stream(object)),
        "start" | "start_tls" | "start_tls_handshake" | "wrap_tls_fd" => {
            Ok(start_tls_handshake(executor, object))
        }
        "resume" | "poll" | "drive" | "handshake_ready" | "tls_ready" => {
            Ok(drive_tls_handshake(executor, object))
        }
        "read" | "read_tls" | "read_plaintext" | "read_tls_plaintext" | "read_ready_tls" => {
            Ok(read_tls_plaintext(executor, object))
        }
        "write" | "write_tls" | "write_plaintext" | "write_tls_plaintext" | "write_bytes_tls" => {
            Ok(write_tls_plaintext(executor, object))
        }
        "flush" | "flush_tls" | "flush_tls_plaintext" => Ok(flush_tls_plaintext(executor, object)),
        "close" | "close_tls" | "drop_tls_session" => Ok(close_tls_session(executor, object)),
        other => Err(format!("unsupported WebSocket TLS stream stage: {other}")),
    }
}

fn plan_tls_stream(object: &Map<String, JsonValue>) -> JsonValue {
    let target = match target_from_request(object) {
        Ok(target) => target,
        Err(message) => return configuration_error_payload("plan", &message),
    };
    if !target.secure {
        return base_payload(
            "plan",
            "tls_not_required",
            json!({
                "ok": true,
                "executed": false,
                "tls_required": false,
                "secure": false,
                "url": target.url,
                "server_name": JsonValue::Null,
                "alpn_protocols": [],
                "execute_tls": false,
                "execute_upgrade_write": false,
                "actions": [
                    {
                        "kind": "skip_websocket_tls_for_plain_ws",
                        "url": target.url,
                        "execute_tls": false,
                    }
                ],
            }),
        );
    }
    let alpn_protocols = alpn_protocol_strings(object);
    base_payload(
        "plan",
        "tls_planned",
        json!({
            "ok": true,
            "executed": false,
            "tls_required": true,
            "secure": true,
            "url": target.url,
            "server_name": target.server_name,
            "host": target.host,
            "port": target.port,
            "alpn_protocols": alpn_protocols,
            "certificate_verification": "webpki_roots",
            "insecure_certificate_verification_allowed": false,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "actions": [
                {
                    "kind": "start_websocket_tls_handshake",
                    "websocket_fd": fd_value(object),
                    "server_name": target.server_name,
                    "alpn_protocols": alpn_protocols,
                    "execute_tls": false,
                    "python_tls_allowed": false,
                }
            ],
        }),
    )
}

fn start_tls_handshake<E>(executor: &E, object: &Map<String, JsonValue>) -> JsonValue
where
    E: WebSocketTlsStartHandshakeExecutor + ?Sized,
{
    let fd = match required_fd(object) {
        Ok(fd) => fd,
        Err(message) => return configuration_error_payload("start_tls_handshake", &message),
    };
    let target = match target_from_request(object) {
        Ok(target) => target,
        Err(message) => return configuration_error_payload("start_tls_handshake", &message),
    };
    if !target.secure {
        return configuration_error_payload(
            "start_tls_handshake",
            "WebSocket TLS stream requires a secure `wss://` target or explicit TLS server name.",
        );
    }
    if let Err(message) = validate_certificate_verification(object) {
        return configuration_error_payload("start_tls_handshake", &message);
    }

    let session_id = tls_session_id(object, fd);
    let request = TlsStartRequest {
        fd,
        session_id,
        server_name: target.server_name,
        alpn_protocols: alpn_protocol_bytes(object),
    };
    let outcome = executor.start_tls_handshake(request);
    tls_outcome_payload("start_tls_handshake", object, outcome)
}

fn drive_tls_handshake<E>(executor: &E, object: &Map<String, JsonValue>) -> JsonValue
where
    E: WebSocketTlsDriveHandshakeExecutor + ?Sized,
{
    let session_id = match required_session_id(object) {
        Ok(session_id) => session_id,
        Err(message) => return configuration_error_payload("drive_tls_handshake", &message),
    };
    let outcome = executor.drive_tls_handshake(&session_id);
    tls_outcome_payload("drive_tls_handshake", object, outcome)
}

fn close_tls_session<E>(executor: &E, object: &Map<String, JsonValue>) -> JsonValue
where
    E: WebSocketTlsCloseSessionExecutor + ?Sized,
{
    let session_id = match required_session_id(object) {
        Ok(session_id) => session_id,
        Err(message) => return configuration_error_payload("close_tls_session", &message),
    };
    let close_fd = optional_bool(object.get("close_fd")).unwrap_or(true);
    let outcome = executor.close_tls_session(&session_id, close_fd);
    tls_outcome_payload("close_tls_session", object, outcome)
}

fn read_tls_plaintext<E>(executor: &E, object: &Map<String, JsonValue>) -> JsonValue
where
    E: WebSocketTlsPlaintextReadExecutor + ?Sized,
{
    let session_id = match required_session_id(object) {
        Ok(session_id) => session_id,
        Err(message) => return configuration_error_payload("read_tls_plaintext", &message),
    };
    let max_read_bytes = optional_usize(object.get("max_read_bytes"))
        .unwrap_or(DEFAULT_MAX_READ_BYTES)
        .max(1);
    let read_chunk_bytes = optional_usize(object.get("read_chunk_bytes"))
        .unwrap_or(DEFAULT_READ_CHUNK_BYTES)
        .max(1)
        .min(max_read_bytes);
    let outcome = executor.read_tls_plaintext(TlsReadRequest {
        session_id,
        max_read_bytes,
        read_chunk_bytes,
    });
    tls_io_payload("read_tls_plaintext", object, outcome)
}

fn write_tls_plaintext<E>(executor: &E, object: &Map<String, JsonValue>) -> JsonValue
where
    E: WebSocketTlsPlaintextWriteExecutor + ?Sized,
{
    let session_id = match required_session_id(object) {
        Ok(session_id) => session_id,
        Err(message) => return configuration_error_payload("write_tls_plaintext", &message),
    };
    let write_bytes = match request_optional_bytes(
        object,
        &["write_bytes", "plaintext_bytes", "bytes"],
        &["write_hex", "plaintext_hex", "hex"],
        &["write_text", "plaintext_text", "text"],
    ) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return configuration_error_payload(
                "write_tls_plaintext",
                "WebSocket TLS plaintext write requires write bytes or hex.",
            );
        }
        Err(message) => return configuration_error_payload("write_tls_plaintext", &message),
    };
    let max_write_bytes =
        optional_usize(object.get("max_write_bytes")).unwrap_or(write_bytes.len());
    let outcome = executor.write_tls_plaintext(TlsWriteRequest {
        session_id,
        write_bytes,
        max_write_bytes,
    });
    tls_io_payload("write_tls_plaintext", object, outcome)
}

fn flush_tls_plaintext<E>(executor: &E, object: &Map<String, JsonValue>) -> JsonValue
where
    E: WebSocketTlsPlaintextWriteExecutor + ?Sized,
{
    let session_id = match required_session_id(object) {
        Ok(session_id) => session_id,
        Err(message) => return configuration_error_payload("flush_tls_plaintext", &message),
    };
    let outcome = executor.write_tls_plaintext(TlsWriteRequest {
        session_id,
        write_bytes: Vec::new(),
        max_write_bytes: 0,
    });
    tls_io_payload("flush_tls_plaintext", object, outcome)
}

fn tls_outcome_payload(
    stage: &str,
    object: &Map<String, JsonValue>,
    outcome: TlsHandshakeOutcome,
) -> JsonValue {
    let state = match outcome.status {
        TlsHandshakeStatus::Handshaking if outcome.wants_read && outcome.wants_write => {
            "tls_handshake_want_read_write"
        }
        TlsHandshakeStatus::Handshaking if outcome.wants_write => "tls_handshake_want_write",
        TlsHandshakeStatus::Handshaking => "tls_handshake_want_read",
        TlsHandshakeStatus::Established => "tls_established",
        TlsHandshakeStatus::Failed => "tls_handshake_error",
        TlsHandshakeStatus::Closed => "tls_session_closed",
    };
    let fd_json = outcome.fd.map(JsonValue::from).unwrap_or(JsonValue::Null);
    let session_id_json = outcome
        .session_id
        .clone()
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);
    let server_name_json = outcome
        .server_name
        .clone()
        .or_else(|| server_name_from_object(object).ok())
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);
    let mut actions = Vec::new();

    match outcome.status {
        TlsHandshakeStatus::Handshaking => actions.push(registration_action(object, &outcome)),
        TlsHandshakeStatus::Established => {
            actions.extend(upgrade_ready_actions(object, &outcome));
        }
        TlsHandshakeStatus::Failed => actions.push(json!({
            "kind": "diagnose_websocket_tls_handshake_error",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "server_name": server_name_json,
            "error_kind": outcome.error_kind.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
            "error": outcome.error.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        })),
        TlsHandshakeStatus::Closed => actions.push(json!({
            "kind": "mark_websocket_tls_session_closed",
            "tls_connection_id": session_id_json,
        })),
    }

    base_payload(
        stage,
        state,
        json!({
            "ok": outcome.status != TlsHandshakeStatus::Failed,
            "executed": stage != "plan",
            "tls_required": true,
            "secure": true,
            "tls_established": outcome.status == TlsHandshakeStatus::Established,
            "tls_handshaking": outcome.status == TlsHandshakeStatus::Handshaking,
            "tls_failed": outcome.status == TlsHandshakeStatus::Failed,
            "tls_closed": outcome.status == TlsHandshakeStatus::Closed,
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "server_name": server_name_json,
            "bytes_read": outcome.bytes_read,
            "bytes_written": outcome.bytes_written,
            "wants_read": outcome.wants_read,
            "wants_write": outcome.wants_write,
            "interest": interest(outcome.wants_read, outcome.wants_write),
            "should_register_readable": outcome.status == TlsHandshakeStatus::Handshaking && outcome.wants_read,
            "should_register_writable": outcome.status == TlsHandshakeStatus::Handshaking && outcome.wants_write,
            "should_write_upgrade_request": outcome.status == TlsHandshakeStatus::Established,
            "execute_tls": true,
            "execute_upgrade_write": false,
            "error_kind": outcome.error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "error": outcome.error.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "actions": actions,
        }),
    )
}

fn tls_io_payload(
    stage: &str,
    object: &Map<String, JsonValue>,
    outcome: TlsIoOutcome,
) -> JsonValue {
    let state = match outcome.status {
        TlsIoStatus::ReadChunk => "tls_read_chunk",
        TlsIoStatus::WouldBlock => "tls_would_block",
        TlsIoStatus::PeerEof => "tls_peer_eof",
        TlsIoStatus::WriteComplete => "tls_write_complete",
        TlsIoStatus::WritePending => "tls_write_pending",
        TlsIoStatus::PartialWrite => "tls_partial_write",
        TlsIoStatus::Failed => "tls_io_error",
    };
    let fd_json = outcome.fd.map(JsonValue::from).unwrap_or(JsonValue::Null);
    let session_id_json = outcome
        .session_id
        .clone()
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);
    let server_name_json = outcome
        .server_name
        .clone()
        .or_else(|| server_name_from_object(object).ok())
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);
    let read_hex = bytes_hex(&outcome.read_bytes);
    let written_hex = bytes_hex(&outcome.written_bytes);
    let remaining_write_hex = bytes_hex(&outcome.remaining_write_bytes);
    let actions = tls_io_actions(
        object,
        &outcome,
        fd_json.clone(),
        session_id_json.clone(),
        read_hex.clone(),
        written_hex.clone(),
        remaining_write_hex.clone(),
    );

    base_payload(
        stage,
        state,
        json!({
            "ok": outcome.status != TlsIoStatus::Failed,
            "executed": true,
            "tls_required": true,
            "secure": true,
            "tls_established": outcome.status != TlsIoStatus::Failed,
            "tls_failed": outcome.status == TlsIoStatus::Failed,
            "tls_closed": outcome.status == TlsIoStatus::PeerEof,
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "server_name": server_name_json,
            "bytes_read": outcome.bytes_read,
            "read_byte_count": outcome.read_bytes.len(),
            "read_bytes": bytes_json(&outcome.read_bytes),
            "read_hex": read_hex,
            "bytes_written": outcome.bytes_written,
            "written_byte_count": outcome.written_bytes.len(),
            "written_bytes": bytes_json(&outcome.written_bytes),
            "written_hex": written_hex,
            "remaining_write_byte_count": outcome.remaining_write_bytes.len(),
            "remaining_write_bytes": bytes_json(&outcome.remaining_write_bytes),
            "remaining_write_hex": remaining_write_hex,
            "tls_wire_bytes_read": outcome.tls_wire_bytes_read,
            "tls_wire_bytes_written": outcome.tls_wire_bytes_written,
            "wants_read": outcome.wants_read,
            "wants_write": outcome.wants_write,
            "interest": interest(outcome.wants_read, outcome.wants_write),
            "read_eof": outcome.status == TlsIoStatus::PeerEof,
            "would_block": outcome.status == TlsIoStatus::WouldBlock,
            "write_complete": outcome.status == TlsIoStatus::WriteComplete,
            "write_pending": outcome.status == TlsIoStatus::WritePending,
            "execute_tls": true,
            "execute_tls_io": true,
            "execute_upgrade_write": false,
            "python_tls_io_allowed": false,
            "error_kind": outcome.error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "error": outcome.error.map(JsonValue::from).unwrap_or(JsonValue::Null),
            "actions": actions,
        }),
    )
}

fn tls_io_actions(
    object: &Map<String, JsonValue>,
    outcome: &TlsIoOutcome,
    fd_json: JsonValue,
    session_id_json: JsonValue,
    read_hex: String,
    written_hex: String,
    remaining_write_hex: String,
) -> Vec<JsonValue> {
    match outcome.status {
        TlsIoStatus::ReadChunk => vec![json!({
            "kind": "deliver_websocket_tls_read_chunk",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "read_byte_count": outcome.read_bytes.len(),
            "read_bytes": bytes_json(&outcome.read_bytes),
            "read_hex": read_hex,
        })],
        TlsIoStatus::WouldBlock => vec![json!({
            "kind": "retry_websocket_tls_io_when_ready",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "interest": interest(outcome.wants_read, outcome.wants_write),
        })],
        TlsIoStatus::PeerEof => vec![json!({
            "kind": "mark_websocket_tls_peer_eof",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
        })],
        TlsIoStatus::WriteComplete => vec![json!({
            "kind": "mark_websocket_tls_write_complete",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "bytes_written": outcome.written_bytes.len(),
            "written_hex": written_hex,
        })],
        TlsIoStatus::WritePending | TlsIoStatus::PartialWrite => vec![json!({
            "kind": "retry_websocket_tls_write_when_writable",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
            "bytes_written": outcome.written_bytes.len(),
            "written_hex": written_hex,
            "remaining_write_byte_count": outcome.remaining_write_bytes.len(),
            "remaining_write_bytes": bytes_json(&outcome.remaining_write_bytes),
            "remaining_write_hex": remaining_write_hex,
            "interest": interest(outcome.wants_read, outcome.wants_write),
        })],
        TlsIoStatus::Failed => vec![json!({
            "kind": "diagnose_websocket_tls_io_error",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "error_kind": outcome.error_kind.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
            "error": outcome.error.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        })],
    }
}

fn registration_action(
    object: &Map<String, JsonValue>,
    outcome: &TlsHandshakeOutcome,
) -> JsonValue {
    json!({
        "kind": "register_websocket_tls_handshake",
        "event_loop_token": event_loop_token(object).map(JsonValue::from).unwrap_or(JsonValue::Null),
        "tls_connection_id": outcome.session_id.clone().map(JsonValue::from).unwrap_or(JsonValue::Null),
        "websocket_fd": outcome.fd.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "interest": interest(outcome.wants_read, outcome.wants_write),
        "reason": "tls_handshake_in_progress",
        "execute_registration": false,
        "worker_key": clean_text(object.get("worker_key")).map(JsonValue::from).unwrap_or(JsonValue::Null),
        "shard_index": optional_u64(object.get("shard_index")).map(JsonValue::from).unwrap_or(JsonValue::Null),
    })
}

fn upgrade_ready_actions(
    object: &Map<String, JsonValue>,
    outcome: &TlsHandshakeOutcome,
) -> Vec<JsonValue> {
    let fd_json = outcome.fd.map(JsonValue::from).unwrap_or(JsonValue::Null);
    let session_id_json = outcome
        .session_id
        .clone()
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null);
    let Some(url) = clean_text(
        object
            .get("websocket_url")
            .or_else(|| object.get("url"))
            .or_else(|| object.get("gateway_url"))
            .or_else(|| object.get("socket_url")),
    ) else {
        return vec![json!({
            "kind": "websocket_tls_ready_for_upgrade",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "execute_upgrade_write": false,
            "reason": "missing_websocket_url",
        })];
    };
    let Some(key) = clean_text(
        object
            .get("sec_websocket_key")
            .or_else(|| object.get("websocket_key"))
            .or_else(|| object.get("key")),
    ) else {
        return vec![json!({
            "kind": "plan_websocket_tls_upgrade_request",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "url": url,
            "execute_upgrade_write": false,
            "reason": "missing_sec_websocket_key",
        })];
    };
    let mut request = json!({
        "stage": "upgrade_request",
        "url": url,
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
    match agent_transport_websocket_handshake_plan_json(&request) {
        Ok(handshake) if handshake["ok"] == true => vec![
            json!({
                "kind": "write_websocket_tls_upgrade_request",
                "websocket_fd": fd_json,
                "tls_connection_id": session_id_json,
                "url": url,
                "request_bytes": handshake["request_bytes"].clone(),
                "request_hex": handshake["request_hex"].clone(),
                "expected_sec_websocket_accept": handshake["expected_sec_websocket_accept"].clone(),
                "execute_write": false,
                "requires_tls_session": true,
            }),
            json!({
                "kind": "await_websocket_tls_upgrade_response",
                "websocket_fd": fd_json,
                "tls_connection_id": session_id_json,
                "expected_status_code": 101,
                "expected_sec_websocket_accept": handshake["expected_sec_websocket_accept"].clone(),
            }),
        ],
        Ok(handshake) => vec![json!({
            "kind": "diagnose_websocket_tls_upgrade_request_configuration_error",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "handshake_result": handshake,
        })],
        Err(error) => vec![json!({
            "kind": "diagnose_websocket_tls_upgrade_request_configuration_error",
            "websocket_fd": fd_json,
            "tls_connection_id": session_id_json,
            "error": error,
        })],
    }
}

fn default_start_tls_handshake(request: TlsStartRequest) -> TlsHandshakeOutcome {
    if registry_contains(&request.session_id) {
        return TlsHandshakeOutcome::failed(
            Some(request.fd),
            Some(request.session_id),
            Some(request.server_name),
            "already_exists",
            "WebSocket TLS session already exists.",
        );
    }
    let server_name = match ServerName::try_from(request.server_name.clone()) {
        Ok(server_name) => server_name,
        Err(_) => {
            return TlsHandshakeOutcome::failed(
                Some(request.fd),
                Some(request.session_id),
                Some(request.server_name),
                "invalid_server_name",
                "WebSocket TLS server name is invalid.",
            );
        }
    };
    let config = match tls_client_config(request.alpn_protocols) {
        Ok(config) => config,
        Err(message) => {
            return TlsHandshakeOutcome::failed(
                Some(request.fd),
                Some(request.session_id),
                Some(request.server_name),
                "tls_config_error",
                message,
            );
        }
    };
    let connection = match ClientConnection::new(config, server_name) {
        Ok(connection) => connection,
        Err(err) => {
            return TlsHandshakeOutcome::failed(
                Some(request.fd),
                Some(request.session_id),
                Some(request.server_name),
                "tls_client_error",
                err.to_string(),
            );
        }
    };
    let stream = unsafe { tcp_stream_from_native_socket(request.fd) };
    if let Err(err) =
        set_native_socket_close_on_exec(request.fd).and_then(|_| stream.set_nonblocking(true))
    {
        return TlsHandshakeOutcome::failed(
            Some(request.fd),
            Some(request.session_id),
            Some(request.server_name),
            "fd_configuration_error",
            err.to_string(),
        );
    }
    let session_id = request.session_id.clone();
    let server_name_text = request.server_name.clone();
    {
        let mut registry = tls_registry().lock().unwrap();
        registry.insert(
            session_id.clone(),
            TlsSession {
                stream,
                connection,
                server_name: server_name_text.clone(),
            },
        );
    }
    default_drive_tls_handshake(&session_id)
}

fn default_drive_tls_handshake(session_id: &str) -> TlsHandshakeOutcome {
    let Some(mut session) = tls_registry().lock().unwrap().remove(session_id) else {
        return TlsHandshakeOutcome::failed(
            None,
            Some(session_id.to_string()),
            None,
            "missing_session",
            "WebSocket TLS session is not registered.",
        );
    };
    let fd = tcp_stream_native_socket(&session.stream);
    let server_name = session.server_name.clone();
    let mut keep_session = true;
    let outcome = match session.connection.complete_io(&mut session.stream) {
        Ok((bytes_read, bytes_written)) => {
            let status = if session.connection.is_handshaking() {
                TlsHandshakeStatus::Handshaking
            } else {
                TlsHandshakeStatus::Established
            };
            TlsHandshakeOutcome {
                status,
                fd: Some(fd),
                session_id: Some(session_id.to_string()),
                server_name: Some(server_name),
                bytes_read,
                bytes_written,
                wants_read: session.connection.wants_read(),
                wants_write: session.connection.wants_write(),
                error_kind: None,
                error: None,
            }
        }
        Err(err) if err.kind() == io::ErrorKind::WouldBlock => TlsHandshakeOutcome {
            status: TlsHandshakeStatus::Handshaking,
            fd: Some(fd),
            session_id: Some(session_id.to_string()),
            server_name: Some(server_name),
            bytes_read: 0,
            bytes_written: 0,
            wants_read: session.connection.wants_read(),
            wants_write: session.connection.wants_write(),
            error_kind: Some("would_block".to_string()),
            error: Some(err.to_string()),
        },
        Err(err) => {
            keep_session = false;
            TlsHandshakeOutcome::failed(
                Some(fd),
                Some(session_id.to_string()),
                Some(server_name),
                io_error_kind(&err),
                err.to_string(),
            )
        }
    };
    if keep_session {
        tls_registry()
            .lock()
            .unwrap()
            .insert(session_id.to_string(), session);
    }
    outcome
}

fn default_close_tls_session(session_id: &str, close_fd: bool) -> TlsHandshakeOutcome {
    let mut registry = tls_registry().lock().unwrap();
    let Some(session) = registry.remove(session_id) else {
        return TlsHandshakeOutcome::failed(
            None,
            Some(session_id.to_string()),
            None,
            "missing_session",
            "WebSocket TLS session is not registered.",
        );
    };
    if !close_fd {
        std::mem::forget(session.stream);
    }
    TlsHandshakeOutcome::closed(session_id.to_string())
}

fn default_read_tls_plaintext(request: TlsReadRequest) -> TlsIoOutcome {
    let Some(mut session) = tls_registry().lock().unwrap().remove(&request.session_id) else {
        return TlsIoOutcome::failed(
            None,
            Some(request.session_id),
            None,
            "missing_session",
            "WebSocket TLS session is not registered.",
        );
    };
    let fd = tcp_stream_native_socket(&session.stream);
    let server_name = session.server_name.clone();
    if session.connection.is_handshaking() {
        tls_registry()
            .lock()
            .unwrap()
            .insert(request.session_id.clone(), session);
        return TlsIoOutcome::failed(
            Some(fd),
            Some(request.session_id),
            Some(server_name),
            "handshake_required",
            "WebSocket TLS session is still handshaking.",
        );
    }

    let mut read_bytes = Vec::new();
    let mut wire_read = 0usize;
    let mut wire_written = 0usize;
    let mut keep_session = true;
    let mut peer_eof = false;
    let mut fatal_error: Option<io::Error> = None;

    while read_bytes.len() < request.max_read_bytes {
        let remaining = request.max_read_bytes.saturating_sub(read_bytes.len());
        let chunk_len = request.read_chunk_bytes.min(remaining).max(1);
        let mut buffer = vec![0u8; chunk_len];
        match session.connection.reader().read(&mut buffer) {
            Ok(0) => {}
            Ok(count) => {
                read_bytes.extend_from_slice(&buffer[..count]);
                continue;
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => {
                fatal_error = Some(err);
                break;
            }
        }

        match session.connection.complete_io(&mut session.stream) {
            Ok((received, sent)) => {
                wire_read += received;
                wire_written += sent;
                if received == 0 && sent == 0 {
                    break;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => {
                keep_session = false;
                peer_eof = true;
                break;
            }
            Err(err) => {
                fatal_error = Some(err);
                keep_session = false;
                break;
            }
        }
    }

    let wants_read = session.connection.wants_read();
    let wants_write = session.connection.wants_write();
    let outcome = if let Some(err) = fatal_error {
        keep_session = false;
        TlsIoOutcome::failed(
            Some(fd),
            Some(request.session_id.clone()),
            Some(server_name.clone()),
            io_error_kind(&err),
            err.to_string(),
        )
    } else if !read_bytes.is_empty() {
        let mut outcome = TlsIoOutcome::read_chunk(
            fd,
            request.session_id.clone(),
            server_name.clone(),
            read_bytes,
            wants_read,
            wants_write,
        );
        outcome.tls_wire_bytes_read = wire_read;
        outcome.tls_wire_bytes_written = wire_written;
        outcome
    } else if peer_eof {
        TlsIoOutcome::peer_eof(fd, request.session_id.clone(), server_name.clone())
    } else {
        let mut outcome = TlsIoOutcome::would_block(
            fd,
            request.session_id.clone(),
            server_name.clone(),
            wants_read,
            wants_write,
        );
        outcome.tls_wire_bytes_read = wire_read;
        outcome.tls_wire_bytes_written = wire_written;
        outcome
    };
    if keep_session {
        tls_registry()
            .lock()
            .unwrap()
            .insert(request.session_id.clone(), session);
    }
    outcome
}

fn default_write_tls_plaintext(request: TlsWriteRequest) -> TlsIoOutcome {
    let Some(mut session) = tls_registry().lock().unwrap().remove(&request.session_id) else {
        return TlsIoOutcome::failed(
            None,
            Some(request.session_id),
            None,
            "missing_session",
            "WebSocket TLS session is not registered.",
        );
    };
    let fd = tcp_stream_native_socket(&session.stream);
    let server_name = session.server_name.clone();
    if session.connection.is_handshaking() {
        tls_registry()
            .lock()
            .unwrap()
            .insert(request.session_id.clone(), session);
        return TlsIoOutcome::failed(
            Some(fd),
            Some(request.session_id),
            Some(server_name),
            "handshake_required",
            "WebSocket TLS session is still handshaking.",
        );
    }

    let write_limit = request.write_bytes.len().min(request.max_write_bytes);
    let mut staged = 0usize;
    let mut fatal_error: Option<io::Error> = None;
    while staged < write_limit {
        match session
            .connection
            .writer()
            .write(&request.write_bytes[staged..write_limit])
        {
            Ok(0) => break,
            Ok(count) => staged += count,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => break,
            Err(err) => {
                fatal_error = Some(err);
                break;
            }
        }
    }

    let mut wire_read = 0usize;
    let mut wire_written = 0usize;
    if fatal_error.is_none() {
        match session.connection.complete_io(&mut session.stream) {
            Ok((received, sent)) => {
                wire_read += received;
                wire_written += sent;
            }
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(err) => fatal_error = Some(err),
        }
    }

    let wants_read = session.connection.wants_read();
    let wants_write = session.connection.wants_write();
    let written_bytes = request.write_bytes[..staged].to_vec();
    let remaining_write_bytes = request.write_bytes[staged..].to_vec();
    let outcome = if let Some(err) = fatal_error {
        TlsIoOutcome::failed(
            Some(fd),
            Some(request.session_id.clone()),
            Some(server_name.clone()),
            io_error_kind(&err),
            err.to_string(),
        )
    } else if remaining_write_bytes.is_empty() && !wants_write {
        TlsIoOutcome::write_complete(
            fd,
            request.session_id.clone(),
            server_name.clone(),
            written_bytes,
            wire_read,
            wire_written,
            wants_read,
        )
    } else {
        TlsIoOutcome::write_pending(
            fd,
            request.session_id.clone(),
            server_name.clone(),
            written_bytes,
            remaining_write_bytes,
            wire_read,
            wire_written,
            wants_read,
            wants_write,
        )
    };
    if outcome.status != TlsIoStatus::Failed {
        tls_registry()
            .lock()
            .unwrap()
            .insert(request.session_id.clone(), session);
    }
    outcome
}

struct TlsSession {
    stream: TcpStream,
    connection: ClientConnection,
    server_name: String,
}

fn tls_registry() -> &'static Mutex<HashMap<String, TlsSession>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, TlsSession>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn registry_contains(session_id: &str) -> bool {
    tls_registry().lock().unwrap().contains_key(session_id)
}

fn tls_client_config(alpn_protocols: Vec<Vec<u8>>) -> Result<Arc<ClientConfig>, String> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let root_store = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let mut config = ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    config.alpn_protocols = alpn_protocols;
    Ok(Arc::new(config))
}

fn target_from_request(object: &Map<String, JsonValue>) -> Result<TlsTarget, String> {
    if let Ok(server_name) = server_name_from_object(object) {
        return Ok(TlsTarget {
            secure: true,
            url: clean_text(
                object
                    .get("websocket_url")
                    .or_else(|| object.get("url"))
                    .or_else(|| object.get("gateway_url"))
                    .or_else(|| object.get("socket_url")),
            )
            .unwrap_or_default(),
            host: server_name.clone(),
            port: optional_u64(object.get("port"))
                .and_then(|port| u16::try_from(port).ok())
                .unwrap_or(443),
            server_name,
        });
    }
    let Some(url) = clean_text(
        object
            .get("websocket_url")
            .or_else(|| object.get("url"))
            .or_else(|| object.get("gateway_url"))
            .or_else(|| object.get("socket_url")),
    ) else {
        return Err(
            "WebSocket TLS stream requires `server_name`, `host`, or a websocket URL.".to_string(),
        );
    };
    parse_websocket_url(&url)
}

fn parse_websocket_url(raw: &str) -> Result<TlsTarget, String> {
    let raw = raw.trim();
    let (scheme, rest) = raw
        .split_once("://")
        .ok_or_else(|| "WebSocket TLS URL must include `ws://` or `wss://` scheme.".to_string())?;
    let secure = match scheme.to_ascii_lowercase().as_str() {
        "wss" => true,
        "ws" => false,
        other => return Err(format!("unsupported WebSocket URL scheme `{other}`.")),
    };
    let default_port = if secure { 443 } else { 80 };
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.trim().is_empty() {
        return Err("WebSocket TLS URL must include a host.".to_string());
    }
    let (host, port) = parse_authority(authority, default_port)?;
    let server_name = host.trim_matches(&['[', ']'][..]).to_string();
    validate_server_name(&server_name)?;
    Ok(TlsTarget {
        secure,
        url: raw.to_string(),
        host,
        port,
        server_name,
    })
}

fn parse_authority(authority: &str, default_port: u16) -> Result<(String, u16), String> {
    if authority.contains('@') {
        return Err("WebSocket TLS URL userinfo is not supported.".to_string());
    }
    if authority.starts_with('[') {
        let end = authority
            .find(']')
            .ok_or_else(|| "WebSocket IPv6 host must be bracketed.".to_string())?;
        let host = authority[..=end].to_string();
        let remainder = &authority[end + 1..];
        if remainder.is_empty() {
            return Ok((host, default_port));
        }
        let Some(port) = remainder.strip_prefix(':') else {
            return Err("WebSocket TLS URL authority is invalid.".to_string());
        };
        return Ok((host, parse_port(port)?));
    }
    let colon_count = authority.chars().filter(|ch| *ch == ':').count();
    if colon_count > 1 {
        return Err("WebSocket IPv6 hosts must be bracketed.".to_string());
    }
    if let Some((host, port)) = authority.rsplit_once(':') {
        if host.trim().is_empty() {
            return Err("WebSocket TLS URL must include a host.".to_string());
        }
        if port.chars().all(|ch| ch.is_ascii_digit()) {
            return Ok((host.to_string(), parse_port(port)?));
        }
        return Err("WebSocket TLS URL port must be numeric.".to_string());
    }
    Ok((authority.to_string(), default_port))
}

fn parse_port(raw: &str) -> Result<u16, String> {
    if raw.is_empty() {
        return Err("WebSocket TLS URL port must not be empty.".to_string());
    }
    let port = raw
        .parse::<u16>()
        .map_err(|_| "WebSocket TLS URL port must be between 1 and 65535.".to_string())?;
    if port == 0 {
        return Err("WebSocket TLS URL port must be between 1 and 65535.".to_string());
    }
    Ok(port)
}

fn validate_server_name(server_name: &str) -> Result<(), String> {
    ServerName::try_from(server_name.to_string())
        .map(|_| ())
        .map_err(|_| "WebSocket TLS server name is invalid.".to_string())
}

fn server_name_from_object(object: &Map<String, JsonValue>) -> Result<String, String> {
    let server_name = clean_text(
        object
            .get("server_name")
            .or_else(|| object.get("tls_server_name"))
            .or_else(|| object.get("host")),
    )
    .ok_or_else(|| "WebSocket TLS stream requires a server name.".to_string())?;
    let server_name = server_name.trim_matches(&['[', ']'][..]).to_string();
    validate_server_name(&server_name)?;
    Ok(server_name)
}

fn validate_certificate_verification(object: &Map<String, JsonValue>) -> Result<(), String> {
    if optional_bool(
        object
            .get("certificate_verification")
            .or_else(|| object.get("verify_certificate"))
            .or_else(|| object.get("verify_certificates")),
    ) == Some(false)
    {
        return Err(
            "WebSocket TLS stream does not allow disabling certificate verification.".to_string(),
        );
    }
    Ok(())
}

fn alpn_protocol_strings(object: &Map<String, JsonValue>) -> Vec<String> {
    alpn_protocol_bytes(object)
        .into_iter()
        .filter_map(|bytes| String::from_utf8(bytes).ok())
        .collect()
}

fn alpn_protocol_bytes(object: &Map<String, JsonValue>) -> Vec<Vec<u8>> {
    let Some(value) = object
        .get("alpn_protocols")
        .or_else(|| object.get("alpn"))
        .or_else(|| object.get("alpn_protocol"))
    else {
        return vec![DEFAULT_ALPN_PROTOCOL.as_bytes().to_vec()];
    };
    if let Some(text) = clean_text(Some(value)) {
        return split_csv(&text)
            .into_iter()
            .map(|item| item.into_bytes())
            .collect();
    }
    if let Some(values) = value.as_array() {
        let protocols = values
            .iter()
            .filter_map(|item| clean_text(Some(item)))
            .map(|item| item.into_bytes())
            .collect::<Vec<_>>();
        if protocols.is_empty() {
            vec![DEFAULT_ALPN_PROTOCOL.as_bytes().to_vec()]
        } else {
            protocols
        }
    } else {
        vec![DEFAULT_ALPN_PROTOCOL.as_bytes().to_vec()]
    }
}

fn split_csv(text: &str) -> Vec<String> {
    text.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn required_fd(object: &Map<String, JsonValue>) -> Result<NativeSocket, String> {
    let raw = optional_u64(
        object
            .get("websocket_fd")
            .or_else(|| object.get("fd"))
            .or_else(|| object.get("socket_fd")),
    )
    .ok_or_else(|| "WebSocket TLS stream requires websocket_fd.".to_string())?;
    native_socket_from_u64(raw)
        .map_err(|_| "WebSocket TLS stream fd is outside native socket range.".to_string())
}

fn required_session_id(object: &Map<String, JsonValue>) -> Result<String, String> {
    if let Some(session_id) = clean_text(
        object
            .get("tls_connection_id")
            .or_else(|| object.get("tls_session_id"))
            .or_else(|| object.get("session_id")),
    ) {
        return Ok(session_id);
    }
    let fd = required_fd(object)?;
    Ok(default_session_id(fd))
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
                "WebSocket TLS plaintext field `{key}` must be a byte array."
            ));
        }
    }
    for key in hex_keys {
        if let Some(value) = object.get(*key) {
            let Some(raw) = value.as_str() else {
                return Err(format!(
                    "WebSocket TLS plaintext field `{key}` must be a hex string."
                ));
            };
            return parse_hex_bytes(raw).map(Some).ok_or_else(|| {
                format!("WebSocket TLS plaintext field `{key}` must be a valid hex string.")
            });
        }
    }
    for key in text_keys {
        if let Some(value) = object.get(*key) {
            let Some(raw) = value.as_str() else {
                return Err(format!(
                    "WebSocket TLS plaintext field `{key}` must be a string."
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

fn tls_session_id(object: &Map<String, JsonValue>, fd: NativeSocket) -> String {
    clean_text(
        object
            .get("tls_connection_id")
            .or_else(|| object.get("tls_session_id"))
            .or_else(|| object.get("session_id")),
    )
    .unwrap_or_else(|| default_session_id(fd))
}

fn default_session_id(fd: NativeSocket) -> String {
    format!("websocket_tls_fd_{fd}")
}

fn fd_value(object: &Map<String, JsonValue>) -> JsonValue {
    optional_u64(
        object
            .get("websocket_fd")
            .or_else(|| object.get("fd"))
            .or_else(|| object.get("socket_fd")),
    )
    .map(JsonValue::from)
    .unwrap_or(JsonValue::Null)
}

fn configuration_error_payload(stage: &str, message: &str) -> JsonValue {
    base_payload(
        stage,
        "configuration_error",
        json!({
            "ok": false,
            "executed": false,
            "tls_required": true,
            "tls_established": false,
            "tls_handshaking": false,
            "tls_failed": false,
            "error": message,
            "execute_tls": false,
            "execute_upgrade_write": false,
            "should_register_readable": false,
            "should_register_writable": false,
            "should_write_upgrade_request": false,
            "actions": [
                {
                    "kind": "diagnose_websocket_tls_configuration_error",
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
        "websocket_tls_contract".to_string(),
        JsonValue::String(WEBSOCKET_TLS_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "websocket_tls_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert("python_tls_allowed".to_string(), JsonValue::Bool(false));
    object.insert(
        "python_websocket_tls_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "python_tls_event_loop_allowed".to_string(),
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
        .ok_or_else(|| "WebSocket TLS stream request must be an object.".to_string())
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

fn event_loop_token(object: &Map<String, JsonValue>) -> Option<u64> {
    optional_u64(
        object
            .get("event_loop_token")
            .or_else(|| object.get("token"))
            .or_else(|| object.get("registration_token")),
    )
}

fn interest(wants_read: bool, wants_write: bool) -> &'static str {
    match (wants_read, wants_write) {
        (true, true) => "read_write",
        (true, false) => "readable",
        (false, true) => "writable",
        (false, false) => "none",
    }
}

fn io_error_kind(err: &io::Error) -> String {
    format!("{:?}", err.kind()).to_ascii_lowercase()
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use ait_core::json_support::json;

    struct StartOnlyTlsExecutor;

    impl WebSocketTlsStartHandshakeExecutor for StartOnlyTlsExecutor {
        fn start_tls_handshake(&self, request: TlsStartRequest) -> TlsHandshakeOutcome {
            TlsHandshakeOutcome::handshaking(
                request.fd,
                request.session_id,
                request.server_name,
                false,
                true,
            )
        }
    }

    struct DriveOnlyTlsExecutor;

    impl WebSocketTlsDriveHandshakeExecutor for DriveOnlyTlsExecutor {
        fn drive_tls_handshake(&self, session_id: &str) -> TlsHandshakeOutcome {
            TlsHandshakeOutcome::established(17, session_id.to_string(), "example.com")
        }
    }

    struct CloseOnlyTlsExecutor;

    impl WebSocketTlsCloseSessionExecutor for CloseOnlyTlsExecutor {
        fn close_tls_session(&self, session_id: &str, _close_fd: bool) -> TlsHandshakeOutcome {
            TlsHandshakeOutcome::closed(session_id.to_string())
        }
    }

    struct ReadOnlyTlsExecutor;

    impl WebSocketTlsPlaintextReadExecutor for ReadOnlyTlsExecutor {
        fn read_tls_plaintext(&self, request: TlsReadRequest) -> TlsIoOutcome {
            assert_eq!(request.max_read_bytes, 8);
            assert_eq!(request.read_chunk_bytes, 4);
            TlsIoOutcome::read_chunk(
                17,
                request.session_id,
                "example.com",
                vec![b'o', b'k'],
                true,
                false,
            )
        }
    }

    struct WriteOnlyTlsExecutor;

    impl WebSocketTlsPlaintextWriteExecutor for WriteOnlyTlsExecutor {
        fn write_tls_plaintext(&self, request: TlsWriteRequest) -> TlsIoOutcome {
            TlsIoOutcome::write_complete(
                17,
                request.session_id,
                "example.com",
                request.write_bytes,
                0,
                2,
                true,
            )
        }
    }

    #[test]
    fn websocket_tls_stage_helpers_accept_single_capability_executors() {
        let start_request = json!({
            "websocket_fd": 17,
            "server_name": "example.com",
            "tls_connection_id": "tls-start",
            "event_loop_token": 71
        });
        let start = start_tls_handshake(&StartOnlyTlsExecutor, start_request.as_object().unwrap());
        assert_eq!(start["websocket_tls_state"], "tls_handshake_want_write");
        assert_eq!(
            start["actions"][0]["kind"],
            "register_websocket_tls_handshake"
        );

        let drive_request = json!({
            "tls_connection_id": "tls-drive",
            "server_name": "example.com"
        });
        let drive = drive_tls_handshake(&DriveOnlyTlsExecutor, drive_request.as_object().unwrap());
        assert_eq!(drive["websocket_tls_state"], "tls_established");
        assert_eq!(drive["tls_established"], true);

        let close_request = json!({
            "tls_connection_id": "tls-close",
            "close_fd": false
        });
        let close = close_tls_session(&CloseOnlyTlsExecutor, close_request.as_object().unwrap());
        assert_eq!(close["websocket_tls_state"], "tls_session_closed");

        let read_request = json!({
            "tls_connection_id": "tls-read",
            "max_read_bytes": 8,
            "read_chunk_bytes": 4,
            "event_loop_token": 72
        });
        let read = read_tls_plaintext(&ReadOnlyTlsExecutor, read_request.as_object().unwrap());
        assert_eq!(read["websocket_tls_state"], "tls_read_chunk");
        assert_eq!(read["read_hex"], "6f6b");

        let write_request = json!({
            "tls_connection_id": "tls-write",
            "write_text": "ok",
            "event_loop_token": 73
        });
        let write = write_tls_plaintext(&WriteOnlyTlsExecutor, write_request.as_object().unwrap());
        assert_eq!(write["websocket_tls_state"], "tls_write_complete");
        assert_eq!(write["written_hex"], "6f6b");
    }
}

#[derive(Debug, Clone)]
struct TlsTarget {
    secure: bool,
    url: String,
    host: String,
    port: u16,
    server_name: String,
}
