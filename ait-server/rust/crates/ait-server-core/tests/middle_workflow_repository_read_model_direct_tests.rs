use ait_server_core::middle::workflow_repository_read_model::{
    repository_detail_read_model, repository_detail_read_model_contract,
    repository_index_read_model, repository_index_read_model_contract,
    repository_worker_status_read_model, repository_worker_status_read_model_contract,
    task_workflow_detail_read_model, task_workflow_detail_read_model_contract,
    RepositoryDetailInput, RepositoryIndexInput, RepositoryWorkerStatusInput,
    TaskWorkflowDetailInput,
};
use serde_json::{json, Value as JsonValue};

#[test]
fn contracts_name_rust_ownership_and_row_sets() {
    let task_contract = task_workflow_detail_read_model_contract();
    assert_eq!(task_contract.domain_id, "task_workflow_detail");
    assert_eq!(
        task_contract.reference_module,
        "rust_owned_no_python_reference"
    );
    assert_eq!(task_contract.public_surface, "native.read.task_detail");
    assert!(!task_contract.mutates_state);
    assert!(task_contract.row_set("changes").is_some());
    assert!(task_contract.row_set("patchset_deltas").is_some());

    let index_contract = repository_index_read_model_contract();
    assert_eq!(index_contract.domain_id, "repository_index");
    assert_eq!(
        index_contract.reference_module,
        "rust_owned_no_python_reference"
    );
    assert!(index_contract.row_set("repositories").is_some());

    let detail_contract = repository_detail_read_model_contract();
    assert_eq!(detail_contract.domain_id, "repository_detail");
    assert_eq!(
        detail_contract.reference_module,
        "rust_owned_no_python_reference"
    );
    assert_eq!(
        detail_contract.public_surface,
        "native.read.repository_detail"
    );
    assert!(detail_contract.row_set("line_work_contexts").is_some());

    let worker_contract = repository_worker_status_read_model_contract();
    assert_eq!(worker_contract.domain_id, "repository_worker_status");
    assert_eq!(
        worker_contract.reference_module,
        "rust_owned_no_python_reference"
    );
    assert_eq!(
        worker_contract.public_surface,
        "native.read.repository_worker_status"
    );
    assert!(worker_contract.row_set("jobs").is_some());
    assert!(worker_contract.row_set("recent_jobs").is_some());
}

#[test]
fn task_workflow_detail_projects_complete_landable_workflow() {
    let input = TaskWorkflowDetailInput::from_value(&json!({
        "task": {
            "task_id": "T-1",
            "repo_name": "demo",
            "repo_id": "repo-demo",
            "title": "Migrate workflow read model",
            "intent": "Move Python projection into Rust.",
            "status": "active",
            "created_at": "2026-07-08T01:00:00Z"
        },
        "repository": {
            "repo_name": "demo",
            "repo_id": "repo-demo",
            "default_line": "main"
        },
        "changes": [{
            "change_id": "C-1",
            "repo_name": "demo",
            "repo_id": "repo-demo",
            "task_id": "T-1",
            "title": "Rust projection",
            "base_line": "main",
            "status": "review",
            "current_patchset_number": 1,
            "selected_patchset_number": 1,
            "created_at": "2026-07-08T01:10:00Z",
            "updated_at": "2026-07-08T01:20:00Z"
        }],
        "patchsets": [{
            "patchset_id": "P-1",
            "repo_id": "repo-demo",
            "change_id": "C-1",
            "patchset_number": 1,
            "base_snapshot_id": "SBASE",
            "revision_snapshot_id": "SREV",
            "summary": "port projection",
            "author_mode": "ai_with_human_review",
            "publish_state": "published",
            "evaluation_state": "pass",
            "created_at": "2026-07-08T01:15:00Z"
        }],
        "reviews": [{
            "review_id": 1,
            "repo_id": "repo-demo",
            "change_id": "C-1",
            "patchset_id": "P-1",
            "reviewer": "codex",
            "action": "approve",
            "comment": "ok",
            "blocking": false,
            "created_at": "2026-07-08T01:25:00Z"
        }],
        "attestations": [{
            "attestation_id": "AT-P-1",
            "repo_id": "repo-demo",
            "patchset_id": "P-1",
            "author_mode": "ai_with_human_review",
            "evaluation_summary_json": "{\"tests\":\"pass\",\"lint\":\"pass\",\"security\":\"pass\",\"license\":\"pass\"}",
            "provenance_summary_json": "{}",
            "detail_json": "{}",
            "updated_at": "2026-07-08T01:30:00Z"
        }],
        "policy_decisions": [{
            "policy_decision_id": 1,
            "repo_id": "repo-demo",
            "patchset_id": "P-1",
            "decision": "pass",
            "checks_json": "[]",
            "created_at": "2026-07-08T01:31:00Z"
        }],
        "refs": [{
            "repo_name": "demo",
            "line_name": "main",
            "head_snapshot_id": "SBASE"
        }],
        "patchset_deltas": [{
            "patchset_id": "P-1",
            "against": "base",
            "files": [{
                "path": "rust/crates/ait-server-core/src/middle/workflow_repository_read_model.rs",
                "status": "added",
                "insertions": 4,
                "deletions": 1,
                "text_renderable": true
            }]
        }],
        "events": [{
            "event_type": "patchset_published",
            "entity_type": "patchset",
            "entity_id": "P-1",
            "payload_json": "{\"patchset_id\":\"P-1\"}",
            "actor_identity": "codex",
            "actor_type": "ai",
            "created_at": "2026-07-08T01:32:00Z"
        }]
    }))
    .expect("input should parse");

    let detail = task_workflow_detail_read_model(&input).expect("detail should project");
    assert_eq!(detail["summary"]["change_count"], 1);
    assert_eq!(detail["summary"]["open_change_count"], 1);
    assert_eq!(detail["summary"]["patchset_count"], 1);
    assert_eq!(detail["aggregate_diff"]["file_entries"], 1);
    assert_eq!(detail["aggregate_diff"]["insertions"], 4);
    assert_eq!(detail["aggregate_diff"]["deletions"], 1);
    assert_eq!(
        detail["changes"][0]["freshness"]["base_is_fresh"],
        JsonValue::Bool(true)
    );
    assert_eq!(detail["task_review"]["acceptance_status"], "complete");
    assert_eq!(detail["task_review"]["suggested_next_action"], "land");
    assert_eq!(detail["code_review"]["verdict"], "safe_to_promote");
    assert_eq!(detail["combined_recommendation"]["action"], "land");
    assert_eq!(detail["timeline"].as_array().unwrap().len(), 1);
}

#[test]
fn task_workflow_detail_preserves_missing_patchset_partial_behavior() {
    let input = TaskWorkflowDetailInput::from_value(&json!({
        "task": {
            "task_id": "T-2",
            "repo_name": "demo",
            "title": "Draft task",
            "intent": "Still below review surface.",
            "status": "active",
            "created_at": "2026-07-08T02:00:00Z"
        },
        "repository": {
            "repo_name": "demo",
            "repo_id": "repo-demo",
            "default_line": "main"
        },
        "changes": [{
            "change_id": "C-2",
            "repo_name": "demo",
            "task_id": "T-2",
            "title": "Draft change",
            "base_line": "main",
            "status": "draft",
            "current_patchset_number": 0,
            "created_at": "2026-07-08T02:10:00Z",
            "updated_at": "2026-07-08T02:10:00Z"
        }]
    }))
    .expect("input should parse");

    let detail = task_workflow_detail_read_model(&input).expect("detail should project");
    assert_eq!(detail["summary"]["change_count"], 1);
    assert_eq!(detail["summary"]["patchset_count"], 0);
    assert!(detail["changes"][0]["current_patchset"].is_null());
    assert_eq!(
        detail["changes"][0]["policy_summary"]["decision"],
        "pending"
    );
    assert!(detail["changes"][0]["attestation_summary"].is_null());
    assert_eq!(detail["task_review"]["acceptance_status"], "needs_followup");
    assert_eq!(
        detail["task_review"]["unresolved_gaps"][0],
        "C-2 has no published patchset yet."
    );
    assert_eq!(detail["code_review"]["verdict"], "needs_fix");
}

#[test]
fn repository_index_projects_counts_groups_and_latest_activity() {
    let input = RepositoryIndexInput::from_value(&json!({
        "repositories": [
            {
                "repo_name": "zeta",
                "repo_id": "repo-z",
                "default_line": "main",
                "created_at": "2026-07-07T00:00:00Z",
                "updated_at": "2026-07-07T01:00:00Z"
            },
            {
                "repo_name": "alpha",
                "repo_id": "repo-a",
                "default_line": "trunk",
                "created_at": "2026-07-08T00:00:00Z",
                "updated_at": "2026-07-08T03:00:00Z"
            }
        ],
        "lines": [
            {"repo_name": "alpha", "line_name": "trunk"},
            {"repo_name": "alpha", "line_name": "feature"},
            {"repo_name": "zeta", "line_name": "main"}
        ],
        "groups": [{
            "group_id": "main",
            "title": "Main",
            "sort_index": "7",
            "system_slug": "main",
            "is_main": true,
            "repo_names": ["alpha", "missing"]
        }]
    }))
    .expect("input should parse");

    let index = repository_index_read_model(&input).expect("index should project");
    assert_eq!(index["count"], 2);
    assert_eq!(index["total_lines"], 3);
    assert_eq!(index["repositories"][0]["repo_name"], "alpha");
    assert_eq!(index["repositories"][0]["line_count"], 2);
    assert_eq!(index["groups"][0]["repo_count"], 1);
    assert_eq!(index["groups"][0]["repositories"][0]["repo_name"], "alpha");
    assert_eq!(index["latest_activity"]["id"], "alpha");
}

#[test]
fn repository_detail_projects_line_storage_and_job_summaries() {
    let input = RepositoryDetailInput::from_value(&json!({
        "repository": {
            "repo_name": "demo",
            "repo_id": "repo-demo",
            "default_line": "main"
        },
        "job_limit": 25,
        "lines": [
            {"repo_name": "demo", "line_name": "main", "head_snapshot_id": "S1"},
            {"repo_name": "demo", "line_name": "feature", "head_snapshot_id": "S2"},
            {"repo_name": "demo", "line_name": "old", "head_snapshot_id": "S0", "status": "archived"}
        ],
        "line_work_contexts": [{
            "line_name": "feature",
            "change_id": "C-10",
            "change_status": "review"
        }],
        "jobs": [
            {"repo_name": "demo", "job_id": 1, "state": "queued"},
            {"repo_name": "demo", "job_id": 2, "state": "failed"},
            {"repo_name": "other", "job_id": 3, "state": "queued"}
        ],
        "ci_runs": [
            {"job_id": 1, "state": "queued", "status": "pending"},
            {"job_id": 2, "state": "succeeded", "status": "fail"}
        ],
        "diagnostics": {
            "stale_running_jobs": 3,
            "delayed_retry_jobs": 2,
            "exhausted_jobs": 1,
            "recommended_action": "inspect"
        },
        "storage": {
            "validation_summary": {
                "state": "warning",
                "recommended_action": "repair",
                "next_actions": ["sync"],
                "reasons": ["drift"],
                "needs_attention": true
            },
            "signals_summary": {
                "drift_count": 5,
                "repairable_drift_count": 4
            }
        }
    }))
    .expect("input should parse");

    let detail = repository_detail_read_model(&input).expect("detail should project");
    assert_eq!(detail["line_summary"]["total_lines"], 3);
    assert_eq!(detail["line_summary"]["active_lines"], 2);
    assert_eq!(detail["line_summary"]["archived_lines"], 1);
    assert!(detail["lines"][0]["work_context"].is_null());
    assert_eq!(detail["lines"][1]["work_context"]["change_id"], "C-10");
    assert_eq!(detail["job_summary"]["recent_jobs"], 2);
    assert_eq!(detail["job_summary"]["active_jobs"], 1);
    assert_eq!(detail["job_summary"]["failed_jobs"], 1);
    assert_eq!(detail["job_summary"]["stale_running_jobs"], 3);
    assert_eq!(detail["storage_summary"]["state"], "warning");
    assert_eq!(detail["storage_summary"]["needs_attention"], true);
    assert_eq!(detail["ci_summary"]["active_runs"], 1);
    assert_eq!(detail["ci_summary"]["failed_runs"], 1);
}

#[test]
fn repository_worker_status_projects_state_counts_and_active_workers() {
    let input = RepositoryWorkerStatusInput::from_value(&json!({
        "repository": {
            "repo_name": "demo",
            "repo_id": "repo-demo",
            "default_line": "main"
        },
        "jobs": [
            {
                "repo_name": "demo",
                "job_id": 5,
                "state": "running",
                "locked_by": "worker-b",
                "locked_at": "2026-07-08T00:30:00Z",
                "updated_at": "2026-07-08T00:40:00Z"
            },
            {
                "repo_name": "demo",
                "job_id": 4,
                "state": "running",
                "locked_by": "worker-a",
                "locked_at": "2026-07-08T00:10:00Z",
                "updated_at": "2026-07-08T00:35:00Z"
            },
            {
                "repo_name": "demo",
                "job_id": 3,
                "state": "running",
                "locked_by": "worker-a",
                "locked_at": "2026-07-08T00:20:00Z",
                "updated_at": "2026-07-08T00:30:00Z"
            },
            {"repo_name": "demo", "job_id": 2, "state": "queued", "updated_at": "2026-07-08T00:25:00Z"},
            {"repo_name": "demo", "job_id": 1, "state": "failed", "updated_at": "2026-07-08T00:15:00Z"},
            {"repo_name": "other", "job_id": 9, "state": "running", "locked_by": "worker-z"}
        ],
        "recent_jobs": [
            {"repo_name": "demo", "job_id": 5, "state": "running"},
            {"repo_name": "other", "job_id": 9, "state": "running"}
        ],
        "diagnostics": {
            "stale_running_jobs": 1,
            "delayed_retry_jobs": 2,
            "exhausted_jobs": 3,
            "recommended_action": "inspect_failed"
        }
    }))
    .expect("input should parse");

    let status = repository_worker_status_read_model(&input).expect("status should project");
    assert_eq!(status["repo_name"], "demo");
    assert_eq!(status["snapshot_at"], "2026-07-08T00:40:00Z");
    assert_eq!(status["state_summary"]["running"], 3);
    assert_eq!(status["state_summary"]["queued"], 1);
    assert_eq!(status["state_summary"]["failed"], 1);
    assert_eq!(status["queued_jobs"], 1);
    assert_eq!(status["running_jobs"], 3);
    assert_eq!(status["succeeded_jobs"], 0);
    assert_eq!(status["failed_jobs"], 1);
    assert_eq!(status["worker_count"], 2);
    assert_eq!(status["workers"][0]["worker_id"], "worker-a");
    assert_eq!(status["workers"][0]["running_jobs"], 2);
    assert_eq!(
        status["workers"][0]["oldest_locked_job"],
        "2026-07-08T00:10:00Z"
    );
    assert_eq!(
        status["workers"][0]["latest_locked_job"],
        "2026-07-08T00:20:00Z"
    );
    assert_eq!(status["workers"][1]["worker_id"], "worker-b");
    assert_eq!(status["diagnostics"]["exhausted_jobs"], 3);
    assert_eq!(status["recent_jobs"].as_array().unwrap().len(), 1);
}

#[test]
fn task_workflow_detail_rejects_non_object_payloads() {
    let err = TaskWorkflowDetailInput::from_value(&json!([])).expect_err("arrays are invalid");
    assert!(err.contains("task workflow detail read-model payload must be a JSON object"));
}
