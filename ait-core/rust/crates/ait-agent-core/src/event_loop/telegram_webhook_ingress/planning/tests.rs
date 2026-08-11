use super::*;

#[test]
fn webhook_ingress_accepts_object_and_plans_dispatch() {
    let planned = agent_telegram_webhook_ingress_plan_json(&json!({
        "raw_payload": r#"{"update_id": 42, "message": {"message_id": 7, "chat": {"id": 99}, "text": "hi"}}"#,
        "fallback_update_key_prefix": "webhook-main",
    }))
    .unwrap();

    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(planned["ingress_contract"], WEBHOOK_INGRESS_CONTRACT);
    assert_eq!(planned["ingress_state"], "accepted");
    assert_eq!(planned["update_count"], 1);
    assert_eq!(planned["dispatch_count"], 1);
    assert_eq!(planned["last_update_id"], 42);
    assert_eq!(planned["should_update_last_update_id"], true);
    assert_eq!(planned["python_ingress_allowed"], false);
    assert_eq!(planned["fallback_update_keys"][0], "webhook-main-42");
    assert_eq!(planned["dispatch_items"][0]["update_key"], "update-42");
}

#[test]
fn webhook_ingress_accepts_update_array_and_preserves_fallback_keys() {
    let planned = agent_telegram_webhook_ingress_plan_json(&json!({
        "raw_payload": r#"[{"message": {"message_id": 10}}, {"update_id": 2, "callback_query": {"id": "cb"}}]"#,
    }))
    .unwrap();

    assert_eq!(planned["update_count"], 2);
    assert_eq!(
        planned["fallback_update_keys"],
        json!(["webhook-0", "webhook-2"])
    );
    assert_eq!(planned["dispatch_items"][0]["update_key"], "message-10");
    assert_eq!(planned["dispatch_items"][1]["update_key"], "update-2");
    assert_eq!(planned["last_update_id"], 2);
}

#[test]
fn webhook_ingress_rejects_python_parser_error_shapes() {
    assert_eq!(
        agent_telegram_webhook_ingress_plan_json(&json!({"raw_payload": "  "})).unwrap_err(),
        "No Telegram webhook payload provided on stdin."
    );
    assert_eq!(
        agent_telegram_webhook_ingress_plan_json(&json!({"raw_payload": "5"})).unwrap_err(),
        "Telegram webhook payload must be a JSON object or array."
    );
    assert_eq!(
        agent_telegram_webhook_ingress_plan_json(&json!({"raw_payload": "[1]"})).unwrap_err(),
        "Telegram webhook update payload item #0 must be a JSON object."
    );
    assert_eq!(
        agent_telegram_webhook_ingress_plan_json(&json!({"raw_payload": "{"})).unwrap_err(),
        "Telegram webhook payload must be valid JSON."
    );
}

#[test]
fn webhook_ingress_rejects_public_request_shape_errors() {
    assert_eq!(
        agent_telegram_webhook_ingress_plan_json(&json!("bad request")).unwrap_err(),
        "request must be a JSON object"
    );
    assert_eq!(
        agent_telegram_webhook_ingress_plan_json(&json!({})).unwrap_err(),
        "raw_payload is required"
    );
}

#[test]
fn webhook_ingress_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramWebhookIngressPlanner = &DefaultTelegramWebhookIngressPlanner;
    let planned = planner
        .plan_json(&json!({
            "raw_payload": r#"{"update_id": 77, "message": {"message_id": 9}}"#,
        }))
        .unwrap();
    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(planned["last_update_id"], 77);
}

#[test]
fn webhook_ingress_bound_entrypoint_accepts_substitute_planner() {
    struct StubWebhookIngressPlanner;

    impl TelegramWebhookIngressPlanner for StubWebhookIngressPlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": "stubbed",
                "raw_payload_seen": request.get("raw_payload").is_some(),
            }))
        }
    }

    let planned = plan_with_telegram_webhook_ingress_planner(
        &StubWebhookIngressPlanner,
        &json!({"raw_payload": "{}"}),
    )
    .unwrap();

    assert_eq!(planned["stage"], "stubbed");
    assert_eq!(planned["raw_payload_seen"], true);
}
