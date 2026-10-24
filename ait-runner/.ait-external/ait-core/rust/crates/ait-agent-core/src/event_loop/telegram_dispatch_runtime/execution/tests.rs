use super::*;
use ait_core::json_support::json;
use std::collections::HashMap;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

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

struct ScriptedExecutor {
    started_sender: mpsc::Sender<String>,
    started: Mutex<Vec<String>>,
    completed: Mutex<Vec<String>>,
    gates: Mutex<HashMap<String, Arc<Gate>>>,
}

impl ScriptedExecutor {
    fn new() -> (Arc<Self>, mpsc::Receiver<String>) {
        let (started_sender, started_receiver) = mpsc::channel();
        (
            Arc::new(Self {
                started_sender,
                started: Mutex::new(Vec::new()),
                completed: Mutex::new(Vec::new()),
                gates: Mutex::new(HashMap::new()),
            }),
            started_receiver,
        )
    }

    fn gate(&self, job: &str) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        self.gates
            .lock()
            .unwrap()
            .insert(job.to_string(), Arc::clone(&gate));
        gate
    }

    fn started(&self) -> Vec<String> {
        self.started.lock().unwrap().clone()
    }

    fn completed(&self) -> Vec<String> {
        self.completed.lock().unwrap().clone()
    }
}

impl TelegramKeyedDispatchJobExecutor for ScriptedExecutor {
    fn execute(
        &self,
        dispatcher_kind: &str,
        _queue_key: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String> {
        let job = request
            .get("job")
            .and_then(JsonValue::as_str)
            .unwrap_or("unnamed")
            .to_string();
        let marker = format!("{dispatcher_kind}:{job}");
        self.started.lock().unwrap().push(marker.clone());
        let _ = self.started_sender.send(marker);
        let gate = self.gates.lock().unwrap().get(&job).cloned();
        if let Some(gate) = gate {
            gate.wait();
        }
        if request.get("mode").and_then(JsonValue::as_str) == Some("fail") {
            return Err("executor-secret-detail".to_string());
        }
        if request.get("mode").and_then(JsonValue::as_str) == Some("panic") {
            panic!("scripted executor panic");
        }
        self.completed.lock().unwrap().push(job.clone());
        Ok(json!({
            "job": job,
            "dispatcher_kind": dispatcher_kind,
            "thread_name": thread::current().name().unwrap_or_default(),
        }))
    }
}

fn admission_plan(inflight_limit: usize) -> JsonValue {
    json!({
        "backend": "linux_epoll",
        "worker_leases": [{"shard_index": 2}],
        "shard_admissions": [{"shard_index": 2, "inflight_limit": inflight_limit}],
    })
}

fn runtime(
    executor: Arc<ScriptedExecutor>,
    worker_count: usize,
    per_key_queue_capacity: usize,
    inflight_limit: usize,
) -> TelegramKeyedDispatchRuntime {
    TelegramKeyedDispatchRuntime::new(
        executor,
        &admission_plan(inflight_limit),
        worker_count,
        per_key_queue_capacity,
    )
    .unwrap()
}

fn receive_started(receiver: &mpsc::Receiver<String>) -> String {
    receiver.recv_timeout(TEST_TIMEOUT).unwrap()
}

#[test]
fn keyed_dispatch_runs_same_key_in_fifo_order() {
    let (executor, started) = ScriptedExecutor::new();
    let first_gate = executor.gate("first");
    let runtime = runtime(Arc::clone(&executor), 2, 4, 8);

    let first = runtime
        .submit_dispatch("chat-1", json!({"job": "first"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:first");
    let second = runtime
        .submit_dispatch("chat-1", json!({"job": "second"}))
        .unwrap();
    assert!(matches!(started.try_recv(), Err(mpsc::TryRecvError::Empty)));

    first_gate.release();
    assert_eq!(first.wait(Some(TEST_TIMEOUT)).unwrap()["job"], "first");
    assert_eq!(receive_started(&started), "dispatch:second");
    assert_eq!(second.wait(Some(TEST_TIMEOUT)).unwrap()["job"], "second");
    assert_eq!(executor.started(), ["dispatch:first", "dispatch:second"]);
    assert_eq!(executor.completed(), ["first", "second"]);
    assert!(runtime.wait_for_idle(Some(TEST_TIMEOUT)));
}

#[test]
fn keyed_dispatch_allows_unrelated_keys_to_progress_concurrently() {
    let (executor, started) = ScriptedExecutor::new();
    let first_gate = executor.gate("blocked");
    let runtime = runtime(executor, 2, 4, 8);

    let blocked = runtime
        .submit_dispatch("chat-1", json!({"job": "blocked"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:blocked");
    let unrelated = runtime
        .submit_dispatch("chat-2", json!({"job": "unrelated"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:unrelated");
    assert_eq!(
        unrelated.wait(Some(TEST_TIMEOUT)).unwrap()["job"],
        "unrelated"
    );

    first_gate.release();
    assert_eq!(blocked.wait(Some(TEST_TIMEOUT)).unwrap()["job"], "blocked");
}

#[test]
fn dispatch_and_reply_use_separate_key_namespaces() {
    let (executor, started) = ScriptedExecutor::new();
    let dispatch_gate = executor.gate("dispatch-blocked");
    let runtime = runtime(executor, 2, 4, 8);

    let dispatch = runtime
        .submit_dispatch("chat-shared", json!({"job": "dispatch-blocked"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:dispatch-blocked");
    let reply = runtime
        .submit_reply("chat-shared", json!({"job": "reply-ready"}))
        .unwrap();
    assert_eq!(receive_started(&started), "reply:reply-ready");
    assert_eq!(
        reply.wait(Some(TEST_TIMEOUT)).unwrap()["job"],
        "reply-ready"
    );

    dispatch_gate.release();
    assert_eq!(
        dispatch.wait(Some(TEST_TIMEOUT)).unwrap()["job"],
        "dispatch-blocked"
    );
}

#[test]
fn worker_pool_names_are_fixed_and_do_not_include_queue_keys() {
    let (executor, _started) = ScriptedExecutor::new();
    let runtime = runtime(executor, 1, 4, 8);

    let result = runtime
        .submit_dispatch("queue-secret", json!({"job": "named"}))
        .unwrap()
        .wait(Some(TEST_TIMEOUT))
        .unwrap();
    assert_eq!(
        result["thread_name"],
        "ait-telegram-dispatch-linux_epoll-s2-w0"
    );
    assert!(!result["thread_name"]
        .as_str()
        .unwrap()
        .contains("queue-secret"));
}

#[test]
fn keyed_dispatch_enforces_global_inflight_limit() {
    let (executor, started) = ScriptedExecutor::new();
    let gate = executor.gate("held");
    let runtime = runtime(executor, 2, 4, 1);

    let held = runtime
        .submit_dispatch("chat-1", json!({"job": "held"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:held");
    let error = runtime
        .submit_dispatch("chat-2", json!({"job": "rejected"}))
        .err()
        .unwrap();
    assert_eq!(error.kind(), TelegramKeyedDispatchErrorKind::InflightLimit);

    gate.release();
    held.wait(Some(TEST_TIMEOUT)).unwrap();
    assert!(matches!(started.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn keyed_dispatch_enforces_per_key_queued_backlog_limit() {
    let (executor, started) = ScriptedExecutor::new();
    let gate = executor.gate("running");
    let runtime = runtime(executor, 1, 1, 4);

    let running = runtime
        .submit_dispatch("chat-1", json!({"job": "running"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:running");
    let queued = runtime
        .submit_dispatch("chat-1", json!({"job": "queued"}))
        .unwrap();
    let error = runtime
        .submit_dispatch("chat-1", json!({"job": "overflow"}))
        .err()
        .unwrap();
    assert_eq!(error.kind(), TelegramKeyedDispatchErrorKind::QueueCapacity);

    gate.release();
    running.wait(Some(TEST_TIMEOUT)).unwrap();
    assert_eq!(receive_started(&started), "dispatch:queued");
    queued.wait(Some(TEST_TIMEOUT)).unwrap();
    assert!(matches!(started.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn stop_rejects_new_work_and_drains_accepted_queue() {
    let (executor, started) = ScriptedExecutor::new();
    let gate = executor.gate("running");
    let runtime = runtime(executor, 1, 4, 8);

    let running = runtime
        .submit_dispatch("chat-1", json!({"job": "running"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:running");
    let queued = runtime
        .submit_dispatch("chat-1", json!({"job": "queued"}))
        .unwrap();
    runtime.request_stop().unwrap();
    let error = runtime
        .submit_reply("chat-2", json!({"job": "late"}))
        .err()
        .unwrap();
    assert_eq!(error.kind(), TelegramKeyedDispatchErrorKind::Stopped);

    gate.release();
    running.wait(Some(TEST_TIMEOUT)).unwrap();
    assert_eq!(receive_started(&started), "dispatch:queued");
    queued.wait(Some(TEST_TIMEOUT)).unwrap();
    assert!(runtime.wait_for_idle(Some(TEST_TIMEOUT)));
    let snapshot = runtime.snapshot_json();
    assert_eq!(snapshot["stopped"], true);
    assert_eq!(snapshot["completed_count"], 2);
}

#[test]
fn future_timeout_does_not_cancel_job_or_idle_tracking() {
    let (executor, started) = ScriptedExecutor::new();
    let gate = executor.gate("held");
    let runtime = runtime(executor, 1, 4, 8);

    let future = runtime
        .submit_dispatch("chat-1", json!({"job": "held"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:held");
    assert!(!runtime.wait_for_idle(Some(Duration::ZERO)));
    let error = future.wait(Some(Duration::ZERO)).unwrap_err();
    assert_eq!(error.kind(), TelegramKeyedDispatchErrorKind::Timeout);

    gate.release();
    assert!(runtime.wait_for_idle(Some(TEST_TIMEOUT)));
    assert_eq!(runtime.snapshot_json()["completed_count"], 1);
}

#[test]
fn executor_errors_and_panics_are_sanitized_and_worker_survives() {
    let (executor, _started) = ScriptedExecutor::new();
    let runtime = runtime(executor, 1, 4, 8);

    let failed = runtime
        .submit_dispatch("chat-1", json!({"job": "fail", "mode": "fail"}))
        .unwrap()
        .wait(Some(TEST_TIMEOUT))
        .unwrap_err();
    assert_eq!(failed.kind(), TelegramKeyedDispatchErrorKind::Executor);
    assert!(!failed.to_string().contains("executor-secret-detail"));

    let panicked = runtime
        .submit_dispatch("chat-1", json!({"job": "panic", "mode": "panic"}))
        .unwrap()
        .wait(Some(TEST_TIMEOUT))
        .unwrap_err();
    assert_eq!(panicked.kind(), TelegramKeyedDispatchErrorKind::Panic);

    let recovered = runtime
        .submit_dispatch("chat-1", json!({"job": "recovered"}))
        .unwrap()
        .wait(Some(TEST_TIMEOUT))
        .unwrap();
    assert_eq!(recovered["job"], "recovered");
    let snapshot = runtime.snapshot_json();
    assert_eq!(snapshot["failed_count"], 2);
    assert_eq!(snapshot["panicked_count"], 1);
}

#[test]
fn dropped_future_still_completes_and_releases_capacity() {
    let (executor, _started) = ScriptedExecutor::new();
    let runtime = runtime(executor, 1, 4, 1);

    drop(
        runtime
            .submit_dispatch("chat-1", json!({"job": "dropped"}))
            .unwrap(),
    );
    assert!(runtime.wait_for_idle(Some(TEST_TIMEOUT)));
    assert_eq!(runtime.snapshot_json()["completed_count"], 1);

    runtime
        .submit_dispatch("chat-2", json!({"job": "next"}))
        .unwrap()
        .wait(Some(TEST_TIMEOUT))
        .unwrap();
}

#[test]
fn invalid_configuration_dispatcher_and_queue_key_fail_closed() {
    let (executor, _started) = ScriptedExecutor::new();
    let worker_error =
        TelegramKeyedDispatchRuntime::new(executor.clone(), &admission_plan(8), 0, 4)
            .err()
            .unwrap();
    assert_eq!(
        worker_error.kind(),
        TelegramKeyedDispatchErrorKind::Configuration
    );
    let queue_error = TelegramKeyedDispatchRuntime::new(executor.clone(), &admission_plan(8), 1, 0)
        .err()
        .unwrap();
    assert_eq!(
        queue_error.kind(),
        TelegramKeyedDispatchErrorKind::Configuration
    );
    let backend_error = TelegramKeyedDispatchRuntime::new(
        executor.clone(),
        &json!({"backend": "secret_backend"}),
        1,
        4,
    )
    .err()
    .unwrap();
    assert_eq!(
        backend_error.kind(),
        TelegramKeyedDispatchErrorKind::Configuration
    );

    let runtime = runtime(executor, 1, 4, 8);
    assert_eq!(
        runtime
            .submit("unknown", "chat-1", json!({}))
            .err()
            .unwrap()
            .kind(),
        TelegramKeyedDispatchErrorKind::InvalidDispatcher
    );
    assert_eq!(
        runtime
            .submit_dispatch("\n", json!({}))
            .err()
            .unwrap()
            .kind(),
        TelegramKeyedDispatchErrorKind::InvalidQueueKey
    );
}

#[test]
fn snapshots_and_rejections_do_not_expose_keys_payloads_or_python_paths() {
    let (executor, started) = ScriptedExecutor::new();
    let gate = executor.gate("payload-secret");
    let runtime = runtime(executor, 1, 4, 1);
    let future = runtime
        .submit_dispatch("queue-secret", json!({"job": "payload-secret"}))
        .unwrap();
    assert_eq!(receive_started(&started), "dispatch:payload-secret");
    let error = runtime
        .submit_dispatch("another-secret", json!({"job": "rejected-secret"}))
        .err()
        .unwrap();
    let rendered_error = error.to_string();
    let rendered_snapshot = runtime.snapshot_json().to_string();
    for secret in [
        "queue-secret",
        "another-secret",
        "payload-secret",
        "rejected-secret",
        "executor-secret-detail",
    ] {
        assert!(!rendered_error.contains(secret));
        assert!(!rendered_snapshot.contains(secret));
    }
    assert_eq!(runtime.snapshot_json()["python_dispatch_allowed"], false);
    assert_eq!(runtime.snapshot_json()["python_executor_allowed"], false);
    assert_eq!(
        runtime.snapshot_json()["execution_contract"],
        EXECUTION_CONTRACT
    );
    assert_eq!(
        runtime.snapshot_json()["migration_stage"],
        EXECUTION_MIGRATION_STAGE
    );

    gate.release();
    future.wait(Some(TEST_TIMEOUT)).unwrap();
}
