use super::{
    agent_slack_socket_mode_runtime_plan_json, plan_with_slack_socket_mode_runtime_planner,
    SlackSocketModeRuntimePlanner,
};
use ait_core::json_support::{json, JsonValue};

#[test]
fn connection_open_request_uses_bearer_app_token() {
    let planned = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "connection_open_request",
        "app_token": "xapp-test-token",
        "request_timeout_seconds": 12.0,
    }))
    .unwrap();

    assert_eq!(planned["stage"], "connection_open_request");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_slack_socket_mode_runtime"
    );
    assert_eq!(
        planned["slack_socket_mode_runtime_contract"],
        "ait_agent_core.event_loop.SlackSocketModeRuntime.v1"
    );
    assert_eq!(planned["python_socket_mode_runtime_allowed"], false);
    assert_eq!(planned["python_websocket_event_loop_allowed"], false);
    assert_eq!(planned["request"]["method"], "POST");
    assert_eq!(
        planned["request"]["url"],
        "https://slack.com/api/apps.connections.open"
    );
    assert_eq!(
        planned["request"]["headers"]["Authorization"],
        "Bearer xapp-test-token"
    );
    assert_eq!(planned["request"]["allow_retry"], true);
    assert_eq!(planned["actions"][0]["kind"], "open_socket_mode_connection");
}

#[test]
fn connect_registers_websocket_readable_when_fd_and_token_are_available() {
    let planned = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "connect",
        "connection_info": {"url": "wss://wss-primary.slack.com/link/?ticket=abc"},
        "worker_lease": {
            "backend": "linux_epoll",
            "shard_index": 2,
            "token": 513
        },
        "websocket_fd": 42,
    }))
    .unwrap();

    assert_eq!(planned["socket_mode_runtime_state"], "websocket_ready");
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["backend"], "linux_epoll");
    assert_eq!(planned["should_connect_websocket"], true);
    assert_eq!(planned["should_register_event_loop"], true);
    assert_eq!(planned["event_loop_registration"]["token"], 513);
    assert_eq!(planned["event_loop_registration"]["fd"], 42);
    assert_eq!(planned["event_loop_registration"]["interest"], "readable");
    assert_eq!(
        planned["actions"][0]["kind"],
        "connect_socket_mode_websocket"
    );
    assert_eq!(planned["actions"][1]["kind"], "register_websocket_readable");
}

#[test]
fn connect_fails_closed_without_socket_url() {
    let planned = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "connect",
        "worker_lease": {
            "backend": "portable_poll",
            "token": 7
        }
    }))
    .unwrap();

    assert_eq!(planned["socket_mode_runtime_state"], "awaiting_socket_url");
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["should_open_connection"], true);
    assert_eq!(planned["should_connect_websocket"], false);
    assert_eq!(planned["should_register_event_loop"], false);
    assert_eq!(planned["python_fallback_allowed"], false);
    assert_eq!(
        planned["error"],
        "Slack Socket Mode connection info did not include a websocket URL."
    );
}

#[test]
fn tick_waits_sends_ping_and_reconnects_on_missed_pong() {
    let waiting = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "tick",
        "now_monotonic_seconds": 10.0,
        "next_ping_at": 15.0,
    }))
    .unwrap();
    assert_eq!(waiting["socket_mode_runtime_state"], "waiting");
    assert_eq!(waiting["should_wait"], true);
    assert_eq!(waiting["should_send_ping"], false);

    let ping = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "tick",
        "now_monotonic_seconds": 15.0,
        "next_ping_at": 15.0,
        "ping_interval_seconds": 25.0,
        "pong_timeout_seconds": 5.0,
        "ping_id": "ping-1",
    }))
    .unwrap();
    assert_eq!(ping["socket_mode_runtime_state"], "ping_planned");
    assert_eq!(ping["should_send_ping"], true);
    assert_eq!(ping["pong_pending"], true);
    assert_eq!(ping["next_ping_at"], 40.0);
    assert_eq!(ping["websocket_ping"]["ping_id"], "ping-1");
    assert_eq!(ping["actions"][0]["kind"], "send_websocket_ping");

    let timeout = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "tick",
        "now_monotonic_seconds": 22.0,
        "pong_pending": true,
        "ping_sent_at": 15.0,
        "pong_timeout_seconds": 5.0,
    }))
    .unwrap();
    assert_eq!(timeout["socket_mode_runtime_state"], "pong_timeout");
    assert_eq!(timeout["ok"], false);
    assert_eq!(timeout["should_reconnect"], true);
    assert_eq!(timeout["actions"][0]["kind"], "reconnect_socket_mode");
}

#[test]
fn payload_delegates_envelope_to_socket_mode_transaction() {
    let planned = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "payload",
        "payload": {
            "envelope_id": "env-runtime-1",
            "type": "slash_commands",
            "accepts_response_payload": true,
            "payload": command_payload()
        },
        "binding": {
            "transport": "slack",
            "surface_id": "D123",
            "conversation_key": "slack:D123"
        },
        "repo_name": "ait",
        "defer_replies": true,
    }))
    .unwrap();

    assert_eq!(planned["socket_mode_runtime_state"], "transaction_planned");
    assert_eq!(planned["should_plan_transaction"], true);
    assert_eq!(
        planned["transaction_plan"]["socket_mode_transaction_state"],
        "command_ack_planned"
    );
    assert_eq!(planned["should_execute_websocket_ack"], true);
    assert_eq!(planned["should_handle_command"], true);
    assert_eq!(planned["actions"][0]["kind"], "execute_websocket_ack");
    assert_eq!(
        planned["actions"][0]["execute_before_command_side_effects"],
        true
    );
    assert_eq!(
        planned["actions"][1]["kind"],
        "dispatch_socket_mode_command"
    );
    assert_eq!(planned["actions"][1]["should_submit_turn"], true);
    assert_eq!(
        planned["ack_response"],
        json!({
            "envelope_id": "env-runtime-1",
            "payload": {"response_type": "ephemeral", "text": "ait is thinking..."}
        })
    );
}

#[test]
fn payload_handles_hello_pong_and_disconnect_without_transaction() {
    let hello = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "payload",
        "payload": {"type": "hello", "num_connections": 1}
    }))
    .unwrap();
    assert_eq!(hello["socket_mode_runtime_state"], "ready");
    assert_eq!(hello["should_mark_ready"], true);
    assert_eq!(hello["should_plan_transaction"], false);

    let pong = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "payload",
        "payload": {"type": "pong"}
    }))
    .unwrap();
    assert_eq!(pong["socket_mode_runtime_state"], "pong_acknowledged");
    assert_eq!(pong["pong_pending"], false);

    let disconnect = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "payload",
        "payload": {"type": "disconnect", "reason": "refresh_requested"}
    }))
    .unwrap();
    assert_eq!(
        disconnect["socket_mode_runtime_state"],
        "disconnect_requested"
    );
    assert_eq!(disconnect["should_reconnect"], true);
    assert_eq!(disconnect["reconnect_reason"], "refresh_requested");
}

#[test]
fn error_recovery_reconnects_or_stops_for_auth_failure() {
    let reconnect = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "error_recovery",
        "error_message": "websocket closed unexpectedly",
        "retry_attempt": 2,
        "retry_base_delay_seconds": 0.5,
    }))
    .unwrap();
    assert_eq!(reconnect["socket_mode_runtime_state"], "reconnect_planned");
    assert_eq!(reconnect["should_reconnect"], true);
    assert_eq!(reconnect["reconnect_delay_seconds"], 2.0);
    assert_eq!(reconnect["actions"][0]["kind"], "reconnect_socket_mode");

    let fatal = agent_slack_socket_mode_runtime_plan_json(&json!({
        "stage": "error_recovery",
        "error_message": "invalid_auth",
    }))
    .unwrap();
    assert_eq!(fatal["socket_mode_runtime_state"], "fatal_auth_error");
    assert_eq!(fatal["ok"], false);
    assert_eq!(fatal["should_stop_runtime"], true);
    assert_eq!(fatal["actions"][0]["kind"], "stop_socket_mode_runtime");
}

#[test]
fn slack_socket_mode_runtime_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl SlackSocketModeRuntimePlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "socket_mode_runtime_state": "substitute",
            }))
        }
    }

    let planned = plan_with_slack_socket_mode_runtime_planner(
        &SubstitutePlanner,
        &json!({ "stage": "connect" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "connect");
    assert_eq!(planned["socket_mode_runtime_state"], "substitute");
}

fn command_payload() -> JsonValue {
    json!({
        "token": "verification-token",
        "team_id": "T123",
        "team_domain": "ait",
        "channel_id": "C123",
        "channel_name": "ops",
        "user_id": "U123",
        "user_name": "Ada",
        "command": "/ait",
        "text": "status",
        "response_url": "https://hooks.slack.com/commands/response",
        "trigger_id": "1337.2468",
    })
}
