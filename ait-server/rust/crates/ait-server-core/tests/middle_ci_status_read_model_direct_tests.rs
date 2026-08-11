use ait_server_core::middle::ci_status_read_model::{
    repository_ci_runs_read_model, repository_ci_runs_read_model_contract, RepositoryCiRunsInput,
};
use serde_json::{json, Value as JsonValue};

fn runs(payload: JsonValue) -> JsonValue {
    let input = RepositoryCiRunsInput::from_value(&payload).expect("input should parse");
    repository_ci_runs_read_model(&input).expect("read model should build")
}

#[test]
fn repository_ci_runs_contract_names_rust_ownership_and_row_sets() {
    let contract = repository_ci_runs_read_model_contract();

    assert_eq!(contract.domain_id, "repository_ci_runs");
    assert_eq!(contract.reference_module, "rust_owned_no_python_reference");
    assert_eq!(contract.public_surface, "native.read.repository_ci_runs");
    assert!(!contract.mutates_state);
    assert!(contract.row_set("jobs").is_some());
}

#[test]
fn repository_ci_runs_filter_by_plane_and_suite_with_latest_indexes() {
    let value = runs(json!({
        "repo_name": "ait-server",
        "limit": 10,
        "plane": "nightly",
        "suite_id": "rust_core",
        "jobs": [
            {
                "job_id": 42,
                "job_type": "repo.ci",
                "state": "succeeded",
                "diagnostic_status": "ok",
                "created_at": "2026-07-08T01:00:00Z",
                "updated_at": "2026-07-08T01:10:00Z",
                "payload": {"repo_name": "ait-server", "plane": "nightly", "suite_ids": ["rust_core"], "target_line": "main"},
                "result": {"status": "pass", "selected_suite_ids": ["rust_core"], "selected_planes": ["nightly"], "suite_results": []}
            },
            {
                "job_id": 43,
                "job_type": "repo.ci",
                "state": "failed",
                "payload": {"repo_name": "ait-server", "plane": "release", "suite_ids": ["full"]},
                "result": {"status": "fail", "selected_suite_ids": ["full"], "selected_planes": ["release"]}
            },
            {
                "job_id": 44,
                "job_type": "patchset.ci",
                "state": "succeeded",
                "payload": {"repo_name": "ait-server"},
                "result": {"status": "pass"}
            }
        ]
    }));

    assert_eq!(value["repo_name"], json!("ait-server"));
    assert_eq!(value["filters"]["plane"], json!("nightly"));
    assert_eq!(value["filters"]["suite_id"], json!("rust_core"));
    assert_eq!(value["count"], json!(1));
    assert_eq!(value["summary"]["active_runs"], json!(0));
    assert_eq!(value["summary"]["failed_runs"], json!(0));
    assert_eq!(value["items"][0]["job_id"], json!(42));
    assert_eq!(value["items"][0]["status"], json!("pass"));
    assert_eq!(
        value["summary"]["latest_by_suite"]["rust_core"]["job_id"],
        json!(42)
    );
    assert_eq!(
        value["summary"]["latest_by_plane"]["nightly"]["status"],
        json!("pass")
    );
}

#[test]
fn repository_ci_runs_status_falls_back_to_worker_state() {
    let value = runs(json!({
        "repo_name": "ait-server",
        "jobs": [
            {"job_id": 1, "job_type": "repo.ci", "state": "queued", "payload": {}, "result": {}},
            {"job_id": 2, "job_type": "repo.ci", "state": "failed", "payload": {}, "result": {}}
        ]
    }));

    assert_eq!(value["count"], json!(2));
    assert_eq!(value["summary"]["active_runs"], json!(1));
    assert_eq!(value["summary"]["failed_runs"], json!(1));
    assert_eq!(value["items"][0]["status"], json!("pending"));
    assert_eq!(value["items"][1]["status"], json!("fail"));
}

#[test]
fn repository_ci_runs_project_task_batch_artifacts_and_rerun_command() {
    let value = runs(json!({
        "repo_name": "ait-server",
        "jobs": [
            {
                "job_id": 7,
                "job_type": "repo.ci",
                "state": "failed",
                "payload": {
                    "repo_name": "ait-server",
                    "suite_ids": ["task_batch"],
                    "target_line": "release/1",
                    "selector": "recent failures",
                    "task_ids": ["RT-1"],
                    "count": 3
                },
                "result": {
                    "blocking_failures": ["task_batch"],
                    "suite_results": [{
                        "suite_id": "task_batch",
                        "runner_kind": "rust_task_batch",
                        "selector": "recent failures",
                        "status": "fail",
                        "selected_tasks": [{"task_id": "RT-1"}],
                        "lineage_findings": {"problem_count": 2},
                        "behavior_regressions": {"status": "fail"},
                        "artifacts": {
                            "summary_json": {"path": ".ait/generated/summary.json", "exists": true, "size_bytes": 99},
                            "log_path": {"path": ".ait/generated/run.log", "exists": true}
                        }
                    }]
                }
            }
        ]
    }));

    let item = &value["items"][0];
    assert_eq!(item["blocking_failures"], json!(["task_batch"]));
    assert_eq!(item["task_batch"]["selected_task_count"], json!(1));
    assert_eq!(
        item["task_batch"]["selected_tasks"][0]["selection_reason"],
        json!("recent failures")
    );
    assert_eq!(item["task_batch"]["lineage_problem_count"], json!(2));
    assert_eq!(
        item["summary_artifacts"][0]["artifact_key"],
        json!("summary_json")
    );
    assert_eq!(item["summary_artifacts"][0]["exists"], json!(true));
    assert_eq!(
        item["rerun"]["cli"],
        json!("ait repo run-ci --suite task_batch --target-line release/1 --selector 'recent failures' --task-id RT-1 --count 3")
    );
}

#[test]
fn repository_ci_runs_rejects_non_object_payload() {
    let error = RepositoryCiRunsInput::from_value(&json!([])).expect_err("payload must be object");

    assert_eq!(
        error,
        "repository CI runs read-model payload must be a JSON object."
    );
}
