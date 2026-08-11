use super::rows::{job_id_i64, row_text};
use super::*;

const SCHEDULER_CI_JOB_TYPES: [&str; 3] = ["patchset.ci", "patchset.ci.aggregate", "repo.ci"];
const SCHEDULER_JOB_TYPES: [&str; 5] = [
    "patchset.ci",
    "patchset.ci.aggregate",
    "repo.ci",
    "main-seed.refresh",
    "content.gc",
];

pub(super) fn scheduler_queued_jobs(
    jobs: &[JsonMap<String, JsonValue>],
    policy: &SchedulerPolicy,
) -> Result<Vec<SchedulerQueuedJob>, String> {
    jobs.iter()
        .enumerate()
        .filter(|(_, job)| {
            row_text(job, "job_type")
                .map(|job_type| is_scheduler_job(&job_type))
                .unwrap_or(false)
        })
        .map(|(index, job)| {
            let job_id = row_text(job, "job_id").unwrap_or_else(|| job_id_i64(job).to_string());
            let job_type = row_text(job, "job_type").unwrap_or_default();
            let payload = job
                .get("payload")
                .and_then(JsonValue::as_object)
                .ok_or_else(|| format!("job {job_id} payload must be a JSON object"))?;
            scheduler_queued_job_from_async_job_with_policy(
                job_id, index, &job_type, payload, policy,
            )
        })
        .collect()
}

pub(super) fn scheduler_running_jobs(
    jobs: &[JsonMap<String, JsonValue>],
    policy: &SchedulerPolicy,
) -> Result<Vec<SchedulerRunningJob>, String> {
    jobs.iter()
        .filter(|job| {
            row_text(job, "job_type")
                .map(|job_type| is_scheduler_job(&job_type))
                .unwrap_or(false)
        })
        .map(|job| {
            let job_id = row_text(job, "job_id").unwrap_or_else(|| job_id_i64(job).to_string());
            let job_type = row_text(job, "job_type").unwrap_or_default();
            let payload = job
                .get("payload")
                .and_then(JsonValue::as_object)
                .ok_or_else(|| format!("job {job_id} payload must be a JSON object"))?;
            scheduler_running_job_from_async_job_with_policy(job_id, &job_type, payload, policy)
        })
        .collect()
}

pub(super) fn is_scheduler_job(job_type: &str) -> bool {
    SCHEDULER_JOB_TYPES.contains(&job_type)
}

pub(super) fn is_scheduler_ci_job(job_type: &str) -> bool {
    SCHEDULER_CI_JOB_TYPES.contains(&job_type)
}

pub(super) fn scheduler_job_json(
    spec: &crate::foundation::scheduler::SchedulerJobSpec,
) -> JsonValue {
    json!({
        "job_kind": spec.job_kind,
        "job_class": format!("{:?}", spec.job_class),
        "read_keys": spec.read_keys,
        "write_keys": spec.write_keys,
        "singleflight_key": spec.singleflight_key,
        "cpu_tokens": spec.cpu_tokens,
        "token_pools": spec.token_pools,
        "priority": spec.priority,
        "queue_key": spec.queue_key,
    })
}

pub(super) fn scheduler_policy_json(policy: &SchedulerPolicy) -> JsonValue {
    json!({
        "host_cpu_cores": policy.host_cpu_cores,
        "reserved_local_cpu_cores": policy.reserved_local_cpu_cores,
        "global_cpu_tokens": policy.global_cpu_tokens,
        "ci_full_shared_cpu_tokens": policy.ci_full_shared_cpu_tokens,
        "full_test_cpu_tokens": policy.full_test_cpu_tokens,
        "full_test_job_cpu_tokens": policy.full_test_job_cpu_tokens,
    })
}
