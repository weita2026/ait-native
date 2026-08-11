use ait_core::json_support::JsonValue;
use serde::{Deserialize, Serialize};

use crate::event_loop::{AgentEventLoopConfig, AgentRuntimeCapacity};
use crate::manifest::{count_manifest_workers, default_empty_manifest, AgentWorkerCount};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRuntimePlanInput {
    pub worker_manifest: JsonValue,
    pub expected_concurrent_workers: Option<usize>,
}

impl Default for AgentRuntimePlanInput {
    fn default() -> Self {
        Self {
            worker_manifest: default_empty_manifest(),
            expected_concurrent_workers: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRuntimePlan {
    pub total_configured_workers: usize,
    pub expected_concurrent_workers: usize,
    pub transport_counts: Vec<AgentWorkerCount>,
    pub capacity: AgentRuntimeCapacity,
    pub python_worker_execution_allowed: bool,
    pub migration_stage: &'static str,
}

pub fn plan_agent_runtime(input: AgentRuntimePlanInput) -> AgentRuntimePlan {
    let transport_counts = count_manifest_workers(&input.worker_manifest);
    let total_configured_workers = transport_counts
        .iter()
        .map(|count| count.configured_workers)
        .sum::<usize>();
    let expected_concurrent_workers = input
        .expected_concurrent_workers
        .unwrap_or(total_configured_workers)
        .max(total_configured_workers);
    let config = AgentEventLoopConfig::for_expected_workers(expected_concurrent_workers);
    AgentRuntimePlan {
        total_configured_workers,
        expected_concurrent_workers,
        transport_counts,
        capacity: AgentRuntimeCapacity::from_config(config),
        python_worker_execution_allowed: false,
        migration_stage: "rust_agent_runtime_foundation",
    }
}

#[cfg(test)]
mod tests;
