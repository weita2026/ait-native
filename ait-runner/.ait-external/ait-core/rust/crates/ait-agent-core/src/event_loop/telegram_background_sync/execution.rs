use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use ait_core::json_support::{json, JsonValue};
use chrono::{SecondsFormat, Utc};

use crate::event_loop::telegram_background_sync_state::TelegramBackgroundSyncStatePlanner;
use crate::runtime::AgentRuntimeBindingStore;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramBackgroundSyncExecution.v2";
const MIGRATION_STAGE: &str = "rust_agent_telegram_background_sync_execution";
const OPERATION_REQUEST_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramBackgroundSyncOperationRequest.v2";
const OPERATION_OUTCOME_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramBackgroundSyncOperationOutcome.v2";
const DEFAULT_BACKOFF_THRESHOLD: u64 = 2;
const DEFAULT_BACKOFF_BASE_SECONDS: f64 = 15.0;
const DEFAULT_BACKOFF_MAX_SECONDS: f64 = 120.0;
const MAX_CONTEXT_BYTES: usize = 1_048_576;
const MAX_CHAT_BYTES: usize = 512;

pub trait TelegramBackgroundSyncOperationPort: Send + Sync + 'static {
    fn run_workflow_notifications(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait TelegramBackgroundSyncStatePort: Send + Sync + 'static {
    fn load_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String>;

    fn patch_binding(&self, chat_id: &JsonValue, patch: &JsonValue) -> Result<bool, String>;
}

impl TelegramBackgroundSyncStatePort for AgentRuntimeBindingStore {
    fn load_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        match self.execute(
            "get_binding",
            &json!({"transport": "telegram", "surface_id": chat_id}),
        )? {
            JsonValue::Null => Ok(None),
            value @ JsonValue::Object(_) => Ok(Some(value)),
            _ => Err("Telegram background sync binding read returned invalid data.".to_string()),
        }
    }

    fn patch_binding(&self, chat_id: &JsonValue, patch: &JsonValue) -> Result<bool, String> {
        let updates = patch
            .as_object()
            .ok_or_else(|| "Telegram background sync patch must be an object.".to_string())?;
        match self.execute(
            "patch_binding",
            &json!({
                "transport": "telegram",
                "surface_id": chat_id,
                "updates": updates,
            }),
        )? {
            JsonValue::Null => Ok(false),
            JsonValue::Object(_) => Ok(true),
            _ => Err("Telegram background sync binding patch returned invalid data.".to_string()),
        }
    }
}

pub trait TelegramBackgroundSyncClockPort: Send + Sync + 'static {
    fn now_iso(&self) -> Result<String, String>;

    fn now_epoch(&self) -> Result<f64, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTelegramBackgroundSyncClockPort;

impl TelegramBackgroundSyncClockPort for SystemTelegramBackgroundSyncClockPort {
    fn now_iso(&self) -> Result<String, String> {
        Ok(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))
    }

    fn now_epoch(&self) -> Result<f64, String> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .map_err(|_| "system clock precedes Unix epoch".to_string())
    }
}

#[derive(Clone, PartialEq)]
pub struct TelegramBackgroundSyncExecution {
    metadata: JsonValue,
}

impl TelegramBackgroundSyncExecution {
    pub fn metadata(&self) -> &JsonValue {
        &self.metadata
    }

    pub fn into_metadata(self) -> JsonValue {
        self.metadata
    }
}

impl fmt::Debug for TelegramBackgroundSyncExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramBackgroundSyncExecution")
            .field("status", &self.metadata.get("background_sync_status"))
            .field("operation_count", &self.metadata.get("operation_count"))
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TelegramBackgroundSyncExecutionErrorKind {
    InvalidRequest,
    State,
    StateContract,
    Planner,
    PlannerContract,
    Clock,
}

impl TelegramBackgroundSyncExecutionErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::State => "state",
            Self::StateContract => "state_contract",
            Self::Planner => "planner",
            Self::PlannerContract => "planner_contract",
            Self::Clock => "clock",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TelegramBackgroundSyncExecutionError {
    kind: TelegramBackgroundSyncExecutionErrorKind,
}

impl TelegramBackgroundSyncExecutionError {
    pub fn kind(self) -> TelegramBackgroundSyncExecutionErrorKind {
        self.kind
    }
}

impl fmt::Display for TelegramBackgroundSyncExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Telegram background sync {} failed.",
            self.kind.code()
        )
    }
}

impl std::error::Error for TelegramBackgroundSyncExecutionError {}

pub fn execute_with_telegram_background_sync_ports<P, S, O, C>(
    planner: &P,
    state: &S,
    operations: &O,
    clock: &C,
    request: &JsonValue,
) -> Result<TelegramBackgroundSyncExecution, TelegramBackgroundSyncExecutionError>
where
    P: TelegramBackgroundSyncStatePlanner + ?Sized,
    S: TelegramBackgroundSyncStatePort + ?Sized,
    O: TelegramBackgroundSyncOperationPort + ?Sized,
    C: TelegramBackgroundSyncClockPort + ?Sized,
{
    let request = ValidatedRequest::parse(request)?;
    let Some(binding) = state
        .load_binding(&request.chat_id)
        .map_err(|_| error(TelegramBackgroundSyncExecutionErrorKind::State))?
    else {
        return Ok(execution(Metadata::noop("missing_binding")));
    };
    let work = planned_work(planner, &binding)?;
    if work.is_empty() {
        return Ok(execution(Metadata::noop("no_work")));
    }
    let now_epoch = clock
        .now_epoch()
        .map_err(|_| error(TelegramBackgroundSyncExecutionErrorKind::Clock))?;
    if !now_epoch.is_finite() || now_epoch < 0.0 {
        return Err(error(TelegramBackgroundSyncExecutionErrorKind::Clock));
    }
    let retry_after = binding
        .get("background_sync_retry_after_epoch")
        .and_then(JsonValue::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(0.0);
    if retry_after > now_epoch {
        return Ok(execution(Metadata {
            status: "backoff_active",
            ok: true,
            retryable: true,
            has_work: true,
            backoff_active: true,
            ..Metadata::default()
        }));
    }

    let mut completed = 0_usize;
    let mut sent_any = false;
    for kind in work.operation_kinds() {
        let operation_request = json!({
            "contract": OPERATION_REQUEST_CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "transport": "telegram",
            "operation_kind": kind,
            "chat_id": request.chat_id,
            "binding": binding,
            "operation_context": request.operation_context,
        });
        let raw = operations.run_workflow_notifications(&operation_request);
        let outcome = match raw {
            Ok(value) => validate_operation_outcome(&value, kind).unwrap_or(OperationOutcome {
                ok: false,
                sent_any: false,
                retryable: false,
            }),
            Err(_) => OperationOutcome {
                ok: false,
                sent_any: false,
                retryable: true,
            },
        };
        if !outcome.ok {
            return complete_failure(
                state,
                clock,
                &request.chat_id,
                work.len(),
                completed,
                outcome.retryable,
                now_epoch,
                &binding,
            );
        }
        completed += 1;
        sent_any |= outcome.sent_any;
    }

    let updated = state
        .patch_binding(
            &request.chat_id,
            &json!({
                "background_sync_failure_streak": 0,
                "background_sync_retry_after_epoch": JsonValue::Null,
                "background_sync_last_failure_at": JsonValue::Null,
                "background_sync_last_error": JsonValue::Null,
            }),
        )
        .map_err(|_| error(TelegramBackgroundSyncExecutionErrorKind::State))?;
    Ok(execution(Metadata {
        status: if updated {
            "completed"
        } else {
            "binding_removed"
        },
        ok: updated,
        terminal: !updated,
        has_work: true,
        sent_any: sent_any && updated,
        operation_count: work.len(),
        completed_operation_count: completed,
        state_updated: updated,
        ..Metadata::default()
    }))
}

#[allow(clippy::too_many_arguments)]
fn complete_failure<S, C>(
    state: &S,
    clock: &C,
    chat_id: &JsonValue,
    operation_count: usize,
    completed: usize,
    retryable: bool,
    now_epoch: f64,
    binding: &JsonValue,
) -> Result<TelegramBackgroundSyncExecution, TelegramBackgroundSyncExecutionError>
where
    S: TelegramBackgroundSyncStatePort + ?Sized,
    C: TelegramBackgroundSyncClockPort + ?Sized,
{
    let previous = binding
        .get("background_sync_failure_streak")
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let streak = previous.saturating_add(1);
    let delay = if retryable && streak >= DEFAULT_BACKOFF_THRESHOLD {
        (DEFAULT_BACKOFF_BASE_SECONDS * 2_f64.powi((streak - DEFAULT_BACKOFF_THRESHOLD) as i32))
            .min(DEFAULT_BACKOFF_MAX_SECONDS)
    } else {
        0.0
    };
    let retry_after = (delay > 0.0).then_some(now_epoch + delay);
    let now_iso = clock
        .now_iso()
        .map_err(|_| error(TelegramBackgroundSyncExecutionErrorKind::Clock))?;
    DateTimeGuard::validate(&now_iso)?;
    let updated = state
        .patch_binding(
            chat_id,
            &json!({
                "background_sync_failure_streak": streak,
                "background_sync_retry_after_epoch": retry_after,
                "background_sync_last_failure_at": now_iso,
                "background_sync_last_error": if retryable { "background_operation_retryable" } else { "background_operation_terminal" },
            }),
        )
        .map_err(|_| error(TelegramBackgroundSyncExecutionErrorKind::State))?;
    Ok(execution(Metadata {
        status: if !updated {
            "binding_removed"
        } else if retryable {
            "failed_retryable"
        } else {
            "failed_terminal"
        },
        ok: false,
        retryable: retryable && updated,
        terminal: !retryable || !updated,
        has_work: true,
        operation_count,
        completed_operation_count: completed,
        failed_operation_count: 1,
        state_updated: updated,
        retry_scheduled: retry_after.is_some() && updated,
        failure_streak: streak,
        ..Metadata::default()
    }))
}

#[derive(Clone, Copy)]
struct Work {
    workflow: bool,
}

impl Work {
    fn is_empty(self) -> bool {
        !self.workflow
    }

    fn len(self) -> usize {
        usize::from(self.workflow)
    }

    fn operation_kinds(self) -> Vec<&'static str> {
        let mut kinds = Vec::with_capacity(1);
        if self.workflow {
            kinds.push("run_workflow_notifications");
        }
        kinds
    }
}

fn planned_work<P>(
    planner: &P,
    binding: &JsonValue,
) -> Result<Work, TelegramBackgroundSyncExecutionError>
where
    P: TelegramBackgroundSyncStatePlanner + ?Sized,
{
    let object = binding
        .as_object()
        .ok_or_else(|| error(TelegramBackgroundSyncExecutionErrorKind::StateContract))?;
    let expected = Work {
        workflow: object
            .get("workflow_notifications_enabled")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false),
    };
    let planned = planner
        .plan_json(&json!({
            "stage": "work",
            "binding": binding,
        }))
        .map_err(|_| error(TelegramBackgroundSyncExecutionErrorKind::Planner))?;
    let planned = planned
        .as_object()
        .ok_or_else(|| error(TelegramBackgroundSyncExecutionErrorKind::PlannerContract))?;
    if planned.get("has_work").and_then(JsonValue::as_bool) != Some(!expected.is_empty())
        || planned
            .get("workflow_notifications_enabled")
            .and_then(JsonValue::as_bool)
            != Some(expected.workflow)
    {
        return Err(error(
            TelegramBackgroundSyncExecutionErrorKind::PlannerContract,
        ));
    }
    Ok(expected)
}

pub(super) fn binding_has_background_sync_work<P>(
    planner: &P,
    binding: &JsonValue,
) -> Result<bool, TelegramBackgroundSyncExecutionError>
where
    P: TelegramBackgroundSyncStatePlanner + ?Sized,
{
    planned_work(planner, binding).map(|work| !work.is_empty())
}

struct ValidatedRequest {
    chat_id: JsonValue,
    operation_context: JsonValue,
}

impl ValidatedRequest {
    fn parse(request: &JsonValue) -> Result<Self, TelegramBackgroundSyncExecutionError> {
        let object = request
            .as_object()
            .ok_or_else(|| error(TelegramBackgroundSyncExecutionErrorKind::InvalidRequest))?;
        let allowed = ["chat_id", "operation_context"];
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(error(
                TelegramBackgroundSyncExecutionErrorKind::InvalidRequest,
            ));
        }
        let chat_id = object
            .get("chat_id")
            .cloned()
            .filter(valid_chat_id)
            .ok_or_else(|| error(TelegramBackgroundSyncExecutionErrorKind::InvalidRequest))?;
        let operation_context = object
            .get("operation_context")
            .filter(|value| value.is_object() && value.to_string().len() <= MAX_CONTEXT_BYTES)
            .cloned()
            .unwrap_or_else(|| json!({}));
        Ok(Self {
            chat_id,
            operation_context,
        })
    }
}

#[derive(Clone, Copy)]
struct OperationOutcome {
    ok: bool,
    sent_any: bool,
    retryable: bool,
}

fn validate_operation_outcome(value: &JsonValue, kind: &str) -> Option<OperationOutcome> {
    let object = value.as_object()?;
    let allowed = [
        "contract",
        "operation_kind",
        "ok",
        "sent_any",
        "retryable",
        "terminal",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str()))
        || object.get("contract").and_then(JsonValue::as_str) != Some(OPERATION_OUTCOME_CONTRACT)
        || object.get("operation_kind").and_then(JsonValue::as_str) != Some(kind)
    {
        return None;
    }
    let ok = object.get("ok").and_then(JsonValue::as_bool)?;
    let sent_any = object.get("sent_any").and_then(JsonValue::as_bool)?;
    let retryable = object.get("retryable").and_then(JsonValue::as_bool)?;
    let terminal = object.get("terminal").and_then(JsonValue::as_bool)?;
    if (ok && (retryable || terminal)) || (!ok && (retryable == terminal || sent_any)) {
        return None;
    }
    Some(OperationOutcome {
        ok,
        sent_any,
        retryable,
    })
}

#[derive(Default)]
struct Metadata {
    status: &'static str,
    ok: bool,
    retryable: bool,
    terminal: bool,
    has_work: bool,
    backoff_active: bool,
    sent_any: bool,
    operation_count: usize,
    completed_operation_count: usize,
    failed_operation_count: usize,
    state_updated: bool,
    retry_scheduled: bool,
    failure_streak: u64,
}

impl Metadata {
    fn noop(status: &'static str) -> Self {
        Self {
            status,
            ok: true,
            ..Self::default()
        }
    }
}

fn execution(metadata: Metadata) -> TelegramBackgroundSyncExecution {
    TelegramBackgroundSyncExecution {
        metadata: json!({
            "contract": CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "stage": "execute",
            "transport": "telegram",
            "background_sync_status": metadata.status,
            "ok": metadata.ok,
            "completed": true,
            "retryable": metadata.retryable,
            "terminal": metadata.terminal,
            "has_work": metadata.has_work,
            "backoff_active": metadata.backoff_active,
            "sent_any": metadata.sent_any,
            "operation_count": metadata.operation_count,
            "completed_operation_count": metadata.completed_operation_count,
            "failed_operation_count": metadata.failed_operation_count,
            "state_updated": metadata.state_updated,
            "retry_scheduled": metadata.retry_scheduled,
            "failure_streak": metadata.failure_streak,
            "python_background_sync_allowed": false,
            "python_operation_execution_allowed": false,
            "python_state_mutation_allowed": false,
            "raw_request_exposed": false,
            "raw_binding_exposed": false,
            "raw_operation_result_exposed": false,
            "operation_context_exposed": false,
            "chat_id_exposed": false,
            "runtime_target_exposed": false,
            "bot_token_exposed": false,
            "queue_payload_exposed": false,
            "formatted_text_exposed": false,
            "state_path_exposed": false,
            "downstream_error_exposed": false,
        }),
    }
}

fn valid_chat_id(value: &JsonValue) -> bool {
    match value {
        JsonValue::Number(number) => number.as_i64().is_some(),
        JsonValue::String(value) => {
            let value = value.trim();
            !value.is_empty()
                && value.len() <= MAX_CHAT_BYTES
                && !value.chars().any(char::is_control)
        }
        _ => false,
    }
}

struct DateTimeGuard;

impl DateTimeGuard {
    fn validate(value: &str) -> Result<(), TelegramBackgroundSyncExecutionError> {
        if value.len() > 128 || chrono::DateTime::parse_from_rfc3339(value).is_err() {
            return Err(error(TelegramBackgroundSyncExecutionErrorKind::Clock));
        }
        Ok(())
    }
}

fn error(kind: TelegramBackgroundSyncExecutionErrorKind) -> TelegramBackgroundSyncExecutionError {
    TelegramBackgroundSyncExecutionError { kind }
}
