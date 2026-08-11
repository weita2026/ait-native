use super::helpers::*;
use super::live_turn_pressure::live_turn_pressure_summary_from_normalized;
use super::operator_metrics::build_operator_metrics;
use super::*;

pub fn operator_readiness_read_model(input: &OperatorMetricsInput) -> Result<JsonValue, String> {
    let metrics = build_operator_metrics(input)?;
    let readiness = build_operator_readiness(input, &metrics);
    Ok(annotate_operator_read_payload(input, readiness))
}

fn build_operator_readiness(input: &OperatorMetricsInput, metrics: &JsonValue) -> JsonValue {
    let summary = object_value(metrics, "summary");
    let storage = object_value(metrics, "storage_metrics");
    let jobs = object_value(metrics, "job_outcome_metrics");
    let workers = object_value(metrics, "worker_metrics");
    let live_turns = object_value(metrics, "live_turn_metrics");
    let live_turn_summary = object_field(&live_turns, "summary");
    let shared_runtime_policy = input.shared_runtime_policy.first().cloned();
    let rust_server_core_seam = input.rust_server_core_seam.first().cloned();
    let postgres_schema = input.postgres_schema.first().cloned();
    let mut checks = vec![readiness_check(
        "server_health",
        "pass",
        "Server process answered the readiness request.",
        Some(format!("db_backend={}", input.db_backend)),
        "none",
    )];

    match shared_runtime_policy
        .as_ref()
        .and_then(|row| object_bool(row, "ok"))
    {
        Some(true) => checks.push(readiness_check(
            "shared_runtime_policy",
            "pass",
            object_text(shared_runtime_policy.as_ref().unwrap(), "reason")
                .unwrap_or_else(|| "Shared runtime policy passed.".to_string()),
            None,
            "none",
        )),
        _ => checks.push(readiness_check(
            "shared_runtime_policy",
            "fail",
            "Unsupported or missing server runtime policy facts are blocked by policy.",
            shared_runtime_policy
                .as_ref()
                .and_then(|row| object_text(row, "reason")),
            "configure_postgres",
        )),
    }

    match rust_server_core_seam
        .as_ref()
        .and_then(|row| object_bool(row, "rust_authority_ready"))
    {
        Some(true) => checks.push(readiness_check(
            "rust_server_core_seam",
            "pass",
            "Rust ait-server-core seam satisfied the current server capability contract.",
            None,
            "none",
        )),
        _ => checks.push(readiness_check(
            "rust_server_core_seam",
            "fail",
            "Rust ait-server-core seam is not ready for the current server capability contract.",
            rust_server_core_seam
                .as_ref()
                .and_then(|row| row.get("issues").cloned())
                .map(|value| value.to_string()),
            "run_core_build",
        )),
    }

    let storage_attention = int_field(&summary, "repos_needing_storage_attention");
    let drift_count = int_field(&storage, "drift_count");
    if storage_attention > 0 || drift_count > 0 {
        checks.push(readiness_check(
            "storage_integrity",
            "fail",
            "One or more repositories need storage attention.",
            Some(format!(
                "repos_needing_attention={storage_attention}; drift_count={drift_count}"
            )),
            "inspect_storage",
        ));
    } else {
        checks.push(readiness_check(
            "storage_integrity",
            "pass",
            "No repository storage drift or attention count is reported.",
            None,
            "none",
        ));
    }

    let stale_jobs = int_field(&jobs, "stale_running_jobs");
    let failed_jobs = int_field(&jobs, "failed_jobs");
    let exhausted_jobs = int_field(&jobs, "exhausted_jobs");
    let delayed_retry_jobs = int_field(&jobs, "delayed_retry_jobs");
    let active_jobs = int_field(&jobs, "active_jobs");
    let job_action = object_text(&jobs, "recommended_action").unwrap_or_else(|| "none".to_string());
    if stale_jobs > 0 || failed_jobs > 0 || exhausted_jobs > 0 {
        checks.push(readiness_check(
            "job_recovery",
            "fail",
            "Job recovery attention is required before treating the server as ready.",
            Some(format!(
                "stale_running_jobs={stale_jobs}; failed_jobs={failed_jobs}; exhausted_jobs={exhausted_jobs}; delayed_retry_jobs={delayed_retry_jobs}"
            )),
            if job_action != "none" { &job_action } else { "inspect_failed" },
        ));
    } else if delayed_retry_jobs > 0 {
        checks.push(readiness_check(
            "job_recovery",
            "warn",
            "Retryable jobs are waiting for their next attempt.",
            Some(format!(
                "delayed_retry_jobs={delayed_retry_jobs}; active_jobs={active_jobs}"
            )),
            if job_action != "none" {
                &job_action
            } else {
                "wait_for_retry"
            },
        ));
    } else {
        checks.push(readiness_check(
            "job_recovery",
            "pass",
            "No stale, failed, exhausted, or delayed retry jobs are reported.",
            Some(format!("active_jobs={active_jobs}")),
            "none",
        ));
    }

    if input.db_backend == "postgres" && input.using_postgres {
        if postgres_schema
            .as_ref()
            .and_then(|row| object_bool(row, "ok"))
            .unwrap_or(false)
        {
            checks.push(readiness_check(
                "postgres_schema_status",
                "pass",
                "PostgreSQL content/control schema versions match this server.",
                None,
                "none",
            ));
        } else {
            checks.push(readiness_check(
                "postgres_schema_status",
                "fail",
                "PostgreSQL schema version checks did not pass.",
                postgres_schema
                    .as_ref()
                    .and_then(|row| object_text(row, "error"))
                    .or_else(|| {
                        Some("content/control schema version mismatch or missing facts".to_string())
                    }),
                "inspect_postgres",
            ));
        }
    } else {
        checks.push(readiness_check(
            "postgres_schema_status",
            "fail",
            "PostgreSQL is required for server runtime state.",
            None,
            "configure_postgres",
        ));
    }

    let failed_checks = checks
        .iter()
        .filter(|check| check["status"] == json!("fail"))
        .count();
    let warning_checks = checks
        .iter()
        .filter(|check| check["status"] == json!("warn"))
        .count();
    let mut action_candidates = checks
        .iter()
        .filter_map(|check| value_text(check, "recommended_action"))
        .filter(|action| action != "none")
        .collect::<Vec<_>>();
    let metrics_action =
        object_text(&summary, "recommended_action").unwrap_or_else(|| "none".to_string());
    if metrics_action != "none" {
        action_candidates.push(metrics_action);
    }
    let recommended_action = ranked_operator_action(&action_candidates);
    json!({
        "repo_name": input.repo_name,
        "snapshot_at": input.snapshot_at,
        "ready": failed_checks == 0,
        "recommended_action": recommended_action,
        "runtime": {
            "db_backend": input.db_backend,
            "using_postgres": input.using_postgres,
            "server_data_root": input.server_data_root,
            "postgres_only_runtime": true,
            "shared_runtime_policy": shared_runtime_policy.unwrap_or_else(|| json_map(json!({"ok": false, "reason": "missing shared runtime policy facts"}))),
            "rust_server_core_seam": rust_server_core_seam.unwrap_or_else(|| json_map(json!({"rust_authority_ready": false, "issues": ["missing rust server-core seam facts"]}))),
        },
        "summary": {
            "repo_count": int_field(&summary, "repo_count"),
            "total_lines": int_field(&summary, "total_lines"),
            "active_workers": int_field(&summary, "active_workers"),
            "active_jobs": active_jobs,
            "failed_jobs": failed_jobs,
            "active_live_turns": int_field(&summary, "active_live_turns"),
            "warning_checks": warning_checks,
            "failed_checks": failed_checks,
        },
        "checks": checks,
        "metrics_summary": summary,
        "repository_names": metrics.get("repositories").and_then(JsonValue::as_array).into_iter().flatten().filter_map(|row| value_text(row, "repo_name")).collect::<Vec<_>>(),
        "storage_summary": {
            "repos_needing_attention": int_field(&storage, "repos_needing_attention"),
            "drift_count": drift_count,
            "repairable_drift_count": int_field(&storage, "repairable_drift_count"),
            "recommended_action": storage.get("recommended_action").cloned().unwrap_or(JsonValue::Null),
        },
        "worker_summary": {
            "active_worker_count": int_field(&workers, "active_worker_count"),
            "queued_jobs": int_field(&workers, "queued_jobs"),
            "running_jobs": int_field(&workers, "running_jobs"),
            "stale_running_jobs": stale_jobs,
            "delayed_retry_jobs": delayed_retry_jobs,
            "exhausted_jobs": exhausted_jobs,
            "active_live_turns": int_field(&workers, "active_live_turns"),
            "oldest_live_turn_age_seconds": round_f64(optional_f64_field(&workers, "oldest_live_turn_age_seconds").unwrap_or(0.0), 3),
        },
        "job_summary": {
            "total_jobs": int_field(&jobs, "total_jobs"),
            "active_jobs": active_jobs,
            "failed_jobs": failed_jobs,
            "stale_running_jobs": stale_jobs,
            "delayed_retry_jobs": delayed_retry_jobs,
            "exhausted_jobs": exhausted_jobs,
            "recommended_action": jobs.get("recommended_action").cloned().unwrap_or(JsonValue::Null),
        },
        "live_turn_summary": {
            "active_turns": int_field(&live_turn_summary, "active_turns"),
            "active_repositories": int_field(&live_turn_summary, "active_repositories"),
            "oldest_active_turn_started_at": live_turn_summary.get("oldest_active_turn_started_at").cloned().unwrap_or(JsonValue::Null),
            "oldest_active_turn_age_seconds": round_f64(optional_f64_field(&live_turn_summary, "oldest_active_turn_age_seconds").unwrap_or(0.0), 3),
            "recent_completed_turns": int_field(&live_turn_summary, "recent_completed_turns"),
            "recent_failed_turns": int_field(&live_turn_summary, "recent_failed_turns"),
            "recent_completed_p95_seconds": live_turn_summary.get("recent_completed_p95_seconds").cloned().unwrap_or(JsonValue::Null),
        },
        "live_turn_pressure": live_turn_pressure_summary_from_normalized(&JsonValue::Object(live_turns.clone())),
        "postgres_schema": postgres_schema.unwrap_or_else(|| json_map(json!({"ok": false, "reason": "missing postgres schema facts"}))),
    })
}
