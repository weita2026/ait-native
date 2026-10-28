use super::actor_plan::call;
use super::actor_registration::{register, request_for};
use super::*;
use ait_server_core::foundation::pack_substrate::{
    build_tree_pack_members, write_tree_pack_archive,
};
use ait_server_core::foundation::remote_binary_db::BinaryDbReadTxn;
use ait_server_core::foundation::workflow_binary_v0::WorkflowBinaryV0Codec;
use ait_server_core::foundation::workflow_package_contract::PACKAGE_CONTENT_TYPE;
use sha2::{Digest, Sha256};
use std::io::Write;

const BASE: &str = "SNP-000000000001";
const REVISION: &str = "SNP-000000000002";
const PACK: &str = "TPK-000000000001";
const TREE: &str = "TRE-00000000000000000001";
const ROUTE: &str = "/v1/native/repository-authorities/1";

#[path = "workflow_package_delta.rs"]
mod delta_batch;

fn tar_member(archive: &mut Vec<u8>, name: &str, bytes: &[u8]) {
    let mut header = [0; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    for (start, len, value) in [
        (100, 8, 0o644),
        (108, 8, 0),
        (116, 8, 0),
        (124, 12, bytes.len() as u64),
        (136, 12, 0),
    ] {
        let octal = format!("{value:0width$o}", width = len - 1);
        header[start..start + octal.len()].copy_from_slice(octal.as_bytes());
    }
    header[148..156].fill(b' ');
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum = header.iter().map(|byte| *byte as u64).sum::<u64>();
    let checksum = format!("{checksum:07o}");
    header[148..155].copy_from_slice(checksum.as_bytes());
    header[155] = 0;
    archive.extend_from_slice(&header);
    archive.extend_from_slice(bytes);
    archive.resize(archive.len() + (512 - bytes.len() % 512) % 512, 0);
}

fn snapshot(id: &str, parent: Option<&str>, time: u64) -> JsonValue {
    json!({"snapshot_id":id,"parent_snapshot_id":parent,"root_tree_pack_id":PACK,"root_entry_ordinal":0,"line_name":"main","file_count":0,"total_bytes":0,"manifest_hash":"00".repeat(32),
        "created_at":chrono::DateTime::from_timestamp(time as i64,0).unwrap().to_rfc3339()})
}

async fn fixture(root: &FsPath) -> (ServerState, Router, Vec<u8>, String) {
    let state = fresh_state(root);
    let app = build_router_with_state(state.clone());
    assert_eq!(
        register(&app, request_for(&state, 1)).await.1["actor_key_index"],
        0
    );
    let path = root.join("fixture-tree.zstpack");
    let members =
        build_tree_pack_members(&json!([{"tree_id":TREE,"entry_count":0}]), &json!([])).unwrap();
    let mut metadata = write_tree_pack_archive(
        path.to_str().unwrap(),
        PACK,
        "1970-01-01T00:01:40Z",
        &members,
    )
    .unwrap();
    metadata["pack_id"] = json!(PACK);
    let tree_bytes = fs::read(path).unwrap();
    state
        .repository_service
        .put_zstd_bulk_tree_pack("1", PACK, tree_bytes.clone())
        .unwrap();
    let locator = json!({"tree_id":TREE,"tree_pack_id":PACK,"entry_count":0,"tree_pack_checksum":metadata["pack_index"]["trees"][0]["checksum"],"created_at":"1970-01-01T00:01:40Z"});
    let baseline = json!({"actor_key_index":0,"contract":"ait.remote_sync.zstd_bulk.commit.v1","object_packs":[],"tree_packs":[metadata],"blob_locators":[],"tree_locators":[locator],"snapshots":[snapshot(BASE,None,100)]});
    let (status, body) = call(
        &app,
        "POST",
        &format!("{ROUTE}/remote-sync/zstd-bulk/commit"),
        Some(baseline),
    )
    .await;
    assert!(status.is_success(), "{body}");
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    ait_server_core::foundation::server_content_binary_db::ServerBinaryDbLineStore::<_, 1>::new(db)
        .set_line_head("main", 1, 100)
        .unwrap();
    let index = &metadata["pack_index"];
    // The released Core zstd-bulk typed codec emits this projection.
    let mut metadata = metadata.clone();
    metadata["pack_index"] = json!({"pack_id":PACK,"pack_format":index["pack_format"],"tree_count":1,"total_bytes":index["total_bytes"],
        "trees":[{"tree_id":TREE,"entry_ordinal":0,"entry_count":0,"checksum":index["trees"][0]["checksum"]}]});
    let manifest = json!({"contract":"workflow-package/v1","repository_index":1,"target_line":"main","base_snapshot_id":BASE,"revision_snapshot_id":REVISION,"author_mode":"ai_with_human_review","summary":"Ready summary",
        "history_entries":[{"local_task_id":"LSET-0001","expected_remote_task_id":null,"task":{"title":"Original source","intent":"Publish completed source"},"pre_land_target_snapshot_id":BASE,"landed_snapshot_id":REVISION,"landed_at_s":250,"changes":[],"snapshots":[{"snapshot_id":REVISION,"created_at_s":200,"local_change_ref":null}]}],
        "content":{"object_packs":[],"tree_packs":[metadata],"blob_locators":[],"tree_locators":[locator],"snapshots":[snapshot(REVISION,Some(BASE),200)],"line_update":null,"capture_binding":null}});
    let mut tar = Vec::new();
    tar_member(
        &mut tar,
        "manifest.json",
        &serde_json::to_vec(&manifest).unwrap(),
    );
    tar_member(&mut tar, &format!("trees/{PACK}"), &tree_bytes);
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
    (state, app, raw, hash)
}

async fn upload(app: &Router, raw: &[u8], hash: &str, job: Option<u32>) -> (StatusCode, JsonValue) {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("{ROUTE}/remote-sync/zstd-bulk/commit"))
        .header(CONTENT_TYPE, PACKAGE_CONTENT_TYPE)
        .header("x-ait-package-sha256", hash)
        .header("x-ait-actor-key-index", "0");
    if let Some(job) = job {
        request = request.header("x-ait-worker-job-index", job.to_string());
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(raw.to_vec())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = hyper::body::to_bytes(response.into_body())
        .await
        .unwrap_or_else(|error| panic!("Package prepare HTTP {status}: read body: {error}"));
    let body: JsonValue = serde_json::from_slice(&bytes).unwrap_or_else(|error| {
        panic!(
            "Package prepare HTTP {status}: invalid JSON ({error}); body={}",
            String::from_utf8_lossy(&bytes)
        )
    });
    if status.is_success() {
        ait_server_core::foundation::workflow_package_contract::PackageExpectedIdentity::parse(
            &body["expected_identity"],
        )
        .unwrap_or_else(|error| panic!("Package prepare HTTP {status}: {error}; body={body}"));
    } else {
        eprintln!(
            "Package prepare job={job:?}, retained={}: HTTP {status}; body={body}",
            raw.is_empty()
        );
    }
    (status, body)
}

async fn prepare_package(app: &Router, raw: &[u8], hash: &str) -> JsonValue {
    let (status, body) = upload(app, raw, hash, None).await;
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "Package prepare HTTP {status}; body={body}"
    );
    body
}

fn no_publication(state: &ServerState) {
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    for file in [
        WorkflowBinaryV0Codec::task_file(),
        WorkflowBinaryV0Codec::patchset_file(),
        WorkflowBinaryV0Codec::review_file(),
        WorkflowBinaryV0Codec::attest_file(),
        WorkflowBinaryV0Codec::land_file(),
    ] {
        assert_eq!(read.record_count(file).unwrap(), 0);
    }
    assert_eq!(read.record_count(ait_server_core::foundation::server_content_binary_db::ServerBinarySnapshotCodec::<2>::record_file()).unwrap(),1);
}

async fn complete(app: &Router, index: u64, tests: &str) {
    let path = format!("{ROUTE}/worker-jobs/{index}");
    let (status, claim) = call(
        app,
        "POST",
        &format!("{path}:claim"),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    assert!(status.is_success(), "{claim}");
    assert_eq!(
        claim["claimed_job"]["runtime_request"]["source"]["kind"],
        "workflow_package"
    );
    complete_claimed(app, index, tests, &claim["claimed_job"]).await;
}

async fn complete_claimed(app: &Router, index: u64, tests: &str, proof: &JsonValue) {
    let path = format!("{ROUTE}/worker-jobs/{index}");
    let (status,result)=call(app,"POST",&format!("{path}:complete"),Some(json!({"attempt_count":proof["attempt_count"],"lease_token":proof["lease_token"],"detail":{"job_kind":7,"result":{"contract":"ait.runner.native-result.v1","status":"succeeded","tests_status":tests,"suite_result_count":2,
        "suite_results":[{"suite_id":"rust_tests","status":tests,"blocking":true},{"suite_id":"cargo_fmt","status":"pass","blocking":true}],"cleanup":{}}}}))).await;
    assert!(status.is_success(), "{result}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_http_keeps_preparation_temporary_and_failed_tests_never_authorize_finish() {
    let root = TestDirectory::new("package-http");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let (first, joined) = tokio::join!(
        upload(&app, &raw, &hash, None),
        upload(&app, &raw, &hash, None)
    );
    assert_eq!(first.0, StatusCode::ACCEPTED, "{}", first.1);
    assert_eq!(joined.0, StatusCode::ACCEPTED, "{}", joined.1);
    assert_eq!(first.1["expected_identity"], joined.1["expected_identity"]);
    assert_ne!(first.1["reused"], joined.1["reused"]);
    no_publication(&state);
    let index = first.1["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    let path = format!("{ROUTE}/worker-jobs/{index}");
    complete(&app, index, "fail").await;
    let (_, failed) = call(&app, "GET", &path, None).await;
    assert_eq!(failed["package_execution"]["ci_state"], "failed");
    assert_eq!(failed["package_execution"]["recoverable"], false);
    let evidence = |identity: &JsonValue, id: &str, payload: JsonValue| json!({"contract":"workflow-package-evidence/v1","operation":"review","idempotency_key":id,"actor_key_index":0,"expected_identity":identity,"payload":payload});
    assert_eq!(
        call(
            &app,
            "POST",
            &path,
            Some(evidence(
                &first.1["expected_identity"],
                "premature",
                json!({"reviewer":"Alice","action":"approve"})
            ))
        )
        .await
        .0,
        StatusCode::CONFLICT
    );
    no_publication(&state);
    let (status, rerun) = upload(&app, &raw, &hash, None).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    let new = rerun["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    assert_ne!(new, index);
    complete(&app, new, "pass").await;
    let identity = &rerun["expected_identity"];
    let path = format!("{ROUTE}/worker-jobs/{new}");
    let (status, _) = call(
        &app,
        "POST",
        &path,
        Some(evidence(
            identity,
            "spoofed",
            json!({"reviewer":"Another human","action":"approve","reviewer_kind":"user"}),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, bot) = call(
        &app,
        "POST",
        &path,
        Some(evidence(
            identity,
            "bot",
            json!({"reviewer":"Codex","action":"approve","reviewer_kind":"bot"}),
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{bot}");
    let attest = json!({"contract":"workflow-package-evidence/v1","operation":"attestation","idempotency_key":"attest","actor_key_index":0,"expected_identity":identity,"payload":{"author_mode":"ai_with_human_review","verification_state":"pass","require_tests_pass":true,"require_human_review":true,"require_lint_pass":true}});
    assert_eq!(
        call(&app, "POST", &path, Some(attest)).await.0,
        StatusCode::OK
    );
    let finish = json!({"contract":"task-land-package-guarded/v1","idempotency_key":format!("workflow-package:{hash}:{new}"),"actor_key_index":0,"target_line":"main","mode":"direct","expected_identity":identity});
    let (status, blocked) = call(
        &app,
        "POST",
        &format!("{ROUTE}/task-land"),
        Some(finish.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{blocked}");
    no_publication(&state);
    let approval = evidence(
        identity,
        "human",
        json!({"reviewer":"Alice","action":"approve","reviewer_kind":"user"}),
    );
    let first_approval = call(&app, "POST", &path, Some(approval.clone())).await;
    assert_eq!(first_approval.0, StatusCode::OK);
    assert_eq!(
        call(&app, "POST", &path, Some(approval)).await.1["recorded_at_s"],
        first_approval.1["recorded_at_s"]
    );
    let dismissal = evidence(
        identity,
        "human-dismissed",
        json!({"reviewer":"Alice","action":"dismiss","reviewer_kind":"user"}),
    );
    assert_eq!(
        call(&app, "POST", &path, Some(dismissal)).await.0,
        StatusCode::OK
    );
    let (status, dismissed) = call(
        &app,
        "POST",
        &format!("{ROUTE}/task-land"),
        Some(finish.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{dismissed}");
    no_publication(&state);
    assert_eq!(
        call(
            &app,
            "POST",
            &path,
            Some(evidence(
                identity,
                "human-approved-again",
                json!({"reviewer":"Alice","action":"approve","reviewer_kind":"user"})
            ))
        )
        .await
        .0,
        StatusCode::OK
    );
    for action in ["code_review_summary", "task_approve"] {
        let (status, result) = call(
            &app,
            "POST",
            &path,
            Some(evidence(
                identity,
                action,
                json!({"reviewer":"Alice","reviewer_kind":"user","action":action}),
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result}");
    }
    let (status, result) = call(
        &app,
        "POST",
        &format!("{ROUTE}/task-land"),
        Some(finish.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "confirmed");
    assert_eq!(result["entries"].as_array().unwrap().len(), 1);
    let (status, replayed) = call(&app, "POST", &format!("{ROUTE}/task-land"), Some(finish)).await;
    assert_eq!(status, StatusCode::OK, "{replayed}");
    assert_eq!(replayed, result);
    let abandoned = json!({"contract":"workflow-package-abandon/v1","actor_key_index":0,"expected_identity":identity,"idempotency_key":format!("workflow-package-abandon:{hash}:{new}")});
    assert_eq!(
        call(&app, "POST", &path, Some(abandoned)).await.0,
        StatusCode::CONFLICT
    );
    let (_, snapshot) = call(&app, "GET", &format!("{ROUTE}/snapshots/{REVISION}"), None).await;
    assert_eq!(
        snapshot["workflow_owner"],
        json!({"task_id":result["entries"][0]["remote_task_id"],"change_ref":null})
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_http_abandonment_waits_for_exact_owned_worker_and_retries_without_new_job() {
    let root = TestDirectory::new("package-abandon");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let ready = prepare_package(&app, &raw, &hash).await;
    let first = ready["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    let abandon = |identity: &JsonValue| json!({"contract":"workflow-package-abandon/v1","actor_key_index":0,"expected_identity":identity,"idempotency_key":format!("workflow-package-abandon:{}:{}",identity["package_hash"].as_str().unwrap(),identity["worker_job_index"].as_u64().unwrap())});
    let first_path = format!("{ROUTE}/worker-jobs/{first}");
    let request = abandon(&ready["expected_identity"]);
    let (status, cancelled) = call(&app, "POST", &first_path, Some(request.clone())).await;
    assert_eq!(status, StatusCode::OK, "{cancelled}");
    assert_eq!(cancelled["state"], "abandoned");
    assert_eq!(
        call(&app, "POST", &first_path, Some(request)).await.1,
        cancelled
    );
    no_publication(&state);
    let ready = prepare_package(&app, &raw, &hash).await;
    let index = ready["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    assert_ne!(index, first);
    let path = format!("{ROUTE}/worker-jobs/{index}");
    let (status, claim) = call(
        &app,
        "POST",
        &format!("{path}:claim"),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let request = abandon(&ready["expected_identity"]);
    let (status, pending) = call(&app, "POST", &path, Some(request.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{pending}");
    assert_eq!(pending["state"], "abandoning");
    let proof = &claim["claimed_job"];
    let lease = json!({"attempt_count":proof["attempt_count"],"lease_token":proof["lease_token"]});
    let (status, heartbeat) = call(
        &app,
        "POST",
        &format!("{path}:heartbeat"),
        Some(lease.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{heartbeat}");
    assert_eq!(heartbeat["cancel_requested"], true);
    // Repeating abandonment cannot prove that an old Runner stopped.
    assert_eq!(
        call(&app, "POST", &path, Some(request.clone())).await.0,
        StatusCode::ACCEPTED
    );
    let (_, job) = call(&app, "GET", &path, None).await;
    assert_eq!(job["package_execution"]["availability"], "available");
    assert_eq!(job["package_execution"]["recoverable"], false);
    let mut release = lease;
    release["detail"] =
        json!({"job_kind":7,"retryable":true,"error":"cancelled after owned process cleanup"});
    let (status, released) = call(&app, "POST", &format!("{path}:fail"), Some(release)).await;
    assert_eq!(status, StatusCode::OK, "{released}");
    let (status, done) = call(&app, "POST", &path, Some(request.clone())).await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["state"], "abandoned");
    assert_eq!(call(&app, "POST", &path, Some(request)).await.1, done);
    no_publication(&state);
    let (_, job) = call(&app, "GET", &path, None).await;
    assert_eq!(job["package_execution"]["availability"], "unavailable");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_http_server_restart_invalidates_queued_and_running_jobs_before_fresh_ready() {
    let root = TestDirectory::new("package-restart");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let ready = prepare_package(&app, &raw, &hash).await;
    let old = ready["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    let (status, reused) = upload(&app, &raw, &hash, Some(old as u32)).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{reused}");
    assert_eq!(reused["expected_identity"], ready["expected_identity"]);
    assert_eq!(reused["reused"], true);
    drop(app);
    drop(state);
    let reopen = || {
        let runtime = Arc::new(
            OperationalBinaryRuntime::open_generation(
                root.path().join("generation"),
                root.path().join("runtime-worker-leases.bin"),
                60,
                15,
            )
            .unwrap(),
        );
        let state = state_from_operational(runtime);
        let app = build_router_with_state(state.clone());
        (state, app)
    };
    let (state, app) = reopen();
    let (status, stale) = upload(&app, &raw, &hash, Some(old as u32)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{stale}");
    let fresh = prepare_package(&app, &raw, &hash).await;
    let running = fresh["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    assert_ne!(running, old);
    let path = format!("{ROUTE}/worker-jobs/{running}");
    let (status, claim) = call(
        &app,
        "POST",
        &format!("{path}:claim"),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let proof = claim["claimed_job"].clone();
    let old_source = state
        .operational_binary
        .repository_worker_runtime(1)
        .unwrap()
        .0
        .prepared_ci_runtime()
        .source(
            ait_server_core::foundation::server_operational_worker_jobs::WorkerJobKey {
                repository_index: 1,
                worker_job_index: running as u32,
            },
        )
        .unwrap()
        .worktree()
        .to_path_buf();
    drop(app);
    drop(state);
    let (state, app) = reopen();
    let (status, heartbeat) = call(
        &app,
        "POST",
        &format!("{path}:heartbeat"),
        Some(json!({"attempt_count":proof["attempt_count"],"lease_token":proof["lease_token"]})),
    )
    .await;
    assert!(!status.is_success(), "{heartbeat}");
    let (status, late) = call(
        &app,
        "POST",
        &format!("{path}:complete"),
        Some(json!({
            "attempt_count":proof["attempt_count"],"lease_token":proof["lease_token"],
            "detail":{"job_kind":7,"result":{"contract":"ait.runner.native-result.v1",
            "status":"succeeded","tests_status":"pass","suite_result_count":2,
            "suite_results":[{"suite_id":"rust_tests","status":"pass","blocking":true},
                {"suite_id":"cargo_fmt","status":"pass","blocking":true}],"cleanup":{}}}
        })),
    )
    .await;
    assert!(!status.is_success(), "{late}");
    let before = state
        .operational_binary
        .repository_worker_runtime(1)
        .unwrap()
        .0
        .list()
        .unwrap();
    let (status, pending_stop) = upload(&app, &[], &hash, Some(running as u32)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{pending_stop}");
    assert_eq!(
        state
            .operational_binary
            .repository_worker_runtime(1)
            .unwrap()
            .0
            .list()
            .unwrap(),
        before
    );
    assert!(old_source.exists());
    // A second boot must not turn this boot's revoked lease into an owner-stop
    // report merely because its native row is now terminal.
    drop(app);
    drop(state);
    let (state, app) = reopen();
    let (status, still_unknown) = upload(&app, &[], &hash, Some(running as u32)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{still_unknown}");
    assert_eq!(
        state
            .operational_binary
            .repository_worker_runtime(1)
            .unwrap()
            .0
            .list()
            .unwrap(),
        before
    );
    assert!(old_source.exists());
    let (first, second) = tokio::join!(
        upload(&app, &raw, &hash, None),
        upload(&app, &raw, &hash, None)
    );
    assert_eq!(first.0, StatusCode::ACCEPTED, "{}", first.1);
    assert_eq!(second.0, StatusCode::ACCEPTED, "{}", second.1);
    assert_eq!(first.1["expected_identity"], second.1["expected_identity"]);
    assert_ne!(first.1["expected_identity"]["worker_job_index"], running);
    assert_eq!(first.1["expected_identity"]["package_hash"], hash);
    // An invalidated lease is not evidence that the remote process has stopped.
    assert!(old_source.exists());
    let (_, jobs) = call(&app, "GET", &format!("{ROUTE}/worker-jobs"), None).await;
    assert_eq!(jobs["count"], 3);
    no_publication(&state);
}

async fn download(app: &Router, index: u64, hash: &str, headers: &[(&str, String)]) -> Response {
    let mut request = Request::builder()
        .uri(format!("{ROUTE}/worker-jobs/{index}"))
        .header("accept", PACKAGE_CONTENT_TYPE)
        .header("x-ait-package-sha256", hash);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    app.clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_download_requires_the_exact_live_claim_and_strict_proof_headers() {
    let root = TestDirectory::new("package-download-owner");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let ready = prepare_package(&app, &raw, &hash).await;
    let index = ready["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    assert_eq!(
        download(&app, index, &hash, &[]).await.status(),
        StatusCode::BAD_REQUEST
    );
    let fake = [
        ("x-ait-worker-attempt-count", "1".into()),
        ("x-ait-worker-lease-token", "11".repeat(16)),
    ];
    assert_eq!(
        download(&app, index, &hash, &fake).await.status(),
        StatusCode::BAD_REQUEST
    );
    let (status, claim) = call(
        &app,
        "POST",
        &format!("{ROUTE}/worker-jobs/{index}:claim"),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claim}");
    let proof = [
        (
            "x-ait-worker-attempt-count",
            claim["claimed_job"]["attempt_count"].to_string(),
        ),
        (
            "x-ait-worker-lease-token",
            claim["claimed_job"]["lease_token"].as_str().unwrap().into(),
        ),
    ];
    for (name, malformed) in [
        ("x-ait-worker-attempt-count", "01"),
        ("x-ait-worker-attempt-count", "0"),
        ("x-ait-worker-attempt-count", "65536"),
        ("x-ait-worker-attempt-count", "2"),
        ("x-ait-worker-lease-token", "invalid"),
    ] {
        let mut bad = proof.clone();
        bad.iter_mut().find(|(key, _)| *key == name).unwrap().1 = malformed.into();
        assert_eq!(
            download(&app, index, &hash, &bad).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    for missing in 0..2 {
        let headers = proof
            .iter()
            .enumerate()
            .filter(|(position, _)| *position != missing)
            .map(|(_, header)| header.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            download(&app, index, &hash, &headers).await.status(),
            StatusCode::BAD_REQUEST
        );
        let mut duplicate = proof.to_vec();
        duplicate.push(proof[missing].clone());
        assert_eq!(
            download(&app, index, &hash, &duplicate).await.status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        download(&app, index, &hash, &fake).await.status(),
        StatusCode::BAD_REQUEST
    );
    let other_root = TestDirectory::new("other-runner-download-proof");
    let (_, other_app, other_raw, other_hash) = fixture(other_root.path()).await;
    let other_ready = prepare_package(&other_app, &other_raw, &other_hash).await;
    let (_, other_claim) = call(
        &other_app,
        "POST",
        &format!(
            "{ROUTE}/worker-jobs/{}:claim",
            other_ready["expected_identity"]["worker_job_index"]
        ),
        Some(json!({"accepted_runtime_contracts":["ait.runner.native-job.v3"]})),
    )
    .await;
    let mut foreign = proof.clone();
    foreign[1].1 = other_claim["claimed_job"]["lease_token"]
        .as_str()
        .unwrap()
        .into();
    assert_eq!(
        download(&app, index, &hash, &foreign).await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        download(&app, index, &"11".repeat(32), &proof)
            .await
            .status(),
        StatusCode::CONFLICT
    );
    let response = download(&app, index, &hash, &proof).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], PACKAGE_CONTENT_TYPE);
    assert_eq!(response.headers()["x-ait-package-sha256"], hash);
    assert_eq!(to_bytes(response.into_body()).await.unwrap(), raw);
    fs::remove_file(
        root.path()
            .join("workflow-packages/generation/1")
            .join(&hash)
            .join(index.to_string())
            .join("package.zst"),
    )
    .unwrap();
    assert_eq!(
        download(&app, index, &hash, &proof).await.status(),
        StatusCode::NOT_FOUND
    );
    no_publication(&state);
}

#[tokio::test]
async fn package_http_missing_source_rejects_old_pin_but_verified_reupload_uses_new_job_after_reset(
) {
    let root = TestDirectory::new("package-source-missing");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let ready = prepare_package(&app, &raw, &hash).await;
    let index = ready["expected_identity"]["worker_job_index"]
        .as_u64()
        .unwrap();
    fs::remove_file(
        root.path()
            .join("workflow-packages/generation/1")
            .join(&hash)
            .join(index.to_string())
            .join("package.zst"),
    )
    .unwrap();
    let (_, job) = call(&app, "GET", &format!("{ROUTE}/worker-jobs/{index}"), None).await;
    assert_eq!(job["package_execution"]["availability"], "unavailable");
    assert_eq!(job["package_execution"]["recoverable"], false);
    let (status, error) = upload(&app, &raw, &hash, Some(index as u32)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{error}");
    assert!(error.to_string().contains("fresh ready"));
    let (_, db) = state.operational_binary.repository_db(1).unwrap();
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(read.record_count(ait_server_core::foundation::operational_binary_v0::ServerOperationalBinaryV0Codec::worker_job_file()).unwrap(),1);
    no_publication(&state);
    drop(read);
    drop(db);
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
    let (status, corrupt) = upload(&app, b"corrupt", &hash, None).await;
    assert!(!status.is_success(), "{corrupt}");
    let (first, second) = tokio::join!(
        upload(&app, &raw, &hash, None),
        upload(&app, &raw, &hash, None)
    );
    assert_eq!(first.0, StatusCode::ACCEPTED, "{}", first.1);
    assert_eq!(second.0, StatusCode::ACCEPTED, "{}", second.1);
    assert_eq!(first.1["expected_identity"], second.1["expected_identity"]);
    assert_ne!(first.1["expected_identity"]["worker_job_index"], index);
    assert_ne!(first.1["ci_state"], "passed");
    let (_, jobs) = call(&app, "GET", &format!("{ROUTE}/worker-jobs"), None).await;
    assert_eq!(jobs["count"], 2);
    no_publication(&state);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn package_http_publication_rechecks_snapshot_identity_after_read_preparation() {
    use ait_server_core::foundation::native_repositories::BinaryDbNativeRepositoryService;
    use ait_server_core::foundation::remote_binary_db::{BinaryDbCommandScope, BinaryDbWriteTxn};
    use ait_server_core::foundation::workflow_package::WorkflowPackage;
    let root = TestDirectory::new("package-content-race");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let (entry, db) = state.operational_binary.repository_db(1).unwrap();
    let native = BinaryDbNativeRepositoryService::new(db.clone());
    let package = WorkflowPackage::unpack(
        &raw,
        &root.path().join("race-inputs"),
        1,
        &hash,
        NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES,
    )
    .unwrap();
    let prepared = native
        .prepare_workflow_package_content(&entry.repo_name, &package)
        .unwrap();
    let mut competing = snapshot(REVISION, Some(BASE), 200);
    competing["message"] = json!("different immutable provenance");
    let (status,result)=call(&app,"POST",&format!("{ROUTE}/remote-sync/zstd-bulk/commit"),Some(json!({"actor_key_index":0,"contract":"ait.remote_sync.zstd_bulk.commit.v1","object_packs":[],"tree_packs":[],"blob_locators":[],"tree_locators":[],"snapshots":[competing]}))).await;
    assert!(status.is_success(), "{result}");
    let mut tx = BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerLand).unwrap();
    let error = native
        .apply_workflow_package_content_in_write(&mut tx, &entry.repo_name, prepared)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("immutable content during publication"),
        "{error}"
    );
    drop(tx);
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .unwrap(),
        0
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::patchset_file())
            .unwrap(),
        0
    );
    drop(read);
    let (_, retained) = call(&app, "GET", &format!("{ROUTE}/snapshots/{REVISION}"), None).await;
    assert_eq!(retained["message"], "different immutable provenance");
    assert!(retained["workflow_owner"].is_null());
}

#[tokio::test]
async fn package_content_preserves_ordered_merge_parents_and_rejects_reassignment() {
    use ait_server_core::foundation::native_repositories::BinaryDbNativeRepositoryService;
    use ait_server_core::foundation::remote_binary_db::{BinaryDbCommandScope, BinaryDbWriteTxn};
    use ait_server_core::foundation::server_content_binary_db::ServerBinarySnapshotParentEdgeCodec;
    use ait_server_core::foundation::workflow_package::WorkflowPackage;
    const BRANCH: &str = "SNP-000000000003";
    const MERGE: &str = "SNP-000000000004";
    let root = TestDirectory::new("package-merge-parents");
    let (state, app, raw, hash) = fixture(root.path()).await;
    let (entry, db) = state.operational_binary.repository_db(1).unwrap();
    let native = BinaryDbNativeRepositoryService::new(db.clone());
    let original = WorkflowPackage::unpack(
        &raw,
        &root.path().join("original-inputs"),
        1,
        &hash,
        NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES,
    )
    .unwrap();
    let mut manifest = serde_json::to_value(&original.manifest).unwrap();
    let mut merge = snapshot(MERGE, Some(BRANCH), 202);
    merge["parent_snapshot_ids"] = json!([BRANCH, REVISION]);
    manifest["revision_snapshot_id"] = json!(MERGE);
    manifest["history_entries"][0]["landed_snapshot_id"] = json!(MERGE);
    manifest["history_entries"][0]["snapshots"] = json!([
        {"snapshot_id":REVISION,"created_at_s":200,"local_change_ref":null},
        {"snapshot_id":BRANCH,"created_at_s":201,"local_change_ref":null},
        {"snapshot_id":MERGE,"created_at_s":202,"local_change_ref":null}
    ]);
    manifest["content"]["snapshots"] = json!([
        snapshot(REVISION, Some(BASE), 200),
        snapshot(BRANCH, Some(BASE), 201),
        merge
    ]);
    let mut tar = Vec::new();
    tar_member(
        &mut tar,
        "manifest.json",
        &serde_json::to_vec(&manifest).unwrap(),
    );
    tar_member(
        &mut tar,
        &format!("trees/{PACK}"),
        &fs::read(original.inputs.join("trees").join(PACK)).unwrap(),
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
    let mut package = WorkflowPackage::unpack(
        &raw,
        &root.path().join("merge-inputs"),
        1,
        &hash,
        NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES,
    )
    .unwrap();
    for _ in 0..2 {
        let prepared = native
            .prepare_workflow_package_content(&entry.repo_name, &package)
            .unwrap();
        let mut tx =
            BinaryDbWriteTxn::begin_serving(&db, BinaryDbCommandScope::ServerLand).unwrap();
        native
            .apply_workflow_package_content_in_write(&mut tx, &entry.repo_name, prepared)
            .unwrap();
        tx.commit().unwrap();
    }
    let read = BinaryDbReadTxn::new(&db);
    assert_eq!(
        read.record_count(ServerBinarySnapshotParentEdgeCodec::<1>::record_file())
            .unwrap(),
        4
    );
    assert_eq!(
        read.record_count(WorkflowBinaryV0Codec::task_file())
            .unwrap(),
        0
    );
    drop(read);
    let (status, projected) = call(&app, "GET", &format!("{ROUTE}/snapshots/{MERGE}"), None).await;
    assert!(status.is_success(), "{projected}");
    assert_eq!(projected["parent_snapshot_id"], BRANCH);
    assert_eq!(projected["parent_snapshot_ids"], json!([BRANCH, REVISION]));
    assert!(projected["workflow_owner"].is_null());
    package.manifest.content.snapshots[2]["parent_snapshot_id"] = json!(REVISION);
    package.manifest.content.snapshots[2]["parent_snapshot_ids"] = json!([REVISION, BRANCH]);
    let error = native
        .prepare_workflow_package_content(&entry.repo_name, &package)
        .err()
        .expect("Existing parent order must be immutable");
    assert!(
        error.to_string().contains("immutable canonical content"),
        "{error}"
    );
}
