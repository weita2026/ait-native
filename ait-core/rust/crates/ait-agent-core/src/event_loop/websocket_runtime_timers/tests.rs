use super::{
    agent_websocket_runtime_timer_scheduler_plan_json, plan_with_websocket_runtime_timer_scheduler,
    WebSocketRuntimeTimerScheduler,
};
use ait_core::json_support::{json, JsonValue};

#[test]
fn websocket_runtime_timer_scheduler_releases_due_orchestration_reconnects_without_python_sleep() {
    let planned = agent_websocket_runtime_timer_scheduler_plan_json(&json!({
        "now_monotonic_seconds": 100.0,
        "websocket_runtime_orchestration": {
            "reconnect_schedules": [{
                "kind": "websocket_reconnect_schedule",
                "transport": "slack_socket_mode",
                "worker_key": "slack/main",
                "event_loop_token": 7,
                "reason": "socket_closed",
                "source_action_kind": "reconnect_socket_mode",
                "scheduled_at_monotonic_seconds": 99.5
            }],
            "runtime_requests": [{
                "stage": "connection_open_request",
                "transport": "slack",
                "worker_key": "slack/main",
                "event_loop_token": 7,
                "reason": "socket_closed",
                "source_action_kind": "reconnect_socket_mode",
                "reconnect_schedule": {
                    "kind": "websocket_reconnect_schedule",
                    "transport": "slack",
                    "worker_key": "slack/main",
                    "event_loop_token": 7,
                    "reason": "socket_closed",
                    "source_action_kind": "reconnect_socket_mode",
                    "scheduled_at_monotonic_seconds": 99.5
                },
                "execute_connect": false,
                "python_websocket_runtime_allowed": false
            }]
        }
    }))
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_websocket_runtime_timer_scheduler"
    );
    assert_eq!(
        planned["websocket_runtime_timer_scheduler_contract"],
        "ait_agent_core.event_loop.WebSocketRuntimeTimerScheduler.v1"
    );
    assert_eq!(
        planned["websocket_runtime_timer_scheduler_state"],
        "timers_due"
    );
    assert_eq!(planned["due_timer_count"], 1);
    assert_eq!(planned["pending_timer_count"], 0);
    assert_eq!(planned["duplicate_timer_count"], 1);
    assert_eq!(planned["next_poll_timeout_milliseconds"], 0);
    assert_eq!(planned["python_websocket_sleep_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(planned["due_timers"][0]["transport"], "slack");
    assert_eq!(
        planned["actions"][0]["kind"],
        "release_socket_mode_reconnect"
    );
    assert_eq!(
        planned["due_runtime_requests"][0]["stage"],
        "connection_open_request"
    );
    assert_eq!(planned["due_runtime_requests"][0]["timer_due"], true);
    assert_eq!(planned["due_runtime_requests"][0]["execute_sleep"], false);
}

#[test]
fn websocket_runtime_timer_scheduler_retains_pending_timer_and_clamps_timeout() {
    let planned = agent_websocket_runtime_timer_scheduler_plan_json(&json!({
        "now_monotonic_seconds": 10.0,
        "max_poll_timeout_seconds": 3.0,
        "reconnect_schedules": [{
            "kind": "websocket_reconnect_schedule",
            "transport": "discord_gateway",
            "worker_key": "discord/ops",
            "event_loop_token": 42,
            "reason": "gateway_retry",
            "source_action_kind": "reconnect_gateway",
            "scheduled_at_monotonic_seconds": 25.0
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_timer_scheduler_state"],
        "timers_pending"
    );
    assert_eq!(planned["due_timer_count"], 0);
    assert_eq!(planned["pending_timer_count"], 1);
    assert_eq!(planned["pending_timers"][0]["transport"], "discord");
    assert_eq!(planned["pending_timers"][0]["due_in_seconds"], 15.0);
    assert_eq!(planned["next_poll_timeout_seconds"], 3.0);
    assert_eq!(planned["next_poll_timeout_milliseconds"], 3000);
    assert_eq!(
        planned["actions"][0]["kind"],
        "keep_gateway_reconnect_timer_pending"
    );
}

#[test]
fn websocket_runtime_timer_scheduler_cancels_matching_reconnect_timer_for_stop_schedule() {
    let planned = agent_websocket_runtime_timer_scheduler_plan_json(&json!({
        "now_monotonic_seconds": 50.0,
        "reconnect_schedules": [{
            "kind": "websocket_reconnect_schedule",
            "transport": "slack",
            "worker_key": "slack/main",
            "event_loop_token": 9,
            "reason": "retry",
            "source_action_kind": "reconnect_socket_mode",
            "scheduled_at_monotonic_seconds": 55.0
        }],
        "stop_schedules": [{
            "kind": "websocket_stop_schedule",
            "transport": "slack_socket_mode",
            "worker_key": "slack/main",
            "event_loop_token": 9,
            "reason": "shutdown",
            "source_action_kind": "stop_socket_mode_runtime"
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_timer_scheduler_state"],
        "stops_applied"
    );
    assert_eq!(planned["canceled_timer_count"], 1);
    assert_eq!(planned["pending_timer_count"], 0);
    assert_eq!(planned["due_timer_count"], 0);
    assert_eq!(
        planned["actions"][0]["kind"],
        "cancel_socket_mode_reconnect_timer"
    );
    assert_eq!(planned["stop_timers"][0]["transport"], "slack");
    assert!(planned["diagnostics"][0]
        .as_str()
        .unwrap()
        .contains("canceled by stop request"));
}

#[test]
fn websocket_runtime_timer_scheduler_materializes_delay_relative_to_now() {
    let planned = agent_websocket_runtime_timer_scheduler_plan_json(&json!({
        "now_monotonic_seconds": 20.0,
        "reconnect_timers": [{
            "kind": "websocket_reconnect_timer",
            "transport": "custom-websocket",
            "worker_key": "custom/a",
            "event_loop_token": 4,
            "reason": "retry",
            "source_action_kind": "schedule_websocket_reconnect",
            "delay_seconds": 2.25
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_timer_scheduler_state"],
        "timers_pending"
    );
    assert_eq!(
        planned["pending_timers"][0]["transport"],
        "custom_websocket"
    );
    assert_eq!(
        planned["pending_timers"][0]["scheduled_at_monotonic_seconds"],
        22.25
    );
    assert_eq!(planned["next_poll_timeout_milliseconds"], 2250);
    assert_eq!(planned["runtime_requests"].as_array().unwrap().len(), 0);
}

#[test]
fn websocket_runtime_timer_scheduler_handles_due_generic_websocket_fallback() {
    let planned = agent_websocket_runtime_timer_scheduler_plan_json(&json!({
        "now_monotonic_seconds": 30.0,
        "timers": [{
            "kind": "websocket_reconnect_timer",
            "transport": "generic_websocket",
            "worker_key": "generic/one",
            "event_loop_token": 3,
            "reason": "retry",
            "source_action_kind": "schedule_websocket_reconnect",
            "scheduled_at_monotonic_seconds": 30.0
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_timer_scheduler_state"],
        "timers_due"
    );
    assert_eq!(
        planned["due_runtime_requests"][0]["stage"],
        "reconnect_websocket"
    );
    assert_eq!(planned["actions"][0]["kind"], "release_websocket_reconnect");
    assert_eq!(planned["due_timers"][0]["transport"], "websocket");
}

#[test]
fn websocket_runtime_timer_scheduler_uses_idle_timeout_when_no_timers() {
    let planned = agent_websocket_runtime_timer_scheduler_plan_json(&json!({
        "now_monotonic_seconds": 1.0,
        "default_idle_timeout_seconds": 90.0,
        "max_poll_timeout_seconds": 15.0
    }))
    .unwrap();

    assert_eq!(planned["websocket_runtime_timer_scheduler_state"], "idle");
    assert_eq!(planned["timer_count"], 0);
    assert_eq!(planned["next_poll_timeout_seconds"], 15.0);
    assert_eq!(planned["next_poll_timeout_milliseconds"], 15000);
}

#[test]
fn websocket_runtime_timer_scheduler_reports_configuration_errors_without_python_fallback() {
    let planned = agent_websocket_runtime_timer_scheduler_plan_json(&json!({
        "reconnect_schedules": [{
            "kind": "websocket_reconnect_schedule",
            "transport": "slack",
            "worker_key": "slack/main",
            "event_loop_token": 1,
            "scheduled_at_monotonic_seconds": 1.0
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_timer_scheduler_state"],
        "configuration_error"
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["python_websocket_sleep_allowed"], false);
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_runtime_timer_scheduler_configuration_error"
    );
}

#[test]
fn websocket_runtime_timer_scheduler_bound_entrypoint_accepts_substitute_scheduler() {
    struct SubstituteScheduler;

    impl WebSocketRuntimeTimerScheduler for SubstituteScheduler {
        fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "websocket_runtime_timer_scheduler_state": "substitute",
                "python_fallback_allowed": false
            }))
        }
    }

    let planned =
        plan_with_websocket_runtime_timer_scheduler(&SubstituteScheduler, &json!({})).unwrap();

    assert_eq!(
        planned["websocket_runtime_timer_scheduler_state"],
        "substitute"
    );
    assert_eq!(planned["python_fallback_allowed"], false);
}
