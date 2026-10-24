use serde_json::{Map as JsonMap, Value as JsonValue};
use std::fmt;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonFieldError {
    message: String,
}

impl JsonFieldError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for JsonFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for JsonFieldError {}

impl From<JsonFieldError> for String {
    fn from(error: JsonFieldError) -> Self {
        error.message
    }
}

pub fn required_text_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<String, JsonFieldError> {
    match object.get(field) {
        None => Err(missing_required(field)),
        Some(JsonValue::String(value)) => Ok(value.clone()),
        Some(JsonValue::Null) => Err(field_type(field, "JSON string", "null")),
        Some(value) => Err(field_type(field, "JSON string", json_kind(value))),
    }
}

pub fn optional_text_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<String>, JsonFieldError> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(Some(value.clone())),
        Some(value) => Err(field_type(field, "JSON string or null", json_kind(value))),
    }
}

pub fn required_bool_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<bool, JsonFieldError> {
    match object.get(field) {
        None => Err(missing_required(field)),
        Some(JsonValue::Bool(value)) => Ok(*value),
        Some(JsonValue::Null) => Err(field_type(field, "JSON boolean", "null")),
        Some(value) => Err(field_type(field, "JSON boolean", json_kind(value))),
    }
}

pub fn optional_bool_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<bool>, JsonFieldError> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Bool(value)) => Ok(Some(*value)),
        Some(value) => Err(field_type(field, "JSON boolean or null", json_kind(value))),
    }
}

pub fn required_integer_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<i64, JsonFieldError> {
    match object.get(field) {
        None => Err(missing_required(field)),
        Some(value) => integer_value(value, field, "JSON integer"),
    }
}

pub fn optional_integer_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<i64>, JsonFieldError> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(value) => integer_value(value, field, "JSON integer or null").map(Some),
    }
}

pub fn required_object_field<'a>(
    object: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<&'a JsonMap<String, JsonValue>, JsonFieldError> {
    match object.get(field) {
        None => Err(missing_required(field)),
        Some(JsonValue::Object(value)) => Ok(value),
        Some(JsonValue::Null) => Err(field_type(field, "JSON object", "null")),
        Some(value) => Err(field_type(field, "JSON object", json_kind(value))),
    }
}

pub fn required_object_value<'a>(
    value: &'a JsonValue,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, JsonFieldError> {
    match value {
        JsonValue::Object(object) => Ok(object),
        JsonValue::Null => Err(value_type(label, "JSON object", "null")),
        other => Err(value_type(label, "JSON object", json_kind(other))),
    }
}

pub fn optional_object_field<'a>(
    object: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<&'a JsonMap<String, JsonValue>>, JsonFieldError> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Object(value)) => Ok(Some(value)),
        Some(value) => Err(field_type(field, "JSON object or null", json_kind(value))),
    }
}

pub fn required_array_field<'a>(
    object: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<&'a [JsonValue], JsonFieldError> {
    match object.get(field) {
        None => Err(missing_required(field)),
        Some(JsonValue::Array(value)) => Ok(value.as_slice()),
        Some(JsonValue::Null) => Err(field_type(field, "JSON array", "null")),
        Some(value) => Err(field_type(field, "JSON array", json_kind(value))),
    }
}

pub fn required_array_value<'a>(
    value: &'a JsonValue,
    label: &str,
) -> Result<&'a [JsonValue], JsonFieldError> {
    match value {
        JsonValue::Array(array) => Ok(array.as_slice()),
        JsonValue::Null => Err(value_type(label, "JSON array", "null")),
        other => Err(value_type(label, "JSON array", json_kind(other))),
    }
}

pub fn optional_array_field<'a>(
    object: &'a JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<&'a [JsonValue]>, JsonFieldError> {
    match object.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::Array(value)) => Ok(Some(value.as_slice())),
        Some(value) => Err(field_type(field, "JSON array or null", json_kind(value))),
    }
}

pub fn required_path_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<PathBuf, JsonFieldError> {
    required_text_field(object, field).map(PathBuf::from)
}

pub fn optional_path_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<PathBuf>, JsonFieldError> {
    optional_text_field(object, field).map(|value| value.map(PathBuf::from))
}

fn integer_value(
    value: &JsonValue,
    field: &str,
    expected: &'static str,
) -> Result<i64, JsonFieldError> {
    match value {
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| field_type(field, expected, json_kind(value))),
        JsonValue::Null => Err(field_type(field, expected, "null")),
        other => Err(field_type(field, expected, json_kind(other))),
    }
}

fn missing_required(field: &str) -> JsonFieldError {
    JsonFieldError::new(format!("Missing required field `{field}`."))
}

fn field_type(field: &str, expected: &str, actual: &str) -> JsonFieldError {
    JsonFieldError::new(format!(
        "Field `{field}` must be a {expected}, got {actual}."
    ))
}

fn value_type(label: &str, expected: &str, actual: &str) -> JsonFieldError {
    JsonFieldError::new(format!("{label} must be a {expected}, got {actual}."))
}

fn json_kind(value: &JsonValue) -> &'static str {
    match value {
        JsonValue::Null => "null",
        JsonValue::Bool(_) => "boolean",
        JsonValue::Number(_) => "number",
        JsonValue::String(_) => "string",
        JsonValue::Array(_) => "array",
        JsonValue::Object(_) => "object",
    }
}
