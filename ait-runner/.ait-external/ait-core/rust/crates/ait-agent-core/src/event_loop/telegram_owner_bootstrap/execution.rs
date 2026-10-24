use std::fmt;
use std::path::PathBuf;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use chrono::{SecondsFormat, Utc};

use super::planning::TelegramOwnerBootstrapPlanner;
use crate::runtime::AgentRuntimeBindingStore;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramOwnerBootstrapExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_owner_bootstrap_execution";
const PLANNING_CONTRACT: &str = "ait_agent_core.event_loop.TelegramOwnerBootstrap.v1";
const PLANNING_STAGE: &str = "rust_agent_telegram_owner_bootstrap";
const MAX_CLOCK_TEXT_LENGTH: usize = 128;
const MAX_MESSAGE_TEXT_LENGTH: usize = 16_384;

pub trait TelegramOwnerBootstrapStatePort: Send + Sync + 'static {
    fn load_bootstrap_auth(&self) -> Result<JsonValue, String>;

    fn load_existing_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String>;

    fn save_bootstrap_auth(&self, auth_state: &JsonValue) -> Result<(), String>;
}

pub trait TelegramOwnerBootstrapClockPort: Send + Sync + 'static {
    fn now_iso(&self) -> Result<String, String>;
}

pub trait TelegramOwnerBootstrapMessagePort: Send + Sync + 'static {
    fn send_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct RuntimeBindingTelegramOwnerBootstrapStatePort {
    store: AgentRuntimeBindingStore,
}

impl RuntimeBindingTelegramOwnerBootstrapStatePort {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self::from_store(AgentRuntimeBindingStore::new(path))
    }

    pub fn from_store(store: AgentRuntimeBindingStore) -> Self {
        Self { store }
    }

    pub fn path(&self) -> &std::path::Path {
        self.store.path()
    }
}

impl TelegramOwnerBootstrapStatePort for RuntimeBindingTelegramOwnerBootstrapStatePort {
    fn load_bootstrap_auth(&self) -> Result<JsonValue, String> {
        self.store.execute("get_bootstrap_auth", &json!({}))
    }

    fn load_existing_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        let result = self.store.execute(
            "get_binding",
            &json!({
                "transport": "telegram",
                "surface_id": chat_id,
            }),
        )?;
        match result {
            JsonValue::Null => Ok(None),
            JsonValue::Object(_) => Ok(Some(result)),
            _ => Err("Runtime binding store returned an invalid Telegram binding.".to_string()),
        }
    }

    fn save_bootstrap_auth(&self, auth_state: &JsonValue) -> Result<(), String> {
        let saved = self
            .store
            .execute("save_bootstrap_auth", &json!({"payload": auth_state}))?;
        saved
            .is_object()
            .then_some(())
            .ok_or_else(|| "Runtime binding store returned invalid bootstrap auth.".to_string())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTelegramOwnerBootstrapClockPort;

impl TelegramOwnerBootstrapClockPort for SystemTelegramOwnerBootstrapClockPort {
    fn now_iso(&self) -> Result<String, String> {
        Ok(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramOwnerBootstrapExecutionErrorKind {
    InvalidRequest,
    Planner,
    PlannerContract,
    State,
    Clock,
    Message,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramOwnerBootstrapExecutionError {
    kind: TelegramOwnerBootstrapExecutionErrorKind,
}

impl TelegramOwnerBootstrapExecutionError {
    pub fn kind(&self) -> TelegramOwnerBootstrapExecutionErrorKind {
        self.kind
    }

    fn new(kind: TelegramOwnerBootstrapExecutionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for TelegramOwnerBootstrapExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest => {
                "Telegram owner bootstrap execution request is invalid."
            }
            TelegramOwnerBootstrapExecutionErrorKind::Planner => {
                "Telegram owner bootstrap planning failed."
            }
            TelegramOwnerBootstrapExecutionErrorKind::PlannerContract => {
                "Telegram owner bootstrap planner contract is invalid."
            }
            TelegramOwnerBootstrapExecutionErrorKind::State => {
                "Telegram owner bootstrap state execution failed."
            }
            TelegramOwnerBootstrapExecutionErrorKind::Clock => {
                "Telegram owner bootstrap clock execution failed."
            }
            TelegramOwnerBootstrapExecutionErrorKind::Message => {
                "Telegram owner bootstrap message delivery failed."
            }
        })
    }
}

impl std::error::Error for TelegramOwnerBootstrapExecutionError {}

pub fn execute_with_telegram_owner_bootstrap_ports<P, S, C, M>(
    planner: &P,
    state: &S,
    clock: &C,
    message: &M,
    request: &JsonValue,
) -> Result<JsonValue, TelegramOwnerBootstrapExecutionError>
where
    P: TelegramOwnerBootstrapPlanner + ?Sized,
    S: TelegramOwnerBootstrapStatePort + ?Sized,
    C: TelegramOwnerBootstrapClockPort + ?Sized,
    M: TelegramOwnerBootstrapMessagePort + ?Sized,
{
    let mut execution_request = validated_request(request)?;
    execution_request.insert("kind".to_string(), json!("state_dependencies"));
    let initial_dependencies = plan(planner, &JsonValue::Object(execution_request.clone()))?;
    let initial_dependencies = parse_dependencies(&initial_dependencies)?;

    let mut auth_state_loaded = false;
    let mut existing_binding_loaded = false;
    if initial_dependencies.load_auth_state {
        let auth_state = state
            .load_bootstrap_auth()
            .map_err(|_| execution_error(TelegramOwnerBootstrapExecutionErrorKind::State))?;
        if !auth_state.is_object() {
            return Err(execution_error(
                TelegramOwnerBootstrapExecutionErrorKind::State,
            ));
        }
        execution_request.insert("auth_state".to_string(), auth_state);
        auth_state_loaded = true;

        let now_iso = clock
            .now_iso()
            .map_err(|_| execution_error(TelegramOwnerBootstrapExecutionErrorKind::Clock))?;
        let now_iso = validated_clock_text(&now_iso)?;
        execution_request.insert("now_iso".to_string(), json!(now_iso));

        let dependencies = plan(planner, &JsonValue::Object(execution_request.clone()))?;
        let dependencies = parse_dependencies(&dependencies)?;
        if !dependencies.load_auth_state {
            return Err(execution_error(
                TelegramOwnerBootstrapExecutionErrorKind::PlannerContract,
            ));
        }
        if dependencies.load_existing_binding {
            let chat_id = execution_request.get("chat_id").ok_or_else(|| {
                execution_error(TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest)
            })?;
            if !valid_chat_id(chat_id) {
                return Err(execution_error(
                    TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest,
                ));
            }
            if let Some(binding) = state
                .load_existing_binding(chat_id)
                .map_err(|_| execution_error(TelegramOwnerBootstrapExecutionErrorKind::State))?
            {
                if !binding.is_object() {
                    return Err(execution_error(
                        TelegramOwnerBootstrapExecutionErrorKind::State,
                    ));
                }
                execution_request.insert("existing_binding".to_string(), binding);
                existing_binding_loaded = true;
            }
        }
    }

    execution_request.insert("kind".to_string(), json!("handle"));
    let planned = plan(planner, &JsonValue::Object(execution_request.clone()))?;
    let action = parse_action(&planned)?;
    let message_target = if action.message_text.is_some() {
        let chat_id = execution_request.get("chat_id").ok_or_else(|| {
            execution_error(TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest)
        })?;
        if !valid_chat_id(chat_id) {
            return Err(execution_error(
                TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest,
            ));
        }
        Some(chat_id.clone())
    } else {
        None
    };

    let mut state_saved = false;
    if let Some(auth_state) = action.auth_state.as_ref() {
        state
            .save_bootstrap_auth(auth_state)
            .map_err(|_| execution_error(TelegramOwnerBootstrapExecutionErrorKind::State))?;
        state_saved = true;
    }

    let mut message_sent = false;
    if let (Some(chat_id), Some(text)) = (message_target.as_ref(), action.message_text.as_deref()) {
        message
            .send_message(chat_id, text)
            .map_err(|_| execution_error(TelegramOwnerBootstrapExecutionErrorKind::Message))?;
        message_sent = true;
    }

    Ok(json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "owner_bootstrap_state": "completed",
        "ok": true,
        "completed": true,
        "decision": action.decision,
        "handled": action.handled,
        "blocked": action.blocked,
        "adopted_owner": action.adopted_owner,
        "auth_state_loaded": auth_state_loaded,
        "existing_binding_loaded": existing_binding_loaded,
        "state_saved": state_saved,
        "message_sent": message_sent,
        "side_effect_count": usize::from(state_saved) + usize::from(message_sent),
        "rust_state_execution_required": true,
        "rust_message_delivery_required": true,
        "python_owner_bootstrap_allowed": false,
        "python_state_mutation_allowed": false,
        "python_message_delivery_allowed": false,
        "request_payload_exposed": false,
        "auth_state_exposed": false,
        "chat_id_exposed": false,
        "message_text_exposed": false,
    }))
}

fn validated_request(
    request: &JsonValue,
) -> Result<Map<String, JsonValue>, TelegramOwnerBootstrapExecutionError> {
    let object = request
        .as_object()
        .ok_or_else(|| execution_error(TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest))?;
    if object
        .get("owner_bootstrap_enabled")
        .and_then(JsonValue::as_bool)
        .is_none()
        || ["auth_state", "existing_binding", "now_iso"]
            .iter()
            .any(|field| object.contains_key(*field))
    {
        return Err(execution_error(
            TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest,
        ));
    }
    if let Some(kind) = clean_text(object.get("kind")) {
        if !matches!(kind.as_str(), "handle" | "gate") {
            return Err(execution_error(
                TelegramOwnerBootstrapExecutionErrorKind::InvalidRequest,
            ));
        }
    }
    Ok(object.clone())
}

fn plan<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, TelegramOwnerBootstrapExecutionError>
where
    P: TelegramOwnerBootstrapPlanner + ?Sized,
{
    planner
        .plan_json(request)
        .map_err(|_| execution_error(TelegramOwnerBootstrapExecutionErrorKind::Planner))
}

struct PlannedDependencies {
    load_auth_state: bool,
    load_existing_binding: bool,
}

fn parse_dependencies(
    value: &JsonValue,
) -> Result<PlannedDependencies, TelegramOwnerBootstrapExecutionError> {
    let object = validated_plan(value, "state_dependencies")?;
    if object.get("handled").and_then(JsonValue::as_bool) != Some(false)
        || object.get("blocked").and_then(JsonValue::as_bool) != Some(false)
        || !object.get("save_auth_state").is_none_or(JsonValue::is_null)
        || !object
            .get("send_message_text")
            .is_none_or(JsonValue::is_null)
    {
        return Err(execution_error(
            TelegramOwnerBootstrapExecutionErrorKind::PlannerContract,
        ));
    }
    let load_auth_state = required_bool(object, "load_auth_state")?;
    let load_existing_binding = required_bool(object, "load_existing_binding")?;
    if load_existing_binding && !load_auth_state {
        return Err(execution_error(
            TelegramOwnerBootstrapExecutionErrorKind::PlannerContract,
        ));
    }
    Ok(PlannedDependencies {
        load_auth_state,
        load_existing_binding,
    })
}

struct PlannedAction {
    decision: &'static str,
    handled: bool,
    blocked: bool,
    adopted_owner: bool,
    auth_state: Option<JsonValue>,
    message_text: Option<String>,
}

fn parse_action(value: &JsonValue) -> Result<PlannedAction, TelegramOwnerBootstrapExecutionError> {
    let object = validated_plan(value, "handle")?;
    let decision = safe_decision(object.get("decision")).ok_or_else(|| {
        execution_error(TelegramOwnerBootstrapExecutionErrorKind::PlannerContract)
    })?;
    let auth_state = match object.get("save_auth_state") {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::Object(_)) => object.get("save_auth_state").cloned(),
        _ => {
            return Err(execution_error(
                TelegramOwnerBootstrapExecutionErrorKind::PlannerContract,
            ))
        }
    };
    let message_text = match object.get("send_message_text") {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::String(text)) => Some(validated_message_text(text)?),
        _ => {
            return Err(execution_error(
                TelegramOwnerBootstrapExecutionErrorKind::PlannerContract,
            ))
        }
    };
    Ok(PlannedAction {
        decision,
        handled: required_bool(object, "handled")?,
        blocked: required_bool(object, "blocked")?,
        adopted_owner: required_bool(object, "adopted_owner")?,
        auth_state,
        message_text,
    })
}

fn validated_plan<'a>(
    value: &'a JsonValue,
    kind: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramOwnerBootstrapExecutionError> {
    let object = value.as_object().ok_or_else(|| {
        execution_error(TelegramOwnerBootstrapExecutionErrorKind::PlannerContract)
    })?;
    if clean_text(object.get("migration_stage")).as_deref() != Some(PLANNING_STAGE)
        || clean_text(object.get("owner_bootstrap_contract")).as_deref() != Some(PLANNING_CONTRACT)
        || clean_text(object.get("kind")).as_deref() != Some(kind)
        || clean_text(object.get("transport")).as_deref() != Some("telegram")
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_owner_bootstrap_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || clean_text(object.get("decision")).is_none()
    {
        return Err(execution_error(
            TelegramOwnerBootstrapExecutionErrorKind::PlannerContract,
        ));
    }
    Ok(object)
}

fn required_bool(
    object: &Map<String, JsonValue>,
    field: &str,
) -> Result<bool, TelegramOwnerBootstrapExecutionError> {
    object
        .get(field)
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| execution_error(TelegramOwnerBootstrapExecutionErrorKind::PlannerContract))
}

fn validated_clock_text(value: &str) -> Result<String, TelegramOwnerBootstrapExecutionError> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > MAX_CLOCK_TEXT_LENGTH
        || value.chars().any(char::is_control)
        || chrono::DateTime::parse_from_rfc3339(value).is_err()
    {
        return Err(execution_error(
            TelegramOwnerBootstrapExecutionErrorKind::Clock,
        ));
    }
    Ok(value.to_string())
}

fn validated_message_text(value: &str) -> Result<String, TelegramOwnerBootstrapExecutionError> {
    if value.is_empty()
        || value.trim() != value
        || value.chars().count() > MAX_MESSAGE_TEXT_LENGTH
        || value.contains('\0')
    {
        return Err(execution_error(
            TelegramOwnerBootstrapExecutionErrorKind::PlannerContract,
        ));
    }
    Ok(value.to_string())
}

fn valid_chat_id(value: &JsonValue) -> bool {
    match value {
        JsonValue::Number(number) => number.as_i64().is_some(),
        JsonValue::String(value) => {
            let value = value.trim();
            !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
        }
        _ => false,
    }
}

fn safe_decision(value: Option<&JsonValue>) -> Option<&'static str> {
    match value.and_then(JsonValue::as_str) {
        Some("disabled") => Some("disabled"),
        Some("missing_user_id") => Some("missing_user_id"),
        Some("owner_mismatch") => Some("owner_mismatch"),
        Some("owner_verified") => Some("owner_verified"),
        Some("blacklisted_user") => Some("blacklisted_user"),
        Some("pending_other_user") => Some("pending_other_user"),
        Some("adopt_existing_private_binding") => Some("adopt_existing_private_binding"),
        Some("prompt_start") => Some("prompt_start"),
        Some("awaiting_start") => Some("awaiting_start"),
        Some("plain_text_required") => Some("plain_text_required"),
        Some("blacklist_after_failures") => Some("blacklist_after_failures"),
        Some("incorrect_password") => Some("incorrect_password"),
        _ => None,
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn execution_error(
    kind: TelegramOwnerBootstrapExecutionErrorKind,
) -> TelegramOwnerBootstrapExecutionError {
    TelegramOwnerBootstrapExecutionError::new(kind)
}

#[cfg(test)]
mod tests;
