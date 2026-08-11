use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ait_core::json_support::{json, JsonValue};

use super::*;

#[derive(Clone)]
struct RecordingMessageExecutor {
    requests: Arc<Mutex<Vec<JsonValue>>>,
    outcome: Arc<Mutex<Result<JsonValue, String>>>,
}

impl RecordingMessageExecutor {
    fn successful() -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            outcome: Arc::new(Mutex::new(Ok(valid_message_outcome()))),
        }
    }

    fn returning(outcome: Result<JsonValue, String>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            outcome: Arc::new(Mutex::new(outcome)),
        }
    }

    fn requests(&self) -> Vec<JsonValue> {
        self.requests.lock().unwrap().clone()
    }
}

impl TelegramUpdateMessageExecutor for RecordingMessageExecutor {
    fn execute_message(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.lock().unwrap().push(request.clone());
        self.outcome.lock().unwrap().clone()
    }
}

fn valid_message_outcome() -> JsonValue {
    json!({
        "contract": MESSAGE_CONTRACT,
        "migration_stage": MESSAGE_MIGRATION_STAGE,
        "stage": "execute",
        "message_delivery_state": "completed",
        "ok": true,
        "completed": true,
        "chunk_count": 1,
        "completed_chunk_count": 1,
        "failed_chunk_index": null,
        "fallback_count": 0,
        "api_call_count": 1,
        "chunk_results": [{
            "index": 0,
            "delivered": true,
            "fallback_used": false,
            "api_call_count": 1,
            "attempt_count": 1,
            "http_status_code": 200,
            "state": "completed",
            "error_kind": null,
        }],
        "error_kind": null,
        "error": null,
        "python_message_delivery_allowed": false,
        "python_message_formatting_allowed": false,
        "raw_api_result_exposed": false,
        "telegram_description_exposed": false,
        "token_bearing_url_exposed": false,
        "chat_id_exposed": false,
        "formatted_text_exposed": false,
        "plain_text_exposed": false,
    })
}

#[test]
fn shared_message_adapter_routes_update_owner_and_command_traits_with_exact_config() {
    let executor = RecordingMessageExecutor::successful();
    let port = NativeTelegramUpdateMessagePort::with_executor(
        "123:bot-token-super-secret",
        Some(12.5),
        true,
        executor.clone(),
    )
    .unwrap();

    TelegramUpdateDeliveryPort::send_message(&port, &json!(100), "update message").unwrap();
    TelegramOwnerBootstrapMessagePort::send_message(&port, &json!("-200"), "owner prompt").unwrap();
    TelegramCommandRuntimeDeliveryPort::send_message(&port, &json!(300), "command result").unwrap();

    let requests = executor.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["chat_id"], 100);
    assert_eq!(requests[0]["text"], "update message");
    assert_eq!(requests[1]["chat_id"], "-200");
    assert_eq!(requests[1]["text"], "owner prompt");
    assert_eq!(requests[2]["text"], "command result");
    for request in requests {
        assert_eq!(request["bot_token"], "123:bot-token-super-secret");
        assert_eq!(request["request_timeout_seconds"], 12.5);
        assert_eq!(request["reply_markdown_enabled"], true);
    }
    let debug = format!("{port:?}");
    assert!(debug.contains("bot_token_exposed: false"));
    assert!(!debug.contains("bot-token-super-secret"));
    assert!(!debug.contains("update message"));
}

#[test]
fn message_adapter_rejects_executor_and_contract_corruption_without_secrets() {
    let mut corrupt_count = valid_message_outcome();
    corrupt_count["completed_chunk_count"] = json!(0);
    let mut corrupt_flags = valid_message_outcome();
    corrupt_flags["python_message_delivery_allowed"] = json!(true);
    let mut corrupt_chunk = valid_message_outcome();
    corrupt_chunk["chunk_results"][0]["http_status_code"] = json!(500);

    for outcome in [
        Err("executor-token-super-secret".to_string()),
        Ok(json!({"private": "contract-super-secret"})),
        Ok(corrupt_count),
        Ok(corrupt_flags),
        Ok(corrupt_chunk),
    ] {
        let port = NativeTelegramUpdateMessagePort::with_executor(
            "token-super-secret",
            None,
            false,
            RecordingMessageExecutor::returning(outcome),
        )
        .unwrap();
        let error = TelegramOwnerBootstrapMessagePort::send_message(
            &port,
            &json!(100),
            "message-super-secret",
        )
        .unwrap_err();
        assert_eq!(error, "Telegram update message execution failed.");
        assert!(!error.contains("secret"));
    }
}

#[test]
fn message_adapter_bounds_inputs_and_configuration_before_execution() {
    let executor = RecordingMessageExecutor::successful();
    let port =
        NativeTelegramUpdateMessagePort::with_executor("token", None, false, executor.clone())
            .unwrap();
    for (chat_id, text) in [
        (JsonValue::Null, "message".to_string()),
        (json!(" padded "), "message".to_string()),
        (json!(100), String::new()),
        (json!(100), "x".repeat(MAX_MESSAGE_CHARS + 1)),
        (json!(100), "nul\0message".to_string()),
    ] {
        assert!(TelegramCommandRuntimeDeliveryPort::send_message(&port, &chat_id, &text).is_err());
    }
    assert!(executor.requests().is_empty());

    for (token, timeout) in [
        ("", None),
        (" token ", None),
        ("token\nsuper-secret", None),
        ("token", Some(0.0)),
        ("token", Some(f64::NAN)),
    ] {
        let error = NativeTelegramUpdateMessagePort::with_executor(
            token,
            timeout,
            false,
            RecordingMessageExecutor::successful(),
        )
        .unwrap_err();
        assert_eq!(error, "Telegram update message configuration is invalid.");
        assert!(!error.contains("super-secret"));
    }
}

#[derive(Clone)]
struct RecordingBootstrapExecutor {
    requests: Arc<Mutex<Vec<JsonValue>>>,
    outcome: Result<JsonValue, String>,
}

impl RecordingBootstrapExecutor {
    fn returning(outcome: Result<JsonValue, String>) -> Self {
        Self {
            requests: Arc::new(Mutex::new(Vec::new())),
            outcome,
        }
    }
}

impl TelegramUpdateOwnerBootstrapExecutor for RecordingBootstrapExecutor {
    fn execute_owner_bootstrap(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.requests.lock().unwrap().push(request.clone());
        self.outcome.clone()
    }
}

fn valid_bootstrap_outcome(decision: &str, handled: bool) -> JsonValue {
    json!({
        "contract": BOOTSTRAP_CONTRACT,
        "migration_stage": BOOTSTRAP_MIGRATION_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "owner_bootstrap_state": "completed",
        "ok": true,
        "completed": true,
        "decision": decision,
        "handled": handled,
        "blocked": handled,
        "adopted_owner": false,
        "auth_state_loaded": handled,
        "existing_binding_loaded": false,
        "state_saved": false,
        "message_sent": false,
        "side_effect_count": 0,
        "rust_state_execution_required": true,
        "rust_message_delivery_required": true,
        "python_owner_bootstrap_allowed": false,
        "python_state_mutation_allowed": false,
        "python_message_delivery_allowed": false,
        "request_payload_exposed": false,
        "auth_state_exposed": false,
        "chat_id_exposed": false,
        "message_text_exposed": false,
    })
}

fn bootstrap_request(
    raw_text: Option<&str>,
    command: Option<(&str, &str)>,
    attachments_present: bool,
) -> TelegramUpdateBootstrapRequest {
    TelegramUpdateBootstrapRequest {
        chat_id: json!(100),
        chat: json!({"id": 100, "type": "private", "first_name": "Wei"}),
        from_user: json!({"id": 7, "username": "wei", "first_name": "Wei"}),
        chat_title: "Wei private".to_string(),
        raw_text: raw_text.map(str::to_string),
        command: command.map(|(name, args)| (name.to_string(), args.to_string())),
        attachments_present,
    }
}

#[test]
fn bootstrap_adapter_translates_the_typed_update_request_exactly() {
    let executor =
        RecordingBootstrapExecutor::returning(Ok(valid_bootstrap_outcome("prompt_start", true)));
    let requests = Arc::clone(&executor.requests);
    let port = NativeTelegramUpdateBootstrapPort::with_executor(
        "repo-password-super-secret",
        true,
        executor,
    )
    .unwrap();

    assert!(port
        .handle_bootstrap(&bootstrap_request(
            Some("raw-message-super-secret"),
            Some(("start", "args-super-secret")),
            true,
        ))
        .unwrap());

    let recorded = requests.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    let request = &recorded[0];
    assert_eq!(request["kind"], "handle");
    assert_eq!(request["owner_bootstrap_enabled"], true);
    assert_eq!(request["expected_password"], "repo-password-super-secret");
    assert_eq!(request["config"]["repo_name"], "repo-password-super-secret");
    assert_eq!(request["chat_id"], 100);
    assert_eq!(request["raw_text"], "raw-message-super-secret");
    assert_eq!(request["command"], json!(["start", "args-super-secret"]));
    assert_eq!(request["command_name"], "start");
    assert_eq!(request["attachments_present"], true);
    let debug = format!("{port:?}");
    assert!(!debug.contains("repo-password-super-secret"));
    assert!(!debug.contains("raw-message-super-secret"));
}

#[test]
fn bootstrap_adapter_rejects_corrupt_outcomes_and_executor_failures() {
    let mut bad_decision = valid_bootstrap_outcome("prompt_start", true);
    bad_decision["decision"] = json!("private-super-secret-decision");
    let mut bad_flags = valid_bootstrap_outcome("prompt_start", true);
    bad_flags["python_owner_bootstrap_allowed"] = json!(true);
    let mut bad_counts = valid_bootstrap_outcome("prompt_start", true);
    bad_counts["side_effect_count"] = json!(1);

    for outcome in [
        Err("executor-super-secret".to_string()),
        Ok(json!({"private": "contract-super-secret"})),
        Ok(bad_decision),
        Ok(bad_flags),
        Ok(bad_counts),
    ] {
        let port = NativeTelegramUpdateBootstrapPort::with_executor(
            "repo-super-secret",
            true,
            RecordingBootstrapExecutor::returning(outcome),
        )
        .unwrap();
        let error = port
            .handle_bootstrap(&bootstrap_request(None, None, false))
            .unwrap_err();
        assert_eq!(error.to_string(), "Telegram update execution port failed.");
        assert!(!error.to_string().contains("secret"));
    }
}

#[derive(Clone, Default)]
struct MemoryState {
    inner: Arc<MemoryStateInner>,
}

struct MemoryStateInner {
    auth: Mutex<JsonValue>,
    binding: Mutex<Option<JsonValue>>,
    loads: AtomicUsize,
    saves: AtomicUsize,
    fail: AtomicBool,
}

impl Default for MemoryStateInner {
    fn default() -> Self {
        Self {
            auth: Mutex::new(json!({})),
            binding: Mutex::new(None),
            loads: AtomicUsize::new(0),
            saves: AtomicUsize::new(0),
            fail: AtomicBool::new(false),
        }
    }
}

impl MemoryState {
    fn with_binding(binding: JsonValue) -> Self {
        let state = Self::default();
        *state.inner.binding.lock().unwrap() = Some(binding);
        state
    }

    fn auth(&self) -> JsonValue {
        self.inner.auth.lock().unwrap().clone()
    }
}

impl TelegramOwnerBootstrapStatePort for MemoryState {
    fn load_bootstrap_auth(&self) -> Result<JsonValue, String> {
        self.inner.loads.fetch_add(1, Ordering::AcqRel);
        if self.inner.fail.load(Ordering::Acquire) {
            Err("state-super-secret".to_string())
        } else {
            Ok(self.auth())
        }
    }

    fn load_existing_binding(&self, _chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        self.inner.loads.fetch_add(1, Ordering::AcqRel);
        Ok(self.inner.binding.lock().unwrap().clone())
    }

    fn save_bootstrap_auth(&self, auth_state: &JsonValue) -> Result<(), String> {
        self.inner.saves.fetch_add(1, Ordering::AcqRel);
        *self.inner.auth.lock().unwrap() = auth_state.clone();
        Ok(())
    }
}

#[derive(Clone)]
struct FixedClock {
    fail: Arc<AtomicBool>,
}

impl Default for FixedClock {
    fn default() -> Self {
        Self {
            fail: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl TelegramOwnerBootstrapClockPort for FixedClock {
    fn now_iso(&self) -> Result<String, String> {
        if self.fail.load(Ordering::Acquire) {
            Err("clock-super-secret".to_string())
        } else {
            Ok("2026-07-18T03:00:00Z".to_string())
        }
    }
}

fn native_bootstrap(
    repo_name: &str,
    enabled: bool,
    state: MemoryState,
    clock: FixedClock,
    messages: RecordingMessageExecutor,
) -> NativeTelegramUpdateBootstrapPort<
    NativeTelegramUpdateOwnerBootstrapExecutor<
        DefaultTelegramOwnerBootstrapPlanner,
        MemoryState,
        FixedClock,
        NativeTelegramUpdateMessagePort<RecordingMessageExecutor>,
    >,
> {
    let message = Arc::new(
        NativeTelegramUpdateMessagePort::with_executor(
            "bot-token-super-secret",
            None,
            false,
            messages,
        )
        .unwrap(),
    );
    NativeTelegramUpdateBootstrapPort::with_executor(
        repo_name,
        enabled,
        NativeTelegramUpdateOwnerBootstrapExecutor::with_ports(
            DefaultTelegramOwnerBootstrapPlanner,
            state,
            clock,
            message,
        ),
    )
    .unwrap()
}

#[test]
fn native_bootstrap_executes_start_and_owner_password_state_machine() {
    let state = MemoryState::default();
    let messages = RecordingMessageExecutor::successful();
    let port = native_bootstrap(
        "repo-password-super-secret",
        true,
        state.clone(),
        FixedClock::default(),
        messages.clone(),
    );

    assert!(port
        .handle_bootstrap(&bootstrap_request(None, Some(("start", "")), false))
        .unwrap());
    assert_eq!(state.auth()["pending_user_id"], "7");
    assert!(port
        .handle_bootstrap(&bootstrap_request(
            Some("repo-password-super-secret"),
            None,
            false,
        ))
        .unwrap());
    assert_eq!(state.auth()["owner_user_id"], "7");
    assert_eq!(state.inner.saves.load(Ordering::Acquire), 2);
    assert_eq!(messages.requests().len(), 2);
    assert!(messages.requests()[0]["text"]
        .as_str()
        .unwrap()
        .contains("repository-name password"));
    assert!(messages.requests()[1]["text"]
        .as_str()
        .unwrap()
        .contains("Owner verified"));
}

#[test]
fn native_bootstrap_covers_disabled_attachment_gate_and_existing_binding_adoption() {
    let disabled_state = MemoryState::default();
    let disabled_messages = RecordingMessageExecutor::successful();
    let disabled = native_bootstrap(
        "repo",
        false,
        disabled_state.clone(),
        FixedClock::default(),
        disabled_messages.clone(),
    );
    assert!(!disabled
        .handle_bootstrap(&bootstrap_request(None, None, true))
        .unwrap());
    assert_eq!(disabled_state.inner.loads.load(Ordering::Acquire), 0);
    assert!(disabled_messages.requests().is_empty());

    let gated_state = MemoryState::default();
    let gated = native_bootstrap(
        "repo",
        true,
        gated_state,
        FixedClock::default(),
        RecordingMessageExecutor::successful(),
    );
    assert!(gated
        .handle_bootstrap(&bootstrap_request(None, None, true))
        .unwrap());

    let adopted_state = MemoryState::with_binding(json!({
        "conversation_key": "telegram:7",
        "chat_type": "private",
        "binding_role": "primary_shared",
    }));
    let adopted_messages = RecordingMessageExecutor::successful();
    let adopted = native_bootstrap(
        "repo",
        true,
        adopted_state.clone(),
        FixedClock::default(),
        adopted_messages.clone(),
    );
    assert!(!adopted
        .handle_bootstrap(&bootstrap_request(Some("hello"), None, false))
        .unwrap());
    assert_eq!(adopted_state.auth()["owner_user_id"], "7");
    assert_eq!(
        adopted_state.auth()["owner_claim_reason"],
        "existing_private_conversation_binding"
    );
    assert!(adopted_messages.requests().is_empty());
}

struct FailingPlanner;

impl TelegramOwnerBootstrapPlanner for FailingPlanner {
    fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Err("planner-super-secret".to_string())
    }
}

#[test]
fn native_bootstrap_maps_state_clock_planner_and_message_failures_to_one_safe_port_error() {
    let failing_state = MemoryState::default();
    failing_state.inner.fail.store(true, Ordering::Release);
    let state_port = native_bootstrap(
        "repo",
        true,
        failing_state,
        FixedClock::default(),
        RecordingMessageExecutor::successful(),
    );

    let failing_clock = FixedClock::default();
    failing_clock.fail.store(true, Ordering::Release);
    let clock_port = native_bootstrap(
        "repo",
        true,
        MemoryState::default(),
        failing_clock,
        RecordingMessageExecutor::successful(),
    );

    let message_state = MemoryState::default();
    let message_port = native_bootstrap(
        "repo",
        true,
        message_state,
        FixedClock::default(),
        RecordingMessageExecutor::returning(Err("message-super-secret".to_string())),
    );

    let planner_executor = NativeTelegramUpdateOwnerBootstrapExecutor::with_ports(
        FailingPlanner,
        MemoryState::default(),
        FixedClock::default(),
        Arc::new(
            NativeTelegramUpdateMessagePort::with_executor(
                "token",
                None,
                false,
                RecordingMessageExecutor::successful(),
            )
            .unwrap(),
        ),
    );
    let planner_port =
        NativeTelegramUpdateBootstrapPort::with_executor("repo", true, planner_executor).unwrap();

    for (port_name, result) in [
        (
            "state",
            state_port.handle_bootstrap(&bootstrap_request(None, None, false)),
        ),
        (
            "clock",
            clock_port.handle_bootstrap(&bootstrap_request(None, Some(("start", "")), false)),
        ),
        (
            "message",
            message_port.handle_bootstrap(&bootstrap_request(None, Some(("start", "")), false)),
        ),
    ] {
        let error = result.expect_err(port_name);
        assert_eq!(error.to_string(), "Telegram update execution port failed.");
        assert!(!error.to_string().contains("secret"));
    }
    let planner_error = planner_port
        .handle_bootstrap(&bootstrap_request(None, None, false))
        .unwrap_err();
    assert_eq!(
        planner_error.to_string(),
        "Telegram update execution port failed."
    );
}

#[test]
fn production_constructors_validate_paths_and_keep_configuration_out_of_debug() {
    let port = NativeTelegramUpdateBootstrapPort::new(
        "repo-password-super-secret",
        "/tmp/runtime-state-super-secret.json",
        true,
        "bot-token-super-secret",
        Some(10.0),
        true,
    )
    .unwrap();
    let debug = format!("{port:?}");
    assert!(!debug.contains("repo-password-super-secret"));
    assert!(!debug.contains("runtime-state-super-secret"));
    assert!(!debug.contains("bot-token-super-secret"));

    for (repo, path) in [
        ("", "/tmp/state.json"),
        (" repo ", "/tmp/state.json"),
        ("repo-super-secret", "/tmp/state\nsuper-secret.json"),
    ] {
        let error = NativeTelegramUpdateBootstrapPort::new(repo, path, true, "token", None, false)
            .unwrap_err();
        assert!(error.contains("configuration is invalid"));
        assert!(!error.contains("super-secret"));
    }
}

#[test]
fn system_update_diagnostic_is_bounded_versioned_and_secret_free() {
    for kind in [
        TelegramUpdateJobErrorKind::InvalidUpdate,
        TelegramUpdateJobErrorKind::InputContract,
        TelegramUpdateJobErrorKind::Lifecycle,
    ] {
        let rendered = update_failure_diagnostic(kind);
        assert!(rendered.len() < 512);
        assert!(rendered.contains("ait.agent.telegram_update.diagnostic.v1"));
        assert!(rendered.contains(&format!("\"code\":\"{}\"", kind.code())));
        assert!(rendered.contains("\"python_fallback_allowed\":false"));
        assert!(rendered.contains("\"private_context_exposed\":false"));
        assert!(!rendered.contains("token"));
        assert!(!rendered.contains("chat_id"));
    }
}
