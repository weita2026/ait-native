use super::*;
use ait_core::json_support::json;

#[test]
fn get_updates_request_builds_url_and_poll_timeout() {
    let planned = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "request",
        "operation": "get_updates",
        "bot_token": "test-token",
        "offset": 3,
        "timeout_seconds": 5,
        "request_timeout_seconds": null
    }))
    .expect("plan");

    let request = &planned["request"];
    assert_eq!(request["ok"], true);
    assert_eq!(request["transport"], "json");
    assert_eq!(request["method"], "GET");
    assert_eq!(
        request["url"],
        "https://api.telegram.org/bottest-token/getUpdates?offset=3&timeout=5&allowed_updates=%5B%22message%22%5D"
    );
    assert_eq!(request["timeout"], 15.0);
}

#[test]
fn get_updates_keeps_larger_explicit_timeout() {
    let planned = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "request",
        "operation": "get_updates",
        "base_url": "https://api.telegram.org/bottest-token",
        "offset": 3,
        "timeout_seconds": 5,
        "request_timeout_seconds": 30.0
    }))
    .expect("plan");

    assert_eq!(planned["request"]["timeout"], 30.0);
}

#[test]
fn send_message_request_builds_json_payload() {
    let planned = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "request",
        "operation": "send_message",
        "bot_token": "test-token",
        "chat_id": 123,
        "text": "<b>Hello</b>",
        "parse_mode": "HTML",
        "request_timeout_seconds": null
    }))
    .expect("plan");

    let request = &planned["request"];
    assert_eq!(
        request["url"],
        "https://api.telegram.org/bottest-token/sendMessage"
    );
    assert_eq!(request["method"], "POST");
    assert_eq!(request["payload"]["chat_id"], 123);
    assert_eq!(request["payload"]["text"], "<b>Hello</b>");
    assert_eq!(request["payload"]["parse_mode"], "HTML");
    assert_eq!(request["payload"]["disable_web_page_preview"], true);
    assert_eq!(request["timeout"], json!(null));
}

#[test]
fn send_attachment_with_telegram_file_id_uses_json_payload() {
    let planned = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "request",
        "operation": "send_attachment",
        "bot_token": "test-token",
        "method_name": "sendDocument",
        "file_field": "document",
        "chat_id": 123,
        "attachment": {
            "telegram_file_id": "tg-doc-123",
            "file_name": "report.pdf",
            "mime_type": "application/pdf"
        }
    }))
    .expect("plan");

    let request = &planned["request"];
    assert_eq!(request["transport"], "json");
    assert_eq!(
        request["url"],
        "https://api.telegram.org/bottest-token/sendDocument"
    );
    assert_eq!(
        request["payload"],
        json!({"chat_id": 123, "document": "tg-doc-123"})
    );
}

#[test]
fn send_attachment_with_local_path_uses_multipart_plan() {
    let planned = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "request",
        "operation": "send_attachment",
        "bot_token": "test-token",
        "method_name": "sendAudio",
        "file_field": "audio",
        "chat_id": 123,
        "attachment": {
            "local_path": "/tmp/demo.mp3",
            "caption": "demo caption",
            "title": "Demo",
            "performer": "AI",
            "duration_seconds": 42
        }
    }))
    .expect("plan");

    let request = &planned["request"];
    assert_eq!(request["transport"], "multipart");
    assert_eq!(request["fields"]["caption"], "demo caption");
    assert_eq!(request["fields"]["title"], "Demo");
    assert_eq!(request["fields"]["performer"], "AI");
    assert_eq!(request["fields"]["duration"], 42);
    assert_eq!(request["file_field"], "audio");
    assert_eq!(request["file_name"], "demo.mp3");
    assert_eq!(request["mime_type"], "audio/mpeg");
    assert_eq!(request["local_path"], "/tmp/demo.mp3");
}

#[test]
fn download_file_request_requires_file_path() {
    let planned = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "request",
        "operation": "download_file",
        "bot_token": "test-token",
        "file_path": ""
    }))
    .expect("plan");

    assert_eq!(planned["should_execute"], false);
    assert_eq!(
        planned["request"]["error"],
        "Telegram file download requires a file_path."
    );
}

#[test]
fn result_stage_normalizes_updates_and_file_info() {
    let updates = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "result",
        "operation": "get_updates",
        "payload": {"ok": true, "result": [{"update_id": 7}]}
    }))
    .expect("plan");
    assert_eq!(updates["completed"], true);
    assert_eq!(updates["result"]["updates"], json!([{"update_id": 7}]));

    let file_info = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "result",
        "operation": "get_file",
        "payload": {"ok": true, "result": {"file_path": "voice/file.ogg"}}
    }))
    .expect("plan");
    assert_eq!(file_info["completed"], true);
    assert_eq!(
        file_info["result"]["file_info"]["file_path"],
        "voice/file.ogg"
    );
}

#[test]
fn result_stage_accepts_direct_payload_with_execution_request() {
    let planned = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "result",
        "execution_request": {
            "operation": "get_updates",
            "telegram_method": "getUpdates"
        },
        "payload": {"ok": true, "result": [{"update_id": 9}]}
    }))
    .expect("plan");

    assert_eq!(planned["completed"], true);
    assert_eq!(planned["result"]["operation"], "get_updates");
    assert_eq!(planned["result"]["telegram_method"], "getUpdates");
    assert_eq!(planned["result"]["updates"], json!([{"update_id": 9}]));
    assert_eq!(
        planned["result"]["payload"],
        json!({"ok": true, "result": [{"update_id": 9}]})
    );
}

#[test]
fn api_method_defaults_aliases_and_error_contract_are_stable() {
    let alias = agent_telegram_api_method_execution_plan_json(&json!({
        "execution_request": {
            "method_kind": "getUpdates",
            "base_url": "https://api.telegram.org/bottest-token/",
            "offset": "8",
            "poll_timeout_seconds": true,
            "request_timeout_seconds": 5.0
        }
    }))
    .expect("plan");
    assert_eq!(alias["stage"], "request");
    assert_eq!(alias["should_execute"], true);
    assert_eq!(alias["request"]["operation"], "get_updates");
    assert_eq!(alias["request"]["timeout"], 11.0);
    assert_eq!(
        alias["request"]["url"],
        "https://api.telegram.org/bottest-token/getUpdates?offset=8&timeout=1&allowed_updates=%5B%22message%22%5D"
    );

    let unknown = agent_telegram_api_method_execution_plan_json(&json!({})).expect("plan");
    assert_eq!(unknown["stage"], "request");
    assert_eq!(unknown["should_execute"], false);
    assert_eq!(unknown["request"]["operation"], "unknown");
    assert_eq!(
        unknown["request"]["error"],
        "unsupported Telegram API method operation `unknown`"
    );

    let invalid = agent_telegram_api_method_execution_plan_json(&json!("bad"));
    assert_eq!(invalid.unwrap_err(), "request must be a JSON object");

    let unsupported_stage = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "unknown"
    }));
    assert_eq!(
        unsupported_stage.unwrap_err(),
        "unsupported Telegram API method execution stage `unknown`"
    );
}

#[test]
fn api_method_result_failure_contract_is_stable() {
    let failed = agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "result",
        "operation": "sendMessage",
        "payload": {"ok": false, "description": "bad request"}
    }))
    .expect("plan");

    assert_eq!(failed["completed"], false);
    assert_eq!(failed["result"]["operation"], "send_message");
    assert_eq!(failed["result"]["telegram_method"], "sendMessage");
    assert_eq!(failed["result"]["ok"], false);
    assert_eq!(failed["result"]["value"], json!(null));
    let error = failed["result"]["error"].as_str().unwrap();
    assert!(error.starts_with("Telegram sendMessage failed: "));
    assert!(error.contains("\"ok\":false"));
    assert!(error.contains("\"description\":\"bad request\""));
}

#[test]
fn api_method_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramApiMethodPlanner = &DefaultTelegramApiMethodPlanner;
    let planned = planner
        .plan_json(&json!({
            "stage": "request",
            "operation": "send_message",
            "bot_token": "test-token",
            "chat_id": 123,
            "text": "hello"
        }))
        .expect("plan");

    assert_eq!(planned["stage"], "request");
    assert_eq!(planned["should_execute"], true);
    assert_eq!(planned["request"]["operation"], "send_message");
    assert_eq!(
        planned["request"]["url"],
        "https://api.telegram.org/bottest-token/sendMessage"
    );
}

#[test]
fn api_method_bound_entrypoint_accepts_substitute_planner() {
    struct StubApiMethodPlanner;

    impl TelegramApiMethodPlanner for StubApiMethodPlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": "stubbed",
                "operation_seen": request.get("operation").cloned().unwrap_or(JsonValue::Null),
            }))
        }
    }

    let planned = plan_with_telegram_api_method_planner(
        &StubApiMethodPlanner,
        &json!({"operation": "send_message"}),
    )
    .expect("plan");

    assert_eq!(planned["stage"], "stubbed");
    assert_eq!(planned["operation_seen"], "send_message");
}
