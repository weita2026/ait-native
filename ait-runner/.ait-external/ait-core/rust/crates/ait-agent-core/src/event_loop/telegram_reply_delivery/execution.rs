use std::fmt;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::telegram_api_method::agent_telegram_api_execute;
use crate::event_loop::telegram_message_formatting::agent_telegram_message_delivery_execute_json;
use crate::event_loop::telegram_polling::agent_telegram_reply_delivery_execution_plan_json;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramReplyDeliveryExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_reply_delivery_execution";
const MESSAGE_CONTRACT: &str = "ait_agent_core.event_loop.TelegramMessageDeliveryExecution.v1";
const MESSAGE_MIGRATION_STAGE: &str = "rust_agent_telegram_message_delivery_execution";
const API_CONTRACT: &str = "ait_agent_core.event_loop.TelegramApiTransportExecution.v1";
const API_MIGRATION_STAGE: &str = "rust_agent_telegram_transport_execution";
const MAX_ATTACHMENTS: usize = 128;
const MAX_REPLY_CHARS: usize = 3_800 * 128;
const MAX_ASSISTANT_EVENT_BYTES: usize = 2 * 1_048_576;

pub trait TelegramReplyDeliveryPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultTelegramReplyDeliveryPlanner;

impl TelegramReplyDeliveryPlanner for DefaultTelegramReplyDeliveryPlanner {
    fn plan_json(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_reply_delivery_execution_plan_json(request)
    }
}

pub trait TelegramReplyDeliveryPort {
    fn execute_message(&self, request: &JsonValue) -> Result<JsonValue, String>;

    fn execute_attachment(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTelegramReplyDeliveryPort;

impl TelegramReplyDeliveryPort for NativeTelegramReplyDeliveryPort {
    fn execute_message(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_message_delivery_execute_json(request)
    }

    fn execute_attachment(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_api_execute(request).map(|execution| execution.metadata().clone())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TelegramReplyDeliveryOperationKind {
    Message,
    Audio,
    Photo,
    Document,
}

impl TelegramReplyDeliveryOperationKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::Message => "send_message",
            Self::Audio => "send_audio",
            Self::Photo => "send_photo",
            Self::Document => "send_document",
        }
    }

    fn method(self) -> &'static str {
        match self {
            Self::Message => "sendMessage",
            Self::Audio => "sendAudio",
            Self::Photo => "sendPhoto",
            Self::Document => "sendDocument",
        }
    }

    fn file_field(self) -> Option<&'static str> {
        match self {
            Self::Message => None,
            Self::Audio => Some("audio"),
            Self::Photo => Some("photo"),
            Self::Document => Some("document"),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum TelegramReplyDeliveryExecutionErrorKind {
    InvalidRequest,
    EmptyReply,
    Planner,
    PlannerContract,
    Message,
    MessageContract,
    Attachment,
    AttachmentContract,
    ResultPlanner,
    ResultContract,
}

impl TelegramReplyDeliveryExecutionErrorKind {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::EmptyReply => "empty_reply",
            Self::Planner => "planner",
            Self::PlannerContract => "planner_contract",
            Self::Message => "message",
            Self::MessageContract => "message_contract",
            Self::Attachment => "attachment",
            Self::AttachmentContract => "attachment_contract",
            Self::ResultPlanner => "result_planner",
            Self::ResultContract => "result_contract",
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct TelegramReplyDeliveryExecutionError {
    kind: TelegramReplyDeliveryExecutionErrorKind,
    operation_index: Option<usize>,
    operation_kind: Option<TelegramReplyDeliveryOperationKind>,
    retryable: bool,
}

impl TelegramReplyDeliveryExecutionError {
    pub fn kind(self) -> TelegramReplyDeliveryExecutionErrorKind {
        self.kind
    }

    pub fn operation_index(self) -> Option<usize> {
        self.operation_index
    }

    pub fn operation_kind(self) -> Option<TelegramReplyDeliveryOperationKind> {
        self.operation_kind
    }

    pub fn is_retryable(self) -> bool {
        self.retryable
    }

    fn new(kind: TelegramReplyDeliveryExecutionErrorKind) -> Self {
        Self {
            kind,
            operation_index: None,
            operation_kind: None,
            retryable: false,
        }
    }

    fn operation(
        kind: TelegramReplyDeliveryExecutionErrorKind,
        index: usize,
        operation_kind: TelegramReplyDeliveryOperationKind,
        retryable: bool,
    ) -> Self {
        Self {
            kind,
            operation_index: Some(index),
            operation_kind: Some(operation_kind),
            retryable,
        }
    }
}

impl fmt::Display for TelegramReplyDeliveryExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self.kind {
            TelegramReplyDeliveryExecutionErrorKind::InvalidRequest => {
                "Telegram reply delivery execution request is invalid."
            }
            TelegramReplyDeliveryExecutionErrorKind::EmptyReply => {
                "Telegram reply delivery received an empty assistant reply."
            }
            TelegramReplyDeliveryExecutionErrorKind::Planner => {
                "Telegram reply delivery planning failed."
            }
            TelegramReplyDeliveryExecutionErrorKind::PlannerContract => {
                "Telegram reply delivery planner contract is invalid."
            }
            TelegramReplyDeliveryExecutionErrorKind::Message => {
                "Telegram reply message delivery failed."
            }
            TelegramReplyDeliveryExecutionErrorKind::MessageContract => {
                "Telegram reply message delivery contract is invalid."
            }
            TelegramReplyDeliveryExecutionErrorKind::Attachment => {
                "Telegram reply attachment delivery failed."
            }
            TelegramReplyDeliveryExecutionErrorKind::AttachmentContract => {
                "Telegram reply attachment delivery contract is invalid."
            }
            TelegramReplyDeliveryExecutionErrorKind::ResultPlanner => {
                "Telegram reply delivery result planning failed."
            }
            TelegramReplyDeliveryExecutionErrorKind::ResultContract => {
                "Telegram reply delivery result contract is invalid."
            }
        })
    }
}

impl std::error::Error for TelegramReplyDeliveryExecutionError {}

pub fn agent_telegram_reply_delivery_execute_json(
    request: &JsonValue,
) -> Result<JsonValue, TelegramReplyDeliveryExecutionError> {
    execute_with_telegram_reply_delivery_ports(
        &DefaultTelegramReplyDeliveryPlanner,
        &NativeTelegramReplyDeliveryPort,
        request,
    )
}

pub fn execute_with_telegram_reply_delivery_ports<P, E>(
    planner: &P,
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, TelegramReplyDeliveryExecutionError>
where
    P: TelegramReplyDeliveryPlanner + ?Sized,
    E: TelegramReplyDeliveryPort + ?Sized,
{
    let source = request.as_object().ok_or_else(|| error(InvalidRequest))?;
    let expected = ExpectedDelivery::parse(source)?;
    let mut planner_request = source.clone();
    planner_request.insert("stage".to_string(), json!("request"));
    let planned = planner
        .plan_json(&JsonValue::Object(planner_request))
        .map_err(|_| error(Planner))?;
    let plan = validate_request_plan(&planned, &expected)?;

    if !plan.should_execute {
        if expected.operations.is_empty() {
            return Err(error(EmptyReply));
        }
        return Ok(outcome("skipped", false, expected.operations.len(), 0));
    }

    let mut operation_results = Vec::with_capacity(expected.operations.len());
    let mut failure = None;
    for (index, operation) in expected.operations.iter().enumerate() {
        let disposition = execute_operation(executor, source, operation);
        match disposition {
            Ok(()) => operation_results.push(json!({
                "index": index,
                "kind": operation.kind.code(),
                "ok": true,
            })),
            Err(kind) => {
                operation_results.push(json!({
                    "index": index,
                    "kind": operation.kind.code(),
                    "ok": false,
                    "error": "Telegram reply delivery operation failed.",
                }));
                failure = Some(TelegramReplyDeliveryExecutionError::operation(
                    kind.kind,
                    index,
                    operation.kind,
                    kind.retryable,
                ));
                break;
            }
        }
    }

    validate_result_plan(
        planner,
        plan.assistant_sequence,
        plan.through_sequence,
        &operation_results,
        failure.is_none(),
    )?;

    if let Some(failure) = failure {
        return Err(failure);
    }

    Ok(outcome(
        "completed",
        true,
        expected.operations.len(),
        operation_results.len(),
    ))
}

use TelegramReplyDeliveryExecutionErrorKind::*;

fn error(kind: TelegramReplyDeliveryExecutionErrorKind) -> TelegramReplyDeliveryExecutionError {
    TelegramReplyDeliveryExecutionError::new(kind)
}

struct ExpectedDelivery {
    execution_request: JsonValue,
    chat_id: JsonValue,
    assistant_event: JsonValue,
    assistant_sequence: i64,
    through_sequence: i64,
    reply_text: String,
    attachments: Vec<JsonValue>,
    requested_should_execute: bool,
    operations: Vec<ExpectedOperation>,
}

impl ExpectedDelivery {
    fn parse(source: &Map<String, JsonValue>) -> Result<Self, TelegramReplyDeliveryExecutionError> {
        let execution_request = execution_request_source(source);
        let execution_object = execution_request.as_object();
        let chat_id = execution_object
            .and_then(|request| request.get("chat_id"))
            .or_else(|| source.get("chat_id"))
            .cloned()
            .ok_or_else(|| error(InvalidRequest))?;
        if !valid_chat_id(&chat_id) {
            return Err(error(InvalidRequest));
        }

        let nested_event = object_field(execution_object, "assistant_event");
        let assistant_event = if nested_event.as_object().is_none_or(Map::is_empty) {
            object_field(Some(source), "assistant_event")
        } else {
            nested_event
        };
        if assistant_event.to_string().len() > MAX_ASSISTANT_EVENT_BYTES {
            return Err(error(InvalidRequest));
        }
        let assistant_sequence = optional_i64(
            execution_object
                .and_then(|request| request.get("assistant_sequence"))
                .or_else(|| source.get("assistant_sequence"))
                .or_else(|| {
                    assistant_event
                        .as_object()
                        .and_then(|event| event.get("sequence"))
                }),
        )
        .unwrap_or(0);
        let through_sequence = optional_i64(
            execution_object
                .and_then(|request| request.get("through_sequence"))
                .or_else(|| source.get("through_sequence")),
        )
        .unwrap_or(assistant_sequence);
        let reply_text = clean_text(source.get("reply_text"))
            .unwrap_or_else(|| assistant_reply_text(&assistant_event));
        if reply_text.contains('\0') || reply_text.chars().count() > MAX_REPLY_CHARS {
            return Err(error(InvalidRequest));
        }
        let attachments = assistant_reply_attachments(&assistant_event);
        if attachments.len() > MAX_ATTACHMENTS {
            return Err(error(InvalidRequest));
        }
        let requested_should_execute =
            optional_bool(source.get("should_execute")).unwrap_or_else(|| {
                execution_object
                    .and_then(|request| optional_bool(request.get("should_execute")))
                    .unwrap_or(true)
            });
        let operations = expected_operations(&chat_id, &reply_text, &attachments);

        Ok(Self {
            execution_request,
            chat_id,
            assistant_event,
            assistant_sequence,
            through_sequence,
            reply_text,
            attachments,
            requested_should_execute,
            operations,
        })
    }
}

struct ExpectedOperation {
    kind: TelegramReplyDeliveryOperationKind,
    chat_id: JsonValue,
    text: Option<String>,
    attachment_index: Option<usize>,
    attachment: Option<JsonValue>,
}

fn expected_operations(
    chat_id: &JsonValue,
    reply_text: &str,
    attachments: &[JsonValue],
) -> Vec<ExpectedOperation> {
    let mut operations =
        Vec::with_capacity(usize::from(!reply_text.is_empty()) + attachments.len());
    if !reply_text.is_empty() {
        operations.push(ExpectedOperation {
            kind: TelegramReplyDeliveryOperationKind::Message,
            chat_id: chat_id.clone(),
            text: Some(reply_text.to_string()),
            attachment_index: None,
            attachment: None,
        });
    }
    operations.extend(attachments.iter().enumerate().map(|(index, attachment)| {
        ExpectedOperation {
            kind: attachment_operation_kind(attachment),
            chat_id: chat_id.clone(),
            text: None,
            attachment_index: Some(index),
            attachment: Some(attachment.clone()),
        }
    }));
    operations
}

struct ValidatedPlan {
    should_execute: bool,
    assistant_sequence: i64,
    through_sequence: i64,
}

fn validate_request_plan(
    planned: &JsonValue,
    expected: &ExpectedDelivery,
) -> Result<ValidatedPlan, TelegramReplyDeliveryExecutionError> {
    let plan = planned.as_object().ok_or_else(|| error(PlannerContract))?;
    if text_is_not(plan.get("stage"), "request")
        || text_is_not(plan.get("execution_kind"), "reply_delivery")
        || text_is_not(plan.get("delivery_kind"), "telegram_assistant_reply")
        || plan.get("expects_result").and_then(JsonValue::as_bool) != Some(true)
    {
        return Err(error(PlannerContract));
    }
    let should_execute = plan
        .get("should_execute")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(PlannerContract))?;
    let expected_should_execute =
        expected.requested_should_execute && !expected.operations.is_empty();
    if should_execute != expected_should_execute {
        return Err(error(PlannerContract));
    }
    let request = plan
        .get("request")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| error(PlannerContract))?;
    if text_is_not(request.get("execution_kind"), "reply_delivery")
        || text_is_not(request.get("delivery_kind"), "telegram_assistant_reply")
        || text_is_not(request.get("callback_group"), "reply_delivery")
        || text_is_not(request.get("operation"), "deliver_assistant_reply")
        || request.get("execution_request") != Some(&expected.execution_request)
        || request.get("chat_id") != Some(&expected.chat_id)
        || request.get("assistant_event") != Some(&expected.assistant_event)
        || request
            .get("assistant_sequence")
            .and_then(JsonValue::as_i64)
            != Some(expected.assistant_sequence)
        || request.get("through_sequence").and_then(JsonValue::as_i64)
            != Some(expected.through_sequence)
        || request.get("reply_text").and_then(JsonValue::as_str)
            != Some(expected.reply_text.as_str())
        || request.get("attachments").and_then(JsonValue::as_array) != Some(&expected.attachments)
        || request.get("attachment_count").and_then(JsonValue::as_u64)
            != Some(expected.attachments.len() as u64)
        || request.get("operation_count").and_then(JsonValue::as_u64)
            != Some(expected.operations.len() as u64)
    {
        return Err(error(PlannerContract));
    }
    let request_ok = request
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| error(PlannerContract))?;
    let expected_ok = !expected.operations.is_empty();
    if request_ok != expected_ok
        || (request_ok && !request.get("error").is_none_or(JsonValue::is_null))
        || (!request_ok && clean_text(request.get("error")).is_none())
    {
        return Err(error(PlannerContract));
    }
    let operations = request
        .get("operations")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| error(PlannerContract))?;
    if operations.len() != expected.operations.len() {
        return Err(error(PlannerContract));
    }
    for (operation, expected_operation) in operations.iter().zip(&expected.operations) {
        validate_planned_operation(operation, expected_operation)?;
    }
    Ok(ValidatedPlan {
        should_execute,
        assistant_sequence: expected.assistant_sequence,
        through_sequence: expected.through_sequence,
    })
}

fn validate_planned_operation(
    operation: &JsonValue,
    expected: &ExpectedOperation,
) -> Result<(), TelegramReplyDeliveryExecutionError> {
    let operation = operation
        .as_object()
        .ok_or_else(|| error(PlannerContract))?;
    if text_is_not(operation.get("kind"), expected.kind.code())
        || text_is_not(operation.get("method"), expected.kind.method())
        || operation.get("chat_id") != Some(&expected.chat_id)
    {
        return Err(error(PlannerContract));
    }
    match expected.kind {
        TelegramReplyDeliveryOperationKind::Message => {
            if operation.get("text").and_then(JsonValue::as_str) != expected.text.as_deref()
                || operation.contains_key("attachment")
                || operation.contains_key("attachment_index")
                || operation.contains_key("file_field")
            {
                return Err(error(PlannerContract));
            }
        }
        _ => {
            if operation.get("file_field").and_then(JsonValue::as_str) != expected.kind.file_field()
                || operation
                    .get("attachment_index")
                    .and_then(JsonValue::as_u64)
                    != expected.attachment_index.map(|value| value as u64)
                || operation.get("attachment") != expected.attachment.as_ref()
                || operation.contains_key("text")
            {
                return Err(error(PlannerContract));
            }
        }
    }
    Ok(())
}

struct OperationFailure {
    kind: TelegramReplyDeliveryExecutionErrorKind,
    retryable: bool,
}

impl OperationFailure {
    fn terminal(kind: TelegramReplyDeliveryExecutionErrorKind) -> Self {
        Self {
            kind,
            retryable: false,
        }
    }
}

fn execute_operation<E>(
    executor: &E,
    source: &Map<String, JsonValue>,
    operation: &ExpectedOperation,
) -> Result<(), OperationFailure>
where
    E: TelegramReplyDeliveryPort + ?Sized,
{
    match operation.kind {
        TelegramReplyDeliveryOperationKind::Message => {
            let mut request = transport_config(source);
            request.insert("chat_id".to_string(), operation.chat_id.clone());
            request.insert(
                "text".to_string(),
                json!(operation.text.as_deref().unwrap_or_default()),
            );
            request.insert(
                "reply_markdown_enabled".to_string(),
                json!(source
                    .get("reply_markdown_enabled")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)),
            );
            let outcome = executor
                .execute_message(&JsonValue::Object(request))
                .map_err(|_| OperationFailure::terminal(Message))?;
            validate_message_outcome(&outcome)
        }
        _ => {
            let mut request = transport_config(source);
            request.insert("operation".to_string(), json!("send_attachment"));
            request.insert("chat_id".to_string(), operation.chat_id.clone());
            request.insert("method_name".to_string(), json!(operation.kind.method()));
            request.insert(
                "file_field".to_string(),
                json!(operation.kind.file_field().unwrap_or("document")),
            );
            request.insert(
                "attachment".to_string(),
                operation.attachment.clone().unwrap_or_else(|| json!({})),
            );
            let outcome = executor
                .execute_attachment(&JsonValue::Object(request))
                .map_err(|_| OperationFailure::terminal(Attachment))?;
            validate_attachment_outcome(&outcome, operation.kind.method())
        }
    }
}

fn validate_message_outcome(outcome: &JsonValue) -> Result<(), OperationFailure> {
    let object = outcome
        .as_object()
        .ok_or_else(|| OperationFailure::terminal(MessageContract))?;
    if text_is_not(object.get("contract"), MESSAGE_CONTRACT)
        || text_is_not(object.get("migration_stage"), MESSAGE_MIGRATION_STAGE)
        || text_is_not(object.get("stage"), "execute")
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
        return Err(OperationFailure::terminal(MessageContract));
    }
    let ok = object
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| OperationFailure::terminal(MessageContract))?;
    if object.get("completed").and_then(JsonValue::as_bool) != Some(ok)
        || safe_message_state(object.get("message_delivery_state")).is_none()
    {
        return Err(OperationFailure::terminal(MessageContract));
    }
    let chunk_count = bounded_usize(object.get("chunk_count"), 128)
        .ok_or_else(|| OperationFailure::terminal(MessageContract))?;
    let completed = bounded_usize(object.get("completed_chunk_count"), 128)
        .ok_or_else(|| OperationFailure::terminal(MessageContract))?;
    if completed > chunk_count {
        return Err(OperationFailure::terminal(MessageContract));
    }
    if ok {
        if clean_text(object.get("message_delivery_state")).as_deref() != Some("completed")
            || chunk_count == 0
            || completed != chunk_count
            || !object
                .get("failed_chunk_index")
                .is_none_or(JsonValue::is_null)
            || !object.get("error_kind").is_none_or(JsonValue::is_null)
            || !object.get("error").is_none_or(JsonValue::is_null)
        {
            return Err(OperationFailure::terminal(MessageContract));
        }
        Ok(())
    } else {
        let error_kind = safe_message_error_kind(object.get("error_kind"))
            .ok_or_else(|| OperationFailure::terminal(MessageContract))?;
        if clean_text(object.get("message_delivery_state")).as_deref() == Some("completed") {
            return Err(OperationFailure::terminal(MessageContract));
        }
        Err(OperationFailure {
            kind: Message,
            retryable: retryable_transport_kind(error_kind),
        })
    }
}

fn validate_attachment_outcome(outcome: &JsonValue, method: &str) -> Result<(), OperationFailure> {
    let object = outcome
        .as_object()
        .ok_or_else(|| OperationFailure::terminal(AttachmentContract))?;
    if text_is_not(object.get("contract"), API_CONTRACT)
        || text_is_not(object.get("migration_stage"), API_MIGRATION_STAGE)
        || text_is_not(object.get("stage"), "execute")
        || text_is_not(object.get("operation"), "send_attachment")
        || text_is_not(object.get("telegram_method"), method)
        || !false_flags(
            object,
            &[
                "python_telegram_api_allowed",
                "python_http_execution_allowed",
                "python_retry_allowed",
                "raw_telegram_payload_exposed",
                "token_bearing_url_exposed",
                "downloaded_bytes_exposed",
                "local_path_exposed",
                "multipart_fields_exposed",
                "file_name_exposed",
            ],
        )
        || object.get("downloaded").and_then(JsonValue::as_bool) != Some(false)
    {
        return Err(OperationFailure::terminal(AttachmentContract));
    }
    let ok = object
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or_else(|| OperationFailure::terminal(AttachmentContract))?;
    if object.get("completed").and_then(JsonValue::as_bool) != Some(ok)
        || object.get("sent").and_then(JsonValue::as_bool) != Some(ok)
        || safe_api_state(object.get("telegram_api_state")).is_none()
        || bounded_usize(object.get("attempts"), 3).is_none()
    {
        return Err(OperationFailure::terminal(AttachmentContract));
    }
    if ok {
        if clean_text(object.get("telegram_api_state")).as_deref() != Some("completed")
            || !matches!(
                clean_text(object.get("transport")).as_deref(),
                Some("json" | "multipart")
            )
            || object.get("attempts").and_then(JsonValue::as_u64) == Some(0)
            || !object.get("error_kind").is_none_or(JsonValue::is_null)
            || !object.get("error").is_none_or(JsonValue::is_null)
        {
            return Err(OperationFailure::terminal(AttachmentContract));
        }
        Ok(())
    } else {
        let error_kind = safe_api_error_kind(object.get("error_kind"))
            .ok_or_else(|| OperationFailure::terminal(AttachmentContract))?;
        if clean_text(object.get("telegram_api_state")).as_deref() == Some("completed") {
            return Err(OperationFailure::terminal(AttachmentContract));
        }
        Err(OperationFailure {
            kind: Attachment,
            retryable: retryable_transport_kind(error_kind),
        })
    }
}

fn validate_result_plan<P>(
    planner: &P,
    assistant_sequence: i64,
    through_sequence: i64,
    operation_results: &[JsonValue],
    success: bool,
) -> Result<(), TelegramReplyDeliveryExecutionError>
where
    P: TelegramReplyDeliveryPlanner + ?Sized,
{
    let planned = planner
        .plan_json(&json!({
            "stage": "result",
            "callback_result": {
                "assistant_sequence": assistant_sequence,
                "through_sequence": through_sequence,
                "operation_count": operation_results.len(),
                "operation_results": operation_results,
            },
        }))
        .map_err(|_| error(ResultPlanner))?;
    let plan = planned.as_object().ok_or_else(|| error(ResultContract))?;
    let result = plan
        .get("result")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| error(ResultContract))?;
    let delivered_count = operation_results
        .iter()
        .filter(|value| value.get("ok").and_then(JsonValue::as_bool) == Some(true))
        .count();
    let failed_count = operation_results.len() - delivered_count;
    if text_is_not(plan.get("stage"), "result")
        || text_is_not(plan.get("execution_kind"), "reply_delivery")
        || text_is_not(plan.get("delivery_kind"), "telegram_assistant_reply")
        || plan.get("completed").and_then(JsonValue::as_bool) != Some(success)
        || text_is_not(result.get("execution_kind"), "reply_delivery")
        || text_is_not(result.get("delivery_kind"), "telegram_assistant_reply")
        || result.get("ok").and_then(JsonValue::as_bool) != Some(success)
        || result.get("delivered").and_then(JsonValue::as_bool) != Some(success)
        || result.get("assistant_sequence").and_then(JsonValue::as_i64) != Some(assistant_sequence)
        || result.get("through_sequence").and_then(JsonValue::as_i64) != Some(through_sequence)
        || result
            .get("operation_results")
            .and_then(JsonValue::as_array)
            .is_none_or(|values| values.as_slice() != operation_results)
        || result.get("operation_count").and_then(JsonValue::as_u64)
            != Some(operation_results.len() as u64)
        || result
            .get("delivered_operation_count")
            .and_then(JsonValue::as_u64)
            != Some(delivered_count as u64)
        || result
            .get("failed_operation_count")
            .and_then(JsonValue::as_u64)
            != Some(failed_count as u64)
        || (success && !result.get("error").is_none_or(JsonValue::is_null))
        || (!success
            && clean_text(result.get("error")).as_deref()
                != Some("Telegram reply delivery operation failed."))
    {
        return Err(error(ResultContract));
    }
    Ok(())
}

fn outcome(state: &str, delivered: bool, operation_count: usize, attempted: usize) -> JsonValue {
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "reply_delivery_state": state,
        "ok": delivered,
        "completed": delivered,
        "delivered": delivered,
        "operation_count": operation_count,
        "attempted_operation_count": attempted,
        "delivered_operation_count": if delivered { attempted } else { 0 },
        "failed_operation_count": 0,
        "failed_operation_index": JsonValue::Null,
        "failed_operation_kind": JsonValue::Null,
        "error_kind": JsonValue::Null,
        "error": JsonValue::Null,
        "python_reply_delivery_allowed": false,
        "python_message_delivery_allowed": false,
        "python_attachment_delivery_allowed": false,
        "raw_planner_result_exposed": false,
        "raw_executor_result_exposed": false,
        "bot_token_exposed": false,
        "chat_id_exposed": false,
        "reply_text_exposed": false,
        "attachment_exposed": false,
        "telegram_description_exposed": false,
        "local_path_exposed": false,
    })
}

fn transport_config(source: &Map<String, JsonValue>) -> Map<String, JsonValue> {
    let mut request = Map::new();
    for key in ["bot_token", "token", "base_url", "request_timeout_seconds"] {
        if let Some(value) = source.get(key) {
            request.insert(key.to_string(), value.clone());
        }
    }
    request
}

fn execution_request_source(source: &Map<String, JsonValue>) -> JsonValue {
    for key in [
        "execution_request",
        "callback_execution_request",
        "request",
        "adapter_request",
    ] {
        if let Some(value) = source.get(key) {
            return value
                .as_object()
                .map(|object| JsonValue::Object(object.clone()))
                .unwrap_or_else(|| json!({}));
        }
    }
    json!({})
}

fn object_field(source: Option<&Map<String, JsonValue>>, key: &str) -> JsonValue {
    source
        .and_then(|object| object.get(key))
        .and_then(JsonValue::as_object)
        .map(|object| JsonValue::Object(object.clone()))
        .unwrap_or_else(|| json!({}))
}

fn assistant_reply_text(event: &JsonValue) -> String {
    let payload = event
        .as_object()
        .and_then(|event| event.get("payload"))
        .and_then(JsonValue::as_object);
    payload
        .and_then(|payload| payload.get("transport_reply_envelope"))
        .and_then(JsonValue::as_object)
        .and_then(|envelope| envelope.get("message"))
        .and_then(JsonValue::as_object)
        .and_then(|message| clean_text(message.get("text")))
        .or_else(|| payload.and_then(|payload| clean_text(payload.get("text"))))
        .unwrap_or_default()
}

fn assistant_reply_attachments(event: &JsonValue) -> Vec<JsonValue> {
    event
        .as_object()
        .and_then(|event| event.get("payload"))
        .and_then(JsonValue::as_object)
        .and_then(|payload| payload.get("transport_reply_envelope"))
        .and_then(JsonValue::as_object)
        .and_then(|envelope| envelope.get("message"))
        .and_then(JsonValue::as_object)
        .and_then(|message| message.get("attachments"))
        .and_then(JsonValue::as_array)
        .map(|attachments| {
            attachments
                .iter()
                .filter(|attachment| attachment.is_object())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn attachment_operation_kind(attachment: &JsonValue) -> TelegramReplyDeliveryOperationKind {
    let kind = lower_attachment_text(attachment, "kind");
    let mime_type = lower_attachment_text(attachment, "mime_type");
    let suffix = attachment_filename_suffix(attachment);
    let audio_suffix = matches!(
        suffix.as_str(),
        ".aac"
            | ".aif"
            | ".aiff"
            | ".alac"
            | ".flac"
            | ".m4a"
            | ".mp3"
            | ".ogg"
            | ".opus"
            | ".wav"
            | ".wma"
    );
    if kind == "audio" || (kind != "document" && (mime_type.starts_with("audio/") || audio_suffix))
    {
        return TelegramReplyDeliveryOperationKind::Audio;
    }
    let photo_suffix = matches!(suffix.as_str(), ".jpg" | ".jpeg" | ".png" | ".webp");
    if kind != "document"
        && (kind == "photo"
            || kind == "image"
            || (mime_type.starts_with("image/") && mime_type != "image/gif")
            || photo_suffix)
    {
        return TelegramReplyDeliveryOperationKind::Photo;
    }
    TelegramReplyDeliveryOperationKind::Document
}

fn lower_attachment_text(attachment: &JsonValue, key: &str) -> String {
    attachment
        .as_object()
        .and_then(|attachment| clean_text(attachment.get(key)))
        .unwrap_or_default()
        .to_ascii_lowercase()
}

fn attachment_filename_suffix(attachment: &JsonValue) -> String {
    let source = attachment
        .as_object()
        .and_then(|attachment| {
            clean_text(attachment.get("file_name"))
                .or_else(|| clean_text(attachment.get("local_path")))
        })
        .unwrap_or_default()
        .to_ascii_lowercase();
    match source.rsplit_once('.') {
        Some((_, suffix)) if !suffix.is_empty() => format!(".{suffix}"),
        _ => String::new(),
    }
}

fn valid_chat_id(value: &JsonValue) -> bool {
    match value {
        JsonValue::Number(number) => number.as_i64().is_some(),
        JsonValue::String(value) => {
            let value = value.trim();
            !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
        }
        _ => false,
    }
}

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" | "" => Some(false),
            _ => None,
        },
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => {
            let text = text.trim();
            if text.is_empty() {
                Some(0)
            } else {
                text.parse::<i64>().ok()
            }
        }
        JsonValue::Bool(true) => Some(1),
        JsonValue::Bool(false) => Some(0),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn bounded_usize(value: Option<&JsonValue>, max: usize) -> Option<usize> {
    value?
        .as_u64()
        .and_then(|value| usize::try_from(value).ok())
        .filter(|value| *value <= max)
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn text_is_not(value: Option<&JsonValue>, expected: &str) -> bool {
    clean_text(value).as_deref() != Some(expected)
}

fn false_flags(object: &Map<String, JsonValue>, flags: &[&str]) -> bool {
    flags
        .iter()
        .all(|flag| object.get(*flag).and_then(JsonValue::as_bool) == Some(false))
}

fn safe_message_state(value: Option<&JsonValue>) -> Option<&'static str> {
    match value.and_then(JsonValue::as_str) {
        Some("completed") => Some("completed"),
        Some("invalid_request") => Some("invalid_request"),
        Some("input_too_large") => Some("input_too_large"),
        Some("planner_failed") => Some("planner_failed"),
        Some("planner_contract_failed") => Some("planner_contract_failed"),
        Some("api_executor_failed") => Some("api_executor_failed"),
        Some("api_contract_failed") => Some("api_contract_failed"),
        Some("delivery_failed") => Some("delivery_failed"),
        _ => None,
    }
}

fn safe_message_error_kind(value: Option<&JsonValue>) -> Option<&'static str> {
    match value.and_then(JsonValue::as_str) {
        Some("contract") => Some("contract"),
        Some("executor") => Some("executor"),
        Some("planning") => Some("planning"),
        Some("telegram_api") => Some("telegram_api"),
        Some("timeout") => Some("timeout"),
        Some("transport") => Some("transport"),
        Some("http") => Some("http"),
        Some("url") => Some("url"),
        Some("invalid_timeout") => Some("invalid_timeout"),
        Some("response") => Some("response"),
        Some("sleep") => Some("sleep"),
        _ => None,
    }
}

fn safe_api_state(value: Option<&JsonValue>) -> Option<&'static str> {
    match value.and_then(JsonValue::as_str) {
        Some("completed") => Some("completed"),
        Some("planning_contract_failed") => Some("planning_contract_failed"),
        Some("planning_rejected") => Some("planning_rejected"),
        Some("unsupported_operation_or_transport") => Some("unsupported_operation_or_transport"),
        Some("file_read_failed") => Some("file_read_failed"),
        Some("attachment_too_large") => Some("attachment_too_large"),
        Some("executor_failed") => Some("executor_failed"),
        Some("http_contract_failed") => Some("http_contract_failed"),
        Some("retry_sleep_failed") => Some("retry_sleep_failed"),
        Some("retry_exhausted") => Some("retry_exhausted"),
        Some("http_failed") => Some("http_failed"),
        Some("result_contract_failed") => Some("result_contract_failed"),
        Some("telegram_api_failed") => Some("telegram_api_failed"),
        _ => None,
    }
}

fn safe_api_error_kind(value: Option<&JsonValue>) -> Option<&'static str> {
    match value.and_then(JsonValue::as_str) {
        Some("contract") => Some("contract"),
        Some("planning") => Some("planning"),
        Some("unsupported") => Some("unsupported"),
        Some("file") => Some("file"),
        Some("capacity") => Some("capacity"),
        Some("executor") => Some("executor"),
        Some("timeout") => Some("timeout"),
        Some("transport") => Some("transport"),
        Some("http") => Some("http"),
        Some("url") => Some("url"),
        Some("invalid_timeout") => Some("invalid_timeout"),
        Some("response") => Some("response"),
        Some("sleep") => Some("sleep"),
        Some("telegram_api") => Some("telegram_api"),
        _ => None,
    }
}

fn retryable_transport_kind(kind: &str) -> bool {
    matches!(kind, "timeout" | "transport" | "sleep")
}

#[cfg(test)]
mod tests;
