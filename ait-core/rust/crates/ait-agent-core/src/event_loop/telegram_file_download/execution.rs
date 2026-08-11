use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use super::planning::{DefaultTelegramFileDownloadPlanner, TelegramFileDownloadPlanner};
use crate::event_loop::telegram_api_method::agent_telegram_api_execute;
use crate::file_store::{agent_file_store_execute_json, AGENT_FILE_STORE_CONTRACT};

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramFileDownloadExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_file_download_execution";
const PLANNING_KIND: &str = "telegram_file_download";
const API_CONTRACT: &str = "ait_agent_core.event_loop.TelegramApiTransportExecution.v1";
const API_MIGRATION_STAGE: &str = "rust_agent_telegram_transport_execution";
const MAX_ATTACHMENT_BYTES: usize = 50 * 1024 * 1024;
const MAX_CONTEXT_BYTES: usize = 2 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 16 * 1024;
const MAX_TELEGRAM_PATH_BYTES: usize = 4 * 1024;
const MAX_TOKEN_BYTES: usize = 4 * 1024;
const MAX_URL_BYTES: usize = 4 * 1024;

static PATH_LOCKS: OnceLock<PathLockRegistry> = OnceLock::new();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramFileDownloadExecutionErrorKind {
    InvalidRequest,
    MissingFileId,
    Planner,
    PlannerContract,
    TelegramFileInfo,
    MissingFilePath,
    CachePath,
    CacheInspect,
    Download,
    PayloadSize,
    Publish,
    ResultContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TelegramFileDownloadExecutionError {
    kind: TelegramFileDownloadExecutionErrorKind,
}

impl TelegramFileDownloadExecutionError {
    pub fn new(kind: TelegramFileDownloadExecutionErrorKind) -> Self {
        Self { kind }
    }

    pub fn kind(self) -> TelegramFileDownloadExecutionErrorKind {
        self.kind
    }
}

impl fmt::Display for TelegramFileDownloadExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Telegram file download execution failed.")
    }
}

impl std::error::Error for TelegramFileDownloadExecutionError {}

#[derive(Clone, PartialEq)]
pub struct TelegramFileDownloadExecution {
    metadata: JsonValue,
}

impl TelegramFileDownloadExecution {
    pub fn metadata(&self) -> &JsonValue {
        &self.metadata
    }

    pub fn into_metadata(self) -> JsonValue {
        self.metadata
    }
}

impl fmt::Debug for TelegramFileDownloadExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramFileDownloadExecution")
            .field("state", &self.metadata.get("file_download_state"))
            .field("cache_hit", &self.metadata.get("cache_hit"))
            .field("downloaded", &self.metadata.get("downloaded"))
            .field("byte_count", &self.metadata.get("byte_count"))
            .field("attachment_exposed", &false)
            .field("local_path_exposed", &false)
            .finish()
    }
}

#[derive(Clone, PartialEq)]
pub struct TelegramFileDownloadApiExecution {
    metadata: JsonValue,
    downloaded_bytes: Option<Vec<u8>>,
}

impl TelegramFileDownloadApiExecution {
    pub fn new(metadata: JsonValue, downloaded_bytes: Option<Vec<u8>>) -> Self {
        Self {
            metadata,
            downloaded_bytes,
        }
    }

    fn into_parts(self) -> (JsonValue, Option<Vec<u8>>) {
        (self.metadata, self.downloaded_bytes)
    }
}

impl fmt::Debug for TelegramFileDownloadApiExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramFileDownloadApiExecution")
            .field("operation", &self.metadata.get("operation"))
            .field(
                "downloaded_byte_count",
                &self.downloaded_bytes.as_ref().map(Vec::len),
            )
            .finish()
    }
}

pub trait TelegramFileDownloadApiPort: Send + Sync + 'static {
    fn execute_api(&self, request: &JsonValue) -> Result<TelegramFileDownloadApiExecution, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTelegramFileDownloadApiPort;

impl TelegramFileDownloadApiPort for NativeTelegramFileDownloadApiPort {
    fn execute_api(&self, request: &JsonValue) -> Result<TelegramFileDownloadApiExecution, String> {
        agent_telegram_api_execute(request)
            .map(|execution| {
                let (metadata, bytes) = execution.into_parts();
                TelegramFileDownloadApiExecution::new(metadata, bytes)
            })
            .map_err(|_| "Telegram file download API execution failed.".to_string())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TelegramFileCacheState {
    Missing,
    Hit { byte_count: usize },
}

pub trait TelegramFileDownloadStorePort: Send + Sync + 'static {
    fn inspect(
        &self,
        cache_root: &Path,
        local_path: &Path,
    ) -> Result<TelegramFileCacheState, String>;

    fn publish(
        &self,
        cache_root: &Path,
        local_path: &Path,
        payload: &[u8],
    ) -> Result<usize, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTelegramFileDownloadStorePort;

impl TelegramFileDownloadStorePort for NativeTelegramFileDownloadStorePort {
    fn inspect(
        &self,
        cache_root: &Path,
        local_path: &Path,
    ) -> Result<TelegramFileCacheState, String> {
        validate_native_cache_path(cache_root, local_path)?;
        let metadata = match fs::symlink_metadata(local_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TelegramFileCacheState::Missing)
            }
            Err(_) => return Err("Telegram attachment cache inspection failed.".to_string()),
        };
        if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
            return Err("Telegram attachment cache target is invalid.".to_string());
        }
        let byte_count = usize::try_from(metadata.len())
            .map_err(|_| "Telegram attachment cache target is too large.".to_string())?;
        if !(1..=MAX_ATTACHMENT_BYTES).contains(&byte_count) {
            return Err("Telegram attachment cache target has an invalid size.".to_string());
        }
        let response = agent_file_store_execute_json(
            &json!({"path": local_path.to_string_lossy(), "operation": "inspect"}),
            None,
        )
        .map_err(|_| "Telegram attachment cache inspection failed.".to_string())?;
        validate_file_store_inspect(&response, local_path)?;
        Ok(TelegramFileCacheState::Hit { byte_count })
    }

    fn publish(
        &self,
        cache_root: &Path,
        local_path: &Path,
        payload: &[u8],
    ) -> Result<usize, String> {
        validate_native_cache_path(cache_root, local_path)?;
        if !(1..=MAX_ATTACHMENT_BYTES).contains(&payload.len()) {
            return Err("Telegram attachment payload has an invalid size.".to_string());
        }
        let response = agent_file_store_execute_json(
            &json!({"path": local_path.to_string_lossy(), "operation": "publish"}),
            Some(payload),
        )
        .map_err(|_| "Telegram attachment cache publication failed.".to_string())?;
        let byte_count = validate_file_store_publish(&response, local_path, payload.len())?;
        validate_native_cache_path(cache_root, local_path)?;
        let metadata = fs::symlink_metadata(local_path)
            .map_err(|_| "Telegram attachment cache publication failed.".to_string())?;
        if metadata.file_type().is_symlink()
            || !metadata.file_type().is_file()
            || usize::try_from(metadata.len()).ok() != Some(byte_count)
        {
            return Err("Telegram attachment cache publication failed.".to_string());
        }
        Ok(byte_count)
    }
}

pub fn agent_telegram_file_download_execute(
    request: &JsonValue,
) -> Result<TelegramFileDownloadExecution, TelegramFileDownloadExecutionError> {
    execute_with_telegram_file_download_ports(
        &DefaultTelegramFileDownloadPlanner,
        &NativeTelegramFileDownloadApiPort,
        &NativeTelegramFileDownloadStorePort,
        request,
    )
}

pub fn execute_with_telegram_file_download_ports<P, A, S>(
    planner: &P,
    api: &A,
    store: &S,
    request: &JsonValue,
) -> Result<TelegramFileDownloadExecution, TelegramFileDownloadExecutionError>
where
    P: TelegramFileDownloadPlanner + ?Sized,
    A: TelegramFileDownloadApiPort + ?Sized,
    S: TelegramFileDownloadStorePort + ?Sized,
{
    let input = ValidatedInput::parse(request)?;
    let request_plan = planner
        .plan_json(&json!({
            "stage": "request",
            "message": input.message,
            "attachment": input.attachment,
            "cache_root": input.cache_root_text,
        }))
        .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::Planner))?;
    let Some(file_id) = input.file_id.as_deref() else {
        validate_missing_file_id_plan(&request_plan)?;
        return Err(error(TelegramFileDownloadExecutionErrorKind::MissingFileId));
    };
    let request_stage = validate_request_plan(&request_plan, &input, file_id)?;

    let get_file_execution = api
        .execute_api(&input.api_request("get_file", "file_id", file_id))
        .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::TelegramFileInfo))?;
    let (get_file_metadata, get_file_bytes) = get_file_execution.into_parts();
    if get_file_bytes.is_some() {
        return Err(error(
            TelegramFileDownloadExecutionErrorKind::TelegramFileInfo,
        ));
    }
    let file_info = validate_get_file_metadata(&get_file_metadata, file_id)?;
    let telegram_file_path = validate_telegram_file_path(&file_info)?;

    let file_info_plan = planner
        .plan_json(&json!({
            "stage": "file_info",
            "execution_request": request_stage,
            "file_info": file_info,
        }))
        .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::Planner))?;
    let file_stage =
        validate_file_info_plan(&file_info_plan, &input, file_id, &telegram_file_path)?;
    let local_path_text = required_clean_text(file_stage, "local_path")
        .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::PlannerContract))?;
    let local_path = PathBuf::from(&local_path_text);
    validate_cache_path(&input.cache_root, &local_path)
        .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::CachePath))?;

    let path_lock = path_locks().lock_for(&local_path)?;
    let _guard = lock_unpoisoned(&path_lock);
    let cache_state = store
        .inspect(&input.cache_root, &local_path)
        .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::CacheInspect))?;
    let (cache_hit, cached_byte_count) = match cache_state {
        TelegramFileCacheState::Missing => (false, None),
        TelegramFileCacheState::Hit { byte_count }
            if (1..=MAX_ATTACHMENT_BYTES).contains(&byte_count) =>
        {
            (true, Some(byte_count))
        }
        TelegramFileCacheState::Hit { .. } => {
            return Err(error(TelegramFileDownloadExecutionErrorKind::PayloadSize))
        }
    };
    let cache_plan = planner
        .plan_json(&json!({
            "stage": "cache",
            "execution_request": file_stage,
            "local_path_exists": cache_hit,
        }))
        .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::Planner))?;
    let cache_stage = validate_cache_plan(
        &cache_plan,
        &telegram_file_path,
        &local_path_text,
        cache_hit,
    )?;

    let (downloaded, byte_count, operation_results) = if cache_hit {
        (false, cached_byte_count.unwrap_or_default(), Vec::new())
    } else {
        let download_execution = api
            .execute_api(&input.api_request("download_file", "file_path", &telegram_file_path))
            .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::Download))?;
        let (download_metadata, payload) = download_execution.into_parts();
        let payload =
            payload.ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::Download))?;
        validate_download_metadata(&download_metadata, payload.len())?;
        if !(1..=MAX_ATTACHMENT_BYTES).contains(&payload.len()) {
            return Err(error(TelegramFileDownloadExecutionErrorKind::PayloadSize));
        }
        let byte_count = store
            .publish(&input.cache_root, &local_path, &payload)
            .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::Publish))?;
        if byte_count != payload.len() {
            return Err(error(TelegramFileDownloadExecutionErrorKind::Publish));
        }
        (
            true,
            byte_count,
            vec![json!({
                "index": 0,
                "kind": "download_file_bytes",
                "ok": true,
                "telegram_file_path": telegram_file_path,
                "local_path": local_path_text,
                "downloaded": true,
            })],
        )
    };

    let result_plan = planner
        .plan_json(&json!({
            "stage": "result",
            "execution_request": cache_stage,
            "operation_results": operation_results,
        }))
        .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::Planner))?;
    let attachment = validate_result_plan(
        &result_plan,
        &telegram_file_path,
        &local_path_text,
        cache_hit,
        downloaded,
    )?;
    Ok(TelegramFileDownloadExecution {
        metadata: json!({
            "contract": CONTRACT,
            "migration_stage": MIGRATION_STAGE,
            "stage": "execute",
            "transport": "telegram",
            "file_download_state": "completed",
            "ok": true,
            "completed": true,
            "attachment": attachment,
            "local_path": local_path_text,
            "cache_hit": cache_hit,
            "downloaded": downloaded,
            "byte_count": byte_count,
            "python_file_download_allowed": false,
            "python_cache_io_allowed": false,
            "python_telegram_api_allowed": false,
            "downloaded_bytes_exposed": false,
            "diagnostic_local_path_exposed": false,
            "diagnostic_telegram_path_exposed": false,
        }),
    })
}

struct ValidatedInput {
    message: JsonValue,
    attachment: JsonValue,
    cache_root: PathBuf,
    cache_root_text: String,
    file_id: Option<String>,
    bot_token: Option<String>,
    request_timeout_seconds: Option<f64>,
    base_url: Option<String>,
    file_base_url: Option<String>,
}

impl ValidatedInput {
    fn parse(request: &JsonValue) -> Result<Self, TelegramFileDownloadExecutionError> {
        let object = request
            .as_object()
            .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::InvalidRequest))?;
        let allowed = [
            "message",
            "attachment",
            "cache_root",
            "bot_token",
            "request_timeout_seconds",
            "base_url",
            "file_base_url",
        ];
        if object.keys().any(|key| !allowed.contains(&key.as_str())) {
            return Err(error(
                TelegramFileDownloadExecutionErrorKind::InvalidRequest,
            ));
        }
        let message = object
            .get("message")
            .filter(|value| value.is_object() && value.to_string().len() <= MAX_CONTEXT_BYTES)
            .cloned()
            .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::InvalidRequest))?;
        let attachment = object
            .get("attachment")
            .filter(|value| value.is_object() && value.to_string().len() <= MAX_CONTEXT_BYTES)
            .cloned()
            .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::InvalidRequest))?;
        let attachment_object = attachment
            .as_object()
            .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::InvalidRequest))?;
        if !matches!(
            clean_text(attachment_object.get("kind")).as_deref(),
            Some("voice" | "audio" | "photo" | "document")
        ) {
            return Err(error(
                TelegramFileDownloadExecutionErrorKind::InvalidRequest,
            ));
        }
        if attachment_object
            .get("file_size_bytes")
            .and_then(JsonValue::as_u64)
            .is_some_and(|value| value == 0 || value > MAX_ATTACHMENT_BYTES as u64)
        {
            return Err(error(TelegramFileDownloadExecutionErrorKind::PayloadSize));
        }
        let cache_root_text = required_clean_text(object, "cache_root")
            .map_err(|_| error(TelegramFileDownloadExecutionErrorKind::InvalidRequest))?;
        let cache_root = PathBuf::from(&cache_root_text);
        if cache_root_text.len() > MAX_PATH_BYTES || !clean_absolute_path(&cache_root) {
            return Err(error(TelegramFileDownloadExecutionErrorKind::CachePath));
        }
        let bot_token = optional_config_text(object.get("bot_token"), MAX_TOKEN_BYTES)?;
        let base_url = optional_config_text(object.get("base_url"), MAX_URL_BYTES)?;
        let file_base_url = optional_config_text(object.get("file_base_url"), MAX_URL_BYTES)?;
        if bot_token.is_none() && (base_url.is_none() || file_base_url.is_none()) {
            return Err(error(
                TelegramFileDownloadExecutionErrorKind::InvalidRequest,
            ));
        }
        let request_timeout_seconds = match object.get("request_timeout_seconds") {
            None | Some(JsonValue::Null) => None,
            Some(value) => value
                .as_f64()
                .filter(|value| value.is_finite() && *value > 0.0 && *value <= 86_400.0)
                .map(Some)
                .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::InvalidRequest))?,
        };
        let file_id = clean_text(attachment_object.get("telegram_file_id"));
        if file_id.as_deref().is_some_and(|value| {
            value.len() > MAX_TOKEN_BYTES || value.chars().any(char::is_control)
        }) {
            return Err(error(
                TelegramFileDownloadExecutionErrorKind::InvalidRequest,
            ));
        }
        Ok(Self {
            message,
            attachment,
            cache_root,
            cache_root_text,
            file_id,
            bot_token,
            request_timeout_seconds,
            base_url,
            file_base_url,
        })
    }

    fn api_request(&self, operation: &str, value_key: &str, value: &str) -> JsonValue {
        let mut request = json!({
            "operation": operation,
            "request_timeout_seconds": self.request_timeout_seconds,
        });
        request[value_key] = json!(value);
        if let Some(bot_token) = &self.bot_token {
            request["bot_token"] = json!(bot_token);
        }
        if let Some(base_url) = &self.base_url {
            request["base_url"] = json!(base_url);
        }
        if let Some(file_base_url) = &self.file_base_url {
            request["file_base_url"] = json!(file_base_url);
        }
        request
    }
}

fn validate_missing_file_id_plan(
    planned: &JsonValue,
) -> Result<(), TelegramFileDownloadExecutionError> {
    let object = planning_envelope(planned, "request")?;
    let request = object
        .get("request")
        .and_then(JsonValue::as_object)
        .ok_or_else(planner_contract_error)?;
    if object.get("should_execute").and_then(JsonValue::as_bool) != Some(false)
        || request.get("ok").and_then(JsonValue::as_bool) != Some(false)
        || request.get("operation_count").and_then(JsonValue::as_u64) != Some(0)
        || !request
            .get("operations")
            .and_then(JsonValue::as_array)
            .is_some_and(Vec::is_empty)
    {
        return Err(planner_contract_error());
    }
    Ok(())
}

fn validate_request_plan<'a>(
    planned: &'a JsonValue,
    input: &ValidatedInput,
    file_id: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramFileDownloadExecutionError> {
    let object = planning_envelope(planned, "request")?;
    let request = object
        .get("request")
        .and_then(JsonValue::as_object)
        .ok_or_else(planner_contract_error)?;
    let operations = request
        .get("operations")
        .and_then(JsonValue::as_array)
        .ok_or_else(planner_contract_error)?;
    if object.get("should_execute").and_then(JsonValue::as_bool) != Some(true)
        || request.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || request.get("message") != Some(&input.message)
        || request.get("attachment") != Some(&input.attachment)
        || text(request, "cache_root") != Some(input.cache_root_text.as_str())
        || text(request, "telegram_file_id") != Some(file_id)
        || request.get("operation_count").and_then(JsonValue::as_u64) != Some(1)
        || operations.len() != 1
        || text_object(&operations[0], "kind") != Some("get_file")
        || text_object(&operations[0], "file_id") != Some(file_id)
    {
        return Err(planner_contract_error());
    }
    Ok(request)
}

fn validate_get_file_metadata(
    metadata: &JsonValue,
    file_id: &str,
) -> Result<JsonValue, TelegramFileDownloadExecutionError> {
    let object = validate_api_success(metadata, "get_file", "json", false)?;
    let file_info = object
        .get("file_info")
        .filter(|value| value.to_string().len() <= MAX_CONTEXT_BYTES)
        .and_then(JsonValue::as_object)
        .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::TelegramFileInfo))?;
    if file_info
        .get("file_id")
        .and_then(JsonValue::as_str)
        .is_some_and(|value| value != file_id)
        || file_info
            .get("file_size")
            .and_then(JsonValue::as_u64)
            .is_some_and(|value| value == 0 || value > MAX_ATTACHMENT_BYTES as u64)
    {
        return Err(error(
            TelegramFileDownloadExecutionErrorKind::TelegramFileInfo,
        ));
    }
    Ok(JsonValue::Object(file_info.clone()))
}

fn validate_telegram_file_path(
    file_info: &JsonValue,
) -> Result<String, TelegramFileDownloadExecutionError> {
    let path = clean_text(file_info.get("file_path"))
        .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::MissingFilePath))?;
    if path.len() > MAX_TELEGRAM_PATH_BYTES
        || path.starts_with('/')
        || path.contains('\\')
        || path.contains('?')
        || path.contains('#')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|segment| segment.is_empty() || matches!(segment, "." | ".."))
    {
        return Err(error(
            TelegramFileDownloadExecutionErrorKind::MissingFilePath,
        ));
    }
    Ok(path)
}

fn validate_file_info_plan<'a>(
    planned: &'a JsonValue,
    input: &ValidatedInput,
    file_id: &str,
    telegram_file_path: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramFileDownloadExecutionError> {
    let object = planning_envelope(planned, "file_info")?;
    let request = object
        .get("request")
        .and_then(JsonValue::as_object)
        .ok_or_else(planner_contract_error)?;
    let attachment = request
        .get("attachment")
        .and_then(JsonValue::as_object)
        .ok_or_else(planner_contract_error)?;
    let operations = request
        .get("operations")
        .and_then(JsonValue::as_array)
        .ok_or_else(planner_contract_error)?;
    let local_path =
        required_clean_text(request, "local_path").map_err(|_| planner_contract_error())?;
    if object.get("should_execute").and_then(JsonValue::as_bool) != Some(true)
        || request.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || request.get("message") != Some(&input.message)
        || text(request, "cache_root") != Some(input.cache_root_text.as_str())
        || text(request, "telegram_file_path") != Some(telegram_file_path)
        || text(attachment, "telegram_file_id") != Some(file_id)
        || text(attachment, "telegram_file_path") != Some(telegram_file_path)
        || request.get("operation_count").and_then(JsonValue::as_u64) != Some(1)
        || operations.len() != 1
        || text_object(&operations[0], "kind") != Some("check_cache")
        || text_object(&operations[0], "local_path") != Some(local_path.as_str())
    {
        return Err(planner_contract_error());
    }
    Ok(request)
}

fn validate_cache_plan<'a>(
    planned: &'a JsonValue,
    telegram_file_path: &str,
    local_path: &str,
    cache_hit: bool,
) -> Result<&'a Map<String, JsonValue>, TelegramFileDownloadExecutionError> {
    let object = planning_envelope(planned, "cache")?;
    let request = object
        .get("request")
        .and_then(JsonValue::as_object)
        .ok_or_else(planner_contract_error)?;
    let operations = request
        .get("operations")
        .and_then(JsonValue::as_array)
        .ok_or_else(planner_contract_error)?;
    if object.get("should_execute").and_then(JsonValue::as_bool) != Some(!cache_hit)
        || request.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || request.get("cache_hit").and_then(JsonValue::as_bool) != Some(cache_hit)
        || text(request, "telegram_file_path") != Some(telegram_file_path)
        || text(request, "local_path") != Some(local_path)
        || request.get("operation_count").and_then(JsonValue::as_u64) != Some(u64::from(!cache_hit))
        || operations.len() != usize::from(!cache_hit)
    {
        return Err(planner_contract_error());
    }
    if !cache_hit
        && (text_object(&operations[0], "kind") != Some("download_file_bytes")
            || text_object(&operations[0], "file_path") != Some(telegram_file_path)
            || text_object(&operations[0], "local_path") != Some(local_path))
    {
        return Err(planner_contract_error());
    }
    Ok(request)
}

fn validate_download_metadata(
    metadata: &JsonValue,
    payload_len: usize,
) -> Result<(), TelegramFileDownloadExecutionError> {
    let object = validate_api_success(metadata, "download_file", "bytes", true)?;
    if object.get("byte_count").and_then(JsonValue::as_u64) != Some(payload_len as u64) {
        return Err(error(TelegramFileDownloadExecutionErrorKind::Download));
    }
    Ok(())
}

fn validate_api_success<'a>(
    metadata: &'a JsonValue,
    operation: &str,
    transport: &str,
    downloaded: bool,
) -> Result<&'a Map<String, JsonValue>, TelegramFileDownloadExecutionError> {
    let object = metadata
        .as_object()
        .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::TelegramFileInfo))?;
    if text(object, "contract") != Some(API_CONTRACT)
        || text(object, "migration_stage") != Some(API_MIGRATION_STAGE)
        || text(object, "stage") != Some("execute")
        || text(object, "telegram_api_state") != Some("completed")
        || text(object, "operation") != Some(operation)
        || text(object, "transport") != Some(transport)
        || object.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || object.get("downloaded").and_then(JsonValue::as_bool) != Some(downloaded)
        || !object.get("error_kind").is_some_and(JsonValue::is_null)
        || !object.get("error").is_some_and(JsonValue::is_null)
        || !false_flag(object, "python_telegram_api_allowed")
        || !false_flag(object, "python_http_execution_allowed")
        || !false_flag(object, "python_retry_allowed")
        || !false_flag(object, "raw_telegram_payload_exposed")
        || !false_flag(object, "token_bearing_url_exposed")
        || !false_flag(object, "downloaded_bytes_exposed")
        || !false_flag(object, "local_path_exposed")
    {
        return Err(error(if operation == "get_file" {
            TelegramFileDownloadExecutionErrorKind::TelegramFileInfo
        } else {
            TelegramFileDownloadExecutionErrorKind::Download
        }));
    }
    Ok(object)
}

fn validate_result_plan(
    planned: &JsonValue,
    telegram_file_path: &str,
    local_path: &str,
    cache_hit: bool,
    downloaded: bool,
) -> Result<JsonValue, TelegramFileDownloadExecutionError> {
    let object = planning_envelope(planned, "result")?;
    let result = object
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::ResultContract))?;
    let attachment = result
        .get("attachment")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::ResultContract))?;
    let operations = result
        .get("operation_results")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| error(TelegramFileDownloadExecutionErrorKind::ResultContract))?;
    if object.get("completed").and_then(JsonValue::as_bool) != Some(true)
        || result.get("ok").and_then(JsonValue::as_bool) != Some(true)
        || !result.get("error").is_some_and(JsonValue::is_null)
        || !result.get("user_message").is_some_and(JsonValue::is_null)
        || text(result, "local_path") != Some(local_path)
        || result.get("cache_hit").and_then(JsonValue::as_bool) != Some(cache_hit)
        || result.get("downloaded").and_then(JsonValue::as_bool) != Some(downloaded)
        || text(attachment, "telegram_file_path") != Some(telegram_file_path)
        || text(attachment, "local_path") != Some(local_path)
        || result.get("operation_count").and_then(JsonValue::as_u64) != Some(u64::from(downloaded))
        || operations.len() != usize::from(downloaded)
    {
        return Err(error(
            TelegramFileDownloadExecutionErrorKind::ResultContract,
        ));
    }
    Ok(JsonValue::Object(attachment.clone()))
}

fn planning_envelope<'a>(
    planned: &'a JsonValue,
    stage: &str,
) -> Result<&'a Map<String, JsonValue>, TelegramFileDownloadExecutionError> {
    let object = planned.as_object().ok_or_else(planner_contract_error)?;
    if text(object, "stage") != Some(stage)
        || text(object, "execution_kind") != Some(PLANNING_KIND)
        || (stage != "result"
            && object.get("expects_result").and_then(JsonValue::as_bool) != Some(true))
    {
        return Err(planner_contract_error());
    }
    Ok(object)
}

fn validate_cache_path(cache_root: &Path, local_path: &Path) -> Result<(), String> {
    if !clean_absolute_path(cache_root)
        || !clean_absolute_path(local_path)
        || cache_root.as_os_str().len() > MAX_PATH_BYTES
        || local_path.as_os_str().len() > MAX_PATH_BYTES
    {
        return Err("Telegram attachment cache path is invalid.".to_string());
    }
    let relative = local_path
        .strip_prefix(cache_root)
        .map_err(|_| "Telegram attachment cache path escapes its root.".to_string())?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err("Telegram attachment cache path is invalid.".to_string());
    }
    Ok(())
}

fn validate_native_cache_path(cache_root: &Path, local_path: &Path) -> Result<(), String> {
    validate_cache_path(cache_root, local_path)?;
    validate_directory_if_present(cache_root)?;
    let parent = local_path
        .parent()
        .ok_or_else(|| "Telegram attachment cache path is invalid.".to_string())?;
    let relative_parent = parent
        .strip_prefix(cache_root)
        .map_err(|_| "Telegram attachment cache path is invalid.".to_string())?;
    let mut current = cache_root.to_path_buf();
    for component in relative_parent.components() {
        let Component::Normal(component) = component else {
            return Err("Telegram attachment cache path is invalid.".to_string());
        };
        current.push(component);
        validate_directory_if_present(&current)?;
    }
    match fs::symlink_metadata(local_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_file() => {
            Err("Telegram attachment cache target is invalid.".to_string())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Telegram attachment cache target is invalid.".to_string()),
    }
}

fn validate_directory_if_present(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() => {
            Err("Telegram attachment cache directory is invalid.".to_string())
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("Telegram attachment cache directory is invalid.".to_string()),
    }
}

fn validate_file_store_inspect(response: &JsonValue, path: &Path) -> Result<(), String> {
    let object = response
        .as_object()
        .ok_or_else(|| "Telegram attachment cache inspection failed.".to_string())?;
    if text(object, "contract") != Some(AGENT_FILE_STORE_CONTRACT)
        || text(object, "operation") != Some("inspect")
        || text(object, "path") != Some(path.to_string_lossy().as_ref())
        || object
            .get("result")
            .and_then(JsonValue::as_object)
            .and_then(|result| result.get("exists"))
            .and_then(JsonValue::as_bool)
            != Some(true)
        || !false_flag(object, "python_file_read_allowed")
        || !false_flag(object, "python_file_mutation_allowed")
    {
        return Err("Telegram attachment cache inspection failed.".to_string());
    }
    Ok(())
}

fn validate_file_store_publish(
    response: &JsonValue,
    path: &Path,
    expected_bytes: usize,
) -> Result<usize, String> {
    let object = response
        .as_object()
        .ok_or_else(|| "Telegram attachment cache publication failed.".to_string())?;
    let result = object
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "Telegram attachment cache publication failed.".to_string())?;
    let byte_count = result
        .get("byte_count")
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| "Telegram attachment cache publication failed.".to_string())?;
    if text(object, "contract") != Some(AGENT_FILE_STORE_CONTRACT)
        || text(object, "operation") != Some("publish")
        || text(object, "path") != Some(path.to_string_lossy().as_ref())
        || result.get("published").and_then(JsonValue::as_bool) != Some(true)
        || byte_count != expected_bytes
        || !false_flag(object, "python_file_read_allowed")
        || !false_flag(object, "python_file_mutation_allowed")
    {
        return Err("Telegram attachment cache publication failed.".to_string());
    }
    Ok(byte_count)
}

fn clean_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::Normal(_)
            )
        })
}

fn optional_config_text(
    value: Option<&JsonValue>,
    max_bytes: usize,
) -> Result<Option<String>, TelegramFileDownloadExecutionError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value))
            if value.trim() == value
                && !value.is_empty()
                && value.len() <= max_bytes
                && !value.chars().any(char::is_control) =>
        {
            Ok(Some(value.clone()))
        }
        _ => Err(error(
            TelegramFileDownloadExecutionErrorKind::InvalidRequest,
        )),
    }
}

fn required_clean_text(object: &Map<String, JsonValue>, key: &str) -> Result<String, ()> {
    clean_text(object.get(key)).ok_or(())
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn text<'a>(object: &'a Map<String, JsonValue>, key: &str) -> Option<&'a str> {
    object.get(key).and_then(JsonValue::as_str)
}

fn text_object<'a>(value: &'a JsonValue, key: &str) -> Option<&'a str> {
    value
        .as_object()
        .and_then(|object| object.get(key))
        .and_then(JsonValue::as_str)
}

fn false_flag(object: &Map<String, JsonValue>, key: &str) -> bool {
    object.get(key).and_then(JsonValue::as_bool) == Some(false)
}

fn error(kind: TelegramFileDownloadExecutionErrorKind) -> TelegramFileDownloadExecutionError {
    TelegramFileDownloadExecutionError::new(kind)
}

fn planner_contract_error() -> TelegramFileDownloadExecutionError {
    error(TelegramFileDownloadExecutionErrorKind::PlannerContract)
}

#[derive(Default)]
struct PathLockRegistry {
    locks: Mutex<BTreeMap<PathBuf, Weak<Mutex<()>>>>,
}

impl PathLockRegistry {
    fn lock_for(&self, path: &Path) -> Result<Arc<Mutex<()>>, TelegramFileDownloadExecutionError> {
        let mut locks = lock_unpoisoned(&self.locks);
        if locks.len() > 1_024 {
            locks.retain(|_, value| value.strong_count() > 0);
        }
        if let Some(lock) = locks.get(path).and_then(Weak::upgrade) {
            return Ok(lock);
        }
        let lock = Arc::new(Mutex::new(()));
        locks.insert(path.to_path_buf(), Arc::downgrade(&lock));
        Ok(lock)
    }
}

fn path_locks() -> &'static PathLockRegistry {
    PATH_LOCKS.get_or_init(PathLockRegistry::default)
}

fn lock_unpoisoned<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(value) => value,
        Err(poisoned) => poisoned.into_inner(),
    }
}

#[cfg(test)]
mod tests;
