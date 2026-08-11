use super::agent_transport_websocket_frame_plan_json;
use ait_core::json_support::json;

#[test]
fn websocket_frame_encode_masks_client_text_frame() {
    let planned = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "encode",
        "opcode": "text",
        "payload_text": "hi",
        "mask_key": [1, 2, 3, 4],
    }))
    .unwrap();

    assert_eq!(planned["stage"], "encode");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_transport_websocket_frame_codec"
    );
    assert_eq!(
        planned["websocket_frame_contract"],
        "ait_agent_core.transport.WebSocketFrame.v1"
    );
    assert_eq!(planned["python_websocket_frame_codec_allowed"], false);
    assert_eq!(planned["opcode_name"], "text");
    assert_eq!(planned["masked"], true);
    assert_eq!(planned["payload_length"], 2);
    assert_eq!(
        planned["frame_bytes"],
        json!([129, 130, 1, 2, 3, 4, 105, 107])
    );
    assert_eq!(planned["frame_hex"], "818201020304696b");
    assert_eq!(planned["actions"][0]["kind"], "write_websocket_frame");
}

#[test]
fn websocket_frame_encode_requires_client_mask_key() {
    let error = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "encode",
        "opcode": "text",
        "payload_text": "hi",
    }))
    .unwrap_err();

    assert_eq!(
        error,
        "WebSocket client-to-server frames require an explicit 4-byte mask_key."
    );
}

#[test]
fn websocket_frame_decode_text_frame_delivers_payload() {
    let planned = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "810568656c6c6f",
    }))
    .unwrap();

    assert_eq!(planned["stage"], "decode");
    assert_eq!(planned["websocket_frame_state"], "decoded");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["complete"], true);
    assert_eq!(planned["opcode_name"], "text");
    assert_eq!(planned["payload_text"], "hello");
    assert_eq!(planned["payload_bytes"], json!([104, 101, 108, 108, 111]));
    assert_eq!(planned["consumed_bytes"], 7);
    assert_eq!(planned["should_deliver_text"], true);
    assert_eq!(planned["actions"][0]["kind"], "deliver_websocket_text");
}

#[test]
fn websocket_frame_decode_ping_plans_pong_before_dispatch() {
    let planned = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_bytes": [137, 2, 111, 107],
    }))
    .unwrap();

    assert_eq!(planned["websocket_frame_state"], "decoded");
    assert_eq!(planned["opcode_name"], "ping");
    assert_eq!(planned["should_send_pong"], true);
    assert_eq!(planned["actions"][0]["kind"], "send_websocket_pong");
    assert_eq!(
        planned["actions"][0]["execute_before_payload_dispatch"],
        true
    );
    assert_eq!(planned["actions"][0]["payload_bytes"], json!([111, 107]));
}

#[test]
fn websocket_frame_decode_binary_frame_delivers_payload_bytes() {
    let planned = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_bytes": [130, 3, 1, 2, 3],
    }))
    .unwrap();

    assert_eq!(planned["websocket_frame_state"], "decoded");
    assert_eq!(planned["opcode_name"], "binary");
    assert_eq!(planned["should_deliver_binary"], true);
    assert_eq!(planned["payload_bytes"], json!([1, 2, 3]));
    assert_eq!(planned["actions"][0]["kind"], "deliver_websocket_binary");
}

#[test]
fn websocket_frame_decode_pong_marks_pong() {
    let planned = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "8a00",
    }))
    .unwrap();

    assert_eq!(planned["websocket_frame_state"], "decoded");
    assert_eq!(planned["opcode_name"], "pong");
    assert_eq!(planned["should_mark_pong"], true);
    assert_eq!(planned["actions"][0]["kind"], "mark_websocket_pong");
}

#[test]
fn websocket_frame_decode_close_frame_parses_status_and_reason() {
    let planned = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "880503e8627965",
    }))
    .unwrap();

    assert_eq!(planned["websocket_frame_state"], "decoded");
    assert_eq!(planned["opcode_name"], "close");
    assert_eq!(planned["should_close_websocket"], true);
    assert_eq!(planned["close_status_code"], 1000);
    assert_eq!(planned["close_reason"], "bye");
    assert_eq!(planned["actions"][0]["kind"], "close_websocket");
}

#[test]
fn websocket_frame_decode_reports_partial_frame() {
    let planned = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "8105",
    }))
    .unwrap();

    assert_eq!(planned["websocket_frame_state"], "partial_frame");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["complete"], false);
    assert_eq!(planned["needed_bytes"], 7);
    assert_eq!(planned["actions"], json!([]));
}

#[test]
fn websocket_frame_decode_fails_closed_for_protocol_errors() {
    let rsv = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "c100",
    }))
    .unwrap();
    assert_eq!(rsv["websocket_frame_state"], "protocol_error");
    assert_eq!(rsv["ok"], false);
    assert_eq!(rsv["should_close_websocket"], true);
    assert_eq!(rsv["error"], "WebSocket frame RSV bits are not supported.");

    let masked_server_frame = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "818201020304696b",
    }))
    .unwrap();
    assert_eq!(
        masked_server_frame["error"],
        "WebSocket server-to-client frame must not be masked."
    );

    let fragmented_control = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "0900",
    }))
    .unwrap();
    assert_eq!(
        fragmented_control["error"],
        "WebSocket control frames must not be fragmented."
    );

    let invalid_opcode = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "8300",
    }))
    .unwrap();
    assert_eq!(
        invalid_opcode["error"],
        "WebSocket frame opcode is invalid."
    );

    let malformed_close = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "8801ff",
    }))
    .unwrap();
    assert_eq!(
        malformed_close["error"],
        "WebSocket close frame payload is malformed."
    );
}

#[test]
fn websocket_frame_decode_exposes_data_fragments_for_stream_assembly() {
    let text_start = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "010568656c6c6f",
    }))
    .unwrap();
    assert_eq!(text_start["websocket_frame_state"], "decoded");
    assert_eq!(text_start["fin"], false);
    assert_eq!(text_start["opcode_name"], "text");
    assert_eq!(text_start["fragmented"], true);
    assert_eq!(text_start["should_deliver_text"], false);
    assert_eq!(text_start["actions"], json!([]));

    let continuation = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "800620776f726c64",
    }))
    .unwrap();
    assert_eq!(continuation["websocket_frame_state"], "decoded");
    assert_eq!(continuation["fin"], true);
    assert_eq!(continuation["opcode_name"], "continuation");
    assert_eq!(continuation["continuation"], true);
    assert_eq!(continuation["payload_text"], " world");
    assert_eq!(continuation["actions"], json!([]));
}

#[test]
fn websocket_frame_decode_rejects_oversized_payloads() {
    let planned = agent_transport_websocket_frame_plan_json(&json!({
        "stage": "decode",
        "frame_hex": "817e0004",
        "max_payload_bytes": 3,
    }))
    .unwrap();

    assert_eq!(planned["websocket_frame_state"], "protocol_error");
    assert_eq!(
        planned["error"],
        "WebSocket frame payload length exceeds the configured limit."
    );
}
