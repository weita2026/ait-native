use crate::json_support::{JsonCodec, JsonValue as Value};

use super::{PlanHttpClientError, PlanHttpClientResult};

pub(super) fn parse_object_payload(payload: Option<Value>) -> PlanHttpClientResult<Value> {
    match payload {
        Some(Value::Object(map)) => Ok(Value::Object(map)),
        Some(_) => Err(PlanHttpClientError::Remote(
            "Rust plan HTTP client expected an object payload.".to_string(),
        )),
        None => Err(PlanHttpClientError::Remote(
            "Rust plan HTTP client expected a non-empty object payload.".to_string(),
        )),
    }
}

pub(super) fn parse_json_bytes_payload(
    method: &str,
    url: &str,
    bytes: Vec<u8>,
) -> PlanHttpClientResult<Option<Value>> {
    if bytes.is_empty() {
        return Ok(None);
    }
    JsonCodec::parse_slice_with_error_prefix(
        &bytes,
        &format!("{method} {url} failed: invalid JSON response"),
    )
    .map(Some)
    .map_err(|err| PlanHttpClientError::Remote(err.to_string()))
}

pub(super) fn parse_list_payload(payload: Option<Value>) -> PlanHttpClientResult<Vec<Value>> {
    match payload {
        Some(Value::Array(rows)) => Ok(rows),
        Some(_) => Err(PlanHttpClientError::Remote(
            "Rust plan HTTP client expected a list payload.".to_string(),
        )),
        None => Err(PlanHttpClientError::Remote(
            "Rust plan HTTP client expected a non-empty list payload.".to_string(),
        )),
    }
}

pub(super) fn parse_any_payload(payload: Option<Value>) -> PlanHttpClientResult<Value> {
    Ok(payload.unwrap_or(Value::Null))
}
