use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonCodecError {
    message: String,
}

impl JsonCodecError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for JsonCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for JsonCodecError {}

impl From<JsonCodecError> for String {
    fn from(error: JsonCodecError) -> Self {
        error.message
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonEncodeStyle {
    Compact,
    Pretty,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct JsonEncodeOptions {
    pub style: JsonEncodeStyle,
    pub trailing_newline: bool,
}

impl JsonEncodeOptions {
    pub fn compact() -> Self {
        Self {
            style: JsonEncodeStyle::Compact,
            trailing_newline: false,
        }
    }

    pub fn pretty() -> Self {
        Self {
            style: JsonEncodeStyle::Pretty,
            trailing_newline: false,
        }
    }

    pub fn with_trailing_newline(mut self) -> Self {
        self.trailing_newline = true;
        self
    }
}

pub struct JsonCodec;

impl JsonCodec {
    pub fn encode_serializable<T>(
        value: &T,
        options: JsonEncodeOptions,
    ) -> Result<String, JsonCodecError>
    where
        T: Serialize + ?Sized,
    {
        Self::encode_serializable_with_error_prefix(value, options, "Failed to encode JSON")
    }

    pub fn encode_serializable_with_error_prefix<T>(
        value: &T,
        options: JsonEncodeOptions,
        error_prefix: &str,
    ) -> Result<String, JsonCodecError>
    where
        T: Serialize + ?Sized,
    {
        let mut text = match options.style {
            JsonEncodeStyle::Compact => serde_json::to_string(value),
            JsonEncodeStyle::Pretty => serde_json::to_string_pretty(value),
        }
        .map_err(|err| JsonCodecError::new(format!("{error_prefix}: {err}")))?;
        if options.trailing_newline {
            text.push('\n');
        }
        Ok(text)
    }

    pub fn encode_value(
        value: &JsonValue,
        options: JsonEncodeOptions,
    ) -> Result<String, JsonCodecError> {
        Self::encode_serializable(value, options)
    }

    pub fn encode_value_to_vec(
        value: &JsonValue,
        options: JsonEncodeOptions,
    ) -> Result<Vec<u8>, JsonCodecError> {
        Self::encode_value_to_vec_with_error_prefix(value, options, "Failed to encode JSON")
    }

    pub fn encode_value_to_vec_with_error_prefix(
        value: &JsonValue,
        options: JsonEncodeOptions,
        error_prefix: &str,
    ) -> Result<Vec<u8>, JsonCodecError> {
        Self::encode_serializable_to_vec_with_error_prefix(value, options, error_prefix)
    }

    pub fn encode_serializable_to_vec_with_error_prefix<T>(
        value: &T,
        options: JsonEncodeOptions,
        error_prefix: &str,
    ) -> Result<Vec<u8>, JsonCodecError>
    where
        T: Serialize + ?Sized,
    {
        Self::encode_serializable_with_error_prefix(value, options, error_prefix)
            .map(String::into_bytes)
    }

    pub fn to_value_serializable<T>(value: &T) -> Result<JsonValue, JsonCodecError>
    where
        T: Serialize + ?Sized,
    {
        serde_json::to_value(value).map_err(|err| JsonCodecError::new(err.to_string()))
    }

    pub fn from_value_deserializable<T>(value: JsonValue) -> Result<T, JsonCodecError>
    where
        T: DeserializeOwned,
    {
        serde_json::from_value(value).map_err(|err| JsonCodecError::new(err.to_string()))
    }

    pub fn parse_value(text: &str, label: &str) -> Result<JsonValue, JsonCodecError> {
        serde_json::from_str(text)
            .map_err(|err| JsonCodecError::new(format!("Invalid {label} JSON: {err}")))
    }

    pub fn parse_deserializable_with_error_prefix<T>(
        text: &str,
        error_prefix: &str,
    ) -> Result<T, JsonCodecError>
    where
        T: DeserializeOwned,
    {
        serde_json::from_str(text)
            .map_err(|err| JsonCodecError::new(format!("{error_prefix}: {err}")))
    }

    pub fn parse_value_with_error_prefix(
        text: &str,
        error_prefix: &str,
    ) -> Result<JsonValue, JsonCodecError> {
        serde_json::from_str(text)
            .map_err(|err| JsonCodecError::new(format!("{error_prefix}: {err}")))
    }

    pub fn parse_slice_with_error_prefix(
        bytes: &[u8],
        error_prefix: &str,
    ) -> Result<JsonValue, JsonCodecError> {
        serde_json::from_slice(bytes)
            .map_err(|err| JsonCodecError::new(format!("{error_prefix}: {err}")))
    }

    pub fn parse_object(
        text: &str,
        label: &str,
    ) -> Result<JsonMap<String, JsonValue>, JsonCodecError> {
        match Self::parse_value(text, label)? {
            JsonValue::Object(object) => Ok(object),
            _ => Err(JsonCodecError::new(format!(
                "{label} JSON must be an object."
            ))),
        }
    }

    pub fn parse_object_with_error_prefix(
        text: &str,
        error_prefix: &str,
        object_error: &str,
    ) -> Result<JsonMap<String, JsonValue>, JsonCodecError> {
        match Self::parse_value_with_error_prefix(text, error_prefix)? {
            JsonValue::Object(object) => Ok(object),
            _ => Err(JsonCodecError::new(object_error)),
        }
    }

    pub fn parse_array_with_error_prefix(
        text: &str,
        error_prefix: &str,
        array_error: &str,
    ) -> Result<Vec<JsonValue>, JsonCodecError> {
        match Self::parse_value_with_error_prefix(text, error_prefix)? {
            JsonValue::Array(values) => Ok(values),
            _ => Err(JsonCodecError::new(array_error)),
        }
    }
}
