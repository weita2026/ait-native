use super::{
    agent_slack_command_http_ingress_plan_json, agent_slack_command_http_transaction_plan_json,
    agent_slack_ingress_runtime_plan_json, agent_slack_socket_mode_transaction_plan_json,
    plan_with_slack_command_http_ingress_planner, plan_with_slack_command_http_transaction_planner,
    plan_with_slack_ingress_runtime_planner, plan_with_slack_socket_mode_transaction_planner,
    SlackCommandHttpIngressPlanner, SlackCommandHttpTransactionPlanner, SlackIngressRuntimePlanner,
    SlackSocketModeTransactionPlanner,
};
use ait_core::json_support::{json, JsonValue};
use hmac::{Hmac, Mac};
use sha2::Sha256;

const SIGNED_RAW_COMMAND: &str = "team_id=T1&channel_id=C1&user_id=U1&command=%2Fait&text=hello+world&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT%2FB%2FC&trigger_id=trig-1";
const SIGNING_SECRET: &str = "test-signing-secret";
const SIGNATURE_TIMESTAMP: &str = "1714990000";
const VALID_SIGNATURE: &str = "v0=e1cf5c0a4fcd1a6765885c2fbc4f52e2b4f9d2456d92737ef12972b300da8bba";
type HmacSha256 = Hmac<Sha256>;

fn command_payload() -> JsonValue {
    json!({
        "team_id": "T-team-1",
        "team_domain": "ait",
        "channel_id": "C-ops-1",
        "channel_name": "ops",
        "user_id": "U-slack-1",
        "user_name": "weita",
        "command": "/ait",
        "text": "Hello from Slack",
        "response_url": "https://hooks.slack.com/commands/T000/B000/abc123",
        "trigger_id": "1337.2468"
    })
}

fn test_slack_signature(raw_payload: &str) -> String {
    let base_string = format!("v0:{SIGNATURE_TIMESTAMP}:{raw_payload}");
    let mut mac = HmacSha256::new_from_slice(SIGNING_SECRET.as_bytes()).unwrap();
    mac.update(base_string.as_bytes());
    let digest = mac.finalize().into_bytes();
    let hex = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("v0={hex}")
}

#[test]
fn command_http_request_verifies_signature_and_normalizes_form_payload() {
    let planned = agent_slack_command_http_ingress_plan_json(&json!({
        "request_path": "/command",
        "command_path": "command",
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature": VALID_SIGNATURE,
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
    }))
    .unwrap();

    assert_eq!(planned["stage"], "http_command_request");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_slack_command_http_ingress"
    );
    assert_eq!(
        planned["slack_command_http_ingress_contract"],
        "ait_agent_core.event_loop.SlackCommandHttpIngress.v1"
    );
    assert_eq!(planned["python_signature_verification_allowed"], false);
    assert_eq!(planned["python_form_parsing_allowed"], false);
    assert_eq!(planned["rust_event_loop_required"], true);
    assert_eq!(
        planned["command_http_ingress_state"],
        "command_payload_ready"
    );
    assert_eq!(planned["ok"], true);
    assert_eq!(planned["http_status"], 200);
    assert_eq!(planned["signature_verified"], true);
    assert_eq!(planned["should_handle_command"], true);
    assert_eq!(planned["command_payload"]["command"], "/ait");
    assert_eq!(planned["command_payload"]["text"], "hello world");
    assert_eq!(
        planned["command_payload"]["response_url"],
        "https://hooks.slack.com/commands/T/B/C"
    );
    assert_eq!(planned["next_ingress_request"]["stage"], "command");
    assert_eq!(
        planned["next_ingress_request"]["payload"],
        planned["command_payload"]
    );
}

#[test]
fn command_http_request_fails_closed_for_signature_errors() {
    let missing_signature = agent_slack_command_http_ingress_plan_json(&json!({
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
    }))
    .unwrap();
    assert_eq!(
        missing_signature["command_http_ingress_state"],
        "missing_signature"
    );
    assert_eq!(missing_signature["http_status"], 401);
    assert_eq!(
        missing_signature["response"],
        json!({"ok": false, "error": "Missing Slack signature header."})
    );
    assert_eq!(missing_signature["should_handle_command"], false);

    let stale_timestamp = agent_slack_command_http_ingress_plan_json(&json!({
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature": VALID_SIGNATURE,
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990601,
    }))
    .unwrap();
    assert_eq!(
        stale_timestamp["command_http_ingress_state"],
        "timestamp_outside_tolerance"
    );
    assert_eq!(stale_timestamp["http_status"], 401);
    assert_eq!(
        stale_timestamp["response"]["error"],
        "Slack request timestamp is outside the allowed tolerance."
    );

    let invalid_signature = agent_slack_command_http_ingress_plan_json(&json!({
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature": "v0=bad",
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
    }))
    .unwrap();
    assert_eq!(
        invalid_signature["command_http_ingress_state"],
        "invalid_signature"
    );
    assert_eq!(invalid_signature["signature_verified"], false);
    assert_eq!(
        invalid_signature["response"]["error"],
        "Invalid Slack request signature."
    );
}

#[test]
fn command_http_request_plans_config_payload_and_path_failures() {
    let missing_secret = agent_slack_command_http_ingress_plan_json(&json!({
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature": VALID_SIGNATURE,
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "now_unix_seconds": 1714990000,
    }))
    .unwrap();
    assert_eq!(
        missing_secret["command_http_ingress_state"],
        "missing_signing_secret"
    );
    assert_eq!(missing_secret["http_status"], 400);
    assert_eq!(missing_secret["error_kind"], "config_error");

    let empty_payload = agent_slack_command_http_ingress_plan_json(&json!({
        "raw_payload": "   ",
        "signature": "v0=424e58698a6f486f3b8f55f0b2549a5cce10e3f1005d48e082d1794c2e9ede50",
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
    }))
    .unwrap();
    assert_eq!(empty_payload["command_http_ingress_state"], "empty_payload");
    assert_eq!(empty_payload["http_status"], 400);
    assert_eq!(
        empty_payload["response"],
        json!({"ok": false, "error": "No Slack command payload provided."})
    );

    let not_found = agent_slack_command_http_ingress_plan_json(&json!({
        "request_path": "/other",
        "command_path": "/command",
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature": VALID_SIGNATURE,
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
    }))
    .unwrap();
    assert_eq!(not_found["command_http_ingress_state"], "not_found");
    assert_eq!(not_found["http_status"], 404);
    assert_eq!(not_found["write_json_response"], false);
    assert_eq!(not_found["should_parse_payload"], false);
}

#[test]
fn command_http_transaction_sequences_request_and_ingress_planning() {
    let planned = agent_slack_command_http_transaction_plan_json(&json!({
        "request_path": "/command",
        "command_path": "command",
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature": VALID_SIGNATURE,
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
        "binding": {
            "conversation_key": "slack:C-ops-1",
            "slack_recent_request_ids": []
        },
        "repo_name": "ait",
        "defer_replies": true,
        "occurred_at": "2026-07-02T10:00:00Z",
    }))
    .unwrap();

    assert_eq!(planned["stage"], "http_command_transaction");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_slack_command_http_transaction"
    );
    assert_eq!(
        planned["slack_command_http_transaction_contract"],
        "ait_agent_core.event_loop.SlackCommandHttpTransaction.v1"
    );
    assert_eq!(planned["python_http_sequencing_allowed"], false);
    assert_eq!(planned["python_signature_verification_allowed"], false);
    assert_eq!(planned["python_form_parsing_allowed"], false);
    assert_eq!(planned["python_ingress_allowed"], false);
    assert_eq!(planned["rust_event_loop_required"], true);
    assert_eq!(
        planned["command_http_transaction_state"],
        "command_http_response_planned"
    );
    assert_eq!(
        planned["command_http_ingress_state"],
        "command_payload_ready"
    );
    assert_eq!(planned["ingress_runtime_state"], "turn_submission_planned");
    assert_eq!(planned["http_status"], 200);
    assert_eq!(planned["write_json_response"], true);
    assert_eq!(
        planned["response"],
        json!({"response_type": "ephemeral", "text": "ait is thinking..."})
    );
    assert_eq!(planned["should_plan_ingress"], true);
    assert_eq!(planned["should_submit_turn"], true);
    assert_eq!(planned["should_start_background_reply"], true);
    assert_eq!(planned["should_execute_inline_reply"], false);
    assert_eq!(
        planned["http_ingress_plan"]["command_payload"]["text"],
        "hello world"
    );
    assert_eq!(planned["ingress_request"]["stage"], "command");
    assert_eq!(
        planned["ingress_request"]["signing_secret"],
        JsonValue::Null
    );
    assert_eq!(planned["ingress_plan"]["request_id"], "trig-1");
    assert_eq!(planned["pending_reply"]["request_id"], "trig-1");
    assert_eq!(planned["actions"][2]["kind"], "start_background_reply");
}

#[test]
fn command_http_transaction_fails_closed_without_ingress_for_request_shell_errors() {
    let not_found = agent_slack_command_http_transaction_plan_json(&json!({
        "request_path": "/other",
        "command_path": "/command",
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature": VALID_SIGNATURE,
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
    }))
    .unwrap();

    assert_eq!(not_found["command_http_transaction_state"], "not_found");
    assert_eq!(not_found["http_status"], 404);
    assert_eq!(not_found["write_json_response"], false);
    assert_eq!(not_found["should_plan_ingress"], false);
    assert_eq!(not_found["ingress_plan"], JsonValue::Null);
    assert_eq!(not_found["actions"], json!([]));

    let invalid_signature = agent_slack_command_http_transaction_plan_json(&json!({
        "raw_payload": SIGNED_RAW_COMMAND,
        "signature": "v0=bad",
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
    }))
    .unwrap();

    assert_eq!(
        invalid_signature["command_http_transaction_state"],
        "invalid_signature"
    );
    assert_eq!(invalid_signature["http_status"], 401);
    assert_eq!(invalid_signature["should_plan_ingress"], false);
    assert_eq!(
        invalid_signature["response"]["error"],
        "Invalid Slack request signature."
    );
}

#[test]
fn command_http_transaction_maps_ingress_contract_errors_to_http_400() {
    let raw_payload = "team_id=T1&user_id=U1&command=%2Fait&text=hello&response_url=https%3A%2F%2Fhooks.slack.com%2Fcommands%2FT%2FB%2FC&trigger_id=trig-missing-channel";
    let planned = agent_slack_command_http_transaction_plan_json(&json!({
        "raw_payload": raw_payload,
        "signature": test_slack_signature(raw_payload),
        "signature_timestamp": SIGNATURE_TIMESTAMP,
        "signing_secret": SIGNING_SECRET,
        "now_unix_seconds": 1714990000,
        "binding": {"conversation_key": "slack:C-ops-1"},
    }))
    .unwrap();

    assert_eq!(planned["command_http_transaction_state"], "ingress_error");
    assert_eq!(planned["http_status"], 400);
    assert_eq!(planned["error_kind"], "invalid_command_payload");
    assert_eq!(
        planned["error"],
        "Slack command payload is missing a channel id."
    );
    assert_eq!(
        planned["response"],
        json!({
            "ok": false,
            "error": "Slack command payload is missing a channel id."
        })
    );
    assert_eq!(planned["should_plan_ingress"], true);
    assert_eq!(planned["should_submit_turn"], false);
    assert_eq!(planned["ingress_plan"], JsonValue::Null);
    assert_eq!(
        planned["http_ingress_plan"]["command_payload"]["trigger_id"],
        "trig-missing-channel"
    );
}

#[test]
fn command_ssl_check_returns_ephemeral_ok_without_python_ingress() {
    let planned = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": {"ssl_check": "1"},
    }))
    .unwrap();

    assert_eq!(planned["stage"], "command");
    assert_eq!(planned["python_ingress_allowed"], false);
    assert_eq!(planned["rust_event_loop_required"], true);
    assert_eq!(planned["ingress_runtime_state"], "ssl_check");
    assert_eq!(
        planned["response"],
        json!({"response_type": "ephemeral", "text": "ok"})
    );
    assert_eq!(planned["should_create_turn"], false);
    assert_eq!(planned["actions"][0]["kind"], "respond_to_ssl_check");
}

#[test]
fn command_requires_text_and_required_fields() {
    let missing_text = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": {
            "channel_id": "C-ops-1",
            "user_id": "U-slack-1",
            "response_url": "https://hooks.slack.com/commands/T000/B000/abc123",
            "text": "  "
        },
    }))
    .unwrap();

    assert_eq!(missing_text["ingress_runtime_state"], "missing_text");
    assert_eq!(
        missing_text["response"],
        json!({
            "response_type": "ephemeral",
            "text": "Slack command must include text content."
        })
    );
    assert_eq!(missing_text["should_submit_turn"], false);

    let missing_channel = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": {
            "user_id": "U-slack-1",
            "response_url": "https://hooks.slack.com/commands/T000/B000/abc123",
            "text": "Hello"
        },
    }))
    .unwrap_err();
    assert_eq!(
        missing_channel,
        "Slack command payload is missing a channel id."
    );

    let missing_response_url = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": {
            "channel_id": "C-ops-1",
            "user_id": "U-slack-1",
            "text": "Hello"
        },
    }))
    .unwrap_err();
    assert_eq!(
        missing_response_url,
        "Slack command payload is missing a response_url."
    );

    let missing_user = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": {
            "channel_id": "C-ops-1",
            "response_url": "https://hooks.slack.com/commands/T000/B000/abc123",
            "text": "Hello"
        },
    }))
    .unwrap_err();
    assert_eq!(missing_user, "Slack command payload is missing a user id.");
}

#[test]
fn command_duplicate_returns_ephemeral_without_turn() {
    let planned = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": command_payload(),
        "binding": {
            "conversation_key": "slack:C-ops-1",
            "slack_recent_request_ids": ["old-request", "1337.2468"]
        },
    }))
    .unwrap();

    assert_eq!(planned["ingress_runtime_state"], "duplicate_ignored");
    assert_eq!(planned["duplicate"], true);
    assert_eq!(planned["accepted"], false);
    assert_eq!(
        planned["response"],
        json!({
            "response_type": "ephemeral",
            "text": "Duplicate Slack command ignored."
        })
    );
    assert_eq!(planned["should_submit_turn"], false);
    assert_eq!(planned["actions"], json!([]));
}

#[test]
fn command_plans_deferred_reply_transport_envelope_and_pending_reply() {
    let planned = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": command_payload(),
        "binding": {
            "conversation_key": "slack:C-ops-1",
            "slack_recent_request_ids": ["old-request"]
        },
        "repo_name": "ait",
        "defer_replies": true,
        "occurred_at": "2026-07-02T10:00:00Z",
    }))
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_slack_ingress_runtime"
    );
    assert_eq!(
        planned["slack_ingress_runtime_contract"],
        "ait_agent_core.event_loop.SlackIngressRuntime.v1"
    );
    assert_eq!(planned["ingress_runtime_state"], "turn_submission_planned");
    assert_eq!(
        planned["response"],
        json!({"response_type": "ephemeral", "text": "ait is thinking..."})
    );
    assert_eq!(planned["should_submit_turn"], true);
    assert_eq!(planned["should_start_background_reply"], true);
    assert_eq!(planned["should_execute_inline_reply"], false);
    assert_eq!(planned["request_id"], "1337.2468");
    assert_eq!(planned["channel_id"], "C-ops-1");
    assert_eq!(planned["channel_title"], "Slack channel · #ops");
    assert_eq!(planned["channel_kind"], "channel");
    assert_eq!(planned["actor_identity"], "slack:U-slack-1");
    assert_eq!(planned["actor_display_name"], "weita");
    assert_eq!(planned["text"], "Hello from Slack");
    assert_eq!(planned["transport_envelope"]["transport"], "slack");
    assert_eq!(planned["transport_envelope"]["event_id"], "1337.2468");
    assert_eq!(
        planned["transport_envelope"]["channel"]["channel_id"],
        "C-ops-1"
    );
    assert_eq!(
        planned["transport_envelope"]["channel"]["channel_title"],
        "Slack channel · #ops"
    );
    assert_eq!(
        planned["transport_envelope"]["metadata"]["command_name"],
        "/ait"
    );
    assert_eq!(
        planned["transport_envelope"]["metadata"]["team_id"],
        "T-team-1"
    );
    assert_eq!(
        planned["transport_envelope"]["metadata"]["response_url_present"],
        true
    );
    assert_eq!(
        planned["pending_reply"]["conversation_key"],
        "slack:C-ops-1"
    );
    assert_eq!(
        planned["pending_reply"]["response_url"],
        "https://hooks.slack.com/commands/T000/B000/abc123"
    );
    assert_eq!(planned["pending_reply"]["request_id"], "1337.2468");
    assert_eq!(
        planned["recent_command_patch"]["slack_recent_request_ids"],
        json!(["old-request", "1337.2468"])
    );
    assert_eq!(planned["actions"][0]["kind"], "upsert_binding");
    assert_eq!(
        planned["actions"][0]["slack_reply_target"]["response_url"],
        "https://hooks.slack.com/commands/T000/B000/abc123"
    );
    assert_eq!(planned["actions"][1]["kind"], "remember_command");
    assert_eq!(planned["actions"][2]["kind"], "start_background_reply");
}

#[test]
fn command_plans_conversation_binding_when_none_exists() {
    let mut payload = command_payload();
    payload["trigger_id"] = json!("1337.9999");
    payload["channel_id"] = json!("D-direct-1");
    payload["channel_name"] = json!("directmessage");
    payload["thread_ts"] = json!("1714990000.000100");

    let planned = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": payload,
        "repo_name": "ait",
    }))
    .unwrap();

    assert_eq!(planned["should_create_turn"], true);
    assert_eq!(
        planned["conversation_key"],
        "slack:D-direct-1:thread:1714990000.000100"
    );
    assert_eq!(planned["channel_kind"], "dm");
    assert_eq!(planned["channel_title"], "Slack DM · D-direct-1");
    assert_eq!(planned["thread_id"], "1714990000.000100");
    assert_eq!(
        planned["pending_reply"]["conversation_key"],
        "slack:D-direct-1:thread:1714990000.000100"
    );
    assert_eq!(planned["pending_reply"]["thread_id"], "1714990000.000100");
    assert_eq!(planned["actions"][0]["kind"], "upsert_binding");
    assert_eq!(planned["actions"][0]["thread_id"], "1714990000.000100");
}

#[test]
fn command_plans_inline_reply_response_decision() {
    let planned = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": command_payload(),
        "binding": {"conversation_key": "slack:C-ops-1"},
        "defer_replies": false,
        "response_type": "in_channel",
        "reply_text": "AI says hello from Slack.",
    }))
    .unwrap();

    assert_eq!(planned["should_start_background_reply"], false);
    assert_eq!(planned["should_execute_inline_reply"], true);
    assert_eq!(
        planned["response"],
        json!({
            "response_type": "in_channel",
            "text": "AI says hello from Slack."
        })
    );
    assert_eq!(planned["actions"][2]["kind"], "execute_inline_reply");
}

#[test]
fn command_request_id_falls_back_without_trigger_id() {
    let mut payload = command_payload();
    payload["trigger_id"] = JsonValue::Null;
    payload["text"] = json!("Fallback request id");

    let planned = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "command",
        "payload": payload,
        "binding": {"conversation_key": "slack:C-ops-1"},
    }))
    .unwrap();

    assert_eq!(
        planned["request_id"],
        "slack:C-ops-1:U-slack-1:/ait:Fallback request id"
    );
    assert_eq!(
        planned["transport_envelope"]["event_id"],
        "slack:C-ops-1:U-slack-1:/ait:Fallback request id"
    );
}

#[test]
fn socket_envelope_wraps_command_ack_when_response_payload_is_accepted() {
    let planned = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "socket_envelope",
        "envelope": {
            "envelope_id": "env-1",
            "type": "slash_commands",
            "accepts_response_payload": true,
            "payload": command_payload()
        },
        "binding": {"conversation_key": "slack:C-ops-1"},
    }))
    .unwrap();

    assert_eq!(planned["stage"], "socket_envelope");
    assert_eq!(planned["ingress_runtime_state"], "command_ack_planned");
    assert_eq!(
        planned["response"],
        json!({
            "envelope_id": "env-1",
            "payload": {"response_type": "ephemeral", "text": "ait is thinking..."}
        })
    );
    assert_eq!(planned["command_plan"]["stage"], "command");
    assert_eq!(planned["command_plan"]["request_id"], "1337.2468");
    assert_eq!(planned["actions"][0]["kind"], "handle_slash_command");
    assert_eq!(planned["actions"][1]["kind"], "ack_socket_envelope");
    assert_eq!(planned["actions"][1]["include_response_payload"], true);
}

#[test]
fn socket_envelope_ignores_non_slash_commands_without_command_plan() {
    let planned = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "socket_envelope",
        "envelope": {
            "envelope_id": "env-2",
            "type": "events_api",
            "payload": {"ignored": true}
        }
    }))
    .unwrap();

    assert_eq!(planned["ingress_runtime_state"], "ignored_envelope");
    assert_eq!(planned["should_handle_command"], false);
    assert_eq!(planned["response"], json!({"envelope_id": "env-2"}));
    assert!(planned.get("command_plan").is_none());
}

#[test]
fn socket_envelope_validates_required_shape() {
    let missing_id = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "socket_envelope",
        "envelope": {"type": "slash_commands", "payload": command_payload()}
    }))
    .unwrap_err();
    assert_eq!(
        missing_id,
        "Slack Socket Mode envelope is missing an envelope id."
    );

    let missing_payload = agent_slack_ingress_runtime_plan_json(&json!({
        "stage": "socket_envelope",
        "envelope": {"envelope_id": "env-3", "type": "slash_commands"}
    }))
    .unwrap_err();
    assert_eq!(
        missing_payload,
        "Slack Socket Mode envelope is missing a slash-command payload object."
    );
}

#[test]
fn socket_mode_transaction_plans_ack_before_command_dispatch() {
    let planned = agent_slack_socket_mode_transaction_plan_json(&json!({
        "envelope": {
            "envelope_id": "env-tx-1",
            "type": "slash_commands",
            "accepts_response_payload": true,
            "payload": command_payload()
        },
        "binding": {"conversation_key": "slack:C-ops-1"},
        "repo_name": "ait",
        "defer_replies": true,
    }))
    .unwrap();

    assert_eq!(planned["stage"], "socket_mode_transaction");
    assert_eq!(
        planned["migration_stage"],
        "rust_agent_slack_socket_mode_transaction"
    );
    assert_eq!(
        planned["slack_socket_mode_transaction_contract"],
        "ait_agent_core.event_loop.SlackSocketModeTransaction.v1"
    );
    assert_eq!(planned["rust_event_loop_required"], true);
    assert_eq!(planned["python_socket_mode_sequencing_allowed"], false);
    assert_eq!(planned["python_socket_mode_ack_allowed"], false);
    assert_eq!(planned["python_websocket_event_loop_allowed"], false);
    assert_eq!(planned["python_ingress_allowed"], false);
    assert_eq!(
        planned["socket_mode_transaction_state"],
        "command_ack_planned"
    );
    assert_eq!(planned["socket_ingress_state"], "command_ack_planned");
    assert_eq!(planned["should_ack_socket_envelope"], true);
    assert_eq!(planned["should_execute_websocket_ack"], true);
    assert_eq!(planned["should_handle_command"], true);
    assert_eq!(planned["should_submit_turn"], true);
    assert_eq!(planned["should_start_background_reply"], true);
    assert_eq!(
        planned["ack_response"],
        json!({
            "envelope_id": "env-tx-1",
            "payload": {"response_type": "ephemeral", "text": "ait is thinking..."}
        })
    );
    assert_eq!(planned["command_plan"]["request_id"], "1337.2468");
    assert_eq!(planned["pending_reply"]["request_id"], "1337.2468");
    assert_eq!(planned["transport_envelope"]["transport"], "slack");
    assert_eq!(planned["actions"][0]["kind"], "ack_socket_envelope");
    assert_eq!(
        planned["actions"][0]["execute_before_command_side_effects"],
        true
    );
    assert_eq!(planned["actions"][1]["kind"], "dispatch_slash_command_plan");
    assert_eq!(planned["actions"][1]["should_submit_turn"], true);
}

#[test]
fn socket_mode_transaction_acks_ignored_envelope_without_command_dispatch() {
    let planned = agent_slack_socket_mode_transaction_plan_json(&json!({
        "envelope": {
            "envelope_id": "env-ignore",
            "type": "events_api",
            "payload": {"ignored": true}
        }
    }))
    .unwrap();

    assert_eq!(
        planned["socket_mode_transaction_state"],
        "ignored_envelope_ack_planned"
    );
    assert_eq!(planned["socket_ingress_state"], "ignored_envelope");
    assert_eq!(planned["accepted"], false);
    assert_eq!(planned["should_ack_socket_envelope"], true);
    assert_eq!(planned["should_handle_command"], false);
    assert_eq!(planned["should_submit_turn"], false);
    assert_eq!(
        planned["ack_response"],
        json!({"envelope_id": "env-ignore"})
    );
    assert_eq!(planned["command_plan"], JsonValue::Null);
    assert_eq!(planned["actions"][0]["kind"], "ack_socket_envelope");
    assert_eq!(planned["actions"].as_array().unwrap().len(), 1);
}

#[test]
fn socket_mode_transaction_fails_closed_without_ack_when_envelope_id_is_missing() {
    let planned = agent_slack_socket_mode_transaction_plan_json(&json!({
        "envelope": {
            "type": "slash_commands",
            "payload": command_payload()
        }
    }))
    .unwrap();

    assert_eq!(
        planned["socket_mode_transaction_state"],
        "invalid_socket_envelope"
    );
    assert_eq!(
        planned["error"],
        "Slack Socket Mode envelope is missing an envelope id."
    );
    assert_eq!(planned["ok"], false);
    assert_eq!(planned["accepted"], false);
    assert_eq!(planned["should_ack_socket_envelope"], false);
    assert_eq!(planned["ack_response"], JsonValue::Null);
    assert_eq!(planned["actions"], json!([]));
}

#[test]
fn socket_mode_transaction_acks_invalid_payload_when_envelope_id_is_available() {
    let planned = agent_slack_socket_mode_transaction_plan_json(&json!({
        "envelope": {
            "envelope_id": "env-invalid",
            "type": "slash_commands"
        }
    }))
    .unwrap();

    assert_eq!(
        planned["socket_mode_transaction_state"],
        "invalid_socket_envelope"
    );
    assert_eq!(
        planned["error"],
        "Slack Socket Mode envelope is missing a slash-command payload object."
    );
    assert_eq!(planned["should_ack_socket_envelope"], true);
    assert_eq!(
        planned["ack_response"],
        json!({"envelope_id": "env-invalid"})
    );
    assert_eq!(planned["should_handle_command"], false);
    assert_eq!(planned["actions"][0]["kind"], "ack_socket_envelope");
    assert_eq!(
        planned["actions"][0]["execute_before_command_side_effects"],
        true
    );
}

#[test]
fn slack_ingress_runtime_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl SlackIngressRuntimePlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "ingress_runtime_state": "substitute",
            }))
        }
    }

    let planned = plan_with_slack_ingress_runtime_planner(
        &SubstitutePlanner,
        &json!({ "stage": "socket_envelope" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "socket_envelope");
    assert_eq!(planned["ingress_runtime_state"], "substitute");
}

#[test]
fn slack_command_http_ingress_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl SlackCommandHttpIngressPlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "command_http_ingress_state": "substitute",
            }))
        }
    }

    let planned = plan_with_slack_command_http_ingress_planner(
        &SubstitutePlanner,
        &json!({ "stage": "http_command_request" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "http_command_request");
    assert_eq!(planned["command_http_ingress_state"], "substitute");
}

#[test]
fn slack_command_http_transaction_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl SlackCommandHttpTransactionPlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "command_http_transaction_state": "substitute",
            }))
        }
    }

    let planned = plan_with_slack_command_http_transaction_planner(
        &SubstitutePlanner,
        &json!({ "stage": "http_command_transaction" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "http_command_transaction");
    assert_eq!(planned["command_http_transaction_state"], "substitute");
}

#[test]
fn slack_socket_mode_transaction_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl SlackSocketModeTransactionPlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "socket_mode_transaction_state": "substitute",
            }))
        }
    }

    let planned = plan_with_slack_socket_mode_transaction_planner(
        &SubstitutePlanner,
        &json!({ "stage": "socket_mode_transaction" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "socket_mode_transaction");
    assert_eq!(planned["socket_mode_transaction_state"], "substitute");
}
