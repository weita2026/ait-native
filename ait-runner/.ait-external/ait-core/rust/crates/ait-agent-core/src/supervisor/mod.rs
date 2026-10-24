use serde::{Deserialize, Serialize};

use crate::runtime::AgentRuntimePlan;
use crate::transport::TransportKind;

mod lifecycle;
mod process;
mod public_payload;
mod termination_context;

pub(crate) use process::runtime_env_value;

pub use lifecycle::{
    acquire_worker_lifecycle_lock, acquire_worker_lifecycle_lock_json,
    plan_worker_supervisor_lifecycle, plan_worker_supervisor_lifecycle_json,
    release_worker_lifecycle_lock, release_worker_lifecycle_lock_json,
    AgentWorkerLifecycleLockAcquireInput, AgentWorkerLifecycleLockAcquireResult,
    AgentWorkerLifecycleLockReleaseInput, AgentWorkerLifecycleLockReleaseResult,
    AgentWorkerLifecycleOperation, AgentWorkerLifecyclePlan, AgentWorkerLifecyclePlanInput,
    AgentWorkerLifecycleSpec, AgentWorkerRuntimePaths,
};
pub use process::{
    inspect_worker_process_status, inspect_worker_process_status_json,
    inspect_worker_process_status_with_port, read_worker_log_tail, read_worker_log_tail_json,
    read_worker_log_tail_with_port, start_worker_process, start_worker_process_json,
    start_worker_process_with_port, stop_worker_process, stop_worker_process_json,
    stop_worker_process_with_port, AgentWorkerLogTail, AgentWorkerLogTailInput,
    AgentWorkerPidFileInspection, AgentWorkerProcessLogTailPort, AgentWorkerProcessPaths,
    AgentWorkerProcessPort, AgentWorkerProcessStartPort, AgentWorkerProcessStatus,
    AgentWorkerProcessStatusInput, AgentWorkerProcessStatusPort, AgentWorkerProcessStopPort,
    AgentWorkerRuntimeHealth, AgentWorkerStartInput, AgentWorkerStartResult, AgentWorkerStartSpec,
    AgentWorkerStopInput, AgentWorkerStopResult, NativeAgentWorkerProcessPort,
};
pub use public_payload::agent_supervisor_public_worker_payload_json;
pub use termination_context::{
    consume_worker_termination_context_json, AGENT_SUPERVISOR_TERMINATION_CONTEXT_CONTRACT,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLaunchPlan {
    pub transport: TransportKind,
    pub configured_workers: usize,
    pub shard_index: usize,
    pub event_loop_backend: String,
}

pub fn plan_worker_launches(plan: &AgentRuntimePlan) -> Vec<WorkerLaunchPlan> {
    let workers_per_shard = plan.capacity.workers_per_shard.max(1);
    let mut next_worker_index = 0usize;
    let mut launches = Vec::new();
    for count in &plan.transport_counts {
        if count.configured_workers == 0 {
            continue;
        }
        let shard_index = next_worker_index / workers_per_shard;
        next_worker_index += count.configured_workers;
        launches.push(WorkerLaunchPlan {
            transport: count.transport,
            configured_workers: count.configured_workers,
            shard_index,
            event_loop_backend: plan.capacity.backend.label().to_string(),
        });
    }
    launches
}

#[cfg(test)]
mod tests;
