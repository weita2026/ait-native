use super::helpers::*;
use super::live_turn_pressure::{
    live_turn_pressure_summary_from_normalized, normalize_live_turn_metrics,
};
use super::*;

pub fn operator_metrics_read_model(input: &OperatorMetricsInput) -> Result<JsonValue, String> {
    let metrics = build_operator_metrics(input)?;
    Ok(annotate_operator_read_payload(input, metrics))
}

pub(super) fn build_operator_metrics(input: &OperatorMetricsInput) -> Result<JsonValue, String> {
    let live_turn_metrics = normalize_live_turn_metrics(&input.live_turn_metrics)?;
    let live_turn_summary = live_turn_metrics
        .get("summary")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    let live_turn_repo_activity = live_turn_metrics
        .get("repo_activity")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(JsonValue::as_object)
        .filter_map(|row| {
            Some((
                object_text(row, "repo_name")?,
                int_field(row, "active_turns"),
            ))
        })
        .collect::<HashMap<_, _>>();
    let storage_by_repo = input
        .repository_storage
        .iter()
        .filter_map(|row| object_text(row, "repo_name").map(|repo| (repo, row)))
        .collect::<HashMap<_, _>>();
    let workers_by_repo = input
        .repository_workers
        .iter()
        .filter_map(|row| object_text(row, "repo_name").map(|repo| (repo, row)))
        .collect::<HashMap<_, _>>();

    let mut storage_state_summary = BTreeMap::<String, i64>::new();
    let mut worker_state_summary = BTreeMap::<String, i64>::new();
    let mut active_workers = BTreeMap::<String, ActiveWorkerAccumulator>::new();
    let mut storage_actions = Vec::new();
    let mut job_actions = Vec::new();
    let mut repository_rows = Vec::new();
    let mut storage_metrics = json!({
        "repo_count": input.repositories.len(),
        "total_lines": input.repositories.iter().map(|repo| int_field(repo, "line_count")).sum::<i64>(),
        "total_snapshots": 0,
        "tracked_blob_count": 0,
        "packed_blob_count": 0,
        "packed_delta_blob_count": 0,
        "pack_count": 0,
        "logical_tracked_blob_bytes": 0,
        "physical_storage_bytes": 0,
        "storage_savings_bytes": 0,
        "drift_count": 0,
        "repairable_drift_count": 0,
        "repos_needing_attention": 0,
    });
    let mut worker_metrics = json!({
        "repo_count": input.repositories.len(),
        "active_worker_count": 0,
        "queued_jobs": 0,
        "running_jobs": 0,
        "succeeded_jobs": 0,
        "failed_jobs": 0,
        "stale_running_jobs": 0,
        "delayed_retry_jobs": 0,
        "exhausted_jobs": 0,
        "active_live_turns": 0,
        "active_live_turn_repositories": 0,
        "oldest_live_turn_age_seconds": 0.0,
    });

    for repo in &input.repositories {
        let repo_name = object_text(repo, "repo_name").unwrap_or_default();
        if repo_name.is_empty() {
            continue;
        }
        let empty_storage = JsonMap::new();
        let empty_workers = JsonMap::new();
        let storage = storage_by_repo
            .get(&repo_name)
            .copied()
            .unwrap_or(&empty_storage);
        let workers = workers_by_repo
            .get(&repo_name)
            .copied()
            .unwrap_or(&empty_workers);
        let validation = object_field(storage, "validation_summary");
        let signals = object_field(storage, "signals_summary");
        let optimization = object_field(storage, "optimization_summary");
        let efficiency = object_field(storage, "efficiency_summary");
        let diagnostics = object_field(workers, "diagnostics");
        let storage_state =
            object_text(&validation, "state").unwrap_or_else(|| "unknown".to_string());
        let storage_action =
            object_text(&validation, "recommended_action").unwrap_or_else(|| "none".to_string());
        let job_action =
            object_text(&diagnostics, "recommended_action").unwrap_or_else(|| "none".to_string());
        increment_count(&mut storage_state_summary, &storage_state, 1);
        storage_actions.push(storage_action.clone());
        job_actions.push(job_action.clone());
        merge_count_summary(
            &mut worker_state_summary,
            object_field(workers, "state_summary"),
        );

        let active_worker_count = int_field(workers, "worker_count");
        let queued_jobs = int_field(workers, "queued_jobs");
        let running_jobs = int_field(workers, "running_jobs");
        let succeeded_jobs = int_field(workers, "succeeded_jobs");
        let failed_jobs = int_field(workers, "failed_jobs");
        let stale_running_jobs = value_int(
            &JsonValue::Object(diagnostics.clone()),
            "stale_running_jobs",
        );
        let delayed_retry_jobs = value_int(
            &JsonValue::Object(diagnostics.clone()),
            "delayed_retry_jobs",
        );
        let exhausted_jobs = value_int(&JsonValue::Object(diagnostics.clone()), "exhausted_jobs");
        let drift_count = int_field(&signals, "drift_count");
        let repairable_drift_count = int_field(&signals, "repairable_drift_count");
        let needs_attention = object_bool(&validation, "needs_attention").unwrap_or(false);

        add_metric(
            &mut storage_metrics,
            "total_snapshots",
            int_field(storage, "snapshot_count"),
        );
        add_metric(
            &mut storage_metrics,
            "tracked_blob_count",
            int_field(&optimization, "tracked_blob_count"),
        );
        add_metric(
            &mut storage_metrics,
            "packed_blob_count",
            int_field(storage, "packed_blob_count"),
        );
        add_metric(
            &mut storage_metrics,
            "packed_delta_blob_count",
            int_field(storage, "packed_delta_blob_count"),
        );
        add_metric(
            &mut storage_metrics,
            "pack_count",
            int_field(storage, "pack_count"),
        );
        add_metric(
            &mut storage_metrics,
            "logical_tracked_blob_bytes",
            int_field(&efficiency, "logical_tracked_blob_bytes"),
        );
        add_metric(
            &mut storage_metrics,
            "physical_storage_bytes",
            int_field(&efficiency, "physical_storage_bytes"),
        );
        add_metric(
            &mut storage_metrics,
            "storage_savings_bytes",
            int_field(&efficiency, "storage_savings_bytes"),
        );
        add_metric(&mut storage_metrics, "drift_count", drift_count);
        add_metric(
            &mut storage_metrics,
            "repairable_drift_count",
            repairable_drift_count,
        );
        add_metric(
            &mut storage_metrics,
            "repos_needing_attention",
            if needs_attention { 1 } else { 0 },
        );
        add_metric(
            &mut worker_metrics,
            "active_worker_count",
            active_worker_count,
        );
        add_metric(&mut worker_metrics, "queued_jobs", queued_jobs);
        add_metric(&mut worker_metrics, "running_jobs", running_jobs);
        add_metric(&mut worker_metrics, "succeeded_jobs", succeeded_jobs);
        add_metric(&mut worker_metrics, "failed_jobs", failed_jobs);
        add_metric(
            &mut worker_metrics,
            "stale_running_jobs",
            stale_running_jobs,
        );
        add_metric(
            &mut worker_metrics,
            "delayed_retry_jobs",
            delayed_retry_jobs,
        );
        add_metric(&mut worker_metrics, "exhausted_jobs", exhausted_jobs);

        for worker in nested_object_rows(workers, "workers") {
            let worker_id = object_text(&worker, "worker_id").unwrap_or_default();
            if worker_id.is_empty() {
                continue;
            }
            let item = active_workers
                .entry(worker_id.clone())
                .or_insert_with(|| ActiveWorkerAccumulator::new(worker_id));
            item.running_jobs += int_field(&worker, "running_jobs");
            item.repositories.insert(repo_name.clone());
            item.oldest_locked_job = oldest_text(
                item.oldest_locked_job.take(),
                object_text(&worker, "oldest_locked_job"),
            );
            item.latest_locked_job = latest_text(
                item.latest_locked_job.take(),
                object_text(&worker, "latest_locked_job"),
            );
        }

        repository_rows.push(json!({
            "repo_name": repo_name,
            "default_line": repo.get("default_line").cloned().unwrap_or(JsonValue::Null),
            "line_count": int_field(repo, "line_count"),
            "snapshot_count": int_field(storage, "snapshot_count"),
            "storage_state": storage_state,
            "storage_recommended_action": storage_action,
            "storage_needs_attention": needs_attention,
            "tracked_blob_count": int_field(&optimization, "tracked_blob_count"),
            "packed_blob_count": int_field(storage, "packed_blob_count"),
            "packed_delta_blob_count": int_field(storage, "packed_delta_blob_count"),
            "physical_storage_bytes": int_field(&efficiency, "physical_storage_bytes"),
            "storage_savings_bytes": int_field(&efficiency, "storage_savings_bytes"),
            "drift_count": drift_count,
            "repairable_drift_count": repairable_drift_count,
            "worker_count": active_worker_count,
            "queued_jobs": queued_jobs,
            "running_jobs": running_jobs,
            "succeeded_jobs": succeeded_jobs,
            "failed_jobs": failed_jobs,
            "stale_running_jobs": stale_running_jobs,
            "delayed_retry_jobs": delayed_retry_jobs,
            "exhausted_jobs": exhausted_jobs,
            "job_recommended_action": job_action,
            "active_live_turns": live_turn_repo_activity.get(&repo_name).copied().unwrap_or(0),
        }));
    }

    let mut active_worker_list = active_workers
        .into_values()
        .map(ActiveWorkerAccumulator::into_json)
        .collect::<Vec<_>>();
    active_worker_list.sort_by(|left, right| {
        value_int(right, "running_jobs")
            .cmp(&value_int(left, "running_jobs"))
            .then_with(|| value_text(left, "worker_id").cmp(&value_text(right, "worker_id")))
    });
    set_metric(
        &mut worker_metrics,
        "active_worker_count",
        active_worker_list.len() as i64,
    );
    worker_metrics["workers"] = JsonValue::Array(active_worker_list);
    worker_metrics["state_summary"] = json!(worker_state_summary);
    worker_metrics["active_live_turns"] = json!(int_field(&live_turn_summary, "active_turns"));
    worker_metrics["active_live_turn_repositories"] =
        json!(int_field(&live_turn_summary, "active_repositories"));
    worker_metrics["oldest_live_turn_age_seconds"] = json!(round_f64(
        optional_f64_field(&live_turn_summary, "oldest_active_turn_age_seconds").unwrap_or(0.0),
        3
    ));

    storage_metrics["storage_state_summary"] = json!(storage_state_summary);
    storage_metrics["recommended_action"] = json!(ranked_operator_action(&storage_actions));

    let job_state_summary = count_rows(&input.jobs, "state");
    let job_type_summary = count_rows(&input.jobs, "job_type");
    let diagnostics = input.job_diagnostics.first().cloned().unwrap_or_default();
    let job_action = object_text(&diagnostics, "recommended_action")
        .unwrap_or_else(|| ranked_operator_action(&job_actions));
    let job_outcome_metrics = json!({
        "total_jobs": input.jobs.len(),
        "state_summary": job_state_summary,
        "job_type_summary": job_type_summary,
        "active_jobs": count_value(&job_state_summary, "queued") + count_value(&job_state_summary, "running"),
        "succeeded_jobs": count_value(&job_state_summary, "succeeded"),
        "failed_jobs": count_value(&job_state_summary, "failed"),
        "stale_running_jobs": int_field(&diagnostics, "stale_running_jobs"),
        "delayed_retry_jobs": int_field(&diagnostics, "delayed_retry_jobs"),
        "retryable_jobs": int_field(&diagnostics, "retryable_jobs"),
        "exhausted_jobs": int_field(&diagnostics, "exhausted_jobs"),
        "recommended_action": job_action,
        "recommended_action_reason": diagnostics.get("recommended_action_reason").cloned().unwrap_or(JsonValue::Null),
        "recent_jobs_limit": input.recent_jobs_limit,
        "stale_after_seconds": input.stale_after_seconds,
        "recent_jobs": input.jobs.iter().take(input.recent_jobs_limit).cloned().map(JsonValue::Object).collect::<Vec<_>>(),
    });
    let operator_action = if value_text(&job_outcome_metrics, "recommended_action").as_deref()
        != Some("none")
    {
        value_text(&job_outcome_metrics, "recommended_action").unwrap_or_else(|| "none".to_string())
    } else if value_int(&storage_metrics, "repos_needing_attention") > 0 {
        "inspect_storage".to_string()
    } else {
        "none".to_string()
    };
    Ok(json!({
        "repo_name": input.repo_name,
        "snapshot_at": input.snapshot_at,
        "summary": {
            "repo_count": value_int(&storage_metrics, "repo_count"),
            "total_lines": value_int(&storage_metrics, "total_lines"),
            "repos_needing_storage_attention": value_int(&storage_metrics, "repos_needing_attention"),
            "active_workers": value_int(&worker_metrics, "active_worker_count"),
            "active_jobs": value_int(&job_outcome_metrics, "active_jobs"),
            "failed_jobs": value_int(&job_outcome_metrics, "failed_jobs"),
            "active_live_turns": value_int(&worker_metrics, "active_live_turns"),
            "recommended_action": operator_action,
        },
        "storage_metrics": storage_metrics,
        "worker_metrics": worker_metrics,
        "job_outcome_metrics": job_outcome_metrics,
        "live_turn_metrics": live_turn_metrics,
        "live_turn_pressure": live_turn_pressure_summary_from_normalized(&live_turn_metrics),
        "repositories": repository_rows,
    }))
}

struct ActiveWorkerAccumulator {
    worker_id: String,
    running_jobs: i64,
    repositories: BTreeSet<String>,
    oldest_locked_job: Option<String>,
    latest_locked_job: Option<String>,
}

impl ActiveWorkerAccumulator {
    fn new(worker_id: String) -> Self {
        Self {
            worker_id,
            running_jobs: 0,
            repositories: BTreeSet::new(),
            oldest_locked_job: None,
            latest_locked_job: None,
        }
    }

    fn into_json(self) -> JsonValue {
        json!({
            "worker_id": self.worker_id,
            "running_jobs": self.running_jobs,
            "repositories": self.repositories.into_iter().collect::<Vec<_>>(),
            "oldest_locked_job": self.oldest_locked_job,
            "latest_locked_job": self.latest_locked_job,
        })
    }
}
