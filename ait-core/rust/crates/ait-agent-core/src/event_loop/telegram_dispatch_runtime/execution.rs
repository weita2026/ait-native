use super::agent_telegram_dispatch_runtime_plan_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{mpsc, Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const EXECUTION_CONTRACT: &str = "ait_agent_core.event_loop.TelegramKeyedDispatchExecution.v1";
const PLANNING_CONTRACT: &str = "ait_agent_core.event_loop.TelegramDispatchRuntime.v1";
const PLANNING_MIGRATION_STAGE: &str = "rust_agent_telegram_dispatch_runtime";
const EXECUTION_MIGRATION_STAGE: &str = "rust_agent_telegram_keyed_dispatch_execution";
const MAX_WORKER_COUNT: usize = 64;
const MAX_PER_KEY_QUEUE_CAPACITY: usize = 1_024;
const MAX_INFLIGHT_LIMIT: usize = 65_536;
const MAX_QUEUE_KEY_LENGTH: usize = 128;

pub trait TelegramKeyedDispatchJobExecutor: Send + Sync + 'static {
    fn execute(
        &self,
        dispatcher_kind: &str,
        queue_key: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramKeyedDispatchErrorKind {
    Configuration,
    InvalidDispatcher,
    InvalidQueueKey,
    Stopped,
    InflightLimit,
    QueueCapacity,
    PlannerContract,
    WorkerUnavailable,
    Executor,
    Panic,
    Timeout,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramKeyedDispatchError {
    kind: TelegramKeyedDispatchErrorKind,
}

impl TelegramKeyedDispatchError {
    pub fn kind(&self) -> TelegramKeyedDispatchErrorKind {
        self.kind
    }

    fn new(kind: TelegramKeyedDispatchErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for TelegramKeyedDispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            TelegramKeyedDispatchErrorKind::Configuration => {
                "Telegram keyed dispatch configuration is invalid."
            }
            TelegramKeyedDispatchErrorKind::InvalidDispatcher => {
                "Telegram keyed dispatch kind is invalid."
            }
            TelegramKeyedDispatchErrorKind::InvalidQueueKey => {
                "Telegram keyed dispatch queue key is invalid."
            }
            TelegramKeyedDispatchErrorKind::Stopped => {
                "Telegram keyed dispatch runtime is stopped."
            }
            TelegramKeyedDispatchErrorKind::InflightLimit => {
                "Telegram keyed dispatch in-flight capacity is exhausted."
            }
            TelegramKeyedDispatchErrorKind::QueueCapacity => {
                "Telegram keyed dispatch queue capacity is exhausted."
            }
            TelegramKeyedDispatchErrorKind::PlannerContract => {
                "Telegram keyed dispatch planner contract is invalid."
            }
            TelegramKeyedDispatchErrorKind::WorkerUnavailable => {
                "Telegram keyed dispatch worker is unavailable."
            }
            TelegramKeyedDispatchErrorKind::Executor => {
                "Telegram keyed dispatch job execution failed."
            }
            TelegramKeyedDispatchErrorKind::Panic => "Telegram keyed dispatch job panicked.",
            TelegramKeyedDispatchErrorKind::Timeout => {
                "Telegram keyed dispatch result wait timed out."
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TelegramKeyedDispatchError {}

pub struct TelegramKeyedDispatchFuture {
    receiver: mpsc::Receiver<JobOutcome>,
}

impl TelegramKeyedDispatchFuture {
    pub fn wait(self, timeout: Option<Duration>) -> Result<JsonValue, TelegramKeyedDispatchError> {
        let outcome = match timeout {
            Some(duration) => match self.receiver.recv_timeout(duration) {
                Ok(outcome) => outcome,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    return Err(TelegramKeyedDispatchError::new(
                        TelegramKeyedDispatchErrorKind::Timeout,
                    ));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(TelegramKeyedDispatchError::new(
                        TelegramKeyedDispatchErrorKind::WorkerUnavailable,
                    ));
                }
            },
            None => self.receiver.recv().map_err(|_| {
                TelegramKeyedDispatchError::new(TelegramKeyedDispatchErrorKind::WorkerUnavailable)
            })?,
        };

        match outcome {
            JobOutcome::Success(value) => Ok(value),
            JobOutcome::Failure(kind) => Err(TelegramKeyedDispatchError::new(kind)),
        }
    }
}

pub struct TelegramKeyedDispatchRuntime {
    shared: Arc<SharedRuntime>,
    workers: Vec<JoinHandle<()>>,
}

impl TelegramKeyedDispatchRuntime {
    pub fn new(
        executor: Arc<dyn TelegramKeyedDispatchJobExecutor>,
        admission_plan: &JsonValue,
        worker_count: usize,
        per_key_queue_capacity: usize,
    ) -> Result<Self, TelegramKeyedDispatchError> {
        if !(1..=MAX_WORKER_COUNT).contains(&worker_count)
            || !(1..=MAX_PER_KEY_QUEUE_CAPACITY).contains(&per_key_queue_capacity)
        {
            return Err(TelegramKeyedDispatchError::new(
                TelegramKeyedDispatchErrorKind::Configuration,
            ));
        }

        let config = configure_runtime(admission_plan)?;
        let shared = Arc::new(SharedRuntime {
            state: Mutex::new(SchedulerState::default()),
            wake: Condvar::new(),
            executor,
            config,
            worker_count,
            per_key_queue_capacity,
        });
        let mut workers = Vec::with_capacity(worker_count);

        for worker_index in 0..worker_count {
            let worker_shared = Arc::clone(&shared);
            let thread_name = format!(
                "ait-telegram-dispatch-{}-s{}-w{}",
                shared.config.backend, shared.config.shard_index, worker_index
            );
            match thread::Builder::new()
                .name(thread_name)
                .spawn(move || worker_loop(worker_shared))
            {
                Ok(handle) => workers.push(handle),
                Err(_) => {
                    {
                        let mut state = lock_state(&shared);
                        state.stopped = true;
                    }
                    shared.wake.notify_all();
                    for handle in workers {
                        let _ = handle.join();
                    }
                    return Err(TelegramKeyedDispatchError::new(
                        TelegramKeyedDispatchErrorKind::WorkerUnavailable,
                    ));
                }
            }
        }

        Ok(Self { shared, workers })
    }

    pub fn submit_dispatch(
        &self,
        queue_key: &str,
        request: JsonValue,
    ) -> Result<TelegramKeyedDispatchFuture, TelegramKeyedDispatchError> {
        self.submit("dispatch", queue_key, request)
    }

    pub fn submit_reply(
        &self,
        queue_key: &str,
        request: JsonValue,
    ) -> Result<TelegramKeyedDispatchFuture, TelegramKeyedDispatchError> {
        self.submit("reply", queue_key, request)
    }

    pub fn submit(
        &self,
        dispatcher_kind: &str,
        queue_key: &str,
        request: JsonValue,
    ) -> Result<TelegramKeyedDispatchFuture, TelegramKeyedDispatchError> {
        let dispatcher_kind = DispatcherKind::parse(dispatcher_kind)?;
        let queue_key = normalize_queue_key(queue_key)?;
        let identity = QueueIdentity {
            dispatcher_kind,
            queue_key,
        };
        let mut state = lock_state(&self.shared);
        let has_executor =
            state.queues.contains_key(&identity) || state.running.contains(&identity);
        let stage = if dispatcher_kind == DispatcherKind::Reply {
            "submit_reply_serialized"
        } else {
            "submit"
        };
        let planner_request = json!({
            "stage": stage,
            "dispatcher_kind": dispatcher_kind.as_str(),
            "queue_key": identity.queue_key,
            "backend": self.shared.config.backend,
            "shard_index": self.shared.config.shard_index,
            "inflight_limit": self.shared.config.inflight_limit,
            "inflight_count": state.inflight,
            "stop_requested": state.stopped,
            "has_executor": has_executor,
        });
        let planned = match agent_telegram_dispatch_runtime_plan_json(&planner_request) {
            Ok(planned) => planned,
            Err(_) => {
                state.rejected = state.rejected.saturating_add(1);
                return Err(TelegramKeyedDispatchError::new(
                    TelegramKeyedDispatchErrorKind::PlannerContract,
                ));
            }
        };

        if let Err(error) = validate_submit_plan(
            &planned,
            stage,
            &identity,
            &self.shared.config,
            state.inflight,
            state.stopped,
            has_executor,
        ) {
            state.rejected = state.rejected.saturating_add(1);
            return Err(error);
        }
        if state.stopped {
            state.rejected = state.rejected.saturating_add(1);
            return Err(TelegramKeyedDispatchError::new(
                TelegramKeyedDispatchErrorKind::Stopped,
            ));
        }
        if state.inflight >= self.shared.config.inflight_limit {
            state.rejected = state.rejected.saturating_add(1);
            return Err(TelegramKeyedDispatchError::new(
                TelegramKeyedDispatchErrorKind::InflightLimit,
            ));
        }
        let queued_count = state.queues.get(&identity).map_or(0, VecDeque::len);
        if queued_count >= self.shared.per_key_queue_capacity {
            state.rejected = state.rejected.saturating_add(1);
            return Err(TelegramKeyedDispatchError::new(
                TelegramKeyedDispatchErrorKind::QueueCapacity,
            ));
        }

        let (sender, receiver) = mpsc::sync_channel(1);
        let should_mark_ready = !state.running.contains(&identity) && queued_count == 0;
        state
            .queues
            .entry(identity.clone())
            .or_default()
            .push_back(QueuedJob { request, sender });
        if should_mark_ready {
            state.ready.push_back(identity);
        }
        state.inflight = state.inflight.saturating_add(1);
        state.submitted = state.submitted.saturating_add(1);
        drop(state);
        self.shared.wake.notify_one();

        Ok(TelegramKeyedDispatchFuture { receiver })
    }

    pub fn wait_for_idle(&self, timeout: Option<Duration>) -> bool {
        let mut state = lock_state(&self.shared);
        if state.inflight == 0 {
            return true;
        }

        match timeout {
            None => {
                while state.inflight != 0 {
                    state = match self.shared.wake.wait(state) {
                        Ok(state) => state,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                }
                true
            }
            Some(duration) => {
                let deadline = Instant::now().checked_add(duration);
                loop {
                    let remaining = match deadline {
                        Some(deadline) => deadline.saturating_duration_since(Instant::now()),
                        None => duration,
                    };
                    if remaining.is_zero() {
                        return state.inflight == 0;
                    }
                    let (next_state, wait_result) =
                        match self.shared.wake.wait_timeout(state, remaining) {
                            Ok(result) => result,
                            Err(poisoned) => poisoned.into_inner(),
                        };
                    state = next_state;
                    if state.inflight == 0 {
                        return true;
                    }
                    if wait_result.timed_out() {
                        return false;
                    }
                }
            }
        }
    }

    pub fn request_stop(&self) -> Result<(), TelegramKeyedDispatchError> {
        let mut state = lock_state(&self.shared);
        let dispatch_queue_count = state
            .queues
            .keys()
            .filter(|identity| identity.dispatcher_kind == DispatcherKind::Dispatch)
            .count();
        let reply_queue_count = state
            .queues
            .keys()
            .filter(|identity| identity.dispatcher_kind == DispatcherKind::Reply)
            .count();
        let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
            "stage": "stop",
            "dispatch_queue_count": dispatch_queue_count,
            "reply_queue_count": reply_queue_count,
        }));
        state.stopped = true;
        let validation = match planned {
            Ok(planned) => validate_stop_plan(&planned, dispatch_queue_count, reply_queue_count),
            Err(_) => Err(TelegramKeyedDispatchError::new(
                TelegramKeyedDispatchErrorKind::PlannerContract,
            )),
        };
        drop(state);
        self.shared.wake.notify_all();
        validation
    }

    pub fn snapshot_json(&self) -> JsonValue {
        let state = lock_state(&self.shared);
        let dispatch_queue_count = state
            .queues
            .keys()
            .filter(|identity| identity.dispatcher_kind == DispatcherKind::Dispatch)
            .count();
        let reply_queue_count = state
            .queues
            .keys()
            .filter(|identity| identity.dispatcher_kind == DispatcherKind::Reply)
            .count();
        let queued_count = state.queues.values().map(VecDeque::len).sum::<usize>();

        json!({
            "execution_contract": EXECUTION_CONTRACT,
            "migration_stage": EXECUTION_MIGRATION_STAGE,
            "transport": "telegram",
            "backend": self.shared.config.backend,
            "shard_index": self.shared.config.shard_index,
            "worker_count": self.shared.worker_count,
            "inflight_limit": self.shared.config.inflight_limit,
            "per_key_queue_capacity": self.shared.per_key_queue_capacity,
            "stopped": state.stopped,
            "inflight_count": state.inflight,
            "queued_count": queued_count,
            "running_count": state.running.len(),
            "ready_queue_count": state.ready.len(),
            "dispatch_queue_count": dispatch_queue_count,
            "reply_queue_count": reply_queue_count,
            "submitted_count": state.submitted,
            "completed_count": state.completed,
            "failed_count": state.failed,
            "panicked_count": state.panicked,
            "rejected_count": state.rejected,
            "rust_keyed_dispatch_required": true,
            "python_dispatch_allowed": false,
            "python_executor_allowed": false,
        })
    }
}

impl Drop for TelegramKeyedDispatchRuntime {
    fn drop(&mut self) {
        let join_workers = {
            let mut state = lock_state(&self.shared);
            state.stopped = true;
            state.inflight == 0
        };
        self.shared.wake.notify_all();
        if join_workers {
            for handle in self.workers.drain(..) {
                let _ = handle.join();
            }
        } else {
            self.workers.clear();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum DispatcherKind {
    Dispatch,
    Reply,
}

impl DispatcherKind {
    fn parse(value: &str) -> Result<Self, TelegramKeyedDispatchError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "dispatch" => Ok(Self::Dispatch),
            "reply" => Ok(Self::Reply),
            _ => Err(TelegramKeyedDispatchError::new(
                TelegramKeyedDispatchErrorKind::InvalidDispatcher,
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Dispatch => "dispatch",
            Self::Reply => "reply",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct QueueIdentity {
    dispatcher_kind: DispatcherKind,
    queue_key: String,
}

struct QueuedJob {
    request: JsonValue,
    sender: mpsc::SyncSender<JobOutcome>,
}

enum JobOutcome {
    Success(JsonValue),
    Failure(TelegramKeyedDispatchErrorKind),
}

#[derive(Default)]
struct SchedulerState {
    stopped: bool,
    queues: HashMap<QueueIdentity, VecDeque<QueuedJob>>,
    ready: VecDeque<QueueIdentity>,
    running: HashSet<QueueIdentity>,
    inflight: usize,
    submitted: u64,
    completed: u64,
    failed: u64,
    panicked: u64,
    rejected: u64,
}

struct RuntimeConfig {
    backend: String,
    shard_index: usize,
    inflight_limit: usize,
}

struct SharedRuntime {
    state: Mutex<SchedulerState>,
    wake: Condvar,
    executor: Arc<dyn TelegramKeyedDispatchJobExecutor>,
    config: RuntimeConfig,
    worker_count: usize,
    per_key_queue_capacity: usize,
}

fn worker_loop(shared: Arc<SharedRuntime>) {
    loop {
        let (identity, job) = {
            let mut state = lock_state(&shared);
            loop {
                if let Some(identity) = state.ready.pop_front() {
                    if state.running.contains(&identity) {
                        continue;
                    }
                    let job = state
                        .queues
                        .get_mut(&identity)
                        .and_then(VecDeque::pop_front);
                    if let Some(job) = job {
                        state.running.insert(identity.clone());
                        break (identity, job);
                    }
                    state.queues.remove(&identity);
                    continue;
                }
                if state.stopped && state.inflight == 0 {
                    return;
                }
                state = match shared.wake.wait(state) {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
            }
        };

        let outcome = match catch_unwind(AssertUnwindSafe(|| {
            shared.executor.execute(
                identity.dispatcher_kind.as_str(),
                &identity.queue_key,
                &job.request,
            )
        })) {
            Ok(Ok(value)) => JobOutcome::Success(value),
            Ok(Err(_)) => JobOutcome::Failure(TelegramKeyedDispatchErrorKind::Executor),
            Err(_) => JobOutcome::Failure(TelegramKeyedDispatchErrorKind::Panic),
        };
        let outcome_kind = match &outcome {
            JobOutcome::Success(_) => None,
            JobOutcome::Failure(kind) => Some(*kind),
        };

        {
            let mut state = lock_state(&shared);
            state.running.remove(&identity);
            let has_more = state
                .queues
                .get(&identity)
                .is_some_and(|queue| !queue.is_empty());
            if has_more {
                state.ready.push_back(identity.clone());
            } else {
                state.queues.remove(&identity);
            }
            state.inflight = state.inflight.saturating_sub(1);
            state.completed = state.completed.saturating_add(1);
            match outcome_kind {
                Some(TelegramKeyedDispatchErrorKind::Panic) => {
                    state.failed = state.failed.saturating_add(1);
                    state.panicked = state.panicked.saturating_add(1);
                }
                Some(_) => state.failed = state.failed.saturating_add(1),
                None => {}
            }
            // Publish the result while the completed state is still locked. A
            // waiter can therefore never observe an idle runtime before its
            // accepted future has been resolved, and a resolved future can
            // never race the release of its in-flight slot.
            let _ = job.sender.send(outcome);
        }
        shared.wake.notify_all();
    }
}

fn configure_runtime(
    admission_plan: &JsonValue,
) -> Result<RuntimeConfig, TelegramKeyedDispatchError> {
    let planned = agent_telegram_dispatch_runtime_plan_json(&json!({
        "stage": "configure",
        "admission_plan": admission_plan,
    }))
    .map_err(|_| {
        TelegramKeyedDispatchError::new(TelegramKeyedDispatchErrorKind::PlannerContract)
    })?;
    let object = validate_base_plan(&planned, "configure", "configured")?;
    let backend = object
        .get("backend")
        .and_then(JsonValue::as_str)
        .filter(|value| matches!(*value, "portable_poll" | "linux_epoll"))
        .ok_or_else(configuration_error)?
        .to_string();
    let shard_index = object
        .get("shard_index")
        .and_then(json_usize)
        .ok_or_else(configuration_error)?;
    let inflight_limit = object
        .get("inflight_limit")
        .and_then(json_usize)
        .filter(|value| (1..=MAX_INFLIGHT_LIMIT).contains(value))
        .ok_or_else(configuration_error)?;
    let actions = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .filter(|actions| actions.len() == 1)
        .ok_or_else(planner_contract_error)?;
    let action = actions[0].as_object().ok_or_else(planner_contract_error)?;
    if action.get("kind").and_then(JsonValue::as_str) != Some("configure_dispatch_runtime")
        || action.get("backend").and_then(JsonValue::as_str) != Some(backend.as_str())
        || action.get("shard_index").and_then(json_usize) != Some(shard_index)
        || action.get("inflight_limit").and_then(json_usize) != Some(inflight_limit)
    {
        return Err(planner_contract_error());
    }

    Ok(RuntimeConfig {
        backend,
        shard_index,
        inflight_limit,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_submit_plan(
    planned: &JsonValue,
    stage: &str,
    identity: &QueueIdentity,
    config: &RuntimeConfig,
    inflight_count: usize,
    stopped: bool,
    has_executor: bool,
) -> Result<(), TelegramKeyedDispatchError> {
    let expected_state = if stopped {
        "stopped"
    } else if inflight_count >= config.inflight_limit {
        "inflight_limit_reached"
    } else {
        "accepted"
    };
    let object = validate_base_plan(planned, stage, expected_state)?;
    if object.get("dispatcher_kind").and_then(JsonValue::as_str)
        != Some(identity.dispatcher_kind.as_str())
        || object.get("queue_key").and_then(JsonValue::as_str) != Some(identity.queue_key.as_str())
        || object.get("backend").and_then(JsonValue::as_str) != Some(config.backend.as_str())
        || object.get("shard_index").and_then(json_usize) != Some(config.shard_index)
        || object.get("inflight_count").and_then(json_usize) != Some(inflight_count)
        || object.get("inflight_limit").and_then(json_usize) != Some(config.inflight_limit)
    {
        return Err(planner_contract_error());
    }

    let actions = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .ok_or_else(planner_contract_error)?;
    if expected_state != "accepted" {
        if object.get("should_submit").and_then(JsonValue::as_bool) != Some(false)
            || object
                .get("should_reserve_inflight_slot")
                .and_then(JsonValue::as_bool)
                != Some(false)
            || !actions.is_empty()
        {
            return Err(planner_contract_error());
        }
        return Ok(());
    }

    let should_create_executor = !has_executor;
    if object.get("should_submit").and_then(JsonValue::as_bool) != Some(true)
        || object
            .get("should_reserve_inflight_slot")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("should_create_executor")
            .and_then(JsonValue::as_bool)
            != Some(should_create_executor)
    {
        return Err(planner_contract_error());
    }
    let expected_kinds: &[&str] = if should_create_executor {
        &[
            "reserve_inflight_slot",
            "ensure_executor",
            "submit_callable",
            "track_future",
        ]
    } else {
        &["reserve_inflight_slot", "submit_callable", "track_future"]
    };
    if actions.len() != expected_kinds.len()
        || actions
            .iter()
            .zip(expected_kinds)
            .any(|(action, expected)| {
                action.get("kind").and_then(JsonValue::as_str) != Some(*expected)
            })
    {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn validate_stop_plan(
    planned: &JsonValue,
    dispatch_queue_count: usize,
    reply_queue_count: usize,
) -> Result<(), TelegramKeyedDispatchError> {
    let object = validate_base_plan(planned, "stop", "stopped")?;
    if object.get("should_stop").and_then(JsonValue::as_bool) != Some(true)
        || object.get("dispatch_queue_count").and_then(json_usize) != Some(dispatch_queue_count)
        || object.get("reply_queue_count").and_then(json_usize) != Some(reply_queue_count)
    {
        return Err(planner_contract_error());
    }
    let actions = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .filter(|actions| actions.len() == 3)
        .ok_or_else(planner_contract_error)?;
    let expected = [
        "shutdown_dispatchers",
        "shutdown_dispatchers",
        "clear_dispatchers",
    ];
    if actions
        .iter()
        .zip(expected)
        .any(|(action, expected)| action.get("kind").and_then(JsonValue::as_str) != Some(expected))
    {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn validate_base_plan<'a>(
    planned: &'a JsonValue,
    stage: &str,
    state: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramKeyedDispatchError> {
    let object = planned.as_object().ok_or_else(planner_contract_error)?;
    if object
        .get("dispatch_runtime_contract")
        .and_then(JsonValue::as_str)
        != Some(PLANNING_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(PLANNING_MIGRATION_STAGE)
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object.get("stage").and_then(JsonValue::as_str) != Some(stage)
        || object
            .get("dispatch_runtime_state")
            .and_then(JsonValue::as_str)
            != Some(state)
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_dispatch_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(planner_contract_error());
    }
    Ok(object)
}

fn normalize_queue_key(value: &str) -> Result<String, TelegramKeyedDispatchError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_QUEUE_KEY_LENGTH
        || value.chars().any(char::is_control)
    {
        return Err(TelegramKeyedDispatchError::new(
            TelegramKeyedDispatchErrorKind::InvalidQueueKey,
        ));
    }
    Ok(value.to_string())
}

fn json_usize(value: &JsonValue) -> Option<usize> {
    value.as_u64().and_then(|value| usize::try_from(value).ok())
}

fn configuration_error() -> TelegramKeyedDispatchError {
    TelegramKeyedDispatchError::new(TelegramKeyedDispatchErrorKind::Configuration)
}

fn planner_contract_error() -> TelegramKeyedDispatchError {
    TelegramKeyedDispatchError::new(TelegramKeyedDispatchErrorKind::PlannerContract)
}

fn lock_state(shared: &SharedRuntime) -> MutexGuard<'_, SchedulerState> {
    match shared.state.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests;
