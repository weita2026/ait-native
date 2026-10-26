use super::retry::agent_transport_retry_timeout_phrase;
use crate::json_support::{encode_value, encode_value_or, parse_value};
use ait_core::json_support::{json, JsonMap as Map, JsonValue};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTransportHttpBytesExecution {
    Success {
        method: String,
        url: String,
        status_code: i64,
        payload: Vec<u8>,
    },
    Error(JsonValue),
}

pub fn agent_transport_http_plan_json_request_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let method = method_text(object.get("method"));
    let payload = object.get("payload");
    let mut headers = default_json_headers();
    let body_text = match payload {
        None | Some(JsonValue::Null) => JsonValue::Null,
        Some(value) => {
            headers.insert(
                "Content-Type".to_string(),
                JsonValue::String("application/json".to_string()),
            );
            JsonValue::String(encode_value(
                value,
                "failed to encode JSON request payload",
            )?)
        }
    };
    merge_headers(&mut headers, object.get("headers"))?;
    Ok(json!({
        "method": method,
        "headers": headers,
        "body_text": body_text,
    }))
}

pub fn agent_transport_http_plan_multipart_request_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let boundary = required_text_field(object, "boundary")?;
    let file_field = required_text_field(object, "file_field")?;
    let file_name = required_text_field(object, "file_name")?;
    let mime_type = required_text_field(object, "mime_type")?;
    let fields = object
        .get("fields")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "transport HTTP multipart request field `fields` must be an object".to_string()
        })?;

    let mut file_prefix_text = String::new();
    for (key, value) in fields {
        if value.is_null() {
            continue;
        }
        file_prefix_text.push_str("--");
        file_prefix_text.push_str(boundary);
        file_prefix_text.push_str("\r\nContent-Disposition: form-data; name=\"");
        file_prefix_text.push_str(key);
        file_prefix_text.push_str("\"\r\n\r\n");
        file_prefix_text.push_str(&pythonish_text(value));
        file_prefix_text.push_str("\r\n");
    }
    file_prefix_text.push_str("--");
    file_prefix_text.push_str(boundary);
    file_prefix_text.push_str("\r\nContent-Disposition: form-data; name=\"");
    file_prefix_text.push_str(file_field);
    file_prefix_text.push_str("\"; filename=\"");
    file_prefix_text.push_str(file_name);
    file_prefix_text.push_str("\"\r\nContent-Type: ");
    file_prefix_text.push_str(mime_type);
    file_prefix_text.push_str("\r\n\r\n");

    let mut headers = default_json_headers();
    headers.insert(
        "Content-Type".to_string(),
        JsonValue::String(format!("multipart/form-data; boundary={boundary}")),
    );
    merge_headers(&mut headers, object.get("headers"))?;

    Ok(json!({
        "method": "POST",
        "headers": headers,
        "file_prefix_text": file_prefix_text,
        "file_suffix_text": format!("\r\n--{boundary}--\r\n"),
    }))
}

pub fn agent_transport_http_response_payload_json(raw: &str) -> JsonValue {
    if raw.trim().is_empty() {
        return json!({
            "kind": "json",
            "value": {},
        });
    }
    match parse_value(raw, "failed to parse transport HTTP response payload") {
        Ok(value) => json!({
            "kind": "json",
            "value": value,
        }),
        Err(_) => json!({
            "kind": "text",
            "value": raw,
        }),
    }
}

pub fn agent_transport_http_execute_json_request_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let url = required_text_field(object, "url")?;
    let timeout = parse_timeout_seconds_field(object.get("timeout_seconds"))?;
    if let Some(invalid_timeout_message) = invalid_timeout_result(
        object,
        timeout,
        method_text(object.get("method")).as_str(),
        url,
    ) {
        return Ok(invalid_timeout_message);
    }

    let planned = agent_transport_http_plan_json_request_json(request)?;
    let method = planned
        .get("method")
        .and_then(JsonValue::as_str)
        .unwrap_or("GET")
        .to_string();
    let body_text = planned.get("body_text").and_then(JsonValue::as_str);
    let headers = planned
        .get("headers")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "transport HTTP execution plan field `headers` must be an object".to_string()
        })?;
    let method_value = match reqwest::Method::from_bytes(method.as_bytes()) {
        Ok(value) => value,
        Err(err) => {
            return Ok(execution_error_payload(
                "url",
                &method,
                url,
                agent_transport_http_url_error_message(&method, url, &err.to_string()),
                None,
                None,
                None,
            ));
        }
    };

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| format!("failed to build transport HTTP client: {err}"))?;
    let mut builder = client.request(method_value, url);
    for (key, value) in headers {
        let header_value = value.as_str().unwrap_or_default();
        builder = builder.header(key.as_str(), header_value);
    }
    if let Some(body_text) = body_text {
        builder = builder.body(body_text.to_string());
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(Duration::from_secs_f64(timeout));
    }

    let response = match builder.send() {
        Ok(response) => response,
        Err(err) => {
            return Ok(reqwest_error_payload(&method, url, timeout, err));
        }
    };
    let status = response.status();
    let status_code = i64::from(status.as_u16());
    let reason = status.canonical_reason().unwrap_or_default().to_string();
    let raw = match response.text() {
        Ok(raw) => raw,
        Err(err) => {
            return Ok(execution_error_payload(
                "transport",
                &method,
                url,
                agent_transport_http_transport_error_message(&method, url, &err.to_string()),
                Some(status_code),
                None,
                Some(reason),
            ));
        }
    };
    if !status.is_success() {
        return Ok(execution_error_payload(
            "http",
            &method,
            url,
            agent_transport_http_error_message(
                &method,
                url,
                status_code,
                Some(&raw),
                Some(&reason),
            ),
            Some(status_code),
            Some(raw),
            Some(reason),
        ));
    }

    let classified = agent_transport_http_response_payload_json(&raw);
    Ok(json!({
        "ok": true,
        "method": method,
        "url": url,
        "status_code": status_code,
        "response_kind": classified.get("kind").cloned().unwrap_or(JsonValue::Null),
        "payload": classified.get("value").cloned().unwrap_or(JsonValue::Null),
    }))
}

pub fn agent_transport_http_execute_multipart_json_request_json(
    request: &JsonValue,
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let file_bytes = required_bytes_field(object, "file_bytes")?;
    agent_transport_http_execute_multipart_json_request_with_bytes(request, &file_bytes)
}

pub fn agent_transport_http_execute_multipart_json_request_with_bytes(
    request: &JsonValue,
    file_bytes: &[u8],
) -> Result<JsonValue, String> {
    let object = request_object(request)?;
    let url = required_text_field(object, "url")?;
    let timeout = parse_timeout_seconds_field(object.get("timeout_seconds"))?;
    let planned = agent_transport_http_plan_multipart_request_json(request)?;
    let method = planned
        .get("method")
        .and_then(JsonValue::as_str)
        .unwrap_or("POST")
        .to_string();
    if let Some(invalid_timeout_message) =
        invalid_timeout_result(object, timeout, method.as_str(), url)
    {
        return Ok(invalid_timeout_message);
    }
    let file_prefix = planned
        .get("file_prefix_text")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            "transport HTTP multipart execution plan field `file_prefix_text` must be a string"
                .to_string()
        })?;
    let file_suffix = planned
        .get("file_suffix_text")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            "transport HTTP multipart execution plan field `file_suffix_text` must be a string"
                .to_string()
        })?;
    let headers = planned
        .get("headers")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| {
            "transport HTTP multipart execution plan field `headers` must be an object".to_string()
        })?;
    let method_value = match reqwest::Method::from_bytes(method.as_bytes()) {
        Ok(value) => value,
        Err(err) => {
            return Ok(execution_error_payload(
                "url",
                &method,
                url,
                agent_transport_http_url_error_message(&method, url, &err.to_string()),
                None,
                None,
                None,
            ));
        }
    };

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| format!("failed to build transport HTTP client: {err}"))?;
    let mut builder = client.request(method_value, url);
    for (key, value) in headers {
        let header_value = value.as_str().unwrap_or_default();
        builder = builder.header(key.as_str(), header_value);
    }

    let mut body = Vec::with_capacity(file_prefix.len() + file_bytes.len() + file_suffix.len());
    body.extend_from_slice(file_prefix.as_bytes());
    body.extend_from_slice(file_bytes);
    body.extend_from_slice(file_suffix.as_bytes());
    builder = builder.body(body);
    if let Some(timeout) = timeout {
        builder = builder.timeout(Duration::from_secs_f64(timeout));
    }

    let response = match builder.send() {
        Ok(response) => response,
        Err(err) => {
            return Ok(reqwest_error_payload(&method, url, timeout, err));
        }
    };
    let status = response.status();
    let status_code = i64::from(status.as_u16());
    let reason = status.canonical_reason().unwrap_or_default().to_string();
    let raw = match response.text() {
        Ok(raw) => raw,
        Err(err) => {
            return Ok(execution_error_payload(
                "transport",
                &method,
                url,
                agent_transport_http_transport_error_message(&method, url, &err.to_string()),
                Some(status_code),
                None,
                Some(reason),
            ));
        }
    };
    if !status.is_success() {
        return Ok(execution_error_payload(
            "http",
            &method,
            url,
            agent_transport_http_error_message(
                &method,
                url,
                status_code,
                Some(&raw),
                Some(&reason),
            ),
            Some(status_code),
            Some(raw),
            Some(reason),
        ));
    }

    let classified = agent_transport_http_response_payload_json(&raw);
    Ok(json!({
        "ok": true,
        "method": method,
        "url": url,
        "status_code": status_code,
        "response_kind": classified.get("kind").cloned().unwrap_or(JsonValue::Null),
        "payload": classified.get("value").cloned().unwrap_or(JsonValue::Null),
    }))
}

pub fn agent_transport_http_execute_bytes_request(
    request: &JsonValue,
) -> Result<AgentTransportHttpBytesExecution, String> {
    let object = request_object(request)?;
    let url = required_text_field(object, "url")?;
    let timeout = parse_timeout_seconds_field(object.get("timeout_seconds"))?;
    let method = method_text(object.get("method"));
    if let Some(invalid_timeout_message) =
        invalid_timeout_result(object, timeout, method.as_str(), url)
    {
        return Ok(AgentTransportHttpBytesExecution::Error(
            invalid_timeout_message,
        ));
    }
    let method_value = match reqwest::Method::from_bytes(method.as_bytes()) {
        Ok(value) => value,
        Err(err) => {
            return Ok(AgentTransportHttpBytesExecution::Error(
                execution_error_payload(
                    "url",
                    &method,
                    url,
                    agent_transport_http_url_error_message(&method, url, &err.to_string()),
                    None,
                    None,
                    None,
                ),
            ));
        }
    };

    let client = reqwest::blocking::Client::builder()
        .build()
        .map_err(|err| format!("failed to build transport HTTP client: {err}"))?;
    let mut builder = client.request(method_value, url);
    let mut headers = Map::new();
    merge_headers(&mut headers, object.get("headers"))?;
    for (key, value) in &headers {
        let header_value = value.as_str().unwrap_or_default();
        builder = builder.header(key.as_str(), header_value);
    }
    if let Some(timeout) = timeout {
        builder = builder.timeout(Duration::from_secs_f64(timeout));
    }

    let response = match builder.send() {
        Ok(response) => response,
        Err(err) => {
            return Ok(AgentTransportHttpBytesExecution::Error(
                reqwest_error_payload(&method, url, timeout, err),
            ));
        }
    };
    let status = response.status();
    let status_code = i64::from(status.as_u16());
    let reason = status.canonical_reason().unwrap_or_default().to_string();
    let payload = match response.bytes() {
        Ok(payload) => payload.to_vec(),
        Err(err) => {
            return Ok(AgentTransportHttpBytesExecution::Error(
                execution_error_payload(
                    "transport",
                    &method,
                    url,
                    agent_transport_http_transport_error_message(&method, url, &err.to_string()),
                    Some(status_code),
                    None,
                    Some(reason),
                ),
            ));
        }
    };
    if !status.is_success() {
        let detail = String::from_utf8_lossy(&payload).to_string();
        return Ok(AgentTransportHttpBytesExecution::Error(
            execution_error_payload(
                "http",
                &method,
                url,
                agent_transport_http_error_message(
                    &method,
                    url,
                    status_code,
                    Some(&detail),
                    Some(&reason),
                ),
                Some(status_code),
                Some(detail),
                Some(reason),
            ),
        ));
    }

    Ok(AgentTransportHttpBytesExecution::Success {
        method,
        url: url.to_string(),
        status_code,
        payload,
    })
}

pub fn agent_transport_http_timeout_message(
    method: &str,
    url: &str,
    timeout: Option<f64>,
) -> String {
    format!(
        "{} {} timed out{}.",
        method_upper(method),
        url,
        agent_transport_retry_timeout_phrase(timeout)
    )
}

pub fn agent_transport_http_invalid_timeout_message(
    method: &str,
    url: &str,
    timeout_repr: &str,
) -> String {
    format!(
        "{} {} failed: invalid timeout value {}.",
        method_upper(method),
        url,
        timeout_repr
    )
}

pub fn agent_transport_http_error_message(
    method: &str,
    url: &str,
    code: i64,
    detail: Option<&str>,
    reason: Option<&str>,
) -> String {
    let detail_text = detail.unwrap_or_default();
    let reason_text = reason.unwrap_or_default();
    let selected = if detail_text.is_empty() {
        reason_text
    } else {
        detail_text
    };
    format!(
        "{} {} failed: {} {}",
        method_upper(method),
        url,
        code,
        selected
    )
}

pub fn agent_transport_http_url_error_message(method: &str, url: &str, reason: &str) -> String {
    format!("{} {} failed: {}", method_upper(method), url, reason)
}

pub fn agent_transport_http_transport_error_message(
    method: &str,
    url: &str,
    error_text: &str,
) -> String {
    format!("{} {} failed: {}", method_upper(method), url, error_text)
}

fn parse_timeout_seconds_field(value: Option<&JsonValue>) -> Result<Option<f64>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => value.as_f64().map(Some).ok_or_else(|| {
            "transport HTTP request field `timeout_seconds` must be a number or null".to_string()
        }),
    }
}

fn invalid_timeout_result(
    object: &Map<String, JsonValue>,
    timeout: Option<f64>,
    method: &str,
    url: &str,
) -> Option<JsonValue> {
    let timeout = timeout?;
    if timeout.is_finite() && timeout > 0.0 {
        return None;
    }
    let timeout_repr = object
        .get("timeout_repr")
        .and_then(JsonValue::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format_pythonish_timeout(timeout));
    Some(execution_error_payload(
        "invalid_timeout",
        method,
        url,
        agent_transport_http_invalid_timeout_message(method, url, timeout_repr.as_str()),
        None,
        None,
        None,
    ))
}

fn reqwest_error_payload(
    method: &str,
    url: &str,
    timeout: Option<f64>,
    err: reqwest::Error,
) -> JsonValue {
    if err.is_timeout() {
        return execution_error_payload(
            "timeout",
            method,
            url,
            agent_transport_http_timeout_message(method, url, timeout),
            None,
            None,
            None,
        );
    }
    if err.is_builder() {
        return execution_error_payload(
            "url",
            method,
            url,
            agent_transport_http_url_error_message(method, url, &err.to_string()),
            None,
            None,
            None,
        );
    }
    execution_error_payload(
        "transport",
        method,
        url,
        agent_transport_http_transport_error_message(method, url, &err.to_string()),
        None,
        None,
        None,
    )
}

fn execution_error_payload(
    kind: &str,
    method: &str,
    url: &str,
    message: String,
    status_code: Option<i64>,
    detail: Option<String>,
    reason: Option<String>,
) -> JsonValue {
    let mut payload = Map::from_iter([
        ("ok".to_string(), JsonValue::Bool(false)),
        (
            "error_kind".to_string(),
            JsonValue::String(kind.to_string()),
        ),
        (
            "method".to_string(),
            JsonValue::String(method_upper(method)),
        ),
        ("url".to_string(), JsonValue::String(url.to_string())),
        ("message".to_string(), JsonValue::String(message)),
    ]);
    if let Some(status_code) = status_code {
        payload.insert(
            "status_code".to_string(),
            JsonValue::Number(status_code.into()),
        );
    }
    if let Some(detail) = detail {
        payload.insert("detail".to_string(), JsonValue::String(detail));
    }
    if let Some(reason) = reason {
        payload.insert("reason".to_string(), JsonValue::String(reason));
    }
    JsonValue::Object(payload)
}

fn format_pythonish_timeout(timeout: f64) -> String {
    if timeout.fract() == 0.0 {
        format!("{timeout:.1}")
    } else {
        timeout.to_string()
    }
}

fn request_object(request: &JsonValue) -> Result<&Map<String, JsonValue>, String> {
    request
        .as_object()
        .ok_or_else(|| "transport HTTP request must be an object".to_string())
}

fn required_text_field<'a>(
    object: &'a Map<String, JsonValue>,
    field_name: &str,
) -> Result<&'a str, String> {
    let value = object
        .get(field_name)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| format!("transport HTTP request field `{field_name}` must be a string"))?;
    if value.is_empty() {
        return Err(format!(
            "transport HTTP request field `{field_name}` must not be empty"
        ));
    }
    Ok(value)
}

fn required_bytes_field(
    object: &Map<String, JsonValue>,
    field_name: &str,
) -> Result<Vec<u8>, String> {
    let value = object.get(field_name).ok_or_else(|| {
        format!("transport HTTP request field `{field_name}` must be a byte array")
    })?;
    let JsonValue::Array(values) = value else {
        return Err(format!(
            "transport HTTP request field `{field_name}` must be a byte array"
        ));
    };
    values
        .iter()
        .map(|value| {
            let Some(byte) = value.as_u64() else {
                return Err(format!(
                    "transport HTTP request field `{field_name}` must contain byte integers"
                ));
            };
            u8::try_from(byte).map_err(|_| {
                format!(
                    "transport HTTP request field `{field_name}` contains a byte outside 0..255"
                )
            })
        })
        .collect()
}

fn method_text(value: Option<&JsonValue>) -> String {
    value
        .and_then(JsonValue::as_str)
        .unwrap_or("GET")
        .trim()
        .to_ascii_uppercase()
}

fn method_upper(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn default_json_headers() -> Map<String, JsonValue> {
    Map::from_iter([(
        "Accept".to_string(),
        JsonValue::String("application/json".to_string()),
    )])
}

fn merge_headers(
    headers: &mut Map<String, JsonValue>,
    value: Option<&JsonValue>,
) -> Result<(), String> {
    match value {
        None | Some(JsonValue::Null) => Ok(()),
        Some(JsonValue::Object(extra_headers)) => {
            for (key, value) in extra_headers {
                headers.insert(key.to_string(), JsonValue::String(pythonish_text(value)));
            }
            Ok(())
        }
        Some(_) => Err("transport HTTP request field `headers` must be an object".to_string()),
    }
}

fn pythonish_text(value: &JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::Bool(true) => "True".to_string(),
        JsonValue::Bool(false) => "False".to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => value.to_string(),
        JsonValue::Array(_) | JsonValue::Object(_) => encode_value_or(value, &value.to_string()),
    }
}

#[cfg(test)]
mod tests;
