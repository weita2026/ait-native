use super::*;
use ait_server_core::foundation::prepared_ci_runtime::{
    ManagedCiPackage, ManagedCiPackagePaths, PreparedCiTarget,
};
use ait_server_core::foundation::workflow_package::WorkflowPackage;
use ait_server_core::foundation::workflow_package_contract::parse_package_hash;

fn manifest(raw: &[u8], hash: &str) -> JsonValue {
    serde_json::to_value(
        WorkflowPackage::read_sealed_manifest(
            raw,
            parse_package_hash(hash).unwrap(),
            1,
            usize::MAX,
        )
        .unwrap(),
    )
    .unwrap()
}

fn repack(root: &FsPath, manifest: &JsonValue) -> (Vec<u8>, String) {
    archive(
        manifest,
        &BTreeMap::from([(
            format!("trees/{PACK}"),
            fs::read(root.join("fixture-tree.zstpack")).unwrap(),
        )]),
    )
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sealed_mode_read_returns_original_mode_pending_running_and_done_without_runner_proof() {
    for mode in [
        "human_only",
        "human_with_ai_assist",
        "ai_with_human_review",
        "ai_only_experimental",
    ] {
        let root = TestDirectory::new("sealed-mode-read-valid");
        let (state, app, original, original_hash) = fixture(root.path()).await;
        let mut sealed = manifest(&original, &original_hash);
        sealed["author_mode"] = json!(mode);
        let (raw, hash) = repack(root.path(), &sealed);
        let (status, prepared) = upload(&app, &raw, &hash, None).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{prepared}");
        let identity = &prepared["expected_identity"];
        let job = identity["worker_job_index"].as_u64().unwrap();
        let path = format!("{ROUTE}/worker-jobs/{job}");
        let (_, queued) = call(&app, "GET", &path, None).await;
        assert_eq!(queued["package_execution"]["expected_identity"], *identity);
        assert_eq!(queued["package_execution"]["author_mode"], mode);
        assert_eq!(queued["package_execution"]["ci_state"], "queued");
        assert_eq!(queued["package_execution"]["availability"], "available");
        assert_eq!(queued["package_execution"]["recoverable"], true);
        assert_eq!(
            download(&app, job, &hash, &[]).await.status(),
            StatusCode::BAD_REQUEST
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("{path}?author_mode=human_only"))
                    .header("x-ait-author-mode", "human_only")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let ignored_hint = response_json(response).await;
        assert_eq!(ignored_hint["package_execution"]["author_mode"], mode);
        let (status, claim) = call(
            &app,
            "POST",
            &format!("{path}:claim"),
            Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{claim}");
        let (_, running) = call(&app, "GET", &path, None).await;
        assert_eq!(running["package_execution"]["author_mode"], mode);
        assert_eq!(running["package_execution"]["ci_state"], "running");
        complete_claimed(&app, job, "pass", &claim["claimed_job"]).await;
        let (_, done) = call(&app, "GET", &path, None).await;
        assert_eq!(done["package_execution"]["author_mode"], mode);
        assert_eq!(done["package_execution"]["expected_identity"], *identity);
        assert_eq!(done["package_execution"]["ci_state"], "passed");
        assert_eq!(done["package_execution"]["recoverable"], true);
        assert_eq!(
            state
                .operational_binary
                .repository_worker_runtime(1)
                .unwrap()
                .0
                .list()
                .unwrap()
                .len(),
            1
        );
        let (status, missing) = call(&app, "GET", &format!("{ROUTE}/worker-jobs/999"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
        no_publication(&state);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sealed_mode_read_missing_or_corrupt_source_is_null_without_erasing_ci() {
    for case in ["archive-missing", "archive-corrupt", "source-missing"] {
        let root = TestDirectory::new("sealed-mode-read-unavailable");
        let (state, app, raw, hash) = fixture(root.path()).await;
        let (_, prepared) = upload(&app, &raw, &hash, None).await;
        let job = prepared["expected_identity"]["worker_job_index"]
            .as_u64()
            .unwrap();
        complete(&app, job, "pass").await;
        let (jobs, _) = state
            .operational_binary
            .repository_worker_runtime(1)
            .unwrap();
        let original = jobs.get(job as u32).unwrap();
        let source = jobs.prepared_ci_runtime().source(original.key).unwrap();
        match case {
            "archive-missing" => fs::remove_file(source.package()).unwrap(),
            "archive-corrupt" => fs::write(source.package(), b"invalid archive bytes").unwrap(),
            "source-missing" => fs::remove_dir_all(source.sources()).unwrap(),
            _ => unreachable!(),
        }
        let (status, observed) =
            call(&app, "GET", &format!("{ROUTE}/worker-jobs/{job}"), None).await;
        assert_eq!(status, StatusCode::OK, "{observed}");
        let execution = &observed["package_execution"];
        assert!(execution.is_object(), "{case}: {observed}");
        assert!(execution.get("author_mode").unwrap().is_null(), "{case}");
        assert_eq!(execution["availability"], "unavailable");
        assert_eq!(execution["recoverable"], false);
        assert_eq!(execution["ci_state"], "passed");
        assert_eq!(jobs.get(job as u32).unwrap(), original);
        no_publication(&state);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sealed_mode_read_rejects_incomplete_native_binding_and_invalid_manifest_modes() {
    for case in [
        "base",
        "final",
        "target",
        "job-path",
        "hash-path",
        "repository",
        "mode-missing",
        "mode-null",
        "mode-unknown",
        "mode-number",
    ] {
        let root = TestDirectory::new("sealed-mode-read-untrusted");
        let (state, app, raw, hash) = fixture(root.path()).await;
        let (_, prepared) = upload(&app, &raw, &hash, None).await;
        let job = prepared["expected_identity"]["worker_job_index"]
            .as_u64()
            .unwrap() as u32;
        let mut sealed = manifest(&raw, &hash);
        match case {
            "repository" => sealed["repository_index"] = json!(2),
            "mode-missing" => {
                sealed.as_object_mut().unwrap().remove("author_mode");
            }
            "mode-null" => sealed["author_mode"] = JsonValue::Null,
            "mode-unknown" => sealed["author_mode"] = json!("unknown"),
            "mode-number" => sealed["author_mode"] = json!(7),
            _ => {}
        }
        let (retained, retained_hash) = repack(root.path(), &sealed);
        drop(app);
        drop(state);
        let (state, app) = reopen(root.path());
        let path = format!("{ROUTE}/worker-jobs/{job}");
        let (_, absent) = call(&app, "GET", &path, None).await;
        assert!(absent["package_execution"].is_null());
        // The old execution was invalidated by Server restart. Construct a new
        // native association to exercise the getter's independent binding checks.
        let job = job + 1;
        let managed_root = root.path().join("workflow-packages/generation");
        let worktree = managed_root
            .join("1")
            .join(if case == "hash-path" {
                "00".repeat(32)
            } else {
                retained_hash.clone()
            })
            .join(if case == "job-path" {
                "99".to_string()
            } else {
                job.to_string()
            });
        let sources = worktree.join("source");
        fs::create_dir_all(&sources).unwrap();
        let package = worktree.join("package.zst");
        fs::write(&package, retained).unwrap();
        let source = ManagedCiPackage::open(
            1,
            ManagedCiPackagePaths {
                managed_root,
                worktree,
                sources,
                package,
            },
            parse_package_hash(&retained_hash).unwrap(),
            PreparedCiTarget {
                target_line: if case == "target" { "other" } else { "main" }.into(),
                base_snapshot_id: if case == "base" { PEER } else { BASE }.into(),
                revision_snapshot_id: if case == "final" { PEER } else { REVISION }.into(),
            },
        )
        .unwrap();
        // Inject a retained native association whose bytes/hash are valid but
        // whose complete binding/mode is not. The read must validate both.
        source.verify().unwrap();
        let (jobs, _) = state
            .operational_binary
            .repository_worker_runtime(1)
            .unwrap();
        let injected = jobs
            .enqueue_prepared_ci(source, 1, chrono::Utc::now().timestamp() as u64)
            .unwrap();
        assert_eq!(injected.key.worker_job_index, job);
        let path = format!("{ROUTE}/worker-jobs/{job}");
        let before = jobs.list().unwrap();
        let (status, observed) = call(&app, "GET", &path, None).await;
        assert_eq!(status, StatusCode::OK, "{case}: {observed}");
        let execution = &observed["package_execution"];
        assert!(execution.is_object(), "{case}: {observed}");
        assert!(
            execution.get("author_mode").unwrap().is_null(),
            "{case}: {observed}"
        );
        assert_eq!(execution["availability"], "unavailable", "{case}");
        assert_eq!(execution["recoverable"], false, "{case}");
        assert_eq!(jobs.list().unwrap(), before);
        no_publication(&state);
    }
}
