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

impl LineEventJobStatePort for StubState {
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

impl LineEventJobBackendPort for StubBackend {
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

impl LineEventJobDeliveryPort for StubDelivery {
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

fn event_plan() -> JsonValue {
    json!({
        "should_submit_turn": true,
        "channel_id": "G-line-1",
        "channel_title": "LINE group · G-line-1",
        "channel_kind": "group",
        "source_user_id": "U-line-1",
        "message_id": "987654321",
        "reply_token": "line-reply-secret",
        "webhook_event_id": "01HXLINEEVENT001",
        "actor_identity": "line:U-line-1",
        "actor_display_name": "U-line-1",
        "text": "Hello from LINE",
        "transport_envelope": {
            "transport": "line",
            "event_id": "01HXLINEEVENT001",
            "message": {"text": "Hello from LINE"},
        },
    })
}

fn request() -> JsonValue {
    json!({
        "event_plan": event_plan(),
        "state_path": "/tmp/line-state.json",
        "runtime_target": {
            "mode": "remote",
            "workflow_mode": "solo_remote",
            "repo_name": "demo-repo",
            "remote_name": "origin",
            "server_url": "http://127.0.0.1:1",
        },
        "channel_access_token": "line-access-secret",
        "api_base_url": "https://api.line.example",
        "timeout_seconds": 17.5,
    })
}

fn binding() -> JsonValue {
    json!({
        "conversation_key": "line:G-line-1",
        "last_synced_sequence": 41,
        "codex_thread_binding": {"thread_id": "codex-line-1"},
        "line_last_reply_token_seen_at": "2026-07-16T01:02:03Z",
    })
}

fn state_responses_for_existing() -> Vec<(&'static str, PortResult)> {
    vec![
        ("has_recent_value", Ok(json!(false))),
        ("get_binding", Ok(binding())),
        ("upsert_binding", Ok(binding())),
        ("remember_recent_value", Ok(binding())),
    ]
}

fn backend_ok(payload: JsonValue) -> PortResult {
    Ok(json!({"ok": true, "payload": payload}))
}

fn successful_turn(reply_text: Option<&str>) -> JsonValue {
    json!({
        "ok": true,
        "conversation_key": "line:G-line-1",
        "reply_text": reply_text.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "provider_thread": {"thread_id": "codex-line-2"},
        "turn_telemetry": {"ait_cli_commands": {"attempted": 1}},
    })
}

fn delivery_ok() -> PortResult {
    Ok(json!({"ok": true, "delivered": true}))
}

#[test]
fn ignored_events_do_not_touch_ports() {
    let trace = trace();
    let state = StubState::new(trace.clone(), vec![]);
    let backend = StubBackend::new(trace.clone(), vec![]);
    let delivery = StubDelivery::new(trace.clone(), vec![]);
    let result = execute_with_line_event_job_ports(
        &state,
        &backend,
        &delivery,
        &json!({"event_plan": {"should_submit_turn": false}}),
    )
    .unwrap();
    assert_eq!(result["event_job_state"], "ignored");
    assert!(trace.borrow().is_empty());
}

#[test]
fn live_reply_uses_conversation_binding_and_never_calls_a_session_backend() {
    let trace2 = trace();
    let state = StubState::new(trace2.clone(), state_responses_for_existing());
    let backend = StubBackend::new(
        trace2.clone(),
        vec![backend_ok(successful_turn(Some("direct LINE reply")))],
    );
    let delivery = StubDelivery::new(trace2.clone(), vec![delivery_ok()]);

    let result =
        execute_with_line_event_job_ports(&state, &backend, &delivery, &request()).unwrap();

    assert_eq!(result["event_job_state"], "processed");
    assert_eq!(result["conversation_key"], "line:G-line-1");
    assert_eq!(result["binding_created"], false);
    assert_eq!(result["sequence"], 42);
    assert_eq!(delivery.requests.borrow()[0]["text"], "direct LINE reply");
    assert_eq!(
        *trace2.borrow(),
        vec![
            "state:has_recent_value",
            "state:get_binding",
            "state:upsert_binding",
            "backend:create_turn",
            "delivery",
            "state:remember_recent_value",
        ]
    );
    let turn = &backend.requests.borrow()[0];
    assert_eq!(turn["arguments"]["conversation_key"], "line:G-line-1");
    assert_eq!(
        turn["arguments"]["provider_thread"]["thread_id"],
        "codex-line-1"
    );
    assert_eq!(turn["arguments"]["payload"]["surface"], "line");
    assert!(!turn.to_string().contains("session_id"));
    assert!(!result.to_string().contains("line-access-secret"));
    assert!(!result.to_string().contains("line-reply-secret"));
}

#[test]
fn local_target_forwards_provider_configuration_to_the_gateway() {
    let trace = trace();
    let state = StubState::new(trace.clone(), state_responses_for_existing());
    let backend = StubBackend::new(
        trace.clone(),
        vec![backend_ok(successful_turn(Some("local reply")))],
    );
    let delivery = StubDelivery::new(trace, vec![delivery_ok()]);
    let mut local = request();
    local["runtime_target"] = json!({
        "mode": "local",
        "workflow_mode": "solo_local",
        "repo_name": "demo-repo",
        "repo_root": "/tmp/demo-repo",
    });
    local["local_reply"] = json!({"program": "/usr/bin/codex", "args": ["exec", "--json"]});

    execute_with_line_event_job_ports(&state, &backend, &delivery, &local).unwrap();
    let requests = backend.requests.borrow();
    assert_eq!(requests[0]["target"]["mode"], "local");
    assert_eq!(requests[0]["local_reply"]["program"], "/usr/bin/codex");
}

#[test]
fn persistent_duplicate_stops_before_binding_backend_and_delivery() {
    let trace = trace();
    let state = StubState::new(trace.clone(), vec![("has_recent_value", Ok(json!(true)))]);
    let backend = StubBackend::new(trace.clone(), vec![]);
    let delivery = StubDelivery::new(trace.clone(), vec![]);
    let result =
        execute_with_line_event_job_ports(&state, &backend, &delivery, &request()).unwrap();
    assert_eq!(result["event_job_state"], "duplicate");
    assert_eq!(*trace.borrow(), vec!["state:has_recent_value"]);
}

#[test]
fn new_binding_uses_a_deterministic_conversation_key() {
    let trace = trace();
    let state = StubState::new(
        trace.clone(),
        vec![
            ("has_recent_value", Ok(json!(false))),
            ("get_binding", Ok(JsonValue::Null)),
            (
                "upsert_binding",
                Ok(json!({"conversation_key": "line:G-line-1"})),
            ),
            (
                "remember_recent_value",
                Ok(json!({"conversation_key": "line:G-line-1"})),
            ),
        ],
    );
    let backend = StubBackend::new(
        trace.clone(),
        vec![backend_ok(successful_turn(Some("new binding reply")))],
    );
    let delivery = StubDelivery::new(trace, vec![delivery_ok()]);
    let result =
        execute_with_line_event_job_ports(&state, &backend, &delivery, &request()).unwrap();

    assert_eq!(result["binding_created"], true);
    assert_eq!(result["conversation_key"], "line:G-line-1");
    let state_requests = state.requests.borrow();
    assert_eq!(
        state_requests[2].1["updates"]["conversation_key"],
        "line:G-line-1"
    );
    assert!(!state_requests[2].1.to_string().contains("session"));
}

#[test]
fn empty_successful_reply_skips_delivery_but_records_event() {
    let trace = trace();
    let state = StubState::new(trace.clone(), state_responses_for_existing());
    let backend = StubBackend::new(
        trace.clone(),
        vec![backend_ok(successful_turn(Some("   ")))],
    );
    let delivery = StubDelivery::new(trace.clone(), vec![]);
    let result =
        execute_with_line_event_job_ports(&state, &backend, &delivery, &request()).unwrap();
    assert_eq!(result["processed"], true);
    assert_eq!(result["delivery_attempted"], false);
    assert_eq!(result["recorded"], true);
}

#[test]
fn failed_turn_is_recorded_before_the_failure_notice() {
    let trace = trace();
    let state = StubState::new(trace.clone(), state_responses_for_existing());
    let backend = StubBackend::new(
        trace.clone(),
        vec![backend_ok(json!({
            "ok": false,
            "conversation_key": "line:G-line-1",
            "error": "model reply failed",
            "turn_telemetry": {"total_commands": {"attempted": 1}},
        }))],
    );
    let delivery = StubDelivery::new(trace.clone(), vec![delivery_ok()]);
    let result =
        execute_with_line_event_job_ports(&state, &backend, &delivery, &request()).unwrap();
    assert_eq!(result["event_job_state"], "turn_failed_reported");
    assert_eq!(result["turn_ok"], false);
    assert_eq!(
        *trace.borrow(),
        vec![
            "state:has_recent_value",
            "state:get_binding",
            "state:upsert_binding",
            "backend:create_turn",
            "state:remember_recent_value",
            "delivery",
        ]
    );
    assert_eq!(
        delivery.requests.borrow()[0]["text"],
        "The AI reply failed.\nmodel reply failed"
    );
}

#[test]
fn successful_turn_delivery_failure_does_not_mark_event_processed() {
    let trace = trace();
    let mut responses = state_responses_for_existing();
    responses.pop();
    let state = StubState::new(trace.clone(), responses);
    let backend = StubBackend::new(
        trace.clone(),
        vec![backend_ok(successful_turn(Some("will not deliver")))],
    );
    let delivery = StubDelivery::new(trace.clone(), vec![Ok(json!({"ok": false}))]);
    let result =
        execute_with_line_event_job_ports(&state, &backend, &delivery, &request()).unwrap();
    assert_eq!(result["event_job_state"], "reply_delivery_failed");
    assert_eq!(result["recorded"], false);
    assert_eq!(trace.borrow().last().map(String::as_str), Some("delivery"));
}

#[test]
fn backend_and_contract_failures_are_stable_and_redacted() {
    let trace = trace();
    let state = StubState::new(trace.clone(), state_responses_for_existing());
    let backend = StubBackend::new(
        trace.clone(),
        vec![Err("upstream line-access-secret".to_string())],
    );
    let delivery = StubDelivery::new(trace, vec![]);
    let result =
        execute_with_line_event_job_ports(&state, &backend, &delivery, &request()).unwrap();
    assert_eq!(result["event_job_state"], "turn_backend_failed");
    assert!(!result.to_string().contains("line-access-secret"));

    let trace2 = self::trace();
    let state = StubState::new(trace2.clone(), state_responses_for_existing());
    let backend = StubBackend::new(
        trace2.clone(),
        vec![backend_ok(
            json!({"ok": true, "conversation_key": "line:wrong", "reply_text": "x"}),
        )],
    );
    let delivery = StubDelivery::new(trace2, vec![]);
    let result =
        execute_with_line_event_job_ports(&state, &backend, &delivery, &request()).unwrap();
    assert_eq!(result["event_job_state"], "turn_conversation_mismatch");
}

#[test]
fn request_validation_happens_before_port_execution() {
    let trace = trace();
    let state = StubState::new(trace.clone(), vec![]);
    let backend = StubBackend::new(trace.clone(), vec![]);
    let delivery = StubDelivery::new(trace.clone(), vec![]);
    let mut invalid_target = request();
    invalid_target["runtime_target"]["workflow_mode"] = json!("local");
    assert!(
        execute_with_line_event_job_ports(&state, &backend, &delivery, &invalid_target).is_err()
    );
    let mut invalid_timeout = request();
    invalid_timeout["timeout_seconds"] = json!(0);
    assert!(
        execute_with_line_event_job_ports(&state, &backend, &delivery, &invalid_timeout).is_err()
    );
    assert!(trace.borrow().is_empty());
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
                "transport": "line",
                "surface_id": "G-line-1",
                "repo_name": "demo-repo",
                "updates": {
                    "conversation_key": "line:G-line-1",
                    "last_synced_sequence": 41,
                    "codex_thread_binding": {"thread_id": "codex-line-1"},
                },
            }),
        )
        .unwrap();
    let trace = trace();
    let backend = StubBackend::new(
        trace.clone(),
        vec![backend_ok(successful_turn(Some("stored reply")))],
    );
    let delivery = StubDelivery::new(trace, vec![delivery_ok()]);
    let mut real_request = request();
    real_request["state_path"] = JsonValue::String(path.to_string_lossy().into_owned());
    let result = execute_with_line_event_job_ports(
        &DefaultLineEventJobStatePort,
        &backend,
        &delivery,
        &real_request,
    )
    .unwrap();
    assert_eq!(result["processed"], true);
    let binding = store
        .execute(
            "get_binding",
            &json!({"transport": "line", "surface_id": "G-line-1"}),
        )
        .unwrap();
    assert_eq!(binding["conversation_key"], "line:G-line-1");
    assert_eq!(binding["last_synced_sequence"], 42);
    assert_eq!(binding["codex_thread_binding"]["thread_id"], "codex-line-2");
    assert_eq!(
        binding["line_recent_webhook_event_ids"],
        json!(["01HXLINEEVENT001"])
    );
}

#[test]
fn public_entrypoint_rejects_non_object_request() {
    assert_eq!(
        agent_line_event_job_execute_json(&json!([])).unwrap_err(),
        "LINE event job request must be an object."
    );
}
