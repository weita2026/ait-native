use super::agent_transport_websocket_stream_plan_json;
use ait_core::json_support::json;

#[test]
fn websocket_stream_consumes_multiple_text_frames_in_one_read() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_hex": "8102686981057468657265",
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "frames_dispatched");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_transport_websocket_stream_boundary"
    );
    assert_eq!(
        planned["websocket_stream_contract"],
        "ait_agent_core.transport.WebSocketStream.v1"
    );
    assert_eq!(planned["python_websocket_stream_allowed"], false);
    assert_eq!(planned["processed_frame_count"], 2);
    assert_eq!(planned["consumed_bytes"], 11);
    assert_eq!(planned["remaining_buffer_hex"], "");
    assert_eq!(planned["actions"][0]["kind"], "deliver_websocket_text");
    assert_eq!(planned["actions"][0]["payload_text"], "hi");
    assert_eq!(planned["actions"][1]["payload_text"], "there");
}

#[test]
fn websocket_stream_preserves_partial_frame_bytes() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "buffer_hex": "8105",
        "read_hex": "6865",
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "partial_frame");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["complete"], false);
    assert_eq!(planned["needed_bytes"], 7);
    assert_eq!(planned["needed_additional_bytes"], 3);
    assert_eq!(planned["remaining_buffer_hex"], "81056865");
    assert_eq!(planned["actions"], json!([]));
}

#[test]
fn websocket_stream_completes_prior_buffer_with_read_chunk() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "buffer_hex": "8105",
        "read_hex": "68656c6c6f",
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "frames_dispatched");
    assert_eq!(planned["complete"], true);
    assert_eq!(planned["processed_frame_count"], 1);
    assert_eq!(planned["actions"][0]["payload_text"], "hello");
    assert_eq!(planned["remaining_buffer_bytes"], json!([]));
}

#[test]
fn websocket_stream_assembles_fragmented_text_across_reads() {
    let started = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_hex": "010568656c6c6f",
    }))
    .unwrap();

    assert_eq!(started["websocket_stream_state"], "fragment_in_progress");
    assert_eq!(started["complete"], false);
    assert_eq!(started["fragment_in_progress"], true);
    assert_eq!(started["fragment_opcode"], "text");
    assert_eq!(started["fragment_payload_hex"], "68656c6c6f");
    assert_eq!(started["actions"], json!([]));

    let completed = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "fragment_opcode": started["fragment_opcode"],
        "fragment_payload_bytes": started["fragment_payload_bytes"],
        "read_hex": "800620776f726c64",
    }))
    .unwrap();

    assert_eq!(completed["websocket_stream_state"], "frames_dispatched");
    assert_eq!(completed["complete"], true);
    assert_eq!(completed["fragment_in_progress"], false);
    assert_eq!(completed["fragment_payload_bytes"], json!([]));
    assert_eq!(completed["actions"][0]["kind"], "deliver_websocket_text");
    assert_eq!(completed["actions"][0]["payload_text"], "hello world");
    assert_eq!(completed["actions"][0]["fragmented"], true);
}

#[test]
fn websocket_stream_allows_control_frames_inside_fragmented_text() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_hex": "010268698901788003212121",
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["complete"], true);
    assert_eq!(planned["processed_frame_count"], 3);
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][0]["opcode"], "pong");
    assert_eq!(planned["actions"][1]["kind"], "deliver_websocket_text");
    assert_eq!(planned["actions"][1]["payload_text"], "hi!!!");
}

#[test]
fn websocket_stream_rejects_orphan_continuation_and_fragment_overflow() {
    let orphan = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_hex": "800178",
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();
    assert_eq!(orphan["websocket_stream_state"], "protocol_error");
    assert_eq!(
        orphan["error"],
        "WebSocket continuation frame arrived without an active fragmented message."
    );

    let overflow = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "fragment_opcode": "text",
        "fragment_payload_bytes": [1, 2, 3],
        "read_hex": "80020405",
        "max_payload_bytes": 4,
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();
    assert_eq!(overflow["websocket_stream_state"], "protocol_error");
    assert_eq!(
        overflow["error"],
        "WebSocket fragmented message payload exceeds the configured limit."
    );
}

#[test]
fn websocket_stream_rejects_invalid_fragment_carry_even_when_payload_is_empty() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "fragment_opcode": "continuation",
        "fragment_payload_bytes": [],
        "read_bytes": [],
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "WebSocket fragmented-message carry requires opcode `text` or `binary`."
    );
}

#[test]
fn websocket_stream_rejects_invalid_utf8_text_messages() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_hex": "8101ff",
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "protocol_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "WebSocket text message must be valid UTF-8."
    );
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][0]["status_code"], 1002);
}

#[test]
fn websocket_stream_ping_writes_masked_pong_frame() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_bytes": [137, 2, 111, 107],
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "frames_dispatched");
    assert_eq!(planned["processed_frame_count"], 1);
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][0]["opcode"], "pong");
    assert_eq!(
        planned["actions"][0]["execute_before_payload_dispatch"],
        true
    );
    assert_eq!(
        planned["actions"][0]["frame_bytes"],
        json!([138, 130, 1, 2, 3, 4, 110, 105])
    );
    assert_eq!(planned["actions"][0]["frame_hex"], "8a82010203046e69");
}

#[test]
fn websocket_stream_ping_without_mask_key_fails_closed() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_bytes": [137, 2, 111, 107],
    }))
    .unwrap();

    assert_eq!(
        planned["error"],
        "WebSocket stream control response requires an explicit 4-byte mask_key."
    );
    assert_eq!(planned["websocket_stream_state"], "configuration_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["should_close_websocket"], true);
    assert_eq!(planned["actions"][0]["kind"], "close_websocket");
}

#[test]
fn websocket_stream_delivers_binary_and_marks_pong() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_bytes": [130, 3, 1, 2, 3, 138, 0],
    }))
    .unwrap();

    assert_eq!(planned["processed_frame_count"], 2);
    assert_eq!(planned["actions"][0]["kind"], "deliver_websocket_binary");
    assert_eq!(planned["actions"][0]["payload_bytes"], json!([1, 2, 3]));
    assert_eq!(planned["actions"][1]["kind"], "mark_websocket_pong");
}

#[test]
fn websocket_stream_close_writes_masked_close_then_closes() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_hex": "880203e8",
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "closing");
    assert_eq!(planned["should_close_websocket"], true);
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][0]["opcode"], "close");
    assert_eq!(
        planned["actions"][0]["frame_bytes"],
        json!([136, 130, 1, 2, 3, 4, 2, 234])
    );
    assert_eq!(planned["actions"][1]["kind"], "close_websocket");
    assert_eq!(planned["actions"][1]["status_code"], 1000);
}

#[test]
fn websocket_stream_protocol_error_writes_protocol_close() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_hex": "8300",
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "protocol_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["should_close_websocket"], true);
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][0]["opcode"], "close");
    assert_eq!(planned["actions"][0]["status_code"], 1002);
    assert_eq!(planned["actions"][1]["kind"], "close_websocket");
    assert_eq!(planned["error"], "WebSocket frame opcode is invalid.");
}

#[test]
fn websocket_stream_oversized_payload_fails_closed() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "read_hex": "817e0004",
        "max_payload_bytes": 3,
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "protocol_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "WebSocket frame payload length exceeds the configured limit."
    );
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
    assert_eq!(planned["actions"][1]["kind"], "close_websocket");
}

#[test]
fn websocket_stream_eof_with_partial_frame_fails_closed() {
    let planned = agent_transport_websocket_stream_plan_json(&json!({
        "stage": "consume_read_chunk",
        "buffer_hex": "8105",
        "read_eof": true,
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["websocket_stream_state"], "protocol_error");
    assert_eq!(planned["ok"], false);
    assert_eq!(
        planned["error"],
        "WebSocket peer closed with an incomplete frame buffered."
    );
    assert_eq!(planned["actions"][1]["kind"], "close_websocket");
}
