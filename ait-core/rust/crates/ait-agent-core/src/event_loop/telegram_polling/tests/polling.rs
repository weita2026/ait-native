use super::super::{
    agent_telegram_polling_cycle_plan_json, agent_telegram_service_runtime_shell_plan_json,
    agent_telegram_update_batch_dispatch_plan_json, agent_telegram_update_dispatch_plan_json,
};
use ait_core::json_support::{json, JsonValue};

#[test]
fn polling_cycle_plan_preserves_offset_and_disabled_sync_timeout() {
    let planned = agent_telegram_polling_cycle_plan_json(&json!({
        "last_update_id": 57,
        "poll_timeout_seconds": 30,
        "background_sync_enabled": false,
        "background_sync_interval_seconds": 60.0,
        "now_monotonic_seconds": 10.0,
        "next_background_sync_at": 11.2,
    }))
    .unwrap();

    assert_eq!(planned["offset"], 58);
    assert_eq!(planned["poll_timeout_seconds"], 30);
    assert_eq!(planned["next_background_sync_at"], JsonValue::Null);
    assert_eq!(planned["background_sync_due"], false);
}

#[test]
fn polling_cycle_plan_clamps_timeout_to_background_deadline() {
    let planned = agent_telegram_polling_cycle_plan_json(&json!({
        "last_update_id": 0,
        "poll_timeout_seconds": 30,
        "background_sync_enabled": true,
        "background_sync_interval_seconds": 60.0,
        "now_monotonic_seconds": 10.0,
        "next_background_sync_at": 11.2,
    }))
    .unwrap();

    assert_eq!(planned["offset"], 1);
    assert_eq!(planned["poll_timeout_seconds"], 2);
    assert_eq!(planned["next_background_sync_at"], 11.2);
    assert_eq!(planned["background_sync_due"], false);
}

#[test]
fn polling_cycle_plan_initializes_and_advances_background_deadlines() {
    let initialized = agent_telegram_polling_cycle_plan_json(&json!({
        "poll_timeout_seconds": 30,
        "background_sync_enabled": true,
        "background_sync_interval_seconds": 30.0,
        "now_monotonic_seconds": 100.0,
        "next_background_sync_at": null,
    }))
    .unwrap();
    assert_eq!(initialized["next_background_sync_at"], 130.0);
    assert_eq!(initialized["background_sync_due"], false);

    let due = agent_telegram_polling_cycle_plan_json(&json!({
        "poll_timeout_seconds": 30,
        "background_sync_enabled": true,
        "background_sync_interval_seconds": 30.0,
        "now_monotonic_seconds": 131.0,
        "next_background_sync_at": 130.0,
    }))
    .unwrap();
    assert_eq!(due["next_background_sync_at"], 161.0);
    assert_eq!(due["background_sync_due"], true);
    assert_eq!(due["should_run_background_sync_once"], true);
}

#[test]
fn update_dispatch_plan_preserves_chat_update_and_message_keys() {
    let chat = agent_telegram_update_dispatch_plan_json(&json!({
        "update": {
            "update_id": 7,
            "message": {"message_id": 12, "chat": {"id": 123}}
        },
        "fallback_update_key": "memory-abc",
    }))
    .unwrap();
    assert_eq!(chat["chat_id"], 123);
    assert_eq!(chat["dispatch_key"], "chat-123");
    assert_eq!(chat["update_key"], "update-7");
    assert_eq!(chat["should_update_last_update_id"], true);

    let message = agent_telegram_update_dispatch_plan_json(&json!({
        "update": {"message": {"message_id": 12, "chat": {}}},
        "fallback_update_key": "memory-abc",
    }))
    .unwrap();
    assert_eq!(message["dispatch_key"], "update-unknown");
    assert_eq!(message["update_key"], "message-12");

    let fallback = agent_telegram_update_dispatch_plan_json(&json!({
        "update": {},
        "fallback_update_key": "memory-abc",
    }))
    .unwrap();
    assert_eq!(fallback["dispatch_key"], "update-unknown");
    assert_eq!(fallback["update_key"], "memory-abc");
}

#[test]
fn update_dispatch_plan_keeps_string_chat_ids() {
    let planned = agent_telegram_update_dispatch_plan_json(&json!({
        "update": {"update_id": "9", "message": {"chat": {"id": "abc"}}}
    }))
    .unwrap();

    assert_eq!(planned["update_id"], 9);
    assert_eq!(planned["chat_id"], "abc");
    assert_eq!(planned["dispatch_key"], "chat-abc");
}

#[test]
fn update_batch_dispatch_plan_preserves_order_and_last_seen_update_id() {
    let planned = agent_telegram_update_batch_dispatch_plan_json(&json!({
        "updates": [
            {"update_id": 9, "message": {"message_id": 5, "chat": {"id": 123}}},
            {"message": {"message_id": 44, "chat": {}}},
            {"update_id": 7},
            {}
        ],
        "fallback_update_keys": [
            "memory-a",
            "memory-b",
            "memory-c",
            "memory-d"
        ]
    }))
    .unwrap();

    assert_eq!(planned["update_count"], 4);
    assert_eq!(planned["last_update_id"], 7);
    assert_eq!(planned["should_update_last_update_id"], true);
    assert_eq!(planned["dispatch_items"][0]["index"], 0);
    assert_eq!(planned["dispatch_items"][0]["dispatch_key"], "chat-123");
    assert_eq!(planned["dispatch_items"][0]["update_key"], "update-9");
    assert_eq!(
        planned["dispatch_items"][1]["dispatch_key"],
        "update-unknown"
    );
    assert_eq!(planned["dispatch_items"][1]["update_key"], "message-44");
    assert_eq!(planned["dispatch_items"][2]["dispatch_key"], "update-7");
    assert_eq!(planned["dispatch_items"][2]["update_key"], "update-7");
    assert_eq!(
        planned["dispatch_items"][3]["dispatch_key"],
        "update-unknown"
    );
    assert_eq!(planned["dispatch_items"][3]["update_key"], "memory-d");
}

#[test]
fn update_batch_dispatch_plan_reports_no_last_update_when_batch_has_no_update_ids() {
    let planned = agent_telegram_update_batch_dispatch_plan_json(&json!({
        "updates": [
            {"message": {"message_id": 44, "chat": {}}},
            {}
        ],
        "fallback_update_keys": [
            "memory-a",
            "memory-b"
        ]
    }))
    .unwrap();

    assert_eq!(planned["last_update_id"], 0);
    assert_eq!(planned["should_update_last_update_id"], false);
    assert_eq!(planned["dispatch_items"][0]["update_key"], "message-44");
    assert_eq!(planned["dispatch_items"][1]["update_key"], "memory-b");
}

#[test]
fn service_runtime_shell_poll_stage_orders_background_before_poll_request() {
    let planned = agent_telegram_service_runtime_shell_plan_json(&json!({
        "stage": "poll",
        "last_update_id": 57,
        "poll_timeout_seconds": 30,
        "background_sync_enabled": true,
        "background_sync_interval_seconds": 30.0,
        "now_monotonic_seconds": 131.0,
        "next_background_sync_at": 130.0,
    }))
    .unwrap();

    assert_eq!(planned["stage"], "poll");
    assert_eq!(planned["next_background_sync_at"], 161.0);
    assert_eq!(planned["background_sync"]["due"], true);
    assert_eq!(planned["poll_request"]["offset"], 58);
    assert_eq!(planned["poll_request"]["timeout_seconds"], 30);
    assert_eq!(planned["actions"][0]["kind"], "run_background_sync_once");
    assert_eq!(planned["actions"][1]["kind"], "poll_updates");
    assert_eq!(planned["actions"][1]["offset"], 58);
    assert_eq!(planned["actions"][1]["timeout_seconds"], 30);
}

#[test]
fn service_runtime_shell_poll_stage_clamps_to_initialized_background_deadline() {
    let planned = agent_telegram_service_runtime_shell_plan_json(&json!({
        "last_update_id": 0,
        "poll_timeout_seconds": 30,
        "background_sync_enabled": true,
        "background_sync_interval_seconds": 1.2,
        "now_monotonic_seconds": 10.0,
        "next_background_sync_at": null,
    }))
    .unwrap();

    assert_eq!(planned["stage"], "poll");
    assert_eq!(planned["next_background_sync_at"], 11.2);
    assert_eq!(planned["background_sync"]["due"], false);
    assert_eq!(planned["poll_request"]["offset"], 1);
    assert_eq!(planned["poll_request"]["timeout_seconds"], 2);
    assert_eq!(planned["actions"].as_array().unwrap().len(), 1);
    assert_eq!(planned["actions"][0]["kind"], "poll_updates");
}

#[test]
fn service_runtime_shell_updates_stage_dispatches_then_advances_last_update_id() {
    let planned = agent_telegram_service_runtime_shell_plan_json(&json!({
        "stage": "updates",
        "updates": [
            {"update_id": 101, "message": {"message_id": 10, "chat": {"id": 123}}},
            {"update_id": 5, "message": {"message_id": 11, "chat": {"id": 456}}},
            {"message": {"message_id": 44, "chat": {}}}
        ],
        "fallback_update_keys": [
            "memory-a",
            "memory-b",
            "memory-c"
        ]
    }))
    .unwrap();

    assert_eq!(planned["stage"], "updates");
    assert_eq!(planned["update_count"], 3);
    assert_eq!(planned["last_update_id"], 5);
    assert_eq!(planned["should_update_last_update_id"], true);
    assert_eq!(planned["actions"][0]["kind"], "dispatch_update");
    assert_eq!(planned["actions"][0]["index"], 0);
    assert_eq!(
        planned["actions"][0]["dispatch_item"]["dispatch_key"],
        "chat-123"
    );
    assert_eq!(planned["actions"][1]["kind"], "dispatch_update");
    assert_eq!(
        planned["actions"][1]["dispatch_item"]["dispatch_key"],
        "chat-456"
    );
    assert_eq!(planned["actions"][2]["kind"], "dispatch_update");
    assert_eq!(
        planned["actions"][2]["dispatch_item"]["update_key"],
        "message-44"
    );
    assert_eq!(planned["actions"][3]["kind"], "update_last_update_id");
    assert_eq!(planned["actions"][3]["last_update_id"], 5);
}

#[test]
fn service_runtime_shell_updates_stage_keeps_empty_batch_as_no_callback_work() {
    let planned = agent_telegram_service_runtime_shell_plan_json(&json!({
        "updates": [],
        "fallback_update_keys": []
    }))
    .unwrap();

    assert_eq!(planned["stage"], "updates");
    assert_eq!(planned["has_updates"], false);
    assert_eq!(planned["should_continue_without_callbacks"], true);
    assert_eq!(planned["actions"].as_array().unwrap().len(), 0);
    assert_eq!(planned["dispatch_plan"]["update_count"], 0);
    assert_eq!(
        planned["dispatch_plan"]["should_update_last_update_id"],
        false
    );
}
