use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::telegram_logical_turn_runtime::TelegramLogicalTurn;
use super::telegram_submission_runtime::TelegramSubmissionExecutionPort;
use super::telegram_turn_inputs::{DefaultTelegramTurnInputPlanner, TelegramTurnInputPlanner};
use super::telegram_workflow_query::{
    DefaultTelegramWorkflowQueryPlanner, TelegramWorkflowQueryPlanner,
};

mod native_adapters;
mod native_command_adapters;
mod native_input_adapters;
mod native_lifecycle_adapters;
mod native_operational_adapters;

pub use native_adapters::{
    DefaultTelegramUpdateMessageExecutor, NativeTelegramUpdateBootstrapPort,
    NativeTelegramUpdateMessagePort, NativeTelegramUpdateOwnerBootstrapExecutor,
    SystemTelegramUpdateDiagnosticsPort, TelegramUpdateMessageExecutor,
    TelegramUpdateOwnerBootstrapExecutor,
};
pub use native_command_adapters::{
    NativeTelegramUpdateCommandPort, NativeTelegramUpdateCommandRuntimeExecutor,
    TelegramUpdateCommandExecutor,
};
pub use native_input_adapters::{
    DefaultTelegramUpdateFileDownloadExecutor, NativeTelegramUpdateInputPort,
    TelegramUpdateFileDownloadExecutor,
};
pub use native_lifecycle_adapters::{
    execute_with_telegram_update_lifecycle_runtime, DefaultTelegramUpdateLifecycleExecutor,
    NativeTelegramUpdateLifecyclePort, NativeTelegramUpdateLifecycleRuntime,
    TelegramUpdateLifecycleExecutor, TelegramUpdateLifecycleRuntimePort,
};
pub use native_operational_adapters::{
    DefaultTelegramUpdateAssistantReplyExecutor, NativeTelegramOperationalTriggerDeliveryPort,
    NativeTelegramUpdateOperationalExecutor, NativeTelegramUpdateOperationalPort,
    SystemTelegramOperationalTriggerDiagnosticsPort, TelegramUpdateAssistantReplyExecutor,
    TelegramUpdateOperationalExecutor, TelegramUpdateOperationalMessagePort,
};

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramUpdateJob.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_update_job";
const TURN_INPUT_CONTRACT: &str = "ait_agent_core.event_loop.TelegramTurnInput.v1";
const TURN_INPUT_STAGE: &str = "rust_agent_telegram_turn_input";
const WORKFLOW_QUERY_CONTRACT: &str = "ait_agent_core.event_loop.TelegramWorkflowQuery.v1";
const WORKFLOW_QUERY_STAGE: &str = "rust_agent_telegram_workflow_query";
const EMPTY_TEXT_HELP: &str = "Send a message after the bot mention, or use /help.";
const UPDATE_FAILURE_MESSAGE: &str =
    "ait Telegram bot hit an unexpected error while processing this update. Check the daemon log and retry if needed.";
const MAX_USERNAME_LENGTH: usize = 128;
const MAX_TEXT_LENGTH: usize = 16_384;
const MAX_TITLE_LENGTH: usize = 1_024;
const MAX_ATTACHMENTS: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelegramUpdateJobConfig {
    username: String,
    speech_to_text_enabled: bool,
    stt_include_audio_uploads: bool,
    defer_normal_text_turn: bool,
}

impl TelegramUpdateJobConfig {
    pub fn new(
        username: impl Into<String>,
        speech_to_text_enabled: bool,
        stt_include_audio_uploads: bool,
        defer_normal_text_turn: bool,
    ) -> Self {
        Self {
            username: username.into(),
            speech_to_text_enabled,
            stt_include_audio_uploads,
            defer_normal_text_turn,
        }
    }

    pub fn username(&self) -> &str {
        self.username.as_str()
    }

    pub fn speech_to_text_enabled(&self) -> bool {
        self.speech_to_text_enabled
    }

    pub fn stt_include_audio_uploads(&self) -> bool {
        self.stt_include_audio_uploads
    }

    pub fn defer_normal_text_turn(&self) -> bool {
        self.defer_normal_text_turn
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramUpdateInputMode {
    SpeechToText,
    DownloadAttachments,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramUpdateInputRequest {
    mode: TelegramUpdateInputMode,
    message: JsonValue,
    candidate_raw_text: Option<String>,
    attachments: Vec<JsonValue>,
}

impl TelegramUpdateInputRequest {
    fn new(
        mode: TelegramUpdateInputMode,
        message: JsonValue,
        candidate_raw_text: Option<String>,
        attachments: Vec<JsonValue>,
    ) -> Self {
        Self {
            mode,
            message,
            candidate_raw_text,
            attachments,
        }
    }

    pub fn mode(&self) -> TelegramUpdateInputMode {
        self.mode
    }

    pub fn message(&self) -> &JsonValue {
        &self.message
    }

    pub fn candidate_raw_text(&self) -> Option<&str> {
        self.candidate_raw_text.as_deref()
    }

    pub fn attachments(&self) -> &[JsonValue] {
        self.attachments.as_slice()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramPreparedUpdateInput {
    raw_text: Option<String>,
    attachments: Vec<JsonValue>,
}

impl TelegramPreparedUpdateInput {
    pub fn new(raw_text: Option<String>, attachments: Vec<JsonValue>) -> Self {
        Self {
            raw_text,
            attachments,
        }
    }

    pub fn raw_text(&self) -> Option<&str> {
        self.raw_text.as_deref()
    }

    pub fn attachments(&self) -> &[JsonValue] {
        self.attachments.as_slice()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramUpdateInputErrorKind {
    SpeechToTextNotEnabled,
    SpeechAttachmentMissing,
    SpeechFileIdMissing,
    SpeechBackendUnavailable,
    SpeechTranscriptionFailed,
    SpeechTimeout,
    SpeechEmpty,
    AttachmentFileIdMissing,
    AttachmentPathMissing,
    AttachmentHostUnsupported,
    AttachmentDownloadFailed,
    Timeout,
}

impl TelegramUpdateInputErrorKind {
    pub fn user_message(self) -> &'static str {
        match self {
            Self::SpeechToTextNotEnabled => {
                "Local STT is not enabled for this Telegram worker. Set `AIT_TELEGRAM_STT_MODE=local-stt` and retry."
            }
            Self::SpeechAttachmentMissing => {
                "No local-STT attachment was found in this Telegram message."
            }
            Self::SpeechFileIdMissing => {
                "The Telegram voice attachment did not include a downloadable file id."
            }
            Self::SpeechBackendUnavailable => {
                "Local STT requires a configured non-Python backend on the Telegram worker host. Configure it there and retry."
            }
            Self::SpeechTranscriptionFailed => {
                "Local STT failed while transcribing that audio. Please retry or send text instead."
            }
            Self::SpeechTimeout => {
                "Local STT timed out while transcribing that audio. Please retry or send text instead."
            }
            Self::SpeechEmpty => {
                "Local STT could not hear any speech in that message. Please retry or send text instead."
            }
            Self::AttachmentFileIdMissing => {
                "That Telegram attachment did not include a downloadable file id."
            }
            Self::AttachmentPathMissing => {
                "Telegram did not return a downloadable file path for that attachment."
            }
            Self::AttachmentHostUnsupported => {
                "This Telegram worker cannot download that attachment on the current host."
            }
            Self::AttachmentDownloadFailed | Self::Timeout => {
                "Telegram file download failed. Please retry in a moment."
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramUpdateInputError {
    kind: TelegramUpdateInputErrorKind,
}

impl TelegramUpdateInputError {
    pub fn new(kind: TelegramUpdateInputErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(self) -> TelegramUpdateInputErrorKind {
        self.kind
    }
}

impl fmt::Display for TelegramUpdateInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Telegram update input preparation failed.")
    }
}

impl std::error::Error for TelegramUpdateInputError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramUpdatePortError;

impl fmt::Display for TelegramUpdatePortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Telegram update execution port failed.")
    }
}

impl std::error::Error for TelegramUpdatePortError {}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramUpdateBootstrapRequest {
    chat_id: JsonValue,
    chat: JsonValue,
    from_user: JsonValue,
    chat_title: String,
    raw_text: Option<String>,
    command: Option<(String, String)>,
    attachments_present: bool,
}

impl TelegramUpdateBootstrapRequest {
    pub fn chat_id(&self) -> &JsonValue {
        &self.chat_id
    }

    pub fn chat(&self) -> &JsonValue {
        &self.chat
    }

    pub fn from_user(&self) -> &JsonValue {
        &self.from_user
    }

    pub fn chat_title(&self) -> &str {
        self.chat_title.as_str()
    }

    pub fn raw_text(&self) -> Option<&str> {
        self.raw_text.as_deref()
    }

    pub fn command(&self) -> Option<(&str, &str)> {
        self.command
            .as_ref()
            .map(|(name, args)| (name.as_str(), args.as_str()))
    }

    pub fn attachments_present(&self) -> bool {
        self.attachments_present
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramUpdateOperationalRequest {
    chat_id: JsonValue,
    chat: JsonValue,
    from_user: JsonValue,
    chat_title: String,
    raw_text: String,
    normalized_text: String,
    command: Option<(String, String)>,
    telegram_message_id: Option<i64>,
    telegram_message_ids: Vec<i64>,
    reply_to_message: Option<JsonValue>,
    attachments: Vec<JsonValue>,
    actor_identity: Option<String>,
    message: JsonValue,
}

impl TelegramUpdateOperationalRequest {
    pub fn chat_id(&self) -> &JsonValue {
        &self.chat_id
    }

    pub fn chat(&self) -> &JsonValue {
        &self.chat
    }

    pub fn from_user(&self) -> &JsonValue {
        &self.from_user
    }

    pub fn chat_title(&self) -> &str {
        self.chat_title.as_str()
    }

    pub fn raw_text(&self) -> &str {
        self.raw_text.as_str()
    }

    pub fn normalized_text(&self) -> &str {
        self.normalized_text.as_str()
    }

    pub fn command(&self) -> Option<(&str, &str)> {
        self.command
            .as_ref()
            .map(|(name, args)| (name.as_str(), args.as_str()))
    }

    pub fn telegram_message_id(&self) -> Option<i64> {
        self.telegram_message_id
    }

    pub fn telegram_message_ids(&self) -> &[i64] {
        self.telegram_message_ids.as_slice()
    }

    pub fn reply_to_message(&self) -> Option<&JsonValue> {
        self.reply_to_message.as_ref()
    }

    pub fn attachments(&self) -> &[JsonValue] {
        self.attachments.as_slice()
    }

    pub fn actor_identity(&self) -> Option<&str> {
        self.actor_identity.as_deref()
    }

    pub fn message(&self) -> &JsonValue {
        &self.message
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramUpdateCommandRequest {
    chat_id: JsonValue,
    chat: JsonValue,
    from_user: JsonValue,
    chat_title: String,
    command_name: String,
    command_args: String,
}

impl TelegramUpdateCommandRequest {
    pub fn chat_id(&self) -> &JsonValue {
        &self.chat_id
    }

    pub fn chat(&self) -> &JsonValue {
        &self.chat
    }

    pub fn from_user(&self) -> &JsonValue {
        &self.from_user
    }

    pub fn chat_title(&self) -> &str {
        self.chat_title.as_str()
    }

    pub fn command_name(&self) -> &str {
        self.command_name.as_str()
    }

    pub fn command_args(&self) -> &str {
        self.command_args.as_str()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TelegramUpdateNormalTurnRequest {
    chat_id: JsonValue,
    chat: JsonValue,
    from_user: JsonValue,
    chat_title: String,
    text: String,
    telegram_message_id: Option<i64>,
    telegram_message_ids: Vec<i64>,
    attachments: Vec<JsonValue>,
    actor_identity: Option<String>,
    defer_reply: bool,
}

impl TelegramUpdateNormalTurnRequest {
    pub fn chat_id(&self) -> &JsonValue {
        &self.chat_id
    }

    pub fn chat(&self) -> &JsonValue {
        &self.chat
    }

    pub fn from_user(&self) -> &JsonValue {
        &self.from_user
    }

    pub fn chat_title(&self) -> &str {
        self.chat_title.as_str()
    }

    pub fn text(&self) -> &str {
        self.text.as_str()
    }

    pub fn telegram_message_id(&self) -> Option<i64> {
        self.telegram_message_id
    }

    pub fn telegram_message_ids(&self) -> &[i64] {
        self.telegram_message_ids.as_slice()
    }

    pub fn attachments(&self) -> &[JsonValue] {
        self.attachments.as_slice()
    }

    pub fn actor_identity(&self) -> Option<&str> {
        self.actor_identity.as_deref()
    }

    pub fn defer_reply(&self) -> bool {
        self.defer_reply
    }
}

pub trait TelegramUpdateInputPort: Send + Sync + 'static {
    fn prepare_input(
        &self,
        request: &TelegramUpdateInputRequest,
    ) -> Result<TelegramPreparedUpdateInput, TelegramUpdateInputError>;
}

pub trait TelegramUpdateBootstrapPort: Send + Sync + 'static {
    fn handle_bootstrap(
        &self,
        request: &TelegramUpdateBootstrapRequest,
    ) -> Result<bool, TelegramUpdatePortError>;
}

pub trait TelegramUpdateOperationalPort: Send + Sync + 'static {
    fn handle_operational_trigger(
        &self,
        request: &TelegramUpdateOperationalRequest,
    ) -> Result<bool, TelegramUpdatePortError>;
}

pub trait TelegramUpdateCommandPort: Send + Sync + 'static {
    fn execute_command(
        &self,
        request: &TelegramUpdateCommandRequest,
    ) -> Result<(), TelegramUpdatePortError>;
}

pub trait TelegramUpdateDeliveryPort: Send + Sync + 'static {
    fn send_message(&self, chat_id: &JsonValue, text: &str) -> Result<(), TelegramUpdatePortError>;
}

pub trait TelegramUpdateLifecyclePort: Send + Sync + 'static {
    fn handle_normal_turn(
        &self,
        request: &TelegramUpdateNormalTurnRequest,
    ) -> Result<(), TelegramUpdatePortError>;

    fn run_background_sync_for_chat(&self, chat_id: &str) -> Result<(), TelegramUpdatePortError>;

    fn execute_reply(
        &self,
        callback_slot: &str,
        args: &[JsonValue],
    ) -> Result<(), TelegramUpdatePortError>;

    fn wait_for_live_replies(
        &self,
        timeout: Option<Duration>,
    ) -> Result<bool, TelegramUpdatePortError>;
}

pub trait TelegramUpdateDiagnosticsPort: Send + Sync + 'static {
    fn record_failure(
        &self,
        kind: TelegramUpdateJobErrorKind,
    ) -> Result<(), TelegramUpdatePortError>;
}

#[derive(Clone)]
pub struct TelegramUpdateJobPorts {
    input: Arc<dyn TelegramUpdateInputPort>,
    bootstrap: Arc<dyn TelegramUpdateBootstrapPort>,
    operational: Arc<dyn TelegramUpdateOperationalPort>,
    command: Arc<dyn TelegramUpdateCommandPort>,
    delivery: Arc<dyn TelegramUpdateDeliveryPort>,
    lifecycle: Arc<dyn TelegramUpdateLifecyclePort>,
    diagnostics: Arc<dyn TelegramUpdateDiagnosticsPort>,
}

impl TelegramUpdateJobPorts {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        input: Arc<dyn TelegramUpdateInputPort>,
        bootstrap: Arc<dyn TelegramUpdateBootstrapPort>,
        operational: Arc<dyn TelegramUpdateOperationalPort>,
        command: Arc<dyn TelegramUpdateCommandPort>,
        delivery: Arc<dyn TelegramUpdateDeliveryPort>,
        lifecycle: Arc<dyn TelegramUpdateLifecyclePort>,
        diagnostics: Arc<dyn TelegramUpdateDiagnosticsPort>,
    ) -> Self {
        Self {
            input,
            bootstrap,
            operational,
            command,
            delivery,
            lifecycle,
            diagnostics,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramUpdateJobErrorKind {
    Configuration,
    InvalidUpdate,
    InvalidDispatchItem,
    InvalidLogicalTurn,
    TurnInputPlanner,
    TurnInputPlannerContract,
    WorkflowPlanner,
    WorkflowPlannerContract,
    InputContract,
    Bootstrap,
    OperationalTrigger,
    Command,
    Delivery,
    Lifecycle,
    Diagnostics,
}

impl TelegramUpdateJobErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Configuration => "configuration",
            Self::InvalidUpdate => "invalid_update",
            Self::InvalidDispatchItem => "invalid_dispatch_item",
            Self::InvalidLogicalTurn => "invalid_logical_turn",
            Self::TurnInputPlanner => "turn_input_planner",
            Self::TurnInputPlannerContract => "turn_input_planner_contract",
            Self::WorkflowPlanner => "workflow_planner",
            Self::WorkflowPlannerContract => "workflow_planner_contract",
            Self::InputContract => "input_contract",
            Self::Bootstrap => "bootstrap",
            Self::OperationalTrigger => "operational_trigger",
            Self::Command => "command",
            Self::Delivery => "delivery",
            Self::Lifecycle => "lifecycle",
            Self::Diagnostics => "diagnostics",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramUpdateJobError {
    kind: TelegramUpdateJobErrorKind,
}

impl TelegramUpdateJobError {
    pub fn kind(self) -> TelegramUpdateJobErrorKind {
        self.kind
    }

    fn new(kind: TelegramUpdateJobErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for TelegramUpdateJobError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TelegramUpdateJobErrorKind::Configuration => {
                "Telegram update job configuration is invalid."
            }
            TelegramUpdateJobErrorKind::InvalidUpdate => "Telegram update job input is invalid.",
            TelegramUpdateJobErrorKind::InvalidDispatchItem => {
                "Telegram update job dispatch item is invalid."
            }
            TelegramUpdateJobErrorKind::InvalidLogicalTurn => {
                "Telegram update job logical turn is invalid."
            }
            TelegramUpdateJobErrorKind::TurnInputPlanner => {
                "Telegram update input planning failed."
            }
            TelegramUpdateJobErrorKind::TurnInputPlannerContract => {
                "Telegram update input planner contract is invalid."
            }
            TelegramUpdateJobErrorKind::WorkflowPlanner => {
                "Telegram update workflow planning failed."
            }
            TelegramUpdateJobErrorKind::WorkflowPlannerContract => {
                "Telegram update workflow planner contract is invalid."
            }
            TelegramUpdateJobErrorKind::InputContract => {
                "Telegram prepared update input is invalid."
            }
            TelegramUpdateJobErrorKind::Bootstrap => "Telegram update bootstrap execution failed.",
            TelegramUpdateJobErrorKind::OperationalTrigger => {
                "Telegram update operational trigger execution failed."
            }
            TelegramUpdateJobErrorKind::Command => "Telegram update command execution failed.",
            TelegramUpdateJobErrorKind::Delivery => "Telegram update message delivery failed.",
            TelegramUpdateJobErrorKind::Lifecycle => "Telegram update lifecycle execution failed.",
            TelegramUpdateJobErrorKind::Diagnostics => {
                "Telegram update diagnostics execution failed."
            }
        })
    }
}

impl std::error::Error for TelegramUpdateJobError {}

#[derive(Clone)]
pub struct TelegramUpdateJob {
    config: TelegramUpdateJobConfig,
    ports: TelegramUpdateJobPorts,
    turn_input_planner: Arc<dyn TelegramTurnInputPlanner + Send + Sync>,
    workflow_planner: Arc<dyn TelegramWorkflowQueryPlanner + Send + Sync>,
}

impl TelegramUpdateJob {
    pub fn new(
        config: TelegramUpdateJobConfig,
        ports: TelegramUpdateJobPorts,
    ) -> Result<Self, TelegramUpdateJobError> {
        Self::with_planners(
            config,
            ports,
            Arc::new(DefaultTelegramTurnInputPlanner),
            Arc::new(DefaultTelegramWorkflowQueryPlanner),
        )
    }

    pub fn with_planners(
        config: TelegramUpdateJobConfig,
        ports: TelegramUpdateJobPorts,
        turn_input_planner: Arc<dyn TelegramTurnInputPlanner + Send + Sync>,
        workflow_planner: Arc<dyn TelegramWorkflowQueryPlanner + Send + Sync>,
    ) -> Result<Self, TelegramUpdateJobError> {
        validate_config(&config)?;
        Ok(Self {
            config,
            ports,
            turn_input_planner,
            workflow_planner,
        })
    }

    fn execute_update(
        &self,
        update: &JsonValue,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, TelegramUpdateJobError> {
        if !dispatch_item.is_object() {
            return Err(error(TelegramUpdateJobErrorKind::InvalidDispatchItem));
        }
        let update_object = update
            .as_object()
            .ok_or_else(|| error(TelegramUpdateJobErrorKind::InvalidUpdate))?;
        let Some(message_value) = update_object.get("message") else {
            return Ok(outcome("ignored", "missing_chat", false, true, None));
        };
        if message_value.is_null() {
            return Ok(outcome("ignored", "missing_chat", false, true, None));
        }
        let message = message_value
            .as_object()
            .ok_or_else(|| error(TelegramUpdateJobErrorKind::InvalidUpdate))?;
        let Some(chat_value) = message.get("chat") else {
            return Ok(outcome("ignored", "missing_chat", false, true, None));
        };
        if chat_value.is_null() {
            return Ok(outcome("ignored", "missing_chat", false, true, None));
        }
        let chat = chat_value
            .as_object()
            .ok_or_else(|| error(TelegramUpdateJobErrorKind::InvalidUpdate))?;
        let Some(chat_id) = chat.get("id").filter(|value| valid_chat_id(value)) else {
            return Ok(outcome("ignored", "missing_chat", false, true, None));
        };
        let from_user = optional_object(message.get("from"))?;
        let context = UpdateContext {
            chat_id: chat_id.clone(),
            chat: JsonValue::Object(chat.clone()),
            from_user,
            message: JsonValue::Object(message.clone()),
            candidate_raw_text: candidate_raw_text(message),
            telegram_message_id: optional_positive_i64(message.get("message_id"))?,
            reply_to_message: optional_object_value(message.get("reply_to_message"))?,
        };
        match self.execute_update_for_chat(&context) {
            Ok(value) => Ok(value),
            Err(failure) => self.report_update_failure(&context.chat_id, failure),
        }
    }

    fn execute_update_for_chat(
        &self,
        context: &UpdateContext,
    ) -> Result<JsonValue, TelegramUpdateJobError> {
        let initial_chat_title = self.plan_chat_title(&context.chat)?;
        let speech_attachments = self.plan_attachments(
            "speech_attachments_from_message",
            &context.message,
            Some((
                "include_audio_uploads",
                self.config.stt_include_audio_uploads,
            )),
        )?;
        let file_attachments = self.plan_attachments(
            "file_attachments_from_message",
            &context.message,
            Some((
                "include_speech_uploads",
                !self.config.speech_to_text_enabled,
            )),
        )?;
        if (!speech_attachments.is_empty() || !file_attachments.is_empty())
            && self
                .ports
                .bootstrap
                .handle_bootstrap(&TelegramUpdateBootstrapRequest {
                    chat_id: context.chat_id.clone(),
                    chat: context.chat.clone(),
                    from_user: context.from_user.clone(),
                    chat_title: initial_chat_title.clone(),
                    raw_text: context.candidate_raw_text.clone(),
                    command: None,
                    attachments_present: true,
                })
                .map_err(|_| error(TelegramUpdateJobErrorKind::Bootstrap))?
        {
            return Ok(outcome("handled", "owner_bootstrap", true, true, None));
        }

        let prepared = if self.config.speech_to_text_enabled && !speech_attachments.is_empty() {
            match self
                .ports
                .input
                .prepare_input(&TelegramUpdateInputRequest::new(
                    TelegramUpdateInputMode::SpeechToText,
                    context.message.clone(),
                    context.candidate_raw_text.clone(),
                    speech_attachments,
                )) {
                Ok(prepared) => validate_prepared_input(prepared)?,
                Err(input_error) => {
                    self.ports
                        .delivery
                        .send_message(&context.chat_id, input_error.kind().user_message())
                        .map_err(|_| error(TelegramUpdateJobErrorKind::Delivery))?;
                    return Ok(outcome(
                        "handled",
                        "input_failure_reported",
                        true,
                        false,
                        None,
                    ));
                }
            }
        } else {
            let mut downloadable =
                self.plan_attachments("music_attachments_from_message", &context.message, None)?;
            downloadable.extend(file_attachments);
            if downloadable.is_empty() {
                TelegramPreparedUpdateInput::new(context.candidate_raw_text.clone(), Vec::new())
            } else {
                match self
                    .ports
                    .input
                    .prepare_input(&TelegramUpdateInputRequest::new(
                        TelegramUpdateInputMode::DownloadAttachments,
                        context.message.clone(),
                        context.candidate_raw_text.clone(),
                        downloadable,
                    )) {
                    Ok(prepared) => validate_prepared_input(prepared)?,
                    Err(input_error) => {
                        self.ports
                            .delivery
                            .send_message(&context.chat_id, input_error.kind().user_message())
                            .map_err(|_| error(TelegramUpdateJobErrorKind::Delivery))?;
                        return Ok(outcome(
                            "handled",
                            "input_failure_reported",
                            true,
                            false,
                            None,
                        ));
                    }
                }
            }
        };

        if prepared
            .raw_text
            .as_deref()
            .is_none_or(|text| text.trim().is_empty())
            && prepared.attachments.is_empty()
        {
            return Ok(outcome("ignored", "empty_update", false, true, None));
        }

        let normalized = self.plan_normalized_text(&prepared)?;
        let entrypoint = self.plan_entrypoint(context, &prepared, &normalized)?;
        if self
            .ports
            .bootstrap
            .handle_bootstrap(&TelegramUpdateBootstrapRequest {
                chat_id: context.chat_id.clone(),
                chat: context.chat.clone(),
                from_user: context.from_user.clone(),
                chat_title: entrypoint.chat_title.clone(),
                raw_text: prepared.raw_text.clone(),
                command: entrypoint.command.clone(),
                attachments_present: !prepared.attachments.is_empty(),
            })
            .map_err(|_| error(TelegramUpdateJobErrorKind::Bootstrap))?
        {
            return Ok(outcome("handled", "owner_bootstrap", true, true, None));
        }

        let operational = TelegramUpdateOperationalRequest {
            chat_id: context.chat_id.clone(),
            chat: context.chat.clone(),
            from_user: context.from_user.clone(),
            chat_title: entrypoint.chat_title.clone(),
            raw_text: prepared.raw_text.clone().unwrap_or_default(),
            normalized_text: normalized.clone(),
            command: entrypoint.command.clone(),
            telegram_message_id: context.telegram_message_id,
            telegram_message_ids: Vec::new(),
            reply_to_message: context.reply_to_message.clone(),
            attachments: prepared.attachments.clone(),
            actor_identity: None,
            message: context.message.clone(),
        };
        if self
            .ports
            .operational
            .handle_operational_trigger(&operational)
            .map_err(|_| error(TelegramUpdateJobErrorKind::OperationalTrigger))?
        {
            return Ok(outcome("handled", "operational_trigger", true, true, None));
        }

        match entrypoint.action {
            EntrypointAction::Command { name, args } => {
                self.ports
                    .command
                    .execute_command(&TelegramUpdateCommandRequest {
                        chat_id: context.chat_id.clone(),
                        chat: context.chat.clone(),
                        from_user: context.from_user.clone(),
                        chat_title: entrypoint.chat_title,
                        command_name: name,
                        command_args: args,
                    })
                    .map_err(|_| error(TelegramUpdateJobErrorKind::Command))?;
                Ok(outcome("handled", "command", true, true, None))
            }
            EntrypointAction::EmptyHelp { message } => {
                self.ports
                    .delivery
                    .send_message(&context.chat_id, &message)
                    .map_err(|_| error(TelegramUpdateJobErrorKind::Delivery))?;
                Ok(outcome("handled", "empty_help", true, true, None))
            }
            EntrypointAction::NormalTurn => {
                self.ports
                    .lifecycle
                    .handle_normal_turn(&TelegramUpdateNormalTurnRequest {
                        chat_id: context.chat_id.clone(),
                        chat: context.chat.clone(),
                        from_user: context.from_user.clone(),
                        chat_title: entrypoint.chat_title,
                        text: normalized,
                        telegram_message_id: context.telegram_message_id,
                        telegram_message_ids: Vec::new(),
                        attachments: prepared.attachments,
                        actor_identity: None,
                        defer_reply: self.config.defer_normal_text_turn,
                    })
                    .map_err(|_| error(TelegramUpdateJobErrorKind::Lifecycle))?;
                Ok(outcome("handled", "normal_turn", true, true, None))
            }
        }
    }

    fn execute_logical_turn(
        &self,
        turn: &TelegramLogicalTurn,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, TelegramUpdateJobError> {
        if !dispatch_item.is_object() {
            return Err(error(TelegramUpdateJobErrorKind::InvalidDispatchItem));
        }
        if turn.text.chars().count() > MAX_TEXT_LENGTH
            || turn.text.trim().is_empty()
            || turn.actor_identity.trim().is_empty()
            || turn.actor_identity.chars().any(char::is_control)
            || turn.telegram_message_ids.len() > 128
            || turn.telegram_message_ids.iter().any(|value| *value <= 0)
            || turn.telegram_message_id.is_some_and(|value| value <= 0)
        {
            return Err(error(TelegramUpdateJobErrorKind::InvalidLogicalTurn));
        }
        let update = turn
            .update
            .as_object()
            .ok_or_else(|| error(TelegramUpdateJobErrorKind::InvalidLogicalTurn))?;
        let Some(message_value) = update.get("message") else {
            return Ok(outcome("ignored", "missing_chat", false, true, None));
        };
        let message = message_value
            .as_object()
            .ok_or_else(|| error(TelegramUpdateJobErrorKind::InvalidLogicalTurn))?;
        let Some(chat_value) = message.get("chat") else {
            return Ok(outcome("ignored", "missing_chat", false, true, None));
        };
        let chat = chat_value
            .as_object()
            .ok_or_else(|| error(TelegramUpdateJobErrorKind::InvalidLogicalTurn))?;
        let Some(chat_id) = chat.get("id").filter(|value| valid_chat_id(value)) else {
            return Ok(outcome("ignored", "missing_chat", false, true, None));
        };
        let from_user = optional_object(message.get("from"))?;
        let chat = JsonValue::Object(chat.clone());
        let message = JsonValue::Object(message.clone());
        let result = (|| {
            let chat_title = self.plan_chat_title(&chat)?;
            let reply_to_message = optional_object_value(
                message
                    .as_object()
                    .and_then(|message| message.get("reply_to_message")),
            )?;
            let request = TelegramUpdateOperationalRequest {
                chat_id: chat_id.clone(),
                chat: chat.clone(),
                from_user: from_user.clone(),
                chat_title: chat_title.clone(),
                raw_text: turn.text.clone(),
                normalized_text: turn.text.clone(),
                command: None,
                telegram_message_id: turn.telegram_message_id,
                telegram_message_ids: turn.telegram_message_ids.clone(),
                reply_to_message,
                attachments: Vec::new(),
                actor_identity: Some(turn.actor_identity.clone()),
                message,
            };
            if self
                .ports
                .operational
                .handle_operational_trigger(&request)
                .map_err(|_| error(TelegramUpdateJobErrorKind::OperationalTrigger))?
            {
                return Ok(outcome("handled", "operational_trigger", true, true, None));
            }
            self.ports
                .lifecycle
                .handle_normal_turn(&TelegramUpdateNormalTurnRequest {
                    chat_id: chat_id.clone(),
                    chat,
                    from_user,
                    chat_title,
                    text: turn.text.clone(),
                    telegram_message_id: turn.telegram_message_id,
                    telegram_message_ids: turn.telegram_message_ids.clone(),
                    attachments: Vec::new(),
                    actor_identity: Some(turn.actor_identity.clone()),
                    defer_reply: self.config.defer_normal_text_turn,
                })
                .map_err(|_| error(TelegramUpdateJobErrorKind::Lifecycle))?;
            Ok(outcome("handled", "normal_turn", true, true, None))
        })();
        match result {
            Ok(outcome) => Ok(outcome),
            Err(failure) => self.report_update_failure(chat_id, failure),
        }
    }

    fn report_update_failure(
        &self,
        chat_id: &JsonValue,
        failure: TelegramUpdateJobError,
    ) -> Result<JsonValue, TelegramUpdateJobError> {
        self.ports
            .diagnostics
            .record_failure(failure.kind())
            .map_err(|_| error(TelegramUpdateJobErrorKind::Diagnostics))?;
        self.ports
            .delivery
            .send_message(chat_id, UPDATE_FAILURE_MESSAGE)
            .map_err(|_| error(TelegramUpdateJobErrorKind::Delivery))?;
        Ok(outcome(
            "failed",
            "failure_reported",
            true,
            false,
            Some(failure.kind()),
        ))
    }

    fn record_lifecycle_failure<T>(
        &self,
        failure: TelegramUpdateJobError,
    ) -> Result<T, TelegramUpdateJobError> {
        self.ports
            .diagnostics
            .record_failure(failure.kind())
            .map_err(|_| error(TelegramUpdateJobErrorKind::Diagnostics))?;
        Err(failure)
    }

    fn plan_attachments(
        &self,
        kind: &str,
        message: &JsonValue,
        boolean: Option<(&str, bool)>,
    ) -> Result<Vec<JsonValue>, TelegramUpdateJobError> {
        let mut request = json!({"kind": kind, "message": message});
        if let (Some(request), Some((key, value))) = (request.as_object_mut(), boolean) {
            request.insert(key.to_string(), json!(value));
        }
        let planned = self
            .turn_input_planner
            .plan_json(&request)
            .map_err(|_| error(TelegramUpdateJobErrorKind::TurnInputPlanner))?;
        let object = validate_turn_input_base(planned, kind, "attachments")?;
        let attachments = object
            .get("attachments")
            .and_then(JsonValue::as_array)
            .filter(|items| {
                items.len() <= MAX_ATTACHMENTS && items.iter().all(JsonValue::is_object)
            })
            .cloned()
            .ok_or_else(|| error(TelegramUpdateJobErrorKind::TurnInputPlannerContract))?;
        Ok(attachments)
    }

    fn plan_normalized_text(
        &self,
        prepared: &TelegramPreparedUpdateInput,
    ) -> Result<String, TelegramUpdateJobError> {
        let planned = self
            .turn_input_planner
            .plan_json(&json!({
                "kind": "normalized_turn_text",
                "raw_text": prepared.raw_text,
                "username": self.config.username,
                "attachments": prepared.attachments,
            }))
            .map_err(|_| error(TelegramUpdateJobErrorKind::TurnInputPlanner))?;
        let object = validate_turn_input_base(planned, "normalized_turn_text", "text")?;
        required_bounded_string(
            &object,
            "text",
            MAX_TEXT_LENGTH,
            TelegramUpdateJobErrorKind::TurnInputPlannerContract,
        )
    }

    fn plan_chat_title(&self, chat: &JsonValue) -> Result<String, TelegramUpdateJobError> {
        let planned = self
            .workflow_planner
            .plan_json(&json!({"kind": "chat_title", "chat": chat}))
            .map_err(|_| error(TelegramUpdateJobErrorKind::WorkflowPlanner))?;
        let object = validate_workflow_base(planned, "chat_title", &["text"])?;
        let title = required_bounded_string(
            &object,
            "text",
            MAX_TITLE_LENGTH,
            TelegramUpdateJobErrorKind::WorkflowPlannerContract,
        )?;
        if title.trim().is_empty() {
            return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
        }
        Ok(title)
    }

    fn plan_entrypoint(
        &self,
        context: &UpdateContext,
        prepared: &TelegramPreparedUpdateInput,
        normalized: &str,
    ) -> Result<Entrypoint, TelegramUpdateJobError> {
        let planned = self
            .workflow_planner
            .plan_json(&json!({
                "kind": "message_entrypoint",
                "chat": context.chat,
                "raw_text": prepared.raw_text,
                "normalized_text": normalized,
                "username": self.config.username,
                "attachments": prepared.attachments,
                "attachments_present": !prepared.attachments.is_empty(),
            }))
            .map_err(|_| error(TelegramUpdateJobErrorKind::WorkflowPlanner))?;
        parse_entrypoint(
            planned,
            prepared.raw_text.as_deref(),
            normalized,
            !prepared.attachments.is_empty(),
        )
    }
}

impl TelegramSubmissionExecutionPort for TelegramUpdateJob {
    fn handle_update(
        &self,
        update: &JsonValue,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.execute_update(update, dispatch_item)
            .map_err(|failure| failure.to_string())
    }

    fn handle_logical_turn(
        &self,
        turn: &TelegramLogicalTurn,
        dispatch_item: &JsonValue,
    ) -> Result<JsonValue, String> {
        self.execute_logical_turn(turn, dispatch_item)
            .map_err(|failure| failure.to_string())
    }

    fn run_background_sync_for_chat(&self, chat_id: &str) -> Result<JsonValue, String> {
        if chat_id.trim().is_empty() || chat_id.len() > 128 || chat_id.chars().any(char::is_control)
        {
            return Err(error(TelegramUpdateJobErrorKind::Lifecycle).to_string());
        }
        self.ports
            .lifecycle
            .run_background_sync_for_chat(chat_id)
            .map_err(|_| error(TelegramUpdateJobErrorKind::Lifecycle))
            .or_else(|failure| self.record_lifecycle_failure(failure))
            .map_err(|failure| failure.to_string())?;
        Ok(outcome("handled", "background_sync", true, true, None))
    }

    fn execute_reply(&self, callback_slot: &str, args: &[JsonValue]) -> Result<JsonValue, String> {
        self.ports
            .lifecycle
            .execute_reply(callback_slot, args)
            .map_err(|_| error(TelegramUpdateJobErrorKind::Lifecycle))
            .or_else(|failure| self.record_lifecycle_failure(failure))
            .map_err(|failure| failure.to_string())?;
        Ok(outcome("handled", "reply", true, true, None))
    }

    fn wait_for_live_replies(&self, timeout: Option<Duration>) -> Result<bool, String> {
        self.ports
            .lifecycle
            .wait_for_live_replies(timeout)
            .map_err(|_| error(TelegramUpdateJobErrorKind::Lifecycle))
            .or_else(|failure| self.record_lifecycle_failure(failure))
            .map_err(|failure| failure.to_string())
    }
}

struct UpdateContext {
    chat_id: JsonValue,
    chat: JsonValue,
    from_user: JsonValue,
    message: JsonValue,
    candidate_raw_text: Option<String>,
    telegram_message_id: Option<i64>,
    reply_to_message: Option<JsonValue>,
}

struct Entrypoint {
    chat_title: String,
    command: Option<(String, String)>,
    action: EntrypointAction,
}

enum EntrypointAction {
    Command { name: String, args: String },
    EmptyHelp { message: String },
    NormalTurn,
}

fn validate_config(config: &TelegramUpdateJobConfig) -> Result<(), TelegramUpdateJobError> {
    let username = config.username.trim();
    if username.chars().count() > MAX_USERNAME_LENGTH || username.chars().any(char::is_control) {
        return Err(error(TelegramUpdateJobErrorKind::Configuration));
    }
    Ok(())
}

fn validate_prepared_input(
    prepared: TelegramPreparedUpdateInput,
) -> Result<TelegramPreparedUpdateInput, TelegramUpdateJobError> {
    if prepared
        .raw_text
        .as_ref()
        .is_some_and(|text| text.chars().count() > MAX_TEXT_LENGTH || text.contains('\0'))
        || prepared.attachments.len() > MAX_ATTACHMENTS
        || prepared.attachments.iter().any(|item| !item.is_object())
    {
        return Err(error(TelegramUpdateJobErrorKind::InputContract));
    }
    Ok(prepared)
}

fn validate_turn_input_base(
    planned: JsonValue,
    kind: &str,
    result_key: &str,
) -> Result<Map<String, JsonValue>, TelegramUpdateJobError> {
    let object = planned
        .as_object()
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::TurnInputPlannerContract))?;
    require_exact_keys(
        object,
        &[
            "migration_stage",
            "turn_input_contract",
            "kind",
            "transport",
            "rust_event_loop_required",
            "python_turn_input_allowed",
            result_key,
        ],
        TelegramUpdateJobErrorKind::TurnInputPlannerContract,
    )?;
    require_exact_string(
        object,
        "migration_stage",
        TURN_INPUT_STAGE,
        TelegramUpdateJobErrorKind::TurnInputPlannerContract,
    )?;
    require_exact_string(
        object,
        "turn_input_contract",
        TURN_INPUT_CONTRACT,
        TelegramUpdateJobErrorKind::TurnInputPlannerContract,
    )?;
    require_exact_string(
        object,
        "kind",
        kind,
        TelegramUpdateJobErrorKind::TurnInputPlannerContract,
    )?;
    validate_common_planner_flags(
        object,
        "python_turn_input_allowed",
        TelegramUpdateJobErrorKind::TurnInputPlannerContract,
    )?;
    Ok(object.clone())
}

fn validate_workflow_base(
    planned: JsonValue,
    kind: &str,
    result_keys: &[&str],
) -> Result<Map<String, JsonValue>, TelegramUpdateJobError> {
    let object = planned
        .as_object()
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?;
    let mut keys = vec![
        "migration_stage",
        "workflow_query_contract",
        "kind",
        "transport",
        "rust_event_loop_required",
        "python_workflow_query_allowed",
    ];
    keys.extend_from_slice(result_keys);
    require_exact_keys(
        object,
        keys.as_slice(),
        TelegramUpdateJobErrorKind::WorkflowPlannerContract,
    )?;
    require_exact_string(
        object,
        "migration_stage",
        WORKFLOW_QUERY_STAGE,
        TelegramUpdateJobErrorKind::WorkflowPlannerContract,
    )?;
    require_exact_string(
        object,
        "workflow_query_contract",
        WORKFLOW_QUERY_CONTRACT,
        TelegramUpdateJobErrorKind::WorkflowPlannerContract,
    )?;
    require_exact_string(
        object,
        "kind",
        kind,
        TelegramUpdateJobErrorKind::WorkflowPlannerContract,
    )?;
    validate_common_planner_flags(
        object,
        "python_workflow_query_allowed",
        TelegramUpdateJobErrorKind::WorkflowPlannerContract,
    )?;
    Ok(object.clone())
}

fn parse_entrypoint(
    planned: JsonValue,
    expected_raw: Option<&str>,
    expected_normalized: &str,
    expected_attachments_present: bool,
) -> Result<Entrypoint, TelegramUpdateJobError> {
    const RESULT_KEYS: &[&str] = &[
        "matched",
        "chat_title",
        "raw_text",
        "normalized_text",
        "attachments_present",
        "command_name",
        "command_args",
        "command",
        "query_kind",
        "query_ref",
        "workflow_query",
        "action_kind",
        "dispatch_command_name",
        "dispatch_command_args",
        "message_text",
    ];
    let object = validate_workflow_base(planned, "message_entrypoint", RESULT_KEYS)?;
    if object.get("matched").and_then(JsonValue::as_bool) != Some(true)
        || object.get("raw_text").and_then(JsonValue::as_str)
            != Some(expected_raw.unwrap_or_default().trim())
        || object.get("normalized_text").and_then(JsonValue::as_str) != Some(expected_normalized)
        || object
            .get("attachments_present")
            .and_then(JsonValue::as_bool)
            != Some(expected_attachments_present)
    {
        return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
    }
    let chat_title = required_bounded_string(
        &object,
        "chat_title",
        MAX_TITLE_LENGTH,
        TelegramUpdateJobErrorKind::WorkflowPlannerContract,
    )?;
    if chat_title.trim().is_empty() {
        return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
    }
    let command = parse_optional_pair(object.get("command"))?;
    let command_name = optional_string(object.get("command_name"))?;
    let command_args = object
        .get("command_args")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?
        .to_string();
    if command
        .as_ref()
        .map(|(name, args)| (name.as_str(), args.as_str()))
        != command_name
            .as_deref()
            .map(|name| (name, command_args.as_str()))
    {
        return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
    }
    let workflow_query = parse_optional_query(object.get("workflow_query"))?;
    let query_kind = optional_string(object.get("query_kind"))?;
    let query_ref = optional_string(object.get("query_ref"))?;
    if workflow_query
        .as_ref()
        .map(|(kind, reference)| (kind.as_str(), reference.as_deref()))
        != query_kind
            .as_deref()
            .map(|kind| (kind, query_ref.as_deref()))
        || expected_attachments_present && (command.is_some() || workflow_query.is_some())
    {
        return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
    }
    let action_kind = object
        .get("action_kind")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?;
    let dispatch_name = optional_string(object.get("dispatch_command_name"))?;
    let dispatch_args = object
        .get("dispatch_command_args")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?
        .to_string();
    let message_text = optional_string(object.get("message_text"))?;
    let action = match action_kind {
        "dispatch_command" => {
            let name = dispatch_name
                .filter(|name| !name.trim().is_empty() && name.chars().count() <= 128)
                .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?;
            if message_text.is_some() || dispatch_args.chars().count() > MAX_TEXT_LENGTH {
                return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
            }
            EntrypointAction::Command {
                name,
                args: dispatch_args,
            }
        }
        "send_empty_text_help" => {
            if dispatch_name.is_some()
                || !dispatch_args.is_empty()
                || message_text.as_deref() != Some(EMPTY_TEXT_HELP)
            {
                return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
            }
            EntrypointAction::EmptyHelp {
                message: EMPTY_TEXT_HELP.to_string(),
            }
        }
        "normal_text_turn" => {
            if dispatch_name.is_some() || !dispatch_args.is_empty() || message_text.is_some() {
                return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
            }
            EntrypointAction::NormalTurn
        }
        _ => return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract)),
    };
    Ok(Entrypoint {
        chat_title,
        command,
        action,
    })
}

fn validate_common_planner_flags(
    object: &Map<String, JsonValue>,
    python_flag: &str,
    kind: TelegramUpdateJobErrorKind,
) -> Result<(), TelegramUpdateJobError> {
    require_exact_string(object, "transport", "telegram", kind)?;
    if object
        .get("rust_event_loop_required")
        .and_then(JsonValue::as_bool)
        != Some(true)
        || object.get(python_flag).and_then(JsonValue::as_bool) != Some(false)
    {
        return Err(error(kind));
    }
    Ok(())
}

fn require_exact_keys(
    object: &Map<String, JsonValue>,
    expected: &[&str],
    kind: TelegramUpdateJobErrorKind,
) -> Result<(), TelegramUpdateJobError> {
    if object.len() != expected.len() || expected.iter().any(|key| !object.contains_key(*key)) {
        return Err(error(kind));
    }
    Ok(())
}

fn require_exact_string(
    object: &Map<String, JsonValue>,
    key: &str,
    expected: &str,
    kind: TelegramUpdateJobErrorKind,
) -> Result<(), TelegramUpdateJobError> {
    if object.get(key).and_then(JsonValue::as_str) != Some(expected) {
        return Err(error(kind));
    }
    Ok(())
}

fn required_bounded_string(
    object: &Map<String, JsonValue>,
    key: &str,
    max_chars: usize,
    kind: TelegramUpdateJobErrorKind,
) -> Result<String, TelegramUpdateJobError> {
    let value = object
        .get(key)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| error(kind))?;
    if value.chars().count() > max_chars || value.chars().any(|ch| ch == '\0') {
        return Err(error(kind));
    }
    Ok(value.to_string())
}

fn parse_optional_pair(
    value: Option<&JsonValue>,
) -> Result<Option<(String, String)>, TelegramUpdateJobError> {
    let Some(value) = value else {
        return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
    };
    if value.is_null() {
        return Ok(None);
    }
    let pair = value
        .as_array()
        .filter(|pair| pair.len() == 2)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?;
    let name = pair[0]
        .as_str()
        .filter(|name| !name.trim().is_empty() && name.chars().count() <= 128)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?;
    let args = pair[1]
        .as_str()
        .filter(|args| args.chars().count() <= MAX_TEXT_LENGTH)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?;
    Ok(Some((name.to_string(), args.to_string())))
}

fn parse_optional_query(
    value: Option<&JsonValue>,
) -> Result<Option<(String, Option<String>)>, TelegramUpdateJobError> {
    let Some(value) = value else {
        return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
    };
    if value.is_null() {
        return Ok(None);
    }
    let pair = value
        .as_array()
        .filter(|pair| pair.len() == 2)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?;
    let kind = pair[0]
        .as_str()
        .filter(|kind| !kind.trim().is_empty() && kind.chars().count() <= 128)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))?;
    let reference = match &pair[1] {
        JsonValue::Null => None,
        JsonValue::String(reference)
            if !reference.trim().is_empty() && reference.chars().count() <= MAX_TEXT_LENGTH =>
        {
            Some(reference.clone())
        }
        _ => return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract)),
    };
    Ok(Some((kind.to_string(), reference)))
}

fn optional_string(value: Option<&JsonValue>) -> Result<Option<String>, TelegramUpdateJobError> {
    let Some(value) = value else {
        return Err(error(TelegramUpdateJobErrorKind::WorkflowPlannerContract));
    };
    if value.is_null() {
        return Ok(None);
    }
    value
        .as_str()
        .map(str::to_string)
        .map(Some)
        .ok_or_else(|| error(TelegramUpdateJobErrorKind::WorkflowPlannerContract))
}

fn candidate_raw_text(message: &Map<String, JsonValue>) -> Option<String> {
    message
        .get("text")
        .and_then(JsonValue::as_str)
        .or_else(|| message.get("caption").and_then(JsonValue::as_str))
        .map(str::to_string)
}

fn optional_object(value: Option<&JsonValue>) -> Result<JsonValue, TelegramUpdateJobError> {
    match value {
        None | Some(JsonValue::Null) => Ok(json!({})),
        Some(JsonValue::Object(object)) => Ok(JsonValue::Object(object.clone())),
        Some(_) => Err(error(TelegramUpdateJobErrorKind::InvalidUpdate)),
    }
}

fn optional_object_value(
    value: Option<&JsonValue>,
) -> Result<Option<JsonValue>, TelegramUpdateJobError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(object)) => Ok(Some(JsonValue::Object(object.clone()))),
        Some(_) => Err(error(TelegramUpdateJobErrorKind::InvalidUpdate)),
    }
}

fn optional_positive_i64(value: Option<&JsonValue>) -> Result<Option<i64>, TelegramUpdateJobError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .filter(|value| *value > 0)
            .map(Some)
            .ok_or_else(|| error(TelegramUpdateJobErrorKind::InvalidUpdate)),
    }
}

fn valid_chat_id(value: &JsonValue) -> bool {
    value.as_i64().is_some()
        || value.as_u64().is_some()
        || value.as_str().is_some_and(|text| {
            !text.trim().is_empty() && text.len() <= 128 && !text.chars().any(char::is_control)
        })
}

fn outcome(
    state: &str,
    action: &str,
    handled: bool,
    ok: bool,
    failure_kind: Option<TelegramUpdateJobErrorKind>,
) -> JsonValue {
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "transport": "telegram",
        "update_state": state,
        "action": action,
        "handled": handled,
        "ok": ok,
        "failure_kind": failure_kind.map(TelegramUpdateJobErrorKind::code),
        "rust_submission_execution_required": true,
        "python_update_execution_allowed": false,
        "python_action_callback_allowed": false,
    })
}

fn error(kind: TelegramUpdateJobErrorKind) -> TelegramUpdateJobError {
    TelegramUpdateJobError::new(kind)
}

#[cfg(test)]
mod tests;
