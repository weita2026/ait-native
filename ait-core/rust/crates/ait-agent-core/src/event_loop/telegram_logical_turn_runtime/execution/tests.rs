use super::*;
use ait_core::json_support::json;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

struct VirtualTime {
    now: Mutex<f64>,
    sleeps: Mutex<Vec<Duration>>,
    fail_clock: bool,
    fail_sleep: bool,
}

impl VirtualTime {
    fn new(now: f64) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            sleeps: Mutex::new(Vec::new()),
            fail_clock: false,
            fail_sleep: false,
        })
    }

    fn failing_clock(now: f64) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            sleeps: Mutex::new(Vec::new()),
            fail_clock: true,
            fail_sleep: false,
        })
    }

    fn failing_sleeper(now: f64) -> Arc<Self> {
        Arc::new(Self {
            now: Mutex::new(now),
            sleeps: Mutex::new(Vec::new()),
            fail_clock: false,
            fail_sleep: true,
        })
    }

    fn set(&self, value: f64) {
        *self.now.lock().unwrap() = value;
    }

    fn sleeps(&self) -> Vec<Duration> {
        self.sleeps.lock().unwrap().clone()
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
        if self.fail_sleep {
            return Err("sleeper-secret-detail".to_string());
        }
        self.sleeps.lock().unwrap().push(duration);
        *self.now.lock().unwrap() += duration.as_secs_f64();
        Ok(())
    }
}

struct BrokenPlanningPort;

impl ExecutionPlanningPort for BrokenPlanningPort {
    fn logical_turn(&self, _request: &JsonValue) -> Result<JsonValue, String> {
        Err("planner-secret-detail".to_string())
    }

    fn update_dispatch(&self, request: &JsonValue) -> Result<JsonValue, String> {
        NativeExecutionPlanningPort.update_dispatch(request)
    }

    fn turn_input(&self, request: &JsonValue) -> Result<JsonValue, String> {
        NativeExecutionPlanningPort.turn_input(request)
    }

    fn workflow_query(&self, request: &JsonValue) -> Result<JsonValue, String> {
        NativeExecutionPlanningPort.workflow_query(request)
    }
}

fn runtime(
    time: Arc<VirtualTime>,
    merge_window: Duration,
    max_messages: usize,
    max_pending_chats: usize,
    max_pending_per_chat: usize,
) -> TelegramLogicalTurnRuntime {
    let clock: Arc<dyn TelegramLogicalTurnClockPort> = time.clone();
    let sleeper: Arc<dyn TelegramLogicalTurnSleepPort> = time;
    TelegramLogicalTurnRuntime::with_ports(
        "aitbot",
        merge_window,
        max_messages,
        Duration::from_millis(200),
        max_pending_chats,
        max_pending_per_chat,
        clock,
        sleeper,
    )
    .unwrap()
}

fn text_update(
    update_id: i64,
    message_id: i64,
    chat_id: JsonValue,
    user_id: i64,
    username: &str,
    text: &str,
) -> JsonValue {
    json!({
        "update_id": update_id,
        "message": {
            "message_id": message_id,
            "text": text,
            "chat": {"id": chat_id, "type": "private"},
            "from": {"id": user_id, "username": username},
        }
    })
}

#[test]
fn disabled_runtime_and_non_text_updates_do_not_buffer() {
    let time = VirtualTime::new(10.0);
    let disabled = runtime(time.clone(), Duration::ZERO, 4, 4, 4);
    let update = text_update(1, 1, json!(123), 7, "wei", "hello");
    assert!(!disabled.merge_enabled());
    assert_eq!(
        disabled.buffer_update(&update, "fallback-1").unwrap(),
        TelegramLogicalTurnBufferOutcome::Disabled
    );
    assert_eq!(
        disabled.claim_update_once(&update, "fallback-1").unwrap(),
        TelegramLogicalTurnStep::Disabled
    );

    let enabled = runtime(time, Duration::from_secs(1), 4, 4, 4);
    let non_text = json!({"update_id": 2, "message": {"chat": {"id": 123}}});
    assert_eq!(
        enabled.buffer_update(&non_text, "fallback-2").unwrap(),
        TelegramLogicalTurnBufferOutcome::NotCandidate
    );
    assert_eq!(enabled.snapshot_json()["pending_update_count"], 0);
}

#[test]
fn duplicate_suppression_and_max_message_flush_preserve_normalized_order() {
    let time = VirtualTime::new(10.0);
    let runtime = runtime(time.clone(), Duration::from_secs(10), 2, 4, 4);
    let first = text_update(1, 10, json!(123), 7, "wei", "@aitbot,  hello\tworld");
    let second = text_update(2, 11, json!(123), 7, "wei", "one more detail");

    assert_eq!(
        runtime.buffer_update(&first, "fallback-1").unwrap(),
        TelegramLogicalTurnBufferOutcome::Buffered
    );
    assert_eq!(
        runtime.buffer_update(&first, "fallback-1").unwrap(),
        TelegramLogicalTurnBufferOutcome::Duplicate
    );
    time.set(10.1);
    assert_eq!(
        runtime.buffer_update(&second, "fallback-2").unwrap(),
        TelegramLogicalTurnBufferOutcome::Buffered
    );

    let TelegramLogicalTurnStep::LogicalTurn(turn) =
        runtime.claim_update_once(&first, "fallback-1").unwrap()
    else {
        panic!("expected logical turn");
    };
    assert_eq!(turn.update, first);
    assert_eq!(turn.text, "hello world\n\none more detail");
    assert_eq!(turn.actor_identity, "telegram:7:@wei");
    assert_eq!(turn.telegram_message_id, Some(11));
    assert_eq!(turn.telegram_message_ids, [10, 11]);
    let snapshot = runtime.snapshot_json();
    assert_eq!(snapshot["pending_update_count"], 0);
    assert_eq!(snapshot["duplicate_count"], 1);
    assert_eq!(snapshot["consumed_count"], 2);
}

#[test]
fn buffered_update_discard_is_exact_idempotent_and_counted() {
    let time = VirtualTime::new(10.0);
    let runtime = runtime(time, Duration::from_secs(10), 4, 4, 4);
    let first = text_update(1, 10, json!(123), 7, "wei", "first secret");
    let second = text_update(2, 11, json!(123), 7, "wei", "second secret");
    runtime.buffer_update(&first, "fallback-1").unwrap();
    runtime.buffer_update(&second, "fallback-2").unwrap();

    assert!(runtime
        .discard_buffered_update(&second, "fallback-2")
        .unwrap());
    assert!(!runtime
        .discard_buffered_update(&second, "fallback-2")
        .unwrap());
    let snapshot = runtime.snapshot_json();
    assert_eq!(snapshot["pending_update_count"], 1);
    assert_eq!(snapshot["discarded_count"], 1);
    let rendered = snapshot.to_string();
    assert!(!rendered.contains("first secret"));
    assert!(!rendered.contains("second secret"));
}

#[test]
fn commands_and_workflow_queries_bypass_merge_and_clean_state() {
    let time = VirtualTime::new(20.0);
    let runtime = runtime(time, Duration::from_secs(1), 4, 4, 4);
    for (id, text) in [(1, "/status"), (2, "queue")] {
        let update = text_update(id, id, json!(123), 7, "wei", text);
        assert_eq!(
            runtime
                .buffer_update(&update, &format!("fallback-{id}"))
                .unwrap(),
            TelegramLogicalTurnBufferOutcome::Buffered
        );
        assert_eq!(
            runtime
                .claim_update_once(&update, &format!("fallback-{id}"))
                .unwrap(),
            TelegramLogicalTurnStep::PassThrough
        );
    }
    assert_eq!(runtime.snapshot_json()["pending_update_count"], 0);
    assert_eq!(runtime.snapshot_json()["pass_through_count"], 2);
}

#[test]
fn actor_boundary_flushes_first_turn_without_consuming_second() {
    let time = VirtualTime::new(30.0);
    let runtime = runtime(time, Duration::from_secs(5), 4, 4, 4);
    let first = text_update(1, 10, json!(123), 7, "wei", "first");
    let second = text_update(2, 11, json!(123), 8, "other", "second");
    runtime.buffer_update(&first, "fallback-1").unwrap();
    runtime.buffer_update(&second, "fallback-2").unwrap();

    let TelegramLogicalTurnStep::LogicalTurn(turn) =
        runtime.claim_update_once(&first, "fallback-1").unwrap()
    else {
        panic!("expected boundary flush");
    };
    assert_eq!(turn.text, "first");
    assert_eq!(turn.telegram_message_ids, [10]);
    assert_eq!(runtime.snapshot_json()["pending_update_count"], 1);
}

#[test]
fn non_mergeable_boundary_flushes_prior_text_and_leaves_command() {
    let time = VirtualTime::new(40.0);
    let runtime = runtime(time, Duration::from_secs(5), 4, 4, 4);
    let first = text_update(1, 10, json!(123), 7, "wei", "first");
    let command = text_update(2, 11, json!(123), 7, "wei", "/status");
    runtime.buffer_update(&first, "fallback-1").unwrap();
    runtime.buffer_update(&command, "fallback-2").unwrap();

    let TelegramLogicalTurnStep::LogicalTurn(turn) =
        runtime.claim_update_once(&first, "fallback-1").unwrap()
    else {
        panic!("expected boundary flush");
    };
    assert_eq!(turn.text, "first");
    assert_eq!(runtime.snapshot_json()["pending_update_count"], 1);
    assert_eq!(
        runtime.claim_update_once(&command, "fallback-2").unwrap(),
        TelegramLogicalTurnStep::PassThrough
    );
}

#[test]
fn single_step_claim_returns_wait_without_sleeping() {
    let time = VirtualTime::new(50.0);
    let runtime = runtime(time.clone(), Duration::from_millis(500), 4, 4, 4);
    let update = text_update(1, 10, json!(123), 7, "wei", "wait for more");
    runtime.buffer_update(&update, "fallback-1").unwrap();

    assert_eq!(
        runtime.claim_update_once(&update, "fallback-1").unwrap(),
        TelegramLogicalTurnStep::Wait(Duration::from_millis(200))
    );
    assert!(time.sleeps().is_empty());
    assert_eq!(runtime.snapshot_json()["pending_update_count"], 1);
}

#[test]
fn complete_claim_loop_uses_virtual_time_and_flushes_quiet_window() {
    let time = VirtualTime::new(60.0);
    let runtime = runtime(time.clone(), Duration::from_millis(500), 4, 4, 4);
    let update = text_update(1, 10, json!(123), 7, "wei", "quiet turn");
    runtime.buffer_update(&update, "fallback-1").unwrap();

    let TelegramLogicalTurnStep::LogicalTurn(turn) =
        runtime.claim_update(&update, "fallback-1").unwrap()
    else {
        panic!("expected virtual-time flush");
    };
    assert_eq!(turn.text, "quiet turn");
    let slept = time.sleeps().iter().map(Duration::as_secs_f64).sum::<f64>();
    assert!((slept - 0.5).abs() < 1e-9);
    assert_eq!(runtime.snapshot_json()["pending_update_count"], 0);
}

#[test]
fn consumed_followup_claim_is_skipped() {
    let time = VirtualTime::new(70.0);
    let runtime = runtime(time, Duration::from_secs(5), 2, 4, 4);
    let first = text_update(1, 10, json!(123), 7, "wei", "first");
    let second = text_update(2, 11, json!(123), 7, "wei", "second");
    runtime.buffer_update(&first, "fallback-1").unwrap();
    runtime.buffer_update(&second, "fallback-2").unwrap();
    assert!(matches!(
        runtime.claim_update_once(&first, "fallback-1").unwrap(),
        TelegramLogicalTurnStep::LogicalTurn(_)
    ));
    assert_eq!(
        runtime.claim_update_once(&second, "fallback-2").unwrap(),
        TelegramLogicalTurnStep::Skip
    );
    assert_eq!(runtime.snapshot_json()["skipped_count"], 1);
}

#[test]
fn bounded_buffer_reports_per_chat_and_chat_capacity() {
    let first = text_update(1, 10, json!(123), 7, "wei", "first");
    let second = text_update(2, 11, json!(123), 7, "wei", "second");
    let other_chat = text_update(3, 12, json!(456), 7, "wei", "other");

    let per_chat = runtime(VirtualTime::new(80.0), Duration::from_secs(1), 4, 4, 1);
    per_chat.buffer_update(&first, "fallback-1").unwrap();
    let error = per_chat.buffer_update(&second, "fallback-2").unwrap_err();
    assert_eq!(error.kind(), TelegramLogicalTurnErrorKind::PerChatCapacity);

    let chats = runtime(VirtualTime::new(80.0), Duration::from_secs(1), 4, 1, 4);
    chats.buffer_update(&first, "fallback-1").unwrap();
    let error = chats.buffer_update(&other_chat, "fallback-3").unwrap_err();
    assert_eq!(error.kind(), TelegramLogicalTurnErrorKind::ChatCapacity);
}

#[test]
fn concurrent_duplicate_admission_accepts_exactly_one_update() {
    for _round in 0..25 {
        let runtime = Arc::new(runtime(
            VirtualTime::new(90.0),
            Duration::from_secs(1),
            4,
            4,
            4,
        ));
        let update = Arc::new(text_update(1, 10, json!(123), 7, "wei", "once"));
        let barrier = Arc::new(Barrier::new(8));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let runtime = Arc::clone(&runtime);
            let update = Arc::clone(&update);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                barrier.wait();
                runtime.buffer_update(&update, "fallback-1").unwrap()
            }));
        }
        let outcomes = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == TelegramLogicalTurnBufferOutcome::Buffered)
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == TelegramLogicalTurnBufferOutcome::Duplicate)
                .count(),
            7
        );
        assert_eq!(runtime.snapshot_json()["pending_update_count"], 1);
    }
}

#[test]
fn invalid_configuration_updates_and_fallback_keys_fail_closed() {
    let configuration =
        TelegramLogicalTurnRuntime::new("aitbot", Duration::from_secs(1), 4, Duration::ZERO, 4, 4)
            .err()
            .unwrap();
    assert_eq!(
        configuration.kind(),
        TelegramLogicalTurnErrorKind::Configuration
    );
    let runtime = runtime(VirtualTime::new(100.0), Duration::from_secs(1), 4, 4, 4);
    assert_eq!(
        runtime
            .buffer_update(&json!("bad"), "fallback")
            .unwrap_err()
            .kind(),
        TelegramLogicalTurnErrorKind::InvalidUpdate
    );
    assert_eq!(
        runtime
            .buffer_update(&text_update(1, 1, json!(123), 7, "wei", "hello"), "\n")
            .unwrap_err()
            .kind(),
        TelegramLogicalTurnErrorKind::InvalidFallbackKey
    );
}

#[test]
fn planner_clock_and_sleeper_failures_are_sanitized() {
    let broken = TelegramLogicalTurnRuntime::with_planning_port(
        "aitbot",
        Duration::from_secs(1),
        4,
        Duration::from_millis(200),
        4,
        4,
        VirtualTime::new(110.0),
        VirtualTime::new(110.0),
        Arc::new(BrokenPlanningPort),
    )
    .err()
    .unwrap();
    assert_eq!(broken.kind(), TelegramLogicalTurnErrorKind::PlannerContract);
    assert!(!broken.to_string().contains("planner-secret-detail"));

    let clock = runtime(
        VirtualTime::failing_clock(110.0),
        Duration::from_secs(1),
        4,
        4,
        4,
    );
    let update = text_update(1, 1, json!(123), 7, "wei", "hello");
    let error = clock.buffer_update(&update, "fallback").unwrap_err();
    assert_eq!(error.kind(), TelegramLogicalTurnErrorKind::Clock);
    assert!(!error.to_string().contains("clock-secret-detail"));

    let sleeper_time = VirtualTime::failing_sleeper(110.0);
    let sleeper = runtime(sleeper_time, Duration::from_secs(1), 4, 4, 4);
    sleeper.buffer_update(&update, "fallback").unwrap();
    let error = sleeper.claim_update(&update, "fallback").unwrap_err();
    assert_eq!(error.kind(), TelegramLogicalTurnErrorKind::Sleeper);
    assert!(!error.to_string().contains("sleeper-secret-detail"));
}

#[test]
fn snapshots_expose_counts_without_pending_identity_or_payload() {
    let runtime = runtime(VirtualTime::new(120.0), Duration::from_secs(1), 4, 4, 4);
    let update = text_update(
        0,
        0,
        json!("chat-secret"),
        7,
        "actor-secret",
        "payload-secret",
    );
    runtime.buffer_update(&update, "fallback-secret").unwrap();
    let snapshot = runtime.snapshot_json();
    let rendered = snapshot.to_string();
    for secret in [
        "chat-secret",
        "actor-secret",
        "payload-secret",
        "fallback-secret",
        "planner-secret-detail",
    ] {
        assert!(!rendered.contains(secret));
    }
    assert_eq!(snapshot["execution_contract"], EXECUTION_CONTRACT);
    assert_eq!(snapshot["migration_stage"], EXECUTION_MIGRATION_STAGE);
    assert_eq!(snapshot["pending_chat_count"], 1);
    assert_eq!(snapshot["pending_update_count"], 1);
    assert_eq!(snapshot["python_logical_turn_allowed"], false);
    assert_eq!(snapshot["python_buffer_allowed"], false);
    assert_eq!(snapshot["python_sleep_allowed"], false);
}
