use super::*;
use crate::attest_json::AttestJson;
use crate::plan_http_client::PlanHttpClientConfig;
use crate::task_workflow_http_adapter::HttpWorkflowCloseoutRemote;
use std::collections::BTreeMap;

fn plan_http_config() -> PlanHttpClientConfig {
    PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
        headers: BTreeMap::from([(String::from("X-Test"), String::from("yes"))]),
        default_timeout_ms: 12_345,
        retry_attempts: 2,
        retry_backoff_ms: 7,
        pool_max_idle_per_host: 3,
    }
}

#[test]
fn attest_json_builds_request_specs() {
    let attest = AttestJson::stateless();
    let statement = crate::attest_json::RemoteAttestationStatement::default();

    let put = attest
        .build_put_attestation_request_spec(
            &plan_http_config(),
            " RCP-1 ",
            " ai_with_human_review ",
            &statement,
        )
        .unwrap();
    assert_eq!(put.method, "PUT");
    assert_eq!(
        put.path,
        "/v1/native/repository-authorities/7/patchsets/RCP-1/attestation"
    );
    assert_eq!(
        put.body,
        Some(json!({
            "author_mode": "ai_with_human_review",
            "verification_state":"unknown",
            "require_tests_pass":true,
            "require_human_review":false,
            "require_lint_pass":false,
            "revoked":false,
        }))
    );

    let get = attest
        .build_get_attestation_request_spec(&plan_http_config(), " RCP-1 ")
        .unwrap();
    assert_eq!(get.method, "GET");
    assert_eq!(
        get.path,
        "/v1/native/repository-authorities/7/patchsets/RCP-1/attestation"
    );
    assert!(get.body.is_none());
}

#[test]
fn attest_json_prefers_repository_authority_paths() {
    let attest = AttestJson::stateless();
    let mut config = plan_http_config();
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));
    let put = attest
        .build_put_attestation_request_spec(
            &config,
            "P-RCT-1/C-01-1",
            "ai_with_human_review",
            &crate::attest_json::RemoteAttestationStatement::default(),
        )
        .unwrap();
    assert_eq!(
        put.path,
        "/v1/native/repository-authorities/9/patchsets/P-RCT-1%2FC-01-1/attestation"
    );
    assert_eq!(
        attest
            .build_get_attestation_request_spec(&config, "P-RCT-1/C-01-1")
            .unwrap()
            .path,
        put.path
    );
}

fn compact_candidate() -> JsonValue {
    json!({"task_id":"RT-0001","change_id":null,"change_ref":null,"patchset_id":"RT-0001/P-01","base_snapshot_id":"B","revision_snapshot_id":"R","author_mode":"ai_with_human_review","invalidated":false})
}
fn compact_statement(state: &str, required: bool) -> JsonValue {
    json!({"attestation_id":"RT-0001/P-01/AT-01","task_id":"RT-0001","change_id":null,"change_ref":null,"patchset_id":"RT-0001/P-01","author_mode":"ai_with_human_review","verification_state":state,"require_tests_pass":required,"require_human_review":false,"require_lint_pass":false,"revoked":false,"ci_backed":false,"evaluation_summary":{"tests":"none","lint":"none"}})
}
fn compact_http(
    responses: Vec<(u16, JsonValue)>,
) -> (
    TaskWorkflowHttpClientConfig,
    JoinHandle<Vec<RecordedHttpRequest>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, response) in responses {
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
                    Err(error) => panic!("compact fixture: {error}; received {}", requests.len()),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            requests.push(read_recorded_http_request(&mut stream));
            if status == 0 {
                thread::spawn(move || {
                    thread::sleep(Duration::from_millis(200));
                    drop(stream);
                });
                continue;
            }
            let body = response.to_string();
            write!(stream,"HTTP/1.1 {status} response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
        }
        requests
    });
    (
        TaskWorkflowHttpClientConfig {
            base_url: format!("http://{address}/"),
            repository_index: Some(crate::server_operational::RepositoryIndex::new(19)),
            headers: BTreeMap::new(),
            default_timeout_ms: 5000,
            retry_attempts: 2,
            retry_backoff_ms: 0,
            pool_max_idle_per_host: 1,
        },
        handle,
    )
}
fn compact_preflight() -> Vec<(u16, JsonValue)> {
    vec![
        (200, compact_candidate()),
        (
            200,
            json!({"task_id":"RT-0001","status":"active","successful_land":null,"current_patchset_id":"RT-0001/P-01"}),
        ),
        (200, compact_candidate()),
    ]
}
#[test]
fn compact_attestation_http_writes_exact_statement_and_reuses_retry() {
    let statement = crate::attest_json::RemoteAttestationStatement {
        verification_state: crate::attest_json::RemoteVerificationState::Pass,
        ..Default::default()
    };
    let mut responses = compact_preflight();
    responses.extend([
        (404, json!({"detail":"Unknown attestation"})),
        (200, compact_statement("pass", true)),
    ]);
    responses.extend(compact_preflight());
    responses.push((200, compact_statement("pass", true)));
    let (config, server) = compact_http(responses);
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);
    for _ in 0..2 {
        let value = remote
            .put_attestation(
                "RT-0001/P-01",
                "ai_with_human_review",
                &statement,
                Some("repo"),
                true,
            )
            .unwrap();
        assert_eq!(value["evaluation_summary"]["tests"], "none");
        assert_eq!(value["ci_backed"], false);
    }
    let requests = server.join().unwrap();
    let writes: Vec<_> = requests.iter().filter(|r| r.method == "PUT").collect();
    assert_eq!(writes.len(), 1);
    assert!(writes[0]
        .target
        .ends_with("/patchsets/RT-0001%2FP-01/attestation"));
    assert_eq!(
        writes[0].body,
        Some(
            json!({"author_mode":"ai_with_human_review","verification_state":"pass","require_tests_pass":true,"require_lint_pass":false,"require_human_review":false,"revoked":false})
        )
    );
}
#[test]
fn compact_attestation_incompatible_readback_and_candidate_drift_stop_writes() {
    for failure in ["readback", "owner", "drift", "existing"] {
        let mut responses = compact_preflight();
        if failure == "drift" {
            responses[2].1["revision_snapshot_id"] = json!("NEW");
        } else if failure == "existing" {
            responses.push((200, json!({"evaluation_summary":{"tests":"pass"}})));
        } else {
            responses.push((404, json!({"detail":"Unknown attestation"})));
            let mut result = compact_statement("unknown", true);
            if failure == "owner" {
                result["task_id"] = json!("RT-9999");
            }
            responses.push((200, result));
        }
        let (config, server) = compact_http(responses);
        let mut remote = HttpWorkflowCloseoutRemote::new(config)
            .unwrap()
            .with_resolved_test_actor(0);
        let statement = crate::attest_json::RemoteAttestationStatement {
            verification_state: crate::attest_json::RemoteVerificationState::Pass,
            ..Default::default()
        };
        assert!(remote
            .put_attestation(
                "RT-0001/P-01",
                "ai_with_human_review",
                &statement,
                Some("repo"),
                true
            )
            .is_err());
        let requests = server.join().unwrap();
        assert_eq!(
            requests.iter().filter(|r| r.method == "PUT").count(),
            usize::from(matches!(failure, "readback" | "owner"))
        );
    }
}
#[test]
fn compact_attestation_optional_retry_keeps_exact_flags_and_required_flags_never_weaken() {
    for required in [false, true] {
        let mut responses = compact_preflight();
        responses.push((200, compact_statement("pass", required)));
        let (config, server) = compact_http(responses);
        let mut remote = HttpWorkflowCloseoutRemote::new(config)
            .unwrap()
            .with_resolved_test_actor(0);
        let statement = crate::attest_json::RemoteAttestationStatement {
            verification_state: crate::attest_json::RemoteVerificationState::Pass,
            require_tests_pass: false,
            ..Default::default()
        };
        let result = remote
            .put_attestation(
                "RT-0001/P-01",
                "ai_with_human_review",
                &statement,
                Some("repo"),
                true,
            )
            .unwrap();
        assert_eq!(result["require_tests_pass"], required);
        assert!(server.join().unwrap().iter().all(|r| r.method == "GET"));
    }
}

#[test]
fn attest_json_builds_evaluation_and_provenance_payloads() {
    let attest = AttestJson::stateless();
    let evaluation =
        attest.build_evaluation_summary(Some(" pass "), Some(" warn "), None, Some(" "));
    assert_eq!(
        evaluation,
        json!({
            "tests": "pass",
            "lint": "warn",
        })
    );

    let (provenance, detail) = attest
        .build_minimum_provenance("ai_with_human_review", Some(" gpt-5 "))
        .unwrap();
    assert_eq!(provenance["model_name"], json!("gpt-5"));
    assert_eq!(provenance["evidence_readiness"], json!("complete"));
    assert_eq!(detail["minimum_evidence"]["policy_readable"], json!(true));
}

#[test]
fn task_workflow_attestation_helpers_accept_attestation_remote_trait() {
    let mut remote = FakeAttestationRemote;
    let remote_port: &mut dyn TaskWorkflowAttestationRemote = &mut remote;

    assert_eq!(
        put_attestation_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            "manual",
            &crate::attest_json::RemoteAttestationStatement::default(),
            Some("repo"),
            true,
        )
        .unwrap()["author_mode"],
        "manual"
    );
    assert_eq!(
        get_attestation_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            Some("repo"),
            true,
        )
        .unwrap()["attestation"],
        true
    );
}

#[test]
fn task_workflow_attestation_helpers_accept_single_capability_ports() {
    let mut writer = FakeAttestationWriterPort;
    let mut reader = FakeAttestationReaderPort;

    assert_eq!(
        put_attestation_with_task_workflow_closeout_remote(
            &mut writer,
            "P-C-1-1",
            "manual",
            &crate::attest_json::RemoteAttestationStatement::default(),
            Some("repo"),
            true,
        )
        .unwrap()["author_mode"],
        "manual"
    );
    assert_eq!(
        get_attestation_with_task_workflow_closeout_remote(
            &mut reader,
            "P-C-1-1",
            Some("repo"),
            true,
        )
        .unwrap()["attestation"],
        true
    );
}

#[test]
fn compact_attestation_transport_timeout_recovers_by_get_without_replaying_append() {
    let mut responses = compact_preflight();
    responses.extend([
        (404, json!({"detail":"Unknown attestation"})),
        (0, compact_statement("pass", true)),
        (200, compact_statement("pass", true)),
    ]);
    let (mut config, server) = compact_http(responses);
    config.default_timeout_ms = 100;
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);
    let statement = crate::attest_json::RemoteAttestationStatement {
        verification_state: crate::attest_json::RemoteVerificationState::Pass,
        ..Default::default()
    };
    let value = remote
        .put_attestation(
            "RT-0001/P-01",
            "ai_with_human_review",
            &statement,
            Some("repo"),
            true,
        )
        .unwrap();
    assert_eq!(value["verification_state"], "pass");
    let requests = server.join().unwrap();
    assert_eq!(requests.iter().filter(|row| row.method == "PUT").count(), 1);
    assert_eq!(requests.last().unwrap().method, "GET");
}
