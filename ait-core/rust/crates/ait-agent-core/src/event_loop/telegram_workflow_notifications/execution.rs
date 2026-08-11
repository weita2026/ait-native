use std::fmt;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use chrono::DateTime;

use super::planning::{
    DefaultTelegramWorkflowNotificationFormatter, TelegramWorkflowNotificationFormatter,
};
use crate::event_loop::telegram_command_runtime::{
    TelegramCommandRuntimeClockPort, TelegramCommandRuntimeReadPort,
};
use crate::event_loop::telegram_message_formatting::agent_telegram_message_delivery_execute_json;
use crate::runtime::AgentRuntimeBindingStore;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramWorkflowNotificationExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_workflow_notification_execution";
const MESSAGE_CONTRACT: &str = "ait_agent_core.event_loop.TelegramMessageDeliveryExecution.v1";
const MESSAGE_MIGRATION_STAGE: &str = "rust_agent_telegram_message_delivery_execution";
const MAX_QUEUE_ITEMS: usize = 10_000;
const MAX_QUEUE_BYTES: usize = 8 * 1_048_576;
const MAX_DIGEST_BYTES: usize = 512 * 1_024;
const MAX_FORMATTED_TEXT_BYTES: usize = 512 * 1_024;
const MAX_TEXT_BYTES: usize = 4_096;
const MAX_CLOCK_BYTES: usize = 128;
const MAX_MESSAGE_CHUNKS: usize = 128;

pub trait TelegramWorkflowNotificationMessagePort: Send + Sync + 'static {
    fn deliver_workflow_notification(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait TelegramWorkflowNotificationStatePort: Send + Sync + 'static {
    fn load_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String>;

    fn commit_digest(
        &self,
        chat_id: &JsonValue,
        previous_digest: &str,
        current_digest: &str,
        notification_at: Option<&str>,
    ) -> Result<Option<JsonValue>, String>;
}

impl TelegramWorkflowNotificationStatePort for AgentRuntimeBindingStore {
    fn load_binding(&self, chat_id: &JsonValue) -> Result<Option<JsonValue>, String> {
        match self.execute(
            "get_binding",
            &json!({"transport": "telegram", "surface_id": chat_id}),
        )? {
            JsonValue::Null => Ok(None),
            value @ JsonValue::Object(_) => Ok(Some(value)),
            _ => Err("Telegram workflow notification binding is invalid.".to_string()),
        }
    }

    fn commit_digest(
        &self,
        chat_id: &JsonValue,
        previous_digest: &str,
        current_digest: &str,
        notification_at: Option<&str>,
    ) -> Result<Option<JsonValue>, String> {
        self.mutate_binding_with("telegram", chat_id, |current| {
            let Some(current) = current.and_then(JsonValue::as_object) else {
                return Ok(None);
            };
            let current_previous = previous_digest_from_state(current)
                .map_err(|_| "Telegram workflow notification binding is invalid.".to_string())?;
            if current
                .get("workflow_notifications_enabled")
                .and_then(JsonValue::as_bool)
                != Some(true)
                || current_previous != previous_digest
            {
                return Ok(None);
            }
            Ok(Some(json!({
                "last_queue_summary_digest": current_digest,
                "last_queue_notification_at": notification_at,
            })))
        })
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTelegramWorkflowNotificationMessagePort;

impl TelegramWorkflowNotificationMessagePort for NativeTelegramWorkflowNotificationMessagePort {
    fn deliver_workflow_notification(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_message_delivery_execute_json(request)
    }
}

#[derive(Clone, PartialEq)]
pub struct TelegramWorkflowNotificationExecution {
    metadata: JsonValue,
}

impl TelegramWorkflowNotificationExecution {
    pub fn metadata(&self) -> &JsonValue {
        &self.metadata
    }

    pub fn into_metadata(self) -> JsonValue {
        self.metadata
    }
}

impl fmt::Debug for TelegramWorkflowNotificationExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TelegramWorkflowNotificationExecution")
            .field("status", &self.metadata.get("notification_status"))
            .field("queue_item_count", &self.metadata.get("queue_item_count"))
            .field("digest_changed", &self.metadata.get("digest_changed"))
            .field(
                "message_delivery_attempted",
                &self.metadata.get("message_delivery_attempted"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TelegramWorkflowNotificationExecutionErrorKind {
    InvalidRequest,
    State,
    Read,
    ReadContract,
    Formatter,
    FormatterContract,
    Clock,
    MessageDelivery,
    MessageDeliveryContract,
}

impl TelegramWorkflowNotificationExecutionErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::State => "state",
            Self::Read => "read",
            Self::ReadContract => "read_contract",
            Self::Formatter => "formatter",
            Self::FormatterContract => "formatter_contract",
            Self::Clock => "clock",
            Self::MessageDelivery => "message_delivery",
            Self::MessageDeliveryContract => "message_delivery_contract",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TelegramWorkflowNotificationExecutionError {
    kind: TelegramWorkflowNotificationExecutionErrorKind,
}

impl TelegramWorkflowNotificationExecutionError {
    pub fn kind(self) -> TelegramWorkflowNotificationExecutionErrorKind {
        self.kind
    }
}

impl fmt::Display for TelegramWorkflowNotificationExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TelegramWorkflowNotificationExecutionErrorKind::InvalidRequest => {
                "Telegram workflow notification request is invalid."
            }
            TelegramWorkflowNotificationExecutionErrorKind::State => {
                "Telegram workflow notification state operation failed."
            }
            TelegramWorkflowNotificationExecutionErrorKind::Read => {
                "Telegram workflow notification queue read failed."
            }
            TelegramWorkflowNotificationExecutionErrorKind::ReadContract => {
                "Telegram workflow notification queue-read contract is invalid."
            }
            TelegramWorkflowNotificationExecutionErrorKind::Formatter => {
                "Telegram workflow notification formatting failed."
            }
            TelegramWorkflowNotificationExecutionErrorKind::FormatterContract => {
                "Telegram workflow notification formatter contract is invalid."
            }
            TelegramWorkflowNotificationExecutionErrorKind::Clock => {
                "Telegram workflow notification clock failed."
            }
            TelegramWorkflowNotificationExecutionErrorKind::MessageDelivery => {
                "Telegram workflow notification delivery failed."
            }
            TelegramWorkflowNotificationExecutionErrorKind::MessageDeliveryContract => {
                "Telegram workflow notification delivery contract is invalid."
            }
        })
    }
}

impl std::error::Error for TelegramWorkflowNotificationExecutionError {}

use TelegramWorkflowNotificationExecutionErrorKind::*;

fn error(
    kind: TelegramWorkflowNotificationExecutionErrorKind,
) -> TelegramWorkflowNotificationExecutionError {
    TelegramWorkflowNotificationExecutionError { kind }
}

#[allow(clippy::too_many_arguments)]
pub fn execute_with_telegram_workflow_notification_ports<F, S, R, M, C>(
    formatter: &F,
    state: &S,
    reads: &R,
    message_delivery: &M,
    clock: &C,
    request: &JsonValue,
) -> Result<TelegramWorkflowNotificationExecution, TelegramWorkflowNotificationExecutionError>
where
    F: TelegramWorkflowNotificationFormatter + ?Sized,
    S: TelegramWorkflowNotificationStatePort + ?Sized,
    R: TelegramCommandRuntimeReadPort + ?Sized,
    M: TelegramWorkflowNotificationMessagePort + ?Sized,
    C: TelegramCommandRuntimeClockPort + ?Sized,
{
    let object = request.as_object().ok_or_else(|| error(InvalidRequest))?;
    let chat_id = validate_chat_id(object.get("chat_id"))?;
    let config = validate_config(object.get("config"))?;
    validate_delivery_options(object)?;

    let Some(initial_binding) = state.load_binding(&chat_id).map_err(|_| error(State))? else {
        return Ok(execution(metadata(MetadataInput::terminal(
            "missing_binding",
        ))));
    };
    let initial_binding_object = initial_binding.as_object().ok_or_else(|| error(State))?;
    if initial_binding_object
        .get("workflow_notifications_enabled")
        .and_then(JsonValue::as_bool)
        != Some(true)
    {
        return Ok(execution(metadata(MetadataInput {
            status: "disabled",
            ..MetadataInput::default()
        })));
    }
    let previous_digest = previous_digest(initial_binding_object)?;

    let queue_payload = reads
        .read_workflow_notification()
        .map_err(|_| error(Read))?;
    let queue_item_count = validate_queue_payload(&queue_payload)?;
    let current = derive_digest(formatter, &queue_payload)?;
    let previous_actionable = derive_actionability(formatter, &previous_digest)?;
    let current_actionable_from_digest = derive_actionability(formatter, &current.digest)?;
    if current.actionable != current_actionable_from_digest {
        return Err(error(FormatterContract));
    }
    let current_actionable = current.actionable;

    if current.digest == previous_digest {
        return Ok(execution(metadata(MetadataInput {
            status: "unchanged",
            queue_item_count,
            current_actionable,
            previous_actionable,
            ..MetadataInput::default()
        })));
    }

    let should_deliver = current.actionable || previous_actionable;
    let formatted_text = if should_deliver {
        Some(format_notification(formatter, &config, &queue_payload)?)
    } else {
        None
    };
    let now_iso = if should_deliver {
        Some(validated_now(clock)?)
    } else {
        None
    };
    let commit = commit_digest(
        state,
        &chat_id,
        &previous_digest,
        &current.digest,
        now_iso.as_deref(),
    )?;
    if !matches!(commit, DigestCommit::Applied) {
        return Ok(execution(metadata(MetadataInput {
            status: "concurrent_binding_change",
            terminal: true,
            queue_item_count,
            current_actionable,
            previous_actionable,
            digest_changed: true,
            ..MetadataInput::default()
        })));
    }

    let Some(formatted_text) = formatted_text else {
        return Ok(execution(metadata(MetadataInput {
            status: "updated_silent",
            queue_item_count,
            current_actionable,
            previous_actionable,
            digest_changed: true,
            state_updated: true,
            ..MetadataInput::default()
        })));
    };

    let delivery_request = build_delivery_request(object, &chat_id, &formatted_text);
    let raw_delivery = message_delivery
        .deliver_workflow_notification(&delivery_request)
        .map_err(|_| error(MessageDelivery))?;
    let disposition = validate_message_delivery(&raw_delivery)?;
    let (status, retryable, terminal, delivered) = match disposition {
        MessageDisposition::Delivered => ("delivered", false, false, true),
        MessageDisposition::Retryable => ("delivery_retryable", true, false, false),
        MessageDisposition::Terminal => ("delivery_terminal", false, true, false),
    };
    Ok(execution(metadata(MetadataInput {
        status,
        retryable,
        terminal,
        queue_item_count,
        current_actionable,
        previous_actionable,
        digest_changed: true,
        state_updated: true,
        message_attempted: true,
        message_delivered: delivered,
    })))
}

struct DigestResult {
    digest: String,
    actionable: bool,
}

fn derive_digest<F>(
    formatter: &F,
    payload: &JsonValue,
) -> Result<DigestResult, TelegramWorkflowNotificationExecutionError>
where
    F: TelegramWorkflowNotificationFormatter + ?Sized,
{
    let request = json!({"kind": "queue_digest", "payload": payload});
    let formatted = validated_format(formatter, &request)?;
    let object = formatted
        .as_object()
        .ok_or_else(|| error(FormatterContract))?;
    let digest = object
        .get("digest")
        .and_then(JsonValue::as_str)
        .filter(|value| value.len() <= MAX_DIGEST_BYTES && !value.contains('\0'))
        .ok_or_else(|| error(FormatterContract))?
        .to_string();
    let actionable = object
        .get("actionable")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(FormatterContract))?;
    if clean_text(object.get("kind")).as_deref() != Some("queue_digest") {
        return Err(error(FormatterContract));
    }
    Ok(DigestResult { digest, actionable })
}

fn derive_actionability<F>(
    formatter: &F,
    digest: &str,
) -> Result<bool, TelegramWorkflowNotificationExecutionError>
where
    F: TelegramWorkflowNotificationFormatter + ?Sized,
{
    let request = json!({"kind": "queue_digest_actionable", "raw": digest});
    let formatted = validated_format(formatter, &request)?;
    let object = formatted
        .as_object()
        .ok_or_else(|| error(FormatterContract))?;
    if clean_text(object.get("kind")).as_deref() != Some("queue_digest_actionable") {
        return Err(error(FormatterContract));
    }
    object
        .get("actionable")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(FormatterContract))
}

fn format_notification<F>(
    formatter: &F,
    config: &JsonValue,
    payload: &JsonValue,
) -> Result<String, TelegramWorkflowNotificationExecutionError>
where
    F: TelegramWorkflowNotificationFormatter + ?Sized,
{
    let request = json!({
        "kind": "workflow_notification",
        "config": config,
        "payload": payload,
    });
    let formatted = validated_format(formatter, &request)?;
    let object = formatted
        .as_object()
        .ok_or_else(|| error(FormatterContract))?;
    let text = object
        .get("text")
        .and_then(JsonValue::as_str)
        .filter(|value| {
            !value.is_empty() && value.len() <= MAX_FORMATTED_TEXT_BYTES && !value.contains('\0')
        })
        .ok_or_else(|| error(FormatterContract))?;
    if clean_text(object.get("kind")).as_deref() != Some("workflow_notification") {
        return Err(error(FormatterContract));
    }
    Ok(text.to_string())
}

fn validated_format<F>(
    formatter: &F,
    request: &JsonValue,
) -> Result<JsonValue, TelegramWorkflowNotificationExecutionError>
where
    F: TelegramWorkflowNotificationFormatter + ?Sized,
{
    let expected = DefaultTelegramWorkflowNotificationFormatter
        .format_json(request)
        .map_err(|_| error(FormatterContract))?;
    let formatted = formatter
        .format_json(request)
        .map_err(|_| error(Formatter))?;
    if formatted != expected {
        return Err(error(FormatterContract));
    }
    Ok(formatted)
}

enum DigestCommit {
    Applied,
    ConcurrentChange,
}

fn commit_digest<S>(
    state: &S,
    chat_id: &JsonValue,
    previous_digest: &str,
    current_digest: &str,
    notification_at: Option<&str>,
) -> Result<DigestCommit, TelegramWorkflowNotificationExecutionError>
where
    S: TelegramWorkflowNotificationStatePort + ?Sized,
{
    let persisted = state
        .commit_digest(chat_id, previous_digest, current_digest, notification_at)
        .map_err(|_| error(State))?;
    let Some(persisted) = persisted.and_then(|value| value.as_object().cloned()) else {
        return Ok(DigestCommit::ConcurrentChange);
    };
    if persisted_value_matches(
        persisted.get("last_queue_summary_digest"),
        &JsonValue::String(current_digest.to_string()),
    ) {
        Ok(DigestCommit::Applied)
    } else {
        Err(error(State))
    }
}

fn persisted_value_matches(actual: Option<&JsonValue>, expected: &JsonValue) -> bool {
    if expected.is_null() {
        actual.is_none_or(JsonValue::is_null)
    } else {
        actual == Some(expected)
    }
}

enum MessageDisposition {
    Delivered,
    Retryable,
    Terminal,
}

fn validate_message_delivery(
    value: &JsonValue,
) -> Result<MessageDisposition, TelegramWorkflowNotificationExecutionError> {
    let object = value
        .as_object()
        .ok_or_else(|| error(MessageDeliveryContract))?;
    if value.to_string().len() > MAX_QUEUE_BYTES
        || clean_text(object.get("contract")).as_deref() != Some(MESSAGE_CONTRACT)
        || clean_text(object.get("migration_stage")).as_deref() != Some(MESSAGE_MIGRATION_STAGE)
        || clean_text(object.get("stage")).as_deref() != Some("execute")
        || !false_flags(
            object,
            &[
                "python_message_delivery_allowed",
                "python_message_formatting_allowed",
                "raw_api_result_exposed",
                "telegram_description_exposed",
                "token_bearing_url_exposed",
                "chat_id_exposed",
                "formatted_text_exposed",
                "plain_text_exposed",
            ],
        )
    {
        return Err(error(MessageDeliveryContract));
    }
    let ok = object
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(MessageDeliveryContract))?;
    let completed = object
        .get("completed")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(MessageDeliveryContract))?;
    let chunk_count = count_field(object, "chunk_count", MAX_MESSAGE_CHUNKS)?;
    let completed_chunk_count = count_field(object, "completed_chunk_count", chunk_count)?;
    let chunk_results = object
        .get("chunk_results")
        .and_then(JsonValue::as_array)
        .filter(|values| values.len() <= chunk_count && values.iter().all(JsonValue::is_object))
        .ok_or_else(|| error(MessageDeliveryContract))?;
    if ok {
        if !completed
            || clean_text(object.get("message_delivery_state")).as_deref() != Some("completed")
            || chunk_count == 0
            || completed_chunk_count != chunk_count
            || chunk_results.len() != chunk_count
            || !object
                .get("failed_chunk_index")
                .is_some_and(JsonValue::is_null)
            || !object.get("error_kind").is_some_and(JsonValue::is_null)
            || !object.get("error").is_some_and(JsonValue::is_null)
        {
            return Err(error(MessageDeliveryContract));
        }
        return Ok(MessageDisposition::Delivered);
    }
    if completed {
        return Err(error(MessageDeliveryContract));
    }
    let error_kind = clean_text(object.get("error_kind"))
        .filter(|value| value.len() <= 64 && !value.chars().any(char::is_control))
        .ok_or_else(|| error(MessageDeliveryContract))?;
    if !matches!(
        error_kind.as_str(),
        "contract"
            | "planning"
            | "unsupported"
            | "executor"
            | "timeout"
            | "transport"
            | "http"
            | "url"
            | "invalid_timeout"
            | "response"
            | "sleep"
            | "telegram_api"
    ) {
        return Err(error(MessageDeliveryContract));
    }
    Ok(
        if matches!(error_kind.as_str(), "timeout" | "transport" | "sleep") {
            MessageDisposition::Retryable
        } else {
            MessageDisposition::Terminal
        },
    )
}

#[derive(Clone, Copy)]
struct MetadataInput {
    status: &'static str,
    retryable: bool,
    terminal: bool,
    queue_item_count: usize,
    current_actionable: bool,
    previous_actionable: bool,
    digest_changed: bool,
    state_updated: bool,
    message_attempted: bool,
    message_delivered: bool,
}

impl Default for MetadataInput {
    fn default() -> Self {
        Self {
            status: "completed",
            retryable: false,
            terminal: false,
            queue_item_count: 0,
            current_actionable: false,
            previous_actionable: false,
            digest_changed: false,
            state_updated: false,
            message_attempted: false,
            message_delivered: false,
        }
    }
}

impl MetadataInput {
    fn terminal(status: &'static str) -> Self {
        Self {
            status,
            terminal: true,
            ..Self::default()
        }
    }
}

fn metadata(input: MetadataInput) -> JsonValue {
    let mut metadata = json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "execution_kind": "workflow_notification",
        "notification_status": input.status,
        "ok": true,
        "completed": !input.retryable && !input.terminal,
        "retryable": input.retryable,
        "terminal": input.terminal,
        "queue_item_count": input.queue_item_count,
        "current_actionable": input.current_actionable,
        "previous_actionable": input.previous_actionable,
        "digest_changed": input.digest_changed,
        "state_updated": input.state_updated,
        "message_delivery_attempted": input.message_attempted,
        "message_delivered": input.message_delivered,
        "sent_any": input.message_delivered,
        "workflow_notification_sent": input.message_delivered,
    })
    .as_object()
    .cloned()
    .expect("workflow-notification metadata core must be an object");
    metadata.extend(
        json!({
            "python_workflow_notification_allowed": false,
            "python_queue_read_allowed": false,
            "python_digest_comparison_allowed": false,
            "python_state_mutation_allowed": false,
            "python_message_delivery_allowed": false,
            "raw_queue_payload_exposed": false,
            "raw_formatter_result_exposed": false,
            "raw_delivery_result_exposed": false,
            "digest_exposed": false,
            "chat_id_exposed": false,
            "runtime_target_exposed": false,
            "formatted_text_exposed": false,
            "bot_token_exposed": false,
            "state_path_exposed": false,
            "downstream_error_exposed": false,
        })
        .as_object()
        .cloned()
        .expect("workflow-notification metadata safety flags must be an object"),
    );
    JsonValue::Object(metadata)
}

fn execution(metadata: JsonValue) -> TelegramWorkflowNotificationExecution {
    TelegramWorkflowNotificationExecution { metadata }
}

fn validate_queue_payload(
    value: &JsonValue,
) -> Result<usize, TelegramWorkflowNotificationExecutionError> {
    let object = value.as_object().ok_or_else(|| error(ReadContract))?;
    let items = object
        .get("items")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| error(ReadContract))?;
    let source = clean_text(object.get("notification_source"));
    if source
        .as_deref()
        .is_some_and(|source| !matches!(source, "remote_queue" | "local_current"))
        || source.as_deref() == Some("local_current") && items.len() > 1
        || items.len() > MAX_QUEUE_ITEMS
        || items.iter().any(|item| !item.is_object())
        || value.to_string().len() > MAX_QUEUE_BYTES
    {
        return Err(error(ReadContract));
    }
    Ok(items.len())
}

fn validate_chat_id(
    value: Option<&JsonValue>,
) -> Result<JsonValue, TelegramWorkflowNotificationExecutionError> {
    let value = value.cloned().ok_or_else(|| error(InvalidRequest))?;
    let text = scalar_text(&value).ok_or_else(|| error(InvalidRequest))?;
    if text.is_empty() || text.len() > 512 || text.chars().any(char::is_control) {
        return Err(error(InvalidRequest));
    }
    Ok(value)
}

fn validate_config(
    value: Option<&JsonValue>,
) -> Result<JsonValue, TelegramWorkflowNotificationExecutionError> {
    let value = value.cloned().ok_or_else(|| error(InvalidRequest))?;
    let object = value.as_object().ok_or_else(|| error(InvalidRequest))?;
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "repo_name" | "ait_web_url"))
        || bounded_text(object.get("repo_name"), 1, MAX_TEXT_BYTES).is_none()
        || optional_bounded_text(object.get("ait_web_url"), MAX_TEXT_BYTES).is_err()
    {
        return Err(error(InvalidRequest));
    }
    Ok(value)
}

fn validate_delivery_options(
    object: &Map<String, JsonValue>,
) -> Result<(), TelegramWorkflowNotificationExecutionError> {
    for key in ["bot_token", "token", "base_url"] {
        optional_bounded_text(object.get(key), MAX_FORMATTED_TEXT_BYTES)?;
    }
    if let Some(value) = object.get("request_timeout_seconds") {
        let valid = match value {
            JsonValue::Null => true,
            JsonValue::Number(number) => number
                .as_f64()
                .is_some_and(|value| value.is_finite() && value > 0.0 && value <= 86_400.0),
            _ => false,
        };
        if !valid {
            return Err(error(InvalidRequest));
        }
    }
    optional_bool(object.get("reply_markdown_enabled"), false)?;
    Ok(())
}

fn build_delivery_request(
    source: &Map<String, JsonValue>,
    chat_id: &JsonValue,
    text: &str,
) -> JsonValue {
    let mut request = Map::new();
    request.insert("chat_id".to_string(), chat_id.clone());
    request.insert("text".to_string(), json!(text));
    for key in [
        "bot_token",
        "token",
        "base_url",
        "request_timeout_seconds",
        "reply_markdown_enabled",
    ] {
        if let Some(value) = source.get(key) {
            request.insert(key.to_string(), value.clone());
        }
    }
    JsonValue::Object(request)
}

fn validated_now<C>(clock: &C) -> Result<String, TelegramWorkflowNotificationExecutionError>
where
    C: TelegramCommandRuntimeClockPort + ?Sized,
{
    let value = clock.now_iso().map_err(|_| error(Clock))?;
    if value.is_empty()
        || value.len() > MAX_CLOCK_BYTES
        || value.chars().any(char::is_control)
        || DateTime::parse_from_rfc3339(&value).is_err()
    {
        return Err(error(Clock));
    }
    Ok(value)
}

fn previous_digest(
    object: &Map<String, JsonValue>,
) -> Result<String, TelegramWorkflowNotificationExecutionError> {
    previous_digest_from_state(object).map_err(|_| error(State))
}

fn previous_digest_from_state(object: &Map<String, JsonValue>) -> Result<String, ()> {
    match object.get("last_queue_summary_digest") {
        None | Some(JsonValue::Null) => Ok(String::new()),
        Some(JsonValue::String(value))
            if value.len() <= MAX_DIGEST_BYTES && !value.contains('\0') =>
        {
            Ok(value.clone())
        }
        _ => Err(()),
    }
}

fn optional_bool(
    value: Option<&JsonValue>,
    default: bool,
) -> Result<bool, TelegramWorkflowNotificationExecutionError> {
    match value {
        None | Some(JsonValue::Null) => Ok(default),
        Some(JsonValue::Bool(value)) => Ok(*value),
        _ => Err(error(InvalidRequest)),
    }
}

fn optional_bounded_text(
    value: Option<&JsonValue>,
    max: usize,
) -> Result<Option<String>, TelegramWorkflowNotificationExecutionError> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => {
            let value = value.trim();
            if value.is_empty()
                || value.len() > max
                || value
                    .chars()
                    .any(|ch| ch == '\0' || ch == '\r' || ch == '\n')
            {
                Err(error(InvalidRequest))
            } else {
                Ok(Some(value.to_string()))
            }
        }
        _ => Err(error(InvalidRequest)),
    }
}

fn bounded_text(value: Option<&JsonValue>, min: usize, max: usize) -> Option<String> {
    let value = clean_text(value)?;
    (value.len() >= min
        && value.len() <= max
        && !value
            .chars()
            .any(|ch| ch == '\0' || ch == '\r' || ch == '\n'))
    .then_some(value)
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

fn count_field(
    object: &Map<String, JsonValue>,
    key: &str,
    max: usize,
) -> Result<usize, TelegramWorkflowNotificationExecutionError> {
    object
        .get(key)
        .and_then(JsonValue::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value <= max)
        .ok_or_else(|| error(MessageDeliveryContract))
}

fn false_flags(object: &Map<String, JsonValue>, flags: &[&str]) -> bool {
    flags
        .iter()
        .all(|flag| object.get(*flag).and_then(JsonValue::as_bool) == Some(false))
}

#[cfg(test)]
mod local_current_payload_tests {
    use super::*;

    #[test]
    fn local_current_read_contract_rejects_queue_shaped_multi_item_payloads() {
        assert_eq!(
            validate_queue_payload(&json!({
                "notification_source": "local_current",
                "items": [{}],
            })),
            Ok(1)
        );
        assert_eq!(
            validate_queue_payload(&json!({
                "notification_source": "local_current",
                "items": [{}, {}],
            }))
            .expect_err("local current is never a queue")
            .kind(),
            TelegramWorkflowNotificationExecutionErrorKind::ReadContract
        );
        assert_eq!(
            validate_queue_payload(&json!({
                "notification_source": "remote_queue",
                "items": [{}, {}],
            })),
            Ok(2)
        );
    }
}
