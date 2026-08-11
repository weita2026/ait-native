use super::*;

#[test]
fn ready_task_land_workspace_context_does_not_invoke_full_status_reader() {
    let temp = tempdir().unwrap();
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let status_reader_called = std::cell::Cell::new(false);

    let (workspace, line_name, revision_snapshot_id) =
        workflow_land_workspace_context_with_status_reader(&repo, true, |_| {
            status_reader_called.set(true);
            Err("full workspace status must not run".to_string())
        })
        .expect("ready task land needs only line metadata");

    assert!(!status_reader_called.get());
    assert_eq!(line_name, "main");
    assert_eq!(revision_snapshot_id, None);
    assert_eq!(workspace["evaluation"], json!("skipped"));
    assert_eq!(
        workspace["read_scope"],
        json!("line_and_bound_worktree_metadata_only")
    );
}

#[test]
fn workflow_land_patchset_read_accepts_closeout_remote_trait() {
    let mut remote = FakeWorkflowCloseoutRemote {
        patchsets: BTreeMap::from([(
            "RCP-1".to_string(),
            json!({
                "patchset_id": "RCP-1",
                "change_id": "RCC-1",
                "revision_snapshot_id": "SNP-REVISION"
            }),
        )]),
        ..Default::default()
    };

    let explicit =
        workflow_land_patchset_read_with_closeout_remote(&mut remote, "fixture-ait", "RCP-1", None)
            .expect("read explicit patchset");
    assert_eq!(explicit["revision_snapshot_id"], json!("SNP-REVISION"));

    let selected = workflow_land_patchset_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        "RCP-1",
        Some("RCC-1"),
    )
    .expect("read selected patchset");
    assert_eq!(selected["change_id"], json!("RCC-1"));

    let err = workflow_land_patchset_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        "RCP-MISSING",
        Some("RCC-1"),
    )
    .expect_err("missing patchset should fail");
    assert!(err.contains("Unknown patchset"));
    assert_eq!(
        remote.requests,
        vec![
            ("RCP-1".to_string(), Some("fixture-ait".to_string()), None),
            (
                "RCP-1".to_string(),
                Some("fixture-ait".to_string()),
                Some("RCC-1".to_string())
            ),
            (
                "RCP-MISSING".to_string(),
                Some("fixture-ait".to_string()),
                Some("RCC-1".to_string())
            )
        ]
    );
}

#[test]
fn workflow_land_ci_read_uses_bounded_readiness_and_fails_closed() {
    let patchset = json!({
        "patchset_id": "RCP-1",
        "change_id": "RCC-1"
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
                "selected_suite_ids": [],
                "suite_result_count": 0,
                "blocking_failure_count": 0,
                "has_runnable_evidence": true,
                "recent_limit_applied": 10,
                "latest_job": {"job_type": "patchset.ci", "state": "succeeded"},
                "recent_jobs": []
            }),
        )]),
        ..Default::default()
    };

    let readiness = workflow_land_patchset_ci_status_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        Some("RCP-1"),
    )
    .expect("bounded readiness");
    assert_eq!(readiness["projection"], json!("readiness"));
    assert_eq!(remote.ci_status_requests.len(), 0);
    assert_eq!(remote.ci_readiness_requests.len(), 1);
    assert_eq!(remote.repo_job_requests, 0);

    remote.ci_statuses.insert(
        "RCP-1".to_string(),
        json!({"patchset_id": "RCP-1", "tests_status": "pass"}),
    );
    assert!(workflow_land_patchset_ci_status_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        Some("RCP-1"),
    )
    .is_none());
    assert_eq!(remote.repo_job_requests, 0);
}

#[test]
fn workflow_land_change_task_read_accepts_change_and_task_record_remote_traits() {
    let mut remote = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            "RCT-1".to_string(),
            json!({
                "task_id": "RCT-1",
                "title": "fixture task"
            }),
        )]),
        changes: BTreeMap::from([
            (
                "RCC-1".to_string(),
                json!({
                    "change_id": "RCC-1",
                    "task_id": "RCT-1",
                    "status": "active"
                }),
            ),
            (
                "RCC-MISSING-TASK".to_string(),
                json!({
                    "change_id": "RCC-MISSING-TASK",
                    "task_id": "RCT-MISSING",
                    "status": "active"
                }),
            ),
        ]),
        ..Default::default()
    };

    let (change, task) =
        workflow_land_change_task_read_with_task_remote(&mut remote, "fixture-ait", "RCC-1")
            .expect("read land change and task");
    assert_eq!(change["change_id"], json!("RCC-1"));
    assert_eq!(task["task_id"], json!("RCT-1"));

    let err =
        workflow_land_change_task_read_with_task_remote(&mut remote, "fixture-ait", "RCC-MISSING")
            .expect_err("missing change should fail");
    assert!(err.contains("Unknown change"));

    let err = workflow_land_change_task_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "RCC-MISSING-TASK",
    )
    .expect_err("missing task should fail");
    assert!(err.contains("Unknown task"));
}

#[test]
fn workflow_land_base_line_read_accepts_line_remote_trait() {
    let mut remote = FakeLineRemote {
        lines: BTreeMap::from([(
            "main".to_string(),
            json!({
                "line_name": "main",
                "head_snapshot_id": "SNP-MAIN"
            }),
        )]),
        ..Default::default()
    };

    let line = workflow_land_base_line_read_with_task_remote(&mut remote, "fixture-ait", "main")
        .expect("read workflow land base line");
    assert_eq!(line["head_snapshot_id"], json!("SNP-MAIN"));

    let err = workflow_land_base_line_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/missing",
    )
    .expect_err("missing base line should fail");
    assert!(err.contains("Unknown line"));

    assert_eq!(
        remote.line_requests,
        vec![
            ("fixture-ait".to_string(), "main".to_string()),
            ("fixture-ait".to_string(), "feature/missing".to_string())
        ]
    );
}

#[test]
fn workflow_land_change_detail_read_accepts_change_remote_trait() {
    let mut remote = FakeChangeRemote {
        change_details: BTreeMap::from([(
            "LCC-1".to_string(),
            json!({
                "change_id": "LCC-1",
                "landing_summary": {
                    "landed_snapshot_id": "SNP-LANDED"
                }
            }),
        )]),
        ..Default::default()
    };

    let detail =
        workflow_land_change_detail_read_with_task_remote(&mut remote, "fixture-ait", "LCC-1");
    assert_eq!(
        detail["landing_summary"]["landed_snapshot_id"],
        json!("SNP-LANDED")
    );

    let missing = workflow_land_change_detail_read_with_task_remote(
        &mut remote,
        "fixture-ait",
        "LCC-MISSING",
    );
    assert_eq!(missing, json!({}));

    assert_eq!(
        remote.change_detail_requests,
        vec![
            ("LCC-1".to_string(), Some("fixture-ait".to_string())),
            ("LCC-MISSING".to_string(), Some("fixture-ait".to_string()))
        ]
    );
}

#[test]
fn workflow_land_policy_read_accepts_closeout_remote_trait() {
    let mut remote = FakeWorkflowCloseoutRemote {
        policies: BTreeMap::from([(
            "LCP-1".to_string(),
            json!({
                "patchset_id": "LCP-1",
                "decision": "pass"
            }),
        )]),
        ..Default::default()
    };

    let policy =
        workflow_land_policy_read_with_closeout_remote(&mut remote, "fixture-ait", Some("LCP-1"))
            .expect("read workflow land policy");
    assert_eq!(policy["decision"], json!("pass"));

    let missing = workflow_land_policy_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        Some("LCP-MISSING"),
    );
    assert!(missing.is_none());

    let no_patchset =
        workflow_land_policy_read_with_closeout_remote(&mut remote, "fixture-ait", None);
    assert!(no_patchset.is_none());

    assert_eq!(
        remote.policy_requests,
        vec![
            ("LCP-1".to_string(), Some("fixture-ait".to_string()), true),
            (
                "LCP-MISSING".to_string(),
                Some("fixture-ait".to_string()),
                true
            )
        ]
    );
}

#[test]
fn workflow_land_attestation_read_accepts_closeout_remote_trait() {
    let mut remote = FakeWorkflowCloseoutRemote {
        attestations: BTreeMap::from([(
            "LCP-1".to_string(),
            json!({
                "patchset_id": "LCP-1",
                "tests_status": "pass"
            }),
        )]),
        ..Default::default()
    };

    let attestation = workflow_land_attestation_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        Some("LCP-1"),
    )
    .expect("read workflow land attestation");
    assert_eq!(attestation["tests_status"], json!("pass"));

    let missing = workflow_land_attestation_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        Some("LCP-MISSING"),
    );
    assert!(missing.is_none());

    let no_patchset =
        workflow_land_attestation_read_with_closeout_remote(&mut remote, "fixture-ait", None);
    assert!(no_patchset.is_none());

    assert_eq!(
        remote.attestation_requests,
        vec![
            ("LCP-1".to_string(), Some("fixture-ait".to_string()), true),
            (
                "LCP-MISSING".to_string(),
                Some("fixture-ait".to_string()),
                true
            )
        ]
    );
}

#[test]
fn workflow_land_review_summary_read_accepts_closeout_remote_trait() {
    let mut remote = FakeWorkflowCloseoutRemote {
        reviews: BTreeMap::from([(
            "LCC-1".to_string(),
            json!({
                "change_id": "LCC-1",
                "review_state": "approved"
            }),
        )]),
        ..Default::default()
    };

    let review_summary =
        workflow_land_review_summary_read_with_closeout_remote(&mut remote, "fixture-ait", "LCC-1")
            .expect("read workflow land review summary");
    assert_eq!(review_summary["review_state"], json!("approved"));

    let err = workflow_land_review_summary_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        "LCC-MISSING",
    )
    .expect_err("missing review summary should fail");
    assert!(err.contains("Unknown review summary"));

    assert_eq!(
        remote.review_requests,
        vec![
            ("LCC-1".to_string(), Some("fixture-ait".to_string()), true),
            (
                "LCC-MISSING".to_string(),
                Some("fixture-ait".to_string()),
                true
            )
        ]
    );
}

#[test]
fn workflow_land_remote_state_accepts_read_capability_traits() {
    let mut task_remote = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            "RCT-REMOTE".to_string(),
            json!({
                "task_id": "RCT-REMOTE",
                "title": "remote hydration"
            }),
        )]),
        changes: BTreeMap::from([(
            "RCC-REMOTE".to_string(),
            json!({
                "change_id": "RCC-REMOTE",
                "task_id": "RCT-REMOTE",
                "status": "ready",
                "base_line": "main",
                "selected_patchset_id": "RCP-REMOTE-2"
            }),
        )]),
        lines: BTreeMap::from([(
            "main".to_string(),
            json!({
                "line_name": "main",
                "head_snapshot_id": "SNP-BASE"
            }),
        )]),
        change_details: BTreeMap::from([(
            "RCC-REMOTE".to_string(),
            json!({
                "change_id": "RCC-REMOTE",
                "landing_summary": {
                    "status": "blocked",
                    "patchset_id": "RCP-REMOTE-2",
                    "result": {
                        "blocker_class": "POLICY_BLOCKED"
                    }
                }
            }),
        )]),
        ..Default::default()
    };
    let mut read_remote = FakeWorkflowRemoteStateReadRemote {
        patchsets: BTreeMap::from([(
            "RCP-REMOTE-2".to_string(),
            json!({
                "patchset_id": "RCP-REMOTE-2",
                "change_id": "RCC-REMOTE",
                "base_snapshot_id": "SNP-BASE",
                "revision_snapshot_id": "SNP-REVISION",
                "author_mode": "ai_with_human_review",
                "ci_run_seq": 1,
                "ci_completed_at_s": 1_783_814_400_u64,
                "ci": {
                    "run_seq": 1,
                    "completed_at_s": 1_783_814_400_u64,
                    "overall_status": "pass",
                    "tests_status": "pass",
                    "lint_status": "none",
                    "selected_suite_count": 1,
                    "suite_result_count": 1,
                    "blocking_failure_count": 0
                }
            }),
        )]),
        reviews: BTreeMap::from([(
            "RCC-REMOTE".to_string(),
            json!({
                "change_id": "RCC-REMOTE",
                "reviews": [
                    {
                        "patchset_id": "RCP-REMOTE-2",
                        "reviewer": "alice",
                        "action": "approve"
                    },
                    {
                        "patchset_id": "RCP-REMOTE-2",
                        "reviewer": "bot",
                        "action": "code_review_summary",
                        "comment": "Summary\n\nTesting\n\nRisks\n\nNotes"
                    }
                ]
            }),
        )]),
        attestations: BTreeMap::from([(
            "RCP-REMOTE-2".to_string(),
            json!({
                "patchset_id": "RCP-REMOTE-2",
                "author_mode": "ai_with_human_review",
                "evaluation_summary": {
                    "tests": "pass"
                }
            }),
        )]),
        policies: BTreeMap::from([(
            "RCP-REMOTE-2".to_string(),
            json!({
                "patchset_id": "RCP-REMOTE-2",
                "decision": "pass",
                "checks": [
                    {
                        "name": "tests",
                        "status": "pass"
                    }
                ]
            }),
        )]),
        ..Default::default()
    };

    let state = workflow_land_remote_state_with_remotes(
        &mut task_remote,
        &mut read_remote,
        "fixture-ait",
        Some("RCC-REMOTE"),
        None,
        false,
    )
    .expect("hydrate workflow land remote state");

    assert_eq!(state["landed"], json!(false));
    assert_eq!(state["resolved_change_id"], json!("RCC-REMOTE"));
    assert_eq!(state["patchset_source"], json!("selected"));
    assert_eq!(state["patchset"]["patchset_id"], json!("RCP-REMOTE-2"));
    assert_eq!(state["remote_base_snapshot_id"], json!("SNP-BASE"));
    assert_eq!(state["tests_state"], json!("pass"));
    assert_eq!(state["policy_decision"], json!("pass"));
    assert_eq!(state["review_approvals"], json!(1));
    assert_eq!(state["team_review_approvals"], json!(1));
    assert_eq!(state["landing_status"], json!("blocked"));
    assert_eq!(state["stale_policy_blocker_cleared"], json!(true));
    assert_eq!(state["patchset_ci_status"]["tests_status"], json!("pass"));
    assert_eq!(
        state["patchset_ci_status"]["projection"],
        json!("embedded_patchset")
    );
    assert_eq!(task_remote.line_requests.len(), 1);
    assert_eq!(task_remote.change_detail_requests.len(), 1);
    assert_eq!(read_remote.review_requests.len(), 1);
    assert_eq!(read_remote.attestation_requests.len(), 1);
    assert_eq!(read_remote.policy_requests.len(), 1);
    assert!(read_remote.ci_status_requests.is_empty());

    task_remote.change_detail_requests.clear();
    read_remote.ci_status_requests.clear();
    let authoritative_state = workflow_land_remote_state_with_remotes(
        &mut task_remote,
        &mut read_remote,
        "fixture-ait",
        Some("RCC-REMOTE"),
        None,
        true,
    )
    .expect("hydrate ready task-land state from persisted evidence");
    assert_eq!(authoritative_state["tests_state"], json!("pass"));
    assert_eq!(authoritative_state["policy_decision"], json!("pass"));
    assert_eq!(
        authoritative_state["patchset_ci_status"]["tests_status"],
        json!("pass")
    );
    assert!(task_remote.change_detail_requests.is_empty());
    assert!(read_remote.ci_status_requests.is_empty());

    let pending_patchset = read_remote
        .patchsets
        .get_mut("RCP-REMOTE-2")
        .expect("pending patchset fixture");
    pending_patchset["ci_run_seq"] = json!(2);
    pending_patchset["ci_completed_at_s"] = json!(0);
    pending_patchset["ci"] = json!({
        "run_seq": 2,
        "completed_at_s": 0,
        "overall_status": "pending",
        "tests_status": "pending",
        "lint_status": "none",
        "selected_suite_count": 1,
        "suite_result_count": 0,
        "blocking_failure_count": 0
    });
    read_remote.ci_statuses.insert(
        "RCP-REMOTE-2".to_string(),
        json!({
            "contract": "ait.server.patchset_ci.readiness.v1",
            "projection": "readiness",
            "patchset_id": "RCP-REMOTE-2",
            "change_id": "RCC-REMOTE",
            "repo_name": "fixture-ait",
            "available": true,
            "ci_run_seq": 2,
            "ci_completed_at_s": 0,
            "tests_status": "pending",
            "selected_suite_ids": ["rust_core"],
            "suite_result_count": 0,
            "blocking_failure_count": 0,
            "has_runnable_evidence": true,
            "recent_limit_applied": 10,
            "latest_job": {
                "job_type": "patchset.ci",
                "state": "running"
            },
            "recent_jobs": []
        }),
    );
    let pending_state = workflow_land_remote_state_with_remotes(
        &mut task_remote,
        &mut read_remote,
        "fixture-ait",
        Some("RCC-REMOTE"),
        None,
        false,
    )
    .expect("hydrate incomplete CI from Worker Job readiness");
    assert_eq!(
        pending_state["patchset_ci_status"]["latest_job"]["state"],
        json!("running")
    );
    assert_eq!(read_remote.ci_status_requests.len(), 1);

    let err = workflow_land_remote_state_with_remotes(
        &mut task_remote,
        &mut read_remote,
        "fixture-ait",
        Some("RCC-REMOTE"),
        Some("RCP-MISSING"),
        false,
    )
    .expect_err("missing explicit patchset should fail");
    assert!(err.contains("Unknown patchset"));
}

#[test]
fn workflow_land_remote_state_landed_change_short_circuits_closeout_reads() {
    let mut task_remote = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            "RCT-LANDED".to_string(),
            json!({
                "task_id": "RCT-LANDED",
                "title": "landed task"
            }),
        )]),
        changes: BTreeMap::from([(
            "RCC-LANDED".to_string(),
            json!({
                "change_id": "RCC-LANDED",
                "task_id": "RCT-LANDED",
                "status": "landed",
                "base_line": "main",
                "selected_patchset_id": "RCP-LANDED-1"
            }),
        )]),
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkflowRemoteStateReadRemote::default();

    let state = workflow_land_remote_state_with_remotes(
        &mut task_remote,
        &mut closeout_remote,
        "fixture-ait",
        Some("RCC-LANDED"),
        None,
        false,
    )
    .expect("hydrate already-landed workflow land remote state");

    assert_eq!(state["landed"], json!(true));
    assert_eq!(state["resolved_change_id"], json!("RCC-LANDED"));
    assert_eq!(state["patchset_source"], json!("selected"));
    assert_eq!(state["patchset"]["patchset_id"], json!("RCP-LANDED-1"));
    assert_eq!(state["base_line_name"], json!("main"));
    assert!(task_remote.line_requests.is_empty());
    assert!(task_remote.change_detail_requests.is_empty());
    assert!(closeout_remote.requests.is_empty());
    assert!(closeout_remote.review_requests.is_empty());
    assert!(closeout_remote.attestation_requests.is_empty());
    assert!(closeout_remote.policy_requests.is_empty());
}

#[test]
fn workflow_land_uses_change_ref_to_isolate_duplicate_short_ids() {
    let mut task_remote = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            "RT-2".to_string(),
            json!({"task_id": "RT-2", "title": "second task"}),
        )]),
        changes: BTreeMap::from([(
            "RT-2/C-01".to_string(),
            json!({
                "change_id": "C-01",
                "change_ref": "RT-2/C-01",
                "task_id": "RT-2",
                "status": "landed",
                "base_line": "main"
            }),
        )]),
        ..Default::default()
    };
    let mut closeout_remote = FakeWorkflowRemoteStateReadRemote {
        patchsets: BTreeMap::from([(
            "RP-WRONG".to_string(),
            json!({
                "patchset_id": "RP-WRONG",
                "change_id": "RT-1/C-01"
            }),
        )]),
        ..Default::default()
    };

    let state = workflow_land_remote_state_with_remotes(
        &mut task_remote,
        &mut closeout_remote,
        "fixture-ait",
        Some("RT-2/C-01"),
        None,
        false,
    )
    .expect("explicit ref routes the intended task-local change");
    assert_eq!(state["resolved_change_id"], json!("C-01"));
    assert_eq!(state["resolved_change_ref"], json!("RT-2/C-01"));
    assert_eq!(task_remote.change_requests[0].0, "RT-2/C-01");

    let error = workflow_land_remote_state_with_remotes(
        &mut task_remote,
        &mut closeout_remote,
        "fixture-ait",
        Some("RT-2/C-01"),
        Some("RP-WRONG"),
        false,
    )
    .expect_err("same short id from another task must not match");
    assert!(error.contains("does not belong to change RT-2/C-01"));
}

#[test]
fn workflow_land_record_task_review_accepts_patchset_review_and_policy_traits() {
    let mut remote = FakeWorkflowReviewActionRemote {
        patchsets: BTreeMap::from([(
            "RCP-REVIEW-1".to_string(),
            json!({
                "patchset_id": "RCP-REVIEW-1"
            }),
        )]),
        ..Default::default()
    };
    let patchset = json!({
        "patchset_id": "RCP-REVIEW-1"
    });

    let payload = workflow_land_record_task_review_with_closeout_remote(
        &mut remote,
        &patchset,
        "RCC-REVIEW",
        "alice",
        "fixture-ait",
    )
    .expect("record workflow land task review");

    assert_eq!(payload["result"]["patchset_id"], json!("RCP-REVIEW-1"));
    assert_eq!(payload["result"]["reviewer"], json!("alice"));
    assert_eq!(payload["result"]["action"], json!("task_approve"));
    assert_eq!(
        payload["result"]["policy_refresh"]["decision"],
        json!("pass")
    );
    assert_eq!(
        remote.patchset_requests,
        vec![(
            "RCP-REVIEW-1".to_string(),
            Some("fixture-ait".to_string()),
            None
        )]
    );
    assert_eq!(remote.recorded_reviews.len(), 1);
    assert_eq!(
        remote.policy_evaluations,
        vec![(
            "RCP-REVIEW-1".to_string(),
            Some("fixture-ait".to_string()),
            false
        )]
    );
}

#[test]
fn workflow_land_record_code_review_summary_accepts_patchset_and_review_traits() {
    let mut remote = FakeWorkflowReviewOnlyActionRemote {
        patchsets: BTreeMap::from([(
            "RCP-REVIEW-2".to_string(),
            json!({
                "patchset_id": "RCP-REVIEW-2"
            }),
        )]),
        ..Default::default()
    };
    let patchset = json!({
        "patchset_id": "RCP-REVIEW-2"
    });
    let review_message = "Reviewed files\n\nrust/crates/ait-cli/src/primitives/workflow.rs\n\nFindings\n\nNo issues found.\n\nRisks\n\nLow.\n\nTests\n\ncargo test -p ait-cli.\n\nRecommendation\n\nLand.";

    let payload = workflow_land_record_code_review_summary_with_closeout_remote(
        &mut remote,
        &patchset,
        "RCC-REVIEW",
        "review-bot",
        review_message,
        "fixture-ait",
    )
    .expect("record workflow land code review summary");

    assert_eq!(payload["result"]["patchset_id"], json!("RCP-REVIEW-2"));
    assert_eq!(payload["result"]["reviewer"], json!("review-bot"));
    assert_eq!(payload["result"]["action"], json!("code_review_summary"));
    assert_eq!(payload["result"]["comment"], json!(review_message));
    assert!(payload["result"].get("policy_refresh").is_none());
    assert_eq!(
        remote.patchset_requests,
        vec![(
            "RCP-REVIEW-2".to_string(),
            Some("fixture-ait".to_string()),
            None
        )]
    );
    assert_eq!(remote.recorded_reviews.len(), 1);

    let invalid_message = "Reviewed files\n\nrust/crates/ait-cli/src/primitives/workflow.rs\n\nFindings\n\nTBD\n\nRisks\n\nLow.\n\nTests\n\ncargo test -p ait-cli.\n\nRecommendation\n\nLand.";
    let err = workflow_land_record_code_review_summary_with_closeout_remote(
        &mut remote,
        &patchset,
        "RCC-REVIEW",
        "review-bot",
        invalid_message,
        "fixture-ait",
    )
    .expect_err("placeholder code review summary should be rejected");
    assert!(err.contains("Findings"));
    assert_eq!(remote.recorded_reviews.len(), 1);
}
