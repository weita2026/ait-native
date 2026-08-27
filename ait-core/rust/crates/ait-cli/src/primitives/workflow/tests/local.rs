use super::*;
use crate::primitives::change_flow::change_local_create_with_change_store;
use crate::primitives::workflow::local_completion::{
    workflow_history_prepare_entries, workflow_staged_history_prepare_request,
    workflow_validate_history_publication_response,
};
use crate::primitives::worktree::create_local_line_with_line_store;

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
fn final_snapshot_promotion_aggregates_remote_head_to_latest_local_head() {
    let candidate = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("LCC-FINAL", "SNP-N"),
        Some("SNP-N"),
        Some("SNP-ZERO"),
        None,
        Some(5),
    )
    .expect("final snapshot candidate");

    assert_eq!(candidate["mode"], json!("solo_local_history_promotion"));
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
        "ait workflow ready LCT-FINAL/C-01 --apply --remote origin"
    );
    assert!(preview["next_action"]["detail"]
        .as_str()
        .unwrap()
        .contains("ait workflow finish LCT-FINAL/C-01 --apply --remote origin"));
}

#[test]
fn final_snapshot_promotion_rejects_an_older_completed_local_row() {
    let error = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("LCC-OLDER", "SNP-N-MINUS-ONE"),
        Some("SNP-N"),
        Some("SNP-ZERO"),
        None,
        Some(4),
    )
    .expect_err("older local row must not promote");

    assert!(error.contains("Only the latest completed local change"));
    assert!(error.contains("SNP-N"));
}

#[test]
fn final_snapshot_promotion_rejects_remote_divergence_before_publish() {
    let error = workflow_final_snapshot_candidate_from_entry(
        &completed_local_entry("LCC-FINAL", "SNP-N"),
        Some("SNP-N"),
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

    assert!(error.contains("do not have complete remote publication records"));
    assert!(error.contains("Refusing to treat this as completed promotion"));
}

#[test]
fn same_head_promotion_rejects_published_but_unlanded_remote_change() {
    let error = workflow_same_head_remote_land_authority(
        &published_completed_local_entry("LCC-FINAL", "SNP-N"),
        Some(&json!({
            "change_ref": "RCT-REMOTE/C-01",
            "status": "ready",
            "landed_snapshot_id": null,
        })),
        "SNP-N",
    )
    .expect_err("unlanded remote Change must not own a same-head target Line");

    assert!(error.contains("is `ready` at `none`"));
    assert!(error.contains("no exact successful remote Land"));
}

#[test]
fn same_head_promotion_rejects_mismatched_remote_landed_snapshot() {
    let error = workflow_same_head_remote_land_authority(
        &published_completed_local_entry("LCC-FINAL", "SNP-N"),
        Some(&json!({
            "change_ref": "RCT-REMOTE/C-01",
            "status": "landed",
            "landed_snapshot_id": "SNP-OTHER",
        })),
        "SNP-N",
    )
    .expect_err("mismatched remote Land must not own a same-head target Line");

    assert!(error.contains("is `landed` at `SNP-OTHER`"));
    assert!(error.contains("no exact successful remote Land"));
}

#[test]
fn same_head_promotion_accepts_exact_remote_landed_change_authority() {
    let authority = workflow_same_head_remote_land_authority(
        &published_completed_local_entry("LCC-FINAL", "SNP-N"),
        Some(&json!({
            "change_ref": "RCT-REMOTE/C-01",
            "status": "landed",
            "landed_snapshot_id": "SNP-N",
        })),
        "SNP-N",
    )
    .expect("exact remote Land must own an idempotent same-head replay");

    assert_eq!(authority["status"], json!("verified"));
    assert_eq!(authority["authority"], json!("remote_landed_change"));
    assert_eq!(authority["remote_change_id"], json!("RCT-REMOTE/C-01"));
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
        workflow_local_change_land_with_change_store(
            &change_store,
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
    workflow_local_change_land_with_change_store(
        &change_store,
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
    create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "fixture-ait",
        &feature_line,
        Some("first Task snapshot"),
        false,
    )
    .expect("first Task snapshot");
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
    workflow_local_change_land_with_change_store(
        &change_store,
        &change_ref,
        "main",
        &landed_snapshot_id,
        Some(&landed_snapshot_id),
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
    workflow_local_change_land_with_change_store(
        &change_store,
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
    assert!(echo_error.contains("does not match requested identity"));

    let mut wrong_owner = responses.clone();
    wrong_owner[0]["change_ref"] = json!("RCT-OTHER/C-01");
    let owner_error = workflow_validate_history_publication_response(&candidates, &wrong_owner)
        .expect_err("wrong Remote Change owner must fail");
    assert!(owner_error.contains("is not owned by Remote Task"));

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
