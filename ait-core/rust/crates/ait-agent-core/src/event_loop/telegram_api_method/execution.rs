use std::thread;
use std::time::Duration;

use ait_core::json_support::{json, JsonMap as Map, JsonValue};

use crate::event_loop::telegram_message_formatting::agent_telegram_message_formatting_plan_json;
use crate::json_support::parse_value;
use crate::transport::{
    agent_transport_http_execute_json_request_json, agent_transport_retry_delay_seconds,
};

use super::agent_telegram_api_method_execution_plan_json;

const CONTRACT: &str = "ait_agent_core.event_loop.TelegramApiJsonExecution.v1";
const MIGRATION_STAGE: &str = "rust_agent_telegram_json_api_execution";
const API_METHOD_EXECUTION_KIND: &str = "telegram_api_method";
const POLL_MAX_ATTEMPTS: u64 = 4;
const DELIVERY_MAX_ATTEMPTS: u64 = 3;
const RETRY_BASE_DELAY_SECONDS: f64 = 1.0;
const REDACTED: &str = "[redacted]";

pub trait TelegramApiJsonHttpExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

pub trait TelegramApiRetrySleeper {
    fn sleep_seconds(&self, seconds: f64) -> Result<(), String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NativeTelegramApiJsonHttpExecutor;

impl TelegramApiJsonHttpExecutor for NativeTelegramApiJsonHttpExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_transport_http_execute_json_request_json(request)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ThreadTelegramApiRetrySleeper;

impl TelegramApiRetrySleeper for ThreadTelegramApiRetrySleeper {
    fn sleep_seconds(&self, seconds: f64) -> Result<(), String> {
        thread::sleep(Duration::from_secs_f64(seconds));
        Ok(())
    }
}

pub fn agent_telegram_api_json_execute_json(request: &JsonValue) -> Result<JsonValue, String> {
    execute_with_telegram_api_json_ports(
        &NativeTelegramApiJsonHttpExecutor,
        &ThreadTelegramApiRetrySleeper,
        request,
    )
}

pub fn execute_with_telegram_api_json_ports<E, S>(
    executor: &E,
    sleeper: &S,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: TelegramApiJsonHttpExecutor + ?Sized,
    S: TelegramApiRetrySleeper + ?Sized,
{
    let object = request
        .as_object()
        .ok_or_else(|| "Telegram JSON API execution request must be an object.".to_string())?;
    let secrets = SecretValues::from_request(object);
    let requested_operation = requested_operation_label(object);
    let mut planner_request = object.clone();
    planner_request.insert(
        "stage".to_string(),
        JsonValue::String("request".to_string()),
    );
    let planned =
        match agent_telegram_api_method_execution_plan_json(&JsonValue::Object(planner_request)) {
            Ok(value) => value,
            Err(_) => {
                return Ok(failure_payload(
                    &ExecutionProgress::new(requested_operation),
                    "planning_contract_failed",
                    "contract",
                    "Telegram JSON API request planning failed.",
                    false,
                ))
            }
        };
    let planned_object = match planned.as_object() {
        Some(value)
            if clean_text(value.get("stage")).as_deref() == Some("request")
                && clean_text(value.get("execution_kind")).as_deref()
                    == Some(API_METHOD_EXECUTION_KIND) =>
        {
            value
        }
        _ => {
            return Ok(failure_payload(
                &ExecutionProgress::new(requested_operation),
                "planning_contract_failed",
                "contract",
                "Telegram JSON API request planning returned an invalid contract.",
                false,
            ))
        }
    };
    let should_execute = match planned_object
        .get("should_execute")
        .and_then(JsonValue::as_bool)
    {
        Some(value) => value,
        None => {
            return Ok(failure_payload(
                &ExecutionProgress::new(requested_operation),
                "planning_contract_failed",
                "contract",
                "Telegram JSON API request planning returned an invalid execution flag.",
                false,
            ))
        }
    };
    let Some(execution_request) = planned_object.get("request").and_then(JsonValue::as_object)
    else {
        return Ok(failure_payload(
            &ExecutionProgress::new(requested_operation),
            "planning_contract_failed",
            "contract",
            "Telegram JSON API request planning omitted its execution request.",
            false,
        ));
    };
    if !should_execute {
        return Ok(failure_payload(
            &ExecutionProgress::new(requested_operation),
            "planning_rejected",
            "planning",
            "Telegram JSON API request planning was rejected.",
            false,
        ));
    }

    let plan =
        match ExecutionPlan::parse(execution_request) {
            Ok(plan) => plan,
            Err(PlanError::Unsupported(progress)) => return Ok(failure_payload(
                &progress,
                "unsupported_operation_or_transport",
                "unsupported",
                "Telegram JSON API execution does not support this planned operation or transport.",
                false,
            )),
            Err(PlanError::Contract(progress)) => {
                return Ok(failure_payload(
                    &progress,
                    "planning_contract_failed",
                    "contract",
                    "Telegram JSON API request planning returned an invalid execution request.",
                    false,
                ))
            }
        };
    let mut progress = plan.progress();

    loop {
        progress.attempts += 1;
        let http_result = match executor.execute_json_request(&plan.http_request) {
            Ok(value) => value,
            Err(_) => {
                return Ok(failure_payload(
                    &progress,
                    "executor_failed",
                    "executor",
                    "Telegram JSON API HTTP executor failed.",
                    false,
                ))
            }
        };
        let Some(http_object) = http_result.as_object() else {
            return Ok(failure_payload(
                &progress,
                "http_contract_failed",
                "contract",
                "Telegram JSON API HTTP executor returned an invalid result.",
                false,
            ));
        };
        let Some(http_ok) = http_object.get("ok").and_then(JsonValue::as_bool) else {
            return Ok(failure_payload(
                &progress,
                "http_contract_failed",
                "contract",
                "Telegram JSON API HTTP executor returned an invalid status.",
                false,
            ));
        };
        progress.status_code = optional_i64(http_object.get("status_code"));
        if !http_ok {
            let error_kind = public_http_error_kind(http_object.get("error_kind"));
            if error_kind == "http" {
                if let Some(telegram_payload) = telegram_http_error_payload(http_object) {
                    return Ok(normalize_telegram_result(
                        &plan,
                        &progress,
                        &telegram_payload,
                        &secrets,
                    ));
                }
            }
            let retryable = matches!(error_kind, "timeout" | "transport");
            if retryable && progress.attempts < plan.max_attempts {
                let delay = agent_transport_retry_delay_seconds(
                    RETRY_BASE_DELAY_SECONDS,
                    (progress.attempts - 1) as i64,
                );
                if sleeper.sleep_seconds(delay).is_err() {
                    return Ok(failure_payload(
                        &progress,
                        "retry_sleep_failed",
                        "sleep",
                        "Telegram JSON API retry sleep failed.",
                        false,
                    ));
                }
                progress.retry_delays_seconds.push(delay);
                continue;
            }
            let exhausted = retryable && progress.attempts >= plan.max_attempts;
            return Ok(failure_payload(
                &progress,
                if exhausted {
                    "retry_exhausted"
                } else {
                    "http_failed"
                },
                error_kind,
                if exhausted {
                    "Telegram JSON API HTTP retries were exhausted."
                } else {
                    "Telegram JSON API HTTP request failed."
                },
                exhausted,
            ));
        }
        if !progress
            .status_code
            .is_some_and(|status_code| (200..300).contains(&status_code))
        {
            return Ok(failure_payload(
                &progress,
                "http_contract_failed",
                "contract",
                "Telegram JSON API HTTP executor returned an invalid success status.",
                false,
            ));
        }
        if clean_text(http_object.get("response_kind")).as_deref() != Some("json") {
            return Ok(failure_payload(
                &progress,
                "http_contract_failed",
                "response",
                "Telegram JSON API HTTP response was not JSON.",
                false,
            ));
        }
        let Some(telegram_payload) = http_object.get("payload").filter(|value| value.is_object())
        else {
            return Ok(failure_payload(
                &progress,
                "http_contract_failed",
                "response",
                "Telegram JSON API HTTP response payload was invalid.",
                false,
            ));
        };
        return Ok(normalize_telegram_result(
            &plan,
            &progress,
            telegram_payload,
            &secrets,
        ));
    }
}

fn telegram_http_error_payload(http_result: &Map<String, JsonValue>) -> Option<JsonValue> {
    let detail = http_result.get("detail").and_then(JsonValue::as_str)?;
    let payload = parse_value(detail, "Telegram HTTP error response JSON").ok()?;
    let object = payload.as_object()?;
    (object.get("ok").and_then(JsonValue::as_bool) == Some(false)).then_some(payload)
}

struct ExecutionPlan {
    operation: &'static str,
    telegram_method: &'static str,
    retry_family: &'static str,
    max_attempts: u64,
    execution_request: JsonValue,
    http_request: JsonValue,
}

impl ExecutionPlan {
    fn parse(request: &Map<String, JsonValue>) -> Result<Self, PlanError> {
        let operation = clean_text(request.get("operation")).unwrap_or_default();
        let safe_operation = supported_operation(&operation, request.get("telegram_method"));
        let progress = ExecutionProgress::new(operation_label(&operation));
        let Some((operation, telegram_method)) = safe_operation else {
            return Err(PlanError::Unsupported(progress));
        };
        if clean_text(request.get("execution_kind")).as_deref() != Some(API_METHOD_EXECUTION_KIND)
            || request.get("ok").and_then(JsonValue::as_bool) != Some(true)
        {
            return Err(PlanError::Contract(progress));
        }
        if clean_text(request.get("transport")).as_deref() != Some("json") {
            return Err(PlanError::Unsupported(progress));
        }
        let method = clean_text(request.get("method"));
        let url = clean_text(request.get("url"));
        let (Some(method), Some(url)) = (method, url) else {
            return Err(PlanError::Contract(progress));
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(PlanError::Contract(progress));
        }
        let (expected_method, expected_result_kind, retry_family) = match operation {
            "get_updates" => ("GET", "updates", "poll"),
            "get_file" => ("POST", "file_info", "delivery"),
            "send_message" => ("POST", "unit", "delivery"),
            "send_attachment" => ("POST", "unit", "delivery"),
            _ => return Err(PlanError::Unsupported(progress)),
        };
        if !method.eq_ignore_ascii_case(expected_method)
            || clean_text(request.get("telegram_method")).as_deref() != Some(telegram_method)
            || clean_text(request.get("result_kind")).as_deref() != Some(expected_result_kind)
            || clean_text(request.get("retry_family")).as_deref() != Some(retry_family)
        {
            return Err(PlanError::Contract(progress));
        }
        let max_attempts = if retry_family == "poll" {
            POLL_MAX_ATTEMPTS
        } else {
            DELIVERY_MAX_ATTEMPTS
        };
        let http_request = json!({
            "method": method,
            "url": url,
            "payload": request.get("payload").cloned().unwrap_or(JsonValue::Null),
            "timeout_seconds": request.get("timeout").cloned().unwrap_or(JsonValue::Null),
        });
        Ok(Self {
            operation,
            telegram_method,
            retry_family,
            max_attempts,
            execution_request: JsonValue::Object(request.clone()),
            http_request,
        })
    }

    fn progress(&self) -> ExecutionProgress {
        ExecutionProgress {
            operation: self.operation,
            telegram_method: self.telegram_method,
            retry_family: self.retry_family,
            max_attempts: self.max_attempts,
            ..ExecutionProgress::default()
        }
    }
}

enum PlanError {
    Unsupported(ExecutionProgress),
    Contract(ExecutionProgress),
}

#[derive(Default)]
struct ExecutionProgress {
    operation: &'static str,
    telegram_method: &'static str,
    retry_family: &'static str,
    max_attempts: u64,
    attempts: u64,
    retry_delays_seconds: Vec<f64>,
    status_code: Option<i64>,
}

impl ExecutionProgress {
    fn new(operation: &'static str) -> Self {
        let (telegram_method, retry_family, max_attempts) = match operation {
            "get_updates" => ("getUpdates", "poll", POLL_MAX_ATTEMPTS),
            "get_file" => ("getFile", "delivery", DELIVERY_MAX_ATTEMPTS),
            "send_message" => ("sendMessage", "delivery", DELIVERY_MAX_ATTEMPTS),
            "send_attachment" => ("sendAttachment", "delivery", DELIVERY_MAX_ATTEMPTS),
            _ => ("unknown", "unknown", 0),
        };
        Self {
            operation,
            telegram_method,
            retry_family,
            max_attempts,
            ..Self::default()
        }
    }
}

fn normalize_telegram_result(
    plan: &ExecutionPlan,
    progress: &ExecutionProgress,
    telegram_payload: &JsonValue,
    secrets: &SecretValues,
) -> JsonValue {
    let Some(telegram_payload_object) = telegram_payload.as_object() else {
        return failure_payload(
            progress,
            "result_contract_failed",
            "response",
            "Telegram JSON API response envelope was invalid.",
            false,
        );
    };
    let telegram_ok = telegram_payload_object
        .get("ok")
        .and_then(JsonValue::as_bool);
    let telegram_parse_error = plan.operation == "send_message"
        && telegram_ok == Some(false)
        && telegram_message_parse_error(telegram_payload_object);
    if telegram_ok.is_none()
        || (telegram_ok == Some(true)
            && !telegram_success_result_shape_is_valid(plan.operation, telegram_payload_object))
    {
        return failure_payload(
            progress,
            "result_contract_failed",
            "response",
            "Telegram JSON API response envelope was invalid.",
            false,
        );
    }
    let planned = match agent_telegram_api_method_execution_plan_json(&json!({
        "stage": "result",
        "execution_request": plan.execution_request,
        "payload": telegram_payload,
    })) {
        Ok(value) => value,
        Err(_) => {
            return failure_payload(
                progress,
                "result_contract_failed",
                "contract",
                "Telegram JSON API result planning failed.",
                false,
            )
        }
    };
    let Some(planned_object) = planned.as_object() else {
        return failure_payload(
            progress,
            "result_contract_failed",
            "contract",
            "Telegram JSON API result planning returned an invalid contract.",
            false,
        );
    };
    if clean_text(planned_object.get("stage")).as_deref() != Some("result")
        || clean_text(planned_object.get("execution_kind")).as_deref()
            != Some(API_METHOD_EXECUTION_KIND)
    {
        return failure_payload(
            progress,
            "result_contract_failed",
            "contract",
            "Telegram JSON API result planning returned an invalid identity.",
            false,
        );
    }
    let Some(result) = planned_object.get("result").and_then(JsonValue::as_object) else {
        return failure_payload(
            progress,
            "result_contract_failed",
            "contract",
            "Telegram JSON API result planning omitted its result.",
            false,
        );
    };
    let result_ok = result.get("ok").and_then(JsonValue::as_bool);
    if clean_text(result.get("execution_kind")).as_deref() != Some(API_METHOD_EXECUTION_KIND)
        || clean_text(result.get("operation")).as_deref() != Some(plan.operation)
        || clean_text(result.get("telegram_method")).as_deref() != Some(plan.telegram_method)
        || planned_object.get("completed").and_then(JsonValue::as_bool) != result_ok
    {
        return failure_payload(
            progress,
            "result_contract_failed",
            "contract",
            "Telegram JSON API result planning returned an invalid result contract.",
            false,
        );
    }
    if result_ok != Some(true) {
        let mut failure = failure_payload(
            progress,
            "telegram_api_failed",
            "telegram_api",
            "Telegram API rejected the request.",
            false,
        );
        if let Some(object) = failure.as_object_mut() {
            object.insert(
                "telegram_parse_error".to_string(),
                json!(telegram_parse_error),
            );
        }
        return failure;
    }

    let (updates, file_info, sent) = match plan.operation {
        "get_updates" => {
            let Some(updates) = result.get("updates").and_then(JsonValue::as_array) else {
                return failure_payload(
                    progress,
                    "result_contract_failed",
                    "contract",
                    "Telegram getUpdates result was invalid.",
                    false,
                );
            };
            (
                secrets.sanitize(&JsonValue::Array(updates.clone())),
                JsonValue::Null,
                false,
            )
        }
        "get_file" => {
            let Some(file_info) = result.get("file_info").and_then(JsonValue::as_object) else {
                return failure_payload(
                    progress,
                    "result_contract_failed",
                    "contract",
                    "Telegram getFile result was invalid.",
                    false,
                );
            };
            (JsonValue::Null, public_file_info(file_info, secrets), false)
        }
        "send_message" | "send_attachment" => (JsonValue::Null, JsonValue::Null, true),
        _ => {
            return failure_payload(
                progress,
                "result_contract_failed",
                "contract",
                "Telegram JSON API result operation was unsupported.",
                false,
            )
        }
    };
    execution_payload(
        progress,
        "completed",
        true,
        true,
        None,
        None,
        false,
        updates,
        file_info,
        sent,
    )
}

fn telegram_success_result_shape_is_valid(
    operation: &str,
    payload: &Map<String, JsonValue>,
) -> bool {
    match operation {
        "get_updates" => payload.get("result").is_some_and(JsonValue::is_array),
        "get_file" | "send_message" | "send_attachment" => {
            payload.get("result").is_some_and(JsonValue::is_object)
        }
        _ => false,
    }
}

#[derive(Default)]
struct SecretValues {
    values: Vec<String>,
}

impl SecretValues {
    fn from_request(request: &Map<String, JsonValue>) -> Self {
        let mut values = Vec::new();
        for key in ["bot_token", "token"] {
            if let Some(value) = clean_text(request.get(key)) {
                if !values.contains(&value) {
                    values.push(value);
                }
            }
        }
        Self { values }
    }

    fn sanitize(&self, value: &JsonValue) -> JsonValue {
        match value {
            JsonValue::String(value) => {
                let sanitized = self
                    .values
                    .iter()
                    .fold(value.clone(), |text, secret| text.replace(secret, REDACTED));
                JsonValue::String(sanitized)
            }
            JsonValue::Array(values) => {
                JsonValue::Array(values.iter().map(|value| self.sanitize(value)).collect())
            }
            JsonValue::Object(values) => JsonValue::Object(
                values
                    .iter()
                    .map(|(key, value)| {
                        let value = if sensitive_key(key) {
                            JsonValue::String(REDACTED.to_string())
                        } else {
                            self.sanitize(value)
                        };
                        (key.clone(), value)
                    })
                    .collect(),
            ),
            value => value.clone(),
        }
    }
}

fn public_file_info(file_info: &Map<String, JsonValue>, secrets: &SecretValues) -> JsonValue {
    JsonValue::Object(
        ["file_id", "file_unique_id", "file_size", "file_path"]
            .into_iter()
            .filter_map(|key| {
                file_info
                    .get(key)
                    .map(|value| (key.to_string(), secrets.sanitize(value)))
            })
            .collect(),
    )
}

fn sensitive_key(key: &str) -> bool {
    matches!(
        key.to_ascii_lowercase().as_str(),
        "authorization" | "bot_token" | "token"
    )
}

fn failure_payload(
    progress: &ExecutionProgress,
    state: &str,
    error_kind: &str,
    error: &str,
    retry_exhausted: bool,
) -> JsonValue {
    execution_payload(
        progress,
        state,
        false,
        false,
        Some(error_kind),
        Some(error),
        retry_exhausted,
        JsonValue::Null,
        JsonValue::Null,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn execution_payload(
    progress: &ExecutionProgress,
    state: &str,
    ok: bool,
    completed: bool,
    error_kind: Option<&str>,
    error: Option<&str>,
    retry_exhausted: bool,
    updates: JsonValue,
    file_info: JsonValue,
    sent: bool,
) -> JsonValue {
    json!({
        "contract": CONTRACT,
        "migration_stage": MIGRATION_STAGE,
        "stage": "execute",
        "telegram_api_state": state,
        "operation": progress.operation,
        "telegram_method": progress.telegram_method,
        "retry_family": progress.retry_family,
        "max_attempts": progress.max_attempts,
        "attempts": progress.attempts,
        "retry_count": progress.retry_delays_seconds.len(),
        "retry_delays_seconds": progress.retry_delays_seconds,
        "retry_exhausted": retry_exhausted,
        "http_status_code": progress.status_code.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "ok": ok,
        "completed": completed,
        "updates": updates,
        "file_info": file_info,
        "sent": sent,
        "telegram_parse_error": false,
        "error_kind": error_kind.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "error": error.map(JsonValue::from).unwrap_or(JsonValue::Null),
        "python_telegram_api_allowed": false,
        "python_http_execution_allowed": false,
        "python_retry_allowed": false,
        "raw_telegram_payload_exposed": false,
        "token_bearing_url_exposed": false,
    })
}

fn telegram_message_parse_error(payload: &Map<String, JsonValue>) -> bool {
    let description = payload
        .get("description")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    agent_telegram_message_formatting_plan_json(&json!({
        "kind": "markdown_parse_error",
        "error": description,
    }))
    .ok()
    .and_then(|planned| planned.get("parse_error").and_then(JsonValue::as_bool))
    .unwrap_or(false)
}

fn supported_operation(
    value: &str,
    telegram_method: Option<&JsonValue>,
) -> Option<(&'static str, &'static str)> {
    match value {
        "get_updates" => Some(("get_updates", "getUpdates")),
        "get_file" => Some(("get_file", "getFile")),
        "send_message" => Some(("send_message", "sendMessage")),
        "send_attachment" => {
            supported_attachment_method(telegram_method).map(|method| ("send_attachment", method))
        }
        _ => None,
    }
}

fn supported_attachment_method(value: Option<&JsonValue>) -> Option<&'static str> {
    match value.and_then(JsonValue::as_str).map(str::trim) {
        Some("sendAudio") => Some("sendAudio"),
        Some("sendPhoto") => Some("sendPhoto"),
        Some("sendDocument") => Some("sendDocument"),
        _ => None,
    }
}

fn requested_operation_label(object: &Map<String, JsonValue>) -> &'static str {
    let operation = clean_text(object.get("operation"))
        .or_else(|| clean_text(object.get("method_kind")))
        .unwrap_or_default();
    operation_label(&operation)
}

fn operation_label(value: &str) -> &'static str {
    match value {
        "get_updates" | "getUpdates" | "telegram.get_updates" => "get_updates",
        "get_file" | "getFile" | "telegram.get_file" => "get_file",
        "send_message" | "sendMessage" | "telegram.send_message" => "send_message",
        "download_file" | "downloadFile" | "download_file_bytes" | "telegram.download_file" => {
            "download_file"
        }
        "send_attachment" | "sendAudio" | "sendPhoto" | "sendDocument" => "send_attachment",
        _ => "unknown",
    }
}

fn public_http_error_kind(value: Option<&JsonValue>) -> &'static str {
    match value.and_then(JsonValue::as_str).map(str::trim) {
        Some("timeout") => "timeout",
        Some("transport") => "transport",
        Some("http") => "http",
        Some("url") => "url",
        Some("invalid_timeout") => "invalid_timeout",
        Some("response") => "response",
        _ => "http",
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    value
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_i64(value: Option<&JsonValue>) -> Option<i64> {
    match value? {
        JsonValue::Number(value) => value.as_i64(),
        JsonValue::String(value) => value.trim().parse::<i64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
