use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use ait_core::json_support::{json, JsonValue};

use super::*;
use crate::event_loop::{
    agent_telegram_callback_action_boundary_plan_json,
    execute_with_telegram_webhook_transaction_ports, DefaultTelegramWebhookTransactionIngressPort,
    TelegramLogicalTurn, TelegramLogicalTurnRuntime, TelegramSubmissionExecutionPort,
};

struct ExecutionGate {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
}

impl Default for ExecutionGate {
    fn default() -> Self {
        Self {
            state: Mutex::new((0, false)),
            changed: Condvar::new(),
        }
    }
}

impl ExecutionGate {
    fn enter_and_wait(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }

    fn wait_until_started(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut state = self.state.lock().unwrap();
        while state.0 == 0 {
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "submission job did not start");
            state = self.changed.wait_timeout(state, remaining).unwrap().0;
        }
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap();
        state.1 = true;
        self.changed.notify_all();
    }
}

struct RecordingExecution {
    handled: Mutex<Vec<(i64, String)>>,
    gate: Arc<ExecutionGate>,
    blocking: bool,
    fail_updates: AtomicBool,
    fail_live_reply_wait: AtomicBool,
}

impl RecordingExecution {
    fn new(blocking: bool) -> Arc<Self> {
        Arc::new(Self {
            handled: Mutex::new(Vec::new()),
            gate: Arc::new(ExecutionGate::default()),
            blocking,
            fail_updates: AtomicBool::new(false),
            fail_live_reply_wait: AtomicBool::new(false),
        })
    }

    fn record_update(
        &self,
        update: &JsonValue,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String> {
        let update_id = update
            .get("update_id")
            .and_then(JsonValue::as_i64)
            .unwrap_or_default();
        let dispatch_key = dispatch_item
            .get("dispatch_key")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string();
        self.handled.lock().unwrap().push((update_id, dispatch_key));
        if self.blocking {
            self.gate.enter_and_wait();
        }
        if self.fail_updates.load(Ordering::Acquire) {
            Err("downstream-executor-secret private-update".to_string())
        } else {
            Ok(json!({"handled": true, "private": "must-not-reach-snapshot"}))
        }
    }
}

impl TelegramSubmissionExecutionPort for RecordingExecution {
    fn handle_update(
        &self,
        update: &JsonValue,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.record_update(update, dispatch_item)
    }

    fn handle_logical_turn(
        &self,
        turn: &TelegramLogicalTurn,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.record_update(&turn.update, dispatch_item)
    }

    fn run_background_sync_for_chat(&self, _chat_id: &str) -> Result<JsonValue, String> {
        Ok(json!({"ok": true}))
    }

    fn execute_reply(
        &self,
        _callback_slot: &str,
        _args: &[JsonValue],
    ) -> Result<JsonValue, String> {
        Ok(json!({"ok": true}))
    }

    fn wait_for_live_replies(&self, _timeout: Option<Duration>) -> Result<bool, String> {
        if self.fail_live_reply_wait.load(Ordering::Acquire) {
            Err("live-reply-secret".to_string())
        } else {
            Ok(true)
        }
    }
}

fn adapter(
    execution: Arc<RecordingExecution>,
    worker_count: usize,
    queue_capacity: usize,
    inflight_limit: usize,
) -> TelegramSubmissionDispatchPort {
    let logical = Arc::new(
        TelegramLogicalTurnRuntime::new(
            "aitbot",
            Duration::ZERO,
            4,
            Duration::from_millis(1),
            16,
            16,
        )
        .unwrap(),
    );
    let execution_port: Arc<dyn TelegramSubmissionExecutionPort> = execution;
    let runtime = TelegramSubmissionRuntime::new(
        execution_port,
        logical,
        &json!({
            "backend": "portable_poll",
            "worker_leases": [{"shard_index": 0}],
            "shard_admissions": [{
                "shard_index": 0,
                "inflight_limit": inflight_limit,
            }],
        }),
        worker_count,
        queue_capacity,
    )
    .unwrap();
    TelegramSubmissionDispatchPort::new(Arc::new(runtime))
}

fn update(update_id: i64, chat_id: i64, text: &str) -> JsonValue {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": update_id * 10,
            "text": text,
            "chat": {"id": chat_id, "type": "private"},
            "from": {"id": 7, "username": "wei"},
        },
    })
}

fn dispatch_item(update: &JsonValue, index: i64, fallback_update_key: &str) -> JsonValue {
    let mut item = agent_telegram_update_dispatch_plan_json(&json!({
        "update": update,
        "fallback_update_key": fallback_update_key,
    }))
    .unwrap();
    item["index"] = json!(index);
    item
}

fn polling_request(update: JsonValue, index: i64) -> JsonValue {
    let item = dispatch_item(&update, index, "polling-fallback");
    agent_telegram_callback_action_boundary_plan_json(&json!({
        "stage": "request",
        "action": {
            "kind": "dispatch_update",
            "index": index,
            "dispatch_item": item,
        },
        "updates": [update],
    }))
    .unwrap()["request"]
        .clone()
}

fn webhook_request(update: JsonValue, index: i64, fallback_update_key: &str) -> JsonValue {
    let item = dispatch_item(&update, index, fallback_update_key);
    json!({
        "source": "telegram_webhook",
        "index": index,
        "update": update,
        "dispatch_item": item,
        "dispatch_key": item["dispatch_key"],
        "queue_key": item["dispatch_key"],
        "update_key": item["update_key"],
        "fallback_update_key": fallback_update_key,
    })
}

#[test]
fn polling_and_webhook_enqueue_into_one_submission_runtime_in_key_order() {
    let execution = RecordingExecution::new(false);
    let port = adapter(Arc::clone(&execution), 2, 8, 16);
    let polling = polling_request(update(1, 7, "private-polling-text"), 0);

    TelegramServiceCycleDispatchPort::dispatch_update(&port, &polling).unwrap();
    let webhook = execute_with_telegram_webhook_transaction_ports(
        &DefaultTelegramWebhookTransactionIngressPort,
        &port,
        &json!({
            "raw_payload": r#"{
                "update_id": 2,
                "message": {
                    "message_id": 20,
                    "text": "private-webhook-text",
                    "chat": {"id": 7, "type": "private"},
                    "from": {"id": 7, "username": "wei"}
                }
            }"#,
        }),
    )
    .unwrap();
    assert_eq!(webhook["dispatched_update_count"], 1);
    assert!(port.wait_for_idle(Some(Duration::from_secs(2))).unwrap());

    assert_eq!(
        *execution.handled.lock().unwrap(),
        vec![(1, "chat-7".to_string()), (2, "chat-7".to_string())]
    );
    let snapshot = port.snapshot_json();
    assert_eq!(snapshot["contract"], CONTRACT);
    assert_eq!(snapshot["submitted_planned_update_count"], 2);
    assert_eq!(snapshot["handled_update_count"], 2);
    assert_eq!(snapshot["inflight_count"], 0);
    assert_eq!(snapshot["python_dispatch_allowed"], false);
    let rendered = snapshot.to_string();
    for private in [
        "private-polling-text",
        "private-webhook-text",
        "chat-7",
        "webhook-2",
    ] {
        assert!(!rendered.contains(private));
    }
    let debug = format!("{port:?}");
    assert!(!debug.contains("private"));
    assert!(!debug.contains("chat-7"));
}

#[test]
fn malformed_or_mismatched_envelopes_fail_closed_before_submission() {
    let execution = RecordingExecution::new(false);
    let port = adapter(Arc::clone(&execution), 1, 8, 16);
    let base = polling_request(update(3, 9, "validation-secret"), 0);
    let mut cases = vec![json!([])];

    let mut missing_update = base.clone();
    missing_update["update"] = JsonValue::Null;
    cases.push(missing_update);
    let mut missing_item = base.clone();
    missing_item["dispatch_item"] = JsonValue::Null;
    cases.push(missing_item);
    let mut wrong_callback = base.clone();
    wrong_callback["callback_kind"] = json!("private-callback-secret");
    cases.push(wrong_callback);
    let mut wrong_queue = base.clone();
    wrong_queue["queue_key"] = json!("private-queue-secret");
    cases.push(wrong_queue);
    let mut wrong_update_key = base.clone();
    wrong_update_key["update_key"] = json!("private-update-key-secret");
    cases.push(wrong_update_key);
    let mut wrong_identity = base.clone();
    wrong_identity["dispatch_item"]["chat_id"] = json!(999);
    cases.push(wrong_identity);
    let mut wrong_index = base;
    wrong_index["dispatch_item"]["index"] = json!(1);
    cases.push(wrong_index);

    for request in cases {
        let error = TelegramServiceCycleDispatchPort::dispatch_update(&port, &request)
            .expect_err("invalid polling envelope");
        assert_eq!(error, DISPATCH_FAILURE);
        assert!(!error.contains("secret"));
    }

    let mut wrong_webhook =
        webhook_request(update(4, 9, "webhook-secret"), 0, "webhook-fallback-secret");
    wrong_webhook["source"] = json!("private-source-secret");
    let error = TelegramWebhookTransactionDispatchPort::dispatch_update(&port, &wrong_webhook)
        .expect_err("invalid webhook envelope");
    assert_eq!(error, DISPATCH_FAILURE);
    assert!(execution.handled.lock().unwrap().is_empty());
    assert_eq!(port.snapshot_json()["submitted_planned_update_count"], 0);
}

#[test]
fn capacity_timeout_stop_and_dropped_future_preserve_lifecycle_contract() {
    let execution = RecordingExecution::new(true);
    let port = adapter(Arc::clone(&execution), 1, 1, 1);
    let first = polling_request(update(5, 11, "blocking-secret"), 0);
    TelegramServiceCycleDispatchPort::dispatch_update(&port, &first).unwrap();
    execution.gate.wait_until_started();

    let second = polling_request(update(6, 12, "capacity-secret"), 0);
    let error = TelegramServiceCycleDispatchPort::dispatch_update(&port, &second)
        .expect_err("capacity must fail closed");
    assert_eq!(error, DISPATCH_FAILURE);
    assert!(!error.contains("capacity-secret"));
    assert!(!port.wait_for_idle(Some(Duration::from_millis(5))).unwrap());
    assert_eq!(port.snapshot_json()["inflight_count"], 1);

    execution.gate.release();
    assert!(port.wait_for_idle(Some(Duration::from_secs(2))).unwrap());
    assert_eq!(execution.handled.lock().unwrap().len(), 1);
    port.request_stop().unwrap();
    assert_eq!(port.snapshot_json()["stopped"], true);

    let stopped = polling_request(update(7, 13, "stopped-secret"), 0);
    let error = TelegramServiceCycleDispatchPort::dispatch_update(&port, &stopped)
        .expect_err("stopped runtime must reject");
    assert_eq!(error, DISPATCH_FAILURE);
    assert!(!error.contains("stopped-secret"));
}

#[test]
fn asynchronous_executor_and_live_reply_failures_surface_only_generic_idle_errors() {
    let execution = RecordingExecution::new(false);
    execution.fail_updates.store(true, Ordering::Release);
    let port = adapter(Arc::clone(&execution), 1, 4, 8);
    let request = webhook_request(
        update(8, 14, "executor-private-text"),
        0,
        "executor-private-fallback",
    );

    TelegramWebhookTransactionDispatchPort::dispatch_update(&port, &request).unwrap();
    let error = port
        .wait_for_idle(Some(Duration::from_secs(2)))
        .expect_err("failed async job must be observable");
    assert_eq!(error, IDLE_FAILURE);
    assert!(!error.contains("executor"));
    let snapshot = port.snapshot_json();
    assert_eq!(snapshot["execution_failure_count"], 1);
    assert_eq!(snapshot["failed_count"], 1);
    let rendered = snapshot.to_string();
    assert!(!rendered.contains("executor-private-text"));
    assert!(!rendered.contains("executor-private-fallback"));
    assert!(!rendered.contains("downstream-executor-secret"));

    let live_execution = RecordingExecution::new(false);
    live_execution
        .fail_live_reply_wait
        .store(true, Ordering::Release);
    let live_port = adapter(live_execution, 1, 4, 8);
    let error = live_port
        .wait_for_idle(Some(Duration::from_millis(1)))
        .expect_err("live reply failure must be generic");
    assert_eq!(error, IDLE_FAILURE);
    assert!(!error.contains("live-reply-secret"));
}
