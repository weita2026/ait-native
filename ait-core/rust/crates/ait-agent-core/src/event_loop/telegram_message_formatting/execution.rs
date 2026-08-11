use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::telegram_api_method::agent_telegram_api_execute;

use super::planning::{DefaultTelegramMessageFormattingPlanner, TelegramMessageFormattingPlanner};

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramMessageDeliveryExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_message_delivery_execution";
const FORMAT_CONTRACT: &str = "ait_agent_core.event_loop.TelegramMessageFormatting.v1";
const API_CONTRACT: &str = "ait_agent_core.event_loop.TelegramApiTransportExecution.v1";
const API_MIGRATION_STAGE: &str = "rust_agent_telegram_transport_execution";
const MAX_TELEGRAM_MESSAGE_CHARS: usize = 3_800;
const MAX_MESSAGE_CHUNKS: usize = 128;
const MAX_INPUT_CHARS: usize = MAX_TELEGRAM_MESSAGE_CHARS * MAX_MESSAGE_CHUNKS;

pub trait TelegramMessageDeliveryApiPort {
    fn execute_send_message(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTelegramMessageDeliveryApiPort;

impl TelegramMessageDeliveryApiPort for NativeTelegramMessageDeliveryApiPort {
    fn execute_send_message(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_telegram_api_execute(request).map(|execution| execution.metadata().clone())
    }
}

pub fn agent_telegram_message_delivery_execute_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    execute_with_telegram_message_delivery_ports(
        &DefaultTelegramMessageFormattingPlanner,
        &NativeTelegramMessageDeliveryApiPort,
        request,
    )
}

pub fn execute_with_telegram_message_delivery_ports<P, E>(
    planner: &P,
    api: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    P: TelegramMessageFormattingPlanner + ?Sized,
    E: TelegramMessageDeliveryApiPort + ?Sized,
{
    let object = request
        .as_object()
        .ok_or_else(|| "Telegram message delivery request must be an object.".to_string())?;
    let Some(chat_id) = object.get("chat_id").filter(|value| valid_chat_id(value)) else {
        return Ok(rejected_payload(
            "invalid_request",
            "Telegram message delivery requires a valid chat target.",
        ));
    };
    let text = match object.get("text") {
        None | Some(JsonValue::Null) => String::new(),
        Some(JsonValue::String(value)) => value.clone(),
        _ => {
            return Ok(rejected_payload(
                "invalid_request",
                "Telegram message delivery requires text.",
            ))
        }
    };
    if text.chars().count() > MAX_INPUT_CHARS || text.contains('\0') {
        return Ok(rejected_payload(
            "input_too_large",
            "Telegram message delivery input exceeded its Rust limit.",
        ));
    }
    let markdown_enabled = object
        .get("reply_markdown_enabled")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let chunk_limit = optional_usize(object.get("limit"))
        .unwrap_or(MAX_TELEGRAM_MESSAGE_CHARS)
        .clamp(1, MAX_TELEGRAM_MESSAGE_CHARS);
    let planned = match planner.plan_json(&json!({
        "kind": "message_chunks",
        "text": text,
        "reply_markdown_enabled": markdown_enabled,
        "limit": chunk_limit,
    })) {
        Ok(value) => value,
        Err(_) => {
            return Ok(rejected_payload(
                "planner_failed",
                "Telegram message formatting planner failed.",
            ))
        }
    };
    let chunks = match parse_chunks(&planned, markdown_enabled, chunk_limit) {
        Ok(chunks) => chunks,
        Err(error) => return Ok(rejected_payload("planner_contract_failed", error)),
    };

    let mut chunk_results = Vec::with_capacity(chunks.len());
    let mut completed_chunk_count = 0usize;
    let mut fallback_count = 0usize;
    let mut api_call_count = 0usize;

    for (index, chunk) in chunks.iter().enumerate() {
        api_call_count += 1;
        let initial = match execute_api_attempt(api, object, chat_id, &chunk.text, chunk.parse_mode)
        {
            Ok(attempt) => attempt,
            Err(ApiAttemptFailure::Executor) => {
                chunk_results.push(failed_chunk_result(
                    index,
                    false,
                    0,
                    None,
                    "api_executor_failed",
                    "executor",
                ));
                return Ok(delivery_payload(
                    "api_executor_failed",
                    false,
                    chunks.len(),
                    completed_chunk_count,
                    Some(index),
                    fallback_count,
                    api_call_count,
                    chunk_results,
                    Some("executor"),
                    Some("Telegram message API executor failed."),
                ));
            }
            Err(ApiAttemptFailure::Contract) => {
                chunk_results.push(failed_chunk_result(
                    index,
                    false,
                    0,
                    None,
                    "api_contract_failed",
                    "contract",
                ));
                return Ok(delivery_payload(
                    "api_contract_failed",
                    false,
                    chunks.len(),
                    completed_chunk_count,
                    Some(index),
                    fallback_count,
                    api_call_count,
                    chunk_results,
                    Some("contract"),
                    Some("Telegram message API result contract failed."),
                ));
            }
        };
        if initial.ok {
            completed_chunk_count += 1;
            chunk_results.push(successful_chunk_result(index, false, &initial));
            continue;
        }

        if chunk.parse_mode.is_some() && initial.telegram_parse_error {
            fallback_count += 1;
            api_call_count += 1;
            let fallback = match execute_api_attempt(api, object, chat_id, &chunk.plain_text, None)
            {
                Ok(attempt) => attempt,
                Err(ApiAttemptFailure::Executor) => {
                    chunk_results.push(failed_chunk_result(
                        index,
                        true,
                        initial.attempts,
                        initial.status_code,
                        "api_executor_failed",
                        "executor",
                    ));
                    return Ok(delivery_payload(
                        "api_executor_failed",
                        false,
                        chunks.len(),
                        completed_chunk_count,
                        Some(index),
                        fallback_count,
                        api_call_count,
                        chunk_results,
                        Some("executor"),
                        Some("Telegram plain fallback executor failed."),
                    ));
                }
                Err(ApiAttemptFailure::Contract) => {
                    chunk_results.push(failed_chunk_result(
                        index,
                        true,
                        initial.attempts,
                        initial.status_code,
                        "api_contract_failed",
                        "contract",
                    ));
                    return Ok(delivery_payload(
                        "api_contract_failed",
                        false,
                        chunks.len(),
                        completed_chunk_count,
                        Some(index),
                        fallback_count,
                        api_call_count,
                        chunk_results,
                        Some("contract"),
                        Some("Telegram plain fallback result contract failed."),
                    ));
                }
            };
            if fallback.ok {
                completed_chunk_count += 1;
                chunk_results.push(json!({
                    "index": index,
                    "delivered": true,
                    "fallback_used": true,
                    "api_call_count": 2,
                    "attempt_count": initial.attempts.saturating_add(fallback.attempts),
                    "http_status_code": fallback.status_code.map(JsonValue::from).unwrap_or(JsonValue::Null),
                    "state": fallback.state,
                    "error_kind": JsonValue::Null,
                }));
                continue;
            }
            let error_kind = fallback.error_kind.unwrap_or("telegram_api");
            chunk_results.push(failed_chunk_result(
                index,
                true,
                initial.attempts.saturating_add(fallback.attempts),
                fallback.status_code,
                fallback.state,
                error_kind,
            ));
            return Ok(delivery_payload(
                "delivery_failed",
                false,
                chunks.len(),
                completed_chunk_count,
                Some(index),
                fallback_count,
                api_call_count,
                chunk_results,
                Some(error_kind),
                Some("Telegram plain fallback delivery failed."),
            ));
        }

        let error_kind = initial.error_kind.unwrap_or("telegram_api");
        chunk_results.push(failed_chunk_result(
            index,
            false,
            initial.attempts,
            initial.status_code,
            initial.state,
            error_kind,
        ));
        return Ok(delivery_payload(
            "delivery_failed",
            false,
            chunks.len(),
            completed_chunk_count,
            Some(index),
            fallback_count,
            api_call_count,
            chunk_results,
            Some(error_kind),
            Some("Telegram message delivery failed."),
        ));
    }

    Ok(delivery_payload(
        "completed",
        true,
        chunks.len(),
        completed_chunk_count,
        None,
        fallback_count,
        api_call_count,
        chunk_results,
        None,
        None,
    ))
}

#[derive(Debug, Clone)]
struct PlannedChunk {
    text: String,
    plain_text: String,
    parse_mode: Option<&'static str>,
}

fn parse_chunks(
    planned: &JsonValue,
    markdown_enabled: bool,
    chunk_limit: usize,
) -> Result<Vec<PlannedChunk>, &'static str> {
    let object = planned
        .as_object()
        .ok_or("Telegram message formatting planner returned an invalid result.")?;
    if clean_text(object.get("migration_stage")).as_deref()
        != Some("rust_agent_telegram_message_formatting")
        || clean_text(object.get("message_format_contract")).as_deref() != Some(FORMAT_CONTRACT)
        || clean_text(object.get("kind")).as_deref() != Some("message_chunks")
        || clean_text(object.get("transport")).as_deref() != Some("telegram")
        || object
            .get("rust_event_loop_required")
            .and_then(JsonValue::as_bool)
            != Some(true)
        || object
            .get("python_message_formatting_allowed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err("Telegram message formatting planner returned an invalid identity.");
    }
    let chunks = object
        .get("chunks")
        .and_then(JsonValue::as_array)
        .ok_or("Telegram message formatting planner omitted chunks.")?;
    if chunks.is_empty() || chunks.len() > MAX_MESSAGE_CHUNKS {
        return Err("Telegram message formatting planner returned an invalid chunk count.");
    }
    chunks
        .iter()
        .map(|chunk| parse_chunk(chunk, markdown_enabled, chunk_limit))
        .collect()
}

fn parse_chunk(
    chunk: &JsonValue,
    markdown_enabled: bool,
    chunk_limit: usize,
) -> Result<PlannedChunk, &'static str> {
    let object = chunk
        .as_object()
        .ok_or("Telegram message formatting planner returned an invalid chunk.")?;
    let text = object
        .get("text")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Telegram message formatting planner returned invalid chunk text.")?;
    let plain_text = object
        .get("plain_text")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
        .ok_or("Telegram message formatting planner returned invalid plain text.")?;
    if text.contains('\0')
        || plain_text.contains('\0')
        || text.chars().count() > chunk_limit
        || plain_text.chars().count() > chunk_limit
    {
        return Err("Telegram message formatting planner returned an oversized chunk.");
    }
    let parse_mode = match object.get("parse_mode") {
        None | Some(JsonValue::Null) => None,
        Some(JsonValue::String(value)) if value == "HTML" => Some("HTML"),
        _ => return Err("Telegram message formatting planner returned an invalid parse mode."),
    };
    if markdown_enabled != parse_mode.is_some() || (parse_mode.is_none() && text != plain_text) {
        return Err("Telegram message formatting planner returned an inconsistent chunk.");
    }
    Ok(PlannedChunk {
        text: text.to_string(),
        plain_text: plain_text.to_string(),
        parse_mode,
    })
}

#[derive(Debug, Clone, Copy)]
enum ApiAttemptFailure {
    Executor,
    Contract,
}

#[derive(Debug, Clone, Copy)]
struct ApiAttempt {
    ok: bool,
    telegram_parse_error: bool,
    attempts: u64,
    status_code: Option<i64>,
    state: &'static str,
    error_kind: Option<&'static str>,
}

fn execute_api_attempt<E>(
    api: &E,
    source: &Map<String, JsonValue>,
    chat_id: &JsonValue,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<ApiAttempt, ApiAttemptFailure>
where
    E: TelegramMessageDeliveryApiPort + ?Sized,
{
    let request = api_request(source, chat_id, text, parse_mode);
    let outcome = api
        .execute_send_message(&request)
        .map_err(|_| ApiAttemptFailure::Executor)?;
    parse_api_attempt(&outcome)
}

fn api_request(
    source: &Map<String, JsonValue>,
    chat_id: &JsonValue,
    text: &str,
    parse_mode: Option<&str>,
) -> JsonValue {
    let mut request = Map::new();
    request.insert("operation".to_string(), json!("send_message"));
    request.insert("chat_id".to_string(), chat_id.clone());
    request.insert("text".to_string(), json!(text));
    if let Some(parse_mode) = parse_mode {
        request.insert("parse_mode".to_string(), json!(parse_mode));
    }
    for key in ["bot_token", "token", "base_url", "request_timeout_seconds"] {
        if let Some(value) = source.get(key) {
            request.insert(key.to_string(), value.clone());
        }
    }
    JsonValue::Object(request)
}

fn parse_api_attempt(outcome: &JsonValue) -> Result<ApiAttempt, ApiAttemptFailure> {
    let object = outcome.as_object().ok_or(ApiAttemptFailure::Contract)?;
    if clean_text(object.get("contract")).as_deref() != Some(API_CONTRACT)
        || clean_text(object.get("migration_stage")).as_deref() != Some(API_MIGRATION_STAGE)
        || clean_text(object.get("stage")).as_deref() != Some("execute")
        || clean_text(object.get("operation")).as_deref() != Some("send_message")
        || clean_text(object.get("telegram_method")).as_deref() != Some("sendMessage")
        || clean_text(object.get("transport")).as_deref() != Some("json")
        || object.get("downloaded").and_then(JsonValue::as_bool) != Some(false)
        || object
            .get("downloaded_bytes_exposed")
            .and_then(JsonValue::as_bool)
            != Some(false)
        || object
            .get("token_bearing_url_exposed")
            .and_then(JsonValue::as_bool)
            != Some(false)
    {
        return Err(ApiAttemptFailure::Contract);
    }
    let ok = object
        .get("ok")
        .and_then(JsonValue::as_bool)
        .ok_or(ApiAttemptFailure::Contract)?;
    if object.get("completed").and_then(JsonValue::as_bool) != Some(ok)
        || object.get("sent").and_then(JsonValue::as_bool) != Some(ok)
    {
        return Err(ApiAttemptFailure::Contract);
    }
    let telegram_parse_error = object
        .get("telegram_parse_error")
        .and_then(JsonValue::as_bool)
        .ok_or(ApiAttemptFailure::Contract)?;
    if ok && telegram_parse_error {
        return Err(ApiAttemptFailure::Contract);
    }
    let attempts = object
        .get("attempts")
        .and_then(JsonValue::as_u64)
        .filter(|attempts| *attempts <= 3)
        .ok_or(ApiAttemptFailure::Contract)?;
    let status_code = api_status_code(object.get("http_status_code"))?;
    if status_code.is_some_and(|code| !(100..600).contains(&code)) {
        return Err(ApiAttemptFailure::Contract);
    }
    let state =
        safe_api_state(object.get("telegram_api_state")).ok_or(ApiAttemptFailure::Contract)?;
    let error_kind = if ok {
        if !object.get("error_kind").is_none_or(JsonValue::is_null) {
            return Err(ApiAttemptFailure::Contract);
        }
        None
    } else {
        Some(safe_api_error_kind(object.get("error_kind")).ok_or(ApiAttemptFailure::Contract)?)
    };
    if (ok
        && (state != "completed"
            || attempts == 0
            || !status_code.is_some_and(|code| (200..300).contains(&code))))
        || (!ok && state == "completed")
        || (telegram_parse_error
            && (state != "telegram_api_failed" || error_kind != Some("telegram_api")))
    {
        return Err(ApiAttemptFailure::Contract);
    }
    Ok(ApiAttempt {
        ok,
        telegram_parse_error,
        attempts,
        status_code,
        state,
        error_kind,
    })
}

fn safe_api_state(value: Option<&JsonValue>) -> Option<&'static str> {
    match value.and_then(JsonValue::as_str) {
        Some("completed") => Some("completed"),
        Some("planning_contract_failed") => Some("planning_contract_failed"),
        Some("planning_rejected") => Some("planning_rejected"),
        Some("unsupported_operation_or_transport") => Some("unsupported_operation_or_transport"),
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

fn successful_chunk_result(index: usize, fallback_used: bool, attempt: &ApiAttempt) -> JsonValue {
    json!({
        "index": index,
        "delivered": true,
        "fallback_used": fallback_used,
        "api_call_count": if fallback_used { 2 } else { 1 },
        "attempt_count": attempt.attempts,
        "http_status_code": attempt.status_code.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "state": attempt.state,
        "error_kind": JsonValue::Null,
    })
}

fn failed_chunk_result(
    index: usize,
    fallback_used: bool,
    attempts: u64,
    status_code: Option<i64>,
    state: &str,
    error_kind: &str,
) -> JsonValue {
    json!({
        "index": index,
        "delivered": false,
        "fallback_used": fallback_used,
        "api_call_count": if fallback_used { 2 } else { 1 },
        "attempt_count": attempts,
        "http_status_code": status_code.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "state": state,
        "error_kind": error_kind,
    })
}

#[allow(clippy::too_many_arguments)]
fn delivery_payload(
    state: &str,
    ok: bool,
    chunk_count: usize,
    completed_chunk_count: usize,
    failed_chunk_index: Option<usize>,
    fallback_count: usize,
    api_call_count: usize,
    chunk_results: Vec<JsonValue>,
    error_kind: Option<&str>,
    error: Option<&str>,
) -> JsonValue {
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "message_delivery_state": state,
        "ok": ok,
        "completed": ok,
        "chunk_count": chunk_count,
        "completed_chunk_count": completed_chunk_count,
        "failed_chunk_index": failed_chunk_index.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "fallback_count": fallback_count,
        "api_call_count": api_call_count,
        "chunk_results": chunk_results,
        "error_kind": error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "error": error.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "python_message_delivery_allowed": false,
        "python_message_formatting_allowed": false,
        "raw_api_result_exposed": false,
        "telegram_description_exposed": false,
        "token_bearing_url_exposed": false,
        "chat_id_exposed": false,
        "formatted_text_exposed": false,
        "plain_text_exposed": false,
    })
}

fn rejected_payload(state: &str, error: &str) -> JsonValue {
    delivery_payload(
        state,
        false,
        0,
        0,
        None,
        0,
        0,
        Vec::new(),
        Some("contract"),
        Some(error),
    )
}

fn valid_chat_id(value: &JsonValue) -> bool {
    match value {
        JsonValue::Number(number) => number.as_i64().is_some(),
        JsonValue::String(value) => {
            let value = value.trim();
            !value.is_empty() && value.len() <= 128 && !value.chars().any(|ch| ch.is_control())
        }
        _ => false,
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    match value? {
        JsonValue::Number(value) => value.as_u64().and_then(|value| usize::try_from(value).ok()),
        JsonValue::String(value) => value.trim().parse::<usize>().ok(),
        _ => None,
    }
}

fn api_status_code(value: Option<&JsonValue>) -> Result<Option<i64>, ApiAttemptFailure> {
    match value {
        Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Number(value)) => {
            value.as_i64().map(Some).ok_or(ApiAttemptFailure::Contract)
        }
        _ => Err(ApiAttemptFailure::Contract),
    }
}

#[cfg(test)]
mod tests;
