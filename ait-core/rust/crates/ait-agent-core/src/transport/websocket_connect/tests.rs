use super::{
    agent_transport_websocket_connect_execute_json, plan_with_websocket_connect_executor,
    TcpConnectOutcome, WebSocketConnectFinishExecutor, WebSocketConnectStartExecutor,
};
use crate::platform::{
    close_native_socket, native_socket_from_u64, native_socket_to_u64, tcp_stream_native_socket,
    NativeSocket,
};
use ait_core::json_support::json;
use std::net::{SocketAddr, TcpListener, TcpStream};

const SAMPLE_KEY: &str = "dGhlIHNhbXBsZSBub25jZQ==";
const SAMPLE_ACCEPT: &str = "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=";

#[test]
fn websocket_connect_plan_parses_wss_target_without_dns_or_python_connect() {
    let planned = agent_transport_websocket_connect_execute_json(&json!({
        "stage": "plan",
        "url": "wss://gateway.discord.gg/?v=10&encoding=json",
    }))
    .unwrap();

    assert_eq!(planned["stage"], "plan");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_transport_websocket_tcp_connect_boundary"
    );
    assert_eq!(
        planned["websocket_connect_contract"],
        "ait_agent_core.transport.WebSocketTcpConnect.v1"
    );
    assert_eq!(planned["websocket_connect_state"], "connect_planned");
    assert_eq!(planned["python_websocket_connect_allowed"], false);
    assert_eq!(planned["python_socket_connect_allowed"], false);
    assert_eq!(planned["execute_dns"], false);
    assert_eq!(planned["execute_connect"], false);
    assert_eq!(planned["execute_tls"], false);
    assert_eq!(planned["secure"], true);
    assert_eq!(planned["host"], "gateway.discord.gg");
    assert_eq!(planned["port"], 443);
    assert_eq!(planned["path_and_query"], "/?v=10&encoding=json");
    assert_eq!(planned["actions"][0]["kind"], "resolve_websocket_host");
    assert_eq!(planned["actions"][0]["blocking_dns_allowed"], false);
    assert_eq!(planned["actions"][1]["kind"], "open_websocket_tcp_connect");
    assert_eq!(planned["actions"][1]["requires_resolved_address"], true);
    assert_eq!(
        planned["actions"][2]["kind"],
        "start_websocket_tls_handshake"
    );
    assert_eq!(planned["actions"][2]["python_tls_allowed"], false);
}

#[test]
fn websocket_connect_start_opens_nonblocking_fd_and_emits_event_loop_registration() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();

    let planned = agent_transport_websocket_connect_execute_json(&json!({
        "stage": "start_tcp_connect",
        "url": format!("ws://127.0.0.1:{}/socket", address.port()),
        "socket_address": address.to_string(),
        "event_loop_token": 77,
        "worker_key": "worker-a",
        "sec_websocket_key": SAMPLE_KEY,
    }))
    .unwrap();

    assert_eq!(planned["ok"], true);
    assert_eq!(planned["execute_connect"], true);
    assert_eq!(planned["python_websocket_connect_allowed"], false);
    let fd = planned["websocket_fd"].as_u64().unwrap();
    assert!(matches!(
        planned["websocket_connect_state"].as_str().unwrap(),
        "tcp_connecting" | "tcp_connected"
    ));
    if planned["websocket_connect_state"] == "tcp_connecting" {
        assert_eq!(planned["connect_in_progress"], true);
        assert_eq!(planned["should_register_writable"], true);
        assert_eq!(
            planned["actions"][0]["kind"],
            "register_websocket_read_write"
        );
        assert_eq!(planned["actions"][0]["event_loop_token"], 77);
        assert_eq!(planned["actions"][0]["execute_registration"], false);
    } else {
        assert_eq!(planned["connected"], true);
        assert_eq!(planned["should_write_upgrade_request"], true);
        assert_eq!(
            planned["actions"][0]["kind"],
            "write_websocket_upgrade_request"
        );
        assert_eq!(
            planned["actions"][0]["expected_sec_websocket_accept"],
            SAMPLE_ACCEPT
        );
        assert_eq!(planned["actions"][0]["execute_write"], false);
    }

    close_native_socket(native_socket_from_u64(fd).unwrap()).unwrap();
}

#[test]
fn websocket_connect_finish_checks_so_error_and_plans_plain_upgrade_write() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let stream = TcpStream::connect(address).unwrap();

    let planned = agent_transport_websocket_connect_execute_json(&json!({
        "stage": "finish_tcp_connect",
        "url": format!("ws://127.0.0.1:{}/socket", address.port()),
        "socket_address": address.to_string(),
        "websocket_fd": native_socket_to_u64(tcp_stream_native_socket(&stream)),
        "sec_websocket_key": SAMPLE_KEY,
    }))
    .unwrap();

    assert_eq!(planned["websocket_connect_state"], "tcp_connected");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["connected"], true);
    assert_eq!(planned["should_write_upgrade_request"], true);
    assert_eq!(planned["should_start_tls"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "write_websocket_upgrade_request"
    );
    assert_eq!(
        planned["actions"][0]["expected_sec_websocket_accept"],
        SAMPLE_ACCEPT
    );
    assert_eq!(
        planned["actions"][1]["kind"],
        "await_websocket_upgrade_response"
    );
}

#[test]
fn websocket_connect_finish_for_wss_requires_rust_tls_boundary_without_python_fallback() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let stream = TcpStream::connect(address).unwrap();

    let planned = agent_transport_websocket_connect_execute_json(&json!({
        "stage": "finish_tcp_connect",
        "url": format!("wss://127.0.0.1:{}/socket", address.port()),
        "socket_address": address.to_string(),
        "websocket_fd": native_socket_to_u64(tcp_stream_native_socket(&stream)),
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_connect_state"],
        "tcp_connected_tls_required"
    );
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["connected"], true);
    assert_eq!(planned["tls_required"], true);
    assert_eq!(planned["should_start_tls"], true);
    assert_eq!(planned["should_write_upgrade_request"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "start_websocket_tls_handshake"
    );
    assert_eq!(planned["actions"][0]["execute_tls"], false);
    assert_eq!(planned["actions"][0]["python_tls_allowed"], false);
}

#[cfg(unix)]
#[test]
fn websocket_connect_finish_reports_socket_errors_without_python_fallback() {
    let mut pipe_fds = [0; 2];
    let pipe_result = unsafe { libc::pipe(pipe_fds.as_mut_ptr()) };
    assert_eq!(pipe_result, 0);

    let planned = agent_transport_websocket_connect_execute_json(&json!({
        "stage": "finish_tcp_connect",
        "websocket_fd": pipe_fds[0],
        "close_on_error": false,
    }))
    .unwrap();

    assert_eq!(planned["websocket_connect_state"], "tcp_connect_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["connect_failed"], true);
    assert_eq!(planned["python_websocket_connect_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_tcp_connect_error"
    );

    unsafe {
        libc::close(pipe_fds[0]);
        libc::close(pipe_fds[1]);
    }
}

#[test]
fn websocket_connect_requires_resolved_socket_address_for_execution() {
    let planned = agent_transport_websocket_connect_execute_json(&json!({
        "stage": "start_tcp_connect",
        "url": "ws://example.test/socket",
    }))
    .unwrap();

    assert_eq!(planned["websocket_connect_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "WebSocket TCP connect requires a resolved socket address."
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_tcp_connect_configuration_error"
    );
}

#[test]
fn websocket_connect_bound_entrypoint_accepts_substitute_executor() {
    struct SubstituteExecutor;

    impl WebSocketConnectStartExecutor for SubstituteExecutor {
        fn start_tcp_connect(&self, _address: SocketAddr) -> TcpConnectOutcome {
            TcpConnectOutcome::in_progress(99 as NativeSocket, 1, "operation in progress")
        }
    }

    impl WebSocketConnectFinishExecutor for SubstituteExecutor {
        fn finish_tcp_connect(&self, fd: NativeSocket) -> TcpConnectOutcome {
            TcpConnectOutcome::connected(fd)
        }
    }

    let planned = plan_with_websocket_connect_executor(
        &SubstituteExecutor,
        &json!({
            "stage": "start_tcp_connect",
            "socket_address": "127.0.0.1:12345",
            "event_loop_token": 12,
        }),
    )
    .unwrap();

    assert_eq!(planned["websocket_connect_state"], "tcp_connecting");
    assert_eq!(planned["websocket_fd"], 99);
    assert_eq!(
        planned["actions"][0]["kind"],
        "register_websocket_read_write"
    );

    let finished = plan_with_websocket_connect_executor(
        &SubstituteExecutor,
        &json!({
            "stage": "finish_tcp_connect",
            "websocket_fd": 99,
        }),
    )
    .unwrap();

    assert_eq!(finished["websocket_connect_state"], "tcp_connected");
    assert_eq!(finished["websocket_fd"], 99);
}
