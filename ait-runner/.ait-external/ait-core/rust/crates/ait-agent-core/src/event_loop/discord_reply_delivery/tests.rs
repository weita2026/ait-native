use super::*;

#[test]
fn plans_direct_channel_delivery_without_event_history() {
    let planned = agent_discord_reply_delivery_execution_plan_json(&json!({
        "stage": "request",
        "execution_request": {
            "reply_mode": "channel_message",
            "channel_id": "C-1",
            "reply_text": "hello",
            "assistant_sequence": 4
        }
    }))
    .expect("plan");
    assert_eq!(planned["should_execute"], true);
    assert_eq!(
        planned["request"]["operations"][0]["kind"],
        "send_channel_message"
    );
    assert!(!planned.to_string().contains("session"));
    assert!(!planned.to_string().contains("events"));
}

#[test]
fn successful_results_complete_without_post_operations() {
    let planned = agent_discord_reply_delivery_execution_plan_json(&json!({
        "stage": "result",
        "callback_result": {
            "reply_mode": "channel_message",
            "assistant_sequence": 4,
            "operation_count": 1,
            "operation_results": [{"ok": true, "delivered": true, "message_id": "M-1"}]
        }
    }))
    .expect("plan");
    assert_eq!(planned["completed"], true);
    assert_eq!(planned["requires_post_operations"], false);
    assert_eq!(planned["result"]["message_ids"], json!(["M-1"]));
}

#[test]
fn attachment_failure_plans_and_accepts_sessionless_text_fallback() {
    let failed = json!({
        "reply_mode": "interaction",
        "assistant_sequence": 4,
        "attachments": [{"local_path": "artifacts/report.md", "file_name": "report.md"}],
        "operation_count": 2,
        "operation_results": [
            {"kind": "edit_original_response", "ok": true, "delivered": true, "message_id": "M-1"},
            {"kind": "send_followup_attachment", "ok": false, "delivered": false, "attachment_index": 0, "error": "missing local file"}
        ]
    });
    let planned = agent_discord_reply_delivery_execution_plan_json(&json!({
        "stage": "result",
        "callback_result": failed,
    }))
    .expect("fallback plan");
    assert_eq!(planned["completed"], false);
    assert_eq!(planned["requires_post_operations"], true);
    assert_eq!(
        planned["result"]["post_operations"][0]["kind"],
        "send_followup"
    );
    assert!(planned["result"]["post_operations"][0]["text"]
        .as_str()
        .is_some_and(|text| text.contains("report.md") && text.contains("missing local file")));
    assert!(!planned.to_string().contains("session"));

    let mut recovered = planned["result"].clone();
    recovered["post_operation_results"] = json!([{
        "kind": "send_followup",
        "ok": true,
        "delivered": true,
        "message_id": "M-2"
    }]);
    let completed = agent_discord_reply_delivery_execution_plan_json(&json!({
        "stage": "result",
        "callback_result": recovered,
    }))
    .expect("completed fallback");
    assert_eq!(completed["completed"], true);
    assert_eq!(completed["requires_post_operations"], false);
    assert_eq!(completed["result"]["message_ids"], json!(["M-1", "M-2"]));
}
