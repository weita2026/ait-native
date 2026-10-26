use super::*;

const CHANGE: &str = "T-0001/C-01";
const PATCHSET: &str = "T-0001/C-01/P-01";

fn fixture(label: &str) -> (TestDb, BinaryDbServerWorkflowV0Store<TestDb>) {
    let db = initialized_db(label);
    seed_content(&db);
    let store = BinaryDbServerWorkflowV0Store::new_frozen(db.clone());
    seed_task_change_and_patchset(&store);
    (db, store)
}

#[test]
fn review_actor_identity_roundtrips_and_separates_equal_names() {
    let (db, store) = fixture("review-actor-identity");
    for payload in [
        json!({"patchset_id": PATCHSET, "reviewer": "Einstein", "reviewer_kind": "bot", "action": "code_review_summary", "comment": "Actual AI Reviewer"}),
        json!({"patchset_id": PATCHSET, "reviewer": "Einstein", "reviewer_kind": "bot", "action": "approve"}),
        json!({"patchset_id": PATCHSET, "reviewer": "Einstein", "action": "approve", "requested_groups": []}),
        json!({"patchset_id": PATCHSET, "reviewer": "Alice", "reviewer_kind": "user", "action": "task_approve"}),
    ] {
        let review = store.record_review(CHANGE, &payload).unwrap();
        assert_eq!(review["reviewer"], payload["reviewer"]);
        assert_eq!(
            review["reviewer_kind"],
            payload
                .get("reviewer_kind")
                .cloned()
                .unwrap_or(json!("user"))
        );
    }
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::actor_file())
            .unwrap(),
        4
    );
    let actor = |index| {
        WorkflowBinaryV0Codec::decode_actor(
            &read
                .read_record(WorkflowBinaryV0Codec::actor_file(), index)
                .unwrap(),
        )
        .unwrap()
    };
    assert_eq!(actor(1).actor_meta & 7, 2);
    assert_eq!(actor(2).actor_meta & 7, 0);
    assert_eq!(actor(1).actor_key_hash, actor(2).actor_key_hash);
    let owners = (0..4)
        .map(|index| {
            WorkflowBinaryV0Codec::decode_review(
                &read
                    .read_record(WorkflowBinaryV0Codec::review_file(), index)
                    .unwrap(),
            )
            .unwrap()
            .actor_index_plus1
        })
        .collect::<Vec<_>>();
    assert_eq!(owners, [2, 2, 3, 4]);
    drop(read);

    let reopened = BinaryDbServerWorkflowV0Store::new_frozen(db);
    let reviews = reopened.list_reviews(CHANGE).unwrap();
    assert_eq!(reviews["reviews"][0]["reviewer_kind"], "bot");
    assert_eq!(reviews["reviews"][2]["reviewer_kind"], "user");
    let projection = reopened.queue_projection_values_nonblocking().unwrap();
    assert_eq!(projection["reviews"][0]["reviewer"], "Einstein");
    assert_eq!(projection["reviews"][0]["reviewer_kind"], "bot");
    let request = reopened
        .request_review(
            CHANGE,
            &json!({"actor_key_index":0,
                "patchset_id": PATCHSET, "reviewer_groups": ["maintainers"]
            }),
        )
        .unwrap();
    assert_eq!(request["reviewer_kind"], "team");
    assert!(request["reviewer"].is_null());
    assert_eq!(request["requested_groups"], json!(["maintainers"]));
}

#[test]
fn review_actor_invalid_and_conflicting_kinds_fail_before_write() {
    let (db, store) = fixture("review-actor-invalid");
    let before = store.get_patchset(None, PATCHSET).unwrap();
    for kind in [
        json!("human"),
        json!("team"),
        json!("unknown"),
        json!("automation"),
        json!("service_account"),
        json!("Bot"),
        json!("bot "),
        json!(""),
        JsonValue::Null,
        json!(2),
        json!(true),
        json!({}),
        json!([]),
    ] {
        let error = store
            .record_review(
                CHANGE,
                &json!({
                    "patchset_id": PATCHSET, "reviewer": "Einstein",
                    "reviewer_kind": kind, "action": "approve"
                }),
            )
            .unwrap_err();
        assert!(error.contains("reviewer_kind"), "{error}");
    }
    for payload in [
        json!({"patchset_id": PATCHSET, "reviewer": "Einstein", "reviewer_kind": "bot", "action": "request"}),
        json!({"patchset_id": PATCHSET, "reviewer": "Einstein", "reviewer_kind": "bot", "action": "approve", "reviewer_groups": ["humans"]}),
    ] {
        assert!(store.record_review(CHANGE, &payload).is_err());
    }
    assert!(store
        .request_review(
            CHANGE,
            &json!({"actor_key_index":0,
                "patchset_id": PATCHSET, "reviewer_kind": "bot", "reviewer_groups": ["humans"]
            })
        )
        .is_err());
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::actor_file())
            .unwrap(),
        1
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::review_file())
            .unwrap(),
        0
    );
    assert_eq!(store.get_patchset(None, PATCHSET).unwrap(), before);
}

#[test]
fn review_actor_bot_allows_generic_policy_but_human_policy_requires_user() {
    let (db, store) = fixture("review-actor-human-policy");
    store.record_review(CHANGE, &json!({
        "patchset_id": PATCHSET, "reviewer": "Einstein", "reviewer_kind": "bot", "action": "approve"
    })).unwrap();
    store.run_patchset_ci(PATCHSET, &json!({})).unwrap();
    store
        .complete_patchset_ci(
            PATCHSET,
            &json!({
                "patchset_id": PATCHSET, "ci_run_seq": 1,
                "selected_suite_count": 1, "suite_result_count": 1,
                "blocking_failure_count": 0, "overall_status": "pass",
                "tests_status": "pass", "lint_status": "pass"
            }),
        )
        .unwrap();
    store
        .put_attestation(
            PATCHSET,
            &json!({
                "verification_state": "pass", "require_tests_pass": true,
                "require_human_review": false
            }),
        )
        .unwrap();
    assert_eq!(store.evaluate_policy(PATCHSET).unwrap()["decision"], "pass");
    store
        .put_attestation(
            PATCHSET,
            &json!({
                "verification_state": "pass", "require_tests_pass": true,
                "require_human_review": true
            }),
        )
        .unwrap();
    assert_eq!(
        store.evaluate_policy(PATCHSET).unwrap()["decision"],
        "pending"
    );
    assert_eq!(
        BinaryDbReadTxn::new(&db)
            .record_count(WorkflowBinaryV0Codec::actor_file())
            .unwrap(),
        2
    );
    let attest = store.get_attestation(PATCHSET).unwrap();
    assert_eq!(attest["require_human_review"], true);
    assert!(attest.get("reviewer").is_none());
    store.record_review(CHANGE, &json!({
        "patchset_id": PATCHSET, "reviewer": "Einstein", "reviewer_kind": "user", "action": "approve"
    })).unwrap();
    assert_eq!(store.evaluate_policy(PATCHSET).unwrap()["decision"], "pass");
}

#[test]
fn review_actor_historical_kinds_are_decoded_without_name_inference() {
    for (kind, name) in [
        (0, "user"),
        (1, "team"),
        (2, "bot"),
        (3, "service_account"),
        (4, "automation"),
        (5, "unknown"),
    ] {
        let (db, store) = fixture(&format!("review-actor-historical-{kind}"));
        store.record_review(CHANGE, &json!({
            "patchset_id": PATCHSET, "reviewer": "Named historical reviewer", "action": "approve"
        })).unwrap();
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerWorkflow).unwrap();
        let mut record = WorkflowBinaryV0Codec::decode_actor(
            &tx.read_record(WorkflowBinaryV0Codec::actor_file(), 1)
                .unwrap(),
        )
        .unwrap();
        record.actor_meta = kind;
        tx.overwrite_record(
            WorkflowBinaryV0Codec::actor_file(),
            1,
            &WorkflowBinaryV0Codec::encode_actor(record).unwrap(),
        )
        .unwrap();
        tx.commit().unwrap();
        drop(tx);
        let reopened = BinaryDbServerWorkflowV0Store::new_frozen(db);
        assert_eq!(
            reopened.list_reviews(CHANGE).unwrap()["reviews"][0]["reviewer_kind"],
            name
        );
        reopened
            .put_attestation(
                PATCHSET,
                &json!({
                    "verification_state": "pass", "require_tests_pass": false,
                    "require_human_review": true
                }),
            )
            .unwrap();
        assert_eq!(
            reopened.evaluate_policy(PATCHSET).unwrap()["decision"],
            if kind == 0 { "pass" } else { "pending" }
        );
    }
}
