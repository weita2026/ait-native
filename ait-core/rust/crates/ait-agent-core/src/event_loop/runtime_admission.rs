use std::collections::BTreeSet;

use ait_core::json_support::JsonValue;
use serde::{Deserialize, Serialize};

use crate::json_support::{decode_from_value, encode_to_value};
use crate::manifest::default_empty_manifest;

use super::{plan_agent_event_loop_reactor, AgentReactorPlanInput};

const MIGRATION_STAGE: &str = "rust_agent_high_concurrency_runtime_admission";
const ADMISSION_CONTRACT: &str = "ait_agent_core.event_loop.AgentRuntimeAdmission.v1";
const LEASE_CONTRACT: &str = "ait_agent_core.event_loop.AgentWorkerLease.v1";

#[derive(Debug, Clone, Deserialize)]
pub struct AgentRuntimeAdmissionInput {
    #[serde(default = "default_empty_manifest")]
    pub worker_manifest: JsonValue,
    #[serde(default)]
    pub expected_concurrent_workers: Option<usize>,
    #[serde(default)]
    pub backend: Option<String>,
    #[serde(default)]
    pub workers_per_shard: Option<usize>,
    #[serde(default = "default_transport_runtime")]
    pub transport_runtime: String,
    #[serde(default)]
    pub allow_python_fallback: bool,
    #[serde(default)]
    pub requested_worker_keys: Vec<String>,
}

impl Default for AgentRuntimeAdmissionInput {
    fn default() -> Self {
        Self {
            worker_manifest: default_empty_manifest(),
            expected_concurrent_workers: None,
            backend: None,
            workers_per_shard: None,
            transport_runtime: default_transport_runtime(),
            allow_python_fallback: false,
            requested_worker_keys: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentRuntimeWorkerLease {
    pub lease_contract: &'static str,
    pub lease_id: String,
    pub worker_key: String,
    pub worker_name: String,
    pub transport: String,
    pub backend: String,
    pub shard_index: usize,
    pub worker_index: usize,
    pub token: u64,
    pub rust_event_loop_required: bool,
    pub python_fallback_allowed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentRuntimeShardAdmission {
    pub shard_index: usize,
    pub backend: String,
    pub worker_capacity: usize,
    pub worker_count: usize,
    pub lease_count: usize,
    pub inflight_limit: usize,
    pub token_range_start: u64,
    pub token_range_end: u64,
    pub leases: Vec<AgentRuntimeWorkerLease>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentRuntimeAdmissionPlan {
    pub migration_stage: &'static str,
    pub admission_contract: &'static str,
    pub backend: String,
    pub transport_runtime: String,
    pub admission_state: String,
    pub launch_allowed: bool,
    pub rust_event_loop_required: bool,
    pub python_worker_execution_allowed: bool,
    pub python_fallback_requested: bool,
    pub expected_concurrent_workers: usize,
    pub total_configured_workers: usize,
    pub selected_worker_count: usize,
    pub workers_per_shard: usize,
    pub shard_count: usize,
    pub high_concurrency: bool,
    pub requires_epoll_for_target_scale: bool,
    pub worker_leases: Vec<AgentRuntimeWorkerLease>,
    pub shard_admissions: Vec<AgentRuntimeShardAdmission>,
    pub rejection_reasons: Vec<String>,
    pub diagnostics: Vec<String>,
}

pub fn plan_agent_runtime_admission(
    input: AgentRuntimeAdmissionInput,
) -> Result<AgentRuntimeAdmissionPlan, String> {
    let transport_runtime = normalize_transport_runtime(&input.transport_runtime);
    let reactor = plan_agent_event_loop_reactor(AgentReactorPlanInput {
        worker_manifest: input.worker_manifest,
        expected_concurrent_workers: input.expected_concurrent_workers,
        backend: input.backend,
        workers_per_shard: input.workers_per_shard,
    })?;
    let requested_keys = input
        .requested_worker_keys
        .iter()
        .map(|key| key.trim().to_string())
        .filter(|key| !key.is_empty())
        .collect::<BTreeSet<_>>();
    let all_worker_keys = reactor
        .reactor_shards
        .iter()
        .flat_map(|shard| shard.workers.iter().map(|worker| worker.worker_key.clone()))
        .collect::<BTreeSet<_>>();
    let mut rejection_reasons = Vec::new();
    if reactor.total_configured_workers == 0 {
        rejection_reasons.push("no ait-agent workers are configured".to_string());
    }
    if input.allow_python_fallback {
        rejection_reasons.push(
            "python fallback execution is disabled for ait-agent runtime admission".to_string(),
        );
    }
    if transport_runtime != "rust" {
        rejection_reasons.push(format!(
            "transport_runtime `{transport_runtime}` is not allowed; ait-agent admission requires `rust`"
        ));
    }
    for requested in requested_keys.difference(&all_worker_keys) {
        rejection_reasons.push(format!(
            "requested worker `{requested}` is not present in the normalized worker manifest"
        ));
    }
    if reactor.requires_epoll_for_target_scale {
        rejection_reasons.push(format!(
            "expected {} concurrent workers requires linux_epoll; backend `{}` is not admitted",
            reactor.expected_concurrent_workers, reactor.backend
        ));
    }

    let worker_leases = reactor
        .reactor_shards
        .iter()
        .flat_map(|shard| {
            shard
                .workers
                .iter()
                .filter(|worker| {
                    requested_keys.is_empty() || requested_keys.contains(&worker.worker_key)
                })
                .map(|worker| AgentRuntimeWorkerLease {
                    lease_contract: LEASE_CONTRACT,
                    lease_id: format!(
                        "agent-worker:{}:shard-{}:token-{}",
                        reactor.backend, worker.shard_index, worker.token
                    ),
                    worker_key: worker.worker_key.clone(),
                    worker_name: worker.worker_name.clone(),
                    transport: worker.transport.clone(),
                    backend: reactor.backend.clone(),
                    shard_index: worker.shard_index,
                    worker_index: worker.worker_index,
                    token: worker.token,
                    rust_event_loop_required: true,
                    python_fallback_allowed: false,
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    if !requested_keys.is_empty() && worker_leases.is_empty() && rejection_reasons.is_empty() {
        rejection_reasons.push("requested worker selection produced no runtime leases".to_string());
    }

    let shard_admissions = reactor
        .reactor_shards
        .iter()
        .map(|shard| {
            let leases = worker_leases
                .iter()
                .filter(|lease| lease.shard_index == shard.shard_index)
                .cloned()
                .collect::<Vec<_>>();
            AgentRuntimeShardAdmission {
                shard_index: shard.shard_index,
                backend: shard.backend.clone(),
                worker_capacity: shard.worker_capacity,
                worker_count: shard.worker_count,
                lease_count: leases.len(),
                inflight_limit: shard.worker_capacity,
                token_range_start: shard.token_range_start,
                token_range_end: shard.token_range_end,
                leases,
            }
        })
        .collect::<Vec<_>>();
    let launch_allowed = reactor.launch_allowed && rejection_reasons.is_empty();
    let admission_state = if launch_allowed {
        "admitted"
    } else {
        "rejected"
    }
    .to_string();
    let selected_worker_count = worker_leases.len();

    Ok(AgentRuntimeAdmissionPlan {
        migration_stage: MIGRATION_STAGE,
        admission_contract: ADMISSION_CONTRACT,
        backend: reactor.backend,
        transport_runtime,
        admission_state,
        launch_allowed,
        rust_event_loop_required: true,
        python_worker_execution_allowed: false,
        python_fallback_requested: input.allow_python_fallback,
        expected_concurrent_workers: reactor.expected_concurrent_workers,
        total_configured_workers: reactor.total_configured_workers,
        selected_worker_count,
        workers_per_shard: reactor.workers_per_shard,
        shard_count: reactor.shard_count,
        high_concurrency: reactor.high_concurrency,
        requires_epoll_for_target_scale: reactor.requires_epoll_for_target_scale,
        worker_leases,
        shard_admissions,
        rejection_reasons,
        diagnostics: reactor.diagnostics,
    })
}

pub fn agent_runtime_admission_plan_json(request: &JsonValue) -> Result<JsonValue, String> {
    let input: AgentRuntimeAdmissionInput =
        decode_from_value(request, "invalid ait-agent runtime admission request")?;
    encode_to_value(
        &plan_agent_runtime_admission(input)?,
        "failed to serialize ait-agent runtime admission plan",
    )
}

fn default_transport_runtime() -> String {
    "rust".to_string()
}

fn normalize_transport_runtime(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

#[cfg(test)]
mod tests;
