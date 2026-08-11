use std::collections::HashSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ait_core::json_support::{json, JsonValue};

use super::execution::binding_has_background_sync_work;
use crate::event_loop::telegram_background_sync_state::{
    DefaultTelegramBackgroundSyncStatePlanner, TelegramBackgroundSyncStatePlanner,
};
use crate::event_loop::telegram_service_cycle::TelegramServiceCycleBackgroundSyncPort;
use crate::event_loop::telegram_submission_runtime::TelegramSubmissionRuntime;
use crate::runtime::AgentRuntimeBindingStore;

const MAX_BINDING_COUNT: usize = 10_000;
const MAX_BINDING_SNAPSHOT_BYTES: usize = 8 * 1_048_576;
const MAX_BINDING_BYTES: usize = 1_048_576;
const MAX_CHAT_ID_BYTES: usize = 123;
const MAX_QUEUE_KEY_BYTES: usize = 128;
const MAX_STATE_PATH_BYTES: usize = 16 * 1_024;

pub trait TelegramBackgroundSyncBindingReadPort: Send + Sync + 'static {
    fn list_active_telegram_bindings(&self) -> Result<Vec<JsonValue>, String>;
}

pub trait TelegramBackgroundSyncSubmissionPort: Send + Sync + 'static {
    fn submit_background_sync_for_chat(
        &self,
        queue_key: &str,
        chat_id: &JsonValue,
    ) -> Result<(), String>;
}

#[derive(Clone)]
pub struct RuntimeBindingTelegramBackgroundSyncReadPort {
    store: AgentRuntimeBindingStore,
}

impl RuntimeBindingTelegramBackgroundSyncReadPort {
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, String> {
        let path = path.into();
        validate_state_path(&path)?;
        Ok(Self {
            store: AgentRuntimeBindingStore::new(path),
        })
    }

    pub fn path(&self) -> &Path {
        self.store.path()
    }
}

impl fmt::Debug for RuntimeBindingTelegramBackgroundSyncReadPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RuntimeBindingTelegramBackgroundSyncReadPort")
            .field("state_path_exposed", &false)
            .finish()
    }
}

impl TelegramBackgroundSyncBindingReadPort for RuntimeBindingTelegramBackgroundSyncReadPort {
    fn list_active_telegram_bindings(&self) -> Result<Vec<JsonValue>, String> {
        let result = self.store.execute(
            "list_bindings",
            &json!({
                "transport": "telegram",
                "include_inactive": false,
            }),
        )?;
        result.as_array().cloned().ok_or_else(binding_read_error)
    }
}

impl TelegramBackgroundSyncSubmissionPort for TelegramSubmissionRuntime {
    fn submit_background_sync_for_chat(
        &self,
        queue_key: &str,
        chat_id: &JsonValue,
    ) -> Result<(), String> {
        let future = TelegramSubmissionRuntime::submit_background_sync_for_chat(
            self,
            Some(queue_key),
            chat_id.clone(),
        )
        .map_err(|_| submission_error())?;
        drop(future);
        Ok(())
    }
}

pub struct NativeTelegramBackgroundSyncServicePort<
    R,
    S,
    P = DefaultTelegramBackgroundSyncStatePlanner,
> {
    bindings: R,
    submissions: Arc<S>,
    planner: P,
}

impl<R, S> NativeTelegramBackgroundSyncServicePort<R, S> {
    pub fn with_ports(bindings: R, submissions: Arc<S>) -> Self {
        Self {
            bindings,
            submissions,
            planner: DefaultTelegramBackgroundSyncStatePlanner,
        }
    }
}

impl<S> NativeTelegramBackgroundSyncServicePort<RuntimeBindingTelegramBackgroundSyncReadPort, S> {
    pub fn new(state_path: impl Into<PathBuf>, submissions: Arc<S>) -> Result<Self, String> {
        Ok(Self::with_ports(
            RuntimeBindingTelegramBackgroundSyncReadPort::new(state_path)?,
            submissions,
        ))
    }
}

impl<R, S, P> NativeTelegramBackgroundSyncServicePort<R, S, P> {
    pub fn with_planner(bindings: R, submissions: Arc<S>, planner: P) -> Self {
        Self {
            bindings,
            submissions,
            planner,
        }
    }
}

impl<R, S, P> fmt::Debug for NativeTelegramBackgroundSyncServicePort<R, S, P> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NativeTelegramBackgroundSyncServicePort")
            .field("binding_snapshot_exposed", &false)
            .field("chat_id_exposed", &false)
            .field("queue_key_exposed", &false)
            .field("state_path_exposed", &false)
            .field("downstream_error_exposed", &false)
            .finish()
    }
}

impl<R, S, P> TelegramServiceCycleBackgroundSyncPort
    for NativeTelegramBackgroundSyncServicePort<R, S, P>
where
    R: TelegramBackgroundSyncBindingReadPort,
    S: TelegramBackgroundSyncSubmissionPort,
    P: TelegramBackgroundSyncStatePlanner + Send + Sync + 'static,
{
    fn run_background_sync_once(&self, request: &JsonValue) -> Result<usize, String> {
        validate_callback_request(request)?;
        let bindings = self
            .bindings
            .list_active_telegram_bindings()
            .map_err(|_| binding_read_error())?;
        validate_snapshot_bounds(&bindings)?;

        let mut seen = HashSet::with_capacity(bindings.len());
        let mut submissions = Vec::new();
        for binding in &bindings {
            let (chat_id, queue_key) = validate_binding(binding)?;
            if !seen.insert(chat_id.clone()) {
                return Err(binding_contract_error());
            }
            let has_work = binding_has_background_sync_work(&self.planner, binding)
                .map_err(|_| work_contract_error())?;
            if has_work {
                submissions.push((JsonValue::String(chat_id), queue_key));
            }
        }

        let mut accepted = 0_usize;
        for (chat_id, queue_key) in submissions {
            self.submissions
                .submit_background_sync_for_chat(&queue_key, &chat_id)
                .map_err(|_| submission_error())?;
            accepted += 1;
        }
        Ok(accepted)
    }
}

fn validate_callback_request(request: &JsonValue) -> Result<(), String> {
    let object = request.as_object().ok_or_else(callback_request_error)?;
    if object.len() != 2
        || object.get("callback_kind").and_then(JsonValue::as_str)
            != Some("run_background_sync_once")
        || object.get("callback_group").and_then(JsonValue::as_str) != Some("background_sync")
    {
        return Err(callback_request_error());
    }
    Ok(())
}

fn validate_snapshot_bounds(bindings: &[JsonValue]) -> Result<(), String> {
    if bindings.len() > MAX_BINDING_COUNT {
        return Err(binding_contract_error());
    }
    let mut snapshot_bytes = 2_usize;
    for (index, binding) in bindings.iter().enumerate() {
        let binding_bytes = binding.to_string().len();
        if binding_bytes > MAX_BINDING_BYTES {
            return Err(binding_contract_error());
        }
        snapshot_bytes = snapshot_bytes
            .saturating_add(binding_bytes)
            .saturating_add(usize::from(index > 0));
        if snapshot_bytes > MAX_BINDING_SNAPSHOT_BYTES {
            return Err(binding_contract_error());
        }
    }
    Ok(())
}

fn validate_binding(binding: &JsonValue) -> Result<(String, String), String> {
    let object = binding.as_object().ok_or_else(binding_contract_error)?;
    let raw_chat_id = object
        .get("surface_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(binding_contract_error)?;
    let chat_id = raw_chat_id.trim();
    if raw_chat_id != chat_id
        || chat_id.is_empty()
        || chat_id.len() > MAX_CHAT_ID_BYTES
        || chat_id.chars().any(char::is_control)
    {
        return Err(binding_contract_error());
    }
    let binding_id = format!("telegram:{chat_id}");
    let queue_key = format!("chat-{chat_id}");
    if object.get("transport").and_then(JsonValue::as_str) != Some("telegram")
        || object
            .get("status")
            .and_then(JsonValue::as_str)
            .is_some_and(|status| status != "active")
        || object.get("binding_id").and_then(JsonValue::as_str) != Some(binding_id.as_str())
        || queue_key.len() > MAX_QUEUE_KEY_BYTES
    {
        return Err(binding_contract_error());
    }
    Ok((chat_id.to_string(), queue_key))
}

fn validate_state_path(path: &Path) -> Result<(), String> {
    let text = path.to_string_lossy();
    if path.as_os_str().is_empty()
        || text.len() > MAX_STATE_PATH_BYTES
        || text.contains('\0')
        || text.contains('\r')
        || text.contains('\n')
    {
        return Err("Telegram background sync state configuration is invalid.".to_string());
    }
    Ok(())
}

fn callback_request_error() -> String {
    "Telegram background sync service request is invalid.".to_string()
}

fn binding_read_error() -> String {
    "Telegram background sync binding read failed.".to_string()
}

fn binding_contract_error() -> String {
    "Telegram background sync binding contract is invalid.".to_string()
}

fn work_contract_error() -> String {
    "Telegram background sync work contract is invalid.".to_string()
}

fn submission_error() -> String {
    "Telegram background sync submission failed.".to_string()
}

#[cfg(test)]
mod tests;
