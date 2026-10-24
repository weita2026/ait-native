use crate::transport::{
    agent_transport_config_split_message_chunks, agent_transport_http_execute_json_request_json,
};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};

const MIGRATION_STAGE: &str = "rust_agent_slack_response_url_delivery_execution";
const RESPONSE_URL_DELIVERY_EXECUTION_CONTRACT: &str =
    "ait_agent_core.event_loop.SlackResponseUrlDeliveryExecution.v1";
const DEFAULT_RESPONSE_TYPE: &str = "in_channel";
const DEFAULT_SLACK_MESSAGE_LIMIT: usize = 3000;
const DEFAULT_TIMEOUT_SECONDS: f64 = 20.0;
const REDACTED_RESPONSE_URL: &str = "[redacted]";

pub trait SlackResponseUrlDeliveryExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultSlackResponseUrlDeliveryExecutor;

impl SlackResponseUrlDeliveryExecutor for DefaultSlackResponseUrlDeliveryExecutor {
    fn execute_json_request(&self, request: &JsonValue) -> Result<JsonValue, String> {
        agent_transport_http_execute_json_request_json(request)
    }
}

pub fn agent_slack_response_url_delivery_execute_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    execute_with_slack_response_url_delivery_executor(
        &DefaultSlackResponseUrlDeliveryExecutor,
        request,
    )
}

pub fn execute_with_slack_response_url_delivery_executor<E>(
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: SlackResponseUrlDeliveryExecutor + ?Sized,
{
    execute_response_url_delivery_json(executor, request)
}

fn execute_response_url_delivery_json<E>(
    executor: &E,
    request: &JsonValue,
) -> Result<JsonValue, String>
where
    E: SlackResponseUrlDeliveryExecutor + ?Sized,
{
    let request = request_object(request)?;
    let operation = operation_object(request);
    let kind = clean_text(operation.get("kind")).unwrap_or_default();
    if kind != "send_response" {
        return Ok(failure_payload(
            "rejected",
            &kind,
            format!(
                "Unsupported Slack response URL delivery operation: {}.",
                if kind.is_empty() { "<missing>" } else { &kind }
            ),
            Vec::new(),
        ));
    }

    let Some(response_url) = clean_text(operation.get("response_url"))
        .or_else(|| clean_text(request.get("response_url")))
    else {
        return Ok(failure_payload(
            "rejected",
            &kind,
            "Slack response URL delivery operation requires response_url.",
            Vec::new(),
        ));
    };
    let text = clean_text(operation.get("text"))
        .or_else(|| clean_text(request.get("text")))
        .unwrap_or_default();
    let response_type = clean_text(operation.get("response_type"))
        .or_else(|| clean_text(request.get("response_type")))
        .unwrap_or_else(|| DEFAULT_RESPONSE_TYPE.to_string());
    let message_limit = optional_usize(operation.get("message_limit"))
        .or_else(|| optional_usize(request.get("message_limit")))
        .unwrap_or(DEFAULT_SLACK_MESSAGE_LIMIT)
        .max(1);
    let timeout_seconds = optional_f64(operation.get("timeout_seconds"))
        .or_else(|| optional_f64(request.get("timeout_seconds")))
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS);
    let replace_original = optional_bool(operation.get("replace_original"))
        .or_else(|| optional_bool(request.get("replace_original")))
        .unwrap_or(false);
    let headers = headers_object(operation, request);
    let chunks = agent_transport_config_split_message_chunks(&text, message_limit);
    let mut operation_results = Vec::new();

    for (index, chunk) in chunks.iter().enumerate() {
        let http_request = json!({
            "method": "POST",
            "url": response_url,
            "payload": {
                "text": chunk,
                "response_type": response_type,
                "replace_original": replace_original,
            },
            "headers": headers,
            "timeout_seconds": timeout_seconds,
        });
        let http_result = match executor.execute_json_request(&http_request) {
            Ok(result) => sanitize_response_url_json(result, &response_url),
            Err(error) => json!({
                "ok": false,
                "error_kind": "executor",
                "method": "POST",
                "url": REDACTED_RESPONSE_URL,
                "message": sanitize_response_url_text(&error, &response_url),
            }),
        };
        let ok = http_result
            .as_object()
            .and_then(|result| optional_bool(result.get("ok")))
            .unwrap_or(false);
        let result = json!({
            "index": index,
            "kind": kind,
            "ok": ok,
            "chunk": chunk,
            "chunk_char_count": chunk.chars().count(),
            "http_request": redacted_http_request(&http_request),
            "http_result": http_result,
        });
        operation_results.push(result);
        if !ok {
            let error = first_failure_message(&operation_results)
                .unwrap_or_else(|| "Slack response URL delivery failed.".to_string());
            return Ok(base_payload(
                "execute",
                "delivery_failed",
                json!({
                    "ok": false,
                    "delivered": false,
                    "kind": kind,
                    "response_url_present": true,
                    "response_type": response_type,
                    "replace_original": replace_original,
                    "message_limit": message_limit,
                    "chunk_count": chunks.len(),
                    "attempted_chunk_count": operation_results.len(),
                    "delivered_chunk_count": successful_result_count(&operation_results),
                    "failed_chunk_count": failed_result_count(&operation_results),
                    "operation_results": operation_results,
                    "error": error,
                }),
            ));
        }
    }

    Ok(base_payload(
        "execute",
        "delivered",
        json!({
            "ok": true,
            "delivered": true,
            "kind": kind,
            "response_url_present": true,
            "response_type": response_type,
            "replace_original": replace_original,
            "message_limit": message_limit,
            "chunk_count": chunks.len(),
            "attempted_chunk_count": operation_results.len(),
            "delivered_chunk_count": successful_result_count(&operation_results),
            "failed_chunk_count": failed_result_count(&operation_results),
            "operation_results": operation_results,
            "error": JsonValue::Null,
        }),
    ))
}

fn failure_payload(
    state: &str,
    kind: &str,
    error: impl Into<String>,
    operation_results: Vec<JsonValue>,
) -> JsonValue {
    base_payload(
        "execute",
        state,
        json!({
            "ok": false,
            "delivered": false,
            "kind": kind,
            "response_url_present": false,
            "chunk_count": 0,
            "attempted_chunk_count": operation_results.len(),
            "delivered_chunk_count": successful_result_count(&operation_results),
            "failed_chunk_count": failed_result_count(&operation_results),
            "operation_results": operation_results,
            "error": error.into(),
        }),
    )
}

fn operation_object(request: &Map<String, JsonValue>) -> &Map<String, JsonValue> {
    request
        .get("operation")
        .or_else(|| request.get("delivery_operation"))
        .and_then(JsonValue::as_object)
        .unwrap_or(request)
}

fn headers_object(
    operation: &Map<String, JsonValue>,
    request: &Map<String, JsonValue>,
) -> JsonValue {
    let mut headers = operation
        .get("headers")
        .or_else(|| request.get("headers"))
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    headers
        .entry("Content-Type".to_string())
        .or_insert_with(|| JsonValue::String("application/json".to_string()));
    JsonValue::Object(headers)
}

fn redacted_http_request(request: &JsonValue) -> JsonValue {
    let mut object = request.as_object().cloned().unwrap_or_default();
    if object.contains_key("url") {
        object.insert(
            "url".to_string(),
            JsonValue::String(REDACTED_RESPONSE_URL.to_string()),
        );
    }
    JsonValue::Object(object)
}

fn sanitize_response_url_json(value: JsonValue, response_url: &str) -> JsonValue {
    match value {
        JsonValue::String(text) => {
            JsonValue::String(sanitize_response_url_text(&text, response_url))
        }
        JsonValue::Array(values) => JsonValue::Array(
            values
                .into_iter()
                .map(|value| sanitize_response_url_json(value, response_url))
                .collect(),
        ),
        JsonValue::Object(object) => JsonValue::Object(
            object
                .into_iter()
                .map(|(key, value)| (key, sanitize_response_url_json(value, response_url)))
                .collect(),
        ),
        other => other,
    }
}

fn sanitize_response_url_text(text: &str, response_url: &str) -> String {
    if response_url.is_empty() {
        text.to_string()
    } else {
        text.replace(response_url, REDACTED_RESPONSE_URL)
    }
}

fn successful_result_count(results: &[JsonValue]) -> usize {
    results
        .iter()
        .filter(|result| {
            result
                .as_object()
                .and_then(|result| optional_bool(result.get("ok")))
                .unwrap_or(false)
        })
        .count()
}

fn failed_result_count(results: &[JsonValue]) -> usize {
    results
        .iter()
        .filter(|result| {
            result
                .as_object()
                .map(|result| !optional_bool(result.get("ok")).unwrap_or(false))
                .unwrap_or(true)
        })
        .count()
}

fn first_failure_message(results: &[JsonValue]) -> Option<String> {
    results.iter().find_map(|result| {
        let result = result.as_object()?;
        if optional_bool(result.get("ok")).unwrap_or(false) {
            return None;
        }
        let http_result = result.get("http_result")?.as_object()?;
        clean_text(http_result.get("message"))
            .or_else(|| clean_text(http_result.get("error")))
            .or_else(|| clean_text(http_result.get("detail")))
    })
}

fn base_payload(stage: &str, state: &str, payload: JsonValue) -> JsonValue {
    let mut object = payload.as_object().cloned().unwrap_or_default();
    object.insert(
        "migration_stage".to_string(),
        JsonValue::String(MIGRATION_STAGE.to_string()),
    );
    object.insert(
        "slack_response_url_delivery_execution_contract".to_string(),
        JsonValue::String(RESPONSE_URL_DELIVERY_EXECUTION_CONTRACT.to_string()),
    );
    object.insert("stage".to_string(), JsonValue::String(stage.to_string()));
    object.insert(
        "transport".to_string(),
        JsonValue::String("slack".to_string()),
    );
    object.insert(
        "rust_event_loop_required".to_string(),
        JsonValue::Bool(true),
    );
    object.insert(
        "python_response_url_delivery_allowed".to_string(),
        JsonValue::Bool(false),
    );
    object.insert(
        "delivery_execution_state".to_string(),
        JsonValue::String(state.to_string()),
    );
    JsonValue::Object(object)
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request.as_object().ok_or_else(|| {
        "Slack response URL delivery execution request must be an object.".to_string()
    })
}

fn optional_bool(value: Option<&JsonValue>) -> Option<bool> {
    match value? {
        JsonValue::Bool(value) => Some(*value),
        JsonValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        },
        JsonValue::Number(number) => number.as_i64().map(|value| value != 0),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_f64(value: Option<&JsonValue>) -> Option<f64> {
    match value? {
        JsonValue::Number(number) => number.as_f64(),
        JsonValue::String(text) => text.trim().parse::<f64>().ok(),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn optional_usize(value: Option<&JsonValue>) -> Option<usize> {
    match value? {
        JsonValue::Number(number) => number
            .as_u64()
            .and_then(|value| usize::try_from(value).ok()),
        JsonValue::String(text) => text.trim().parse::<usize>().ok(),
        JsonValue::Null | JsonValue::Bool(_) | JsonValue::Array(_) | JsonValue::Object(_) => None,
    }
}

fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = match value? {
        JsonValue::String(text) => text.trim().to_string(),
        JsonValue::Number(number) => number.to_string(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Null | JsonValue::Array(_) | JsonValue::Object(_) => return None,
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}
