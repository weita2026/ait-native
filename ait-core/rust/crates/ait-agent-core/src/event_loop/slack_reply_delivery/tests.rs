use super::{
    agent_slack_background_reply_transaction_execute_json, agent_slack_reply_delivery_plan_json,
    agent_slack_response_url_delivery_execute_json,
    execute_with_slack_background_reply_transaction,
    execute_with_slack_response_url_delivery_executor, plan_with_slack_reply_delivery_planner,
    SlackReplyDeliveryPlanner, SlackResponseUrlDeliveryExecutor,
};
use ait_core::json_support::{json, JsonValue};
use std::cell::RefCell;

fn pending_reply() -> JsonValue {
    json!({
        "conversation_key": "slack:C-ops-1:1714990000.000100",
        "channel_id": "C-ops-1",
        "channel_title": "Slack channel · #ops",
        "channel_kind": "channel",
        "response_url": "https://hooks.slack.com/commands/T000/B000/abc123",
        "request_id": "1337.2468",
        "actor_identity": "slack:U-slack-1",
        "actor_display_name": "weita",
        "text": "Hello from Slack",
        "transport_envelope": {"transport": "slack", "event_id": "1337.2468"},
        "source_user_id": "U-slack-1",
        "team_id": "T-team-1",
        "command_name": "/ait",
        "thread_id": "1714990000.000100"
    })
}

fn successful_turn(reply_text: JsonValue, _legacy_assistant_event: JsonValue) -> JsonValue {
    json!({
        "ok": true,
        "conversation_key": "slack:C-ops-1:1714990000.000100",
        "reply_text": reply_text,
        "provider_thread": {"thread_id": "codex-slack-1"},
    })
}

#[test]
fn turn_result_extracts_direct_reply_text_and_plans_delivery_patch() {
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "turn_result",
        "pending_reply": pending_reply(),
        "turn": successful_turn(json!("  AI says hello from Slack.  "), json!({
            "sequence": 8,
            "payload": {"text": "ignored assistant text"}
        })),
        "existing_recent_request_ids": ["old-request", "1337.2468", "other-request"],
        "recent_command_limit": 3,
        "response_type": "in_channel"
    }))
    .unwrap();

    assert_eq!(
        planned["migration_stage"],
        "rust_agent_slack_reply_delivery"
    );
    assert_eq!(
        planned["slack_reply_delivery_contract"],
        "ait_agent_core.event_loop.SlackReplyDelivery.v1"
    );
    assert_eq!(planned["python_reply_delivery_allowed"], false);
    assert_eq!(planned["rust_event_loop_required"], true);
    assert_eq!(planned["reply_delivery_state"], "response_delivery_planned");
    assert_eq!(planned["turn_ok"], true);
    assert_eq!(planned["reply_text"], "AI says hello from Slack.");
    assert_eq!(planned["last_synced_sequence"], 1);
    assert_eq!(planned["should_deliver_response"], true);
    assert_eq!(planned["should_send_response"], true);
    assert_eq!(planned["delivery_operation"]["kind"], "send_response");
    assert_eq!(
        planned["delivery_operation"]["response_url"],
        "https://hooks.slack.com/commands/T000/B000/abc123"
    );
    assert_eq!(planned["delivery_operation"]["response_type"], "in_channel");
    assert_eq!(
        planned["state_patch"]["slack_recent_request_ids"],
        json!(["old-request", "other-request", "1337.2468"])
    );
    assert_eq!(planned["state_patch"]["slack_last_request_id"], "1337.2468");
    assert_eq!(
        planned["state_patch"]["slack_last_source_user_id"],
        "U-slack-1"
    );
    assert_eq!(planned["state_patch"]["slack_last_team_id"], "T-team-1");
    assert_eq!(planned["state_patch"]["slack_last_command_name"], "/ait");
    assert_eq!(planned["state_patch"]["last_synced_sequence"], 1);
    assert_eq!(planned["actions"][0]["kind"], "record_delivered_command");
    assert_eq!(planned["actions"][1]["kind"], "send_response");
}

#[test]
fn turn_result_does_not_fall_back_to_a_legacy_assistant_event() {
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "turn_result",
        "pending_reply": pending_reply(),
        "turn": successful_turn(json!("  "), json!({
            "sequence": 9,
            "payload": {"text": "  Assistant payload reply.  "}
        }))
    }))
    .unwrap();

    assert_eq!(planned["reply_text"], "");
    assert_eq!(planned["last_synced_sequence"], 1);
    assert_eq!(planned["should_deliver_response"], false);
}

#[test]
fn turn_result_does_not_read_a_legacy_transport_reply_event() {
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "turn_result",
        "pending_reply": pending_reply(),
        "turn": successful_turn(JsonValue::Null, json!({
            "sequence": 10,
            "payload": {
                "text": "",
                "transport_reply_envelope": {
                    "message": {"text": " Reply envelope text. "}
                }
            }
        }))
    }))
    .unwrap();

    assert_eq!(planned["reply_text"], "");
    assert_eq!(planned["last_synced_sequence"], 1);
    assert_eq!(planned["delivery_operation"], JsonValue::Null);
}

#[test]
fn turn_result_empty_reply_records_patch_without_delivery() {
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "turn_result",
        "pending_reply": pending_reply(),
        "turn": successful_turn(JsonValue::Null, json!({
            "sequence": 11,
            "payload": {}
        }))
    }))
    .unwrap();

    assert_eq!(
        planned["reply_delivery_state"],
        "turn_completed_without_reply"
    );
    assert_eq!(planned["reply_text"], "");
    assert_eq!(planned["should_deliver_response"], false);
    assert_eq!(planned["should_send_response"], false);
    assert_eq!(planned["delivery_operation"], JsonValue::Null);
    assert_eq!(planned["actions"].as_array().unwrap().len(), 1);
    assert_eq!(planned["actions"][0]["kind"], "record_delivered_command");
}

#[test]
fn turn_result_failed_turn_returns_logged_failure_message_and_patch() {
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "turn_result",
        "pending_reply": pending_reply(),
        "turn": {
            "ok": false,
            "conversation_key": "slack:C-ops-1:1714990000.000100",
            "error": "Backend model timed out."
        },
        "response_type": "in_channel"
    }))
    .unwrap();

    assert_eq!(planned["turn_ok"], false);
    assert_eq!(
        planned["reply_text"],
        "The AI reply failed.\nBackend model timed out."
    );
    assert_eq!(planned["error_text"], "Backend model timed out.");
    assert_eq!(planned["last_synced_sequence"], 1);
    assert_eq!(planned["should_deliver_response"], true);
    assert_eq!(
        planned["delivery_operation"]["text"],
        "The AI reply failed.\nBackend model timed out."
    );
    assert_eq!(planned["state_patch"]["last_synced_sequence"], 1);
}

#[test]
fn turn_result_accepts_the_direct_gateway_shape_without_session_events() {
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "turn_result",
        "pending_reply": pending_reply(),
        "turn": {
            "ok": true,
            "conversation_key": "slack:C-ops-1:1714990000.000100",
            "reply_text": "AI says hello"
        }
    }))
    .unwrap();

    assert_eq!(planned["reply_text"], "AI says hello");
    assert_eq!(planned["last_synced_sequence"], 1);
}

#[test]
fn background_error_plans_ephemeral_error_response_without_state_patch() {
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "background_result",
        "pending_reply": pending_reply(),
        "error": "network unavailable"
    }))
    .unwrap();

    assert_eq!(
        planned["reply_delivery_state"],
        "background_error_delivery_planned"
    );
    assert_eq!(
        planned["reply_text"],
        "ait Slack bot error: network unavailable"
    );
    assert_eq!(planned["response_type"], "ephemeral");
    assert_eq!(planned["delivery_operation"]["response_type"], "ephemeral");
    assert_eq!(
        planned["delivery_operation"]["text"],
        "ait Slack bot error: network unavailable"
    );
    assert_eq!(planned["state_patch"], JsonValue::Null);
    assert_eq!(planned["remember_command_patch"], JsonValue::Null);
    assert_eq!(planned["actions"][0]["kind"], "send_error_response");
}

#[test]
fn inline_response_wraps_turn_reply_text() {
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "inline_response",
        "pending_reply": pending_reply(),
        "turn": successful_turn(json!("Inline AI reply."), json!({
            "sequence": 13,
            "payload": {}
        })),
        "response_type": "ephemeral"
    }))
    .unwrap();

    assert_eq!(planned["stage"], "inline_response");
    assert_eq!(planned["reply_delivery_state"], "inline_response_planned");
    assert_eq!(planned["should_send_response"], false);
    assert_eq!(planned["should_return_inline_response"], true);
    assert_eq!(planned["delivery_operation"], JsonValue::Null);
    assert_eq!(
        planned["response"],
        json!({"response_type": "ephemeral", "text": "Inline AI reply."})
    );
    assert_eq!(planned["actions"][1]["kind"], "return_inline_response");
}

#[test]
fn slack_reply_delivery_bound_entrypoint_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl SlackReplyDeliveryPlanner for SubstitutePlanner {
        fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "stage": request["stage"].clone(),
                "reply_delivery_state": "substitute",
            }))
        }
    }

    let planned = plan_with_slack_reply_delivery_planner(
        &SubstitutePlanner,
        &json!({ "stage": "inline_response" }),
    )
    .unwrap();

    assert_eq!(planned["stage"], "inline_response");
    assert_eq!(planned["reply_delivery_state"], "substitute");
}

#[derive(Debug)]
struct CapturingResponseUrlExecutor {
    requests: RefCell<Vec<JsonValue>>,
    responses: RefCell<Vec<JsonValue>>,
}

impl CapturingResponseUrlExecutor {
    fn new(responses: Vec<JsonValue>) -> Self {
        Self {
            requests: RefCell::new(Vec::new()),
            responses: RefCell::new(responses),
        }
    }
}

impl SlackResponseUrlDeliveryExecutor for CapturingResponseUrlExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.borrow_mut().push(request.clone());
        Ok(if self.responses.borrow().is_empty() {
            json!({"ok": true, "status_code": 200, "payload": {}})
        } else {
            self.responses.borrow_mut().remove(0)
        })
    }
}

#[test]
fn response_url_delivery_execution_splits_chunks_and_posts_json_payloads() {
    let executor = CapturingResponseUrlExecutor::new(vec![
        json!({"ok": true, "status_code": 200, "payload": {}}),
        json!({"ok": true, "status_code": 200, "payload": {}}),
        json!({"ok": true, "status_code": 200, "payload": {}}),
    ]);
    let planned = agent_slack_reply_delivery_plan_json(&json!({
        "stage": "background_result",
        "pending_reply": pending_reply(),
        "turn": successful_turn(json!("alpha beta gamma"), json!({
            "sequence": 8,
            "payload": {}
        })),
        "response_type": "ephemeral"
    }))
    .unwrap();
    let result = execute_with_slack_response_url_delivery_executor(
        &executor,
        &json!({
            "operation": planned["delivery_operation"],
            "message_limit": 8,
            "timeout_seconds": 7.5
        }),
    )
    .unwrap();

    assert_eq!(
        result["migration_stage"],
        "rust_agent_slack_response_url_delivery_execution"
    );
    assert_eq!(
        result["slack_response_url_delivery_execution_contract"],
        "ait_agent_core.event_loop.SlackResponseUrlDeliveryExecution.v1"
    );
    assert_eq!(result["python_response_url_delivery_allowed"], false);
    assert_eq!(result["rust_event_loop_required"], true);
    assert_eq!(result["delivery_execution_state"], "delivered");
    assert_eq!(result["ok"], true);
    assert_eq!(result["delivered"], true);
    assert_eq!(result["chunk_count"], 3);
    assert_eq!(result["delivered_chunk_count"], 3);
    assert_eq!(
        result["operation_results"][0]["http_request"]["url"],
        "[redacted]"
    );

    let requests = executor.requests.borrow();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["method"], "POST");
    assert_eq!(
        requests[0]["url"],
        "https://hooks.slack.com/commands/T000/B000/abc123"
    );
    assert_eq!(requests[0]["headers"]["Content-Type"], "application/json");
    assert_eq!(requests[0]["timeout_seconds"], 7.5);
    assert_eq!(
        requests[0]["payload"],
        json!({
            "text": "alpha",
            "response_type": "ephemeral",
            "replace_original": false,
        })
    );
    assert_eq!(requests[1]["payload"]["text"], "beta");
    assert_eq!(requests[2]["payload"]["text"], "gamma");
}

#[test]
fn response_url_delivery_execution_stops_after_failed_chunk_and_redacts_url() {
    let response_url = "https://hooks.slack.com/commands/T000/B000/abc123";
    let executor = CapturingResponseUrlExecutor::new(vec![
        json!({"ok": true, "status_code": 200, "payload": {}}),
        json!({
            "ok": false,
            "error_kind": "http",
            "method": "POST",
            "url": response_url,
            "message": format!("POST {response_url} failed: 500 boom"),
        }),
        json!({"ok": true, "status_code": 200, "payload": {}}),
    ]);
    let result = execute_with_slack_response_url_delivery_executor(
        &executor,
        &json!({
            "operation": {
                "kind": "send_response",
                "response_url": response_url,
                "text": "alpha beta gamma",
                "response_type": "in_channel",
            },
            "message_limit": 8
        }),
    )
    .unwrap();

    assert_eq!(result["ok"], false);
    assert_eq!(result["delivered"], false);
    assert_eq!(result["delivery_execution_state"], "delivery_failed");
    assert_eq!(result["attempted_chunk_count"], 2);
    assert_eq!(result["delivered_chunk_count"], 1);
    assert_eq!(result["failed_chunk_count"], 1);
    assert_eq!(executor.requests.borrow().len(), 2);
    assert_eq!(
        result["operation_results"][1]["http_result"]["url"],
        "[redacted]"
    );
    assert_eq!(result["error"], "POST [redacted] failed: 500 boom");
}

#[test]
fn response_url_delivery_execution_rejects_unsupported_operation_without_network() {
    let executor = CapturingResponseUrlExecutor::new(Vec::new());
    let result = execute_with_slack_response_url_delivery_executor(
        &executor,
        &json!({
            "operation": {
                "kind": "delete_original",
                "response_url": "https://hooks.slack.com/commands/T000/B000/abc123",
                "text": "ignored"
            }
        }),
    )
    .unwrap();

    assert_eq!(result["ok"], false);
    assert_eq!(result["delivery_execution_state"], "rejected");
    assert_eq!(
        result["error"],
        "Unsupported Slack response URL delivery operation: delete_original."
    );
    assert!(executor.requests.borrow().is_empty());
}

#[test]
fn response_url_delivery_default_entrypoint_uses_rust_http_executor_fail_closed() {
    let result = agent_slack_response_url_delivery_execute_json(&json!({
        "operation": {
            "kind": "send_response",
            "response_url": "https://hooks.slack.com/commands/T000/B000/abc123",
            "text": "hello",
            "response_type": "ephemeral"
        },
        "timeout_seconds": 0
    }))
    .unwrap();

    assert_eq!(result["ok"], false);
    assert_eq!(result["delivery_execution_state"], "delivery_failed");
    assert_eq!(
        result["operation_results"][0]["http_result"]["error_kind"],
        "invalid_timeout"
    );
    assert_eq!(
        result["operation_results"][0]["http_result"]["url"],
        "[redacted]"
    );
}

#[test]
fn background_reply_transaction_plans_and_executes_response_delivery() {
    let executor = CapturingResponseUrlExecutor::new(vec![
        json!({"ok": true, "status_code": 200, "payload": {}}),
        json!({"ok": true, "status_code": 200, "payload": {}}),
    ]);
    let result = execute_with_slack_background_reply_transaction(
        &super::DefaultSlackReplyDeliveryPlanner,
        &executor,
        &json!({
            "stage": "background_result",
            "pending_reply": pending_reply(),
            "turn": successful_turn(json!("alpha beta"), json!({
                "sequence": 8,
                "payload": {}
            })),
            "response_type": "ephemeral",
            "message_limit": 6,
            "timeout_seconds": 7.5
        }),
    )
    .unwrap();

    assert_eq!(
        result["migration_stage"],
        "rust_agent_slack_background_reply_transaction"
    );
    assert_eq!(
        result["slack_background_reply_transaction_contract"],
        "ait_agent_core.event_loop.SlackBackgroundReplyTransaction.v1"
    );
    assert_eq!(result["python_background_reply_transaction_allowed"], false);
    assert_eq!(result["python_reply_delivery_allowed"], false);
    assert_eq!(result["python_response_url_delivery_allowed"], false);
    assert_eq!(result["background_reply_transaction_state"], "completed");
    assert_eq!(result["ok"], true);
    assert_eq!(result["completed"], true);
    assert_eq!(result["reply_delivery_state"], "response_delivery_planned");
    assert_eq!(result["delivery_execution_state"], "delivered");
    assert_eq!(result["should_execute_response_url_delivery"], true);
    assert_eq!(result["should_apply_state_patch"], true);
    assert_eq!(result["state_patch_application_state"], "ready");
    assert_eq!(result["state_patch"]["last_synced_sequence"], 1);
    assert_eq!(result["delivery_result"]["chunk_count"], 2);

    let requests = executor.requests.borrow();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0]["payload"]["text"], "alpha");
    assert_eq!(requests[1]["payload"]["text"], "beta");
    assert_eq!(requests[0]["timeout_seconds"], 7.5);
}

#[test]
fn background_reply_transaction_blocks_state_patch_when_delivery_fails() {
    let response_url = "https://hooks.slack.com/commands/T000/B000/abc123";
    let executor = CapturingResponseUrlExecutor::new(vec![json!({
        "ok": false,
        "error_kind": "http",
        "method": "POST",
        "url": response_url,
        "message": format!("POST {response_url} failed: 503 unavailable"),
    })]);
    let result = execute_with_slack_background_reply_transaction(
        &super::DefaultSlackReplyDeliveryPlanner,
        &executor,
        &json!({
            "stage": "background_result",
            "pending_reply": pending_reply(),
            "turn": successful_turn(json!("hello"), json!({
                "sequence": 8,
                "payload": {}
            }))
        }),
    )
    .unwrap();

    assert_eq!(result["ok"], false);
    assert_eq!(result["completed"], false);
    assert_eq!(
        result["background_reply_transaction_state"],
        "delivery_failed"
    );
    assert_eq!(result["delivery_execution_state"], "delivery_failed");
    assert_eq!(result["should_apply_state_patch"], false);
    assert_eq!(
        result["state_patch_application_state"],
        "blocked_by_delivery_failure"
    );
    assert_eq!(result["state_patch"]["last_synced_sequence"], 1);
    assert_eq!(
        result["delivery_result"]["operation_results"][0]["http_result"]["url"],
        "[redacted]"
    );
    assert_eq!(result["error"], "POST [redacted] failed: 503 unavailable");
}

#[test]
fn background_reply_transaction_records_patch_without_network_for_empty_reply() {
    let executor = CapturingResponseUrlExecutor::new(Vec::new());
    let result = execute_with_slack_background_reply_transaction(
        &super::DefaultSlackReplyDeliveryPlanner,
        &executor,
        &json!({
            "stage": "background_result",
            "pending_reply": pending_reply(),
            "turn": successful_turn(JsonValue::Null, json!({
                "sequence": 9,
                "payload": {}
            }))
        }),
    )
    .unwrap();

    assert_eq!(result["ok"], true);
    assert_eq!(result["completed"], true);
    assert_eq!(
        result["background_reply_transaction_state"],
        "completed_without_response_delivery"
    );
    assert_eq!(result["delivery_result"], JsonValue::Null);
    assert_eq!(result["should_execute_response_url_delivery"], false);
    assert_eq!(result["should_apply_state_patch"], true);
    assert_eq!(result["state_patch_application_state"], "ready");
    assert_eq!(result["state_patch"]["last_synced_sequence"], 1);
    assert!(executor.requests.borrow().is_empty());
}

#[test]
fn background_reply_transaction_delivers_background_error_without_state_patch() {
    let executor = CapturingResponseUrlExecutor::new(vec![json!({
        "ok": true,
        "status_code": 200,
        "payload": {}
    })]);
    let result = execute_with_slack_background_reply_transaction(
        &super::DefaultSlackReplyDeliveryPlanner,
        &executor,
        &json!({
            "stage": "background_result",
            "pending_reply": pending_reply(),
            "error": "worker crashed"
        }),
    )
    .unwrap();

    assert_eq!(result["ok"], true);
    assert_eq!(result["background_reply_transaction_state"], "completed");
    assert_eq!(
        result["reply_delivery_state"],
        "background_error_delivery_planned"
    );
    assert_eq!(result["delivery_execution_state"], "delivered");
    assert_eq!(result["state_patch"], JsonValue::Null);
    assert_eq!(result["should_apply_state_patch"], false);
    assert_eq!(result["state_patch_application_state"], "not_required");
    assert_eq!(
        executor.requests.borrow()[0]["payload"]["text"],
        "ait Slack bot error: worker crashed"
    );
}

#[test]
fn background_reply_transaction_accepts_substitute_planner() {
    struct SubstitutePlanner;

    impl SlackReplyDeliveryPlanner for SubstitutePlanner {
        fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
            Ok(json!({
                "reply_delivery_state": "turn_completed_without_reply",
                "should_send_response": false,
                "state_patch": {"last_synced_sequence": 42},
                "remember_command_patch": {"last_synced_sequence": 42},
            }))
        }
    }

    let executor = CapturingResponseUrlExecutor::new(Vec::new());
    let result = execute_with_slack_background_reply_transaction(
        &SubstitutePlanner,
        &executor,
        &json!({"pending_reply": pending_reply()}),
    )
    .unwrap();

    assert_eq!(
        result["background_reply_transaction_state"],
        "completed_without_response_delivery"
    );
    assert_eq!(result["should_apply_state_patch"], true);
    assert!(executor.requests.borrow().is_empty());
}

#[test]
fn background_reply_transaction_default_entrypoint_uses_rust_http_executor_fail_closed() {
    let result = agent_slack_background_reply_transaction_execute_json(&json!({
        "stage": "background_result",
        "pending_reply": pending_reply(),
        "turn": successful_turn(json!("hello"), json!({
            "sequence": 8,
            "payload": {}
        })),
        "timeout_seconds": 0
    }))
    .unwrap();

    assert_eq!(result["ok"], false);
    assert_eq!(
        result["background_reply_transaction_state"],
        "delivery_failed"
    );
    assert_eq!(result["delivery_execution_state"], "delivery_failed");
    assert_eq!(result["should_apply_state_patch"], false);
    assert_eq!(
        result["state_patch_application_state"],
        "blocked_by_delivery_failure"
    );
    assert_eq!(
        result["delivery_result"]["operation_results"][0]["http_result"]["error_kind"],
        "invalid_timeout"
    );
}
