use super::*;

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires an authorized Unix root test process to assign a real foreign UID"]
async fn retained_reprepare_rejects_foreign_uid_root_even_when_server_can_write() {
    use std::os::unix::fs::MetadataExt;
    let uid = std::process::Command::new("id").arg("-u").output().unwrap();
    assert!(uid.status.success());
    assert_eq!(
        String::from_utf8(uid.stdout).unwrap().trim(),
        "0",
        "requires a Unix root test process"
    );
    let root = TestDirectory::new("retained-foreign-uid-root");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let ready = prepare_package(&app, &raw, &hash).await;
    let job = ready["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap() as u32;
    complete(&app, u64::from(job), "pass").await;
    drop(app);
    drop(state);
    let managed = root.path().join("workflow-packages/generation");
    struct RestoreOwner(std::path::PathBuf);
    impl Drop for RestoreOwner {
        fn drop(&mut self) {
            let restored = std::process::Command::new("chown")
                .args(["-R", "0"])
                .arg(&self.0)
                .status();
            assert!(
                restored.is_ok_and(|status| status.success()),
                "restore disposable runtime ownership"
            );
        }
    }
    let _restore = RestoreOwner(managed.clone());
    assert!(std::process::Command::new("chown")
        .args(["-R", "1"])
        .arg(&managed)
        .status()
        .unwrap()
        .success());
    let metadata = fs::symlink_metadata(&managed).unwrap();
    assert_eq!(metadata.uid(), 1);
    assert_eq!(metadata.mode() & 0o022, 0);
    let probe = managed.join("server-write-probe");
    fs::write(&probe, b"Server can write despite foreign ownership").unwrap();
    fs::remove_file(&probe).unwrap();
    let source = managed.join("1").join(&hash).join(job.to_string());
    assert_eq!(fs::symlink_metadata(&source).unwrap().uid(), 1);
    assert_eq!(
        fs::symlink_metadata(source.join("package.zst"))
            .unwrap()
            .uid(),
        1
    );
    let (state, app) = reopen(root.path());
    let before = records(&state);
    let (status, error) = upload(&app, &[], &hash, Some(job)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(
        error.to_string().contains("exclusively Server-owned"),
        "{error}"
    );
    assert_eq!(records(&state), before);
    assert_eq!(fs::read(source.join("package.zst")).unwrap(), raw);
    no_publication(&state);
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

async fn submit_evidence(app: &Router, body: &JsonValue) -> JsonValue {
    let job = body["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    let (status, result) = call(
        app,
        "POST",
        &format!("{ROUTE}/worker-jobs/{job}"),
        Some(body.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["accepted"], true);
    assert!(result["recorded_at_s"].as_u64().unwrap() > 0);
    result
}

fn human_review(identity: &JsonValue, key: &str, action: &str) -> JsonValue {
    json!({"contract":"workflow-package-evidence/v1","operation":"review","idempotency_key":key,
        "actor_key_index":0,"expected_identity":identity,
        "payload":{"reviewer":"Alice","reviewer_kind":"user","action":action}})
}

async fn approve(app: &Router, identity: &JsonValue) -> (JsonValue, JsonValue) {
    attest(app, identity, "ai_with_human_review").await;
    review(app, identity, "independent-summary", "code_review_summary").await;
    review(app, identity, "independent-task-approval", "task_approve").await;
    let request = human_review(identity, "human-approval", "approve");
    let accepted = submit_evidence(app, &request).await;
    (request, accepted)
}

fn records(
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

// The owner withdrew same-Job persistence. Exercise the replacement contract:
// old evidence fails closed, existing ready creates a new execution of sealed bytes.
#[cfg(any(unix, windows))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn package_restart_reprepares_same_sealed_mode_with_new_ci_and_agent_review() {
    let root = TestDirectory::new("package-restart-reprepare-positive");
    let (state, app, original, original_hash) = fixture(root.path()).await;
    let mut sealed = serde_json::to_value(
        ait_server_core::foundation::workflow_package::WorkflowPackage::read_sealed_manifest(
            &original,
            ait_server_core::foundation::workflow_package_contract::parse_package_hash(
                &original_hash,
            )
            .unwrap(),
            1,
            usize::MAX,
        )
        .unwrap(),
    )
    .unwrap();
    sealed["author_mode"] = json!("ai_only_experimental");
    let (raw, hash) = archive(
        &sealed,
        &BTreeMap::from([(
            format!("trees/{PACK}"),
            fs::read(root.path().join("fixture-tree.zstpack")).unwrap(),
        )]),
    );
    let prepared = prepare_package(&app, &raw, &hash).await;
    let old_identity = prepared["expected_identity"].clone();
    let old_job = old_identity["worker_job_index"].as_u64().unwrap();
    complete(&app, old_job, "pass").await;
    attest(&app, &old_identity, "ai_only_experimental").await;
    review(&app, &old_identity, "old-summary", "code_review_summary").await;
    review(&app, &old_identity, "old-approval", "task_approve").await;
    let old_records = records(&state);
    let old_source = state
        .operational_binary
        .repository_worker_runtime(1)
        .unwrap()
        .0
        .prepared_ci_runtime()
        .source(old_records[0].key)
        .unwrap()
        .worktree()
        .to_path_buf();
    drop(app);
    drop(state);
    let (state, app) = reopen(root.path());
    let (_, old_read) = call(&app, "GET", &format!("{ROUTE}/worker-jobs/{old_job}"), None).await;
    assert!(old_read["package_execution"].is_null());
    assert_eq!(records(&state), old_records);
    assert!(!finish(&app, &old_identity).await.0.is_success());
    assert_eq!(
        upload(&app, &raw, &hash, Some(old_job as u32)).await.0,
        StatusCode::CONFLICT
    );
    let (first, replay) = tokio::join!(
        upload(&app, &[], &hash, Some(old_job as u32)),
        upload(&app, &[], &hash, Some(old_job as u32)),
    );
    assert_eq!(first.0, StatusCode::ACCEPTED, "{}", first.1);
    assert_eq!(replay.0, StatusCode::ACCEPTED, "{}", replay.1);
    assert_eq!(first.1["expected_identity"], replay.1["expected_identity"]);
    let fresh = first.1;
    let identity = fresh["expected_identity"].clone();
    let job = identity["worker_job_index"].as_u64().unwrap();
    assert_ne!(job, old_job);
    for field in ["package_hash", "base_snapshot_id", "revision_snapshot_id"] {
        assert_eq!(identity[field], old_identity[field]);
    }
    let (_, observed) = call(&app, "GET", &format!("{ROUTE}/worker-jobs/{job}"), None).await;
    assert_eq!(
        observed["package_execution"]["author_mode"],
        "ai_only_experimental"
    );
    assert!(!old_source.exists());
    let new_source = state
        .operational_binary
        .repository_worker_runtime(1)
        .unwrap()
        .0
        .prepared_ci_runtime()
        .source(records(&state)[1].key)
        .unwrap();
    assert_eq!(fs::read(new_source.package()).unwrap(), raw);
    assert!(!finish(&app, &identity).await.0.is_success());
    complete(&app, job, "pass").await;
    // Retry the old selector after the first response was lost and CI finished.
    // The old archive was already cleaned; the valid replacement must be joined.
    let (status, repeated) = upload(&app, &[], &hash, Some(old_job as u32)).await;
    assert_eq!(status, StatusCode::OK, "{repeated}");
    assert_eq!(repeated["expected_identity"], identity);
    assert_eq!(repeated["ci_state"], "passed");
    assert_eq!(repeated["reused"], true);
    assert_eq!(records(&state).len(), 2);
    assert!(!finish(&app, &identity).await.0.is_success());
    // No old agent review/attestation is borrowed; no extra human approval is added.
    attest(&app, &identity, "ai_only_experimental").await;
    review(
        &app,
        &identity,
        "new-independent-summary",
        "code_review_summary",
    )
    .await;
    review(&app, &identity, "new-independent-approval", "task_approve").await;
    let (status, result) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "confirmed");
    assert_eq!(result["expected_identity"], identity);
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    assert_eq!(records(&state).len(), 2);
    assert_eq!(records(&state)[0].record.patchset_index_plus1, 0);
    let accepted = counts(&state);
    let (status, replay) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["submission_id"], result["submission_id"]);
    assert_eq!(counts(&state), accepted);
    assert!(!finish(&app, &old_identity).await.0.is_success());
    let (status, accepted_reprepare) = upload(&app, &[], &hash, Some(old_job as u32)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{accepted_reprepare}");
    assert!(accepted_reprepare.to_string().contains("exact readback"));
    assert_eq!(counts(&state), accepted);
    assert_eq!(records(&state).len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_reprepare_rejects_unknown_reset_and_unavailable_foreign_or_ambiguous_source_without_jobs(
) {
    for case in [
        "unknown-reset",
        "missing",
        "corrupt",
        "foreign",
        "foreign-runtime",
        "ambiguous",
        "unknown-job",
    ] {
        let root = TestDirectory::new("retained-reprepare-negative");
        let (state, app, raw, hash) = fixture(root.path()).await;
        let ready = prepare_package(&app, &raw, &hash).await;
        let job = ready["expected_identity"]["worker_job_index"]
            .as_u64()
            .unwrap() as u32;
        complete(&app, u64::from(job), "pass").await;
        let (state, app) = if case == "unknown-reset" {
            (state, app)
        } else {
            drop(app);
            drop(state);
            reopen(root.path())
        };
        let worktree = root
            .path()
            .join("workflow-packages/generation/1")
            .join(&hash)
            .join(job.to_string());
        let package = worktree.join("package.zst");
        match case {
            "foreign-runtime" => fs::write(worktree.join("foreign-runtime"), b"keep").unwrap(),
            "missing" => fs::remove_file(&package).unwrap(),
            "corrupt" => fs::write(&package, b"corrupt").unwrap(),
            "foreign" => {
                #[cfg(unix)]
                {
                    let foreign = root.path().join("foreign-package.zst");
                    fs::write(&foreign, &raw).unwrap();
                    fs::remove_file(&package).unwrap();
                    std::os::unix::fs::symlink(foreign, &package).unwrap();
                }
                #[cfg(not(unix))]
                fs::write(&package, b"foreign").unwrap();
            }
            "ambiguous" => {
                let duplicate = root
                    .path()
                    .join("workflow-packages/generation/1")
                    .join("aa".repeat(32))
                    .join(job.to_string());
                fs::create_dir_all(duplicate.join("source")).unwrap();
                fs::write(duplicate.join("package.zst"), &raw).unwrap();
            }
            _ => {}
        }
        let before = records(&state);
        let selected = if case == "unknown-job" { u32::MAX } else { job };
        let (status, error) = upload(&app, &[], &hash, Some(selected)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{case}: {error}");
        assert_eq!(records(&state), before);
        assert!(worktree.exists());
        no_publication(&state);
    }
}

#[cfg(windows)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn windows_retained_reprepare_holds_hostile_acl_and_junction_before_allocation() {
    for case in ["writable-parent", "writable-archive", "junction"] {
        let root = TestDirectory::new("windows-retained-source-scope");
        let (state, app, raw, hash) = fixture(root.path()).await;
        let ready = prepare_package(&app, &raw, &hash).await;
        let job = ready["expected_identity"]["worker_job_index"]
            .as_u64()
            .unwrap() as u32;
        complete(&app, u64::from(job), "pass").await;
        drop(app);
        drop(state);
        let (state, app) = reopen(root.path());
        let worktree = root
            .path()
            .join("workflow-packages/generation/1")
            .join(&hash)
            .join(job.to_string());
        let archive = worktree.join("package.zst");
        let parent = worktree.parent().unwrap();
        let protected = if case == "writable-parent" {
            parent
        } else {
            &archive
        };
        let saved_source = root.path().join("saved-source");
        let foreign = root.path().join("foreign source & %PATH% !junction!");
        if case == "junction" {
            fs::rename(worktree.join("source"), &saved_source).unwrap();
            fs::create_dir(&foreign).unwrap();
            fs::write(foreign.join("keep"), b"keep").unwrap();
            windows_fixture::junction(&worktree.join("source"), &foreign);
        } else {
            let mut command = std::process::Command::new("icacls.exe");
            command.arg(protected).args(["/grant", "*S-1-1-0:(W)"]);
            let result = command.output().unwrap();
            assert!(
                result.status.success(),
                "{case}: {command:?}: status={}; stdout={}; stderr={}",
                result.status,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let before = records(&state);
        let (status, error) = upload(&app, &[], &hash, Some(job)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{case}: {error}");
        assert_eq!(records(&state), before);
        assert_eq!(fs::read(&archive).unwrap(), raw);
        no_publication(&state);
        if case == "junction" {
            assert_eq!(fs::read(foreign.join("keep")).unwrap(), b"keep");
            fs::remove_dir(worktree.join("source")).unwrap();
            fs::rename(saved_source, worktree.join("source")).unwrap();
        } else {
            let mut command = std::process::Command::new("icacls.exe");
            command.arg(protected).args(["/remove:g", "*S-1-1-0"]);
            let result = command.output().unwrap();
            assert!(
                result.status.success(),
                "{case}: {command:?}: status={}; stdout={}; stderr={}",
                result.status,
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let (status, fresh) = upload(&app, &[], &hash, Some(job)).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{case}: {fresh}");
        assert_ne!(fresh["expected_identity"]["worker_job_index"], job);
        assert_eq!(records(&state).len(), before.len() + 1);
        assert!(!worktree.exists());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_reprepare_requires_unambiguous_existing_headers_before_mutation() {
    use ait_server_core::foundation::workflow_package_contract::PACKAGE_CONTENT_TYPE;
    let root = TestDirectory::new("retained-reprepare-headers");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let ready = prepare_package(&app, &raw, &hash).await;
    let job = ready["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap()
        .to_string();
    let before = records(&state);
    for target in [
        "x-ait-package-sha256",
        "x-ait-actor-key-index",
        "x-ait-worker-job-index",
    ] {
        for duplicate in [false, true] {
            let mut request = Request::builder()
                .method("POST")
                .uri(format!("{ROUTE}/remote-sync/zstd-bulk/commit"))
                .header(CONTENT_TYPE, PACKAGE_CONTENT_TYPE);
            for (name, value) in [
                ("x-ait-package-sha256", hash.as_str()),
                ("x-ait-actor-key-index", "0"),
                ("x-ait-worker-job-index", job.as_str()),
            ] {
                if name != target || duplicate {
                    request = request.header(name, value);
                }
                if name == target && duplicate {
                    request = request.header(name, value);
                }
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::BAD_REQUEST,
                "{target}/{duplicate}"
            );
            assert_eq!(records(&state), before);
        }
    }
    no_publication(&state);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[cfg(any(unix, windows))]
async fn retained_reprepare_does_not_guess_a_same_hash_job_as_the_original_replacement() {
    let root = TestDirectory::new("retained-reprepare-origin");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let original = prepare_package(&app, &raw, &hash).await;
    let old = original["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap() as u32;
    complete(&app, u64::from(old), "pass").await;
    drop(app);
    drop(state);
    let (state, app) = reopen(root.path());
    let other = prepare_package(&app, &raw, &hash).await;
    let old_archive = root
        .path()
        .join("workflow-packages/generation/1")
        .join(&hash)
        .join(old.to_string())
        .join("package.zst");
    fs::remove_file(&old_archive).unwrap();
    let before = records(&state);
    let (status, unknown) = upload(&app, &[], &hash, Some(old)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{unknown}");
    assert_eq!(records(&state), before);
    // Only after proving the original bytes may this existing valid execution
    // become the remembered replacement for exact old-selector retries.
    fs::write(&old_archive, &raw).unwrap();
    let (status, joined) = upload(&app, &[], &hash, Some(old)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{joined}");
    assert_eq!(joined["expected_identity"], other["expected_identity"]);
    assert_eq!(records(&state), before);
    assert!(!old_archive.exists());
    let (status, retry) = upload(&app, &[], &hash, Some(old)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{retry}");
    assert_eq!(retry["expected_identity"], other["expected_identity"]);
    assert_eq!(records(&state), before);
    no_publication(&state);
}

#[cfg(any(unix, windows))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_reprepare_accepts_a_real_terminal_runner_failure_after_restart() {
    let root = TestDirectory::new("retained-reprepare-terminal-failure");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let original = prepare_package(&app, &raw, &hash).await;
    let old = original["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap() as u32;
    let path = format!("{ROUTE}/worker-jobs/{old}");
    let (status, claim) = call(
        &app,
        "POST",
        &format!("{path}:claim"),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let proof = &claim["claimed_job"];
    let (status, failed) = call(&app, "POST", &format!("{path}:fail"), Some(json!({
        "attempt_count":proof["attempt_count"],"lease_token":proof["lease_token"],
        "detail":{"job_kind":7,"retryable":false,"error":"owned process stopped after terminal failure"}
    }))).await;
    assert_eq!(status, StatusCode::OK, "{failed}");
    let before = records(&state);
    assert_eq!(
        before[0].record.state_kind,
        ait_server_core::foundation::operational_binary_v0::WORKER_JOB_STATE_FAILED
    );
    assert_eq!(
        before[0].record.error_kind,
        ait_server_core::foundation::operational_binary_v0::WORKER_JOB_ERROR_TERMINAL_EXECUTION
    );
    assert!(state
        .operational_binary
        .repository_worker_runtime(1)
        .unwrap()
        .1
        .entries()
        .unwrap()
        .is_empty());
    drop(app);
    drop(state);
    let (state, app) = reopen(root.path());
    let (status, ready) = upload(&app, &[], &hash, Some(old)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{ready}");
    assert_ne!(ready["expected_identity"]["worker_job_index"], old);
    assert_eq!(ready["expected_identity"]["package_hash"], hash);
    assert_eq!(records(&state)[0], before[0]);
    assert_eq!(records(&state).len(), 2);
    no_publication(&state);
}

#[cfg(not(any(unix, windows)))]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn retained_reprepare_without_native_owner_validation_holds_before_allocation_or_cleanup() {
    let root = TestDirectory::new("retained-reprepare-platform-ownership");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let original = prepare_package(&app, &raw, &hash).await;
    let old = original["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap() as u32;
    complete(&app, u64::from(old), "pass").await;
    drop(app);
    drop(state);
    let (state, app) = reopen(root.path());
    let before = records(&state);
    let archive = root
        .path()
        .join("workflow-packages/generation/1")
        .join(&hash)
        .join(old.to_string())
        .join("package.zst");
    let (status, error) = upload(&app, &[], &hash, Some(old)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(error
        .to_string()
        .contains("ownership verification is unavailable"));
    assert_eq!(records(&state), before);
    assert_eq!(fs::read(archive).unwrap(), raw);
    no_publication(&state);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn package_restart_failed_tests_cannot_be_upgraded_by_an_old_completion_proof() {
    let root = TestDirectory::new("package-restart-stale-completion");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let prepared = prepare_package(&app, &raw, &hash).await;
    let identity = prepared["expected_identity"].clone();
    let job = identity["worker_job_index"].as_u64().unwrap();
    let path = format!("{ROUTE}/worker-jobs/{job}");
    let (status, claim) = call(
        &app,
        "POST",
        &format!("{path}:claim"),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let proof = claim["claimed_job"].clone();
    complete_claimed(&app, job, "fail", &proof).await;
    let (_, failed) = call(&app, "GET", &path, None).await;
    assert_eq!(failed["package_execution"]["ci_state"], "failed");
    let original_jobs = records(&state);
    drop(app);
    drop(state);

    let (state, app) = reopen(root.path());
    let (status, stale) = call(
        &app,
        "POST",
        &format!("{path}:complete"),
        Some(json!({
        "attempt_count":proof["attempt_count"],"lease_token":proof["lease_token"],
        "detail":{"job_kind":7,"result":{"contract":"ait.runner.native-result.v1",
            "status":"succeeded","tests_status":"pass","suite_result_count":2,
            "suite_results":[{"suite_id":"rust_tests","status":"pass","blocking":true},
                {"suite_id":"cargo_fmt","status":"pass","blocking":true}],"cleanup":{}}}})),
    )
    .await;
    assert!(!status.is_success(), "{stale}");
    let (_, observed) = call(&app, "GET", &path, None).await;
    assert_ne!(observed["package_execution"]["ci_state"], "passed");
    assert_ne!(observed["package_execution"]["recoverable"], true);
    assert!(!finish(&app, &identity).await.0.is_success());
    assert_eq!(records(&state), original_jobs);
    no_publication(&state);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn package_restart_revocation_is_not_cleared_by_replaying_the_old_approval() {
    let root = TestDirectory::new("package-restart-revocation");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let prepared = prepare_package(&app, &raw, &hash).await;
    let identity = prepared["expected_identity"].clone();
    let job = identity["worker_job_index"].as_u64().unwrap();
    complete(&app, job, "pass").await;
    let (approved, original_ack) = approve(&app, &identity).await;
    let revoked = human_review(&identity, "human-revocation", "request_changes");
    submit_evidence(&app, &revoked).await;
    let (status, blocked) = finish(&app, &identity).await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    assert!(blocked.to_string().contains("PACKAGE_REVIEW_BLOCKED"));
    let original_jobs = records(&state);
    drop(app);
    drop(state);

    let (state, app) = reopen(root.path());
    let (status, retry) = call(
        &app,
        "POST",
        &format!("{ROUTE}/worker-jobs/{job}"),
        Some(approved),
    )
    .await;
    // Unknown legacy state may reject the retry. A recovered execution must
    // return the first ACK without accepting the old positive decision anew.
    if status.is_success() {
        assert_eq!(retry, original_ack);
    } else {
        assert_eq!(status, StatusCode::CONFLICT, "{retry}");
    }
    assert!(!finish(&app, &identity).await.0.is_success());
    assert_eq!(records(&state), original_jobs);
    no_publication(&state);
}
