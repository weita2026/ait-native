use super::*;
use crate::event_loop::telegram_logical_turn_runtime::{
    TelegramLogicalTurnClockPort, TelegramLogicalTurnSleepPort,
};
use ait_core::json_support::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};

const TEST_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    wake: Condvar,
}

impl Gate {
    fn wait(&self) {
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

struct TestExecutionPort {
    started: mpsc::Sender<String>,
    completed: Mutex<Vec<String>>,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
    live_reply_calls: AtomicUsize,
    live_reply_idle: AtomicBool,
    live_reply_error: AtomicBool,
}

impl TestExecutionPort {
    fn new() -> (Arc<Self>, mpsc::Receiver<String>) {
        let (started, receiver) = mpsc::channel();
        (
            Arc::new(Self {
                started,
                completed: Mutex::new(Vec::new()),
                gates: Mutex::new(HashMap::new()),
                live_reply_calls: AtomicUsize::new(0),
                live_reply_idle: AtomicBool::new(true),
                live_reply_error: AtomicBool::new(false),
            }),
            receiver,
        )
    }

    fn gate(&self, marker: &str) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        self.gates
            .lock()
            .unwrap()
            .insert(marker.to_string(), Arc::clone(&gate));
        gate
    }

    fn run_marker(&self, marker: String, mode: Option<&str>) -> Result<(), String> {
        let _ = self.started.send(marker.clone());
        let gate = { self.gates.lock().unwrap().get(&marker).cloned() };
        if let Some(gate) = gate {
            gate.wait();
        }
        match mode {
            Some("fail") => return Err("downstream-secret-detail".to_string()),
            Some("panic") => panic!("downstream-secret-panic"),
            _ => {}
        }
        self.completed.lock().unwrap().push(marker);
        Ok(())
    }

    fn completed(&self) -> Vec<String> {
        self.completed.lock().unwrap().clone()
    }
}

impl TelegramSubmissionExecutionPort for TestExecutionPort {
    fn handle_update(
        &self,
        update: &JsonValue,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String> {
        let label = update
            .get("label")
            .and_then(JsonValue::as_str)
            .unwrap_or("update");
        self.run_marker(
            format!("update:{label}"),
            update.get("mode").and_then(JsonValue::as_str),
        )?;
        Ok(json!({
            "kind": "update",
            "label": label,
            "dispatch_key": dispatch_item.get("dispatch_key").cloned().unwrap_or(JsonValue::Null),
        }))
    }

    fn handle_logical_turn(
        &self,
        turn: &TelegramLogicalTurn,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String> {
        let update_id = turn
            .update
            .get("update_id")
            .and_then(JsonValue::as_i64)
            .unwrap_or_default();
        self.run_marker(format!("logical:{update_id}"), None)?;
        Ok(json!({
            "kind": "logical_turn",
            "first_update_id": update_id,
            "text": turn.text,
            "message_ids": turn.telegram_message_ids,
            "dispatch_key": dispatch_item.get("dispatch_key").cloned().unwrap_or(JsonValue::Null),
        }))
    }

    fn run_background_sync_for_chat(&self, chat_id: &str) -> Result<JsonValue, String> {
        self.run_marker(format!("background:{chat_id}"), None)?;
        Ok(json!({"kind": "background_sync", "chat_id": chat_id}))
    }

    fn execute_reply(&self, callback_slot: &str, args: &[JsonValue]) -> Result<JsonValue, String> {
        let mode = if callback_slot == "fail_reply" {
            Some("fail")
        } else {
            None
        };
        self.run_marker(format!("reply:{callback_slot}"), mode)?;
        Ok(json!({
            "kind": "reply",
            "callback_slot": callback_slot,
            "arg_count": args.len(),
        }))
    }

    fn wait_for_live_replies(&self, _timeout: Option<Duration>) -> Result<bool, String> {
        self.live_reply_calls.fetch_add(1, Ordering::AcqRel);
        if self.live_reply_error.load(Ordering::Acquire) {
            Err("live-reply-secret-detail".to_string())
        } else {
            Ok(self.live_reply_idle.load(Ordering::Acquire))
        }
    }
}

struct VirtualTime {
    now: Mutex<f64>,
    fail_clock: bool,
}

impl VirtualTime {
    fn new(now: f64) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            fail_clock: false,
        })
    }

    fn failing(now: f64) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            fail_clock: true,
        })
    }
}

impl TelegramLogicalTurnClockPort for VirtualTime {
    fn now_monotonic_seconds(&self) -> Result<f64, String> {
        if self.fail_clock {
            Err("clock-secret-detail".to_string())
        } else {
            Ok(*self.now.lock().unwrap())
        }
    }
}

impl TelegramLogicalTurnSleepPort for VirtualTime {
    fn sleep(&self, duration: Duration) -> Result<(), String> {
        *self.now.lock().unwrap() += duration.as_secs_f64();
        Ok(())
    }
}

struct BlockingVirtualTime {
    now: Mutex<f64>,
    released: Mutex<bool>,
    wake: Condvar,
    first_sleep: Mutex<Option<mpsc::Sender<()>>>,
}

impl BlockingVirtualTime {
    fn new(now: f64) -> (Arc<Self>, mpsc::Receiver<()>) {
        let (sender, receiver) = mpsc::channel();
        (
            Arc::new(Self {
                now: Mutex::new(now),
                released: Mutex::new(false),
                wake: Condvar::new(),
                first_sleep: Mutex::new(Some(sender)),
            }),
            receiver,
        )
    }

    fn release(&self) {
        *self.released.lock().unwrap() = true;
        self.wake.notify_all();
    }
}

impl TelegramLogicalTurnClockPort for BlockingVirtualTime {
    fn now_monotonic_seconds(&self) -> Result<f64, String> {
        Ok(*self.now.lock().unwrap())
    }
}

impl TelegramLogicalTurnSleepPort for BlockingVirtualTime {
    fn sleep(&self, duration: Duration) -> Result<(), String> {
        if let Some(sender) = self.first_sleep.lock().unwrap().take() {
            let _ = sender.send(());
        }
        let mut released = self.released.lock().unwrap();
        while !*released {
            released = self.wake.wait(released).unwrap();
        }
        drop(released);
        *self.now.lock().unwrap() += duration.as_secs_f64();
        Ok(())
    }
}

struct BrokenPlanner;

impl SubmissionExecutionPlanningPort for BrokenPlanner {
    fn plan(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Ok(json!({"invalid": "planner-secret-detail"}))
    }
}

fn admission_plan(inflight_limit: usize) -> JsonValue {
    json!({
        "backend": "portable_poll",
        "worker_leases": [{"shard_index": 0}],
        "shard_admissions": [{"shard_index": 0, "inflight_limit": inflight_limit}],
    })
}

fn logical_runtime(
    time: Arc<dyn TelegramLogicalTurnClockPort>,
    sleeper: Arc<dyn TelegramLogicalTurnSleepPort>,
    merge_window: Duration,
    max_messages: usize,
    max_pending_chats: usize,
    max_pending_per_chat: usize,
) -> Arc<TelegramLogicalTurnRuntime> {
    Arc::new(
        TelegramLogicalTurnRuntime::with_ports(
            "aitbot",
            merge_window,
            max_messages,
            Duration::from_millis(100),
            max_pending_chats,
            max_pending_per_chat,
            time,
            sleeper,
        )
        .unwrap(),
    )
}

fn disabled_logical_runtime() -> Arc<TelegramLogicalTurnRuntime> {
    let time = VirtualTime::new(1.0);
    let clock: Arc<dyn TelegramLogicalTurnClockPort> = time.clone();
    let sleeper: Arc<dyn TelegramLogicalTurnSleepPort> = time;
    logical_runtime(clock, sleeper, Duration::ZERO, 4, 16, 16)
}

fn runtime(
    port: Arc<TestExecutionPort>,
    logical: Arc<TelegramLogicalTurnRuntime>,
    worker_count: usize,
    queue_capacity: usize,
    inflight_limit: usize,
) -> TelegramSubmissionRuntime {
    let execution: Arc<dyn TelegramSubmissionExecutionPort> = port;
    TelegramSubmissionRuntime::new(
        execution,
        logical,
        &admission_plan(inflight_limit),
        worker_count,
        queue_capacity,
    )
    .unwrap()
}

fn update(update_id: i64, chat_id: i64, label: &str) -> JsonValue {
    json!({
        "update_id": update_id,
        "label": label,
        "message": {
            "message_id": update_id * 10,
            "text": label,
            "chat": {"id": chat_id, "type": "private"},
            "from": {"id": 7, "username": "wei"},
        }
    })
}

fn receive(receiver: &mpsc::Receiver<String>) -> String {
    receiver.recv_timeout(TEST_TIMEOUT).unwrap()
}

#[test]
fn raw_and_preplanned_updates_execute_with_rust_selected_dispatch_items() {
    let (port, started) = TestExecutionPort::new();
    let runtime = runtime(port, disabled_logical_runtime(), 2, 8, 16);

    let raw = runtime.submit_update(update(1, 10, "raw")).unwrap();
    assert_eq!(receive(&started), "update:raw");
    assert_eq!(
        raw.wait(Some(TEST_TIMEOUT)).unwrap()["dispatch_key"],
        "chat-10"
    );

    let planned = runtime
        .submit_planned_update(
            update(2, 20, "planned"),
            json!({"dispatch_key": "custom-chat", "update_key": "planned-2"}),
        )
        .unwrap();
    assert_eq!(receive(&started), "update:planned");
    assert_eq!(
        planned.wait(Some(TEST_TIMEOUT)).unwrap()["dispatch_key"],
        "custom-chat"
    );
}

#[test]
fn same_chat_is_fifo_while_unrelated_and_reply_namespaces_progress() {
    let (port, started) = TestExecutionPort::new();
    let first_gate = port.gate("update:first");
    let runtime = runtime(port.clone(), disabled_logical_runtime(), 3, 8, 16);

    let first = runtime.submit_update(update(1, 1, "first")).unwrap();
    assert_eq!(receive(&started), "update:first");
    let second = runtime.submit_update(update(2, 1, "second")).unwrap();
    let unrelated = runtime.submit_update(update(3, 2, "unrelated")).unwrap();
    let reply = runtime
        .submit_reply_serialized("chat-1", "send_reply", vec![json!("reply-secret")])
        .unwrap();

    let mut concurrent = vec![receive(&started), receive(&started)];
    concurrent.sort();
    assert_eq!(concurrent, ["reply:send_reply", "update:unrelated"]);
    assert!(matches!(started.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert_eq!(
        unrelated.wait(Some(TEST_TIMEOUT)).unwrap()["kind"],
        "update"
    );
    assert_eq!(reply.wait(Some(TEST_TIMEOUT)).unwrap()["kind"], "reply");

    first_gate.release();
    assert_eq!(first.wait(Some(TEST_TIMEOUT)).unwrap()["label"], "first");
    assert_eq!(receive(&started), "update:second");
    assert_eq!(second.wait(Some(TEST_TIMEOUT)).unwrap()["label"], "second");
    let completed = port.completed();
    let mut concurrent_completed = completed[..2].to_vec();
    concurrent_completed.sort();
    assert_eq!(
        concurrent_completed,
        ["reply:send_reply", "update:unrelated"]
    );
    assert_eq!(&completed[2..], ["update:first", "update:second"]);
}

#[test]
fn background_sync_shares_the_update_queue_namespace() {
    let (port, started) = TestExecutionPort::new();
    let gate = port.gate("update:blocking");
    let runtime = runtime(port, disabled_logical_runtime(), 2, 8, 16);

    let first = runtime.submit_update(update(1, 7, "blocking")).unwrap();
    assert_eq!(receive(&started), "update:blocking");
    let background = runtime
        .submit_background_sync_for_chat(Some("chat-7"), json!(7))
        .unwrap();
    assert!(matches!(started.try_recv(), Err(mpsc::TryRecvError::Empty)));

    gate.release();
    first.wait(Some(TEST_TIMEOUT)).unwrap();
    assert_eq!(receive(&started), "background:7");
    assert_eq!(
        background.wait(Some(TEST_TIMEOUT)).unwrap()["kind"],
        "background_sync"
    );
}

#[test]
fn merge_execution_delivers_first_raw_update_and_skips_consumed_follower() {
    let (time, sleep_started) = BlockingVirtualTime::new(10.0);
    let clock: Arc<dyn TelegramLogicalTurnClockPort> = time.clone();
    let sleeper: Arc<dyn TelegramLogicalTurnSleepPort> = time.clone();
    let logical = logical_runtime(clock, sleeper, Duration::from_secs(10), 2, 4, 4);
    let (port, started) = TestExecutionPort::new();
    let runtime = runtime(port, logical, 2, 8, 16);

    let first = runtime.submit_update(update(1, 9, "first text")).unwrap();
    sleep_started.recv_timeout(TEST_TIMEOUT).unwrap();
    let second = runtime.submit_update(update(2, 9, "second text")).unwrap();
    time.release();

    assert_eq!(receive(&started), "logical:1");
    let turn = first.wait(Some(TEST_TIMEOUT)).unwrap();
    assert_eq!(turn["kind"], "logical_turn");
    assert_eq!(turn["first_update_id"], 1);
    assert_eq!(turn["text"], "first text\n\nsecond text");
    assert_eq!(turn["message_ids"], json!([10, 20]));
    let skipped = second.wait(Some(TEST_TIMEOUT)).unwrap();
    assert_eq!(skipped["submission_state"], "skipped");
    assert_eq!(skipped["handled"], false);
}

#[test]
fn duplicate_buffering_is_atomic_and_second_job_is_skipped() {
    let (time, sleep_started) = BlockingVirtualTime::new(10.0);
    let clock: Arc<dyn TelegramLogicalTurnClockPort> = time.clone();
    let sleeper: Arc<dyn TelegramLogicalTurnSleepPort> = time.clone();
    let logical = logical_runtime(clock, sleeper, Duration::from_secs(1), 4, 4, 4);
    let (port, started) = TestExecutionPort::new();
    let runtime = runtime(port, logical, 2, 8, 16);
    let duplicated = update(1, 9, "duplicate-secret-text");

    let first = runtime.submit_update(duplicated.clone()).unwrap();
    sleep_started.recv_timeout(TEST_TIMEOUT).unwrap();
    let second = runtime.submit_update(duplicated).unwrap();
    time.release();

    assert_eq!(receive(&started), "logical:1");
    assert_eq!(
        first.wait(Some(TEST_TIMEOUT)).unwrap()["kind"],
        "logical_turn"
    );
    assert_eq!(
        second.wait(Some(TEST_TIMEOUT)).unwrap()["submission_state"],
        "skipped"
    );
    let snapshot = runtime.snapshot_json();
    assert_eq!(snapshot["logical_duplicate_count"], 1);
    assert_eq!(snapshot["skipped_duplicate_count"], 1);
}

#[test]
fn command_text_passes_through_without_merge_waiting() {
    let time = VirtualTime::new(10.0);
    let clock: Arc<dyn TelegramLogicalTurnClockPort> = time.clone();
    let sleeper: Arc<dyn TelegramLogicalTurnSleepPort> = time;
    let logical = logical_runtime(clock, sleeper, Duration::from_secs(5), 4, 4, 4);
    let (port, started) = TestExecutionPort::new();
    let runtime = runtime(port, logical, 1, 4, 8);
    let mut command = update(1, 1, "command");
    command["message"]["text"] = json!("/queue");

    let future = runtime.submit_update(command).unwrap();
    assert_eq!(receive(&started), "update:command");
    assert_eq!(future.wait(Some(TEST_TIMEOUT)).unwrap()["kind"], "update");
}

#[test]
fn logical_capacity_and_clock_failures_are_secret_safe() {
    let (time, sleep_started) = BlockingVirtualTime::new(10.0);
    let clock: Arc<dyn TelegramLogicalTurnClockPort> = time.clone();
    let sleeper: Arc<dyn TelegramLogicalTurnSleepPort> = time.clone();
    let logical = logical_runtime(clock, sleeper, Duration::from_secs(5), 4, 4, 1);
    let (port, _started) = TestExecutionPort::new();
    let capacity_runtime = runtime(port, logical, 1, 4, 8);
    let first = capacity_runtime
        .submit_update(update(1, 1, "first"))
        .unwrap();
    sleep_started.recv_timeout(TEST_TIMEOUT).unwrap();
    let error = capacity_runtime
        .submit_update(update(2, 1, "capacity-secret"))
        .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramSubmissionExecutionErrorKind::LogicalTurnCapacity
    );
    assert!(!error.to_string().contains("capacity-secret"));
    time.release();
    first.wait(Some(TEST_TIMEOUT)).unwrap();

    let failing = VirtualTime::failing(10.0);
    let failing_clock: Arc<dyn TelegramLogicalTurnClockPort> = failing.clone();
    let failing_sleeper: Arc<dyn TelegramLogicalTurnSleepPort> = failing;
    let logical = logical_runtime(
        failing_clock,
        failing_sleeper,
        Duration::from_secs(1),
        4,
        4,
        4,
    );
    let (port, _) = TestExecutionPort::new();
    let failing_runtime = runtime(port, logical, 1, 4, 8);
    let error = failing_runtime
        .submit_update(update(3, 3, "clock-secret"))
        .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramSubmissionExecutionErrorKind::LogicalTurnRuntime
    );
    assert!(!error.to_string().contains("clock-secret"));
}

#[test]
fn capacity_timeout_stop_and_callback_validation_are_explicit() {
    let (port, started) = TestExecutionPort::new();
    let gate = port.gate("update:blocking");
    let runtime = runtime(port, disabled_logical_runtime(), 1, 4, 1);
    let first = runtime.submit_update(update(1, 1, "blocking")).unwrap();
    assert_eq!(receive(&started), "update:blocking");

    let capacity = runtime.submit_update(update(2, 2, "late")).unwrap_err();
    assert_eq!(
        capacity.kind(),
        TelegramSubmissionExecutionErrorKind::InflightLimit
    );
    let timeout = first.wait(Some(Duration::ZERO)).unwrap_err();
    assert_eq!(
        timeout.kind(),
        TelegramSubmissionExecutionErrorKind::Timeout
    );
    let invalid = runtime
        .submit_reply_serialized("chat-1", "bad/slot", Vec::new())
        .unwrap_err();
    assert_eq!(
        invalid.kind(),
        TelegramSubmissionExecutionErrorKind::InvalidCallbackSlot
    );

    gate.release();
    assert!(runtime.wait_for_idle(Some(TEST_TIMEOUT)).unwrap());
    runtime.request_stop().unwrap();
    let stopped = runtime
        .submit_update(update(3, 3, "stopped-secret"))
        .unwrap_err();
    assert_eq!(
        stopped.kind(),
        TelegramSubmissionExecutionErrorKind::Stopped
    );
}

#[test]
fn dispatch_rejection_rolls_back_only_the_new_logical_buffer_entry() {
    let time = VirtualTime::new(10.0);
    let clock: Arc<dyn TelegramLogicalTurnClockPort> = time.clone();
    let sleeper: Arc<dyn TelegramLogicalTurnSleepPort> = time;
    let logical = logical_runtime(clock, sleeper, Duration::from_secs(5), 4, 4, 4);
    let (port, started) = TestExecutionPort::new();
    let gate = port.gate("update:blocking-command");
    let runtime = runtime(port, logical, 1, 4, 1);
    let mut command = update(1, 1, "blocking-command");
    command["message"]["text"] = json!("/queue");
    let first = runtime.submit_update(command).unwrap();
    assert_eq!(receive(&started), "update:blocking-command");

    let rejected = runtime
        .submit_update(update(2, 2, "buffered-secret"))
        .unwrap_err();
    assert_eq!(
        rejected.kind(),
        TelegramSubmissionExecutionErrorKind::InflightLimit
    );
    let snapshot = runtime.snapshot_json();
    assert_eq!(snapshot["logical_pending_update_count"], 0);
    assert_eq!(snapshot["dispatch_inflight_count"], 1);
    assert!(!snapshot.to_string().contains("buffered-secret"));

    gate.release();
    first.wait(Some(TEST_TIMEOUT)).unwrap();
}

#[test]
fn executor_errors_and_panics_are_sanitized_and_counted() {
    let (port, _started) = TestExecutionPort::new();
    let runtime = runtime(port, disabled_logical_runtime(), 2, 4, 8);
    let mut failing = update(1, 1, "payload-secret");
    failing["mode"] = json!("fail");
    let mut panicking = update(2, 2, "panic-secret");
    panicking["mode"] = json!("panic");

    let executor_error = runtime
        .submit_update(failing)
        .unwrap()
        .wait(Some(TEST_TIMEOUT))
        .unwrap_err();
    assert_eq!(
        executor_error.kind(),
        TelegramSubmissionExecutionErrorKind::Executor
    );
    let panic_error = runtime
        .submit_update(panicking)
        .unwrap()
        .wait(Some(TEST_TIMEOUT))
        .unwrap_err();
    assert_eq!(
        panic_error.kind(),
        TelegramSubmissionExecutionErrorKind::Panic
    );
    let rendered = runtime.snapshot_json().to_string();
    for secret in [
        "payload-secret",
        "panic-secret",
        "downstream-secret-detail",
        "downstream-secret-panic",
        "chat-1",
    ] {
        assert!(!rendered.contains(secret));
        assert!(!executor_error.to_string().contains(secret));
        assert!(!panic_error.to_string().contains(secret));
    }
    assert!(
        runtime.snapshot_json()["dispatch_failed_count"]
            .as_u64()
            .unwrap()
            >= 2
    );
    assert_eq!(runtime.snapshot_json()["dispatch_panicked_count"], 1);
}

#[test]
fn idle_wait_short_circuits_live_replies_and_surfaces_sanitized_port_failure() {
    let (port, started) = TestExecutionPort::new();
    let gate = port.gate("update:blocking");
    let runtime = runtime(port.clone(), disabled_logical_runtime(), 1, 4, 8);
    let future = runtime.submit_update(update(1, 1, "blocking")).unwrap();
    assert_eq!(receive(&started), "update:blocking");

    assert!(!runtime.wait_for_idle(Some(Duration::ZERO)).unwrap());
    assert_eq!(port.live_reply_calls.load(Ordering::Acquire), 0);
    gate.release();
    future.wait(Some(TEST_TIMEOUT)).unwrap();

    port.live_reply_idle.store(false, Ordering::Release);
    assert!(!runtime.wait_for_idle(Some(TEST_TIMEOUT)).unwrap());
    assert_eq!(port.live_reply_calls.load(Ordering::Acquire), 1);
    port.live_reply_error.store(true, Ordering::Release);
    let error = runtime.wait_for_idle(Some(TEST_TIMEOUT)).unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramSubmissionExecutionErrorKind::LiveReply
    );
    assert!(!error.to_string().contains("live-reply-secret-detail"));
}

#[test]
fn malformed_planner_output_fails_closed_without_leaking_it() {
    let (port, _) = TestExecutionPort::new();
    let execution: Arc<dyn TelegramSubmissionExecutionPort> = port;
    let planner: Arc<dyn SubmissionExecutionPlanningPort> = Arc::new(BrokenPlanner);
    let runtime = TelegramSubmissionRuntime::with_planning_port(
        execution,
        disabled_logical_runtime(),
        &admission_plan(8),
        1,
        4,
        planner,
    )
    .unwrap();

    let error = runtime
        .submit_update(update(1, 1, "planner-secret"))
        .unwrap_err();
    assert_eq!(
        error.kind(),
        TelegramSubmissionExecutionErrorKind::PlannerContract
    );
    assert!(!error.to_string().contains("planner-secret-detail"));
}

#[test]
fn snapshot_is_count_only_and_reply_arguments_never_escape() {
    let (port, started) = TestExecutionPort::new();
    let runtime = runtime(port, disabled_logical_runtime(), 1, 4, 8);
    let reply = runtime
        .submit_reply_serialized(
            "secret-chat-key",
            "send_document",
            vec![json!({"token": "argument-secret"})],
        )
        .unwrap();
    assert_eq!(receive(&started), "reply:send_document");
    assert_eq!(reply.wait(Some(TEST_TIMEOUT)).unwrap()["arg_count"], 1);

    let snapshot = runtime.snapshot_json();
    assert_eq!(snapshot["execution_contract"], EXECUTION_CONTRACT);
    assert_eq!(snapshot["migration_stage"], EXECUTION_MIGRATION_STAGE);
    assert_eq!(snapshot["submitted_reply_count"], 1);
    assert_eq!(snapshot["reply_execution_count"], 1);
    assert_eq!(snapshot["python_submission_allowed"], false);
    let rendered = snapshot.to_string();
    for secret in [
        "secret-chat-key",
        "send_document",
        "argument-secret",
        "token",
    ] {
        assert!(!rendered.contains(secret));
    }
}
