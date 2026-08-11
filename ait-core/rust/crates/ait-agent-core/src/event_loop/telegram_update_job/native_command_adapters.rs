use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ait_core::json_support::{json, JsonValue};

use super::{
    NativeTelegramUpdateMessagePort, TelegramUpdateCommandPort, TelegramUpdateCommandRequest,
    TelegramUpdatePortError,
};
use crate::event_loop::telegram_command_runtime::{
    execute_with_telegram_command_runtime_ports, DefaultTelegramCommandRuntimePlanner,
    RuntimeBindingTelegramCommandRuntimeStatePort, SelectedBackendTelegramCommandRuntimeReadPort,
    SystemTelegramCommandRuntimeClockPort, TelegramCommandRuntimeDeliveryPort,
};
use crate::event_loop::telegram_workflow_query::DefaultTelegramWorkflowQueryPlanner;
use crate::transport_config::{AgentRuntimeMode, AgentRuntimeTarget, AgentWorkflowMode};

const COMMAND_CONTRACT: &str = "ait_agent_core.event_loop.TelegramCommandRuntimeExecution.v1";
const COMMAND_MIGRATION_STAGE: &str = "rust_agent_telegram_command_runtime_execution";
const MAX_CHAT_ID_BYTES: usize = 512;
const MAX_CHAT_TITLE_BYTES: usize = 4_096;
const MAX_COMMAND_NAME_BYTES: usize = 128;
const MAX_COMMAND_ARGS_BYTES: usize = 1_048_576;
const MAX_CONTEXT_BYTES: usize = 2 * 1_048_576;
const MAX_CONFIG_TEXT_BYTES: usize = 512 * 1_024;
const MAX_STATE_PATH_BYTES: usize = 16 * 1_024;
const MAX_DEPENDENCY_ROUNDS: usize = 4;

pub trait TelegramUpdateCommandExecutor: Send + Sync + 'static {
    fn execute_command_runtime(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub struct NativeTelegramUpdateCommandRuntimeExecutor<D = NativeTelegramUpdateMessagePort> {
    reads: SelectedBackendTelegramCommandRuntimeReadPort,
    state: RuntimeBindingTelegramCommandRuntimeStatePort,
    delivery: Arc<D>,
    config: JsonValue,
}

impl NativeTelegramUpdateCommandRuntimeExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_path: impl Into<PathBuf>,
        runtime_target: AgentRuntimeTarget,
        request_timeout_seconds: Option<f64>,
        bot_token: impl Into<String>,
        ait_web_url: Option<String>,
        username: impl Into<String>,
        background_sync_enabled: bool,
        background_sync_interval_seconds: f64,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        let state_path = state_path.into();
        let bot_token = bot_token.into();
        let delivery = Arc::new(
            NativeTelegramUpdateMessagePort::new(
                bot_token.clone(),
                request_timeout_seconds,
                reply_markdown_enabled,
            )
            .map_err(|_| command_configuration_error())?,
        );
        Self::with_delivery(
            state_path,
            runtime_target,
            request_timeout_seconds,
            ait_web_url,
            username,
            background_sync_enabled,
            background_sync_interval_seconds,
            delivery,
        )
    }
}

impl<D> NativeTelegramUpdateCommandRuntimeExecutor<D> {
    #[allow(clippy::too_many_arguments)]
    pub fn with_delivery(
        state_path: impl Into<PathBuf>,
        runtime_target: AgentRuntimeTarget,
        request_timeout_seconds: Option<f64>,
        ait_web_url: Option<String>,
        username: impl Into<String>,
        background_sync_enabled: bool,
        background_sync_interval_seconds: f64,
        delivery: Arc<D>,
    ) -> Result<Self, String> {
        let state_path = state_path.into();
        validate_native_configuration(&state_path, &runtime_target, request_timeout_seconds)?;
        let username = username.into();
        if (!username.is_empty() && !valid_config_text(&username, MAX_CONFIG_TEXT_BYTES))
            || ait_web_url
                .as_deref()
                .is_some_and(|value| !valid_config_text(value, MAX_CONFIG_TEXT_BYTES))
            || !background_sync_interval_seconds.is_finite()
            || background_sync_interval_seconds < 0.0
        {
            return Err(command_configuration_error());
        }
        let username = (!username.is_empty()).then_some(username);
        let config = json!({
            "repo_name": runtime_target.repo_name,
            "ait_web_url": ait_web_url,
            "background_sync_enabled": background_sync_enabled,
            "background_sync_interval_seconds": background_sync_interval_seconds,
            "runtime_mode": runtime_target.mode.as_str(),
            "runtime_remote_name": runtime_target.remote_name,
            "username": username,
        });
        Ok(Self {
            reads: SelectedBackendTelegramCommandRuntimeReadPort::new(
                runtime_target,
                request_timeout_seconds,
            ),
            state: RuntimeBindingTelegramCommandRuntimeStatePort::new(state_path),
            delivery,
            config,
        })
    }
}

impl<D> fmt::Debug for NativeTelegramUpdateCommandRuntimeExecutor<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateCommandRuntimeExecutor")
            .field("native_command_planner", &true)
            .field("native_runtime_reads", &true)
            .field("locked_binding_state", &true)
            .field("configuration_exposed", &false)
            .field("runtime_target_exposed", &false)
            .field("state_path_exposed", &false)
            .field("message_configuration_exposed", &false)
            .finish()
    }
}

impl<D> TelegramUpdateCommandExecutor for NativeTelegramUpdateCommandRuntimeExecutor<D>
where
    D: TelegramCommandRuntimeDeliveryPort,
{
    fn execute_command_runtime(&self, request: &JsonValue) -> Result<JsonValue, String> {
        execute_with_telegram_command_runtime_ports(
            &DefaultTelegramCommandRuntimePlanner,
            &DefaultTelegramWorkflowQueryPlanner,
            &self.reads,
            &self.state,
            &SystemTelegramCommandRuntimeClockPort,
            self.delivery.as_ref(),
            &self.config,
            request,
        )
        .map_err(|_| command_execution_error())
    }
}

pub struct NativeTelegramUpdateCommandPort<E = NativeTelegramUpdateCommandRuntimeExecutor> {
    executor: E,
}

impl NativeTelegramUpdateCommandPort {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_path: impl Into<PathBuf>,
        runtime_target: AgentRuntimeTarget,
        request_timeout_seconds: Option<f64>,
        bot_token: impl Into<String>,
        ait_web_url: Option<String>,
        username: impl Into<String>,
        background_sync_enabled: bool,
        background_sync_interval_seconds: f64,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        Ok(Self::with_executor(
            NativeTelegramUpdateCommandRuntimeExecutor::new(
                state_path,
                runtime_target,
                request_timeout_seconds,
                bot_token,
                ait_web_url,
                username,
                background_sync_enabled,
                background_sync_interval_seconds,
                reply_markdown_enabled,
            )?,
        ))
    }
}

impl<E> NativeTelegramUpdateCommandPort<E> {
    pub fn with_executor(executor: E) -> Self {
        Self { executor }
    }
}

impl<E> fmt::Debug for NativeTelegramUpdateCommandPort<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateCommandPort")
            .field("native_command_runtime", &true)
            .field("configuration_exposed", &false)
            .field("request_payload_exposed", &false)
            .field("executor_output_exposed", &false)
            .finish()
    }
}

impl<E> TelegramUpdateCommandPort for NativeTelegramUpdateCommandPort<E>
where
    E: TelegramUpdateCommandExecutor,
{
    fn execute_command(
        &self,
        request: &TelegramUpdateCommandRequest,
    ) -> Result<(), TelegramUpdatePortError> {
        validate_command_request(request).map_err(|_| TelegramUpdatePortError)?;
        let outcome = self
            .executor
            .execute_command_runtime(&json!({
                "chat_id": request.chat_id(),
                "chat": request.chat(),
                "from_user": request.from_user(),
                "chat_title": request.chat_title(),
                "name": request.command_name(),
                "args": request.command_args(),
            }))
            .map_err(|_| TelegramUpdatePortError)?;
        validate_command_outcome(&outcome).map_err(|_| TelegramUpdatePortError)
    }
}

fn validate_command_request(request: &TelegramUpdateCommandRequest) -> Result<(), ()> {
    if !valid_chat_id(request.chat_id())
        || !request.chat().is_object()
        || !request.from_user().is_object()
        || request.chat().to_string().len() > MAX_CONTEXT_BYTES
        || request.from_user().to_string().len() > MAX_CONTEXT_BYTES
        || !valid_bounded_text(request.chat_title(), MAX_CHAT_TITLE_BYTES, true)
        || !valid_bounded_text(request.command_name(), MAX_COMMAND_NAME_BYTES, false)
        || !valid_bounded_text(request.command_args(), MAX_COMMAND_ARGS_BYTES, true)
    {
        return Err(());
    }
    Ok(())
}

fn validate_command_outcome(outcome: &JsonValue) -> Result<(), ()> {
    let object = outcome.as_object().ok_or(())?;
    let decision = object
        .get("decision")
        .and_then(JsonValue::as_str)
        .filter(|value| {
            matches!(
                *value,
                "message_delivered" | "stage_completed" | "handler_invoked"
            )
        })
        .ok_or(())?;
    let dependency_rounds =
        bounded_count_unit(object.get("dependency_rounds"), 0, MAX_DEPENDENCY_ROUNDS)?;
    let queue_read = bool_field_unit(object, "queue_read")?;
    let detail_read = bool_field_unit(object, "detail_read")?;
    let binding_loaded = bool_field_unit(object, "binding_loaded")?;
    let state_patched = bool_field_unit(object, "state_patched")?;
    let message_sent = bool_field_unit(object, "message_sent")?;
    let handler_invoked = bool_field_unit(object, "handler_invoked")?;
    let side_effect_count = bounded_count_unit(object.get("side_effect_count"), 0, 4)?;
    let expected_side_effects =
        usize::from(state_patched) + usize::from(message_sent) + usize::from(handler_invoked);
    if text(object, "contract") != Some(COMMAND_CONTRACT)
        || text(object, "migration_stage") != Some(COMMAND_MIGRATION_STAGE)
        || text(object, "stage") != Some("execute")
        || text(object, "transport") != Some("telegram")
        || text(object, "command_runtime_state") != Some("completed")
        || !bool_field_unit(object, "ok")?
        || !bool_field_unit(object, "completed")?
        || side_effect_count != expected_side_effects
        || dependency_rounds < usize::from(queue_read) + usize::from(detail_read)
        || (decision == "message_delivered"
            && (!message_sent
                || dependency_rounds != 0
                || queue_read
                || detail_read
                || binding_loaded
                || state_patched
                || handler_invoked))
        || (decision == "handler_invoked" && (!handler_invoked || message_sent || state_patched))
        || (decision == "stage_completed" && (!message_sent || handler_invoked))
        || !bool_field_unit(object, "rust_command_execution_required")?
        || !all_false_unit(
            object,
            &[
                "python_command_runtime_allowed",
                "python_runtime_read_allowed",
                "python_state_mutation_allowed",
                "python_message_delivery_allowed",
                "request_payload_exposed",
                "config_exposed",
                "planner_payload_exposed",
                "dependency_payload_exposed",
                "chat_id_exposed",
                "target_ref_exposed",
                "message_text_exposed",
                "state_patch_exposed",
            ],
        )
    {
        return Err(());
    }
    Ok(())
}

fn validate_native_configuration(
    state_path: &Path,
    target: &AgentRuntimeTarget,
    request_timeout_seconds: Option<f64>,
) -> Result<(), String> {
    let path = state_path.to_string_lossy();
    if state_path.as_os_str().is_empty()
        || path.len() > MAX_STATE_PATH_BYTES
        || path
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
        || !valid_config_text(&target.repo_name, MAX_CONFIG_TEXT_BYTES)
        || target.repo_root.as_os_str().is_empty()
        || target.repo_root.to_string_lossy().len() > MAX_CONFIG_TEXT_BYTES
        || target
            .repo_root
            .to_string_lossy()
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
        || request_timeout_seconds
            .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 86_400.0)
        || match target.mode {
            AgentRuntimeMode::Local => {
                target.workflow_mode != AgentWorkflowMode::SoloLocal
                    || target.remote_name.is_some()
                    || target.server_url.is_some()
            }
            AgentRuntimeMode::Remote => {
                !matches!(
                    target.workflow_mode,
                    AgentWorkflowMode::SoloRemote | AgentWorkflowMode::TeamRemote
                ) || target
                    .remote_name
                    .as_deref()
                    .is_none_or(|value| !valid_config_text(value, MAX_CONFIG_TEXT_BYTES))
                    || target
                        .server_url
                        .as_deref()
                        .is_none_or(|value| !valid_config_text(value, MAX_CONFIG_TEXT_BYTES))
            }
        }
    {
        return Err(command_configuration_error());
    }
    Ok(())
}

fn valid_chat_id(value: &JsonValue) -> bool {
    match value {
        JsonValue::Number(number) => number.as_i64().is_some(),
        JsonValue::String(value) => {
            valid_bounded_text(value, MAX_CHAT_ID_BYTES, false)
                && !value.chars().any(char::is_control)
        }
        _ => false,
    }
}

fn valid_bounded_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= max_bytes
        && !value.contains('\0')
        && !value.contains('\r')
}

fn valid_config_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= max_bytes
        && !value
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
}

fn text<'a>(
    object: &'a ait_core::json_support::JsonMap<String, JsonValue>,
    key: &str,
) -> Option<&'a str> {
    object.get(key).and_then(JsonValue::as_str)
}

fn bool_field_unit(
    object: &ait_core::json_support::JsonMap<String, JsonValue>,
    key: &str,
) -> Result<bool, ()> {
    object.get(key).and_then(JsonValue::as_bool).ok_or(())
}

fn bounded_count_unit(value: Option<&JsonValue>, min: usize, max: usize) -> Result<usize, ()> {
    value
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| (min..=max).contains(value))
        .ok_or(())
}

fn all_false(object: &ait_core::json_support::JsonMap<String, JsonValue>, keys: &[&str]) -> bool {
    keys.iter()
        .all(|key| object.get(*key).and_then(JsonValue::as_bool) == Some(false))
}

fn all_false_unit(
    object: &ait_core::json_support::JsonMap<String, JsonValue>,
    keys: &[&str],
) -> bool {
    all_false(object, keys)
}

fn command_configuration_error() -> String {
    "Telegram update command configuration is invalid.".to_string()
}

fn command_execution_error() -> String {
    "Telegram update command execution failed.".to_string()
}
