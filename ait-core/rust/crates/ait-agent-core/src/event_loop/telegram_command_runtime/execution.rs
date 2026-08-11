use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use chrono::{SecondsFormat, Utc};

use super::planning::TelegramCommandRuntimePlanner;
use crate::event_loop::telegram_workflow_query::TelegramWorkflowQueryPlanner;
use crate::runtime::{
    AgentRuntimeBackend, AgentRuntimeBindingStore, SelectedAitRuntimeBackend,
    AGENT_RUNTIME_BACKEND_CONTRACT,
};
use crate::transport_config::{AgentRuntimeMode, AgentRuntimeTarget, AgentWorkflowMode};

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramCommandRuntimeExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_command_runtime_execution";
const PLANNING_CONTRACT: &str = "ait_agent_core.event_loop.TelegramCommandRuntime.v1";
const PLANNING_STAGE: &str = "rust_agent_telegram_command_runtime";
const WORKFLOW_QUERY_CONTRACT: &str = "ait_agent_core.event_loop.TelegramWorkflowQuery.v1";
const WORKFLOW_QUERY_STAGE: &str = "rust_agent_telegram_workflow_query";
const BOT_ACTOR_IDENTITY: &str = "ait-agent-telegram";
const BOT_ACTOR_TYPE: &str = "telegram_bot";
const MAX_DEPENDENCY_ROUNDS: usize = 4;
const MAX_TEXT_LENGTH: usize = 4_096;
const MAX_MESSAGE_TEXT_LENGTH: usize = 16_384;
const MAX_CLOCK_TEXT_LENGTH: usize = 128;

type DependencyAction<'a> = (String, &'a Map<String, JsonValue>);

pub trait TelegramCommandRuntimeReadPort: Send + Sync + 'static {
    fn read_workflow_notification(&self) -> Result<JsonValue, String> {
        self.read_task_queue()
    }

    fn read_task_queue(&self) -> Result<JsonValue, String>;

    fn read_task(&self, target_ref: &str) -> Result<JsonValue, String>;

    fn read_task_audit(&self, target_ref: &str) -> Result<JsonValue, String>;

    fn read_change(&self, target_ref: &str) -> Result<JsonValue, String>;
}

pub trait TelegramCommandRuntimeStatePort: Send + Sync + 'static {
    fn load_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String>;

    fn patch_chat(&self, chat_id: &JsonValue, patch: &JsonValue) -> Result<(), String>;
}

pub trait TelegramCommandRuntimeClockPort: Send + Sync + 'static {
    fn now_iso(&self) -> Result<String, String>;
}

pub trait TelegramCommandRuntimeDeliveryPort: Send + Sync + 'static {
    fn send_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct RuntimeBindingTelegramCommandRuntimeStatePort {
    store: AgentRuntimeBindingStore,
}

impl RuntimeBindingTelegramCommandRuntimeStatePort {
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

impl TelegramCommandRuntimeStatePort for RuntimeBindingTelegramCommandRuntimeStatePort {
    fn load_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        match self.store.execute(
            "get_binding",
            &json!({"transport": "telegram", "surface_id": chat_id}),
        )? {
            JsonValue::Null => Ok(None),
            value @ JsonValue::Object(_) => Ok(Some(value)),
            _ => Err("Runtime binding store returned an invalid Telegram binding.".to_string()),
        }
    }

    fn patch_chat(&self, chat_id: &JsonValue, patch: &JsonValue) -> Result<(), String> {
        validate_notification_patch(patch)
            .map_err(|_| "Telegram command runtime state patch is invalid.".to_string())?;
        let updates = patch
            .as_object()
            .ok_or_else(|| "Telegram command runtime patch must be an object.".to_string())?;
        let saved = self.store.execute(
            "patch_binding",
            &json!({
                "transport": "telegram",
                "surface_id": chat_id,
                "updates": updates,
            }),
        )?;
        match saved {
            JsonValue::Null | JsonValue::Object(_) => Ok(()),
            _ => Err("Runtime binding store returned an invalid Telegram patch.".to_string()),
        }
    }
}

#[derive(Clone)]
pub struct SelectedBackendTelegramCommandRuntimeReadPort<B = SelectedAitRuntimeBackend> {
    target: AgentRuntimeTarget,
    timeout_seconds: Option<f64>,
    backend: B,
}

impl<B> fmt::Debug for SelectedBackendTelegramCommandRuntimeReadPort<B> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectedBackendTelegramCommandRuntimeReadPort")
            .field("runtime_mode", &self.target.mode.as_str())
            .field("timeout_configured", &self.timeout_seconds.is_some())
            .field("runtime_target_exposed", &false)
            .finish()
    }
}

impl SelectedBackendTelegramCommandRuntimeReadPort {
    pub fn new(target: AgentRuntimeTarget, timeout_seconds: Option<f64>) -> Self {
        Self::with_backend(
            target,
            timeout_seconds,
            SelectedAitRuntimeBackend::default(),
        )
    }
}

impl<B> SelectedBackendTelegramCommandRuntimeReadPort<B> {
    pub fn with_backend(
        target: AgentRuntimeTarget,
        timeout_seconds: Option<f64>,
        backend: B,
    ) -> Self {
        Self {
            target,
            timeout_seconds,
            backend,
        }
    }

    pub fn target(&self) -> &AgentRuntimeTarget {
        &self.target
    }

    fn target_payload_for(target: &AgentRuntimeTarget) -> JsonValue {
        let mut payload = json!({
            "mode": target.mode.as_str(),
            "workflow_mode": target.workflow_mode.as_str(),
            "repo_name": target.repo_name,
        });
        match target.mode {
            AgentRuntimeMode::Local => {
                payload["repo_root"] = json!(target.repo_root.to_string_lossy());
            }
            AgentRuntimeMode::Remote => {
                payload["remote_name"] = optional_text_json(target.remote_name.as_deref());
                payload["server_url"] = optional_text_json(target.server_url.as_deref());
            }
        }
        payload
    }

    fn backend_request_for(
        &self,
        target: &AgentRuntimeTarget,
        operation: &str,
        arguments: JsonValue,
    ) -> JsonValue {
        json!({
            "operation": operation,
            "target": Self::target_payload_for(target),
            "actor": {
                "identity": BOT_ACTOR_IDENTITY,
                "type": BOT_ACTOR_TYPE,
            },
            "timeout_seconds": self.timeout_seconds,
            "arguments": arguments,
        })
    }
}

impl<B> SelectedBackendTelegramCommandRuntimeReadPort<B>
where
    B: AgentRuntimeBackend,
{
    fn read_object_for(
        &self,
        target: &AgentRuntimeTarget,
        operation: &str,
        arguments: JsonValue,
    ) -> Result<JsonValue, String> {
        let response = self
            .backend
            .execute(&self.backend_request_for(target, operation, arguments))?;
        let response = validated_backend_response(&response, operation)?;
        if response.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            return Err("Telegram command runtime backend read failed.".to_string());
        }
        response
            .get("payload")
            .filter(|value| value.is_object())
            .cloned()
            .ok_or_else(|| {
                "Telegram command runtime backend returned an invalid payload.".to_string()
            })
    }

    fn read_object(&self, operation: &str, arguments: JsonValue) -> Result<JsonValue, String> {
        self.read_object_for(&self.target, operation, arguments)
    }
}

impl<B> TelegramCommandRuntimeReadPort for SelectedBackendTelegramCommandRuntimeReadPort<B>
where
    B: AgentRuntimeBackend + Send + Sync + 'static,
{
    fn read_workflow_notification(&self) -> Result<JsonValue, String> {
        if self.target.workflow_mode != AgentWorkflowMode::SoloLocal {
            return notification_payload_with_source(self.read_task_queue()?, "remote_queue");
        }
        if self.target.mode != AgentRuntimeMode::Local {
            return Err(
                "solo_local workflow notifications require a local runtime target".to_string(),
            );
        }
        if self.target.server_url.is_some() {
            let mut remote_target = self.target.clone();
            remote_target.mode = AgentRuntimeMode::Remote;
            if let Ok(payload) = self.read_object_for(&remote_target, "read_task_queue", json!({}))
            {
                return notification_payload_with_source(payload, "remote_queue");
            }
        }
        notification_payload_with_source(
            self.read_object("read_current_workflow", json!({}))?,
            "local_current",
        )
    }

    fn read_task_queue(&self) -> Result<JsonValue, String> {
        self.read_object("read_task_queue", json!({}))
    }

    fn read_task(&self, target_ref: &str) -> Result<JsonValue, String> {
        self.read_object("read_task", json!({"task_id": target_ref}))
    }

    fn read_task_audit(&self, target_ref: &str) -> Result<JsonValue, String> {
        self.read_object("read_task_audit", json!({"task_id": target_ref}))
    }

    fn read_change(&self, target_ref: &str) -> Result<JsonValue, String> {
        self.read_object("read_change", json!({"change_id": target_ref}))
    }
}

fn notification_payload_with_source(payload: JsonValue, source: &str) -> Result<JsonValue, String> {
    let mut payload = payload
        .as_object()
        .cloned()
        .ok_or_else(|| "Telegram workflow notification payload must be an object.".to_string())?;
    payload.insert(
        "notification_source".to_string(),
        JsonValue::String(source.to_string()),
    );
    Ok(JsonValue::Object(payload))
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTelegramCommandRuntimeClockPort;

impl TelegramCommandRuntimeClockPort for SystemTelegramCommandRuntimeClockPort {
    fn now_iso(&self) -> Result<String, String> {
        Ok(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramCommandRuntimeExecutionErrorKind {
    InvalidRequest,
    Configuration,
    Planner,
    PlannerContract,
    WorkflowQuery,
    RuntimeRead,
    State,
    Clock,
    Delivery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramCommandRuntimeExecutionError {
    kind: TelegramCommandRuntimeExecutionErrorKind,
}

impl TelegramCommandRuntimeExecutionError {
    pub fn kind(&self) -> TelegramCommandRuntimeExecutionErrorKind {
        self.kind
    }

    fn new(kind: TelegramCommandRuntimeExecutionErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for TelegramCommandRuntimeExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TelegramCommandRuntimeExecutionErrorKind::InvalidRequest => {
                "Telegram command runtime execution request is invalid."
            }
            TelegramCommandRuntimeExecutionErrorKind::Configuration => {
                "Telegram command runtime execution configuration is invalid."
            }
            TelegramCommandRuntimeExecutionErrorKind::Planner => {
                "Telegram command runtime planning failed."
            }
            TelegramCommandRuntimeExecutionErrorKind::PlannerContract => {
                "Telegram command runtime planner contract is invalid."
            }
            TelegramCommandRuntimeExecutionErrorKind::WorkflowQuery => {
                "Telegram command runtime workflow query failed."
            }
            TelegramCommandRuntimeExecutionErrorKind::RuntimeRead => {
                "Telegram command runtime backend read failed."
            }
            TelegramCommandRuntimeExecutionErrorKind::State => {
                "Telegram command runtime state execution failed."
            }
            TelegramCommandRuntimeExecutionErrorKind::Clock => {
                "Telegram command runtime clock execution failed."
            }
            TelegramCommandRuntimeExecutionErrorKind::Delivery => {
                "Telegram command runtime message delivery failed."
            }
        })
    }
}

impl std::error::Error for TelegramCommandRuntimeExecutionError {}

#[allow(clippy::too_many_arguments)]
pub fn execute_with_telegram_command_runtime_ports<P, Q, R, S, C, D>(
    planner: &P,
    workflow_query: &Q,
    reads: &R,
    state: &S,
    clock: &C,
    delivery: &D,
    config: &JsonValue,
    request: &JsonValue,
) -> Result<JsonValue, TelegramCommandRuntimeExecutionError>
where
    P: TelegramCommandRuntimePlanner + ?Sized,
    Q: TelegramWorkflowQueryPlanner + ?Sized,
    R: TelegramCommandRuntimeReadPort + ?Sized,
    S: TelegramCommandRuntimeStatePort + ?Sized,
    C: TelegramCommandRuntimeClockPort + ?Sized,
    D: TelegramCommandRuntimeDeliveryPort + ?Sized,
{
    let request = validated_request(request)?;
    let config = validated_config(config)?;
    let dispatch_request = json!({
        "stage": "dispatch_command",
        "chat_id": request.chat_id,
        "name": request.name,
        "args": request.args,
    });
    let dispatch_plan = plan(planner, &dispatch_request)?;
    let dispatch_actions = validated_actions(&dispatch_plan, "dispatch_command")?;
    if dispatch_actions.len() != 1 {
        return Err(contract_error());
    }

    let expected_command = canonical_command_name(&request.name);
    validate_dispatch_plan(&dispatch_plan, &request, expected_command)?;
    let dispatch_action = action_object(&dispatch_actions[0])?;
    let action_kind = required_text(dispatch_action.get("kind"))?;
    match action_kind.as_str() {
        "send_message" => {
            if expected_command.is_some_and(|name| name != "ping") {
                return Err(contract_error());
            }
            let text = validated_send_message_action(dispatch_action, &request.chat_id)?;
            if dispatch_plan
                .get("message_text")
                .and_then(JsonValue::as_str)
                != Some(text.as_str())
            {
                return Err(contract_error());
            }
            delivery
                .send_message(&request.chat_id, &text)
                .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::Delivery))?;
            Ok(execution_result(
                "message_delivered",
                ExecutionFacts {
                    message_sent: true,
                    ..ExecutionFacts::default()
                },
            ))
        }
        "run_command_runtime_stage" => {
            let command_name = expected_command.ok_or_else(contract_error)?;
            if command_name == "ping" {
                return Err(contract_error());
            }
            let mut stage_request = build_stage_request(
                dispatch_action,
                command_name,
                &request,
                &config,
                workflow_query,
                state,
                clock,
            )?;
            execute_stage(
                planner,
                reads,
                state,
                delivery,
                &request.chat_id,
                &mut stage_request,
            )
        }
        _ => Err(contract_error()),
    }
}

#[derive(Debug, Default)]
struct ExecutionFacts {
    dependency_rounds: usize,
    queue_read: bool,
    detail_read: bool,
    binding_loaded: bool,
    state_patched: bool,
    message_sent: bool,
}

fn execution_result(decision: &str, facts: ExecutionFacts) -> JsonValue {
    let side_effect_count = usize::from(facts.state_patched) + usize::from(facts.message_sent);
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "command_runtime_state": "completed",
        "ok": true,
        "completed": true,
        "decision": decision,
        "dependency_rounds": facts.dependency_rounds,
        "queue_read": facts.queue_read,
        "detail_read": facts.detail_read,
        "binding_loaded": facts.binding_loaded,
        "state_patched": facts.state_patched,
        "message_sent": facts.message_sent,
        "side_effect_count": side_effect_count,
        "rust_command_execution_required": true,
        "python_command_runtime_allowed": false,
        "python_runtime_read_allowed": false,
        "python_state_mutation_allowed": false,
        "python_message_delivery_allowed": false,
        "request_payload_exposed": false,
        "config_exposed": false,
        "planner_payload_exposed": false,
        "dependency_payload_exposed": false,
        "chat_id_exposed": false,
        "target_ref_exposed": false,
        "message_text_exposed": false,
        "state_patch_exposed": false,
    })
}

struct ValidatedRequest {
    chat_id: JsonValue,
    chat: JsonValue,
    name: String,
    args: String,
}

fn validated_request(
    request: &JsonValue,
) -> Result<ValidatedRequest, TelegramCommandRuntimeExecutionError> {
    let object = request
        .as_object()
        .ok_or_else(|| execution_error(TelegramCommandRuntimeExecutionErrorKind::InvalidRequest))?;
    validate_only_fields(
        object,
        &["chat_id", "chat", "from_user", "chat_title", "name", "args"],
    )
    .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::InvalidRequest))?;
    let chat_id = object
        .get("chat_id")
        .cloned()
        .ok_or_else(|| execution_error(TelegramCommandRuntimeExecutionErrorKind::InvalidRequest))?;
    if !valid_chat_id(&chat_id) {
        return Err(execution_error(
            TelegramCommandRuntimeExecutionErrorKind::InvalidRequest,
        ));
    }
    let chat = object
        .get("chat")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| execution_error(TelegramCommandRuntimeExecutionErrorKind::InvalidRequest))?;
    object
        .get("from_user")
        .filter(|value| value.is_object())
        .ok_or_else(|| execution_error(TelegramCommandRuntimeExecutionErrorKind::InvalidRequest))?;
    bounded_text(object.get("chat_title"), MAX_TEXT_LENGTH, true)
        .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::InvalidRequest))?;
    let name = bounded_text(object.get("name"), MAX_TEXT_LENGTH, false)
        .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::InvalidRequest))?
        .trim_start_matches('/')
        .to_ascii_lowercase();
    let args = bounded_text(object.get("args"), MAX_TEXT_LENGTH, true)
        .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::InvalidRequest))?;
    Ok(ValidatedRequest {
        chat_id,
        chat,
        name,
        args,
    })
}

struct ValidatedConfig {
    planner_payload: JsonValue,
    username: Option<String>,
}

fn validated_config(
    config: &JsonValue,
) -> Result<ValidatedConfig, TelegramCommandRuntimeExecutionError> {
    let object = config
        .as_object()
        .ok_or_else(|| execution_error(TelegramCommandRuntimeExecutionErrorKind::Configuration))?;
    validate_only_fields(
        object,
        &[
            "repo_name",
            "ait_web_url",
            "background_sync_enabled",
            "background_sync_interval_seconds",
            "runtime_mode",
            "runtime_remote_name",
            "username",
        ],
    )
    .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::Configuration))?;

    let mut planner = Map::new();
    for key in [
        "repo_name",
        "ait_web_url",
        "runtime_mode",
        "runtime_remote_name",
    ] {
        if let Some(value) = object.get(key) {
            if !value.is_null() {
                let text = bounded_text(Some(value), MAX_TEXT_LENGTH, false).map_err(|_| {
                    execution_error(TelegramCommandRuntimeExecutionErrorKind::Configuration)
                })?;
                planner.insert(key.to_string(), json!(text));
            }
        }
    }
    if let Some(value) = object.get("background_sync_enabled") {
        let enabled = value.as_bool().ok_or_else(|| {
            execution_error(TelegramCommandRuntimeExecutionErrorKind::Configuration)
        })?;
        planner.insert("background_sync_enabled".to_string(), json!(enabled));
    } else {
        planner.insert("background_sync_enabled".to_string(), json!(false));
    }
    if let Some(value) = object.get("background_sync_interval_seconds") {
        if !value.is_null() {
            let number = value
                .as_f64()
                .filter(|number| number.is_finite() && *number >= 0.0);
            let number = number.ok_or_else(|| {
                execution_error(TelegramCommandRuntimeExecutionErrorKind::Configuration)
            })?;
            planner.insert(
                "background_sync_interval_seconds".to_string(),
                json!(number),
            );
        }
    }
    let username = object
        .get("username")
        .filter(|value| !value.is_null())
        .map(|value| bounded_text(Some(value), MAX_TEXT_LENGTH, false))
        .transpose()
        .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::Configuration))?;
    Ok(ValidatedConfig {
        planner_payload: JsonValue::Object(planner),
        username,
    })
}

#[allow(clippy::too_many_arguments)]
fn build_stage_request<Q, S, C>(
    action: &Map<String, JsonValue>,
    command_name: &str,
    request: &ValidatedRequest,
    config: &ValidatedConfig,
    workflow_query: &Q,
    state: &S,
    clock: &C,
) -> Result<Map<String, JsonValue>, TelegramCommandRuntimeExecutionError>
where
    Q: TelegramWorkflowQueryPlanner + ?Sized,
    S: TelegramCommandRuntimeStatePort + ?Sized,
    C: TelegramCommandRuntimeClockPort + ?Sized,
{
    validate_only_fields(
        action,
        &[
            "kind",
            "command_name",
            "requested_name",
            "args",
            "stage",
            "stage_request",
            "binding_policy",
            "include_chat",
            "include_username",
            "include_config",
            "include_observed_at",
            "target_ref_query",
            "target_ref_expected_kind",
        ],
    )?;
    let spec = stage_spec(command_name, &request.args).ok_or_else(contract_error)?;
    if required_text(action.get("command_name"))? != command_name
        || required_text(action.get("requested_name"))? != request.name
        || required_text_allow_empty(action.get("args"))? != request.args
        || required_text(action.get("stage"))? != spec.stage
        || required_text(action.get("binding_policy"))? != spec.binding_policy
        || required_bool(action.get("include_chat"))? != spec.include_chat
        || required_bool(action.get("include_username"))? != spec.include_username
        || !required_bool(action.get("include_config"))?
        || required_bool(action.get("include_observed_at"))? != spec.include_observed_at
        || optional_text(action.get("target_ref_query")) != spec.target_ref_query
        || optional_text(action.get("target_ref_expected_kind")) != spec.target_ref_expected_kind
    {
        return Err(contract_error());
    }

    let planner_chat_id =
        JsonValue::String(chat_id_text(&request.chat_id).ok_or_else(contract_error)?);
    let expected_action_base =
        stage_base_request(command_name, spec.stage, &planner_chat_id, &request.args);
    if action.get("stage_request") != Some(&JsonValue::Object(expected_action_base)) {
        return Err(contract_error());
    }
    let mut stage_request =
        stage_base_request(command_name, spec.stage, &request.chat_id, &request.args);
    stage_request.insert("config".to_string(), config.planner_payload.clone());
    if spec.include_chat {
        stage_request.insert("chat".to_string(), sanitized_chat(&request.chat));
    }
    if spec.include_username {
        if let Some(username) = config.username.as_ref() {
            stage_request.insert("username".to_string(), json!(username));
        }
    }
    if spec.include_observed_at {
        let observed_at = clock
            .now_iso()
            .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::Clock))?;
        let observed_at = validated_clock_text(&observed_at)?;
        stage_request.insert("observed_at".to_string(), json!(observed_at));
    }

    let mut binding_loaded = false;
    if spec.binding_policy == "read_existing" {
        let binding = state
            .load_binding(&request.chat_id)
            .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::State))?;
        if binding.as_ref().is_some_and(|value| !value.is_object()) {
            return Err(execution_error(
                TelegramCommandRuntimeExecutionErrorKind::State,
            ));
        }
        binding_loaded = binding.is_some();
        stage_request.insert("binding".to_string(), binding.unwrap_or(JsonValue::Null));
    }
    stage_request.insert(
        "_executor_binding_loaded".to_string(),
        json!(binding_loaded),
    );

    if let (Some(query_text), Some(expected_kind)) = (
        spec.target_ref_query.as_deref(),
        spec.target_ref_expected_kind.as_deref(),
    ) {
        if let Some((kind, target)) = detect_workflow_query(workflow_query, query_text)? {
            if kind == expected_kind {
                let target = target
                    .map(|value| value.trim().to_ascii_uppercase())
                    .filter(|value| !value.is_empty());
                stage_request.insert(
                    "target_ref".to_string(),
                    target.map(JsonValue::String).unwrap_or(JsonValue::Null),
                );
            }
        }
    }
    Ok(stage_request)
}

fn execute_stage<P, R, S, D>(
    planner: &P,
    reads: &R,
    state: &S,
    delivery: &D,
    chat_id: &JsonValue,
    stage_request: &mut Map<String, JsonValue>,
) -> Result<JsonValue, TelegramCommandRuntimeExecutionError>
where
    P: TelegramCommandRuntimePlanner + ?Sized,
    R: TelegramCommandRuntimeReadPort + ?Sized,
    S: TelegramCommandRuntimeStatePort + ?Sized,
    D: TelegramCommandRuntimeDeliveryPort + ?Sized,
{
    let binding_loaded = stage_request
        .remove("_executor_binding_loaded")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let expected_stage = required_text(stage_request.get("stage"))?;
    let mut seen_dependencies = HashSet::new();
    let mut facts = ExecutionFacts {
        binding_loaded,
        ..ExecutionFacts::default()
    };

    loop {
        let planned = plan(planner, &JsonValue::Object(stage_request.clone()))?;
        let actions = validated_actions(&planned, &expected_stage)?;
        if actions.is_empty() {
            return Err(contract_error());
        }
        let dependency = dependency_action(actions)?;
        if let Some((kind, action)) = dependency {
            if facts.dependency_rounds >= MAX_DEPENDENCY_ROUNDS
                || !seen_dependencies.insert(kind.clone())
            {
                return Err(contract_error());
            }
            apply_dependency(&kind, action, reads, stage_request, &mut facts)?;
            facts.dependency_rounds += 1;
            continue;
        }
        validate_terminal_plan_message(&planned, actions)?;
        apply_terminal_actions(actions, state, delivery, chat_id, &mut facts)?;
        return Ok(execution_result("stage_completed", facts));
    }
}

fn apply_dependency<R>(
    kind: &str,
    action: &Map<String, JsonValue>,
    reads: &R,
    request: &mut Map<String, JsonValue>,
    facts: &mut ExecutionFacts,
) -> Result<(), TelegramCommandRuntimeExecutionError>
where
    R: TelegramCommandRuntimeReadPort + ?Sized,
{
    match kind {
        "read_task_queue" => {
            validate_only_fields(action, &["kind"])?;
            let payload = reads.read_task_queue().map_err(|_| {
                execution_error(TelegramCommandRuntimeExecutionErrorKind::RuntimeRead)
            })?;
            require_object_payload(payload, "queue_payload", request)?;
            facts.queue_read = true;
        }
        "read_task" | "read_task_audit" | "read_change" | "read_change_land" => {
            validate_only_fields(action, &["kind", "target_ref"])?;
            let target_ref = required_text(action.get("target_ref"))?;
            if request.get("target_ref").and_then(JsonValue::as_str) != Some(target_ref.as_str()) {
                return Err(contract_error());
            }
            let payload = match kind {
                "read_task" => reads.read_task(&target_ref),
                "read_task_audit" => reads.read_task_audit(&target_ref),
                _ => reads.read_change(&target_ref),
            }
            .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::RuntimeRead))?;
            require_object_payload(payload, "detail_payload", request)?;
            facts.detail_read = true;
        }
        _ => return Err(contract_error()),
    }
    Ok(())
}

fn apply_terminal_actions<S, D>(
    actions: &[JsonValue],
    state: &S,
    delivery: &D,
    chat_id: &JsonValue,
    facts: &mut ExecutionFacts,
) -> Result<(), TelegramCommandRuntimeExecutionError>
where
    S: TelegramCommandRuntimeStatePort + ?Sized,
    D: TelegramCommandRuntimeDeliveryPort + ?Sized,
{
    if actions.len() > 2 {
        return Err(contract_error());
    }
    let kinds = actions
        .iter()
        .map(action_object)
        .map(|action| action.and_then(|action| required_text(action.get("kind"))))
        .collect::<Result<Vec<_>, _>>()?;
    if kinds.as_slice() != ["send_message"] && kinds.as_slice() != ["patch_chat", "send_message"] {
        return Err(contract_error());
    }
    for action in actions {
        let action = action_object(action)?;
        match required_text(action.get("kind"))?.as_str() {
            "patch_chat" => {
                validate_only_fields(action, &["kind", "chat_id", "patch"])?;
                validate_matching_chat_id(action.get("chat_id"), chat_id)?;
                let patch = action.get("patch").ok_or_else(contract_error)?;
                validate_notification_patch(patch)?;
                state.patch_chat(chat_id, patch).map_err(|_| {
                    execution_error(TelegramCommandRuntimeExecutionErrorKind::State)
                })?;
                facts.state_patched = true;
            }
            "send_message" => {
                let text = validated_send_message_action(action, chat_id)?;
                delivery.send_message(chat_id, &text).map_err(|_| {
                    execution_error(TelegramCommandRuntimeExecutionErrorKind::Delivery)
                })?;
                facts.message_sent = true;
            }
            _ => return Err(contract_error()),
        }
    }
    Ok(())
}

fn dependency_action(
    actions: &[JsonValue],
) -> Result<Option<DependencyAction<'_>>, TelegramCommandRuntimeExecutionError> {
    let mut dependency = None;
    for action in actions {
        let action = action_object(action)?;
        let kind = required_text(action.get("kind"))?;
        if matches!(
            kind.as_str(),
            "read_task_queue"
                | "read_task"
                | "read_task_audit"
                | "read_change"
                | "read_change_land"
        ) {
            if dependency.is_some() {
                return Err(contract_error());
            }
            dependency = Some((kind, action));
        }
    }
    if dependency.is_some() && actions.len() != 1 {
        return Err(contract_error());
    }
    Ok(dependency)
}

struct StageSpec {
    stage: &'static str,
    binding_policy: &'static str,
    include_chat: bool,
    include_username: bool,
    include_observed_at: bool,
    target_ref_query: Option<String>,
    target_ref_expected_kind: Option<String>,
}

fn stage_spec(command_name: &str, args: &str) -> Option<StageSpec> {
    let spec = match command_name {
        "help" => StageSpec {
            stage: "help_text",
            binding_policy: "read_existing",
            include_chat: true,
            include_username: true,
            include_observed_at: false,
            target_ref_query: None,
            target_ref_expected_kind: None,
        },
        "queue" | "attention" | "ready" => StageSpec {
            stage: "queue_summary_command",
            binding_policy: "none",
            include_chat: false,
            include_username: false,
            include_observed_at: false,
            target_ref_query: None,
            target_ref_expected_kind: None,
        },
        "task" | "audit" | "change" | "land" => StageSpec {
            stage: "workflow_detail_command",
            binding_policy: "none",
            include_chat: false,
            include_username: false,
            include_observed_at: false,
            target_ref_query: Some(if args.trim().is_empty() {
                command_name.to_string()
            } else {
                format!("{command_name} {}", args.trim())
            }),
            target_ref_expected_kind: Some(command_name.to_string()),
        },
        "notify" => StageSpec {
            stage: "notify_command",
            binding_policy: "read_existing",
            include_chat: false,
            include_username: false,
            include_observed_at: true,
            target_ref_query: None,
            target_ref_expected_kind: None,
        },
        _ => return None,
    };
    Some(spec)
}

fn stage_base_request(
    command_name: &str,
    stage: &str,
    chat_id: &JsonValue,
    args: &str,
) -> Map<String, JsonValue> {
    let mut request = Map::new();
    request.insert("chat_id".to_string(), chat_id.clone());
    request.insert("stage".to_string(), json!(stage));
    match command_name {
        "queue" | "attention" | "ready" => {
            request.insert("summary_kind".to_string(), json!(command_name));
        }
        "task" | "audit" | "change" | "land" => {
            request.insert("command_name".to_string(), json!(command_name));
        }
        "notify" => {
            request.insert("args".to_string(), json!(args));
        }
        _ => {}
    }
    request
}

fn canonical_command_name(name: &str) -> Option<&'static str> {
    match name {
        "start" | "help" => Some("help"),
        "queue" => Some("queue"),
        "attention" => Some("attention"),
        "ready" => Some("ready"),
        "task" => Some("task"),
        "audit" => Some("audit"),
        "change" => Some("change"),
        "land" => Some("land"),
        "notify" => Some("notify"),
        "ping" => Some("ping"),
        _ => None,
    }
}

fn validate_dispatch_plan(
    planned: &JsonValue,
    request: &ValidatedRequest,
    expected_command: Option<&str>,
) -> Result<(), TelegramCommandRuntimeExecutionError> {
    let object = planned.as_object().ok_or_else(contract_error)?;
    if required_text(object.get("requested_name"))? != request.name
        || required_text_allow_empty(object.get("args"))? != request.args
    {
        return Err(contract_error());
    }
    let (expected_mode, expected_name) = match expected_command {
        Some("ping") => ("ping", "ping"),
        Some(command_name) => ("stage", command_name),
        None => ("unknown", request.name.as_str()),
    };
    if required_text(object.get("mode"))? != expected_mode
        || required_text(object.get("command_name"))? != expected_name
    {
        return Err(contract_error());
    }
    Ok(())
}

fn validate_terminal_plan_message(
    planned: &JsonValue,
    actions: &[JsonValue],
) -> Result<(), TelegramCommandRuntimeExecutionError> {
    let planned_message = planned
        .get("message_text")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    let action_message = actions
        .iter()
        .filter_map(JsonValue::as_object)
        .find(|action| action.get("kind").and_then(JsonValue::as_str) == Some("send_message"))
        .and_then(|action| action.get("message_text"))
        .and_then(JsonValue::as_str)
        .ok_or_else(contract_error)?;
    if planned_message != action_message {
        return Err(contract_error());
    }
    Ok(())
}

fn detect_workflow_query<Q>(
    planner: &Q,
    text: &str,
) -> Result<Option<(String, Option<String>)>, TelegramCommandRuntimeExecutionError>
where
    Q: TelegramWorkflowQueryPlanner + ?Sized,
{
    let planned = planner
        .plan_json(&json!({"kind": "detect_workflow_query", "text": text}))
        .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::WorkflowQuery))?;
    let object = planned.as_object().ok_or_else(contract_error)?;
    if object.get("migration_stage").and_then(JsonValue::as_str) != Some(WORKFLOW_QUERY_STAGE)
        || object
            .get("workflow_query_contract")
            .and_then(JsonValue::as_str)
            != Some(WORKFLOW_QUERY_CONTRACT)
        || object.get("kind").and_then(JsonValue::as_str) != Some("detect_workflow_query")
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object
            .get("python_workflow_query_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(contract_error());
    }
    let matched = object
        .get("matched")
        .and_then(JsonValue::as_bool)
        .ok_or_else(contract_error)?;
    if !matched {
        return Ok(None);
    }
    let kind = required_text(object.get("query_kind"))?;
    let reference = optional_text(object.get("query_ref"));
    Ok(Some((kind, reference)))
}

fn plan<P>(
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, TelegramCommandRuntimeExecutionError>
where
    P: TelegramCommandRuntimePlanner + ?Sized,
{
    planner
        .plan_json(request)
        .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::Planner))
}

fn validated_actions<'a>(
    planned: &'a JsonValue,
    expected_stage: &str,
) -> Result<&'a Vec<JsonValue>, TelegramCommandRuntimeExecutionError> {
    let object = planned.as_object().ok_or_else(contract_error)?;
    if object.get("migration_stage").and_then(JsonValue::as_str) != Some(PLANNING_STAGE)
        || object
            .get("command_runtime_contract")
            .and_then(JsonValue::as_str)
            != Some(PLANNING_CONTRACT)
        || object.get("execution_kind").and_then(JsonValue::as_str)
            != Some("telegram_command_runtime")
        || object.get("stage").and_then(JsonValue::as_str) != Some(expected_stage)
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
    {
        return Err(contract_error());
    }
    let actions = object
        .get("actions")
        .and_then(JsonValue::as_array)
        .ok_or_else(contract_error)?;
    if object.get("action_count").and_then(JsonValue::as_u64) != Some(actions.len() as u64) {
        return Err(contract_error());
    }
    Ok(actions)
}

fn validated_send_message_action(
    action: &Map<String, JsonValue>,
    chat_id: &JsonValue,
) -> Result<String, TelegramCommandRuntimeExecutionError> {
    validate_only_fields(action, &["kind", "chat_id", "message_text"])?;
    validate_matching_chat_id(action.get("chat_id"), chat_id)?;
    bounded_text(action.get("message_text"), MAX_MESSAGE_TEXT_LENGTH, false)
        .map_err(|_| contract_error())
}

fn validate_notification_patch(
    patch: &JsonValue,
) -> Result<(), TelegramCommandRuntimeExecutionError> {
    let patch = patch.as_object().ok_or_else(contract_error)?;
    validate_only_fields(
        patch,
        &[
            "workflow_notifications_enabled",
            "last_queue_summary_digest",
            "last_queue_notification_at",
        ],
    )?;
    if patch.is_empty()
        || patch
            .get("workflow_notifications_enabled")
            .and_then(JsonValue::as_bool)
            .is_none()
    {
        return Err(contract_error());
    }
    for key in ["last_queue_summary_digest", "last_queue_notification_at"] {
        if let Some(value) = patch.get(key) {
            if !value.is_null()
                && value
                    .as_str()
                    .map(str::trim)
                    .filter(|value| !value.is_empty() && value.len() <= MAX_TEXT_LENGTH)
                    .is_none()
            {
                return Err(contract_error());
            }
        }
    }
    Ok(())
}

fn require_object_payload(
    payload: JsonValue,
    field: &str,
    request: &mut Map<String, JsonValue>,
) -> Result<(), TelegramCommandRuntimeExecutionError> {
    if !payload.is_object() {
        return Err(execution_error(
            TelegramCommandRuntimeExecutionErrorKind::RuntimeRead,
        ));
    }
    request.insert(field.to_string(), payload);
    Ok(())
}

fn sanitized_chat(chat: &JsonValue) -> JsonValue {
    let chat_type = chat
        .as_object()
        .and_then(|chat| chat.get("type"))
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= MAX_TEXT_LENGTH);
    match chat_type {
        Some(chat_type) => json!({"type": chat_type}),
        None => json!({}),
    }
}

fn validate_matching_chat_id(
    actual: Option<&JsonValue>,
    expected: &JsonValue,
) -> Result<(), TelegramCommandRuntimeExecutionError> {
    let actual = actual.ok_or_else(contract_error)?;
    if chat_id_text(actual).as_deref() != chat_id_text(expected).as_deref() {
        return Err(contract_error());
    }
    Ok(())
}

fn valid_chat_id(value: &JsonValue) -> bool {
    chat_id_text(value).is_some()
}

fn chat_id_text(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(value) => {
            let value = value.trim();
            (!value.is_empty() && value.len() <= MAX_TEXT_LENGTH).then(|| value.to_string())
        }
        JsonValue::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn validated_clock_text(value: &str) -> Result<String, TelegramCommandRuntimeExecutionError> {
    let value = value.trim();
    if value.is_empty() || value.len() > MAX_CLOCK_TEXT_LENGTH {
        return Err(execution_error(
            TelegramCommandRuntimeExecutionErrorKind::Clock,
        ));
    }
    chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| execution_error(TelegramCommandRuntimeExecutionErrorKind::Clock))?;
    Ok(value.to_string())
}

fn validated_backend_response<'a>(
    response: &'a JsonValue,
    operation: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    let response = response
        .as_object()
        .ok_or_else(|| "Telegram command runtime backend returned a non-object.".to_string())?;
    if response.get("contract").and_then(JsonValue::as_str) != Some(AGENT_RUNTIME_BACKEND_CONTRACT)
        || response.get("operation").and_then(JsonValue::as_str) != Some(operation)
        || response.get("ok").and_then(JsonValue::as_bool).is_none()
        || response
            .get("backend")
            .and_then(JsonValue::as_str)
            .is_none()
    {
        return Err("Telegram command runtime backend contract is invalid.".to_string());
    }
    Ok(response)
}

fn optional_text_json(value: Option<&str>) -> JsonValue {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(JsonValue::from)
        .unwrap_or(JsonValue::Null)
}

fn action_object(
    action: &JsonValue,
) -> Result<&Map<String, JsonValue>, TelegramCommandRuntimeExecutionError> {
    action.as_object().ok_or_else(contract_error)
}

fn validate_only_fields(
    object: &Map<String, JsonValue>,
    fields: &[&str],
) -> Result<(), TelegramCommandRuntimeExecutionError> {
    if object.keys().any(|key| !fields.contains(&key.as_str())) {
        return Err(contract_error());
    }
    Ok(())
}

fn required_text(
    value: Option<&JsonValue>,
) -> Result<String, TelegramCommandRuntimeExecutionError> {
    bounded_text(value, MAX_TEXT_LENGTH, false).map_err(|_| contract_error())
}

fn required_text_allow_empty(
    value: Option<&JsonValue>,
) -> Result<String, TelegramCommandRuntimeExecutionError> {
    bounded_text(value, MAX_TEXT_LENGTH, true).map_err(|_| contract_error())
}

fn optional_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .filter(|value| !value.is_null())
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= MAX_TEXT_LENGTH)
        .map(str::to_string)
}

fn bounded_text(value: Option<&JsonValue>, max: usize, allow_empty: bool) -> Result<String, ()> {
    let value = value.and_then(JsonValue::as_str).ok_or(())?.trim();
    if value.len() > max || (!allow_empty && value.is_empty()) {
        return Err(());
    }
    Ok(value.to_string())
}

fn required_bool(value: Option<&JsonValue>) -> Result<bool, TelegramCommandRuntimeExecutionError> {
    value
        .and_then(JsonValue::as_bool)
        .ok_or_else(contract_error)
}

fn contract_error() -> TelegramCommandRuntimeExecutionError {
    execution_error(TelegramCommandRuntimeExecutionErrorKind::PlannerContract)
}

fn execution_error(
    kind: TelegramCommandRuntimeExecutionErrorKind,
) -> TelegramCommandRuntimeExecutionError {
    TelegramCommandRuntimeExecutionError::new(kind)
}

#[cfg(test)]
mod solo_local_workflow_notification_tests {
    use std::path::PathBuf;
    use std::sync::Mutex;

    use super::*;
    use crate::event_loop::telegram_workflow_notifications::agent_telegram_workflow_notification_format_json;

    struct HybridBackend {
        remote_available: bool,
        requests: Mutex<Vec<JsonValue>>,
    }

    impl HybridBackend {
        fn new(remote_available: bool) -> Self {
            Self {
                remote_available,
                requests: Mutex::new(Vec::new()),
            }
        }
    }

    impl AgentRuntimeBackend for HybridBackend {
        fn execute(&self, request: &JsonValue) -> Result<JsonValue, String> {
            self.requests.lock().unwrap().push(request.clone());
            let operation = request["operation"].as_str().unwrap();
            let mode = request["target"]["mode"].as_str().unwrap();
            if mode == "remote" && !self.remote_available {
                return Ok(json!({
                    "contract": AGENT_RUNTIME_BACKEND_CONTRACT,
                    "operation": operation,
                    "backend": "remote",
                    "ok": false,
                }));
            }
            let payload = if mode == "remote" {
                json!({
                    "items": [{
                        "task": {"task_id": "RT-1", "title": "Remote task"},
                        "workflow": {"state": "attention_required", "reason": "Policy pending"},
                        "next_action": {"code": "inspect_policy"}
                    }]
                })
            } else {
                assert_eq!(operation, "read_current_workflow");
                json!({
                    "notification_source": "local_current",
                    "items": [{
                        "task": {"task_id": "LT-1", "title": "Local task"},
                        "focus_change": {"change_id": "LC-1", "status": "draft"},
                        "workflow": {"state": "in_progress", "reason": "Local work"},
                        "next_action": {"code": "continue_change"},
                        "updated_at": "2026-07-19T00:00:00Z"
                    }]
                })
            };
            Ok(json!({
                "contract": AGENT_RUNTIME_BACKEND_CONTRACT,
                "operation": operation,
                "backend": mode,
                "ok": true,
                "payload": payload,
            }))
        }
    }

    fn target(
        workflow_mode: AgentWorkflowMode,
        mode: AgentRuntimeMode,
        with_server: bool,
    ) -> AgentRuntimeTarget {
        AgentRuntimeTarget {
            mode,
            workflow_mode,
            repo_name: "fixture".to_string(),
            repo_root: PathBuf::from("/tmp/fixture"),
            remote_name: with_server.then(|| "origin".to_string()),
            server_url: with_server.then(|| "http://127.0.0.1:8088".to_string()),
        }
    }

    #[test]
    fn solo_local_prefers_optional_server_and_falls_back_to_one_local_current_item() {
        let connected_backend = HybridBackend::new(true);
        let connected = SelectedBackendTelegramCommandRuntimeReadPort::with_backend(
            target(AgentWorkflowMode::SoloLocal, AgentRuntimeMode::Local, true),
            Some(1.0),
            connected_backend,
        );
        let remote = connected
            .read_workflow_notification()
            .expect("remote notification");
        assert_eq!(remote["notification_source"], "remote_queue");
        assert_eq!(connected.backend.requests.lock().unwrap().len(), 1);
        assert_eq!(
            connected.backend.requests.lock().unwrap()[0]["target"]["mode"],
            "remote"
        );

        let disconnected_backend = HybridBackend::new(false);
        let disconnected = SelectedBackendTelegramCommandRuntimeReadPort::with_backend(
            target(AgentWorkflowMode::SoloLocal, AgentRuntimeMode::Local, true),
            Some(1.0),
            disconnected_backend,
        );
        let local = disconnected
            .read_workflow_notification()
            .expect("local fallback");
        assert_eq!(local["notification_source"], "local_current");
        assert_eq!(local["items"].as_array().unwrap().len(), 1);
        let requests = disconnected.backend.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0]["target"]["mode"], "remote");
        assert_eq!(requests[1]["target"]["mode"], "local");
        assert_eq!(requests[1]["operation"], "read_current_workflow");

        let local_only_backend = HybridBackend::new(false);
        let local_only = SelectedBackendTelegramCommandRuntimeReadPort::with_backend(
            target(AgentWorkflowMode::SoloLocal, AgentRuntimeMode::Local, false),
            None,
            local_only_backend,
        );
        assert_eq!(
            local_only.read_workflow_notification().unwrap()["notification_source"],
            "local_current"
        );
        assert_eq!(local_only.backend.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn remote_workflow_modes_never_use_the_solo_local_fallback() {
        let backend = HybridBackend::new(false);
        let reader = SelectedBackendTelegramCommandRuntimeReadPort::with_backend(
            target(
                AgentWorkflowMode::SoloRemote,
                AgentRuntimeMode::Remote,
                true,
            ),
            Some(1.0),
            backend,
        );

        assert!(reader.read_workflow_notification().is_err());
        let requests = reader.backend.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["target"]["mode"], "remote");
        assert_eq!(requests[0]["operation"], "read_task_queue");
    }

    #[test]
    fn local_current_formatter_is_single_state_content_and_has_a_source_stable_digest() {
        let payload = json!({
            "notification_source": "local_current",
            "items": [{
                "task": {"task_id": "LT-9", "title": "Current local work"},
                "focus_change": {"change_id": "LC-9", "status": "draft"},
                "workflow": {"state": "in_progress", "reason": "Keep working"},
                "next_action": {"code": "continue_change"},
                "updated_at": "2026-07-19T12:00:00Z"
            }]
        });
        let formatted = agent_telegram_workflow_notification_format_json(&json!({
            "kind": "workflow_notification",
            "config": {"repo_name": "fixture"},
            "payload": payload,
        }))
        .expect("local formatter");
        let text = formatted["text"].as_str().unwrap();
        assert!(text.starts_with("workflow (fixture) · local\n\nCurrent workflow"));
        assert!(text.contains("LT-9 · Current local work"));
        assert!(text.contains("state=in_progress"));
        assert!(text.contains("change=LC-9 · status=draft"));
        assert!(!text.contains("Ready to"));
        assert!(!text.contains("… and"));

        let local_digest = agent_telegram_workflow_notification_format_json(&json!({
            "kind": "queue_digest",
            "payload": payload,
        }))
        .unwrap();
        let remote_digest = agent_telegram_workflow_notification_format_json(&json!({
            "kind": "queue_digest",
            "payload": {"notification_source": "remote_queue", "items": []},
        }))
        .unwrap();
        let legacy_remote_digest = agent_telegram_workflow_notification_format_json(&json!({
            "kind": "queue_digest",
            "payload": {"items": []},
        }))
        .unwrap();
        assert_ne!(local_digest["digest"], remote_digest["digest"]);
        assert_eq!(remote_digest["digest"], legacy_remote_digest["digest"]);
        assert_eq!(local_digest["actionable"], true);
    }
}

#[cfg(test)]
#[path = "execution/tests.rs"]
mod tests;
