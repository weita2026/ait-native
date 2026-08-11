use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::{
    TelegramPreparedUpdateInput, TelegramUpdateInputError, TelegramUpdateInputErrorKind,
    TelegramUpdateInputMode, TelegramUpdateInputPort, TelegramUpdateInputRequest,
};
use crate::event_loop::telegram_file_download::{
    agent_telegram_file_download_execute, TelegramFileDownloadExecutionError,
    TelegramFileDownloadExecutionErrorKind,
};
use crate::event_loop::telegram_stt_execution::{
    ExternalProgramTelegramSttExecutor, TelegramSttExecutionErrorKind, TelegramSttExecutor,
    TELEGRAM_STT_EXECUTION_CONTRACT,
};
use crate::event_loop::telegram_turn_inputs::{
    DefaultTelegramTurnInputPlanner, TelegramTurnInputPlanner,
};
use crate::transport_config::{TelegramSttMode, TelegramWorkerConfig};

const FILE_CONTRACT: &str = "ait_agent_core.event_loop.TelegramFileDownloadExecution.v1";
const FILE_STAGE: &str = "rust_agent_telegram_file_download_execution";
const STT_STAGE: &str = "rust_agent_telegram_external_stt_execution";
const TURN_INPUT_CONTRACT: &str = "ait_agent_core.event_loop.TelegramTurnInput.v1";
const TURN_INPUT_STAGE: &str = "rust_agent_telegram_turn_input";
const MAX_ATTACHMENTS: usize = 32;
const MAX_CONTEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 512 * 1024;
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_CONFIG_BYTES: usize = 16 * 1024;
const MAX_TOKEN_BYTES: usize = 4 * 1024;
const MAX_ATTACHMENT_BYTES: u64 = 50 * 1024 * 1024;

pub trait TelegramUpdateFileDownloadExecutor: Send + Sync + 'static {
    fn execute_file_download(
        &self,
        request: &JsonValue,
    ) -> Result<JsonValue, TelegramFileDownloadExecutionError>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramUpdateFileDownloadExecutor;

impl TelegramUpdateFileDownloadExecutor for DefaultTelegramUpdateFileDownloadExecutor {
    fn execute_file_download(
        &self,
        request: &JsonValue,
    ) -> Result<JsonValue, TelegramFileDownloadExecutionError> {
        agent_telegram_file_download_execute(request).map(|execution| execution.into_metadata())
    }
}

pub struct NativeTelegramUpdateInputPort<
    F = DefaultTelegramUpdateFileDownloadExecutor,
    S = ExternalProgramTelegramSttExecutor,
> {
    config: TelegramInputExecutionConfig,
    file_download: F,
    stt: Option<S>,
}

impl
    NativeTelegramUpdateInputPort<
        DefaultTelegramUpdateFileDownloadExecutor,
        ExternalProgramTelegramSttExecutor,
    >
{
    pub fn from_config(config: &TelegramWorkerConfig) -> Result<Self, String> {
        let execution_config = TelegramInputExecutionConfig::from_worker(config)?;
        let stt = match execution_config.stt_program.as_ref() {
            Some(program) => Some(
                ExternalProgramTelegramSttExecutor::new(
                    program,
                    Duration::from_secs_f64(execution_config.stt_timeout_seconds),
                )
                .map_err(|_| input_configuration_error())?,
            ),
            None => None,
        };
        Ok(Self {
            config: execution_config,
            file_download: DefaultTelegramUpdateFileDownloadExecutor,
            stt,
        })
    }
}

impl<F, S> NativeTelegramUpdateInputPort<F, S> {
    pub fn with_executors(
        config: &TelegramWorkerConfig,
        file_download: F,
        stt: Option<S>,
    ) -> Result<Self, String> {
        Ok(Self {
            config: TelegramInputExecutionConfig::from_worker(config)?,
            file_download,
            stt,
        })
    }
}

impl<F, S> fmt::Debug for NativeTelegramUpdateInputPort<F, S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramUpdateInputPort")
            .field("stt_enabled", &self.config.stt_enabled)
            .field("stt_program_configured", &self.stt.is_some())
            .field("stt_timeout_seconds", &self.config.stt_timeout_seconds)
            .field("cache_root_exposed", &false)
            .field("bot_token_exposed", &false)
            .field("stt_program_path_exposed", &false)
            .field("stt_model_exposed", &false)
            .finish()
    }
}

impl<F, S> TelegramUpdateInputPort for NativeTelegramUpdateInputPort<F, S>
where
    F: TelegramUpdateFileDownloadExecutor,
    S: TelegramSttExecutor,
{
    fn prepare_input(
        &self,
        request: &TelegramUpdateInputRequest,
    ) -> Result<TelegramPreparedUpdateInput, TelegramUpdateInputError> {
        validate_input_request(request)?;
        match request.mode() {
            TelegramUpdateInputMode::DownloadAttachments => {
                let attachments = request
                    .attachments()
                    .iter()
                    .map(|attachment| {
                        self.download_attachment(request.message(), attachment, false)
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(TelegramPreparedUpdateInput::new(
                    request.candidate_raw_text().map(str::to_string),
                    attachments,
                ))
            }
            TelegramUpdateInputMode::SpeechToText => {
                if !self.config.stt_enabled {
                    return Err(input_error(
                        TelegramUpdateInputErrorKind::SpeechToTextNotEnabled,
                    ));
                }
                let Some(first) = request.attachments().first() else {
                    return Err(input_error(
                        TelegramUpdateInputErrorKind::SpeechAttachmentMissing,
                    ));
                };
                let resolved = self.download_attachment(request.message(), first, true)?;
                let Some(stt) = self.stt.as_ref() else {
                    return Err(input_error(
                        TelegramUpdateInputErrorKind::SpeechBackendUnavailable,
                    ));
                };
                let local_path = resolved
                    .get("local_path")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| {
                        input_error(TelegramUpdateInputErrorKind::AttachmentDownloadFailed)
                    })?;
                let execution = stt
                    .execute_stt(&json!({
                        "operation": "transcribe",
                        "local_path": local_path,
                        "model": self.config.stt_model,
                        "device": self.config.stt_device,
                        "compute_type": self.config.stt_compute_type,
                        "language": self.config.stt_language,
                    }))
                    .map_err(|failure| map_stt_error(failure.kind()))?;
                let transcript = validate_stt_execution(&execution)?;
                let text = speech_turn_text(request.message(), &transcript)?;
                let mut attachments = request.attachments().to_vec();
                attachments[0] = resolved;
                Ok(TelegramPreparedUpdateInput::new(Some(text), attachments))
            }
        }
    }
}

impl<F, S> NativeTelegramUpdateInputPort<F, S>
where
    F: TelegramUpdateFileDownloadExecutor,
{
    fn download_attachment(
        &self,
        message: &JsonValue,
        attachment: &JsonValue,
        speech: bool,
    ) -> Result<JsonValue, TelegramUpdateInputError> {
        if clean_text(attachment.get("telegram_file_id")).is_none() {
            return Err(input_error(if speech {
                TelegramUpdateInputErrorKind::SpeechFileIdMissing
            } else {
                TelegramUpdateInputErrorKind::AttachmentFileIdMissing
            }));
        }
        let mut request = json!({
            "message": message,
            "attachment": attachment,
            "cache_root": self.config.cache_root.to_string_lossy(),
            "bot_token": self.config.bot_token,
            "request_timeout_seconds": self.config.request_timeout_seconds,
        });
        if let Some(value) = self.config.api_base_url.as_ref() {
            request["base_url"] = json!(value);
        }
        if let Some(value) = self.config.file_base_url.as_ref() {
            request["file_base_url"] = json!(value);
        }
        let execution = self
            .file_download
            .execute_file_download(&request)
            .map_err(|failure| map_file_error(failure.kind(), speech))?;
        validate_file_execution(&execution, &self.config.cache_root)
    }
}

#[derive(Clone)]
struct TelegramInputExecutionConfig {
    cache_root: PathBuf,
    bot_token: String,
    request_timeout_seconds: Option<f64>,
    api_base_url: Option<String>,
    file_base_url: Option<String>,
    stt_enabled: bool,
    stt_program: Option<PathBuf>,
    stt_timeout_seconds: f64,
    stt_model: String,
    stt_device: String,
    stt_compute_type: Option<String>,
    stt_language: Option<String>,
}

impl TelegramInputExecutionConfig {
    fn from_worker(config: &TelegramWorkerConfig) -> Result<Self, String> {
        let state_path = PathBuf::from(&config.shared.paths.sync_state_path);
        let cache_root = state_path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .map(|value| value.join("telegram-downloads"))
            .ok_or_else(input_configuration_error)?;
        let value = Self {
            cache_root,
            bot_token: config.token.expose().to_string(),
            request_timeout_seconds: config.shared.request_timeout_seconds,
            api_base_url: None,
            file_base_url: None,
            stt_enabled: config.stt_mode == TelegramSttMode::LocalStt,
            stt_program: config.stt_program.clone(),
            stt_timeout_seconds: config.stt_timeout_seconds,
            stt_model: config.stt_model.clone(),
            stt_device: config.stt_device.clone(),
            stt_compute_type: config.stt_compute_type.clone(),
            stt_language: config.stt_language.clone(),
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), String> {
        if !clean_absolute_path(&self.cache_root)
            || self.cache_root.to_string_lossy().len() > MAX_PATH_BYTES
            || !valid_text(&self.bot_token, MAX_TOKEN_BYTES)
            || self
                .request_timeout_seconds
                .is_some_and(|value| !value.is_finite() || value <= 0.0 || value > 86_400.0)
            || !self.stt_timeout_seconds.is_finite()
            || !(0.01..=3_600.0).contains(&self.stt_timeout_seconds)
            || !valid_text(&self.stt_model, MAX_CONFIG_BYTES)
            || !valid_text(&self.stt_device, MAX_CONFIG_BYTES)
            || self
                .stt_compute_type
                .as_deref()
                .is_some_and(|value| !valid_text(value, MAX_CONFIG_BYTES))
            || self
                .stt_language
                .as_deref()
                .is_some_and(|value| !valid_text(value, MAX_CONFIG_BYTES))
        {
            return Err(input_configuration_error());
        }
        Ok(())
    }
}

fn validate_input_request(
    request: &TelegramUpdateInputRequest,
) -> Result<(), TelegramUpdateInputError> {
    if !request.message().is_object()
        || request.message().to_string().len() > MAX_CONTEXT_BYTES
        || request.attachments().is_empty()
        || request.attachments().len() > MAX_ATTACHMENTS
        || request
            .attachments()
            .iter()
            .any(|value| !value.is_object() || value.to_string().len() > MAX_CONTEXT_BYTES)
        || request.candidate_raw_text().is_some_and(|value| {
            value.len() > MAX_TEXT_BYTES || value.contains('\0') || value.contains('\r')
        })
    {
        return Err(input_error(match request.mode() {
            TelegramUpdateInputMode::SpeechToText => {
                TelegramUpdateInputErrorKind::SpeechAttachmentMissing
            }
            TelegramUpdateInputMode::DownloadAttachments => {
                TelegramUpdateInputErrorKind::AttachmentDownloadFailed
            }
        }));
    }
    Ok(())
}

fn validate_file_execution(
    execution: &JsonValue,
    cache_root: &Path,
) -> Result<JsonValue, TelegramUpdateInputError> {
    let object = execution.as_object().ok_or_else(download_contract_error)?;
    if text(object, "contract") != Some(FILE_CONTRACT)
        || text(object, "migration_stage") != Some(FILE_STAGE)
        || text(object, "stage") != Some("execute")
        || text(object, "transport") != Some("telegram")
        || text(object, "file_download_state") != Some("completed")
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || object
            .get("cache_hit")
            .and_then(JsonValue::as_bool)
            .is_none()
        || object
            .get("downloaded")
            .and_then(JsonValue::as_bool)
            .is_none()
        || !false_flags(
            object,
            &[
                "python_file_download_allowed",
                "python_cache_io_allowed",
                "python_telegram_api_allowed",
                "downloaded_bytes_exposed",
                "diagnostic_local_path_exposed",
                "diagnostic_telegram_path_exposed",
            ],
        )
    {
        return Err(download_contract_error());
    }
    let local_path_text = object
        .get("local_path")
        .and_then(JsonValue::as_str)
        .filter(|value| value.len() <= MAX_PATH_BYTES)
        .ok_or_else(download_contract_error)?;
    let local_path = PathBuf::from(local_path_text);
    if !clean_absolute_path(&local_path) || !local_path.starts_with(cache_root) {
        return Err(input_error(
            TelegramUpdateInputErrorKind::AttachmentHostUnsupported,
        ));
    }
    let metadata = fs::symlink_metadata(&local_path).map_err(|_| download_contract_error())?;
    let byte_count = object
        .get("byte_count")
        .and_then(JsonValue::as_u64)
        .ok_or_else(download_contract_error)?;
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_file()
        || !(1..=MAX_ATTACHMENT_BYTES).contains(&metadata.len())
        || metadata.len() != byte_count
    {
        return Err(download_contract_error());
    }
    let attachment = object
        .get("attachment")
        .and_then(JsonValue::as_object)
        .ok_or_else(download_contract_error)?;
    if attachment.get("local_path").and_then(JsonValue::as_str) != Some(local_path_text) {
        return Err(download_contract_error());
    }
    Ok(JsonValue::Object(attachment.clone()))
}

fn validate_stt_execution(execution: &JsonValue) -> Result<String, TelegramUpdateInputError> {
    let object = execution
        .as_object()
        .ok_or_else(transcription_contract_error)?;
    if text(object, "contract") != Some(TELEGRAM_STT_EXECUTION_CONTRACT)
        || text(object, "migration_stage") != Some(STT_STAGE)
        || text(object, "stage") != Some("execute")
        || text(object, "transport") != Some("telegram")
        || text(object, "stt_state") != Some("completed")
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || !false_flags(
            object,
            &[
                "python_stt_allowed",
                "python_runtime_allowed",
                "shell_execution_allowed",
                "inherited_environment_allowed",
                "request_payload_exposed",
                "response_payload_exposed",
                "audio_path_exposed",
                "program_path_exposed",
                "stderr_exposed",
                "downstream_error_exposed",
            ],
        )
    {
        return Err(transcription_contract_error());
    }
    let transcript = object
        .get("transcript")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .ok_or_else(transcription_contract_error)?;
    if transcript.is_empty() {
        return Err(input_error(TelegramUpdateInputErrorKind::SpeechEmpty));
    }
    if transcript.len() > MAX_TEXT_BYTES || transcript.contains('\0') {
        return Err(transcription_contract_error());
    }
    Ok(transcript.to_string())
}

fn speech_turn_text(
    message: &JsonValue,
    transcript: &str,
) -> Result<String, TelegramUpdateInputError> {
    let planned = DefaultTelegramTurnInputPlanner
        .plan_json(&json!({
            "kind": "speech_turn_text",
            "caption": message.get("caption"),
            "transcript": transcript,
        }))
        .map_err(|_| transcription_contract_error())?;
    let object = planned
        .as_object()
        .ok_or_else(transcription_contract_error)?;
    if text(object, "turn_input_contract") != Some(TURN_INPUT_CONTRACT)
        || text(object, "migration_stage") != Some(TURN_INPUT_STAGE)
        || text(object, "kind") != Some("speech_turn_text")
        || text(object, "transport") != Some("telegram")
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_turn_input_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(transcription_contract_error());
    }
    object
        .get("text")
        .and_then(JsonValue::as_str)
        .filter(|value| valid_text(value, MAX_TEXT_BYTES))
        .map(str::to_string)
        .ok_or_else(transcription_contract_error)
}

fn map_file_error(
    kind: TelegramFileDownloadExecutionErrorKind,
    speech: bool,
) -> TelegramUpdateInputError {
    use TelegramFileDownloadExecutionErrorKind::*;
    input_error(match kind {
        MissingFileId if speech => TelegramUpdateInputErrorKind::SpeechFileIdMissing,
        MissingFileId => TelegramUpdateInputErrorKind::AttachmentFileIdMissing,
        MissingFilePath => TelegramUpdateInputErrorKind::AttachmentPathMissing,
        CachePath | CacheInspect => TelegramUpdateInputErrorKind::AttachmentHostUnsupported,
        InvalidRequest | Planner | PlannerContract | TelegramFileInfo | Download | PayloadSize
        | Publish | ResultContract => TelegramUpdateInputErrorKind::AttachmentDownloadFailed,
    })
}

fn map_stt_error(kind: TelegramSttExecutionErrorKind) -> TelegramUpdateInputError {
    use TelegramSttExecutionErrorKind::*;
    input_error(match kind {
        Configuration | Unavailable => TelegramUpdateInputErrorKind::SpeechBackendUnavailable,
        Timeout => TelegramUpdateInputErrorKind::SpeechTimeout,
        Empty => TelegramUpdateInputErrorKind::SpeechEmpty,
        InvalidRequest | Io | Exit | OutputLimit | Contract | Transcription => {
            TelegramUpdateInputErrorKind::SpeechTranscriptionFailed
        }
    })
}

fn clean_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        && !path.to_string_lossy().chars().any(char::is_control)
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
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

fn false_flags(object: &Map<String, JsonValue>, keys: &[&str]) -> bool {
    keys.iter()
        .all(|key| object.get(*key).and_then(JsonValue::as_bool) == Some(false))
}

fn text<'a>(object: &'a Map<String, JsonValue>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(JsonValue::as_str)
}

fn input_error(kind: TelegramUpdateInputErrorKind) -> TelegramUpdateInputError {
    TelegramUpdateInputError::new(kind)
}

fn download_contract_error() -> TelegramUpdateInputError {
    input_error(TelegramUpdateInputErrorKind::AttachmentDownloadFailed)
}

fn transcription_contract_error() -> TelegramUpdateInputError {
    input_error(TelegramUpdateInputErrorKind::SpeechTranscriptionFailed)
}

fn input_configuration_error() -> String {
    "Telegram native input configuration is invalid.".to_string()
}

#[cfg(test)]
#[path = "native_input_adapters/tests.rs"]
mod tests;
