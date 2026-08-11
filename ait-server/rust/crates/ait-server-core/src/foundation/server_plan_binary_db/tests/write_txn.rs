use super::*;

#[test]
fn server_plan_write_txn_requires_commit_point() {
    let service = service("write-commit-point-required");
    let root = service.db().authority_root().as_path().to_path_buf();
    let tx = service
        .store()
        .begin_write(ServerPlanBinaryDbWritePurpose::CreatePlan)
        .expect("write txn should begin");

    let error = tx
        .commit()
        .expect_err("commit without commit point should fail");

    assert!(error.contains("reached commit without a commit point"));
    assert!(!root.join("server-plan.write.journal").exists());
}

#[test]
fn server_plan_write_txn_rejects_wrong_commit_point_for_purpose() {
    let service = service("write-wrong-commit-point");
    let mut tx = service
        .store()
        .begin_write(ServerPlanBinaryDbWritePurpose::CreatePlan)
        .expect("write txn should begin");

    let error = tx
        .set_commit_point(ServerPlanBinaryDbCommitPoint::PlanStatusUpdated { plan_index: 0 })
        .expect_err("wrong commit point should fail");

    assert!(error.contains("cannot commit"));
}

#[test]
fn server_plan_write_txn_rejects_wrong_append_for_purpose() {
    let service = service("write-wrong-append");
    let mut tx = service
        .store()
        .begin_write(ServerPlanBinaryDbWritePurpose::CreatePlan)
        .expect("write txn should begin");
    let record = PlanRecord {
        plan_meta: PLAN_STATE_DRAFT_META,
        reserved0: 0,
        payload_len: 0,
        payload_offset: 0,
        latest_revision_index_plus1: 0,
        published_plan_index_plus1: 0,
        published_latest_revision_index_plus1: 0,
        created_at_s: 0,
        updated_at_s: 0,
        published_at_s: 0,
    };

    let error = tx
        .overwrite_plan(0, record, b"title")
        .expect_err("CreatePlan must not overwrite an existing plan");

    assert!(error.contains("cannot perform"));
}

#[test]
fn server_plan_write_txn_rejects_a_stale_intent_under_the_lock_before_append() {
    let service = service("write-plan-cas");
    let created = service
        .create_plan("repo-bin", &create_payload("CAS plan", "SBDB-CAS-1"))
        .expect("plan should create");
    let plan_id = created["plan_id"].as_str().expect("plan id");
    let (expected, _) = service
        .store()
        .current_plan_record(0)
        .expect("initial plan should read");

    service
        .update_plan_status(plan_id, &json!({"status": "archived"}))
        .expect("the first prepared intent should update the plan");

    let error = service
        .store()
        .begin_write_with_plan_cas(ServerPlanBinaryDbWritePurpose::RevisePlan, 0, &expected)
        .err()
        .expect("the stale prepared intent should fail under the write lock");

    assert!(error.contains("state advanced under the Binary DB write lock"));
    assert_eq!(
        service
            .store()
            .record_count(plan_revision_file())
            .expect("revision count should read"),
        1,
        "CAS failure must happen before a revision append"
    );
    assert_eq!(
        service
            .store()
            .record_count(plan_file())
            .expect("plan count should read"),
        1,
        "the winning status update must overwrite the stable plan record"
    );
    assert!(
        !service
            .db()
            .authority_root()
            .as_path()
            .join("server-plan.write.journal")
            .exists(),
        "the rejected transaction should clean up its journal"
    );
}

#[test]
fn server_plan_write_layout_is_const_generic_and_fails_closed() {
    let service_v1: BinaryDbServerPlanServiceV1<FilesystemServerRemoteBinaryDb> =
        service("write-layout-v1");
    let _tx = service_v1
        .store()
        .begin_write(ServerPlanBinaryDbWritePurpose::CreatePlan)
        .expect("v1 write txn should begin");
    let unsupported_layout_service = BinaryDbServerPlanService::<
        FilesystemServerRemoteBinaryDb,
        UNSUPPORTED_TEST_LAYOUT,
    >::with_write_layout(
        FilesystemServerRemoteBinaryDb::test_fixture(
            RepoId::new("REPO-PLAN-BIN"),
            RepoName::new("repo-bin"),
            make_root("unsupported-write-layout"),
            StoreGeneration::new(1),
        ),
    );

    let error = unsupported_layout_service
        .store()
        .begin_write(ServerPlanBinaryDbWritePurpose::CreatePlan)
        .err()
        .expect("unsupported write layout should fail closed");

    assert!(error.contains(&format!(
        "unsupported server Plan Binary DB write layout {UNSUPPORTED_TEST_LAYOUT}"
    )));
}

#[test]
fn server_plan_compact_v1_reads_use_persisted_layout_not_write_layout() {
    let service_v1 = service("persisted-read-layout");
    let created = service_v1
        .create_plan(
            "repo-bin",
            &create_payload("Persisted layout", "SBDB-PARITY-11"),
        )
        .expect("v1 plan should create");
    let plan_id = created["plan_id"].as_str().expect("plan id").to_string();
    let legacy_hash_path = service_v1
        .db()
        .authority_root()
        .as_path()
        .join("plan_item_ref.hash");
    fs::write(&legacy_hash_path, b"stale hash side file must be ignored")
        .expect("legacy hash side file should seed");
    let unsupported_layout_reader = BinaryDbServerPlanService::<
        FilesystemServerRemoteBinaryDb,
        UNSUPPORTED_TEST_LAYOUT,
    >::with_write_layout(service_v1.db().clone());

    let detail = unsupported_layout_reader
        .get_plan(&plan_id)
        .expect("v1 persisted files should read through unsupported write facade");

    assert_eq!(detail["title"], "Persisted layout");
    assert_eq!(
        detail["head_revision"]["items"][0]["plan_item_ref"],
        "SBDB-PARITY-11"
    );
    assert!(
        legacy_hash_path.exists(),
        "fixture should prove stale plan_item_ref.hash is ignored, not deleted"
    );
    let write_error = match unsupported_layout_reader
        .store()
        .begin_write(ServerPlanBinaryDbWritePurpose::CreatePlan)
    {
        Ok(_) => panic!("unsupported write layout must still fail closed"),
        Err(err) => err,
    };
    assert!(write_error.contains(&format!(
        "unsupported server Plan Binary DB write layout {UNSUPPORTED_TEST_LAYOUT}"
    )));
}

#[test]
fn server_plan_write_txn_rejects_append_after_commit_point() {
    let service = service("write-commit-point-closes");
    let mut tx = service
        .store()
        .begin_write(ServerPlanBinaryDbWritePurpose::CreatePlan)
        .expect("write txn should begin");
    tx.set_commit_point(ServerPlanBinaryDbCommitPoint::PlanCreated {
        plan_index: 0,
        revision_index: 0,
    })
    .expect("matching commit point should set");

    let error = tx
        .append_items(&[])
        .expect_err("append after commit point should fail");

    assert!(error.contains("already reached commit point"));
    tx.commit().expect("empty guarded txn should still close");
}
