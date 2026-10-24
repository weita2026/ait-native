use super::*;
use crate::primitives::change_flow::{
    change_local_close_with_change_store, change_local_create_with_change_store,
    change_local_mark_published_with_change_store,
};
use crate::primitives::workflow::local_completion::{
    workflow_history_candidate_identity, workflow_history_idempotency_key,
    workflow_history_prepare_entries, workflow_history_remote_is_prepared,
    workflow_staged_history_prepare_request, workflow_staged_history_promotion_id,
    workflow_supported_correction_manifest, workflow_validate_history_publication_response,
};
use crate::primitives::worktree::create_local_line_with_line_store;

#[test]
fn workflow_task_input_uses_recorded_cross_ordinal_publication_without_writes() {
    for (selected_ordinal, remote_ordinal) in [(1, 2), (2, 1)] {
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
        let task_store = repo.task_store().unwrap();
        let change_store = repo.change_store().unwrap();
        let task = task_local_create_with_task_store(
            &task_store,
            "fixture-ait",
            "Cross ordinal publication",
            "Preserve exact remote mapping",
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let task_id = required_string_field(&task, "task_id").unwrap();
        let mut selected_ref = String::new();
        for ordinal in 1..=2 {
            let change = change_local_create_with_change_store(
                &change_store,
                "fixture-ait",
                &task_id,
                "Mapped work",
                "main",
                None,
                None,
            )
            .unwrap();
            let reference = required_string_field(&change, "change_ref").unwrap();
            assert!(reference.ends_with(&format!("/C-{ordinal:02}")));
            if ordinal == selected_ordinal {
                selected_ref = reference;
            } else {
                change_local_close_with_change_store(&change_store, &reference, "archived")
                    .unwrap();
            }
        }
        let unpublished =
            workflow_task_change_reference(&repo, &task_id, Some("origin")).unwrap_err();
        assert!(unpublished.contains("still local work"), "{unpublished}");
        let remote_ref = format!("RT-0009/C-{remote_ordinal:02}");
        task_local_mark_published_with_task_store(
            &task_store,
            &task_id,
            Some("origin"),
            Some("RT-0009"),
        )
        .unwrap();
        change_local_mark_published_with_change_store(
            &change_store,
            &selected_ref,
            Some("origin"),
            Some(&remote_ref),
            false,
        )
        .unwrap();
        let before = change_list(&repo, true, None).unwrap();
        assert_eq!(
            workflow_task_change_reference(&repo, &task_id, Some("origin")).unwrap(),
            "RT-0009"
        );
        assert_eq!(
            crate::primitives::resolve_public_patchset_input(
                &repo,
                &format!("{task_id}/P-02"),
                Some("origin")
            )
            .unwrap(),
            "RT-0009/P-02"
        );
        assert_eq!(
            workflow_task_change_reference(&repo, &selected_ref, Some("origin")).unwrap(),
            selected_ref
        );
        let error = workflow_task_change_reference(&repo, &task_id, Some("other")).unwrap_err();
        assert!(error.contains("selected remote other"), "{error}");
        assert_eq!(change_list(&repo, true, None).unwrap(), before);
    }
}

#[test]
fn completed_task_input_keeps_the_existing_history_boundary() {
    let (_temp, repo, base, revision, reference) = local_land_fixture(1);
    let (task_id, _) = reference.split_once('/').unwrap();
    let resolved = workflow_task_change_reference(&repo, task_id, Some("origin")).unwrap();
    assert_eq!(resolved, task_id);
    let (entries, _) =
        workflow_local_history_entries(&repo, &resolved, "main", &base, &revision).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["local_change_ref"], reference);
    assert_eq!(entries[0]["pre_land_target_snapshot_id"], base);
    assert_eq!(entries[0]["landed_snapshot_id"], revision);
}

#[test]
fn completed_task_promotes_multiple_revisions_as_one_receipt() {
    let (temp, repo, base, _, _) = local_land_fixture(0);
    let task_store = repo.task_store().unwrap();
    let change_store = repo.change_store().unwrap();
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Two accepted changes",
        "Preserve promotion boundary",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    let mut previous = base.clone();
    let mut last_ref = String::new();
    for ordinal in 1..=2 {
        let change = change_local_create_with_change_store(
            &change_store,
            "fixture-ait",
            &task_id,
            "Independent work",
            "main",
            None,
            Some(&previous),
        )
        .unwrap();
        last_ref = required_string_field(&change, "change_ref").unwrap();
        fs::write(
            temp.path().join("history.txt"),
            format!("accepted {ordinal}"),
        )
        .unwrap();
        let snapshot = create_local_snapshot(
            temp.path().to_str().unwrap(),
            "fixture-ait",
            "main",
            Some("accepted work"),
            false,
        )
        .unwrap();
        let revision = required_string_field(&snapshot, "snapshot_id").unwrap();
        let child = required_string_field(&change, "change_id").unwrap();
        bind_fixture_snapshot(&repo, &task_id, Some(&child), &revision).unwrap();
        previous = revision;
    }
    set_local_line_head(&repo, "main", Some(&base)).unwrap();
    task_store
        .finish_local_task(
            &task_id,
            &previous,
            "main",
            Some(&base),
            Some(&last_ref),
            None,
        )
        .unwrap();
    let before = change_list(&repo, true, None).unwrap();
    assert_eq!(
        workflow_task_change_reference(&repo, &task_id, Some("origin")).unwrap(),
        task_id
    );
    let (entries, _) =
        workflow_local_history_entries(&repo, &last_ref, "main", &base, &previous).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["snapshots"].as_array().unwrap().len(), 2);
    assert_eq!(change_list(&repo, true, None).unwrap(), before);
}

#[test]
fn patchset_head_relation_preserves_snapshot_direction() {
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
    fs::write(temp.path().join("line.txt"), "older\n").unwrap();
    let older = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("older head"),
        false,
    )
    .unwrap();
    let older_snapshot_id = required_string_field(&older, "snapshot_id").unwrap();
    fs::write(temp.path().join("line.txt"), "selected\n").unwrap();
    let selected = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("selected revision"),
        false,
    )
    .unwrap();
    let selected_snapshot_id = required_string_field(&selected, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();

    assert_eq!(
        workflow_patchset_head_relation(
            &repo,
            Some(&selected_snapshot_id),
            Some(&older_snapshot_id),
        )
        .unwrap(),
        WorkflowPatchsetHeadRelation::PatchsetDescendsFromCurrent
    );
    assert_eq!(
        workflow_patchset_head_relation(
            &repo,
            Some(&older_snapshot_id),
            Some(&selected_snapshot_id),
        )
        .unwrap(),
        WorkflowPatchsetHeadRelation::CurrentDescendsFromPatchset
    );
    assert_eq!(
        workflow_patchset_head_relation(
            &repo,
            Some("SNP-000000000000"),
            Some(&selected_snapshot_id),
        )
        .unwrap(),
        WorkflowPatchsetHeadRelation::Unknown
    );
}

#[test]
fn workflow_local_land_sync_remote_info_accepts_line_and_change_remote_traits() {
    let mut line_remote = FakeLineRemote {
        lines: BTreeMap::from([(
            "main".to_string(),
            json!({
                "line_name": "main",
                "head_snapshot_id": "SNP-LANDED"
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
        ..Default::default()
    };
    let land_result = json!({
        "status": "succeeded",
        "result": {
            "target_line": "main"
        }
    });
    let result = land_result.get("result").expect("land result object");

    let landed_snapshot_id = workflow_local_land_landed_snapshot_id_with_task_remote(
        &mut line_remote,
        "fixture-ait",
        "main",
        result,
        &land_result,
        None,
    );
    let task_id =
        workflow_local_land_task_id_with_task_remote(&mut change_remote, "fixture-ait", "RCC-1")
            .expect("remote task id");

    assert_eq!(landed_snapshot_id.as_deref(), Some("SNP-LANDED"));
    assert_eq!(task_id, "RCT-1");
}

#[test]
fn workflow_attach_local_land_sync_accepts_change_and_line_remote_traits() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "base").expect("base fixture");
    let base_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local land sync base"),
        false,
    )
    .expect("create base snapshot");
    let base_snapshot_id =
        required_string_field(&base_snapshot, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "landed").expect("landed fixture");
    let landed_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local land sync landed"),
        false,
    )
    .expect("create landed snapshot");
    let landed_snapshot_id =
        required_string_field(&landed_snapshot, "snapshot_id").expect("landed snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    set_local_line_head(&repo, "main", Some(&base_snapshot_id)).expect("reset local main");
    let mut remote = FakeWorkflowReadRemote {
        lines: BTreeMap::from([(
            "main".to_string(),
            json!({
                "line_name": "main",
                "head_snapshot_id": landed_snapshot_id
            }),
        )]),
        changes: BTreeMap::from([(
            "RCC-1".to_string(),
            json!({
                "change_id": "RCC-1",
                "task_id": "RCT-1"
            }),
        )]),
        ..Default::default()
    };
    let land_result = json!({
        "status": "succeeded",
        "result": {
            "target_line": "main"
        }
    });

    let payload = workflow_attach_local_land_sync_with_task_remote(
        &repo,
        &mut remote,
        "fixture-ait",
        "RCC-1",
        &land_result,
        None,
    )
    .expect("attach local land sync");

    assert_eq!(
        local_line_head_snapshot_id(&repo, "main")
            .expect("main head")
            .as_deref(),
        Some(landed_snapshot_id.as_str())
    );
    assert_eq!(payload["local_sync"]["status"], json!("synced"));
    assert_eq!(
        payload["local_sync"]["landed_snapshot_id"],
        json!(landed_snapshot_id)
    );
    assert_eq!(
        payload["bound_worktree_cleanup"]["reason"],
        json!("task_land_main_seed_finalizer")
    );
    assert_eq!(
        payload["bound_worktree_cleanup"]["status"],
        json!("deferred")
    );
    assert_eq!(
        remote.line_requests,
        vec![("fixture-ait".to_string(), "main".to_string())]
    );
}

#[test]
fn workflow_land_submit_action_accepts_change_line_and_closeout_remote_traits() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("src.txt"), "base").expect("base fixture");
    let base_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("land submit base"),
        false,
    )
    .expect("create base snapshot");
    let base_snapshot_id =
        required_string_field(&base_snapshot, "snapshot_id").expect("base snapshot id");
    fs::write(repo_root.join("src.txt"), "landed").expect("landed fixture");
    let landed_snapshot = create_local_snapshot(
        repo_root.to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("land submit landed"),
        false,
    )
    .expect("create landed snapshot");
    let landed_snapshot_id =
        required_string_field(&landed_snapshot, "snapshot_id").expect("landed snapshot id");
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    set_local_line_head(&repo, "main", Some(&base_snapshot_id)).expect("reset local main");
    let mut task_remote = FakeWorkflowReadRemote {
        lines: BTreeMap::from([(
            "main".to_string(),
            json!({
                "line_name": "main",
                "head_snapshot_id": landed_snapshot_id
            }),
        )]),
        changes: BTreeMap::from([(
            "RCC-LAND".to_string(),
            json!({
                "change_id": "RCC-LAND",
                "task_id": "RCT-LAND"
            }),
        )]),
        ..Default::default()
    };
    let patchset = json!({
        "patchset_id": "RCP-LAND-1",
        "change_id": "RCC-LAND",
        "revision_snapshot_id": landed_snapshot_id,
    });
    let mut closeout_remote = FakeWorkflowCloseoutRemote {
        patchsets: BTreeMap::from([("RCP-LAND-1".to_string(), patchset.clone())]),
        ..Default::default()
    };
    let mut guard_seen = Vec::new();

    let payload = workflow_land_submit_action_with_task_and_closeout_remotes(
        &repo,
        &mut task_remote,
        &mut closeout_remote,
        "fixture-ait",
        "RCC-LAND",
        &patchset,
        Some("main"),
        "merge",
        None,
        true,
        |task_id, change_id| {
            guard_seen.push((task_id.map(str::to_string), change_id.to_string()));
            Ok(())
        },
    )
    .expect("submit land and attach local sync through trait remotes");

    assert_eq!(
        local_line_head_snapshot_id(&repo, "main")
            .expect("main head")
            .as_deref(),
        Some(landed_snapshot_id.as_str())
    );
    assert_eq!(
        guard_seen,
        vec![(Some("RCT-LAND".to_string()), "RCC-LAND".to_string())]
    );
    assert_eq!(closeout_remote.requests.len(), 1);
    assert_eq!(closeout_remote.land_submissions.len(), 1);
    assert_eq!(
        closeout_remote.land_submissions[0]["patchset_id"],
        json!("RCP-LAND-1")
    );
    assert_eq!(payload["result"]["local_sync"]["status"], json!("synced"));
    assert_eq!(
        payload["result"]["local_sync"]["landed_snapshot_id"],
        json!(landed_snapshot_id)
    );
    assert_eq!(
        task_remote.change_requests,
        vec![
            ("RCC-LAND".to_string(), Some("fixture-ait".to_string())),
            ("RCC-LAND".to_string(), Some("fixture-ait".to_string()))
        ]
    );
}

fn completed_local_entry(change_id: &str, landed_snapshot_id: &str) -> JsonValue {
    json!({
        "status": "ready",
        "task_id": "LCT-FINAL",
        "change_id": change_id,
        "target_line": "main",
        "state": {
            "routing": {
                "kind": "completed_local",
                "local_task_id": "LCT-FINAL",
                "local_change_id": change_id,
                "target_line": "main",
            },
            "task": {
                "task_id": "LCT-FINAL",
                "status": "completed",
                "publication_state": "local_draft",
            },
            "change": {
                "change_id": change_id,
                "task_id": "LCT-FINAL",
                "status": "landed",
                "publication_state": "local_draft",
                "base_line": "main",
                "landed_snapshot_id": landed_snapshot_id,
            },
        },
    })
}

fn published_completed_local_entry(change_id: &str, landed_snapshot_id: &str) -> JsonValue {
    let mut entry = completed_local_entry(change_id, landed_snapshot_id);
    entry["state"]["task"]["publication_state"] = json!("published");
    entry["state"]["task"]["published_task_id"] = json!("RCT-REMOTE");
    entry["state"]["change"]["publication_state"] = json!("published");
    entry["state"]["change"]["published_change_id"] = json!("RCT-REMOTE/C-01");
    entry
}

#[test]
fn completed_local_finish_keeps_the_published_aggregate_patchset_authoritative() {
    let candidate = workflow_final_snapshot_candidate_from_entry(
        &published_completed_local_entry("C-01", "SNP-N"),
        Some("SNP-N"),
        false,
        Some("SNP-ZERO"),
        None,
        Some(1),
    )
    .expect("published completed-local promotion candidate");

    let (remote_change_id, patchset_is_authoritative) =
        workflow_completed_local_finish_authority(Some(&candidate))
            .expect("completed-local finish authority");

    assert_eq!(remote_change_id.as_deref(), Some("RCT-REMOTE/C-01"));
    assert!(patchset_is_authoritative);
    assert_eq!(
        workflow_completed_local_command_change_ref(&candidate)
            .expect("exact completed-local command reference"),
        "LCT-FINAL/C-01"
    );
}

#[test]
fn unfinished_or_direct_remote_finish_retains_workspace_authoring_checks() {
    let unpublished = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("C-01", "SNP-N"),
        Some("SNP-N"),
        false,
        Some("SNP-ZERO"),
        None,
        Some(1),
    )
    .expect("unpublished completed-local promotion candidate");

    let unpublished_authority = workflow_completed_local_finish_authority(Some(&unpublished))
        .expect("unpublished finish authority");
    let direct_remote_authority =
        workflow_completed_local_finish_authority(None).expect("direct remote finish authority");

    assert_eq!(unpublished_authority, (None, false));
    assert_eq!(direct_remote_authority, (None, false));
}

#[test]
fn final_snapshot_promotion_aggregates_remote_head_to_latest_local_head() {
    let candidate = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("LCC-FINAL", "SNP-N"),
        Some("SNP-N"),
        false,
        Some("SNP-ZERO"),
        None,
        Some(5),
    )
    .expect("final snapshot candidate");

    assert_eq!(
        candidate["mode"],
        json!("completed_local_history_promotion")
    );
    assert_eq!(candidate["base_snapshot_id"], json!("SNP-ZERO"));
    assert_eq!(candidate["revision_snapshot_id"], json!("SNP-N"));
    assert_eq!(candidate["aggregate_snapshot_count"], json!(5));
    assert_eq!(candidate["remote_already_contains_revision"], json!(false));
}

#[test]
fn final_snapshot_promotion_null_remote_selects_exact_pre_land_base() {
    let candidate = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("LCC-FINAL", "SNP-N"),
        Some("SNP-N"),
        false,
        None,
        Some("SNP-PRE-LAND"),
        Some(1),
    )
    .expect("null remote candidate should use the selected Change pre-land base");

    assert_eq!(candidate["base_snapshot_id"], json!("SNP-PRE-LAND"));
    assert_eq!(candidate["revision_snapshot_id"], json!("SNP-N"));
    assert_eq!(candidate["aggregate_snapshot_count"], json!(1));
    assert_eq!(
        candidate["remote_head_initialization_required"],
        json!(true)
    );
    assert_eq!(candidate["remote_already_contains_revision"], json!(false));
}

#[test]
fn final_snapshot_promotion_null_remote_rejects_final_snapshot_as_base() {
    let error = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("LCC-FINAL", "SNP-N"),
        Some("SNP-N"),
        false,
        None,
        Some("SNP-N"),
        Some(0),
    )
    .expect_err("null remote must not initialize directly at the final local Snapshot");

    assert!(error.contains("Refusing to initialize null remote `main` directly"));
    assert!(error.contains("requires an earlier pre-land bootstrap boundary"));
}

#[test]
fn final_snapshot_promotion_preview_uses_an_exact_local_change_reference() {
    let mut candidate = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("C-01", "SNP-N"),
        Some("SNP-N"),
        false,
        Some("SNP-ZERO"),
        None,
        Some(1),
    )
    .expect("final snapshot candidate");
    candidate["remote_name"] = json!("origin");

    let preview = workflow_final_snapshot_promotion_preview(&candidate)
        .expect("final snapshot promotion preview");

    assert_eq!(preview["change_id"], "C-01");
    assert_eq!(preview["local_change_ref"], "LCT-FINAL/C-01");
    assert_eq!(
        preview["next_action"]["command"],
        "ait workflow ready LCT-FINAL --apply --remote origin"
    );
    assert!(preview["next_action"]["detail"]
        .as_str()
        .unwrap()
        .contains("ait workflow finish LCT-FINAL --apply --remote origin"));
}

#[test]
fn final_snapshot_promotion_rejects_an_older_completed_local_row() {
    let error = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("LCC-OLDER", "SNP-N-MINUS-ONE"),
        Some("SNP-N"),
        true,
        Some("SNP-ZERO"),
        None,
        Some(4),
    )
    .expect_err("older local row must not promote");

    assert!(error.contains("Only the latest completed local change"));
    assert!(error.contains("SNP-N"));
}

#[test]
fn published_final_snapshot_promotion_resumes_from_a_proven_local_descendant() {
    let candidate = workflow_final_snapshot_candidate_from_entry(
        &published_completed_local_entry("C-01", "SNP-PUBLISHED"),
        Some("SNP-NEWER-LOCAL"),
        true,
        Some("SNP-REMOTE-BASE"),
        None,
        Some(4),
    )
    .expect("published promotion should resume from a proven local descendant");

    assert_eq!(candidate["revision_snapshot_id"], json!("SNP-PUBLISHED"));
    assert_eq!(
        candidate["local_target_head_snapshot_id"],
        json!("SNP-NEWER-LOCAL")
    );
    assert_eq!(candidate["local_target_head_is_revision"], json!(false));
    assert_eq!(
        candidate["local_target_head_contains_revision"],
        json!(true)
    );
    assert_eq!(candidate["published_descendant_resume"], json!(true));
}

#[test]
fn published_final_snapshot_promotion_rejects_a_non_descendant_local_head() {
    let error = workflow_final_snapshot_candidate_from_entry(
        &published_completed_local_entry("C-01", "SNP-PUBLISHED"),
        Some("SNP-DIVERGED-LOCAL"),
        false,
        Some("SNP-REMOTE-BASE"),
        None,
        Some(4),
    )
    .expect_err("published promotion must not resume from divergent local history");

    assert!(error.contains("may resume only when Snapshot ancestry proves"));
    assert!(error.contains("SNP-DIVERGED-LOCAL"));
}

#[test]
fn final_snapshot_promotion_rejects_remote_divergence_before_publish() {
    let error = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("LCC-FINAL", "SNP-N"),
        Some("SNP-N"),
        false,
        Some("SNP-REMOTE-DIVERGED"),
        None,
        None,
    )
    .expect_err("diverged remote head must fail closed");

    assert!(error.contains("does not descend from remote"));
    assert!(error.contains("Pull/reconcile"));
}

#[test]
fn same_head_promotion_rejects_unpublished_local_authority() {
    let error = workflow_same_head_remote_land_authority(
        &completed_local_entry("LCC-FINAL", "SNP-N"),
        None,
        "SNP-N",
    )
    .expect_err("unpublished same-head state must fail closed");

    assert!(error.contains("no exact publication mapping"));
}

#[test]
fn same_head_promotion_rejects_published_but_unlanded_remote_change() {
    let error = workflow_same_head_remote_land_authority(
        &published_completed_local_entry("LCC-FINAL", "SNP-N"),
        Some(&json!({
            "task_id":"RCT-REMOTE", "status":"active", "successful_land":null,
            "current_patchset_id":"RCT-REMOTE/P-01",
        })),
        "SNP-N",
    )
    .expect_err("unlanded remote Change must not own a same-head target Line");

    assert!(error.contains("no actual successful receipt"));
}

fn actual_remote_task_receipt_fixture(snapshot: &str) -> JsonValue {
    json!({"task_id":"RCT-REMOTE","status":"completed","current_patchset_id":"RCT-REMOTE/P-09",
        "successful_land":{"repository_index":7,"task_id":"RCT-REMOTE","submission_id":"RCT-REMOTE/L-01",
            "change_id":"C-01","change_ref":"RCT-REMOTE/C-01","patchset_id":"RCT-REMOTE/P-01",
            "base_snapshot_id":"B","revision_snapshot_id":"R","target_line":"main","target_line_index":0,
            "pre_land_target_snapshot_id":"B","landed_snapshot_id":snapshot,"mode":"direct","status":"succeeded",
            "created_at":"2026-10-18T00:00:00Z","updated_at":"2026-10-18T00:00:00Z"}}
    )
}

#[test]
fn same_head_promotion_rejects_mismatched_remote_landed_snapshot() {
    let error = workflow_same_head_remote_land_authority(
        &published_completed_local_entry("LCC-FINAL", "SNP-N"),
        Some(&actual_remote_task_receipt_fixture("SNP-OTHER")),
        "SNP-N",
    )
    .expect_err("mismatched remote Land must not own a same-head target Line");

    assert!(error.contains("does not own the exact applied target Snapshot"));
}

#[test]
fn same_head_promotion_accepts_actual_task_receipt_independent_of_current_candidate() {
    let authority = workflow_same_head_remote_land_authority(
        &published_completed_local_entry("LCC-FINAL", "SNP-N"),
        Some(&actual_remote_task_receipt_fixture("SNP-N")),
        "SNP-N",
    )
    .expect("exact remote Land must own an idempotent same-head replay");

    assert_eq!(authority["status"], json!("verified"));
    assert_eq!(
        authority["authority"],
        json!("remote_task_successful_receipt")
    );
    assert_eq!(authority["remote_task_id"], json!("RCT-REMOTE"));
    assert_eq!(authority["receipt"]["change_ref"], json!("RCT-REMOTE/C-01"));
    assert_eq!(authority["landed_snapshot_id"], json!("SNP-N"));
}

fn ten_local_land_fixture() -> (tempfile::TempDir, RepoRuntime, String, String, String) {
    local_land_fixture(10)
}

fn local_land_fixture(
    local_land_count: usize,
) -> (tempfile::TempDir, RepoRuntime, String, String, String) {
    let temp = tempdir().expect("history repo tempdir");
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init history repo");
    fs::write(temp.path().join("history.txt"), "base").expect("base fixture");
    let base = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("history base"),
        false,
    )
    .expect("base snapshot");
    let base_snapshot_id = required_string_field(&base, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).expect("history runtime");
    let task_store = repo.task_store().unwrap();
    let change_store = repo.change_store().unwrap();
    let mut previous_snapshot_id = base_snapshot_id.clone();
    let mut final_change_ref = String::new();
    let mut final_snapshot_id = String::new();

    for ordinal in 1..=local_land_count {
        let task = task_local_create_with_task_store(
            &task_store,
            "fixture-ait",
            &format!("Local Task {ordinal}"),
            &format!("Preserve local history {ordinal}"),
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let task_id = required_string_field(&task, "task_id").unwrap();
        let change = change_local_create_with_change_store(
            &change_store,
            "fixture-ait",
            &task_id,
            &format!("Local Change {ordinal}"),
            "main",
            None,
            Some(&previous_snapshot_id),
        )
        .unwrap();
        let change_ref = required_string_field(&change, "change_ref").unwrap();
        fs::write(temp.path().join("history.txt"), format!("landed {ordinal}")).unwrap();
        let snapshot = create_local_snapshot(
            temp.path().to_string_lossy().as_ref(),
            "fixture-ait",
            "main",
            Some(&format!("local land {ordinal}")),
            false,
        )
        .unwrap();
        let snapshot_id = required_string_field(&snapshot, "snapshot_id").unwrap();
        finish_fixture_revision(
            &repo,
            &change_ref,
            "main",
            &snapshot_id,
            Some(&previous_snapshot_id),
        )
        .unwrap();
        workflow_local_task_close_with_task_store(&task_store, &task_id, "completed").unwrap();
        final_change_ref = change_ref;
        final_snapshot_id = snapshot_id.clone();
        previous_snapshot_id = snapshot_id;
    }
    (
        temp,
        repo,
        base_snapshot_id,
        final_snapshot_id,
        final_change_ref,
    )
}

#[test]
fn history_promotion_uses_rebased_pre_land_target_as_remote_fork() {
    let temp = tempdir().expect("rebased history tempdir");
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init rebased history repo");
    fs::write(temp.path().join("history.txt"), "common base").expect("base fixture");
    let base = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("common base"),
        false,
    )
    .expect("base snapshot");
    let base_snapshot_id = required_string_field(&base, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).expect("rebased history runtime");
    create_local_line_with_line_store(
        &repo.line_store().unwrap(),
        "feature/old-main",
        Some(&base_snapshot_id),
        "2026-09-07T00:00:00Z",
    )
    .expect("old main line");
    fs::write(temp.path().join("history.txt"), "old main").expect("old main fixture");
    let old_fork = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "feature/old-main",
        Some("old main head"),
        false,
    )
    .expect("old fork snapshot");
    let old_fork_snapshot_id = required_string_field(&old_fork, "snapshot_id").unwrap();

    let task_store = repo.task_store().unwrap();
    let change_store = repo.change_store().unwrap();
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Rebased local Task",
        "Publish after replacing the target ancestry",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    let change = change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        &task_id,
        "Rebased local Change",
        "main",
        None,
        Some(&old_fork_snapshot_id),
    )
    .unwrap();
    let change_ref = required_string_field(&change, "change_ref").unwrap();

    set_local_line_head(&repo, "main", Some(&base_snapshot_id)).expect("replace local main");
    fs::write(temp.path().join("history.txt"), "replacement main")
        .expect("replacement main fixture");
    let replacement = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("replacement main head"),
        false,
    )
    .expect("replacement main snapshot");
    let replacement_snapshot_id = required_string_field(&replacement, "snapshot_id").unwrap();
    let feature_line = task_feature_line_name(&task_id).unwrap();
    create_local_line_with_line_store(
        &repo.line_store().unwrap(),
        &feature_line,
        Some(&replacement_snapshot_id),
        "2026-09-07T00:00:01Z",
    )
    .expect("rebased feature line");
    fs::write(temp.path().join("history.txt"), "rebased result").expect("rebased result fixture");
    let landed = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        &feature_line,
        Some("rebased feature head"),
        false,
    )
    .expect("rebased landed snapshot");
    let landed_snapshot_id = required_string_field(&landed, "snapshot_id").unwrap();
    finish_fixture_revision(
        &repo,
        &change_ref,
        "main",
        &landed_snapshot_id,
        Some(&replacement_snapshot_id),
    )
    .unwrap();
    workflow_local_task_close_with_task_store(&task_store, &task_id, "completed").unwrap();

    let (entries, _) = workflow_local_history_entries(
        &repo,
        &change_ref,
        "main",
        &replacement_snapshot_id,
        &landed_snapshot_id,
    )
    .expect("rebased history promotion");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0]["promotion_fork_snapshot_id"],
        replacement_snapshot_id
    );
    assert_eq!(
        entries[0]["promotion_fork_source"],
        "rebased_pre_land_target"
    );
    let prepared = workflow_history_prepare_entries(&repo, &json!({"history_entries": entries}))
        .expect("rebased remote request");
    assert_eq!(
        prepared[0]["change"]["fork_snapshot_id"],
        replacement_snapshot_id
    );
    let stored = workflow_local_change_read_with_change_store(&change_store, &change_ref).unwrap();
    assert_eq!(stored["fork_snapshot_id"], old_fork_snapshot_id);
}

#[test]
fn history_promotion_rejects_unrelated_historical_and_pre_land_forks() {
    let temp = tempdir().expect("unrelated history tempdir");
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init unrelated history repo");
    fs::write(temp.path().join("history.txt"), "common base").expect("base fixture");
    let base = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("common base"),
        false,
    )
    .unwrap();
    let base_snapshot_id = required_string_field(&base, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let branch_snapshot = |line: &str, value: &str| {
        create_local_line_with_line_store(
            &repo.line_store().unwrap(),
            line,
            Some(&base_snapshot_id),
            "2026-09-07T00:00:00Z",
        )
        .unwrap();
        fs::write(temp.path().join("history.txt"), value).unwrap();
        let snapshot = create_local_snapshot(
            temp.path().to_string_lossy().as_ref(),
            "fixture-ait",
            line,
            Some(value),
            false,
        )
        .unwrap();
        required_string_field(&snapshot, "snapshot_id").unwrap()
    };
    let historical_fork = branch_snapshot("feature/historical", "historical fork");
    let pre_land_target = branch_snapshot("feature/pre-land", "unrelated pre-land");
    let landed_snapshot_id = branch_snapshot("feature/landed", "unrelated landed");
    let error =
        crate::primitives::workflow::local_completion::workflow_history_promotion_fork_snapshot_id(
            &repo,
            "LT-0001/C-01",
            &historical_fork,
            &pre_land_target,
            &landed_snapshot_id,
        )
        .unwrap_err();
    assert!(error.contains("descends from neither"), "{error}");
}

#[test]
fn history_promotion_collects_ten_consecutive_local_lands() {
    let (_temp, repo, base_snapshot_id, final_snapshot_id, final_change_ref) =
        ten_local_land_fixture();

    let (entries, plan_artifacts) = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        &base_snapshot_id,
        &final_snapshot_id,
    )
    .expect("ten-entry history");

    assert_eq!(entries.len(), 10);
    assert!(plan_artifacts.is_empty());
    assert_eq!(
        entries.first().unwrap()["pre_land_target_snapshot_id"],
        base_snapshot_id
    );
    assert_eq!(
        entries.last().unwrap()["landed_snapshot_id"],
        final_snapshot_id
    );
    for pair in entries.windows(2) {
        assert_eq!(
            pair[0]["landed_snapshot_id"],
            pair[1]["pre_land_target_snapshot_id"]
        );
    }
    assert!(entries.iter().all(|entry| {
        entry["snapshots"]
            .as_array()
            .is_some_and(|snapshots| snapshots.len() == 1)
    }));
}

#[test]
fn history_promotion_adopts_unowned_direct_boundary_snapshot() {
    let (temp, repo, base_snapshot_id, first_landed_snapshot_id, _first_change_ref) =
        local_land_fixture(1);

    // An unowned direct Snapshot advances main between two landed Changes,
    // exactly like the historical repo-root authoring incident.
    fs::write(temp.path().join("orphan.txt"), "unowned direct snapshot").unwrap();
    let orphan = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("reviewable snapshot"),
        false,
    )
    .expect("orphan snapshot");
    let orphan_snapshot_id = required_string_field(&orphan, "snapshot_id").unwrap();

    let task_store = repo.task_store().unwrap();
    let change_store = repo.change_store().unwrap();
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Adopting Task",
        "Land after the orphan boundary",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    let change = change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        &task_id,
        "Adopting Change",
        "main",
        None,
        Some(&orphan_snapshot_id),
    )
    .unwrap();
    let change_ref = required_string_field(&change, "change_ref").unwrap();
    fs::write(temp.path().join("history.txt"), "landed after orphan").unwrap();
    let landed = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local land after orphan"),
        false,
    )
    .unwrap();
    let landed_snapshot_id = required_string_field(&landed, "snapshot_id").unwrap();
    finish_fixture_revision(
        &repo,
        &change_ref,
        "main",
        &landed_snapshot_id,
        Some(&orphan_snapshot_id),
    )
    .unwrap();
    workflow_local_task_close_with_task_store(&task_store, &task_id, "completed").unwrap();

    let (entries, _) = workflow_local_history_entries(
        &repo,
        &change_ref,
        "main",
        &base_snapshot_id,
        &landed_snapshot_id,
    )
    .expect("adopted history");

    assert_eq!(entries.len(), 2);
    let adopting = entries.last().unwrap();
    assert_eq!(
        adopting["pre_land_target_snapshot_id"],
        first_landed_snapshot_id
    );
    assert_eq!(
        adopting["adopted_boundary_snapshot_ids"],
        json!([orphan_snapshot_id])
    );
    let chain: Vec<&str> = adopting["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["snapshot_id"].as_str())
        .collect();
    assert!(chain.contains(&orphan_snapshot_id.as_str()));
    assert!(chain.contains(&landed_snapshot_id.as_str()));
    assert_eq!(
        entries.first().unwrap()["adopted_boundary_snapshot_ids"],
        json!([])
    );
}

#[test]
fn history_promotion_collects_sixty_five_consecutive_local_lands_without_a_ceiling() {
    let (_temp, repo, base_snapshot_id, final_snapshot_id, final_change_ref) =
        local_land_fixture(65);

    let (entries, plan_artifacts) = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        &base_snapshot_id,
        &final_snapshot_id,
    )
    .expect("sixty-five-entry history");

    assert_eq!(entries.len(), 65);
    assert!(plan_artifacts.is_empty());
    assert_eq!(
        entries.first().unwrap()["pre_land_target_snapshot_id"],
        base_snapshot_id
    );
    assert_eq!(
        entries.last().unwrap()["landed_snapshot_id"],
        final_snapshot_id
    );
    assert!(entries
        .windows(2)
        .all(|pair| { pair[0]["landed_snapshot_id"] == pair[1]["pre_land_target_snapshot_id"] }));
}

#[test]
fn history_promotion_stages_sixty_five_entries_as_exact_sixty_four_plus_one() {
    let entries = (0..65)
        .map(|ordinal| {
            json!({
                "local_task_id": format!("LCT-{ordinal:04}"),
                "local_change_ref": format!("LCT-{ordinal:04}/C-01"),
                "pre_land_target_snapshot_id": format!("SNP-{ordinal}"),
                "landed_snapshot_id": format!("SNP-{}", ordinal + 1),
            })
        })
        .collect::<Vec<_>>();
    let first = workflow_staged_history_prepare_request(
        "history-promotion-v2:stable",
        "main",
        "SNP-0",
        "SNP-65",
        0,
        65,
        None,
        "ai_with_human_review",
        "staged history",
        &entries[..64],
    )
    .expect("first bounded stage");
    let second = workflow_staged_history_prepare_request(
        "history-promotion-v2:stable",
        "main",
        "SNP-0",
        "SNP-65",
        1,
        65,
        Some("RCT-64/C-01/P-02"),
        "ai_with_human_review",
        "staged history",
        &entries[64..],
    )
    .expect("final bounded stage");

    assert_eq!(first["entries"].as_array().unwrap().len(), 64);
    assert_eq!(first["stage_ordinal"], 0);
    assert_eq!(first["final_stage"], false);
    assert_eq!(first["stage_base_snapshot_id"], "SNP-0");
    assert_eq!(first["stage_revision_snapshot_id"], "SNP-64");
    assert!(first["previous_stage_patchset_id"].is_null());
    assert_eq!(second["entries"].as_array().unwrap().len(), 1);
    assert_eq!(second["stage_ordinal"], 1);
    assert_eq!(second["final_stage"], true);
    assert_eq!(second["stage_base_snapshot_id"], "SNP-64");
    assert_eq!(second["stage_revision_snapshot_id"], "SNP-65");
    assert_eq!(second["previous_stage_patchset_id"], "RCT-64/C-01/P-02");
    assert_ne!(first["idempotency_key"], second["idempotency_key"]);
    assert_eq!(
        first,
        workflow_staged_history_prepare_request(
            "history-promotion-v2:stable",
            "main",
            "SNP-0",
            "SNP-65",
            0,
            65,
            None,
            "ai_with_human_review",
            "staged history",
            &entries[..64],
        )
        .expect("deterministic stage replay")
    );
    assert!(workflow_staged_history_prepare_request(
        "history-promotion-v2:stable",
        "main",
        "SNP-0",
        "SNP-65",
        1,
        65,
        None,
        "ai_with_human_review",
        "staged history",
        &entries[64..],
    )
    .expect_err("continuation without predecessor must fail")
    .contains("predecessor"));
}

#[test]
fn history_promotion_recovers_empty_boundary_from_all_task_owned_snapshots() {
    let temp = tempdir().expect("history recovery tempdir");
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init history recovery repo");
    fs::write(temp.path().join("history.txt"), "base").expect("base fixture");
    let base = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("history recovery base"),
        false,
    )
    .expect("base snapshot");
    let base_snapshot_id = required_string_field(&base, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).expect("history runtime");
    let task_store = repo.task_store().unwrap();
    let change_store = repo.change_store().unwrap();
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Recover history boundary",
        "Prove Task-owned Snapshot lineage",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    let change = change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        &task_id,
        "Recover history boundary",
        "main",
        None,
        Some(&base_snapshot_id),
    )
    .unwrap();
    let change_ref = required_string_field(&change, "change_ref").unwrap();
    let feature_line = task_feature_line_name(&task_id).unwrap();
    create_local_line_with_line_store(
        &repo.line_store().unwrap(),
        &feature_line,
        Some(&base_snapshot_id),
        "2026-07-28T00:00:00Z",
    )
    .expect("create Task feature Line");

    fs::write(temp.path().join("history.txt"), "first").expect("first task snapshot fixture");
    let first = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        &feature_line,
        Some("first Task snapshot"),
        false,
    )
    .expect("first Task snapshot");
    bind_fixture_snapshot(
        &repo,
        &task_id,
        Some("C-01"),
        &required_string_field(&first, "snapshot_id").unwrap(),
    )
    .unwrap();
    fs::write(temp.path().join("history.txt"), "second").expect("second task snapshot fixture");
    let landed = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        &feature_line,
        Some("second Task snapshot"),
        false,
    )
    .expect("second Task snapshot");
    let landed_snapshot_id = required_string_field(&landed, "snapshot_id").unwrap();
    finish_fixture_revision(
        &repo,
        &change_ref,
        "main",
        &landed_snapshot_id,
        Some(&base_snapshot_id),
    )
    .unwrap();
    // Admit an exact legacy empty pre-target projection without performing a second apply.
    let receipt_path = repo.root.join(".ait/binary-db/land.bin");
    let mut receipt = fs::read(&receipt_path).unwrap();
    let accepted = receipt[24..28].to_vec();
    receipt[20..24].copy_from_slice(&accepted);
    fs::write(receipt_path, receipt).unwrap();
    workflow_local_task_close_with_task_store(&task_store, &task_id, "completed").unwrap();

    let (entries, _) = workflow_local_history_entries(
        &repo,
        &change_ref,
        "main",
        &base_snapshot_id,
        &landed_snapshot_id,
    )
    .expect("recover Task-owned boundary");

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["pre_land_target_snapshot_id"], base_snapshot_id);
    assert_eq!(
        entries[0]["pre_land_boundary_source"],
        "task_owned_snapshot_lineage_recovery"
    );
    assert_eq!(entries[0]["snapshots"].as_array().unwrap().len(), 2);
    let stored = workflow_local_change_read_with_change_store(&change_store, &change_ref).unwrap();
    assert_eq!(stored["pre_land_target_snapshot_id"], landed_snapshot_id);
}

#[test]
fn history_promotion_keeps_content_unchanged_task_owned_rebase_as_one_boundary() {
    let (_temp, repo, base, integrated, _) = local_land_fixture(1);
    let task_store = repo.task_store().unwrap();
    let change_store = repo.change_store().unwrap();
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Already integrated author",
        "Retain owned rebase lineage",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    let change = change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        &task_id,
        "Retain rebase receipt",
        "main",
        None,
        Some(&base),
    )
    .unwrap();
    let reference = required_string_field(&change, "change_ref").unwrap();
    let line = task_feature_line_name(&task_id).unwrap();
    create_local_line_with_line_store(
        &repo.line_store().unwrap(),
        &line,
        Some(&integrated),
        "2026-10-10T00:00:00Z",
    )
    .unwrap();
    let content = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .content_for_root(repo.workspace_root());
    let receipt = content
        .create_snapshot_content_with_parents_and_options(
            "fixture-ait",
            &line,
            std::slice::from_ref(&integrated),
            Some("Task-owned rebase receipt"),
            false,
            ait_core::local_snapshot::SnapshotAuthoringOptions {
                allow_unchanged_tree: true,
            },
        )
        .unwrap();
    let revision = required_string_field(&receipt, "snapshot_id").unwrap();
    assert_eq!(
        content
            .snapshot_tree_root_locator(&integrated)
            .unwrap()
            .root_tree_id,
        content
            .snapshot_tree_root_locator(&revision)
            .unwrap()
            .root_tree_id
    );
    finish_fixture_revision(&repo, &reference, "main", &revision, Some(&integrated)).unwrap();
    workflow_local_task_close_with_task_store(&task_store, &task_id, "completed").unwrap();
    let (entries, _) =
        workflow_local_history_entries(&repo, &reference, "main", &base, &revision).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[1]["local_change_ref"], reference);
    assert_eq!(entries[1]["pre_land_target_snapshot_id"], integrated);
    assert_eq!(entries[1]["landed_snapshot_id"], revision);
    assert_eq!(entries[1]["snapshots"].as_array().unwrap().len(), 1);
    assert_ne!(
        entries[0]["landed_snapshot_id"],
        entries[1]["landed_snapshot_id"]
    );
}

#[test]
fn history_promotion_rejects_genuinely_empty_boundary_at_change_fork() {
    let temp = tempdir().expect("empty history tempdir");
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init empty history repo");
    fs::write(temp.path().join("history.txt"), "base").expect("base fixture");
    let base = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("empty history base"),
        false,
    )
    .expect("base snapshot");
    let base_snapshot_id = required_string_field(&base, "snapshot_id").unwrap();
    fs::write(temp.path().join("history.txt"), "existing target").expect("target fixture");
    let existing_target = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("existing target snapshot"),
        false,
    )
    .expect("existing target snapshot");
    let existing_target_snapshot_id =
        required_string_field(&existing_target, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).expect("history runtime");
    let task_store = repo.task_store().unwrap();
    let change_store = repo.change_store().unwrap();
    let task = task_local_create_with_task_store(
        &task_store,
        "fixture-ait",
        "Reject empty history",
        "Do not invent a Land boundary",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    let change = change_local_create_with_change_store(
        &change_store,
        "fixture-ait",
        &task_id,
        "Reject empty history",
        "main",
        None,
        Some(&existing_target_snapshot_id),
    )
    .unwrap();
    let change_ref = required_string_field(&change, "change_ref").unwrap();
    finish_fixture_revision(
        &repo,
        &change_ref,
        "main",
        &existing_target_snapshot_id,
        Some(&existing_target_snapshot_id),
    )
    .unwrap();
    workflow_local_task_close_with_task_store(&task_store, &task_id, "completed").unwrap();

    let error = workflow_local_history_entries(
        &repo,
        &change_ref,
        "main",
        &base_snapshot_id,
        &existing_target_snapshot_id,
    )
    .expect_err("genuinely empty Land must fail closed");

    assert!(error.contains("genuinely empty Land boundary"));
    assert!(error.contains(&existing_target_snapshot_id));
}

#[test]
fn history_promotion_publishes_ambiguous_child_ids_by_exact_change_reference() {
    let (_temp, repo, base_snapshot_id, final_snapshot_id, final_change_ref) =
        ten_local_land_fixture();
    let (entries, _) = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        &base_snapshot_id,
        &final_snapshot_id,
    )
    .expect("ten-entry history");
    let response_entries = entries
        .iter()
        .enumerate()
        .map(|(ordinal, entry)| {
            let remote_task_id = format!("RT-{:04}", ordinal + 1);
            json!({
                "local_task_id": entry["local_task_id"],
                "local_change_id": entry["local_change_id"],
                "local_change_ref": entry["local_change_ref"],
                "task_id": remote_task_id,
                "change_ref": format!("{remote_task_id}/C-01"),
                "receipt_patchset_id": format!("{remote_task_id}/C-01/P-01"),
            })
        })
        .collect::<Vec<_>>();

    let publication_remote_name =
        contextual_publication_remote_name("camera-server").expect("configured Remote alias");
    let mappings = workflow_mark_history_published(
        &repo,
        publication_remote_name,
        &entries,
        &response_entries,
    )
    .expect("publish all exact history mappings");
    assert_eq!(mappings.len(), 10);
    for (ordinal, entry) in entries.iter().enumerate() {
        let exact_ref = required_string_field(entry, "local_change_ref").unwrap();
        let change_store = repo.change_store().unwrap();
        let change =
            workflow_local_change_read_with_change_store(&change_store, &exact_ref).unwrap();
        assert_eq!(change["publication_state"], "published");
        assert_eq!(change["published_remote_name"], "origin");
        assert_eq!(
            change["published_change_id"],
            format!("RT-{:04}/C-01", ordinal + 1)
        );
    }
    assert_eq!(
        task_land_exact_atomic_reference(&repo, &final_change_ref).unwrap(),
        "RT-0010/C-01"
    );
}

#[test]
fn history_promotion_retry_heals_task_first_publication_interruption() {
    let (_temp, repo, base_snapshot_id, final_snapshot_id, final_change_ref) =
        ten_local_land_fixture();
    let (initial_entries, _) = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        &base_snapshot_id,
        &final_snapshot_id,
    )
    .expect("initial ten-entry history");
    let interrupted_entry = &initial_entries[3];
    let interrupted_task_id =
        required_string_field(interrupted_entry, "local_task_id").expect("local task");
    let task_store = repo.task_store().expect("task store");
    task_local_mark_published_with_task_store(
        &task_store,
        &interrupted_task_id,
        Some("origin"),
        Some("RT-0004"),
    )
    .expect("simulate task-first publication write");

    let (retry_entries, _) = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        &base_snapshot_id,
        &final_snapshot_id,
    )
    .expect("partial local write remains retryable");
    assert_eq!(retry_entries.len(), 10);
    assert_eq!(
        retry_entries[3]["publication_recovery_required"],
        json!(true)
    );
    let retry_request_entries =
        workflow_history_prepare_entries(&repo, &json!({"history_entries": retry_entries.clone()}))
            .expect("build task-first retry admission entries");
    assert_eq!(
        retry_request_entries[3]["expected_remote_task_id"],
        json!("RT-0004")
    );
    assert_eq!(
        retry_request_entries[3]["expected_remote_change_ref"],
        JsonValue::Null
    );
    assert_eq!(
        retry_request_entries[0]["expected_remote_task_id"],
        JsonValue::Null
    );
    let response_entries = retry_entries
        .iter()
        .enumerate()
        .map(|(ordinal, entry)| {
            let remote_task_id = format!("RT-{:04}", ordinal + 1);
            json!({
                "local_task_id": entry["local_task_id"],
                "local_change_id": entry["local_change_id"],
                "local_change_ref": entry["local_change_ref"],
                "task_id": remote_task_id,
                "change_ref": format!("{remote_task_id}/C-01"),
                "receipt_patchset_id": format!("{remote_task_id}/C-01/P-01"),
            })
        })
        .collect::<Vec<_>>();

    workflow_mark_history_published(&repo, "origin", &retry_entries, &response_entries)
        .expect("idempotent replay heals the interrupted mapping");
    let healed_change = workflow_local_change_read_with_change_store(
        &repo.change_store().expect("change store"),
        &required_string_field(interrupted_entry, "local_change_ref").expect("local change"),
    )
    .expect("healed change");
    assert_eq!(healed_change["publication_state"], json!("published"));
    assert_eq!(healed_change["published_change_id"], json!("RT-0004/C-01"));

    let (published_entries, _) = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        &base_snapshot_id,
        &final_snapshot_id,
    )
    .expect("published history remains collectable");
    let published_request_entries =
        workflow_history_prepare_entries(&repo, &json!({"history_entries": published_entries}))
            .expect("build fully published admission entries");
    assert_eq!(
        published_request_entries[3]["expected_remote_task_id"],
        json!("RT-0004")
    );
    assert_eq!(
        published_request_entries[3]["expected_remote_change_ref"],
        json!("RT-0004/C-01")
    );
}

#[test]
fn history_promotion_mapping_mismatch_fails_before_any_local_publication_write() {
    let (_temp, repo, base_snapshot_id, final_snapshot_id, final_change_ref) =
        ten_local_land_fixture();
    let (initial_entries, _) = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        &base_snapshot_id,
        &final_snapshot_id,
    )
    .expect("initial ten-entry history");
    let interrupted_task_id = required_string_field(&initial_entries[3], "local_task_id").unwrap();
    task_local_mark_published_with_task_store(
        &repo.task_store().unwrap(),
        &interrupted_task_id,
        Some("origin"),
        Some("RT-0004"),
    )
    .expect("simulate task-first publication write");
    let (retry_entries, _) = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        &base_snapshot_id,
        &final_snapshot_id,
    )
    .expect("collect retry history");
    let mut response_entries = retry_entries
        .iter()
        .enumerate()
        .map(|(ordinal, entry)| {
            let remote_task_id = format!("RT-{:04}", ordinal + 1);
            json!({
                "local_task_id": entry["local_task_id"],
                "local_change_id": entry["local_change_id"],
                "local_change_ref": entry["local_change_ref"],
                "task_id": remote_task_id,
                "change_ref": format!("{remote_task_id}/C-01"),
                "receipt_patchset_id": format!("{remote_task_id}/C-01/P-01"),
            })
        })
        .collect::<Vec<_>>();
    response_entries[3]["task_id"] = json!("RT-9999");
    response_entries[3]["change_ref"] = json!("RT-9999/C-01");

    let error = workflow_mark_history_published(&repo, "origin", &retry_entries, &response_entries)
        .expect_err("immutable mapping replacement must fail before local writes");
    assert!(error.contains("immutable publication mapping"), "{error}");

    let first_task_id = required_string_field(&retry_entries[0], "local_task_id").unwrap();
    let first_change_ref = required_string_field(&retry_entries[0], "local_change_ref").unwrap();
    let first_task =
        workflow_local_task_read_with_task_store(&repo.task_store().unwrap(), &first_task_id)
            .unwrap();
    let first_change = workflow_local_change_read_with_change_store(
        &repo.change_store().unwrap(),
        &first_change_ref,
    )
    .unwrap();
    assert_eq!(first_task["publication_state"], json!("local_draft"));
    assert_eq!(first_change["publication_state"], json!("local_draft"));
    let interrupted_task =
        workflow_local_task_read_with_task_store(&repo.task_store().unwrap(), &interrupted_task_id)
            .unwrap();
    let interrupted_change = workflow_local_change_read_with_change_store(
        &repo.change_store().unwrap(),
        &required_string_field(&retry_entries[3], "local_change_ref").unwrap(),
    )
    .unwrap();
    assert_eq!(interrupted_task["published_task_id"], json!("RT-0004"));
    assert_eq!(
        interrupted_change["publication_state"],
        json!("local_draft")
    );
}

#[test]
fn history_promotion_response_validation_rejects_echo_owner_and_duplicate_conflicts() {
    let candidates = vec![
        json!({
            "local_task_id": "LCT-0001",
            "local_change_id": "C-01",
            "local_change_ref": "LCT-0001/C-01",
            "task": {"publication_state": "local_draft"},
            "change": {"publication_state": "local_draft"},
        }),
        json!({
            "local_task_id": "LCT-0002",
            "local_change_id": "C-01",
            "local_change_ref": "LCT-0002/C-01",
            "task": {"publication_state": "local_draft"},
            "change": {"publication_state": "local_draft"},
        }),
    ];
    let responses = vec![
        json!({
            "local_task_id": "LCT-0001",
            "local_change_id": "C-01",
            "local_change_ref": "LCT-0001/C-01",
            "task_id": "RCT-1001",
            "change_ref": "RCT-1001/C-01",
        }),
        json!({
            "local_task_id": "LCT-0002",
            "local_change_id": "C-01",
            "local_change_ref": "LCT-0002/C-01",
            "task_id": "RCT-1002",
            "change_ref": "RCT-1002/C-01",
        }),
    ];

    let mut wrong_echo = responses.clone();
    wrong_echo[0]["local_task_id"] = json!("LCT-OTHER");
    let echo_error = workflow_validate_history_publication_response(&candidates, &wrong_echo)
        .expect_err("wrong echoed Local identity must fail");
    assert!(echo_error.contains("local_task_id differs"));

    let mut wrong_owner = responses.clone();
    wrong_owner[0]["change_ref"] = json!("RCT-OTHER/C-01");
    let owner_error = workflow_validate_history_publication_response(&candidates, &wrong_owner)
        .expect_err("wrong Remote Change owner must fail");
    assert!(owner_error.contains("is not owned by Task"));

    let mut duplicate_owner = responses;
    duplicate_owner[1]["task_id"] = json!("RCT-1001");
    duplicate_owner[1]["change_ref"] = json!("RCT-1001/C-02");
    let duplicate_error =
        workflow_validate_history_publication_response(&candidates, &duplicate_owner)
            .expect_err("duplicate Remote Task owner must fail");
    assert!(duplicate_error.contains("repeats Remote Task"));
}

#[test]
fn history_promotion_deduplicates_plan_artifacts_before_remote_sync() {
    let paths = workflow_unique_history_plan_artifact_paths([
        "docs/sprints/b.md".to_string(),
        "docs/sprints/a.md".to_string(),
        "docs/sprints/b.md".to_string(),
        "docs/sprints/a.md".to_string(),
    ]);

    assert_eq!(
        paths,
        vec![
            "docs/sprints/a.md".to_string(),
            "docs/sprints/b.md".to_string()
        ]
    );
}

#[test]
fn history_promotion_deduplicates_exact_plan_publications_in_plan_id_order() {
    let publications = workflow_unique_history_plan_publications([
        ("PR-700".to_string(), "docs/sprints/b.md".to_string()),
        ("PR-649".to_string(), "docs/sprints/a.md".to_string()),
        ("PR-700".to_string(), "docs/sprints/b.md".to_string()),
    ])
    .expect("exact Plan publications should deduplicate");

    assert_eq!(
        publications,
        vec![
            ("PR-649".to_string(), "docs/sprints/a.md".to_string()),
            ("PR-700".to_string(), "docs/sprints/b.md".to_string()),
        ]
    );

    let error = workflow_unique_history_plan_publications([
        ("PR-649".to_string(), "docs/sprints/a.md".to_string()),
        ("PR-649".to_string(), "docs/sprints/other.md".to_string()),
    ])
    .expect_err("one exact Plan cannot resolve to two head paths");
    assert!(error.contains("conflicting head artifact paths"));
}

#[test]
fn history_promotion_rejects_a_local_land_gap() {
    let (_temp, repo, _base_snapshot_id, final_snapshot_id, final_change_ref) =
        ten_local_land_fixture();

    let error = workflow_local_history_entries(
        &repo,
        &final_change_ref,
        "main",
        "SNP-NOT-A-BOUNDARY",
        &final_snapshot_id,
    )
    .expect_err("gap must fail closed");

    assert!(error.contains("gap"));
}

#[test]
fn same_head_atomic_sync_skips_line_write_and_main_seed_refresh() {
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
    fs::write(temp.path().join("same.txt"), "same").unwrap();
    let snapshot = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("same head"),
        false,
    )
    .unwrap();
    let snapshot_id = required_string_field(&snapshot, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let result = workflow_attach_local_land_sync_from_atomic_response(
        &repo,
        "RCT-1",
        &json!({"status": "succeeded"}),
        "main",
        &snapshot_id,
    )
    .unwrap();
    assert_eq!(result["local_sync"]["status"], "already_synced");
    assert_eq!(result["local_sync"]["same_head"], true);
    assert_eq!(
        result["local_sync"]["workspace_restore"]["reason"],
        "already_at_trusted_local_landed_snapshot"
    );

    let mut output = json!({
        "apply_status": "done",
        "target_line": "main",
        "landed_snapshot_id": snapshot_id.clone(),
        "local_line_sync": result["local_sync"].clone(),
    });
    let output_snapshot_id = output["landed_snapshot_id"].as_str().map(str::to_string);
    task_land_attach_cli_main_seed_sync(&repo, &mut output, "main", output_snapshot_id.as_deref());
    assert_eq!(output["main_seed_sync"]["status"], "skipped");
    assert_eq!(
        output["main_seed_sync"]["reason"],
        "already_at_trusted_local_landed_snapshot"
    );
}

#[test]
fn descendant_atomic_sync_preserves_the_newer_local_head_and_main_seed() {
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
    fs::write(temp.path().join("line.txt"), "remote landed\n").unwrap();
    let landed = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("remote landed"),
        false,
    )
    .unwrap();
    let landed_snapshot_id = required_string_field(&landed, "snapshot_id").unwrap();
    fs::write(temp.path().join("line.txt"), "newer local work\n").unwrap();
    let local = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("newer local descendant"),
        false,
    )
    .unwrap();
    let local_snapshot_id = required_string_field(&local, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();

    let result = workflow_attach_local_land_sync_from_atomic_response(
        &repo,
        "RCT-1",
        &json!({"status": "succeeded"}),
        "main",
        &landed_snapshot_id,
    )
    .unwrap();

    assert_eq!(
        local_line_head_snapshot_id(&repo, "main")
            .unwrap()
            .as_deref(),
        Some(local_snapshot_id.as_str())
    );
    assert_eq!(result["local_sync"]["status"], "local_descendant_preserved");
    assert_eq!(result["local_sync"]["same_head"], false);
    assert_eq!(
        result["local_sync"]["local_head_contains_landed_snapshot"],
        true
    );
    assert_eq!(result["local_sync"]["auto_rebase"], false);
    assert_eq!(
        result["local_sync"]["line_head_snapshot_id"],
        local_snapshot_id
    );
    assert_eq!(
        result["local_sync"]["workspace_restore"]["reason"],
        "local_head_already_contains_landed_snapshot"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("line.txt")).unwrap(),
        "newer local work\n"
    );

    let mut output = json!({
        "apply_status": "done",
        "target_line": "main",
        "landed_snapshot_id": landed_snapshot_id,
        "local_line_sync": result["local_sync"].clone(),
    });
    let output_landed_snapshot_id = output["landed_snapshot_id"]
        .as_str()
        .map(str::to_string)
        .unwrap();
    task_land_attach_cli_main_seed_sync(
        &repo,
        &mut output,
        "main",
        Some(&output_landed_snapshot_id),
    );
    assert_eq!(output["main_seed_sync"]["status"], "skipped");
    assert_eq!(
        output["main_seed_sync"]["reason"],
        "local_head_already_contains_landed_snapshot"
    );
    assert_eq!(output["main_seed_sync"]["snapshot_id"], local_snapshot_id);
}

#[test]
fn divergent_atomic_sync_retains_the_existing_remote_land_update_path() {
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
    fs::write(temp.path().join("line.txt"), "base\n").unwrap();
    let base = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("shared base"),
        false,
    )
    .unwrap();
    let base_snapshot_id = required_string_field(&base, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    create_local_line_with_line_store(
        &repo.line_store().unwrap(),
        "remote-candidate",
        Some(&base_snapshot_id),
        "2026-09-01T00:00:00Z",
    )
    .unwrap();

    fs::write(temp.path().join("line.txt"), "newer local work\n").unwrap();
    let local = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "main",
        Some("local branch"),
        false,
    )
    .unwrap();
    let local_snapshot_id = required_string_field(&local, "snapshot_id").unwrap();
    fs::write(temp.path().join("line.txt"), "remote branch\n").unwrap();
    let remote = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        "remote-candidate",
        Some("remote branch"),
        false,
    )
    .unwrap();
    let remote_snapshot_id = required_string_field(&remote, "snapshot_id").unwrap();
    assert!(snapshot_distance_if_ancestor(
        &repo,
        Some(&remote_snapshot_id),
        Some(&local_snapshot_id)
    )
    .unwrap()
    .is_none());

    let result = workflow_attach_local_land_sync_from_atomic_response(
        &repo,
        "RCT-1",
        &json!({"status": "succeeded"}),
        "main",
        &remote_snapshot_id,
    )
    .unwrap();

    assert_eq!(
        local_line_head_snapshot_id(&repo, "main")
            .unwrap()
            .as_deref(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(result["local_sync"]["status"], "synced");
    assert_eq!(
        result["local_sync"]["local_head_contains_landed_snapshot"],
        false
    );
    assert_eq!(result["local_sync"]["auto_rebase"], true);
    assert_eq!(
        fs::read_to_string(temp.path().join("line.txt")).unwrap(),
        "remote branch\n"
    );
}

// These unit fixtures build immutable content directly. Add the exact intended
// owner through the production coordinated writer before exercising finish.
fn finish_fixture_revision(
    repo: &RepoRuntime,
    change_ref: &str,
    target: &str,
    snapshot_id: &str,
    pre: Option<&str>,
) -> Result<JsonValue, String> {
    let (task, change) = change_ref
        .rsplit_once('/')
        .ok_or("fixture requires exact Change")?;
    bind_fixture_snapshot(repo, task, Some(change), snapshot_id)?;
    super::workflow_local_change_land_with_change_store(
        &repo.change_store()?,
        change_ref,
        target,
        snapshot_id,
        pre,
    )
}

pub(super) fn bind_fixture_snapshot(
    repo: &RepoRuntime,
    task: &str,
    change: Option<&str>,
    snapshot_id: &str,
) -> Result<(), String> {
    use ait_core::binary_db::BinaryDbCommandScope;
    use ait_core::content_binary_db::{
        BinaryDbContentWriteCoordinator, BinaryDbSnapshotWriteInput,
    };
    let stores = repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>();
    let content = stores.content();
    let snapshot = content
        .snapshots()
        .snapshot_by_id(snapshot_id)?
        .ok_or("fixture Snapshot missing")?;
    let root = content.snapshot_tree_root_locator(snapshot_id)?;
    let binding = ait_core::workflow_binary_db::SnapshotWorkflowBinding {
        task_id: task.into(),
        change_id: change.unwrap_or_default().into(),
        line_name: snapshot.line_name.clone(),
    };
    BinaryDbContentWriteCoordinator::new(
        content.blobs(),
        content.object_packs(),
        content.tree_packs(),
        content.trees(),
        content.snapshots(),
    )
    .record_snapshot_with_workflow_binding(
        BinaryDbCommandScope::ContentWrite,
        &BinaryDbSnapshotWriteInput {
            snapshot_id: snapshot.snapshot_id,
            parent_snapshot_ids: snapshot.parent_snapshot_ids,
            root_tree_pack_id: root.root_tree_pack_id,
            root_entry_ordinal: root.root_entry_ordinal,
            manifest_hash: snapshot.manifest_hash,
            message: snapshot.message,
            line_name: snapshot.line_name,
            snapshot_kind: snapshot.snapshot_kind,
            file_count: snapshot.file_count,
            total_bytes: snapshot.total_bytes,
            created_at: snapshot.created_at,
        },
        &binding,
        &repo.repo_name(),
        &repo.id_namespace_prefix(),
    )?;
    Ok(())
}

fn task_only_receipt_fixture() -> (tempfile::TempDir, RepoRuntime, String, String, String) {
    let temp = tempdir().unwrap();
    init_repo(&InitRequest {
        root: temp.path().into(),
        name: Some("fixture-ait".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    fs::write(temp.path().join("history.txt"), "base").unwrap();
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
    let store = repo.task_store().unwrap();
    let task = task_local_create_with_task_store(
        &store,
        "fixture-ait",
        "Initial work",
        "Promote Task receipt",
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let task_id = required_string_field(&task, "task_id").unwrap();
    fs::write(temp.path().join("history.txt"), "initial Task result").unwrap();
    let snapshot = create_local_snapshot(
        temp.path().to_str().unwrap(),
        "fixture-ait",
        "main",
        Some("initial"),
        false,
    )
    .unwrap();
    let candidate = required_string_field(&snapshot, "snapshot_id").unwrap();
    bind_fixture_snapshot(&repo, &task_id, None, &candidate).unwrap();
    set_local_line_head(&repo, "main", Some(&base_id)).unwrap();
    store
        .finish_local_task(&task_id, &candidate, "main", Some(&base_id), None, None)
        .unwrap();
    (temp, repo, base_id, candidate, task_id)
}

#[test]
fn task_only_receipt_promotes_optional_change_without_fabrication() {
    let (_temp, repo, base_id, candidate, task_id) = task_only_receipt_fixture();
    let store = repo.task_store().unwrap();
    assert!(store.changes_for_task(&task_id).unwrap().is_empty());
    assert_eq!(
        workflow_task_change_reference(&repo, &task_id, Some("origin")).unwrap(),
        task_id
    );
    let (entries, _) =
        workflow_local_history_entries(&repo, &task_id, "main", &base_id, &candidate).unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0]["change"].is_null());
    assert!(entries[0]["local_change_ref"].is_null());
    let prepared =
        workflow_history_prepare_entries(&repo, &json!({"history_entries":entries})).unwrap();
    assert!(prepared[0]["change"].is_null());
    assert!(prepared[0]["expected_remote_change_ref"].is_null());
    let response = vec![
        json!({"local_task_id":task_id,"local_change_id":null,"local_change_ref":null,"task_id":"RT-0099","change_ref":null,"receipt_patchset_id":"RT-0099/P-01"}),
    ];
    workflow_mark_history_published(&repo, "origin", &entries, &response).unwrap();
    let task = TaskStore::get_task(&store, &task_id).unwrap();
    assert_eq!(task["published_task_id"], "RT-0099");
    assert!(store.changes_for_task(&task_id).unwrap().is_empty());
    let mut forged = response;
    forged[0]["change_ref"] = json!("RT-0099/C-01");
    assert!(workflow_validate_history_publication_response(&entries, &forged).is_err());
}

fn history_dispatch_fixture(
    legacy: bool,
) -> (
    tempfile::TempDir,
    RepoRuntime,
    JsonValue,
    JsonValue,
    JsonValue,
) {
    let (temp, mut repo, base, revision, _) = local_land_fixture(1);
    repo.config.insert("repository_index".into(), json!(7));
    let entry = json!({"local_task_id":"LCT-0001", "local_change_id":null,"local_change_ref":null,
        "task":{"publication_state":"published","published_task_id":"RCT-0001"},"change":null,
        "pre_land_target_snapshot_id":base,"landed_snapshot_id":revision,"landed_at_s":1});
    let candidate = json!({"base_snapshot_id":base,"revision_snapshot_id":revision,"target_line":"main",
        "state":{"task":{"published_task_id":"RCT-0001"}},"history_entries":[entry]});
    let receipt = json!({"local_task_id":"LCT-0001","local_change_id":if legacy {json!("C-01")}else{JsonValue::Null},
        "local_change_ref":if legacy {json!("LCT-0001/C-01")}else{JsonValue::Null},
        "task_id":"RCT-0001","change_ref":if legacy {json!("RCT-0001/C-01")}else{JsonValue::Null},
        "pre_land_target_snapshot_id":base,"landed_snapshot_id":revision,"landed_at_s":1});
    let manifest = json!({"contract":if legacy {"ait-history-promotion/v1"}else{"ait-history-promotion/v3"},
        "base_snapshot_id":base,"revision_snapshot_id":revision,"target_line":"main","entries":[receipt],
        "aggregate":{"task_id":"RCT-0001","change_ref":if legacy {json!("RCT-0001/C-01")}else{JsonValue::Null},
            "patchset_id":if legacy {"RCT-0001/C-01/P-02"}else{"RCT-0001/P-02"}}});
    let state = json!({"task":{"task_id":"RCT-0001","status":"active","current_patchset_id":"RCT-0001/P-02"},
        "patchset":{"task_id":"RCT-0001","change_id":null,"change_ref":null,"patchset_id":"RCT-0001/P-02",
            "base_snapshot_id":base,"revision_snapshot_id":revision,"source_kind":"history_promotion_aggregate",
            "summary":format!("ait-history-promotion/v1 {manifest}"),"withdrawn":false,"invalidated":false}});
    (temp, repo, candidate, state, manifest)
}

#[test]
fn history_dispatch_legacy_batch_requires_prepare_while_exact_v3_reuses() {
    for legacy in [true, false] {
        let (_temp, repo, candidate, state, _) = history_dispatch_fixture(legacy);
        let original = state.clone();
        for _ in 0..2 {
            assert_eq!(
                workflow_history_remote_is_prepared(&repo, &candidate, &state).unwrap(),
                !legacy
            );
        }
        assert_eq!(
            state, original,
            "read-only dispatch must not rewrite provenance"
        );
    }
}

#[test]
fn history_dispatch_rejects_unknown_stale_foreign_and_partial_batches() {
    for legacy in [true, false] {
        let (_temp, repo, candidate, state, manifest) = history_dispatch_fixture(legacy);
        for case in 0..14 {
            let mut candidate = candidate.clone();
            let mut state = state.clone();
            let mut manifest = manifest.clone();
            match case {
                0 => manifest["contract"] = json!("ait-history-promotion/v99"),
                1 => state["task"]["current_patchset_id"] = json!("RCT-0001/P-03"),
                2 => manifest["aggregate"]["task_id"] = json!("RCT-OTHER"),
                3 => manifest["base_snapshot_id"] = json!("SNP-FOREIGN"),
                4 => manifest["entries"][0]["landed_at_s"] = json!(2),
                5 => manifest["entries"][0]["task_id"] = json!("RCT-OTHER"),
                6 => manifest["entries"] = json!([]),
                7 => state["patchset"]["source_kind"] = json!("ordinary"),
                8 => state["patchset"]["invalidated"] = json!(true),
                9 => manifest["aggregate"]["patchset_id"] = json!("RCT-0001/C-01/P-03"),
                10 => manifest["stage_ordinal"] = json!(0),
                11 => candidate["history_entries"][0]["local_change_ref"] = json!("LCT-0001/C-01"),
                12 => state["task"]["status"] = json!("completed"),
                _ => state["freshness"] = json!({"base_is_fresh":false}),
            }
            state["patchset"]["summary"] = json!(format!("ait-history-promotion/v1 {manifest}"));
            assert!(
                workflow_history_remote_is_prepared(&repo, &candidate, &state).is_err(),
                "legacy={legacy} case={case}"
            );
        }
    }
}

#[test]
fn history_dispatch_keeps_successful_receipt_authority_and_rejects_terminal_inference() {
    let (_temp, repo, candidate, mut state, _) = history_dispatch_fixture(true);
    state["task"]["status"] = json!("completed");
    assert!(workflow_history_remote_is_prepared(&repo, &candidate, &state).is_err());
    state["task"]["current_patchset_id"] = json!("RCT-0001/P-09");
    state["patchset_source"] = json!("accepted_task_receipt");
    state["landing_summary"] =
        json!({"status":"succeeded","task_id":"RCT-0001","patchset_id":"RCT-0001/P-02"});
    assert!(workflow_history_remote_is_prepared(&repo, &candidate, &state).unwrap());
    state["landing_summary"]["patchset_id"] = json!("RCT-0001/P-03");
    assert!(workflow_history_remote_is_prepared(&repo, &candidate, &state).is_err());
}

#[test]
fn history_dispatch_versioned_retry_identity_and_guarded_refresh_stay_exact() {
    let (_temp, repo, candidate, state, _) = history_dispatch_fixture(false);
    let entries = candidate["history_entries"].as_array().unwrap();
    let base = candidate["base_snapshot_id"].as_str().unwrap();
    let revision = candidate["revision_snapshot_id"].as_str().unwrap();
    let key = workflow_history_idempotency_key(&repo, "main", base, revision, entries).unwrap();
    assert!(key.starts_with("history-promotion-v3:"));
    assert_eq!(
        key,
        workflow_history_idempotency_key(&repo, "main", base, revision, entries).unwrap()
    );
    assert_ne!(
        key,
        workflow_history_idempotency_key(&repo, "main", base, "SNP-OTHER", entries).unwrap()
    );
    let identity = workflow_history_candidate_identity(&repo, &state["patchset"]).unwrap();
    assert!(validate_guarded_action(Some(&identity), "prepare_final_snapshot_promotion").is_err());
    assert!(validate_guarded_action(Some(&identity), "refresh_patchset").is_err());
    let mut newer = state["patchset"].clone();
    newer["patchset_id"] = json!("RCT-0001/P-03");
    assert!(identity.validate_patchset(7, &newer).is_err());
}

#[test]
fn history_dispatch_reuses_supported_descendant_correction_and_preserves_contract_prefix() {
    let (temp, repo, candidate, mut state, mut manifest) = history_dispatch_fixture(false);
    assert!(workflow_supported_correction_manifest(&state["patchset"]).is_ok());
    fs::write(temp.path().join("history.txt"), "review correction").unwrap();
    let correction = create_local_snapshot(
        temp.path().to_str().unwrap(),
        "fixture-ait",
        "main",
        Some("correction"),
        false,
    )
    .unwrap();
    let revision = required_string_field(&correction, "snapshot_id").unwrap();
    manifest["contract"] = json!("ait-history-promotion-correction/v2");
    manifest["correction"] =
        json!({"receipt_revision_snapshot_id":candidate["revision_snapshot_id"]});
    manifest["revision_snapshot_id"] = json!(revision);
    state["patchset"]["revision_snapshot_id"] = json!(revision);
    state["patchset"]["summary"] = json!(format!("ait-history-promotion/v1 {manifest}"));
    assert!(workflow_supported_correction_manifest(&state["patchset"]).is_ok());
    assert!(workflow_history_remote_is_prepared(&repo, &candidate, &state).unwrap());
    for contract in [
        "ait-history-promotion/v1",
        "ait-history-promotion-correction/v1",
        "ait-history-promotion-stage/v2",
        "unknown",
    ] {
        manifest["contract"] = json!(contract);
        state["patchset"]["summary"] = json!(format!("ait-history-promotion/v1 {manifest}"));
        assert!(workflow_supported_correction_manifest(&state["patchset"]).is_err());
    }
    manifest["contract"] = json!("ait-history-promotion/v3");
    manifest["stage_ordinal"] = json!(0);
    state["patchset"]["summary"] = json!(format!("ait-history-promotion/v1 {manifest}"));
    assert!(workflow_supported_correction_manifest(&state["patchset"]).is_err());
}

#[test]
fn history_dispatch_reuses_exact_final_stage_without_enabling_staged_correction() {
    let (_temp, repo, mut candidate, mut state, mut manifest) = history_dispatch_fixture(false);
    let base = required_string_field(&candidate, "base_snapshot_id").unwrap();
    let revision = required_string_field(&candidate, "revision_snapshot_id").unwrap();
    let mut previous = base.clone();
    let mut entries = Vec::new();
    for n in 1..=65 {
        let next = if n == 65 {
            revision.clone()
        } else {
            format!("SNP-STAGE-{n}")
        };
        entries.push(json!({"local_task_id":format!("LCT-{n:04}"),"local_change_id":null,"local_change_ref":null,
            "task":{"publication_state":"published","published_task_id":format!("RCT-{n:04}")},"change":null,
            "pre_land_target_snapshot_id":previous,"landed_snapshot_id":next,"landed_at_s":n}));
        previous = next;
    }
    let promotion_id =
        workflow_staged_history_promotion_id(&repo, "main", &base, &revision, &entries).unwrap();
    let mut receipt = entries[64].clone();
    receipt["task_id"] = json!("RCT-0065");
    receipt["change_ref"] = JsonValue::Null;
    manifest["entries"] = json!([receipt]);
    manifest["aggregate"]["task_id"] = json!("RCT-0065");
    manifest["aggregate"]["patchset_id"] = json!("RCT-0065/P-02");
    manifest["promotion_id"] = json!(promotion_id);
    manifest["stage_ordinal"] = json!(1);
    manifest["total_entry_count"] = json!(65);
    manifest["final_stage"] = json!(true);
    manifest["stage_base_snapshot_id"] = entries[64]["pre_land_target_snapshot_id"].clone();
    manifest["stage_revision_snapshot_id"] = json!(revision);
    candidate["history_entries"] = json!(entries);
    candidate["state"]["task"]["published_task_id"] = json!("RCT-0065");
    state["task"]["task_id"] = json!("RCT-0065");
    state["task"]["current_patchset_id"] = json!("RCT-0065/P-02");
    state["patchset"]["task_id"] = json!("RCT-0065");
    state["patchset"]["patchset_id"] = json!("RCT-0065/P-02");
    state["patchset"]["summary"] = json!(format!("ait-history-promotion/v1 {manifest}"));
    assert!(workflow_history_remote_is_prepared(&repo, &candidate, &state).unwrap());
    assert!(workflow_supported_correction_manifest(&state["patchset"]).is_err());
    candidate["history_entries"][0]["local_task_id"] = json!("LCT-FOREIGN");
    assert!(workflow_history_remote_is_prepared(&repo, &candidate, &state).is_err());
}

#[test]
fn published_local_ready_reaches_history_dispatch_and_exact_retries_keep_remote_identity() {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tiny_http::{Response, Server};
    for (legacy, direct_remote) in [(true, false), (false, false), (true, true)] {
        let (temp, mut repo, base, revision, local_task) = task_only_receipt_fixture();
        task_local_mark_published_with_task_store(
            &repo.task_store().unwrap(),
            &local_task,
            Some("origin"),
            Some("RT-0099"),
        )
        .unwrap();
        let (entries, _) =
            workflow_local_history_entries(&repo, &local_task, "main", &base, &revision).unwrap();
        let mut receipt = entries[0].clone();
        receipt["task_id"] = json!("RT-0099");
        receipt["change_ref"] = JsonValue::Null;
        if legacy {
            receipt["local_change_ref"] = json!(format!("{local_task}/C-01"));
        }
        let id = if legacy {
            "RT-0099/P-02"
        } else {
            "RT-0099/P-03"
        };
        let manifest = json!({"contract":if legacy {"ait-history-promotion/v1"} else {"ait-history-promotion/v3"},
            "base_snapshot_id":base,"revision_snapshot_id":revision,"target_line":"main","entries":[receipt],
            "aggregate":{"task_id":"RT-0099","change_ref":null,"patchset_id":id}});
        let patchset = json!({"task_id":"RT-0099","change_id":null,"change_ref":null,"patchset_id":id,
            "base_snapshot_id":base,"revision_snapshot_id":revision,"source_kind":"history_promotion_aggregate",
            "summary":format!("ait-history-promotion/v1 {manifest}"),"author_mode":"ai_with_human_review",
            "ci":{"run_seq":1,"completed_at_s":1,"tests_status":"pass","lint_status":"none","suite_result_count":1,"blocking_failure_count":0}});
        let server = Server::http("127.0.0.1:0").unwrap();
        let address = server.server_addr().to_ip().unwrap();
        repo.config.insert("repository_index".into(), json!(7));
        repo.config.insert("default_remote".into(), json!("origin"));
        repo.config.insert("remotes".into(), json!({"origin":{"remote_id":1,"url":format!("http://{address}"),"repo_name":"fixture-ait","created_at":"2026-06-20T00:00:00Z"}}));
        fs::write(
            temp.path().join(".ait/config.json"),
            JsonValue::Object(repo.config.clone()).to_string(),
        )
        .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let patchset_id = id.to_string();
        let handle = std::thread::spawn(move || {
            while let Some(request) = server.recv_timeout(Duration::from_millis(500)).unwrap() {
                let method = request.method().as_str().to_string();
                let url = request.url().to_string();
                let path = url.split('?').next().unwrap();
                log.lock().unwrap().push((method.clone(), url.clone()));
                let value = if path.ends_with("/tasks/RT-0099") {
                    json!({"task_id":"RT-0099","status":"active","successful_land":null,"current_patchset_id":patchset_id})
                } else if path.ends_with("/lines/main") {
                    json!({"line_name":"main","status":"active","head_snapshot_id":base})
                } else if path.ends_with("/changes/RT-0099/reviews") {
                    json!({"task_id":"RT-0099","change_id":null,"change_ref":null,"current_patchset_id":patchset_id,"reviews":[]})
                } else if path.ends_with("/attestation") {
                    json!({"task_id":"RT-0099","change_id":null,"change_ref":null,"patchset_id":patchset_id,"attestation_id":format!("{patchset_id}/AT-01"),"author_mode":"ai_with_human_review","verification_state":"pass","require_tests_pass":true,"require_lint_pass":false,"require_human_review":false,"revoked":false,"ci_backed":true,"evaluation_summary":{"tests":"pass","lint":"none"}})
                } else if path.ends_with("/policy") {
                    json!({"patchset_id":patchset_id,"decision":"pass","checks":[]})
                } else if path.ends_with(&format!("/patchsets/{}", patchset_id.replace('/', "%2F")))
                {
                    patchset.clone()
                } else {
                    json!({"error":format!("unexpected {method} {url}")})
                };
                let status = if value.get("error").is_some() {
                    500
                } else {
                    200
                };
                request
                    .respond(
                        Response::from_string(value.to_string())
                            .with_status_code(status)
                            .with_header(
                                tiny_http::Header::from_bytes("Content-Type", "application/json")
                                    .unwrap(),
                            ),
                    )
                    .unwrap();
            }
        });
        let mut dispatches = 0;
        for _ in 0..if legacy && !direct_remote { 1 } else { 2 } {
            let result = workflow_ready_apply_guarded_with_ci_wait_hook(
                &repo,
                if direct_remote {
                    "RT-0099"
                } else {
                    &local_task
                },
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some("origin"),
                None,
                Some(|event: &JsonValue| {
                    if event["status"] == "starting"
                        && event["code"] == "prepare_final_snapshot_promotion"
                    {
                        dispatches += 1;
                        return Err("fixture reached guarded history preparation".into());
                    }
                    Ok(())
                }),
                None::<fn(&mut JsonValue) -> Result<bool, String>>,
            );
            if legacy && !direct_remote {
                assert_eq!(
                    result.unwrap_err(),
                    "fixture reached guarded history preparation"
                );
            } else {
                let result = result.unwrap();
                assert_eq!(result["patchset"]["patchset_id"], id);
            }
        }
        handle.join().unwrap();
        assert_eq!(dispatches, usize::from(legacy && !direct_remote));
        assert!(requests
            .lock()
            .unwrap()
            .iter()
            .all(|(method, _)| method == "GET"));
    }
}
