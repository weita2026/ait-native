use super::*;
use ait_server_core::foundation::pack_substrate::{build_pack_members, write_pack_archive};
use ait_server_core::foundation::remote_binary_db::{BinaryDbCommandScope, BinaryDbWriteTxn};
use ait_server_core::foundation::server_content_binary_db::ServerBinaryRepositoryContentStore;
use ait_server_core::foundation::workflow_package::WorkflowPackage;
use std::collections::BTreeMap;

#[path = "workflow_package_complete.rs"]
mod complete_batch;

struct DeltaFixture {
    state: ServerState,
    app: Router,
    raw: Vec<u8>,
    hash: String,
    manifest: JsonValue,
    base: String,
    delta: String,
    expected_delta: Vec<u8>,
}

async fn delta_fixture(root: &FsPath, include_base: bool) -> DeltaFixture {
    let (state, app, raw, hash) = fixture(root).await;
    let package =
        WorkflowPackage::unpack(&raw, &root.join("initial-inputs"), 1, &hash, usize::MAX).unwrap();
    let mut manifest = serde_json::to_value(&package.manifest).unwrap();
    let base_data = vec![b'a'; 512];
    let mut delta_data = base_data.clone();
    delta_data[100] = b'b';
    let base = format!("BLB-{}", &format!("{:x}", Sha256::digest(&base_data))[..20]);
    let delta = format!(
        "BLB-{}",
        &format!("{:x}", Sha256::digest(&delta_data))[..20]
    );
    let mut packs = Vec::new();
    let mut locators = Vec::new();
    let mut bytes_by_pack = BTreeMap::new();
    for (number, id, data, is_delta) in [
        (1, &delta, &delta_data, true),
        (2, &base, &base_data, false),
    ] {
        if !include_base && !is_delta {
            continue;
        }
        let pack_id = format!("PCK-{number:012X}");
        let candidates = json!([{"entry_name":format!("blobs/{id}"),"blob_id":id,"data":data,"path_hint":"file"}]);
        let initial = json!({"file":{"blob_id":base,"data":base_data,"chain_depth":0}});
        let members = build_pack_members(&candidates, 4, is_delta.then_some(&initial)).unwrap();
        assert_eq!(
            members[0]["entry_type"],
            if is_delta { "delta" } else { "full" }
        );
        let path = root.join(&pack_id);
        let mut metadata = write_pack_archive(
            path.to_str().unwrap(),
            &pack_id,
            "1970-01-01T00:03:20Z",
            &members,
        )
        .unwrap();
        metadata["pack_id"] = json!(pack_id);
        locators.push(json!({"blob_id":id,"sha256":format!("{:x}",Sha256::digest(data)),"size_bytes":data.len(),"pack_id":pack_id,
            "pack_entry_name":format!("blobs/{id}"),"pack_entry_type":members[0]["entry_type"],"pack_base_blob_id":members[0]["base_blob_id"],
            "pack_chain_depth":members[0]["chain_depth"],"created_at":"1970-01-01T00:03:20Z"}));
        bytes_by_pack.insert(pack_id, fs::read(path).unwrap());
        packs.push(metadata);
    }
    locators.sort_by(|left, right| left["blob_id"].as_str().cmp(&right["blob_id"].as_str()));
    manifest["content"]["object_packs"] = json!(packs);
    manifest["content"]["blob_locators"] = json!(locators);
    let mut tar = Vec::new();
    tar_member(
        &mut tar,
        "manifest.json",
        &serde_json::to_vec(&manifest).unwrap(),
    );
    for (id, bytes) in bytes_by_pack {
        tar_member(&mut tar, &format!("objects/{id}"), &bytes);
    }
    tar_member(
        &mut tar,
        &format!("trees/{PACK}"),
        &fs::read(package.inputs.join("trees").join(PACK)).unwrap(),
    );
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
    DeltaFixture {
        state,
        app,
        raw,
        hash,
        manifest,
        base,
        delta,
        expected_delta: delta_data,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_batch_imports_a_later_history_base_and_missing_base_rolls_back() {
    for include_base in [true, false] {
        let root = TestDirectory::new("package-cross-pack-delta");
        let f = delta_fixture(root.path(), include_base).await;
        let (_, db) = f.state.operational_binary.repository_db(1).unwrap();
        let content = ServerBinaryRepositoryContentStore::new(db.clone());
        // Reproduce the original per-pack lookup failure on the same input.
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerRemoteSyncCommit)
                .unwrap();
        let first = &f.manifest["content"]["object_packs"][0];
        let locators = f.manifest["content"]["blob_locators"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|locator| locator["pack_id"] == first["pack_id"])
            .cloned()
            .collect::<Vec<_>>();
        let error = content
            .append_object_pack_in_tx(&mut tx, first, &locators)
            .unwrap_err();
        assert!(error.to_string().contains("delta base blob"), "{error}");
        tx.abort().unwrap();
        no_publication(&f.state);
        let (status, prepared) = upload(&f.app, &f.raw, &f.hash, None).await;
        if !include_base {
            assert_eq!(status, StatusCode::BAD_REQUEST, "{prepared}");
            assert!(
                prepared.to_string().contains("delta base blob"),
                "{prepared}"
            );
            no_publication(&f.state);
            continue;
        }
        assert_eq!(status, StatusCode::ACCEPTED, "{prepared}");
        let identity = &prepared["expected_identity"];
        assert_eq!(identity["package_hash"], f.hash);
        let job = identity["worker_job_index"].as_u64().unwrap();
        complete(&f.app, job, "pass").await;
        let path = format!("{ROUTE}/worker-jobs/{job}");
        for (operation, key, payload) in [
            (
                "review",
                "approve",
                json!({"reviewer":"Alice","reviewer_kind":"user","action":"approve"}),
            ),
            (
                "review",
                "summary",
                json!({"reviewer":"Alice","reviewer_kind":"user","action":"code_review_summary"}),
            ),
            (
                "review",
                "task",
                json!({"reviewer":"Alice","reviewer_kind":"user","action":"task_approve"}),
            ),
            (
                "attestation",
                "attest",
                json!({"author_mode":"ai_with_human_review","verification_state":"pass","require_tests_pass":true,"require_human_review":true,"require_lint_pass":true}),
            ),
        ] {
            let evidence = json!({"contract":"workflow-package-evidence/v1","operation":operation,"idempotency_key":key,"actor_key_index":0,"expected_identity":identity,"payload":payload});
            let (status, result) = call(&f.app, "POST", &path, Some(evidence)).await;
            assert_eq!(status, StatusCode::OK, "{result}");
        }
        let finish = json!({"contract":"task-land-package-guarded/v1","idempotency_key":format!("workflow-package:{}:{job}",f.hash),"actor_key_index":0,"target_line":"main","mode":"direct","expected_identity":identity});
        let (status, result) =
            call(&f.app, "POST", &format!("{ROUTE}/task-land"), Some(finish)).await;
        let (_, current) = call(&f.app, "GET", &path, None).await;
        if include_base {
            assert_eq!(status, StatusCode::OK, "{result}");
            assert_eq!(result["outcome"], "confirmed");
            assert_eq!(result["expected_identity"]["package_hash"], f.hash);
            assert!(current["job"]["patchset_index"].is_number());
            let delta = content.blob(&f.delta).unwrap().unwrap();
            let base = content.blob(&f.base).unwrap().unwrap();
            assert_eq!(delta.base_blob_id, Some(f.base.clone()));
            assert!(delta.member.pack_index < base.member.pack_index);
            assert_eq!(
                content
                    .blob_bytes_with_read(&BinaryDbReadTxn::new(&db), &f.delta)
                    .unwrap(),
                Some(f.expected_delta)
            );
        } else {
            assert_eq!(status, StatusCode::CONFLICT, "{result}");
            assert!(result.to_string().contains("delta base blob"), "{result}");
            no_publication(&f.state);
            assert!(current["job"]["patchset_index"].is_null());
            assert_eq!(current["package_execution"]["ci_state"], "passed");
            assert!(content.blob(&f.delta).unwrap().is_none());
            assert!(content.object_pack("PCK-000000000001").unwrap().is_none());
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn bulk_commit_imports_the_complete_cross_pack_batch() {
    let root = TestDirectory::new("bulk-cross-pack-delta");
    let f = delta_fixture(root.path(), true).await;
    for pack in f.manifest["content"]["object_packs"].as_array().unwrap() {
        let id = pack["pack_id"].as_str().unwrap();
        f.state
            .repository_service
            .put_zstd_bulk_object_pack("1", id, fs::read(root.path().join(id)).unwrap())
            .unwrap();
    }
    let request = json!({"contract":"ait.remote_sync.zstd_bulk.commit.v1","actor_key_index":0,"object_packs":f.manifest["content"]["object_packs"],"blob_locators":f.manifest["content"]["blob_locators"],"tree_packs":[],"tree_locators":[],"snapshots":[]});
    let (status, result) = call(
        &f.app,
        "POST",
        &format!("{ROUTE}/remote-sync/zstd-bulk/commit"),
        Some(request),
    )
    .await;
    assert!(status.is_success(), "{result}");
    let (_, db) = f.state.operational_binary.repository_db(1).unwrap();
    let content = ServerBinaryRepositoryContentStore::new(db.clone());
    assert_eq!(
        content.blob(&f.delta).unwrap().unwrap().base_blob_id,
        Some(f.base)
    );
    assert_eq!(
        content
            .blob_bytes_with_read(&BinaryDbReadTxn::new(&db), &f.delta)
            .unwrap(),
        Some(f.expected_delta)
    );
}
