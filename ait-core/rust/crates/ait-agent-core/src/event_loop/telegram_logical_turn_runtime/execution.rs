use super::agent_telegram_logical_turn_runtime_plan_json;
use crate::event_loop::telegram_polling::agent_telegram_update_dispatch_plan_json;
use crate::event_loop::telegram_turn_inputs::agent_telegram_turn_input_plan_json;
use crate::event_loop::telegram_workflow_query::agent_telegram_workflow_query_plan_json;
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

const EXECUTION_CONTRACT: &str = "ait_agent_core.event_loop.TelegramLogicalTurnExecution.v1";
const EXECUTION_MIGRATION_STAGE: &str = "rust_agent_telegram_logical_turn_execution";
const LOGICAL_PLAN_CONTRACT: &str = "ait_agent_core.event_loop.TelegramLogicalTurnRuntime.v1";
const LOGICAL_PLAN_MIGRATION_STAGE: &str = "rust_agent_telegram_logical_turn_runtime";
const TURN_INPUT_CONTRACT: &str = "ait_agent_core.event_loop.TelegramTurnInput.v1";
const TURN_INPUT_MIGRATION_STAGE: &str = "rust_agent_telegram_turn_input";
const WORKFLOW_QUERY_CONTRACT: &str = "ait_agent_core.event_loop.TelegramWorkflowQuery.v1";
const WORKFLOW_QUERY_MIGRATION_STAGE: &str = "rust_agent_telegram_workflow_query";
const MAX_USERNAME_LENGTH: usize = 128;
const MAX_IDENTITY_LENGTH: usize = 256;
const MAX_MERGE_WINDOW: Duration = Duration::from_secs(300);
const MAX_POLL_INTERVAL: Duration = Duration::from_secs(60);
const MAX_MESSAGES: usize = 1_024;
const MAX_PENDING_CHATS: usize = 4_096;
const MAX_PENDING_PER_CHAT: usize = 1_024;

pub trait TelegramLogicalTurnClockPort: Send + Sync + 'static {
    fn now_monotonic_seconds(&self) -> Result<f64, String>;
}

pub trait TelegramLogicalTurnSleepPort: Send + Sync + 'static {
    fn sleep(&self, duration: Duration) -> Result<(), String>;
}

#[derive(Debug)]
pub struct MonotonicTelegramLogicalTurnClock {
    epoch: Instant,
}

impl Default for MonotonicTelegramLogicalTurnClock {
    fn default() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}

impl TelegramLogicalTurnClockPort for MonotonicTelegramLogicalTurnClock {
    fn now_monotonic_seconds(&self) -> Result<f64, String> {
        Ok(self.epoch.elapsed().as_secs_f64())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ThreadTelegramLogicalTurnSleeper;

impl TelegramLogicalTurnSleepPort for ThreadTelegramLogicalTurnSleeper {
    fn sleep(&self, duration: Duration) -> Result<(), String> {
        thread::sleep(duration);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramLogicalTurnErrorKind {
    Configuration,
    InvalidUpdate,
    InvalidFallbackKey,
    ChatCapacity,
    PerChatCapacity,
    PlannerContract,
    Clock,
    Sleeper,
    State,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramLogicalTurnError {
    kind: TelegramLogicalTurnErrorKind,
}

impl TelegramLogicalTurnError {
    pub fn kind(&self) -> TelegramLogicalTurnErrorKind {
        self.kind
    }

    fn new(kind: TelegramLogicalTurnErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for TelegramLogicalTurnError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            TelegramLogicalTurnErrorKind::Configuration => {
                "Telegram logical-turn configuration is invalid."
            }
            TelegramLogicalTurnErrorKind::InvalidUpdate => {
                "Telegram logical-turn update is invalid."
            }
            TelegramLogicalTurnErrorKind::InvalidFallbackKey => {
                "Telegram logical-turn fallback key is invalid."
            }
            TelegramLogicalTurnErrorKind::ChatCapacity => {
                "Telegram logical-turn chat capacity is exhausted."
            }
            TelegramLogicalTurnErrorKind::PerChatCapacity => {
                "Telegram logical-turn per-chat capacity is exhausted."
            }
            TelegramLogicalTurnErrorKind::PlannerContract => {
                "Telegram logical-turn planner contract is invalid."
            }
            TelegramLogicalTurnErrorKind::Clock => "Telegram logical-turn clock is unavailable.",
            TelegramLogicalTurnErrorKind::Sleeper => "Telegram logical-turn wait failed.",
            TelegramLogicalTurnErrorKind::State => "Telegram logical-turn state is invalid.",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for TelegramLogicalTurnError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramLogicalTurnBufferOutcome {
    Disabled,
    NotCandidate,
    Buffered,
    Duplicate,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramLogicalTurn {
    pub update: JsonValue,
    pub text: String,
    pub actor_identity: String,
    pub telegram_message_id: Option<i64>,
    pub telegram_message_ids: Vec<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TelegramLogicalTurnStep {
    Disabled,
    NotCandidate,
    Skip,
    PassThrough,
    Wait(Duration),
    LogicalTurn(TelegramLogicalTurn),
}

pub struct TelegramLogicalTurnRuntime {
    state: Mutex<BufferState>,
    config: RuntimeConfig,
    clock: Arc<dyn TelegramLogicalTurnClockPort>,
    sleeper: Arc<dyn TelegramLogicalTurnSleepPort>,
    planner: Arc<dyn ExecutionPlanningPort>,
}

impl TelegramLogicalTurnRuntime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        username: impl Into<String>,
        merge_window: Duration,
        max_messages: usize,
        poll_interval: Duration,
        max_pending_chats: usize,
        max_pending_per_chat: usize,
    ) -> Result<Self, TelegramLogicalTurnError> {
        Self::with_ports(
            username,
            merge_window,
            max_messages,
            poll_interval,
            max_pending_chats,
            max_pending_per_chat,
            Arc::new(MonotonicTelegramLogicalTurnClock::default()),
            Arc::new(ThreadTelegramLogicalTurnSleeper),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_ports(
        username: impl Into<String>,
        merge_window: Duration,
        max_messages: usize,
        poll_interval: Duration,
        max_pending_chats: usize,
        max_pending_per_chat: usize,
        clock: Arc<dyn TelegramLogicalTurnClockPort>,
        sleeper: Arc<dyn TelegramLogicalTurnSleepPort>,
    ) -> Result<Self, TelegramLogicalTurnError> {
        Self::with_planning_port(
            username,
            merge_window,
            max_messages,
            poll_interval,
            max_pending_chats,
            max_pending_per_chat,
            clock,
            sleeper,
            Arc::new(NativeExecutionPlanningPort),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn with_planning_port(
        username: impl Into<String>,
        merge_window: Duration,
        max_messages: usize,
        poll_interval: Duration,
        max_pending_chats: usize,
        max_pending_per_chat: usize,
        clock: Arc<dyn TelegramLogicalTurnClockPort>,
        sleeper: Arc<dyn TelegramLogicalTurnSleepPort>,
        planner: Arc<dyn ExecutionPlanningPort>,
    ) -> Result<Self, TelegramLogicalTurnError> {
        let username = username.into().trim().to_string();
        if username.chars().count() > MAX_USERNAME_LENGTH
            || username.chars().any(char::is_control)
            || merge_window > MAX_MERGE_WINDOW
            || poll_interval > MAX_POLL_INTERVAL
            || !(1..=MAX_MESSAGES).contains(&max_messages)
            || !(1..=MAX_PENDING_CHATS).contains(&max_pending_chats)
            || !(1..=MAX_PENDING_PER_CHAT).contains(&max_pending_per_chat)
        {
            return Err(configuration_error());
        }
        let merge_enabled = !merge_window.is_zero() && max_messages > 1;
        if merge_enabled && poll_interval.is_zero() {
            return Err(configuration_error());
        }

        let merge_plan = planner
            .logical_turn(&json!({
                "stage": "merge_enabled",
                "merge_window_seconds": merge_window.as_secs_f64(),
                "max_messages": max_messages,
            }))
            .map_err(|_| planner_contract_error())?;
        let object = validate_logical_plan(&merge_plan, "merge_enabled")?;
        if object.get("logical_turn_state").and_then(JsonValue::as_str)
            != Some(if merge_enabled { "enabled" } else { "disabled" })
            || object.get("merge_enabled").and_then(JsonValue::as_bool) != Some(merge_enabled)
            || object.get("max_messages").and_then(json_usize) != Some(max_messages)
            || !object
                .get("actions")
                .and_then(JsonValue::as_array)
                .is_some_and(Vec::is_empty)
        {
            return Err(planner_contract_error());
        }

        Ok(Self {
            state: Mutex::new(BufferState::default()),
            config: RuntimeConfig {
                username,
                merge_window,
                max_messages,
                poll_interval,
                max_pending_chats,
                max_pending_per_chat,
                merge_enabled,
            },
            clock,
            sleeper,
            planner,
        })
    }

    pub fn merge_enabled(&self) -> bool {
        self.config.merge_enabled
    }

    pub fn buffer_update(
        &self,
        update: &JsonValue,
        fallback_update_key: &str,
    ) -> Result<TelegramLogicalTurnBufferOutcome, TelegramLogicalTurnError> {
        if !self.config.merge_enabled {
            return Ok(TelegramLogicalTurnBufferOutcome::Disabled);
        }
        let received_at = self.read_clock()?;
        let Some(candidate) = self.classify_update(update, fallback_update_key, received_at)?
        else {
            return Ok(TelegramLogicalTurnBufferOutcome::NotCandidate);
        };

        let mut state = lock_state(&self.state);
        let queue_payload = state
            .queues
            .get(&candidate.chat_key)
            .map(|queue| queue.iter().map(PendingUpdate::payload).collect::<Vec<_>>())
            .unwrap_or_default();
        let duplicate = state.queues.get(&candidate.chat_key).is_some_and(|queue| {
            queue
                .iter()
                .any(|item| item.update_key == candidate.update_key)
        });
        let plan = self
            .planner
            .logical_turn(&json!({
                "stage": "buffer_submitted_text_update",
                "candidate": candidate.payload(),
                "queue": queue_payload,
            }))
            .map_err(|_| planner_contract_error())?;
        validate_buffer_plan(&plan, &candidate, duplicate)?;
        if duplicate {
            state.duplicate_count = state.duplicate_count.saturating_add(1);
            return Ok(TelegramLogicalTurnBufferOutcome::Duplicate);
        }
        if !state.queues.contains_key(&candidate.chat_key)
            && state.queues.len() >= self.config.max_pending_chats
        {
            state.rejected_count = state.rejected_count.saturating_add(1);
            return Err(TelegramLogicalTurnError::new(
                TelegramLogicalTurnErrorKind::ChatCapacity,
            ));
        }
        if state
            .queues
            .get(&candidate.chat_key)
            .map_or(0, VecDeque::len)
            >= self.config.max_pending_per_chat
        {
            state.rejected_count = state.rejected_count.saturating_add(1);
            return Err(TelegramLogicalTurnError::new(
                TelegramLogicalTurnErrorKind::PerChatCapacity,
            ));
        }

        state
            .queues
            .entry(candidate.chat_key.clone())
            .or_default()
            .push_back(candidate);
        state.buffered_count = state.buffered_count.saturating_add(1);
        Ok(TelegramLogicalTurnBufferOutcome::Buffered)
    }

    pub fn discard_buffered_update(
        &self,
        update: &JsonValue,
        fallback_update_key: &str,
    ) -> Result<bool, TelegramLogicalTurnError> {
        if !self.config.merge_enabled {
            return Ok(false);
        }
        let received_at = self.read_clock()?;
        let Some(candidate) = self.classify_update(update, fallback_update_key, received_at)?
        else {
            return Ok(false);
        };
        let mut state = lock_state(&self.state);
        let queue_payload = state
            .queues
            .get(&candidate.chat_key)
            .map(|queue| queue.iter().map(PendingUpdate::payload).collect::<Vec<_>>())
            .unwrap_or_default();
        let current_index = state.queues.get(&candidate.chat_key).and_then(|queue| {
            queue
                .iter()
                .position(|item| item.update_key == candidate.update_key)
        });
        let plan = self
            .planner
            .logical_turn(&json!({
                "stage": "discard_buffered_text_update",
                "candidate": candidate.payload(),
                "queue": queue_payload,
            }))
            .map_err(|_| planner_contract_error())?;
        validate_discard_plan(&plan, &candidate, current_index)?;
        let Some(index) = current_index else {
            return Ok(false);
        };
        let empty = {
            let queue = state
                .queues
                .get_mut(&candidate.chat_key)
                .ok_or_else(state_error)?;
            queue.remove(index).ok_or_else(state_error)?;
            queue.is_empty()
        };
        if empty {
            state.queues.remove(&candidate.chat_key);
        }
        state.discarded_count = state.discarded_count.saturating_add(1);
        Ok(true)
    }

    pub fn claim_update_once(
        &self,
        update: &JsonValue,
        fallback_update_key: &str,
    ) -> Result<TelegramLogicalTurnStep, TelegramLogicalTurnError> {
        if !self.config.merge_enabled {
            return Ok(TelegramLogicalTurnStep::Disabled);
        }
        let received_at = self.read_clock()?;
        let Some(candidate) = self.classify_update(update, fallback_update_key, received_at)?
        else {
            return Ok(TelegramLogicalTurnStep::NotCandidate);
        };
        self.claim_candidate_once(&candidate)
    }

    pub fn claim_update(
        &self,
        update: &JsonValue,
        fallback_update_key: &str,
    ) -> Result<TelegramLogicalTurnStep, TelegramLogicalTurnError> {
        if !self.config.merge_enabled {
            return Ok(TelegramLogicalTurnStep::Disabled);
        }
        let received_at = self.read_clock()?;
        let Some(candidate) = self.classify_update(update, fallback_update_key, received_at)?
        else {
            return Ok(TelegramLogicalTurnStep::NotCandidate);
        };
        loop {
            match self.claim_candidate_once(&candidate)? {
                TelegramLogicalTurnStep::Wait(duration) => {
                    self.sleeper.sleep(duration).map_err(|_| {
                        TelegramLogicalTurnError::new(TelegramLogicalTurnErrorKind::Sleeper)
                    })?
                }
                result => return Ok(result),
            }
        }
    }

    pub fn snapshot_json(&self) -> JsonValue {
        let state = lock_state(&self.state);
        let pending_update_count = state.queues.values().map(VecDeque::len).sum::<usize>();
        json!({
            "execution_contract": EXECUTION_CONTRACT,
            "migration_stage": EXECUTION_MIGRATION_STAGE,
            "transport": "telegram",
            "merge_enabled": self.config.merge_enabled,
            "merge_window_seconds": self.config.merge_window.as_secs_f64(),
            "poll_interval_seconds": self.config.poll_interval.as_secs_f64(),
            "max_messages": self.config.max_messages,
            "max_pending_chats": self.config.max_pending_chats,
            "max_pending_per_chat": self.config.max_pending_per_chat,
            "pending_chat_count": state.queues.len(),
            "pending_update_count": pending_update_count,
            "buffered_count": state.buffered_count,
            "duplicate_count": state.duplicate_count,
            "consumed_count": state.consumed_count,
            "pass_through_count": state.pass_through_count,
            "skipped_count": state.skipped_count,
            "discarded_count": state.discarded_count,
            "rejected_count": state.rejected_count,
            "rust_logical_turn_required": true,
            "python_logical_turn_allowed": false,
            "python_buffer_allowed": false,
            "python_sleep_allowed": false,
        })
    }
}

#[derive(Clone)]
struct PendingUpdate {
    update_key: String,
    chat_key: String,
    update: JsonValue,
    normalized_text: String,
    mergeable: bool,
    actor_identity: String,
    received_at: f64,
    telegram_message_id: Option<i64>,
}

impl PendingUpdate {
    fn payload(&self) -> JsonValue {
        json!({
            "update_key": self.update_key,
            "chat_key": self.chat_key,
            "normalized_text": self.normalized_text,
            "mergeable": self.mergeable,
            "actor_identity": self.actor_identity,
            "received_at": self.received_at,
            "telegram_message_id": self.telegram_message_id,
        })
    }
}

#[derive(Default)]
struct BufferState {
    queues: HashMap<String, VecDeque<PendingUpdate>>,
    buffered_count: u64,
    duplicate_count: u64,
    consumed_count: u64,
    pass_through_count: u64,
    skipped_count: u64,
    discarded_count: u64,
    rejected_count: u64,
}

struct RuntimeConfig {
    username: String,
    merge_window: Duration,
    max_messages: usize,
    poll_interval: Duration,
    max_pending_chats: usize,
    max_pending_per_chat: usize,
    merge_enabled: bool,
}

trait ExecutionPlanningPort: Send + Sync + 'static {
    fn logical_turn(&self, request: &JsonValue) -> Result<JsonValue, String>;
    fn update_dispatch(&self, request: &JsonValue) -> Result<JsonValue, String>;
    fn turn_input(&self, request: &JsonValue) -> Result<JsonValue, String>;
    fn workflow_query(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
struct NativeExecutionPlanningPort;

impl ExecutionPlanningPort for NativeExecutionPlanningPort {
    fn logical_turn(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_logical_turn_runtime_plan_json(request)
    }

    fn update_dispatch(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_update_dispatch_plan_json(request)
    }

    fn turn_input(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_turn_input_plan_json(request)
    }

    fn workflow_query(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_workflow_query_plan_json(request)
    }
}

fn validate_buffer_plan(
    plan: &JsonValue,
    candidate: &PendingUpdate,
    duplicate: bool,
) -> Result<(), TelegramLogicalTurnError> {
    let object = validate_logical_plan(plan, "buffer_submitted_text_update")?;
    if object.get("logical_turn_state").and_then(JsonValue::as_str)
        != Some(if duplicate { "duplicate" } else { "append" })
        || object.get("should_append").and_then(JsonValue::as_bool) != Some(!duplicate)
        || object.get("duplicate").and_then(JsonValue::as_bool) != Some(duplicate)
        || object.get("chat_key").and_then(JsonValue::as_str) != Some(candidate.chat_key.as_str())
        || object.get("update_key").and_then(JsonValue::as_str)
            != Some(candidate.update_key.as_str())
    {
        return Err(planner_contract_error());
    }
    let actions = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .ok_or_else(planner_contract_error)?;
    if duplicate {
        if !actions.is_empty() {
            return Err(planner_contract_error());
        }
    } else if actions.len() != 1
        || actions[0].get("kind").and_then(JsonValue::as_str) != Some("append_pending_text_update")
        || actions[0].get("chat_key").and_then(JsonValue::as_str)
            != Some(candidate.chat_key.as_str())
        || actions[0].get("update_key").and_then(JsonValue::as_str)
            != Some(candidate.update_key.as_str())
    {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn validate_discard_plan(
    plan: &JsonValue,
    candidate: &PendingUpdate,
    current_index: Option<usize>,
) -> Result<(), TelegramLogicalTurnError> {
    let object = validate_logical_plan(plan, "discard_buffered_text_update")?;
    let should_remove = current_index.is_some();
    if object.get("logical_turn_state").and_then(JsonValue::as_str)
        != Some(if should_remove { "discard" } else { "missing" })
        || object.get("should_remove").and_then(JsonValue::as_bool) != Some(should_remove)
        || object.get("chat_key").and_then(JsonValue::as_str) != Some(candidate.chat_key.as_str())
        || object.get("update_key").and_then(JsonValue::as_str)
            != Some(candidate.update_key.as_str())
        || object.get("current_index").and_then(json_usize) != current_index
    {
        return Err(planner_contract_error());
    }
    let actions = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .ok_or_else(planner_contract_error)?;
    if let Some(index) = current_index {
        if actions.len() != 1
            || actions[0].get("kind").and_then(JsonValue::as_str)
                != Some("discard_pending_text_update")
            || actions[0].get("chat_key").and_then(JsonValue::as_str)
                != Some(candidate.chat_key.as_str())
            || actions[0].get("update_key").and_then(JsonValue::as_str)
                != Some(candidate.update_key.as_str())
            || actions[0].get("current_index").and_then(json_usize) != Some(index)
        {
            return Err(planner_contract_error());
        }
    } else if !actions.is_empty() {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn validate_missing_claim(
    object: &Map<String, JsonValue>,
    candidate: &PendingUpdate,
) -> Result<(), TelegramLogicalTurnError> {
    if object.get("logical_turn_state").and_then(JsonValue::as_str) != Some("missing")
        || object.get("return_kind").and_then(JsonValue::as_str) != Some("skip")
    {
        return Err(planner_contract_error());
    }
    validate_claim_action(object, "skip_missing_logical_turn", candidate)
}

fn validate_pass_through_claim(
    object: &Map<String, JsonValue>,
    candidate: &PendingUpdate,
    index: usize,
) -> Result<(), TelegramLogicalTurnError> {
    if object.get("logical_turn_state").and_then(JsonValue::as_str) != Some("non_mergeable")
        || object.get("return_kind").and_then(JsonValue::as_str) != Some("pass_through")
        || object.get("current_index").and_then(json_usize) != Some(index)
        || object.get("should_remove").and_then(JsonValue::as_bool) != Some(true)
    {
        return Err(planner_contract_error());
    }
    validate_claim_action(object, "remove_pending_text_update", candidate)
}

fn validate_wait_claim(
    object: &Map<String, JsonValue>,
    candidate: &PendingUpdate,
    index: usize,
    config: &RuntimeConfig,
) -> Result<Duration, TelegramLogicalTurnError> {
    if object.get("return_kind").and_then(JsonValue::as_str) != Some("wait")
        || object.get("current_index").and_then(json_usize) != Some(index)
        || object.get("should_wait").and_then(JsonValue::as_bool) != Some(true)
    {
        return Err(planner_contract_error());
    }
    validate_claim_action(object, "wait_for_quiet_window", candidate)?;
    let seconds = object
        .get("sleep_for_seconds")
        .and_then(JsonValue::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(planner_contract_error)?;
    if seconds > config.poll_interval.as_secs_f64() + f64::EPSILON {
        return Err(planner_contract_error());
    }
    let action_seconds = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .and_then(|actions| actions.first())
        .and_then(|action| action.get("sleep_for_seconds"))
        .and_then(JsonValue::as_f64)
        .ok_or_else(planner_contract_error)?;
    if (seconds - action_seconds).abs() > f64::EPSILON {
        return Err(planner_contract_error());
    }
    let duration = Duration::try_from_secs_f64(seconds).map_err(|_| state_error())?;
    Ok(if duration.is_zero() && seconds > 0.0 {
        Duration::from_nanos(1)
    } else {
        duration
    })
}

fn build_and_consume_turn(
    object: &Map<String, JsonValue>,
    candidate: &PendingUpdate,
    index: usize,
    state: &mut BufferState,
    max_messages: usize,
) -> Result<(TelegramLogicalTurn, usize), TelegramLogicalTurnError> {
    if object.get("return_kind").and_then(JsonValue::as_str) != Some("logical_turn")
        || object.get("current_index").and_then(json_usize) != Some(index)
        || object.get("should_emit").and_then(JsonValue::as_bool) != Some(true)
    {
        return Err(planner_contract_error());
    }
    let consume_count = object
        .get("consume_count")
        .and_then(json_usize)
        .filter(|count| (1..=max_messages).contains(count))
        .ok_or_else(planner_contract_error)?;
    let selected = state
        .queues
        .get(&candidate.chat_key)
        .ok_or_else(state_error)?
        .iter()
        .skip(index)
        .take(consume_count)
        .cloned()
        .collect::<Vec<_>>();
    if selected.len() != consume_count
        || selected.iter().any(|item| !item.mergeable)
        || selected
            .iter()
            .any(|item| item.actor_identity != selected[0].actor_identity)
    {
        return Err(planner_contract_error());
    }
    let text = selected
        .iter()
        .map(|item| item.normalized_text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
        .trim()
        .to_string();
    let message_ids = selected
        .iter()
        .filter_map(|item| item.telegram_message_id)
        .collect::<Vec<_>>();
    let telegram_message_id = message_ids.last().copied();
    let logical_turn = object
        .get("logical_turn")
        .and_then(JsonValue::as_object)
        .ok_or_else(planner_contract_error)?;
    if logical_turn.get("text").and_then(JsonValue::as_str) != Some(text.as_str())
        || logical_turn
            .get("actor_identity")
            .and_then(JsonValue::as_str)
            != Some(selected[0].actor_identity.as_str())
        || logical_turn.get("telegram_message_id").and_then(json_i64) != telegram_message_id
        || json_i64_array(logical_turn.get("telegram_message_ids")) != Some(message_ids.clone())
    {
        return Err(planner_contract_error());
    }
    let actions = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .filter(|actions| actions.len() == 2)
        .ok_or_else(planner_contract_error)?;
    if actions[0].get("kind").and_then(JsonValue::as_str) != Some("consume_logical_turn")
        || actions[0].get("start_index").and_then(json_usize) != Some(index)
        || actions[0].get("consume_count").and_then(json_usize) != Some(consume_count)
        || actions[1].get("kind").and_then(JsonValue::as_str) != Some("build_logical_turn")
        || actions[1].get("text").and_then(JsonValue::as_str) != Some(text.as_str())
    {
        return Err(planner_contract_error());
    }

    let empty = {
        let queue = state
            .queues
            .get_mut(&candidate.chat_key)
            .ok_or_else(state_error)?;
        queue.drain(index..index + consume_count);
        queue.is_empty()
    };
    if empty {
        state.queues.remove(&candidate.chat_key);
    }
    Ok((
        TelegramLogicalTurn {
            update: selected[0].update.clone(),
            text,
            actor_identity: selected[0].actor_identity.clone(),
            telegram_message_id,
            telegram_message_ids: message_ids,
        },
        consume_count,
    ))
}

fn validate_claim_action(
    object: &Map<String, JsonValue>,
    expected_kind: &str,
    candidate: &PendingUpdate,
) -> Result<(), TelegramLogicalTurnError> {
    let actions = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .filter(|actions| actions.len() == 1)
        .ok_or_else(planner_contract_error)?;
    if actions[0].get("kind").and_then(JsonValue::as_str) != Some(expected_kind)
        || actions[0].get("chat_key").and_then(JsonValue::as_str)
            != Some(candidate.chat_key.as_str())
        || actions[0].get("update_key").and_then(JsonValue::as_str)
            != Some(candidate.update_key.as_str())
    {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn validate_logical_plan<'a>(
    plan: &'a JsonValue,
    stage: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramLogicalTurnError> {
    let object = plan.as_object().ok_or_else(planner_contract_error)?;
    if object
        .get("logical_turn_runtime_contract")
        .and_then(JsonValue::as_str)
        != Some(LOGICAL_PLAN_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(LOGICAL_PLAN_MIGRATION_STAGE)
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object.get("stage").and_then(JsonValue::as_str) != Some(stage)
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_logical_turn_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(planner_contract_error());
    }
    Ok(object)
}

fn validate_turn_input_plan<'a>(
    plan: &'a JsonValue,
    kind: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramLogicalTurnError> {
    let object = plan.as_object().ok_or_else(planner_contract_error)?;
    if object
        .get("turn_input_contract")
        .and_then(JsonValue::as_str)
        != Some(TURN_INPUT_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(TURN_INPUT_MIGRATION_STAGE)
        || object.get("kind").and_then(JsonValue::as_str) != Some(kind)
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_turn_input_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(planner_contract_error());
    }
    Ok(object)
}

fn validate_workflow_query_plan<'a>(
    plan: &'a JsonValue,
    kind: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramLogicalTurnError> {
    let object = plan.as_object().ok_or_else(planner_contract_error)?;
    if object
        .get("workflow_query_contract")
        .and_then(JsonValue::as_str)
        != Some(WORKFLOW_QUERY_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(WORKFLOW_QUERY_MIGRATION_STAGE)
        || object.get("kind").and_then(JsonValue::as_str) != Some(kind)
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_workflow_query_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(planner_contract_error());
    }
    Ok(object)
}

fn validate_single_action(
    object: &Map<String, JsonValue>,
    kind: &str,
) -> Result<(), TelegramLogicalTurnError> {
    if object
        .get("actions")
        .and_then(JsonValue::as_array)
        .filter(|actions| actions.len() == 1)
        .and_then(|actions| actions[0].get("kind"))
        .and_then(JsonValue::as_str)
        != Some(kind)
    {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn normalize_identity(
    value: &str,
    kind: TelegramLogicalTurnErrorKind,
) -> Result<String, TelegramLogicalTurnError> {
    let value = value.trim();
    if value.is_empty()
        || value.chars().count() > MAX_IDENTITY_LENGTH
        || value.chars().any(char::is_control)
    {
        return Err(TelegramLogicalTurnError::new(kind));
    }
    Ok(value.to_string())
}

fn scalar_identity_text(value: &JsonValue) -> Result<String, TelegramLogicalTurnError> {
    match value {
        JsonValue::String(value) => {
            normalize_identity(value, TelegramLogicalTurnErrorKind::InvalidUpdate)
        }
        JsonValue::Number(value) => normalize_identity(
            &value.to_string(),
            TelegramLogicalTurnErrorKind::InvalidUpdate,
        ),
        _ => Err(TelegramLogicalTurnError::new(
            TelegramLogicalTurnErrorKind::InvalidUpdate,
        )),
    }
}

fn json_usize(value: &JsonValue) -> Option<usize> {
    value.as_u64().and_then(|value| usize::try_from(value).ok())
}

fn json_i64(value: &JsonValue) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
}

fn json_i64_array(value: Option<&JsonValue>) -> Option<Vec<i64>> {
    value?
        .as_array()?
        .iter()
        .map(json_i64)
        .collect::<Option<Vec<_>>>()
}

fn configuration_error() -> TelegramLogicalTurnError {
    TelegramLogicalTurnError::new(TelegramLogicalTurnErrorKind::Configuration)
}

fn planner_contract_error() -> TelegramLogicalTurnError {
    TelegramLogicalTurnError::new(TelegramLogicalTurnErrorKind::PlannerContract)
}

fn state_error() -> TelegramLogicalTurnError {
    TelegramLogicalTurnError::new(TelegramLogicalTurnErrorKind::State)
}

fn lock_state(mutex: &Mutex<BufferState>) -> MutexGuard<'_, BufferState> {
    match mutex.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

impl TelegramLogicalTurnRuntime {
    fn claim_candidate_once(
        &self,
        candidate: &PendingUpdate,
    ) -> Result<TelegramLogicalTurnStep, TelegramLogicalTurnError> {
        let now = self.read_clock()?;
        let mut state = lock_state(&self.state);
        let queue_payload = state
            .queues
            .get(&candidate.chat_key)
            .map(|queue| queue.iter().map(PendingUpdate::payload).collect::<Vec<_>>())
            .unwrap_or_default();
        let current_index = state.queues.get(&candidate.chat_key).and_then(|queue| {
            queue
                .iter()
                .position(|item| item.update_key == candidate.update_key)
        });
        let plan = self
            .planner
            .logical_turn(&json!({
                "stage": "claim_logical_turn",
                "candidate": {
                    "chat_key": candidate.chat_key,
                    "update_key": candidate.update_key,
                },
                "queue": queue_payload,
                "merge_window_seconds": self.config.merge_window.as_secs_f64(),
                "poll_interval_seconds": self.config.poll_interval.as_secs_f64(),
                "max_messages": self.config.max_messages,
                "now_monotonic_seconds": now,
            }))
            .map_err(|_| planner_contract_error())?;
        let object = validate_logical_plan(&plan, "claim_logical_turn")?;

        let result = match current_index {
            None => {
                validate_missing_claim(object, candidate)?;
                state.skipped_count = state.skipped_count.saturating_add(1);
                TelegramLogicalTurnStep::Skip
            }
            Some(index) => {
                let mergeable = state
                    .queues
                    .get(&candidate.chat_key)
                    .and_then(|queue| queue.get(index))
                    .is_some_and(|item| item.mergeable);
                if !mergeable {
                    validate_pass_through_claim(object, candidate, index)?;
                    let empty = {
                        let queue = state
                            .queues
                            .get_mut(&candidate.chat_key)
                            .ok_or_else(state_error)?;
                        queue.remove(index).ok_or_else(state_error)?;
                        queue.is_empty()
                    };
                    if empty {
                        state.queues.remove(&candidate.chat_key);
                    }
                    state.pass_through_count = state.pass_through_count.saturating_add(1);
                    TelegramLogicalTurnStep::PassThrough
                } else {
                    match object.get("logical_turn_state").and_then(JsonValue::as_str) {
                        Some("wait") => TelegramLogicalTurnStep::Wait(validate_wait_claim(
                            object,
                            candidate,
                            index,
                            &self.config,
                        )?),
                        Some("emit") => {
                            let (turn, consumed) = build_and_consume_turn(
                                object,
                                candidate,
                                index,
                                &mut state,
                                self.config.max_messages,
                            )?;
                            state.consumed_count = state
                                .consumed_count
                                .saturating_add(u64::try_from(consumed).unwrap_or(u64::MAX));
                            TelegramLogicalTurnStep::LogicalTurn(turn)
                        }
                        _ => return Err(planner_contract_error()),
                    }
                }
            }
        };
        Ok(result)
    }

    fn classify_update(
        &self,
        update: &JsonValue,
        fallback_update_key: &str,
        received_at: f64,
    ) -> Result<Option<PendingUpdate>, TelegramLogicalTurnError> {
        let update_object = update.as_object().ok_or_else(|| {
            TelegramLogicalTurnError::new(TelegramLogicalTurnErrorKind::InvalidUpdate)
        })?;
        let fallback_update_key = normalize_identity(
            fallback_update_key,
            TelegramLogicalTurnErrorKind::InvalidFallbackKey,
        )?;
        let metadata = self
            .planner
            .logical_turn(&json!({
                "stage": "candidate_metadata",
                "update": update,
            }))
            .map_err(|_| planner_contract_error())?;
        let metadata = validate_logical_plan(&metadata, "candidate_metadata")?;
        validate_single_action(metadata, "classify_pending_text_update")?;
        if metadata
            .get("is_text_candidate")
            .and_then(JsonValue::as_bool)
            != Some(true)
        {
            if metadata
                .get("logical_turn_state")
                .and_then(JsonValue::as_str)
                != Some("not_candidate")
            {
                return Err(planner_contract_error());
            }
            return Ok(None);
        }
        let raw_text = metadata
            .get("raw_text")
            .and_then(JsonValue::as_str)
            .ok_or_else(planner_contract_error)?;
        let chat_id = metadata.get("chat_id").cloned().unwrap_or(JsonValue::Null);
        if chat_id.is_null() {
            return Ok(None);
        }
        let chat_id_text = scalar_identity_text(&chat_id)?;
        let telegram_message_id = metadata.get("telegram_message_id").and_then(json_i64);

        let dispatch = self
            .planner
            .update_dispatch(&json!({
                "update": update,
                "fallback_update_key": fallback_update_key,
            }))
            .map_err(|_| planner_contract_error())?;
        let dispatch = dispatch.as_object().ok_or_else(planner_contract_error)?;
        let update_key = normalize_identity(
            dispatch
                .get("update_key")
                .and_then(JsonValue::as_str)
                .ok_or_else(planner_contract_error)?,
            TelegramLogicalTurnErrorKind::InvalidUpdate,
        )?;
        let chat_key = normalize_identity(
            dispatch
                .get("dispatch_key")
                .and_then(JsonValue::as_str)
                .ok_or_else(planner_contract_error)?,
            TelegramLogicalTurnErrorKind::InvalidUpdate,
        )?;

        let normalized_plan = self
            .planner
            .turn_input(&json!({
                "kind": "normalize_user_text",
                "text": raw_text,
                "username": self.config.username,
            }))
            .map_err(|_| planner_contract_error())?;
        let normalized_plan = validate_turn_input_plan(&normalized_plan, "normalize_user_text")?;
        let normalized_text = normalized_plan
            .get("text")
            .and_then(JsonValue::as_str)
            .ok_or_else(planner_contract_error)?
            .to_string();

        let command_plan = self
            .planner
            .workflow_query(&json!({
                "kind": "parse_command",
                "text": raw_text,
                "username": self.config.username,
            }))
            .map_err(|_| planner_contract_error())?;
        let command_plan = validate_workflow_query_plan(&command_plan, "parse_command")?;
        let command_present =
            command_plan.get("matched").and_then(JsonValue::as_bool) == Some(true);
        let command = command_plan
            .get("command")
            .cloned()
            .unwrap_or(JsonValue::Null);

        let workflow_plan = self
            .planner
            .workflow_query(&json!({
                "kind": "detect_workflow_query",
                "text": normalized_text,
            }))
            .map_err(|_| planner_contract_error())?;
        let workflow_plan = validate_workflow_query_plan(&workflow_plan, "detect_workflow_query")?;
        let workflow_query_present =
            workflow_plan.get("matched").and_then(JsonValue::as_bool) == Some(true);
        let workflow_query = workflow_plan
            .get("workflow_query")
            .cloned()
            .unwrap_or(JsonValue::Null);

        let message = update_object.get("message").and_then(JsonValue::as_object);
        let from_user = message
            .and_then(|message| message.get("from"))
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let actor_plan = self
            .planner
            .workflow_query(&json!({
                "kind": "actor_identity",
                "from_user": from_user,
                "chat_id": chat_id_text,
            }))
            .map_err(|_| planner_contract_error())?;
        let actor_plan = validate_workflow_query_plan(&actor_plan, "actor_identity")?;
        let actor_identity = actor_plan
            .get("text")
            .and_then(JsonValue::as_str)
            .ok_or_else(planner_contract_error)?
            .to_string();

        let classified = self
            .planner
            .logical_turn(&json!({
                "stage": "classify_pending_text_update",
                "update": update,
                "update_key": update_key,
                "chat_key": chat_key,
                "normalized_text": normalized_text,
                "actor_identity": actor_identity,
                "received_at": received_at,
                "command": command,
                "workflow_query": workflow_query,
            }))
            .map_err(|_| planner_contract_error())?;
        let classified = validate_logical_plan(&classified, "classify_pending_text_update")?;
        if classified
            .get("logical_turn_state")
            .and_then(JsonValue::as_str)
            != Some("classified_candidate")
            || classified
                .get("is_text_candidate")
                .and_then(JsonValue::as_bool)
                != Some(true)
            || classified
                .get("command_present")
                .and_then(JsonValue::as_bool)
                != Some(command_present)
            || classified
                .get("workflow_query_present")
                .and_then(JsonValue::as_bool)
                != Some(workflow_query_present)
        {
            return Err(planner_contract_error());
        }
        let candidate = classified
            .get("candidate")
            .and_then(JsonValue::as_object)
            .ok_or_else(planner_contract_error)?;
        let mergeable = !normalized_text.is_empty() && !command_present && !workflow_query_present;
        if candidate.get("update_key").and_then(JsonValue::as_str) != Some(update_key.as_str())
            || candidate.get("chat_key").and_then(JsonValue::as_str) != Some(chat_key.as_str())
            || candidate.get("normalized_text").and_then(JsonValue::as_str)
                != Some(normalized_text.as_str())
            || candidate.get("actor_identity").and_then(JsonValue::as_str)
                != Some(actor_identity.as_str())
            || candidate.get("mergeable").and_then(JsonValue::as_bool) != Some(mergeable)
            || candidate.get("telegram_message_id").and_then(json_i64) != telegram_message_id
        {
            return Err(planner_contract_error());
        }

        Ok(Some(PendingUpdate {
            update_key,
            chat_key,
            update: update.clone(),
            normalized_text,
            mergeable,
            actor_identity,
            received_at,
            telegram_message_id,
        }))
    }

    fn read_clock(&self) -> Result<f64, TelegramLogicalTurnError> {
        let now = self
            .clock
            .now_monotonic_seconds()
            .map_err(|_| TelegramLogicalTurnError::new(TelegramLogicalTurnErrorKind::Clock))?;
        if !now.is_finite() || now < 0.0 {
            return Err(TelegramLogicalTurnError::new(
                TelegramLogicalTurnErrorKind::Clock,
            ));
        }
        Ok(now)
    }
}

#[cfg(test)]
mod tests;
