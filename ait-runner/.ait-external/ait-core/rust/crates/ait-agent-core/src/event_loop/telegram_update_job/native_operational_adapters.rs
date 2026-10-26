use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{
    NativeTelegramUpdateMessagePort, TelegramUpdateOperationalPort,
    TelegramUpdateOperationalRequest, TelegramUpdatePortError,
};
use crate::event_loop::telegram_command_trigger::DefaultTelegramCommandTriggerOperationExecutor;
use crate::event_loop::telegram_event_triggers::{
    DefaultTelegramEventTriggerPlanner, NativeTelegramEventTriggerRegistryLoader,
};
use crate::event_loop::telegram_operational_trigger::{
    execute_with_telegram_operational_trigger_ports,
    DefaultTelegramOperationalTriggerCallbackPlanner,
    RuntimeBindingTelegramOperationalTriggerStatePort, TelegramOperationalTriggerDeliveryPort,
    TelegramOperationalTriggerDiagnosticsPort, TelegramOperationalTriggerExecutionConfig,
    TelegramOperationalTriggerExecutionErrorKind, TelegramOperationalTriggerPorts,
};
use crate::event_loop::telegram_reply_delivery::agent_telegram_reply_delivery_execute_json;

const OPERATIONAL_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramOperationalTriggerExecution.v1";
const OPERATIONAL_STAGE: &str = "rust_agent_telegram_operational_trigger_execution";
const REPLY_CONTRACT: &str = "ait_agent_core.event_loop.TelegramReplyDeliveryExecution.v1";
const REPLY_STAGE: &str = "rust_agent_telegram_reply_delivery_execution";
const MAX_BOT_TOKEN_BYTES: usize = 4_096;
const MAX_REPO_NAME_BYTES: usize = 512;
const MAX_REPO_ROOT_BYTES: usize = 512 * 1_024;
const MAX_STATE_PATH_BYTES: usize = 16 * 1_024;
const MAX_CHAT_ID_BYTES: usize = 512;
const MAX_CHAT_TITLE_BYTES: usize = 4_096;
const MAX_TEXT_BYTES: usize = 1_048_576;
const MAX_COMMAND_NAME_BYTES: usize = 128;
const MAX_COMMAND_ARGS_BYTES: usize = 1_048_576;
const MAX_ACTOR_BYTES: usize = 16 * 1_024;
const MAX_CONTEXT_BYTES: usize = 2 * 1_048_576;
const MAX_ATTACHMENTS: usize = 32;
const MAX_MESSAGE_IDS: usize = 1_024;
const MAX_OPERATION_COUNT: usize = 128;

pub trait TelegramUpdateAssistantReplyExecutor: Send + Sync + 'static {
    fn execute_assistant_reply(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramUpdateAssistantReplyExecutor;

impl TelegramUpdateAssistantReplyExecutor for DefaultTelegramUpdateAssistantReplyExecutor {
    fn execute_assistant_reply(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_reply_delivery_execute_json(request)
            .map_err(|_| operational_delivery_error())
    }
}

pub trait TelegramUpdateOperationalMessagePort: Send + Sync + 'static {
    fn send_operational_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String>;
}

impl<E> TelegramUpdateOperationalMessagePort for NativeTelegramUpdateMessagePort<E>
where
    E: super::TelegramUpdateMessageExecutor,
{
    fn send_operational_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String> {
        use crate::event_loop::telegram_command_runtime::TelegramCommandRuntimeDeliveryPort;

        self.send_message(chat_id, text)
            .map_err(|_| operational_delivery_error())
    }
}

pub struct NativeTelegramOperationalTriggerDeliveryPort<
    R = DefaultTelegramUpdateAssistantReplyExecutor,
    M = NativeTelegramUpdateMessagePort,
> {
    bot_token: String,
    request_timeout_seconds: Option<f64>,
    reply_markdown_enabled: bool,
    reply: R,
    message: M,
}

impl
    NativeTelegramOperationalTriggerDeliveryPort<
        DefaultTelegramUpdateAssistantReplyExecutor,
        NativeTelegramUpdateMessagePort,
    >
{
    pub fn new(
        bot_token: impl Into<String>,
        request_timeout_seconds: Option<f64>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        let bot_token = bot_token.into();
        let message = NativeTelegramUpdateMessagePort::new(
            bot_token.clone(),
            request_timeout_seconds,
            reply_markdown_enabled,
        )
        .map_err(|_| operational_configuration_error())?;
        Self::with_ports(
            bot_token,
            request_timeout_seconds,
            reply_markdown_enabled,
            DefaultTelegramUpdateAssistantReplyExecutor,
            message,
        )
    }
}

impl<R, M> NativeTelegramOperationalTriggerDeliveryPort<R, M> {
    pub fn with_ports(
        bot_token: impl Into<String>,
        request_timeout_seconds: Option<f64>,
        reply_markdown_enabled: bool,
        reply: R,
        message: M,
    ) -> Result<Self, String> {
        let bot_token = bot_token.into();
        if !valid_config_text(&bot_token, MAX_BOT_TOKEN_BYTES)
            || request_timeout_seconds
                .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 86_400.0)
        {
            return Err(operational_configuration_error());
        }
        Ok(Self {
            bot_token,
            request_timeout_seconds,
            reply_markdown_enabled,
            reply,
            message,
        })
    }
}

impl<R, M> fmt::Debug for NativeTelegramOperationalTriggerDeliveryPort<R, M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramOperationalTriggerDeliveryPort")
            .field(
                "request_timeout_configured",
                &self.request_timeout_seconds.is_some(),
            )
            .field("reply_markdown_enabled", &self.reply_markdown_enabled)
            .field("native_reply_delivery", &true)
            .field("native_failure_message", &true)
            .field("bot_token_exposed", &false)
            .finish()
    }
}

impl<R, M> TelegramOperationalTriggerDeliveryPort
    for NativeTelegramOperationalTriggerDeliveryPort<R, M>
where
    R: TelegramUpdateAssistantReplyExecutor,
    M: TelegramUpdateOperationalMessagePort,
{
    fn send_assistant_event_reply(
        &self,
        chat_id: &JsonValue,
        assistant_event: &JsonValue,
    ) -> Result<(), String> {
        if !valid_chat_id(chat_id)
            || !assistant_event.is_object()
            || assistant_event.to_string().len() > MAX_CONTEXT_BYTES
        {
            return Err(operational_delivery_error());
        }
        let outcome = self
            .reply
            .execute_assistant_reply(&json!({
                "chat_id": chat_id,
                "assistant_event": assistant_event,
                "bot_token": self.bot_token,
                "request_timeout_seconds": self.request_timeout_seconds,
                "reply_markdown_enabled": self.reply_markdown_enabled,
                "should_execute": true,
            }))
            .map_err(|_| operational_delivery_error())?;
        validate_reply_outcome(&outcome).map_err(|_| operational_delivery_error())
    }

    fn send_failure_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String> {
        self.message
            .send_operational_message(chat_id, text)
            .map_err(|_| operational_delivery_error())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTelegramOperationalTriggerDiagnosticsPort;

impl TelegramOperationalTriggerDiagnosticsPort for SystemTelegramOperationalTriggerDiagnosticsPort {
    fn record_failure(
        &self,
        kind: TelegramOperationalTriggerExecutionErrorKind,
    ) -> Result<(), String> {
        writeln!(
            std::io::stderr().lock(),
            "{{\"schema\":\"ait.agent.telegram_operational_trigger.diagnostic.v1\",\"level\":\"error\",\"code\":\"{}\",\"message\":\"Telegram operational trigger execution failed.\",\"python_fallback_allowed\":false}}",
            kind.code(),
        )
        .map_err(|_| operational_diagnostics_error())
    }
}

pub trait TelegramUpdateOperationalExecutor: Send + Sync + 'static {
    fn execute_operational_trigger(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub struct NativeTelegramUpdateOperationalExecutor {
    config: TelegramOperationalTriggerExecutionConfig,
    state: RuntimeBindingTelegramOperationalTriggerStatePort,
    delivery: NativeTelegramOperationalTriggerDeliveryPort,
}

impl NativeTelegramUpdateOperationalExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_name: impl Into<String>,
        repo_root: impl Into<PathBuf>,
        state_path: impl Into<PathBuf>,
        event_trigger_registry: JsonValue,
        bot_token: impl Into<String>,
        request_timeout_seconds: Option<f64>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        let repo_name = repo_name.into();
        let repo_root = repo_root.into();
        let state_path = state_path.into();
        validate_native_configuration(&repo_name, &repo_root, &state_path)?;
        if !event_trigger_registry.is_object() {
            return Err(operational_configuration_error());
        }
        Ok(Self {
            config: TelegramOperationalTriggerExecutionConfig::new(
                repo_name,
                repo_root,
                event_trigger_registry,
            ),
            state: RuntimeBindingTelegramOperationalTriggerStatePort::new(state_path),
            delivery: NativeTelegramOperationalTriggerDeliveryPort::new(
                bot_token,
                request_timeout_seconds,
                reply_markdown_enabled,
            )?,
        })
    }
}

impl fmt::Debug for NativeTelegramUpdateOperationalExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateOperationalExecutor")
            .field("native_event_planner", &true)
            .field("native_callback_planner", &true)
            .field("locked_binding_state", &true)
            .field("native_handler_operation", &true)
            .field("secret_safe_diagnostics", &true)
            .field("native_reply_delivery", &true)
            .field("configuration_exposed", &false)
            .field("state_path_exposed", &false)
            .field("trigger_registry_exposed", &false)
            .finish()
    }
}

impl TelegramUpdateOperationalExecutor for NativeTelegramUpdateOperationalExecutor {
    fn execute_operational_trigger(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let ports = TelegramOperationalTriggerPorts::new(
            &DefaultTelegramEventTriggerPlanner,
            &DefaultTelegramOperationalTriggerCallbackPlanner,
            &self.state,
            &DefaultTelegramCommandTriggerOperationExecutor,
            &SystemTelegramOperationalTriggerDiagnosticsPort,
            &self.delivery,
        );
        execute_with_telegram_operational_trigger_ports(&ports, &self.config, request)
            .map_err(|_| operational_execution_error())
    }
}

pub struct NativeTelegramUpdateOperationalPort<E = NativeTelegramUpdateOperationalExecutor> {
    executor: E,
}

impl NativeTelegramUpdateOperationalPort<NativeTelegramUpdateOperationalExecutor> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_name: impl Into<String>,
        repo_root: impl Into<PathBuf>,
        state_path: impl Into<PathBuf>,
        bot_token: impl Into<String>,
        request_timeout_seconds: Option<f64>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        let repo_root = repo_root.into();
        let registry = NativeTelegramEventTriggerRegistryLoader::new()
            .load(&repo_root)
            .map_err(|_| operational_configuration_error())?;
        Ok(Self::with_executor(
            NativeTelegramUpdateOperationalExecutor::new(
                repo_name,
                repo_root,
                state_path,
                registry,
                bot_token,
                request_timeout_seconds,
                reply_markdown_enabled,
            )?,
        ))
    }
}

impl<E> NativeTelegramUpdateOperationalPort<E> {
    pub fn with_executor(executor: E) -> Self {
        Self { executor }
    }
}

impl<E> fmt::Debug for NativeTelegramUpdateOperationalPort<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateOperationalPort")
            .field("native_operational_runtime", &true)
            .field("request_payload_exposed", &false)
            .field("executor_output_exposed", &false)
            .finish()
    }
}

impl<E> TelegramUpdateOperationalPort for NativeTelegramUpdateOperationalPort<E>
where
    E: TelegramUpdateOperationalExecutor,
{
    fn handle_operational_trigger(
        &self,
        request: &TelegramUpdateOperationalRequest,
    ) -> Result<bool, TelegramUpdatePortError> {
        validate_operational_request(request).map_err(|_| TelegramUpdatePortError)?;
        let command = request
            .command()
            .map(|(name, args)| json!([name, args]))
            .unwrap_or(JsonValue::Null);
        let outcome = self
            .executor
            .execute_operational_trigger(&json!({
                "chat_id": request.chat_id(),
                "chat": request.chat(),
                "from_user": request.from_user(),
                "chat_title": request.chat_title(),
                "context": {
                    "raw_text": request.raw_text(),
                    "normalized_text": request.normalized_text(),
                    "command": command,
                    "telegram_message_id": request.telegram_message_id(),
                    "telegram_message_ids": request.telegram_message_ids(),
                    "reply_to_message": request.reply_to_message(),
                    "attachments": request.attachments(),
                    "actor_identity": request.actor_identity(),
                    "message": request.message(),
                },
            }))
            .map_err(|_| TelegramUpdatePortError)?;
        validate_operational_outcome(&outcome).map_err(|_| TelegramUpdatePortError)
    }
}

fn validate_operational_request(request: &TelegramUpdateOperationalRequest) -> Result<(), ()> {
    if !valid_chat_id(request.chat_id())
        || !request.chat().is_object()
        || !request.from_user().is_object()
        || !request.message().is_object()
        || !valid_bounded_text(request.chat_title(), MAX_CHAT_TITLE_BYTES, true)
        || !valid_bounded_text(request.raw_text(), MAX_TEXT_BYTES, true)
        || !valid_bounded_text(request.normalized_text(), MAX_TEXT_BYTES, true)
        || request.command().is_some_and(|(name, args)| {
            !valid_bounded_text(name, MAX_COMMAND_NAME_BYTES, false)
                || !valid_bounded_text(args, MAX_COMMAND_ARGS_BYTES, true)
        })
        || request
            .telegram_message_id()
            .is_some_and(|value| value <= 0)
        || request.telegram_message_ids().len() > MAX_MESSAGE_IDS
        || request
            .telegram_message_ids()
            .iter()
            .any(|value| *value <= 0)
        || request
            .reply_to_message()
            .is_some_and(|value| !value.is_object())
        || request.attachments().len() > MAX_ATTACHMENTS
        || request.attachments().iter().any(|value| !value.is_object())
        || request
            .actor_identity()
            .is_some_and(|value| !valid_bounded_text(value, MAX_ACTOR_BYTES, false))
        || request.chat().to_string().len() > MAX_CONTEXT_BYTES
        || request.from_user().to_string().len() > MAX_CONTEXT_BYTES
        || request.message().to_string().len() > MAX_CONTEXT_BYTES
        || request
            .reply_to_message()
            .is_some_and(|value| value.to_string().len() > MAX_CONTEXT_BYTES)
        || JsonValue::Array(request.attachments().to_vec())
            .to_string()
            .len()
            > MAX_CONTEXT_BYTES
    {
        return Err(());
    }
    Ok(())
}

fn validate_operational_outcome(outcome: &JsonValue) -> Result<bool, ()> {
    let object = outcome.as_object().ok_or(())?;
    let ok = required_bool(object, "ok")?;
    let matched = required_bool(object, "matched")?;
    let handled = required_bool(object, "handled")?;
    let result_callback_planned = required_bool(object, "result_callback_planned")?;
    let assistant_event_sent = required_bool(object, "assistant_event_sent")?;
    let failure_message_sent = required_bool(object, "failure_message_sent")?;
    let operation_count = bounded_count(object.get("operation_count"), MAX_OPERATION_COUNT)?;
    let completed_operation_count =
        bounded_count(object.get("completed_operation_count"), MAX_OPERATION_COUNT)?;
    let failure_kind = object.get("failure_kind").ok_or(())?;
    let failure_kind_valid = failure_kind.as_str().is_some_and(|value| {
        matches!(
            value,
            "invalid_request"
                | "event_planner"
                | "event_planner_contract"
                | "state"
                | "callback_planner"
                | "callback_planner_contract"
                | "operation"
                | "diagnostics"
                | "delivery"
        )
    });
    if text(object, "contract") != Some(OPERATIONAL_CONTRACT)
        || text(object, "migration_stage") != Some(OPERATIONAL_STAGE)
        || text(object, "stage") != Some("execute")
        || text(object, "transport") != Some("telegram")
        || !required_bool(object, "completed")?
        || required_bool(object, "python_executor_allowed")?
        || completed_operation_count > operation_count
        || assistant_event_sent && (!ok || !handled)
        || (!matched
            && (!ok
                || handled
                || operation_count != 0
                || completed_operation_count != 0
                || result_callback_planned
                || assistant_event_sent
                || failure_message_sent
                || !failure_kind.is_null()))
        || (matched
            && ok
            && (completed_operation_count != operation_count
                || !result_callback_planned
                || failure_message_sent
                || !failure_kind.is_null()))
        || (matched
            && !ok
            && (!handled || assistant_event_sent || !failure_message_sent || !failure_kind_valid))
    {
        return Err(());
    }
    Ok(handled)
}

fn validate_reply_outcome(outcome: &JsonValue) -> Result<(), ()> {
    let object = outcome.as_object().ok_or(())?;
    let operation_count = bounded_count(object.get("operation_count"), MAX_OPERATION_COUNT)?;
    if text(object, "contract") != Some(REPLY_CONTRACT)
        || text(object, "migration_stage") != Some(REPLY_STAGE)
        || text(object, "stage") != Some("execute")
        || text(object, "transport") != Some("telegram")
        || text(object, "reply_delivery_state") != Some("completed")
        || !required_bool(object, "ok")?
        || !required_bool(object, "completed")?
        || !required_bool(object, "delivered")?
        || operation_count == 0
        || bounded_count(object.get("attempted_operation_count"), MAX_OPERATION_COUNT)?
            != operation_count
        || bounded_count(object.get("delivered_operation_count"), MAX_OPERATION_COUNT)?
            != operation_count
        || bounded_count(object.get("failed_operation_count"), MAX_OPERATION_COUNT)? != 0
        || !object
            .get("failed_operation_index")
            .is_some_and(JsonValue::is_null)
        || !object
            .get("failed_operation_kind")
            .is_some_and(JsonValue::is_null)
        || !object.get("error_kind").is_some_and(JsonValue::is_null)
        || !object.get("error").is_some_and(JsonValue::is_null)
        || !all_false(
            object,
            &[
                "python_reply_delivery_allowed",
                "python_message_delivery_allowed",
                "python_attachment_delivery_allowed",
                "raw_planner_result_exposed",
                "raw_executor_result_exposed",
                "bot_token_exposed",
                "chat_id_exposed",
                "reply_text_exposed",
                "attachment_exposed",
                "telegram_description_exposed",
                "local_path_exposed",
            ],
        )
    {
        return Err(());
    }
    Ok(())
}

fn validate_native_configuration(
    repo_name: &str,
    repo_root: &Path,
    state_path: &Path,
) -> Result<(), String> {
    let root = repo_root.to_string_lossy();
    let state = state_path.to_string_lossy();
    if !valid_config_text(repo_name, MAX_REPO_NAME_BYTES)
        || repo_root.as_os_str().is_empty()
        || root.len() > MAX_REPO_ROOT_BYTES
        || root
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
        || state_path.as_os_str().is_empty()
        || state.len() > MAX_STATE_PATH_BYTES
        || state
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
    {
        return Err(operational_configuration_error());
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

fn valid_config_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

fn valid_bounded_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= max_bytes
        && !value.contains('\0')
        && !value.contains('\r')
}

fn required_bool(object: &Map<String, JsonValue>, key: &str) -> Result<bool, ()> {
    object.get(key).and_then(JsonValue::as_bool).ok_or(())
}

fn bounded_count(value: Option<&JsonValue>, max: usize) -> Result<usize, ()> {
    value
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value <= max)
        .ok_or(())
}

fn text<'a>(object: &'a Map<String, JsonValue>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(JsonValue::as_str)
}

fn all_false(object: &Map<String, JsonValue>, keys: &[&str]) -> bool {
    keys.iter()
        .all(|key| object.get(*key).and_then(JsonValue::as_bool) == Some(false))
}

fn operational_configuration_error() -> String {
    "Telegram update operational configuration is invalid.".to_string()
}

fn operational_execution_error() -> String {
    "Telegram update operational execution failed.".to_string()
}

fn operational_delivery_error() -> String {
    "Telegram update operational delivery failed.".to_string()
}

fn operational_diagnostics_error() -> String {
    "Telegram update operational diagnostics failed.".to_string()
}

#[cfg(test)]
mod tests;
