use super::agent_telegram_submission_runtime_plan_json;
use crate::event_loop::telegram_dispatch_runtime::{
    TelegramKeyedDispatchError, TelegramKeyedDispatchErrorKind, TelegramKeyedDispatchFuture,
    TelegramKeyedDispatchJobExecutor, TelegramKeyedDispatchRuntime,
};
use crate::event_loop::telegram_logical_turn_runtime::{
    TelegramLogicalTurn, TelegramLogicalTurnBufferOutcome, TelegramLogicalTurnError,
    TelegramLogicalTurnErrorKind, TelegramLogicalTurnRuntime, TelegramLogicalTurnStep,
};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

const EXECUTION_CONTRACT: &str = "ait_agent_core.event_loop.TelegramSubmissionExecution.v1";
const EXECUTION_MIGRATION_STAGE: &str = "rust_agent_telegram_submission_execution";
const PLANNING_CONTRACT: &str = "ait_agent_core.event_loop.TelegramSubmissionRuntime.v1";
const PLANNING_MIGRATION_STAGE: &str = "rust_agent_telegram_submission_runtime";
const MAX_CALLBACK_SLOT_LENGTH: usize = 64;
const MAX_QUEUE_KEY_LENGTH: usize = 128;
const EXECUTION_FAILURE: &str = "Telegram submission job execution failed.";

pub trait TelegramSubmissionExecutionPort: Send + Sync + 'static {
    fn handle_update(
        &self,
        update: &JsonValue,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String>;

    fn handle_logical_turn(
        &self,
        turn: &TelegramLogicalTurn,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String>;

    fn run_background_sync_for_chat(&self, chat_id: &str) -> Result<JsonValue, String>;

    fn execute_reply(&self, callback_slot: &str, args: &[JsonValue]) -> Result<JsonValue, String>;

    fn wait_for_live_replies(&self, timeout: Option<Duration>) -> Result<bool, String>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramSubmissionExecutionErrorKind {
    Configuration,
    InvalidUpdate,
    InvalidDispatchItem,
    InvalidQueueKey,
    InvalidCallbackSlot,
    PlannerContract,
    LogicalTurnInput,
    LogicalTurnCapacity,
    LogicalTurnRuntime,
    Stopped,
    InflightLimit,
    QueueCapacity,
    WorkerUnavailable,
    Executor,
    Panic,
    Timeout,
    LiveReply,
    State,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramSubmissionExecutionError {
    kind: TelegramSubmissionExecutionErrorKind,
}

impl TelegramSubmissionExecutionError {
    pub fn kind(&self) -> TelegramSubmissionExecutionErrorKind {
        self.kind
    }

    fn new(kind: TelegramSubmissionExecutionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for TelegramSubmissionExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            TelegramSubmissionExecutionErrorKind::Configuration => {
                "Telegram submission configuration is invalid."
            }
            TelegramSubmissionExecutionErrorKind::InvalidUpdate => {
                "Telegram submission update is invalid."
            }
            TelegramSubmissionExecutionErrorKind::InvalidDispatchItem => {
                "Telegram submission dispatch item is invalid."
            }
            TelegramSubmissionExecutionErrorKind::InvalidQueueKey => {
                "Telegram submission queue key is invalid."
            }
            TelegramSubmissionExecutionErrorKind::InvalidCallbackSlot => {
                "Telegram submission callback slot is invalid."
            }
            TelegramSubmissionExecutionErrorKind::PlannerContract => {
                "Telegram submission planner contract is invalid."
            }
            TelegramSubmissionExecutionErrorKind::LogicalTurnInput => {
                "Telegram submission logical-turn input is invalid."
            }
            TelegramSubmissionExecutionErrorKind::LogicalTurnCapacity => {
                "Telegram submission logical-turn capacity is exhausted."
            }
            TelegramSubmissionExecutionErrorKind::LogicalTurnRuntime => {
                "Telegram submission logical-turn runtime failed."
            }
            TelegramSubmissionExecutionErrorKind::Stopped => {
                "Telegram submission runtime is stopped."
            }
            TelegramSubmissionExecutionErrorKind::InflightLimit => {
                "Telegram submission in-flight capacity is exhausted."
            }
            TelegramSubmissionExecutionErrorKind::QueueCapacity => {
                "Telegram submission queue capacity is exhausted."
            }
            TelegramSubmissionExecutionErrorKind::WorkerUnavailable => {
                "Telegram submission worker is unavailable."
            }
            TelegramSubmissionExecutionErrorKind::Executor => {
                "Telegram submission executor failed."
            }
            TelegramSubmissionExecutionErrorKind::Panic => "Telegram submission job panicked.",
            TelegramSubmissionExecutionErrorKind::Timeout => {
                "Telegram submission result wait timed out."
            }
            TelegramSubmissionExecutionErrorKind::LiveReply => {
                "Telegram submission live-reply wait failed."
            }
            TelegramSubmissionExecutionErrorKind::State => "Telegram submission state is invalid.",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TelegramSubmissionExecutionError {}

pub struct TelegramSubmissionFuture {
    inner: TelegramKeyedDispatchFuture,
}

impl fmt::Debug for TelegramSubmissionFuture {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramSubmissionFuture")
            .finish_non_exhaustive()
    }
}

impl TelegramSubmissionFuture {
    pub fn wait(
        self,
        timeout: Option<Duration>,
    ) -> Result<JsonValue, TelegramSubmissionExecutionError> {
        self.inner.wait(timeout).map_err(map_dispatch_error)
    }
}

pub struct TelegramSubmissionRuntime {
    dispatch: TelegramKeyedDispatchRuntime,
    logical_turn: Arc<TelegramLogicalTurnRuntime>,
    execution: Arc<dyn TelegramSubmissionExecutionPort>,
    planner: Arc<dyn SubmissionExecutionPlanningPort>,
    stats: Arc<Mutex<ExecutionStats>>,
    stopped: AtomicBool,
    next_fallback_id: AtomicU64,
}

impl TelegramSubmissionRuntime {
    pub fn new(
        execution: Arc<dyn TelegramSubmissionExecutionPort>,
        logical_turn: Arc<TelegramLogicalTurnRuntime>,
        admission_plan: &JsonValue,
        worker_count: usize,
        per_key_queue_capacity: usize,
    ) -> Result<Self, TelegramSubmissionExecutionError> {
        Self::with_planning_port(
            execution,
            logical_turn,
            admission_plan,
            worker_count,
            per_key_queue_capacity,
            Arc::new(NativeSubmissionExecutionPlanningPort),
        )
    }

    fn with_planning_port(
        execution: Arc<dyn TelegramSubmissionExecutionPort>,
        logical_turn: Arc<TelegramLogicalTurnRuntime>,
        admission_plan: &JsonValue,
        worker_count: usize,
        per_key_queue_capacity: usize,
        planner: Arc<dyn SubmissionExecutionPlanningPort>,
    ) -> Result<Self, TelegramSubmissionExecutionError> {
        let stats = Arc::new(Mutex::new(ExecutionStats::default()));
        let job_executor: Arc<dyn TelegramKeyedDispatchJobExecutor> =
            Arc::new(SubmissionJobExecutor {
                execution: Arc::clone(&execution),
                logical_turn: Arc::clone(&logical_turn),
                stats: Arc::clone(&stats),
            });
        let dispatch = TelegramKeyedDispatchRuntime::new(
            job_executor,
            admission_plan,
            worker_count,
            per_key_queue_capacity,
        )
        .map_err(map_dispatch_error)?;

        Ok(Self {
            dispatch,
            logical_turn,
            execution,
            planner,
            stats,
            stopped: AtomicBool::new(false),
            next_fallback_id: AtomicU64::new(1),
        })
    }

    pub fn submit_update(
        &self,
        update: JsonValue,
    ) -> Result<TelegramSubmissionFuture, TelegramSubmissionExecutionError> {
        if !update.is_object() {
            return Err(execution_error(
                TelegramSubmissionExecutionErrorKind::InvalidUpdate,
            ));
        }
        let fallback_update_key = self.next_fallback_update_key()?;
        let merge_enabled = self.logical_turn.merge_enabled();
        let planned = self.plan(&json!({
            "stage": "submit_update",
            "update": update.clone(),
            "fallback_update_key": fallback_update_key,
            "logical_turn_merge_enabled": merge_enabled,
            "service_runtime_stopped": self.stopped.load(Ordering::Acquire),
        }))?;
        let submission = validate_update_submission_plan(
            &planned,
            "submit_update",
            &update,
            None,
            merge_enabled,
            self.stopped.load(Ordering::Acquire),
        )?;
        let logical_update_key = dispatch_update_key(&submission.dispatch_item)
            .unwrap_or_else(|| fallback_update_key.clone());
        let buffer_outcome = self.buffer_update(&update, &logical_update_key, merge_enabled)?;
        let future = match self.dispatch.submit_dispatch(
            &submission.queue_key,
            update_job_request(
                update.clone(),
                submission.dispatch_item,
                logical_update_key.clone(),
                buffer_outcome,
            ),
        ) {
            Ok(future) => future,
            Err(error) => {
                self.rollback_buffered_update(buffer_outcome, &update, &logical_update_key)?;
                return Err(map_dispatch_error(error));
            }
        };
        let mut stats = lock_stats(&self.stats);
        stats.submitted_update_count = stats.submitted_update_count.saturating_add(1);
        Ok(TelegramSubmissionFuture { inner: future })
    }

    pub fn submit_planned_update(
        &self,
        update: JsonValue,
        dispatch_item: JsonValue,
    ) -> Result<TelegramSubmissionFuture, TelegramSubmissionExecutionError> {
        if !update.is_object() {
            return Err(execution_error(
                TelegramSubmissionExecutionErrorKind::InvalidUpdate,
            ));
        }
        if !dispatch_item.is_object() {
            return Err(execution_error(
                TelegramSubmissionExecutionErrorKind::InvalidDispatchItem,
            ));
        }
        let fallback_update_key = self.next_fallback_update_key()?;
        let merge_enabled = self.logical_turn.merge_enabled();
        let planned = self.plan(&json!({
            "stage": "submit_planned_update",
            "update": update.clone(),
            "dispatch_item": dispatch_item.clone(),
            "logical_turn_merge_enabled": merge_enabled,
            "service_runtime_stopped": self.stopped.load(Ordering::Acquire),
        }))?;
        let submission = validate_update_submission_plan(
            &planned,
            "submit_planned_update",
            &update,
            Some(&dispatch_item),
            merge_enabled,
            self.stopped.load(Ordering::Acquire),
        )?;
        let logical_update_key = dispatch_update_key(&submission.dispatch_item)
            .unwrap_or_else(|| fallback_update_key.clone());
        let buffer_outcome = self.buffer_update(&update, &logical_update_key, merge_enabled)?;
        let future = match self.dispatch.submit_dispatch(
            &submission.queue_key,
            update_job_request(
                update.clone(),
                submission.dispatch_item,
                logical_update_key.clone(),
                buffer_outcome,
            ),
        ) {
            Ok(future) => future,
            Err(error) => {
                self.rollback_buffered_update(buffer_outcome, &update, &logical_update_key)?;
                return Err(map_dispatch_error(error));
            }
        };
        let mut stats = lock_stats(&self.stats);
        stats.submitted_planned_update_count =
            stats.submitted_planned_update_count.saturating_add(1);
        Ok(TelegramSubmissionFuture { inner: future })
    }

    pub fn submit_background_sync_for_chat(
        &self,
        queue_key: Option<&str>,
        chat_id: JsonValue,
    ) -> Result<TelegramSubmissionFuture, TelegramSubmissionExecutionError> {
        let mut request = json!({
            "stage": "submit_background_sync_for_chat",
            "chat_id": chat_id,
            "service_runtime_stopped": self.stopped.load(Ordering::Acquire),
        });
        if let Some(queue_key) = queue_key {
            request["queue_key"] = json!(queue_key);
        }
        let planned = self.plan(&request)?;
        let submission =
            validate_background_submission_plan(&planned, self.stopped.load(Ordering::Acquire))?;
        let future = self
            .dispatch
            .submit_dispatch(
                &submission.queue_key,
                json!({
                    "job_kind": "run_background_sync_for_chat",
                    "chat_id": submission.chat_id,
                }),
            )
            .map_err(map_dispatch_error)?;
        let mut stats = lock_stats(&self.stats);
        stats.submitted_background_sync_count =
            stats.submitted_background_sync_count.saturating_add(1);
        Ok(TelegramSubmissionFuture { inner: future })
    }

    pub fn submit_reply_serialized(
        &self,
        queue_key: &str,
        callback_slot: &str,
        args: Vec<JsonValue>,
    ) -> Result<TelegramSubmissionFuture, TelegramSubmissionExecutionError> {
        let callback_slot = normalize_callback_slot(callback_slot)?;
        let planned = self.plan(&json!({
            "stage": "submit_reply_serialized",
            "queue_key": queue_key,
            "callback_slot": callback_slot,
            "args": args.clone(),
            "service_runtime_stopped": self.stopped.load(Ordering::Acquire),
        }))?;
        let submission = validate_reply_submission_plan(
            &planned,
            &callback_slot,
            &args,
            self.stopped.load(Ordering::Acquire),
        )?;
        let future = self
            .dispatch
            .submit_reply(
                &submission.queue_key,
                json!({
                    "job_kind": "execute_reply",
                    "callback_slot": callback_slot,
                    "args": args,
                }),
            )
            .map_err(map_dispatch_error)?;
        let mut stats = lock_stats(&self.stats);
        stats.submitted_reply_count = stats.submitted_reply_count.saturating_add(1);
        Ok(TelegramSubmissionFuture { inner: future })
    }

    pub fn wait_for_idle(
        &self,
        timeout: Option<Duration>,
    ) -> Result<bool, TelegramSubmissionExecutionError> {
        let service_runtime_idle = self.dispatch.wait_for_idle(timeout);
        let live_reply_manager_idle = if service_runtime_idle {
            self.execution
                .wait_for_live_replies(timeout)
                .map_err(|_| execution_error(TelegramSubmissionExecutionErrorKind::LiveReply))?
        } else {
            false
        };
        let planned = self.plan(&json!({
            "stage": "wait_for_idle",
            "timeout_seconds": timeout.map(|value| value.as_secs_f64()),
            "service_runtime_idle": service_runtime_idle,
            "live_reply_manager_idle": live_reply_manager_idle,
        }))?;
        validate_idle_plan(
            &planned,
            timeout,
            service_runtime_idle,
            live_reply_manager_idle,
        )?;
        let mut stats = lock_stats(&self.stats);
        stats.idle_wait_count = stats.idle_wait_count.saturating_add(1);
        if !service_runtime_idle || !live_reply_manager_idle {
            stats.idle_timeout_count = stats.idle_timeout_count.saturating_add(1);
        }
        Ok(service_runtime_idle && live_reply_manager_idle)
    }

    pub fn request_stop(&self) -> Result<(), TelegramSubmissionExecutionError> {
        self.stopped.store(true, Ordering::Release);
        self.dispatch.request_stop().map_err(map_dispatch_error)
    }

    pub fn snapshot_json(&self) -> JsonValue {
        let stats = lock_stats(&self.stats);
        let dispatch = self.dispatch.snapshot_json();
        let logical = self.logical_turn.snapshot_json();
        json!({
            "execution_contract": EXECUTION_CONTRACT,
            "migration_stage": EXECUTION_MIGRATION_STAGE,
            "transport": "telegram",
            "stopped": self.stopped.load(Ordering::Acquire),
            "submitted_update_count": stats.submitted_update_count,
            "submitted_planned_update_count": stats.submitted_planned_update_count,
            "submitted_background_sync_count": stats.submitted_background_sync_count,
            "submitted_reply_count": stats.submitted_reply_count,
            "handled_update_count": stats.handled_update_count,
            "handled_logical_turn_count": stats.handled_logical_turn_count,
            "skipped_duplicate_count": stats.skipped_duplicate_count,
            "background_sync_execution_count": stats.background_sync_execution_count,
            "reply_execution_count": stats.reply_execution_count,
            "execution_failure_count": stats.execution_failure_count,
            "idle_wait_count": stats.idle_wait_count,
            "idle_timeout_count": stats.idle_timeout_count,
            "dispatch_inflight_count": count_field(&dispatch, "inflight_count"),
            "dispatch_queued_count": count_field(&dispatch, "queued_count"),
            "dispatch_running_count": count_field(&dispatch, "running_count"),
            "dispatch_failed_count": count_field(&dispatch, "failed_count"),
            "dispatch_panicked_count": count_field(&dispatch, "panicked_count"),
            "logical_pending_chat_count": count_field(&logical, "pending_chat_count"),
            "logical_pending_update_count": count_field(&logical, "pending_update_count"),
            "logical_duplicate_count": count_field(&logical, "duplicate_count"),
            "rust_submission_execution_required": true,
            "rust_future_tracking_required": true,
            "python_submission_allowed": false,
            "python_callback_execution_allowed": false,
            "python_future_cleanup_allowed": false,
        })
    }

    fn plan(&self, request: &JsonValue) -> Result<JsonValue, TelegramSubmissionExecutionError> {
        self.planner
            .plan(request)
            .map_err(|_| execution_error(TelegramSubmissionExecutionErrorKind::PlannerContract))
    }

    fn buffer_update(
        &self,
        update: &JsonValue,
        fallback_update_key: &str,
        merge_enabled: bool,
    ) -> Result<TelegramLogicalTurnBufferOutcome, TelegramSubmissionExecutionError> {
        if !merge_enabled {
            return Ok(TelegramLogicalTurnBufferOutcome::Disabled);
        }
        self.logical_turn
            .buffer_update(update, fallback_update_key)
            .map_err(map_logical_turn_error)
    }

    fn next_fallback_update_key(&self) -> Result<String, TelegramSubmissionExecutionError> {
        let id = self
            .next_fallback_id
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| execution_error(TelegramSubmissionExecutionErrorKind::State))?;
        Ok(format!("rust-submission-{id}"))
    }

    fn rollback_buffered_update(
        &self,
        buffer_outcome: TelegramLogicalTurnBufferOutcome,
        update: &JsonValue,
        fallback_update_key: &str,
    ) -> Result<(), TelegramSubmissionExecutionError> {
        if buffer_outcome == TelegramLogicalTurnBufferOutcome::Buffered {
            self.logical_turn
                .discard_buffered_update(update, fallback_update_key)
                .map_err(map_logical_turn_error)?;
        }
        Ok(())
    }
}

struct UpdateSubmission {
    queue_key: String,
    dispatch_item: JsonValue,
}

struct BackgroundSubmission {
    queue_key: String,
    chat_id: String,
}

struct ReplySubmission {
    queue_key: String,
}

trait SubmissionExecutionPlanningPort: Send + Sync + 'static {
    fn plan(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Clone, Copy, Debug, Default)]
struct NativeSubmissionExecutionPlanningPort;

impl SubmissionExecutionPlanningPort for NativeSubmissionExecutionPlanningPort {
    fn plan(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_submission_runtime_plan_json(request)
    }
}

struct SubmissionJobExecutor {
    execution: Arc<dyn TelegramSubmissionExecutionPort>,
    logical_turn: Arc<TelegramLogicalTurnRuntime>,
    stats: Arc<Mutex<ExecutionStats>>,
}

impl TelegramKeyedDispatchJobExecutor for SubmissionJobExecutor {
    fn execute(
        &self,
        dispatcher_kind: &str,
        _queue_key: &str,
        request: &JsonValue,
    ) -> Result<JsonValue, String> {
        let result = self.execute_inner(dispatcher_kind, request);
        if result.is_err() {
            let mut stats = lock_stats(&self.stats);
            stats.execution_failure_count = stats.execution_failure_count.saturating_add(1);
        }
        result.map_err(|_| EXECUTION_FAILURE.to_string())
    }
}

impl SubmissionJobExecutor {
    fn execute_inner(&self, dispatcher_kind: &str, request: &JsonValue) -> Result<JsonValue, ()> {
        let object = request.as_object().ok_or(())?;
        let job_kind = clean_text(object.get("job_kind")).ok_or(())?;
        match job_kind.as_str() {
            "handle_submitted_update" if dispatcher_kind == "dispatch" => {
                self.execute_update(object)
            }
            "run_background_sync_for_chat" if dispatcher_kind == "dispatch" => {
                let chat_id = clean_text(object.get("chat_id")).ok_or(())?;
                let result = self
                    .execution
                    .run_background_sync_for_chat(&chat_id)
                    .map_err(|_| ())?;
                let mut stats = lock_stats(&self.stats);
                stats.background_sync_execution_count =
                    stats.background_sync_execution_count.saturating_add(1);
                Ok(result)
            }
            "execute_reply" if dispatcher_kind == "reply" => {
                let callback_slot = clean_text(object.get("callback_slot")).ok_or(())?;
                normalize_callback_slot(&callback_slot).map_err(|_| ())?;
                let args = object.get("args").and_then(JsonValue::as_array).ok_or(())?;
                let result = self
                    .execution
                    .execute_reply(&callback_slot, args)
                    .map_err(|_| ())?;
                let mut stats = lock_stats(&self.stats);
                stats.reply_execution_count = stats.reply_execution_count.saturating_add(1);
                Ok(result)
            }
            _ => Err(()),
        }
    }

    fn execute_update(&self, object: &Map<String, JsonValue>) -> Result<JsonValue, ()> {
        let update = object
            .get("update")
            .filter(|value| value.is_object())
            .ok_or(())?;
        let dispatch_item = object
            .get("dispatch_item")
            .filter(|value| value.is_object())
            .ok_or(())?;
        let fallback_update_key = clean_text(object.get("fallback_update_key")).ok_or(())?;
        let step = self
            .logical_turn
            .claim_update(update, &fallback_update_key)
            .map_err(|_| ())?;
        match step {
            TelegramLogicalTurnStep::Disabled
            | TelegramLogicalTurnStep::NotCandidate
            | TelegramLogicalTurnStep::PassThrough => {
                let result = self
                    .execution
                    .handle_update(update, dispatch_item)
                    .map_err(|_| ())?;
                let mut stats = lock_stats(&self.stats);
                stats.handled_update_count = stats.handled_update_count.saturating_add(1);
                Ok(result)
            }
            TelegramLogicalTurnStep::Skip => {
                let mut stats = lock_stats(&self.stats);
                stats.skipped_duplicate_count = stats.skipped_duplicate_count.saturating_add(1);
                Ok(json!({
                    "execution_contract": EXECUTION_CONTRACT,
                    "migration_stage": EXECUTION_MIGRATION_STAGE,
                    "submission_state": "skipped",
                    "handled": false,
                    "duplicate_or_consumed": true,
                }))
            }
            TelegramLogicalTurnStep::LogicalTurn(turn) => {
                let result = self
                    .execution
                    .handle_logical_turn(&turn, dispatch_item)
                    .map_err(|_| ())?;
                let mut stats = lock_stats(&self.stats);
                stats.handled_logical_turn_count =
                    stats.handled_logical_turn_count.saturating_add(1);
                Ok(result)
            }
            TelegramLogicalTurnStep::Wait(_) => Err(()),
        }
    }
}

#[derive(Default)]
struct ExecutionStats {
    submitted_update_count: u64,
    submitted_planned_update_count: u64,
    submitted_background_sync_count: u64,
    submitted_reply_count: u64,
    handled_update_count: u64,
    handled_logical_turn_count: u64,
    skipped_duplicate_count: u64,
    background_sync_execution_count: u64,
    reply_execution_count: u64,
    execution_failure_count: u64,
    idle_wait_count: u64,
    idle_timeout_count: u64,
}

fn validate_update_submission_plan(
    planned: &JsonValue,
    stage: &str,
    update: &JsonValue,
    expected_dispatch_item: Option<&JsonValue>,
    merge_enabled: bool,
    stopped: bool,
) -> Result<UpdateSubmission, TelegramSubmissionExecutionError> {
    let object = validate_submission_base(planned, stage)?;
    validate_submission_admission(object, stopped)?;
    let queue_key = normalize_queue_key(required_clean_text(object, "queue_key")?)?;
    let dispatch_item = object
        .get("dispatch_item")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| {
            execution_error(TelegramSubmissionExecutionErrorKind::InvalidDispatchItem)
        })?;
    if dispatch_item
        .get("dispatch_key")
        .and_then(JsonValue::as_str)
        != Some(queue_key.as_str())
        || expected_dispatch_item.is_some_and(|expected| expected != &dispatch_item)
    {
        return Err(execution_error(
            TelegramSubmissionExecutionErrorKind::PlannerContract,
        ));
    }
    let actions = required_actions(object)?;
    let expected_action_count = if merge_enabled { 2 } else { 1 };
    if actions.len() != expected_action_count {
        return Err(execution_error(
            TelegramSubmissionExecutionErrorKind::PlannerContract,
        ));
    }
    if merge_enabled
        && (actions[0].get("kind").and_then(JsonValue::as_str)
            != Some("buffer_submitted_text_update")
            || actions[0].get("update") != Some(update))
    {
        return Err(execution_error(
            TelegramSubmissionExecutionErrorKind::PlannerContract,
        ));
    }
    let submit = actions.last().ok_or_else(planner_contract_error)?;
    if submit.get("kind").and_then(JsonValue::as_str) != Some("submit_serialized")
        || submit.get("callback").and_then(JsonValue::as_str) != Some("handle_submitted_update")
        || submit.get("queue_key").and_then(JsonValue::as_str) != Some(queue_key.as_str())
        || submit
            .get("args")
            .and_then(JsonValue::as_array)
            .filter(|args| args.len() == 1 && args.first() == Some(update))
            .is_none()
        || object.get("submit_action") != Some(submit)
    {
        return Err(execution_error(
            TelegramSubmissionExecutionErrorKind::PlannerContract,
        ));
    }
    Ok(UpdateSubmission {
        queue_key,
        dispatch_item,
    })
}

fn validate_background_submission_plan(
    planned: &JsonValue,
    stopped: bool,
) -> Result<BackgroundSubmission, TelegramSubmissionExecutionError> {
    let object = validate_submission_base(planned, "submit_background_sync_for_chat")?;
    validate_submission_admission(object, stopped)?;
    let queue_key = normalize_queue_key(required_clean_text(object, "queue_key")?)?;
    let details = object
        .get("details")
        .and_then(JsonValue::as_object)
        .ok_or_else(planner_contract_error)?;
    let chat_id = required_clean_text(details, "chat_id_text")?;
    let actions = required_actions(object)?;
    if actions.len() != 1 {
        return Err(planner_contract_error());
    }
    let submit = &actions[0];
    if submit.get("kind").and_then(JsonValue::as_str) != Some("submit_serialized")
        || submit.get("callback").and_then(JsonValue::as_str)
            != Some("run_background_sync_for_chat")
        || submit.get("queue_key").and_then(JsonValue::as_str) != Some(queue_key.as_str())
        || submit
            .get("args")
            .and_then(JsonValue::as_array)
            .filter(|args| args.len() == 1 && args[0].as_str() == Some(chat_id.as_str()))
            .is_none()
        || object.get("submit_action") != Some(submit)
    {
        return Err(planner_contract_error());
    }
    Ok(BackgroundSubmission { queue_key, chat_id })
}

fn validate_reply_submission_plan(
    planned: &JsonValue,
    callback_slot: &str,
    args: &[JsonValue],
    stopped: bool,
) -> Result<ReplySubmission, TelegramSubmissionExecutionError> {
    let object = validate_submission_base(planned, "submit_reply_serialized")?;
    validate_submission_admission(object, stopped)?;
    let queue_key = normalize_queue_key(required_clean_text(object, "queue_key")?)?;
    let actions = required_actions(object)?;
    if actions.len() != 1 {
        return Err(planner_contract_error());
    }
    let submit = &actions[0];
    if submit.get("kind").and_then(JsonValue::as_str) != Some("submit_reply_serialized")
        || submit.get("callback").and_then(JsonValue::as_str) != Some(callback_slot)
        || submit.get("queue_key").and_then(JsonValue::as_str) != Some(queue_key.as_str())
        || submit
            .get("args")
            .and_then(JsonValue::as_array)
            .map(Vec::as_slice)
            != Some(args)
        || object.get("submit_action") != Some(submit)
    {
        return Err(planner_contract_error());
    }
    Ok(ReplySubmission { queue_key })
}

fn validate_idle_plan(
    planned: &JsonValue,
    timeout: Option<Duration>,
    service_runtime_idle: bool,
    live_reply_manager_idle: bool,
) -> Result<(), TelegramSubmissionExecutionError> {
    let object = validate_submission_base(planned, "wait_for_idle")?;
    let idle = service_runtime_idle && live_reply_manager_idle;
    if object.get("idle").and_then(JsonValue::as_bool) != Some(idle)
        || object
            .get("service_runtime_idle")
            .and_then(JsonValue::as_bool)
            != Some(service_runtime_idle)
        || object
            .get("live_reply_manager_idle")
            .and_then(JsonValue::as_bool)
            != Some(live_reply_manager_idle)
        || object
            .get("checked_live_reply_manager")
            .and_then(JsonValue::as_bool)
            != Some(service_runtime_idle)
        || object.get("submission_state").and_then(JsonValue::as_str)
            != Some(if idle { "idle" } else { "busy" })
    {
        return Err(planner_contract_error());
    }
    let timeout_matches = match timeout {
        Some(timeout) => object
            .get("timeout_seconds")
            .and_then(JsonValue::as_f64)
            .is_some_and(|value| (value - timeout.as_secs_f64()).abs() <= f64::EPSILON),
        None => object.get("timeout_seconds") == Some(&JsonValue::Null),
    };
    let actions = required_actions(object)?;
    if !timeout_matches
        || actions.len() != 2
        || actions[0].get("kind").and_then(JsonValue::as_str) != Some("wait_service_runtime_idle")
        || actions[1].get("kind").and_then(JsonValue::as_str)
            != Some("wait_live_reply_manager_idle")
        || actions[1].get("enabled").and_then(JsonValue::as_bool) != Some(service_runtime_idle)
    {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn validate_submission_base<'a>(
    planned: &'a JsonValue,
    stage: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramSubmissionExecutionError> {
    let object = planned.as_object().ok_or_else(planner_contract_error)?;
    if object
        .get("submission_runtime_contract")
        .and_then(JsonValue::as_str)
        != Some(PLANNING_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(PLANNING_MIGRATION_STAGE)
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object.get("stage").and_then(JsonValue::as_str) != Some(stage)
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_submission_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object
            .get("service_runtime_dispatch_port_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
    {
        return Err(planner_contract_error());
    }
    Ok(object)
}

fn validate_submission_admission(
    object: &Map<String, JsonValue>,
    stopped: bool,
) -> Result<(), TelegramSubmissionExecutionError> {
    if stopped {
        if object.get("submission_state").and_then(JsonValue::as_str) == Some("rejected")
            && object.get("should_submit").and_then(JsonValue::as_bool) == Some(false)
            && object
                .get("actions")
                .and_then(JsonValue::as_array)
                .is_some_and(Vec::is_empty)
        {
            return Err(execution_error(
                TelegramSubmissionExecutionErrorKind::Stopped,
            ));
        }
        return Err(planner_contract_error());
    }
    if object.get("submission_state").and_then(JsonValue::as_str) != Some("planned")
        || object.get("should_submit").and_then(JsonValue::as_bool) != Some(true)
        || !object
            .get("rejection_reasons")
            .and_then(JsonValue::as_array)
            .is_some_and(Vec::is_empty)
    {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn update_job_request(
    update: JsonValue,
    dispatch_item: JsonValue,
    fallback_update_key: String,
    buffer_outcome: TelegramLogicalTurnBufferOutcome,
) -> JsonValue {
    json!({
        "job_kind": "handle_submitted_update",
        "update": update,
        "dispatch_item": dispatch_item,
        "fallback_update_key": fallback_update_key,
        "buffer_state": match buffer_outcome {
            TelegramLogicalTurnBufferOutcome::Disabled => "disabled",
            TelegramLogicalTurnBufferOutcome::NotCandidate => "not_candidate",
            TelegramLogicalTurnBufferOutcome::Buffered => "buffered",
            TelegramLogicalTurnBufferOutcome::Duplicate => "duplicate",
        },
    })
}

fn dispatch_update_key(dispatch_item: &JsonValue) -> Option<String> {
    clean_text(dispatch_item.get("update_key"))
}

fn required_actions(
    object: &Map<String, JsonValue>,
) -> Result<&Vec<JsonValue>, TelegramSubmissionExecutionError> {
    object
        .get("actions")
        .and_then(JsonValue::as_array)
        .ok_or_else(planner_contract_error)
}

fn required_clean_text(
    object: &Map<String, JsonValue>,
    key: &str,
) -> Result<String, TelegramSubmissionExecutionError> {
    clean_text(object.get(key)).ok_or_else(planner_contract_error)
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let value = value?.as_str()?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn normalize_queue_key(value: String) -> Result<String, TelegramSubmissionExecutionError> {
    if value.chars().count() > MAX_QUEUE_KEY_LENGTH || value.chars().any(char::is_control) {
        return Err(execution_error(
            TelegramSubmissionExecutionErrorKind::InvalidQueueKey,
        ));
    }
    Ok(value)
}

fn normalize_callback_slot(value: &str) -> Result<String, TelegramSubmissionExecutionError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_CALLBACK_SLOT_LENGTH
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(execution_error(
            TelegramSubmissionExecutionErrorKind::InvalidCallbackSlot,
        ));
    }
    Ok(value.to_string())
}

fn map_dispatch_error(error: TelegramKeyedDispatchError) -> TelegramSubmissionExecutionError {
    let kind = match error.kind() {
        TelegramKeyedDispatchErrorKind::Configuration => {
            TelegramSubmissionExecutionErrorKind::Configuration
        }
        TelegramKeyedDispatchErrorKind::InvalidDispatcher => {
            TelegramSubmissionExecutionErrorKind::PlannerContract
        }
        TelegramKeyedDispatchErrorKind::InvalidQueueKey => {
            TelegramSubmissionExecutionErrorKind::InvalidQueueKey
        }
        TelegramKeyedDispatchErrorKind::Stopped => TelegramSubmissionExecutionErrorKind::Stopped,
        TelegramKeyedDispatchErrorKind::InflightLimit => {
            TelegramSubmissionExecutionErrorKind::InflightLimit
        }
        TelegramKeyedDispatchErrorKind::QueueCapacity => {
            TelegramSubmissionExecutionErrorKind::QueueCapacity
        }
        TelegramKeyedDispatchErrorKind::PlannerContract => {
            TelegramSubmissionExecutionErrorKind::PlannerContract
        }
        TelegramKeyedDispatchErrorKind::WorkerUnavailable => {
            TelegramSubmissionExecutionErrorKind::WorkerUnavailable
        }
        TelegramKeyedDispatchErrorKind::Executor => TelegramSubmissionExecutionErrorKind::Executor,
        TelegramKeyedDispatchErrorKind::Panic => TelegramSubmissionExecutionErrorKind::Panic,
        TelegramKeyedDispatchErrorKind::Timeout => TelegramSubmissionExecutionErrorKind::Timeout,
    };
    execution_error(kind)
}

fn map_logical_turn_error(error: TelegramLogicalTurnError) -> TelegramSubmissionExecutionError {
    let kind = match error.kind() {
        TelegramLogicalTurnErrorKind::InvalidUpdate
        | TelegramLogicalTurnErrorKind::InvalidFallbackKey => {
            TelegramSubmissionExecutionErrorKind::LogicalTurnInput
        }
        TelegramLogicalTurnErrorKind::ChatCapacity
        | TelegramLogicalTurnErrorKind::PerChatCapacity => {
            TelegramSubmissionExecutionErrorKind::LogicalTurnCapacity
        }
        TelegramLogicalTurnErrorKind::Configuration
        | TelegramLogicalTurnErrorKind::PlannerContract
        | TelegramLogicalTurnErrorKind::Clock
        | TelegramLogicalTurnErrorKind::Sleeper
        | TelegramLogicalTurnErrorKind::State => {
            TelegramSubmissionExecutionErrorKind::LogicalTurnRuntime
        }
    };
    execution_error(kind)
}

fn planner_contract_error() -> TelegramSubmissionExecutionError {
    execution_error(TelegramSubmissionExecutionErrorKind::PlannerContract)
}

fn execution_error(kind: TelegramSubmissionExecutionErrorKind) -> TelegramSubmissionExecutionError {
    TelegramSubmissionExecutionError::new(kind)
}

fn count_field(value: &JsonValue, key: &str) -> u64 {
    value.get(key).and_then(JsonValue::as_u64).unwrap_or(0)
}

fn lock_stats(stats: &Mutex<ExecutionStats>) -> MutexGuard<'_, ExecutionStats> {
    match stats.lock() {
        Ok(stats) => stats,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests;
