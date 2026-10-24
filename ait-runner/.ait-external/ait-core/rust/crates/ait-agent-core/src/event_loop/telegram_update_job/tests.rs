use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use super::*;

const PLANNER_SECRET: &str = "planner-secret-should-never-escape";

struct Harness {
    events: Mutex<Vec<String>>,
    input_results: Mutex<VecDeque<Result<TelegramPreparedUpdateInput, TelegramUpdateInputError>>>,
    bootstrap_results: Mutex<VecDeque<Result<bool, TelegramUpdatePortError>>>,
    operational_results: Mutex<VecDeque<Result<bool, TelegramUpdatePortError>>>,
    input_requests: Mutex<Vec<TelegramUpdateInputRequest>>,
    bootstrap_requests: Mutex<Vec<TelegramUpdateBootstrapRequest>>,
    operational_requests: Mutex<Vec<TelegramUpdateOperationalRequest>>,
    command_requests: Mutex<Vec<TelegramUpdateCommandRequest>>,
    normal_turn_requests: Mutex<Vec<TelegramUpdateNormalTurnRequest>>,
    recorded_failures: Mutex<Vec<TelegramUpdateJobErrorKind>>,
    command_fails: AtomicBool,
    delivery_fails: AtomicBool,
    normal_turn_fails: AtomicBool,
    background_fails: AtomicBool,
    reply_fails: AtomicBool,
    live_reply_fails: AtomicBool,
    diagnostics_fails: AtomicBool,
    live_reply_idle: AtomicBool,
}

impl Default for Harness {
    fn default() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
            input_results: Mutex::new(VecDeque::new()),
            bootstrap_results: Mutex::new(VecDeque::new()),
            operational_results: Mutex::new(VecDeque::new()),
            input_requests: Mutex::new(Vec::new()),
            bootstrap_requests: Mutex::new(Vec::new()),
            operational_requests: Mutex::new(Vec::new()),
            command_requests: Mutex::new(Vec::new()),
            normal_turn_requests: Mutex::new(Vec::new()),
            recorded_failures: Mutex::new(Vec::new()),
            command_fails: AtomicBool::new(false),
            delivery_fails: AtomicBool::new(false),
            normal_turn_fails: AtomicBool::new(false),
            background_fails: AtomicBool::new(false),
            reply_fails: AtomicBool::new(false),
            live_reply_fails: AtomicBool::new(false),
            diagnostics_fails: AtomicBool::new(false),
            live_reply_idle: AtomicBool::new(true),
        }
    }
}

impl Harness {
    fn push_event(&self, event: impl Into<String>) {
        lock(&self.events).push(event.into());
    }

    fn events(&self) -> Vec<String> {
        lock(&self.events).clone()
    }

    fn push_bootstrap(&self, handled: bool) {
        lock(&self.bootstrap_results).push_back(Ok(handled));
    }

    fn push_operational(&self, handled: bool) {
        lock(&self.operational_results).push_back(Ok(handled));
    }

    fn push_input(&self, result: Result<TelegramPreparedUpdateInput, TelegramUpdateInputError>) {
        lock(&self.input_results).push_back(result);
    }
}

impl TelegramUpdateInputPort for Harness {
    fn prepare_input(
        &self,
        request: &TelegramUpdateInputRequest,
    ) -> Result<TelegramPreparedUpdateInput, TelegramUpdateInputError> {
        self.push_event("input");
        lock(&self.input_requests).push(request.clone());
        lock(&self.input_results).pop_front().unwrap_or_else(|| {
            Ok(TelegramPreparedUpdateInput::new(
                request.candidate_raw_text().map(str::to_string),
                request.attachments().to_vec(),
            ))
        })
    }
}

impl TelegramUpdateBootstrapPort for Harness {
    fn handle_bootstrap(
        &self,
        request: &TelegramUpdateBootstrapRequest,
    ) -> Result<bool, TelegramUpdatePortError> {
        self.push_event("bootstrap");
        lock(&self.bootstrap_requests).push(request.clone());
        lock(&self.bootstrap_results)
            .pop_front()
            .unwrap_or(Ok(false))
    }
}

impl TelegramUpdateOperationalPort for Harness {
    fn handle_operational_trigger(
        &self,
        request: &TelegramUpdateOperationalRequest,
    ) -> Result<bool, TelegramUpdatePortError> {
        self.push_event("operational");
        lock(&self.operational_requests).push(request.clone());
        lock(&self.operational_results)
            .pop_front()
            .unwrap_or(Ok(false))
    }
}

impl TelegramUpdateCommandPort for Harness {
    fn execute_command(
        &self,
        request: &TelegramUpdateCommandRequest,
    ) -> Result<(), TelegramUpdatePortError> {
        self.push_event("command");
        lock(&self.command_requests).push(request.clone());
        if self.command_fails.load(Ordering::SeqCst) {
            Err(TelegramUpdatePortError)
        } else {
            Ok(())
        }
    }
}

impl TelegramUpdateDeliveryPort for Harness {
    fn send_message(
        &self,
        _chat_id: &JsonValue,
        text: &str,
    ) -> Result<(), TelegramUpdatePortError> {
        self.push_event(format!("delivery:{text}"));
        if self.delivery_fails.load(Ordering::SeqCst) {
            Err(TelegramUpdatePortError)
        } else {
            Ok(())
        }
    }
}

impl TelegramUpdateLifecyclePort for Harness {
    fn handle_normal_turn(
        &self,
        request: &TelegramUpdateNormalTurnRequest,
    ) -> Result<(), TelegramUpdatePortError> {
        self.push_event("normal_turn");
        lock(&self.normal_turn_requests).push(request.clone());
        if self.normal_turn_fails.load(Ordering::SeqCst) {
            Err(TelegramUpdatePortError)
        } else {
            Ok(())
        }
    }

    fn run_background_sync_for_chat(&self, _chat_id: &str) -> Result<(), TelegramUpdatePortError> {
        self.push_event("background_sync");
        if self.background_fails.load(Ordering::SeqCst) {
            Err(TelegramUpdatePortError)
        } else {
            Ok(())
        }
    }

    fn execute_reply(
        &self,
        _callback_slot: &str,
        _args: &[JsonValue],
    ) -> Result<(), TelegramUpdatePortError> {
        self.push_event("reply");
        if self.reply_fails.load(Ordering::SeqCst) {
            Err(TelegramUpdatePortError)
        } else {
            Ok(())
        }
    }

    fn wait_for_live_replies(
        &self,
        _timeout: Option<Duration>,
    ) -> Result<bool, TelegramUpdatePortError> {
        self.push_event("live_reply_wait");
        if self.live_reply_fails.load(Ordering::SeqCst) {
            Err(TelegramUpdatePortError)
        } else {
            Ok(self.live_reply_idle.load(Ordering::SeqCst))
        }
    }
}

impl TelegramUpdateDiagnosticsPort for Harness {
    fn record_failure(
        &self,
        kind: TelegramUpdateJobErrorKind,
    ) -> Result<(), TelegramUpdatePortError> {
        self.push_event(format!("diagnostic:{}", kind.code()));
        lock(&self.recorded_failures).push(kind);
        if self.diagnostics_fails.load(Ordering::SeqCst) {
            Err(TelegramUpdatePortError)
        } else {
            Ok(())
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn job(harness: &Arc<Harness>) -> TelegramUpdateJob {
    job_with_config(
        harness,
        TelegramUpdateJobConfig::new("ait_bot", false, false, true),
    )
}

fn job_with_config(harness: &Arc<Harness>, config: TelegramUpdateJobConfig) -> TelegramUpdateJob {
    TelegramUpdateJob::new(config, ports(harness)).expect("job")
}

fn ports(harness: &Arc<Harness>) -> TelegramUpdateJobPorts {
    TelegramUpdateJobPorts::new(
        harness.clone(),
        harness.clone(),
        harness.clone(),
        harness.clone(),
        harness.clone(),
        harness.clone(),
        harness.clone(),
    )
}

fn text_update(text: &str) -> JsonValue {
    json!({
        "update_id": 101,
        "message": {
            "message_id": 17,
            "chat": {"id": 42, "title": "Operations"},
            "from": {"id": 9, "username": "ada", "first_name": "Ada"},
            "text": text,
            "reply_to_message": {"message_id": 16, "text": "prior"},
        },
    })
}

fn document_update() -> JsonValue {
    json!({
        "update_id": 102,
        "message": {
            "message_id": 18,
            "chat": {"id": 42, "title": "Operations"},
            "from": {"id": 9, "username": "ada"},
            "caption": "review this",
            "document": {
                "file_id": "telegram-file-1",
                "file_unique_id": "unique-1",
                "file_name": "report.pdf",
                "mime_type": "application/pdf",
                "file_size": 1234,
            },
        },
    })
}

fn voice_update() -> JsonValue {
    json!({
        "update_id": 103,
        "message": {
            "message_id": 19,
            "chat": {"id": 42, "title": "Operations"},
            "from": {"id": 9, "username": "ada"},
            "voice": {
                "file_id": "telegram-voice-1",
                "file_unique_id": "voice-unique-1",
                "mime_type": "audio/ogg",
                "duration": 3,
            },
        },
    })
}

fn dispatch_item() -> JsonValue {
    json!({"dispatch_key": "chat-42", "update_key": "update-101"})
}

#[test]
fn telegram_update_job_rejects_invalid_configuration_without_echoing_input() {
    let harness = Arc::new(Harness::default());
    let error = TelegramUpdateJob::new(
        TelegramUpdateJobConfig::new("secret\nusername", false, false, false),
        ports(&harness),
    )
    .err()
    .expect("configuration error");

    assert_eq!(error.kind(), TelegramUpdateJobErrorKind::Configuration);
    assert_eq!(
        error.to_string(),
        "Telegram update job configuration is invalid."
    );
    assert!(!error.to_string().contains("secret"));
}

#[test]
fn telegram_update_job_ignores_updates_without_a_chat() {
    let harness = Arc::new(Harness::default());
    let outcome = job(&harness)
        .handle_update(&json!({"update_id": 1}), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["update_state"], "ignored");
    assert_eq!(outcome["action"], "missing_chat");
    assert_eq!(outcome["handled"], false);
    assert!(harness.events().is_empty());
}

#[test]
fn telegram_update_job_attachment_pre_gate_runs_before_input_io() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(true);

    let outcome = job(&harness)
        .handle_update(&document_update(), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "owner_bootstrap");
    assert_eq!(harness.events(), vec!["bootstrap"]);
    assert!(lock(&harness.input_requests).is_empty());
    let requests = lock(&harness.bootstrap_requests);
    assert!(requests[0].attachments_present());
    assert_eq!(requests[0].command(), None);
    assert_eq!(requests[0].raw_text(), Some("review this"));
}

#[test]
fn telegram_update_job_reports_typed_input_failure_and_stops() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(false);
    harness.push_input(Err(TelegramUpdateInputError::new(
        TelegramUpdateInputErrorKind::AttachmentDownloadFailed,
    )));

    let outcome = job(&harness)
        .handle_update(&document_update(), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "input_failure_reported");
    assert_eq!(outcome["ok"], false);
    assert_eq!(
        harness.events(),
        vec![
            "bootstrap",
            "input",
            "delivery:Telegram file download failed. Please retry in a moment."
        ]
    );
    assert!(lock(&harness.operational_requests).is_empty());
}

#[test]
fn telegram_update_job_speech_input_uses_explicit_stt_mode() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(false);
    harness.push_input(Ok(TelegramPreparedUpdateInput::new(
        Some("transcribed speech".to_string()),
        vec![json!({"kind": "voice", "local_path": "/safe/audio.ogg"})],
    )));
    harness.push_bootstrap(true);
    let job = job_with_config(
        &harness,
        TelegramUpdateJobConfig::new("ait_bot", true, false, false),
    );

    let outcome = job
        .handle_update(&voice_update(), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "owner_bootstrap");
    assert_eq!(harness.events(), vec!["bootstrap", "input", "bootstrap"]);
    let requests = lock(&harness.input_requests);
    assert_eq!(requests[0].mode(), TelegramUpdateInputMode::SpeechToText);
    assert_eq!(requests[0].attachments().len(), 1);
    let bootstrap = lock(&harness.bootstrap_requests);
    assert_eq!(bootstrap[1].raw_text(), Some("transcribed speech"));
}

#[test]
fn telegram_update_job_operational_trigger_short_circuits_command_and_reply() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(false);
    harness.push_operational(true);

    let outcome = job(&harness)
        .handle_update(&text_update("/queue"), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "operational_trigger");
    assert_eq!(harness.events(), vec!["bootstrap", "operational"]);
    assert!(lock(&harness.command_requests).is_empty());
    assert!(lock(&harness.normal_turn_requests).is_empty());
}

#[test]
fn telegram_update_job_dispatches_slash_command_after_gates() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(false);
    harness.push_operational(false);

    let outcome = job(&harness)
        .handle_update(&text_update("/audit T-123"), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "command");
    assert_eq!(
        harness.events(),
        vec!["bootstrap", "operational", "command"]
    );
    let commands = lock(&harness.command_requests);
    assert_eq!(commands[0].command_name(), "audit");
    assert_eq!(commands[0].command_args(), "T-123");
    let bootstrap = lock(&harness.bootstrap_requests);
    assert_eq!(bootstrap[0].command(), Some(("audit", "T-123")));
}

#[test]
fn telegram_update_job_dispatches_detected_workflow_query_without_forging_slash_command() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(false);
    harness.push_operational(false);

    let outcome = job(&harness)
        .handle_update(&text_update("queue summary"), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "command");
    let commands = lock(&harness.command_requests);
    assert_eq!(commands[0].command_name(), "queue");
    assert_eq!(commands[0].command_args(), "");
    let bootstrap = lock(&harness.bootstrap_requests);
    assert_eq!(bootstrap[0].command(), None);
    let operational = lock(&harness.operational_requests);
    assert_eq!(operational[0].command(), None);
}

#[test]
fn telegram_update_job_sends_exact_empty_input_help() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(false);
    harness.push_operational(false);

    let outcome = job(&harness)
        .handle_update(&text_update("@ait_bot:"), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "empty_help");
    assert_eq!(
        harness.events(),
        vec![
            "bootstrap",
            "operational",
            "delivery:Send a message after the bot mention, or use /help."
        ]
    );
}

#[test]
fn telegram_update_job_routes_normal_turn_with_message_context() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(false);
    harness.push_operational(false);

    let outcome = job(&harness)
        .handle_update(&text_update("  hello   team  "), &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "normal_turn");
    assert_eq!(
        harness.events(),
        vec!["bootstrap", "operational", "normal_turn"]
    );
    let turns = lock(&harness.normal_turn_requests);
    assert_eq!(turns[0].text(), "hello team");
    assert_eq!(turns[0].telegram_message_id(), Some(17));
    assert!(turns[0].telegram_message_ids().is_empty());
    assert_eq!(turns[0].actor_identity(), None);
    assert!(turns[0].defer_reply());
}

#[test]
fn telegram_update_job_logical_turn_preserves_actor_and_message_ids() {
    let harness = Arc::new(Harness::default());
    harness.push_operational(false);
    let turn = TelegramLogicalTurn {
        update: text_update("first raw message"),
        text: "merged logical turn".to_string(),
        actor_identity: "telegram:9".to_string(),
        telegram_message_id: Some(17),
        telegram_message_ids: vec![17, 18, 19],
    };

    let outcome = job(&harness)
        .handle_logical_turn(&turn, &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "normal_turn");
    assert_eq!(harness.events(), vec!["operational", "normal_turn"]);
    let operational = lock(&harness.operational_requests);
    assert_eq!(operational[0].raw_text(), "merged logical turn");
    assert_eq!(operational[0].actor_identity(), Some("telegram:9"));
    assert_eq!(operational[0].telegram_message_ids(), &[17, 18, 19]);
    let turns = lock(&harness.normal_turn_requests);
    assert_eq!(turns[0].actor_identity(), Some("telegram:9"));
    assert_eq!(turns[0].telegram_message_ids(), &[17, 18, 19]);
    assert!(turns[0].attachments().is_empty());
}

#[test]
fn telegram_update_job_logical_operational_trigger_short_circuits_reply() {
    let harness = Arc::new(Harness::default());
    harness.push_operational(true);
    let turn = TelegramLogicalTurn {
        update: text_update("first raw message"),
        text: "merged trigger".to_string(),
        actor_identity: "telegram:9".to_string(),
        telegram_message_id: Some(17),
        telegram_message_ids: vec![17, 18],
    };

    let outcome = job(&harness)
        .handle_logical_turn(&turn, &dispatch_item())
        .expect("outcome");

    assert_eq!(outcome["action"], "operational_trigger");
    assert_eq!(harness.events(), vec!["operational"]);
    assert!(lock(&harness.normal_turn_requests).is_empty());
}

#[test]
fn telegram_update_job_delegates_all_submission_lifecycle_methods() {
    let harness = Arc::new(Harness::default());
    let job = job(&harness);

    let background = job.run_background_sync_for_chat("42").expect("background");
    let reply = job
        .execute_reply("deliver_reply", &[json!({"opaque": true})])
        .expect("reply");
    let idle = job
        .wait_for_live_replies(Some(Duration::from_millis(25)))
        .expect("idle");

    assert_eq!(background["action"], "background_sync");
    assert_eq!(reply["action"], "reply");
    assert!(idle);
    assert_eq!(
        harness.events(),
        vec!["background_sync", "reply", "live_reply_wait"]
    );
}

#[test]
fn telegram_update_job_records_lifecycle_failures_without_leaking_payloads() {
    let harness = Arc::new(Harness::default());
    harness.reply_fails.store(true, Ordering::SeqCst);

    let error = job(&harness)
        .execute_reply(PLANNER_SECRET, &[json!({"secret": PLANNER_SECRET})])
        .expect_err("reply error");

    assert_eq!(error, "Telegram update lifecycle execution failed.");
    assert!(!error.contains(PLANNER_SECRET));
    assert_eq!(harness.events(), vec!["reply", "diagnostic:lifecycle"]);
}

#[test]
fn telegram_update_job_reports_command_failure_once_and_stops() {
    let harness = Arc::new(Harness::default());
    harness.command_fails.store(true, Ordering::SeqCst);
    harness.push_bootstrap(false);
    harness.push_operational(false);

    let outcome = job(&harness)
        .handle_update(&text_update("/queue"), &dispatch_item())
        .expect("reported failure");

    assert_eq!(outcome["update_state"], "failed");
    assert_eq!(outcome["failure_kind"], "command");
    assert_eq!(
        harness.events(),
        vec![
            "bootstrap",
            "operational",
            "command",
            "diagnostic:command",
            concat!(
                "delivery:ait Telegram bot hit an unexpected error while processing this update. ",
                "Check the daemon log and retry if needed."
            )
        ]
    );
}

#[test]
fn telegram_update_job_invalid_prepared_input_fails_closed_before_actions() {
    let harness = Arc::new(Harness::default());
    harness.push_bootstrap(false);
    harness.push_input(Ok(TelegramPreparedUpdateInput::new(
        Some("safe caption".to_string()),
        vec![json!(PLANNER_SECRET)],
    )));

    let outcome = job(&harness)
        .handle_update(&document_update(), &dispatch_item())
        .expect("reported failure");

    assert_eq!(outcome["failure_kind"], "input_contract");
    assert!(!outcome.to_string().contains(PLANNER_SECRET));
    assert_eq!(
        &harness.events()[..3],
        &["bootstrap", "input", "diagnostic:input_contract"]
    );
    assert!(lock(&harness.operational_requests).is_empty());
}

struct CorruptWorkflowPlanner;

impl TelegramWorkflowQueryPlanner for CorruptWorkflowPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let mut planned = DefaultTelegramWorkflowQueryPlanner.plan_json(request)?;
        if request.get("kind").and_then(JsonValue::as_str) == Some("message_entrypoint") {
            planned["forged_secret"] = json!(PLANNER_SECRET);
        }
        Ok(planned)
    }
}

struct CorruptTurnInputPlanner;

impl TelegramTurnInputPlanner for CorruptTurnInputPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let mut planned = DefaultTelegramTurnInputPlanner.plan_json(request)?;
        if request.get("kind").and_then(JsonValue::as_str) == Some("normalized_turn_text") {
            planned["forged_secret"] = json!(PLANNER_SECRET);
        }
        Ok(planned)
    }
}

#[test]
fn telegram_update_job_corrupt_workflow_envelope_fails_closed_and_is_secret_safe() {
    let harness = Arc::new(Harness::default());
    let job = TelegramUpdateJob::with_planners(
        TelegramUpdateJobConfig::new("ait_bot", false, false, false),
        ports(&harness),
        Arc::new(DefaultTelegramTurnInputPlanner),
        Arc::new(CorruptWorkflowPlanner),
    )
    .expect("job");

    let outcome = job
        .handle_update(&text_update("hello"), &dispatch_item())
        .expect("reported failure");

    assert_eq!(outcome["failure_kind"], "workflow_planner_contract");
    assert!(!outcome.to_string().contains(PLANNER_SECRET));
    assert!(lock(&harness.bootstrap_requests).is_empty());
    assert!(lock(&harness.operational_requests).is_empty());
}

#[test]
fn telegram_update_job_corrupt_turn_input_envelope_fails_closed_and_is_secret_safe() {
    let harness = Arc::new(Harness::default());
    let job = TelegramUpdateJob::with_planners(
        TelegramUpdateJobConfig::new("ait_bot", false, false, false),
        ports(&harness),
        Arc::new(CorruptTurnInputPlanner),
        Arc::new(DefaultTelegramWorkflowQueryPlanner),
    )
    .expect("job");

    let outcome = job
        .handle_update(&text_update("hello"), &dispatch_item())
        .expect("reported failure");

    assert_eq!(outcome["failure_kind"], "turn_input_planner_contract");
    assert!(!outcome.to_string().contains(PLANNER_SECRET));
    assert!(lock(&harness.bootstrap_requests).is_empty());
    assert!(lock(&harness.operational_requests).is_empty());
}

#[test]
fn telegram_update_job_diagnostic_failure_returns_only_stable_error() {
    let harness = Arc::new(Harness::default());
    harness.command_fails.store(true, Ordering::SeqCst);
    harness.diagnostics_fails.store(true, Ordering::SeqCst);
    harness.push_bootstrap(false);
    harness.push_operational(false);

    let error = job(&harness)
        .handle_update(&text_update("/queue"), &dispatch_item())
        .expect_err("diagnostic error");

    assert_eq!(error, "Telegram update diagnostics execution failed.");
    assert!(!error.contains(PLANNER_SECRET));
    assert_eq!(
        harness.events(),
        vec!["bootstrap", "operational", "command", "diagnostic:command"]
    );
}
