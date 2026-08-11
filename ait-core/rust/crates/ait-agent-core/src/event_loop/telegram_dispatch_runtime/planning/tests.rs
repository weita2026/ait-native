use super::*;
use ait_core::json_support::json;

#[test]
fn dispatch_runtime_configure_uses_admission_plan_shard_and_limit() {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "configure",
        "admission_plan": {
            "backend": "linux_epoll",
            "workers_per_shard": 32,
            "worker_leases": [
                {"shard_index": 3}
            ],
            "shard_admissions": [
                {"shard_index": 3, "inflight_limit": 32}
            ]
        }
    }))
    .unwrap();

    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(
        planned["dispatch_runtime_contract"],
        DISPATCH_RUNTIME_CONTRACT
    );
    assert_eq!(planned["python_dispatch_allowed"], false);
    assert_eq!(planned["backend"], "linux_epoll");
    assert_eq!(planned["shard_index"], 3);
    assert_eq!(planned["inflight_limit"], 32);
    assert_eq!(planned["actions"][0]["kind"], "configure_dispatch_runtime");
}

#[test]
fn dispatch_runtime_uses_the_selected_lease_shard_limit_from_a_full_admission_plan() {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "configure",
        "admission_plan": {
            "backend": "linux_epoll",
            "worker_leases": [{"shard_index": 2}],
            "shard_admissions": [
                {"shard_index": 0, "inflight_limit": 8},
                {"shard_index": 1, "inflight_limit": 16},
                {"shard_index": 2, "inflight_limit": 31}
            ]
        }
    }))
    .unwrap();

    assert_eq!(planned["shard_index"], 2);
    assert_eq!(planned["inflight_limit"], 31);
    assert_eq!(planned["actions"][0]["inflight_limit"], 31);
}

#[test]
fn dispatch_runtime_submit_plans_executor_creation_and_slot_tracking() {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "submit",
        "dispatcher_kind": "dispatch",
        "queue_key": "chat-1",
        "backend": "linux_epoll",
        "shard_index": 2,
        "inflight_limit": 4,
        "inflight_count": 1,
        "has_executor": false,
    }))
    .unwrap();

    assert_eq!(planned["dispatch_runtime_state"], "accepted");
    assert_eq!(planned["should_submit"], true);
    assert_eq!(planned["should_create_executor"], true);
    assert_eq!(planned["should_reserve_inflight_slot"], true);
    assert_eq!(
        planned["thread_name_prefix"],
        "ait-telegram-dispatch-linux_epoll-s2-chat-1"
    );
    assert_eq!(planned["actions"][0]["kind"], "reserve_inflight_slot");
    assert_eq!(planned["actions"][1]["kind"], "ensure_executor");
    assert_eq!(planned["actions"][2]["kind"], "submit_callable");
    assert_eq!(planned["actions"][3]["kind"], "track_future");
}

#[test]
fn dispatch_runtime_thread_name_prefix_uses_defaults_and_normalized_kind() {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "thread_name_prefix",
        "dispatcher_kind": "Reply",
        "queue_key": " chat-1 ",
    }))
    .unwrap();

    assert_eq!(planned["stage"], "thread_name_prefix");
    assert_eq!(planned["dispatch_runtime_state"], "planned");
    assert_eq!(planned["dispatcher_kind"], "reply");
    assert_eq!(planned["backend"], "portable_poll");
    assert_eq!(planned["shard_index"], 0);
    assert_eq!(planned["queue_key"], "chat-1");
    assert_eq!(
        planned["thread_name_prefix"],
        "ait-telegram-reply-portable_poll-s0-chat-1"
    );
    assert_eq!(planned["actions"][0]["kind"], "thread_name_prefix");
}

#[test]
fn dispatch_runtime_submit_serialized_reuses_existing_executor() {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "submit_serialized",
        "dispatcher_kind": "Reply",
        "queue_key": "chat-2",
        "inflight_limit": "3",
        "inflight_count": "2",
        "has_executor": "yes",
    }))
    .unwrap();

    assert_eq!(planned["stage"], "submit_serialized");
    assert_eq!(planned["dispatch_runtime_state"], "accepted");
    assert_eq!(planned["dispatcher_kind"], "reply");
    assert_eq!(planned["should_create_executor"], false);
    assert_eq!(planned["should_reserve_inflight_slot"], true);
    assert_eq!(planned["actions"].as_array().unwrap().len(), 3);
    assert_eq!(planned["actions"][0]["kind"], "reserve_inflight_slot");
    assert_eq!(planned["actions"][1]["kind"], "submit_callable");
    assert_eq!(planned["actions"][2]["kind"], "track_future");
}

#[test]
fn dispatch_runtime_submit_rejects_stopped_runtime_with_python_repr_queue() {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "submit_reply_serialized",
        "queue_key": "chat-1",
        "stop_requested": true,
    }))
    .unwrap();

    assert_eq!(planned["dispatch_runtime_state"], "stopped");
    assert_eq!(planned["should_submit"], false);
    assert_eq!(planned["rejection_message"], "Telegram dispatch runtime is stopped; refusing Python fallback submission for queue 'chat-1'.");
    assert_eq!(planned["actions"].as_array().unwrap().len(), 0);
}

#[test]
fn dispatch_runtime_submit_rejects_inflight_limit() {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "submit",
        "queue_key": "chat-2",
        "inflight_limit": 1,
        "inflight_count": 1,
    }))
    .unwrap();

    assert_eq!(planned["dispatch_runtime_state"], "inflight_limit_reached");
    assert_eq!(planned["should_submit"], false);
    assert_eq!(planned["rejection_message"], "Telegram dispatch runtime inflight limit 1 reached; refusing Python fallback submission for queue 'chat-2'.");
}

#[test]
fn dispatch_runtime_stop_returns_shutdown_actions() {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "stop",
        "dispatch_queue_count": 2,
        "reply_queue_count": 1,
    }))
    .unwrap();

    assert_eq!(planned["dispatch_runtime_state"], "stopped");
    assert_eq!(planned["should_stop"], true);
    assert_eq!(planned["actions"][0]["kind"], "shutdown_dispatchers");
    assert_eq!(planned["actions"][1]["dispatcher_kind"], "reply");
    assert_eq!(planned["actions"][2]["kind"], "clear_dispatchers");
}

#[test]
fn dispatch_runtime_errors_match_public_contract() {
    assert_eq!(
        agent_telegram_dispatch_runtime_plan_json(&json!("bad request")).unwrap_err(),
        "request must be a JSON object"
    );
    assert_eq!(
        agent_telegram_dispatch_runtime_plan_json(&json!({
            "stage": "unknown",
        }))
        .unwrap_err(),
        "unsupported Telegram dispatch runtime stage: unknown"
    );
    assert_eq!(
        agent_telegram_dispatch_runtime_plan_json(&json!({
            "stage": "submit",
        }))
        .unwrap_err(),
        "queue_key is required"
    );
}

#[test]
fn dispatch_runtime_default_planner_uses_public_json_contract() {
    let planner: &dyn TelegramDispatchRuntimePlanner = &DefaultTelegramDispatchRuntimePlanner;
    let planned = planner
        .plan_json(&json!({
            "stage": "stop",
            "dispatch_queue_count": 1,
            "reply_queue_count": 1,
        }))
        .unwrap();

    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(
        planned["dispatch_runtime_contract"],
        DISPATCH_RUNTIME_CONTRACT
    );
    assert_eq!(planned["stage"], "stop");
    assert_eq!(planned["dispatch_runtime_state"], "stopped");
}

#[test]
fn dispatch_runtime_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl TelegramDispatchRuntimePlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "dispatch_runtime_state": "substitute",
            }))
        }
    }

    let planned = plan_with_telegram_dispatch_runtime_planner(
        &SubstitutePlanner,
        &json!({ "stage": "submit" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "submit");
    assert_eq!(planned["dispatch_runtime_state"], "substitute");
}
