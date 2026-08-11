use super::{
    build_transport_binding_metadata_json, build_transport_event_envelope_json,
    build_transport_reply_envelope_json, compact_transport_event_envelope_json,
    compact_transport_reply_envelope_json, transport_envelope_ir_version,
    transport_envelope_schema_json, TransportEnvelopeJson, TRANSPORT_ENVELOPE_IR_VERSION,
};
use crate::json_support::json;

#[test]
fn schema_is_sessionless_v2() {
    assert_eq!(transport_envelope_ir_version(), "ait.transport_envelope.v2");
    assert_eq!(
        transport_envelope_ir_version(),
        TRANSPORT_ENVELOPE_IR_VERSION
    );

    let schema = transport_envelope_schema_json();
    let encoded = schema.to_string();
    assert!(!encoded.contains("session_metadata"));
    assert!(!encoded.contains("sessionMetadata"));
    assert!(!encoded.contains("shared_session"));
    assert!(encoded.contains("conversation_key"));
}

#[test]
fn binding_metadata_uses_conversation_identity() {
    let reply_target = json!({"channel_id": "C123", "thread_id": "TH999"});
    let metadata_extra = json!({"tenant": "acme", "transport": "cannot-override"});
    let binding = build_transport_binding_metadata_json(
        "discord",
        "C123",
        Some("ops"),
        Some("thread"),
        Some("TH999"),
        "discord:C123:TH999",
        Some(&reply_target),
        Some(&metadata_extra),
    );

    assert_eq!(binding["transport"], "discord");
    assert_eq!(binding["surface_id"], "C123");
    assert_eq!(binding["thread_id"], "TH999");
    assert_eq!(binding["conversation_key"], "discord:C123:TH999");
    assert_eq!(binding["transport_reply_target"], reply_target);
    assert_eq!(binding["tenant"], "acme");
    assert!(!binding.to_string().contains("session"));
}

#[test]
fn stateless_wrapper_matches_public_builders() {
    let contract = TransportEnvelopeJson::stateless();
    assert_eq!(contract.ir_version(), transport_envelope_ir_version());
    assert_eq!(contract.schema_json(), transport_envelope_schema_json());

    let wrapper = contract.binding_metadata_json(
        "telegram",
        "123",
        Some("Wei"),
        Some("private"),
        None,
        "telegram:123",
        None,
        None,
    );
    let public = build_transport_binding_metadata_json(
        "telegram",
        "123",
        Some("Wei"),
        Some("private"),
        None,
        "telegram:123",
        None,
        None,
    );
    assert_eq!(wrapper, public);
}

#[test]
fn event_builder_normalizes_message_identity_and_attachments() {
    let event = build_transport_event_envelope_json(
        "telegram",
        "telegram:456:@weita",
        "123",
        "Uploaded song",
        Some("456"),
        Some("weita"),
        Some("Wei Ta"),
        None,
        Some("Wei"),
        Some("private"),
        None,
        Some(&json!(11)),
        Some(&json!([10, 11, 11, 0])),
        None,
        None,
        None,
        Some(&json!([{
            "media_kind": "music",
            "file_id": "tg-audio-001",
            "duration": 42
        }])),
        None,
    );

    assert_eq!(event["message"]["message_ids"], json!([10, 11]));
    assert_eq!(event["message"]["logical_turn_message_count"], 2);
    assert_eq!(event["message"]["attachments"][0]["kind"], "music");

    let compact = compact_transport_event_envelope_json(&event);
    assert_eq!(compact["transport"], "telegram");
    assert_eq!(compact["channel"]["channel_id"], "123");
    assert_eq!(compact["message"]["message_ids"], json!([10, 11]));
}

#[test]
fn reply_builder_and_compaction_preserve_delivery_coordinates() {
    let reply = build_transport_reply_envelope_json(
        "slack",
        "C123",
        "done",
        Some("release-room"),
        Some("channel"),
        Some("T456"),
        None,
        Some("slack:C123:message:99"),
        Some(&json!(99)),
        Some(&json!([98, 99, 99])),
        None,
        Some(&json!({"delivery_id": "delivery-1"})),
    );

    assert_eq!(reply["delivery_kind"], "chat_reply");
    assert_eq!(reply["reply_to"]["message_ids"], json!([98, 99]));
    assert_eq!(reply["metadata"]["delivery_id"], "delivery-1");

    let compact = compact_transport_reply_envelope_json(&reply);
    assert_eq!(compact["target"]["channel_id"], "C123");
    assert_eq!(compact["target"]["thread_id"], "T456");
    assert_eq!(compact["reply_to"]["message_ids"], json!([98, 99]));
}
