use super::*;
use sha2::{Digest, Sha256};

fn fingerprint(db: &TestDb) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(ServerRemoteBinaryDb::authority_root(db).as_path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "bin" || x == "idx"))
        .map(|p| {
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                fs::read(p).unwrap(),
            )
        })
        .collect()
}

fn legacy_six(
    label: &str,
) -> (
    TestDb,
    BinaryDbServerWorkflowV0Store<TestDb>,
    JsonValue,
    Vec<String>,
) {
    let db = initialized_db(label);
    let ids = seed_history_content(&db, 6);
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone()).with_repository_index(0);
    let mut request = history_promotion_request(&ids);
    for (n, entry) in request["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        entry["change"] = JsonValue::Null;
        entry["local_change_id"] = JsonValue::Null;
        entry["local_change_ref"] = JsonValue::Null;
        let item = format!("legacy/{n}");
        let mut payload = atomic_plan_payload("Legacy continuation", &item);
        payload["artifact_path"] = json!(format!("docs/sprints/legacy-{n}.md"));
        let plan = plan_service(db.clone())
            .create_plan("repo", &payload)
            .unwrap();
        entry["task"]["plan_id"] = plan["plan_id"].clone();
        entry["task"]["origin_plan_revision_id"] = plan["head_revision_id"].clone();
        entry["task"]["plan_item_ref"] = json!(item);
    }
    store.prepare_history_promotion("repo", &request).unwrap();
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    for index in 0..7u32 {
        let mut p = WorkflowBinaryV0Codec::decode_frozen_patchset(
            &tx.read_record(WorkflowBinaryV0Codec::patchset_file(), index)
                .unwrap(),
        )
        .unwrap();
        let raw = tx
            .read_payload(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                p.summary_offset,
                u32::from(p.summary_len),
            )
            .unwrap();
        let text = String::from_utf8(raw).unwrap();
        let (prefix, body) = text.split_once(' ').unwrap();
        let mut v: JsonValue = serde_json::from_str(body).unwrap();
        if index < 6 {
            v["contract"] = json!("ait-local-land-receipt/v1");
            v["change_ref"] = json!(format!("T-{:04}/C-01", index + 1));
            v["local_change_id"] = json!("C-01");
            v["local_change_ref"] = json!(format!("LCT-{:04}/C-01", index + 1));
        } else {
            v["contract"] = json!("ait-history-promotion/v1");
            v["idempotency_key"] = json!("source-era-key");
            v["request_sha256"] = json!("0".repeat(64));
            v["aggregate"]["change_index"] = json!(5);
            v["aggregate"]["change_ref"] = json!("T-0006/C-01");
            v["aggregate"]["patchset_id"] = json!("T-0006/C-01/P-02");
            for (n, e) in v["entries"].as_array_mut().unwrap().iter_mut().enumerate() {
                e["change_index"] = json!(n);
                e["change_ref"] = json!(format!("T-{:04}/C-01", n + 1));
                e["local_change_id"] = json!("C-01");
                e["local_change_ref"] = json!(format!("LCT-{:04}/C-01", n + 1));
                e["receipt_patchset_id"] = json!(format!("T-{:04}/C-01/P-01", n + 1));
            }
        }
        let encoded = format!("{prefix} {v}");
        let range = tx
            .append_payload(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                encoded.as_bytes(),
            )
            .unwrap();
        p.summary_offset = range.payload_offset;
        p.summary_len = range.payload_len.try_into().unwrap();
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            index,
            &WorkflowBinaryV0Codec::encode_frozen_patchset(p).unwrap(),
        )
        .unwrap();
    }
    tx.commit().unwrap();
    drop(tx);
    request["idempotency_key"] = json!("new-versioned-operation");
    for (n, e) in request["entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        e["expected_remote_task_id"] = json!(format!("T-{:04}", n + 1));
        e["expected_remote_change_ref"] = JsonValue::Null;
    }
    validate_frozen_server_workflow_v0(&db).unwrap();
    (db, store, request, ids)
}

fn ready(store: &BinaryDbServerWorkflowV0Store<TestDb>, id: &str) {
    store
        .record_review(
            "T-0006",
            &json!({"patchset_id":id,"reviewer":"owner","action":"approve"}),
        )
        .unwrap();
    let run = store
        .run_patchset_ci(id, &json!({"trigger":"workflow_ready"}))
        .unwrap();
    store.complete_patchset_ci(id,&json!({"patchset_id":id,"ci_run_seq":run["ci_run_seq"],"selected_suite_count":1,"suite_result_count":1,"blocking_failure_count":0,"overall_status":"pass","tests_status":"pass","lint_status":"pass"})).unwrap();
    store.put_attestation(id,&json!({"verification_state":"pass","require_tests_pass":true,"require_human_review":true,"require_lint_pass":true})).unwrap();
    assert_eq!(store.evaluate_policy(id).unwrap()["decision"], "pass");
}

#[test]
fn legacy_six_receipts_prepare_one_fresh_aggregate_and_retry_without_old_gate_reuse() {
    let (db, store, request, ids) = legacy_six("legacy-six-continuation");
    ready(&store, "T-0006/P-02");
    let before = fingerprint(&db);
    let old_payload = before["patchset_summary_payload.bin"].clone();
    let prepared = store.prepare_history_promotion("repo", &request).unwrap();
    assert_eq!(prepared["aggregate"]["patchset_id"], "T-0006/P-03");
    let after = fingerprint(&db);
    assert!(after["patchset_summary_payload.bin"].starts_with(&old_payload));
    for key in [
        "task_payload.bin",
        "change.bin",
        "snapshot_link.bin",
        "attest.bin",
        "review.bin",
        "policy.bin",
        "land.bin",
    ] {
        assert_eq!(before[key], after[key], "{key}");
    }
    let before_tasks = before["task.bin"][4..].chunks_exact(64).collect::<Vec<_>>();
    let after_tasks = after["task.bin"][4..].chunks_exact(64).collect::<Vec<_>>();
    assert_eq!(&before_tasks[..5], &after_tasks[..5]);
    assert_eq!(
        after_tasks[5][0]
            & (TASK_META_COMPLETED | crate::foundation::workflow_binary_v0::TASK_META_CANCELED),
        0
    );
    assert_eq!(
        after_tasks[5][0] & crate::foundation::workflow_binary_v0::TASK_META_READY_TO_LAND,
        0
    );
    assert_eq!(&before_tasks[5][52..60], &after_tasks[5][52..60]);
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::patchset_file())
            .unwrap(),
        8
    );
    let retry = store.prepare_history_promotion("repo", &request).unwrap();
    assert_eq!(retry["replayed"], true);
    assert_eq!(fingerprint(&db), after);
    let land = json!({"actor_key_index":0,"contract":"task-land-atomic-guarded/v2","idempotency_key":"real-aggregate-acceptance",
        "task_or_change_ref":"T-0006","target_line":"main","mode":"direct",
        "expected_identity":{"repository_index":0,"patchset_id":"T-0006/P-03","base_snapshot_id":ids[0],"revision_snapshot_id":ids[6]}});
    assert!(store.submit_task_land("T-0006", &land).is_err());
    assert_eq!(fingerprint(&db), after);
    ready(&store, "T-0006/P-03");
    let accepted = store.submit_task_land("T-0006", &land).unwrap();
    assert_eq!(accepted["land"]["patchset_id"], "T-0006/P-03");
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        1
    );
    for n in 1..=5 {
        let t = store.get_task(None, &format!("T-{n:04}")).unwrap();
        assert_eq!(t["status"], "active");
        assert!(t["successful_land"].is_null());
    }
    validate_frozen_server_workflow_v0(&db).unwrap();
}

#[test]
fn legacy_reuse_rejects_claimed_changes_missing_task_partial_batch_and_content_drift_without_effects(
) {
    for case in 0..8 {
        let (db, store, mut request, _) = legacy_six(&format!("legacy-reject-{case}"));
        match case {
            0 => {
                request["entries"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("expected_remote_task_id");
            }
            1 => {
                request["entries"][0]["task"]["title"] = json!("Different source content");
            }
            2 => {
                request["entries"].as_array_mut().unwrap().pop();
                let last = request["entries"].as_array().unwrap().last().unwrap()
                    ["landed_snapshot_id"]
                    .clone();
                request["revision_snapshot_id"] = last;
            }
            3 => {
                request["entries"][0]["expected_remote_change_ref"] = json!("T-0001/C-01");
            }
            4 => {
                request["entries"][0]["landed_at_s"] = json!(999);
            }
            5 => {
                store.prepare_history_promotion("repo", &request).unwrap();
                request["idempotency_key"] =
                    json!("cannot-replace-current-v3-with-another-operation");
            }
            6 => {
                let mut tx =
                    BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow)
                        .unwrap();
                let raw = tx
                    .read_record(WorkflowBinaryV0Codec::snapshot_link_file(), 0)
                    .unwrap();
                let mut link = WorkflowBinaryV0Codec::decode_snapshot_link(&raw).unwrap();
                link.task_index = 1;
                tx.overwrite_record(
                    WorkflowBinaryV0Codec::snapshot_link_file(),
                    0,
                    &WorkflowBinaryV0Codec::encode_snapshot_link(link).unwrap(),
                )
                .unwrap();
                tx.commit().unwrap();
            }
            _ => {
                for key in ["plan_id", "origin_plan_revision_id", "plan_item_ref"] {
                    request["entries"][0]["task"][key] = request["entries"][1]["task"][key].clone();
                }
            }
        }
        let before = fingerprint(&db);
        assert!(store.prepare_history_promotion("repo", &request).is_err());
        assert_eq!(fingerprint(&db), before);
    }
}

#[test]
fn unknown_staged_or_drifted_legacy_batch_is_rejected_without_append() {
    for case in 0..7 {
        let (db, store, request, _) = legacy_six(&format!("legacy-stored-drift-{case}"));
        let index = if case == 0 { 0 } else { 6 };
        let id = if index == 0 {
            "T-0001/P-01"
        } else {
            "T-0006/P-02"
        };
        let old = store.get_patchset(None, id).unwrap();
        let (prefix, body) = old["summary"].as_str().unwrap().split_once(' ').unwrap();
        let mut v: JsonValue = serde_json::from_str(body).unwrap();
        match case {
            0 | 1 => v["contract"] = json!("unsupported-history-contract"),
            2 => v["stage_ordinal"] = json!(0),
            3 => v["entries"][0]["receipt_patchset_index"] = json!(1),
            4 => v["entries"][0]["snapshot_link_count"] = json!(2),
            5 => v["aggregate"]["change_ref"] = json!("T-0005/C-01"),
            _ => v["promotion_id"] = json!("staged-source"),
        }
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
        let mut p = WorkflowBinaryV0Codec::decode_frozen_patchset(
            &tx.read_record(WorkflowBinaryV0Codec::patchset_file(), index)
                .unwrap(),
        )
        .unwrap();
        let encoded = format!("{prefix} {v}");
        let range = tx
            .append_payload(
                WorkflowBinaryV0Codec::patchset_summary_file(),
                encoded.as_bytes(),
            )
            .unwrap();
        p.summary_offset = range.payload_offset;
        p.summary_len = range.payload_len.try_into().unwrap();
        tx.overwrite_record(
            WorkflowBinaryV0Codec::patchset_file(),
            index,
            &WorkflowBinaryV0Codec::encode_frozen_patchset(p).unwrap(),
        )
        .unwrap();
        tx.commit().unwrap();
        drop(tx);
        let before = fingerprint(&db);
        assert!(
            store.prepare_history_promotion("repo", &request).is_err(),
            "case {case}"
        );
        assert_eq!(fingerprint(&db), before);
    }
}

#[test]
fn old_v1_aggregate_correction_and_old_operation_key_remain_blocked() {
    let (db, store, mut request, ids) = legacy_six("legacy-v1-direct-rejection");
    ready(&store, "T-0006/P-02");
    let before = fingerprint(&db);
    let land = json!({"actor_key_index":0,"contract":"task-land-atomic-guarded/v2","idempotency_key":"old-v1-direct",
        "task_or_change_ref":"T-0006","target_line":"main","mode":"direct",
        "expected_identity":{"repository_index":0,"patchset_id":"T-0006/P-02","base_snapshot_id":ids[0],"revision_snapshot_id":ids[6]}});
    assert!(store.submit_task_land("T-0006", &land).is_err());
    assert_eq!(fingerprint(&db), before);
    request["idempotency_key"] = json!("source-era-key");
    assert!(store
        .prepare_history_promotion("repo", &request)
        .unwrap_err()
        .contains("IDEMPOTENCY_CONFLICT"));
    assert_eq!(fingerprint(&db), before);
    let old = store.get_patchset(None, "T-0006/P-02").unwrap();
    let encoded = old["summary"]
        .as_str()
        .unwrap()
        .strip_prefix("ait-history-promotion/v1 ")
        .unwrap();
    let manifest: JsonValue = serde_json::from_str(encoded).unwrap();
    let correction = json!({"actor_key_index":0,"contract":"history-promotion-correction-prepare/v2","idempotency_key":"old-v1-correction",
        "expected_identity":{"repository_index":0,"patchset_id":"T-0006/P-02","base_snapshot_id":ids[0],"revision_snapshot_id":ids[6]},
        "expected_receipts_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&manifest["entries"]).unwrap())),
        "target_line":"main","revision_snapshot_id":ids[6],"author_mode":"ai_with_human_review","summary":"Reject old aggregate mutation","snapshots":[]});
    assert!(store
        .prepare_history_promotion("repo", &correction)
        .unwrap_err()
        .contains("AGGREGATE_CORRECTION_UNSUPPORTED"));
    assert_eq!(fingerprint(&db), before);
}

#[test]
fn legacy_continuation_can_gain_real_c01_correction_without_resolving_quoted_c01() {
    let (db, store, request, ids) = legacy_six("legacy-six-real-correction");
    let old_summaries = fingerprint(&db)["patchset_summary_payload.bin"].clone();
    let prepared = store.prepare_history_promotion("repo", &request).unwrap();
    ready(&store, "T-0006/P-03");
    store
        .create_change(
            "repo",
            &json!({"actor_key_index":0,"task_id":"T-0006","title":"Correct the reviewed aggregate",
        "base_line":"main","fork_snapshot_id":ids[6]}),
        )
        .unwrap();
    let line = ServerBinaryDbLineStore::<_, 1>::new(db.clone())
        .create_line("feature/t-0006", 0, 8)
        .unwrap();
    let revision = "SNP-000000000008";
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerSnapshotCapture).unwrap();
    let content = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
    content
        .append_snapshot_in_tx(
            &mut tx,
            revision,
            ServerBinarySnapshotRecord {
                actor_index_plus1: 0,
                snapshot_meta: 0,
                history_flags: 0,
                payload_len: 0,
                payload_offset: 0,
                snapshot_hash48: server_snapshot_hash48_from_id(revision).unwrap(),
                parent_snapshot_index_plus1: 7,
                root_tree_pack_index_plus1: 0,
                root_entry_ordinal: 0,
                line_index_plus1: line + 1,
                manifest_hash: [0; 32],
                file_count: 0,
                total_bytes: 0,
                created_at_s: 8,
            },
            &ServerBinarySnapshotPayload {
                line_name: "feature/t-0006".into(),
                message: None,
            },
        )
        .unwrap();
    store.record_snapshot_capture_in_write(&mut tx,&json!({"contract":"ait.snapshot_capture_binding.v2",
        "repository_index":0,"snapshot_id":revision,"task_id":"T-0006","change_ref":"T-0006/C-01","line_name":"feature/t-0006"}),false, 1).unwrap();
    tx.commit().unwrap();
    drop(tx);
    let correction = json!({"actor_key_index":0,"contract":"history-promotion-correction-prepare/v2","idempotency_key":"legacy-six-new-real-c01",
        "expected_identity":{"repository_index":0,"patchset_id":"T-0006/P-03","base_snapshot_id":ids[0],"revision_snapshot_id":ids[6]},
        "expected_receipts_sha256":format!("{:x}",Sha256::digest(serde_json::to_vec(&prepared["entries"]).unwrap())),
        "target_line":"main","revision_snapshot_id":revision,"author_mode":"ai_with_human_review",
        "summary":"Correct the fresh aggregate while preserving six legacy receipts",
        "snapshots":[{"snapshot_id":revision,"created_at_s":8}]});
    let corrected = store
        .prepare_history_promotion("repo", &correction)
        .unwrap();
    assert_eq!(corrected["aggregate"]["patchset_id"], "T-0006/C-01/P-04");
    assert_eq!(corrected["aggregate"]["change_ref"], "T-0006/C-01");
    assert_eq!(corrected["entries"], prepared["entries"]);
    assert!(fingerprint(&db)["patchset_summary_payload.bin"].starts_with(&old_summaries));
    for n in 1..=6 {
        assert!(
            store.get_patchset(None, &format!("T-{n:04}/P-01")).unwrap()["change_ref"].is_null()
        );
    }
    let before = fingerprint(&db);
    assert_eq!(
        store
            .prepare_history_promotion("repo", &correction)
            .unwrap()["replayed"],
        true
    );
    assert_eq!(fingerprint(&db), before);
    let land = json!({"actor_key_index":0,"contract":"task-land-atomic-guarded/v2","idempotency_key":"legacy-six-correction-acceptance",
        "task_or_change_ref":"T-0006","target_line":"main","mode":"direct",
        "expected_identity":{"repository_index":0,"patchset_id":"T-0006/C-01/P-04","base_snapshot_id":ids[0],"revision_snapshot_id":revision}});
    assert!(store.submit_task_land("T-0006", &land).is_err());
    assert_eq!(fingerprint(&db), before);
    ready(&store, "T-0006/P-04");
    let accepted = store.submit_task_land("T-0006", &land).unwrap();
    assert_eq!(accepted["land"]["patchset_id"], "T-0006/C-01/P-04");
    let after = fingerprint(&db);
    assert_eq!(
        store.submit_task_land("T-0006", &land).unwrap()["land"],
        accepted["land"]
    );
    assert_eq!(fingerprint(&db), after);
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::land_file())
            .unwrap(),
        1
    );
    for n in 1..=5 {
        let t = store.get_task(None, &format!("T-{n:04}")).unwrap();
        assert_eq!(t["status"], "active");
        assert!(t["successful_land"].is_null());
    }
    validate_frozen_server_workflow_v0(&db).unwrap();
}
