use super::{
    agent_compact_transport_event_envelope_json, agent_transport_binding_metadata_json,
    agent_transport_envelope_ir_version, agent_transport_event_envelope_json,
    agent_transport_reply_envelope_json,
};
use ait_core::json_support::json;

#[test]
fn agent_transport_envelope_boundary_preserves_core_event_shape() {
    let event = agent_transport_event_envelope_json(
        "telegram",
        "telegram:456:@weita",
        "123",
        "hello",
        Some("456"),
        Some("weita"),
        Some("Wei Ta"),
        None,
        Some("Wei"),
        Some("private"),
        None,
        Some(&json!(11)),
        Some(&json!([10, 11, 11])),
        None,
        None,
        None,
        Some(&json!([{"kind": "audio", "file_id": "tg-audio-001"}])),
        None,
    );

    assert_eq!(
        agent_transport_envelope_ir_version(),
        "ait.transport_envelope.v2"
    );
    assert_eq!(event["transport"], json!("telegram"));
    assert_eq!(event["event_id"], json!("telegram:123:message:10-11"));
    assert_eq!(event["message"]["message_id"], json!(11));
    assert_eq!(
        agent_compact_transport_event_envelope_json(&event),
        json!({
            "transport": "telegram",
            "event_kind": "message",
            "event_id": "telegram:123:message:10-11",
            "channel": {"channel_id": "123"},
            "message": {
                "message_id": 11,
                "message_ids": [10, 11],
                "logical_turn_message_count": 2,
                "attachments": [{"kind": "audio", "telegram_file_id": "tg-audio-001"}]
            }
        })
    );
}

#[test]
fn agent_transport_envelope_boundary_preserves_binding_and_reply_shape() {
    assert_eq!(
        agent_transport_binding_metadata_json(
            "slack",
            "C123",
            Some("release-room"),
            Some("channel"),
            None,
            "slack:C123",
            Some(&json!({"channel_id": "C123"})),
            None,
        ),
        json!({
            "transport": "slack",
            "surface_id": "C123",
            "surface_title": "release-room",
            "surface_kind": "channel",
            "conversation_key": "slack:C123",
            "transport_reply_target": {"channel_id": "C123"}
        })
    );

    assert_eq!(
        agent_transport_reply_envelope_json(
            "line",
            "U123",
            "received",
            None,
            None,
            None,
            None,
            Some("line:U123:message:9"),
            Some(&json!(9)),
            Some(&json!([8, 9])),
            None,
            None,
        ),
        json!({
            "schema_version": 1,
            "transport": "line",
            "delivery_kind": "chat_reply",
            "target": {"channel_id": "U123"},
            "reply_to": {
                "event_id": "line:U123:message:9",
                "message_id": 9,
                "message_ids": [8, 9],
                "logical_turn_message_count": 2
            },
            "message": {"text": "received"}
        })
    );
}
