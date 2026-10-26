use super::*;
use crate::foundation::workflow_binary_v0_adapter::tests::{
    history_promotion_request, initialized_db, seed_history_content, TestDb,
};

#[test]
fn aggregate_correction_rejects_target_drift_and_receipt_digest_changes() {
    for moved in [false, true] {
        let f = fixture("aggregate-correction-target-digest");
        let mut request = correction_request(&f, &f.prepared, 11, "drift");
        if moved {
            ServerBinaryDbLineStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(f.db.clone())
                .set_line_head("main", 2, 300)
                .unwrap();
        } else {
            request["expected_receipts_sha256"] = json!("00");
        }
        let before = counts(&f);
        let error = f
            .store
            .prepare_history_promotion("repo", &request)
            .unwrap_err();
        assert!(
            if moved {
                error.contains("STALE_HISTORY_PROMOTION_BASE")
            } else {
                error.contains("receipt digest")
            },
            "{error}"
        );
        assert_eq!(counts(&f), before);
    }
}

#[test]
fn aggregate_correction_requires_atomic_finish_for_corrected_candidates() {
    let f = fixture("aggregate-correction-atomic-only");
    let request = correction_request(&f, &f.prepared, 11, "atomic-correction");
    let corrected = f.store.prepare_history_promotion("repo", &request).unwrap();
    let change = corrected["aggregate"]["change_ref"].as_str().unwrap();
    pass_fixture_gates(&f, corrected["aggregate"]["patchset_id"].as_str().unwrap());
    let before = counts(&f);
    let error = f
        .store
        .submit_land(
            change,
            &json!({"actor_key_index":0,"target_line":"main","mode":"direct"}),
        )
        .unwrap_err();
    assert!(
        error.contains("AGGREGATE_ATOMIC_FINISH_REQUIRED"),
        "{error}"
    );
    assert_eq!(counts(&f), before);
}

#[test]
fn aggregate_correction_staged_source_is_rejected_before_mutation() {
    use crate::foundation::workflow_binary_v0_adapter::tests::staged_history_promotion_request;
    let db = initialized_db("aggregate-correction-staged-reject");
    let snapshots = seed_history_content(&db, 65);
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone()).with_repository_index(0);
    let first = store
        .prepare_history_promotion(
            "repo",
            &staged_history_promotion_request(&snapshots, 0, None),
        )
        .unwrap();
    let previous = first["stage"]["patchset_id"].as_str().unwrap();
    let prepared = store
        .prepare_history_promotion(
            "repo",
            &staged_history_promotion_request(&snapshots, 1, Some(previous)),
        )
        .unwrap();
    let f = Fixture {
        db,
        store,
        snapshots,
        prepared,
    };
    let mut request = correction_request(&f, &f.prepared, 11, "staged-correction");
    request["expected_identity"]["revision_snapshot_id"] = json!(f.snapshots[65]);
    let before = counts(&f);
    let error = f
        .store
        .prepare_history_promotion("repo", &request)
        .unwrap_err();
    assert!(
        error.contains("AGGREGATE_CORRECTION_UNSUPPORTED"),
        "{error}"
    );
    assert_eq!(counts(&f), before);
}

struct Fixture {
    db: TestDb,
    store: BinaryDbServerWorkflowV0Store<TestDb>,
    snapshots: Vec<String>,
    prepared: JsonValue,
}

fn fixture(label: &str) -> Fixture {
    let db = initialized_db(label);
    let mut snapshots = seed_history_content(&db, 10);
    snapshots.extend((11..=12).map(|i| format!("SNP-{:012X}", i + 1)));
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone()).with_repository_index(0);
    let mut request = history_promotion_request(&snapshots[..8]);
    request["revision_snapshot_id"] = json!(snapshots[10]);
    request["entries"][6]["landed_snapshot_id"] = json!(snapshots[10]);
    request["entries"][6]["snapshots"] = json!((7..=10)
        .map(|i| json!({
            "snapshot_id": snapshots[i], "created_at_s": i + 1
        }))
        .collect::<Vec<_>>());
    let prepared = store.prepare_history_promotion("repo", &request).unwrap();
    Fixture {
        db,
        store,
        snapshots,
        prepared,
    }
}

fn append_capture_tail(f: &Fixture, end: usize, change_ref: &str, bind: bool) {
    let content =
        ServerBinaryDbSnapshotStore::<_, SERVER_CONTENT_BINARY_LAYOUT_ID>::new(f.db.clone());
    let task = change_ref.rsplit_once('/').unwrap().0;
    let line = format!("feature/{}", task.to_ascii_lowercase());
    for i in 11..=end {
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&f.db, BinaryDbCommandScope::ServerSnapshotCapture)
                .unwrap();
        if content
            .snapshot_by_id_in_write(&tx, &f.snapshots[i])
            .unwrap()
            .is_some()
        {
            continue;
        }
        let parent = content
            .snapshot_by_id_in_write(&tx, &f.snapshots[i - 1])
            .unwrap()
            .unwrap()
            .0;
        let record = ServerBinarySnapshotRecord {
            actor_index_plus1: 0,
            snapshot_meta: 0,
            history_flags: 0,
            payload_len: 0,
            payload_offset: 0,
            snapshot_hash48:
                crate::foundation::server_content_binary_db::server_snapshot_hash48_from_id(
                    &f.snapshots[i],
                )
                .unwrap(),
            parent_snapshot_index_plus1: parent + 1,
            root_tree_pack_index_plus1: 0,
            root_entry_ordinal: 0,
            line_index_plus1: 0,
            manifest_hash: [0; 32],
            file_count: 0,
            total_bytes: 0,
            created_at_s: (i + 1) as u64,
        };
        let payload = crate::foundation::server_content_binary_db::ServerBinarySnapshotPayload {
            line_name: line.clone(),
            message: Some("New correction capture".to_string()),
        };
        content
            .append_snapshot_in_tx(&mut tx, &f.snapshots[i], record, &payload)
            .unwrap();
        if bind {
            f.store
                .record_snapshot_capture_in_write(
                    &mut tx,
                    &json!({
                        "contract":"ait.snapshot_capture_binding.v2", "repository_index":0,
                        "snapshot_id":f.snapshots[i], "task_id":task, "change_ref":change_ref, "line_name":line,
                    }),
                    false, 1
                )
                .unwrap();
        }
        tx.commit().unwrap();
    }
}

fn correction_request(f: &Fixture, prior: &JsonValue, end: usize, key: &str) -> JsonValue {
    append_capture_tail(
        f,
        end,
        f.prepared["aggregate"]["change_ref"].as_str().unwrap(),
        true,
    );
    json!({"actor_key_index":0,
        "contract": CORRECTION_PREPARE_CONTRACT,
        "idempotency_key": key,
        "expected_identity": {
            "repository_index": 0,
            "patchset_id": prior["aggregate"]["patchset_id"],
            "base_snapshot_id": f.snapshots[0],
            "revision_snapshot_id": prior["revision_snapshot_id"]
        },
        "expected_receipts_sha256": receipt_entries_sha256(&f.prepared["entries"]).unwrap(),
        "target_line": "main",
        "revision_snapshot_id": f.snapshots[end],
        "author_mode": "ai_with_human_review",
        "summary": "Preserve the complete batch through review correction",
        "snapshots": (11..=end).map(|i| json!({
            "snapshot_id": f.snapshots[i], "created_at_s": i + 1
        })).collect::<Vec<_>>()
    })
}

fn counts(f: &Fixture) -> Vec<u32> {
    let read = BinaryDbReadTxn::new(&f.db);
    [
        WorkflowBinaryV0Codec::task_file(),
        WorkflowBinaryV0Codec::change_file(),
        WorkflowBinaryV0Codec::patchset_file(),
        WorkflowBinaryV0Codec::snapshot_link_file(),
        WorkflowBinaryV0Codec::land_file(),
    ]
    .into_iter()
    .map(|file| read.record_count(file).unwrap())
    .collect()
}

fn pass_fixture_gates(f: &Fixture, patchset: &str) {
    let change = f.prepared["aggregate"]["change_ref"].as_str().unwrap();
    let ci = f.store.run_patchset_ci(patchset, &json!({})).unwrap();
    f.store
        .complete_patchset_ci(
            patchset,
            &json!({
                "patchset_id": patchset, "ci_run_seq": ci["ci_run_seq"],
                "selected_suite_count": 1, "suite_result_count": 1,
                "blocking_failure_count": 0, "overall_status": "pass",
                "tests_status": "pass", "lint_status": "pass"
            }),
        )
        .unwrap();
    f.store
        .record_review(
            change,
            &json!({
                "patchset_id": patchset, "reviewer": "fixture-reviewer", "action": "approve"
            }),
        )
        .unwrap();
    f.store
        .put_attestation(
            patchset,
            &json!({
                "verification_state": "pass", "require_tests_pass": true,
                "require_human_review": true, "require_lint_pass": true
            }),
        )
        .unwrap();
    assert_eq!(
        f.store.evaluate_policy(patchset).unwrap()["decision"],
        "pass"
    );
}

fn guarded_finish(f: &Fixture, candidate: &JsonValue) -> Result<JsonValue, String> {
    let change = candidate["aggregate"]["change_ref"].as_str().unwrap();
    f.store.submit_task_land(
        change,
        &json!({"actor_key_index":0,
            "contract":"task-land-atomic-guarded/v2", "idempotency_key": "corrected-batch-finish",
            "task_or_change_ref": change, "target_line": "main", "mode": "direct",
            "expected_identity": {"repository_index": 0,
                "patchset_id": candidate["aggregate"]["patchset_id"],
                "base_snapshot_id": candidate["base_snapshot_id"],
                "revision_snapshot_id": candidate["revision_snapshot_id"]}
        }),
    )
}

#[test]
fn aggregate_correction_preserves_seven_receipts_and_ten_original_links() {
    let f = fixture("aggregate-correction-seven");
    let original_id = f.prepared["aggregate"]["patchset_id"].as_str().unwrap();
    pass_fixture_gates(&f, original_id);
    let original_receipts = f.prepared["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            f.store
                .get_patchset(None, entry["receipt_patchset_id"].as_str().unwrap())
                .unwrap()
        })
        .collect::<Vec<_>>();
    let original_links = (0..10)
        .map(|i| {
            BinaryDbReadTxn::new(&f.db)
                .read_record(WorkflowBinaryV0Codec::snapshot_link_file(), i)
                .unwrap()
        })
        .collect::<Vec<_>>();
    let request = correction_request(&f, &f.prepared, 11, "correction-first");
    let corrected = f.store.prepare_history_promotion("repo", &request).unwrap();
    assert_eq!(corrected["entries"], f.prepared["entries"]);
    assert_eq!(
        corrected["correction"]["receipt_revision_snapshot_id"],
        f.snapshots[10]
    );
    assert_eq!(corrected["revision_snapshot_id"], f.snapshots[11]);
    assert_eq!(counts(&f), vec![7, 7, 9, 11, 0]);
    let id = corrected["aggregate"]["patchset_id"].as_str().unwrap();
    assert!(id.ends_with("/P-03"));
    assert_eq!(corrected["aggregate"]["patchset"]["ci_run_seq"], 0);
    assert_ne!(f.store.evaluate_policy(id).unwrap()["decision"], "pass");
    assert!(guarded_finish(&f, &corrected).is_err());
    assert_eq!(counts(&f)[4], 0);
    pass_fixture_gates(&f, id);
    let finished = guarded_finish(&f, &corrected).unwrap();
    assert_eq!(finished["landed_snapshot_id"], f.snapshots[11]);
    assert_eq!(
        finished["history_promotion"]["entries"],
        f.prepared["entries"]
    );
    assert_eq!(counts(&f), vec![7, 7, 9, 11, 1]);
    for (entry, before) in f.prepared["entries"]
        .as_array()
        .unwrap()
        .iter()
        .zip(original_receipts)
    {
        assert_eq!(
            f.store
                .get_task(Some("repo"), entry["task_id"].as_str().unwrap())
                .unwrap()["status"],
            if entry["task_id"] == f.prepared["aggregate"]["task_id"] {
                "completed"
            } else {
                "active"
            }
        );
        assert_eq!(
            f.store
                .get_change(Some("repo"), entry["change_ref"].as_str().unwrap())
                .unwrap()["status"],
            if entry["task_id"] == f.prepared["aggregate"]["task_id"] {
                "landed"
            } else {
                "review"
            }
        );
        assert_eq!(
            f.store
                .get_patchset(None, entry["receipt_patchset_id"].as_str().unwrap())
                .unwrap(),
            before
        );
    }
    for (i, original) in original_links.iter().enumerate() {
        assert_eq!(
            &BinaryDbReadTxn::new(&f.db)
                .read_record(WorkflowBinaryV0Codec::snapshot_link_file(), i as u32)
                .unwrap(),
            original
        );
    }
    assert_eq!(guarded_finish(&f, &corrected).unwrap()["replayed"], true);
    assert_eq!(counts(&f)[4], 1);
    validate_frozen_server_workflow_v0(&f.db).unwrap();
}

#[test]
fn aggregate_correction_multiple_refresh_and_old_retry_do_not_reselect_or_duplicate() {
    let f = fixture("aggregate-correction-retry");
    let request = correction_request(&f, &f.prepared, 11, "correction-first");
    let first = f.store.prepare_history_promotion("repo", &request).unwrap();
    let before_retry = counts(&f);
    let retried = f.store.prepare_history_promotion("repo", &request).unwrap();
    assert_eq!(retried["replayed"], true);
    assert_eq!(retried["aggregate"], first["aggregate"]);
    assert_eq!(counts(&f), before_retry);
    let mut short_request = request.clone();
    let full = request["expected_identity"]["patchset_id"]
        .as_str()
        .unwrap();
    let (owner, patch) = full.rsplit_once("/P-").unwrap();
    let task = owner.split('/').next().unwrap();
    short_request["expected_identity"]["patchset_id"] = json!(format!("{task}/P-{patch}"));
    assert_eq!(
        f.store
            .prepare_history_promotion("repo", &short_request)
            .unwrap()["replayed"],
        true
    );
    assert_eq!(counts(&f), before_retry);
    let second_request = correction_request(&f, &first, 12, "correction-second");
    let second = f
        .store
        .prepare_history_promotion("repo", &second_request)
        .unwrap();
    assert!(second["aggregate"]["patchset_id"]
        .as_str()
        .unwrap()
        .ends_with("/P-04"));
    assert_eq!(second["entries"], f.prepared["entries"]);
    assert_eq!(
        second["correction"]["snapshots"].as_array().unwrap().len(),
        2
    );
    assert_eq!(counts(&f), vec![7, 7, 10, 12, 0]);
    let replayed_old = f.store.prepare_history_promotion("repo", &request).unwrap();
    assert_eq!(
        replayed_old["aggregate"]["patchset_id"],
        first["aggregate"]["patchset_id"]
    );
    let change = f
        .store
        .get_change(
            None,
            f.prepared["aggregate"]["change_ref"].as_str().unwrap(),
        )
        .unwrap();
    assert_eq!(
        change["current_patchset_id"],
        second["aggregate"]["patchset_id"]
    );
    let mut conflicting = request.clone();
    conflicting["summary"] = json!("changed payload");
    assert!(f
        .store
        .prepare_history_promotion("repo", &conflicting)
        .unwrap_err()
        .contains("IDEMPOTENCY_CONFLICT"));
    let mut stale = correction_request(&f, &first, 12, "stale-prior");
    stale["expected_identity"]["patchset_id"] = f.prepared["aggregate"]["patchset_id"].clone();
    assert!(f
        .store
        .prepare_history_promotion("repo", &stale)
        .unwrap_err()
        .contains("EXPECTED_CANDIDATE_MISMATCH"));
    assert_eq!(counts(&f), vec![7, 7, 10, 12, 0]);
    pass_fixture_gates(&f, second["aggregate"]["patchset_id"].as_str().unwrap());
    assert_eq!(
        guarded_finish(&f, &second).unwrap()["landed_snapshot_id"],
        f.snapshots[12]
    );
    assert_eq!(counts(&f)[4], 1);
    validate_frozen_server_workflow_v0(&f.db).unwrap();
}

#[test]
fn aggregate_correction_rejects_invalid_identity_and_incomplete_or_altered_dag() {
    for case in [
        "repository",
        "revision",
        "base",
        "missing",
        "duplicate",
        "extra",
        "time",
        "diverged",
    ] {
        let f = fixture(&format!("aggregate-correction-{case}"));
        let mut request = correction_request(&f, &f.prepared, 12, case);
        match case {
            "repository" => request["expected_identity"]["repository_index"] = json!(99),
            "revision" => {
                request["expected_identity"]["revision_snapshot_id"] = json!(f.snapshots[9])
            }
            "base" => request["expected_identity"]["base_snapshot_id"] = json!(f.snapshots[1]),
            "missing" => {
                request["snapshots"].as_array_mut().unwrap().remove(0);
            }
            "duplicate" => {
                let first = request["snapshots"][0].clone();
                request["snapshots"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, first);
            }
            "extra" => request["snapshots"]
                .as_array_mut()
                .unwrap()
                .insert(0, json!({"snapshot_id":f.snapshots[9],"created_at_s":10})),
            "time" => request["snapshots"][0]["created_at_s"] = json!(999),
            "diverged" => request["revision_snapshot_id"] = json!(f.snapshots[9]),
            _ => unreachable!(),
        }
        let before = counts(&f);
        assert!(
            f.store.prepare_history_promotion("repo", &request).is_err(),
            "{case}"
        );
        assert_eq!(counts(&f), before, "{case}");
    }
}

#[test]
fn aggregate_correction_rejects_foreign_snapshot_links_before_append() {
    let f = fixture("aggregate-correction-foreign-link");
    append_capture_tail(&f, 11, "T-0001/C-01", true);
    let before = counts(&f);
    let request = correction_request(&f, &f.prepared, 11, "foreign-link");
    assert!(f
        .store
        .prepare_history_promotion("repo", &request)
        .unwrap_err()
        .contains("foreign"));
    assert_eq!(counts(&f), before);
}

#[test]
fn aggregate_correction_blocks_plain_publish_and_reserved_manifest_forgery() {
    let f = fixture("aggregate-correction-publish-guard");
    let change = f.prepared["aggregate"]["change_ref"].as_str().unwrap();
    for summary in [
        "ordinary correction",
        HISTORY_PROMOTION_SUMMARY_PREFIX,
        HISTORY_PROMOTION_STAGE_SUMMARY_PREFIX,
        LOCAL_LAND_RECEIPT_SUMMARY_PREFIX,
    ] {
        let before = counts(&f);
        let error = f
            .store
            .publish_patchset(
                change,
                &json!({"actor_key_index":0,
                    "base_snapshot_id": f.snapshots[0], "revision_snapshot_id": f.snapshots[11],
                    "summary": summary, "author_mode": "ai_with_human_review"
                }),
            )
            .unwrap_err();
        assert!(error.contains("AGGREGATE_REFRESH_REQUIRED"), "{error}");
        assert_eq!(counts(&f), before);
    }
}

#[test]
fn aggregate_correction_legacy_refresh_without_manifest_cannot_finish() {
    let f = fixture("aggregate-correction-legacy-missing-manifest");
    append_capture_tail(
        &f,
        11,
        f.prepared["aggregate"]["change_ref"].as_str().unwrap(),
        false,
    );
    // Reproduce a persisted pre-fix ordinary refresh through the shared native
    // encoder. Public ordinary publication now rejects this transition.
    let mut tx =
        BinaryDbWriteTxn::begin_serving(&f.db, BinaryDbCommandScope::ServerWorkflow).unwrap();
    f.store
        .prepare_history_promotion_write_set(&mut tx)
        .unwrap();
    let owner = f
        .store
        .workflow_owner(
            &tx,
            f.prepared["aggregate"]["change_ref"]
                .as_str()
                .unwrap_or(f.prepared["aggregate"]["task_id"].as_str().unwrap()),
        )
        .unwrap();
    let (_, legacy_id) = f
        .store
        .append_history_patchset_in_write(
            &mut tx,
            owner,
            0,
            11,
            "",
            &json!({"legacy":"ordinary refresh before the fix"}),
            "ai_with_human_review",
            200,
            1,
        )
        .unwrap();
    tx.commit().unwrap();
    drop(tx);
    pass_fixture_gates(&f, &legacy_id);
    let mut candidate = f.prepared.clone();
    candidate["aggregate"]["patchset_id"] = json!(legacy_id);
    candidate["revision_snapshot_id"] = json!(f.snapshots[11]);
    let before = counts(&f);
    assert!(guarded_finish(&f, &candidate)
        .unwrap_err()
        .contains("AGGREGATE_HISTORY_MANIFEST_REQUIRED"));
    assert_eq!(counts(&f), before);
    assert_eq!(counts(&f)[4], 0);
}

#[test]
fn aggregate_correction_rejects_existing_unbound_history_without_backfill() {
    let f = fixture("aggregate-correction-unbound-history");
    append_capture_tail(
        &f,
        11,
        f.prepared["aggregate"]["change_ref"].as_str().unwrap(),
        false,
    );
    let before = counts(&f);
    let request = correction_request(&f, &f.prepared, 11, "unbound-correction");
    let error = f
        .store
        .prepare_history_promotion("repo", &request)
        .unwrap_err();
    assert!(error.contains("durable capture ownership"), "{error}");
    assert_eq!(counts(&f), before);
    assert_eq!(before[3], 10);
}
