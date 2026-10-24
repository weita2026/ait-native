use super::*;
use ait_core::json_support::{json, JsonValue};

#[test]
fn request_stage_emits_get_file_operation() {
    let planned = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "request",
        "message": {"message_id": 11, "chat": {"id": 123}},
        "attachment": {
            "kind": "voice",
            "telegram_file_id": "tg-voice-001",
            "telegram_file_unique_id": "unique-voice-001"
        },
        "cache_root": "/tmp/telegram-downloads"
    }))
    .expect("plan");

    assert_eq!(planned["execution_kind"], "telegram_file_download");
    assert_eq!(planned["should_execute"], true);
    let request = &planned["request"];
    assert_eq!(request["ok"], true);
    assert_eq!(request["telegram_file_id"], "tg-voice-001");
    assert_eq!(request["operations"][0]["kind"], "get_file");
    assert_eq!(request["operations"][0]["file_id"], "tg-voice-001");
}

#[test]
fn request_stage_fails_closed_without_file_id() {
    let planned = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "request",
        "message": {"message_id": 11, "chat": {"id": 123}},
        "attachment": {"kind": "voice"}
    }))
    .expect("plan");

    assert_eq!(planned["should_execute"], false);
    assert_eq!(planned["request"]["ok"], false);
    assert_eq!(
        planned["request"]["user_message"],
        "That Telegram attachment did not include a downloadable file id."
    );
    assert_eq!(planned["request"]["operation_count"], 0);
}

#[test]
fn file_info_stage_resolves_attachment_and_cache_check() {
    let planned = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "file_info",
        "request": {
            "message": {"message_id": 11, "chat": {"id": 123}},
            "attachment": {
                "kind": "audio",
                "telegram_file_id": "tg-audio-001",
                "telegram_file_unique_id": "unique/audio 001",
                "file_name": "../Demo Track.mp3"
            },
            "cache_root": "/tmp/telegram-downloads"
        },
        "file_info": {"file_path": "music/demo-track.mp3"}
    }))
    .expect("plan");

    let request = &planned["request"];
    assert_eq!(request["ok"], true);
    assert_eq!(
        request["attachment"]["telegram_file_path"],
        "music/demo-track.mp3"
    );
    assert_eq!(request["attachment"]["file_name"], "../Demo Track.mp3");
    assert_eq!(
        request["local_path"],
        "/tmp/telegram-downloads/123/11/unique_audio_001-Demo_Track.mp3"
    );
    assert_eq!(request["operations"][0]["kind"], "check_cache");
    assert_eq!(
        request["operations"][0]["local_path"],
        request["local_path"]
    );
}

#[test]
fn file_info_stage_defaults_photo_name_and_mime() {
    let planned = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "file_info",
        "request": {
            "message": {"message_id": 12, "chat": {"id": 456}},
            "attachment": {
                "kind": "photo",
                "telegram_file_id": "tg-photo-002",
                "telegram_file_unique_id": "unique-large"
            },
            "cache_root": "/tmp/telegram-downloads"
        },
        "file_info": {"file_path": "photos/file_22.jpg"}
    }))
    .expect("plan");

    let attachment = &planned["request"]["attachment"];
    assert_eq!(attachment["file_name"], "file_22.jpg");
    assert_eq!(attachment["mime_type"], "image/jpeg");
    assert_eq!(
        planned["request"]["local_path"],
        "/tmp/telegram-downloads/456/12/unique-large-file_22.jpg"
    );
}

#[test]
fn cache_stage_emits_download_only_on_cache_miss() {
    let miss = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "cache",
        "request": {
            "attachment": {"telegram_file_path": "voice/file_31.ogg"},
            "telegram_file_path": "voice/file_31.ogg",
            "local_path": "/tmp/cache/voice.ogg"
        },
        "local_path_exists": false
    }))
    .expect("plan");
    assert_eq!(miss["should_execute"], true);
    assert_eq!(
        miss["request"]["operations"][0]["kind"],
        "download_file_bytes"
    );
    assert_eq!(
        miss["request"]["operations"][0]["file_path"],
        "voice/file_31.ogg"
    );

    let hit = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "cache",
        "request": {
            "attachment": {"telegram_file_path": "voice/file_31.ogg"},
            "telegram_file_path": "voice/file_31.ogg",
            "local_path": "/tmp/cache/voice.ogg"
        },
        "local_path_exists": true
    }))
    .expect("plan");
    assert_eq!(hit["should_execute"], false);
    assert_eq!(
        hit["request"]["operations"].as_array().unwrap(),
        &Vec::<JsonValue>::new()
    );
    assert_eq!(hit["request"]["cache_hit"], true);
}

#[test]
fn cache_stage_uses_cache_hit_alias_and_attachment_path() {
    let planned = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "cache",
        "attachment": {"telegram_file_path": "photos/file_44.jpg"},
        "local_path": "/tmp/cache/photo.jpg",
        "cache_hit": "no",
    }))
    .expect("plan");

    assert_eq!(planned["should_execute"], true);
    assert_eq!(planned["request"]["cache_hit"], false);
    assert_eq!(
        planned["request"]["operations"][0]["telegram_file_path"],
        "photos/file_44.jpg"
    );
    assert_eq!(
        planned["request"]["operations"][0]["local_path"],
        "/tmp/cache/photo.jpg"
    );
}

#[test]
fn result_stage_normalizes_attachment_local_path_and_errors() {
    let ok = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "result",
        "request": {
            "attachment": {"kind": "voice", "telegram_file_path": "voice/file_31.ogg"},
            "local_path": "/tmp/cache/voice.ogg"
        },
        "callback_result": {
            "ok": true,
            "local_path": "/tmp/cache/voice.ogg",
            "downloaded": true,
            "operation_results": [
                {"kind": "download_file_bytes", "ok": true}
            ]
        }
    }))
    .expect("plan");
    assert_eq!(ok["completed"], true);
    assert_eq!(
        ok["result"]["attachment"]["local_path"],
        "/tmp/cache/voice.ogg"
    );
    assert_eq!(ok["result"]["downloaded"], true);

    let failed = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "result",
        "request": {
            "attachment": {"kind": "voice"},
            "local_path": "/tmp/cache/voice.ogg"
        },
        "callback_result": {
            "operation_results": [
                {"kind": "download_file_bytes", "ok": false, "error": "network down"}
            ]
        }
    }))
    .expect("plan");
    assert_eq!(failed["completed"], false);
    assert_eq!(failed["result"]["error"], "network down");
    assert_eq!(
        failed["result"]["user_message"],
        "Telegram file download failed. Please retry in a moment."
    );
}

#[test]
fn result_stage_builds_callback_payload_from_execution_request_and_operations() {
    let planned = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "result",
        "execution_request": {
            "attachment": {
                "kind": "voice",
                "telegram_file_path": "voice/file_31.ogg"
            },
            "local_path": "/tmp/cache/voice.ogg",
            "cache_hit": false,
            "operations": [
                {
                    "kind": "download_file_bytes",
                    "file_path": "voice/file_31.ogg",
                    "local_path": "/tmp/cache/voice.ogg"
                }
            ]
        },
        "operation_results": [
            {
                "index": 0,
                "kind": "download_file_bytes",
                "ok": true,
                "telegram_file_path": "voice/file_31.ogg",
                "local_path": "/tmp/cache/voice.ogg"
            }
        ]
    }))
    .expect("plan");

    assert_eq!(planned["completed"], true);
    assert_eq!(planned["result"]["ok"], true);
    assert_eq!(planned["result"]["cache_hit"], false);
    assert_eq!(planned["result"]["downloaded"], true);
    assert_eq!(planned["result"]["operation_count"], 1);
    assert_eq!(planned["result"]["local_path"], "/tmp/cache/voice.ogg");
    assert_eq!(
        planned["result"]["attachment"]["local_path"],
        "/tmp/cache/voice.ogg"
    );
    assert_eq!(
        planned["result"]["attachment"]["telegram_file_path"],
        "voice/file_31.ogg"
    );
}

#[test]
fn result_stage_preserves_cache_hit_from_execution_request_without_callback_result() {
    let planned = agent_telegram_file_download_execution_plan_json(&json!({
        "stage": "result",
        "execution_request": {
            "attachment": {
                "kind": "photo",
                "telegram_file_path": "photos/file_44.jpg"
            },
            "local_path": "/tmp/cache/photo.jpg",
            "cache_hit": true,
            "operations": []
        },
        "operation_results": []
    }))
    .expect("plan");

    assert_eq!(planned["completed"], true);
    assert_eq!(planned["result"]["cache_hit"], true);
    assert_eq!(planned["result"]["downloaded"], false);
    assert_eq!(planned["result"]["operation_count"], 0);
    assert_eq!(planned["result"]["local_path"], "/tmp/cache/photo.jpg");
}

#[test]
fn file_download_errors_match_public_contract() {
    assert_eq!(
        agent_telegram_file_download_execution_plan_json(&json!("bad request")).unwrap_err(),
        "request must be a JSON object"
    );
    assert_eq!(
        agent_telegram_file_download_execution_plan_json(&json!({
            "stage": "unknown"
        }))
        .unwrap_err(),
        "unsupported Telegram file download execution stage `unknown`"
    );
}

#[test]
fn file_download_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramFileDownloadPlanner = &DefaultTelegramFileDownloadPlanner;
    let planned = planner
        .plan_json(&json!({
            "stage": "request",
            "attachment": {
                "telegram_file_id": "tg-file-001"
            },
        }))
        .unwrap();

    assert_eq!(planned["stage"], "request");
    assert_eq!(planned["execution_kind"], EXECUTION_KIND);
    assert_eq!(planned["should_execute"], true);
    assert_eq!(planned["request"]["operations"][0]["kind"], "get_file");
}

#[test]
fn file_download_bound_entrypoint_accepts_substitute_planner() {
    struct StubFileDownloadPlanner;

    impl TelegramFileDownloadPlanner for StubFileDownloadPlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": "stubbed",
                "attachment_seen": request.get("attachment").is_some(),
            }))
        }
    }

    let planned = plan_with_telegram_file_download_planner(
        &StubFileDownloadPlanner,
        &json!({"attachment": {"telegram_file_id": "tg-file-001"}}),
    )
    .unwrap();

    assert_eq!(planned["stage"], "stubbed");
    assert_eq!(planned["attachment_seen"], true);
}
