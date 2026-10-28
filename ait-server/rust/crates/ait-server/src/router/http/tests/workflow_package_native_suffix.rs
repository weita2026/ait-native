use super::*;
use ait_server_core::foundation::server_content_binary_db::ServerBinaryDbLineStore;
use ait_server_core::foundation::server_operational_repository_registry::ServerOperationalRepositoryRegistry;
use ait_server_core::foundation::server_workflow_store::ServerWorkflowChangeStore;
use ait_server_core::foundation::workflow_package::WorkflowPackage;

const CORE_ROUTE: &str = "/v1/native/repository-authorities/0";
// Unmodified outputs of Core's native_server_package_inputs replay fixture.
const PREFIX: &[u8] = include_bytes!("fixtures/native_suffix/accepted-prefix.zst");
const SUFFIX: &[u8] = include_bytes!("fixtures/native_suffix/fresh-suffix.zst");

fn empty_namespace_state(root: &FsPath) -> ServerState {
    let generation = root.join("generation");
    initialize_fresh_generation(&generation, 1_786_000_000).unwrap();
    let registry = ServerOperationalRepositoryRegistry::new(
        generation.join("global"),
        generation.join("repositories"),
    )
    .unwrap();
    let entry = registry.get(0).unwrap();
    registry
        .update_live_metadata(
            0,
            entry.record.lifecycle_kind,
            [0, 0],
            entry.record.policy_flags,
            entry.record.updated_at_s,
        )
        .unwrap();
    state_from_operational(Arc::new(
        OperationalBinaryRuntime::open_generation(
            generation,
            root.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .unwrap(),
    ))
}

async fn package_evidence(
    app: &Router,
    identity: &JsonValue,
    key: &str,
    operation: &str,
    payload: JsonValue,
) {
    let (status, result) = call(app, "POST", &format!("{CORE_ROUTE}/worker-jobs/{}", identity["worker_job_index"]),
        Some(json!({"contract":"workflow-package-evidence/v1","operation":operation,
            "idempotency_key":key,"actor_key_index":0,"expected_identity":identity,"payload":payload}))).await;
    assert_eq!(status, StatusCode::OK, "{operation}: {result}");
}

async fn apply_package(app: &Router, raw: &[u8]) -> (JsonValue, JsonValue) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("{CORE_ROUTE}/remote-sync/zstd-bulk/commit"))
                .header(CONTENT_TYPE, PACKAGE_CONTENT_TYPE)
                .header("x-ait-package-sha256", format!("{:x}", Sha256::digest(raw)))
                .header("x-ait-actor-key-index", "0")
                .body(Body::from(raw.to_vec()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body: JsonValue =
        serde_json::from_slice(&to_bytes(response.into_body()).await.unwrap()).unwrap();
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "native Core package prepare: {body}"
    );
    let identity = body["expected_identity"].clone();
    let job = format!("{CORE_ROUTE}/worker-jobs/{}", identity["worker_job_index"]);
    let (status, claim) = call(
        app,
        "POST",
        &format!("{job}:claim"),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let proof = &claim["claimed_job"];
    // Controlled protocol evidence in an isolated fixture, not a real Runner
    // qualification or independent review of the release candidate.
    let (status, completed) = call(app, "POST", &format!("{job}:complete"), Some(json!({
        "attempt_count":proof["attempt_count"],"lease_token":proof["lease_token"],
        "detail":{"job_kind":7,"result":{"contract":"ait.runner.native-result.v1","status":"succeeded",
            "tests_status":"pass","suite_result_count":2,"suite_results":[
                {"suite_id":"rust_tests","status":"pass","blocking":true},
                {"suite_id":"cargo_fmt","status":"pass","blocking":true}],"cleanup":{}}}}))).await;
    assert_eq!(status, StatusCode::OK, "{completed}");
    package_evidence(app, &identity, "native-attest", "attestation", json!({"author_mode":"ai_only_experimental",
        "verification_state":"pass","require_tests_pass":true,"require_lint_pass":true,"require_human_review":false})).await;
    for action in ["code_review_summary", "task_approve"] {
        package_evidence(
            app,
            &identity,
            action,
            "review",
            json!({"reviewer":"Independent fixture reviewer",
            "reviewer_kind":"bot","action":action,"comment":"Controlled native package fixture"}),
        )
        .await;
    }
    let (status, result) = call(
        app,
        "POST",
        &format!("{CORE_ROUTE}/task-land"),
        Some(finish_request(&identity)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "native Core package finish: {result}"
    );
    (identity, result)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn native_core_suffix_only_package_preserves_accepted_prefix_and_change_owner() {
    let root = TestDirectory::new("native-core-suffix");
    let state = empty_namespace_state(root.path());
    let app = build_router_with_state(state.clone());
    assert_eq!(
        register(&app, request_for(&state, 0)).await.1["actor_key_index"],
        0
    );
    let hash = format!("{:x}", Sha256::digest(PREFIX));
    let prefix = WorkflowPackage::unpack(
        PREFIX,
        &root.path().join("prefix-inputs"),
        0,
        &hash,
        usize::MAX,
    )
    .unwrap();
    let suffix = WorkflowPackage::unpack(
        SUFFIX,
        &root.path().join("suffix-inputs"),
        0,
        &format!("{:x}", Sha256::digest(SUFFIX)),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(prefix.manifest.history_entries.len(), 2);
    assert_eq!(suffix.manifest.history_entries.len(), 1);
    assert_eq!(suffix.manifest.history_entries[0].changes.len(), 1);
    assert_eq!(
        suffix.manifest.base_snapshot_id,
        prefix.manifest.revision_snapshot_id
    );
    assert!(prefix
        .manifest
        .history_entries
        .iter()
        .all(|entry| entry.local_task_id != suffix.manifest.history_entries[0].local_task_id));
    for pack in &prefix.manifest.content.object_packs {
        let id = pack["pack_id"].as_str().unwrap();
        state
            .repository_service
            .put_zstd_bulk_object_pack(
                "0",
                id,
                fs::read(prefix.inputs.join("objects").join(id)).unwrap(),
            )
            .unwrap();
    }
    for pack in &prefix.manifest.content.tree_packs {
        let id = pack["pack_id"].as_str().unwrap();
        state
            .repository_service
            .put_zstd_bulk_tree_pack(
                "0",
                id,
                fs::read(prefix.inputs.join("trees").join(id)).unwrap(),
            )
            .unwrap();
    }
    // Seed only the actual baseline Snapshot and its immutable content. Formal
    // prefix mappings/receipts must come from accepting the unchanged package.
    let mut seed = serde_json::to_value(&prefix.manifest.content).unwrap();
    seed.as_object_mut().unwrap().remove("capture_binding");
    seed.as_object_mut().unwrap().remove("line_update");
    seed["actor_key_index"] = json!(0);
    seed["contract"] = json!("ait.remote_sync.zstd_bulk.commit.v1");
    seed["snapshots"] = json!(prefix
        .manifest
        .content
        .snapshots
        .iter()
        .filter(
            |snapshot| snapshot["snapshot_id"].as_str() == Some(&prefix.manifest.base_snapshot_id)
        )
        .collect::<Vec<_>>());
    let (status, result) = call(
        &app,
        "POST",
        &format!("{CORE_ROUTE}/remote-sync/zstd-bulk/commit"),
        Some(seed),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "baseline seed: {result}");
    let (_, db) = state.operational_binary.repository_db(0).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    let base = ait_server_core::foundation::server_content_binary_db::ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone())
        .snapshot_by_id(&read, &prefix.manifest.base_snapshot_id).unwrap().unwrap().0;
    drop(read);
    ServerBinaryDbLineStore::<_, 1>::new(db.clone())
        .set_line_head("main", base + 1, 1_786_000_001)
        .unwrap();
    let (_, accepted) = apply_package(&app, PREFIX).await;
    assert_eq!(accepted["entries"].as_array().unwrap().len(), 2);
    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(db.clone(), "")
        .unwrap()
        .with_repository_index(0);
    for source in &prefix.manifest.history_entries {
        let mapping = accepted["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|mapping| {
                mapping["local_task_id"].as_str() == Some(source.local_task_id.as_str())
            })
            .unwrap();
        for snapshot in &source.snapshots {
            assert!(snapshot.local_change_ref.is_none());
            assert_eq!(
                workflow
                    .snapshot_workflow_owner(&snapshot.snapshot_id)
                    .unwrap(),
                json!({"task_id":mapping["remote_task_id"],"change_ref":null})
            );
        }
    }
    let authority = ServerRemoteBinaryDb::authority_root(&db);
    let frozen_prefix = [
        WorkflowBinaryV0Codec::task_file(),
        WorkflowBinaryV0Codec::change_file(),
        WorkflowBinaryV0Codec::land_file(),
        WorkflowBinaryV0Codec::snapshot_link_file(),
    ]
    .map(|file| {
        (
            file.as_str().to_string(),
            fs::read(authority.as_path().join(file.as_str())).unwrap(),
        )
    });
    let (identity, accepted_suffix) = apply_package(&app, SUFFIX).await;
    assert_eq!(accepted_suffix["entries"].as_array().unwrap().len(), 1);
    for (file, bytes) in frozen_prefix {
        assert!(
            fs::read(authority.as_path().join(&file))
                .unwrap()
                .starts_with(&bytes),
            "accepted prefix changed: {file}"
        );
    }
    let task = accepted_suffix["entries"][0]["remote_task_id"]
        .as_str()
        .unwrap();
    let change = workflow.get_change(None, &format!("{task}/C-01")).unwrap();
    assert_eq!(
        change["fork_snapshot_id"],
        suffix.manifest.history_entries[0].changes[0].change["fork_snapshot_id"]
    );
    let source = &suffix.manifest.history_entries[0];
    assert!(source.snapshots.len() >= 2);
    for snapshot in &source.snapshots {
        assert_eq!(
            snapshot.local_change_ref.as_deref(),
            Some(source.changes[0].local_change_ref.as_str())
        );
        assert_eq!(
            workflow
                .snapshot_workflow_owner(&snapshot.snapshot_id)
                .unwrap(),
            json!({"task_id":task,"change_ref":format!("{task}/C-01")})
        );
    }
    let (status, retry) = call(
        &app,
        "POST",
        &format!("{CORE_ROUTE}/task-land"),
        Some(finish_request(&identity)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{retry}");
    // Existing cleanup readback retains acceptance IDs without inventing
    // Local mapping membership from an unsealed caller assertion.
    assert_eq!(retry["outcome"], "incomplete");
    assert_eq!(retry["entries"], json!([]));
    assert_eq!(retry["patchset_id"], accepted_suffix["patchset_id"]);
    assert_eq!(retry["submission_id"], accepted_suffix["submission_id"]);
}
