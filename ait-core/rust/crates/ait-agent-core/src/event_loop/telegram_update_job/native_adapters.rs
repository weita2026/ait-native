use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ait_core::json_support::{json, JsonValue};

use super::{
    TelegramUpdateBootstrapPort, TelegramUpdateBootstrapRequest, TelegramUpdateDeliveryPort,
    TelegramUpdateDiagnosticsPort, TelegramUpdateJobErrorKind, TelegramUpdatePortError,
};
use crate::event_loop::telegram_command_runtime::TelegramCommandRuntimeDeliveryPort;
use crate::event_loop::telegram_message_formatting::agent_telegram_message_delivery_execute_json;
use crate::event_loop::telegram_owner_bootstrap::{
    execute_with_telegram_owner_bootstrap_ports, DefaultTelegramOwnerBootstrapPlanner,
    RuntimeBindingTelegramOwnerBootstrapStatePort, SystemTelegramOwnerBootstrapClockPort,
    TelegramOwnerBootstrapClockPort, TelegramOwnerBootstrapMessagePort,
    TelegramOwnerBootstrapPlanner, TelegramOwnerBootstrapStatePort,
};

const MESSAGE_CONTRACT: &str = "ait_agent_core.event_loop.TelegramMessageDeliveryExecution.v1";
const MESSAGE_MIGRATION_STAGE: &str = "rust_agent_telegram_message_delivery_execution";
const BOOTSTRAP_CONTRACT: &str = "ait_agent_core.event_loop.TelegramOwnerBootstrapExecution.v1";
const BOOTSTRAP_MIGRATION_STAGE: &str = "rust_agent_telegram_owner_bootstrap_execution";
const MAX_BOT_TOKEN_BYTES: usize = 4_096;
const MAX_CHAT_ID_BYTES: usize = 128;
const MAX_MESSAGE_CHARS: usize = 3_800 * 128;
const MAX_REPO_NAME_BYTES: usize = 512;
const MAX_CHAT_TITLE_BYTES: usize = 4_096;
const MAX_RAW_TEXT_BYTES: usize = 1_048_576;
const MAX_COMMAND_NAME_BYTES: usize = 128;
const MAX_COMMAND_ARGS_BYTES: usize = 1_048_576;
const MAX_CONTEXT_BYTES: usize = 2 * 1_048_576;
const MAX_STATE_PATH_BYTES: usize = 16 * 1_024;
const MAX_MESSAGE_CHUNKS: usize = 128;

pub trait TelegramUpdateMessageExecutor: Send + Sync + 'static {
    fn execute_message(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramUpdateMessageExecutor;

impl TelegramUpdateMessageExecutor for DefaultTelegramUpdateMessageExecutor {
    fn execute_message(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_message_delivery_execute_json(request)
    }
}

pub struct NativeTelegramUpdateMessagePort<E = DefaultTelegramUpdateMessageExecutor> {
    bot_token: String,
    request_timeout_seconds: Option<f64>,
    reply_markdown_enabled: bool,
    executor: E,
}

impl NativeTelegramUpdateMessagePort<DefaultTelegramUpdateMessageExecutor> {
    pub fn new(
        bot_token: impl Into<String>,
        request_timeout_seconds: Option<f64>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        Self::with_executor(
            bot_token,
            request_timeout_seconds,
            reply_markdown_enabled,
            DefaultTelegramUpdateMessageExecutor,
        )
    }
}

impl<E> NativeTelegramUpdateMessagePort<E> {
    pub fn with_executor(
        bot_token: impl Into<String>,
        request_timeout_seconds: Option<f64>,
        reply_markdown_enabled: bool,
        executor: E,
    ) -> Result<Self, String> {
        let bot_token = bot_token.into();
        if !valid_bounded_text(&bot_token, MAX_BOT_TOKEN_BYTES)
            || request_timeout_seconds.is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            return Err(message_configuration_error());
        }
        Ok(Self {
            bot_token,
            request_timeout_seconds,
            reply_markdown_enabled,
            executor,
        })
    }
}

impl<E> NativeTelegramUpdateMessagePort<E>
where
    E: TelegramUpdateMessageExecutor,
{
    fn send(&self, chat_id: &JsonValue, text: &str) -> Result<(), String> {
        if !valid_chat_id(chat_id)
            || text.is_empty()
            || text.chars().count() > MAX_MESSAGE_CHARS
            || text.contains('\0')
        {
            return Err(message_execution_error());
        }
        let outcome = self
            .executor
            .execute_message(&json!({
                "chat_id": chat_id,
                "text": text,
                "bot_token": self.bot_token,
                "request_timeout_seconds": self.request_timeout_seconds,
                "reply_markdown_enabled": self.reply_markdown_enabled,
            }))
            .map_err(|_| message_execution_error())?;
        validate_message_outcome(&outcome).map_err(|_| message_execution_error())
    }
}

impl<E> fmt::Debug for NativeTelegramUpdateMessagePort<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateMessagePort")
            .field(
                "request_timeout_configured",
                &self.request_timeout_seconds.is_some(),
            )
            .field("reply_markdown_enabled", &self.reply_markdown_enabled)
            .field("bot_token_exposed", &false)
            .field("executor_output_exposed", &false)
            .finish()
    }
}

impl<E> TelegramOwnerBootstrapMessagePort for NativeTelegramUpdateMessagePort<E>
where
    E: TelegramUpdateMessageExecutor,
{
    fn send_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String> {
        self.send(chat_id, text)
    }
}

impl<E> TelegramCommandRuntimeDeliveryPort for NativeTelegramUpdateMessagePort<E>
where
    E: TelegramUpdateMessageExecutor,
{
    fn send_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String> {
        self.send(chat_id, text)
    }
}

impl<E> TelegramUpdateDeliveryPort for NativeTelegramUpdateMessagePort<E>
where
    E: TelegramUpdateMessageExecutor,
{
    fn send_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), TelegramUpdatePortError> {
        self.send(chat_id, text)
            .map_err(|_| TelegramUpdatePortError)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTelegramUpdateDiagnosticsPort;

impl TelegramUpdateDiagnosticsPort for SystemTelegramUpdateDiagnosticsPort {
    fn record_failure(
        &self,
        kind: TelegramUpdateJobErrorKind,
    ) -> Result<(), TelegramUpdatePortError> {
        writeln!(
            std::io::stderr().lock(),
            "{}",
            update_failure_diagnostic(kind)
        )
        .map_err(|_| TelegramUpdatePortError)
    }
}

fn update_failure_diagnostic(kind: TelegramUpdateJobErrorKind) -> String {
    format!(
        "{{\"schema\":\"ait.agent.telegram_update.diagnostic.v1\",\"level\":\"error\",\"code\":\"{}\",\"message\":\"Telegram update execution failed.\",\"python_fallback_allowed\":false,\"private_context_exposed\":false}}",
        kind.code()
    )
}

pub trait TelegramUpdateOwnerBootstrapExecutor: Send + Sync + 'static {
    fn execute_owner_bootstrap(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub struct NativeTelegramUpdateOwnerBootstrapExecutor<
    P = DefaultTelegramOwnerBootstrapPlanner,
    S = RuntimeBindingTelegramOwnerBootstrapStatePort,
    C = SystemTelegramOwnerBootstrapClockPort,
    M = NativeTelegramUpdateMessagePort,
> {
    planner: P,
    state: S,
    clock: C,
    message: Arc<M>,
}

impl<P, S, C, M> NativeTelegramUpdateOwnerBootstrapExecutor<P, S, C, M> {
    pub fn with_ports(planner: P, state: S, clock: C, message: Arc<M>) -> Self {
        Self {
            planner,
            state,
            clock,
            message,
        }
    }
}

impl<P, S, C, M> fmt::Debug for NativeTelegramUpdateOwnerBootstrapExecutor<P, S, C, M> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateOwnerBootstrapExecutor")
            .field("native_planner", &true)
            .field("native_state", &true)
            .field("native_clock", &true)
            .field("native_message", &true)
            .field("state_path_exposed", &false)
            .field("message_configuration_exposed", &false)
            .finish()
    }
}

impl<P, S, C, M> TelegramUpdateOwnerBootstrapExecutor
    for NativeTelegramUpdateOwnerBootstrapExecutor<P, S, C, M>
where
    P: TelegramOwnerBootstrapPlanner + Send + Sync + 'static,
    S: TelegramOwnerBootstrapStatePort,
    C: TelegramOwnerBootstrapClockPort,
    M: TelegramOwnerBootstrapMessagePort,
{
    fn execute_owner_bootstrap(&self, request: &JsonValue) -> Result<JsonValue, String> {
        execute_with_telegram_owner_bootstrap_ports(
            &self.planner,
            &self.state,
            &self.clock,
            self.message.as_ref(),
            request,
        )
        .map_err(|_| bootstrap_execution_error())
    }
}

pub struct NativeTelegramUpdateBootstrapPort<E = NativeTelegramUpdateOwnerBootstrapExecutor> {
    repo_name: String,
    owner_bootstrap_enabled: bool,
    executor: E,
}

impl NativeTelegramUpdateBootstrapPort<NativeTelegramUpdateOwnerBootstrapExecutor> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo_name: impl Into<String>,
        state_path: impl Into<PathBuf>,
        owner_bootstrap_enabled: bool,
        bot_token: impl Into<String>,
        request_timeout_seconds: Option<f64>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        let state_path = state_path.into();
        validate_state_path(&state_path)?;
        let message = Arc::new(NativeTelegramUpdateMessagePort::new(
            bot_token,
            request_timeout_seconds,
            reply_markdown_enabled,
        )?);
        Self::with_executor(
            repo_name,
            owner_bootstrap_enabled,
            NativeTelegramUpdateOwnerBootstrapExecutor::with_ports(
                DefaultTelegramOwnerBootstrapPlanner,
                RuntimeBindingTelegramOwnerBootstrapStatePort::new(state_path),
                SystemTelegramOwnerBootstrapClockPort,
                message,
            ),
        )
    }
}

impl<E> NativeTelegramUpdateBootstrapPort<E> {
    pub fn with_executor(
        repo_name: impl Into<String>,
        owner_bootstrap_enabled: bool,
        executor: E,
    ) -> Result<Self, String> {
        let repo_name = repo_name.into();
        if !valid_bounded_text(&repo_name, MAX_REPO_NAME_BYTES) {
            return Err(bootstrap_configuration_error());
        }
        Ok(Self {
            repo_name,
            owner_bootstrap_enabled,
            executor,
        })
    }
}

impl<E> fmt::Debug for NativeTelegramUpdateBootstrapPort<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateBootstrapPort")
            .field("owner_bootstrap_enabled", &self.owner_bootstrap_enabled)
            .field("repo_name_exposed", &false)
            .field("request_payload_exposed", &false)
            .field("executor_output_exposed", &false)
            .finish()
    }
}

impl<E> TelegramUpdateBootstrapPort for NativeTelegramUpdateBootstrapPort<E>
where
    E: TelegramUpdateOwnerBootstrapExecutor,
{
    fn handle_bootstrap(
        &self,
        request: &TelegramUpdateBootstrapRequest,
    ) -> Result<bool, TelegramUpdatePortError> {
        validate_bootstrap_request(request).map_err(|_| TelegramUpdatePortError)?;
        let command = request
            .command()
            .map(|(name, args)| json!([name, args]))
            .unwrap_or(JsonValue::Null);
        let command_name = request
            .command()
            .map(|(name, _)| json!(name))
            .unwrap_or(JsonValue::Null);
        let outcome = self
            .executor
            .execute_owner_bootstrap(&json!({
                "kind": "handle",
                "owner_bootstrap_enabled": self.owner_bootstrap_enabled,
                "expected_password": self.repo_name,
                "config": {
                    "repo_name": self.repo_name,
                    "owner_bootstrap_enabled": self.owner_bootstrap_enabled,
                },
                "chat_id": request.chat_id(),
                "chat": request.chat(),
                "from_user": request.from_user(),
                "chat_title": request.chat_title(),
                "raw_text": request.raw_text(),
                "command": command,
                "command_name": command_name,
                "attachments_present": request.attachments_present(),
            }))
            .map_err(|_| TelegramUpdatePortError)?;
        validate_bootstrap_outcome(&outcome).map_err(|_| TelegramUpdatePortError)
    }
}

fn validate_message_outcome(outcome: &JsonValue) -> Result<(), ()> {
    let object = outcome.as_object().ok_or(())?;
    if object.get("contract").and_then(JsonValue::as_str) != Some(MESSAGE_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(MESSAGE_MIGRATION_STAGE)
        || object.get("stage").and_then(JsonValue::as_str) != Some("execute")
        || object
            .get("message_delivery_state")
            .and_then(JsonValue::as_str)
            != Some("completed")
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || !object
            .get("failed_chunk_index")
            .is_some_and(JsonValue::is_null)
        || !object.get("error_kind").is_some_and(JsonValue::is_null)
        || !object.get("error").is_some_and(JsonValue::is_null)
        || !false_flag(object, "python_message_delivery_allowed")
        || !false_flag(object, "python_message_formatting_allowed")
        || !false_flag(object, "raw_api_result_exposed")
        || !false_flag(object, "telegram_description_exposed")
        || !false_flag(object, "token_bearing_url_exposed")
        || !false_flag(object, "chat_id_exposed")
        || !false_flag(object, "formatted_text_exposed")
        || !false_flag(object, "plain_text_exposed")
    {
        return Err(());
    }
    let chunk_count = bounded_count(object.get("chunk_count"), 1, MAX_MESSAGE_CHUNKS)?;
    if bounded_count(
        object.get("completed_chunk_count"),
        chunk_count,
        chunk_count,
    )? != chunk_count
    {
        return Err(());
    }
    let chunks = object
        .get("chunk_results")
        .and_then(JsonValue::as_array)
        .filter(|chunks| chunks.len() == chunk_count)
        .ok_or(())?;
    let mut fallback_count = 0_usize;
    let mut api_call_count = 0_usize;
    for (index, chunk) in chunks.iter().enumerate() {
        let chunk = chunk.as_object().ok_or(())?;
        let fallback = chunk
            .get("fallback_used")
            .and_then(JsonValue::as_bool)
            .ok_or(())?;
        let calls = bounded_count(chunk.get("api_call_count"), 1, 2)?;
        if chunk.get("index").and_then(JsonValue::as_u64) != Some(index as u64)
            || chunk.get("delivered").and_then(JsonValue::as_bool) != Some(true)
            || calls != if fallback { 2 } else { 1 }
            || bounded_count(chunk.get("attempt_count"), 1, 6).is_err()
            || chunk.get("state").and_then(JsonValue::as_str) != Some("completed")
            || !chunk.get("error_kind").is_some_and(JsonValue::is_null)
            || !chunk
                .get("http_status_code")
                .and_then(JsonValue::as_i64)
                .is_some_and(|status| (200..300).contains(&status))
        {
            return Err(());
        }
        fallback_count += usize::from(fallback);
        api_call_count += calls;
    }
    if object.get("fallback_count").and_then(JsonValue::as_u64) != Some(fallback_count as u64)
        || object.get("api_call_count").and_then(JsonValue::as_u64) != Some(api_call_count as u64)
    {
        return Err(());
    }
    Ok(())
}

fn validate_bootstrap_request(request: &TelegramUpdateBootstrapRequest) -> Result<(), ()> {
    if !valid_chat_id(request.chat_id())
        || !request.chat().is_object()
        || !request.from_user().is_object()
        || !valid_optional_bounded_text(Some(request.chat_title()), MAX_CHAT_TITLE_BYTES)
        || !valid_optional_bounded_text(request.raw_text(), MAX_RAW_TEXT_BYTES)
        || request.command().is_some_and(|(name, args)| {
            !valid_bounded_text(name, MAX_COMMAND_NAME_BYTES)
                || !valid_optional_bounded_text(Some(args), MAX_COMMAND_ARGS_BYTES)
        })
        || request.chat().to_string().len() > MAX_CONTEXT_BYTES
        || request.from_user().to_string().len() > MAX_CONTEXT_BYTES
    {
        return Err(());
    }
    Ok(())
}

fn validate_bootstrap_outcome(outcome: &JsonValue) -> Result<bool, ()> {
    let object = outcome.as_object().ok_or(())?;
    let handled = object
        .get("handled")
        .and_then(JsonValue::as_bool)
        .ok_or(())?;
    let blocked = object
        .get("blocked")
        .and_then(JsonValue::as_bool)
        .ok_or(())?;
    let adopted_owner = object
        .get("adopted_owner")
        .and_then(JsonValue::as_bool)
        .ok_or(())?;
    let state_saved = object
        .get("state_saved")
        .and_then(JsonValue::as_bool)
        .ok_or(())?;
    let message_sent = object
        .get("message_sent")
        .and_then(JsonValue::as_bool)
        .ok_or(())?;
    if object.get("contract").and_then(JsonValue::as_str) != Some(BOOTSTRAP_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str)
            != Some(BOOTSTRAP_MIGRATION_STAGE)
        || object.get("stage").and_then(JsonValue::as_str) != Some("execute")
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object
            .get("owner_bootstrap_state")
            .and_then(JsonValue::as_str)
            != Some("completed")
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || safe_bootstrap_decision(object.get("decision")).is_none()
        || object
            .get("auth_state_loaded")
            .and_then(JsonValue::as_bool)
            .is_none()
        || object
            .get("existing_binding_loaded")
            .and_then(JsonValue::as_bool)
            .is_none()
        || blocked != handled
        || (adopted_owner && (handled || !state_saved))
        || (message_sent && !handled)
        || object.get("side_effect_count").and_then(JsonValue::as_u64)
            != Some((usize::from(state_saved) + usize::from(message_sent)) as u64)
        || object
            .get("rust_state_execution_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("rust_message_delivery_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || !false_flag(object, "python_owner_bootstrap_allowed")
        || !false_flag(object, "python_state_mutation_allowed")
        || !false_flag(object, "python_message_delivery_allowed")
        || !false_flag(object, "request_payload_exposed")
        || !false_flag(object, "auth_state_exposed")
        || !false_flag(object, "chat_id_exposed")
        || !false_flag(object, "message_text_exposed")
    {
        return Err(());
    }
    Ok(handled)
}

fn safe_bootstrap_decision(value: Option<&JsonValue>) -> Option<&'static str> {
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

fn valid_chat_id(value: &JsonValue) -> bool {
    match value {
        JsonValue::Number(number) => number.as_i64().is_some(),
        JsonValue::String(value) => {
            value.trim() == value
                && valid_bounded_text(value, MAX_CHAT_ID_BYTES)
                && !value.chars().any(char::is_control)
        }
        _ => false,
    }
}

fn valid_bounded_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= max_bytes
        && !value.contains('\0')
        && !value.contains('\r')
        && !value.contains('\n')
}

fn valid_optional_bounded_text(value: Option<&str>, max_bytes: usize) -> bool {
    value.is_none_or(|value| {
        value.len() <= max_bytes && !value.contains('\0') && !value.contains('\r')
    })
}

fn bounded_count(value: Option<&JsonValue>, min: usize, max: usize) -> Result<usize, ()> {
    value
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| (min..=max).contains(value))
        .ok_or(())
}

fn false_flag(object: &ait_core::json_support::JsonMap<String, JsonValue>, key: &str) -> bool {
    object.get(key).and_then(JsonValue::as_bool) == Some(false)
}

fn validate_state_path(path: &Path) -> Result<(), String> {
    let text = path.to_string_lossy();
    if path.as_os_str().is_empty()
        || text.len() > MAX_STATE_PATH_BYTES
        || text.contains('\0')
        || text.contains('\r')
        || text.contains('\n')
    {
        return Err(bootstrap_configuration_error());
    }
    Ok(())
}

fn message_configuration_error() -> String {
    "Telegram update message configuration is invalid.".to_string()
}

fn message_execution_error() -> String {
    "Telegram update message execution failed.".to_string()
}

fn bootstrap_configuration_error() -> String {
    "Telegram update bootstrap configuration is invalid.".to_string()
}

fn bootstrap_execution_error() -> String {
    "Telegram update bootstrap execution failed.".to_string()
}

#[cfg(test)]
mod tests;
