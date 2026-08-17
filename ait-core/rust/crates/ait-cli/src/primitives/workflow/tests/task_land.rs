use super::*;
use crate::primitives::change_flow::change_local_create_with_change_store;
use crate::primitives::task::task_local_create_with_task_store;
use crate::primitives::workflow::task_land::task_land_apply_local;
use crate::task_land_contract::{attach_task_land_contract, task_land_exit_code};

#[test]
fn workflow_land_preserves_reviewer_actions_before_atomic_task_land_history() {
    let output = json!({
        "applied_actions": [{"code": "submit_land", "delivery": "atomic_task_land"}],
        "mutation_receipts": [{"action": "submit_land", "delivery": "atomic_response"}],
    });
    let merged = workflow_land_attach_atomic_task_land_history(
        output,
        vec![
            json!({"code": "record_code_review_summary"}),
            json!({"code": "record_review"}),
            json!({"code": "evaluate_policy"}),
        ],
        vec![json!({"action": "record_review"})],
    )
    .expect("merge reviewer and atomic history");

    let action_codes = merged["applied_actions"]
        .as_array()
        .expect("applied actions")
        .iter()
        .map(|row| row["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        action_codes,
        vec![
            "record_code_review_summary",
            "record_review",
            "evaluate_policy",
            "submit_land"
        ]
    );
    assert_eq!(
        merged["reviewer_workflow"]["reviewer_action_count"].as_u64(),
        Some(3)
    );
    assert_eq!(
        merged["reviewer_workflow"]["finalizer"].as_str(),
        Some("task-land-atomic/v1")
    );
}

#[test]
fn task_land_remote_change_id_accepts_change_and_task_record_remote_traits() {
    let mut remote = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            "RCT-1".to_string(),
            json!({
                "task_id": "RCT-1"
            }),
        )]),
        changes: BTreeMap::from([(
            "RCC-1".to_string(),
            json!({
                "change_id": "RCC-1",
                "task_id": "RCT-1",
                "status": "active"
            }),
        )]),
        change_rows: vec![json!({
            "change_id": "RCC-1",
            "task_id": "RCT-1",
            "status": "active",
            "created_at": "2026-01-01T00:00:00Z"
        })],
        ..Default::default()
    };

    assert_eq!(
        task_land_remote_change_id_with_task_remote(&mut remote, "fixture-ait", "RCC-1")
            .expect("change lookup"),
        Some("RCC-1".to_string())
    );
    assert_eq!(
        task_land_remote_change_id_with_task_remote(&mut remote, "fixture-ait", "RCT-1")
            .expect("task lookup"),
        Some("RCC-1".to_string())
    );

    let mut ambiguous_remote = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            "RCT-2".to_string(),
            json!({
                "task_id": "RCT-2"
            }),
        )]),
        change_rows: vec![
            json!({
                "change_id": "RCC-2",
                "task_id": "RCT-2",
                "status": "active",
                "created_at": "2026-01-01T00:00:00Z"
            }),
            json!({
                "change_id": "RCC-3",
                "task_id": "RCT-2",
                "status": "draft",
                "created_at": "2026-01-02T00:00:00Z"
            }),
        ],
        ..Default::default()
    };

    let err =
        task_land_remote_change_id_with_task_remote(&mut ambiguous_remote, "fixture-ait", "RCT-2")
            .expect_err("ambiguous task changes");
    assert!(err.contains("multiple landable changes"));
}

#[test]
fn task_land_keeps_change_ref_when_short_ids_repeat_across_tasks() {
    let mut remote = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([
            ("RT-1".to_string(), json!({"task_id": "RT-1"})),
            ("RT-2".to_string(), json!({"task_id": "RT-2"})),
        ]),
        changes: BTreeMap::from([
            (
                "RT-1/C-01".to_string(),
                json!({
                    "change_id": "C-01",
                    "change_ref": "RT-1/C-01",
                    "task_id": "RT-1",
                    "status": "active"
                }),
            ),
            (
                "RT-2/C-01".to_string(),
                json!({
                    "change_id": "C-01",
                    "change_ref": "RT-2/C-01",
                    "task_id": "RT-2",
                    "status": "active"
                }),
            ),
        ]),
        change_rows: vec![
            json!({
                "change_id": "C-01",
                "change_ref": "RT-1/C-01",
                "task_id": "RT-1",
                "status": "active"
            }),
            json!({
                "change_id": "C-01",
                "change_ref": "RT-2/C-01",
                "task_id": "RT-2",
                "status": "active"
            }),
        ],
        ..Default::default()
    };

    assert_eq!(
        task_land_remote_change_id_with_task_remote(&mut remote, "fixture-ait", "RT-2/C-01")
            .expect("explicit ref lookup"),
        Some("RT-2/C-01".to_string())
    );
    assert_eq!(
        task_land_remote_change_id_with_task_remote(&mut remote, "fixture-ait", "RT-1")
            .expect("task-scoped lookup"),
        Some("RT-1/C-01".to_string())
    );
}

#[test]
fn task_land_remote_id_reads_accept_change_and_task_record_remote_traits() {
    let mut task_remote = FakeWorkflowReadRemote {
        tasks: BTreeMap::from([(
            "RCT-1".to_string(),
            json!({
                "task_id": "RCT-1"
            }),
        )]),
        ..Default::default()
    };
    let mut change_remote = FakeChangeRemote {
        changes: BTreeMap::from([(
            "RCC-1".to_string(),
            json!({
                "change_id": "RCC-1",
                "task_id": "RCT-1"
            }),
        )]),
        change_rows: vec![json!({
            "change_id": "RCC-1",
            "task_id": "RCT-1",
            "status": "active"
        })],
        ..Default::default()
    };

    let change =
        task_land_remote_change_read_with_task_remote(&mut change_remote, "fixture-ait", "RCC-1")
            .expect("read remote land change");
    assert_eq!(change["change_id"], json!("RCC-1"));

    let task =
        task_land_remote_task_read_with_task_remote(&mut task_remote, "fixture-ait", "RCT-1")
            .expect("read remote land task");
    assert_eq!(task["task_id"], json!("RCT-1"));

    let changes = task_land_remote_change_rows_with_task_remote(&mut change_remote, "fixture-ait")
        .expect("read remote land change rows");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["change_id"], json!("RCC-1"));

    let err = task_land_remote_change_read_with_task_remote(
        &mut change_remote,
        "fixture-ait",
        "RCC-MISSING",
    )
    .expect_err("missing change should fail");
    assert!(err.contains("Unknown change"));
}

#[test]
fn task_land_bound_line_capture_uses_exact_task_identity_and_rejects_ambiguity() {
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
    let primary = task_feature_line_name("LCT-42").unwrap();
    create_local_line(&repo, &primary, None).unwrap();

    let captured = task_land_capture_bound_line(&repo, "LCT-42")
        .unwrap()
        .unwrap();
    assert_eq!(captured["line_name"], primary);
    assert_eq!(captured["binding_source"], "task_identity_fallback");
    assert!(captured["line_id"].as_str().is_some());

    let legacy_only = legacy_task_feature_line_name("LCT-43").unwrap();
    create_local_line(&repo, &legacy_only, None).unwrap();
    let legacy_captured = task_land_capture_bound_line(&repo, "LCT-43")
        .unwrap()
        .unwrap();
    assert_eq!(legacy_captured["line_name"], legacy_only);

    let registered = task_feature_line_name("LCT-44").unwrap();
    create_local_line(&repo, &registered, None).unwrap();
    write_worktree_registration(
        &repo,
        "lct-44",
        &temp.path().join("lct-44"),
        None,
        &registered,
        "2026-07-26T00:00:00Z",
        Some("fixture"),
        "LCT-44",
        Some("C-01"),
        None,
        None,
        Some("main"),
        Some("main"),
    )
    .unwrap();
    let registered_capture = task_land_capture_bound_line(&repo, "LCT-44")
        .unwrap()
        .unwrap();
    assert_eq!(
        registered_capture["binding_source"],
        "bound_worktree_registry"
    );

    let legacy = legacy_task_feature_line_name("LCT-42").unwrap();
    create_local_line(&repo, &legacy, None).unwrap();
    let error = task_land_capture_bound_line(&repo, "LCT-42")
        .expect_err("multiple task-derived lines must be ambiguous");
    assert!(error.contains("multiple task-derived feature Lines"));
}

#[test]
fn archived_primary_task_line_uses_the_active_legacy_candidate() {
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
    let task_id = "RRT-0001";
    let primary = task_feature_line_name(task_id).unwrap();
    let legacy = legacy_task_feature_line_name(task_id).unwrap();
    create_local_line(&repo, &primary, None).unwrap();
    archive_local_line(&repo, &primary).unwrap();

    let ensured = ensure_task_feature_line(&repo, task_id, "main", None, true)
        .expect("archived primary Line uses the unused exact legacy candidate");
    assert_eq!(ensured["line_name"], legacy);
    assert_eq!(ensured["status"], "active");
    assert_eq!(
        local_line_row(&repo, &primary).unwrap()["status"],
        "archived"
    );

    let captured = task_land_capture_bound_line(&repo, task_id)
        .expect("sole active candidate is unambiguous")
        .expect("active legacy candidate");
    assert_eq!(captured["line_name"], legacy);
    assert_eq!(captured["status"], "active");
    assert_eq!(captured["binding_source"], "task_identity_fallback");

    archive_local_line(&repo, &legacy).unwrap();
    let error = ensure_task_feature_line(&repo, task_id, "main", None, true)
        .expect_err("two archived exact candidates must never be reused");
    assert!(
        error.contains("All exact feature Line candidates"),
        "{error}"
    );
}

#[test]
fn task_land_remote_line_closeout_is_idempotent_and_fails_closed_on_drift() {
    let line_name = "feature/rct-42";
    let mut remote = FakeWorkflowReadRemote {
        lines: BTreeMap::from([(
            line_name.to_string(),
            json!({
                "line_id": "RLNE-42",
                "line_name": line_name,
                "head_snapshot_id": "SNP-ACCEPTED",
                "status": "active"
            }),
        )]),
        ..Default::default()
    };

    let archived = task_land_archive_remote_bound_line_with_task_remote(
        &mut remote,
        "fixture-ait",
        line_name,
        "SNP-ACCEPTED",
    )
    .unwrap();
    assert_eq!(archived["status"], "archived");
    assert_eq!(
        remote.line_close_requests,
        vec![(
            "fixture-ait".to_string(),
            line_name.to_string(),
            "archived".to_string()
        )]
    );

    let resumed = task_land_archive_remote_bound_line_with_task_remote(
        &mut remote,
        "fixture-ait",
        line_name,
        "SNP-ACCEPTED",
    )
    .unwrap();
    assert_eq!(resumed["status"], "already_archived");
    assert_eq!(remote.line_close_requests.len(), 1);

    let absent = task_land_archive_remote_bound_line_with_task_remote(
        &mut remote,
        "fixture-ait",
        "feature/rct-missing",
        "SNP-ACCEPTED",
    )
    .unwrap();
    assert_eq!(absent["status"], "absent");
    assert_eq!(absent["reason"], "remote_line_absent");

    let mut drifted = FakeWorkflowReadRemote {
        lines: BTreeMap::from([(
            line_name.to_string(),
            json!({
                "line_id": "RLNE-42",
                "line_name": line_name,
                "head_snapshot_id": "SNP-NEWER",
                "status": "active"
            }),
        )]),
        ..Default::default()
    };
    let error = task_land_archive_remote_bound_line_with_task_remote(
        &mut drifted,
        "fixture-ait",
        line_name,
        "SNP-ACCEPTED",
    )
    .expect_err("head drift must block archival");
    assert!(error.contains("head drifted"));
    assert!(drifted.line_close_requests.is_empty());
}

#[test]
fn task_land_remote_line_closeout_verifies_stable_identity_when_available() {
    let line_name = "feature/rct-43";
    let mut remote = FakeWorkflowReadRemote {
        lines: BTreeMap::from([(
            line_name.to_string(),
            json!({
                "line_id": "RLNE-43",
                "line_name": line_name,
                "head_snapshot_id": "SNP-ACCEPTED",
                "status": "active"
            }),
        )]),
        close_line_identity_override: Some("RLNE-OTHER".to_string()),
        ..Default::default()
    };

    let error = task_land_archive_remote_bound_line_with_task_remote(
        &mut remote,
        "fixture-ait",
        line_name,
        "SNP-ACCEPTED",
    )
    .expect_err("identity mismatch must fail closeout");
    assert!(error.contains("changed stable identity"));
}

#[test]
fn authoritative_resume_reads_the_selected_patchset_revision_for_line_closeout() {
    let patchset_id = "RCT-1216/C-01/P-01";
    let output = json!({
        "patchset_is_authoritative": true,
        "patchset_source": "selected",
        "change": {
            "change_id": "C-01",
            "change_ref": "RCT-1216/C-01",
            "task_id": "RCT-1216",
            "selected_patchset_id": patchset_id,
            "status": "landed"
        },
        "patchset": {
            "patchset_id": patchset_id,
            "change_id": "C-01",
            "change_ref": "RCT-1216/C-01"
        }
    });
    let mut remote = FakeWorkflowCloseoutRemote {
        patchsets: BTreeMap::from([(
            patchset_id.to_string(),
            json!({
                "patchset_id": patchset_id,
                "change_id": "C-01",
                "change_ref": "RCT-1216/C-01",
                "revision_snapshot_id": "SNP-ACCEPTED"
            }),
        )]),
        ..Default::default()
    };

    let revision = task_land_selected_patchset_revision_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        &output,
    )
    .unwrap();
    assert_eq!(revision, "SNP-ACCEPTED");
    assert_eq!(
        remote.requests,
        vec![(
            patchset_id.to_string(),
            Some("fixture-ait".to_string()),
            Some("RCT-1216/C-01".to_string())
        )]
    );

    let mut mismatched_output = output.clone();
    mismatched_output["patchset"]["patchset_id"] =
        JsonValue::String("RCT-1216/C-01/P-02".to_string());
    let error = task_land_selected_patchset_revision_with_closeout_remote(
        &mut remote,
        "fixture-ait",
        &mismatched_output,
    )
    .expect_err("selected/projection mismatch must fail closed");
    assert!(error.contains("closeout projection names"));
}

fn task_land_line_fixture(
    task_id: &str,
) -> (tempfile::TempDir, RepoRuntime, String, String, JsonValue) {
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
    fs::write(temp.path().join("accepted.txt"), task_id).unwrap();
    let snapshot = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("accepted revision"),
        false,
    )
    .unwrap();
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let line_name = task_feature_line_name(task_id).unwrap();
    create_local_line(&repo, &line_name, Some(&snapshot_id)).unwrap();
    let captured = task_land_capture_bound_line(&repo, task_id)
        .unwrap()
        .unwrap();
    (temp, repo, line_name, snapshot_id, captured)
}

#[test]
fn final_local_task_land_archives_its_feature_line_and_resume_is_idempotent() {
    let (_temp, repo, line_name, snapshot_id, captured) = task_land_line_fixture("LCT-ARCHIVE");
    let mut output = json!({
        "apply_status": "done",
        "task_id": "LCT-ARCHIVE",
        "task_status": "completed",
        "target_line": "main",
        "landed_snapshot_id": snapshot_id,
        "bound_worktree_cleanup": {
            "status": "removed"
        }
    });

    task_land_attach_bound_line_closeout(
        &repo,
        &mut output,
        true,
        None,
        Ok(Some(captured.clone())),
    );
    assert_eq!(output["bound_line_closeout"]["status"], "archived");
    assert_eq!(output["bound_line_closeout"]["line_name"], line_name);
    assert_eq!(
        local_line_row(&repo, &line_name).unwrap()["status"],
        "archived"
    );

    task_land_attach_bound_line_closeout(&repo, &mut output, true, None, Ok(Some(captured)));
    assert_eq!(output["bound_line_closeout"]["status"], "already_archived");
}

#[test]
fn remote_closeout_archives_an_empty_local_placeholder_but_local_closeout_rejects_it() {
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
    let remote_line_name = task_feature_line_name("RCT-EMPTY").unwrap();
    create_local_line(&repo, &remote_line_name, None).unwrap();
    let remote_candidate = task_land_capture_bound_line(&repo, "RCT-EMPTY")
        .unwrap()
        .unwrap();

    let archived = task_land_archive_local_bound_line(
        &repo,
        &remote_candidate,
        "SNP-REMOTE-ACCEPTED",
        Some("main"),
        true,
    )
    .unwrap();
    assert_eq!(archived["status"], "archived");
    assert_eq!(archived["head_state"], "empty_remote_placeholder");
    assert_eq!(
        local_line_row(&repo, &remote_line_name).unwrap()["status"],
        "archived"
    );

    let resumed = task_land_archive_local_bound_line(
        &repo,
        &remote_candidate,
        "SNP-REMOTE-ACCEPTED",
        Some("main"),
        true,
    )
    .unwrap();
    assert_eq!(resumed["status"], "already_archived");
    assert_eq!(resumed["head_state"], "empty_remote_placeholder");

    let local_line_name = task_feature_line_name("LCT-EMPTY").unwrap();
    create_local_line(&repo, &local_line_name, None).unwrap();
    let local_candidate = task_land_capture_bound_line(&repo, "LCT-EMPTY")
        .unwrap()
        .unwrap();
    let error = task_land_archive_local_bound_line(
        &repo,
        &local_candidate,
        "SNP-LOCAL-LANDED",
        Some("main"),
        false,
    )
    .expect_err("local closeout must not accept an empty feature Line");
    assert!(error.contains("head drifted"));
    assert_eq!(
        local_line_row(&repo, &local_line_name).unwrap()["status"],
        "active"
    );
}

#[test]
fn non_final_task_and_head_drift_leave_feature_line_active() {
    let (_temp, repo, line_name, snapshot_id, captured) = task_land_line_fixture("LCT-NONFINAL");
    let mut non_final = json!({
        "apply_status": "done",
        "task_id": "LCT-NONFINAL",
        "task_status": "active",
        "target_line": "main",
        "landed_snapshot_id": snapshot_id,
    });
    task_land_attach_bound_line_closeout(
        &repo,
        &mut non_final,
        true,
        None,
        Ok(Some(captured.clone())),
    );
    assert_eq!(
        non_final["bound_line_closeout"]["reason"],
        "task_still_active"
    );
    assert_eq!(
        local_line_row(&repo, &line_name).unwrap()["status"],
        "active"
    );

    fs::write(repo.workspace_root().join("newer.txt"), "newer").unwrap();
    let newer = create_local_snapshot(
        repo.workspace_root().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("newer revision"),
        false,
    )
    .unwrap();
    let newer_id = required_string_field(&newer, "snapshot_id").unwrap();
    set_local_line_head(&repo, &line_name, Some(&newer_id)).unwrap();
    let mut drifted = json!({
        "apply_status": "done",
        "task_id": "LCT-NONFINAL",
        "task_status": "completed",
        "target_line": "main",
        "landed_snapshot_id": snapshot_id,
        "bound_worktree_cleanup": {
            "status": "removed"
        }
    });
    task_land_attach_bound_line_closeout(&repo, &mut drifted, true, None, Ok(Some(captured)));
    assert_eq!(drifted["bound_line_closeout"]["status"], "failed");
    assert!(drifted["bound_line_closeout"]["error"]
        .as_str()
        .unwrap()
        .contains("head drifted"));
    assert_eq!(
        local_line_row(&repo, &line_name).unwrap()["status"],
        "active"
    );
}

#[test]
fn stopped_task_land_never_attempts_plan_checklist_closeout() {
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
    let mut output = json!({
        "apply_status": "stopped",
        "task": {
            "task_id": "LCT-1",
            "plan_id": "CPL-MISSING",
            "plan_item_ref": "missing/item"
        }
    });
    task_land_attach_plan_checklist_closeout(&repo, &mut output, true, None);
    assert!(output.get("plan_checklist_closeout").is_none());
}

#[test]
fn local_task_land_uses_captured_task_binding_after_worktree_cleanup() {
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
    let mut output = json!({
        "apply_status": "done",
        "task_status": "completed",
        "task": {
            "task_id": "LCT-REMOVED-WORKTREE"
        }
    });

    task_land_attach_plan_checklist_closeout(&repo, &mut output, true, None);

    assert_eq!(
        output["plan_checklist_closeout"]["status"],
        json!("skipped")
    );
    assert_eq!(
        output["plan_checklist_closeout"]["reason"],
        json!("no_plan_binding")
    );
}

#[test]
fn local_non_final_change_leaves_task_and_plan_item_open() {
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
    let mut output = json!({
        "apply_status": "done",
        "task_id": "LCT-MULTI",
        "task_status": "active",
        "open_peer_change_count": 1,
        "task": {
            "task_id": "LCT-MULTI",
            "plan_id": "CPL-NOT-READ",
            "origin_plan_revision_id": "plan-revision:9",
            "plan_item_ref": "multi/final"
        }
    });

    task_land_attach_plan_checklist_closeout(&repo, &mut output, true, None);

    assert_eq!(output["plan_checklist_closeout"]["status"], "deferred");
    assert_eq!(
        output["plan_checklist_closeout"]["reason"],
        "task_still_active"
    );
    assert_eq!(
        output["plan_checklist_closeout"]["open_peer_change_count"],
        1
    );
    assert_eq!(output["closeout_status"], "change_landed_task_active");
}

#[test]
fn local_task_land_reuses_already_landed_state_for_idempotent_closeout() {
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
    fs::write(temp.path().join("landed.txt"), "already landed").unwrap();
    let landed_snapshot = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("already landed local closeout"),
        false,
    )
    .unwrap();
    let landed_snapshot_id = landed_snapshot["snapshot_id"].as_str().unwrap().to_string();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let task_store = repo.task_store().unwrap();
    let change_store = repo.change_store().unwrap();
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Recover local closeout",
        "Prove already-landed phase reuse",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = task["task_id"].as_str().unwrap();
    let change = change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        task_id,
        "Recover closeout",
        "main",
        None,
        None,
    )
    .unwrap();
    let change_id = change["change_id"].as_str().unwrap();
    workflow_local_change_land_with_change_store(
        &change_store,
        change_id,
        "main",
        &landed_snapshot_id,
        None,
    )
    .unwrap();
    workflow_local_task_close_with_task_store(&task_store, task_id, "completed").unwrap();

    let recovered = task_land_apply_local(
        &repo,
        change_id,
        None,
        None,
        None,
        None::<fn(&JsonValue) -> Result<(), String>>,
    )
    .unwrap()
    .unwrap();

    assert_eq!(recovered["execution_status"], "already_landed");
    assert_eq!(recovered["change_status"], "landed");
    assert_eq!(recovered["task_status"], "completed");
    assert_eq!(recovered["closeout_status"], "complete_unbound");
    assert_eq!(
        recovered["plan_checklist_closeout"]["reason"],
        "no_plan_binding"
    );
}

#[test]
fn remote_task_land_defers_plan_sync_without_remote_or_plan_access() {
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
    let mut output = json!({
        "apply_status": "done",
        "task": {
            "task_id": "RCT-REMOTE-DONE",
            "status": "completed",
            "plan_id": "PR-42",
            "origin_plan_revision_id": "plan-revision:7",
            "plan_item_ref": "remote-land/implement"
        }
    });

    // This remote deliberately does not exist. A Plan show/sync or remote-row
    // lookup would make the closeout fail instead of producing this result.
    task_land_attach_plan_checklist_closeout(
        &repo,
        &mut output,
        false,
        Some("unconfigured-origin"),
    );

    let closeout = &output["plan_checklist_closeout"];
    assert_eq!(closeout["status"], json!("deferred"));
    assert_eq!(
        closeout["reason"],
        json!("remote_plan_sync_is_separate_from_task_land")
    );
    assert_eq!(closeout["remote"], json!("unconfigured-origin"));
    assert_eq!(closeout["task_id"], json!("RCT-REMOTE-DONE"));
    assert_eq!(closeout["plan_id"], json!("PR-42"));
    assert_eq!(closeout["plan_item_ref"], json!("remote-land/implement"));
    assert_eq!(
        closeout["command"],
        json!("ait plan sync <bound-sprint-card-path> --remote unconfigured-origin")
    );
    assert_eq!(closeout["updated"], json!(false));
}

#[test]
fn remote_non_final_change_keeps_bound_plan_item_open() {
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
    let mut output = json!({
        "apply_status": "done",
        "task": {
            "task_id": "RCT-REMOTE-MULTI",
            "status": "active",
            "plan_id": "PR-99",
            "origin_plan_revision_id": "plan-revision:12",
            "plan_item_ref": "remote-multi/final"
        }
    });

    task_land_attach_plan_checklist_closeout(&repo, &mut output, false, Some("missing"));

    assert_eq!(output["plan_checklist_closeout"]["status"], "deferred");
    assert_eq!(
        output["plan_checklist_closeout"]["reason"],
        "task_still_active"
    );
    assert_eq!(output["closeout_status"], "change_landed_task_active");
}

fn atomic_task_land_cli_response() -> JsonValue {
    json!({
        "contract": "task-land-atomic/v1",
        "repo_name": "fixture-ait",
        "repository_index": 7,
        "idempotency_key": "task-land-atomic:key",
        "replayed": false,
        "status": "succeeded",
        "task_id": "RCT-ATOMIC",
        "task_status": "completed",
        "change_id": "C-01",
        "change_ref": "RCT-ATOMIC/C-01",
        "change_status": "landed",
        "patchset_id": "RCT-ATOMIC/C-01/P-01",
        "target_line": "main",
        "landed_snapshot_id": "SNP-ATOMIC",
        "task": {
            "task_id": "RCT-ATOMIC",
            "status": "completed",
            "plan_id": "PR-1",
            "plan_item_ref": "atomic/land"
        },
        "change": {
            "task_id": "RCT-ATOMIC",
            "change_id": "C-01",
            "change_ref": "RCT-ATOMIC/C-01",
            "status": "landed",
            "selected_patchset_id": "RCT-ATOMIC/C-01/P-01"
        },
        "patchset": {
            "patchset_id": "RCT-ATOMIC/C-01/P-01",
            "revision_snapshot_id": "SNP-ATOMIC"
        },
        "land": {
            "submission_id": "RCT-ATOMIC/C-01/L-01",
            "status": "succeeded",
            "target_line": "main",
            "landed_snapshot_id": "SNP-ATOMIC"
        }
    })
}

#[test]
fn atomic_task_land_output_preserves_existing_contract_with_one_remote_mutation() {
    let response = atomic_task_land_cli_response();
    let local = json!({
        "result": {
            "target_line": "main",
            "landed_snapshot_id": "SNP-ATOMIC"
        },
        "local_sync": {
            "status": "synced",
            "line": "main",
            "landed_snapshot_id": "SNP-ATOMIC"
        },
        "bound_worktree_cleanup": {
            "status": "deferred",
            "reason": "task_land_main_seed_finalizer"
        }
    });
    let action = task_land_atomic_action_result(&response, local).unwrap();
    let mut output = task_land_atomic_output(&response, action).unwrap();
    attach_task_land_contract(&mut output, false);

    assert_eq!(output["apply_status"], "done");
    assert_eq!(output["repository_index"], 7);
    assert_eq!(output["atomic_task_land"]["remote_mutation_count"], 1);
    assert_eq!(output["applied_actions"].as_array().unwrap().len(), 2);
    assert_eq!(output["mutation_receipts"].as_array().unwrap().len(), 2);
    assert_eq!(output["task"]["status"], "completed");
    assert_eq!(output["change"]["status"], "landed");
    assert_eq!(output["patchset"]["patchset_id"], "RCT-ATOMIC/C-01/P-01");
    assert!(output["workspace"]["clean"].is_null());
    assert_eq!(output["workspace"]["evaluation"], "skipped");
    assert_eq!(
        output["workspace"]["reason"],
        "ready_patchset_is_authoritative"
    );
    assert_eq!(
        output["workspace"]["read_scope"],
        "line_and_bound_worktree_metadata_only"
    );
    assert_eq!(
        output["closeout_status"],
        "execution_complete_plan_separate"
    );
}

#[test]
fn atomic_task_land_main_seed_failure_is_partial_and_preserves_worktree() {
    let response = atomic_task_land_cli_response();
    let action = task_land_atomic_action_result(
        &response,
        json!({
            "result": {
                "target_line": "main",
                "landed_snapshot_id": "SNP-ATOMIC"
            },
            "local_sync": {
                "status": "synced",
                "line": "main",
                "landed_snapshot_id": "SNP-ATOMIC"
            }
        }),
    )
    .unwrap();
    let mut output = task_land_atomic_output(&response, action).unwrap();
    output["main_seed_sync"] = json!({
        "status": "failed",
        "reason": "post_land_cli_main_seed_sync_failed",
        "error": "permission denied"
    });
    task_land_defer_bound_cleanup(
        &mut output,
        "main_seed_sync_failed",
        "repair and retry",
        Some("permission denied"),
    );
    attach_task_land_contract(&mut output, false);

    assert_eq!(output["closeout_status"], "partial");
    assert_eq!(task_land_exit_code(&output), 2);
    assert_eq!(output["bound_worktree_cleanup"]["status"], "deferred");
    assert_eq!(output["bound_worktree_cleanup"]["removed"], false);
    assert_eq!(
        output["closeout_recovery"]["command"],
        "ait task land RCT-ATOMIC/C-01"
    );
}
