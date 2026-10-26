use super::agent_websocket_shard_event_batch_plan_json;
use crate::platform::{connected_tcp_pair, tcp_stream_native_socket};
use ait_core::json_support::json;
use std::io::Read;

#[test]
fn websocket_shard_routes_multiple_tokens_in_ordered_epoll_batch() {
    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "linux_epoll",
        "shard_index": 4,
        "expected_concurrent_workers": 512,
        "connections": [
            {
                "worker_key": "slack/team-a",
                "transport": "slack",
                "event_loop_token": 11,
                "websocket_fd": 101
            },
            {
                "worker_key": "discord/ops",
                "transport": "discord",
                "event_loop_token": 12,
                "websocket_fd": 102,
                "bot_token": "bot-secret",
                "platform": "linux"
            }
        ],
        "events": [
            {
                "token": 11,
                "readable": true,
                "read_bytes": server_text_frame(r#"{"type":"hello","num_connections":1}"#)
            },
            {
                "token": 12,
                "readable": true,
                "read_bytes": server_text_frame(r#"{"op":11}"#)
            }
        ]
    }))
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_websocket_shard_event_batch_boundary"
    );
    assert_eq!(
        planned["websocket_shard_event_batch_contract"],
        "ait_agent_core.event_loop.WebSocketShardEventBatch.v1"
    );
    assert_eq!(
        planned["websocket_shard_event_batch_state"],
        "events_planned"
    );
    assert_eq!(planned["backend"], "linux_epoll");
    assert_eq!(planned["shard_index"], 4);
    assert_eq!(planned["high_concurrency"], true);
    assert_eq!(planned["known_event_count"], 2);
    assert_eq!(planned["unknown_event_count"], 0);
    assert_eq!(planned["python_websocket_shard_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(planned["turn_results"][0]["worker_key"], "slack/team-a");
    assert_eq!(
        planned["turn_results"][0]["websocket_turn_state"],
        "payloads_dispatched"
    );
    assert_eq!(planned["turn_results"][1]["worker_key"], "discord/ops");
    assert_eq!(
        planned["actions"][0]["kind"],
        "websocket_shard_worker_action"
    );
    assert_eq!(planned["actions"][0]["worker_key"], "slack/team-a");
    assert_eq!(
        planned["actions"][0]["action"]["kind"],
        "dispatch_slack_socket_mode_payload"
    );
    assert_eq!(planned["actions"][3]["worker_key"], "discord/ops");
    assert_eq!(
        planned["actions"][3]["action"]["kind"],
        "dispatch_discord_gateway_payload"
    );
}

#[test]
fn websocket_shard_preserves_partial_buffer_per_connection() {
    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "linux_epoll",
        "connections": [{
            "worker_key": "slack/team-a",
            "transport": "slack",
            "event_loop_token": 3,
            "websocket_fd": 9,
            "receive_buffer_hex": "8105"
        }],
        "events": [{
            "token": 3,
            "read_hex": "6865"
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_shard_event_batch_state"],
        "events_planned"
    );
    assert_eq!(
        planned["connection_state_updates"][0]["websocket_turn_state"],
        "partial_frame"
    );
    assert_eq!(
        planned["connection_state_updates"][0]["remaining_buffer_hex"],
        "81056865"
    );
    assert_eq!(
        planned["connection_state_updates"][0]["should_keep_registered"],
        true
    );
}

#[test]
fn websocket_shard_hangup_unregisters_and_reconnects_for_worker() {
    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "linux_epoll",
        "connections": [{
            "worker_key": "slack/team-a",
            "transport": "slack",
            "event_loop_token": 7,
            "websocket_fd": 17
        }],
        "events": [{
            "token": 7,
            "hangup": true
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_shard_event_batch_state"],
        "partial_failure"
    );
    assert_eq!(
        planned["connection_state_updates"][0]["websocket_turn_state"],
        "hangup_reconnect"
    );
    assert_eq!(
        planned["connection_state_updates"][0]["should_unregister"],
        true
    );
    assert_eq!(
        planned["connection_state_updates"][0]["should_reconnect"],
        true
    );
    assert_eq!(planned["actions"][0]["action"]["kind"], "close_websocket");
    assert_eq!(
        planned["actions"][1]["action"]["kind"],
        "unregister_websocket_readable"
    );
    assert_eq!(
        planned["actions"][2]["action"]["kind"],
        "reconnect_socket_mode"
    );
}

#[test]
fn websocket_shard_diagnoses_unknown_tokens_without_python_fallback() {
    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "linux_epoll",
        "connections": [{
            "worker_key": "slack/team-a",
            "transport": "slack",
            "event_loop_token": 1,
            "websocket_fd": 10
        }],
        "events": [{
            "token": 99,
            "readable": true,
            "read_bytes": server_text_frame(r#"{"type":"hello"}"#)
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_shard_event_batch_state"],
        "unknown_tokens"
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["known_event_count"], 0);
    assert_eq!(planned["unknown_event_count"], 1);
    assert_eq!(planned["python_websocket_shard_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_unknown_websocket_token"
    );
    assert_eq!(planned["actions"][0]["event_loop_token"], 99);
}

#[test]
fn websocket_shard_rejects_duplicate_tokens_fail_closed() {
    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "linux_epoll",
        "connections": [
            {
                "worker_key": "slack/team-a",
                "transport": "slack",
                "event_loop_token": 5,
                "websocket_fd": 10
            },
            {
                "worker_key": "discord/ops",
                "transport": "discord",
                "event_loop_token": 5,
                "websocket_fd": 11
            }
        ],
        "events": []
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_shard_event_batch_state"],
        "invalid_connection_state"
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["launch_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_invalid_websocket_shard_state"
    );
    assert!(planned["diagnostics"][0]
        .as_str()
        .unwrap()
        .contains("token 5"));
}

#[test]
fn websocket_shard_rejects_high_concurrency_portable_poll() {
    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "portable_poll",
        "expected_concurrent_workers": 128,
        "connections": [{
            "worker_key": "slack/team-a",
            "transport": "slack",
            "event_loop_token": 1,
            "websocket_fd": 10
        }],
        "events": []
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_shard_event_batch_state"],
        "backend_requires_epoll"
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["launch_allowed"], false);
    assert_eq!(planned["high_concurrency"], true);
    assert_eq!(planned["requires_epoll_for_target_scale"], true);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "reject_websocket_shard_backend"
    );
}

#[test]
fn websocket_shard_routes_writable_event_and_carries_pending_write_state() {
    let (mut reader, writer) = connected_tcp_pair();

    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "linux_epoll",
        "connections": [{
            "worker_key": "discord/ops",
            "transport": "discord",
            "event_loop_token": 31,
            "websocket_fd": tcp_stream_native_socket(&writer),
            "pending_write_hex": "01020304"
        }],
        "events": [{
            "token": 31,
            "readable": false,
            "writable": true,
            "max_write_bytes": 2
        }]
    }))
    .unwrap();

    let mut received = [0u8; 2];
    reader.read_exact(&mut received).unwrap();

    assert_eq!(
        planned["websocket_shard_event_batch_state"],
        "events_planned"
    );
    assert_eq!(planned["known_event_count"], 1);
    assert_eq!(
        planned["turn_results"][0]["websocket_turn_state"],
        "partial_write"
    );
    assert_eq!(
        planned["connection_state_updates"][0]["remaining_write_hex"],
        "0304"
    );
    assert_eq!(
        planned["connection_state_updates"][0]["should_register_read_write"],
        true
    );
    assert_eq!(
        planned["actions"][0]["action"]["kind"],
        "queue_websocket_fd_write_retry"
    );
    assert_eq!(
        planned["actions"][2]["action"]["kind"],
        "keep_websocket_read_write_registered"
    );
    assert_eq!(received, [1, 2]);
}

#[test]
fn websocket_shard_routes_combined_readable_writable_event_through_ready_turn() {
    let (mut reader, writer) = connected_tcp_pair();

    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "linux_epoll",
        "connections": [{
            "worker_key": "slack/team-a",
            "transport": "slack",
            "event_loop_token": 32,
            "websocket_fd": tcp_stream_native_socket(&writer),
            "pending_write_hex": "050607"
        }],
        "events": [{
            "token": 32,
            "readable": true,
            "writable": true,
            "read_bytes": server_text_frame(r#"{"type":"hello","num_connections":1}"#)
        }]
    }))
    .unwrap();

    let mut received = [0u8; 3];
    reader.read_exact(&mut received).unwrap();

    assert_eq!(
        planned["websocket_shard_event_batch_state"],
        "events_planned"
    );
    assert_eq!(
        planned["turn_results"][0]["websocket_turn_state"],
        "readable_writable_turns"
    );
    assert_eq!(
        planned["connection_state_updates"][0]["remaining_write_byte_count"],
        0
    );
    assert_eq!(
        planned["connection_state_updates"][0]["should_register_read_write"],
        false
    );
    assert_eq!(
        planned["actions"][0]["action"]["kind"],
        "dispatch_slack_socket_mode_payload"
    );
    assert_eq!(
        planned["actions"][2]["action"]["kind"],
        "mark_websocket_fd_write_complete"
    );
    assert_eq!(received, [5, 6, 7]);
}

#[test]
fn websocket_shard_skips_events_that_are_neither_readable_nor_writable() {
    let planned = agent_websocket_shard_event_batch_plan_json(&json!({
        "backend": "linux_epoll",
        "connections": [{
            "worker_key": "slack/team-a",
            "transport": "slack",
            "event_loop_token": 41,
            "websocket_fd": 10
        }],
        "events": [{
            "token": 41,
            "readable": false,
            "writable": false,
            "hangup": false
        }]
    }))
    .unwrap();

    assert_eq!(planned["websocket_shard_event_batch_state"], "idle");
    assert_eq!(planned["known_event_count"], 0);
    assert_eq!(planned["unknown_event_count"], 0);
    assert_eq!(planned["actions"][0]["kind"], "skip_websocket_event");
    assert_eq!(
        planned["actions"][0]["reason"],
        "event was neither readable, writable, nor hangup"
    );
}

fn server_text_frame(payload: &str) -> Vec<u8> {
    let bytes = payload.as_bytes();
    assert!(bytes.len() < 126);
    let mut frame = vec![0x81, bytes.len() as u8];
    frame.extend(bytes);
    frame
}
