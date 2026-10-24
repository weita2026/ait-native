use std::cell::{Cell, RefCell};
use std::rc::Rc;

use ait_core::json_support::{json, JsonValue};

use super::*;

#[derive(Clone)]
struct StubState {
    last_update_id: Cell<i64>,
    fail_operation: RefCell<Option<String>>,
    calls: Rc<RefCell<Vec<String>>>,
}

impl StubState {
    fn new(last_update_id: i64, calls: Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            last_update_id: Cell::new(last_update_id),
            fail_operation: RefCell::new(None),
            calls,
        }
    }
}

impl TelegramServiceCycleStatePort for StubState {
    fn execute_state(
        &self,
        _path: &str,
        operation: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.calls.borrow_mut().push(format!("state:{operation}"));
        if self.fail_operation.borrow().as_deref() == Some(operation) {
            return Err("state-secret-value".to_string());
        }
        match operation {
            "load" => Ok(json!({"last_update_id": self.last_update_id.get()})),
            "update_last_update_id" => {
                let update_id = optional_i64(request.get("update_id")).unwrap_or(0);
                self.last_update_id.set(update_id);
                Ok(json!({"last_update_id": update_id}))
            }
            _ => Err("unsupported stub state operation".to_string()),
        }
    }
}

struct StubPoll {
    updates: RefCell<Vec<JsonValue>>,
    fail: Cell<bool>,
    calls: Rc<RefCell<Vec<String>>>,
    requests: RefCell<Vec<JsonValue>>,
}

impl StubPoll {
    fn new(updates: Vec<JsonValue>, calls: Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            updates: RefCell::new(updates),
            fail: Cell::new(false),
            calls,
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramServiceCyclePollPort for StubPoll {
    fn poll_updates(&self, request: &JsonValue) -> Result<Vec<JsonValue>, String> {
        self.calls.borrow_mut().push("poll".to_string());
        self.requests.borrow_mut().push(request.clone());
        if self.fail.get() {
            return Err("bot-token-secret poll failure".to_string());
        }
        Ok(self.updates.borrow().clone())
    }
}

struct StubDispatch {
    fail_call: Cell<Option<usize>>,
    calls: Rc<RefCell<Vec<String>>>,
    requests: RefCell<Vec<JsonValue>>,
}

impl StubDispatch {
    fn new(calls: Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            fail_call: Cell::new(None),
            calls,
            requests: RefCell::new(Vec::new()),
        }
    }
}

impl TelegramServiceCycleDispatchPort for StubDispatch {
    fn dispatch_update(&self, request: &JsonValue) -> Result<(), String> {
        let call_number = self.requests.borrow().len() + 1;
        self.calls
            .borrow_mut()
            .push(format!("dispatch:{call_number}"));
        self.requests.borrow_mut().push(request.clone());
        if self.fail_call.get() == Some(call_number) {
            return Err("dispatch-secret-value".to_string());
        }
        Ok(())
    }
}

struct StubBackgroundSync {
    future_count: usize,
    fail: Cell<bool>,
    calls: Rc<RefCell<Vec<String>>>,
}

impl StubBackgroundSync {
    fn new(calls: Rc<RefCell<Vec<String>>>) -> Self {
        Self {
            future_count: 0,
            fail: Cell::new(false),
            calls,
        }
    }
}

impl TelegramServiceCycleBackgroundSyncPort for StubBackgroundSync {
    fn run_background_sync_once(&self, _request: &JsonValue) -> Result<usize, String> {
        self.calls.borrow_mut().push("background".to_string());
        if self.fail.get() {
            return Err("background-secret-value".to_string());
        }
        Ok(self.future_count)
    }
}

fn request() -> JsonValue {
    json!({
        "state_path": "/tmp/telegram-service-cycle-state.json",
        "poll_timeout_seconds": 30,
        "background_sync_enabled": false,
        "background_sync_interval_seconds": 60.0,
        "now_monotonic_seconds": 100.0,
        "next_background_sync_at": null,
    })
}

fn execute(
    state: &StubState,
    poll: &StubPoll,
    dispatch: &StubDispatch,
    background: &StubBackgroundSync,
    request: &JsonValue,
) -> JsonValue {
    execute_with_telegram_service_cycle_ports(state, poll, dispatch, background, request)
        .expect("service-cycle execution")
}

#[test]
fn empty_poll_preserves_cursor_and_uses_planned_offset_timeout() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(57, calls.clone());
    let poll = StubPoll::new(Vec::new(), calls.clone());
    let dispatch = StubDispatch::new(calls.clone());
    let background = StubBackgroundSync::new(calls.clone());

    let outcome = execute(&state, &poll, &dispatch, &background, &request());

    assert_eq!(outcome["contract"], CONTRACT);
    assert_eq!(outcome["service_cycle_state"], "empty_poll");
    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["update_count"], 0);
    assert_eq!(outcome["last_update_id_before"], 57);
    assert_eq!(outcome["last_update_id_after"], 57);
    assert_eq!(outcome["cursor_updated"], false);
    assert_eq!(outcome["python_service_loop_allowed"], false);
    assert_eq!(outcome["python_telegram_api_allowed"], false);
    assert_eq!(outcome["python_update_dispatch_allowed"], false);
    assert_eq!(
        poll.requests.borrow()[0]["poll_request"],
        json!({"offset": 58, "timeout_seconds": 30})
    );
    assert_eq!(&*calls.borrow(), &["state:load", "poll"]);
}

#[test]
fn dispatches_batch_in_order_then_advances_cursor_exactly_once() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(4, calls.clone());
    let poll = StubPoll::new(
        vec![
            json!({"update_id": 9, "message": {"message_id": 1, "chat": {"id": 123}}}),
            json!({"update_id": 7, "message": {"message_id": 2, "chat": {"id": 456}}}),
            json!({"message": {"message_id": 3, "chat": {}}}),
        ],
        calls.clone(),
    );
    let dispatch = StubDispatch::new(calls.clone());
    let background = StubBackgroundSync::new(calls.clone());

    let outcome = execute(&state, &poll, &dispatch, &background, &request());

    assert_eq!(outcome["service_cycle_state"], "completed");
    assert_eq!(outcome["update_count"], 3);
    assert_eq!(outcome["dispatched_count"], 3);
    assert_eq!(outcome["last_update_id_before"], 4);
    assert_eq!(outcome["last_update_id_after"], 7);
    assert_eq!(outcome["cursor_updated"], true);
    assert_eq!(state.last_update_id.get(), 7);
    assert_eq!(
        dispatch
            .requests
            .borrow()
            .iter()
            .map(|request| request["queue_key"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec!["chat-123", "chat-456", "update-unknown"]
    );
    assert_eq!(
        &*calls.borrow(),
        &[
            "state:load",
            "poll",
            "dispatch:1",
            "dispatch:2",
            "dispatch:3",
            "state:update_last_update_id",
        ]
    );
}

#[test]
fn batch_without_update_ids_dispatches_without_cursor_mutation() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(12, calls.clone());
    let poll = StubPoll::new(
        vec![
            json!({"message": {"message_id": 44, "chat": {}}}),
            json!({}),
        ],
        calls.clone(),
    );
    let dispatch = StubDispatch::new(calls.clone());
    let background = StubBackgroundSync::new(calls.clone());

    let outcome = execute(&state, &poll, &dispatch, &background, &request());

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["dispatched_count"], 2);
    assert_eq!(outcome["cursor_updated"], false);
    assert_eq!(outcome["last_update_id_after"], 12);
    assert_eq!(state.last_update_id.get(), 12);
    assert!(!calls
        .borrow()
        .iter()
        .any(|call| call == "state:update_last_update_id"));
}

#[test]
fn due_background_sync_runs_before_poll_and_disabled_sync_never_runs() {
    let due_calls = Rc::new(RefCell::new(Vec::new()));
    let due_state = StubState::new(0, due_calls.clone());
    let due_poll = StubPoll::new(Vec::new(), due_calls.clone());
    let due_dispatch = StubDispatch::new(due_calls.clone());
    let mut due_background = StubBackgroundSync::new(due_calls.clone());
    due_background.future_count = 2;
    let mut due_request = request();
    due_request["background_sync_enabled"] = JsonValue::Bool(true);
    due_request["background_sync_interval_seconds"] = json!(30.0);
    due_request["now_monotonic_seconds"] = json!(131.0);
    due_request["next_background_sync_at"] = json!(130.0);

    let due = execute(
        &due_state,
        &due_poll,
        &due_dispatch,
        &due_background,
        &due_request,
    );
    assert_eq!(due["background_sync_due"], true);
    assert_eq!(due["background_sync_ran"], true);
    assert_eq!(due["background_future_count"], 2);
    assert_eq!(due["next_background_sync_at"], 161.0);
    assert_eq!(&*due_calls.borrow(), &["state:load", "background", "poll"]);

    let disabled_calls = Rc::new(RefCell::new(Vec::new()));
    let disabled_state = StubState::new(0, disabled_calls.clone());
    let disabled_poll = StubPoll::new(Vec::new(), disabled_calls.clone());
    let disabled_dispatch = StubDispatch::new(disabled_calls.clone());
    let disabled_background = StubBackgroundSync::new(disabled_calls.clone());
    let mut disabled_request = due_request;
    disabled_request["background_sync_enabled"] = JsonValue::Bool(false);
    let disabled = execute(
        &disabled_state,
        &disabled_poll,
        &disabled_dispatch,
        &disabled_background,
        &disabled_request,
    );
    assert_eq!(disabled["background_sync_due"], false);
    assert_eq!(disabled["background_sync_ran"], false);
    assert_eq!(&*disabled_calls.borrow(), &["state:load", "poll"]);
}

#[test]
fn state_load_poll_and_background_failures_are_stable_and_secret_safe() {
    let state_calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(0, state_calls.clone());
    *state.fail_operation.borrow_mut() = Some("load".to_string());
    let poll = StubPoll::new(Vec::new(), state_calls.clone());
    let dispatch = StubDispatch::new(state_calls.clone());
    let background = StubBackgroundSync::new(state_calls);
    let mut state_failure_request = request();
    state_failure_request["next_background_sync_at"] = json!(144.0);
    let state_failure = execute(
        &state,
        &poll,
        &dispatch,
        &background,
        &state_failure_request,
    );
    assert_eq!(state_failure["service_cycle_state"], "state_load_failed");
    assert_eq!(state_failure["error_kind"], "state");
    assert_eq!(state_failure["next_background_sync_at"], 144.0);
    assert!(!state_failure.to_string().contains("state-secret-value"));

    let poll_calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(0, poll_calls.clone());
    let poll = StubPoll::new(Vec::new(), poll_calls.clone());
    poll.fail.set(true);
    let dispatch = StubDispatch::new(poll_calls.clone());
    let background = StubBackgroundSync::new(poll_calls);
    let poll_failure = execute(&state, &poll, &dispatch, &background, &request());
    assert_eq!(poll_failure["service_cycle_state"], "poll_failed");
    assert_eq!(poll_failure["error_kind"], "poll");
    assert!(!poll_failure.to_string().contains("bot-token-secret"));

    let background_calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(0, background_calls.clone());
    let poll = StubPoll::new(Vec::new(), background_calls.clone());
    let dispatch = StubDispatch::new(background_calls.clone());
    let background = StubBackgroundSync::new(background_calls);
    background.fail.set(true);
    let mut due_request = request();
    due_request["background_sync_enabled"] = JsonValue::Bool(true);
    due_request["now_monotonic_seconds"] = json!(61.0);
    due_request["next_background_sync_at"] = json!(60.0);
    let background_failure = execute(&state, &poll, &dispatch, &background, &due_request);
    assert_eq!(
        background_failure["service_cycle_state"],
        "background_sync_failed"
    );
    assert!(!background_failure
        .to_string()
        .contains("background-secret-value"));
    assert!(poll.requests.borrow().is_empty());
}

#[test]
fn dispatch_failure_stops_batch_before_cursor_update() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(3, calls.clone());
    let poll = StubPoll::new(
        vec![json!({"update_id": 4}), json!({"update_id": 5})],
        calls.clone(),
    );
    let dispatch = StubDispatch::new(calls.clone());
    dispatch.fail_call.set(Some(2));
    let background = StubBackgroundSync::new(calls.clone());

    let outcome = execute(&state, &poll, &dispatch, &background, &request());

    assert_eq!(outcome["service_cycle_state"], "dispatch_failed");
    assert_eq!(outcome["dispatched_count"], 1);
    assert_eq!(outcome["cursor_updated"], false);
    assert_eq!(state.last_update_id.get(), 3);
    assert!(!outcome.to_string().contains("dispatch-secret-value"));
    assert!(!calls
        .borrow()
        .iter()
        .any(|call| call == "state:update_last_update_id"));
}

#[test]
fn cursor_failure_occurs_only_after_all_dispatches_and_preserves_public_cursor() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(3, calls.clone());
    *state.fail_operation.borrow_mut() = Some("update_last_update_id".to_string());
    let poll = StubPoll::new(
        vec![json!({"update_id": 4}), json!({"update_id": 5})],
        calls.clone(),
    );
    let dispatch = StubDispatch::new(calls.clone());
    let background = StubBackgroundSync::new(calls.clone());

    let outcome = execute(&state, &poll, &dispatch, &background, &request());

    assert_eq!(outcome["service_cycle_state"], "cursor_update_failed");
    assert_eq!(outcome["dispatched_count"], 2);
    assert_eq!(outcome["last_update_id_before"], 3);
    assert_eq!(outcome["last_update_id_after"], 3);
    assert_eq!(outcome["cursor_updated"], false);
    assert_eq!(
        &*calls.borrow(),
        &[
            "state:load",
            "poll",
            "dispatch:1",
            "dispatch:2",
            "state:update_last_update_id",
        ]
    );
}

#[test]
fn invalid_request_is_rejected_before_any_port_runs() {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let state = StubState::new(0, calls.clone());
    let poll = StubPoll::new(Vec::new(), calls.clone());
    let dispatch = StubDispatch::new(calls.clone());
    let background = StubBackgroundSync::new(calls.clone());

    let error = execute_with_telegram_service_cycle_ports(
        &state,
        &poll,
        &dispatch,
        &background,
        &json!({"state_path": "", "poll_timeout_seconds": -1}),
    )
    .expect_err("invalid request");

    assert!(error.contains("state_path"));
    assert!(calls.borrow().is_empty());
}
