use ait_server_core::middle::metrics_read_model::{
    operator_metrics_read_model, operator_metrics_read_model_contract,
    operator_readiness_read_model, runtime_metrics_read_model, runtime_metrics_read_model_contract,
    OperatorMetricsInput, RuntimeMetricsInput,
};
use serde_json::{json, Value as JsonValue};

fn runtime(payload: JsonValue) -> JsonValue {
    let input = RuntimeMetricsInput::from_value(&payload).expect("runtime input should parse");
    runtime_metrics_read_model(&input).expect("runtime metrics should project")
}

fn operator_input(payload: JsonValue) -> OperatorMetricsInput {
    OperatorMetricsInput::from_value(&payload).expect("operator input should parse")
}

fn operator_metrics(payload: JsonValue) -> JsonValue {
    operator_metrics_read_model(&operator_input(payload)).expect("operator metrics should project")
}

fn operator_readiness(payload: JsonValue) -> JsonValue {
    operator_readiness_read_model(&operator_input(payload))
        .expect("operator readiness should project")
}

#[test]
fn metrics_contracts_name_rust_ownership_and_row_sets() {
    let runtime_contract = runtime_metrics_read_model_contract();
    assert_eq!(runtime_contract.domain_id, "runtime_metrics");
    assert_eq!(
        runtime_contract.reference_module,
        "rust_owned_no_python_reference"
    );
    assert_eq!(
        runtime_contract.public_surface,
        "middle.metrics_read_model.runtime_metrics"
    );
    assert!(!runtime_contract.mutates_state);
    assert!(runtime_contract.row_set("repo_activity").is_some());
    assert!(runtime_contract.row_set("recent_completed_turns").is_some());

    let operator_contract = operator_metrics_read_model_contract();
    assert_eq!(operator_contract.domain_id, "operator_metrics");
    assert_eq!(
        operator_contract.reference_module,
        "rust_owned_no_python_reference"
    );
    assert_eq!(
        operator_contract.public_surface,
        "middle.metrics_read_model.operator_metrics"
    );
    assert!(!operator_contract.mutates_state);
    assert!(operator_contract.row_set("repositories").is_some());
    assert!(operator_contract.row_set("repository_storage").is_some());
    assert!(operator_contract.row_set("job_diagnostics").is_some());
    assert!(operator_contract.row_set("postgres_schema").is_some());
}

#[test]
fn runtime_metrics_normalize_live_turns_and_pressure_states() {
    let value = runtime(json!({
        "live_turn_metrics": {
            "active_turn_count": "3",
            "oldest_active_turn_started_at": "2026-07-08T00:00:00Z",
            "oldest_active_turn_age_seconds": 130.12345,
            "recent_completed_p95_seconds": "7.4567",
            "active_turns_by_repo": {"ait": "2", "ait-server": 1, "empty": 0},
            "recent_completed_turns": [{"turn_id": "done-1"}],
            "recent_failed_turns": [{"turn_id": "fail-1"}, {"turn_id": "fail-2"}],
            "snapshot_at_epoch_seconds": 1780000000
        }
    }));

    assert_eq!(
        value["live_turn_metrics"]["summary"]["active_turns"],
        json!(3)
    );
    assert_eq!(
        value["live_turn_metrics"]["summary"]["active_repositories"],
        json!(2)
    );
    assert_eq!(
        value["live_turn_metrics"]["summary"]["oldest_active_turn_age_seconds"],
        json!(130.123)
    );
    assert_eq!(
        value["live_turn_metrics"]["summary"]["recent_completed_p95_seconds"],
        json!(7.457)
    );
    assert_eq!(value["live_turn_pressure"]["pressure_state"], json!("busy"));
    assert_eq!(
        value["live_turn_pressure"]["active_repositories_by_name"],
        json!({"ait": 2, "ait-server": 1})
    );
}

#[test]
fn operator_metrics_aggregate_storage_workers_jobs_and_actions() {
    let value = operator_metrics(json!({
        "repo_name": "ait",
        "snapshot_at": "2026-07-08T01:00:00Z",
        "cached_at": "2026-07-08T01:00:01Z",
        "cache_age_seconds": 1.2345,
        "cache_ttl_seconds": 9,
        "recent_jobs_limit": 2,
        "stale_after_seconds": 600,
        "live_turn_metrics": {
            "active_turns_by_repo": {"ait": 2},
            "active_turns": 2,
            "oldest_active_turn_age_seconds": 10
        },
        "repositories": [
            {"repo_name": "ait", "default_line": "main", "line_count": 2},
            {"repo_name": "ait-server", "default_line": "main", "line_count": 1}
        ],
        "repository_storage": [
            {
                "repo_name": "ait",
                "snapshot_count": 5,
                "packed_blob_count": 11,
                "packed_delta_blob_count": 3,
                "pack_count": 2,
                "validation_summary": {"state": "attention", "needs_attention": true, "recommended_action": "repack"},
                "signals_summary": {"drift_count": 2, "repairable_drift_count": 1},
                "optimization_summary": {"tracked_blob_count": 20},
                "efficiency_summary": {"logical_tracked_blob_bytes": 100, "physical_storage_bytes": 60, "storage_savings_bytes": 40}
            },
            {
                "repo_name": "ait-server",
                "snapshot_count": 3,
                "packed_blob_count": 7,
                "packed_delta_blob_count": 1,
                "pack_count": 1,
                "validation_summary": {"state": "ok", "needs_attention": false, "recommended_action": "none"},
                "signals_summary": {"drift_count": 0, "repairable_drift_count": 0},
                "optimization_summary": {"tracked_blob_count": 8},
                "efficiency_summary": {"logical_tracked_blob_bytes": 30, "physical_storage_bytes": 20, "storage_savings_bytes": 10}
            }
        ],
        "repository_workers": [
            {
                "repo_name": "ait",
                "worker_count": 1,
                "queued_jobs": 2,
                "running_jobs": 1,
                "succeeded_jobs": 5,
                "failed_jobs": 1,
                "state_summary": {"queued": 2, "running": 1},
                "diagnostics": {"stale_running_jobs": 1, "delayed_retry_jobs": 0, "exhausted_jobs": 0, "recommended_action": "reclaim_stale"},
                "workers": [{"worker_id": "w1", "running_jobs": 1, "oldest_locked_job": "2026-07-08T00:00:00Z", "latest_locked_job": "2026-07-08T00:10:00Z"}]
            },
            {
                "repo_name": "ait-server",
                "worker_count": 1,
                "queued_jobs": 0,
                "running_jobs": 2,
                "succeeded_jobs": 4,
                "failed_jobs": 0,
                "state_summary": {"running": 2},
                "diagnostics": {"stale_running_jobs": 0, "delayed_retry_jobs": 1, "exhausted_jobs": 0, "recommended_action": "wait_for_retry"},
                "workers": [{"worker_id": "w1", "running_jobs": 2, "oldest_locked_job": "2026-07-08T00:05:00Z", "latest_locked_job": "2026-07-08T00:20:00Z"}]
            }
        ],
        "jobs": [
            {"job_id": 3, "repo_name": "ait", "job_type": "patchset_ci", "state": "running"},
            {"job_id": 2, "repo_name": "ait", "job_type": "land", "state": "queued"},
            {"job_id": 1, "repo_name": "ait-server", "job_type": "land", "state": "failed"}
        ],
        "job_diagnostics": [{
            "stale_running_jobs": 1,
            "delayed_retry_jobs": 1,
            "retryable_jobs": 1,
            "exhausted_jobs": 0,
            "recommended_action": "reclaim_stale",
            "recommended_action_reason": "running job is stale"
        }]
    }));

    assert_eq!(value["summary"]["repo_count"], json!(2));
    assert_eq!(value["summary"]["total_lines"], json!(3));
    assert_eq!(
        value["summary"]["repos_needing_storage_attention"],
        json!(1)
    );
    assert_eq!(
        value["summary"]["recommended_action"],
        json!("reclaim_stale")
    );
    assert_eq!(value["storage_metrics"]["total_snapshots"], json!(8));
    assert_eq!(value["storage_metrics"]["tracked_blob_count"], json!(28));
    assert_eq!(
        value["storage_metrics"]["storage_state_summary"],
        json!({"attention": 1, "ok": 1})
    );
    assert_eq!(
        value["storage_metrics"]["recommended_action"],
        json!("repack")
    );
    assert_eq!(value["worker_metrics"]["active_worker_count"], json!(1));
    assert_eq!(
        value["worker_metrics"]["workers"][0]["running_jobs"],
        json!(3)
    );
    assert_eq!(
        value["worker_metrics"]["state_summary"],
        json!({"queued": 2, "running": 3})
    );
    assert_eq!(value["job_outcome_metrics"]["total_jobs"], json!(3));
    assert_eq!(value["job_outcome_metrics"]["active_jobs"], json!(2));
    assert_eq!(
        value["job_outcome_metrics"]["recent_jobs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(value["live_turn_pressure"]["pressure_state"], json!("busy"));
    assert_eq!(value["repositories"][0]["active_live_turns"], json!(2));
    assert_eq!(value["cache_age_seconds"], json!(1.235));
    assert_eq!(value["cache_ttl_seconds"], json!(9.0));
    assert_eq!(value["cached_at"], json!("2026-07-08T01:00:01Z"));
}

#[test]
fn operator_readiness_fails_closed_on_missing_or_bad_critical_facts() {
    let value = operator_readiness(json!({
        "db_backend": "local-file",
        "using_postgres": false,
        "repositories": [{"repo_name": "ait", "line_count": 1}],
        "repository_storage": [{
            "repo_name": "ait",
            "validation_summary": {"state": "attention", "needs_attention": true},
            "signals_summary": {"drift_count": 1}
        }],
        "jobs": [{"job_id": 1, "state": "failed", "job_type": "land"}],
        "job_diagnostics": [{"stale_running_jobs": 0, "exhausted_jobs": 1, "recommended_action": "inspect_failed"}]
    }));

    assert_eq!(value["ready"], json!(false));
    assert_eq!(value["summary"]["failed_checks"], json!(5));
    assert_eq!(value["recommended_action"], json!("inspect_failed"));
    let checks = value["checks"].as_array().expect("checks");
    assert!(checks
        .iter()
        .any(|check| check["name"] == json!("shared_runtime_policy")
            && check["status"] == json!("fail")));
    assert!(checks
        .iter()
        .any(|check| check["name"] == json!("rust_server_core_seam")
            && check["status"] == json!("fail")));
    assert!(checks
        .iter()
        .any(|check| check["name"] == json!("postgres_schema_status")
            && check["status"] == json!("fail")));
}

#[test]
fn operator_readiness_passes_with_clean_facts() {
    let value = operator_readiness(json!({
        "db_backend": "postgres",
        "using_postgres": true,
        "server_data_root": "/tmp/ait",
        "repositories": [{"repo_name": "ait", "line_count": 1}],
        "repository_storage": [{
            "repo_name": "ait",
            "validation_summary": {"state": "ok", "needs_attention": false, "recommended_action": "none"},
            "signals_summary": {"drift_count": 0, "repairable_drift_count": 0}
        }],
        "repository_workers": [{
            "repo_name": "ait",
            "state_summary": {},
            "diagnostics": {"recommended_action": "none"}
        }],
        "shared_runtime_policy": [{"ok": true, "reason": "postgres runtime selected"}],
        "rust_server_core_seam": [{"rust_authority_ready": true, "issues": []}],
        "postgres_schema": [{"ok": true, "checks": {}}],
        "live_turn_metrics": {"active_turns": 0}
    }));

    assert_eq!(value["ready"], json!(true));
    assert_eq!(value["recommended_action"], json!("none"));
    assert_eq!(value["summary"]["failed_checks"], json!(0));
    assert_eq!(value["runtime"]["server_data_root"], json!("/tmp/ait"));
    assert_eq!(value["runtime"]["postgres_only_runtime"], json!(true));
    assert_eq!(value["postgres_schema"]["ok"], json!(true));
}
