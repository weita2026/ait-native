use super::agent_websocket_event_loop_turn_plan_json;
use crate::platform::{connected_tcp_pair, tcp_stream_native_socket};
use ait_core::json_support::{json, JsonValue};
use std::io::{Read, Write};
use std::time::Duration;

#[test]
fn websocket_turn_dispatches_slack_text_payload_to_socket_mode_runtime() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "slack",
        "backend": "linux_epoll",
        "shard_index": 2,
        "event_loop_token": 9,
        "websocket_fd": 44,
        "read_bytes": server_text_frame(r#"{"type":"hello","num_connections":1}"#),
    }))
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_websocket_event_loop_turn_boundary"
    );
    assert_eq!(
        planned["websocket_turn_contract"],
        "ait_agent_core.event_loop.WebSocketTurn.v1"
    );
    assert_eq!(planned["websocket_turn_state"], "payloads_dispatched");
    assert_eq!(planned["transport"], "slack");
    assert_eq!(planned["backend"], "linux_epoll");
    assert_eq!(planned["event_loop_token"], 9);
    assert_eq!(planned["python_websocket_event_loop_allowed"], false);
    assert_eq!(planned["processed_text_payload_count"], 1);
    assert_eq!(
        planned["runtime_plans"][0]["socket_mode_runtime_state"],
        "ready"
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "dispatch_slack_socket_mode_payload"
    );
    assert_eq!(planned["actions"][1]["kind"], "mark_socket_mode_ready");
    assert_eq!(
        planned["actions"][2]["kind"],
        "keep_websocket_readable_registered"
    );
}

#[test]
fn websocket_turn_readable_reads_fd_and_dispatches_payload() {
    let (client, mut peer) = connected_tcp_pair();
    peer.write_all(&server_text_frame(
        r#"{"type":"hello","num_connections":1}"#,
    ))
    .unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut available = [0_u8; 1];
    assert_eq!(client.peek(&mut available).unwrap(), 1);

    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "stage": "readable_turn",
        "transport": "slack",
        "backend": "portable_poll",
        "shard_index": 1,
        "event_loop_token": 31,
        "websocket_fd": tcp_stream_native_socket(&client),
        "max_read_bytes": 1024,
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "payloads_dispatched");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["read_source"], "fd_io");
    assert_eq!(planned["read_byte_count"], 38);
    assert_eq!(
        planned["fd_io_result"]["websocket_fd_io_state"],
        "read_chunk"
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "deliver_websocket_fd_read_chunk"
    );
    assert_eq!(
        planned["actions"][2]["kind"],
        "dispatch_slack_socket_mode_payload"
    );
    assert_eq!(
        planned["actions"][4]["kind"],
        "keep_websocket_readable_registered"
    );
    assert_eq!(planned["python_websocket_turn_allowed"], false);
}

#[test]
fn websocket_turn_readable_would_block_stays_registered_without_python_read() {
    let (client, _peer) = connected_tcp_pair();

    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "stage": "readable_turn",
        "transport": "discord",
        "event_loop_token": 32,
        "websocket_fd": tcp_stream_native_socket(&client),
        "max_read_bytes": 1024,
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "read_would_block");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["read_source"], "fd_io");
    assert_eq!(planned["read_would_block"], true);
    assert_eq!(planned["read_byte_count"], 0);
    assert_eq!(
        planned["fd_io_result"]["websocket_fd_io_state"],
        "would_block"
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "mark_websocket_fd_would_block"
    );
    assert_eq!(
        planned["actions"][1]["kind"],
        "keep_websocket_readable_registered"
    );
}

#[test]
fn websocket_turn_readable_without_fd_fails_closed_before_python_fallback() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "stage": "readable_turn",
        "transport": "slack",
        "event_loop_token": 33,
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["fd_io_result"], JsonValue::Null);
    assert_eq!(planned["should_close_websocket"], true);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(
        planned["error"],
        "WebSocket readable turn requires websocket_fd when no read bytes are supplied."
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_readable_turn_configuration_error"
    );
    assert_eq!(planned["actions"][1]["kind"], "close_websocket");
    assert_eq!(
        planned["actions"][2]["kind"],
        "unregister_websocket_readable"
    );
    assert_eq!(planned["actions"][3]["kind"], "reconnect_socket_mode");
}

#[test]
fn websocket_turn_dispatches_discord_text_payload_to_gateway_runtime() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "discord",
        "event_loop_registration": {
            "backend": "linux_epoll",
            "shard_index": 1,
            "token": 22,
            "fd": 45
        },
        "read_bytes": server_text_frame(r#"{"op":11}"#),
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "payloads_dispatched");
    assert_eq!(planned["transport"], "discord");
    assert_eq!(
        planned["runtime_plans"][0]["gateway_runtime_state"],
        "heartbeat_acknowledged"
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "dispatch_discord_gateway_payload"
    );
    assert_eq!(planned["actions"][1]["kind"], "mark_heartbeat_acknowledged");
    assert_eq!(
        planned["actions"][2]["kind"],
        "keep_websocket_readable_registered"
    );
}

#[test]
fn websocket_turn_preserves_partial_stream_buffer_for_next_read() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "slack",
        "event_loop_token": 3,
        "websocket_fd": 7,
        "buffer_hex": "8105",
        "read_hex": "6865",
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "partial_frame");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["remaining_buffer_hex"], "81056865");
    assert_eq!(planned["needed_additional_bytes"], 3);
    assert_eq!(
        planned["actions"][0]["kind"],
        "keep_websocket_readable_registered"
    );
}

#[test]
fn websocket_turn_writes_ping_pong_before_dispatching_payload() {
    let mut read_bytes = vec![137, 2, b'o', b'k'];
    read_bytes.extend(server_text_frame(r#"{"type":"hello"}"#));
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "slack",
        "event_loop_token": 5,
        "websocket_fd": 8,
        "read_bytes": read_bytes,
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "payloads_dispatched");
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][0]["opcode"], "pong");
    assert_eq!(
        planned["actions"][1]["kind"],
        "dispatch_slack_socket_mode_payload"
    );
    assert_eq!(planned["actions"][2]["kind"], "mark_socket_mode_ready");
}

#[test]
fn websocket_turn_encodes_discord_hello_identify_as_masked_text_frame() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "discord",
        "event_loop_token": 6,
        "websocket_fd": 8,
        "bot_token": "bot-secret",
        "platform": "linux",
        "read_bytes": server_text_frame(r#"{"op":10,"d":{"heartbeat_interval":45000}}"#),
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "payloads_dispatched");
    assert_eq!(
        planned["runtime_plans"][0]["gateway_runtime_state"],
        "ready_for_gateway_loop"
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "dispatch_discord_gateway_payload"
    );
    assert_eq!(planned["actions"][1]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][1]["opcode"], "text");
    assert_eq!(
        planned["actions"][1]["runtime_action_kind"],
        "send_gateway_identify"
    );
    assert!(
        planned["actions"][1]["frame_bytes"]
            .as_array()
            .unwrap()
            .len()
            > 6
    );
    assert_eq!(planned["actions"][2]["kind"], "send_gateway_identify");
}

#[test]
fn websocket_turn_protocol_error_fails_closed_and_reconnects() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "discord",
        "event_loop_token": 7,
        "websocket_fd": 9,
        "read_hex": "8300",
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "failed_closed");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["should_unregister"], true);
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][1]["kind"], "close_websocket");
    assert_eq!(
        planned["actions"][2]["kind"],
        "unregister_websocket_readable"
    );
    assert_eq!(planned["actions"][3]["kind"], "reconnect_gateway");
}

#[test]
fn websocket_turn_hangup_unregisters_and_reconnects_without_stream_decode() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "slack",
        "event": {
            "token": 11,
            "fd": 12,
            "hangup": true
        }
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "hangup_reconnect");
    assert_eq!(planned["stream_plan"], JsonValue::Null);
    assert_eq!(planned["actions"][0]["kind"], "close_websocket");
    assert_eq!(
        planned["actions"][1]["kind"],
        "unregister_websocket_readable"
    );
    assert_eq!(planned["actions"][2]["kind"], "reconnect_socket_mode");
}

#[test]
fn websocket_turn_missing_transport_context_closes_without_python_fallback() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "event_loop_token": 12,
        "websocket_fd": 13,
        "read_bytes": server_text_frame(r#"{"type":"hello"}"#),
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "transport_context_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["transport"], "unknown");
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(
        planned["error"],
        "WebSocket event-loop turn requires transport `slack` or `discord`."
    );
    assert_eq!(planned["actions"][0]["kind"], "close_websocket");
    assert_eq!(
        planned["actions"][1]["kind"],
        "unregister_websocket_readable"
    );
}

#[test]
fn websocket_turn_rejects_binary_payloads_fail_closed() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "slack",
        "event_loop_token": 13,
        "websocket_fd": 14,
        "read_bytes": [130, 1, 7],
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "failed_closed");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "WebSocket binary payloads are not supported by ait-agent Slack Socket Mode or Discord gateway turn planners."
    );
    assert_eq!(planned["actions"][0]["kind"], "close_websocket");
    assert_eq!(
        planned["actions"][1]["kind"],
        "unregister_websocket_readable"
    );
    assert_eq!(planned["actions"][2]["kind"], "reconnect_socket_mode");
}

#[test]
fn websocket_turn_missing_runtime_write_mask_key_fails_closed() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "discord",
        "event_loop_token": 17,
        "websocket_fd": 18,
        "bot_token": "bot-secret",
        "platform": "linux",
        "read_bytes": server_text_frame(r#"{"op":10,"d":{"heartbeat_interval":45000}}"#),
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "failed_closed");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "WebSocket runtime action `send_gateway_identify` requires an explicit 4-byte mask_key for outbound frame planning."
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "dispatch_discord_gateway_payload"
    );
    assert_eq!(planned["actions"][1]["kind"], "close_websocket");
    assert_eq!(
        planned["actions"][2]["kind"],
        "unregister_websocket_readable"
    );
    assert_eq!(planned["actions"][3]["kind"], "reconnect_gateway");
}

#[test]
fn websocket_turn_rejects_invalid_json_text_payloads_fail_closed() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "transport": "discord",
        "event_loop_token": 15,
        "websocket_fd": 16,
        "read_bytes": server_text_frame("not-json"),
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "failed_closed");
    assert_eq!(planned["ok"], false);
    assert!(planned["error"]
        .as_str()
        .unwrap()
        .starts_with("WebSocket text payload was not valid JSON:"));
    assert_eq!(planned["actions"][0]["kind"], "close_websocket");
    assert_eq!(
        planned["actions"][1]["kind"],
        "unregister_websocket_readable"
    );
    assert_eq!(planned["actions"][2]["kind"], "reconnect_gateway");
}

#[test]
fn websocket_turn_writable_drains_pending_write_and_restores_readable_interest() {
    let (mut reader, writer) = connected_tcp_pair();

    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "stage": "writable_turn",
        "transport": "slack",
        "backend": "linux_epoll",
        "shard_index": 3,
        "event_loop_token": 21,
        "websocket_fd": tcp_stream_native_socket(&writer),
        "pending_write_hex": "01020304",
    }))
    .unwrap();

    let mut received = [0u8; 4];
    reader.read_exact(&mut received).unwrap();

    assert_eq!(planned["websocket_turn_state"], "write_complete");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["bytes_written"], 4);
    assert_eq!(planned["remaining_write_byte_count"], 0);
    assert_eq!(planned["write_complete"], true);
    assert_eq!(planned["should_register_read_write"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "mark_websocket_fd_write_complete"
    );
    assert_eq!(
        planned["actions"][1]["kind"],
        "clear_websocket_pending_write"
    );
    assert_eq!(
        planned["actions"][2]["kind"],
        "keep_websocket_readable_registered"
    );
    assert_eq!(received, [1, 2, 3, 4]);
}

#[test]
fn websocket_turn_writable_carries_partial_write_and_keeps_read_write_interest() {
    let (mut reader, writer) = connected_tcp_pair();

    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "stage": "fd_writable",
        "transport": "discord",
        "event_loop_token": 22,
        "websocket_fd": tcp_stream_native_socket(&writer),
        "pending_write_bytes": [9, 8, 7, 6],
        "max_write_bytes": 2,
    }))
    .unwrap();

    let mut received = [0u8; 2];
    reader.read_exact(&mut received).unwrap();

    assert_eq!(planned["websocket_turn_state"], "partial_write");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["bytes_written"], 2);
    assert_eq!(planned["remaining_write_hex"], "0706");
    assert_eq!(planned["should_register_read_write"], true);
    assert_eq!(
        planned["actions"][0]["kind"],
        "queue_websocket_fd_write_retry"
    );
    assert_eq!(
        planned["actions"][1]["kind"],
        "carry_websocket_pending_write"
    );
    assert_eq!(
        planned["actions"][2]["kind"],
        "keep_websocket_read_write_registered"
    );
    assert_eq!(
        planned["actions"][2]["registration"]["interest"],
        "read_write"
    );
    assert_eq!(received, [9, 8]);
}

#[test]
fn websocket_turn_writable_empty_queue_keeps_readable_registration() {
    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "stage": "writable",
        "transport": "slack",
        "event_loop_token": 23,
        "websocket_fd": 24,
    }))
    .unwrap();

    assert_eq!(planned["websocket_turn_state"], "write_queue_empty");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["write_complete"], true);
    assert_eq!(planned["should_register_read_write"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "keep_websocket_readable_registered"
    );
}

#[test]
fn websocket_turn_ready_processes_readable_payload_and_writable_pending_bytes() {
    let (mut reader, writer) = connected_tcp_pair();

    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "stage": "ready_turn",
        "transport": "slack",
        "event_loop_token": 24,
        "websocket_fd": tcp_stream_native_socket(&writer),
        "readable": true,
        "writable": true,
        "read_bytes": server_text_frame(r#"{"type":"hello","num_connections":1}"#),
        "pending_write_hex": "aabbcc",
    }))
    .unwrap();

    let mut received = [0u8; 3];
    reader.read_exact(&mut received).unwrap();

    assert_eq!(planned["websocket_turn_state"], "readable_writable_turns");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["processed_text_payload_count"], 1);
    assert_eq!(planned["remaining_write_byte_count"], 0);
    assert_eq!(
        planned["actions"][0]["kind"],
        "dispatch_slack_socket_mode_payload"
    );
    assert_eq!(planned["actions"][1]["kind"], "mark_socket_mode_ready");
    assert_eq!(
        planned["actions"][2]["kind"],
        "mark_websocket_fd_write_complete"
    );
    assert_eq!(
        planned["actions"][3]["kind"],
        "clear_websocket_pending_write"
    );
    assert_eq!(
        planned["actions"][4]["kind"],
        "keep_websocket_readable_registered"
    );
    assert_eq!(received, [0xaa, 0xbb, 0xcc]);
}

#[test]
fn websocket_turn_ready_reads_fd_before_draining_pending_write() {
    let (client, mut peer) = connected_tcp_pair();
    peer.write_all(&server_text_frame(
        r#"{"type":"hello","num_connections":1}"#,
    ))
    .unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    let mut available = [0_u8; 1];
    assert_eq!(client.peek(&mut available).unwrap(), 1);

    let planned = agent_websocket_event_loop_turn_plan_json(&json!({
        "stage": "ready_turn",
        "transport": "slack",
        "event_loop_token": 34,
        "websocket_fd": tcp_stream_native_socket(&client),
        "readable": true,
        "writable": true,
        "pending_write_hex": "aabbcc",
        "max_read_bytes": 1024,
    }))
    .unwrap();

    let mut received = [0u8; 3];
    peer.read_exact(&mut received).unwrap();

    assert_eq!(planned["websocket_turn_state"], "readable_writable_turns");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["read_turn_plan"]["read_source"], "fd_io");
    assert_eq!(planned["write_turn_plan"]["bytes_written"], 3);
    let action_kinds = planned["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|action| action["kind"].as_str().unwrap())
        .collect::<Vec<_>>();
    let fd_read_index = action_kinds
        .iter()
        .position(|kind| *kind == "deliver_websocket_fd_read_chunk")
        .unwrap();
    let dispatch_index = action_kinds
        .iter()
        .position(|kind| *kind == "dispatch_slack_socket_mode_payload")
        .unwrap();
    let write_index = action_kinds
        .iter()
        .position(|kind| *kind == "mark_websocket_fd_write_complete")
        .unwrap();
    assert!(fd_read_index < dispatch_index);
    assert!(dispatch_index < write_index);
    assert_eq!(received, [0xaa, 0xbb, 0xcc]);
}

fn server_text_frame(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    assert!(bytes.len() < 126);
    let mut frame = Vec::with_capacity(bytes.len() + 2);
    frame.push(0x81);
    frame.push(bytes.len() as u8);
    frame.extend_from_slice(bytes);
    frame
}
