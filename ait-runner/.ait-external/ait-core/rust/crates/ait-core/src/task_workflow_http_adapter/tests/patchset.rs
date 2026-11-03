use super::*;
use crate::task_workflow_http_adapter::HttpWorkflowCloseoutRemote;

fn candidate_sequence(
    responses: Vec<JsonValue>,
) -> (
    TaskWorkflowHttpClientConfig,
    JoinHandle<Vec<RecordedHttpRequest>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for response in responses {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && std::time::Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(error) => panic!(
                        "candidate fixture: {error}; received {} requests",
                        requests.len()
                    ),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            requests.push(read_recorded_http_request(&mut stream));
            let body = response.to_string();
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
        requests
    });
    (
        TaskWorkflowHttpClientConfig {
            base_url: format!("http://{address}/"),
            repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
            headers: BTreeMap::new(),
            default_timeout_ms: 5000,
            retry_attempts: 0,
            retry_backoff_ms: 0,
            pool_max_idle_per_host: 1,
        },
        handle,
    )
}

fn current_candidate(ordinal: u8) -> JsonValue {
    json!({"task_id":"RT-0001","change_id":null,"change_ref":null,"patchset_id":format!("RT-0001/P-{ordinal:02}"),"base_snapshot_id":"B","revision_snapshot_id":"R"})
}

#[test]
fn task_only_publication_preflights_task_get_and_uses_existing_owner_route() {
    let candidate = current_candidate(1);
    let (config, server) = candidate_sequence(vec![
        json!({"task_id":"RT-0001","status":"active","successful_land":null,"current_patchset_id":null}),
        candidate.clone(),
    ]);
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);
    assert_eq!(
        remote
            .publish_patchset("RT-0001", "B", "R", "reviewed", "codex", Some("repo"), true)
            .unwrap(),
        candidate
    );
    let requests = server.join().unwrap();
    assert_eq!(requests[0].method, "GET");
    assert!(
        requests[0].target.ends_with("/tasks/RT-0001"),
        "{}",
        requests[0].target
    );
    assert_eq!(requests[1].method, "POST");
    assert!(requests[1].target.ends_with("/changes/RT-0001/patchsets"));
    assert_eq!(
        requests[1].body.as_ref().unwrap()["revision_snapshot_id"],
        "R"
    );
}

#[test]
fn repeated_publication_reuses_only_valid_matching_task_current_candidate() {
    for invalid in [false, true] {
        let mut candidate = current_candidate(2);
        candidate["invalidated"] = json!(invalid);
        let (config, server) = candidate_sequence(vec![
            json!({"task_id":"RT-0001","status":"active","successful_land":null,"current_patchset_id":"RT-0001/P-02"}),
            candidate.clone(),
        ]);
        let mut remote = HttpWorkflowCloseoutRemote::new(config)
            .unwrap()
            .with_resolved_test_actor(0);
        let result =
            remote.publish_patchset("RT-0001", "B", "R", "retry", "codex", Some("repo"), true);
        if invalid {
            assert!(result.is_err());
        } else {
            assert_eq!(result.unwrap(), candidate);
        }
        let requests = server.join().unwrap();
        assert!(requests.iter().all(|request| request.method == "GET"));
        assert!(
            requests[1].target.contains("/patchsets/RT-0001%2FP-02"),
            "{}",
            requests[1].target
        );
    }
}

#[test]
fn select_validates_current_task_candidate_without_accepting_an_older_ordinal() {
    for requested in ["RT-0001/P-01", "RT-0001/P-02"] {
        let candidate = current_candidate(2);
        let mut responses = vec![
            json!({"task_id":"RT-0001","status":"active","successful_land":null,"current_patchset_id":"RT-0001/P-02"}),
            candidate.clone(),
        ];
        if requested.ends_with("02") {
            responses.push(json!({"task_id":"RT-0001","change_id":null,"change_ref":null,"patchset_id":"RT-0001/P-02","current_patchset_id":"RT-0001/P-02","selection_changed":false,"patchset":candidate}));
        }
        let (config, server) = candidate_sequence(responses);
        let mut remote = HttpWorkflowCloseoutRemote::new(config)
            .unwrap()
            .with_resolved_test_actor(0);
        let result = remote.select_patchset("RT-0001", requested, Some("repo"), true);
        assert_eq!(result.is_ok(), requested.ends_with("02"));
        let requests = server.join().unwrap();
        if requested.ends_with("02") {
            assert!(requests[2]
                .target
                .ends_with("/changes/RT-0001:selectPatchset"));
        } else {
            assert_eq!(requests.len(), 2);
        }
    }
}

#[test]
fn task_workflow_patchset_ci_readiness_http_adapter_uses_bounded_projection() {
    let response = json!({
        "contract": "ait.server.patchset_ci.readiness.v2",
        "projection": "readiness",
        "patchset_id": "P-C-1-1",
        "recent_limit_applied": 20
    });
    let (mut config, server) = serve_task_workflow_json_once(response.clone());
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .expect("HTTP workflow closeout remote")
        .with_resolved_test_actor(0);

    assert_eq!(
        remote
            .read_patchset_ci_readiness("P-C-1-1", 200, None, true)
            .expect("bounded readiness response"),
        response
    );
    let request = server.join().expect("readiness fixture server");
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.target,
        "/v1/native/repository-authorities/7/read/patchsets/P-C-1-1/ci-status?recent_limit=20&projection=readiness"
    );
    assert_eq!(request.body, None);
}

#[test]
fn task_workflow_patchset_ci_dispatch_uses_repository_index_not_repository_name() {
    let response = json!({
        "patchset_id": "P-C-1-1",
        "queued": true
    });
    let (mut config, server) = serve_task_workflow_json_once(response.clone());
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .expect("HTTP workflow closeout remote")
        .with_resolved_test_actor(0);

    assert_eq!(
        remote
            .run_patchset_ci(
                "P-C-1-1",
                "workflow_ready_apply",
                Some("workflow_ready_foreground"),
                Some("renamed-repository"),
                true,
            )
            .expect("repository-index CI dispatch"),
        response
    );
    let request = server.join().expect("runCi fixture server");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.target,
        "/v1/native/repository-authorities/7/patchsets/P-C-1-1:runCi"
    );
    assert_eq!(
        request.body,
        Some(json!({
            "trigger": "workflow_ready_apply",
            "execution_profile": "workflow_ready_foreground"
        }))
    );
}

#[test]
fn task_workflow_patchset_helpers_accept_patchset_remote_trait() {
    let mut remote = FakePatchsetRemote;
    let remote_port: &mut dyn TaskWorkflowPatchsetRemote = &mut remote;

    assert_eq!(
        publish_patchset_with_task_workflow_closeout_remote(
            remote_port,
            "C-1",
            "SNP-BASE",
            "SNP-REV",
            "summary",
            "codex",
            Some("repo"),
            true,
        )
        .unwrap()["patchset_id"],
        "P-C-1-1"
    );
    assert_eq!(
        list_patchsets_with_task_workflow_closeout_remote(remote_port, "C-1", Some("repo"))
            .unwrap()[0]["patchset_id"],
        "P-C-1-1"
    );
    assert_eq!(
        get_patchset_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            Some("repo"),
            Some("C-1"),
        )
        .unwrap()["change_ref"],
        "C-1"
    );
    assert_eq!(
        select_patchset_with_task_workflow_closeout_remote(
            remote_port,
            "C-1",
            "P-C-1-1",
            Some("repo"),
            true,
        )
        .unwrap()["selected"],
        true
    );
    assert_eq!(
        run_patchset_ci_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            "workflow_ready_apply",
            Some("foreground"),
            Some("repo"),
            true,
        )
        .unwrap()["queued"],
        true
    );
}

#[test]
fn task_workflow_patchset_helpers_accept_single_capability_ports() {
    let mut lister = FakePatchsetListerPort;
    assert_eq!(
        list_patchsets_with_task_workflow_closeout_remote(&mut lister, "C-1", Some("repo"))
            .unwrap()[0]["patchset_id"],
        "P-C-1-1"
    );

    let mut reader = FakePatchsetReaderPort;
    assert_eq!(
        get_patchset_with_task_workflow_closeout_remote(
            &mut reader,
            "P-C-1-1",
            Some("repo"),
            Some("C-1"),
        )
        .unwrap()["change_ref"],
        "C-1"
    );

    let mut publisher = FakePatchsetPublisherPort;
    assert_eq!(
        publish_patchset_with_task_workflow_closeout_remote(
            &mut publisher,
            "C-1",
            "SNP-BASE",
            "SNP-REV",
            "summary",
            "codex",
            Some("repo"),
            true,
        )
        .unwrap()["revision_snapshot_id"],
        "SNP-REV"
    );

    let mut selector = FakePatchsetSelectorPort;
    assert_eq!(
        select_patchset_with_task_workflow_closeout_remote(
            &mut selector,
            "C-1",
            "P-C-1-1",
            Some("repo"),
            true,
        )
        .unwrap()["selected"],
        true
    );

    let mut ci_runner = FakePatchsetCiRunnerPort;
    assert_eq!(
        run_patchset_ci_with_task_workflow_closeout_remote(
            &mut ci_runner,
            "P-C-1-1",
            "workflow_ready_apply",
            Some("foreground"),
            Some("repo"),
            true,
        )
        .unwrap()["queued"],
        true
    );

    let mut ci_status_reader = FakePatchsetCiStatusReaderPort;
    assert_eq!(
        read_patchset_ci_status_with_task_workflow_closeout_remote(
            &mut ci_status_reader,
            "P-C-1-1",
            10,
            Some("repo"),
            true,
        )
        .unwrap()["tests_status"],
        "pass"
    );
    assert_eq!(
        read_patchset_ci_readiness_with_task_workflow_closeout_remote(
            &mut ci_status_reader,
            "P-C-1-1",
            10,
            Some("repo"),
            true,
        )
        .unwrap()["tests_status"],
        "pass"
    );

    let mut repo_job_lister = FakeRepoJobListerPort;
    assert_eq!(
        list_repo_jobs_with_task_workflow_closeout_remote(
            &mut repo_job_lister,
            "repo",
            None,
            10,
            false,
            300,
        )
        .unwrap()["repo_name"],
        "repo"
    );
}

#[test]
fn task_workflow_patchset_ci_helpers_accept_patchset_ci_remote_trait() {
    let mut remote = FakePatchsetCiRemote;
    let remote_port: &mut dyn TaskWorkflowPatchsetCiRemote = &mut remote;

    assert_eq!(
        read_patchset_ci_status_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            10,
            Some("repo"),
            true,
        )
        .unwrap()["tests_status"],
        "pass"
    );
    assert_eq!(
        read_patchset_ci_readiness_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            10,
            Some("repo"),
            true,
        )
        .unwrap()["tests_status"],
        "pass"
    );
    assert_eq!(
        list_repo_jobs_with_task_workflow_closeout_remote(
            remote_port,
            "repo",
            None,
            10,
            false,
            300,
        )
        .unwrap()["repo_name"],
        "repo"
    );
}
