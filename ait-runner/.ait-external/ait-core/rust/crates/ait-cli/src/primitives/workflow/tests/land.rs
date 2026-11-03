use super::*;

#[test]
fn ai_only_code_review_uses_existing_policy_semantics() {
    let mut patchset = json!({
        "author_mode":"ai_only_experimental",
        "diff_stats":{"paths":{"modified":["src/lib.rs"]}}
    });
    assert!(workflow_requires_code_review_summary(
        Some(&patchset),
        None,
        None
    ));
    assert!(!workflow_requires_code_review_summary(
        Some(&patchset),
        None,
        Some(&json!({"effective_requirements":{"require_code_review_summary":false}})),
    ));
    patchset["author_mode"] = json!("human_only");
    assert!(!workflow_requires_code_review_summary(
        Some(&patchset),
        None,
        None
    ));
}

#[test]
fn review_lane_counts_keep_bot_and_user_actors_distinct_without_requiring_humans() {
    let summary = json!({"reviews":[
        {"patchset_id":"P-1","reviewer":"Einstein","reviewer_kind":"user","action":"task_approve"},
        {"patchset_id":"P-1","reviewer":"Einstein","reviewer_kind":"bot","action":"task_defer"},
        {"patchset_id":"P-1","reviewer":"Faraday","reviewer_kind":"bot","action":"task_approve"},
        {"patchset_id":"P-1","reviewer":"Legacy Human","action":"task_approve"},
        {"patchset_id":"P-1","reviewer":"Faraday","reviewer_kind":"bot","action":"approve"},
        {"patchset_id":"P-OLD","reviewer":"Other","reviewer_kind":"user","action":"task_approve"}
    ]});
    let counts = workflow_review_lane_counts(&summary, Some("P-1"));
    assert_eq!(counts["task_approvals"], 3);
    assert_eq!(counts["eligible_task_approvals"], 3);
    assert_eq!(counts["team_approvals"], 1);
    assert_eq!(counts["approvals"], 3);
    assert_eq!(counts["human_approvals"], 1);
    assert_eq!(counts["human_task_approvals"], 1);
    let bots = json!({"reviews":[{"patchset_id":"P-1","reviewer":"Faraday","reviewer_kind":"bot","action":"task_approve"}]});
    let counts = workflow_review_lane_counts(&bots, Some("P-1"));
    assert_eq!(counts["task_approvals"], 1);
    assert_eq!(counts["human_task_approvals"], 0);
    let unknown = json!({"reviews":[{"patchset_id":"P-1","reviewer":"Unknown","reviewer_kind":null,"action":"task_approve"}]});
    assert_eq!(
        workflow_review_lane_counts(&unknown, Some("P-1"))["human_approvals"],
        0
    );
}

#[test]
fn review_lane_human_counts_require_positive_kind_evidence_in_rows_or_summary() {
    for kind in [
        None,
        Some(JsonValue::Null),
        Some(json!("unknown")),
        Some(json!("bot")),
        Some(json!("service_account")),
        Some(json!("user")),
    ] {
        let mut row =
            json!({"patchset_id":"P-1","reviewer":"Legacy Human","action":"task_approve"});
        let human = i64::from(kind.as_ref().is_some_and(|kind| kind == "user"));
        if let Some(kind) = kind {
            row["reviewer_kind"] = kind;
        }
        let counts = workflow_review_lane_counts(&json!({"reviews":[row]}), Some("P-1"));
        assert_eq!(
            counts["task_approvals"], 1,
            "generic legacy approval remains applicable"
        );
        assert_eq!(counts["approvals"], 1);
        assert_eq!(counts["human_approvals"], human);
        assert_eq!(counts["human_task_approvals"], human);
        assert_eq!(counts["eligible_human_approvals"], human);
    }
    let mut summary = json!({"current_patchset_id":"P-1","task_approvals":1,"approvals":1});
    let counts = workflow_review_lane_counts(&summary, Some("P-1"));
    assert_eq!(counts["task_approvals"], 1);
    assert_eq!(counts["human_task_approvals"], 0);
    assert_eq!(counts["human_approvals"], 0);
    assert_eq!(counts["eligible_human_approvals"], 0);
    summary["human_task_approvals"] = json!(1);
    summary["human_approvals"] = json!(1);
    let counts = workflow_review_lane_counts(&summary, Some("P-1"));
    assert_eq!(counts["human_task_approvals"], 1);
    assert_eq!(counts["human_approvals"], 1);
}

#[test]
fn patchset_refresh_rejects_republishing_an_older_current_head() {
    let patchset = json!({"patchset_id": "RCT-1/C-01/P-02"});
    let refresh = workflow_patchset_refresh_context(
        Some(&patchset),
        None,
        "main",
        Some("SNP-BASE"),
        Some("SNP-BASE"),
        Some("SNP-SELECTED"),
        Some("SNP-OLDER"),
        true,
        Some(false),
        WorkflowPatchsetHeadRelation::PatchsetDescendsFromCurrent,
    )
    .expect("older current head requires recovery context");

    assert_eq!(
        refresh["reason_code"],
        json!("current_head_behind_patchset")
    );
    assert_eq!(refresh["republish_allowed"], json!(false));
    assert_eq!(
        refresh["head_relation"],
        json!("patchset_descends_from_current")
    );
    let detail = refresh["detail"].as_str().unwrap();
    assert!(detail.contains("Do not republish the older current head"));
    assert!(!detail.contains("newer head"));
}

#[test]
fn patchset_refresh_only_republishes_a_proven_newer_current_head() {
    let patchset = json!({"patchset_id": "RCT-1/C-01/P-02"});
    let refresh = workflow_patchset_refresh_context(
        Some(&patchset),
        None,
        "main",
        Some("SNP-BASE"),
        Some("SNP-BASE"),
        Some("SNP-SELECTED"),
        Some("SNP-NEWER"),
        true,
        Some(false),
        WorkflowPatchsetHeadRelation::CurrentDescendsFromPatchset,
    )
    .expect("proven newer current head can refresh");

    assert_eq!(refresh["reason_code"], json!("head_diverged_republish"));
    assert_eq!(refresh["republish_allowed"], json!(true));
    assert_eq!(
        refresh["head_relation"],
        json!("current_descends_from_patchset")
    );
}

#[test]
fn patchset_refresh_fails_closed_when_snapshot_relation_is_unknown() {
    let patchset = json!({"patchset_id": "RCT-1/C-01/P-02"});
    let refresh = workflow_patchset_refresh_context(
        Some(&patchset),
        None,
        "main",
        Some("SNP-BASE"),
        Some("SNP-BASE"),
        Some("SNP-SELECTED"),
        Some("SNP-UNKNOWN"),
        true,
        Some(false),
        WorkflowPatchsetHeadRelation::Unknown,
    )
    .expect("unknown relation requires recovery context");

    assert_eq!(refresh["reason_code"], json!("head_relation_unproven"));
    assert_eq!(refresh["republish_allowed"], json!(false));
    assert!(!refresh["detail"].as_str().unwrap().contains("newer head"));
}

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
        "patchset_id": "RT-0200/P-01",
        "task_id":"RT-0200","change_id":"C-01","change_ref":"RT-0200/C-01"
    });
    let mut remote = FakeWorkflowCloseoutRemote {
        patchsets: BTreeMap::from([("RT-0200/P-01".to_string(), patchset.clone())]),
        ci_statuses: BTreeMap::from([(
            "RT-0200/P-01".to_string(),
            json!({
                "contract": "ait.server.patchset_ci.readiness.v2",
                "projection": "readiness",
                "patchset_id": "RT-0200/P-01",
                "task_id":"RT-0200","change_id":"C-01","change_ref":"RT-0200/C-01",
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
        Some("RT-0200/P-01"),
    )
    .expect("bounded readiness");
    assert_eq!(readiness["projection"], json!("readiness"));
    assert_eq!(remote.ci_status_requests.len(), 0);
    assert_eq!(remote.ci_readiness_requests.len(), 1);
    assert_eq!(remote.repo_job_requests, 0);

    remote.ci_statuses.insert(
        "RT-0200/P-01".to_string(),
        json!({"patchset_id": "RT-0200/P-01", "tests_status": "pass"}),
    );
    assert!(workflow_land_patchset_ci_status_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        Some("RT-0200/P-01"),
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
        .expect("read workflow finish base line");
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
            .expect("read workflow finish policy");
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
                "verification_state":"pass","require_tests_pass":true,"require_human_review":false,"require_lint_pass":false,"revoked":false,"ci_backed":false
            }),
        )]),
        ..Default::default()
    };

    let attestation = workflow_land_attestation_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        Some("LCP-1"),
    )
    .expect("read workflow finish attestation")
    .expect("statement");
    assert_eq!(attestation["verification_state"], json!("pass"));

    let missing = workflow_land_attestation_read_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        Some("LCP-MISSING"),
    );
    assert!(missing.unwrap().is_none());

    let no_patchset =
        workflow_land_attestation_read_with_closeout_remote(&mut remote, "fixture-ait", None);
    assert!(no_patchset.unwrap().is_none());

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
            .expect("read workflow finish review summary");
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
    let task = "RT-0200";
    let change = "RT-0200/C-01";
    let id = "RT-0200/P-02";
    let candidate = json!({"task_id":task,"change_id":"C-01","change_ref":change,"patchset_id":id,"base_snapshot_id":"SNP-BASE","revision_snapshot_id":"SNP-REVISION","author_mode":"ai_with_human_review","ci":{"run_seq":1,"completed_at_s":1,"tests_status":"pass","lint_status":"none","suite_result_count":1,"blocking_failure_count":0}});
    let mut tasks = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            task.into(),
            json!({"task_id":task,"status":"active","successful_land":null,"current_patchset_id":id}),
        )]),
        changes: BTreeMap::from([(
            change.into(),
            json!({"task_id":task,"change_id":"C-01","change_ref":change,"status":"review","base_line":"main","selected_patchset_id":"RT-0200/P-01"}),
        )]),
        lines: BTreeMap::from([(
            "main".into(),
            json!({"line_name":"main","head_snapshot_id":"SNP-BASE"}),
        )]),
        ..Default::default()
    };
    let mut remote = FakeWorkflowRemoteStateReadRemote {
        patchsets: BTreeMap::from([(id.into(), candidate)]),
        reviews: BTreeMap::from([(
            task.into(),
            json!({"task_id":task,"reviews":[{"patchset_id":id,"reviewer":"alice","action":"task_approve","actor_type":"user"},{"patchset_id":id,"reviewer":"bot","action":"code_review_summary"}]}),
        )]),
        attestations: BTreeMap::from([(
            id.into(),
            json!({"task_id":task,"change_id":"C-01","change_ref":change,"patchset_id":id,"author_mode":"ai_with_human_review","verification_state":"pass","require_tests_pass":true,"require_lint_pass":false,"require_human_review":false,"revoked":false,"ci_backed":true,"evaluation_summary":{"tests":"pass","lint":"none"}}),
        )]),
        policies: BTreeMap::from([(
            id.into(),
            json!({"patchset_id":id,"decision":"pass","checks":[]}),
        )]),
        ..Default::default()
    };
    for authoritative in [false, true] {
        let state = workflow_land_remote_state_with_remotes(
            &mut tasks,
            &mut remote,
            "fixture-ait",
            Some(change),
            None,
            authoritative,
        )
        .unwrap();
        assert_eq!(state["landed"], false);
        assert_eq!(state["resolved_change_id"], "C-01");
        assert_eq!(state["resolved_change_ref"], change);
        assert_eq!(state["patchset_source"], "task_latest");
        assert_eq!(state["patchset"]["patchset_id"], id);
        assert_eq!(state["tests_state"], "pass");
        assert_eq!(state["policy_decision"], "pass");
        assert_eq!(
            state["patchset_ci_status"]["projection"],
            "embedded_patchset"
        );
        assert!(state["landing_summary"].is_null());
    }
    assert!(tasks.change_detail_requests.is_empty());
    assert!(remote.ci_status_requests.is_empty());
    assert_eq!(remote.review_requests.len(), 2);
    remote.patchsets.get_mut(id).unwrap()["ci"] =
        json!({"run_seq":2,"completed_at_s":0,"tests_status":"pending","suite_result_count":0});
    remote.ci_statuses.insert(id.into(),json!({"contract":"ait.server.patchset_ci.readiness.v2","projection":"readiness","repo_name":"fixture-ait","patchset_id":id,"task_id":task,"change_id":"C-01","change_ref":change,"available":true,"tests_status":"pending","ci_run_seq":2,"ci_completed_at_s":0,"selected_suite_ids":["rust_core"],"suite_result_count":0,"blocking_failure_count":0,"has_runnable_evidence":true,"recent_limit_applied":10,"latest_job":{"job_type":"patchset.ci","state":"running"},"recent_jobs":[]}));
    let pending = workflow_land_remote_state_with_remotes(
        &mut tasks,
        &mut remote,
        "fixture-ait",
        Some(change),
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        pending["patchset_ci_status"]["latest_job"]["state"],
        "running"
    );
    assert_eq!(remote.ci_status_requests.len(), 1);
    let error = workflow_land_remote_state_with_remotes(
        &mut tasks,
        &mut remote,
        "fixture-ait",
        Some(change),
        Some("RT-0200/P-03"),
        false,
    )
    .unwrap_err();
    assert!(error.contains("Unknown patchset"));
    remote.patchsets.get_mut(id).unwrap()["invalidated"] = json!(true);
    let error = workflow_land_remote_state_with_remotes(
        &mut tasks,
        &mut remote,
        "fixture-ait",
        Some(change),
        None,
        false,
    )
    .unwrap_err();
    assert!(error.contains("latest candidate is withdrawn or invalidated"));
}

fn accepted_fixture_receipt(task: &str, change: Option<&str>, id: &str) -> JsonValue {
    json!({"repository_index":7,"submission_id":format!("{task}/L-01"),"task_id":task,"change_id":change.map(|value|value.rsplit('/').next().unwrap()),"change_ref":change,"patchset_id":id,"base_snapshot_id":"B","revision_snapshot_id":"R","target_line":"main","target_line_index":0,"pre_land_target_snapshot_id":"B","landed_snapshot_id":"R","mode":"direct","status":"succeeded","created_at":"2026-10-18T00:00:00Z","updated_at":"2026-10-18T00:00:00Z"})
}

#[test]
fn workflow_land_remote_state_landed_change_short_circuits_closeout_reads() {
    let task = "RT-0202";
    let change = "RT-0202/C-01";
    let id = "RT-0202/P-01";
    let mut tasks = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            task.into(),
            json!({"task_id":task,"status":"completed","successful_land":accepted_fixture_receipt(task,Some(change),id),"current_patchset_id":"RT-0202/P-02"}),
        )]),
        changes: BTreeMap::from([(
            change.into(),
            json!({"task_id":task,"change_id":"C-01","change_ref":change,"status":"landed","base_line":"main"}),
        )]),
        ..Default::default()
    };
    let mut remote = FakeWorkflowRemoteStateReadRemote {
        patchsets: BTreeMap::from([(
            id.into(),
            json!({"task_id":task,"change_id":"C-01","change_ref":change,"patchset_id":id,"base_snapshot_id":"B","revision_snapshot_id":"R"}),
        )]),
        ..Default::default()
    };
    let state = workflow_land_remote_state_with_remotes(
        &mut tasks,
        &mut remote,
        "fixture-ait",
        Some(change),
        None,
        false,
    )
    .unwrap();
    assert_eq!(state["landed"], true);
    assert_eq!(state["patchset_source"], "accepted_task_receipt");
    assert_eq!(state["patchset"]["patchset_id"], id);
    assert_eq!(
        state["landing_summary"]["submission_id"],
        format!("{task}/L-01")
    );
    assert!(tasks.line_requests.is_empty());
    assert!(tasks.change_detail_requests.is_empty());
    assert_eq!(remote.requests.len(), 1);
    assert!(remote.review_requests.is_empty());
    assert!(remote.attestation_requests.is_empty());
    assert!(remote.policy_requests.is_empty());
    tasks.tasks.get_mut(task).unwrap()["successful_land"] = JsonValue::Null;
    let error = workflow_land_remote_state_with_remotes(
        &mut tasks,
        &mut remote,
        "fixture-ait",
        Some(change),
        None,
        false,
    )
    .unwrap_err();
    assert!(error.contains("no actual successful receipt"));
}

#[test]
fn workflow_land_uses_change_ref_to_isolate_duplicate_short_ids() {
    let task = "RT-2";
    let change = "RT-2/C-01";
    let id = "RT-2/P-01";
    let mut tasks = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            task.into(),
            json!({"task_id":task,"status":"completed","current_patchset_id":id,"successful_land":accepted_fixture_receipt(task,Some(change),id)}),
        )]),
        changes: BTreeMap::from([(
            change.into(),
            json!({"task_id":task,"change_id":"C-01","change_ref":change,"status":"landed"}),
        )]),
        ..Default::default()
    };
    let mut remote = FakeWorkflowRemoteStateReadRemote {
        patchsets: BTreeMap::from([
            (
                id.into(),
                json!({"task_id":task,"change_id":"C-01","change_ref":change,"patchset_id":id,"base_snapshot_id":"B","revision_snapshot_id":"R"}),
            ),
            (
                "RT-1/P-01".into(),
                json!({"task_id":"RT-1","change_id":"C-01","change_ref":"RT-1/C-01","patchset_id":"RT-1/P-01","base_snapshot_id":"B","revision_snapshot_id":"R"}),
            ),
        ]),
        ..Default::default()
    };
    let state = workflow_land_remote_state_with_remotes(
        &mut tasks,
        &mut remote,
        "fixture-ait",
        Some(change),
        None,
        false,
    )
    .unwrap();
    assert_eq!(state["resolved_change_id"], "C-01");
    assert_eq!(state["resolved_change_ref"], change);
    assert_eq!(tasks.change_requests[0].0, change);
    assert!(workflow_land_remote_state_with_remotes(
        &mut tasks,
        &mut remote,
        "fixture-ait",
        Some(change),
        Some("RT-1/P-01"),
        false
    )
    .is_err());
    assert!(remote.review_requests.is_empty());
    assert!(remote.attestation_requests.is_empty());
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
    .expect("record workflow finish task review");

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
    .expect("record workflow finish code review summary");

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

#[test]
fn aggregate_correction_refresh_preserves_ancestry_instead_of_ordinary_rebase() {
    let patchset =
        json!({"patchset_id":"RCT-1/C-01/P-02","source_kind":"history_promotion_aggregate"});
    let retarget =
        json!({"needs_retarget":true,"fork_snapshot_id":"SNP-SELECTED","rebase_state":"idle"});
    let result = workflow_patchset_refresh_context(
        Some(&patchset),
        Some(&retarget),
        "main",
        Some("SNP-BASE"),
        Some("SNP-BASE"),
        Some("SNP-SELECTED"),
        Some("SNP-CORRECTED"),
        true,
        Some(false),
        WorkflowPatchsetHeadRelation::CurrentDescendsFromPatchset,
    )
    .unwrap();
    assert_eq!(result["reason_code"], "aggregate_correction");
    assert_eq!(result["republish_allowed"], true);
    assert_eq!(result["rebase_required"], false);
    let stale = workflow_patchset_refresh_context(
        Some(&patchset),
        Some(&retarget),
        "main",
        Some("SNP-MOVED"),
        Some("SNP-BASE"),
        Some("SNP-SELECTED"),
        Some("SNP-CORRECTED"),
        false,
        Some(false),
        WorkflowPatchsetHeadRelation::CurrentDescendsFromPatchset,
    )
    .unwrap();
    assert_eq!(stale["republish_allowed"], false);
    let foreign = workflow_patchset_refresh_context(
        Some(&patchset),
        None,
        "main",
        Some("SNP-BASE"),
        Some("SNP-BASE"),
        Some("SNP-SELECTED"),
        Some("SNP-FOREIGN"),
        true,
        Some(false),
        WorkflowPatchsetHeadRelation::Diverged,
    )
    .unwrap();
    assert_eq!(foreign["republish_allowed"], false);
}

#[test]
fn aggregate_correction_receipt_encoding_matches_server_digest_bytes() {
    // Receipt hashes cross the CLI/server boundary. Preserve compact serde bytes,
    // including object key order, escaping, Unicode and integer timestamps.
    let entries = json!([{
        "local_task_id": "LCT-1086",
        "local_change_id": "C-01",
        "base_snapshot_id": "SNP-BASE",
        "revision_snapshot_id": "SNP-RECEIPT",
        "summary": "修正\n\"review\"\\receipt",
        "snapshots": [{"snapshot_id":"SNP-RECEIPT","created_at_s":1792052112_i64}]
    }]);
    let cli_bytes = crate::json_support::encode_value_to_vec(&entries, "receipt").unwrap();
    let server_bytes = serde_json::to_vec(&entries).unwrap();
    assert_eq!(cli_bytes, server_bytes);
    assert_eq!(
        sha256_hex_bytes(&cli_bytes),
        sha256_hex_bytes(&server_bytes)
    );
    assert_eq!(
        crate::json_support::parse_value(std::str::from_utf8(&cli_bytes).unwrap(), "receipt")
            .unwrap(),
        entries
    );
}

#[test]
fn aggregate_correction_command_hints_use_guarded_ready_in_retargeted_worktree() {
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
    let patchset = json!({"patchset_id":"RCT-1/C-01/P-02",
        "source_kind":"history_promotion_aggregate"});
    let retarget = json!({"needs_retarget":true,"rebase_state":"idle"});
    let hints = workflow_ready_command_hints(
        &repo,
        "RCT-1",
        Some("origin"),
        Some(&patchset),
        "main",
        Some(&retarget),
    );
    assert_eq!(
        hints["publish_command"],
        "ait workflow ready RCT-1 --apply --remote origin"
    );
    let normal = workflow_ready_command_hints(
        &repo,
        "RCT-1",
        Some("origin"),
        None,
        "main",
        Some(&retarget),
    );
    assert_eq!(normal["publish_command"], "ait worktree rebase --onto main");
}

#[test]
fn aggregate_correction_rejects_inferred_change_ownership_and_foreign_evidence() {
    use super::super::local_completion::workflow_aggregate_correction_owner_matches;
    use crate::primitives::workspace::bound_worktree_snapshot_ownership_rows_with_snapshot_store;
    use crate::primitives::worktree::create_local_line_with_line_store;
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
    fs::write(temp.path().join("source.txt"), "original").unwrap();
    let base = create_local_snapshot(
        temp.path().to_str().unwrap(),
        "fixture-ait",
        "main",
        Some("base"),
        false,
    )
    .unwrap();
    let base_id = required_string_field(&base, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let line = "feature/rct-1";
    create_local_line_with_line_store(
        &repo.line_store().unwrap(),
        line,
        Some(&base_id),
        "2026-10-15T00:00:00Z",
    )
    .unwrap();
    fs::write(temp.path().join("source.txt"), "correction").unwrap();
    let snapshot = create_local_snapshot(
        temp.path().to_str().unwrap(),
        "fixture-ait",
        line,
        Some("correction"),
        false,
    )
    .unwrap();
    let id = required_string_field(&snapshot, "snapshot_id").unwrap();
    let metadata = CurrentWorktreeMetadata {
        name: "rct-1".to_string(),
        bound_task_id: Some("RCT-1".to_string()),
        bound_change_id: Some("C-01".to_string()),
        bound_change_ref: Some("RCT-1/C-01".to_string()),
        fork_snapshot_id: Some(base_id),
        ..Default::default()
    };
    let rows = bound_worktree_snapshot_ownership_rows_with_snapshot_store(
        &repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
                &repo.workspace_root(),
            )
            .unwrap(),
        &[id],
        &metadata,
        line,
    )
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["change_id"], JsonValue::Null);
    let matches = |owner: &JsonValue| {
        workflow_aggregate_correction_owner_matches(
            owner,
            "RCT-1",
            Some("RCT-1/C-01"),
            "rct-1",
            line,
        )
    };
    assert!(!matches(&rows[0]));
    for (field, value) in [
        ("task_id", "RCT-2"),
        ("change_id", "C-02"),
        ("change_id", "RCT-2/C-01"),
        ("worktree_name", "rct-2"),
        ("line_name", "feature/rct-2"),
        ("ownership_source", "durable_snapshot_binding"),
    ] {
        let mut foreign = rows[0].clone();
        foreign[field] = json!(value);
        assert!(!matches(&foreign), "accepted foreign {field}={value}");
    }
    for change in ["C-01", "RCT-1/C-01"] {
        let mut durable = rows[0].clone();
        durable["change_id"] = json!(change);
        durable["ownership_source"] = json!("durable_snapshot_binding");
        assert!(matches(&durable));
    }
}
