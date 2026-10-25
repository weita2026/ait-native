use super::*;
use ait_server_core::foundation::remote_binary_db::ServerRemoteBinaryDb;
use ait_server_core::foundation::server_content_binary_db::{
    ServerBinaryDbLineStore, ServerBinarySnapshotCodec,
};
use ait_server_core::foundation::server_operational_worker_jobs::{
    ServerOperationalWorkerJobStore, WorkerJobDomainAuthority,
};
use ait_server_core::foundation::workflow_binary_v0_adapter::BinaryDbServerWorkflowV0Store;
use ait_server_core::foundation::workflow_package::WorkflowPackageManifest;
use ait_server_core::foundation::workflow_package_contract::PackageFinishRequest;

#[path = "workflow_package_rebased_change.rs"]
mod rebased_change;

#[path = "workflow_package_cleanup.rs"]
mod cleanup;

#[path = "workflow_package_restart.rs"]
mod restart;

#[path = "workflow_package_mode_read.rs"]
mod mode_read;

const PEER: &str = "SNP-000000000003";
const ORIGINAL_TASK: &str = "SNP-000000000004";
const ORIGINAL_CHANGE: &str = "SNP-000000000005";
const TESTED_TASK: &str = "SNP-000000000006";
const TESTED_CHANGE: &str = "SNP-000000000007";

fn tree_id(number: u64) -> String {
    format!("TRE-{number:020X}")
}

fn directory(name: &str, target: u64) -> JsonValue {
    json!({"entry_name":name,"entry_type":"tree","target_id":tree_id(target),"mode":"tree","size_bytes":null})
}

fn file(name: &str, blob: &str) -> JsonValue {
    json!({"entry_name":name,"entry_type":"blob","target_id":blob,"mode":"100644","size_bytes":512})
}

fn archive(manifest: &JsonValue, members: &BTreeMap<String, Vec<u8>>) -> (Vec<u8>, String) {
    let mut tar = Vec::new();
    tar_member(
        &mut tar,
        "manifest.json",
        &serde_json::to_vec(manifest).unwrap(),
    );
    for (path, bytes) in members {
        tar_member(&mut tar, path, bytes);
    }
    tar.resize(tar.len() + 1024, 0);
    let mut encoder = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
    encoder.include_contentsize(true).unwrap();
    encoder.include_checksum(false).unwrap();
    encoder
        .set_pledged_src_size(Some(tar.len() as u64))
        .unwrap();
    encoder.write_all(&tar).unwrap();
    let raw = encoder.finish().unwrap();
    let hash = format!("{:x}", Sha256::digest(&raw));
    (raw, hash)
}

async fn evidence(
    app: &Router,
    identity: &JsonValue,
    key: &str,
    operation: &str,
    payload: JsonValue,
) {
    let body = json!({"contract":"workflow-package-evidence/v1","operation":operation,"idempotency_key":key,"actor_key_index":0,"expected_identity":identity,"payload":payload});
    let (status, result) = call(
        app,
        "POST",
        &format!(
            "{ROUTE}/worker-jobs/{}",
            identity["worker_job_index"].as_u64().unwrap()
        ),
        Some(body),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
}

async fn attest(app: &Router, identity: &JsonValue, mode: &str) {
    evidence(app,identity,"attest","attestation",json!({"author_mode":mode,"verification_state":"pass","require_tests_pass":true,"require_lint_pass":true,"require_human_review":mode=="ai_with_human_review"})).await;
}

async fn review(app: &Router, identity: &JsonValue, key: &str, action: &str) {
    evidence(app,identity,key,"review",json!({"reviewer":"Independent fixture reviewer","reviewer_kind":"bot","action":action,"comment":"Review applies to this exact disposable candidate"})).await;
}

fn finish_request(identity: &JsonValue) -> JsonValue {
    json!({"contract":"task-land-package-guarded/v1","idempotency_key":format!("workflow-package:{}:{}",identity["package_hash"].as_str().unwrap(),identity["worker_job_index"].as_u64().unwrap()),"actor_key_index":0,"target_line":"main","mode":"direct","expected_identity":identity})
}

async fn finish(app: &Router, identity: &JsonValue) -> (StatusCode, JsonValue) {
    call(
        app,
        "POST",
        &format!("{ROUTE}/task-land"),
        Some(finish_request(identity)),
    )
    .await
}

struct CompleteFixture {
    delta: DeltaFixture,
    mapped: JsonValue,
}

async fn complete_fixture(root: &FsPath, invalid: Option<&str>) -> CompleteFixture {
    let mut f = delta_fixture(root, true).await;
    // Establish the existing mapping with a genuine native publication, never
    // a fabricated successful Remote receipt.
    let initial_members = BTreeMap::from([(
        format!("trees/{PACK}"),
        fs::read(root.join("initial-inputs/trees").join(PACK)).unwrap(),
    )]);
    let mut initial = f.manifest.clone();
    initial["content"]["object_packs"] = json!([]);
    initial["content"]["blob_locators"] = json!([]);
    let (raw, hash) = archive(&initial, &initial_members);
    let (status, ready) = upload(&f.app, &raw, &hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    let identity = &ready["expected_identity"];
    complete(
        &f.app,
        identity["worker_job_index"].as_u64().unwrap(),
        "pass",
    )
    .await;
    attest(&f.app, identity, "ai_with_human_review").await;
    evidence(
        &f.app,
        identity,
        "human",
        "review",
        json!({"reviewer":"Alice","reviewer_kind":"user","action":"approve"}),
    )
    .await;
    review(&f.app, identity, "summary", "code_review_summary").await;
    review(&f.app, identity, "task-approved", "task_approve").await;
    let (status, mapped) = finish(&f.app, identity).await;
    assert_eq!(status, StatusCode::OK, "{mapped}");

    // A concurrent native content commit moves the tested base. The original
    // Local receipt chain still starts at REVISION, not at this peer Snapshot.
    let peer = json!({"contract":"ait.remote_sync.zstd_bulk.commit.v1","actor_key_index":0,"object_packs":[],"tree_packs":[],"blob_locators":[],"tree_locators":[],"snapshots":[snapshot(PEER,Some(REVISION),300)]});
    let (status, result) = call(
        &f.app,
        "POST",
        &format!("{ROUTE}/remote-sync/zstd-bulk/commit"),
        Some(peer),
    )
    .await;
    assert!(status.is_success(), "{result}");
    let (_, db) = f.state.operational_binary.repository_db(1).unwrap();
    ServerBinaryDbLineStore::<_, 1>::new(db)
        .set_line_head("main", 3, 300)
        .unwrap();

    let mut members = BTreeMap::new();
    for row in f.manifest["content"]["object_packs"].as_array().unwrap() {
        let id = row["pack_id"].as_str().unwrap();
        members.insert(format!("objects/{id}"), fs::read(root.join(id)).unwrap());
    }
    // Pack groups A -> B -> C -> A are cyclic, while the actual Tree graph
    // root -> middle -> bottom -> leaf is a valid DAG. Blobs include an early
    // delta whose base lives in the later Object pack.
    let mut root_rows = vec![
        directory("existing", 1),
        directory("nested", 3),
        directory("same", 4),
        file("base", &f.base),
    ];
    if invalid == Some("conflict") {
        root_rows.retain(|row| row["entry_name"] != "existing");
    }
    let leaf_rows = vec![file("delta", &f.delta)];
    let mut declarations = vec![
        (2, vec![(2, root_rows), (4, leaf_rows)]),
        (3, vec![(3, vec![directory("deeper", 5)])]),
        (4, vec![(5, vec![directory("back-to-first-pack", 4)])]),
    ];
    // Invalid retained history need not be reachable from the tested revision.
    // Ready materializes only that revision; finish must validate every pack.
    if invalid == Some("missing") {
        declarations[1].1.push((20, vec![directory("missing", 99)]));
    }
    if invalid == Some("cycle") {
        declarations[1].1.push((20, vec![directory("later", 21)]));
        declarations[2].1.push((21, vec![directory("back", 20)]));
    }
    if invalid == Some("conflict") {
        declarations[2].1.push((1, vec![file("changed", &f.base)]));
    }
    let mut tree_packs = Vec::new();
    let mut tree_locators = Vec::new();
    let mut root_ordinal = 0;
    for (pack_number, trees) in declarations {
        let id = format!("TPK-{pack_number:012X}");
        let rows = trees
            .iter()
            .flat_map(|(number, rows)| {
                rows.iter().map(move |row| {
                    let mut row = row.clone();
                    row["tree_id"] = json!(tree_id(*number));
                    row
                })
            })
            .collect::<Vec<_>>();
        let declarations = trees
            .iter()
            .map(|(number, rows)| json!({"tree_id":tree_id(*number),"entry_count":rows.len()}))
            .collect::<Vec<_>>();
        let tree_members = build_tree_pack_members(&json!(declarations), &json!(rows)).unwrap();
        let path = root.join(&id);
        let mut metadata = write_tree_pack_archive(
            path.to_str().unwrap(),
            &id,
            "1970-01-01T00:06:40Z",
            &tree_members,
        )
        .unwrap();
        metadata["pack_id"] = json!(id);
        for tree in metadata["pack_index"]["trees"].as_array().unwrap() {
            if tree["tree_id"] == tree_id(2) {
                root_ordinal = tree["entry_ordinal"].as_u64().unwrap();
            }
            tree_locators.push(json!({"tree_id":tree["tree_id"],"tree_pack_id":id,"entry_count":tree["entry_count"],"tree_pack_checksum":tree["checksum"],"created_at":"1970-01-01T00:06:40Z"}));
        }
        members.insert(format!("trees/{id}"), fs::read(path).unwrap());
        tree_packs.push(metadata);
    }
    tree_locators.sort_by(|a, b| a["tree_id"].as_str().cmp(&b["tree_id"].as_str()));
    f.manifest["author_mode"] = json!("ai_only_experimental");
    f.manifest["base_snapshot_id"] = json!(PEER);
    f.manifest["revision_snapshot_id"] = json!(TESTED_CHANGE);
    f.manifest["history_entries"][0]["expected_remote_task_id"] =
        mapped["entries"][0]["remote_task_id"].clone();
    f.manifest["history_entries"].as_array_mut().unwrap().push(json!({"local_task_id":"LSET-0002","expected_remote_task_id":null,"task":{"title":"Original mixed source","intent":"Keep original and rebased owners exact"},"pre_land_target_snapshot_id":REVISION,"landed_snapshot_id":ORIGINAL_CHANGE,"landed_at_s":550,
        "changes":[{"local_change_id":"C-01","local_change_ref":"LSET-0002/C-01","expected_remote_change_ref":null,"change":{"title":"Actual source Change","base_line":"main","fork_snapshot_id":REVISION}}],
        "snapshots":[{"snapshot_id":ORIGINAL_TASK,"created_at_s":400,"local_change_ref":null},{"snapshot_id":ORIGINAL_CHANGE,"created_at_s":500,"local_change_ref":"LSET-0002/C-01"},{"snapshot_id":TESTED_TASK,"created_at_s":600,"local_change_ref":null},{"snapshot_id":TESTED_CHANGE,"created_at_s":700,"local_change_ref":"LSET-0002/C-01"}]}));
    let snapshots = [
        (ORIGINAL_TASK, REVISION, 400),
        (ORIGINAL_CHANGE, ORIGINAL_TASK, 500),
        (TESTED_TASK, PEER, 600),
        (TESTED_CHANGE, TESTED_TASK, 700),
    ]
    .into_iter()
    .map(|(id, parent, time)| {
        let mut row = snapshot(id, Some(parent), time);
        row["root_tree_pack_id"] = json!("TPK-000000000002");
        row["root_entry_ordinal"] = json!(root_ordinal);
        row["file_count"] = json!(3);
        row["total_bytes"] = json!(1536);
        row
    })
    .collect::<Vec<_>>();
    f.manifest["content"]["snapshots"] = json!(snapshots);
    f.manifest["content"]["tree_packs"] = json!(tree_packs);
    f.manifest["content"]["tree_locators"] = json!(tree_locators);
    (f.raw, f.hash) = archive(&f.manifest, &members);
    CompleteFixture { delta: f, mapped }
}

fn counts(state: &ServerState) -> Vec<u32> {
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    [
        WorkflowBinaryV0Codec::task_file(),
        WorkflowBinaryV0Codec::change_file(),
        WorkflowBinaryV0Codec::patchset_file(),
        WorkflowBinaryV0Codec::review_file(),
        WorkflowBinaryV0Codec::attest_file(),
        WorkflowBinaryV0Codec::policy_file(),
        WorkflowBinaryV0Codec::land_file(),
        ServerBinarySnapshotCodec::<2>::record_file(),
    ]
    .into_iter()
    .map(|file| read.record_count(file).unwrap())
    .collect()
}

struct ReadDomain;
impl WorkerJobDomainAuthority for ReadDomain {
    fn validate_patchset_index(
        &self,
        _: u32,
    ) -> ait_server_core::foundation::remote_binary_db::StoreResult<()> {
        Ok(())
    }
    fn validate_snapshot_index(
        &self,
        _: u32,
    ) -> ait_server_core::foundation::remote_binary_db::StoreResult<()> {
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn complete_batch_mixed_mapping_rebased_owners_and_governance_commit_once() {
    let root = TestDirectory::new("package-complete-batch");
    let f = complete_fixture(root.path(), None).await;
    let before = counts(&f.delta.state);
    let (status, ready) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    let identity = &ready["expected_identity"];
    let job = identity["worker_job_index"].as_u64().unwrap();
    complete(&f.delta.app, job, "pass").await;
    attest(&f.delta.app, identity, "ai_only_experimental").await;
    review(&f.delta.app, identity, "summary", "code_review_summary").await;
    // Creating the aggregate makes Task review pending. A summary alone is
    // insufficient; only an exact Task approval clears that pending gate.
    let (status, blocked) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    assert!(
        blocked
            .to_string()
            .contains("PACKAGE_TASK_APPROVAL_REQUIRED"),
        "{blocked}"
    );
    assert_eq!(counts(&f.delta.state), before);
    let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    assert!(content.tree(&tree_id(2)).unwrap().is_none());
    assert!(content.blob(&f.delta.delta).unwrap().is_none());
    review(&f.delta.app, identity, "blocked", "task_request_changes").await;
    let (status, blocked) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    assert!(
        blocked.to_string().contains("PACKAGE_REVIEW_BLOCKED"),
        "{blocked}"
    );
    assert_eq!(counts(&f.delta.state), before);
    review(&f.delta.app, identity, "task-approved", "task_approve").await;
    let (status, result) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "confirmed");
    assert_eq!(result["entries"].as_array().unwrap().len(), 2);
    assert_eq!(result["entries"][0], f.mapped["entries"][0]);
    assert_eq!(
        result["entries"][1]["changes"][0]["local_change_ref"],
        "LSET-0002/C-01"
    );
    assert_eq!(counts(&f.delta.state)[0], 2);
    assert_eq!(counts(&f.delta.state)[6], 2);
    assert_eq!(counts(&f.delta.state)[2], 4); // two source receipts, two aggregates
    assert_eq!(
        content
            .blob_bytes_with_read(&BinaryDbReadTxn::new(&db), &f.delta.delta)
            .unwrap(),
        Some(f.delta.expected_delta)
    );
    for (id, change) in [
        (ORIGINAL_TASK, false),
        (ORIGINAL_CHANGE, true),
        (TESTED_TASK, false),
        (TESTED_CHANGE, true),
    ] {
        let (_, snapshot) = call(
            &f.delta.app,
            "GET",
            &format!("{ROUTE}/snapshots/{id}"),
            None,
        )
        .await;
        assert_eq!(
            snapshot["workflow_owner"]["task_id"],
            result["entries"][1]["remote_task_id"]
        );
        assert_eq!(
            snapshot["workflow_owner"]["change_ref"],
            if change {
                result["entries"][1]["changes"][0]["remote_change_ref"].clone()
            } else {
                JsonValue::Null
            }
        );
    }
    // Query the original Job and immutable formal records with the retained
    // manifest, just as recovery does; lookup must never apply content again.
    let (entry, _) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let namespace = entry
        .record
        .namespace_ascii
        .into_iter()
        .filter(|b| *b != 0)
        .map(char::from)
        .collect::<String>();
    let jobs = ServerOperationalWorkerJobStore::new(
        1,
        ServerRemoteBinaryDb::authority_root(&db).as_path(),
        Arc::new(ReadDomain),
    )
    .unwrap();
    let bound = jobs.get(job as u32).unwrap();
    let workflow = BinaryDbServerWorkflowV0Store::new_remote_frozen(db, &namespace)
        .unwrap()
        .with_repository_index(1);
    let source =
        WorkflowPackageManifest::parse(&serde_json::to_vec(&f.delta.manifest).unwrap()).unwrap();
    let request = PackageFinishRequest::parse(&finish_request(identity)).unwrap();
    let after = counts(&f.delta.state);
    let looked_up = workflow
        .lookup_workflow_package_finish(bound, &request, Some(&source))
        .unwrap()
        .unwrap();
    assert_eq!(looked_up["outcome"], "incomplete");
    assert_eq!(looked_up["expected_identity"], result["expected_identity"]);
    assert_eq!(looked_up["patchset_id"], result["patchset_id"]);
    assert_eq!(looked_up["submission_id"], result["submission_id"]);
    assert_eq!(looked_up["entries"], json!([]));
    assert_eq!(counts(&f.delta.state), after);
    let (status, replayed) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::OK, "{replayed}");
    assert_eq!(replayed["patchset_id"], result["patchset_id"]);
    assert_eq!(counts(&f.delta.state), after);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn complete_batch_invalid_tree_or_stale_base_rolls_back_every_authoritative_write() {
    for invalid in ["missing", "cycle", "conflict", "stale"] {
        let root = TestDirectory::new("package-complete-invalid");
        let f = complete_fixture(root.path(), Some(invalid)).await;
        let (status, ready) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
        if invalid != "stale" {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid}: {ready}");
            let message = match invalid {
                "missing" => "is missing for tree entry",
                "cycle" => "cyclic Tree",
                _ => "already exists",
            };
            assert!(ready.to_string().contains(message), "{invalid}: {ready}");
            continue;
        }
        assert_eq!(status, StatusCode::ACCEPTED, "{invalid}: {ready}");
        let identity = &ready["expected_identity"];
        let job = identity["worker_job_index"].as_u64().unwrap();
        complete(&f.delta.app, job, "pass").await;
        attest(&f.delta.app, identity, "ai_only_experimental").await;
        review(&f.delta.app, identity, "approve", "task_approve").await;
        let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
        if invalid == "stale" {
            ServerBinaryDbLineStore::<_, 1>::new(db.clone())
                .set_line_head("main", 2, 800)
                .unwrap();
        }
        let before = counts(&f.delta.state);
        let (status, result) = finish(&f.delta.app, identity).await;
        assert_eq!(status, StatusCode::CONFLICT, "{invalid}: {result}");
        let message = match invalid {
            "missing" => "is missing for tree entry",
            "cycle" => "cyclic Tree",
            "conflict" => "already exists",
            _ => "target no longer equals the tested base",
        };
        assert!(result.to_string().contains(message), "{invalid}: {result}");
        assert_eq!(counts(&f.delta.state), before);
        let content = ServerBinaryRepositoryContentStore::new(db);
        assert!(content.tree(&tree_id(2)).unwrap().is_none());
        assert!(content.tree_pack("TPK-000000000002").unwrap().is_none());
        assert!(content.blob(&f.delta.delta).unwrap().is_none());
        assert!(content.object_pack("PCK-000000000001").unwrap().is_none());
        let (_, current) = call(
            &f.delta.app,
            "GET",
            &format!("{ROUTE}/worker-jobs/{job}"),
            None,
        )
        .await;
        assert!(current["job"]["patchset_index"].is_null());
        let (_, original) = call(
            &f.delta.app,
            "GET",
            &format!("{ROUTE}/snapshots/{REVISION}"),
            None,
        )
        .await;
        assert_eq!(
            original["workflow_owner"]["task_id"],
            f.mapped["entries"][0]["remote_task_id"]
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn complete_batch_bulk_commit_imports_mixed_content_and_snapshot_roots() {
    let root = TestDirectory::new("complete-bulk-content");
    let f = complete_fixture(root.path(), None).await;
    let package = WorkflowPackage::unpack(
        &f.delta.raw,
        &root.path().join("bulk-inputs"),
        1,
        &f.delta.hash,
        usize::MAX,
    )
    .unwrap();
    for (directory, rows, tree) in [
        ("objects", &package.manifest.content.object_packs, false),
        ("trees", &package.manifest.content.tree_packs, true),
    ] {
        for row in rows {
            let id = row["pack_id"].as_str().unwrap();
            let bytes = fs::read(package.inputs.join(directory).join(id)).unwrap();
            if tree {
                f.delta
                    .state
                    .repository_service
                    .put_zstd_bulk_tree_pack("1", id, bytes)
                    .unwrap();
            } else {
                f.delta
                    .state
                    .repository_service
                    .put_zstd_bulk_object_pack("1", id, bytes)
                    .unwrap();
            }
        }
    }
    let mut body = f.delta.manifest["content"].clone();
    body.as_object_mut().unwrap().remove("capture_binding");
    body.as_object_mut().unwrap().remove("line_update");
    body["contract"] = json!("ait.remote_sync.zstd_bulk.commit.v1");
    body["actor_key_index"] = json!(0);
    let before = counts(&f.delta.state);
    let (status, result) = call(
        &f.delta.app,
        "POST",
        &format!("{ROUTE}/remote-sync/zstd-bulk/commit"),
        Some(body),
    )
    .await;
    assert!(status.is_success(), "{result}");
    let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    assert!(content.tree(&tree_id(2)).unwrap().is_some());
    assert_eq!(
        content
            .blob_bytes_with_read(&BinaryDbReadTxn::new(&db), &f.delta.delta)
            .unwrap(),
        Some(f.delta.expected_delta)
    );
    let after = counts(&f.delta.state);
    assert_eq!(&after[..7], &before[..7]);
    assert_eq!(after[7], before[7] + 4);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn complete_batch_restart_cannot_reuse_terminal_ci_or_governance() {
    let root = TestDirectory::new("package-passed-restart");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let ready = prepare_package(&app, &raw, &hash).await;
    let identity = &ready["expected_identity"];
    let job = identity["worker_job_index"].as_u64().unwrap();
    complete(&app, job, "pass").await;
    attest(&app, identity, "ai_with_human_review").await;
    evidence(
        &app,
        identity,
        "human",
        "review",
        json!({"reviewer":"Alice","reviewer_kind":"user","action":"approve"}),
    )
    .await;
    drop(app);
    drop(state);
    let reopened = Arc::new(
        OperationalBinaryRuntime::open_generation(
            root.path().join("generation"),
            root.path().join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .unwrap(),
    );
    let state = state_from_operational(reopened);
    let app = build_router_with_state(state.clone());
    let (status, result) = finish(&app, identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    assert!(
        result
            .to_string()
            .contains("selected successful CI attempt"),
        "{result}"
    );
    let (status, result) = upload(&app, &raw, &hash, Some(job as u32)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    let (status, fresh) = upload(&app, &raw, &hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{fresh}");
    let fresh_identity = &fresh["expected_identity"];
    let fresh_job = fresh_identity["worker_job_index"].as_u64().unwrap();
    assert_ne!(fresh_job, job);
    let (status, result) = finish(&app, fresh_identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    complete(&app, fresh_job, "pass").await;
    // A new successful attempt does not inherit the old human approval or
    // Attestation. It must establish both for the new exact identity.
    let (status, result) = finish(&app, fresh_identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    no_publication(&state);
    attest(&app, fresh_identity, "ai_with_human_review").await;
    evidence(
        &app,
        fresh_identity,
        "human-new-job",
        "review",
        json!({"reviewer":"Alice","reviewer_kind":"user","action":"approve"}),
    )
    .await;
    review(&app, fresh_identity, "summary", "code_review_summary").await;
    review(&app, fresh_identity, "task-approved", "task_approve").await;
    let (status, result) = finish(&app, fresh_identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let (_, old) = call(&app, "GET", &format!("{ROUTE}/worker-jobs/{job}"), None).await;
    assert!(old["job"]["patchset_index"].is_null());
}

fn retained_members(
    package: &ait_server_core::foundation::workflow_package::WorkflowPackage,
) -> BTreeMap<String, Vec<u8>> {
    let mut members = BTreeMap::new();
    for (rows, directory) in [
        (&package.manifest.content.object_packs, "objects"),
        (&package.manifest.content.tree_packs, "trees"),
    ] {
        for row in rows {
            let id = row["pack_id"].as_str().unwrap();
            members.insert(
                format!("{directory}/{id}"),
                fs::read(package.inputs.join(directory).join(id)).unwrap(),
            );
        }
    }
    members
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_projection_materializes_exact_forks_archives_last_new_and_preserves_authoring() {
    use ait_server_core::foundation::workflow_package::{projected_package_line, WorkflowPackage};
    let root = TestDirectory::new("package-line-projection");
    let mut f = complete_fixture(root.path(), None).await;
    let package = WorkflowPackage::unpack(
        &f.delta.raw,
        &root.path().join("variant-inputs"),
        1,
        &f.delta.hash,
        usize::MAX,
    )
    .unwrap();
    let members = retained_members(&package);
    let source_name = "feature/local/中文";
    let entry = &mut f.delta.manifest["history_entries"][1];
    entry["changes"] = json!([
        {"local_change_id":"C-03","local_change_ref":"LSET-0002/C-03","expected_remote_change_ref":null,"change":{"title":"First historical Change","base_line":source_name,"fork_snapshot_id":REVISION}},
        {"local_change_id":"C-08","local_change_ref":"LSET-0002/C-08","expected_remote_change_ref":null,"change":{"title":"Later historical Change","base_line":source_name,"fork_snapshot_id":ORIGINAL_TASK}}
    ]);
    for (i, row) in entry["snapshots"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .enumerate()
    {
        row["local_change_ref"] = json!(if i % 2 == 0 {
            "LSET-0002/C-03"
        } else {
            "LSET-0002/C-08"
        });
    }
    // A raw authoring name equal to the future projection must retain fallback
    // identity, even if publication will create that canonical Line later.
    let prefix = f.mapped["entries"][0]["remote_task_id"]
        .as_str()
        .unwrap()
        .rsplit_once('-')
        .unwrap()
        .0;
    let projected = projected_package_line(&format!("{prefix}-0002"), source_name).unwrap();
    for row in f.delta.manifest["content"]["snapshots"]
        .as_array_mut()
        .unwrap()
    {
        row["line_name"] = json!(projected);
    }
    let (raw, hash) = archive(&f.delta.manifest, &members);
    let before = counts(&f.delta.state);
    let (status, ready) = upload(&f.delta.app, &raw, &hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    assert_eq!(counts(&f.delta.state), before);
    let identity = &ready["expected_identity"];
    complete(
        &f.delta.app,
        identity["worker_job_index"].as_u64().unwrap(),
        "pass",
    )
    .await;
    attest(&f.delta.app, identity, "ai_only_experimental").await;
    review(&f.delta.app, identity, "summary", "code_review_summary").await;
    review(&f.delta.app, identity, "task", "task_approve").await;
    let (status, result) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let task_id = result["entries"][1]["remote_task_id"].as_str().unwrap();
    let projected = projected_package_line(task_id, source_name).unwrap();
    let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    let lines = ServerBinaryDbLineStore::<_, 1>::new(db.clone());
    let (line_index, line) = lines.line_by_name(&read, &projected).unwrap().unwrap();
    let snapshots = ait_server_core::foundation::server_content_binary_db::ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
    let (fork, _) = snapshots
        .snapshot_by_id(&read, ORIGINAL_TASK)
        .unwrap()
        .unwrap();
    assert_eq!(line.head_snapshot_index(), Some(fork));
    assert!(line.is_archived());
    assert!(line.created_at_s > 700 && line.archived_at_s >= line.created_at_s);
    for (i, expected_fork) in [REVISION, ORIGINAL_TASK].iter().enumerate() {
        let record = WorkflowBinaryV0Codec::decode_change(
            &read
                .read_record(WorkflowBinaryV0Codec::change_file(), i as u32)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(record.base_line_index_plus1, line_index + 1);
        assert_eq!(
            record.fork_snapshot_index_plus1,
            snapshots
                .snapshot_by_id(&read, expected_fork)
                .unwrap()
                .unwrap()
                .0
                + 1
        );
    }
    for id in [ORIGINAL_TASK, ORIGINAL_CHANGE, TESTED_TASK, TESTED_CHANGE] {
        let (_, record) = snapshots.snapshot_by_id(&read, id).unwrap().unwrap();
        assert_eq!(record.line_index_plus1, 0);
    }
    let native_line = f
        .delta
        .state
        .repository_service
        .get_line("1", &projected)
        .unwrap();
    let (status, detail) = call(
        &f.delta.app,
        "GET",
        &format!("{ROUTE}/lines/{projected}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail, native_line);
    assert_eq!(detail["line_name"], projected);
    assert_eq!(detail["head_snapshot_id"], ORIGINAL_TASK);
    assert_eq!(detail["status"], "archived");
    for (method, suffix, body) in [
        ("PUT", "", json!({"head_snapshot_id":ORIGINAL_CHANGE})),
        ("POST", ":close", json!({"status":"archived"})),
    ] {
        let (status, body) = call(
            &f.delta.app,
            method,
            &format!("{ROUTE}/lines/{projected}{suffix}"),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.to_string().contains("Reserved Remote"), "{body}");
    }
    assert_eq!(
        lines.line_by_name(&read, &projected).unwrap().unwrap().1,
        line
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_preflight_rejects_independent_reference_classes_without_jobs_or_lines() {
    use ait_server_core::foundation::workflow_package::{projected_package_line, WorkflowPackage};
    let root = TestDirectory::new("package-preflight-reference-classes");
    let f = complete_fixture(root.path(), None).await;
    let package = WorkflowPackage::unpack(
        &f.delta.raw,
        &root.path().join("preflight-inputs"),
        1,
        &f.delta.hash,
        usize::MAX,
    )
    .unwrap();
    let members = retained_members(&package);
    let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    let job_file = ait_server_core::foundation::operational_binary_v0::ServerOperationalBinaryV0Codec::worker_job_file();
    let before_jobs = read.record_count(job_file.clone()).unwrap();
    let before_lines = read.record_count(ait_server_core::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::record_file()).unwrap();
    let before = counts(&f.delta.state);
    for (kind, expected) in [
        ("task-limit", "64 source Tasks"),
        ("no-anchor-task", "no Snapshot anchor"),
        ("no-anchor-change", "no owned Snapshot anchor"),
        ("fork", "Required Snapshot"),
        ("plan", "Plan"),
        ("line-length", "Projected Line exceeds"),
        ("no-op", "Final aggregate Snapshot"),
        ("terminal", "terminal"),
        (
            "snapshot-message",
            "differs from immutable canonical content",
        ),
    ] {
        let mut manifest = f.delta.manifest.clone();
        match kind {
            "task-limit" => {
                let entry = manifest["history_entries"][1].clone();
                manifest["history_entries"] = json!(vec![entry; 65]);
            }
            "no-anchor-task" => {
                manifest["history_entries"][1]["snapshots"] = json!([]);
                manifest["history_entries"][1]["changes"] = json!([]);
            }
            "no-anchor-change" => {
                manifest["history_entries"][1]["changes"][0]["local_change_id"] = json!("C-03");
                manifest["history_entries"][1]["changes"][0]["local_change_ref"] =
                    json!("LSET-0002/C-03");
                for s in manifest["history_entries"][1]["snapshots"]
                    .as_array_mut()
                    .unwrap()
                {
                    s["local_change_ref"] = JsonValue::Null;
                }
            }
            "fork" => {
                manifest["history_entries"][1]["changes"][0]["change"]["fork_snapshot_id"] =
                    json!("SNP-00000000FFFF")
            }
            "plan" => {
                manifest["history_entries"][1]["task"]["plan_id"] = json!("PR-99");
                manifest["history_entries"][1]["task"]["origin_plan_revision_id"] =
                    json!("plan-revision:99");
                manifest["history_entries"][1]["task"]["plan_item_ref"] = json!("unknown/item");
            }
            "line-length" => {
                manifest["history_entries"][1]["changes"][0]["change"]["base_line"] =
                    json!("界".repeat(11000))
            }
            "no-op" => {
                manifest["history_entries"][1]["snapshots"] = json!([]);
                manifest["history_entries"][1]["changes"] = json!([]);
                manifest["history_entries"][1]["expected_remote_task_id"] =
                    f.mapped["entries"][0]["remote_task_id"].clone();
            }
            "terminal" => {
                manifest["history_entries"] = json!([manifest["history_entries"][0].clone()]);
                manifest["revision_snapshot_id"] = json!(REVISION);
            }
            "snapshot-message" => {
                let mut dependency = snapshot(BASE, None, 100);
                dependency["message"] = json!("Changed immutable dependency message");
                manifest["content"]["snapshots"]
                    .as_array_mut()
                    .unwrap()
                    .insert(0, dependency);
            }
            _ => unreachable!(),
        }
        let (raw, hash) = archive(&manifest, &members);
        let (status, result) = upload(&f.delta.app, &raw, &hash, None).await;
        assert!(!status.is_success(), "{kind}: {result}");
        // A no-op with a conflicting prior mapping is held at that earlier
        // exact receipt boundary; it cannot silently become the old owner.
        if kind == "no-op" {
            assert!(
                result
                    .to_string()
                    .contains("Published original revision is missing"),
                "{result}"
            );
        } else {
            assert!(result.to_string().contains(expected), "{kind}: {result}");
        }
        assert_eq!(counts(&f.delta.state), before, "{kind}");
        assert_eq!(
            read.record_count(job_file.clone()).unwrap(),
            before_jobs,
            "{kind}"
        );
        assert_eq!(read.record_count(ait_server_core::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::record_file()).unwrap(), before_lines, "{kind}");
    }
    // An exact-looking orphan cannot be taken over, including archived names.
    let prefix = f.mapped["entries"][0]["remote_task_id"]
        .as_str()
        .unwrap()
        .rsplit_once('-')
        .unwrap()
        .0;
    let name = projected_package_line(&format!("{prefix}-0002"), "main").unwrap();
    drop(read);
    let lines = ServerBinaryDbLineStore::<_, 1>::new(db.clone());
    lines.create_line(&name, 2, 300).unwrap();
    lines.archive_line(&name, 301).unwrap();
    let (status, result) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
    assert!(!status.is_success(), "{result}");
    assert!(result.to_string().contains("name collision"), "{result}");
    assert_eq!(
        BinaryDbReadTxn::new(&db).record_count(job_file).unwrap(),
        before_jobs
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_preflight_rejects_task_only_claim_on_stored_change_owner_before_ci() {
    use ait_server_core::foundation::workflow_package::WorkflowPackage;
    let root = TestDirectory::new("package-preflight-stored-change-owner");
    let mut f = complete_fixture(root.path(), None).await;
    let (status, ready) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    let identity = &ready["expected_identity"];
    complete(
        &f.delta.app,
        identity["worker_job_index"].as_u64().unwrap(),
        "pass",
    )
    .await;
    attest(&f.delta.app, identity, "ai_only_experimental").await;
    review(&f.delta.app, identity, "summary", "code_review_summary").await;
    review(&f.delta.app, identity, "task", "task_approve").await;
    let (status, published) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::OK, "{published}");
    let package = WorkflowPackage::unpack(
        &f.delta.raw,
        &root.path().join("stored-owner-inputs"),
        1,
        &f.delta.hash,
        usize::MAX,
    )
    .unwrap();
    let members = retained_members(&package);
    for (entry, mapping) in f.delta.manifest["history_entries"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip(published["entries"].as_array().unwrap())
    {
        entry["expected_remote_task_id"] = mapping["remote_task_id"].clone();
        for (change, mapped) in entry["changes"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(mapping["changes"].as_array().unwrap())
        {
            change["expected_remote_change_ref"] = mapped["remote_change_ref"].clone();
        }
    }
    let original = "SNP-000000000008";
    let tested = "SNP-000000000009";
    f.delta.manifest["base_snapshot_id"] = json!(TESTED_CHANGE);
    f.delta.manifest["revision_snapshot_id"] = json!(tested);
    f.delta.manifest["history_entries"].as_array_mut().unwrap().push(json!({
        "local_task_id":"LSET-0003","expected_remote_task_id":null,
        "task":{"title":"New final source","intent":"Extend an exact terminal prefix"},
        "pre_land_target_snapshot_id":ORIGINAL_CHANGE,"landed_snapshot_id":original,"landed_at_s":950,
        "changes":[],"snapshots":[
            {"snapshot_id":original,"created_at_s":800,"local_change_ref":null},
            {"snapshot_id":tested,"created_at_s":1000,"local_change_ref":null}
        ]
    }));
    f.delta.manifest["content"]["snapshots"]
        .as_array_mut()
        .unwrap()
        .extend([
            snapshot(original, Some(ORIGINAL_CHANGE), 800),
            snapshot(tested, Some(TESTED_CHANGE), 1000),
        ]);
    // The other Change-owned anchor remains, so Task/Change inference alone
    // cannot detect this false Task-only declaration on the terminal prefix.
    f.delta.manifest["history_entries"][1]["snapshots"][1]["local_change_ref"] = JsonValue::Null;
    let before = counts(&f.delta.state);
    let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    let job_file = ait_server_core::foundation::operational_binary_v0::ServerOperationalBinaryV0Codec::worker_job_file();
    let line_file = ait_server_core::foundation::server_content_binary_db::ServerBinaryLineCodec::<1>::record_file();
    let before_jobs = read.record_count(job_file.clone()).unwrap();
    let before_lines = read.record_count(line_file.clone()).unwrap();
    let (raw, hash) = archive(&f.delta.manifest, &members);
    let (status, result) = upload(&f.delta.app, &raw, &hash, None).await;
    assert!(!status.is_success(), "{result}");
    assert!(
        result
            .to_string()
            .contains("exact Task/Change owner differs"),
        "{result}"
    );
    assert_eq!(counts(&f.delta.state), before);
    assert_eq!(read.record_count(job_file).unwrap(), before_jobs);
    assert_eq!(read.record_count(line_file).unwrap(), before_lines);
}
