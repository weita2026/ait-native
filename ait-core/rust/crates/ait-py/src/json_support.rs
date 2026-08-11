use ait_core::json_support::JsonCodec;
use ait_core::json_support::{JsonMap as Map, JsonValue};

pub(crate) fn parse_json_value_with_error_prefix(
    text: &str,
    error_prefix: &str,
) -> Result<JsonValue, String> {
    JsonCodec::parse_value_with_error_prefix(text, error_prefix).map_err(String::from)
}

pub(crate) fn parse_json_array_with_error_prefix(
    text: &str,
    error_prefix: &str,
    array_error: &str,
) -> Result<Vec<JsonValue>, String> {
    JsonCodec::parse_array_with_error_prefix(text, error_prefix, array_error).map_err(String::from)
}

pub(crate) fn parse_json_object_with_error_prefix(
    text: &str,
    error_prefix: &str,
    object_error: &str,
) -> Result<Map<String, JsonValue>, String> {
    JsonCodec::parse_object_with_error_prefix(text, error_prefix, object_error)
        .map_err(String::from)
}

pub(crate) fn parse_json_object_or_empty(text: &str) -> Map<String, JsonValue> {
    match JsonCodec::parse_value_with_error_prefix(text, "Invalid JSON") {
        Ok(JsonValue::Object(payload)) => payload,
        _ => Map::new(),
    }
}

pub(crate) fn encode_json_value_compact(value: &JsonValue) -> Result<String, String> {
    JsonCodec::encode_value(value, ait_core::json_support::JsonEncodeOptions::compact())
        .map_err(|err| strip_encode_error_prefix(err.message()))
}

pub(crate) fn encode_json_value_compact_or_default(value: &JsonValue) -> String {
    encode_json_value_compact(value).unwrap_or_default()
}

pub(crate) fn encode_json_value_pretty(value: &JsonValue) -> Result<String, String> {
    JsonCodec::encode_value(value, ait_core::json_support::JsonEncodeOptions::pretty())
        .map_err(|err| strip_encode_error_prefix(err.message()))
}

fn strip_encode_error_prefix(message: &str) -> String {
    message
        .strip_prefix("Failed to encode JSON: ")
        .unwrap_or(message)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ait_core::json_support::json;

    #[test]
    fn parse_value_preserves_error_prefix() {
        let error = parse_json_value_with_error_prefix("{", "headers_json must be valid JSON")
            .expect_err("invalid JSON should fail");
        assert!(error.starts_with("headers_json must be valid JSON:"));
    }

    #[test]
    fn parse_object_or_empty_tolerates_missing_or_non_object_payloads() {
        assert!(parse_json_object_or_empty("[]").is_empty());
        assert!(parse_json_object_or_empty("{").is_empty());
    }

    #[test]
    fn compact_and_pretty_encoding_match_expected_shape() {
        let payload = json!({"name": "plan"});
        assert_eq!(
            encode_json_value_compact(&payload).expect("compact JSON"),
            "{\"name\":\"plan\"}"
        );
        assert!(encode_json_value_pretty(&payload)
            .expect("pretty JSON")
            .contains("\n  \"name\": \"plan\"\n"));
    }
}
