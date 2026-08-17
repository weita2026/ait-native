use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{
    NativeTelegramUpdateMessagePort, TelegramUpdateLifecyclePort, TelegramUpdateNormalTurnRequest,
    TelegramUpdatePortError,
};
use crate::event_loop::telegram_background_sync::{
    execute_with_telegram_background_sync_ports, NativeTelegramBackgroundSyncChildExecutionPort,
    NativeTelegramBackgroundSyncOperationPort, SystemTelegramBackgroundSyncClockPort,
};
use crate::event_loop::telegram_background_sync_state::DefaultTelegramBackgroundSyncStatePlanner;
use crate::event_loop::telegram_command_runtime::TelegramCommandRuntimeDeliveryPort;
use crate::event_loop::telegram_reply_delivery::agent_telegram_reply_delivery_execute_json;
use crate::event_loop::telegram_reply_spool::{
    execute_with_telegram_reply_spool_ports, DefaultTelegramReplySpoolPlanner,
    RuntimeBindingTelegramReplySpoolStatePort, SystemTelegramReplySpoolClockPort,
};
use crate::event_loop::telegram_workflow_query::{
    DefaultTelegramWorkflowQueryPlanner, TelegramWorkflowQueryPlanner,
};
use crate::runtime::{
    AgentRuntimeBackend, AgentRuntimeBindingStore, SelectedAitRuntimeBackend,
    AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT, AGENT_RUNTIME_BACKEND_CONTRACT,
};
use crate::transport::agent_transport_event_envelope_json;
use crate::transport_config::{AgentRuntimeMode, AgentRuntimeTarget};

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramUpdateLifecycleExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_update_lifecycle_execution";
const BACKGROUND_CONTRACT: &str = "ait_agent_core.event_loop.TelegramBackgroundSyncExecution.v2";
const BACKGROUND_STAGE: &str = "rust_agent_telegram_background_sync_execution";
const REPLY_DELIVERY_CONTRACT: &str = "ait_agent_core.event_loop.TelegramReplyDeliveryExecution.v1";
const REPLY_DELIVERY_STAGE: &str = "rust_agent_telegram_reply_delivery_execution";
const REPLY_SPOOL_CONTRACT: &str = "ait_agent_core.event_loop.TelegramReplySpoolExecution.v1";
const REPLY_SPOOL_STAGE: &str = "rust_agent_telegram_reply_spool_execution";
const WORKFLOW_QUERY_CONTRACT: &str = "ait_agent_core.event_loop.TelegramWorkflowQuery.v1";
const WORKFLOW_QUERY_STAGE: &str = "rust_agent_telegram_workflow_query";
const PENDING_REPLY_CALLBACK: &str = "run_pending_reply_turn_safe";
const TURN_FAILURE_MESSAGE: &str =
    "ait Telegram bot could not generate a reply. Please retry in a moment.";
const TURN_FAILURE_STATE: &str = "Telegram turn execution failed.";
const MAX_CHAT_ID_BYTES: usize = 512;
const MAX_CHAT_TITLE_BYTES: usize = 4_096;
const MAX_TEXT_BYTES: usize = 262_144;
const MAX_ACTOR_BYTES: usize = 4_096;
const MAX_CONVERSATION_KEY_BYTES: usize = 4_096;
const MAX_CONTEXT_BYTES: usize = 2 * 1_048_576;
const MAX_ATTACHMENTS: usize = 32;
const MAX_MESSAGE_IDS: usize = 1_000;
const MAX_CONFIG_BYTES: usize = 512 * 1_024;
const MAX_STATE_PATH_BYTES: usize = 16 * 1_024;
const MAX_IDLE_TIMEOUT: Duration = Duration::from_secs(86_400);
const REPLY_SPOOL_LIMIT: i64 = 100;

pub trait TelegramUpdateLifecycleExecutor: Send + Sync + 'static {
    fn execute_lifecycle(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait TelegramUpdateLifecycleRuntimePort: Send + Sync + 'static {
    fn ensure_conversation_binding(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let chat_id = request
            .get("chat_id")
            .and_then(scalar_text)
            .ok_or_else(lifecycle_execution_error)?;
        Ok(json!({
            "conversation_key": telegram_conversation_key(&chat_id),
        }))
    }

    fn execute_turn_backend(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn mutate_reply_spool(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn deliver_assistant_reply(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn send_failure_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String>;

    fn execute_background_sync(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub struct NativeTelegramUpdateLifecyclePort<E = DefaultTelegramUpdateLifecycleExecutor> {
    executor: E,
}

impl NativeTelegramUpdateLifecyclePort<DefaultTelegramUpdateLifecycleExecutor> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_path: impl Into<PathBuf>,
        runtime_target: AgentRuntimeTarget,
        request_timeout_seconds: Option<f64>,
        local_reply: Option<JsonValue>,
        bot_token: impl Into<String>,
        ait_web_url: Option<String>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        Ok(Self::with_executor(
            DefaultTelegramUpdateLifecycleExecutor::new(
                state_path,
                runtime_target,
                request_timeout_seconds,
                local_reply,
                bot_token,
                ait_web_url,
                reply_markdown_enabled,
            )?,
        ))
    }
}

impl<E> NativeTelegramUpdateLifecyclePort<E> {
    pub fn with_executor(executor: E) -> Self {
        Self { executor }
    }
}

impl<E> fmt::Debug for NativeTelegramUpdateLifecyclePort<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateLifecyclePort")
            .field("native_lifecycle_execution", &true)
            .field("adapter_owned_async_work", &false)
            .field("request_payload_exposed", &false)
            .field("executor_output_exposed", &false)
            .field("configuration_exposed", &false)
            .finish()
    }
}

impl<E> TelegramUpdateLifecyclePort for NativeTelegramUpdateLifecyclePort<E>
where
    E: TelegramUpdateLifecycleExecutor,
{
    fn handle_normal_turn(
        &self,
        request: &TelegramUpdateNormalTurnRequest,
    ) -> Result<(), TelegramUpdatePortError> {
        validate_normal_turn_request(request).map_err(|_| TelegramUpdatePortError)?;
        let outcome = self
            .executor
            .execute_lifecycle(&normal_turn_request(request))
            .map_err(|_| TelegramUpdatePortError)?;
        validate_lifecycle_outcome(&outcome, "normal_turn", None)
            .map(|_| ())
            .map_err(|_| TelegramUpdatePortError)
    }

    fn run_background_sync_for_chat(&self, chat_id: &str) -> Result<(), TelegramUpdatePortError> {
        if !valid_bounded_text(chat_id, MAX_CHAT_ID_BYTES, false)
            || chat_id.chars().any(char::is_control)
        {
            return Err(TelegramUpdatePortError);
        }
        let outcome = self
            .executor
            .execute_lifecycle(&json!({
                "operation": "background_sync",
                "chat_id": chat_id,
            }))
            .map_err(|_| TelegramUpdatePortError)?;
        validate_lifecycle_outcome(&outcome, "background_sync", None)
            .map(|_| ())
            .map_err(|_| TelegramUpdatePortError)
    }

    fn execute_reply(
        &self,
        callback_slot: &str,
        args: &[JsonValue],
    ) -> Result<(), TelegramUpdatePortError> {
        if callback_slot != PENDING_REPLY_CALLBACK
            || args.len() != 1
            || !args[0].is_object()
            || args[0].to_string().len() > MAX_CONTEXT_BYTES
        {
            return Err(TelegramUpdatePortError);
        }
        let outcome = self
            .executor
            .execute_lifecycle(&json!({
                "operation": "reply",
                "callback_slot": callback_slot,
                "args": args,
            }))
            .map_err(|_| TelegramUpdatePortError)?;
        validate_lifecycle_outcome(&outcome, "reply", None)
            .map(|_| ())
            .map_err(|_| TelegramUpdatePortError)
    }

    fn wait_for_live_replies(
        &self,
        timeout: Option<Duration>,
    ) -> Result<bool, TelegramUpdatePortError> {
        if timeout.is_some_and(|value| value > MAX_IDLE_TIMEOUT) {
            return Err(TelegramUpdatePortError);
        }
        let outcome = self
            .executor
            .execute_lifecycle(&json!({
                "operation": "wait_for_idle",
                "timeout_seconds": timeout.map(|value| value.as_secs_f64()),
            }))
            .map_err(|_| TelegramUpdatePortError)?;
        validate_lifecycle_outcome(&outcome, "wait_for_idle", Some(true))
            .map(|facts| facts.idle)
            .map_err(|_| TelegramUpdatePortError)
    }
}

pub struct DefaultTelegramUpdateLifecycleExecutor {
    runtime: NativeTelegramUpdateLifecycleRuntime,
}

impl DefaultTelegramUpdateLifecycleExecutor {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_path: impl Into<PathBuf>,
        runtime_target: AgentRuntimeTarget,
        request_timeout_seconds: Option<f64>,
        local_reply: Option<JsonValue>,
        bot_token: impl Into<String>,
        ait_web_url: Option<String>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        Ok(Self {
            runtime: NativeTelegramUpdateLifecycleRuntime::new(
                state_path,
                runtime_target,
                request_timeout_seconds,
                local_reply,
                bot_token,
                ait_web_url,
                reply_markdown_enabled,
            )?,
        })
    }
}

impl fmt::Debug for DefaultTelegramUpdateLifecycleExecutor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DefaultTelegramUpdateLifecycleExecutor")
            .field("native_runtime", &true)
            .field("configuration_exposed", &false)
            .finish()
    }
}

impl TelegramUpdateLifecycleExecutor for DefaultTelegramUpdateLifecycleExecutor {
    fn execute_lifecycle(&self, request: &JsonValue) -> Result<JsonValue, String> {
        execute_with_telegram_update_lifecycle_runtime(
            &self.runtime,
            &DefaultTelegramWorkflowQueryPlanner,
            request,
        )
    }
}

pub struct NativeTelegramUpdateLifecycleRuntime {
    binding_store: AgentRuntimeBindingStore,
    spool_state: RuntimeBindingTelegramReplySpoolStatePort,
    backend: SelectedAitRuntimeBackend,
    background:
        NativeTelegramBackgroundSyncOperationPort<NativeTelegramBackgroundSyncChildExecutionPort>,
    message: NativeTelegramUpdateMessagePort,
    runtime_target: AgentRuntimeTarget,
    request_timeout_seconds: Option<f64>,
    local_reply: Option<JsonValue>,
    bot_token: String,
    reply_markdown_enabled: bool,
}

impl NativeTelegramUpdateLifecycleRuntime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_path: impl Into<PathBuf>,
        runtime_target: AgentRuntimeTarget,
        request_timeout_seconds: Option<f64>,
        local_reply: Option<JsonValue>,
        bot_token: impl Into<String>,
        ait_web_url: Option<String>,
        reply_markdown_enabled: bool,
    ) -> Result<Self, String> {
        let state_path = state_path.into();
        let bot_token = bot_token.into();
        validate_native_configuration(
            &state_path,
            &runtime_target,
            request_timeout_seconds,
            &bot_token,
            ait_web_url.as_deref(),
        )?;
        Ok(Self {
            binding_store: AgentRuntimeBindingStore::new(state_path.clone()),
            spool_state: RuntimeBindingTelegramReplySpoolStatePort::new(state_path.clone()),
            backend: SelectedAitRuntimeBackend::default(),
            background: NativeTelegramBackgroundSyncOperationPort::new(
                state_path,
                runtime_target.clone(),
                request_timeout_seconds,
                bot_token.clone(),
                ait_web_url,
                reply_markdown_enabled,
            )?,
            message: NativeTelegramUpdateMessagePort::new(
                bot_token.clone(),
                request_timeout_seconds,
                reply_markdown_enabled,
            )?,
            runtime_target,
            request_timeout_seconds,
            local_reply,
            bot_token,
            reply_markdown_enabled,
        })
    }

    fn runtime_target_json(&self) -> JsonValue {
        let mut target = json!({
            "mode": self.runtime_target.mode.as_str(),
            "workflow_mode": self.runtime_target.workflow_mode.as_str(),
            "repo_name": self.runtime_target.repo_name,
            "repo_root": self.runtime_target.repo_root.to_string_lossy(),
        });
        match self.runtime_target.mode {
            AgentRuntimeMode::Local => {}
            AgentRuntimeMode::Remote => {
                target["remote_name"] = json!(self.runtime_target.remote_name);
                target["server_url"] = json!(self.runtime_target.server_url);
            }
        }
        target
    }

    fn turn_backend_request(&self, request: &JsonValue) -> JsonValue {
        let mut request = request.clone();
        request["target"] = self.runtime_target_json();
        request["timeout_seconds"] = json!(self.request_timeout_seconds);
        if let Some(local_reply) = &self.local_reply {
            request["local_reply"] = local_reply.clone();
        }
        request
    }
}

impl fmt::Debug for NativeTelegramUpdateLifecycleRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateLifecycleRuntime")
            .field("runtime_mode", &self.runtime_target.mode.as_str())
            .field(
                "request_timeout_configured",
                &self.request_timeout_seconds.is_some(),
            )
            .field("reply_markdown_enabled", &self.reply_markdown_enabled)
            .field("state_path_exposed", &false)
            .field("runtime_target_exposed", &false)
            .field("bot_token_exposed", &false)
            .finish()
    }
}

impl TelegramUpdateLifecycleRuntimePort for NativeTelegramUpdateLifecycleRuntime {
    fn ensure_conversation_binding(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let object = request.as_object().ok_or_else(lifecycle_execution_error)?;
        let chat_id = object
            .get("chat_id")
            .and_then(scalar_text)
            .ok_or_else(lifecycle_execution_error)?;
        let chat = object
            .get("chat")
            .and_then(JsonValue::as_object)
            .ok_or_else(lifecycle_execution_error)?;
        let chat_title = required_bounded_text(object, "chat_title", MAX_CHAT_TITLE_BYTES, true)?;
        let conversation_key = telegram_conversation_key(&chat_id);
        self.binding_store
            .execute(
                "upsert_binding",
                &json!({
                    "transport": "telegram",
                    "surface_id": chat_id,
                    "repo_name": self.runtime_target.repo_name,
                    "surface_title": chat_title,
                    "surface_kind": chat.get("type").cloned().unwrap_or(JsonValue::Null),
                    "updates": {
                        "conversation_key": conversation_key,
                        "telegram_chat_id": chat_id,
                        "telegram_chat_title": chat_title,
                        "telegram_chat_type": chat.get("type").cloned().unwrap_or(JsonValue::Null),
                    },
                }),
            )
            .map_err(|_| lifecycle_execution_error())
    }

    fn execute_turn_backend(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let request = self.turn_backend_request(request);
        self.backend.execute(&request)
    }

    fn mutate_reply_spool(&self, request: &JsonValue) -> Result<JsonValue, String> {
        execute_with_telegram_reply_spool_ports(
            &DefaultTelegramReplySpoolPlanner,
            &self.spool_state,
            &SystemTelegramReplySpoolClockPort,
            request,
        )
        .map_err(|_| lifecycle_execution_error())
    }

    fn deliver_assistant_reply(&self, request: &JsonValue) -> Result<JsonValue, String> {
        let mut request = request.clone();
        request["bot_token"] = json!(self.bot_token);
        request["request_timeout_seconds"] = json!(self.request_timeout_seconds);
        request["reply_markdown_enabled"] = json!(self.reply_markdown_enabled);
        request["should_execute"] = json!(true);
        agent_telegram_reply_delivery_execute_json(&request)
            .map_err(|_| lifecycle_execution_error())
    }

    fn send_failure_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), String> {
        self.message.send_message(chat_id, text)
    }

    fn execute_background_sync(&self, request: &JsonValue) -> Result<JsonValue, String> {
        execute_with_telegram_background_sync_ports(
            &DefaultTelegramBackgroundSyncStatePlanner,
            &self.binding_store,
            &self.background,
            &SystemTelegramBackgroundSyncClockPort,
            request,
        )
        .map(|execution| execution.into_metadata())
        .map_err(|_| lifecycle_execution_error())
    }
}

pub fn execute_with_telegram_update_lifecycle_runtime<R, P>(
    runtime: &R,
    planner: &P,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    R: TelegramUpdateLifecycleRuntimePort + ?Sized,
    P: TelegramWorkflowQueryPlanner + ?Sized,
{
    let object = request.as_object().ok_or_else(lifecycle_request_error)?;
    let operation = clean_text(object.get("operation")).ok_or_else(lifecycle_request_error)?;
    match operation.as_str() {
        "normal_turn" => {
            let normal = ValidatedNormalTurn::parse(object)?;
            let binding = runtime.ensure_conversation_binding(&json!({
                "chat_id": normal.chat_id,
                "chat": normal.chat,
                "chat_title": normal.chat_title,
            }))?;
            let conversation_key = required_bounded_text(
                binding.as_object().ok_or_else(lifecycle_request_error)?,
                "conversation_key",
                MAX_CONVERSATION_KEY_BYTES,
                false,
            )?;
            let actor_identity = match normal.actor_identity.clone() {
                Some(value) => value,
                None => workflow_text(
                    planner,
                    "actor_identity",
                    &json!({
                        "kind": "actor_identity",
                        "from_user": normal.from_user,
                        "chat_id": scalar_text(&normal.chat_id).ok_or_else(lifecycle_request_error)?,
                    }),
                    false,
                )?,
            };
            let actor_display_name = workflow_optional_text(
                planner,
                "user_display_name",
                &json!({
                    "kind": "user_display_name",
                    "from_user": normal.from_user,
                }),
            )?;
            let envelope =
                build_transport_envelope(&normal, &actor_identity, actor_display_name.as_deref())?;
            let pending = normal.pending_turn(&conversation_key, &actor_identity, envelope);
            remember_spool(runtime, &pending, "queued", false, None, None)?;
            execute_pending_turn(runtime, &pending, "normal_turn")
        }
        "reply" => {
            if clean_text(object.get("callback_slot")).as_deref() != Some(PENDING_REPLY_CALLBACK) {
                return Err(lifecycle_request_error());
            }
            let args = object
                .get("args")
                .and_then(JsonValue::as_array)
                .filter(|values| values.len() == 1)
                .ok_or_else(lifecycle_request_error)?;
            let pending = validate_pending_turn(&args[0])?;
            execute_pending_turn(runtime, &pending, "reply")
        }
        "background_sync" => {
            let chat_id = object.get("chat_id").ok_or_else(lifecycle_request_error)?;
            if !valid_chat_id(chat_id) {
                return Err(lifecycle_request_error());
            }
            let background = runtime.execute_background_sync(&json!({
                "chat_id": chat_id,
                "operation_context": {},
            }))?;
            let status = validate_background_outcome(&background)?;
            Ok(outcome(
                "background_sync",
                status,
                background
                    .get("ok")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                true,
            ))
        }
        "wait_for_idle" => {
            validate_optional_timeout(object.get("timeout_seconds"))?;
            Ok(outcome("wait_for_idle", "idle", true, true))
        }
        _ => Err(lifecycle_request_error()),
    }
}

fn execute_pending_turn<R>(
    runtime: &R,
    pending: &JsonValue,
    public_operation: &str,
) -> Result<JsonValue, String>
where
    R: TelegramUpdateLifecycleRuntimePort + ?Sized,
{
    let pending_object = pending.as_object().ok_or_else(lifecycle_request_error)?;
    let chat_id = pending_object
        .get("chat_id")
        .ok_or_else(lifecycle_request_error)?;
    let backend_chat_id = scalar_text(chat_id).ok_or_else(lifecycle_request_error)?;
    let conversation_key = required_bounded_text(
        pending_object,
        "conversation_key",
        MAX_CONVERSATION_KEY_BYTES,
        false,
    )?;
    remember_spool(runtime, pending, "attempting", true, None, None)?;
    let reply_text = if let Some(ready_reply) = clean_text(pending_object.get("ready_reply_text")) {
        ready_reply
    } else {
        let backend_request = json!({
            "operation": "create_telegram_turn",
            "actor": {
                "identity": required_bounded_text(pending_object, "actor_identity", MAX_ACTOR_BYTES, false)?,
                "type": "telegram_user",
            },
            "arguments": {
                "conversation_key": conversation_key,
                "provider_thread": pending_object
                    .get("provider_thread")
                    .filter(|value| value.is_object())
                    .cloned()
                    .unwrap_or(JsonValue::Null),
                "payload": {
                    "text": required_bounded_text(pending_object, "text", MAX_TEXT_BYTES, false)?,
                    "chat_id": backend_chat_id,
                    "chat_title": required_bounded_text(pending_object, "chat_title", MAX_CHAT_TITLE_BYTES, true)?,
                    "chat_type": pending_object.get("chat_type"),
                    "telegram_message_id": pending_object.get("telegram_message_id"),
                    "telegram_message_ids": pending_object.get("telegram_message_ids"),
                    "transport_envelope": pending_object.get("transport_envelope"),
                },
            },
        });
        let backend = match runtime.execute_turn_backend(&backend_request) {
            Ok(value) => value,
            Err(_) => return report_turn_failure(runtime, pending, public_operation),
        };
        let turn = match validate_backend_turn(&backend, &conversation_key) {
            Ok(value) => value,
            Err(_) => return report_turn_failure(runtime, pending, public_operation),
        };
        if turn.get("ok").and_then(JsonValue::as_bool) != Some(true) {
            return report_turn_failure(runtime, pending, public_operation);
        }
        let reply_text = match clean_text(turn.get("reply_text")) {
            Some(value) => value,
            None => return report_turn_failure(runtime, pending, public_operation),
        };
        remember_spool(
            runtime,
            pending,
            "ready",
            false,
            None,
            Some(&JsonValue::Object(turn.clone())),
        )?;
        reply_text
    };
    let delivered = runtime.deliver_assistant_reply(&json!({
        "chat_id": chat_id,
        "assistant_event": {},
        "reply_text": reply_text,
    }));
    let delivered = match delivered {
        Ok(value) if validate_reply_delivery_outcome(&value).is_ok() => value,
        _ => return report_turn_failure(runtime, pending, public_operation),
    };
    let _ = delivered;
    clear_spool(runtime, pending)?;
    Ok(outcome(public_operation, "completed", true, true))
}

fn report_turn_failure<R>(
    runtime: &R,
    pending: &JsonValue,
    public_operation: &str,
) -> Result<JsonValue, String>
where
    R: TelegramUpdateLifecycleRuntimePort + ?Sized,
{
    let chat_id = pending
        .get("chat_id")
        .ok_or_else(lifecycle_execution_error)?;
    remember_spool(
        runtime,
        pending,
        "failed",
        false,
        Some(TURN_FAILURE_STATE),
        None,
    )?;
    runtime.send_failure_message(chat_id, TURN_FAILURE_MESSAGE)?;
    Ok(outcome(public_operation, "turn_failed", true, true))
}

fn remember_spool<R>(
    runtime: &R,
    pending: &JsonValue,
    status: &str,
    attempt_increment: bool,
    last_error: Option<&str>,
    turn: Option<&JsonValue>,
) -> Result<(), String>
where
    R: TelegramUpdateLifecycleRuntimePort + ?Sized,
{
    let result = runtime.mutate_reply_spool(&json!({
        "stage": "remember",
        "pending_turn": pending,
        "status": status,
        "attempt_increment": attempt_increment,
        "last_error": last_error,
        "ready_reply_text": turn.and_then(|value| value.get("reply_text")),
        "provider_thread": turn.and_then(|value| value.get("provider_thread")),
        "turn_telemetry": turn.and_then(|value| value.get("turn_telemetry")),
        "spool_limit": REPLY_SPOOL_LIMIT,
    }))?;
    validate_spool_outcome(&result, "remember", true)
}

fn clear_spool<R>(runtime: &R, pending: &JsonValue) -> Result<(), String>
where
    R: TelegramUpdateLifecycleRuntimePort + ?Sized,
{
    let result = runtime.mutate_reply_spool(&json!({
        "stage": "clear",
        "pending_turn": pending,
    }))?;
    validate_spool_outcome(&result, "clear", false)
}

#[derive(Clone)]
struct ValidatedNormalTurn {
    chat_id: JsonValue,
    chat: JsonValue,
    from_user: JsonValue,
    chat_title: String,
    text: String,
    telegram_message_id: Option<i64>,
    telegram_message_ids: Vec<i64>,
    attachments: Vec<JsonValue>,
    actor_identity: Option<String>,
}

impl ValidatedNormalTurn {
    fn parse(object: &Map<String, JsonValue>) -> Result<Self, String> {
        let allowed = [
            "operation",
            "chat_id",
            "chat",
            "from_user",
            "chat_title",
            "text",
            "telegram_message_id",
            "telegram_message_ids",
            "attachments",
            "actor_identity",
            "defer_reply",
        ];
        if object.keys().any(|key| !allowed.contains(&key.as_str()))
            || object
                .get("defer_reply")
                .and_then(JsonValue::as_bool)
                .is_none()
        {
            return Err(lifecycle_request_error());
        }
        let chat_id = object
            .get("chat_id")
            .cloned()
            .filter(valid_chat_id)
            .ok_or_else(lifecycle_request_error)?;
        let chat = bounded_object(object.get("chat"), MAX_CONTEXT_BYTES)?;
        let from_user = bounded_object(object.get("from_user"), MAX_CONTEXT_BYTES)?;
        if chat.get("id").and_then(scalar_text) != scalar_text(&chat_id) {
            return Err(lifecycle_request_error());
        }
        let chat_title = required_bounded_text(object, "chat_title", MAX_CHAT_TITLE_BYTES, true)?;
        let text = required_bounded_text(object, "text", MAX_TEXT_BYTES, false)?;
        let telegram_message_id = optional_positive_i64(object.get("telegram_message_id"))?;
        let telegram_message_ids = positive_i64_array(object.get("telegram_message_ids"))?;
        let attachments = object
            .get("attachments")
            .and_then(JsonValue::as_array)
            .filter(|items| {
                items.len() <= MAX_ATTACHMENTS
                    && items.iter().all(JsonValue::is_object)
                    && JsonValue::Array((*items).clone()).to_string().len() <= MAX_CONTEXT_BYTES
            })
            .cloned()
            .ok_or_else(lifecycle_request_error)?;
        let actor_identity = optional_bounded_text(object.get("actor_identity"), MAX_ACTOR_BYTES)?;
        Ok(Self {
            chat_id,
            chat,
            from_user,
            chat_title,
            text,
            telegram_message_id,
            telegram_message_ids,
            attachments,
            actor_identity,
        })
    }

    fn pending_turn(
        &self,
        conversation_key: &str,
        actor_identity: &str,
        envelope: JsonValue,
    ) -> JsonValue {
        json!({
            "conversation_key": conversation_key,
            "chat_id": self.chat_id,
            "chat_type": self.chat.get("type"),
            "chat_title": self.chat_title,
            "actor_identity": actor_identity,
            "text": self.text,
            "telegram_message_id": self.telegram_message_id,
            "telegram_message_ids": self.telegram_message_ids,
            "transport_envelope": envelope,
        })
    }
}

fn normal_turn_request(request: &TelegramUpdateNormalTurnRequest) -> JsonValue {
    json!({
        "operation": "normal_turn",
        "chat_id": request.chat_id(),
        "chat": request.chat(),
        "from_user": request.from_user(),
        "chat_title": request.chat_title(),
        "text": request.text(),
        "telegram_message_id": request.telegram_message_id(),
        "telegram_message_ids": request.telegram_message_ids(),
        "attachments": request.attachments(),
        "actor_identity": request.actor_identity(),
        "defer_reply": request.defer_reply(),
    })
}

fn validate_normal_turn_request(request: &TelegramUpdateNormalTurnRequest) -> Result<(), ()> {
    if !valid_chat_id(request.chat_id())
        || !request.chat().is_object()
        || !request.from_user().is_object()
        || request.chat().to_string().len() > MAX_CONTEXT_BYTES
        || request.from_user().to_string().len() > MAX_CONTEXT_BYTES
        || !valid_bounded_text(request.chat_title(), MAX_CHAT_TITLE_BYTES, true)
        || !valid_bounded_text(request.text(), MAX_TEXT_BYTES, false)
        || request
            .telegram_message_id()
            .is_some_and(|value| value <= 0)
        || request.telegram_message_ids().len() > MAX_MESSAGE_IDS
        || request
            .telegram_message_ids()
            .iter()
            .any(|value| *value <= 0)
        || request.attachments().len() > MAX_ATTACHMENTS
        || request.attachments().iter().any(|value| !value.is_object())
        || request
            .actor_identity()
            .is_some_and(|value| !valid_bounded_text(value, MAX_ACTOR_BYTES, false))
    {
        return Err(());
    }
    Ok(())
}

fn build_transport_envelope(
    normal: &ValidatedNormalTurn,
    actor_identity: &str,
    actor_display_name: Option<&str>,
) -> Result<JsonValue, String> {
    let channel_id = scalar_text(&normal.chat_id).ok_or_else(lifecycle_request_error)?;
    let actor_transport_id = normal.from_user.get("id").and_then(scalar_text);
    let actor_username = clean_text(normal.from_user.get("username"));
    let channel_kind = clean_text(normal.chat.get("type"));
    let message_id = normal.telegram_message_id.map(JsonValue::from);
    let message_ids = JsonValue::Array(
        normal
            .telegram_message_ids
            .iter()
            .copied()
            .map(JsonValue::from)
            .collect(),
    );
    let attachments = JsonValue::Array(normal.attachments.clone());
    let envelope = agent_transport_event_envelope_json(
        "telegram",
        actor_identity,
        &channel_id,
        &normal.text,
        actor_transport_id.as_deref(),
        actor_username.as_deref(),
        actor_display_name,
        normal.from_user.get("is_bot").and_then(JsonValue::as_bool),
        Some(&normal.chat_title),
        channel_kind.as_deref(),
        None,
        message_id.as_ref(),
        Some(&message_ids),
        None,
        None,
        None,
        Some(&attachments),
        None,
    );
    if !envelope.is_object()
        || envelope.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || envelope.to_string().len() > MAX_CONTEXT_BYTES
    {
        return Err(lifecycle_execution_error());
    }
    Ok(envelope)
}

fn workflow_text<P>(
    planner: &P,
    kind: &str,
    request: &JsonValue,
    allow_empty: bool,
) -> Result<String, String>
where
    P: TelegramWorkflowQueryPlanner + ?Sized,
{
    let planned = planner
        .plan_json(request)
        .map_err(|_| lifecycle_execution_error())?;
    let object = validate_workflow_plan(&planned, kind)?;
    let value = object
        .get("text")
        .and_then(JsonValue::as_str)
        .ok_or_else(lifecycle_execution_error)?;
    if !valid_bounded_text(value, MAX_ACTOR_BYTES, allow_empty) {
        return Err(lifecycle_execution_error());
    }
    Ok(value.to_string())
}

fn workflow_optional_text<P>(
    planner: &P,
    kind: &str,
    request: &JsonValue,
) -> Result<Option<String>, String>
where
    P: TelegramWorkflowQueryPlanner + ?Sized,
{
    let planned = planner
        .plan_json(request)
        .map_err(|_| lifecycle_execution_error())?;
    let object = validate_workflow_plan(&planned, kind)?;
    match object.get("text") {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) if valid_bounded_text(value, MAX_ACTOR_BYTES, false) => {
            Ok(Some(value.clone()))
        }
        _ => Err(lifecycle_execution_error()),
    }
}

fn validate_workflow_plan<'a>(
    planned: &'a JsonValue,
    kind: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    let object = planned.as_object().ok_or_else(lifecycle_execution_error)?;
    if object
        .get("workflow_query_contract")
        .and_then(JsonValue::as_str)
        != Some(WORKFLOW_QUERY_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str) != Some(WORKFLOW_QUERY_STAGE)
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
        return Err(lifecycle_execution_error());
    }
    Ok(object)
}

fn validate_backend_turn<'a>(
    backend: &'a JsonValue,
    conversation_key: &str,
) -> Result<&'a Map<String, JsonValue>, String> {
    let object = backend.as_object().ok_or_else(lifecycle_execution_error)?;
    if object.get("contract").and_then(JsonValue::as_str) != Some(AGENT_RUNTIME_BACKEND_CONTRACT)
        || object.get("backend_contract").and_then(JsonValue::as_str)
            != Some(AGENT_GATEWAY_REPLY_RUNTIME_CONTRACT)
        || object.get("operation").and_then(JsonValue::as_str) != Some("create_telegram_turn")
        || object.get("backend").and_then(JsonValue::as_str) != Some("gateway")
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || object
            .get("python_backend_selection_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(lifecycle_execution_error());
    }
    let payload = object
        .get("payload")
        .and_then(JsonValue::as_object)
        .ok_or_else(lifecycle_execution_error)?;
    let turn_ok = payload
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(lifecycle_execution_error)?;
    if payload.get("conversation_key").and_then(JsonValue::as_str) != Some(conversation_key)
        || turn_ok && clean_text(payload.get("reply_text")).is_none()
    {
        return Err(lifecycle_execution_error());
    }
    Ok(payload)
}

fn validate_spool_outcome(value: &JsonValue, stage: &str, must_apply: bool) -> Result<(), String> {
    let object = value.as_object().ok_or_else(lifecycle_execution_error)?;
    if object.get("contract").and_then(JsonValue::as_str) != Some(REPLY_SPOOL_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str) != Some(REPLY_SPOOL_STAGE)
        || object.get("stage").and_then(JsonValue::as_str) != Some(stage)
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || must_apply && object.get("applied").and_then(JsonValue::as_bool) != Some(true)
        || object
            .get("python_reply_spool_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(lifecycle_execution_error());
    }
    Ok(())
}

fn validate_reply_delivery_outcome(value: &JsonValue) -> Result<(), String> {
    let object = value.as_object().ok_or_else(lifecycle_execution_error)?;
    if object.get("contract").and_then(JsonValue::as_str) != Some(REPLY_DELIVERY_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str) != Some(REPLY_DELIVERY_STAGE)
        || object.get("stage").and_then(JsonValue::as_str) != Some("execute")
        || object
            .get("reply_delivery_state")
            .and_then(JsonValue::as_str)
            != Some("completed")
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || object.get("delivered").and_then(JsonValue::as_bool) != Some(true)
        || !false_flags(
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
        return Err(lifecycle_execution_error());
    }
    Ok(())
}

fn validate_background_outcome(value: &JsonValue) -> Result<&str, String> {
    let object = value.as_object().ok_or_else(lifecycle_execution_error)?;
    if object.get("contract").and_then(JsonValue::as_str) != Some(BACKGROUND_CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str) != Some(BACKGROUND_STAGE)
        || object.get("stage").and_then(JsonValue::as_str) != Some("execute")
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || !false_flags(
            object,
            &[
                "python_background_sync_allowed",
                "python_operation_execution_allowed",
                "python_state_mutation_allowed",
                "raw_request_exposed",
                "raw_binding_exposed",
                "raw_operation_result_exposed",
                "operation_context_exposed",
                "chat_id_exposed",
                "runtime_target_exposed",
                "bot_token_exposed",
                "queue_payload_exposed",
                "formatted_text_exposed",
                "state_path_exposed",
                "downstream_error_exposed",
            ],
        )
    {
        return Err(lifecycle_execution_error());
    }
    let status = object
        .get("background_sync_status")
        .and_then(JsonValue::as_str)
        .ok_or_else(lifecycle_execution_error)?;
    let ok = required_bool_field(object, "ok")?;
    let retryable = required_bool_field(object, "retryable")?;
    let terminal = required_bool_field(object, "terminal")?;
    for key in [
        "has_work",
        "backoff_active",
        "sent_any",
        "state_updated",
        "retry_scheduled",
    ] {
        required_bool_field(object, key)?;
    }
    for key in [
        "operation_count",
        "completed_operation_count",
        "failed_operation_count",
        "failure_streak",
    ] {
        object
            .get(key)
            .and_then(JsonValue::as_u64)
            .ok_or_else(lifecycle_execution_error)?;
    }
    let disposition_valid = match status {
        "missing_binding" | "no_work" | "completed" => ok && !retryable && !terminal,
        "backoff_active" => ok && retryable && !terminal,
        "failed_retryable" => !ok && retryable && !terminal,
        "failed_terminal" | "binding_removed" => !ok && !retryable && terminal,
        _ => false,
    };
    if !disposition_valid {
        return Err(lifecycle_execution_error());
    }
    Ok(status)
}

struct LifecycleFacts {
    idle: bool,
}

fn validate_lifecycle_outcome(
    value: &JsonValue,
    operation: &str,
    expected_idle: Option<bool>,
) -> Result<LifecycleFacts, ()> {
    let object = value.as_object().ok_or(())?;
    let status = object
        .get("lifecycle_state")
        .and_then(JsonValue::as_str)
        .ok_or(())?;
    let idle = object.get("idle").and_then(JsonValue::as_bool).ok_or(())?;
    if object.get("contract").and_then(JsonValue::as_str) != Some(CONTRACT)
        || object.get("migration_stage").and_then(JsonValue::as_str) != Some(MIGRATION_STAGE)
        || object.get("stage").and_then(JsonValue::as_str) != Some("execute")
        || object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object.get("operation").and_then(JsonValue::as_str) != Some(operation)
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || object.get("ok").and_then(JsonValue::as_bool).is_none()
        || !false_flags(
            object,
            &[
                "python_lifecycle_allowed",
                "python_reply_turn_allowed",
                "python_background_sync_allowed",
                "python_reply_spool_allowed",
                "request_payload_exposed",
                "chat_id_exposed",
                "actor_identity_exposed",
                "message_text_exposed",
                "attachment_exposed",
                "bot_token_exposed",
                "runtime_target_exposed",
                "downstream_error_exposed",
            ],
        )
        || expected_idle.is_some_and(|expected| idle != expected)
        || !valid_lifecycle_status(operation, status)
    {
        return Err(());
    }
    Ok(LifecycleFacts { idle })
}

fn outcome(operation: &str, status: &str, ok: bool, idle: bool) -> JsonValue {
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "transport": "telegram",
        "operation": operation,
        "lifecycle_state": status,
        "ok": ok,
        "completed": true,
        "idle": idle,
        "python_lifecycle_allowed": false,
        "python_reply_turn_allowed": false,
        "python_background_sync_allowed": false,
        "python_reply_spool_allowed": false,
        "request_payload_exposed": false,
        "chat_id_exposed": false,
        "actor_identity_exposed": false,
        "message_text_exposed": false,
        "attachment_exposed": false,
        "bot_token_exposed": false,
        "runtime_target_exposed": false,
        "downstream_error_exposed": false,
    })
}

fn validate_pending_turn(value: &JsonValue) -> Result<JsonValue, String> {
    let object = value.as_object().ok_or_else(lifecycle_request_error)?;
    let allowed = [
        "conversation_key",
        "chat_id",
        "chat_type",
        "chat_title",
        "actor_identity",
        "text",
        "telegram_message_id",
        "telegram_message_ids",
        "transport_envelope",
        "ready_reply_text",
        "provider_thread",
        "turn_telemetry",
    ];
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(lifecycle_request_error());
    }
    required_bounded_text(
        object,
        "conversation_key",
        MAX_CONVERSATION_KEY_BYTES,
        false,
    )?;
    let chat_id = object.get("chat_id").ok_or_else(lifecycle_request_error)?;
    if !valid_chat_id(chat_id) {
        return Err(lifecycle_request_error());
    }
    match object.get("chat_type") {
        Some(JsonValue::Null) => {}
        Some(JsonValue::String(value))
            if valid_bounded_text(value, MAX_CHAT_TITLE_BYTES, false) => {}
        _ => return Err(lifecycle_request_error()),
    }
    required_bounded_text(object, "chat_title", MAX_CHAT_TITLE_BYTES, true)?;
    let actor_identity = required_bounded_text(object, "actor_identity", MAX_ACTOR_BYTES, false)?;
    let text = required_bounded_text(object, "text", MAX_TEXT_BYTES, false)?;
    optional_positive_i64(object.get("telegram_message_id"))?;
    positive_i64_array(object.get("telegram_message_ids"))?;
    let envelope = object
        .get("transport_envelope")
        .and_then(JsonValue::as_object)
        .ok_or_else(lifecycle_request_error)?;
    if envelope.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || envelope
            .get("channel")
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("channel_id"))
            .and_then(JsonValue::as_str)
            != scalar_text(chat_id).as_deref()
        || envelope
            .get("actor")
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("actor_identity"))
            .and_then(JsonValue::as_str)
            != Some(actor_identity.as_str())
        || envelope
            .get("message")
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("text"))
            .and_then(JsonValue::as_str)
            != Some(text.as_str())
        || object.get("ready_reply_text").is_some_and(|value| {
            !value.is_null()
                && value
                    .as_str()
                    .is_none_or(|text| !valid_bounded_text(text, MAX_TEXT_BYTES, false))
        })
        || object
            .get("provider_thread")
            .is_some_and(|value| !value.is_null() && !value.is_object())
        || object
            .get("turn_telemetry")
            .is_some_and(|value| !value.is_null() && !value.is_object())
        || value.to_string().len() > MAX_CONTEXT_BYTES
    {
        return Err(lifecycle_request_error());
    }
    Ok(value.clone())
}

fn telegram_conversation_key(chat_id: &str) -> String {
    format!("telegram:{chat_id}")
}

fn bounded_object(value: Option<&JsonValue>, max_bytes: usize) -> Result<JsonValue, String> {
    value
        .filter(|value| value.is_object() && value.to_string().len() <= max_bytes)
        .cloned()
        .ok_or_else(lifecycle_request_error)
}

fn optional_bounded_text(
    value: Option<&JsonValue>,
    max_bytes: usize,
) -> Result<Option<String>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) if valid_bounded_text(value, max_bytes, false) => {
            Ok(Some(value.clone()))
        }
        _ => Err(lifecycle_request_error()),
    }
}

fn optional_positive_i64(value: Option<&JsonValue>) -> Result<Option<i64>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(lifecycle_request_error),
    }
}

fn positive_i64_array(value: Option<&JsonValue>) -> Result<Vec<i64>, String> {
    let values = value
        .and_then(JsonValue::as_array)
        .ok_or_else(lifecycle_request_error)?;
    if values.len() > MAX_MESSAGE_IDS {
        return Err(lifecycle_request_error());
    }
    values
        .iter()
        .map(|value| {
            value
                .as_i64()
                .filter(|value| *value > 0)
                .ok_or_else(lifecycle_request_error)
        })
        .collect()
}

fn required_bounded_text(
    object: &Map<String, JsonValue>,
    key: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<String, String> {
    object
        .get(key)
        .and_then(JsonValue::as_str)
        .filter(|value| valid_bounded_text(value, max_bytes, allow_empty))
        .map(str::to_string)
        .ok_or_else(lifecycle_request_error)
}

fn validate_optional_timeout(value: Option<&JsonValue>) -> Result<(), String> {
    match value {
        None | Some(JsonValue::Null) => Ok(()),
        Some(value)
            if value
                .as_f64()
                .is_some_and(|value| value.is_finite() && (0.0..=86_400.0).contains(&value)) =>
        {
            Ok(())
        }
        _ => Err(lifecycle_request_error()),
    }
}

fn validate_native_configuration(
    state_path: &Path,
    target: &AgentRuntimeTarget,
    timeout: Option<f64>,
    bot_token: &str,
    ait_web_url: Option<&str>,
) -> Result<(), String> {
    let state_text = state_path.to_string_lossy();
    if state_path.as_os_str().is_empty()
        || state_text.len() > MAX_STATE_PATH_BYTES
        || state_text
            .chars()
            .any(|value| matches!(value, '\0' | '\r' | '\n'))
        || !valid_bounded_text(&target.repo_name, MAX_CONFIG_BYTES, false)
        || target.repo_root.as_os_str().is_empty()
        || target.repo_root.to_string_lossy().len() > MAX_CONFIG_BYTES
        || !valid_bounded_text(bot_token, 4_096, false)
        || timeout.is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 86_400.0)
        || ait_web_url.is_some_and(|value| !valid_bounded_text(value, MAX_CONFIG_BYTES, false))
    {
        return Err(lifecycle_configuration_error());
    }
    Ok(())
}

fn valid_chat_id(value: &JsonValue) -> bool {
    scalar_text(value).is_some_and(|value| {
        valid_bounded_text(&value, MAX_CHAT_ID_BYTES, false) && !value.chars().any(char::is_control)
    })
}

fn valid_bounded_text(value: &str, max_bytes: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty())
        && value.len() <= max_bytes
        && !value.contains('\0')
        && !value.contains('\r')
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

fn false_flags(object: &Map<String, JsonValue>, keys: &[&str]) -> bool {
    keys.iter()
        .all(|key| object.get(*key).and_then(JsonValue::as_bool) == Some(false))
}

fn required_bool_field(object: &Map<String, JsonValue>, key: &str) -> Result<bool, String> {
    object
        .get(key)
        .and_then(JsonValue::as_bool)
        .ok_or_else(lifecycle_execution_error)
}

fn valid_lifecycle_status(operation: &str, status: &str) -> bool {
    match operation {
        "normal_turn" | "reply" => matches!(status, "completed" | "turn_failed"),
        "background_sync" => matches!(
            status,
            "missing_binding"
                | "no_work"
                | "backoff_active"
                | "completed"
                | "failed_retryable"
                | "failed_terminal"
                | "binding_removed"
        ),
        "wait_for_idle" => status == "idle",
        _ => false,
    }
}

fn lifecycle_request_error() -> String {
    "Telegram update lifecycle request is invalid.".to_string()
}

fn lifecycle_configuration_error() -> String {
    "Telegram update lifecycle configuration is invalid.".to_string()
}

fn lifecycle_execution_error() -> String {
    "Telegram update lifecycle execution failed.".to_string()
}

#[cfg(test)]
mod tests;
