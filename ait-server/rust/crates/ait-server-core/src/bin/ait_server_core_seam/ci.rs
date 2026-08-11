use super::json_helpers::{optional_array, print_json};
use super::scheduler::{
    scheduler_job_spec_json, scheduler_policy_from_payload, scheduler_policy_json,
    scheduler_running_job_from_value,
};
use super::*;

pub(super) fn patchset_ci_schedule_admission_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let payload = payload_value.as_object().ok_or_else(|| {
        "patchset-ci-schedule-admission payload must be a JSON object.".to_string()
    })?;
    let manifests = payload
        .get("manifests")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "`manifests` must be a JSON array.".to_string())?;
    let policy = scheduler_policy_from_payload(payload.get("policy"))?;
    let dispatch = plan_patchset_ci_dispatch_from_manifest_values(manifests, payload)?;
    let running_values = optional_array(payload, "running")?.unwrap_or(&[]);
    let running_jobs = running_values
        .iter()
        .enumerate()
        .map(|(index, value)| scheduler_running_job_from_value(value, index, &policy))
        .collect::<Result<Vec<_>, _>>()?;
    let queued_scheduler_jobs = dispatch
        .queued_jobs
        .iter()
        .map(|job| {
            scheduler_queued_job_from_async_job_with_policy(
                job.job_id.clone(),
                job.queued_ordinal,
                &job.job.job_type,
                &job.payload,
                &policy,
            )
            .map_err(|exc| format!("{}: {exc}", job.job_id))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let decision = if queued_scheduler_jobs.is_empty() && !dispatch.blocked_jobs.is_empty() {
        SchedulerAdmissionDecision::Wait {
            reason: dispatch
                .blocked_jobs
                .iter()
                .map(|job| format!("{} {}", job.job_id, job.reason))
                .collect::<Vec<_>>()
                .join("; "),
        }
    } else {
        admit_next(&queued_scheduler_jobs, &running_jobs, &policy)
    };
    let queued_jobs = dispatch
        .queued_jobs
        .iter()
        .map(|job| patchset_ci_dispatch_job_json(job, &queued_scheduler_jobs))
        .collect::<Vec<_>>();
    let blocked_jobs = dispatch
        .blocked_jobs
        .iter()
        .map(|job| {
            json!({
                "job_id": &job.job_id,
                "job": patchset_ci_job_plan_json(&job.job),
                "payload": &job.payload,
                "reason": &job.reason,
            })
        })
        .collect::<Vec<_>>();

    print_json(&json!({
        "plan": patchset_ci_plan_json(&dispatch.plan),
        "scope": &dispatch.scope,
        "decision": patchset_ci_schedule_decision_json(
            &decision,
            &dispatch.queued_jobs,
            &queued_scheduler_jobs,
        ),
        "queued_jobs": queued_jobs,
        "blocked_jobs": blocked_jobs,
        "policy": scheduler_policy_json(&policy),
        "running_job_count": running_jobs.len(),
    }))
}

fn patchset_ci_schedule_decision_json(
    decision: &SchedulerAdmissionDecision,
    dispatch_jobs: &[PatchsetCiDispatchJob],
    queued_scheduler_jobs: &[SchedulerQueuedJob],
) -> JsonValue {
    match decision {
        SchedulerAdmissionDecision::Admit { job_id } => {
            let dispatch_job = dispatch_jobs
                .iter()
                .find(|job| job.job_id == job_id.as_str());
            let scheduler_job = queued_scheduler_jobs
                .iter()
                .find(|job| job.job_id == job_id.as_str());
            json!({
                "kind": "admit",
                "job_id": job_id,
                "job": dispatch_job.map(|job| patchset_ci_job_plan_json(&job.job)),
                "payload": dispatch_job.map(|job| &job.payload),
                "scheduler_job": scheduler_job.map(|job| scheduler_job_spec_json(&job.spec)),
                "admitted_cpu_tokens": scheduler_job.map(|job| job.spec.cpu_tokens),
                "runner_parallelism": scheduler_job.map(|job| job.spec.cpu_tokens),
            })
        }
        SchedulerAdmissionDecision::Attach {
            job_id,
            active_job_id,
            singleflight_key,
        } => {
            let dispatch_job = dispatch_jobs
                .iter()
                .find(|job| job.job_id == job_id.as_str());
            json!({
                "kind": "attach",
                "job_id": job_id,
                "active_job_id": active_job_id,
                "singleflight_key": singleflight_key,
                "job": dispatch_job.map(|job| patchset_ci_job_plan_json(&job.job)),
                "payload": dispatch_job.map(|job| &job.payload),
            })
        }
        SchedulerAdmissionDecision::Wait { reason } => json!({
            "kind": "wait",
            "reason": reason,
        }),
    }
}

fn patchset_ci_dispatch_job_json(
    dispatch_job: &PatchsetCiDispatchJob,
    queued_scheduler_jobs: &[SchedulerQueuedJob],
) -> JsonValue {
    let scheduler_job = queued_scheduler_jobs
        .iter()
        .find(|job| job.job_id == dispatch_job.job_id.as_str());
    json!({
        "job_id": &dispatch_job.job_id,
        "queued_ordinal": dispatch_job.queued_ordinal,
        "job": patchset_ci_job_plan_json(&dispatch_job.job),
        "payload": &dispatch_job.payload,
        "scheduler_job": scheduler_job.map(|job| scheduler_job_spec_json(&job.spec)),
    })
}

fn patchset_ci_plan_json(plan: &PatchsetCiPlan) -> JsonValue {
    json!({
        "selected_suite_ids": &plan.selected_suite_ids,
        "blocking_suite_ids": &plan.blocking_suite_ids,
        "informational_suite_ids": &plan.informational_suite_ids,
        "ready_critical_suite_ids": &plan.ready_critical_suite_ids,
        "background_suite_ids": &plan.background_suite_ids,
        "ready_aggregation": {
            "stage": &plan.ready_aggregation.stage,
            "suite_ids": &plan.ready_aggregation.suite_ids,
            "updates_tests_status": plan.ready_aggregation.updates_tests_status,
        },
        "informational_aggregation": plan.informational_aggregation.as_ref().map(|aggregation| json!({
            "stage": &aggregation.stage,
            "suite_ids": &aggregation.suite_ids,
            "updates_tests_status": aggregation.updates_tests_status,
        })),
    })
}

fn patchset_ci_job_plan_json(job: &PatchsetCiJobPlan) -> JsonValue {
    json!({
        "job_type": &job.job_type,
        "suite_id": &job.suite_id,
        "suite_ids": &job.suite_ids,
        "stage": &job.stage,
        "workflow_ready_foreground": job.workflow_ready_foreground,
        "updates_tests_status": job.updates_tests_status,
    })
}

pub(super) fn patchset_ci_workflow_ready_evidence_command(
    payload_json: &str,
) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let payload = payload_value
        .as_object()
        .ok_or_else(|| "payload_json must be a JSON object.".to_string())?;
    let manifests = payload
        .get("manifests")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "`manifests` must be a JSON array.".to_string())?;
    let suite_evidence = payload
        .get("suite_evidence")
        .ok_or_else(|| "`suite_evidence` is required.".to_string())?;
    print_json(&workflow_ready_server_evidence_from_manifest_values(
        manifests,
        suite_evidence,
    )?)
}

pub(super) fn patchset_ci_run_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&patchset_ci_run_json(&payload_value)?)
}

pub(super) fn patchset_ci_host_command(operation: &str, payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    let value = match operation {
        "contract-available" => patchset_ci_contract_available_json(&payload_value)?,
        "suite-catalog" => patchset_ci_suite_catalog_json(&payload_value)?,
        "completion" => patchset_ci_completion_json(&payload_value)?,
        "active-state" => patchset_ci_active_state_json(&payload_value)?,
        "status-summary" => patchset_ci_status_summary_json(&payload_value)?,
        other => {
            return Err(format!(
                "Unsupported patchset-ci-host operation: {other}. Expected one of: contract-available, suite-catalog, completion, active-state, status-summary."
            ))
        }
    };
    print_json(&value)
}

pub(super) fn repo_ci_run_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&repo_ci_run_json(&payload_value)?)
}

pub(super) fn ci_main_seed_prewarm_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&ci_main_seed_prewarm_json(&payload_value)?)
}

pub(super) fn ci_command_bundle_run_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&ci_command_bundle_run_json(&payload_value)?)
}

pub(super) fn ci_test_shard_plan_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&ci_test_shard_plan_json(&payload_value)?)
}

pub(super) fn ci_test_shard_prepare_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&ci_test_shard_prepare_json(&payload_value)?)
}

pub(super) fn ci_test_shard_run_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&ci_test_shard_run_json(&payload_value)?)
}

pub(super) fn ci_test_shard_cleanup_command(payload_json: &str) -> Result<(), String> {
    let payload_value: JsonValue = serde_json::from_str(payload_json)
        .map_err(|exc| format!("payload_json must be valid JSON: {exc}"))?;
    print_json(&ci_test_shard_cleanup_json(&payload_value)?)
}
