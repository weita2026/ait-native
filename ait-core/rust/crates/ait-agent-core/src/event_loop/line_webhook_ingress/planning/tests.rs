use super::*;

fn signed_request(raw_payload: &str) -> JsonValue {
    json!({
        "raw_payload": raw_payload,
        "signature": build_line_signature(raw_payload, "line-channel-secret").unwrap(),
        "channel_secret": "line-channel-secret",
    })
}

#[test]
fn webhook_ingress_verifies_signature_and_plans_text_event() {
    let raw_payload = r#"{"destination":"U-bot","events":[{"type":"message","replyToken":"reply-token-1","webhookEventId":"01HXLINEEVENT001","timestamp":1714990000000,"mode":"active","deliveryContext":{"isRedelivery":true},"source":{"type":"user","userId":"U-user-1"},"message":{"id":"987654321","type":"text","text":"Hello from LINE"}}]}"#;
    let planned = agent_line_webhook_ingress_plan_json(&signed_request(raw_payload)).unwrap();

    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(
        planned["line_webhook_ingress_contract"],
        WEBHOOK_INGRESS_CONTRACT
    );
    assert_eq!(planned["webhook_ingress_state"], "accepted");
    assert_eq!(planned["python_signature_verification_allowed"], false);
    assert_eq!(planned["python_json_parsing_allowed"], false);
    assert_eq!(planned["python_event_planning_allowed"], false);
    assert_eq!(planned["signature_verified"], true);
    assert_eq!(planned["event_count"], 1);
    assert_eq!(planned["accepted_event_count"], 1);

    let event = &planned["event_plans"][0];
    assert_eq!(event["event_ingress_state"], "text_message_ready");
    assert_eq!(event["channel_id"], "U-user-1");
    assert_eq!(event["channel_title"], "LINE user · U-user-1");
    assert_eq!(event["actor_identity"], "line:U-user-1");
    assert_eq!(event["webhook_event_id"], "01HXLINEEVENT001");
    assert_eq!(event["pending_reply"]["reply_token"], "reply-token-1");
    assert_eq!(event["pending_reply"]["text"], "Hello from LINE");
    assert_eq!(event["transport_envelope"]["transport"], "line");
    assert_eq!(event["transport_envelope"]["event_id"], "01HXLINEEVENT001");
    assert_eq!(
        event["transport_envelope"]["metadata"]["is_redelivery"],
        true
    );
    assert_eq!(
        event["transport_envelope"]["message"]["message_id"],
        987654321
    );
}

#[test]
fn webhook_ingress_ignores_non_text_events_without_python_planning() {
    let raw_payload = r#"{"events":[{"type":"follow","source":{"type":"user","userId":"U-user-1"}},{"type":"message","source":{"type":"user","userId":"U-user-1"},"message":{"type":"image","id":"m-1"}}]}"#;
    let planned = agent_line_webhook_ingress_plan_json(&signed_request(raw_payload)).unwrap();

    assert_eq!(planned["event_count"], 2);
    assert_eq!(planned["accepted_event_count"], 0);
    assert_eq!(
        planned["event_plans"][0]["event_ingress_state"],
        "ignored_unsupported_event_type"
    );
    assert_eq!(
        planned["event_plans"][1]["event_ingress_state"],
        "ignored_unsupported_message_type"
    );
    assert_eq!(planned["event_plans"][1]["should_submit_turn"], false);
}

#[test]
fn webhook_ingress_fails_closed_for_signature_and_payload_errors() {
    let missing_signature = agent_line_webhook_ingress_plan_json(&json!({
        "raw_payload": "{\"events\":[]}",
        "channel_secret": "line-channel-secret",
    }))
    .unwrap();
    assert_eq!(
        missing_signature["webhook_ingress_state"],
        "missing_signature"
    );
    assert_eq!(missing_signature["http_status"], 401);
    assert_eq!(missing_signature["should_handle_webhook"], false);

    let invalid_signature = agent_line_webhook_ingress_plan_json(&json!({
        "raw_payload": "{\"events\":[]}",
        "signature": "bad-signature",
        "channel_secret": "line-channel-secret",
    }))
    .unwrap();
    assert_eq!(
        invalid_signature["webhook_ingress_state"],
        "invalid_signature"
    );
    assert_eq!(invalid_signature["signature_verified"], false);

    let invalid_payload = agent_line_webhook_ingress_plan_json(&json!({
        "raw_payload": "{}",
        "signature": build_line_signature("{}", "line-channel-secret").unwrap(),
        "channel_secret": "line-channel-secret",
    }))
    .unwrap();
    assert_eq!(invalid_payload["webhook_ingress_state"], "missing_events");
    assert_eq!(
        invalid_payload["error"],
        "LINE webhook payload must include an events list."
    );
}

#[test]
fn webhook_ingress_fallback_event_id_is_deterministic_when_now_is_supplied() {
    let raw_payload = r#"{"events":[{"type":"message","source":{"type":"group","groupId":"G-1","userId":"U-1"},"message":{"id":"m-1","type":"text","text":"hi"}}]}"#;
    let mut request = signed_request(raw_payload);
    request["now_iso"] = json!("2026-07-03T08:00:00+00:00");
    let planned = agent_line_webhook_ingress_plan_json(&request).unwrap();

    assert_eq!(
        planned["event_plans"][0]["webhook_event_id"],
        "line:G-1:m-1:2026-07-03T08:00:00+00:00"
    );
    assert_eq!(
        planned["event_plans"][0]["transport_envelope"]["channel"]["channel_id"],
        "G-1"
    );
}

#[test]
fn webhook_ingress_rejects_public_request_shape_errors() {
    assert_eq!(
        agent_line_webhook_ingress_plan_json(&json!("bad request")).unwrap_err(),
        "LINE webhook ingress request must be an object."
    );
    let missing_config = agent_line_webhook_ingress_plan_json(&json!({})).unwrap();
    assert_eq!(
        missing_config["webhook_ingress_state"],
        "missing_channel_secret"
    );
    assert_eq!(missing_config["http_status"], 400);
}

#[test]
fn webhook_ingress_bound_entrypoint_accepts_substitute_planner() {
    struct StubLineWebhookIngressPlanner;

    impl LineWebhookIngressPlanner for StubLineWebhookIngressPlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": "stubbed",
                "raw_payload_seen": request.get("raw_payload").is_some(),
            }))
        }
    }

    let planned = plan_with_line_webhook_ingress_planner(
        &StubLineWebhookIngressPlanner,
        &json!({"raw_payload": "{}"}),
    )
    .unwrap();

    assert_eq!(planned["stage"], "stubbed");
    assert_eq!(planned["raw_payload_seen"], true);
}
