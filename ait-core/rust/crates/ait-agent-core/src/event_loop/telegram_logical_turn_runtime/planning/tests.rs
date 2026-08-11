use super::*;

#[test]
fn logical_turn_candidate_metadata_extracts_text_and_chat() {
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "candidate_metadata",
        "update": {
            "update_id": 1,
            "message": {
                "message_id": 7,
                "chat": {"id": 99},
                "text": " hi ",
            }
        }
    }))
    .unwrap();

    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(
        planned["logical_turn_runtime_contract"],
        LOGICAL_TURN_RUNTIME_CONTRACT
    );
    assert_eq!(planned["python_logical_turn_allowed"], false);
    assert_eq!(planned["is_text_candidate"], true);
    assert_eq!(planned["raw_text"], " hi ");
    assert_eq!(planned["chat_id"], 99);
    assert_eq!(planned["telegram_message_id"], 7);
}

#[test]
fn logical_turn_merge_enabled_contract_normalizes_thresholds() {
    let disabled = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "merge_enabled",
        "merge_window_seconds": "0",
        "max_messages": "4",
    }))
    .unwrap();

    assert_eq!(disabled["stage"], "merge_enabled");
    assert_eq!(disabled["logical_turn_state"], "disabled");
    assert_eq!(disabled["merge_enabled"], false);
    assert_eq!(disabled["max_messages"], 4);

    let enabled = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "logical_turn_merge_enabled",
        "merge_window_seconds": "1.5",
        "max_messages": "2",
    }))
    .unwrap();

    assert_eq!(enabled["stage"], "merge_enabled");
    assert_eq!(enabled["logical_turn_state"], "enabled");
    assert_eq!(enabled["merge_enabled"], true);
    assert_eq!(enabled["merge_window_seconds"], 1.5);
    assert_eq!(enabled["actions"].as_array().unwrap().len(), 0);
}

#[test]
fn logical_turn_default_planner_satisfies_trait_entrypoint() {
    let planner: &dyn TelegramLogicalTurnRuntimePlanner = &DefaultTelegramLogicalTurnRuntimePlanner;
    let planned = planner
        .plan_json(&json!({
            "stage": "merge_enabled",
            "merge_window_seconds": 1.0,
            "max_messages": 2,
        }))
        .unwrap();

    assert_eq!(planned["logical_turn_state"], "enabled");
    assert_eq!(
        planned["logical_turn_runtime_contract"],
        LOGICAL_TURN_RUNTIME_CONTRACT
    );
}

#[test]
fn logical_turn_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl TelegramLogicalTurnRuntimePlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "logical_turn_state": "substitute",
            }))
        }
    }

    let planned = plan_with_telegram_logical_turn_runtime_planner(
        &SubstitutePlanner,
        &json!({ "stage": "merge_enabled" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "merge_enabled");
    assert_eq!(planned["logical_turn_state"], "substitute");
}

#[test]
fn logical_turn_classification_returns_candidate_payload() {
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "classify_pending_text_update",
        "update": {
            "update_id": 1,
            "message": {
                "message_id": 7,
                "chat": {"id": 99},
                "text": " hi ",
            }
        },
        "update_key": "update-1",
        "chat_key": "chat-99",
        "normalized_text": "hi",
        "actor_identity": "telegram:456:@weita",
        "received_at": 12.25,
        "command": null,
        "workflow_query": null,
    }))
    .unwrap();

    assert_eq!(planned["stage"], "classify_pending_text_update");
    assert_eq!(planned["logical_turn_state"], "classified_candidate");
    assert_eq!(planned["mergeable"], true);
    assert_eq!(planned["candidate"]["update_key"], "update-1");
    assert_eq!(planned["candidate"]["chat_key"], "chat-99");
    assert_eq!(planned["candidate"]["normalized_text"], "hi");
    assert_eq!(planned["candidate"]["mergeable"], true);
    assert_eq!(
        planned["candidate"]["actor_identity"],
        "telegram:456:@weita"
    );
    assert_eq!(planned["candidate"]["received_at"], 12.25);
    assert_eq!(planned["candidate"]["telegram_message_id"], 7);
}

#[test]
fn logical_turn_classification_marks_commands_non_mergeable() {
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "classify_pending_text_update",
        "update": {
            "update_id": 1,
            "message": {
                "message_id": 7,
                "chat": {"id": 99},
                "text": "/status",
            }
        },
        "update_key": "update-1",
        "chat_key": "chat-99",
        "normalized_text": "/status",
        "actor_identity": "telegram:456:@weita",
        "received_at": 12.25,
        "command": ["status", ""],
        "workflow_query": null,
    }))
    .unwrap();

    assert_eq!(planned["mergeable"], false);
    assert_eq!(planned["command_present"], true);
    assert_eq!(planned["candidate"]["mergeable"], false);
}

#[test]
fn logical_turn_buffer_plan_rejects_duplicate_update_keys() {
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "buffer_submitted_text_update",
        "candidate": {
            "chat_key": "chat-1",
            "update_key": "update-1",
        },
        "queue": [
            {"chat_key": "chat-1", "update_key": "update-1"}
        ],
    }))
    .unwrap();

    assert_eq!(planned["logical_turn_state"], "duplicate");
    assert_eq!(planned["should_append"], false);
    assert_eq!(planned["actions"].as_array().unwrap().len(), 0);
}

#[test]
fn logical_turn_discard_plan_removes_only_the_matching_buffered_update() {
    let candidate = json!({"chat_key": "chat-1", "update_key": "update-2"});
    let queue = json!([
        {"chat_key": "chat-1", "update_key": "update-1"},
        {"chat_key": "chat-1", "update_key": "update-2"}
    ]);
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "discard_buffered_text_update",
        "candidate": candidate,
        "queue": queue,
    }))
    .unwrap();

    assert_eq!(planned["logical_turn_state"], "discard");
    assert_eq!(planned["should_remove"], true);
    assert_eq!(planned["current_index"], 1);
    assert_eq!(planned["actions"].as_array().unwrap().len(), 1);
    assert_eq!(planned["actions"][0]["kind"], "discard_pending_text_update");

    let missing = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "discard_buffered_text_update",
        "candidate": {"chat_key": "chat-1", "update_key": "update-3"},
        "queue": queue,
    }))
    .unwrap();
    assert_eq!(missing["logical_turn_state"], "missing");
    assert_eq!(missing["should_remove"], false);
    assert!(missing["actions"].as_array().unwrap().is_empty());
}

#[test]
fn logical_turn_claim_waits_for_quiet_window() {
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "claim_logical_turn",
        "candidate": {
            "chat_key": "chat-1",
            "update_key": "update-1",
        },
        "queue": [
            {
                "chat_key": "chat-1",
                "update_key": "update-1",
                "normalized_text": "one",
                "mergeable": true,
                "actor_identity": "user:1",
                "received_at": 10.0,
                "telegram_message_id": 1,
            }
        ],
        "merge_window_seconds": 1.0,
        "poll_interval_seconds": 0.25,
        "max_messages": 4,
        "now_monotonic_seconds": 10.2,
    }))
    .unwrap();

    assert_eq!(planned["logical_turn_state"], "wait");
    assert_eq!(planned["should_wait"], true);
    assert_eq!(planned["sleep_for_seconds"], 0.25);
    assert_eq!(planned["actions"][0]["kind"], "wait_for_quiet_window");
}

#[test]
fn logical_turn_claim_passes_through_non_mergeable_candidate() {
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "claim_logical_turn",
        "candidate": {
            "chat_key": "chat-1",
            "update_key": "update-1",
        },
        "queue": [
            {
                "chat_key": "chat-1",
                "update_key": "update-1",
                "normalized_text": "/status",
                "mergeable": false,
                "actor_identity": "user:1",
                "received_at": 10.0,
                "telegram_message_id": 1,
            }
        ],
        "merge_window_seconds": 1.0,
        "poll_interval_seconds": 0.25,
        "max_messages": 4,
        "now_monotonic_seconds": 10.2,
    }))
    .unwrap();

    assert_eq!(planned["logical_turn_state"], "non_mergeable");
    assert_eq!(planned["return_kind"], "pass_through");
    assert_eq!(planned["should_remove"], true);
    assert_eq!(planned["should_emit"], false);
    assert_eq!(planned["actions"][0]["kind"], "remove_pending_text_update");
}

#[test]
fn logical_turn_claim_emits_when_max_messages_reached() {
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "claim_logical_turn",
        "candidate": {
            "chat_key": "chat-1",
            "update_key": "update-1",
        },
        "queue": [
            {
                "chat_key": "chat-1",
                "update_key": "update-1",
                "normalized_text": "one",
                "mergeable": true,
                "actor_identity": "user:1",
                "received_at": 10.0,
                "telegram_message_id": 1,
            },
            {
                "chat_key": "chat-1",
                "update_key": "update-2",
                "normalized_text": "two",
                "mergeable": true,
                "actor_identity": "user:1",
                "received_at": 10.1,
                "telegram_message_id": 2,
            },
            {
                "chat_key": "chat-1",
                "update_key": "update-3",
                "normalized_text": "three",
                "mergeable": true,
                "actor_identity": "user:1",
                "received_at": 10.2,
                "telegram_message_id": 3,
            }
        ],
        "merge_window_seconds": 5.0,
        "poll_interval_seconds": 0.25,
        "max_messages": 2,
        "now_monotonic_seconds": 10.2,
    }))
    .unwrap();

    assert_eq!(planned["logical_turn_state"], "emit");
    assert_eq!(planned["reached_limit"], true);
    assert_eq!(planned["boundary_seen"], false);
    assert_eq!(planned["consume_count"], 2);
    assert_eq!(planned["logical_turn"]["text"], "one\n\ntwo");
    assert_eq!(
        planned["logical_turn"]["telegram_message_ids"],
        json!([1, 2])
    );
}

#[test]
fn logical_turn_claim_emits_when_boundary_seen() {
    let planned = agent_telegram_logical_turn_runtime_plan_json(&json!({
        "stage": "claim_logical_turn",
        "candidate": {
            "chat_key": "chat-1",
            "update_key": "update-1",
        },
        "queue": [
            {
                "chat_key": "chat-1",
                "update_key": "update-1",
                "normalized_text": "one",
                "mergeable": true,
                "actor_identity": "user:1",
                "received_at": 10.0,
                "telegram_message_id": 1,
            },
            {
                "chat_key": "chat-1",
                "update_key": "update-2",
                "normalized_text": "two",
                "mergeable": false,
                "actor_identity": "user:1",
                "received_at": 10.1,
                "telegram_message_id": 2,
            }
        ],
        "merge_window_seconds": 1.0,
        "poll_interval_seconds": 0.25,
        "max_messages": 4,
        "now_monotonic_seconds": 10.2,
    }))
    .unwrap();

    assert_eq!(planned["logical_turn_state"], "emit");
    assert_eq!(planned["should_emit"], true);
    assert_eq!(planned["consume_count"], 1);
    assert_eq!(planned["logical_turn"]["text"], "one");
    assert_eq!(planned["logical_turn"]["telegram_message_id"], 1);
    assert_eq!(planned["actions"][0]["kind"], "consume_logical_turn");
    assert_eq!(planned["actions"][1]["kind"], "build_logical_turn");
}
