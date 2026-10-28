use super::*;
use ait_server_core::foundation::workflow_package::WorkflowPackage;

async fn zero_fixture(root: &FsPath) -> (ServerState, Router, JsonValue) {
    let (state, app, raw, hash) = fixture(root).await;
    let package =
        WorkflowPackage::unpack(&raw, &root.join("zero-inputs"), 1, &hash, usize::MAX).unwrap();
    let mut manifest = serde_json::to_value(&package.manifest).unwrap();
    manifest["author_mode"] = json!("ai_only_experimental");
    let (raw, hash) = archive(
        &manifest,
        &BTreeMap::from([(
            format!("trees/{PACK}"),
            fs::read(package.inputs.join("trees").join(PACK)).unwrap(),
        )]),
    );
    let ready = prepare_package(&app, &raw, &hash).await;
    let identity = ready["expected_identity"].clone();
    complete(&app, identity["worker_job_index"].as_u64().unwrap(), "pass").await;
    attest(&app, &identity, "ai_only_experimental").await;
    for (key, action) in [
        ("zero-approve", "approve"),
        ("zero-summary", "code_review_summary"),
        ("zero-task", "task_approve"),
    ] {
        review(&app, &identity, key, action).await;
    }
    (state, app, identity)
}

fn reopen_zero(root: &FsPath) -> (ServerState, Router) {
    let runtime = Arc::new(
        OperationalBinaryRuntime::open_generation(
            root.join("generation"),
            root.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .unwrap(),
    );
    let state = state_from_operational(runtime);
    let app = build_router_with_state(state.clone());
    (state, app)
}

fn authority_bytes(state: &ServerState) -> BTreeMap<String, Vec<u8>> {
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    fs::read_dir(db.authority_root().as_path())
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|value| value.to_str()),
                Some("bin" | "idx")
            )
        })
        .map(|path| {
            (
                path.file_name().unwrap().to_str().unwrap().into(),
                fs::read(path).unwrap(),
            )
        })
        .collect()
}

fn parent(root: &FsPath, identity: &JsonValue) -> std::path::PathBuf {
    root.join("workflow-packages/generation/1")
        .join(identity["package_hash"].as_str().unwrap())
}

async fn later_unpublished_package(root: &FsPath, app: &Router) -> JsonValue {
    const LATER: &str = "SNP-ABCDEF000003";
    let inputs = root.join("zero-inputs");
    let mut manifest: JsonValue =
        serde_json::from_slice(&fs::read(inputs.join("manifest.json")).unwrap()).unwrap();
    manifest["author_mode"] = json!("ai_only_experimental");
    manifest["base_snapshot_id"] = json!(REVISION);
    manifest["revision_snapshot_id"] = json!(LATER);
    manifest["history_entries"] = json!([{
        "local_task_id":"LSET-0002","expected_remote_task_id":null,
        "task":{"title":"Later source","intent":"Keep later unpublished CI separate"},
        "pre_land_target_snapshot_id":REVISION,"landed_snapshot_id":LATER,
        "landed_at_s":350,"changes":[],
        "snapshots":[{"snapshot_id":LATER,"created_at_s":300,"local_change_ref":null}]
    }]);
    manifest["content"]["snapshots"] = json!([snapshot(LATER, Some(REVISION), 300)]);
    let (raw, hash) = archive(
        &manifest,
        &BTreeMap::from([(
            format!("trees/{PACK}"),
            fs::read(inputs.join("trees").join(PACK)).unwrap(),
        )]),
    );
    let ready = prepare_package(app, &raw, &hash).await;
    let identity = ready["expected_identity"].clone();
    complete(app, identity["worker_job_index"].as_u64().unwrap(), "pass").await;
    identity
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_zero_change_replay_preserves_a_later_unpublished_job_and_its_source() {
    let root = TestDirectory::new("accepted-zero-later-job");
    let (state, app, identity) = zero_fixture(root.path()).await;
    let (status, original) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    let later = later_unpublished_package(root.path(), &app).await;
    assert_ne!(later["worker_job_index"], identity["worker_job_index"]);
    let later_source = parent(root.path(), &later)
        .join(later["worker_job_index"].as_u64().unwrap().to_string())
        .join("package.zst");
    let later_bytes = fs::read(&later_source).unwrap();
    let before = authority_bytes(&state);
    drop(app);
    drop(state);
    let (state, app) = reopen_zero(root.path());
    assert_eq!(authority_bytes(&state), before);
    let (status, replay) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay, original);
    assert_eq!(authority_bytes(&state), before);
    assert_eq!(fs::read(later_source).unwrap(), later_bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_zero_change_restart_recovers_full_mapping_without_any_formal_write() {
    let root = TestDirectory::new("accepted-zero-restart");
    let (state, app, identity) = zero_fixture(root.path()).await;
    let (status, original) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    assert_eq!(original["outcome"], "confirmed");
    assert_eq!(original["entries"][0]["changes"], json!([]));
    assert!(!parent(root.path(), &identity).exists());
    let before = authority_bytes(&state);
    drop(app);
    drop(state);
    let (state, app) = reopen_zero(root.path());
    assert_eq!(authority_bytes(&state), before);
    for _ in 0..2 {
        let (status, replay) = finish(&app, &identity).await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_eq!(replay, original);
        assert_eq!(authority_bytes(&state), before);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_zero_change_mapping_does_not_bypass_unknown_cleanup_or_touch_another_job() {
    let root = TestDirectory::new("accepted-zero-cleanup");
    let (state, app, identity) = zero_fixture(root.path()).await;
    let (status, original) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    drop(app);
    drop(state);
    let (state, app) = reopen_zero(root.path());
    let before = authority_bytes(&state);
    let own_parent = parent(root.path(), &identity);
    let own = own_parent.join(identity["worker_job_index"].as_u64().unwrap().to_string());
    fs::create_dir_all(&own).unwrap();
    fs::write(own.join("unowned"), b"preserve unknown payload").unwrap();
    let other = root
        .path()
        .join("workflow-packages/generation/1")
        .join("f".repeat(64))
        .join("2");
    fs::create_dir_all(&other).unwrap();
    fs::write(other.join("keep"), b"another Job").unwrap();
    let (status, pending) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{pending}");
    assert!(pending
        .to_string()
        .contains("PACKAGE_FINISH_CLEANUP_UNKNOWN"));
    assert_eq!(
        fs::read(own.join("unowned")).unwrap(),
        b"preserve unknown payload"
    );
    assert_eq!(authority_bytes(&state), before);
    // This disposable fixture owns the injected payload. Only the native
    // bounded empty-container path is exercised after removing that injection.
    fs::remove_file(own.join("unowned")).unwrap();
    let (status, replay) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay, original);
    assert!(!own_parent.exists());
    assert_eq!(fs::read(other.join("keep")).unwrap(), b"another Job");
    assert_eq!(authority_bytes(&state), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn accepted_zero_change_rejects_conflicting_original_pins_without_new_work() {
    let root = TestDirectory::new("accepted-zero-pin");
    let (state, app, identity) = zero_fixture(root.path()).await;
    assert_eq!(finish(&app, &identity).await.0, StatusCode::OK);
    let before = authority_bytes(&state);
    for field in [
        "repository",
        "job",
        "hash",
        "base",
        "revision",
        "target",
        "mode",
        "actor",
    ] {
        let mut request = finish_request(&identity);
        match field {
            "repository" => request["expected_identity"]["repository_index"] = json!(2),
            "job" => request["expected_identity"]["worker_job_index"] = json!(999),
            "hash" => request["expected_identity"]["package_hash"] = json!("f".repeat(64)),
            "base" => request["expected_identity"]["base_snapshot_id"] = json!(REVISION),
            "revision" => request["expected_identity"]["revision_snapshot_id"] = json!(BASE),
            "target" => request["target_line"] = json!("other"),
            "mode" => request["mode"] = json!("merge"),
            "actor" => request["actor_key_index"] = json!(u32::MAX),
            _ => unreachable!(),
        }
        request["idempotency_key"] = json!(format!(
            "workflow-package:{}:{}",
            request["expected_identity"]["package_hash"]
                .as_str()
                .unwrap(),
            request["expected_identity"]["worker_job_index"]
                .as_u64()
                .unwrap()
        ));
        let (status, result) =
            call(&app, "POST", &format!("{ROUTE}/task-land"), Some(request)).await;
        assert!(!status.is_success(), "{field}: {result}");
        assert_eq!(authority_bytes(&state), before, "{field}");
    }
    assert_eq!(finish(&app, &identity).await.0, StatusCode::OK);
    assert_eq!(authority_bytes(&state), before);
}
