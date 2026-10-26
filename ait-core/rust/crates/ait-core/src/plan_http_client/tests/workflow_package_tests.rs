use super::*;
use crate::workflow_expected_identity::PackageWorkflowExpectedIdentity;
use crate::workflow_package::{
    EncodedWorkflowPackage, WorkflowPackageManifest, WORKFLOW_PACKAGE_MEDIA_TYPE,
};
use sha2::{Digest, Sha256};

fn identity() -> PackageWorkflowExpectedIdentity {
    PackageWorkflowExpectedIdentity {
        repository_index: 0,
        worker_job_index: 0,
        package_hash: "ab".repeat(32),
        base_snapshot_id: "SNP-000000000001".into(),
        revision_snapshot_id: "SNP-000000000002".into(),
    }
}
fn cfg() -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        repository_index: Some(crate::server_operational::RepositoryIndex::new(0)),
        ..config()
    }
}

#[test]
fn package_finish_uses_bulk_deadline_without_changing_ordinary_requests() {
    let identity = identity();
    for default_timeout_ms in [
        PlanHttpClientConfig::default().default_timeout_ms,
        1_800_000,
    ] {
        let config = PlanHttpClientConfig {
            default_timeout_ms,
            ..cfg()
        };
        let finish =
            build_finish_workflow_package_request_spec(&config, &identity, "main", "direct", 0)
                .unwrap();
        assert_eq!(finish.timeout_ms, default_timeout_ms.max(900_000));
        let evidence = build_workflow_package_evidence_request_spec(
            &config,
            &identity,
            "attestation",
            "original-key",
            0,
            json!({}),
        )
        .unwrap();
        let abandon = build_abandon_workflow_package_request_spec(&config, &identity, 0).unwrap();
        let job = build_get_worker_job_request_spec(
            &config,
            crate::server_operational::WorkerJobKey::new(
                crate::server_operational::RepositoryIndex::new(0),
                crate::server_operational::WorkerJobIndex::new(0),
            ),
        )
        .unwrap();
        for ordinary in [evidence, abandon, job] {
            assert_eq!(ordinary.timeout_ms, default_timeout_ms);
        }
        assert_eq!(config.default_timeout_ms, default_timeout_ms);
    }
}

#[test]
fn workflow_package_specs_preserve_raw_bytes_exact_zero_job_and_actor_without_metadata_commit() {
    let mut id = identity();
    let bytes = vec![0x28, 0xb5, 0x2f, 0xfd, 0, 255, 0];
    id.package_hash = format!("{:x}", Sha256::digest(&bytes));
    let package = EncodedWorkflowPackage {
        bytes: bytes.clone(),
        package_hash: id.package_hash.clone(),
    };
    let manifest = WorkflowPackageManifest {
        repository_index: 0,
        target_line: "main".into(),
        base_snapshot_id: id.base_snapshot_id.clone(),
        revision_snapshot_id: id.revision_snapshot_id.clone(),
        author_mode: "ai_with_human_review".into(),
        summary: None,
        history_entries: Vec::new(),
        content: crate::repository_pack_json::ZstdBulkCommitRequest::from_json_rows(
            None,
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            None,
        )
        .unwrap(),
    };
    let fresh =
        build_prepare_workflow_package_request_spec(&cfg(), &manifest, &package, 0, None).unwrap();
    assert_eq!(fresh.method, "POST");
    assert_eq!(
        fresh.path,
        "/v1/native/repository-authorities/0/remote-sync/zstd-bulk/commit"
    );
    assert_eq!(fresh.headers["Content-Type"], WORKFLOW_PACKAGE_MEDIA_TYPE);
    assert_eq!(fresh.headers["X-Ait-Actor-Key-Index"], "0");
    assert!(!fresh.headers.contains_key("X-Ait-Worker-Job-Index"));
    assert_eq!(fresh.body, Some(bytes));
    let recovery =
        build_prepare_workflow_package_request_spec(&cfg(), &manifest, &package, 3, Some(&id))
            .unwrap();
    assert_eq!(recovery.headers["X-Ait-Worker-Job-Index"], "0");
    let finish =
        build_finish_workflow_package_request_spec(&cfg(), &id, "main", "direct", 3).unwrap();
    let body = finish.body.unwrap();
    assert_eq!(body["contract"], "task-land-package-guarded/v1");
    assert_eq!(
        body["idempotency_key"],
        format!("workflow-package:{}:0", id.package_hash)
    );
    assert_eq!(body["expected_identity"], id.to_value());
    assert_eq!(body.as_object().unwrap().len(), 6);
    assert!(body.get("task_or_change_ref").is_none());
    let mut different = id;
    different.worker_job_index = 1;
    different.revision_snapshot_id = "SNP-000000000003".into();
    assert!(build_prepare_workflow_package_request_spec(
        &cfg(),
        &manifest,
        &package,
        3,
        Some(&different)
    )
    .is_err());
}

#[test]
fn retained_reprepare_uses_only_empty_bytes_and_exact_existing_selectors() {
    let id = identity();
    let request = build_reprepare_workflow_package_request_spec(&cfg(), &id, 3).unwrap();
    assert_eq!(
        request.path,
        "/v1/native/repository-authorities/0/remote-sync/zstd-bulk/commit"
    );
    assert_eq!(request.headers["Content-Type"], WORKFLOW_PACKAGE_MEDIA_TYPE);
    assert_eq!(request.headers["X-Ait-Package-Sha256"], id.package_hash);
    assert_eq!(request.headers["X-Ait-Actor-Key-Index"], "3");
    assert_eq!(request.headers["X-Ait-Worker-Job-Index"], "0");
    assert_eq!(request.body, Some(Vec::new()));
    let mut foreign = id;
    foreign.repository_index = 1;
    assert!(build_reprepare_workflow_package_request_spec(&cfg(), &foreign, 3).is_err());
}

#[test]
fn abandonment_uses_its_own_tag_on_the_exact_bare_job_route_without_runner_lease_or_evidence_operation(
) {
    let id = identity();
    let request = build_abandon_workflow_package_request_spec(&cfg(), &id, 0).unwrap();
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.path,
        "/v1/native/repository-authorities/0/worker-jobs/0"
    );
    let body = request.body.unwrap();
    assert_eq!(body.as_object().unwrap().len(), 4);
    assert_eq!(body["contract"], "workflow-package-abandon/v1");
    assert_eq!(body["actor_key_index"], 0);
    assert_eq!(body["expected_identity"], json!(id));
    assert_eq!(
        body["idempotency_key"],
        format!("workflow-package-abandon:{}:0", id.package_hash)
    );
    for state in ["abandoning", "abandoned", "unknown"] {
        let value = json!({"expected_identity":id,"state":state});
        assert!(crate::workflow_package::PackageAbandonment::from_value(&value, &id).is_ok());
    }
    for value in [
        json!({"expected_identity":id,"state":"cancelled"}),
        json!({"expected_identity":id}),
        json!({"state":"abandoned"}),
    ] {
        assert!(crate::workflow_package::PackageAbandonment::from_value(&value, &id).is_err());
    }
    let mut wrong = id.clone();
    wrong.worker_job_index = 1;
    assert!(crate::workflow_package::PackageAbandonment::from_value(
        &json!({"expected_identity":wrong,"state":"abandoned"}),
        &id
    )
    .is_err());
}

#[test]
fn startup_health_http503_keeps_nullable_progress_and_never_enters_generic_busy_retry() {
    let (url, server) = spawn_scripted_http_server(vec![
        (
            503,
            r#"{"ready":false,"phase":"future_phase","processed":4,"total":null,"last_progress_at":"2026-10-20T00:00:00Z"}"#,
        ),
        (
            200,
            r#"{"ready":true,"phase":null,"processed":null,"total":null,"last_progress_at":null}"#,
        ),
    ]);
    let mut config = cfg();
    config.base_url = url;
    let mut manager = PlanHttpClientManager::new(config).unwrap();
    let upgrading = manager.get_upgrade_health().unwrap();
    assert!(!upgrading.ready);
    assert_eq!(upgrading.processed, Some(4));
    assert_eq!(upgrading.total, None);
    assert_eq!(upgrading.progress().unwrap().phase, "future_phase");
    let ready = manager.get_upgrade_health().unwrap();
    assert!(ready.ready);
    assert!(ready.progress().is_none());
    assert_eq!(manager.inspect().retry_count, 0);
    assert_eq!(server.join().unwrap(), ["GET", "GET"]);
}

#[test]
fn lost_package_finish_response_is_not_automatically_resubmitted_even_with_global_retries() {
    assert_lost_package_mutation_has_no_transport_replay(false);
}

#[test]
fn lost_retained_reprepare_response_is_not_automatically_resubmitted() {
    assert_lost_package_mutation_has_no_transport_replay(true);
}

fn assert_lost_package_mutation_has_no_transport_replay(retained: bool) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = find_header_end(&request) {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if request.len() >= end + length {
                    break;
                }
            }
        }
        let path = if retained {
            "remote-sync/zstd-bulk/commit"
        } else {
            "task-land"
        };
        assert!(String::from_utf8_lossy(&request)
            .starts_with(&format!("POST /v1/native/repository-authorities/0/{path} ")));
        if retained {
            let end = find_header_end(&request).unwrap();
            assert_eq!(request.len(), end);
        }
        drop(stream);
        listener.set_nonblocking(true).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_millis(250);
        let mut extra = 0;
        while std::time::Instant::now() < deadline {
            if let Ok((stream, _)) = listener.accept() {
                extra += 1;
                drop(stream);
            }
            thread::sleep(Duration::from_millis(5));
        }
        extra
    });
    let mut config = cfg();
    config.base_url = url;
    config.retry_attempts = 3;
    let mut manager = resolved_actor_client(config);
    let result = if retained {
        manager.reprepare_workflow_package(&identity())
    } else {
        manager.finish_workflow_package(&identity(), "main", "direct")
    };
    assert!(result.is_err());
    assert_eq!(manager.inspect().retry_count, 0);
    assert_eq!(manager.config.retry_attempts, 3);
    assert_eq!(server.join().unwrap(), 0);
}

#[test]
fn native_bound_job_observation_permits_only_exact_repository_job_cleanup_replay() {
    for (job, bound, reads) in [
        (
            r#"{"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":0}}"#,
            true,
            2,
        ),
        (
            r#"{"job":{"repository_index":1,"worker_job_index":0,"tombstoned":false,"patchset_index":1}}"#,
            false,
            1,
        ),
        (
            r#"{"job":{"repository_index":0,"worker_job_index":1,"tombstoned":false,"patchset_index":1}}"#,
            false,
            1,
        ),
        (
            r#"{"job":{"repository_index":0,"worker_job_index":0,"tombstoned":true,"patchset_index":1}}"#,
            false,
            1,
        ),
        (
            r#"{"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":4294967296}}"#,
            false,
            2,
        ),
        (
            r#"{"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false,"patchset_index":null},"package_execution":null}"#,
            false,
            1,
        ),
        (
            r#"{"job":{"repository_index":0,"worker_job_index":0,"tombstoned":false}}"#,
            false,
            1,
        ),
    ] {
        let mut replies = vec![(200, job)];
        if reads == 2 {
            replies.push((404, "{}"));
        }
        let (url, server) = spawn_scripted_http_server(replies);
        let mut config = cfg();
        config.base_url = url;
        config.retry_attempts = 0;
        let mut manager = PlanHttpClientManager::new(config).unwrap();
        let (completion, observed) =
            manager.observe_workflow_package_completion(&identity(), "main", &[], "fixture");
        assert_eq!(observed, bound, "{job}");
        assert_eq!(
            completion.outcome,
            crate::workflow_package::PackageCompletionOutcome::Unknown
        );
        assert!(completion.entries.is_empty());
        assert_eq!(server.join().unwrap(), vec!["GET"; reads]);
    }
}
