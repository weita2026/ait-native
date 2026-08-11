use super::*;

#[test]
fn workflow_ready_ci_poll_reuses_workspace_and_refreshes_only_remote_ci_state() {
    let repo_tmp = tempdir().expect("repo tempdir");
    init_repo(&InitRequest {
        root: repo_tmp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_tmp.path()).expect("repo runtime");
    let patchset = json!({
        "patchset_id": "RCP-1",
        "change_id": "RCC-1",
        "base_snapshot_id": "SNP-BASE",
        "revision_snapshot_id": "SNP-REVISION",
    });
    let mut remote = FakeWorkflowCloseoutRemote {
        patchsets: BTreeMap::from([("RCP-1".to_string(), patchset.clone())]),
        ci_statuses: BTreeMap::from([(
            "RCP-1".to_string(),
            json!({
                "contract": "ait.server.patchset_ci.readiness.v1",
                "projection": "readiness",
                "patchset_id": "RCP-1",
                "change_id": "RCC-1",
                "repo_name": "fixture-ait",
                "available": true,
                "tests_status": "pass",
                "selected_suite_ids": ["suite-1"],
                "suite_result_count": 1,
                "blocking_failure_count": 0,
                "has_runnable_evidence": true,
                "recent_limit_applied": 10,
                "latest_job": {
                    "job_type": "patchset.ci",
                    "state": "succeeded"
                },
                "recent_jobs": []
            }),
        )]),
        ..Default::default()
    };
    let state = json!({
        "change": {
            "change_id": "RCC-1",
            "task_id": "RCT-1",
            "base_line": "main",
            "status": "active"
        },
        "task": {"task_id": "RCT-1", "status": "active"},
        "patchset": patchset,
        "workspace": {
            "clean": true,
            "changed_count": 0,
            "current_line": "feature/rct-1",
            "head_snapshot_id": "SNP-REVISION",
            "workspace_status": "clean",
            "workspace_matches_patchset": true
        },
        "base_line": {"line_name": "main", "head_snapshot_id": "SNP-BASE"},
        "freshness": {
            "base_is_fresh": true,
            "preflight_state": "fresh",
            "recovery_required": false,
            "worktree_needs_retarget": false,
            "rebase_state": "idle",
            "remote_base_snapshot_id": "SNP-BASE",
            "patchset_base_snapshot_id": "SNP-BASE",
            "patchset_revision_snapshot_id": "SNP-REVISION"
        },
        "next_action": {"code": "waiting_for_ci"}
    });

    let refreshed = workflow_ready_ci_poll_payload_with_closeout_remote(
        &repo,
        &mut remote,
        "fixture-ait",
        &state,
        "RCC-1",
    )
    .expect("refresh CI-only ready state");

    assert_eq!(refreshed["workspace"], state["workspace"]);
    assert_eq!(
        refreshed["patchset_ci_status"]["tests_status"],
        json!("pass")
    );
    assert_eq!(
        refreshed["next_action"]["code"],
        json!("record_attestation")
    );
    assert_eq!(remote.requests.len(), 1);
    assert_eq!(remote.ci_status_requests.len(), 0);
    assert_eq!(remote.ci_readiness_requests.len(), 1);
    assert_eq!(remote.repo_job_requests, 0);

    remote.ci_statuses.insert(
        "RCP-1".to_string(),
        json!({"patchset_id": "RCP-1", "tests_status": "pass"}),
    );
    let error = workflow_ready_ci_poll_payload_with_closeout_remote(
        &repo,
        &mut remote,
        "fixture-ait",
        &state,
        "RCC-1",
    )
    .expect_err("malformed readiness must fail closed");
    assert!(error.contains("missing non-empty contract"));
    assert_eq!(remote.ci_status_requests.len(), 0);
    assert_eq!(remote.ci_readiness_requests.len(), 2);
    assert_eq!(remote.repo_job_requests, 0);
}
