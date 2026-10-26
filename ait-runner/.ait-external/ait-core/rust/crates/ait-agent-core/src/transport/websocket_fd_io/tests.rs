use super::agent_transport_websocket_fd_io_execute_json;
use crate::platform::{native_socket_to_u64, tcp_stream_native_socket};
use ait_core::json_support::json;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};

fn connected_pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let accept = thread::spawn(move || listener.accept().unwrap().0);
    let client = TcpStream::connect(address).unwrap();
    (client, accept.join().unwrap())
}

fn socket_json(stream: &TcpStream) -> u64 {
    native_socket_to_u64(tcp_stream_native_socket(stream))
}

#[test]
fn websocket_fd_io_readable_fd_drains_available_bytes_without_python_fallback() {
    let (reader, mut writer) = connected_pair();
    writer.write_all(&[0x81, 0x02, b'h', b'i']).unwrap();

    let deadline = Instant::now() + Duration::from_secs(1);
    let planned = loop {
        let planned = agent_transport_websocket_fd_io_execute_json(&json!({
            "stage": "read_ready_fd",
            "backend": "linux_epoll",
            "shard_index": 2,
            "event_loop_token": 17,
            "websocket_fd": socket_json(&reader),
            "max_read_bytes": 1024,
            "read_chunk_bytes": 64
        }))
        .unwrap();
        if planned["bytes_read"].as_u64().unwrap_or_default() > 0 {
            break planned;
        }
        assert!(Instant::now() < deadline, "TCP fixture data did not arrive");
        thread::yield_now();
    };

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_transport_websocket_fd_io_boundary"
    );
    assert_eq!(
        planned["websocket_fd_io_contract"],
        "ait_agent_core.transport.WebSocketFdIo.v1"
    );
    assert_eq!(planned["websocket_fd_io_state"], "read_chunk");
    assert_eq!(planned["backend"], "linux_epoll");
    assert_eq!(planned["shard_index"], 2);
    assert_eq!(planned["event_loop_token"], 17);
    assert_eq!(planned["bytes_read"], 4);
    assert_eq!(planned["read_hex"], "81026869");
    assert_eq!(planned["would_block"], true);
    assert_eq!(planned["python_websocket_fd_io_allowed"], false);
    assert_eq!(planned["python_socket_io_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "deliver_websocket_fd_read_chunk"
    );
    assert_eq!(
        planned["actions"][1]["kind"],
        "mark_websocket_fd_would_block"
    );
}

#[test]
fn websocket_fd_io_readable_fd_reports_peer_eof() {
    let (reader, writer) = connected_pair();
    drop(writer);

    let deadline = Instant::now() + Duration::from_secs(1);
    let planned = loop {
        let planned = agent_transport_websocket_fd_io_execute_json(&json!({
            "stage": "readable_fd",
            "websocket_fd": socket_json(&reader)
        }))
        .unwrap();
        if planned["websocket_fd_io_state"] != "would_block" {
            break planned;
        }
        assert!(Instant::now() < deadline, "TCP fixture EOF did not arrive");
        thread::yield_now();
    };

    assert_eq!(planned["websocket_fd_io_state"], "peer_eof");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["bytes_read"], 0);
    assert_eq!(planned["read_eof"], true);
    assert_eq!(planned["actions"][0]["kind"], "mark_websocket_fd_peer_eof");
}

#[test]
fn websocket_fd_io_readable_fd_reports_would_block_without_bytes() {
    let (reader, _writer) = connected_pair();

    let planned = agent_transport_websocket_fd_io_execute_json(&json!({
        "stage": "read_ready",
        "websocket_fd": socket_json(&reader)
    }))
    .unwrap();

    assert_eq!(planned["websocket_fd_io_state"], "would_block");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["bytes_read"], 0);
    assert_eq!(planned["would_block"], true);
    assert_eq!(
        planned["actions"][0]["kind"],
        "mark_websocket_fd_would_block"
    );
}

#[test]
fn websocket_fd_io_write_frame_bytes_to_fd() {
    let (mut reader, writer) = connected_pair();

    let planned = agent_transport_websocket_fd_io_execute_json(&json!({
        "stage": "write_frame",
        "websocket_fd": socket_json(&writer),
        "event_loop_token": 19,
        "write_bytes": [0x81, 0x02, 0x6f, 0x6b]
    }))
    .unwrap();

    let mut received = [0u8; 4];
    reader.read_exact(&mut received).unwrap();

    assert_eq!(planned["websocket_fd_io_state"], "write_complete");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["write_complete"], true);
    assert_eq!(planned["bytes_written"], 4);
    assert_eq!(planned["remaining_write_byte_count"], 0);
    assert_eq!(
        planned["actions"][0]["kind"],
        "mark_websocket_fd_write_complete"
    );
    assert_eq!(received, [0x81, 0x02, 0x6f, 0x6b]);
}

#[test]
fn websocket_fd_io_write_limit_returns_partial_carry() {
    let (mut reader, writer) = connected_pair();

    let planned = agent_transport_websocket_fd_io_execute_json(&json!({
        "stage": "write_bytes",
        "websocket_fd": socket_json(&writer),
        "write_hex": "0102030405",
        "max_write_bytes": 2
    }))
    .unwrap();

    let mut received = [0u8; 2];
    reader.read_exact(&mut received).unwrap();

    assert_eq!(planned["websocket_fd_io_state"], "partial_write");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["complete"], false);
    assert_eq!(planned["bytes_written"], 2);
    assert_eq!(planned["write_limit_reached"], true);
    assert_eq!(planned["remaining_write_hex"], "030405");
    assert_eq!(
        planned["actions"][0]["kind"],
        "queue_websocket_fd_write_retry"
    );
    assert_eq!(received, [1, 2]);
}

#[test]
fn websocket_fd_io_rejects_missing_fd_without_python_fallback() {
    let planned = agent_transport_websocket_fd_io_execute_json(&json!({
        "stage": "read_ready_fd"
    }))
    .unwrap();

    assert_eq!(planned["websocket_fd_io_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["python_socket_io_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_fd_io_configuration_error"
    );
}

#[test]
fn websocket_fd_io_rejects_invalid_hex_without_python_fallback() {
    let planned = agent_transport_websocket_fd_io_execute_json(&json!({
        "stage": "write_frame",
        "websocket_fd": 0,
        "write_hex": "abc"
    }))
    .unwrap();

    assert_eq!(planned["websocket_fd_io_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["python_websocket_fd_io_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
}
