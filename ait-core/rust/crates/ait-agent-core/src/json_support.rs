use ait_core::json_support::JsonValue;
use ait_core::json_support::{JsonCodec, JsonEncodeOptions};
use serde::{de::DeserializeOwned, Serialize};

pub(crate) fn decode_from_value<T>(value: &JsonValue, error_prefix: &str) -> Result<T, String>
where
    T: DeserializeOwned,
{
    JsonCodec::from_value_deserializable(value.clone())
        .map_err(|err| format!("{error_prefix}: {err}"))
}

pub(crate) fn encode_to_value<T>(value: &T, error_prefix: &str) -> Result<JsonValue, String>
where
    T: Serialize + ?Sized,
{
    JsonCodec::to_value_serializable(value).map_err(|err| format!("{error_prefix}: {err}"))
}

pub(crate) fn parse_value(text: &str, error_prefix: &str) -> Result<JsonValue, String> {
    JsonCodec::parse_value_with_error_prefix(text, error_prefix).map_err(String::from)
}

pub(crate) fn encode_value(value: &JsonValue, error_prefix: &str) -> Result<String, String> {
    JsonCodec::encode_serializable_with_error_prefix(
        value,
        JsonEncodeOptions::compact(),
        error_prefix,
    )
    .map_err(String::from)
}

pub(crate) fn encode_value_or(value: &JsonValue, fallback: &str) -> String {
    JsonCodec::encode_value(value, JsonEncodeOptions::compact())
        .unwrap_or_else(|_| fallback.to_string())
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
