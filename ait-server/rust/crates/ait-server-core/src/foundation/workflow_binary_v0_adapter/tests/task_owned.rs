use super::*;

const TASK: &str = "RCPT-0001";
const BASE: &str = "SNP-0000000000A1";
const FIRST: &str = "SNP-0000000000B1";
const SECOND: &str = "SNP-0000000000B2";
type Store = BinaryDbServerWorkflowV0Store<TestDb>;

fn fixture(label: &str) -> (TestDb, Store, u32) {
    let db = initialized_db(label);
    seed_content(&db);
    let store = Store::new_remote_frozen(db.clone(), "CP")
        .unwrap()
        .with_repository_index(0);
    store
        .create_task(
            "repo",
            &json!({"actor_key_index":0,"title":"Task-owned work","intent":"Preserve exact candidates"}),
        )
        .unwrap();
    let line = ServerBinaryDbLineStore::<_, 1>::new(db.clone())
        .create_line("feature/rcpt-0001", 0, 1)
        .unwrap();
    (db, store, line)
}

fn capture(
    db: &TestDb,
    store: &Store,
    line: u32,
    id: &str,
    parent: u32,
    change: Option<&str>,
) -> u32 {
    let mut tx =
        BinaryDbWriteTxn::begin_serving(db, BinaryDbCommandScope::ServerSnapshotCapture).unwrap();
    let record = ServerBinarySnapshotRecord {
        actor_index_plus1: 1,
        snapshot_meta: 0,
        history_flags: 0,
        payload_len: 0,
        payload_offset: 0,
        snapshot_hash48: server_snapshot_hash48_from_id(id).unwrap(),
        parent_snapshot_index_plus1: parent + 1,
        root_tree_pack_index_plus1: 0,
        root_entry_ordinal: 0,
        line_index_plus1: line + 1,
        manifest_hash: [0; 32],
        file_count: 0,
        total_bytes: 0,
        created_at_s: 7,
    };
    let index = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone())
        .append_snapshot_in_tx(
            &mut tx,
            id,
            record,
            &ServerBinarySnapshotPayload {
                line_name: "feature/rcpt-0001".to_string(),
                message: None,
            },
        )
        .unwrap();
    store.record_snapshot_capture_in_write(&mut tx, &json!({"contract":"ait.snapshot_capture_binding.v2",
        "repository_index":0,"snapshot_id":id,"task_id":TASK,"change_ref":change,"line_name":"feature/rcpt-0001"}), false, 1).unwrap();
    tx.commit().unwrap();
    index
}

fn publish(store: &Store, owner: &str, revision: &str) -> JsonValue {
    store
        .publish_patchset(
            owner,
            &json!({"actor_key_index":0,"base_snapshot_id":BASE,"revision_snapshot_id":revision,
        "summary":"Frozen reviewed Task candidate","author_mode":"ai_with_human_review"}),
        )
        .unwrap()
}

fn ready(store: &Store, id: &str) {
    store
        .record_review(
            TASK,
            &json!({"patchset_id":id,"reviewer":"owner","action":"approve"}),
        )
        .unwrap();
    let run = store
        .run_patchset_ci(id, &json!({"trigger":"workflow_ready"}))
        .unwrap();
    store
        .complete_patchset_ci(
            id,
            &json!({"patchset_id":id,"ci_run_seq":run["ci_run_seq"],
        "selected_suite_count":1,"suite_result_count":1,"blocking_failure_count":0,
        "overall_status":"pass","tests_status":"pass","lint_status":"pass"}),
        )
        .unwrap();
    store
        .put_attestation(
            id,
            &json!({"verification_state":"pass","require_tests_pass":true,
        "require_human_review":true,"require_lint_pass":true}),
        )
        .unwrap();
    assert_eq!(store.evaluate_policy(id).unwrap()["decision"], "pass");
}

fn request(id: &str, revision: &str) -> JsonValue {
    json!({"actor_key_index":0,"contract":"task-land-atomic-guarded/v2","idempotency_key":"task-owned-finish",
        "task_or_change_ref":TASK,"target_line":"main","mode":"direct",
        "expected_identity":{"repository_index":0,"patchset_id":id,"base_snapshot_id":BASE,"revision_snapshot_id":revision}})
}

#[test]
fn snapshot_workflow_owner_joins_fixed_links_and_preserves_task_only_null() {
    let (db, store, line) = fixture("snapshot-owner-readback");
    assert_eq!(
        store.snapshot_workflow_owner(BASE).unwrap(),
        JsonValue::Null
    );
    let change = store
        .create_change(
            "repo",
            &json!({"actor_key_index":0,"task_id":TASK,
        "title":"Actual Change","base_line":"main","fork_snapshot_id":BASE}),
        )
        .unwrap();
    let first = capture(&db, &store, line, FIRST, 0, None);
    capture(
        &db,
        &store,
        line,
        SECOND,
        first,
        change["change_ref"].as_str(),
    );
    assert_eq!(
        store.snapshot_workflow_owner(FIRST).unwrap(),
        json!({"task_id":TASK,"change_ref":null})
    );
    assert_eq!(
        store.snapshot_workflow_owner(SECOND).unwrap(),
        json!({"task_id":TASK,"change_ref":change["change_ref"]})
    );
    assert!(store.snapshot_workflow_owner("SNP-00000000FFFF").is_err());
}

#[test]
fn snapshot_workflow_owner_rejects_ambiguous_fixed_ownership() {
    use crate::foundation::workflow_binary_v0::V0SnapshotLinkRecord;
    let (db, store, line) = fixture("snapshot-owner-conflict");
    let first = capture(&db, &store, line, FIRST, 0, None);
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    tx.append_record(
        WorkflowBinaryV0Codec::snapshot_link_file(),
        &WorkflowBinaryV0Codec::encode_snapshot_link(V0SnapshotLinkRecord {
            link_meta: 0,
            snapshot_ordinal: 1,
            task_index: 0,
            change_index_plus1: 0,
            content_snapshot_index: first,
            created_at_s: 8,
        })
        .unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    assert!(store
        .snapshot_workflow_owner(FIRST)
        .unwrap_err()
        .contains("duplicate"));
}

#[test]
fn real_revision_after_task_only_candidate_requires_its_exact_reviewed_fork() {
    let (db, store, line) = fixture("real-revision-reviewed-fork");
    let first = capture(&db, &store, line, FIRST, 0, None);
    publish(&store, TASK, FIRST);
    for fork in [None, Some(BASE), Some(SECOND)] {
        let error = store.create_change("repo", &json!({"actor_key_index":0,"task_id":TASK,"title":"Reviewed correction","base_line":"main","fork_snapshot_id":fork})).unwrap_err();
        assert!(error.contains("exact current Task candidate"), "{error}");
    }
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .unwrap(),
        0
    );
    drop(read);
    store.create_change("repo", &json!({"actor_key_index":0,"task_id":TASK,"title":"Reviewed correction","base_line":"main","fork_snapshot_id":FIRST})).unwrap();
    assert_eq!(
        store.get_change(Some("repo"), "RCPT-0001/C-01").unwrap()["fork_snapshot_id"],
        FIRST
    );
    capture(&db, &store, line, SECOND, first, Some("RCPT-0001/C-01"));
    assert_eq!(
        publish(&store, "RCPT-0001/C-01", SECOND)["patchset_id"],
        "RCPT-0001/C-01/P-02"
    );
    assert!(store
        .capture_binding_with_read(&BinaryDbReadTxn::new(&db), first)
        .unwrap()
        .unwrap()["change_ref"]
        .is_null());
}

#[test]
fn compact_manual_verification_keeps_optional_and_required_ci_separate() {
    let (db, store, line) = fixture("compact-manual-verification");
    capture(&db, &store, line, FIRST, 0, None);
    let candidate = publish(&store, TASK, FIRST);
    let id = candidate["patchset_id"].as_str().unwrap();
    let statement = store.put_attestation(id, &json!({"verification_state":"pass","require_tests_pass":false,"require_human_review":false,"require_lint_pass":false})).unwrap();
    assert_eq!(statement["verification_state"], "pass");
    assert_eq!(statement["ci_backed"], false);
    assert_eq!(statement["evaluation_summary"]["tests"], "none");
    assert_eq!(store.evaluate_policy(id).unwrap()["decision"], "pass");
    store
        .put_attestation(
            id,
            &json!({"verification_state":"pass","require_tests_pass":true}),
        )
        .unwrap();
    assert_ne!(store.evaluate_policy(id).unwrap()["decision"], "pass");
    for (verification, revoked) in [
        ("unknown", false),
        ("pending", false),
        ("fail", false),
        ("pass", true),
    ] {
        store.put_attestation(id, &json!({"verification_state":verification,"revoked":revoked,"require_tests_pass":false})).unwrap();
        assert_ne!(store.evaluate_policy(id).unwrap()["decision"], "pass");
    }
    let current = store.get_patchset(Some("repo"), id).unwrap();
    assert_eq!(current["ci_run_seq"], 0);
    assert_eq!(current["ci_completed_at_s"], 0);
    assert!(store.get_task(Some("repo"), TASK).unwrap()["successful_land"].is_null());
}

#[test]
fn zero_change_capture_finish_and_retry_read_the_original_real_receipt() {
    let (db, store, line) = fixture("task-owned-capture-finish");
    assert_eq!(
        store
            .resolve_user_actor(&UserActorIdentity::new("Bob", "bob@example.test").unwrap())
            .unwrap(),
        1
    );
    let index = capture(&db, &store, line, FIRST, 0, None);
    let binding = store
        .capture_binding_with_read(&BinaryDbReadTxn::new(&db), index)
        .unwrap()
        .unwrap();
    assert!(binding["change_ref"].is_null());
    assert_eq!(binding["line_name"], "feature/rcpt-0001");
    assert!(binding.get("worktree_name").is_none());
    let candidate = publish(&store, TASK, FIRST);
    assert_eq!(candidate["patchset_id"], "RCPT-0001/P-01");
    assert!(candidate["change_ref"].is_null());
    ready(&store, "RCPT-0001/P-01");
    let audit = store.read_task_audit("repo", TASK, "main").unwrap();
    assert_eq!(audit["verdict"]["code"], "ready_to_land");
    assert_eq!(audit["changes"], json!([]));
    assert!(store.get_task(Some("repo"), TASK).unwrap()["successful_land"].is_null());
    let service =
        crate::foundation::server_queue_binary_db::BinaryDbServerWorkflowReadModelService::new(
            db.clone(),
            store.clone().into_arc(),
        );
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let queue = loop {
        match service.read_queue_summary(Some("repo"), Some("active"), true) {
            Ok(value) => break value,
            Err(error) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "Task-owned queue did not warm: {error}"
                );
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    };
    assert_eq!(
        queue["task_queue"]["items"][0]["current_patchset"]["patchset_id"],
        "RCPT-0001/P-01"
    );
    assert_eq!(queue["task_queue"]["items"][0]["changes"]["total"], 0);
    assert_eq!(
        queue["reviewer_inbox"]["items"][0]["current_patchset"]["patchset_id"],
        "RCPT-0001/P-01"
    );
    assert!(queue["reviewer_inbox"]["items"][0]["change_ref"].is_null());
    let mut payload = request("RCPT-0001/P-01", FIRST);
    payload["actor_key_index"] = json!(1);
    let first = store.submit_task_land(TASK, &payload).unwrap();
    assert_eq!(first["status"], "succeeded");
    assert!(first["change"].is_null());
    assert_eq!(first["land"]["submission_id"], "RCPT-0001/L-01");
    assert_eq!(first["land"]["actor_key_index"], 1);
    assert_eq!(
        store.get_task(Some("repo"), TASK).unwrap()["actor_key_index"],
        1
    );
    ServerBinaryDbLineStore::<_, 1>::new(db.clone())
        .set_line_head("main", 3, 9)
        .unwrap();
    payload["actor_key_index"] = json!(0);
    let retried = store.submit_task_land(TASK, &payload).unwrap();
    assert_eq!(retried["replayed"], true);
    assert_eq!(retried["land"], first["land"]);
    assert_eq!(retried["landed_snapshot_id"], FIRST);
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        ServerBinarySnapshotCodec::<2>::decode_record(
            &read
                .read_record(ServerBinarySnapshotCodec::<2>::record_file(), index)
                .unwrap()
        )
        .unwrap()
        .actor_index_plus1,
        1
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::change_file())
            .unwrap(),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        1
    );
    assert_eq!(
        fs::metadata(db.authority_root().as_path().join("snapshot_link.bin"))
            .unwrap()
            .len(),
        26
    );
    assert!(!db
        .authority_root()
        .as_path()
        .join("snapshot_link_payload.bin")
        .exists());
    assert!(store
        .submit_task_land(TASK, &request("RCPT-0001/P-01", SECOND))
        .unwrap_err()
        .contains("MISMATCH"));
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn a_new_candidate_stops_old_finish_and_select_is_read_only() {
    let (db, store, line) = fixture("task-owned-candidate-race");
    let first = capture(&db, &store, line, FIRST, 0, None);
    capture(&db, &store, line, SECOND, first, None);
    publish(&store, TASK, FIRST);
    ready(&store, "RCPT-0001/P-01");
    let error = store
        .submit_task_land_after_prepare_for_test(TASK, &request("RCPT-0001/P-01", FIRST), || {
            publish(&store, TASK, SECOND);
            Ok(())
        })
        .unwrap_err();
    assert!(error.contains("MISMATCH"), "{error}");
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        0
    );
    drop(read);
    assert!(store
        .submit_task_land(TASK, &request("RCPT-0001/P-02", SECOND))
        .unwrap_err()
        .contains("NOT_READY"));
    let task_before = fs::read(db.authority_root().as_path().join("task.bin")).unwrap();
    let patchsets_before = fs::read(db.authority_root().as_path().join("patchset.bin")).unwrap();
    assert!(store
        .select_patchset(
            TASK,
            &json!({"actor_key_index":0,"patchset_id":"RCPT-0001/P-01"})
        )
        .is_err());
    assert_eq!(
        store
            .select_patchset(
                TASK,
                &json!({"actor_key_index":0,"patchset_id":"RCPT-0001/P-02"})
            )
            .unwrap()["selection_changed"],
        false
    );
    assert_eq!(
        task_before,
        fs::read(db.authority_root().as_path().join("task.bin")).unwrap()
    );
    assert_eq!(
        patchsets_before,
        fs::read(db.authority_root().as_path().join("patchset.bin")).unwrap()
    );
    ready(&store, "RCPT-0001/P-02");
    store
        .submit_task_land(TASK, &request("RCPT-0001/P-02", SECOND))
        .unwrap();
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn explicit_revision_preserves_initial_ownership_and_uses_task_scoped_ordinals() {
    let (db, store, line) = fixture("task-owned-explicit-revision");
    let first = capture(&db, &store, line, FIRST, 0, None);
    publish(&store, TASK, FIRST);
    let change = store
        .create_change(
            "repo",
            &json!({"actor_key_index":0,"task_id":TASK,"title":"Correct the reviewed candidate",
        "base_line":"main","fork_snapshot_id":FIRST}),
        )
        .unwrap();
    assert_eq!(change["change_ref"], "RCPT-0001/C-01");
    let second = capture(&db, &store, line, SECOND, first, Some("RCPT-0001/C-01"));
    let candidate = publish(&store, "RCPT-0001/C-01", SECOND);
    assert_eq!(candidate["patchset_id"], "RCPT-0001/C-01/P-02");
    assert_eq!(
        store.get_patchset(None, "RCPT-0001/P-02").unwrap()["patchset_id"],
        "RCPT-0001/C-01/P-02"
    );
    assert!(store.get_patchset(None, "RCPT-0001/C-02/P-02").is_err());
    assert!(store.get_patchset(None, "RCPT-0001/C-01/P-01").is_err());
    ready(&store, "RCPT-0001/C-01/P-02");
    let result = store
        .submit_task_land(TASK, &request("RCPT-0001/C-01/P-02", SECOND))
        .unwrap();
    assert_eq!(result["change_ref"], "RCPT-0001/C-01");
    let read = BinaryDbReadTxn::new(&db);
    assert!(store
        .capture_binding_with_read(&read, first)
        .unwrap()
        .unwrap()["change_ref"]
        .is_null());
    assert_eq!(
        store
            .capture_binding_with_read(&read, second)
            .unwrap()
            .unwrap()["change_ref"],
        "RCPT-0001/C-01"
    );
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn repeated_ready_reuses_current_candidate_and_ci_evidence() {
    let (db, store, line) = fixture("task-owned-ready-retry");
    capture(&db, &store, line, FIRST, 0, None);
    publish(&store, TASK, FIRST);
    ready(&store, "RCPT-0001/P-01");
    assert_eq!(publish(&store, TASK, FIRST)["ci_run_seq"], 1);
    let retry = store
        .run_patchset_ci("RCPT-0001/P-01", &json!({"trigger":"workflow_ready"}))
        .unwrap();
    assert_eq!(retry["ci_run_seq"], 1);
    assert_eq!(retry["ci"]["overall_status"], "pass");
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .unwrap(),
        1
    );
}

#[test]
fn old_contracts_and_non_change_capture_references_fail_without_effects() {
    let (db, store, _) = fixture("task-owned-wire-reject");
    let before = fs::read(db.authority_root().as_path().join("task.bin")).unwrap();
    assert!(store
        .start_plan_bound_task("repo", &json!({"contract":"task-start-atomic/v1"}))
        .is_err());
    for contract in [
        "history-promotion-prepare/v1",
        "history-promotion-prepare/v2",
    ] {
        assert!(store
            .prepare_history_promotion("repo", &json!({"actor_key_index":0,"contract":contract}))
            .is_err());
    }
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerSnapshotCapture).unwrap();
    let request = json!({"contract":"ait.snapshot_capture_binding.v2","repository_index":0,
        "snapshot_id":FIRST,"task_id":TASK,"change_ref":TASK,"line_name":"feature/rcpt-0001"});
    assert!(store
        .record_snapshot_capture_in_write(&mut tx, &request, false, 1)
        .is_err());
    drop(tx);
    assert_eq!(
        before,
        fs::read(db.authority_root().as_path().join("task.bin")).unwrap()
    );
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::snapshot_link_file())
            .unwrap(),
        0
    );
}

#[test]
fn task_only_history_can_gain_a_real_correction_without_rewriting_receipts() {
    use sha2::{Digest, Sha256};
    let db = initialized_db("task-owned-history-real-correction");
    let snapshots = seed_history_content(&db, 1);
    let store = Store::new_remote_frozen(db.clone(), "CP")
        .unwrap()
        .with_repository_index(0);
    let mut promotion = history_promotion_request(&snapshots);
    for entry in promotion["entries"].as_array_mut().unwrap() {
        entry["local_change_id"] = JsonValue::Null;
        entry["local_change_ref"] = JsonValue::Null;
        entry["change"] = JsonValue::Null;
    }
    let prepared = store.prepare_history_promotion("repo", &promotion).unwrap();
    assert_eq!(prepared["aggregate"]["patchset_id"], "RCPT-0001/P-02");
    assert!(prepared["aggregate"]["change_ref"].is_null());
    assert!(prepared["entries"][0]["change_ref"].is_null());
    assert!(store.get_task(None, TASK).unwrap()["successful_land"].is_null());
    store
        .create_change(
            "repo",
            &json!({"actor_key_index":0,"task_id":TASK,"title":"Correct the reviewed aggregate",
        "base_line":"main","fork_snapshot_id":snapshots[1]}),
        )
        .unwrap();
    let line = ServerBinaryDbLineStore::<_, 1>::new(db.clone())
        .create_line("feature/rcpt-0001", 0, 3)
        .unwrap();
    capture(&db, &store, line, FIRST, 1, Some("RCPT-0001/C-01"));
    let correction = json!({"actor_key_index":0,"contract":"history-promotion-correction-prepare/v2","idempotency_key":"task-only-to-c01",
        "expected_identity":{"repository_index":0,"patchset_id":"RCPT-0001/P-02",
            "base_snapshot_id":snapshots[0],"revision_snapshot_id":snapshots[1]},
        "expected_receipts_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&prepared["entries"]).unwrap())),
        "target_line":"main","revision_snapshot_id":FIRST,"author_mode":"ai_with_human_review",
        "summary":"Retain Task-only history while correcting its reviewed aggregate",
        "snapshots":[{"snapshot_id":FIRST,"created_at_s":7}]});
    let corrected = store
        .prepare_history_promotion("repo", &correction)
        .unwrap();
    assert_eq!(corrected["entries"], prepared["entries"]);
    assert_eq!(corrected["aggregate"]["change_ref"], "RCPT-0001/C-01");
    assert_eq!(corrected["aggregate"]["patchset_id"], "RCPT-0001/C-01/P-03");
    assert_eq!(
        corrected["aggregate"]["patchset"]["change_ref"],
        "RCPT-0001/C-01"
    );
    assert!(store.get_patchset(None, "RCPT-0001/P-01").unwrap()["change_ref"].is_null());
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        0
    );
    ready(&store, "RCPT-0001/P-03");
    let mut finish = request("RCPT-0001/P-03", FIRST);
    finish["expected_identity"]["base_snapshot_id"] = json!(snapshots[0]);
    let accepted = store.submit_task_land(TASK, &finish).unwrap();
    assert_eq!(accepted["land"]["change_ref"], "RCPT-0001/C-01");
    assert_eq!(accepted["land"]["patchset_id"], "RCPT-0001/C-01/P-03");
    assert_eq!(
        store.get_task(None, TASK).unwrap()["successful_land"],
        accepted["land"]
    );
    assert_eq!(
        store.submit_task_land(TASK, &finish).unwrap()["land"],
        accepted["land"]
    );
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn task_patchset_ordinals_include_withdrawn_candidates_and_reject_exhaustion() {
    let (db, store, line) = fixture("task-patchset-exhaustion");
    let first = capture(&db, &store, line, FIRST, 0, None);
    capture(&db, &store, line, SECOND, first, None);
    publish(&store, TASK, FIRST);
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    let mut retired = WorkflowBinaryV0Codec::decode_frozen_patchset(
        &tx.read_record(WorkflowBinaryV0Codec::patchset_file(), 0)
            .unwrap(),
    )
    .unwrap();
    retired.patchset_meta |= 1;
    tx.overwrite_record(
        WorkflowBinaryV0Codec::patchset_file(),
        0,
        &WorkflowBinaryV0Codec::encode_frozen_patchset(retired).unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(tx);
    for number in 2..=64 {
        let revision = if number % 2 == 0 { SECOND } else { FIRST };
        let candidate = publish(&store, TASK, revision);
        assert_eq!(candidate["patchset_id"], format!("{TASK}/P-{number:02}"));
    }
    let before = fs::read(db.authority_root().as_path().join("patchset.bin")).unwrap();
    let error=store.publish_patchset(TASK,&json!({"actor_key_index":0,"base_snapshot_id":BASE,"revision_snapshot_id":FIRST,"summary":"exhausted","author_mode":"ai_with_human_review"})).unwrap_err();
    assert!(error.contains("exhausted"), "{error}");
    assert_eq!(
        before,
        fs::read(db.authority_root().as_path().join("patchset.bin")).unwrap()
    );
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn short_patchset_lookup_rejects_duplicate_allocated_task_ordinals() {
    let (db, store, line) = fixture("task-short-lookup-duplicate");
    let first = capture(&db, &store, line, FIRST, 0, None);
    capture(&db, &store, line, SECOND, first, None);
    publish(&store, TASK, FIRST);
    publish(&store, TASK, SECOND);
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    let mut duplicate = WorkflowBinaryV0Codec::decode_frozen_patchset(
        &tx.read_record(WorkflowBinaryV0Codec::patchset_file(), 1)
            .unwrap(),
    )
    .unwrap();
    duplicate.patch_ordinal = 0;
    tx.overwrite_record(
        WorkflowBinaryV0Codec::patchset_file(),
        1,
        &WorkflowBinaryV0Codec::encode_frozen_patchset(duplicate).unwrap(),
    )
    .unwrap();
    tx.commit().unwrap();
    drop(tx);
    let before = fs::read(db.authority_root().as_path().join("patchset.bin")).unwrap();
    assert!(store
        .get_patchset(Some("repo"), "RCPT-0001/P-01")
        .unwrap_err()
        .contains("allocated ordinal"));
    assert_eq!(
        before,
        fs::read(db.authority_root().as_path().join("patchset.bin")).unwrap()
    );
}
