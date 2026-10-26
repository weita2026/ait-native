use super::{
    agent_transport_websocket_tls_execute_json, plan_with_websocket_tls_executor,
    TlsHandshakeOutcome, TlsIoOutcome, TlsReadRequest, TlsStartRequest, TlsWriteRequest,
    WebSocketTlsCloseSessionExecutor, WebSocketTlsDriveHandshakeExecutor,
    WebSocketTlsPlaintextReadExecutor, WebSocketTlsPlaintextWriteExecutor,
    WebSocketTlsStartHandshakeExecutor,
};
use crate::platform::tcp_stream_into_native_socket;
use ait_core::json_support::json;
use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

const SAMPLE_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const SAMPLE_ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

#[test]
fn websocket_tls_plan_parses_wss_target_without_python_tls() {
    let planned = agent_transport_websocket_tls_execute_json(&json!({
        "stage": "plan",
        "url": "wss://gateway.discord.gg/?v=10&encoding=json",
    }))
    .unwrap();

    assert_eq!(planned["stage"], "plan");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_transport_websocket_tls_stream_boundary"
    );
    assert_eq!(
        planned["websocket_tls_contract"],
        "ait_agent_core.transport.WebSocketTlsStream.v1"
    );
    assert_eq!(planned["websocket_tls_state"], "tls_planned");
    assert_eq!(planned["tls_required"], true);
    assert_eq!(planned["server_name"], "gateway.discord.gg");
    assert_eq!(planned["alpn_protocols"], json!(["http/1.1"]));
    assert_eq!(planned["python_tls_allowed"], false);
    assert_eq!(planned["execute_tls"], false);
    assert_eq!(planned["execute_upgrade_write"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "start_websocket_tls_handshake"
    );
    assert_eq!(planned["actions"][0]["python_tls_allowed"], false);
}

#[test]
fn websocket_tls_plan_skips_plain_ws_targets() {
    let planned = agent_transport_websocket_tls_execute_json(&json!({
        "stage": "plan",
        "url": "ws://localhost:8080/socket",
    }))
    .unwrap();

    assert_eq!(planned["websocket_tls_state"], "tls_not_required");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["tls_required"], false);
    assert_eq!(planned["execute_tls"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "skip_websocket_tls_for_plain_ws"
    );
}

#[test]
fn websocket_tls_start_with_substitute_executor_emits_write_interest_registration() {
    let planned = plan_with_websocket_tls_executor(
        &SubstituteTlsExecutor,
        &json!({
            "stage": "start_tls_handshake",
            "websocket_fd": 99,
            "server_name": "gateway.discord.gg",
            "tls_connection_id": "tls-test-1",
            "event_loop_token": 17,
            "worker_key": "worker-a",
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_tls_state"], "tls_handshake_want_write");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["tls_handshaking"], true);
    assert_eq!(planned["should_register_writable"], true);
    assert_eq!(planned["interest"], "writable");
    assert_eq!(
        planned["actions"][0]["kind"],
        "register_websocket_tls_handshake"
    );
    assert_eq!(planned["actions"][0]["event_loop_token"], 17);
    assert_eq!(planned["actions"][0]["execute_registration"], false);
    assert_eq!(planned["python_tls_event_loop_allowed"], false);
}

#[test]
fn websocket_tls_established_state_plans_tls_upgrade_write_without_execution() {
    let planned = plan_with_websocket_tls_executor(
        &SubstituteTlsExecutor,
        &json!({
            "stage": "resume",
            "websocket_fd": 99,
            "tls_connection_id": "tls-test-1",
            "url": "wss://gateway.discord.gg/?v=10&encoding=json",
            "sec_websocket_key": SAMPLE_KEY,
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_tls_state"], "tls_established");
    assert_eq!(planned["tls_established"], true);
    assert_eq!(planned["should_write_upgrade_request"], true);
    assert_eq!(planned["execute_upgrade_write"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "write_websocket_tls_upgrade_request"
    );
    assert_eq!(planned["actions"][0]["requires_tls_session"], true);
    assert_eq!(planned["actions"][0]["execute_write"], false);
    assert_eq!(
        planned["actions"][0]["expected_sec_websocket_accept"],
        SAMPLE_ACCEPT
    );
    assert_eq!(
        planned["actions"][1]["kind"],
        "await_websocket_tls_upgrade_response"
    );
}

#[test]
fn websocket_tls_failure_diagnoses_without_python_fallback() {
    let planned = plan_with_websocket_tls_executor(
        &FailingTlsExecutor,
        &json!({
            "stage": "resume",
            "tls_connection_id": "tls-missing",
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_tls_state"], "tls_handshake_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["tls_failed"], true);
    assert_eq!(planned["python_tls_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_tls_handshake_error"
    );
    assert_eq!(planned["actions"][0]["error_kind"], "missing_session");
}

#[test]
fn websocket_tls_read_delivers_plaintext_chunk_without_python_io() {
    let planned = plan_with_websocket_tls_executor(
        &SubstituteTlsExecutor,
        &json!({
            "stage": "read_tls",
            "tls_connection_id": "tls-test-1",
            "event_loop_token": 21,
            "max_read_bytes": 1024,
            "read_chunk_bytes": 64,
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_tls_state"], "tls_read_chunk");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["read_hex"], "81026f6b");
    assert_eq!(planned["python_tls_io_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "deliver_websocket_tls_read_chunk"
    );
    assert_eq!(planned["actions"][0]["event_loop_token"], 21);
    assert_eq!(planned["actions"][0]["read_byte_count"], 4);
}

#[test]
fn websocket_tls_write_sends_plaintext_without_python_io() {
    let planned = plan_with_websocket_tls_executor(
        &SubstituteTlsExecutor,
        &json!({
            "stage": "write_tls",
            "tls_connection_id": "tls-test-1",
            "event_loop_token": 22,
            "write_text": "ok",
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_tls_state"], "tls_write_complete");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["written_hex"], "6f6b");
    assert_eq!(planned["write_complete"], true);
    assert_eq!(planned["python_tls_io_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "mark_websocket_tls_write_complete"
    );
    assert_eq!(planned["actions"][0]["event_loop_token"], 22);
}

#[test]
fn websocket_tls_rejects_disabled_certificate_verification() {
    let planned = agent_transport_websocket_tls_execute_json(&json!({
        "stage": "start_tls_handshake",
        "websocket_fd": 99,
        "server_name": "gateway.discord.gg",
        "verify_certificate": false,
    }))
    .unwrap();

    assert_eq!(planned["websocket_tls_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "WebSocket TLS stream does not allow disabling certificate verification."
    );
}

#[test]
fn websocket_tls_default_executor_starts_nonblocking_client_hello_without_python() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut buffer = [0u8; 1024];
        let received = stream.read(&mut buffer).unwrap_or(0);
        tx.send(received).unwrap();
        thread::sleep(Duration::from_millis(200));
    });

    let client = TcpStream::connect(address).unwrap();
    let fd = tcp_stream_into_native_socket(client);
    let session_id = format!("tls-local-{fd}");
    let planned = agent_transport_websocket_tls_execute_json(&json!({
        "stage": "start_tls_handshake",
        "websocket_fd": fd,
        "server_name": "example.com",
        "tls_connection_id": session_id,
        "event_loop_token": 41,
    }))
    .unwrap();

    assert_eq!(planned["ok"], true);
    assert_eq!(planned["tls_handshaking"], true);
    assert_eq!(planned["python_tls_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "register_websocket_tls_handshake"
    );
    assert_eq!(planned["actions"][0]["event_loop_token"], 41);
    let received = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(received > 0, "server should receive a TLS ClientHello");

    let closed = agent_transport_websocket_tls_execute_json(&json!({
        "stage": "close_tls",
        "tls_connection_id": session_id,
    }))
    .unwrap();
    assert_eq!(closed["websocket_tls_state"], "tls_session_closed");
    assert_eq!(closed["ok"], true);
    server.join().unwrap();
}

struct SubstituteTlsExecutor;

impl WebSocketTlsStartHandshakeExecutor for SubstituteTlsExecutor {
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

impl WebSocketTlsDriveHandshakeExecutor for SubstituteTlsExecutor {
    fn drive_tls_handshake(&self, session_id: &str) -> TlsHandshakeOutcome {
        TlsHandshakeOutcome::established(99, session_id.to_string(), "gateway.discord.gg")
    }
}

impl WebSocketTlsCloseSessionExecutor for SubstituteTlsExecutor {
    fn close_tls_session(&self, session_id: &str, _close_fd: bool) -> TlsHandshakeOutcome {
        TlsHandshakeOutcome::closed(session_id.to_string())
    }
}

impl WebSocketTlsPlaintextReadExecutor for SubstituteTlsExecutor {
    fn read_tls_plaintext(&self, request: TlsReadRequest) -> TlsIoOutcome {
        assert_eq!(request.max_read_bytes, 1024);
        assert_eq!(request.read_chunk_bytes, 64);
        TlsIoOutcome::read_chunk(
            99,
            request.session_id,
            "gateway.discord.gg",
            vec![0x81, 0x02, b'o', b'k'],
            true,
            false,
        )
    }
}

impl WebSocketTlsPlaintextWriteExecutor for SubstituteTlsExecutor {
    fn write_tls_plaintext(&self, request: TlsWriteRequest) -> TlsIoOutcome {
        TlsIoOutcome::write_complete(
            99,
            request.session_id,
            "gateway.discord.gg",
            request.write_bytes,
            0,
            7,
            true,
        )
    }
}

struct FailingTlsExecutor;

impl WebSocketTlsStartHandshakeExecutor for FailingTlsExecutor {
    fn start_tls_handshake(&self, request: TlsStartRequest) -> TlsHandshakeOutcome {
        TlsHandshakeOutcome::failed(
            Some(request.fd),
            Some(request.session_id),
            Some(request.server_name),
            "tls_error",
            "handshake failed",
        )
    }
}

impl WebSocketTlsDriveHandshakeExecutor for FailingTlsExecutor {
    fn drive_tls_handshake(&self, session_id: &str) -> TlsHandshakeOutcome {
        TlsHandshakeOutcome::failed(
            None,
            Some(session_id.to_string()),
            None,
            "missing_session",
            "WebSocket TLS session is not registered.",
        )
    }
}

impl WebSocketTlsCloseSessionExecutor for FailingTlsExecutor {
    fn close_tls_session(&self, session_id: &str, _close_fd: bool) -> TlsHandshakeOutcome {
        TlsHandshakeOutcome::failed(
            None,
            Some(session_id.to_string()),
            None,
            "missing_session",
            "WebSocket TLS session is not registered.",
        )
    }
}

impl WebSocketTlsPlaintextReadExecutor for FailingTlsExecutor {}

impl WebSocketTlsPlaintextWriteExecutor for FailingTlsExecutor {}
