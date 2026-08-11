use std::fmt;
use std::path::{Path, PathBuf};

use ait_core::json_support::{json, JsonValue};

use super::TelegramBackgroundSyncOperationPort;
use crate::event_loop::telegram_command_runtime::SelectedBackendTelegramCommandRuntimeReadPort;
use crate::event_loop::telegram_command_runtime::SystemTelegramCommandRuntimeClockPort;
use crate::event_loop::telegram_workflow_notifications::{
    execute_with_telegram_workflow_notification_ports,
    DefaultTelegramWorkflowNotificationFormatter, NativeTelegramWorkflowNotificationMessagePort,
};
use crate::runtime::AgentRuntimeBindingStore;
use crate::transport_config::{AgentRuntimeMode, AgentRuntimeTarget, AgentWorkflowMode};

const OPERATION_REQUEST_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramBackgroundSyncOperationRequest.v2";
const OPERATION_OUTCOME_CONTRACT: &str =
    "ait_agent_core.event_loop.TelegramBackgroundSyncOperationOutcome.v2";
const OPERATION_MIGRATION_STAGE: &str = "rust_agent_telegram_background_sync_execution";
const MAX_CHAT_BYTES: usize = 512;
const MAX_BINDING_BYTES: usize = 8 * 1_048_576;
const MAX_CONTEXT_BYTES: usize = 1_048_576;
const MAX_CONFIG_BYTES: usize = 512 * 1_024;
const MAX_STATE_PATH_BYTES: usize = 16 * 1_024;

pub trait TelegramBackgroundSyncChildExecutionPort: Send + Sync + 'static {
    fn execute_workflow_notifications(&self, chat_id: &JsonValue) -> Result<JsonValue, String>;
}

pub struct NativeTelegramBackgroundSyncOperationPort<C> {
    children: C,
}

impl<C> NativeTelegramBackgroundSyncOperationPort<C> {
    pub fn with_port(children: C) -> Self {
        Self { children }
    }
}

impl<C> fmt::Debug for NativeTelegramBackgroundSyncOperationPort<C> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramBackgroundSyncOperationPort")
            .field("rust_child_execution", &true)
            .field("configuration_exposed", &false)
            .finish()
    }
}

impl NativeTelegramBackgroundSyncOperationPort<NativeTelegramBackgroundSyncChildExecutionPort> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_path: impl Into<PathBuf>,
        runtime_target: AgentRuntimeTarget,
        request_timeout_seconds: Option<f64>,
        bot_token: impl Into<String>,
        ait_web_url: Option<String>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        let children = NativeTelegramBackgroundSyncChildExecutionPort::new(
            state_path,
            runtime_target,
            request_timeout_seconds,
            bot_token,
            ait_web_url,
            reply_markdown_enabled,
        )?;
        Ok(Self::with_port(children))
    }
}

impl<C> TelegramBackgroundSyncOperationPort for NativeTelegramBackgroundSyncOperationPort<C>
where
    C: TelegramBackgroundSyncChildExecutionPort,
{
    fn run_workflow_notifications(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let request = ValidatedOperationRequest::parse(request, "run_workflow_notifications")?;
        let metadata = self
            .children
            .execute_workflow_notifications(&request.chat_id)?;
        normalize_child_outcome("run_workflow_notifications", &metadata)
    }
}

pub struct NativeTelegramBackgroundSyncChildExecutionPort {
    state: AgentRuntimeBindingStore,
    workflow_reader: SelectedBackendTelegramCommandRuntimeReadPort,
    repo_name: String,
    bot_token: String,
    ait_web_url: Option<String>,
    request_timeout_seconds: Option<f64>,
    reply_markdown_enabled: bool,
}

impl NativeTelegramBackgroundSyncChildExecutionPort {
    pub fn new(
        state_path: impl Into<PathBuf>,
        runtime_target: AgentRuntimeTarget,
        request_timeout_seconds: Option<f64>,
        bot_token: impl Into<String>,
        ait_web_url: Option<String>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        let state_path = state_path.into();
        let bot_token = bot_token.into();
        validate_configuration(
            &state_path,
            &runtime_target,
            request_timeout_seconds,
            &bot_token,
            ait_web_url.as_deref(),
        )?;
        Ok(Self {
            state: AgentRuntimeBindingStore::new(state_path),
            workflow_reader: SelectedBackendTelegramCommandRuntimeReadPort::new(
                runtime_target.clone(),
                request_timeout_seconds,
            ),
            repo_name: runtime_target.repo_name,
            bot_token,
            ait_web_url,
            request_timeout_seconds,
            reply_markdown_enabled,
        })
    }
}

impl fmt::Debug for NativeTelegramBackgroundSyncChildExecutionPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramBackgroundSyncChildExecutionPort")
            .field("native_binding_state", &true)
            .field("native_workflow_reader", &true)
            .field("configuration_exposed", &false)
            .finish()
    }
}

impl TelegramBackgroundSyncChildExecutionPort for NativeTelegramBackgroundSyncChildExecutionPort {
    fn execute_workflow_notifications(&self, chat_id: &JsonValue) -> Result<JsonValue, String> {
        execute_with_telegram_workflow_notification_ports(
            &DefaultTelegramWorkflowNotificationFormatter,
            &self.state,
            &self.workflow_reader,
            &NativeTelegramWorkflowNotificationMessagePort,
            &SystemTelegramCommandRuntimeClockPort,
            &json!({
                "chat_id": chat_id,
                "config": {
                    "repo_name": self.repo_name,
                    "ait_web_url": self.ait_web_url,
                },
                "bot_token": self.bot_token,
                "request_timeout_seconds": self.request_timeout_seconds,
                "reply_markdown_enabled": self.reply_markdown_enabled,
            }),
        )
        .map(|execution| execution.into_metadata())
        .map_err(|_| "Telegram workflow notification execution failed.".to_string())
    }
}

struct ValidatedOperationRequest {
    chat_id: JsonValue,
}

impl ValidatedOperationRequest {
    fn parse(request: &JsonValue, expected_kind: &str) -> Result<Self, String> {
        let object = request
            .as_object()
            .ok_or_else(|| "Telegram background operation request is invalid.".to_string())?;
        let allowed = [
            "contract",
            "migration_stage",
            "transport",
            "operation_kind",
            "chat_id",
            "binding",
            "operation_context",
        ];
        if object.len() != allowed.len()
            || object.keys().any(|key| !allowed.contains(&key.as_str()))
            || object.get("contract").and_then(JsonValue::as_str)
                != Some(OPERATION_REQUEST_CONTRACT)
            || object.get("migration_stage").and_then(JsonValue::as_str)
                != Some(OPERATION_MIGRATION_STAGE)
            || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
            || object.get("operation_kind").and_then(JsonValue::as_str) != Some(expected_kind)
        {
            return Err("Telegram background operation request is invalid.".to_string());
        }
        let chat_id = object
            .get("chat_id")
            .cloned()
            .filter(valid_chat_id)
            .ok_or_else(|| "Telegram background operation request is invalid.".to_string())?;
        if !object
            .get("binding")
            .is_some_and(|value| value.is_object() && value.to_string().len() <= MAX_BINDING_BYTES)
            || !object.get("operation_context").is_some_and(|value| {
                value.is_object() && value.to_string().len() <= MAX_CONTEXT_BYTES
            })
        {
            return Err("Telegram background operation request is invalid.".to_string());
        }
        Ok(Self { chat_id })
    }
}

fn normalize_child_outcome(kind: &str, metadata: &JsonValue) -> Result<JsonValue, String> {
    let object = metadata
        .as_object()
        .ok_or_else(|| "Telegram background child returned invalid data.".to_string())?;
    let ok = object
        .get("ok")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    let sent_any = object
        .get("message_delivered")
        .or_else(|| object.get("notification_sent"))
        .or_else(|| object.get("sent_any"))
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let retryable = !ok
        && object
            .get("retryable")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true);
    Ok(json!({
        "contract": OPERATION_OUTCOME_CONTRACT,
        "operation_kind": kind,
        "ok": ok,
        "sent_any": sent_any && ok,
        "retryable": retryable,
        "terminal": !ok && !retryable,
    }))
}

fn validate_configuration(
    state_path: &Path,
    target: &AgentRuntimeTarget,
    timeout: Option<f64>,
    bot_token: &str,
    ait_web_url: Option<&str>,
) -> Result<(), String> {
    let path = state_path.to_string_lossy();
    if state_path.as_os_str().is_empty()
        || path.len() > MAX_STATE_PATH_BYTES
        || path
            .chars()
            .any(|value| matches!(value, '\0' | '\r' | '\n'))
        || target.repo_name.trim().is_empty()
        || target.repo_name.len() > MAX_CONFIG_BYTES
        || target.repo_root.as_os_str().is_empty()
        || bot_token.trim().is_empty()
        || timeout.is_some_and(|value| !value.is_finite() || value <= 0.0)
        || ait_web_url
            .is_some_and(|value| value.trim().is_empty() || value.len() > MAX_CONFIG_BYTES)
        || target.mode == AgentRuntimeMode::Local
            && target.workflow_mode != AgentWorkflowMode::SoloLocal
    {
        return Err("Telegram background operation configuration is invalid.".to_string());
    }
    Ok(())
}

fn valid_chat_id(value: &JsonValue) -> bool {
    match value {
        JsonValue::Number(value) => value.as_i64().is_some(),
        JsonValue::String(value) => {
            let value = value.trim();
            !value.is_empty()
                && value.len() <= MAX_CHAT_BYTES
                && !value.chars().any(char::is_control)
        }
        _ => false,
    }
}
