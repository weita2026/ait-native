use ait_core::json_support::{json, JsonValue};
use std::collections::BTreeSet;

pub const DEFAULT_RETRYABLE_ERRNOS: [i64; 6] = [54, 60, 61, 104, 110, 111];
pub const DEFAULT_RETRYABLE_MARKERS: [&str; 7] = [
    "timed out",
    "connection reset by peer",
    "remote end closed connection without response",
    "temporarily unavailable",
    "connection aborted",
    "broken pipe",
    "network is unreachable",
];
pub const DEFAULT_SERVER_READ_MARKERS: [&str; 4] = [
    "500 internal server error",
    "502 bad gateway",
    "503 service unavailable",
    "504 gateway timeout",
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ErrorFrame {
    class_names: Vec<String>,
    errno: Option<i64>,
    text: String,
}

pub fn agent_transport_retry_default_errnos_json() -> JsonValue {
    json!(DEFAULT_RETRYABLE_ERRNOS)
}

pub fn agent_transport_retry_default_markers_json() -> JsonValue {
    json!(DEFAULT_RETRYABLE_MARKERS)
}

pub fn agent_transport_retry_default_server_read_markers_json() -> JsonValue {
    json!(DEFAULT_SERVER_READ_MARKERS)
}

pub fn agent_transport_retry_timeout_value(
    timeout: Option<f64>,
    minimum: Option<f64>,
) -> Option<f64> {
    match (timeout, minimum) {
        (None, minimum) => minimum,
        (Some(timeout), None) => Some(timeout),
        (Some(timeout), Some(minimum)) => {
            if minimum > timeout {
                Some(minimum)
            } else {
                Some(timeout)
            }
        }
    }
}

pub fn agent_transport_retry_timeout_phrase(timeout: Option<f64>) -> String {
    match timeout {
        Some(value) => format!(" after {} seconds", format_python_general_float(value)),
        None => String::new(),
    }
}

pub fn agent_transport_retry_delay_seconds(base_delay_seconds: f64, retry_index: i64) -> f64 {
    let base = if base_delay_seconds < 0.0 {
        0.0
    } else {
        base_delay_seconds
    };
    let exponent = retry_index.max(0).min(i32::MAX as i64) as i32;
    base * 2_f64.powi(exponent)
}

pub fn agent_transport_retry_is_loopback_url(value: &str) -> bool {
    let Some(authority) = url_authority(value) else {
        return false;
    };
    let host = url_host_from_authority(authority);
    matches!(host.as_deref(), Some("127.0.0.1" | "localhost" | "::1"))
}

pub fn agent_transport_retry_is_retryable_transport_error_json(
    request: &JsonValue,
) -> Result<bool, String> {
    let frames = parse_error_chain(request)?;
    let errnos = parse_errno_set(request.get("errnos"))?;
    let markers = parse_marker_list(request.get("markers"), &DEFAULT_RETRYABLE_MARKERS)?;
    Ok(is_retryable_transport_error_frames(
        &frames, &errnos, &markers,
    ))
}

pub fn agent_transport_retry_is_retryable_server_read_error_json(
    request: &JsonValue,
) -> Result<bool, String> {
    let frames = parse_error_chain(request)?;
    let errnos = parse_errno_set(request.get("errnos"))?;
    let transport_markers =
        parse_marker_list(request.get("transport_markers"), &DEFAULT_RETRYABLE_MARKERS)?;
    let server_markers =
        parse_marker_list(request.get("server_markers"), &DEFAULT_SERVER_READ_MARKERS)?;
    if is_retryable_transport_error_frames(&frames, &errnos, &transport_markers) {
        return Ok(true);
    }
    let text = frames
        .first()
        .map(|frame| frame.text.trim().to_ascii_lowercase())
        .unwrap_or_default();
    Ok(!text.is_empty()
        && server_markers
            .iter()
            .any(|marker| text.contains(marker.as_str())))
}

fn is_retryable_transport_error_frames(
    frames: &[ErrorFrame],
    errnos: &BTreeSet<i64>,
    markers: &[String],
) -> bool {
    frames.iter().any(|frame| {
        is_retryable_transport_class(frame)
            || (is_os_error(frame) && frame.errno.is_some_and(|errno| errnos.contains(&errno)))
            || frame_has_retryable_marker(frame, markers)
    })
}

fn is_retryable_transport_class(frame: &ErrorFrame) -> bool {
    [
        "TimeoutError",
        "RemoteDisconnected",
        "ConnectionResetError",
        "BrokenPipeError",
        "ConnectionAbortedError",
    ]
    .iter()
    .any(|name| frame_has_class(frame, name))
}

fn is_os_error(frame: &ErrorFrame) -> bool {
    frame_has_class(frame, "OSError")
}

fn frame_has_retryable_marker(frame: &ErrorFrame, markers: &[String]) -> bool {
    let text = frame.text.trim().to_ascii_lowercase();
    !text.is_empty() && markers.iter().any(|marker| text.contains(marker.as_str()))
}

fn frame_has_class(frame: &ErrorFrame, expected_simple_name: &str) -> bool {
    frame.class_names.iter().any(|name| {
        let text = name.trim();
        text == expected_simple_name
            || text
                .rsplit('.')
                .next()
                .is_some_and(|simple| simple == expected_simple_name)
    })
}

fn parse_error_chain(request: &JsonValue) -> Result<Vec<ErrorFrame>, String> {
    let Some(value) = request
        .get("chain")
        .or_else(|| request.get("exception_chain"))
    else {
        return Ok(Vec::new());
    };
    let frames = value
        .as_array()
        .ok_or_else(|| "transport retry request field `chain` must be a list".to_string())?;
    frames
        .iter()
        .enumerate()
        .map(|(index, value)| parse_error_frame(index, value))
        .collect()
}

fn parse_error_frame(index: usize, value: &JsonValue) -> Result<ErrorFrame, String> {
    let object = value
        .as_object()
        .ok_or_else(|| format!("transport retry chain entry {index} must be an object"))?;
    let mut class_names = Vec::new();
    if let Some(values) = object.get("class_names").and_then(JsonValue::as_array) {
        for value in values {
            if let Some(text) = value.as_str() {
                let normalized = text.trim();
                if !normalized.is_empty() {
                    class_names.push(normalized.to_string());
                }
            }
        }
    }
    for key in ["class_name", "qualified_class_name", "type"] {
        if let Some(text) = object.get(key).and_then(JsonValue::as_str) {
            let normalized = text.trim();
            if !normalized.is_empty() {
                class_names.push(normalized.to_string());
            }
        }
    }
    class_names.sort();
    class_names.dedup();
    let errno = object.get("errno").and_then(json_i64);
    let text = object
        .get("text")
        .or_else(|| object.get("message"))
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();
    Ok(ErrorFrame {
        class_names,
        errno,
        text,
    })
}

fn parse_errno_set(value: Option<&JsonValue>) -> Result<BTreeSet<i64>, String> {
    match value {
        None | Some(JsonValue::Null) => Ok(DEFAULT_RETRYABLE_ERRNOS.into_iter().collect()),
        Some(JsonValue::Array(values)) => {
            let mut errnos = BTreeSet::new();
            for value in values {
                let Some(errno) = json_i64(value) else {
                    return Err("transport retry `errnos` entries must be integers".to_string());
                };
                errnos.insert(errno);
            }
            Ok(errnos)
        }
        Some(_) => Err("transport retry `errnos` must be a list of integers".to_string()),
    }
}

fn parse_marker_list(
    value: Option<&JsonValue>,
    defaults: &[&'static str],
) -> Result<Vec<String>, String> {
    match value {
        None | Some(JsonValue::Null) => {
            Ok(defaults.iter().map(|value| (*value).to_string()).collect())
        }
        Some(JsonValue::Array(values)) => {
            let mut markers = Vec::new();
            for value in values {
                let Some(text) = value.as_str() else {
                    return Err("transport retry marker entries must be strings".to_string());
                };
                markers.push(text.to_string());
            }
            Ok(markers)
        }
        Some(_) => Err("transport retry markers must be a list of strings".to_string()),
    }
}

fn json_i64(value: &JsonValue) -> Option<i64> {
    match value {
        JsonValue::Number(number) => number.as_i64(),
        JsonValue::String(text) => text.trim().parse::<i64>().ok(),
        _ => None,
    }
}

fn url_authority(value: &str) -> Option<&str> {
    let text = value.trim();
    if let Some((_, rest)) = text.split_once("://") {
        return Some(rest.split(['/', '?', '#']).next().unwrap_or_default());
    }
    text.strip_prefix("//")
        .map(|rest| rest.split(['/', '?', '#']).next().unwrap_or_default())
}

fn url_host_from_authority(authority: &str) -> Option<String> {
    let without_userinfo = authority.rsplit('@').next().unwrap_or_default();
    if without_userinfo.is_empty() {
        return None;
    }
    if let Some(rest) = without_userinfo.strip_prefix('[') {
        let (host, _) = rest.split_once(']')?;
        let host = host.trim().to_ascii_lowercase();
        return (!host.is_empty()).then_some(host);
    }
    let host = without_userinfo
        .split(':')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn format_python_general_float(value: f64) -> String {
    if value.is_nan() {
        return "nan".to_string();
    }
    if value.is_infinite() {
        return if value.is_sign_negative() {
            "-inf".to_string()
        } else {
            "inf".to_string()
        };
    }
    if value == 0.0 {
        return "0".to_string();
    }
    let abs = value.abs();
    let exponent = abs.log10().floor() as i32;
    if !(-4..6).contains(&exponent) {
        return format_python_general_float_scientific(value);
    }
    let digits_before_decimal = if abs < 1.0 { 0 } else { exponent + 1 };
    let precision = (6 - digits_before_decimal).max(0) as usize;
    trim_fixed_float(format!("{value:.precision$}"))
}

fn format_python_general_float_scientific(value: f64) -> String {
    let raw = format!("{value:.5e}");
    let Some((mantissa, exponent)) = raw.split_once('e') else {
        return raw;
    };
    let mantissa = trim_fixed_float(mantissa.to_string());
    let exponent_value = exponent.parse::<i32>().unwrap_or(0);
    let sign = if exponent_value < 0 { '-' } else { '+' };
    format!("{mantissa}e{sign}{:02}", exponent_value.abs())
}

fn trim_fixed_float(mut text: String) -> String {
    if let Some(dot_index) = text.find('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.len() == dot_index + 1 {
            text.pop();
        }
    }
    text
}

#[cfg(test)]
mod tests;
