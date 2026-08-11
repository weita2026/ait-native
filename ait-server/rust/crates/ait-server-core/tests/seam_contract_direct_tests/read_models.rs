#[test]
fn queue_read_model_summary_command_filters_active_queue_payloads() {
    let payload = json!({
        "repo_name": "ait",
        "status": "active",
        "include_all_changes": true,
        "tasks": [
            {"task_id": "RT-1", "repo_name": "ait", "status": "active", "title": "Active", "created_at": "2026-06-25T00:00:00Z"},
            {"task_id": "RT-2", "repo_name": "ait", "status": "completed", "title": "Done", "created_at": "2026-06-24T00:00:00Z"}
        ],
        "changes": [
            {"change_id": "RC-1", "task_id": "RT-1", "repo_name": "ait", "status": "draft", "title": "Draft", "base_line": "main", "updated_at": "2026-06-25T01:00:00Z"},
            {"change_id": "RC-2", "task_id": "RT-2", "repo_name": "ait", "status": "landed", "title": "Landed", "base_line": "main", "updated_at": "2026-06-24T01:00:00Z"}
        ]
    })
    .to_string();
    let value = stdout_json(&run_seam_with_stdin(
        &["queue-read-model-summary", "-"],
        &payload,
    ));

    assert_eq!(value["task_queue"]["count"], json!(1));
    assert_eq!(
        value["query_plan"]["queue_change_task_ids"],
        json!(["RT-1"])
    );
    assert_eq!(value["change_inventory"]["count"], json!(1));
    assert_eq!(
        value["change_inventory"]["items"][0]["change_id"],
        json!("RC-1")
    );
}

#[test]
fn repository_ci_runs_read_model_command_projects_job_rows() {
    let payload = json!({
        "repo_name": "ait-server",
        "limit": 10,
        "plane": "nightly",
        "suite_id": "rust_core",
        "jobs": [
            {
                "job_id": 42,
                "job_type": "repo.ci",
                "state": "succeeded",
                "payload": {"plane": "nightly", "suite_ids": ["rust_core"]},
                "result": {"status": "pass", "selected_suite_ids": ["rust_core"], "selected_planes": ["nightly"]}
            },
            {
                "job_id": 43,
                "job_type": "patchset.ci",
                "state": "succeeded",
                "payload": {},
                "result": {"status": "pass"}
            }
        ]
    })
    .to_string();
    let value = stdout_json(&run_seam_with_stdin(
        &["repository-ci-runs-read-model", "-"],
        &payload,
    ));

    assert_eq!(value["repo_name"], json!("ait-server"));
    assert_eq!(value["filters"]["plane"], json!("nightly"));
    assert_eq!(value["count"], json!(1));
    assert_eq!(value["items"][0]["job_id"], json!(42));
    assert_eq!(
        value["summary"]["latest_by_suite"]["rust_core"]["status"],
        json!("pass")
    );
}

#[test]
fn secondary_read_model_commands_project_authority_and_reviewer_payloads() {
    let authority_payload = json!({
        "repo_name": "ait-server",
        "documents": [
            {"path": "docs/plan.md", "markdown": "# Plan\n\n[Engineering](engineering_plan.md)"},
            {"path": "docs/engineering_plan.md", "markdown": "# Engineering"},
            {"path": "docs/runtime.md", "markdown": "# Runtime\nAuthority: [Engineering](engineering_plan.md)"}
        ]
    })
    .to_string();
    let authority = stdout_json(&run_seam_with_stdin(
        &["authority-map-read-model", "-"],
        &authority_payload,
    ));
    assert_eq!(authority["repo_name"], json!("ait-server"));
    assert_eq!(
        authority["layer1"]["related_documents"][0]["path"],
        json!("docs/engineering_plan.md")
    );
    assert_eq!(
        authority["layer2"][0]["children"][0]["path"],
        json!("docs/runtime.md")
    );

    let reviewer_payload = json!({
        "changes": [{
            "repo_name": "ait",
            "change_id": "C-1",
            "title": "Reviewer inbox",
            "base_line": "main",
            "status": "review",
            "current_patchset_id": "P-1",
            "updated_at": "2026-07-08T00:00:00Z"
        }],
        "patchsets": [{
            "change_id": "C-1",
            "patchset_id": "P-1",
            "patchset_number": 1,
            "base_snapshot_id": "SBASE"
        }],
        "attestations": [{
            "patchset_id": "P-1",
            "author_mode": "ai_with_human_review",
            "evaluation_summary_json": "{\"tests\":\"pass\"}",
            "provenance_summary_json": "{\"model_name\":\"GPT-5 Codex\"}"
        }],
        "policy_decisions": [{
            "patchset_id": "P-1",
            "decision": "pass",
            "checks_json": "[]",
            "effective_requirements_json": "{\"require_tests\":true}"
        }],
        "refs": [{
            "repo_name": "ait",
            "line_name": "main",
            "head_snapshot_id": "SBASE"
        }]
    })
    .to_string();
    let reviewer = stdout_json(&run_seam_with_stdin(
        &["reviewer-inbox-read-model", "-"],
        &reviewer_payload,
    ));
    assert_eq!(reviewer["count"], json!(1));
    assert_eq!(reviewer["items"][0]["change_id"], json!("C-1"));
    assert_eq!(reviewer["items"][0]["freshness"]["state"], json!("fresh"));
    assert_eq!(reviewer["items"][0]["attestation"]["tests"], json!("pass"));
}

#[test]
fn metrics_read_model_commands_project_runtime_and_operator_payloads() {
    let runtime_payload = json!({
        "live_turn_metrics": {
            "active_turns": 4,
            "active_turns_by_repo": {"ait": 3, "ait-server": 1},
            "oldest_active_turn_age_seconds": 301.456
        }
    })
    .to_string();
    let runtime = stdout_json(&run_seam_with_stdin(
        &["runtime-metrics-read-model", "-"],
        &runtime_payload,
    ));
    assert_eq!(
        runtime["live_turn_metrics"]["summary"]["active_turns"],
        json!(4)
    );
    assert_eq!(
        runtime["live_turn_pressure"]["pressure_state"],
        json!("saturated")
    );

    let operator_payload = json!({
        "snapshot_at": "2026-07-08T01:00:00Z",
        "repositories": [{"repo_name": "ait", "line_count": 1}],
        "repository_storage": [{
            "repo_name": "ait",
            "validation_summary": {"state": "ok", "needs_attention": false, "recommended_action": "none"},
            "signals_summary": {"drift_count": 0}
        }],
        "jobs": [{"job_id": 1, "job_type": "land", "state": "queued"}],
        "shared_runtime_policy": [{"ok": true, "reason": "postgres"}],
        "rust_server_core_seam": [{"rust_authority_ready": true, "issues": []}],
        "postgres_schema": [{"ok": true}],
        "live_turn_metrics": {"active_turns": 0}
    })
    .to_string();
    let metrics = stdout_json(&run_seam_with_stdin(
        &["operator-metrics-read-model", "-"],
        &operator_payload,
    ));
    assert_eq!(metrics["summary"]["repo_count"], json!(1));
    assert_eq!(metrics["job_outcome_metrics"]["active_jobs"], json!(1));

    let readiness = stdout_json(&run_seam_with_stdin(
        &["operator-readiness-read-model", "-"],
        &operator_payload,
    ));
    assert_eq!(readiness["ready"], json!(true));
    assert_eq!(readiness["summary"]["failed_checks"], json!(0));
}

#[test]
fn workflow_task_detail_read_model_command_projects_task_summary() {
    let payload = json!({
        "task": {
            "task_id": "RT-10",
            "repo_name": "ait",
            "title": "Workflow detail",
            "intent": "Port task detail projection.",
            "status": "active",
            "created_at": "2026-07-08T00:00:00Z"
        },
        "repository": {
            "repo_name": "ait",
            "repo_id": "repo-ait",
            "default_line": "main"
        },
        "changes": [{
            "change_id": "RC-10",
            "repo_name": "ait",
            "task_id": "RT-10",
            "title": "Draft",
            "base_line": "main",
            "status": "draft",
            "current_patchset_number": 0,
            "created_at": "2026-07-08T00:01:00Z",
            "updated_at": "2026-07-08T00:01:00Z"
        }]
    })
    .to_string();
    let value = stdout_json(&run_seam_with_stdin(
        &["workflow-task-detail-read-model", "-"],
        &payload,
    ));

    assert_eq!(value["summary"]["change_count"], json!(1));
    assert_eq!(
        value["task_review"]["unresolved_gaps"][0],
        json!("RC-10 has no published patchset yet.")
    );
    assert_eq!(value["code_review"]["verdict"], json!("needs_fix"));
}

#[test]
fn repository_worker_status_read_model_command_projects_workers() {
    let payload = json!({
        "repository": {"repo_name": "ait", "repo_id": "repo-ait"},
        "jobs": [
            {
                "repo_name": "ait",
                "job_id": 2,
                "state": "running",
                "locked_by": "worker-a",
                "locked_at": "2026-07-08T00:00:00Z",
                "updated_at": "2026-07-08T00:05:00Z"
            },
            {"repo_name": "ait", "job_id": 1, "state": "queued", "updated_at": "2026-07-08T00:01:00Z"}
        ],
        "recent_jobs": [{"repo_name": "ait", "job_id": 2, "state": "running"}],
        "diagnostics": {"recommended_action": "monitor_workers"}
    })
    .to_string();
    let value = stdout_json(&run_seam_with_stdin(
        &["repository-worker-status-read-model", "-"],
        &payload,
    ));

    assert_eq!(value["repo_name"], json!("ait"));
    assert_eq!(value["state_summary"]["running"], json!(1));
    assert_eq!(value["queued_jobs"], json!(1));
    assert_eq!(value["workers"][0]["worker_id"], json!("worker-a"));
    assert_eq!(value["recent_jobs"][0]["job_id"], json!(2));
}
