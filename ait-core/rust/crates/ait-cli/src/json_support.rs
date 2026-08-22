use ait_core::json_support::{JsonCodec, JsonEncodeOptions};
use ait_core::json_support::{JsonMap, JsonValue};
use std::io::Write;

pub(crate) fn parse_value(text: &str, error_prefix: &str) -> Result<JsonValue, String> {
    JsonCodec::parse_value_with_error_prefix(text, error_prefix).map_err(String::from)
}

pub(crate) fn parse_value_error_string(text: &str) -> Result<JsonValue, String> {
    parse_value(text, "Invalid JSON").map_err(|err| strip_error_prefix(err, "Invalid JSON"))
}

pub(crate) fn parse_value_or(text: &str, fallback: JsonValue) -> JsonValue {
    parse_value(text, "Invalid JSON").unwrap_or(fallback)
}

pub(crate) fn parse_value_option(text: &str) -> Option<JsonValue> {
    parse_value(text, "Invalid JSON").ok()
}

pub(crate) fn parse_object_or_empty(text: &str) -> JsonMap<String, JsonValue> {
    match parse_value(text, "Invalid JSON") {
        Ok(JsonValue::Object(object)) => object,
        _ => JsonMap::new(),
    }
}

pub(crate) fn parse_slice_value(bytes: &[u8], error_prefix: &str) -> Result<JsonValue, String> {
    JsonCodec::parse_slice_with_error_prefix(bytes, error_prefix).map_err(String::from)
}

pub(crate) fn encode_value(value: &JsonValue, error_prefix: &str) -> Result<String, String> {
    JsonCodec::encode_serializable_with_error_prefix(
        value,
        JsonEncodeOptions::compact(),
        error_prefix,
    )
    .map_err(String::from)
}

pub(crate) fn encode_value_pretty(value: &JsonValue, error_prefix: &str) -> Result<String, String> {
    JsonCodec::encode_serializable_with_error_prefix(
        value,
        JsonEncodeOptions::pretty(),
        error_prefix,
    )
    .map_err(String::from)
}

pub(crate) fn encode_value_pretty_with_newline(
    value: &JsonValue,
    error_prefix: &str,
) -> Result<String, String> {
    JsonCodec::encode_serializable_with_error_prefix(
        value,
        JsonEncodeOptions::pretty().with_trailing_newline(),
        error_prefix,
    )
    .map_err(String::from)
}

pub(crate) fn encode_value_pretty_with_newline_error_string(
    value: &JsonValue,
) -> Result<String, String> {
    encode_value_pretty_with_newline(value, "Failed to encode JSON")
        .map_err(|err| strip_error_prefix(err, "Failed to encode JSON"))
}

pub(crate) fn encode_value_to_vec(
    value: &JsonValue,
    error_prefix: &str,
) -> Result<Vec<u8>, String> {
    JsonCodec::encode_value_to_vec_with_error_prefix(
        value,
        JsonEncodeOptions::compact(),
        error_prefix,
    )
    .map_err(String::from)
}

#[cfg(test)]
pub(crate) fn encode_value_to_vec_error_string(value: &JsonValue) -> Result<Vec<u8>, String> {
    encode_value_to_vec(value, "Failed to encode JSON")
        .map_err(|err| strip_error_prefix(err, "Failed to encode JSON"))
}

pub(crate) fn encode_value_pretty_to_vec(
    value: &JsonValue,
    error_prefix: &str,
) -> Result<Vec<u8>, String> {
    JsonCodec::encode_serializable_to_vec_with_error_prefix(
        value,
        JsonEncodeOptions::pretty(),
        error_prefix,
    )
    .map_err(String::from)
}

pub(crate) fn encode_value_or(value: &JsonValue, fallback: &str) -> String {
    JsonCodec::encode_value(value, JsonEncodeOptions::compact())
        .unwrap_or_else(|_| fallback.to_string())
}

pub(crate) fn encode_string_or(value: &str, fallback: &str) -> String {
    encode_value_or(&JsonValue::String(value.to_string()), fallback)
}

pub(crate) fn write_pretty_value<W>(
    writer: &mut W,
    value: &JsonValue,
    error_prefix: &str,
) -> Result<(), String>
where
    W: Write + ?Sized,
{
    let text = encode_value_pretty(value, error_prefix)?;
    writer
        .write_all(text.as_bytes())
        .map_err(|err| format!("{error_prefix}: {err}"))
}

fn strip_error_prefix(message: String, prefix: &str) -> String {
    message
        .strip_prefix(&format!("{prefix}: "))
        .unwrap_or(message.as_str())
        .to_string()
}
