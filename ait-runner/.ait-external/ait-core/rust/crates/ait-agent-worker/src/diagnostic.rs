use std::collections::BTreeMap;
use std::fmt;

use ait_core::json_support::{JsonCodec, JsonEncodeOptions, JsonValue};
use serde::Serialize;

pub const WORKER_ERROR_CONTRACT: &str = "ait.agent.worker.error.v1";
pub const EXIT_INVALID_REQUEST: u8 = 2;
pub const EXIT_INVALID_CONFIGURATION: u8 = 3;
pub const EXIT_RUNTIME_UNAVAILABLE: u8 = 4;

#[derive(Debug, Clone)]
pub struct WorkerDiagnostic {
    pub code: &'static str,
    pub message: String,
    pub exit_code: u8,
    pub details: Box<BTreeMap<String, JsonValue>>,
}

impl WorkerDiagnostic {
    pub fn new(code: &'static str, message: impl Into<String>, exit_code: u8) -> Self {
        Self {
            code,
            message: message.into(),
            exit_code,
            details: Box::default(),
        }
    }

    pub fn with_detail(mut self, key: impl Into<String>, value: impl Into<JsonValue>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }

    pub fn render_json(&self) -> String {
        #[derive(Serialize)]
        struct DiagnosticPayload<'a> {
            contract: &'static str,
            binary: &'static str,
            status: &'static str,
            code: &'static str,
            message: &'a str,
            exit_code: u8,
            details: &'a BTreeMap<String, JsonValue>,
            python_worker_execution_allowed: bool,
        }

        let payload = DiagnosticPayload {
            contract: WORKER_ERROR_CONTRACT,
            binary: "ait-agent-worker",
            status: "error",
            code: self.code,
            message: &self.message,
            exit_code: self.exit_code,
            details: &self.details,
            python_worker_execution_allowed: false,
        };
        JsonCodec::encode_serializable(
            &payload,
            JsonEncodeOptions::compact().with_trailing_newline(),
        )
        .unwrap_or_else(|_| {
            concat!(
                "{\"contract\":\"ait.agent.worker.error.v1\",",
                "\"binary\":\"ait-agent-worker\",\"status\":\"error\",",
                "\"code\":\"diagnostic_serialization_failed\",",
                "\"message\":\"Failed to serialize worker diagnostic.\",",
                "\"exit_code\":4,\"details\":{},",
                "\"python_worker_execution_allowed\":false}\n"
            )
            .to_string()
        })
    }
}

impl fmt::Display for WorkerDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for WorkerDiagnostic {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_json_is_structured_and_disallows_python_fallback() {
        let diagnostic = WorkerDiagnostic::new(
            "unsupported_transport_runtime",
            "Runner is unavailable.",
            EXIT_RUNTIME_UNAVAILABLE,
        )
        .with_detail("transport", "telegram");

        let rendered = diagnostic.render_json();

        assert!(rendered.ends_with('\n'));
        assert!(rendered.contains("\"contract\":\"ait.agent.worker.error.v1\""));
        assert!(rendered.contains("\"transport\":\"telegram\""));
        assert!(rendered.contains("\"python_worker_execution_allowed\":false"));
    }
}
