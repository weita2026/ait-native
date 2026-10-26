use std::fmt;
use std::path::{Path, PathBuf};

use ait_core::json_support::{json, JsonCodec, JsonEncodeOptions, JsonMap as Map, JsonValue};
use chrono::{SecondsFormat, Utc};

use super::planning::TelegramReplySpoolPlanner;
use crate::runtime::AgentRuntimeBindingStore;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramReplySpoolExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_reply_spool_execution";
const PLANNING_KIND: &str = "telegram_reply_spool";
const DEFAULT_SPOOL_LIMIT: i64 = 100;
const MAX_SPOOL_LIMIT: i64 = 10_000;
const MAX_CONVERSATION_KEY_LENGTH: usize = 4_096;
const MAX_CHAT_TEXT_LENGTH: usize = 512;
const MAX_PENDING_TEXT_LENGTH: usize = 262_144;
const MAX_ACTOR_IDENTITY_LENGTH: usize = 4_096;
const MAX_ERROR_TEXT_LENGTH: usize = 262_144;
const MAX_EVENT_BYTES: usize = 2 * 1_048_576;
const MAX_CLOCK_TEXT_LENGTH: usize = 128;

pub type TelegramReplySpoolMutation<'a> =
    dyn FnMut(Option<&JsonValue>) -> Result<Option<JsonValue>, String> + 'a;

pub trait TelegramReplySpoolStatePort: Send + Sync + 'static {
    fn load_link(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String>;

    fn mutate_link(
        &self,
        chat_id: &JsonValue,
        mutation: &mut TelegramReplySpoolMutation<'_>,
    ) -> Result<Option<JsonValue>, String>;
}

pub trait TelegramReplySpoolClockPort: Send + Sync + 'static {
    fn now_iso(&self) -> Result<String, String>;
}

#[derive(Debug, Clone)]
pub struct RuntimeBindingTelegramReplySpoolStatePort {
    store: AgentRuntimeBindingStore,
}

impl RuntimeBindingTelegramReplySpoolStatePort {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::from_store(AgentRuntimeBindingStore::new(path))
    }

    pub fn from_store(store: AgentRuntimeBindingStore) -> Self {
        Self { store }
    }

    pub fn path(&self) -> &Path {
        self.store.path()
    }
}

impl TelegramReplySpoolStatePort for RuntimeBindingTelegramReplySpoolStatePort {
    fn load_link(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        let value = self.store.execute(
            "get_binding",
            &json!({
                "transport": "telegram",
                "surface_id": chat_id,
            }),
        )?;
        match value {
            JsonValue::Null => Ok(None),
            JsonValue::Object(_) => Ok(Some(value)),
            _ => Err("Runtime binding store returned an invalid Telegram link.".to_string()),
        }
    }

    fn mutate_link(
        &self,
        chat_id: &JsonValue,
        mutation: &mut TelegramReplySpoolMutation<'_>,
    ) -> Result<Option<JsonValue>, String> {
        self.store
            .mutate_binding_with("telegram", chat_id, mutation)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTelegramReplySpoolClockPort;

impl TelegramReplySpoolClockPort for SystemTelegramReplySpoolClockPort {
    fn now_iso(&self) -> Result<String, String> {
        Ok(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct TelegramReplySpoolEntries {
    entries: Vec<JsonValue>,
}

impl TelegramReplySpoolEntries {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &JsonValue> {
        self.entries.iter()
    }

    pub fn into_entries(self) -> Vec<JsonValue> {
        self.entries
    }
}

impl fmt::Debug for TelegramReplySpoolEntries {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramReplySpoolEntries")
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TelegramReplySpoolExecutionErrorKind {
    InvalidRequest,
    Clock,
    State,
    Planner,
    PlannerContract,
}

impl TelegramReplySpoolExecutionErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::Clock => "clock",
            Self::State => "state",
            Self::Planner => "planner",
            Self::PlannerContract => "planner_contract",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TelegramReplySpoolExecutionError {
    kind: TelegramReplySpoolExecutionErrorKind,
}

impl TelegramReplySpoolExecutionError {
    pub fn kind(self) -> TelegramReplySpoolExecutionErrorKind {
        self.kind
    }

    fn new(kind: TelegramReplySpoolExecutionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for TelegramReplySpoolExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TelegramReplySpoolExecutionErrorKind::InvalidRequest => {
                "Telegram reply spool execution request is invalid."
            }
            TelegramReplySpoolExecutionErrorKind::Clock => "Telegram reply spool clock failed.",
            TelegramReplySpoolExecutionErrorKind::State => {
                "Telegram reply spool state operation failed."
            }
            TelegramReplySpoolExecutionErrorKind::Planner => {
                "Telegram reply spool planning failed."
            }
            TelegramReplySpoolExecutionErrorKind::PlannerContract => {
                "Telegram reply spool planner contract is invalid."
            }
        })
    }
}

impl std::error::Error for TelegramReplySpoolExecutionError {}

pub fn execute_with_telegram_reply_spool_ports<P, S, C>(
    planner: &P,
    state: &S,
    clock: &C,
    request: &JsonValue,
) -> Result<JsonValue, TelegramReplySpoolExecutionError>
where
    P: TelegramReplySpoolPlanner + ?Sized,
    S: TelegramReplySpoolStatePort + ?Sized,
    C: TelegramReplySpoolClockPort + ?Sized,
{
    let object = request.as_object().ok_or_else(|| error(InvalidRequest))?;
    let stage = clean_text(object.get("stage")).ok_or_else(|| error(InvalidRequest))?;
    match stage.as_str() {
        "entries" => execute_entries(planner, state, object),
        "remember" | "clear" => execute_mutation(planner, state, clock, object, &stage),
        _ => Err(error(InvalidRequest)),
    }
}

pub fn load_telegram_reply_spool_entries<P, S>(
    planner: &P,
    state: &S,
    chat_id: &JsonValue,
) -> Result<TelegramReplySpoolEntries, TelegramReplySpoolExecutionError>
where
    P: TelegramReplySpoolPlanner + ?Sized,
    S: TelegramReplySpoolStatePort + ?Sized,
{
    validate_chat_id(chat_id)?;
    let link = state.load_link(chat_id).map_err(|_| error(State))?;
    let planned = planner
        .plan_json(&json!({
            "stage": "entries",
            "link": link,
        }))
        .map_err(|_| error(Planner))?;
    let entries = validate_entries_plan(&planned)?;
    Ok(TelegramReplySpoolEntries { entries })
}

fn execute_entries<P, S>(
    planner: &P,
    state: &S,
    object: &Map<String, JsonValue>,
) -> Result<JsonValue, TelegramReplySpoolExecutionError>
where
    P: TelegramReplySpoolPlanner + ?Sized,
    S: TelegramReplySpoolStatePort + ?Sized,
{
    let chat_id = object.get("chat_id").ok_or_else(|| error(InvalidRequest))?;
    let entries = load_telegram_reply_spool_entries(planner, state, chat_id)?;
    Ok(outcome("entries", false, entries.len(), "loaded"))
}

fn execute_mutation<P, S, C>(
    planner: &P,
    state: &S,
    clock: &C,
    object: &Map<String, JsonValue>,
    stage: &str,
) -> Result<JsonValue, TelegramReplySpoolExecutionError>
where
    P: TelegramReplySpoolPlanner + ?Sized,
    S: TelegramReplySpoolStatePort + ?Sized,
    C: TelegramReplySpoolClockPort + ?Sized,
{
    let pending_turn = validate_pending_turn(object.get("pending_turn"))?;
    let chat_id = pending_turn
        .get("chat_id")
        .cloned()
        .ok_or_else(|| error(InvalidRequest))?;
    let spool_limit = if stage == "remember" {
        validate_spool_limit(object.get("spool_limit"))?
    } else {
        MAX_SPOOL_LIMIT
    };
    let now_iso = if stage == "remember" {
        let value = clock.now_iso().map_err(|_| error(Clock))?;
        validate_clock_text(&value)?;
        Some(value)
    } else {
        None
    };
    let status = if stage == "remember" {
        Some(validate_status(object.get("status"))?)
    } else {
        None
    };
    validate_optional_bool(object.get("attempt_increment"))?;
    validate_optional_text(object.get("last_error"), MAX_ERROR_TEXT_LENGTH)?;
    validate_optional_text(object.get("ready_reply_text"), MAX_PENDING_TEXT_LENGTH)?;
    validate_optional_object_size(object.get("user_event"), MAX_EVENT_BYTES)?;
    validate_optional_object_size(object.get("assistant_event"), MAX_EVENT_BYTES)?;
    validate_optional_object_size(object.get("provider_thread"), MAX_EVENT_BYTES)?;
    validate_optional_object_size(object.get("turn_telemetry"), MAX_EVENT_BYTES)?;

    let mut summary: Option<MutationSummary> = None;
    let mut closure_error_kind = None;
    let mut mutation = |current_link: Option<&JsonValue>| -> Result<Option<JsonValue>, String> {
        let mut planner_request = Map::new();
        planner_request.insert("stage".to_string(), json!(stage));
        planner_request.insert(
            "pending_turn".to_string(),
            JsonValue::Object(pending_turn.clone()),
        );
        planner_request.insert(
            "current_link".to_string(),
            current_link.cloned().unwrap_or(JsonValue::Null),
        );
        planner_request.insert("spool_limit".to_string(), json!(spool_limit));
        for key in [
            "attempt_increment",
            "last_error",
            "user_event",
            "assistant_event",
            "ready_reply_text",
            "provider_thread",
            "turn_telemetry",
        ] {
            if let Some(value) = object.get(key) {
                planner_request.insert(key.to_string(), value.clone());
            }
        }
        if let Some(status) = status.as_ref() {
            planner_request.insert("status".to_string(), json!(status));
        }
        if let Some(now_iso) = now_iso.as_ref() {
            planner_request.insert("now_iso".to_string(), json!(now_iso));
        }

        let planned = match planner.plan_json(&JsonValue::Object(planner_request)) {
            Ok(value) => value,
            Err(_) => {
                closure_error_kind = Some(Planner);
                return Err("Telegram reply spool planning failed.".to_string());
            }
        };
        let validated = match validate_mutation_plan(&planned, stage, spool_limit) {
            Ok(value) => value,
            Err(_) => {
                closure_error_kind = Some(PlannerContract);
                return Err("Telegram reply spool planner contract is invalid.".to_string());
            }
        };
        summary = Some(MutationSummary {
            applied: validated.patch.is_some(),
            entry_count: validated.entry_count,
            reason: validated.reason,
        });
        Ok(validated.patch)
    };

    state
        .mutate_link(&chat_id, &mut mutation)
        .map_err(|_| error(closure_error_kind.unwrap_or(State)))?;
    let summary = summary.ok_or_else(|| error(State))?;
    Ok(outcome(
        stage,
        summary.applied,
        summary.entry_count,
        summary.reason,
    ))
}

#[derive(Debug)]
struct ValidatedMutationPlan {
    patch: Option<JsonValue>,
    entry_count: usize,
    reason: &'static str,
}

#[derive(Debug)]
struct MutationSummary {
    applied: bool,
    entry_count: usize,
    reason: &'static str,
}

fn validate_mutation_plan(
    planned: &JsonValue,
    expected_stage: &str,
    spool_limit: i64,
) -> Result<ValidatedMutationPlan, TelegramReplySpoolExecutionError> {
    let object = planned.as_object().ok_or_else(|| error(PlannerContract))?;
    if clean_text(object.get("stage")).as_deref() != Some(expected_stage)
        || clean_text(object.get("execution_kind")).as_deref() != Some(PLANNING_KIND)
    {
        return Err(error(PlannerContract));
    }
    let patch_required = object
        .get("patch_required")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(PlannerContract))?;
    let result = object
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| error(PlannerContract))?;
    if clean_text(result.get("execution_kind")).as_deref() != Some(PLANNING_KIND)
        || result.get("patch_required").and_then(JsonValue::as_bool) != Some(patch_required)
    {
        return Err(error(PlannerContract));
    }
    if !patch_required {
        if !object.get("patch_payload").is_none_or(JsonValue::is_null)
            || !result.get("patch_payload").is_none_or(JsonValue::is_null)
        {
            return Err(error(PlannerContract));
        }
        let reason = match clean_text(result.get("reason")).as_deref() {
            Some("missing_current_link") => "missing_link",
            Some("conversation_mismatch") => "conversation_mismatch",
            _ => return Err(error(PlannerContract)),
        };
        return Ok(ValidatedMutationPlan {
            patch: None,
            entry_count: 0,
            reason,
        });
    }

    let patch = object
        .get("patch_payload")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| error(PlannerContract))?;
    if patch.len() != 1 || !patch.contains_key("telegram_reply_spool") {
        return Err(error(PlannerContract));
    }
    let entries = validate_entry_array(patch.get("telegram_reply_spool"), spool_limit)?;
    if object.get("entries") != Some(&JsonValue::Array(entries.clone()))
        || result.get("entries") != Some(&JsonValue::Array(entries.clone()))
        || result.get("patch_payload") != object.get("patch_payload")
    {
        return Err(error(PlannerContract));
    }
    Ok(ValidatedMutationPlan {
        patch: Some(JsonValue::Object(patch.clone())),
        entry_count: entries.len(),
        reason: "updated",
    })
}

fn validate_entries_plan(
    planned: &JsonValue,
) -> Result<Vec<JsonValue>, TelegramReplySpoolExecutionError> {
    let object = planned.as_object().ok_or_else(|| error(PlannerContract))?;
    if clean_text(object.get("stage")).as_deref() != Some("entries")
        || clean_text(object.get("execution_kind")).as_deref() != Some(PLANNING_KIND)
    {
        return Err(error(PlannerContract));
    }
    let entries = validate_entry_array(object.get("entries"), MAX_SPOOL_LIMIT)?;
    let entry_count = object
        .get("entry_count")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| error(PlannerContract))?;
    let result = object
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| error(PlannerContract))?;
    if entry_count != entries.len()
        || result.get("entries") != Some(&JsonValue::Array(entries.clone()))
        || result.get("entry_count").and_then(JsonValue::as_u64) != Some(entries.len() as u64)
        || clean_text(result.get("execution_kind")).as_deref() != Some(PLANNING_KIND)
    {
        return Err(error(PlannerContract));
    }
    Ok(entries)
}

fn validate_entry_array(
    value: Option<&JsonValue>,
    spool_limit: i64,
) -> Result<Vec<JsonValue>, TelegramReplySpoolExecutionError> {
    let entries = value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| error(PlannerContract))?;
    if entries.len() > spool_limit.max(0) as usize {
        return Err(error(PlannerContract));
    }
    if entries.iter().any(|entry| !valid_spool_entry(entry)) {
        return Err(error(PlannerContract));
    }
    Ok(entries.clone())
}

fn valid_spool_entry(entry: &JsonValue) -> bool {
    let Some(entry) = entry.as_object() else {
        return false;
    };
    bounded_text(entry.get("spool_key"), 1, MAX_PENDING_TEXT_LENGTH).is_some()
        && bounded_text(
            entry.get("conversation_key"),
            1,
            MAX_CONVERSATION_KEY_LENGTH,
        )
        .is_some()
        && validate_chat_id_value(entry.get("chat_id"))
        && matches!(
            clean_text(entry.get("status")).as_deref(),
            Some("queued" | "attempting" | "ready" | "failed")
        )
        && entry
            .get("attempt_count")
            .and_then(JsonValue::as_i64)
            .is_some_and(|value| value >= 0)
}

fn validate_pending_turn(
    value: Option<&JsonValue>,
) -> Result<Map<String, JsonValue>, TelegramReplySpoolExecutionError> {
    let pending = value
        .and_then(JsonValue::as_object)
        .cloned()
        .ok_or_else(|| error(InvalidRequest))?;
    if bounded_text(
        pending.get("conversation_key"),
        1,
        MAX_CONVERSATION_KEY_LENGTH,
    )
    .is_none()
        || !validate_chat_id_value(pending.get("chat_id"))
        || bounded_text(pending.get("chat_title"), 0, MAX_CHAT_TEXT_LENGTH).is_none()
        || bounded_text(pending.get("actor_identity"), 1, MAX_ACTOR_IDENTITY_LENGTH).is_none()
        || bounded_text(pending.get("text"), 1, MAX_PENDING_TEXT_LENGTH).is_none()
    {
        return Err(error(InvalidRequest));
    }
    if let Some(value) = pending.get("chat_type").filter(|value| !value.is_null()) {
        if bounded_text(Some(value), 1, MAX_CHAT_TEXT_LENGTH).is_none() {
            return Err(error(InvalidRequest));
        }
    }
    validate_message_id(pending.get("telegram_message_id"))?;
    if let Some(ids) = pending.get("telegram_message_ids") {
        let ids = ids.as_array().ok_or_else(|| error(InvalidRequest))?;
        if ids.len() > 1_000 {
            return Err(error(InvalidRequest));
        }
        for value in ids {
            validate_message_id(Some(value))?;
        }
    }
    validate_optional_object_size(pending.get("transport_envelope"), MAX_EVENT_BYTES)?;
    validate_optional_object_size(pending.get("watch_spec"), MAX_EVENT_BYTES)?;
    validate_optional_text(pending.get("ready_reply_text"), MAX_PENDING_TEXT_LENGTH)?;
    validate_optional_object_size(pending.get("provider_thread"), MAX_EVENT_BYTES)?;
    validate_optional_object_size(pending.get("turn_telemetry"), MAX_EVENT_BYTES)?;
    Ok(pending)
}

fn validate_message_id(value: Option<&JsonValue>) -> Result<(), TelegramReplySpoolExecutionError> {
    if value.is_none_or(JsonValue::is_null) {
        return Ok(());
    }
    value
        .and_then(JsonValue::as_i64)
        .filter(|value| *value > 0)
        .map(|_| ())
        .ok_or_else(|| error(InvalidRequest))
}

fn validate_chat_id(value: &JsonValue) -> Result<(), TelegramReplySpoolExecutionError> {
    validate_chat_id_value(Some(value))
        .then_some(())
        .ok_or_else(|| error(InvalidRequest))
}

fn validate_chat_id_value(value: Option<&JsonValue>) -> bool {
    value
        .and_then(scalar_text)
        .is_some_and(|value| !value.is_empty() && value.len() <= MAX_CHAT_TEXT_LENGTH)
}

fn validate_spool_limit(
    value: Option<&JsonValue>,
) -> Result<i64, TelegramReplySpoolExecutionError> {
    let value = value
        .and_then(JsonValue::as_i64)
        .unwrap_or(DEFAULT_SPOOL_LIMIT);
    if (1..=MAX_SPOOL_LIMIT).contains(&value) {
        Ok(value)
    } else {
        Err(error(InvalidRequest))
    }
}

fn validate_status(value: Option<&JsonValue>) -> Result<String, TelegramReplySpoolExecutionError> {
    let value = clean_text(value).ok_or_else(|| error(InvalidRequest))?;
    matches!(value.as_str(), "queued" | "attempting" | "ready" | "failed")
        .then_some(value)
        .ok_or_else(|| error(InvalidRequest))
}

fn validate_optional_bool(
    value: Option<&JsonValue>,
) -> Result<(), TelegramReplySpoolExecutionError> {
    if value.is_none_or(JsonValue::is_null) || value.and_then(JsonValue::as_bool).is_some() {
        Ok(())
    } else {
        Err(error(InvalidRequest))
    }
}

fn validate_optional_text(
    value: Option<&JsonValue>,
    max_length: usize,
) -> Result<(), TelegramReplySpoolExecutionError> {
    if value.is_none_or(JsonValue::is_null)
        || value
            .and_then(JsonValue::as_str)
            .is_some_and(|value| value.len() <= max_length)
    {
        Ok(())
    } else {
        Err(error(InvalidRequest))
    }
}

fn validate_optional_object_size(
    value: Option<&JsonValue>,
    max_bytes: usize,
) -> Result<(), TelegramReplySpoolExecutionError> {
    if value.is_none_or(JsonValue::is_null) {
        return Ok(());
    }
    let value = value
        .filter(|value| value.is_object())
        .ok_or_else(|| error(InvalidRequest))?;
    let bytes = JsonCodec::encode_value_to_vec(value, JsonEncodeOptions::compact())
        .map_err(|_| error(InvalidRequest))?;
    (bytes.len() <= max_bytes)
        .then_some(())
        .ok_or_else(|| error(InvalidRequest))
}

fn validate_clock_text(value: &str) -> Result<(), TelegramReplySpoolExecutionError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_CLOCK_TEXT_LENGTH {
        Err(error(Clock))
    } else {
        Ok(())
    }
}

fn bounded_text(value: Option<&JsonValue>, min: usize, max: usize) -> Option<String> {
    let value = value?.as_str()?;
    let trimmed = value.trim();
    (trimmed.len() >= min && value.len() <= max).then(|| value.to_string())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn scalar_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => Some(value.trim().to_string()),
        JsonValue::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn outcome(stage: &str, applied: bool, entry_count: usize, reason: &str) -> JsonValue {
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": stage,
        "completed": true,
        "ok": true,
        "applied": applied,
        "entry_count": entry_count,
        "reason": reason,
        "python_reply_spool_allowed": false,
    })
}

use TelegramReplySpoolExecutionErrorKind::{
    Clock, InvalidRequest, Planner, PlannerContract, State,
};

fn error(kind: TelegramReplySpoolExecutionErrorKind) -> TelegramReplySpoolExecutionError {
    TelegramReplySpoolExecutionError::new(kind)
}

#[cfg(test)]
mod tests;
