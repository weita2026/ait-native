use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use tempfile::TempDir;

use super::*;

type PortResult = Result<JsonValue, String>;

#[derive(Clone)]
struct StubState {
    trace: Rc<RefCell<Vec<String>>>,
    responses: Rc<RefCell<VecDeque<(String, PortResult)>>>,
    requests: Rc<RefCell<Vec<(String, JsonValue)>>>,
}

impl StubState {
    fn new(trace: Rc<RefCell<Vec<String>>>, responses: Vec<(&str, PortResult)>) -> Self {
        Self {
            trace,
            responses: Rc::new(RefCell::new(
                responses
                    .into_iter()
                    .map(|(operation, result)| (operation.to_string(), result))
                    .collect(),
            )),
            requests: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl SlackCommandJobStatePort for StubState {
    fn execute_state(
        &self,
        _path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.trace.borrow_mut().push(format!("state:{operation}"));
        self.requests
            .borrow_mut()
            .push((operation.to_string(), request.clone()));
        let (expected, result) = self
            .responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected state operation {operation}"));
        assert_eq!(expected, operation);
        result
    }
}

#[derive(Clone)]
struct StubBackend {
    trace: Rc<RefCell<Vec<String>>>,
    responses: Rc<RefCell<VecDeque<PortResult>>>,
    requests: Rc<RefCell<Vec<JsonValue>>>,
}

impl StubBackend {
    fn new(trace: Rc<RefCell<Vec<String>>>, responses: Vec<PortResult>) -> Self {
        Self {
            trace,
            responses: Rc::new(RefCell::new(responses.into())),
            requests: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl SlackCommandJobBackendPort for StubBackend {
    fn execute_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let operation = request["operation"].as_str().unwrap_or("invalid");
        self.trace.borrow_mut().push(format!("backend:{operation}"));
        self.requests.borrow_mut().push(request.clone());
        self.responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected backend operation {operation}"))
    }
}

#[derive(Clone)]
struct StubDelivery {
    trace: Rc<RefCell<Vec<String>>>,
    responses: Rc<RefCell<VecDeque<PortResult>>>,
    requests: Rc<RefCell<Vec<JsonValue>>>,
}

impl StubDelivery {
    fn new(trace: Rc<RefCell<Vec<String>>>, responses: Vec<PortResult>) -> Self {
        Self {
            trace,
            responses: Rc::new(RefCell::new(responses.into())),
            requests: Rc::new(RefCell::new(Vec::new())),
        }
    }
}

impl SlackCommandJobDeliveryPort for StubDelivery {
    fn execute_delivery(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.trace.borrow_mut().push("delivery".to_string());
        self.requests.borrow_mut().push(request.clone());
        self.responses
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| panic!("unexpected delivery"))
    }
}

fn trace() -> Rc<RefCell<Vec<String>>> {
    Rc::new(RefCell::new(Vec::new()))
}

fn command_payload() -> JsonValue {
    json!({
        "team_id": "T-slack-1",
        "channel_id": "C-slack-1",
        "channel_name": "general",
        "user_id": "U-slack-1",
        "user_name": "alice",
        "command": "/ait",
        "text": "status RT-2735",
        "trigger_id": "trigger-slack-1",
        "response_url": "https://hooks.slack.test/secret-response",
        "thread_ts": "1712345.6789",
    })
}

fn request() -> JsonValue {
    json!({
        "command_payload": command_payload(),
        "state_path": "/tmp/slack-state.json",
        "runtime_target": {
            "mode": "remote",
            "workflow_mode": "solo_remote",
            "repo_name": "demo-repo",
            "remote_name": "origin",
            "server_url": "http://127.0.0.1:1",
        },
        "ack_text": "working",
        "response_type": "in_channel",
        "occurred_at": "2026-07-17T03:04:05Z",
        "timeout_seconds": 17.5,
    })
}

fn binding(recent: Vec<&str>) -> JsonValue {
    json!({
        "transport": "slack",
        "surface_id": "C-slack-1",
        "thread_id": "1712345.6789",
        "conversation_key": "slack:C-slack-1:1712345.6789",
        "repo_name": "demo-repo",
        "slack_recent_request_ids": recent,
        "last_synced_sequence": 41,
        "codex_thread_binding": {"thread_id": "codex-slack-1"},
    })
}

fn state_responses_for_existing() -> Vec<(&'static str, PortResult)> {
    vec![
        ("get_binding", Ok(binding(vec![]))),
        ("upsert_binding", Ok(binding(vec![]))),
        (
            "remember_recent_value",
            Ok(binding(vec!["trigger-slack-1"])),
        ),
        (
            "patch_binding",
            Ok(json!({
                "conversation_key": "slack:C-slack-1:1712345.6789",
                "last_synced_sequence": 42,
                "codex_thread_binding": {"thread_id": "codex-slack-2"},
            })),
        ),
    ]
}

fn backend_ok(payload: JsonValue) -> PortResult {
    Ok(json!({"ok": true, "payload": payload}))
}

fn successful_turn() -> JsonValue {
    json!({
        "ok": true,
        "conversation_key": "slack:C-slack-1:1712345.6789",
        "reply_text": "Slack answer",
        "provider_thread": {"thread_id": "codex-slack-2"},
        "turn_telemetry": {"ait_cli_commands": {"attempted": 1}},
    })
}

fn delivery_ok() -> PortResult {
    Ok(json!({
        "ok": true,
        "delivery_ok": true,
        "should_send_response": true,
        "should_apply_state_patch": true,
        "remember_command_patch": {
            "slack_recent_request_ids": ["trigger-slack-1"],
            "slack_last_request_id": "trigger-slack-1",
            "last_synced_sequence": 42,
            "codex_thread_binding": {"thread_id": "codex-slack-2"},
        },
    }))
}

fn error_delivery_ok() -> PortResult {
    Ok(json!({
        "ok": true,
        "delivery_ok": true,
        "should_send_response": true,
        "should_apply_state_patch": false,
        "remember_command_patch": null,
    }))
}

#[test]
fn existing_binding_executes_without_any_session_backend_call() {
    let trace = trace();
    let state = StubState::new(trace.clone(), state_responses_for_existing());
    let backend = StubBackend::new(trace.clone(), vec![backend_ok(successful_turn())]);
    let delivery = StubDelivery::new(trace.clone(), vec![delivery_ok()]);

    let result = execute_with_slack_command_job_ports(&state, &backend, &delivery, &request())
        .expect("Slack command job");

    assert_eq!(result["command_job_state"], "processed");
    assert_eq!(result["conversation_key"], "slack:C-slack-1:1712345.6789");
    assert_eq!(result["binding_created"], false);
    assert_eq!(result["sequence"], 42);
    assert_eq!(
        *trace.borrow(),
        vec![
            "state:get_binding",
            "state:upsert_binding",
            "state:remember_recent_value",
            "backend:create_turn",
            "delivery",
            "state:patch_binding",
        ]
    );
    let turn = &backend.requests.borrow()[0];
    assert_eq!(
        turn["arguments"]["conversation_key"],
        "slack:C-slack-1:1712345.6789"
    );
    assert_eq!(
        turn["arguments"]["provider_thread"]["thread_id"],
        "codex-slack-1"
    );
    assert!(!turn.to_string().contains("session_id"));
    let pending = &delivery.requests.borrow()[0]["pending_reply"];
    assert_eq!(pending["conversation_key"], "slack:C-slack-1:1712345.6789");
    assert!(pending.get("session_id").is_none());
    assert!(!result.to_string().contains("hooks.slack.test"));
}

#[test]
fn persistent_duplicate_stops_before_gateway_and_delivery() {
    let trace = trace();
    let state = StubState::new(
        trace.clone(),
        vec![("get_binding", Ok(binding(vec!["trigger-slack-1"])))],
    );
    let backend = StubBackend::new(trace.clone(), vec![]);
    let delivery = StubDelivery::new(trace.clone(), vec![]);
    let result = execute_with_slack_command_job_ports(&state, &backend, &delivery, &request())
        .expect("duplicate result");
    assert_eq!(result["command_job_state"], "duplicate");
    assert_eq!(*trace.borrow(), vec!["state:get_binding"]);
}

#[test]
fn new_binding_uses_the_transport_conversation_key() {
    let trace = trace();
    let state = StubState::new(
        trace.clone(),
        vec![
            ("get_binding", Ok(JsonValue::Null)),
            ("upsert_binding", Ok(binding(vec![]))),
            (
                "remember_recent_value",
                Ok(binding(vec!["trigger-slack-1"])),
            ),
            ("patch_binding", Ok(binding(vec!["trigger-slack-1"]))),
        ],
    );
    let backend = StubBackend::new(trace.clone(), vec![backend_ok(successful_turn())]);
    let delivery = StubDelivery::new(trace, vec![delivery_ok()]);
    let result = execute_with_slack_command_job_ports(&state, &backend, &delivery, &request())
        .expect("new binding result");
    assert_eq!(result["binding_created"], true);
    assert_eq!(result["conversation_key"], "slack:C-slack-1:1712345.6789");
    let upsert = &state.requests.borrow()[1].1;
    assert_eq!(
        upsert["updates"]["conversation_key"],
        "slack:C-slack-1:1712345.6789"
    );
    assert!(!upsert.to_string().contains("session"));
}

#[test]
fn delivery_failure_blocks_post_delivery_state_patch() {
    let trace = trace();
    let mut state_responses = state_responses_for_existing();
    state_responses.pop();
    let state = StubState::new(trace.clone(), state_responses);
    let backend = StubBackend::new(trace.clone(), vec![backend_ok(successful_turn())]);
    let delivery = StubDelivery::new(
        trace.clone(),
        vec![Ok(json!({
            "ok": false,
            "delivery_ok": false,
            "should_send_response": true,
            "should_apply_state_patch": false,
        }))],
    );
    let result = execute_with_slack_command_job_ports(&state, &backend, &delivery, &request())
        .expect("delivery failure");
    assert_eq!(result["command_job_state"], "response_delivery_failed");
    assert_eq!(result["recorded"], false);
    assert_eq!(trace.borrow().last().map(String::as_str), Some("delivery"));
}

#[test]
fn gateway_failure_is_reported_without_leaking_backend_or_response_url() {
    let trace = trace();
    let mut state_responses = state_responses_for_existing();
    state_responses.pop();
    let state = StubState::new(trace.clone(), state_responses);
    let backend = StubBackend::new(
        trace.clone(),
        vec![Err(
            "provider echoed https://hooks.slack.test/secret-response".to_string(),
        )],
    );
    let delivery = StubDelivery::new(trace, vec![error_delivery_ok()]);
    let result = execute_with_slack_command_job_ports(&state, &backend, &delivery, &request())
        .expect("gateway failure");
    assert_eq!(result["command_job_state"], "turn_backend_failed");
    assert_eq!(result["delivery_attempted"], true);
    assert_eq!(result["delivered"], true);
    assert!(!result.to_string().contains("hooks.slack.test"));
}

#[test]
fn mismatched_conversation_is_rejected_and_reported() {
    let trace = trace();
    let mut state_responses = state_responses_for_existing();
    state_responses.pop();
    let state = StubState::new(trace.clone(), state_responses);
    let backend = StubBackend::new(
        trace.clone(),
        vec![backend_ok(json!({
            "ok": true,
            "conversation_key": "slack:wrong",
            "reply_text": "wrong",
        }))],
    );
    let delivery = StubDelivery::new(trace, vec![error_delivery_ok()]);
    let result = execute_with_slack_command_job_ports(&state, &backend, &delivery, &request())
        .expect("contract failure");
    assert_eq!(result["command_job_state"], "turn_payload_invalid");
    assert_eq!(result["error_kind"], "backend_contract");
}

#[test]
fn local_target_forwards_provider_configuration_and_validation_is_early() {
    let trace = trace();
    let state = StubState::new(trace.clone(), state_responses_for_existing());
    let backend = StubBackend::new(trace.clone(), vec![backend_ok(successful_turn())]);
    let delivery = StubDelivery::new(trace, vec![delivery_ok()]);
    let mut local = request();
    local["runtime_target"] = json!({
        "mode": "local",
        "workflow_mode": "solo_local",
        "repo_name": "demo-repo",
        "repo_root": "/tmp/demo-repo",
    });
    local["local_reply"] = json!({"program": "/usr/bin/codex", "args": ["exec", "--json"]});
    execute_with_slack_command_job_ports(&state, &backend, &delivery, &local).unwrap();
    assert_eq!(backend.requests.borrow()[0]["target"]["mode"], "local");
    assert_eq!(
        backend.requests.borrow()[0]["local_reply"]["program"],
        "/usr/bin/codex"
    );

    let trace2 = self::trace();
    let state = StubState::new(trace2.clone(), vec![]);
    let backend = StubBackend::new(trace2.clone(), vec![]);
    let delivery = StubDelivery::new(trace2.clone(), vec![]);
    let mut invalid = request();
    invalid["timeout_seconds"] = json!(0);
    assert!(execute_with_slack_command_job_ports(&state, &backend, &delivery, &invalid).is_err());
    assert!(trace2.borrow().is_empty());
}

#[test]
fn real_binding_store_persists_dedupe_delivery_cursor_and_codex_binding() {
    let temp = TempDir::new().unwrap();
    let path = temp.path().join("runtime-state.json");
    let store = AgentRuntimeBindingStore::new(&path);
    store
        .execute(
            "upsert_binding",
            &json!({
                "transport": "slack",
                "surface_id": "C-slack-1",
                "thread_id": "1712345.6789",
                "repo_name": "demo-repo",
                "updates": {
                    "conversation_key": "slack:C-slack-1:1712345.6789",
                    "last_synced_sequence": 41,
                    "codex_thread_binding": {"thread_id": "codex-slack-1"},
                },
            }),
        )
        .unwrap();
    let trace = trace();
    let backend = StubBackend::new(trace.clone(), vec![backend_ok(successful_turn())]);
    let delivery = StubDelivery::new(trace, vec![delivery_ok()]);
    let mut real_request = request();
    real_request["state_path"] = JsonValue::String(path.to_string_lossy().into_owned());
    let result = execute_with_slack_command_job_ports(
        &DefaultSlackCommandJobStatePort,
        &backend,
        &delivery,
        &real_request,
    )
    .unwrap();
    assert_eq!(result["processed"], true);
    let binding = store
        .execute(
            "get_binding",
            &json!({
                "transport": "slack",
                "surface_id": "C-slack-1",
                "thread_id": "1712345.6789",
            }),
        )
        .unwrap();
    assert_eq!(binding["conversation_key"], "slack:C-slack-1:1712345.6789");
    assert_eq!(binding["last_synced_sequence"], 42);
    assert_eq!(
        binding["codex_thread_binding"]["thread_id"],
        "codex-slack-2"
    );
    assert_eq!(
        binding["slack_recent_request_ids"],
        json!(["trigger-slack-1"])
    );
}
