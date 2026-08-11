use std::collections::BTreeSet;

use ait_core::json_support::JsonValue;
use serde::{Deserialize, Serialize};

use crate::json_support::{decode_from_value, encode_to_value};
use crate::manifest::{default_empty_manifest, list_manifest_workers};
use crate::runtime::{plan_agent_runtime, AgentRuntimePlan, AgentRuntimePlanInput};
use crate::transport::TransportKind;

pub const DEFAULT_RUST_WORKER_BINARY: &str = "ait-agent-worker";
const MIGRATION_STAGE: &str = "rust_agent_cli_launch_contract";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCliPlanInput {
    #[serde(default = "default_empty_manifest")]
    pub worker_manifest: JsonValue,
    #[serde(default)]
    pub expected_concurrent_workers: Option<usize>,
    #[serde(default)]
    pub rust_worker_binary: Option<String>,
    #[serde(default)]
    pub available_rust_transports: Vec<TransportKind>,
}

impl Default for AgentCliPlanInput {
    fn default() -> Self {
        Self {
            worker_manifest: default_empty_manifest(),
            expected_concurrent_workers: None,
            rust_worker_binary: None,
            available_rust_transports: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkerLaunchState {
    Ready,
    MissingRustTransportRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWorkerCommandPlan {
    pub worker_key: String,
    pub worker_name: String,
    pub transport: TransportKind,
    pub argv: Vec<String>,
    pub shard_index: usize,
    pub event_loop_backend: String,
    pub launch_state: AgentWorkerLaunchState,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCliLaunchPlan {
    pub runtime: AgentRuntimePlan,
    pub workers: Vec<AgentWorkerCommandPlan>,
    pub launch_allowed: bool,
    pub blocked_worker_count: usize,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: &'static str,
}

pub fn plan_agent_cli_launch(input: AgentCliPlanInput) -> AgentCliLaunchPlan {
    let runtime = plan_agent_runtime(AgentRuntimePlanInput {
        worker_manifest: input.worker_manifest.clone(),
        expected_concurrent_workers: input.expected_concurrent_workers,
    });
    let binary = normalize_worker_binary(input.rust_worker_binary.as_deref());
    let available_transports = input
        .available_rust_transports
        .into_iter()
        .collect::<BTreeSet<_>>();
    let workers_per_shard = runtime.capacity.workers_per_shard.max(1);
    let event_loop_backend = runtime.capacity.backend.label().to_string();
    let workers = list_manifest_workers(&input.worker_manifest)
        .into_iter()
        .enumerate()
        .map(|(worker_index, worker)| {
            let shard_index = worker_index / workers_per_shard;
            let runtime_available = available_transports.contains(&worker.transport);
            let launch_state = if runtime_available {
                AgentWorkerLaunchState::Ready
            } else {
                AgentWorkerLaunchState::MissingRustTransportRuntime
            };
            let diagnostic = if runtime_available {
                None
            } else {
                Some(format!(
                    "Rust {} worker runtime is not available; refusing Python fallback.",
                    worker.transport
                ))
            };
            AgentWorkerCommandPlan {
                worker_key: worker.key,
                worker_name: worker.name.clone(),
                transport: worker.transport,
                argv: vec![
                    binary.clone(),
                    "run".to_string(),
                    "--transport".to_string(),
                    worker.transport.as_str().to_string(),
                    "--worker".to_string(),
                    worker.name,
                    "--event-loop-backend".to_string(),
                    event_loop_backend.clone(),
                    "--shard".to_string(),
                    shard_index.to_string(),
                ],
                shard_index,
                event_loop_backend: event_loop_backend.clone(),
                launch_state,
                diagnostic,
            }
        })
        .collect::<Vec<_>>();
    let blocked_worker_count = workers
        .iter()
        .filter(|worker| worker.launch_state != AgentWorkerLaunchState::Ready)
        .count();
    AgentCliLaunchPlan {
        runtime,
        launch_allowed: blocked_worker_count == 0,
        blocked_worker_count,
        workers,
        python_worker_execution_allowed: false,
        migration_stage: MIGRATION_STAGE,
    }
}

pub fn plan_agent_cli_launch_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentCliPlanInput =
        decode_from_value(request, "invalid ait-agent CLI launch request")?;
    encode_to_value(
        &plan_agent_cli_launch(input),
        "failed to serialize ait-agent CLI launch plan",
    )
}

fn normalize_worker_binary(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_RUST_WORKER_BINARY)
        .to_string()
}

#[cfg(test)]
mod tests;
