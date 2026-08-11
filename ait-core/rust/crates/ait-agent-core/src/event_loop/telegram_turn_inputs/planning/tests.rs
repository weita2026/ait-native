use super::{
    agent_telegram_turn_input_plan_json, plan_with_telegram_turn_input_planner,
    DefaultTelegramTurnInputPlanner, TelegramTurnInputPlanner,
};
use ait_core::json_support::{json, JsonValue};

struct SubstituteTurnInputPlanner;

impl TelegramTurnInputPlanner for SubstituteTurnInputPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        Ok(json!({
            "kind": request.get("kind").cloned().unwrap_or(JsonValue::Null),
            "substitute": true,
            "transport": "telegram",
        }))
    }
}

#[test]
fn turn_input_normalizes_text_and_bot_mention() {
    let planned = agent_telegram_turn_input_plan_json(&json!({
        "kind": "normalize_user_text",
        "text": "@ait_test_bot，  hello\tworld\r\n\r\n\r\nnext",
        "username": "AIT_TEST_BOT",
    }))
    .unwrap();
    assert_eq!(planned["migration_stage"], "rust_agent_telegram_turn_input");
    assert_eq!(
        planned["turn_input_contract"],
        "ait_agent_core.event_loop.TelegramTurnInput.v1"
    );
    assert_eq!(planned["python_turn_input_allowed"], false);
    assert_eq!(planned["text"], "hello world\n\nnext");
}

#[test]
fn turn_input_extracts_speech_music_and_file_attachments() {
    let message = json!({
        "caption": "listen",
        "voice": {"file_id": "voice-1", "file_unique_id": "v1", "mime_type": "audio/ogg", "duration": 3, "file_size": 2048},
        "audio": {"file_id": "audio-1", "file_name": "song.mp3", "title": "Song", "performer": "Artist", "duration": 42, "file_size": 1048576},
        "document": {"file_id": "doc-1", "file_name": "report.pdf", "mime_type": "application/pdf", "file_size": 512},
        "photo": [
            {"file_id": "small", "width": 10, "height": 10, "file_size": 100},
            {"file_id": "large", "width": 20, "height": 20, "file_size": 200}
        ]
    });

    let speech = agent_telegram_turn_input_plan_json(&json!({
        "kind": "speech_attachments_from_message",
        "message": message,
        "include_audio_uploads": true,
    }))
    .unwrap();
    assert_eq!(speech["attachments"][0]["kind"], "voice");
    assert_eq!(speech["attachments"][1]["kind"], "audio");

    let music = agent_telegram_turn_input_plan_json(&json!({
        "kind": "music_attachments_from_message",
        "message": message,
    }))
    .unwrap();
    assert_eq!(music["attachments"][0]["media_kind"], "music");

    let files = agent_telegram_turn_input_plan_json(&json!({
        "kind": "file_attachments_from_message",
        "message": message,
        "include_speech_uploads": true,
    }))
    .unwrap();
    assert_eq!(files["attachments"][0]["kind"], "voice");
    assert_eq!(files["attachments"][1]["kind"], "document");
    assert_eq!(files["attachments"][2]["telegram_file_id"], "large");
}

#[test]
fn turn_input_formats_attachment_and_speech_text() {
    let planned = agent_telegram_turn_input_plan_json(&json!({
        "kind": "normalized_turn_text",
        "raw_text": " @bot hi ",
        "username": "bot",
        "attachments": [
            {"kind": "document", "file_name": "report.pdf", "mime_type": "application/pdf", "file_size_bytes": 1536, "telegram_file_id": "doc-1"}
        ],
    }))
    .unwrap();
    assert_eq!(
        planned["text"],
        "hi\n\nTelegram attachment upload:\n- report.pdf (kind=document, application/pdf, 1.5 KB)"
    );

    let speech = agent_telegram_turn_input_plan_json(&json!({
        "kind": "speech_turn_text",
        "caption": "caption",
        "transcript": "hello\tworld\r\n\r\n\r\nnext",
    }))
    .unwrap();
    assert_eq!(
        speech["text"],
        "caption\n\n[local speech transcript]\nhello world\n\nnext"
    );
}

#[test]
fn turn_input_extracts_reply_payloads_and_send_kind() {
    let event = json!({
        "payload": {
            "text": "fallback",
            "transport_reply_envelope": {
                "message": {
                    "text": "reply text",
                    "attachments": [
                        {"kind": "image", "local_path": "/tmp/photo.webp", "mime_type": "image/webp"}
                    ]
                }
            }
        }
    });
    let text = agent_telegram_turn_input_plan_json(&json!({
        "kind": "transport_reply_text",
        "assistant_event": event,
    }))
    .unwrap();
    assert_eq!(text["text"], "reply text");
    let attachments = agent_telegram_turn_input_plan_json(&json!({
        "kind": "transport_reply_attachments",
        "assistant_event": event,
    }))
    .unwrap();
    assert_eq!(attachments["attachments"][0]["kind"], "image");

    let send = agent_telegram_turn_input_plan_json(&json!({
        "kind": "attachment_send_kind",
        "attachment": {"kind": "image", "local_path": "/tmp/photo.webp", "mime_type": "image/webp"},
    }))
    .unwrap();
    assert_eq!(send["send_as_audio"], false);
    assert_eq!(send["send_as_photo"], true);
}

#[test]
fn turn_input_export_contract_covers_transitional_python_wrapper_surface() {
    let attachments = agent_telegram_turn_input_plan_json(&json!({
        "kind": "transport_reply_attachments",
        "assistant_event": {
            "payload": {
                "transport_reply_envelope": {
                    "message": {
                        "attachments": [
                            {"kind": "audio", "file_name": "demo.mp3"}
                        ]
                    }
                }
            }
        }
    }))
    .unwrap();

    assert_eq!(
        attachments["migration_stage"],
        "rust_agent_telegram_turn_input"
    );
    assert_eq!(attachments["transport"], "telegram");
    assert_eq!(attachments["python_turn_input_allowed"], false);
    assert_eq!(attachments["attachments"][0]["file_name"], "demo.mp3");

    let text = agent_telegram_turn_input_plan_json(&json!({
        "kind": "speech_turn_text",
        "caption": "caption",
        "transcript": "transcript",
    }))
    .unwrap();
    assert_eq!(
        text["text"],
        "caption\n\n[local speech transcript]\ntranscript"
    );

    let send_kind = agent_telegram_turn_input_plan_json(&json!({
        "kind": "attachment_send_kind",
        "attachment": {"kind": "audio", "file_name": "demo.mp3"}
    }))
    .unwrap();
    assert_eq!(send_kind["send_as_audio"], true);
    assert_eq!(send_kind["send_as_photo"], false);
}

#[test]
fn turn_input_defaults_aliases_and_error_contract_are_stable() {
    let default_plan = agent_telegram_turn_input_plan_json(&json!({
        "text": "@bot:  hello\tthere\r\nnext",
        "username": "bot"
    }))
    .unwrap();
    assert_eq!(default_plan["kind"], "normalize_user_text");
    assert_eq!(default_plan["transport"], "telegram");
    assert_eq!(default_plan["rust_event_loop_required"], true);
    assert_eq!(default_plan["python_turn_input_allowed"], false);
    assert_eq!(default_plan["text"], "hello there\nnext");

    let alias = agent_telegram_turn_input_plan_json(&json!({
        "stage": "strip_leading_bot_mention",
        "text": "@ait-bot - keep the suffix",
        "username": "ait-bot"
    }))
    .unwrap();
    assert_eq!(alias["kind"], "strip_leading_bot_mention");
    assert_eq!(alias["text"], "- keep the suffix");

    let no_separator = agent_telegram_turn_input_plan_json(&json!({
        "stage": "strip_leading_bot_mention",
        "text": "@ait-botched text",
        "username": "ait-bot"
    }))
    .unwrap();
    assert_eq!(no_separator["text"], "@ait-botched text");

    let invalid = agent_telegram_turn_input_plan_json(&json!("bad"));
    assert_eq!(invalid.unwrap_err(), "request must be a JSON object");

    let unsupported = agent_telegram_turn_input_plan_json(&json!({
        "kind": "unknown"
    }));
    assert_eq!(
        unsupported.unwrap_err(),
        "unsupported Telegram turn input plan kind `unknown`"
    );
}

#[test]
fn turn_input_reply_fallbacks_summaries_and_send_kind_edges_are_stable() {
    let fallback_text = agent_telegram_turn_input_plan_json(&json!({
        "kind": "transport_reply_text",
        "assistant_event": {"payload": {"text": "fallback only"}}
    }))
    .unwrap();
    assert_eq!(fallback_text["text"], "fallback only");

    let empty_attachments = agent_telegram_turn_input_plan_json(&json!({
        "kind": "transport_reply_attachments",
        "assistant_event": {"payload": {"transport_reply_envelope": {"message": {"attachments": ["skip", 5]}}}}
    }))
    .unwrap();
    assert_eq!(empty_attachments["attachments"], json!([]));

    let music_summary = agent_telegram_turn_input_plan_json(&json!({
        "kind": "attachment_summary",
        "attachments": [{
            "media_kind": "music",
            "file_name": "song.flac",
            "title": "Track",
            "performer": "Artist",
            "mime_type": "audio/flac",
            "duration_seconds": "65",
            "file_size_bytes": 2097152
        }]
    }))
    .unwrap();
    assert_eq!(
        music_summary["text"],
        "Telegram music upload:\n- song.flac (title=Track, performer=Artist, audio/flac, 65s, 2.0 MB)"
    );

    let audio_extension = agent_telegram_turn_input_plan_json(&json!({
        "kind": "attachment_send_kind",
        "attachment": {"kind": "file", "local_path": "/tmp/SONG.MP3"}
    }))
    .unwrap();
    assert_eq!(audio_extension["send_as_audio"], true);
    assert_eq!(audio_extension["send_as_photo"], false);

    let document_image = agent_telegram_turn_input_plan_json(&json!({
        "kind": "attachment_send_kind",
        "attachment": {"kind": "document", "file_name": "photo.png", "mime_type": "image/png"}
    }))
    .unwrap();
    assert_eq!(document_image["send_as_audio"], false);
    assert_eq!(document_image["send_as_photo"], false);

    let gif = agent_telegram_turn_input_plan_json(&json!({
        "kind": "attachment_send_kind",
        "attachment": {"mime_type": "image/gif"}
    }))
    .unwrap();
    assert_eq!(gif["send_as_audio"], false);
    assert_eq!(gif["send_as_photo"], false);
}

#[test]
fn turn_input_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramTurnInputPlanner = &DefaultTelegramTurnInputPlanner;
    let planned = planner
        .plan_json(&json!({
            "kind": "normalize_user_text",
            "text": "@bot hello",
            "username": "bot"
        }))
        .unwrap();

    assert_eq!(planned["kind"], "normalize_user_text");
    assert_eq!(planned["text"], "hello");
}

#[test]
fn turn_input_bound_entrypoint_accepts_substitute_planner() {
    let planner = SubstituteTurnInputPlanner;
    let planned =
        plan_with_telegram_turn_input_planner(&planner, &json!({"kind": "normalize_user_text"}))
            .unwrap();
    assert_eq!(planned["kind"], "normalize_user_text");
    assert_eq!(planned["substitute"], true);
    assert_eq!(planned["transport"], "telegram");
}
