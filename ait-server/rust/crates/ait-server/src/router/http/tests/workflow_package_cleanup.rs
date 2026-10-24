use super::*;
use ait_server_core::foundation::server_operational_runtime_leases::{
    RuntimeLeaseToken, RuntimePackageDownloadError,
};
use ait_server_core::foundation::workflow_package::WorkflowPackage;
use ait_server_core::foundation::workflow_package_contract::parse_package_hash;

fn hash_parent(root: &FsPath, hash: &str) -> std::path::PathBuf {
    root.join("workflow-packages/generation/1").join(hash)
}

async fn reviewed_fixture(root: &FsPath) -> (CompleteFixture, JsonValue) {
    let f = complete_fixture(root, None).await;
    let (status, ready) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    let identity = ready["expected_identity"].clone();
    complete(
        &f.delta.app,
        identity["worker_job_index"].as_u64().unwrap(),
        "pass",
    )
    .await;
    attest(&f.delta.app, &identity, "ai_only_experimental").await;
    review(&f.delta.app, &identity, "summary", "code_review_summary").await;
    review(&f.delta.app, &identity, "approved", "task_approve").await;
    (f, identity)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_download_pin_blocks_full_finish_until_the_last_reader_releases() {
    let root = TestDirectory::new("package-strict-held-download");
    let f = complete_fixture(root.path(), None).await;
    let (_, ready) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
    let identity = &ready["expected_identity"];
    let job = identity["worker_job_index"].as_u64().unwrap();
    let path = format!("{ROUTE}/worker-jobs/{job}");
    let (status, claim) = call(
        &f.delta.app,
        "POST",
        &format!("{path}:claim"),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let proof = &claim["claimed_job"];
    let attempt = proof["attempt_count"].as_u64().unwrap() as u16;
    let token = RuntimeLeaseToken::parse_hex(proof["lease_token"].as_str().unwrap()).unwrap();
    let hash = parse_package_hash(&f.delta.hash).unwrap();
    let (store, leases) = f
        .delta
        .state
        .operational_binary
        .repository_worker_runtime(1)
        .unwrap();
    let (pinned_tx, pinned_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let download = std::thread::spawn(move || {
        let calls = std::cell::Cell::new(0);
        leases.download_prepared_ci_package(&store, job as u32, attempt, token, hash, || {
            if calls.replace(calls.get() + 1) == 1 {
                pinned_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(30))
                    .map_err(|error| {
                        ait_server_core::foundation::remote_binary_db::BinaryDbError::other(
                            error.to_string(),
                        )
                    })?;
            }
            Ok(chrono::Utc::now().timestamp() as u64)
        })
    });
    pinned_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();
    let (status, uncommitted) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{uncommitted}");
    assert!(!uncommitted.to_string().contains("PACKAGE_FINISH_CLEANUP_"));
    complete_claimed(&f.delta.app, job, "pass", proof).await;
    attest(&f.delta.app, identity, "ai_only_experimental").await;
    review(&f.delta.app, identity, "summary", "code_review_summary").await;
    review(&f.delta.app, identity, "approved", "task_approve").await;
    let (status, pending) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{pending}");
    assert!(pending
        .to_string()
        .contains("PACKAGE_FINISH_CLEANUP_PENDING"));
    let committed = counts(&f.delta.state);
    let parent = hash_parent(root.path(), &f.delta.hash);
    assert!(parent.join(job.to_string()).is_dir());
    let accepted = immutable_result(&f.delta.state, identity);
    assert_eq!(accepted["outcome"], "incomplete");
    assert!(accepted["patchset_id"].is_string());
    assert!(accepted["submission_id"].is_string());
    let jobs = job_records(&f.delta.state);
    let (status, rejected) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
    assert!(!status.is_success(), "{rejected}");
    assert!(rejected.to_string().contains("exact readback"));
    assert_eq!(counts(&f.delta.state), committed);
    assert_eq!(job_records(&f.delta.state), jobs);
    assert_eq!(
        finish(&f.delta.app, identity).await.0,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(counts(&f.delta.state), committed);
    release_tx.send(()).unwrap();
    assert!(matches!(
        download.join().unwrap(),
        Err(RuntimePackageDownloadError::Ownership(_))
    ));
    let (status, result) = finish(&f.delta.app, identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "confirmed");
    assert_eq!(result["entries"].as_array().unwrap().len(), 2);
    assert!(!parent.exists());
    assert_eq!(counts(&f.delta.state), committed);
}

fn reopen(root: &FsPath) -> (ServerState, Router) {
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

#[cfg(any(unix, windows))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn committed_restart_cleans_only_empty_runtime_containers_without_changing_acceptance() {
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    let root = TestDirectory::new("package-empty-runtime-restart");
    let (f, identity) = reviewed_fixture(root.path()).await;
    let (status, original) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    drop(f);
    let (state, app) = reopen(root.path());
    let before = counts(&state);
    let job = identity["worker_job_index"].as_u64().unwrap();
    let path = format!("{ROUTE}/worker-jobs/{job}");
    let parent = hash_parent(root.path(), identity["package_hash"].as_str().unwrap());
    let worktree = parent.join(job.to_string());
    // Final control-file unlink gap and a subsequently recreated empty slot.
    for _ in 0..2 {
        fs::create_dir_all(&worktree).unwrap();
        assert_eq!(call(&app, "GET", &path, None).await.0, StatusCode::OK);
        assert!(!worktree.exists());
        assert!(!parent.exists());
        assert_eq!(counts(&state), before);
    }
    fs::create_dir_all(&worktree).unwrap();
    fs::write(worktree.join("preserve"), b"foreign payload").unwrap();
    let (status, pending) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{pending}");
    assert!(pending
        .to_string()
        .contains("PACKAGE_FINISH_CLEANUP_UNKNOWN"));
    assert_eq!(
        fs::read(worktree.join("preserve")).unwrap(),
        b"foreign payload"
    );
    fs::remove_dir_all(&worktree).unwrap();
    let foreign = root.path().join("foreign-runtime");
    fs::create_dir(&foreign).unwrap();
    fs::write(foreign.join("keep"), b"keep").unwrap();
    #[cfg(unix)]
    symlink(&foreign, &worktree).unwrap();
    #[cfg(windows)]
    {
        let result = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&worktree)
            .arg(&foreign)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "native junction fixture failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    assert!(!finish(&app, &identity).await.0.is_success());
    assert_eq!(fs::read(foreign.join("keep")).unwrap(), b"keep");
    let accepted = immutable_result(&state, &identity);
    assert_eq!(accepted["patchset_id"], original["patchset_id"]);
    assert_eq!(accepted["submission_id"], original["submission_id"]);
    assert_eq!(counts(&state), before);
}

fn immutable_result(state: &ServerState, identity: &JsonValue) -> JsonValue {
    let (entry, db) = state.operational_binary.repository_db(1).unwrap();
    let namespace = entry
        .record
        .namespace_ascii
        .into_iter()
        .filter(|byte| *byte != 0)
        .map(char::from)
        .collect::<String>();
    let jobs = ServerOperationalWorkerJobStore::new(
        1,
        ServerRemoteBinaryDb::authority_root(&db).as_path(),
        Arc::new(ReadDomain),
    )
    .unwrap();
    let job = jobs
        .get(identity["worker_job_index"].as_u64().unwrap() as u32)
        .unwrap();
    BinaryDbServerWorkflowV0Store::new_remote_frozen(db, &namespace)
        .unwrap()
        .with_repository_index(1)
        .lookup_workflow_package_finish(
            job,
            &PackageFinishRequest::parse(&finish_request(identity)).unwrap(),
            None,
        )
        .unwrap()
        .unwrap()
}

fn job_records(
    state: &ServerState,
) -> Vec<ait_server_core::foundation::server_operational_worker_jobs::WorkerJobEntry> {
    state
        .operational_binary
        .repository_worker_runtime(1)
        .unwrap()
        .0
        .list()
        .unwrap()
}

fn assert_accepted_mapping_pending(result: &JsonValue, identity: &JsonValue, original: &JsonValue) {
    assert_eq!(result["outcome"], "incomplete", "{result}");
    assert_eq!(result["expected_identity"], *identity);
    assert_eq!(result["patchset_id"], original["patchset_id"]);
    assert_eq!(result["submission_id"], original["submission_id"]);
    assert!(result["patchset_id"].is_string());
    assert!(result["submission_id"].is_string());
    assert_eq!(result["entries"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_ready_and_finish_retries_preserve_the_original_after_head_move_and_restart() {
    use ait_server_core::foundation::server_content_binary_db::{
        server_snapshot_hash48_from_id, ServerBinaryDbSnapshotStore, ServerBinarySnapshotPayload,
    };
    let root = TestDirectory::new("package-accepted-retries");
    let (f, identity) = reviewed_fixture(root.path()).await;
    let (status, original) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    assert_eq!(original["outcome"], "confirmed");
    let raw = f.delta.raw.clone();
    let hash = f.delta.hash.clone();
    let job = identity["worker_job_index"].as_u64().unwrap() as u32;
    assert!(!hash_parent(root.path(), &hash).exists());

    // A later target head is unrelated to this immutable successful receipt.
    let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let snapshots = ServerBinaryDbSnapshotStore::<_, 1>::new(db.clone());
    let read = BinaryDbReadTxn::new(&db);
    let (parent, mut record) = snapshots
        .snapshot_by_id(&read, TESTED_CHANGE)
        .unwrap()
        .unwrap();
    drop(read);
    let later = "SNP-000000000008";
    record.snapshot_hash48 = server_snapshot_hash48_from_id(later).unwrap();
    record.parent_snapshot_index_plus1 = parent + 1;
    record.created_at_s += 60;
    let index = snapshots
        .append_snapshot(
            later,
            record,
            &ServerBinarySnapshotPayload {
                line_name: "main".into(),
                message: Some("Later independent target head".into()),
            },
        )
        .unwrap();
    ServerBinaryDbLineStore::<_, 1>::new(db)
        .set_line_head("main", index + 1, 2000)
        .unwrap();
    let committed = counts(&f.delta.state);
    let jobs = job_records(&f.delta.state);
    for selected in [None, Some(job), Some(job + 1)] {
        let (status, rejected) = upload(&f.delta.app, &raw, &hash, selected).await;
        assert!(!status.is_success(), "{rejected}");
        assert!(
            rejected.to_string().contains("exact readback"),
            "{rejected}"
        );
        let (status, replay) = finish(&f.delta.app, &identity).await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_accepted_mapping_pending(&replay, &identity, &original);
        assert_eq!(counts(&f.delta.state), committed);
        assert_eq!(job_records(&f.delta.state), jobs);
        assert!(!hash_parent(root.path(), &hash).exists());
    }
    drop(f);
    let (state, app) = reopen(root.path());
    for selected in [None, Some(job)] {
        let (status, rejected) = upload(&app, &raw, &hash, selected).await;
        assert!(!status.is_success(), "{rejected}");
        assert!(
            rejected.to_string().contains("exact readback"),
            "{rejected}"
        );
        let (status, replay) = finish(&app, &identity).await;
        assert_eq!(status, StatusCode::OK, "{replay}");
        assert_accepted_mapping_pending(&replay, &identity, &original);
        assert_eq!(counts(&state), committed);
        assert_eq!(job_records(&state), jobs);
        assert!(!hash_parent(root.path(), &hash).exists());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn accepted_finish_rejects_every_conflicting_pin_without_republication() {
    let root = TestDirectory::new("package-accepted-wrong-pins");
    let (f, identity) = reviewed_fixture(root.path()).await;
    let (status, original) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    let committed = counts(&f.delta.state);
    let jobs = job_records(&f.delta.state);
    for field in [
        "repository_index",
        "worker_job_index",
        "package_hash",
        "base_snapshot_id",
        "revision_snapshot_id",
        "target_line",
        "mode",
    ] {
        let mut request = finish_request(&identity);
        match field {
            "repository_index" => request["expected_identity"][field] = json!(2),
            "worker_job_index" => request["expected_identity"][field] = json!(9999),
            "package_hash" => request["expected_identity"][field] = json!("00".repeat(32)),
            "base_snapshot_id" | "revision_snapshot_id" => {
                request["expected_identity"][field] = json!(BASE)
            }
            "target_line" => request[field] = json!("other-target"),
            "mode" => request[field] = json!("merge"),
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
        let (status, rejected) = call(
            &f.delta.app,
            "POST",
            &format!("{ROUTE}/task-land"),
            Some(request),
        )
        .await;
        assert!(!status.is_success(), "{field}: {rejected}");
        assert!(rejected.get("patchset_id").is_none());
        assert!(rejected.get("submission_id").is_none());
        assert_eq!(counts(&f.delta.state), committed);
        assert_eq!(job_records(&f.delta.state), jobs);
    }
    let (status, replay) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_accepted_mapping_pending(&replay, &identity, &original);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreadable_formal_acceptance_holds_both_ready_and_finish_without_new_work() {
    let root = TestDirectory::new("package-accepted-unreadable");
    let (f, identity) = reviewed_fixture(root.path()).await;
    let (status, original) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{original}");
    let committed = counts(&f.delta.state);
    let jobs = job_records(&f.delta.state);
    let (_, db) = f.delta.state.operational_binary.repository_db(1).unwrap();
    let path = db
        .resolve_payload_path(&WorkflowBinaryV0Codec::patchset_summary_file())
        .unwrap();
    let original_payload = fs::read(&path).unwrap();
    let job = jobs
        .iter()
        .find(|job| {
            Some(u64::from(job.key.worker_job_index)) == identity["worker_job_index"].as_u64()
        })
        .unwrap();
    let read = BinaryDbReadTxn::new(&db);
    let patchset = WorkflowBinaryV0Codec::decode_frozen_patchset(
        &read
            .read_record(
                WorkflowBinaryV0Codec::patchset_file(),
                job.record.patchset_index_plus1 - 1,
            )
            .unwrap(),
    )
    .unwrap();
    let raw = read
        .read_payload(
            WorkflowBinaryV0Codec::patchset_summary_file(),
            patchset.summary_offset,
            u32::from(patchset.summary_len),
        )
        .unwrap();
    drop(read);
    let text = WorkflowBinaryV0Codec::decode_single_text_payload(&raw, "Patchset summary").unwrap();
    let (prefix, text) = text.split_once(' ').unwrap();
    let manifest: JsonValue = serde_json::from_str(text).unwrap();
    let offset = original_payload
        .windows(raw.len())
        .position(|part| part == raw)
        .unwrap();
    let mut corruptions = vec![("unreadable", Vec::new())];
    for field in [
        "key",
        "target",
        "base",
        "revision",
        "task",
        "patchset",
        "patchset_index",
    ] {
        let mut value = manifest.clone();
        match field {
            "key" => {
                let key = value["idempotency_key"].as_str().unwrap();
                value["idempotency_key"] = json!(format!("{}:9", key.rsplit_once(':').unwrap().0));
            }
            "target" => value["target_line"] = json!("lost"),
            "base" => value["base_snapshot_id"] = json!(BASE),
            "revision" => value["revision_snapshot_id"] = json!(BASE),
            "task" | "patchset" => {
                let key = if field == "task" {
                    "task_id"
                } else {
                    "patchset_id"
                };
                let original = value["aggregate"][key].as_str().unwrap();
                value["aggregate"][key] = json!(format!("X{}", &original[1..]));
            }
            "patchset_index" => value["aggregate"]["patchset_index"] = json!(9),
            _ => unreachable!(),
        }
        let encoded = WorkflowBinaryV0Codec::encode_single_text_payload(
            &format!("{prefix} {value}"),
            "Patchset summary",
        )
        .unwrap();
        assert_eq!(
            encoded.len(),
            raw.len(),
            "same-size isolated corruption: {field}"
        );
        let mut payload = original_payload.clone();
        payload[offset..offset + encoded.len()].copy_from_slice(&encoded);
        corruptions.push((field, payload));
    }
    for (field, payload) in corruptions {
        fs::write(&path, payload).unwrap();
        let (status, rejected) = finish(&f.delta.app, &identity).await;
        assert!(!status.is_success(), "{field}: {rejected}");
        if field == "key" {
            assert!(
                rejected
                    .to_string()
                    .contains("Accepted original Job/Patchset identity differs"),
                "{rejected}"
            );
        } else if field != "unreadable" {
            assert!(
                rejected
                    .to_string()
                    .contains("Immutable package aggregate identity differs"),
                "{field}: {rejected}"
            );
        }
        assert!(rejected.get("patchset_id").is_none());
        assert!(rejected.get("submission_id").is_none());
        let (status, rejected) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
        assert!(!status.is_success(), "{rejected}");
        assert_eq!(counts(&f.delta.state), committed);
        assert_eq!(job_records(&f.delta.state), jobs);
    }
    fs::write(&path, original_payload).unwrap();
    let (status, replay) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_accepted_mapping_pending(&replay, &identity, &original);
}

#[cfg(unix)]
struct ReadOnlyDirectory {
    path: std::path::PathBuf,
    original: fs::Permissions,
}

#[cfg(unix)]
impl ReadOnlyDirectory {
    fn new(path: &FsPath) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let original = fs::metadata(path).unwrap().permissions();
        fs::set_permissions(path, fs::Permissions::from_mode(0o555)).unwrap();
        Self {
            path: path.to_path_buf(),
            original,
        }
    }
}

#[cfg(unix)]
impl Drop for ReadOnlyDirectory {
    fn drop(&mut self) {
        fs::set_permissions(&self.path, self.original.clone()).unwrap();
    }
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn committed_cleanup_error_and_job_get_preserve_the_full_native_replay() {
    let root = TestDirectory::new("package-strict-source-cleanup");
    let (f, identity) = reviewed_fixture(root.path()).await;
    let parent = hash_parent(root.path(), &f.delta.hash);
    let worktree = parent.join(identity["worker_job_index"].to_string());
    let before = counts(&f.delta.state);
    let denied = ReadOnlyDirectory::new(&worktree.join("source"));
    let (status, error) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
    assert!(error.to_string().contains("PACKAGE_FINISH_CLEANUP_PENDING"));
    assert!(error.to_string().contains("publication committed"));
    let committed = counts(&f.delta.state);
    assert_ne!(committed, before);
    assert!(worktree.exists());
    let immutable = immutable_result(&f.delta.state, &identity);
    assert_eq!(immutable["outcome"], "incomplete");
    assert_eq!(immutable["expected_identity"], identity);
    assert!(immutable["patchset_id"].is_string());
    assert!(immutable["submission_id"].is_string());
    assert_eq!(immutable["entries"], json!([]));
    let jobs = job_records(&f.delta.state);
    let (status, rejected) = upload(&f.delta.app, &f.delta.raw, &f.delta.hash, None).await;
    assert!(!status.is_success(), "{rejected}");
    assert!(rejected.to_string().contains("exact readback"));
    assert_eq!(job_records(&f.delta.state), jobs);
    let (status, repeated) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{repeated}");
    assert_eq!(counts(&f.delta.state), committed);
    drop(denied);
    let path = format!("{ROUTE}/worker-jobs/{}", identity["worker_job_index"]);
    let (status, job) = call(&f.delta.app, "GET", &path, None).await;
    assert_eq!(status, StatusCode::OK, "{job}");
    assert!(job["job"]["patchset_index"].is_number());
    assert_eq!(job["job"]["attempt_count"], 1);
    assert!(!parent.exists());
    let (status, result) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "confirmed");
    assert_eq!(result["entries"].as_array().unwrap().len(), 2);
    assert_eq!(result["entries"][0], f.mapped["entries"][0]);
    assert_eq!(result["patchset_id"], immutable["patchset_id"]);
    assert_eq!(counts(&f.delta.state), committed);
    let (status, after_release) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{after_release}");
    assert_eq!(after_release["outcome"], "incomplete");
    assert_eq!(after_release["patchset_id"], result["patchset_id"]);
    assert_eq!(after_release["submission_id"], result["submission_id"]);
    assert_eq!(after_release["entries"], json!([]));
    assert_eq!(counts(&f.delta.state), committed);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finish_parent_failure_after_child_removal_is_retryable_without_republication() {
    let root = TestDirectory::new("package-strict-parent-cleanup");
    let (f, identity) = reviewed_fixture(root.path()).await;
    let parent = hash_parent(root.path(), &f.delta.hash);
    let worktree = parent.join(identity["worker_job_index"].to_string());
    let denied = ReadOnlyDirectory::new(parent.parent().unwrap());
    let (status, error) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{error}");
    assert!(error.to_string().contains("PACKAGE_FINISH_CLEANUP_PENDING"));
    assert!(error.to_string().contains("hash parent"));
    assert!(!worktree.exists());
    assert!(parent.is_dir());
    let committed = counts(&f.delta.state);
    let mut wrong = identity.clone();
    wrong["package_hash"] = json!("11".repeat(32));
    let (status, result) = finish(&f.delta.app, &wrong).await;
    assert_eq!(status, StatusCode::CONFLICT, "{result}");
    assert!(!result.to_string().contains("PACKAGE_FINISH_CLEANUP_"));
    assert!(parent.is_dir());
    assert_eq!(counts(&f.delta.state), committed);
    fs::create_dir(&worktree).unwrap();
    let replacement = worktree.join("foreign-resource");
    fs::write(&replacement, b"preserve").unwrap();
    let (status, unknown) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{unknown}");
    assert!(unknown
        .to_string()
        .contains("PACKAGE_FINISH_CLEANUP_UNKNOWN"));
    assert_eq!(fs::read(&replacement).unwrap(), b"preserve");
    assert_eq!(counts(&f.delta.state), committed);
    fs::remove_dir_all(&worktree).unwrap();
    drop(denied);
    fs::create_dir(&worktree).unwrap();
    let (status, result) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "confirmed");
    assert_eq!(result["entries"].as_array().unwrap().len(), 2);
    assert!(!parent.exists());
    assert_eq!(counts(&f.delta.state), committed);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn restart_retained_unowned_source_holds_strict_finish_but_not_immutable_reads() {
    let root = TestDirectory::new("package-strict-unknown-owner");
    let (f, identity) = reviewed_fixture(root.path()).await;
    let parent = hash_parent(root.path(), &f.delta.hash);
    let worktree = parent.join(identity["worker_job_index"].to_string());
    let denied = ReadOnlyDirectory::new(&worktree.join("source"));
    assert_eq!(
        finish(&f.delta.app, &identity).await.0,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    let committed = counts(&f.delta.state);
    let original = immutable_result(&f.delta.state, &identity);
    let raw = f.delta.raw.clone();
    let hash = f.delta.hash.clone();
    let jobs = job_records(&f.delta.state);
    drop(denied);
    drop(f);
    let (state, app) = reopen(root.path());
    let (status, error) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(error.to_string().contains("PACKAGE_FINISH_CLEANUP_UNKNOWN"));
    assert!(error.to_string().contains("publication committed"));
    assert!(worktree.exists());
    assert_eq!(immutable_result(&state, &identity), original);
    assert_eq!(original["outcome"], "incomplete");
    assert!(original["patchset_id"].is_string());
    let (status, rejected) = upload(&app, &raw, &hash, None).await;
    assert!(!status.is_success(), "{rejected}");
    assert!(rejected.to_string().contains("exact readback"));
    assert_eq!(job_records(&state), jobs);
    assert_eq!(counts(&state), committed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_source_after_restart_preserves_acceptance_without_inventing_membership() {
    let root = TestDirectory::new("package-strict-no-source");
    let (f, identity) = reviewed_fixture(root.path()).await;
    let parent = hash_parent(root.path(), &f.delta.hash);
    let (status, result) = finish(&f.delta.app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "confirmed");
    assert!(!parent.exists());
    let committed = counts(&f.delta.state);
    drop(f);
    let (state, app) = reopen(root.path());
    let (status, replay) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["outcome"], "incomplete");
    assert_eq!(replay["entries"], json!([]));
    assert_eq!(replay["patchset_id"], result["patchset_id"]);
    assert_eq!(replay["submission_id"], result["submission_id"]);
    assert_eq!(counts(&state), committed);
}

fn abandon_request(identity: &JsonValue) -> JsonValue {
    json!({"contract":"workflow-package-abandon/v1","actor_key_index":0,"expected_identity":identity,
        "idempotency_key":format!("workflow-package-abandon:{}:{}",identity["package_hash"].as_str().unwrap(),identity["worker_job_index"])})
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn known_abandon_removes_only_its_job_and_keeps_a_nonempty_hash_parent() {
    let root = TestDirectory::new("package-strict-abandon-sibling");
    let (_, app, raw, hash) = fixture(root.path()).await;
    let (status, ready) = upload(&app, &raw, &hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    let identity = &ready["expected_identity"];
    let parent = hash_parent(root.path(), &hash);
    let sibling = parent.join("another-job");
    fs::create_dir(&sibling).unwrap();
    #[cfg(unix)]
    let _denied = ReadOnlyDirectory::new(parent.parent().unwrap());
    let path = format!("{ROUTE}/worker-jobs/{}", identity["worker_job_index"]);
    let (status, result) = call(&app, "POST", &path, Some(abandon_request(identity))).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["state"], "abandoned");
    assert!(!parent
        .join(identity["worker_job_index"].to_string())
        .exists());
    assert!(sibling.is_dir());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn abandon_parent_io_failure_keeps_pending_and_missing_parent_retry_completes() {
    let root = TestDirectory::new("package-strict-abandon-parent");
    let (_, app, raw, hash) = fixture(root.path()).await;
    let (_, ready) = upload(&app, &raw, &hash, None).await;
    let identity = &ready["expected_identity"];
    let parent = hash_parent(root.path(), &hash);
    let path = format!("{ROUTE}/worker-jobs/{}", identity["worker_job_index"]);
    let denied = ReadOnlyDirectory::new(parent.parent().unwrap());
    let request = abandon_request(identity);
    let (status, pending) = call(&app, "POST", &path, Some(request.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    assert_eq!(pending["state"], "abandoning");
    assert!(parent.is_dir());
    assert!(!parent
        .join(identity["worker_job_index"].to_string())
        .exists());
    drop(denied);
    fs::remove_dir(&parent).unwrap();
    let (status, done) = call(&app, "POST", &path, Some(request)).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["state"], "abandoned");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn same_hash_ready_and_abandon_do_not_race_parent_child_creation() {
    let root = TestDirectory::new("package-strict-concurrent-parent");
    let (_, app, raw, hash) = fixture(root.path()).await;
    let (_, ready) = upload(&app, &raw, &hash, None).await;
    let identity = &ready["expected_identity"];
    let path = format!("{ROUTE}/worker-jobs/{}", identity["worker_job_index"]);
    let (prepared, abandoned) = tokio::join!(
        upload(&app, &raw, &hash, None),
        call(&app, "POST", &path, Some(abandon_request(identity))),
    );
    assert_eq!(prepared.0, StatusCode::ACCEPTED, "{}", prepared.1);
    assert_eq!(abandoned.0, StatusCode::OK, "{}", abandoned.1);
    let (status, next) = upload(&app, &raw, &hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{next}");
    assert_ne!(
        next["expected_identity"]["worker_job_index"],
        identity["worker_job_index"]
    );
    assert!(hash_parent(root.path(), &hash)
        .join(next["expected_identity"]["worker_job_index"].to_string())
        .is_dir());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rejected_preparation_removes_its_exact_empty_hash_parent_without_a_job() {
    let root = TestDirectory::new("package-strict-prepare-parent");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let mut manifest = serde_json::to_value(
        WorkflowPackage::read_sealed_manifest(
            &raw,
            parse_package_hash(&hash).unwrap(),
            1,
            usize::MAX,
        )
        .unwrap(),
    )
    .unwrap();
    manifest["history_entries"][0]["task"]["plan_id"] = json!("PR-9999");
    manifest["history_entries"][0]["task"]["plan_item_ref"] = json!("missing/item");
    let members = BTreeMap::from([(
        format!("trees/{PACK}"),
        fs::read(root.path().join("fixture-tree.zstpack")).unwrap(),
    )]);
    let (raw, hash) = archive(&manifest, &members);
    let before = counts(&state);
    let (status, result) = upload(&app, &raw, &hash, None).await;
    assert!(!status.is_success(), "{result}");
    assert!(!result.to_string().contains("PACKAGE_FINISH_CLEANUP_"));
    assert!(!hash_parent(root.path(), &hash).exists());
    assert_eq!(counts(&state), before);
    let (_, jobs) = call(&app, "GET", &format!("{ROUTE}/worker-jobs"), None).await;
    assert_eq!(jobs["count"], 0);

    #[cfg(unix)]
    {
        let parent = hash_parent(root.path(), &hash);
        fs::create_dir_all(&parent).unwrap();
        let denied = ReadOnlyDirectory::new(parent.parent().unwrap());
        let (status, result) = upload(&app, &raw, &hash, None).await;
        assert!(!status.is_success(), "{result}");
        assert!(result
            .to_string()
            .contains("preparation cleanup incomplete"));
        assert!(result.to_string().contains("history:"));
        assert!(!result.to_string().contains("PACKAGE_FINISH_CLEANUP_"));
        assert!(parent.is_dir());
        assert!(!parent.join("0").exists());
        assert_eq!(counts(&state), before);
        drop(denied);
    }
}
