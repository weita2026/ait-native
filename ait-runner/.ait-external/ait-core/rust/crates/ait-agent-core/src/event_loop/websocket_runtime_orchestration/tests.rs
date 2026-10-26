use super::{
    agent_websocket_runtime_orchestration_plan_json,
    plan_with_websocket_runtime_orchestration_planner, WebSocketRuntimeOrchestrationPlanner,
};
use ait_core::json_support::{json, JsonValue};

#[test]
fn websocket_runtime_orchestration_schedules_lifecycle_reconnects_without_python_fallback() {
    let planned = agent_websocket_runtime_orchestration_plan_json(&json!({
        "stage": "orchestrate",
        "now_monotonic_seconds": 10.0,
        "reconnect_base_delay_seconds": 0.5,
        "reconnect_max_delay_seconds": 20.0,
        "websocket_lifecycle_result": {
            "reconnect_requests": [
                {
                    "transport": "slack",
                    "worker_key": "slack/team-a",
                    "event_loop_token": 7,
                    "reason": "socket_mode_pong_timeout",
                    "retry_attempt": 2
                },
                {
                    "transport": "discord",
                    "worker_key": "discord/ops",
                    "event_loop_token": 9,
                    "reason": "gateway_reconnect_requested",
                    "retry_attempt": 1,
                    "session_id": "discord-session",
                    "resume_gateway_url": "wss://gateway.discord.gg",
                    "sequence": 42
                }
            ]
        }
    }))
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_websocket_runtime_reconnect_orchestration"
    );
    assert_eq!(
        planned["websocket_runtime_orchestration_contract"],
        "ait_agent_core.event_loop.WebSocketRuntimeReconnectOrchestration.v1"
    );
    assert_eq!(
        planned["websocket_runtime_orchestration_state"],
        "reconnect_scheduled"
    );
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["reconnect_request_count"], 2);
    assert_eq!(planned["scheduled_reconnect_count"], 2);
    assert_eq!(planned["reconnect_schedules"][0]["delay_seconds"], 2.0);
    assert_eq!(
        planned["reconnect_schedules"][0]["scheduled_at_monotonic_seconds"],
        12.0
    );
    assert_eq!(
        planned["runtime_requests"][0]["stage"],
        "connection_open_request"
    );
    assert_eq!(planned["runtime_requests"][0]["transport"], "slack");
    assert_eq!(planned["runtime_requests"][0]["execute_connect"], false);
    assert_eq!(planned["runtime_requests"][1]["stage"], "gateway_url");
    assert_eq!(
        planned["runtime_requests"][1]["resume_gateway_url"],
        "wss://gateway.discord.gg"
    );
    assert_eq!(planned["python_websocket_runtime_allowed"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
}

#[test]
fn websocket_runtime_orchestration_applies_stop_precedence() {
    let planned = agent_websocket_runtime_orchestration_plan_json(&json!({
        "reconnect_requests": [{
            "transport": "socket_mode",
            "worker_key": "slack/team-a",
            "event_loop_token": 11,
            "reason": "runtime_error"
        }],
        "stop_requests": [{
            "transport": "slack_socket_mode",
            "worker_key": "slack/team-a",
            "reason": "auth_failure"
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_orchestration_state"],
        "stop_precedence_applied"
    );
    assert_eq!(planned["reconnect_request_count"], 1);
    assert_eq!(planned["stop_request_count"], 1);
    assert_eq!(planned["scheduled_reconnect_count"], 0);
    assert_eq!(planned["scheduled_stop_count"], 1);
    assert_eq!(planned["suppressed_reconnect_count"], 1);
    assert_eq!(planned["stop_schedules"][0]["transport"], "slack");
    assert_eq!(
        planned["runtime_requests"][0]["stage"],
        "stop_socket_mode_runtime"
    );
}

#[test]
fn websocket_runtime_orchestration_deduplicates_compatible_actions() {
    let planned = agent_websocket_runtime_orchestration_plan_json(&json!({
        "actions": [
            {
                "kind": "reconnect_socket_mode",
                "worker_key": "slack/team-a",
                "reason": "disconnect"
            },
            {
                "kind": "reconnect_socket_mode",
                "worker_key": "slack/team-a",
                "reason": "disconnect"
            },
            {
                "kind": "noop_websocket_action"
            }
        ]
    }))
    .unwrap();

    assert_eq!(planned["request_count"], 1);
    assert_eq!(planned["duplicate_request_count"], 1);
    assert_eq!(planned["skipped_request_count"], 1);
    assert_eq!(planned["scheduled_reconnect_count"], 1);
    assert_eq!(
        planned["actions"][0]["kind"],
        "schedule_socket_mode_reconnect"
    );
}

#[test]
fn websocket_runtime_orchestration_fails_closed_when_attempts_are_exhausted() {
    let planned = agent_websocket_runtime_orchestration_plan_json(&json!({
        "max_reconnect_attempts": 3,
        "reconnect_requests": [{
            "kind": "reconnect_gateway",
            "transport": "discord_gateway",
            "worker_key": "discord/ops",
            "retry_attempt": 3,
            "reason": "heartbeat_ack_timeout"
        }]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_orchestration_state"],
        "reconnect_exhausted"
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["scheduled_reconnect_count"], 0);
    assert_eq!(planned["exhausted_reconnect_count"], 1);
    assert_eq!(
        planned["actions"][0]["kind"],
        "diagnose_websocket_reconnect_attempt_exhausted"
    );
    assert_eq!(planned["python_fallback_allowed"], false);
}

#[test]
fn websocket_runtime_orchestration_handles_generic_websocket_handoff() {
    let planned = agent_websocket_runtime_orchestration_plan_json(&json!({
        "reconnect_requests": [{
            "transport": "custom-websocket",
            "worker_key": "custom/one",
            "delay_seconds": 0,
            "reason": "test"
        }]
    }))
    .unwrap();

    assert_eq!(planned["scheduled_reconnect_count"], 1);
    assert_eq!(planned["reconnect_schedules"][0]["should_wait"], false);
    assert_eq!(
        planned["runtime_requests"][0]["stage"],
        "reconnect_websocket"
    );
    assert_eq!(
        planned["actions"][0]["kind"],
        "schedule_websocket_reconnect"
    );
}

#[test]
fn websocket_runtime_orchestration_reports_configuration_errors_before_scheduling() {
    let planned = agent_websocket_runtime_orchestration_plan_json(&json!({
        "reconnect_requests": [
            "bad-request",
            {
                "kind": "reconnect_websocket"
            }
        ]
    }))
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_orchestration_state"],
        "configuration_error"
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["scheduled_reconnect_count"], 0);
    assert!(planned["diagnostics"][0]
        .as_str()
        .unwrap()
        .contains("must be an object"));
    assert_eq!(planned["python_fallback_allowed"], false);
}

#[test]
fn websocket_runtime_orchestration_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl WebSocketRuntimeOrchestrationPlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "websocket_runtime_orchestration_state": "substitute",
            }))
        }
    }

    let planned = plan_with_websocket_runtime_orchestration_planner(
        &SubstitutePlanner,
        &json!({ "stage": "orchestrate" }),
    )
    .unwrap();

    assert_eq!(
        planned["websocket_runtime_orchestration_state"],
        "substitute"
    );
}
