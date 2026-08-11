use std::fmt;
use std::path::PathBuf;

pub type TomlTable = toml::map::Map<String, toml::Value>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TomlFieldError {
    message: String,
}

impl TomlFieldError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for TomlFieldError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TomlFieldError {}

impl From<TomlFieldError> for String {
    fn from(error: TomlFieldError) -> Self {
        error.message
    }
}

pub fn required_text_field(table: &TomlTable, key: &str) -> Result<String, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::String(text)) => Ok(text.clone()),
        Some(value) => Err(type_error(key, "a TOML string", value)),
        None => Err(missing_required(key)),
    }
}

pub fn optional_text_field(table: &TomlTable, key: &str) -> Result<Option<String>, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::String(text)) => Ok(Some(text.clone())),
        Some(value) => Err(type_error(key, "a TOML string", value)),
        None => Ok(None),
    }
}

pub fn required_integer_field(table: &TomlTable, key: &str) -> Result<i64, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Integer(number)) => Ok(*number),
        Some(value) => Err(type_error(key, "a TOML integer", value)),
        None => Err(missing_required(key)),
    }
}

pub fn optional_integer_field(table: &TomlTable, key: &str) -> Result<Option<i64>, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Integer(number)) => Ok(Some(*number)),
        Some(value) => Err(type_error(key, "a TOML integer", value)),
        None => Ok(None),
    }
}

pub fn required_bool_field(table: &TomlTable, key: &str) -> Result<bool, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Boolean(flag)) => Ok(*flag),
        Some(value) => Err(type_error(key, "a TOML boolean", value)),
        None => Err(missing_required(key)),
    }
}

pub fn optional_bool_field(table: &TomlTable, key: &str) -> Result<Option<bool>, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Boolean(flag)) => Ok(Some(*flag)),
        Some(value) => Err(type_error(key, "a TOML boolean", value)),
        None => Ok(None),
    }
}

pub fn required_array_field(
    table: &TomlTable,
    key: &str,
) -> Result<Vec<toml::Value>, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Array(items)) => Ok(items.clone()),
        Some(value) => Err(type_error(key, "a TOML array", value)),
        None => Err(missing_required(key)),
    }
}

pub fn optional_array_field(
    table: &TomlTable,
    key: &str,
) -> Result<Option<Vec<toml::Value>>, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Array(items)) => Ok(Some(items.clone())),
        Some(value) => Err(type_error(key, "a TOML array", value)),
        None => Ok(None),
    }
}

pub fn required_table_field(table: &TomlTable, key: &str) -> Result<TomlTable, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Table(value)) => Ok(value.clone()),
        Some(value) => Err(type_error(key, "a TOML table", value)),
        None => Err(missing_required(key)),
    }
}

pub fn optional_table_field(
    table: &TomlTable,
    key: &str,
) -> Result<Option<TomlTable>, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Table(value)) => Ok(Some(value.clone())),
        Some(value) => Err(type_error(key, "a TOML table", value)),
        None => Ok(None),
    }
}

pub fn required_path_field(table: &TomlTable, key: &str) -> Result<PathBuf, TomlFieldError> {
    required_text_field(table, key).map(PathBuf::from)
}

pub fn optional_path_field(
    table: &TomlTable,
    key: &str,
) -> Result<Option<PathBuf>, TomlFieldError> {
    optional_text_field(table, key).map(|value| value.map(PathBuf::from))
}

pub fn required_string_list_field(
    table: &TomlTable,
    key: &str,
) -> Result<Vec<String>, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Array(items)) => string_list(key, items),
        Some(value) => Err(type_error(key, "a TOML string array", value)),
        None => Err(missing_required(key)),
    }
}

pub fn optional_string_list_field(
    table: &TomlTable,
    key: &str,
) -> Result<Option<Vec<String>>, TomlFieldError> {
    match table.get(key) {
        Some(toml::Value::Array(items)) => Ok(Some(string_list(key, items)?)),
        Some(value) => Err(type_error(key, "a TOML string array", value)),
        None => Ok(None),
    }
}

fn string_list(key: &str, items: &[toml::Value]) -> Result<Vec<String>, TomlFieldError> {
    items
        .iter()
        .enumerate()
        .map(|(index, item)| match item {
            toml::Value::String(text) => Ok(text.clone()),
            value => Err(TomlFieldError::new(format!(
                "Field `{key}` item {index} must be a TOML string, got {}.",
                toml_type_name(value)
            ))),
        })
        .collect()
}

fn missing_required(key: &str) -> TomlFieldError {
    TomlFieldError::new(format!("Missing required TOML field `{key}`."))
}

fn type_error(key: &str, expected: &str, value: &toml::Value) -> TomlFieldError {
    TomlFieldError::new(format!(
        "Field `{key}` must be {expected}, got {}.",
        toml_type_name(value)
    ))
}

fn toml_type_name(value: &toml::Value) -> &'static str {
    match value {
        toml::Value::String(_) => "string",
        toml::Value::Integer(_) => "integer",
        toml::Value::Float(_) => "float",
        toml::Value::Boolean(_) => "boolean",
        toml::Value::Datetime(_) => "datetime",
        toml::Value::Array(_) => "array",
        toml::Value::Table(_) => "table",
    }
}
