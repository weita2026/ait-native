use super::*;

#[test]
fn submission_runtime_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramSubmissionRuntimePlanner = &DefaultTelegramSubmissionRuntimePlanner;

    let planned = planner
        .plan_json(&json!({
            "stage": "forget_future",
            "future_token": "F-1"
        }))
        .unwrap();

    assert_eq!(planned["stage"], "forget_future");
    assert_eq!(
        planned["submission_runtime_contract"],
        SUBMISSION_RUNTIME_CONTRACT
    );
}

#[test]
fn submission_runtime_bound_entrypoint_accepts_substitute_planner() {
    struct StubSubmissionRuntimePlanner;

    impl TelegramSubmissionRuntimePlanner for StubSubmissionRuntimePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": "stubbed",
                "observed_stage": request.get("stage").cloned().unwrap_or(JsonValue::Null),
            }))
        }
    }

    let planned = plan_with_telegram_submission_runtime_planner(
        &StubSubmissionRuntimePlanner,
        &json!({
            "stage": "forget_future",
            "future_token": "F-1",
        }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "stubbed");
    assert_eq!(planned["observed_stage"], "forget_future");
}

#[test]
fn submission_runtime_submit_update_uses_rust_dispatch_key_and_buffers_merge() {
    let planned = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_update",
        "update": {
            "update_id": 42,
            "message": {"message_id": 7, "chat": {"id": 99}, "text": "hi"}
        },
        "fallback_update_key": "memory-a",
        "logical_turn_merge_enabled": true,
    }))
    .unwrap();

    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(
        planned["submission_runtime_contract"],
        SUBMISSION_RUNTIME_CONTRACT
    );
    assert_eq!(planned["python_submission_allowed"], false);
    assert_eq!(planned["submission_state"], "planned");
    assert_eq!(planned["should_submit"], true);
    assert_eq!(planned["queue_key"], "chat-99");
    assert_eq!(planned["dispatch_item"]["update_key"], "update-42");
    assert_eq!(
        planned["actions"][0]["kind"],
        "buffer_submitted_text_update"
    );
    assert_eq!(planned["actions"][1]["kind"], "submit_serialized");
    assert_eq!(planned["actions"][1]["callback"], "handle_submitted_update");
    assert_eq!(planned["actions"][1]["queue_key"], "chat-99");
    assert_eq!(
        planned["details"]["should_buffer_submitted_text_update"],
        true
    );
}

#[test]
fn submission_runtime_defaults_aliases_and_error_contract_are_stable() {
    let default_plan = agent_telegram_submission_runtime_plan_json(&json!({
        "update": {
            "update_id": 43,
            "message": {"message_id": 8, "chat": {"id": 101}, "text": "hi"}
        }
    }))
    .unwrap();
    assert_eq!(default_plan["stage"], "submit_update");
    assert_eq!(default_plan["transport"], "telegram");
    assert_eq!(default_plan["rust_event_loop_required"], true);
    assert_eq!(default_plan["python_submission_allowed"], false);
    assert_eq!(default_plan["service_runtime_dispatch_port_required"], true);
    assert_eq!(default_plan["queue_key"], "chat-101");

    let alias = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_background_sync",
        "chat_id": 321,
    }))
    .unwrap();
    assert_eq!(alias["stage"], "submit_background_sync_for_chat");
    assert_eq!(alias["queue_key"], "chat-321");
    assert_eq!(alias["submit_action"]["args"], json!(["321"]));

    let invalid = agent_telegram_submission_runtime_plan_json(&json!("bad"));
    assert_eq!(invalid.unwrap_err(), "request must be a JSON object");

    let unsupported = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "unknown"
    }));
    assert_eq!(
        unsupported.unwrap_err(),
        "unsupported Telegram submission runtime stage: unknown"
    );
}

#[test]
fn submission_runtime_validation_and_forget_future_contracts_are_stable() {
    let missing_update = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_update"
    }));
    assert_eq!(missing_update.unwrap_err(), "update is required");

    let invalid_update = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_update",
        "update": "bad"
    }));
    assert_eq!(invalid_update.unwrap_err(), "update must be a JSON object");

    let missing_dispatch_item = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_planned_update",
        "update": {"message": {"chat": {"id": 10}}},
    }));
    assert_eq!(
        missing_dispatch_item.unwrap_err(),
        "dispatch_item is required"
    );

    let missing_dispatch_key = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_planned_update",
        "update": {"message": {"chat": {"id": 10}}},
        "dispatch_item": {}
    }));
    assert_eq!(
        missing_dispatch_key.unwrap_err(),
        "dispatch_item.dispatch_key is required"
    );

    let missing_chat = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_background_sync_for_chat",
        "chat_id": null
    }));
    assert_eq!(missing_chat.unwrap_err(), "chat_id is required");

    let missing_reply_queue = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_reply_serialized"
    }));
    assert_eq!(missing_reply_queue.unwrap_err(), "queue_key is required");

    let forget = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "forget_future",
        "future_token": {"id": "F-1"}
    }))
    .unwrap();
    assert_eq!(forget["stage"], "forget_future");
    assert_eq!(forget["should_submit"], false);
    assert_eq!(forget["submission_state"], "planned");
    assert_eq!(forget["actions"][0]["kind"], "forget_future");
    assert_eq!(forget["actions"][0]["future_token"]["id"], "F-1");
}

#[test]
fn submission_runtime_submit_planned_update_consumes_dispatch_item_key() {
    let planned = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_planned_update",
        "update": {"message": {"chat": {"id": 10}}},
        "dispatch_item": {
            "index": 0,
            "dispatch_key": "planned-chat-10",
            "update_key": "memory-a"
        },
    }))
    .unwrap();

    assert_eq!(planned["queue_key"], "planned-chat-10");
    assert_eq!(planned["dispatch_item"]["update_key"], "memory-a");
    assert_eq!(planned["actions"][0]["kind"], "submit_serialized");
    assert_eq!(planned["actions"][0]["queue_key"], "planned-chat-10");
}

#[test]
fn submission_runtime_rejects_post_stop_fallback_submissions() {
    let planned = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_planned_update",
        "service_runtime_stopped": true,
        "update": {"message": {"chat": {"id": 10}}},
        "dispatch_item": {"dispatch_key": "chat-10"},
    }))
    .unwrap();

    assert_eq!(planned["submission_state"], "rejected");
    assert_eq!(planned["should_submit"], false);
    assert_eq!(planned["actions"].as_array().unwrap().len(), 0);
    assert_eq!(
        planned["rejection_reasons"][0]["kind"],
        "service_runtime_stopped"
    );
}

#[test]
fn submission_runtime_background_sync_uses_chat_dispatch_queue() {
    let planned = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_background_sync_for_chat",
        "chat_id": "abc",
    }))
    .unwrap();

    assert_eq!(planned["stage"], "submit_background_sync_for_chat");
    assert_eq!(planned["queue_key"], "chat-abc");
    assert_eq!(
        planned["submit_action"]["callback"],
        "run_background_sync_for_chat"
    );
    assert_eq!(planned["submit_action"]["args"], json!(["abc"]));
}

#[test]
fn submission_runtime_reply_serialized_preserves_reply_queue_and_args() {
    let planned = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "submit_reply_serialized",
        "queue_key": "chat-99",
        "callback_slot": "send_reply",
        "args": ["a", 2],
    }))
    .unwrap();

    assert_eq!(planned["queue_key"], "chat-99");
    assert_eq!(planned["submit_action"]["kind"], "submit_reply_serialized");
    assert_eq!(planned["submit_action"]["callback"], "send_reply");
    assert_eq!(planned["submit_action"]["args"], json!(["a", 2]));
}

#[test]
fn submission_runtime_wait_for_idle_checks_service_before_live_replies() {
    let busy = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "wait_for_idle",
        "service_runtime_idle": false,
        "live_reply_manager_idle": true,
        "timeout_seconds": 0.5,
    }))
    .unwrap();
    assert_eq!(busy["idle"], false);
    assert_eq!(busy["checked_live_reply_manager"], false);
    assert_eq!(busy["actions"][0]["kind"], "wait_service_runtime_idle");
    assert_eq!(busy["actions"][1]["enabled"], false);

    let idle = agent_telegram_submission_runtime_plan_json(&json!({
        "stage": "wait_for_idle",
        "service_runtime_idle": true,
        "live_reply_manager_idle": true,
    }))
    .unwrap();
    assert_eq!(idle["idle"], true);
    assert_eq!(idle["checked_live_reply_manager"], true);
}
