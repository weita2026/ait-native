use std::fmt;
use std::path::{Path, PathBuf};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::super::telegram_command_trigger::TelegramCommandTriggerOperationExecutor;
use super::super::telegram_event_triggers::TelegramEventTriggerPlanner;
use super::super::telegram_polling::agent_telegram_operational_trigger_callback_plan_json;
use crate::runtime::AgentRuntimeBindingStore;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramOperationalTriggerExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_operational_trigger_execution";
const EVENT_TRIGGER_CONTRACT: &str = "ait_agent_core.event_loop.TelegramEventTrigger.v1";
const EVENT_TRIGGER_STAGE: &str = "rust_agent_telegram_event_trigger";
const FAILURE_MESSAGE: &str =
    "ait Telegram operational trigger failed: Telegram operational trigger execution failed.";
const MAX_HANDLER_OUTPUT_BYTES: usize = 1_048_576;

#[derive(Clone)]
pub struct TelegramOperationalTriggerExecutionConfig {
    repo_name: String,
    repo_root: PathBuf,
    event_trigger_registry: JsonValue,
}

impl TelegramOperationalTriggerExecutionConfig {
    pub fn new(
        repo_name: impl Into<String>,
        repo_root: impl Into<PathBuf>,
        event_trigger_registry: JsonValue,
    ) -> Self {
        Self {
            repo_name: repo_name.into(),
            repo_root: repo_root.into(),
            event_trigger_registry,
        }
    }

    pub fn repo_name(&self) -> &str {
        self.repo_name.as_str()
    }

    pub fn repo_root(&self) -> &Path {
        self.repo_root.as_path()
    }

    pub fn event_trigger_registry(&self) -> &JsonValue {
        &self.event_trigger_registry
    }
}

pub trait TelegramOperationalTriggerCallbackPlanner: Send + Sync + 'static {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramOperationalTriggerCallbackPlanner;

impl TelegramOperationalTriggerCallbackPlanner
    for DefaultTelegramOperationalTriggerCallbackPlanner
{
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_operational_trigger_callback_plan_json(request)
    }
}

pub trait TelegramOperationalTriggerStatePort: Send + Sync + 'static {
    fn load_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String>;
}

#[derive(Debug, Clone)]
pub struct RuntimeBindingTelegramOperationalTriggerStatePort {
    store: AgentRuntimeBindingStore,
}

impl RuntimeBindingTelegramOperationalTriggerStatePort {
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

impl TelegramOperationalTriggerStatePort for RuntimeBindingTelegramOperationalTriggerStatePort {
    fn load_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
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
            _ => Err("Runtime binding store returned an invalid Telegram binding.".to_string()),
        }
    }
}

pub trait TelegramOperationalTriggerDiagnosticsPort: Send + Sync + 'static {
    fn record_failure(
        &self,
        kind: TelegramOperationalTriggerExecutionErrorKind,
    ) -> Result<(), String>;
}

pub trait TelegramOperationalTriggerDeliveryPort: Send + Sync + 'static {
    fn send_assistant_event_reply(
        &self,
        chat_id: &JsonValue,
        assistant_event: &JsonValue,
    ) -> Result<(), String>;

    fn send_failure_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String>;
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TelegramOperationalTriggerExecutionErrorKind {
    InvalidRequest,
    EventPlanner,
    EventPlannerContract,
    State,
    CallbackPlanner,
    CallbackPlannerContract,
    Operation,
    Diagnostics,
    Delivery,
}

impl TelegramOperationalTriggerExecutionErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::EventPlanner => "event_planner",
            Self::EventPlannerContract => "event_planner_contract",
            Self::State => "state",
            Self::CallbackPlanner => "callback_planner",
            Self::CallbackPlannerContract => "callback_planner_contract",
            Self::Operation => "operation",
            Self::Diagnostics => "diagnostics",
            Self::Delivery => "delivery",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TelegramOperationalTriggerExecutionError {
    kind: TelegramOperationalTriggerExecutionErrorKind,
}

impl TelegramOperationalTriggerExecutionError {
    pub fn kind(self) -> TelegramOperationalTriggerExecutionErrorKind {
        self.kind
    }

    fn new(kind: TelegramOperationalTriggerExecutionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for TelegramOperationalTriggerExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TelegramOperationalTriggerExecutionErrorKind::InvalidRequest => {
                "Telegram operational trigger execution request is invalid."
            }
            TelegramOperationalTriggerExecutionErrorKind::EventPlanner => {
                "Telegram operational trigger event planning failed."
            }
            TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract => {
                "Telegram operational trigger event planner contract is invalid."
            }
            TelegramOperationalTriggerExecutionErrorKind::State => {
                "Telegram operational trigger state execution failed."
            }
            TelegramOperationalTriggerExecutionErrorKind::CallbackPlanner => {
                "Telegram operational trigger callback planning failed."
            }
            TelegramOperationalTriggerExecutionErrorKind::CallbackPlannerContract => {
                "Telegram operational trigger callback planner contract is invalid."
            }
            TelegramOperationalTriggerExecutionErrorKind::Operation => {
                "Telegram operational trigger handler execution failed."
            }
            TelegramOperationalTriggerExecutionErrorKind::Diagnostics => {
                "Telegram operational trigger diagnostics failed."
            }
            TelegramOperationalTriggerExecutionErrorKind::Delivery => {
                "Telegram operational trigger delivery failed."
            }
        })
    }
}

impl std::error::Error for TelegramOperationalTriggerExecutionError {}

pub struct TelegramOperationalTriggerPorts<
    'a,
    E: ?Sized,
    C: ?Sized,
    S: ?Sized,
    O: ?Sized,
    G: ?Sized,
    D: ?Sized,
> {
    event_planner: &'a E,
    callback_planner: &'a C,
    state: &'a S,
    operation: &'a O,
    diagnostics: &'a G,
    delivery: &'a D,
}

impl<'a, E: ?Sized, C: ?Sized, S: ?Sized, O: ?Sized, G: ?Sized, D: ?Sized>
    TelegramOperationalTriggerPorts<'a, E, C, S, O, G, D>
{
    pub fn new(
        event_planner: &'a E,
        callback_planner: &'a C,
        state: &'a S,
        operation: &'a O,
        diagnostics: &'a G,
        delivery: &'a D,
    ) -> Self {
        Self {
            event_planner,
            callback_planner,
            state,
            operation,
            diagnostics,
            delivery,
        }
    }
}

pub fn execute_with_telegram_operational_trigger_ports<E, C, S, O, G, D>(
    ports: &TelegramOperationalTriggerPorts<'_, E, C, S, O, G, D>,
    config: &TelegramOperationalTriggerExecutionConfig,
    request: &JsonValue,
) -> Result<JsonValue, TelegramOperationalTriggerExecutionError>
where
    E: TelegramEventTriggerPlanner + ?Sized,
    C: TelegramOperationalTriggerCallbackPlanner + ?Sized,
    S: TelegramOperationalTriggerStatePort + ?Sized,
    O: TelegramCommandTriggerOperationExecutor + ?Sized,
    G: TelegramOperationalTriggerDiagnosticsPort + ?Sized,
    D: TelegramOperationalTriggerDeliveryPort + ?Sized,
{
    let validated = ValidatedRequest::parse(request)?;
    let validated_config = ValidatedConfig::parse(config)?;
    let dispatch = dispatch_trigger(ports.event_planner, &validated_config, &validated)?;
    let Some(dispatch) = dispatch else {
        return Ok(execution_outcome(ExecutionOutcomeFacts::unmatched()));
    };

    match execute_matched(
        ports.callback_planner,
        ports.state,
        ports.operation,
        ports.delivery,
        &validated_config,
        &validated,
        &dispatch,
    ) {
        Ok(outcome) => Ok(execution_outcome(ExecutionOutcomeFacts {
            matched: true,
            handled: outcome.handled,
            ok: true,
            operation_count: outcome.operation_count,
            completed_operation_count: outcome.completed_operation_count,
            result_callback_planned: outcome.result_callback_planned,
            assistant_event_sent: outcome.assistant_event_sent,
            failure_kind: None,
        })),
        Err(failure) => {
            ports
                .diagnostics
                .record_failure(failure.error.kind())
                .map_err(|_| error(TelegramOperationalTriggerExecutionErrorKind::Diagnostics))?;
            ports
                .delivery
                .send_failure_message(&validated.chat_id, FAILURE_MESSAGE)
                .map_err(|_| error(TelegramOperationalTriggerExecutionErrorKind::Delivery))?;
            Ok(execution_outcome(ExecutionOutcomeFacts {
                matched: true,
                handled: true,
                ok: false,
                operation_count: failure.operation_count,
                completed_operation_count: failure.completed_operation_count,
                result_callback_planned: failure.result_callback_planned,
                assistant_event_sent: false,
                failure_kind: Some(failure.error.kind()),
            }))
        }
    }
}

struct ValidatedConfig {
    repo_name: String,
    repo_root: String,
    triggers: Vec<JsonValue>,
}

impl ValidatedConfig {
    fn parse(
        config: &TelegramOperationalTriggerExecutionConfig,
    ) -> Result<Self, TelegramOperationalTriggerExecutionError> {
        let repo_name = config.repo_name.trim().to_string();
        let repo_root = config
            .repo_root
            .to_str()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))?
            .to_string();
        let registry = config
            .event_trigger_registry
            .as_object()
            .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))?;
        let triggers = registry
            .get("telegram_operational")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))?
            .clone();
        if triggers.iter().any(|trigger| !trigger.is_object()) {
            return Err(error(
                TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
            ));
        }
        Ok(Self {
            repo_name,
            repo_root,
            triggers,
        })
    }
}

struct ValidatedRequest {
    chat_id: JsonValue,
    chat: JsonValue,
    from_user: JsonValue,
    chat_title: String,
    context: JsonValue,
    raw_text: String,
    normalized_text: String,
    command: JsonValue,
    reply_to_message_id: Option<i64>,
}

impl ValidatedRequest {
    fn parse(request: &JsonValue) -> Result<Self, TelegramOperationalTriggerExecutionError> {
        let object = request
            .as_object()
            .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))?;
        require_only_keys(
            object,
            &["chat_id", "chat", "from_user", "chat_title", "context"],
        )?;
        let chat_id = object
            .get("chat_id")
            .filter(|value| valid_chat_id(value))
            .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))?
            .clone();
        let chat = required_object_value(object, "chat")?;
        let from_user = required_object_value(object, "from_user")?;
        let chat_title = required_string(object, "chat_title")?;
        let context = object
            .get("context")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))?;
        require_only_keys(
            context,
            &[
                "raw_text",
                "normalized_text",
                "command",
                "telegram_message_id",
                "telegram_message_ids",
                "reply_to_message",
                "attachments",
                "actor_identity",
                "message",
            ],
        )?;
        let raw_text = required_string(context, "raw_text")?;
        let normalized_text = required_string(context, "normalized_text")?;
        let command = validated_command(context.get("command"))?;
        validated_optional_positive_i64(context.get("telegram_message_id"))?;
        validated_positive_i64_array(context.get("telegram_message_ids"))?;
        validated_object_array(context.get("attachments"))?;
        validated_optional_string(context.get("actor_identity"))?;
        validated_optional_object(context.get("message"))?;
        let reply_to_message = validated_optional_object(context.get("reply_to_message"))?;
        let reply_to_message_id = reply_to_message
            .and_then(|reply| reply.get("message_id"))
            .and_then(JsonValue::as_i64)
            .filter(|value| *value > 0);

        Ok(Self {
            chat_id,
            chat,
            from_user,
            chat_title,
            context: JsonValue::Object(context.clone()),
            raw_text,
            normalized_text,
            command,
            reply_to_message_id,
        })
    }
}

struct Dispatch {
    trigger: JsonValue,
    match_payload: JsonValue,
}

fn dispatch_trigger<E>(
    planner: &E,
    config: &ValidatedConfig,
    request: &ValidatedRequest,
) -> Result<Option<Dispatch>, TelegramOperationalTriggerExecutionError>
where
    E: TelegramEventTriggerPlanner + ?Sized,
{
    let planned = planner
        .plan_json(&json!({
            "stage": "operational_dispatch",
            "triggers": config.triggers,
            "raw_text": request.raw_text,
            "normalized_text": request.normalized_text,
            "command": request.command,
            "reply_to_message_id": request.reply_to_message_id,
        }))
        .map_err(|_| error(TelegramOperationalTriggerExecutionErrorKind::EventPlanner))?;
    let object = planned
        .as_object()
        .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract))?;
    require_exact_text(
        object,
        "migration_stage",
        EVENT_TRIGGER_STAGE,
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    require_exact_text(
        object,
        "event_trigger_contract",
        EVENT_TRIGGER_CONTRACT,
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    require_exact_text(
        object,
        "stage",
        "operational_dispatch",
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    require_exact_text(
        object,
        "transport",
        "telegram",
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    require_exact_bool(
        object,
        "rust_event_loop_required",
        true,
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    require_exact_bool(
        object,
        "python_event_trigger_allowed",
        false,
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    let matched = required_bool_with_kind(
        object,
        "matched",
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    let handled = required_bool_with_kind(
        object,
        "handled",
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    if matched != handled {
        return Err(error(
            TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
        ));
    }
    if !matched {
        if !object.get("trigger").is_some_and(JsonValue::is_null)
            || !object.get("match_payload").is_some_and(JsonValue::is_null)
        {
            return Err(error(
                TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
            ));
        }
        return Ok(None);
    }
    let trigger = required_object_value_with_kind(
        object,
        "trigger",
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    validate_selected_trigger(&trigger, config)?;
    let match_payload = required_object_value_with_kind(
        object,
        "match_payload",
        TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
    )?;
    Ok(Some(Dispatch {
        trigger,
        match_payload,
    }))
}

struct MatchedOutcome {
    handled: bool,
    operation_count: usize,
    completed_operation_count: usize,
    result_callback_planned: bool,
    assistant_event_sent: bool,
}

struct MatchedFailure {
    error: TelegramOperationalTriggerExecutionError,
    operation_count: usize,
    completed_operation_count: usize,
    result_callback_planned: bool,
}

impl MatchedFailure {
    fn new(kind: TelegramOperationalTriggerExecutionErrorKind) -> Self {
        Self {
            error: error(kind),
            operation_count: 0,
            completed_operation_count: 0,
            result_callback_planned: false,
        }
    }
}

fn execute_matched<C, S, O, D>(
    callback_planner: &C,
    state: &S,
    operation: &O,
    delivery: &D,
    config: &ValidatedConfig,
    request: &ValidatedRequest,
    dispatch: &Dispatch,
) -> Result<MatchedOutcome, MatchedFailure>
where
    C: TelegramOperationalTriggerCallbackPlanner + ?Sized,
    S: TelegramOperationalTriggerStatePort + ?Sized,
    O: TelegramCommandTriggerOperationExecutor + ?Sized,
    D: TelegramOperationalTriggerDeliveryPort + ?Sized,
{
    let binding = state
        .load_binding(&request.chat_id)
        .map_err(|_| MatchedFailure::new(TelegramOperationalTriggerExecutionErrorKind::State))?;
    if binding.as_ref().is_some_and(|binding| !binding.is_object()) {
        return Err(MatchedFailure::new(
            TelegramOperationalTriggerExecutionErrorKind::State,
        ));
    }
    let callback_request = callback_request(
        config,
        request,
        dispatch,
        binding.unwrap_or(JsonValue::Null),
    );
    let planned = callback_planner.plan_json(&callback_request).map_err(|_| {
        MatchedFailure::new(TelegramOperationalTriggerExecutionErrorKind::CallbackPlanner)
    })?;
    let operations =
        parse_callback_request(&planned, config, &dispatch.trigger).map_err(MatchedFailure::new)?;
    let operation_count = operations.len();
    let mut operation_results = Vec::with_capacity(operation_count);
    for operation_request in &operations {
        match operation.execute_operation_json(operation_request) {
            Ok(result) => {
                validate_operation_result(&result).map_err(|kind| MatchedFailure {
                    error: error(kind),
                    operation_count,
                    completed_operation_count: operation_results.len(),
                    result_callback_planned: false,
                })?;
                operation_results.push(result);
            }
            Err(_) => {
                let result_request =
                    callback_result_request(request, dispatch, &operation_results, operation_count);
                let partial =
                    callback_planner
                        .plan_json(&result_request)
                        .map_err(|_| MatchedFailure {
                            error: error(
                                TelegramOperationalTriggerExecutionErrorKind::CallbackPlanner,
                            ),
                            operation_count,
                            completed_operation_count: operation_results.len(),
                            result_callback_planned: false,
                        })?;
                parse_callback_result(&partial, operation_count).map_err(|kind| {
                    MatchedFailure {
                        error: error(kind),
                        operation_count,
                        completed_operation_count: operation_results.len(),
                        result_callback_planned: false,
                    }
                })?;
                return Err(MatchedFailure {
                    error: error(TelegramOperationalTriggerExecutionErrorKind::Operation),
                    operation_count,
                    completed_operation_count: operation_results.len(),
                    result_callback_planned: true,
                });
            }
        }
    }

    let result_request =
        callback_result_request(request, dispatch, &operation_results, operation_count);
    let result_plan = callback_planner
        .plan_json(&result_request)
        .map_err(|_| MatchedFailure {
            error: error(TelegramOperationalTriggerExecutionErrorKind::CallbackPlanner),
            operation_count,
            completed_operation_count: operation_results.len(),
            result_callback_planned: false,
        })?;
    let result =
        parse_callback_result(&result_plan, operation_count).map_err(|kind| MatchedFailure {
            error: error(kind),
            operation_count,
            completed_operation_count: operation_results.len(),
            result_callback_planned: false,
        })?;
    if !result.ok {
        return Err(MatchedFailure {
            error: error(TelegramOperationalTriggerExecutionErrorKind::Operation),
            operation_count,
            completed_operation_count: operation_results.len(),
            result_callback_planned: true,
        });
    }

    let mut assistant_event_sent = false;
    if let Some(assistant_event) = result.assistant_event.as_ref() {
        delivery
            .send_assistant_event_reply(&request.chat_id, assistant_event)
            .map_err(|_| MatchedFailure {
                error: error(TelegramOperationalTriggerExecutionErrorKind::Delivery),
                operation_count,
                completed_operation_count: operation_results.len(),
                result_callback_planned: true,
            })?;
        assistant_event_sent = true;
    }
    Ok(MatchedOutcome {
        handled: result.handled,
        operation_count,
        completed_operation_count: operation_results.len(),
        result_callback_planned: true,
        assistant_event_sent,
    })
}

fn callback_request(
    config: &ValidatedConfig,
    request: &ValidatedRequest,
    dispatch: &Dispatch,
    binding: JsonValue,
) -> JsonValue {
    json!({
        "stage": "request",
        "trigger": dispatch.trigger,
        "match_payload": dispatch.match_payload,
        "chat_id": request.chat_id,
        "chat": request.chat,
        "from_user": request.from_user,
        "chat_title": request.chat_title,
        "context": request.context,
        "repo_name": config.repo_name,
        "repo_root": config.repo_root,
        "reply_to_message_id": request.reply_to_message_id,
        "binding": binding,
    })
}

fn callback_result_request(
    request: &ValidatedRequest,
    dispatch: &Dispatch,
    operation_results: &[JsonValue],
    operation_count: usize,
) -> JsonValue {
    json!({
        "stage": "result",
        "trigger": dispatch.trigger,
        "chat_id": request.chat_id,
        "chat": request.chat,
        "chat_title": request.chat_title,
        "context": request.context,
        "operation_results": operation_results,
        "operation_count": operation_count,
    })
}

fn parse_callback_request(
    planned: &JsonValue,
    config: &ValidatedConfig,
    trigger: &JsonValue,
) -> Result<Vec<JsonValue>, TelegramOperationalTriggerExecutionErrorKind> {
    let kind = TelegramOperationalTriggerExecutionErrorKind::CallbackPlannerContract;
    let object = planned.as_object().ok_or(kind)?;
    require_exact_text_kind(object, "stage", "request", kind)?;
    require_exact_text_kind(
        object,
        "execution_kind",
        "operational_trigger_callback",
        kind,
    )?;
    require_exact_text_kind(object, "callback_group", "command_trigger", kind)?;
    require_exact_text_kind(object, "trigger_kind", "telegram_operational_trigger", kind)?;
    require_exact_bool_kind(object, "should_execute", true, kind)?;
    require_exact_bool_kind(object, "expects_result", true, kind)?;
    require_exact_bool_kind(object, "completed", false, kind)?;
    let request = object
        .get("request")
        .and_then(JsonValue::as_object)
        .ok_or(kind)?;
    require_exact_text_kind(
        request,
        "execution_kind",
        "operational_trigger_callback",
        kind,
    )?;
    require_exact_text_kind(request, "callback_group", "command_trigger", kind)?;
    require_exact_text_kind(
        request,
        "trigger_kind",
        "telegram_operational_trigger",
        kind,
    )?;
    require_exact_bool_kind(request, "ok", true, kind)?;
    if !request.get("error").is_some_and(JsonValue::is_null) {
        return Err(kind);
    }
    require_exact_text_kind(request, "operation", "run_handler", kind)?;
    let operations = request
        .get("operations")
        .and_then(JsonValue::as_array)
        .ok_or(kind)?;
    if operations.len() != 1
        || request.get("operation_count").and_then(JsonValue::as_u64) != Some(1)
    {
        return Err(kind);
    }
    validate_handler_operation(&operations[0], config, trigger)?;
    Ok(operations.clone())
}

fn validate_handler_operation(
    operation: &JsonValue,
    config: &ValidatedConfig,
    trigger: &JsonValue,
) -> Result<(), TelegramOperationalTriggerExecutionErrorKind> {
    let kind = TelegramOperationalTriggerExecutionErrorKind::CallbackPlannerContract;
    let object = operation.as_object().ok_or(kind)?;
    require_only_keys_kind(
        object,
        &[
            "kind",
            "method",
            "trigger_id",
            "reply_to_message_id",
            "handler_command",
            "cwd",
            "repo_root",
            "stdin_json",
            "env_overrides",
            "pythonpath_repo_src",
        ],
        kind,
    )?;
    require_exact_text_kind(object, "kind", "run_handler", kind)?;
    require_exact_text_kind(object, "method", "subprocess.run", kind)?;
    require_exact_text_kind(object, "cwd", config.repo_root.as_str(), kind)?;
    require_exact_text_kind(object, "repo_root", config.repo_root.as_str(), kind)?;
    if !object.get("stdin_json").is_some_and(JsonValue::is_object) {
        return Err(kind);
    }
    let env = object
        .get("env_overrides")
        .and_then(JsonValue::as_object)
        .ok_or(kind)?;
    if env.len() != 1
        || env.get("AIT_REPO_ROOT").and_then(JsonValue::as_str) != Some(config.repo_root.as_str())
    {
        return Err(kind);
    }
    let expected_repo_src = format!("{}/src", config.repo_root);
    if object
        .get("pythonpath_repo_src")
        .and_then(JsonValue::as_str)
        != Some(expected_repo_src.as_str())
    {
        return Err(kind);
    }
    let trigger_object = trigger.as_object().ok_or(kind)?;
    let expected_command = trigger_object
        .get("handler_command")
        .and_then(JsonValue::as_array)
        .ok_or(kind)?;
    let operation_command = object
        .get("handler_command")
        .and_then(JsonValue::as_array)
        .ok_or(kind)?;
    if operation_command != expected_command
        || operation_command.is_empty()
        || operation_command
            .iter()
            .any(|value| value.as_str().is_none_or(|text| text.trim().is_empty()))
    {
        return Err(kind);
    }
    if object.get("trigger_id") != trigger_object.get("trigger_id") {
        return Err(kind);
    }
    if object
        .get("reply_to_message_id")
        .is_none_or(|value| !value.is_null() && value.as_i64().is_none_or(|number| number <= 0))
    {
        return Err(kind);
    }
    Ok(())
}

fn validate_operation_result(
    result: &JsonValue,
) -> Result<(), TelegramOperationalTriggerExecutionErrorKind> {
    let kind = TelegramOperationalTriggerExecutionErrorKind::Operation;
    let object = result.as_object().ok_or(kind)?;
    require_exact_text_kind(object, "kind", "run_handler", kind)?;
    require_exact_text_kind(object, "method", "std::process::Command", kind)?;
    let ok = object.get("ok").and_then(JsonValue::as_bool).ok_or(kind)?;
    let returncode = object
        .get("returncode")
        .and_then(JsonValue::as_i64)
        .ok_or(kind)?;
    let stdout = object
        .get("stdout")
        .and_then(JsonValue::as_str)
        .ok_or(kind)?;
    let stderr = object
        .get("stderr")
        .and_then(JsonValue::as_str)
        .ok_or(kind)?;
    if stdout.len().saturating_add(stderr.len()) > MAX_HANDLER_OUTPUT_BYTES {
        return Err(kind);
    }
    let error_value = object.get("error").ok_or(kind)?;
    if !error_value.is_null() && !error_value.is_string() {
        return Err(kind);
    }
    if ok != (returncode == 0) || (ok && !error_value.is_null()) {
        return Err(kind);
    }
    for key in ["handler_response", "response"] {
        if object.get(key).is_some_and(|value| !value.is_object()) {
            return Err(kind);
        }
    }
    Ok(())
}

fn validate_callback_result_envelope(
    planned: &JsonValue,
) -> Result<(), TelegramOperationalTriggerExecutionErrorKind> {
    let kind = TelegramOperationalTriggerExecutionErrorKind::CallbackPlannerContract;
    let object = planned.as_object().ok_or(kind)?;
    require_exact_text_kind(object, "stage", "result", kind)?;
    require_exact_text_kind(
        object,
        "execution_kind",
        "operational_trigger_callback",
        kind,
    )?;
    require_exact_text_kind(object, "callback_group", "command_trigger", kind)?;
    require_exact_text_kind(object, "trigger_kind", "telegram_operational_trigger", kind)?;
    require_exact_bool_kind(object, "should_execute", false, kind)?;
    require_exact_bool_kind(object, "expects_result", false, kind)?;
    object
        .get("completed")
        .and_then(JsonValue::as_bool)
        .ok_or(kind)?;
    if !object.get("result").is_some_and(JsonValue::is_object) {
        return Err(kind);
    }
    Ok(())
}

struct CallbackResult {
    ok: bool,
    handled: bool,
    assistant_event: Option<JsonValue>,
}

fn parse_callback_result(
    planned: &JsonValue,
    expected_operation_count: usize,
) -> Result<CallbackResult, TelegramOperationalTriggerExecutionErrorKind> {
    validate_callback_result_envelope(planned)?;
    let kind = TelegramOperationalTriggerExecutionErrorKind::CallbackPlannerContract;
    let object = planned.as_object().ok_or(kind)?;
    let result = object
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or(kind)?;
    require_exact_text_kind(
        result,
        "execution_kind",
        "operational_trigger_callback",
        kind,
    )?;
    require_exact_text_kind(result, "callback_group", "command_trigger", kind)?;
    require_exact_text_kind(result, "trigger_kind", "telegram_operational_trigger", kind)?;
    let ok = result.get("ok").and_then(JsonValue::as_bool).ok_or(kind)?;
    if object.get("completed").and_then(JsonValue::as_bool) != Some(ok) {
        return Err(kind);
    }
    let handled = result
        .get("handled")
        .and_then(JsonValue::as_bool)
        .ok_or(kind)?;
    let should_send = result
        .get("should_send_assistant_event")
        .and_then(JsonValue::as_bool)
        .ok_or(kind)?;
    let command_result = result
        .get("command_result")
        .and_then(JsonValue::as_object)
        .ok_or(kind)?;
    if command_result
        .get("operation_count")
        .and_then(JsonValue::as_u64)
        != Some(expected_operation_count as u64)
    {
        return Err(kind);
    }
    if command_result
        .get("operation_results")
        .and_then(JsonValue::as_array)
        .is_none_or(|results| results.len() > expected_operation_count)
    {
        return Err(kind);
    }
    let assistant_event = result.get("assistant_event").ok_or(kind)?;
    if should_send {
        if !ok || !handled {
            return Err(kind);
        }
        validate_assistant_event(assistant_event)?;
        Ok(CallbackResult {
            ok,
            handled,
            assistant_event: Some(assistant_event.clone()),
        })
    } else {
        if !assistant_event.is_null() {
            return Err(kind);
        }
        Ok(CallbackResult {
            ok,
            handled,
            assistant_event: None,
        })
    }
}

fn validate_assistant_event(
    event: &JsonValue,
) -> Result<(), TelegramOperationalTriggerExecutionErrorKind> {
    let kind = TelegramOperationalTriggerExecutionErrorKind::CallbackPlannerContract;
    let object = event.as_object().ok_or(kind)?;
    require_exact_text_kind(object, "event_type", "assistant.reply", kind)?;
    let payload = object
        .get("payload")
        .and_then(JsonValue::as_object)
        .ok_or(kind)?;
    if payload.get("text").and_then(JsonValue::as_str).is_none()
        || !payload
            .get("transport_reply_envelope")
            .is_some_and(JsonValue::is_object)
    {
        return Err(kind);
    }
    Ok(())
}

fn validate_selected_trigger(
    trigger: &JsonValue,
    config: &ValidatedConfig,
) -> Result<(), TelegramOperationalTriggerExecutionError> {
    let object = trigger
        .as_object()
        .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract))?;
    object
        .get("trigger_id")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract))?;
    let commands = object
        .get("handler_command")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract))?;
    if commands.is_empty()
        || commands
            .iter()
            .any(|value| value.as_str().is_none_or(|text| text.trim().is_empty()))
        || !config.triggers.iter().any(|candidate| candidate == trigger)
    {
        return Err(error(
            TelegramOperationalTriggerExecutionErrorKind::EventPlannerContract,
        ));
    }
    Ok(())
}

struct ExecutionOutcomeFacts {
    matched: bool,
    handled: bool,
    ok: bool,
    operation_count: usize,
    completed_operation_count: usize,
    result_callback_planned: bool,
    assistant_event_sent: bool,
    failure_kind: Option<TelegramOperationalTriggerExecutionErrorKind>,
}

impl ExecutionOutcomeFacts {
    fn unmatched() -> Self {
        Self {
            matched: false,
            handled: false,
            ok: true,
            operation_count: 0,
            completed_operation_count: 0,
            result_callback_planned: false,
            assistant_event_sent: false,
            failure_kind: None,
        }
    }
}

fn execution_outcome(facts: ExecutionOutcomeFacts) -> JsonValue {
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "ok": facts.ok,
        "completed": true,
        "matched": facts.matched,
        "handled": facts.handled,
        "operation_count": facts.operation_count,
        "completed_operation_count": facts.completed_operation_count,
        "result_callback_planned": facts.result_callback_planned,
        "assistant_event_sent": facts.assistant_event_sent,
        "failure_message_sent": facts.failure_kind.is_some(),
        "failure_kind": facts.failure_kind.map(TelegramOperationalTriggerExecutionErrorKind::code),
        "python_executor_allowed": false,
    })
}

fn require_only_keys(
    object: &Map<String, JsonValue>,
    allowed: &[&str],
) -> Result<(), TelegramOperationalTriggerExecutionError> {
    if object.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        Err(error(
            TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
        ))
    }
}

fn require_only_keys_kind(
    object: &Map<String, JsonValue>,
    allowed: &[&str],
    kind: TelegramOperationalTriggerExecutionErrorKind,
) -> Result<(), TelegramOperationalTriggerExecutionErrorKind> {
    if object.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        Err(kind)
    }
}

fn required_object_value(
    object: &Map<String, JsonValue>,
    key: &str,
) -> Result<JsonValue, TelegramOperationalTriggerExecutionError> {
    required_object_value_with_kind(
        object,
        key,
        TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
    )
}

fn required_object_value_with_kind(
    object: &Map<String, JsonValue>,
    key: &str,
    kind: TelegramOperationalTriggerExecutionErrorKind,
) -> Result<JsonValue, TelegramOperationalTriggerExecutionError> {
    object
        .get(key)
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| error(kind))
}

fn required_string(
    object: &Map<String, JsonValue>,
    key: &str,
) -> Result<String, TelegramOperationalTriggerExecutionError> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))
}

fn required_bool_with_kind(
    object: &Map<String, JsonValue>,
    key: &str,
    kind: TelegramOperationalTriggerExecutionErrorKind,
) -> Result<bool, TelegramOperationalTriggerExecutionError> {
    object
        .get(key)
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(kind))
}

fn require_exact_text(
    object: &Map<String, JsonValue>,
    key: &str,
    expected: &str,
    kind: TelegramOperationalTriggerExecutionErrorKind,
) -> Result<(), TelegramOperationalTriggerExecutionError> {
    require_exact_text_kind(object, key, expected, kind).map_err(error)
}

fn require_exact_text_kind(
    object: &Map<String, JsonValue>,
    key: &str,
    expected: &str,
    kind: TelegramOperationalTriggerExecutionErrorKind,
) -> Result<(), TelegramOperationalTriggerExecutionErrorKind> {
    if object.get(key).and_then(JsonValue::as_str) == Some(expected) {
        Ok(())
    } else {
        Err(kind)
    }
}

fn require_exact_bool(
    object: &Map<String, JsonValue>,
    key: &str,
    expected: bool,
    kind: TelegramOperationalTriggerExecutionErrorKind,
) -> Result<(), TelegramOperationalTriggerExecutionError> {
    require_exact_bool_kind(object, key, expected, kind).map_err(error)
}

fn require_exact_bool_kind(
    object: &Map<String, JsonValue>,
    key: &str,
    expected: bool,
    kind: TelegramOperationalTriggerExecutionErrorKind,
) -> Result<(), TelegramOperationalTriggerExecutionErrorKind> {
    if object.get(key).and_then(JsonValue::as_bool) == Some(expected) {
        Ok(())
    } else {
        Err(kind)
    }
}

fn validated_command(
    value: Option<&JsonValue>,
) -> Result<JsonValue, TelegramOperationalTriggerExecutionError> {
    let Some(value) = value else {
        return Ok(JsonValue::Null);
    };
    if value.is_null() {
        return Ok(JsonValue::Null);
    }
    let values = value
        .as_array()
        .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))?;
    if values.len() != 2
        || values[0].as_str().is_none_or(|text| text.trim().is_empty())
        || values[1].as_str().is_none()
    {
        return Err(error(
            TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
        ));
    }
    Ok(value.clone())
}

fn validated_optional_positive_i64(
    value: Option<&JsonValue>,
) -> Result<(), TelegramOperationalTriggerExecutionError> {
    if value.is_none_or(|value| value.is_null() || value.as_i64().is_some_and(|number| number > 0))
    {
        Ok(())
    } else {
        Err(error(
            TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
        ))
    }
}

fn validated_positive_i64_array(
    value: Option<&JsonValue>,
) -> Result<(), TelegramOperationalTriggerExecutionError> {
    let Some(value) = value else {
        return Ok(());
    };
    let values = value
        .as_array()
        .ok_or_else(|| error(TelegramOperationalTriggerExecutionErrorKind::InvalidRequest))?;
    if values
        .iter()
        .all(|value| value.as_i64().is_some_and(|number| number > 0))
    {
        Ok(())
    } else {
        Err(error(
            TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
        ))
    }
}

fn validated_object_array(
    value: Option<&JsonValue>,
) -> Result<(), TelegramOperationalTriggerExecutionError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value
        .as_array()
        .is_some_and(|values| values.iter().all(JsonValue::is_object))
    {
        Ok(())
    } else {
        Err(error(
            TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
        ))
    }
}

fn validated_optional_string(
    value: Option<&JsonValue>,
) -> Result<(), TelegramOperationalTriggerExecutionError> {
    if value.is_none_or(|value| value.is_null() || value.is_string()) {
        Ok(())
    } else {
        Err(error(
            TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
        ))
    }
}

fn validated_optional_object(
    value: Option<&JsonValue>,
) -> Result<Option<&Map<String, JsonValue>>, TelegramOperationalTriggerExecutionError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(object)) => Ok(Some(object)),
        _ => Err(error(
            TelegramOperationalTriggerExecutionErrorKind::InvalidRequest,
        )),
    }
}

fn valid_chat_id(value: &JsonValue) -> bool {
    value.as_str().is_some_and(|text| !text.trim().is_empty())
        || value.as_i64().is_some_and(|number| number != 0)
}

fn error(
    kind: TelegramOperationalTriggerExecutionErrorKind,
) -> TelegramOperationalTriggerExecutionError {
    TelegramOperationalTriggerExecutionError::new(kind)
}

#[cfg(test)]
mod tests;
