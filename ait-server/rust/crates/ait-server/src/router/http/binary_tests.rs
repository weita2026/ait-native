use super::*;
use crate::fresh_generation::initialize_fresh_generation;
use crate::repository_retirement::REMOTE_AUTHORITY_FILE_MEDIA_TYPE;
use axum::body::Body;
use axum::http::Request;
use hyper::body::to_bytes;
use std::collections::BTreeMap;
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::{env, fs};
use tower::ServiceExt;

static TEST_DIRECTORY_SEQUENCE: AtomicU64 = AtomicU64::new(1);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new(label: &str) -> Self {
        let path = env::temp_dir().join(format!(
            "ait-server-{label}-{}-{}",
            std::process::id(),
            TEST_DIRECTORY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create isolated Binary server test directory");
        Self(path)
    }

    fn path(&self) -> &FsPath {
        &self.0
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        const MAX_REMOVE_ATTEMPTS: usize = 20;

        for attempt in 1..=MAX_REMOVE_ATTEMPTS {
            match fs::remove_dir_all(&self.0) {
                Ok(()) => return,
                Err(error)
                    if error.kind() == std::io::ErrorKind::DirectoryNotEmpty
                        && attempt < MAX_REMOVE_ATTEMPTS =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => {
                    panic!("remove isolated Binary server test directory: {error}");
                }
            }
        }
    }
}

fn state_from_operational(operational: Arc<OperationalBinaryRuntime>) -> ServerState {
    let binary =
        BinaryServingServices::new(operational.clone()).expect("construct Binary serving services");
    ServerState {
        service_endpoints: service_endpoints(),
        runtime_service: binary.runtime,
        workflow_service: binary.workflow,
        repository_service: binary.repository,
        operational_binary: operational,
        zstd_pack_upload_admission: Arc::new(tokio::sync::Semaphore::new(
            NATIVE_ZSTD_BULK_UPLOAD_CONCURRENCY,
        )),
    }
}

fn fresh_state(root: &FsPath) -> ServerState {
    let generation = root.join("generation");
    initialize_fresh_generation(&generation, 1_786_000_000)
        .expect("initialize frozen Binary v0 generation");
    let operational = Arc::new(
        OperationalBinaryRuntime::open_generation(
            generation,
            root.join("runtime-worker-leases.bin"),
            60,
            15,
        )
        .expect("open frozen Binary v0 generation"),
    );
    state_from_operational(operational)
}

#[test]
fn runtime_activation_removes_only_abandoned_zstd_pack_staging_files() {
    let directory = TestDirectory::new("abandoned-pack-activation");
    let generation = directory.path().join("generation");
    initialize_fresh_generation(&generation, 1_786_000_000)
        .expect("initialize frozen Binary v0 generation");
    let pack_directory = generation.join("repositories/0/.ait/objects/packs");
    fs::create_dir_all(&pack_directory).expect("Pack directory should create");
    let abandoned = pack_directory.join("PCK-000000000104.zstpack.upload-999-0.tmp");
    let unrelated = pack_directory.join("unrelated.tmp");
    fs::write(&abandoned, b"partial Pack").expect("abandoned Pack stage should write");
    fs::write(&unrelated, b"unrelated").expect("unrelated temp file should write");

    let runtime = OperationalBinaryRuntime::open_generation(
        generation,
        directory.path().join("runtime-worker-leases.bin"),
        60,
        15,
    )
    .expect("runtime activation should clean abandoned Pack staging");
    assert!(!abandoned.exists());
    assert!(unrelated.exists());
    drop(runtime);
}

async fn response_json(response: Response) -> JsonValue {
    let bytes = to_bytes(response.into_body())
        .await
        .expect("read Binary route response");
    serde_json::from_slice(&bytes).expect("Binary route response is JSON")
}

fn post_json(uri: &str, payload: JsonValue) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&payload).expect("encode Binary route request"),
        ))
        .expect("Binary route request")
}

fn chunked_body(bytes: Vec<u8>, chunk_bytes: usize) -> Body {
    let (mut sender, body) = Body::channel();
    tokio::spawn(async move {
        let mut offset = 0_usize;
        while offset < bytes.len() {
            let end = offset.saturating_add(chunk_bytes).min(bytes.len());
            let chunk = Bytes::copy_from_slice(&bytes[offset..end]);
            if sender.send_data(chunk).await.is_err() {
                return;
            }
            offset = end;
        }
    });
    body
}

fn put_pack(uri: &str, media_type: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header(CONTENT_TYPE, media_type)
        .body(body)
        .expect("zstd Pack route request")
}

fn uploaded_object_pack_bytes(pack_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "pack_id": pack_id,
        "pack_format": "ait-pack-v3-zstd-chunked",
        "index_entry_name": "zstd-chunked-object-index",
        "pack_index_checksum": "router-object-index-checksum",
        "member_count": 0,
        "total_bytes": 0,
        "entries": [],
    }))
    .expect("object Pack test body should encode")
}

fn uploaded_tree_pack_bytes(pack_id: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "pack_id": pack_id,
        "pack_format": "ait-tree-pack-v2-zstd-chunked",
        "index_entry_name": "zstd-chunked-tree-index",
        "pack_index_checksum": "router-tree-index-checksum",
        "tree_count": 0,
        "total_bytes": 0,
        "trees": [],
    }))
    .expect("tree Pack test body should encode")
}

#[tokio::test]
async fn fresh_install_serves_fixed_numeric_repository_authorities() {
    let directory = TestDirectory::new("fresh-router");
    let router = build_router_with_state(fresh_state(directory.path()));

    let health = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("health request"),
        )
        .await
        .expect("health response");
    assert_eq!(health.status(), StatusCode::OK);
    let health = response_json(health).await;
    assert_eq!(health["ready"], true);
    assert_eq!(health["authority_backend"], "binary_v0");
    assert_eq!(health["repository_identity"], "repository_index");
    assert_eq!(
        health["operational_capabilities"]["runner_contracts"],
        json!(["ait.runner.native-job.v3"])
    );

    let retired_task_graph_probe = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/0/sprints/PR-0/revisions/plan-revision:0/task-graph-artifact")
                .body(Body::empty())
                .expect("retired task-graph probe request"),
        )
        .await
        .expect("retired task-graph probe response");
    assert_eq!(retired_task_graph_probe.status(), StatusCode::NOT_FOUND);

    let handshake = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/handshake")
                .body(Body::empty())
                .expect("handshake request"),
        )
        .await
        .expect("handshake response");
    assert_eq!(handshake.status(), StatusCode::OK);
    let handshake = response_json(handshake).await;
    assert_eq!(handshake["package_version"], json!("1.0.1"));
    assert_eq!(
        handshake["operational_capabilities"]["runner_contracts"],
        json!(["ait.runner.native-job.v3"])
    );
    assert_eq!(
        handshake["ci_capabilities"]["native_runner"]["contract"],
        json!("ait.runner.native-job.v3")
    );
    assert_eq!(
        handshake["ci_capabilities"]["native_runner"]["repository_entrypoint"],
        json!("ci/run")
    );
    assert_eq!(
        handshake["ci_capabilities"]["native_runner"]["platform_entrypoints"],
        json!({
            "darwin": "ci/run.sh",
            "linux": "ci/run.sh",
            "windows": "ci/run.ps1",
        })
    );
    assert_eq!(
        handshake["ci_capabilities"]["native_runner"]["command_execution"],
        json!("direct_argv_without_command_string_concatenation")
    );
    assert_eq!(
        handshake["supported_async_job_types"],
        json!([
            "content.gc",
            "content.optimize",
            "content.pack",
            "land.process",
            "main-seed.refresh",
            "patchset.ci",
            "patchset.ci.aggregate",
            "policy.evaluate",
            "reconcile.repo",
            "repo.ci",
        ])
    );
    assert!(!handshake["supported_async_job_types"]
        .as_array()
        .expect("supported Job kinds")
        .iter()
        .any(|kind| kind == "agent.turn.submit"));

    let repositories = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities")
                .body(Body::empty())
                .expect("Repository authority request"),
        )
        .await
        .expect("Repository authority response");
    assert_eq!(repositories.status(), StatusCode::OK);
    let repositories = response_json(repositories).await;
    assert_eq!(repositories["count"], 4);
    assert_eq!(
        repositories["repositories"]
            .as_array()
            .expect("Repository array")
            .iter()
            .map(|repository| {
                (
                    repository["repository_index"]
                        .as_u64()
                        .expect("Repository index"),
                    repository["repository_name"]
                        .as_str()
                        .expect("Repository name"),
                    repository["namespace"]
                        .as_str()
                        .expect("Repository namespace"),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (0, "ait-core", "C"),
            (1, "ait-server", "SE"),
            (2, "ait-python", "P"),
            (3, "ait-node", "N"),
        ]
    );

    let repository = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/1")
                .body(Body::empty())
                .expect("numeric Repository authority request"),
        )
        .await
        .expect("numeric Repository authority response");
    assert_eq!(repository.status(), StatusCode::OK);
    let repository = response_json(repository).await;
    assert_eq!(repository["repository"]["repository_index"], 1);
    assert_eq!(repository["repository"]["repository_name"], "ait-server");
    assert_eq!(
        repository["pack_storage"]["contract"],
        "ait.repository.pack_storage.v1"
    );
    assert_eq!(repository["pack_storage"]["object_pack_count"], 0);
    assert_eq!(repository["pack_storage"]["tree_pack_count"], 0);
    assert_eq!(repository["pack_storage"]["zstd_only_verified"], true);
    assert_eq!(repository["pack_storage"]["validation"]["state"], "valid");

    let default_line = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/1/lines/main")
                .body(Body::empty())
                .expect("fresh Repository default-Line request"),
        )
        .await
        .expect("fresh Repository default-Line response");
    assert_eq!(default_line.status(), StatusCode::OK);
    let default_line = response_json(default_line).await;
    assert_eq!(default_line["line_name"], "main");
    assert_eq!(default_line["status"], "active");
    assert!(default_line["head_snapshot_id"].is_null());

    let snapshot_existence = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/snapshots:exists",
            json!({"snapshot_ids": ["SNP-000000000000"]}),
        ))
        .await
        .expect("numeric Snapshot existence response");
    let snapshot_existence_status = snapshot_existence.status();
    let snapshot_existence = response_json(snapshot_existence).await;
    assert_eq!(
        snapshot_existence_status,
        StatusCode::OK,
        "{snapshot_existence}"
    );
    assert_eq!(snapshot_existence["repository_index"], 1);
    assert_eq!(snapshot_existence["missing"], json!(["SNP-000000000000"]));

    for path in [
        "/v1/native/repository-authorities/ait-server",
        "/v1/native/repository-authorities/01",
    ] {
        let rejected = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("non-numeric Repository request"),
            )
            .await
            .expect("non-numeric Repository response");
        assert_eq!(rejected.status(), StatusCode::BAD_REQUEST, "{path}");
    }

    let retired_name_route = router
        .oneshot(
            Request::builder()
                .uri("/v1/native/repositories/ait-server")
                .body(Body::empty())
                .expect("retired name route request"),
        )
        .await
        .expect("retired name route response");
    assert_eq!(retired_name_route.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn repository_registration_is_numeric_live_and_namespace_idempotent() {
    let directory = TestDirectory::new("repository-registration");
    let router = build_router_with_state(fresh_state(directory.path()));
    let registration = json!({
        "repository_name": "ait-runner",
        "namespace": "R",
        "policy_flags": 0b1000_0011,
    });

    let created = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities",
            registration.clone(),
        ))
        .await
        .expect("Repository registration response");
    assert_eq!(created.status(), StatusCode::CREATED);
    let created = response_json(created).await;
    assert_eq!(created["contract"], "ait.server.repository-registration.v1");
    assert_eq!(created["created"], true);
    assert_eq!(created["repository"]["repository_index"], 4);
    assert_eq!(created["repository"]["namespace"], "R");

    let default_line = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/4/lines/main")
                .body(Body::empty())
                .expect("registered Repository default-Line request"),
        )
        .await
        .expect("registered Repository default-Line response");
    assert_eq!(default_line.status(), StatusCode::OK);
    let default_line = response_json(default_line).await;
    assert_eq!(default_line["line_name"], "main");
    assert_eq!(default_line["status"], "active");
    assert!(default_line["head_snapshot_id"].is_null());

    let repeated = router
        .clone()
        .oneshot(post_json("/v1/native/repository-authorities", registration))
        .await
        .expect("idempotent Repository registration response");
    assert_eq!(repeated.status(), StatusCode::OK);
    let repeated = response_json(repeated).await;
    assert_eq!(repeated["created"], false);
    assert_eq!(repeated["repository"]["repository_index"], 4);

    let lines = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/4/lines")
                .body(Body::empty())
                .expect("idempotent Repository Line-list request"),
        )
        .await
        .expect("idempotent Repository Line-list response");
    assert_eq!(lines.status(), StatusCode::OK);
    let lines = response_json(lines).await;
    let lines = lines.as_array().expect("Repository Line array");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["line_name"], "main");
    assert!(lines[0]["head_snapshot_id"].is_null());

    let conflict = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities",
            json!({
                "repository_name": "different",
                "namespace": "R",
                "policy_flags": 0b1000_0011,
            }),
        ))
        .await
        .expect("Repository namespace conflict response");
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    for namespace in ["R!", "RUN", "測"] {
        let malformed = router
            .clone()
            .oneshot(post_json(
                "/v1/native/repository-authorities",
                json!({
                    "repository_name": "invalid",
                    "namespace": namespace,
                    "policy_flags": 0b1000_0011,
                }),
            ))
            .await
            .expect("malformed Repository namespace response");
        assert_eq!(malformed.status(), StatusCode::BAD_REQUEST, "{namespace}");
    }

    let duplicate_name = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities",
            json!({
                "repository_name": "ait-runner",
                "namespace": "R2",
                "policy_flags": 0b1000_0011,
            }),
        ))
        .await
        .expect("duplicate Repository discovery name response");
    assert_eq!(duplicate_name.status(), StatusCode::CREATED);
    let duplicate_name = response_json(duplicate_name).await;
    assert_eq!(duplicate_name["repository"]["repository_index"], 5);

    let repository = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/4")
                .body(Body::empty())
                .expect("live numeric Repository request"),
        )
        .await
        .expect("live numeric Repository response");
    assert_eq!(repository.status(), StatusCode::OK);

    let plans = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/4/sprints")
                .body(Body::empty())
                .expect("live numeric Repository Plan request"),
        )
        .await
        .expect("live numeric Repository Plan response");
    assert_eq!(plans.status(), StatusCode::OK);

    let discovered = router
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities?repository_name=ait-runner")
                .body(Body::empty())
                .expect("Repository discovery request"),
        )
        .await
        .expect("Repository discovery response");
    assert_eq!(discovered.status(), StatusCode::OK);
    let discovered = response_json(discovered).await;
    assert_eq!(discovered["count"], 2);
    assert_eq!(
        discovered["repositories"]
            .as_array()
            .expect("Repository discovery rows")
            .iter()
            .map(|repository| repository["repository_index"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![4, 5]
    );
}

#[tokio::test]
async fn retirement_abort_route_rolls_back_and_readmits_repository_mutation() {
    let directory = TestDirectory::new("retirement-abort-route");
    let router = build_router_with_state(fresh_state(directory.path()));

    let retirement = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/retirement",
            json!({}),
        ))
        .await
        .expect("Repository retirement response");
    assert_eq!(retirement.status(), StatusCode::OK);

    let blocked = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/snapshots:exists",
            json!({"snapshot_ids": ["SNP-000000000000"]}),
        ))
        .await
        .expect("retiring Repository mutation response");
    assert_eq!(blocked.status(), StatusCode::CONFLICT);

    let aborted = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/retirement/abort",
            json!({}),
        ))
        .await
        .expect("Repository retirement abort response");
    assert_eq!(aborted.status(), StatusCode::OK);
    let aborted = response_json(aborted).await;
    assert_eq!(
        aborted["contract"],
        "ait.server.repository-retirement-abort.v1"
    );
    assert_eq!(aborted["aborted"], true);
    assert_eq!(aborted["already_aborted"], false);
    assert_eq!(aborted["repository"]["lifecycle_kind"], 1);

    let repeated = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/retirement/abort",
            json!({}),
        ))
        .await
        .expect("repeated Repository retirement abort response");
    assert_eq!(repeated.status(), StatusCode::OK);
    assert_eq!(response_json(repeated).await["already_aborted"], true);

    let readmitted = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/snapshots:exists",
            json!({"snapshot_ids": ["SNP-000000000000"]}),
        ))
        .await
        .expect("aborted Repository mutation response");
    assert_eq!(readmitted.status(), StatusCode::OK);

    let retirement = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/retirement",
            json!({}),
        ))
        .await
        .expect("second Repository retirement response");
    let manifest = response_json(retirement).await["manifest"].clone();
    let purged = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/retirement/purge",
            manifest,
        ))
        .await
        .expect("Repository purge response");
    assert_eq!(purged.status(), StatusCode::OK);

    let abort_after_commit = router
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/retirement/abort",
            json!({}),
        ))
        .await
        .expect("post-purge Repository retirement abort response");
    assert_eq!(abort_after_commit.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn retirement_transfer_routes_block_mutation_and_restore_under_a_new_index() {
    let directory = TestDirectory::new("retirement-transfer-routes");
    let router = build_router_with_state(fresh_state(directory.path()));

    let retirement = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/retirement",
            json!({}),
        ))
        .await
        .expect("Repository retirement response");
    assert_eq!(retirement.status(), StatusCode::OK);
    let retirement = response_json(retirement).await;
    assert_eq!(retirement["ready_for_export"], true);
    let manifest = retirement["manifest"].clone();

    let blocked = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/tasks",
            json!({
                "title": "must not be admitted",
                "intent": "retiring Repository rejects mutation",
            }),
        ))
        .await
        .expect("retiring Repository mutation response");
    assert_eq!(blocked.status(), StatusCode::CONFLICT);

    let mut exported = BTreeMap::new();
    for file in manifest["files"].as_array().expect("export manifest files") {
        let path = file["path"].as_str().expect("manifest file path");
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/native/repository-authorities/1/retirement/files/{path}"
                    ))
                    .body(Body::empty())
                    .expect("retirement file request"),
            )
            .await
            .expect("retirement file response");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert_eq!(
            response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some(REMOTE_AUTHORITY_FILE_MEDIA_TYPE)
        );
        exported.insert(
            path.to_string(),
            to_bytes(response.into_body())
                .await
                .expect("read authority file"),
        );
    }

    let purged = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/1/retirement/purge",
            manifest.clone(),
        ))
        .await
        .expect("Repository purge response");
    assert_eq!(purged.status(), StatusCode::OK);
    let purged = response_json(purged).await;
    assert_eq!(purged["purged"], true);
    assert_eq!(purged["repository"]["repository_index"], 1);

    let disposable = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-restores",
            json!({
                "manifest": manifest,
                "policy_flags": 0b1000_0011,
            }),
        ))
        .await
        .expect("disposable Repository restore session response");
    assert_eq!(disposable.status(), StatusCode::CREATED);
    let disposable = response_json(disposable).await;
    let disposable_token = disposable["restore_token"]
        .as_str()
        .expect("disposable restore token");
    for expected_state in ["aborted", "absent"] {
        let aborted = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/v1/native/repository-restores/{disposable_token}/abort"
                    ))
                    .body(Body::empty())
                    .expect("restore abort request"),
            )
            .await
            .expect("restore abort response");
        assert_eq!(aborted.status(), StatusCode::OK);
        assert_eq!(response_json(aborted).await["state"], expected_state);
    }

    let oversized = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-restores",
            json!({
                "manifest": manifest,
                "policy_flags": 0b1000_0011,
            }),
        ))
        .await
        .expect("oversized-upload Repository restore session response");
    let oversized = response_json(oversized).await;
    let oversized_token = oversized["restore_token"]
        .as_str()
        .expect("oversized-upload restore token");
    let rejected = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!(
                    "/v1/native/repository-restores/{oversized_token}/files/worker_job.bin"
                ))
                .header(CONTENT_TYPE, REMOTE_AUTHORITY_FILE_MEDIA_TYPE)
                .header(
                    "content-length",
                    (NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES as u64 + 1).to_string(),
                )
                .body(Body::empty())
                .expect("oversized restore upload request"),
        )
        .await
        .expect("oversized restore upload response");
    assert_eq!(rejected.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let oversized_abort = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/native/repository-restores/{oversized_token}/abort"
                ))
                .body(Body::empty())
                .expect("oversized restore abort inspection"),
        )
        .await
        .expect("oversized restore abort inspection response");
    assert_eq!(response_json(oversized_abort).await["state"], "absent");

    let session = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-restores",
            json!({
                "manifest": manifest,
                "policy_flags": 0b1000_0011,
            }),
        ))
        .await
        .expect("Repository restore session response");
    assert_eq!(session.status(), StatusCode::CREATED);
    let session = response_json(session).await;
    let token = session["restore_token"]
        .as_str()
        .expect("restore token")
        .to_string();
    for (path, bytes) in exported {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/v1/native/repository-restores/{token}/files/{path}"
                    ))
                    .header(CONTENT_TYPE, REMOTE_AUTHORITY_FILE_MEDIA_TYPE)
                    .body(Body::from(bytes))
                    .expect("restore file upload request"),
            )
            .await
            .expect("restore file upload response");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
    }
    let restored = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/v1/native/repository-restores/{token}/commit"))
                .body(Body::empty())
                .expect("restore commit request"),
        )
        .await
        .expect("restore commit response");
    assert_eq!(restored.status(), StatusCode::OK);
    let restored = response_json(restored).await;
    assert_eq!(restored["repository"]["repository_index"], 4);
    assert_eq!(restored["repository"]["repository_name"], "ait-server");
}

#[tokio::test]
async fn purged_repository_is_rejected_by_cached_services_and_skipped_after_restart() {
    let directory = TestDirectory::new("purged-repository-restart");
    let generation = directory.path().join("generation");
    let lease_replica = directory.path().join("runtime-worker-leases.bin");
    let state = fresh_state(directory.path());
    let operational = state.operational_binary.clone();
    let router = build_router_with_state(state);

    let retirement = operational
        .begin_repository_retirement(1)
        .expect("begin Repository retirement");
    BinaryServingServices::new(operational.clone())
        .expect("retiring Repository remains eligible for serving");
    let purged = operational
        .purge_retired_repository(1, &retirement["manifest"])
        .expect("purge acknowledged Repository");
    assert_eq!(purged["repository"]["lifecycle_kind"], 3);
    assert_eq!(operational.repository_indexes(), vec![0, 1, 2, 3]);
    assert_eq!(operational.serving_repository_indexes(), vec![0, 2, 3]);

    let cached_read = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/1/read/queue-summary")
                .body(Body::empty())
                .expect("purged Repository queue read request"),
        )
        .await
        .expect("purged Repository queue read response");
    assert_ne!(cached_read.status(), StatusCode::OK);
    drop(router);
    drop(operational);

    let reopened = Arc::new(
        OperationalBinaryRuntime::open_generation(generation, lease_replica, 60, 15)
            .expect("reopen generation containing purged Repository history"),
    );
    assert_eq!(reopened.repository_indexes(), vec![0, 1, 2, 3]);
    assert_eq!(reopened.serving_repository_indexes(), vec![0, 2, 3]);
    let restarted = build_router_with_state(state_from_operational(reopened));
    let health = restarted
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .expect("post-purge restart health request"),
        )
        .await
        .expect("post-purge restart health response");
    assert_eq!(health.status(), StatusCode::OK);
}

#[tokio::test]
async fn numeric_repository_queue_routes_serve_the_existing_binary_projection() {
    let directory = TestDirectory::new("numeric-queue-routes");
    let router = build_router_with_state(fresh_state(directory.path()));

    let cold = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/1/read/queue-summary?status=active")
                .body(Body::empty())
                .expect("cold queue-summary request"),
        )
        .await
        .expect("cold queue-summary response");
    assert_eq!(cold.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        cold.headers().get(RETRY_AFTER),
        Some(&HeaderValue::from_static("1"))
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let summary = loop {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/native/repository-authorities/1/read/queue-summary?status=active")
                    .body(Body::empty())
                    .expect("queue-summary retry request"),
            )
            .await
            .expect("queue-summary retry response");
        if response.status() == StatusCode::OK {
            break response_json(response).await;
        }
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(
            std::time::Instant::now() < deadline,
            "Binary queue projection did not warm before the deadline"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    assert_eq!(summary["task_queue"]["count"], 0);
    assert_eq!(summary["reviewer_inbox"]["count"], 0);
    assert_eq!(summary["task_queue"]["filters"]["repo_name"], "ait-server");

    for (path, expected_filter) in [
        (
            "/v1/native/repository-authorities/1/read/task-queue?status=active",
            "ait-server",
        ),
        (
            "/v1/native/repository-authorities/1/read/reviewer-inbox",
            "ait-server",
        ),
    ] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("numeric queue projection request"),
            )
            .await
            .expect("numeric queue projection response");
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        let payload = response_json(response).await;
        assert_eq!(payload["filters"]["repo_name"], expected_filter, "{path}");
    }

    let non_canonical = router
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities/01/read/task-queue")
                .body(Body::empty())
                .expect("non-canonical queue route request"),
        )
        .await
        .expect("non-canonical queue route response");
    assert_eq!(non_canonical.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn numeric_repository_route_owns_plan_linked_task_identity() {
    let directory = TestDirectory::new("numeric-plan-task");
    let router = build_router_with_state(fresh_state(directory.path()));

    let plan = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/0/sprints",
            json!({
                "title": "Numeric Repository Plan",
                "status": "draft",
                "summary": "Route authority is the numeric Repository PK",
                "artifact_path": "docs/sprints/numeric.md",
                "artifact_heading": "Numeric",
                "items": [{
                    "plan_item_ref": "NUMERIC-PLAN-TASK",
                    "text": "Create a scoped Task",
                    "checkbox_state": "open",
                    "heading_path": ["Numeric"],
                    "line_number": 1,
                }],
                "actor_identity": "tester",
                "actor_type": "human",
            }),
        ))
        .await
        .expect("numeric Plan create response");
    assert_eq!(plan.status(), StatusCode::OK);
    let plan = response_json(plan).await;
    assert_eq!(plan["plan_id"], "PR-0");
    assert_eq!(plan["repo_name"], "ait-core");

    let task = router
        .clone()
        .oneshot(post_json(
            "/v1/native/repository-authorities/0/tasks",
            json!({
                "title": "Numeric Repository Task",
                "intent": "Prove route-scoped Plan linkage",
                "plan_id": "PR-0",
                "origin_plan_revision_id": "plan-revision:0",
                "plan_item_ref": "NUMERIC-PLAN-TASK",
            }),
        ))
        .await
        .expect("numeric Task create response");
    assert_eq!(task.status(), StatusCode::OK);
    let task = response_json(task).await;
    assert_eq!(task["task_id"], "RCT-0001");
    assert_eq!(task["repo_name"], "ait-core");
    assert_eq!(task["plan_id"], "PR-0");

    let conflicting_body_identity = router
        .oneshot(post_json(
            "/v1/native/repository-authorities/0/tasks",
            json!({
                "title": "Conflicting identity",
                "intent": "Must fail closed",
                "repo_id": "1",
            }),
        ))
        .await
        .expect("retired body identity response");
    assert_eq!(conflicting_body_identity.status(), StatusCode::BAD_REQUEST);
    let error = response_json(conflicting_body_identity).await;
    assert!(error["error"]
        .as_str()
        .expect("error text")
        .contains("Repository authority comes from the numeric route"));
}

#[tokio::test]
async fn zstd_pack_routes_stream_chunked_bodies_and_preserve_exact_bytes() {
    let directory = TestDirectory::new("streaming-pack-routes");
    let router = build_router_with_state(fresh_state(directory.path()));

    let object_pack_id = "PCK-000000000101";
    let object_uri = format!(
        "/v1/native/repository-authorities/0/remote-sync/zstd-bulk/object-packs/{object_pack_id}"
    );
    let object_bytes = uploaded_object_pack_bytes(object_pack_id);
    let response = router
        .clone()
        .oneshot(put_pack(
            &object_uri,
            ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE,
            chunked_body(object_bytes.clone(), 7),
        ))
        .await
        .expect("chunked object Pack upload response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["status"], "uploaded");
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(&object_uri)
                .body(Body::empty())
                .expect("object Pack download request"),
        )
        .await
        .expect("object Pack download response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body())
            .await
            .expect("object Pack response body"),
        object_bytes
    );

    let tree_pack_id = "TPK-000000000102";
    let tree_uri = format!(
        "/v1/native/repository-authorities/0/remote-sync/zstd-bulk/tree-packs/{tree_pack_id}"
    );
    let tree_bytes = uploaded_tree_pack_bytes(tree_pack_id);
    let response = router
        .clone()
        .oneshot(put_pack(
            &tree_uri,
            ZSTD_BULK_TREE_PACK_MEDIA_TYPE,
            chunked_body(tree_bytes.clone(), 11),
        ))
        .await
        .expect("chunked tree Pack upload response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["status"], "uploaded");
    let response = router
        .oneshot(
            Request::builder()
                .uri(&tree_uri)
                .body(Body::empty())
                .expect("tree Pack download request"),
        )
        .await
        .expect("tree Pack download response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body())
            .await
            .expect("tree Pack response body"),
        tree_bytes
    );
}

#[tokio::test]
async fn zstd_pack_route_streams_beyond_axum_default_body_limit() {
    let directory = TestDirectory::new("streaming-pack-default-limit");
    let router = build_router_with_state(fresh_state(directory.path()));
    let pack_id = "PCK-000000000105";
    let uri =
        format!("/v1/native/repository-authorities/0/remote-sync/zstd-bulk/object-packs/{pack_id}");
    let mut payload: JsonValue = serde_json::from_slice(&uploaded_object_pack_bytes(pack_id))
        .expect("object Pack test payload should parse");
    payload["streaming_padding"] = JsonValue::String("x".repeat(3 * 1024 * 1024));
    let payload = serde_json::to_vec(&payload).expect("large object Pack test body should encode");
    assert!(payload.len() > 2 * 1024 * 1024);

    let response = router
        .oneshot(put_pack(
            &uri,
            ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE,
            chunked_body(payload, 64 * 1024),
        ))
        .await
        .expect("large chunked object Pack upload response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response_json(response).await["status"], "uploaded");
}

#[tokio::test]
async fn zstd_pack_routes_reject_empty_declared_oversize_and_exhausted_admission() {
    let directory = TestDirectory::new("streaming-pack-rejections");
    let state = fresh_state(directory.path());
    let admission = state.zstd_pack_upload_admission.clone();
    let router = build_router_with_state(state);
    let uri =
        "/v1/native/repository-authorities/0/remote-sync/zstd-bulk/object-packs/PCK-000000000103";

    let empty = router
        .clone()
        .oneshot(put_pack(
            uri,
            ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE,
            Body::empty(),
        ))
        .await
        .expect("empty object Pack response");
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);

    let declared_oversize = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(uri)
                .header(CONTENT_TYPE, ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE)
                .header(
                    CONTENT_LENGTH,
                    (NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES as u64 + 1).to_string(),
                )
                .body(Body::empty())
                .expect("declared oversized Pack request"),
        )
        .await
        .expect("declared oversized Pack response");
    assert_eq!(declared_oversize.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let permits = admission.available_permits();
    let held_permits = admission
        .clone()
        .acquire_many_owned(permits as u32)
        .await
        .expect("test should reserve every Pack upload permit");
    let unavailable = router
        .oneshot(put_pack(
            uri,
            ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE,
            Body::from(uploaded_object_pack_bytes("PCK-000000000103")),
        ))
        .await
        .expect("exhausted Pack admission response");
    assert_eq!(unavailable.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        unavailable
            .headers()
            .get(RETRY_AFTER)
            .and_then(|value| value.to_str().ok()),
        Some("1")
    );
    drop(held_permits);
}

#[tokio::test]
async fn zstd_pack_chunk_stream_enforces_limit_before_writing_overflow_chunk() {
    let directory = TestDirectory::new("streaming-pack-chunk-limit");
    let path = directory.path().join("staged-pack.tmp");
    let mut file = tokio::fs::File::create(&path)
        .await
        .expect("test staging file should create");
    let mut body = chunked_body(b"abcd".to_vec(), 3);

    let response = stream_zstd_pack_body(&mut body, &mut file, 3, "object-pack")
        .await
        .expect_err("overflowing chunk stream must fail")
        .into_response();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    file.sync_all()
        .await
        .expect("test staging file should sync");
    drop(file);
    assert_eq!(
        fs::read(path).expect("bounded staging bytes should read"),
        b"abc"
    );
}
