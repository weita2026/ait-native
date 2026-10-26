use super::*;
use ait_server_core::foundation::operational_binary_v0::WORKER_JOB_KIND_PATCHSET_CI;
use ait_server_core::foundation::server_operational_repository_registry::{
    RepositoryCreateSpec, ServerOperationalRepositoryRegistry,
};
use ait_server_core::foundation::server_repository_scope::ServerRepositoryScope;

#[tokio::test]
async fn runtime_exclusions_cover_read_mutation_discovery_and_worker_routes() {
    let root = TestDirectory::new("http-repository-exclusion");
    let mut config =
        ServerBinaryLifecycleConfig::from_server_data_root(root.0.join("server-data")).unwrap();
    crate::operational_binary_runtime::initialize_installed_runtime(&config).unwrap();
    let source = config.activation().unwrap().unwrap();
    ServerOperationalRepositoryRegistry::new(
        source.generation_root.join("global"),
        source.generation_root.join("repositories"),
    )
    .unwrap()
    .append_repository(&RepositoryCreateSpec {
        repo_name: "excluded-fixture".into(),
        namespace_ascii: *b"XE",
        policy_flags: 0x83,
        created_at_s: 100,
    })
    .unwrap();
    let opaque = source.generation_root.join("repositories/4/worker_job.bin");
    fs::write(&opaque, b"Must not be opened").unwrap();
    let registry_bytes = fs::read(source.generation_root.join("global/repository.bin")).unwrap();
    config.repository_scope = ServerRepositoryScope::parse(Some("4")).unwrap();
    let app = build_router(config);
    for (method, path) in [
        ("GET", "/v1/native/repository-authorities/4"),
        ("GET", "/v1/native/repository-authorities/4/tasks"),
        (
            "GET",
            "/v1/native/repository-authorities/4/read/queue-summary",
        ),
        ("GET", "/v1/native/repository-authorities/4/retirement"),
        ("POST", "/v1/native/repository-authorities/4/retirement"),
        (
            "POST",
            "/v1/native/repository-authorities/4/retirement/purge",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{method} {path}");
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/native/repository-authorities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body()).await.unwrap()).unwrap();
    assert_eq!(body["count"], 4);
    let response = app.oneshot(Request::builder().method("POST").uri("/v1/native/worker-jobs:claim")
        .header(CONTENT_TYPE, "application/json").body(Body::from(json!({
            "accepted_runtime_contracts":["ait.runner.native-job.v3"],"accepted_job_kinds":[WORKER_JOB_KIND_PATCHSET_CI]
        }).to_string())).unwrap()).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(fs::read(&opaque).unwrap(), b"Must not be opened");
    assert_eq!(
        fs::read(source.generation_root.join("global/repository.bin")).unwrap(),
        registry_bytes
    );
}
