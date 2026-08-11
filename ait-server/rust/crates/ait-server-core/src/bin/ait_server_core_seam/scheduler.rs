use super::json_helpers::{
    optional_array, optional_usize, print_json, required_array, required_object, required_text,
};
use super::*;

pub(super) fn scheduler_shape_async_job_command(
    job_type: &str,
    payload_json: &str,
) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let payload = payload_value
        .as_object()
        .ok_or_else(|| format!("{job_type} payload must be a JSON object."))?;
    let spec = scheduler_job_spec_from_async_job(job_type, payload)?;
    print_json(&scheduler_job_spec_json(&spec))
}

pub(super) fn scheduler_admit_async_jobs_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let payload = payload_value
        .as_object()
        .ok_or_else(|| "scheduler-admit-async-jobs payload must be a JSON object.".to_string())?;
    let policy = scheduler_policy_from_payload(payload.get("policy"))?;
    let queued_values = required_array(payload, "queued")?;
    let running_values = optional_array(payload, "running")?.unwrap_or(&[]);

    let queued_jobs: Vec<SchedulerQueuedJob> = queued_values
        .iter()
        .enumerate()
        .map(|(index, value)| scheduler_queued_job_from_value(value, index, &policy))
        .collect::<Result<Vec<_>, _>>()?;
    let running_jobs = running_values
        .iter()
        .enumerate()
        .map(|(index, value)| scheduler_running_job_from_value(value, index, &policy))
        .collect::<Result<Vec<_>, _>>()?;
    let decision = admit_next(&queued_jobs, &running_jobs, &policy);

    print_json(&json!({
        "decision": scheduler_admission_decision_json(&decision, &queued_jobs),
        "policy": scheduler_policy_json(&policy),
        "queued_job_count": queued_jobs.len(),
        "running_job_count": running_jobs.len(),
    }))
}

pub(super) fn scheduler_status_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let payload = payload_value
        .as_object()
        .ok_or_else(|| "scheduler-status payload must be a JSON object.".to_string())?;
    let policy = scheduler_policy_from_payload(payload.get("policy"))?;
    let queued_values = optional_array(payload, "queued")?.unwrap_or(&[]);
    let running_values = optional_array(payload, "running")?.unwrap_or(&[]);
    let queued_jobs: Vec<SchedulerQueuedJob> = queued_values
        .iter()
        .enumerate()
        .map(|(index, value)| scheduler_queued_job_from_value(value, index, &policy))
        .collect::<Result<Vec<_>, _>>()?;
    let running_jobs = running_values
        .iter()
        .enumerate()
        .map(|(index, value)| scheduler_running_job_from_value(value, index, &policy))
        .collect::<Result<Vec<_>, _>>()?;
    let decision = admit_next(&queued_jobs, &running_jobs, &policy);

    print_json(&json!({
        "status": scheduler_status_name(&queued_jobs, &running_jobs),
        "policy": scheduler_policy_json(&policy),
        "capacity": scheduler_capacity_json(&policy, &running_jobs),
        "thread_pool": {
            "state_source": "scheduler_snapshot_payload",
            "running_leases": running_jobs.len(),
            "worker_count": JsonValue::Null,
            "note": "seam status is a scheduler snapshot; live worker count is owned by the server process that hosts ScheduledExecutorPool"
        },
        "queued_job_count": queued_jobs.len(),
        "running_job_count": running_jobs.len(),
        "queued_jobs": queued_jobs
            .iter()
            .map(scheduler_queued_job_status_json)
            .collect::<Vec<_>>(),
        "running_jobs": running_jobs
            .iter()
            .map(scheduler_running_job_status_json)
            .collect::<Vec<_>>(),
        "next_admission": scheduler_admission_decision_json(&decision, &queued_jobs),
    }))
}

pub(super) fn scheduler_queued_job_from_value(
    value: &JsonValue,
    default_ordinal: usize,
    policy: &SchedulerPolicy,
) -> Result<SchedulerQueuedJob, String> {
    let row = value
        .as_object()
        .ok_or_else(|| format!("queued[{default_ordinal}] must be a JSON object."))?;
    let job_id =
        required_text(row, "job_id").map_err(|exc| format!("queued[{default_ordinal}]: {exc}"))?;
    let job_type = required_text(row, "job_type")
        .map_err(|exc| format!("queued[{default_ordinal}]: {exc}"))?;
    let payload = required_object(row, "payload")
        .map_err(|exc| format!("queued[{default_ordinal}]: {exc}"))?;
    let queued_ordinal = optional_usize(row, "queued_ordinal")
        .map_err(|exc| format!("queued[{default_ordinal}]: {exc}"))?
        .unwrap_or(default_ordinal);
    scheduler_queued_job_from_async_job_with_policy(
        job_id,
        queued_ordinal,
        &job_type,
        payload,
        policy,
    )
    .map_err(|exc| format!("queued[{default_ordinal}]: {exc}"))
}

pub(super) fn scheduler_running_job_from_value(
    value: &JsonValue,
    index: usize,
    policy: &SchedulerPolicy,
) -> Result<SchedulerRunningJob, String> {
    let row = value
        .as_object()
        .ok_or_else(|| format!("running[{index}] must be a JSON object."))?;
    let job_id = required_text(row, "job_id").map_err(|exc| format!("running[{index}]: {exc}"))?;
    let job_type =
        required_text(row, "job_type").map_err(|exc| format!("running[{index}]: {exc}"))?;
    let payload =
        required_object(row, "payload").map_err(|exc| format!("running[{index}]: {exc}"))?;
    scheduler_running_job_from_async_job_with_policy(job_id, &job_type, payload, policy)
        .map_err(|exc| format!("running[{index}]: {exc}"))
}

fn scheduler_status_name(
    queued_jobs: &[SchedulerQueuedJob],
    running_jobs: &[SchedulerRunningJob],
) -> &'static str {
    match (queued_jobs.is_empty(), running_jobs.is_empty()) {
        (true, true) => "idle",
        (false, true) => "queued",
        (true, false) => "running",
        (false, false) => "running_with_backlog",
    }
}

fn scheduler_capacity_json(
    policy: &SchedulerPolicy,
    running_jobs: &[SchedulerRunningJob],
) -> JsonValue {
    json!({
        "global_cpu_tokens": scheduler_token_pool_json(
            "global_cpu_tokens",
            policy.global_cpu_tokens,
            running_jobs,
        ),
        "ci_full_shared_cpu_tokens": scheduler_token_pool_json(
            "ci_full_shared_cpu_tokens",
            policy.ci_full_shared_cpu_tokens,
            running_jobs,
        ),
        "full_test_cpu_tokens": scheduler_token_pool_json(
            "full_test_cpu_tokens",
            policy.full_test_cpu_tokens,
            running_jobs,
        ),
        "interactive_reserved_tokens": scheduler_token_pool_json(
            "interactive_reserved_tokens",
            policy.interactive_reserved_tokens,
            running_jobs,
        ),
        "sync_cpu_tokens": scheduler_token_pool_json(
            "sync_cpu_tokens",
            policy.sync_cpu_tokens,
            running_jobs,
        ),
        "maintenance_cpu_tokens": scheduler_token_pool_json(
            "maintenance_cpu_tokens",
            policy.maintenance_cpu_tokens,
            running_jobs,
        ),
    })
}

fn scheduler_token_pool_json(
    pool_name: &str,
    capacity: usize,
    running_jobs: &[SchedulerRunningJob],
) -> JsonValue {
    let used = scheduler_token_pool_used(pool_name, running_jobs);
    json!({
        "capacity": capacity,
        "used": used,
        "available": capacity.saturating_sub(used),
        "over_capacity": used > capacity,
    })
}

fn scheduler_token_pool_used(pool_name: &str, running_jobs: &[SchedulerRunningJob]) -> usize {
    running_jobs
        .iter()
        .filter(|job| job.spec.token_pools.iter().any(|pool| pool == pool_name))
        .map(|job| job.spec.cpu_tokens)
        .sum()
}

fn scheduler_queued_job_status_json(job: &SchedulerQueuedJob) -> JsonValue {
    json!({
        "job_id": &job.job_id,
        "queued_ordinal": job.queued_ordinal,
        "scheduler_job": scheduler_job_spec_json(&job.spec),
    })
}

fn scheduler_running_job_status_json(job: &SchedulerRunningJob) -> JsonValue {
    json!({
        "job_id": &job.job_id,
        "scheduler_job": scheduler_job_spec_json(&job.spec),
    })
}

pub(super) fn scheduler_policy_from_payload(
    value: Option<&JsonValue>,
) -> Result<SchedulerPolicy, String> {
    let Some(value) = value else {
        return Ok(SchedulerPolicy::default());
    };
    if value.is_null() {
        return Ok(SchedulerPolicy::default());
    }

    let policy = value
        .as_object()
        .ok_or_else(|| "`policy` must be a JSON object.".to_string())?;
    let posture = match policy.get("posture") {
        None | Some(JsonValue::Null) => SchedulerDeploymentPosture::from_environment(),
        Some(JsonValue::String(value)) => SchedulerDeploymentPosture::parse(value)
            .ok_or_else(|| format!("Field `policy.posture` is not supported: {value}."))?,
        Some(_) => return Err("Field `policy.posture` must be a string.".to_string()),
    };
    let host_cpu_cores = optional_usize(policy, "host_cpu_cores")
        .map_err(|exc| exc.replace("Field `host_cpu_cores`", "Field `policy.host_cpu_cores`"))?;
    match host_cpu_cores {
        Some(0) => Err("Field `policy.host_cpu_cores` must be a positive integer.".to_string()),
        Some(value) => Ok(SchedulerPolicy::for_host_cpu_cores(value, posture)),
        None => Ok(SchedulerPolicy::for_detected_host(posture)),
    }
}

fn scheduler_admission_decision_json(
    decision: &SchedulerAdmissionDecision,
    queued_jobs: &[SchedulerQueuedJob],
) -> JsonValue {
    match decision {
        SchedulerAdmissionDecision::Admit { job_id } => {
            let job = queued_jobs
                .iter()
                .find(|queued_job| queued_job.job_id == job_id.as_str())
                .map(|queued_job| scheduler_job_spec_json(&queued_job.spec));
            json!({
                "kind": "admit",
                "job_id": job_id,
                "job": job,
            })
        }
        SchedulerAdmissionDecision::Attach {
            job_id,
            active_job_id,
            singleflight_key,
        } => json!({
            "kind": "attach",
            "job_id": job_id,
            "active_job_id": active_job_id,
            "singleflight_key": singleflight_key,
        }),
        SchedulerAdmissionDecision::Wait { reason } => json!({
            "kind": "wait",
            "reason": reason,
        }),
    }
}

pub(super) fn scheduler_policy_json(policy: &SchedulerPolicy) -> JsonValue {
    json!({
        "host_cpu_cores": policy.host_cpu_cores,
        "reserved_local_cpu_cores": policy.reserved_local_cpu_cores,
        "global_cpu_tokens": policy.global_cpu_tokens,
        "ci_full_shared_cpu_tokens": policy.ci_full_shared_cpu_tokens,
        "full_test_cpu_tokens": policy.full_test_cpu_tokens,
        "full_test_job_cpu_tokens": policy.full_test_job_cpu_tokens,
        "interactive_reserved_tokens": policy.interactive_reserved_tokens,
        "sync_cpu_tokens": policy.sync_cpu_tokens,
        "maintenance_cpu_tokens": policy.maintenance_cpu_tokens,
    })
}

pub(super) fn scheduler_job_spec_json(spec: &SchedulerJobSpec) -> JsonValue {
    json!({
        "job_kind": &spec.job_kind,
        "job_class": format!("{:?}", &spec.job_class),
        "read_keys": &spec.read_keys,
        "write_keys": &spec.write_keys,
        "singleflight_key": &spec.singleflight_key,
        "cpu_tokens": spec.cpu_tokens,
        "token_pools": &spec.token_pools,
        "priority": spec.priority,
        "queue_key": &spec.queue_key,
    })
}
