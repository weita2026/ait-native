use ait_core::json_support::JsonValue;
use serde::{Deserialize, Serialize};

use crate::json_support::{decode_from_value, encode_to_value};
use crate::manifest::{
    count_manifest_workers, default_empty_manifest, list_manifest_workers, AgentWorkerCount,
};

use super::{
    AgentEventLoopBackend, AgentEventLoopConfig, AgentRuntimeCapacity,
    DEFAULT_WORKERS_PER_EPOLL_SHARD, DEFAULT_WORKERS_PER_POLL_SHARD,
};

const MIGRATION_STAGE: &str = "rust_agent_event_loop_reactor_plan";
const DRIVER_CONTRACT: &str = "ait_agent_core.event_loop.AgentEventLoopDriver.v1";

#[derive(Debug, Clone, Deserialize)]
pub struct AgentReactorPlanInput {
    #[serde(default = "default_empty_manifest")]
    pub worker_manifest: JsonValue,
    #[serde(default)]
    pub expected_concurrent_workers: Option<usize>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub workers_per_shard: Option<usize>,
}

impl Default for AgentReactorPlanInput {
    fn default() -> Self {
        Self {
            worker_manifest: default_empty_manifest(),
            expected_concurrent_workers: None,
            backend: None,
            workers_per_shard: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentReactorWorkerSlot {
    pub worker_key: String,
    pub worker_name: String,
    pub transport: String,
    pub worker_index: usize,
    pub shard_index: usize,
    pub token: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentReactorShardPlan {
    pub shard_index: usize,
    pub backend: String,
    pub worker_capacity: usize,
    pub worker_count: usize,
    pub token_range_start: u64,
    pub token_range_end: u64,
    pub workers: Vec<AgentReactorWorkerSlot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentReactorPlan {
    pub migration_stage: &'static str,
    pub driver_contract: &'static str,
    pub backend: String,
    pub total_configured_workers: usize,
    pub expected_concurrent_workers: usize,
    pub workers_per_shard: usize,
    pub shard_count: usize,
    pub high_concurrency: bool,
    pub requires_epoll_for_target_scale: bool,
    pub launch_allowed: bool,
    pub python_worker_execution_allowed: bool,
    pub transport_counts: Vec<AgentWorkerCount>,
    pub reactor_shards: Vec<AgentReactorShardPlan>,
    pub diagnostics: Vec<String>,
}

pub fn plan_agent_event_loop_reactor(
    input: AgentReactorPlanInput,
) -> Result<AgentReactorPlan, String> {
    let backend = match input.backend.as_deref() {
        Some(raw) => AgentEventLoopBackend::from_label(raw)
            .ok_or_else(|| format!("invalid ait-agent event-loop backend `{raw}`"))?,
        None => AgentEventLoopBackend::current_platform_default(),
    };
    let workers_per_shard = input
        .workers_per_shard
        .unwrap_or_else(|| default_workers_per_shard(backend))
        .max(1);
    let workers = list_manifest_workers(&input.worker_manifest);
    let transport_counts = count_manifest_workers(&input.worker_manifest);
    let total_configured_workers = workers.len();
    let expected_concurrent_workers = input
        .expected_concurrent_workers
        .unwrap_or(total_configured_workers)
        .max(total_configured_workers);
    let capacity = AgentRuntimeCapacity::from_config(AgentEventLoopConfig {
        backend,
        workers_per_shard,
        expected_workers: expected_concurrent_workers,
    });
    let worker_slots = workers
        .into_iter()
        .enumerate()
        .map(|(worker_index, worker)| AgentReactorWorkerSlot {
            worker_key: worker.key,
            worker_name: worker.name,
            transport: worker.transport.as_str().to_string(),
            worker_index,
            shard_index: worker_index / workers_per_shard,
            token: (worker_index as u64) + 1,
        })
        .collect::<Vec<_>>();
    let shard_count = capacity.shard_count.max(required_shards_for_workers(
        worker_slots.len(),
        workers_per_shard,
    ));
    let mut reactor_shards = (0..shard_count)
        .map(|shard_index| {
            let token_range_start = (shard_index * workers_per_shard) as u64 + 1;
            let token_range_end = ((shard_index + 1) * workers_per_shard) as u64;
            let workers = worker_slots
                .iter()
                .filter(|slot| slot.shard_index == shard_index)
                .cloned()
                .collect::<Vec<_>>();
            AgentReactorShardPlan {
                shard_index,
                backend: backend.label().to_string(),
                worker_capacity: workers_per_shard,
                worker_count: workers.len(),
                token_range_start,
                token_range_end,
                workers,
            }
        })
        .collect::<Vec<_>>();
    if reactor_shards.is_empty() && expected_concurrent_workers > 0 {
        reactor_shards.push(AgentReactorShardPlan {
            shard_index: 0,
            backend: backend.label().to_string(),
            worker_capacity: workers_per_shard,
            worker_count: 0,
            token_range_start: 1,
            token_range_end: workers_per_shard as u64,
            workers: Vec::new(),
        });
    }

    let mut diagnostics = Vec::new();
    if capacity.requires_epoll_for_target_scale {
        diagnostics.push(format!(
            "Expected {expected_concurrent_workers} concurrent workers requires linux_epoll; \
             backend {} is only for transitional low-concurrency operation.",
            backend.label()
        ));
    }
    Ok(AgentReactorPlan {
        migration_stage: MIGRATION_STAGE,
        driver_contract: DRIVER_CONTRACT,
        backend: backend.label().to_string(),
        total_configured_workers,
        expected_concurrent_workers,
        workers_per_shard,
        shard_count,
        high_concurrency: capacity.high_concurrency,
        requires_epoll_for_target_scale: capacity.requires_epoll_for_target_scale,
        launch_allowed: !capacity.requires_epoll_for_target_scale,
        python_worker_execution_allowed: false,
        transport_counts,
        reactor_shards,
        diagnostics,
    })
}

pub fn agent_event_loop_reactor_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentReactorPlanInput =
        decode_from_value(request, "invalid ait-agent reactor plan request")?;
    encode_to_value(
        &plan_agent_event_loop_reactor(input)?,
        "failed to serialize ait-agent reactor plan",
    )
}

fn default_workers_per_shard(backend: AgentEventLoopBackend) -> usize {
    match backend {
        AgentEventLoopBackend::LinuxEpoll => DEFAULT_WORKERS_PER_EPOLL_SHARD,
        AgentEventLoopBackend::PortablePoll => DEFAULT_WORKERS_PER_POLL_SHARD,
    }
}

fn required_shards_for_workers(worker_count: usize, workers_per_shard: usize) -> usize {
    if worker_count == 0 {
        0
    } else {
        worker_count.div_ceil(workers_per_shard.max(1))
    }
}

#[cfg(test)]
mod tests;
