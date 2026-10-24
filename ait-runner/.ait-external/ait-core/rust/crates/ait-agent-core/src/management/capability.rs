use std::collections::BTreeSet;
use std::process::Command;
use std::str::FromStr;

use ait_core::json_support::JsonValue;

use crate::json_support::parse_value;
use crate::transport::TransportKind;

pub const AGENT_WORKER_CAPABILITY_CONTRACT: &str = "ait.agent.worker.capabilities.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentWorkerCapabilityReport {
    pub supported_transports: Vec<TransportKind>,
    pub python_worker_execution_allowed: bool,
}

pub trait AgentWorkerCapabilityProbe {
    fn probe(&self, worker_binary: &str) -> Result<AgentWorkerCapabilityReport, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NativeAgentWorkerCapabilityProbe;

impl AgentWorkerCapabilityProbe for NativeAgentWorkerCapabilityProbe {
    fn probe(&self, worker_binary: &str) -> Result<AgentWorkerCapabilityReport, String> {
        let binary = worker_binary.trim();
        if binary.is_empty() {
            return Err("Rust ait-agent worker binary must not be empty.".to_string());
        }
        let output = Command::new(binary)
            .args(["capabilities", "--json"])
            .output()
            .map_err(|err| {
                format!(
                    "Failed to execute Rust worker capability probe `{binary} capabilities --json`: {err}"
                )
            })?;
        if !output.status.success() {
            let diagnostic = compact_diagnostic(&output.stderr);
            return Err(if diagnostic.is_empty() {
                format!(
                    "Rust worker capability probe `{binary} capabilities --json` exited with status {}.",
                    output.status
                )
            } else {
                format!(
                    "Rust worker capability probe `{binary} capabilities --json` exited with status {}: {diagnostic}",
                    output.status
                )
            });
        }
        let stdout = String::from_utf8(output.stdout).map_err(|err| {
            format!("Rust worker capability probe returned non-UTF-8 output: {err}")
        })?;
        parse_capability_report(&stdout)
    }
}

pub fn parse_capability_report(payload: &str) -> Result<AgentWorkerCapabilityReport, String> {
    let value = parse_value(payload, "Invalid Rust worker capability JSON")?;
    let contract = value
        .get("contract")
        .and_then(JsonValue::as_str)
        .unwrap_or_default();
    if contract != AGENT_WORKER_CAPABILITY_CONTRACT {
        return Err(format!(
            "Rust worker capability contract mismatch: expected `{AGENT_WORKER_CAPABILITY_CONTRACT}`, got `{contract}`."
        ));
    }
    let python_worker_execution_allowed = value
        .get("python_worker_execution_allowed")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true);
    if python_worker_execution_allowed {
        return Err(
            "Rust worker capability report did not prohibit Python worker execution.".to_string(),
        );
    }
    let mut supported = BTreeSet::new();
    for transport in value
        .get("supported_transports")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let text = transport.as_str().ok_or_else(|| {
            "Rust worker capability report contains a non-string transport.".to_string()
        })?;
        supported.insert(TransportKind::from_str(text)?);
    }
    Ok(AgentWorkerCapabilityReport {
        supported_transports: supported.into_iter().collect(),
        python_worker_execution_allowed,
    })
}

fn compact_diagnostic(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(1000)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fail_closed_capability_report() {
        let report = parse_capability_report(
            r#"{
                "contract": "ait.agent.worker.capabilities.v1",
                "supported_transports": ["slack", "telegram", "slack"],
                "python_worker_execution_allowed": false
            }"#,
        )
        .unwrap();

        assert_eq!(
            report.supported_transports,
            vec![TransportKind::Telegram, TransportKind::Slack]
        );
        assert!(!report.python_worker_execution_allowed);
    }

    #[test]
    fn rejects_capability_contracts_that_allow_python() {
        let error = parse_capability_report(
            r#"{
                "contract": "ait.agent.worker.capabilities.v1",
                "supported_transports": [],
                "python_worker_execution_allowed": true
            }"#,
        )
        .unwrap_err();

        assert!(error.contains("did not prohibit Python"));
    }
}
