use crate::json_support::{json, JsonValue};

pub mod bindings;
pub mod doctor;
pub mod link;
pub mod lockfile;
pub mod manifest;
pub mod materializer;
pub mod readiness;
pub mod release;
pub mod resolver;
pub mod status;
pub mod update;

pub type ExternalResult<T> = Result<T, ExternalError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalError {
    code: String,
    message: String,
}

impl ExternalError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::with_code("external_error", message)
    }

    pub fn with_code(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn to_json_value(&self) -> JsonValue {
        json!({
            "code": self.code,
            "message": self.message,
        })
    }
}

impl std::fmt::Display for ExternalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExternalError {}

#[cfg(test)]
mod tests;
