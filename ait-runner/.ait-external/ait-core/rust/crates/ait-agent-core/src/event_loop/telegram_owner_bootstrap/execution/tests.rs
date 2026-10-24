use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use ait_core::json_support::{json, JsonValue};
use tempfile::tempdir;

use super::*;
use crate::event_loop::{
    agent_telegram_owner_bootstrap_plan_json, DefaultTelegramOwnerBootstrapPlanner,
};
use crate::runtime::AgentRuntimeBindingStore;

type Calls = Arc<Mutex<Vec<String>>>;

struct RecordingState {
    auth: Mutex<JsonValue>,
    binding: Mutex<Option<JsonValue>>,
    calls: Calls,
    fail_load: AtomicBool,
    fail_save: AtomicBool,
}

impl RecordingState {
    fn new(auth: JsonValue, binding: Option<JsonValue>, calls: Calls) -> Self {
        Self {
            auth: Mutex::new(auth),
            binding: Mutex::new(binding),
            calls,
            fail_load: AtomicBool::new(false),
            fail_save: AtomicBool::new(false),
        }
    }

    fn auth(&self) -> JsonValue {
        self.auth.lock().unwrap().clone()
    }
}

impl TelegramOwnerBootstrapStatePort for RecordingState {
    fn load_bootstrap_auth(&self) -> Result<JsonValue, String> {
        self.calls.lock().unwrap().push("load_auth".to_string());
        if self.fail_load.load(Ordering::Acquire) {
            Err("private-state-load-secret".to_string())
        } else {
            Ok(self.auth())
        }
    }

    fn load_existing_binding(&self, _chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        self.calls.lock().unwrap().push("load_binding".to_string());
        Ok(self.binding.lock().unwrap().clone())
    }

    fn save_bootstrap_auth(&self, auth_state: &JsonValue) -> Result<(), String> {
        self.calls.lock().unwrap().push("save_auth".to_string());
        if self.fail_save.load(Ordering::Acquire) {
            return Err("private-state-save-secret".to_string());
        }
        *self.auth.lock().unwrap() = auth_state.clone();
        Ok(())
    }
}

struct RecordingClock {
    value: String,
    calls: Calls,
    fail: AtomicBool,
}

impl RecordingClock {
    fn new(calls: Calls) -> Self {
        Self {
            value: "2026-07-17T14:30:00Z".to_string(),
            calls,
            fail: AtomicBool::new(false),
        }
    }
}

impl TelegramOwnerBootstrapClockPort for RecordingClock {
    fn now_iso(&self) -> Result<String, String> {
        self.calls.lock().unwrap().push("clock".to_string());
        if self.fail.load(Ordering::Acquire) {
            Err("private-clock-secret".to_string())
        } else {
            Ok(self.value.clone())
        }
    }
}

struct RecordingMessage {
    messages: Mutex<Vec<String>>,
    calls: Calls,
    fail: AtomicBool,
}

impl RecordingMessage {
    fn new(calls: Calls) -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
            calls,
            fail: AtomicBool::new(false),
        }
    }
}

impl TelegramOwnerBootstrapMessagePort for RecordingMessage {
    fn send_message(&self, _chat_id: &JsonValue, text: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push("send_message".to_string());
        self.messages.lock().unwrap().push(text.to_string());
        if self.fail.load(Ordering::Acquire) {
            Err("private-message-secret".to_string())
        } else {
            Ok(())
        }
    }
}

fn base_request() -> JsonValue {
    json!({
        "kind": "handle",
        "owner_bootstrap_enabled": true,
        "expected_password": "repo-password-secret",
        "config": {
            "repo_name": "repo-password-secret",
            "owner_bootstrap_enabled": true,
        },
        "chat_id": 123,
        "chat": {"id": 123, "type": "private", "first_name": "Wei"},
        "from_user": {"id": 456, "username": "weita", "first_name": "Wei"},
        "chat_title": "private-chat-title",
        "raw_text": JsonValue::Null,
        "command": JsonValue::Null,
        "command_name": JsonValue::Null,
        "attachments_present": false,
    })
}

fn execute(
    state: &RecordingState,
    clock: &RecordingClock,
    message: &RecordingMessage,
    request: &JsonValue,
) -> Result<JsonValue, TelegramOwnerBootstrapExecutionError> {
    execute_with_telegram_owner_bootstrap_ports(
        &DefaultTelegramOwnerBootstrapPlanner,
        state,
        clock,
        message,
        request,
    )
}

#[test]
fn disabled_bootstrap_has_no_dependencies_or_side_effects() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(
        json!({"private": "private-disabled-state-secret"}),
        None,
        Arc::clone(&calls),
    );
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));

    let outcome = execute(
        &state,
        &clock,
        &message,
        &json!({"kind": "handle", "owner_bootstrap_enabled": false}),
    )
    .unwrap();

    assert_eq!(outcome["decision"], "disabled");
    assert_eq!(outcome["handled"], false);
    assert_eq!(outcome["side_effect_count"], 0);
    assert_eq!(outcome["auth_state_loaded"], false);
    assert!(calls.lock().unwrap().is_empty());
    assert!(!outcome
        .to_string()
        .contains("private-disabled-state-secret"));
}

#[test]
fn existing_private_binding_is_loaded_then_adopted_as_owner() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(
        json!({}),
        Some(json!({
            "transport": "telegram",
            "surface_id": "123",
            "conversation_key": "telegram:123",
        })),
        Arc::clone(&calls),
    );
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));

    let outcome = execute(&state, &clock, &message, &base_request()).unwrap();

    assert_eq!(outcome["decision"], "adopt_existing_private_binding");
    assert_eq!(outcome["adopted_owner"], true);
    assert_eq!(outcome["existing_binding_loaded"], true);
    assert_eq!(outcome["state_saved"], true);
    assert_eq!(outcome["message_sent"], false);
    assert_eq!(state.auth()["owner_user_id"], "456");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["load_auth", "clock", "load_binding", "save_auth"]
    );
    let rendered = outcome.to_string();
    assert!(!rendered.contains("telegram:123"));
    assert!(!rendered.contains("123"));
}

#[test]
fn start_prompt_and_password_success_persist_before_message_delivery() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(json!({}), None, Arc::clone(&calls));
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));
    let mut start = base_request();
    start["command"] = json!(["start", ""]);
    start["command_name"] = json!("start");

    let prompted = execute(&state, &clock, &message, &start).unwrap();
    assert_eq!(prompted["decision"], "prompt_start");
    assert_eq!(prompted["side_effect_count"], 2);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "load_auth",
            "clock",
            "load_binding",
            "save_auth",
            "send_message"
        ]
    );
    assert_eq!(state.auth()["pending_user_id"], "456");

    calls.lock().unwrap().clear();
    let mut password = base_request();
    password["raw_text"] = json!("repo-password-secret");
    let verified = execute(&state, &clock, &message, &password).unwrap();

    assert_eq!(verified["decision"], "owner_verified");
    assert_eq!(verified["handled"], true);
    assert_eq!(state.auth()["owner_user_id"], "456");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["load_auth", "clock", "save_auth", "send_message"]
    );
    let rendered = verified.to_string();
    for private in ["repo-password-secret", "private-chat-title", "456", "123"] {
        assert!(!rendered.contains(private));
    }
}

#[test]
fn repeated_bad_passwords_persist_attempts_then_blacklist_without_leaking_text() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(
        json!({
            "pending_user_id": "456",
            "pending_chat_id": "123",
            "pending_started_at": "2026-07-17T14:00:00Z",
        }),
        None,
        Arc::clone(&calls),
    );
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));
    let mut request = base_request();
    request["raw_text"] = json!("bad-password-secret");

    for remaining in [2, 1] {
        let outcome = execute(&state, &clock, &message, &request).unwrap();
        assert_eq!(outcome["decision"], "incorrect_password");
        assert_eq!(state.auth()["failed_attempts"]["456"], 3 - remaining);
        assert!(!outcome.to_string().contains("bad-password-secret"));
    }
    let blocked = execute(&state, &clock, &message, &request).unwrap();
    assert_eq!(blocked["decision"], "blacklist_after_failures");
    assert!(state.auth()["blacklist"]["456"].is_object());
    assert_eq!(message.messages.lock().unwrap().len(), 3);

    let before = calls.lock().unwrap().len();
    let already_blocked = execute(&state, &clock, &message, &request).unwrap();
    assert_eq!(already_blocked["decision"], "blacklisted_user");
    assert_eq!(already_blocked["side_effect_count"], 0);
    assert_eq!(calls.lock().unwrap().len(), before + 2);
    assert_eq!(message.messages.lock().unwrap().len(), 3);
}

#[test]
fn existing_owner_match_continues_and_mismatch_blocks_without_side_effects() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(json!({"owner_user_id": "456"}), None, Arc::clone(&calls));
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));

    let verified = execute(&state, &clock, &message, &base_request()).unwrap();
    assert_eq!(verified["decision"], "owner_verified");
    assert_eq!(verified["handled"], false);
    assert_eq!(verified["blocked"], false);
    assert_eq!(verified["side_effect_count"], 0);

    let mut mismatch = base_request();
    mismatch["from_user"]["id"] = json!(999);
    let blocked = execute(&state, &clock, &message, &mismatch).unwrap();
    assert_eq!(blocked["decision"], "owner_mismatch");
    assert_eq!(blocked["handled"], true);
    assert_eq!(blocked["blocked"], true);
    assert_eq!(blocked["side_effect_count"], 0);
}

#[test]
fn state_failure_stops_message_while_message_failure_keeps_persisted_state() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(json!({}), None, Arc::clone(&calls));
    state.fail_save.store(true, Ordering::Release);
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));
    let mut start = base_request();
    start["command_name"] = json!("start");

    let error = execute(&state, &clock, &message, &start).unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::State
    );
    assert_eq!(
        error.to_string(),
        "Telegram owner bootstrap state execution failed."
    );
    assert!(!error.to_string().contains("secret"));
    assert!(message.messages.lock().unwrap().is_empty());
    assert_eq!(calls.lock().unwrap().last().unwrap(), "save_auth");

    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(json!({}), None, Arc::clone(&calls));
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));
    message.fail.store(true, Ordering::Release);
    let error = execute(&state, &clock, &message, &start).unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::Message
    );
    assert_eq!(state.auth()["pending_user_id"], "456");
    let recorded_calls = calls.lock().unwrap();
    assert_eq!(
        &recorded_calls[recorded_calls.len().saturating_sub(2)..],
        ["save_auth", "send_message"]
    );
}

struct MalformedDependencyPlanner;

impl TelegramOwnerBootstrapPlanner for MalformedDependencyPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let mut planned = agent_telegram_owner_bootstrap_plan_json(request)?;
        planned["load_auth_state"] = json!("private-contract-secret");
        Ok(planned)
    }
}

struct MalformedHandlePlanner;

impl TelegramOwnerBootstrapPlanner for MalformedHandlePlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let mut planned = agent_telegram_owner_bootstrap_plan_json(request)?;
        if request["kind"] == "handle" {
            planned["send_message_text"] = json!(["private-message-secret"]);
        }
        Ok(planned)
    }
}

struct InconsistentDependencyPlanner;

impl TelegramOwnerBootstrapPlanner for InconsistentDependencyPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let mut planned = agent_telegram_owner_bootstrap_plan_json(request)?;
        planned["load_auth_state"] = json!(false);
        planned["load_existing_binding"] = json!(true);
        Ok(planned)
    }
}

struct FailingPlanner;

impl TelegramOwnerBootstrapPlanner for FailingPlanner {
    fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Err("private-planner-secret".to_string())
    }
}

#[test]
fn invalid_requests_plans_state_and_clock_fail_closed_with_stable_errors() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(json!({}), None, Arc::clone(&calls));
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));

    for request in [
        json!([]),
        json!({"owner_bootstrap_enabled": true, "kind": "unsupported-secret"}),
        json!({
            "owner_bootstrap_enabled": true,
            "auth_state": {"owner_user_id": "attacker-secret"},
        }),
    ] {
        let error = execute(&state, &clock, &message, &request).unwrap_err();
        assert_eq!(
            error.kind(),
            TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest
        );
        assert!(!error.to_string().contains("secret"));
    }
    assert!(calls.lock().unwrap().is_empty());

    let error = execute_with_telegram_owner_bootstrap_ports(
        &FailingPlanner,
        &state,
        &clock,
        &message,
        &base_request(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::Planner
    );

    let error = execute_with_telegram_owner_bootstrap_ports(
        &MalformedDependencyPlanner,
        &state,
        &clock,
        &message,
        &base_request(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::PlannerContract
    );
    assert!(calls.lock().unwrap().is_empty());

    let error = execute_with_telegram_owner_bootstrap_ports(
        &InconsistentDependencyPlanner,
        &state,
        &clock,
        &message,
        &base_request(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::PlannerContract
    );
    assert!(calls.lock().unwrap().is_empty());

    let error = execute_with_telegram_owner_bootstrap_ports(
        &MalformedHandlePlanner,
        &state,
        &clock,
        &message,
        &base_request(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::PlannerContract
    );
    assert!(message.messages.lock().unwrap().is_empty());

    let calls = Arc::new(Mutex::new(Vec::new()));
    let invalid_state = RecordingState::new(JsonValue::Null, None, Arc::clone(&calls));
    let clock = RecordingClock::new(Arc::clone(&calls));
    let message = RecordingMessage::new(Arc::clone(&calls));
    let error = execute(&invalid_state, &clock, &message, &base_request()).unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::State
    );
    assert_eq!(*calls.lock().unwrap(), vec!["load_auth"]);

    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(json!({}), None, Arc::clone(&calls));
    let mut clock = RecordingClock::new(Arc::clone(&calls));
    clock.value.clear();
    let message = RecordingMessage::new(Arc::clone(&calls));
    let error = execute(&state, &clock, &message, &base_request()).unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::Clock
    );
    assert_eq!(*calls.lock().unwrap(), vec!["load_auth", "clock"]);

    let calls = Arc::new(Mutex::new(Vec::new()));
    let state = RecordingState::new(json!({}), None, Arc::clone(&calls));
    let mut clock = RecordingClock::new(Arc::clone(&calls));
    clock.value = "private-invalid-clock-secret".to_string();
    let message = RecordingMessage::new(Arc::clone(&calls));
    let error = execute(&state, &clock, &message, &base_request()).unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOwnerBootstrapExecutionErrorKind::Clock
    );
    assert!(!error.to_string().contains("private-invalid-clock-secret"));
}

#[test]
fn runtime_binding_state_port_serializes_concurrent_auth_writes_and_preserves_bindings() {
    let temp = tempdir().unwrap();
    let store = AgentRuntimeBindingStore::new(temp.path().join("bindings.json"));
    store
        .execute(
            "upsert_binding",
            &json!({
                "transport": "telegram",
                "surface_id": "123",
                "repo_name": "ait",
                "updates": {
                    "conversation_key": "telegram:123",
                },
                "now_iso": "2026-07-17T14:00:00Z",
            }),
        )
        .unwrap();
    store
        .execute(
            "upsert_binding",
            &json!({
                "transport": "line",
                "surface_id": "line-secret",
                "repo_name": "ait",
                "updates": {
                    "conversation_key": "line:line-secret",
                },
                "now_iso": "2026-07-17T14:00:00Z",
            }),
        )
        .unwrap();
    let port = RuntimeBindingTelegramOwnerBootstrapStatePort::from_store(store.clone());
    let barrier = Arc::new(Barrier::new(9));
    let mut threads = Vec::new();
    for writer in 0..8 {
        let port = port.clone();
        let barrier = Arc::clone(&barrier);
        threads.push(thread::spawn(move || {
            barrier.wait();
            port.save_bootstrap_auth(&json!({
                "writer": writer,
                "audit_marker": "preserved",
            }))
            .unwrap();
        }));
    }
    barrier.wait();
    for handle in threads {
        handle.join().unwrap();
    }

    let loaded = store.load().unwrap();
    assert_eq!(
        loaded["telegram_bootstrap_auth"]["audit_marker"],
        "preserved"
    );
    assert!(loaded["telegram_bootstrap_auth"]["writer"]
        .as_i64()
        .is_some());
    assert_eq!(
        loaded["surface_bindings"]["telegram:123"]["conversation_key"],
        "telegram:123"
    );
    assert_eq!(
        loaded["surface_bindings"]["line:line-secret"]["conversation_key"],
        "line:line-secret"
    );
    let binding = port.load_existing_binding(&json!("123")).unwrap().unwrap();
    assert_eq!(binding["conversation_key"], "telegram:123");
}
