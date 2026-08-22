use super::*;

#[test]
fn workflow_ready_waits_after_its_exact_ci_request_instead_of_resubmitting() {
    let state = json!({
        "change": {"change_id": "RCC-1", "status": "review"},
        "patchset": {"patchset_id": "RCP-1"},
        "next_action": {
            "code": "run_patchset_ci",
            "command": "ait patchset rerun-ci RCP-1"
        }
    });
    let requested = BTreeSet::from(["RCP-1".to_string()]);

    let pending = workflow_ready_ci_pending_wait_state(&state, "run_patchset_ci", &requested)
        .expect("normalize the just-requested Patchset")
        .expect("same-Patchset CI request must enter the pending wait");

    assert_eq!(pending["next_action"]["code"], json!("waiting_for_ci"));
    assert_eq!(state["next_action"]["code"], json!("run_patchset_ci"));
    assert!(
        workflow_ready_ci_pending_wait_state(&state, "run_patchset_ci", &BTreeSet::new())
            .expect("inspect a not-yet-requested Patchset")
            .is_none()
    );
    assert!(workflow_ready_ci_pending_wait_state(
        &state,
        "run_patchset_ci",
        &BTreeSet::from(["RCP-2".to_string()]),
    )
    .expect("inspect a different requested Patchset")
    .is_none());
    assert!(
        workflow_ready_ci_pending_wait_state(&state, "record_attestation", &requested)
            .expect("inspect a non-CI action")
            .is_none()
    );

    let mut already_waiting = state.clone();
    already_waiting["next_action"]["code"] = json!("waiting_for_ci");
    assert_eq!(
        workflow_ready_ci_pending_wait_state(&already_waiting, "waiting_for_ci", &BTreeSet::new())
            .expect("retain an authoritative waiting state"),
        Some(already_waiting)
    );
}

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
        Some("mirror"),
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
    assert_eq!(
        refreshed["next_action"]["command"],
        json!("ait attest put RCP-1 --tests pass --remote mirror")
    );
    assert_eq!(remote.requests.len(), 1);
    assert_eq!(remote.ci_status_requests.len(), 0);
    assert_eq!(remote.ci_readiness_requests.len(), 1);
    assert_eq!(remote.repo_job_requests, 0);

    let mut authoritative_state = state.clone();
    authoritative_state["workspace"] = json!({
        "clean": false,
        "changed_count": 1,
        "changed_paths": ["error.log"],
        "current_line": "main",
        "head_snapshot_id": "SNP-UNLANDED-WORKSPACE",
        "workspace_status": "dirty",
        "workspace_matches_patchset": false
    });
    authoritative_state["ignore_workspace_authoring"] = json!(true);
    authoritative_state["patchset_is_authoritative"] = json!(true);

    let authoritative_refreshed = workflow_ready_ci_poll_payload_with_closeout_remote(
        &repo,
        &mut remote,
        "fixture-ait",
        &authoritative_state,
        "RCC-1",
        Some("mirror"),
    )
    .expect("refresh authoritative completed-local CI state");

    assert_eq!(
        authoritative_refreshed["workspace"],
        authoritative_state["workspace"]
    );
    assert_eq!(
        authoritative_refreshed["ignore_workspace_authoring"],
        json!(true)
    );
    assert_eq!(
        authoritative_refreshed["patchset_is_authoritative"],
        json!(true)
    );
    assert_eq!(
        authoritative_refreshed["next_action"]["code"],
        json!("record_attestation")
    );
    assert_ne!(
        authoritative_refreshed["next_action"]["code"],
        json!("snapshot_create")
    );

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
        Some("mirror"),
    )
    .expect_err("malformed readiness must fail closed");
    assert!(error.contains("missing non-empty contract"));
    assert_eq!(remote.ci_status_requests.len(), 0);
    assert_eq!(remote.ci_readiness_requests.len(), 3);
    assert_eq!(remote.repo_job_requests, 0);
}

#[test]
fn workflow_ready_ci_poll_rejects_partial_completed_local_authority() {
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
    let mut remote = FakeWorkflowCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-1".to_string(),
            json!({
                "patchset_id": "RCP-1",
                "change_id": "RCC-1",
                "base_snapshot_id": "SNP-BASE",
                "revision_snapshot_id": "SNP-REVISION",
            }),
        )]),
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
                "latest_job": {"job_type": "patchset.ci", "state": "succeeded"},
                "recent_jobs": []
            }),
        )]),
        ..Default::default()
    };
    let state = json!({
        "change": {"change_id": "RCC-1", "base_line": "main", "status": "active"},
        "task": {"task_id": "RCT-1", "status": "active"},
        "patchset": {"patchset_id": "RCP-1"},
        "workspace": {"clean": false},
        "base_line": {"line_name": "main", "head_snapshot_id": "SNP-BASE"},
        "freshness": {"base_is_fresh": true},
        "ignore_workspace_authoring": true,
        "patchset_is_authoritative": false,
        "next_action": {"code": "waiting_for_ci"}
    });

    let error = workflow_ready_ci_poll_payload_with_closeout_remote(
        &repo,
        &mut remote,
        "fixture-ait",
        &state,
        "RCC-1",
        None,
    )
    .expect_err("partial completed-local authority must fail closed");

    assert!(error.contains("inconsistent workspace and Patchset authority flags"));
}
