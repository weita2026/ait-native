use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tempfile::tempdir;

use super::*;
#[cfg(unix)]
use crate::event_loop::DefaultTelegramCommandTriggerOperationExecutor;
use crate::event_loop::DefaultTelegramEventTriggerPlanner;

type Calls = Arc<Mutex<Vec<String>>>;

struct RecordingCallbackPlanner {
    calls: Calls,
    mutate: Option<fn(&mut JsonValue)>,
    fail_stage: Option<&'static str>,
}

impl RecordingCallbackPlanner {
    fn new(calls: Calls) -> Self {
        Self {
            calls,
            mutate: None,
            fail_stage: None,
        }
    }
}

impl TelegramOperationalTriggerCallbackPlanner for RecordingCallbackPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let stage = request["stage"].as_str().unwrap_or("missing");
        self.calls.lock().unwrap().push(format!("callback:{stage}"));
        if self.fail_stage == Some(stage) {
            return Err("private-callback-planner-secret".to_string());
        }
        let mut planned = DefaultTelegramOperationalTriggerCallbackPlanner.plan_json(request)?;
        if let Some(mutate) = self.mutate {
            mutate(&mut planned);
        }
        Ok(planned)
    }
}

struct RecordingState {
    calls: Calls,
    binding: Mutex<Option<JsonValue>>,
    fail: AtomicBool,
}

impl RecordingState {
    fn new(calls: Calls) -> Self {
        Self {
            calls,
            binding: Mutex::new(Some(json!({
                "conversation_key": "telegram:123",
            }))),
            fail: AtomicBool::new(false),
        }
    }
}

impl TelegramOperationalTriggerStatePort for RecordingState {
    fn load_binding(&self, _chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        self.calls.lock().unwrap().push("state".to_string());
        if self.fail.load(Ordering::Acquire) {
            Err("private-state-secret".to_string())
        } else {
            Ok(self.binding.lock().unwrap().clone())
        }
    }
}

enum OperationBehavior {
    Success(String),
    Failure,
    Error,
    Malformed,
}

struct RecordingOperation {
    calls: Calls,
    requests: Mutex<Vec<JsonValue>>,
    behavior: Mutex<OperationBehavior>,
}

impl RecordingOperation {
    fn success(calls: Calls, stdout: impl Into<String>) -> Self {
        Self {
            calls,
            requests: Mutex::new(Vec::new()),
            behavior: Mutex::new(OperationBehavior::Success(stdout.into())),
        }
    }
}

impl TelegramCommandTriggerOperationExecutor for RecordingOperation {
    fn execute_operation_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        self.calls.lock().unwrap().push("operation".to_string());
        self.requests.lock().unwrap().push(request.clone());
        match &*self.behavior.lock().unwrap() {
            OperationBehavior::Success(stdout) => Ok(json!({
                "kind": "run_handler",
                "method": "std::process::Command",
                "ok": true,
                "returncode": 0,
                "stdout": stdout,
                "stderr": "",
                "error": JsonValue::Null,
            })),
            OperationBehavior::Failure => Ok(json!({
                "kind": "run_handler",
                "method": "std::process::Command",
                "ok": false,
                "returncode": 7,
                "stdout": "",
                "stderr": "private-handler-secret",
                "error": "private-handler-secret",
            })),
            OperationBehavior::Error => Err("private-operation-port-secret".to_string()),
            OperationBehavior::Malformed => Ok(json!({
                "kind": "unknown",
                "private": "private-malformed-operation-secret",
            })),
        }
    }
}

struct RecordingDiagnostics {
    calls: Calls,
    failures: Mutex<Vec<TelegramOperationalTriggerExecutionErrorKind>>,
    fail: AtomicBool,
}

impl RecordingDiagnostics {
    fn new(calls: Calls) -> Self {
        Self {
            calls,
            failures: Mutex::new(Vec::new()),
            fail: AtomicBool::new(false),
        }
    }
}

impl TelegramOperationalTriggerDiagnosticsPort for RecordingDiagnostics {
    fn record_failure(
        &self,
        kind: TelegramOperationalTriggerExecutionErrorKind,
    ) -> Result<(), String> {
        self.calls.lock().unwrap().push("diagnostic".to_string());
        self.failures.lock().unwrap().push(kind);
        if self.fail.load(Ordering::Acquire) {
            Err("private-diagnostic-secret".to_string())
        } else {
            Ok(())
        }
    }
}

struct RecordingDelivery {
    calls: Calls,
    assistant_events: Mutex<Vec<JsonValue>>,
    failure_messages: Mutex<Vec<String>>,
    fail_assistant: AtomicBool,
    fail_failure: AtomicBool,
}

impl RecordingDelivery {
    fn new(calls: Calls) -> Self {
        Self {
            calls,
            assistant_events: Mutex::new(Vec::new()),
            failure_messages: Mutex::new(Vec::new()),
            fail_assistant: AtomicBool::new(false),
            fail_failure: AtomicBool::new(false),
        }
    }
}

impl TelegramOperationalTriggerDeliveryPort for RecordingDelivery {
    fn send_assistant_event_reply(
        &self,
        _chat_id: &JsonValue,
        assistant_event: &JsonValue,
    ) -> Result<(), String> {
        self.calls.lock().unwrap().push("assistant".to_string());
        self.assistant_events
            .lock()
            .unwrap()
            .push(assistant_event.clone());
        if self.fail_assistant.load(Ordering::Acquire) {
            Err("private-assistant-delivery-secret".to_string())
        } else {
            Ok(())
        }
    }

    fn send_failure_message(&self, _chat_id: &JsonValue, text: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push("failure".to_string());
        self.failure_messages.lock().unwrap().push(text.to_string());
        if self.fail_failure.load(Ordering::Acquire) {
            Err("private-failure-delivery-secret".to_string())
        } else {
            Ok(())
        }
    }
}

fn normalized_trigger(handler_command: JsonValue) -> JsonValue {
    json!({
        "trigger_id": "router",
        "display_trigger": "router",
        "handler_command": handler_command,
        "source_path": "docs/event_trigger/private-trigger-path.md",
        "match": {
            "phrases": [],
            "commands": ["router"],
            "pattern": JsonValue::Null,
            "allow_trailing_punctuation": true,
            "reply_only": false,
            "case_sensitive": false,
        },
        "priority": 0,
    })
}

fn config(
    repo_root: &Path,
    handler_command: JsonValue,
) -> TelegramOperationalTriggerExecutionConfig {
    TelegramOperationalTriggerExecutionConfig::new(
        "private-repo-secret",
        repo_root,
        json!({"telegram_operational": [normalized_trigger(handler_command)]}),
    )
}

fn request() -> JsonValue {
    json!({
        "chat_id": 123,
        "chat": {"id": 123, "type": "private", "title": "private-chat-secret"},
        "from_user": {"id": 456, "username": "private-user-secret"},
        "chat_title": "private-chat-secret",
        "context": {
            "raw_text": "/router now private-message-secret",
            "normalized_text": "/router now private-message-secret",
            "command": ["router", "now private-message-secret"],
            "telegram_message_id": 99,
            "telegram_message_ids": [99],
            "reply_to_message": {"message_id": 88},
            "attachments": [],
            "actor_identity": "private-actor-secret",
            "message": {"private": "private-message-object-secret"},
        },
    })
}

fn execute(
    callback: &RecordingCallbackPlanner,
    state: &RecordingState,
    operation: &RecordingOperation,
    diagnostics: &RecordingDiagnostics,
    delivery: &RecordingDelivery,
    config: &TelegramOperationalTriggerExecutionConfig,
    request: &JsonValue,
) -> Result<JsonValue, TelegramOperationalTriggerExecutionError> {
    let ports = TelegramOperationalTriggerPorts::new(
        &DefaultTelegramEventTriggerPlanner,
        callback,
        state,
        operation,
        diagnostics,
        delivery,
    );
    execute_with_telegram_operational_trigger_ports(&ports, config, request)
}

#[test]
fn unmatched_input_has_no_state_callback_operation_or_delivery_effects() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let operation = RecordingOperation::success(Arc::clone(&calls), "{}");
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));
    let mut input = request();
    input["context"]["command"] = json!(["other", ""]);
    input["context"]["raw_text"] = json!("ordinary message");
    input["context"]["normalized_text"] = json!("ordinary message");

    let outcome = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &config(temp.path(), json!(["native-handler"])),
        &input,
    )
    .unwrap();

    assert_eq!(outcome["matched"], false);
    assert_eq!(outcome["handled"], false);
    assert_eq!(outcome["operation_count"], 0);
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn handled_reply_executes_ordered_transaction_and_returns_secret_safe_facts() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let operation = RecordingOperation::success(
        Arc::clone(&calls),
        r#"{"reply":{"text":"private-reply-secret"}}"#,
    );
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));

    let outcome = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &config(temp.path(), json!(["native-handler"])),
        &request(),
    )
    .unwrap();

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["matched"], true);
    assert_eq!(outcome["handled"], true);
    assert_eq!(outcome["operation_count"], 1);
    assert_eq!(outcome["completed_operation_count"], 1);
    assert_eq!(outcome["result_callback_planned"], true);
    assert_eq!(outcome["assistant_event_sent"], true);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "state",
            "callback:request",
            "operation",
            "callback:result",
            "assistant"
        ]
    );
    let operation_request = operation.requests.lock().unwrap()[0].clone();
    assert_eq!(operation_request["kind"], "run_handler");
    assert_eq!(
        operation_request["stdin_json"]["binding"]["conversation_key"],
        "telegram:123"
    );
    let rendered = outcome.to_string();
    for secret in [
        "private-repo-secret",
        "private-trigger-path",
        "telegram:123",
        "private-message-secret",
        "private-reply-secret",
        "123",
    ] {
        assert!(!rendered.contains(secret));
    }
}

#[test]
fn attachment_reply_and_unhandled_success_preserve_delivery_decision() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let operation = RecordingOperation::success(
        Arc::clone(&calls),
        r#"{"reply":{"attachments":[{"kind":"document","path":"private-file-secret"}]}}"#,
    );
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));
    let cfg = config(temp.path(), json!(["native-handler"]));

    let attached = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &cfg,
        &request(),
    )
    .unwrap();
    assert_eq!(attached["handled"], true);
    assert_eq!(attached["assistant_event_sent"], true);
    assert_eq!(delivery.assistant_events.lock().unwrap().len(), 1);

    *operation.behavior.lock().unwrap() = OperationBehavior::Success("{}".to_string());
    calls.lock().unwrap().clear();
    let unhandled = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &cfg,
        &request(),
    )
    .unwrap();
    assert_eq!(unhandled["ok"], true);
    assert_eq!(unhandled["matched"], true);
    assert_eq!(unhandled["handled"], false);
    assert_eq!(unhandled["assistant_event_sent"], false);
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["state", "callback:request", "operation", "callback:result"]
    );
}

#[test]
fn nonzero_handler_result_is_consumed_with_stable_failure_delivery() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let operation = RecordingOperation::success(Arc::clone(&calls), "{}");
    *operation.behavior.lock().unwrap() = OperationBehavior::Failure;
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));

    let outcome = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &config(temp.path(), json!(["native-handler"])),
        &request(),
    )
    .unwrap();

    assert_eq!(outcome["ok"], false);
    assert_eq!(outcome["handled"], true);
    assert_eq!(outcome["failure_kind"], "operation");
    assert_eq!(outcome["completed_operation_count"], 1);
    assert_eq!(outcome["failure_message_sent"], true);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "state",
            "callback:request",
            "operation",
            "callback:result",
            "diagnostic",
            "failure"
        ]
    );
    assert_eq!(
        *diagnostics.failures.lock().unwrap(),
        vec![TelegramOperationalTriggerExecutionErrorKind::Operation]
    );
    assert_eq!(
        *delivery.failure_messages.lock().unwrap(),
        vec![FAILURE_MESSAGE.to_string()]
    );
    assert!(!outcome.to_string().contains("private-handler-secret"));
}

#[test]
fn operation_port_error_plans_partial_result_before_reporting_failure() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let operation = RecordingOperation::success(Arc::clone(&calls), "{}");
    *operation.behavior.lock().unwrap() = OperationBehavior::Error;
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));

    let outcome = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &config(temp.path(), json!(["native-handler"])),
        &request(),
    )
    .unwrap();

    assert_eq!(outcome["operation_count"], 1);
    assert_eq!(outcome["completed_operation_count"], 0);
    assert_eq!(outcome["result_callback_planned"], true);
    assert_eq!(outcome["failure_kind"], "operation");
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "state",
            "callback:request",
            "operation",
            "callback:result",
            "diagnostic",
            "failure"
        ]
    );
    assert!(!outcome
        .to_string()
        .contains("private-operation-port-secret"));
}

#[test]
fn state_and_callback_failures_never_reach_later_side_effects() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    state.fail.store(true, Ordering::Release);
    let operation = RecordingOperation::success(Arc::clone(&calls), "{}");
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));
    let cfg = config(temp.path(), json!(["native-handler"]));

    let state_failure = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &cfg,
        &request(),
    )
    .unwrap();
    assert_eq!(state_failure["failure_kind"], "state");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["state", "diagnostic", "failure"]
    );

    state.fail.store(false, Ordering::Release);
    calls.lock().unwrap().clear();
    let mut failing_callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    failing_callback.fail_stage = Some("request");
    let callback_failure = execute(
        &failing_callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &cfg,
        &request(),
    )
    .unwrap();
    assert_eq!(callback_failure["failure_kind"], "callback_planner");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["state", "callback:request", "diagnostic", "failure"]
    );
    assert!(!callback_failure
        .to_string()
        .contains("private-callback-planner-secret"));
}

fn disable_callback_execution(planned: &mut JsonValue) {
    if planned["stage"] == "request" {
        planned["should_execute"] = json!(false);
        planned["request"]["ok"] = json!(false);
        planned["request"]["error"] = json!("private-empty-handler-secret");
        planned["request"]["operation"] = JsonValue::Null;
        planned["request"]["operations"] = json!([]);
        planned["request"]["operation_count"] = json!(0);
    }
}

fn forge_unknown_operation(planned: &mut JsonValue) {
    if planned["stage"] == "request" {
        planned["request"]["operations"][0]["kind"] = json!("unknown_action");
    }
}

fn forge_result_event(planned: &mut JsonValue) {
    if planned["stage"] == "result" {
        planned["result"]["assistant_event"] = json!({
            "event_type": "private-forged-event-secret",
            "payload": {},
        });
        planned["result"]["should_send_assistant_event"] = json!(true);
    }
}

fn forge_request_count(planned: &mut JsonValue) {
    if planned["stage"] == "request" {
        planned["request"]["operation_count"] = json!(2);
    }
}

fn forge_result_count(planned: &mut JsonValue) {
    if planned["stage"] == "result" {
        planned["result"]["command_result"]["operation_count"] = json!(2);
    }
}

#[test]
fn disabled_empty_handler_unknown_action_and_forged_event_fail_closed() {
    let temp = tempdir().unwrap();
    for (mutate, expected_calls) in [
        (
            disable_callback_execution as fn(&mut JsonValue),
            vec!["state", "callback:request", "diagnostic", "failure"],
        ),
        (
            forge_unknown_operation as fn(&mut JsonValue),
            vec!["state", "callback:request", "diagnostic", "failure"],
        ),
        (
            forge_result_event as fn(&mut JsonValue),
            vec![
                "state",
                "callback:request",
                "operation",
                "callback:result",
                "diagnostic",
                "failure",
            ],
        ),
        (
            forge_request_count as fn(&mut JsonValue),
            vec!["state", "callback:request", "diagnostic", "failure"],
        ),
        (
            forge_result_count as fn(&mut JsonValue),
            vec![
                "state",
                "callback:request",
                "operation",
                "callback:result",
                "diagnostic",
                "failure",
            ],
        ),
    ] {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
        callback.mutate = Some(mutate);
        let state = RecordingState::new(Arc::clone(&calls));
        let operation =
            RecordingOperation::success(Arc::clone(&calls), r#"{"reply":{"text":"done"}}"#);
        let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
        let delivery = RecordingDelivery::new(Arc::clone(&calls));
        let outcome = execute(
            &callback,
            &state,
            &operation,
            &diagnostics,
            &delivery,
            &config(temp.path(), json!(["native-handler"])),
            &request(),
        )
        .unwrap();

        assert_eq!(outcome["ok"], false);
        assert_eq!(outcome["failure_kind"], "callback_planner_contract");
        assert_eq!(*calls.lock().unwrap(), expected_calls);
        assert!(!outcome.to_string().contains("private-"));
    }
}

#[test]
fn malformed_operation_result_is_never_forwarded_to_result_planner() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let operation = RecordingOperation::success(Arc::clone(&calls), "{}");
    *operation.behavior.lock().unwrap() = OperationBehavior::Malformed;
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));

    let outcome = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &config(temp.path(), json!(["native-handler"])),
        &request(),
    )
    .unwrap();
    assert_eq!(outcome["failure_kind"], "operation");
    assert_eq!(outcome["result_callback_planned"], false);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "state",
            "callback:request",
            "operation",
            "diagnostic",
            "failure"
        ]
    );
    assert!(!outcome
        .to_string()
        .contains("private-malformed-operation-secret"));
}

#[test]
fn malformed_state_binding_and_registry_fail_at_their_trust_boundaries() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    *state.binding.lock().unwrap() = Some(json!("private-invalid-binding-secret"));
    let operation = RecordingOperation::success(Arc::clone(&calls), "{}");
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));

    let outcome = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &config(temp.path(), json!(["native-handler"])),
        &request(),
    )
    .unwrap();
    assert_eq!(outcome["failure_kind"], "state");
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["state", "diagnostic", "failure"]
    );
    assert!(!outcome
        .to_string()
        .contains("private-invalid-binding-secret"));

    calls.lock().unwrap().clear();
    let invalid_config = TelegramOperationalTriggerExecutionConfig::new(
        "repo",
        temp.path(),
        json!({"telegram_operational": ["private-invalid-trigger-secret"]}),
    );
    let error = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &invalid_config,
        &request(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOperationalTriggerExecutionErrorKind::InvalidRequest
    );
    assert!(calls.lock().unwrap().is_empty());
    assert!(!error.to_string().contains("private-invalid-trigger-secret"));
}

struct FixedEventPlanner {
    result: Result<JsonValue, String>,
}

impl TelegramEventTriggerPlanner for FixedEventPlanner {
    fn plan_json(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        self.result.clone()
    }
}

#[test]
fn forged_request_and_event_contract_drift_fail_before_state_access() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let operation = RecordingOperation::success(Arc::clone(&calls), "{}");
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));
    let cfg = config(temp.path(), json!(["native-handler"]));
    let mut forged = request();
    forged["trigger"] = json!({"private": "private-forged-trigger-secret"});

    let error = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &cfg,
        &forged,
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOperationalTriggerExecutionErrorKind::InvalidRequest
    );
    assert!(calls.lock().unwrap().is_empty());
    assert!(!error.to_string().contains("private-forged-trigger-secret"));

    let event = FixedEventPlanner {
        result: Ok(json!({
            "migration_stage": EVENT_TRIGGER_STAGE,
            "event_trigger_contract": EVENT_TRIGGER_CONTRACT,
            "stage": "operational_dispatch",
            "transport": "telegram",
            "rust_event_loop_required": true,
            "python_event_trigger_allowed": true,
            "matched": true,
            "handled": true,
            "trigger": normalized_trigger(json!(["native-handler"])),
            "match_payload": {},
        })),
    };
    let ports = TelegramOperationalTriggerPorts::new(
        &event,
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
    );
    let error =
        execute_with_telegram_operational_trigger_ports(&ports, &cfg, &request()).unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract
    );
    assert!(calls.lock().unwrap().is_empty());

    let event = FixedEventPlanner {
        result: Err("private-event-planner-secret".to_string()),
    };
    let ports = TelegramOperationalTriggerPorts::new(
        &event,
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
    );
    let error =
        execute_with_telegram_operational_trigger_ports(&ports, &cfg, &request()).unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOperationalTriggerExecutionErrorKind::EventPlanner
    );
    assert!(!error.to_string().contains("private-event-planner-secret"));
}

#[test]
fn diagnostics_and_failure_delivery_errors_remain_secret_safe() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    state.fail.store(true, Ordering::Release);
    let operation = RecordingOperation::success(Arc::clone(&calls), "{}");
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));
    let cfg = config(temp.path(), json!(["native-handler"]));

    diagnostics.fail.store(true, Ordering::Release);
    let error = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &cfg,
        &request(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOperationalTriggerExecutionErrorKind::Diagnostics
    );
    assert_eq!(*calls.lock().unwrap(), vec!["state", "diagnostic"]);
    assert!(!error.to_string().contains("private-diagnostic-secret"));

    diagnostics.fail.store(false, Ordering::Release);
    delivery.fail_failure.store(true, Ordering::Release);
    calls.lock().unwrap().clear();
    let error = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &cfg,
        &request(),
    )
    .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramOperationalTriggerExecutionErrorKind::Delivery
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["state", "diagnostic", "failure"]
    );
    assert!(!error
        .to_string()
        .contains("private-failure-delivery-secret"));
}

#[test]
fn assistant_delivery_failure_is_reported_once_then_consumed_by_failure_message() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let operation = RecordingOperation::success(Arc::clone(&calls), r#"{"reply":{"text":"done"}}"#);
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));
    delivery.fail_assistant.store(true, Ordering::Release);

    let outcome = execute(
        &callback,
        &state,
        &operation,
        &diagnostics,
        &delivery,
        &config(temp.path(), json!(["native-handler"])),
        &request(),
    )
    .unwrap();
    assert_eq!(outcome["ok"], false);
    assert_eq!(outcome["failure_kind"], "delivery");
    assert_eq!(outcome["assistant_event_sent"], false);
    assert_eq!(delivery.assistant_events.lock().unwrap().len(), 1);
    assert_eq!(delivery.failure_messages.lock().unwrap().len(), 1);
    assert_eq!(
        *diagnostics.failures.lock().unwrap(),
        vec![TelegramOperationalTriggerExecutionErrorKind::Delivery]
    );
}

#[test]
fn locked_binding_state_adapter_reads_current_telegram_binding() {
    let temp = tempdir().unwrap();
    let store = AgentRuntimeBindingStore::new(temp.path().join("bindings.json"));
    store
        .execute(
            "upsert_binding",
            &json!({
                "transport": "telegram",
                "surface_id": 123,
                "repo_name": "ait",
                "surface_title": "private-title-secret",
                "updates": {
                    "conversation_key": "telegram:123",
                    "unrelated": {"preserved": true},
                },
            }),
        )
        .unwrap();
    let state = RuntimeBindingTelegramOperationalTriggerStatePort::from_store(store.clone());

    let loaded = state.load_binding(&json!(123)).unwrap().unwrap();
    assert_eq!(loaded["conversation_key"], "telegram:123");
    assert_eq!(loaded["surface_title"], "private-title-secret");
    assert_eq!(state.path(), store.path());
    assert!(state.load_binding(&json!(999)).unwrap().is_none());
}

#[test]
#[cfg(unix)]
fn production_command_operation_adapter_executes_native_handler_end_to_end() {
    let temp = tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let callback = RecordingCallbackPlanner::new(Arc::clone(&calls));
    let state = RecordingState::new(Arc::clone(&calls));
    let diagnostics = RecordingDiagnostics::new(Arc::clone(&calls));
    let delivery = RecordingDelivery::new(Arc::clone(&calls));
    let command = json!([
        "/bin/sh",
        "-c",
        "printf '{\"reply\":{\"text\":\"native reply\"}}'"
    ]);

    let ports = TelegramOperationalTriggerPorts::new(
        &DefaultTelegramEventTriggerPlanner,
        &callback,
        &state,
        &DefaultTelegramCommandTriggerOperationExecutor,
        &diagnostics,
        &delivery,
    );
    let outcome = execute_with_telegram_operational_trigger_ports(
        &ports,
        &config(temp.path(), command),
        &request(),
    )
    .unwrap();

    assert_eq!(outcome["ok"], true);
    assert_eq!(outcome["handled"], true);
    assert_eq!(outcome["assistant_event_sent"], true);
    assert_eq!(delivery.assistant_events.lock().unwrap().len(), 1);
}
