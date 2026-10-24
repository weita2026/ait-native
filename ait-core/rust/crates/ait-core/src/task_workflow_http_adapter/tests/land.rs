use super::*;
use crate::plan_http_client::{
    build_get_land_request_spec, build_retry_land_request_spec, build_submit_land_request_spec,
    build_submit_task_land_request_spec, PlanHttpClientConfig,
};
use crate::plan_workflow_json::PlanWorkflowJson;
use crate::task_workflow_http_adapter::HttpWorkflowCloseoutRemote;
use std::time::Instant;

fn atomic_task_land_response() -> JsonValue {
    let mut response = json!({
        "contract": "task-land-atomic/v2",
        "repo_name": "repo",
        "repository_index": 7,
        "idempotency_key": "task-land-atomic:key",
        "replayed": false,
        "status": "succeeded",
        "task_id": "RCT-1",
        "task_status": "completed",
        "change_id": "C-01",
        "change_ref": "RCT-1/C-01",
        "change_status": "landed",
        "patchset_id": "RCT-1/P-01",
        "target_line": "main",
        "landed_snapshot_id": "SNP-2",
        "task": {
            "task_id": "RCT-1",
            "status": "completed"
        },
        "change": {
            "task_id": "RCT-1",
            "change_id": "C-01",
            "change_ref": "RCT-1/C-01",
            "status": "landed",
            "selected_patchset_id": "RCT-1/P-01"
        },
        "patchset": {
            "task_id":"RCT-1", "change_ref":"RCT-1/C-01",
            "patchset_id": "RCT-1/P-01",
            "base_snapshot_id":"SNP-1",
            "revision_snapshot_id": "SNP-2"
        },
        "land": {
            "submission_id": "RCT-1/L-01",
            "repository_index":7,"task_id":"RCT-1","change_id":"C-01","change_ref":"RCT-1/C-01","patchset_id":"RCT-1/P-01",
            "base_snapshot_id":"SNP-1","revision_snapshot_id":"SNP-2","target_line_index":0,"pre_land_target_snapshot_id":"SNP-1","mode":"direct",
            "created_at":"2026-10-18T00:00:00Z","updated_at":"2026-10-18T00:00:01Z",
            "status": "succeeded",
            "target_line": "main",
            "landed_snapshot_id": "SNP-2"
        }
    });
    response["task"]["successful_land"] = response["land"].clone();
    response["task"]["current_patchset_id"] = response["patchset_id"].clone();
    response
}

fn atomic_task_land_response_for_mode(mode: &str) -> JsonValue {
    let mut response = atomic_task_land_response();
    response["land"]["mode"] = json!(mode);
    response["task"]["successful_land"]["mode"] = json!(mode);
    response
}

#[test]
fn task_only_atomic_receipt_preserves_accepted_candidate_and_actual_applied_result() {
    use crate::workflow_expected_identity::{WorkflowExpectedIdentity, GUARDED_TASK_LAND_CONTRACT};
    let expected=WorkflowExpectedIdentity::from_value(&json!({"repository_index":7,"patchset_id":"RCT-1/P-01","base_snapshot_id":"SNP-1","revision_snapshot_id":"SNP-2"})).unwrap();
    let mut response = atomic_task_land_response();
    response["contract"] = json!(GUARDED_TASK_LAND_CONTRACT);
    response["expected_identity"] = json!(expected);
    response["patchset"]["base_snapshot_id"] = json!("SNP-1");
    for field in ["change", "change_id", "change_ref", "change_status"] {
        response[field] = JsonValue::Null;
    }
    response["patchset"]["change_ref"] = JsonValue::Null;
    response["land"]["change_ref"] = JsonValue::Null;
    response["land"]["change_id"] = JsonValue::Null;
    response["land"]["mode"] = json!("merge");
    // Merge acceptance can apply a result different from the reviewed revision.
    response["landed_snapshot_id"] = json!("SNP-MERGE");
    response["land"]["landed_snapshot_id"] = json!("SNP-MERGE");
    response["task"]["successful_land"] = response["land"].clone();
    response["task"]["current_patchset_id"] = json!("RCT-1/P-09");
    for mutation in [
        None,
        Some("task_id"),
        Some("patchset_id"),
        Some("change_ref"),
    ] {
        let mut actual = response.clone();
        if let Some(field) = mutation {
            actual["land"][field] = json!("FOREIGN");
        }
        let (mut config, server) = serve_task_workflow_json_once(actual);
        config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
        let mut remote = HttpWorkflowCloseoutRemote::new(config)
            .unwrap()
            .with_resolved_test_actor(0);
        let result = remote.submit_task_land_guarded(
            "RCT-1",
            Some("main"),
            "merge",
            "task-land-atomic:key",
            Some("repo"),
            Some(&expected),
        );
        if mutation.is_none() {
            let accepted = result.unwrap();
            assert!(accepted["change"].is_null());
            assert_eq!(accepted["landed_snapshot_id"], "SNP-MERGE");
            assert_eq!(accepted["patchset"]["revision_snapshot_id"], "SNP-2");
        } else {
            assert!(result.is_err(), "{mutation:?}");
        }
        server.join().unwrap();
    }
}

#[test]
fn guarded_atomic_http_pins_identity_retries_same_body_and_requires_server_acknowledgement() {
    use crate::workflow_expected_identity::{WorkflowExpectedIdentity, GUARDED_TASK_LAND_CONTRACT};
    let expected = WorkflowExpectedIdentity::from_value(&json!({"repository_index":7,"patchset_id":"RCT-1/P-01","revision_snapshot_id":"SNP-2","base_snapshot_id":"SNP-1"})).unwrap();
    let mut response = atomic_task_land_response();
    response["contract"] = json!(GUARDED_TASK_LAND_CONTRACT);
    response["patchset"]["base_snapshot_id"] = json!("SNP-1");
    response["expected_identity"] = json!(expected);
    response["replayed"] = json!(true);
    let (mut config, server) = serve_task_workflow_timeout_then_json(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);
    remote
        .submit_task_land_guarded(
            "RCT-1",
            Some("main"),
            "direct",
            "task-land-atomic:key",
            Some("repo"),
            Some(&expected),
        )
        .unwrap();
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(
        requests[0].body.as_ref().unwrap()["contract"],
        GUARDED_TASK_LAND_CONTRACT
    );
    assert_eq!(
        requests[0].body.as_ref().unwrap()["expected_identity"],
        json!(expected)
    );

    let (mut config, server) = serve_task_workflow_json_once(atomic_task_land_response());
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);
    let error = remote
        .submit_task_land_guarded(
            "RCT-1",
            Some("main"),
            "direct",
            "task-land-atomic:key",
            Some("repo"),
            Some(&expected),
        )
        .unwrap_err();
    assert!(error.to_string().contains("contract"));
    server.join().unwrap();
}

#[test]
fn guarded_atomic_http_rejects_wrong_numeric_authority_before_transport() {
    let expected = crate::workflow_expected_identity::WorkflowExpectedIdentity::from_value(&json!({"repository_index":8,"patchset_id":"RCT-1/P-01","revision_snapshot_id":"R","base_snapshot_id":"B"})).unwrap();
    let config = PlanHttpClientConfig {
        repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
        ..PlanHttpClientConfig::default()
    };
    assert!(crate::land_json::LandJson::stateless()
        .build_submit_task_land_guarded_request_spec(
            &config,
            "RCT-1",
            Some("main"),
            "direct",
            "key",
            Some("repo"),
            Some(&expected)
        )
        .is_err());
}

#[test]
fn guarded_atomic_http_old_server_rejection_never_falls_back_to_unguarded() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let first = read_recorded_http_request(&mut stream);
        let body =
            json!({"error":"Atomic Task Land contract must be task-land-atomic/v2"}).to_string();
        write!(stream, "HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        drop(stream);
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + Duration::from_millis(200);
        let mut requests = vec![first];
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    requests.push(read_recorded_http_request(&mut stream));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(10))
                }
                Err(error) => panic!("fixture accept: {error}"),
            }
        }
        requests
    });
    let config = TaskWorkflowHttpClientConfig {
        base_url: format!("http://{address}/"),
        repository_index: Some(crate::server_operational::RepositoryIndex::new(7)),
        ..TaskWorkflowHttpClientConfig::default()
    };
    let expected = crate::workflow_expected_identity::WorkflowExpectedIdentity::from_value(&json!({"repository_index":7,"patchset_id":"RCT-1/P-01","revision_snapshot_id":"R","base_snapshot_id":"B"})).unwrap();
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);
    let error = remote
        .submit_task_land_guarded(
            "RCT-1",
            Some("main"),
            "direct",
            "key",
            Some("repo"),
            Some(&expected),
        )
        .unwrap_err();
    assert!(error.to_string().contains("task-land-atomic/v2"));
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].body.as_ref().unwrap()["contract"],
        "task-land-atomic-guarded/v2"
    );
}

#[test]
fn atomic_task_land_http_adapter_uses_one_exact_repository_mutation() {
    let (mut config, server) =
        serve_task_workflow_json_once(atomic_task_land_response_for_mode("merge"));
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let spec_config = config.clone();
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);

    let response = submit_task_land_with_task_workflow_closeout_remote(
        &mut remote,
        "RCT-1",
        Some("main"),
        "merge",
        "task-land-atomic:key",
        Some("repo"),
    )
    .unwrap();
    assert_eq!(response["task"]["status"], "completed");
    assert_eq!(response["change"]["status"], "landed");
    assert_eq!(response["land"]["status"], "succeeded");

    let recorded = server.join().unwrap();
    assert_eq!(recorded.method, "POST");
    assert_eq!(
        recorded.target,
        "/v1/native/repository-authorities/7/task-land"
    );
    assert_eq!(
        recorded.body,
        Some(json!({
            "actor_key_index": 0,
            "contract": "task-land-atomic/v2",
            "idempotency_key": "task-land-atomic:key",
            "task_or_change_ref": "RCT-1",
            "target_line": "main",
            "mode": "merge"
        }))
    );

    let spec = build_submit_task_land_request_spec(
        &spec_config,
        "RCT-1",
        Some("main"),
        "merge",
        "task-land-atomic:key",
        Some("repo"),
    )
    .unwrap();
    assert_eq!(spec.path, "/v1/native/repository-authorities/7/task-land");
}

#[test]
fn atomic_task_land_http_adapter_fails_closed_on_projection_mismatch() {
    let mut response = atomic_task_land_response();
    response["land"]["landed_snapshot_id"] = json!("SNP-WRONG");
    let (mut config, server) = serve_task_workflow_json_once(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);

    let error = remote
        .submit_task_land(
            "RCT-1/C-01",
            Some("main"),
            "merge",
            "task-land-atomic:key",
            Some("repo"),
        )
        .unwrap_err();
    assert!(error.to_string().contains("landed_snapshot_id"));
    server.join().unwrap();
}

#[test]
fn atomic_task_land_timeout_retries_only_the_identical_atomic_request() {
    let mut response = atomic_task_land_response_for_mode("merge");
    response["replayed"] = json!(true);
    let (mut config, server) = serve_task_workflow_timeout_then_json(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);

    let result = remote
        .submit_task_land(
            "RCT-1",
            Some("main"),
            "merge",
            "task-land-atomic:key",
            Some("repo"),
        )
        .unwrap();
    assert_eq!(result["replayed"], true);

    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].method, "POST");
    assert_eq!(requests[0].target, requests[1].target);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(
        requests[0].target,
        "/v1/native/repository-authorities/7/task-land"
    );
}

#[test]
fn atomic_task_land_repeated_timeouts_resume_the_identical_atomic_request() {
    let mut response = atomic_task_land_response_for_mode("merge");
    response["replayed"] = json!(true);
    let (mut config, server) = serve_task_workflow_repeated_timeouts_then_json(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);

    let result = remote
        .submit_task_land(
            "RCT-1",
            Some("main"),
            "merge",
            "task-land-atomic:key",
            Some("repo"),
        )
        .unwrap();
    assert_eq!(result["replayed"], true);

    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| request.method == "POST"));
    assert!(requests
        .windows(2)
        .all(|pair| pair[0].target == pair[1].target && pair[0].body == pair[1].body));
    assert_eq!(
        requests[0].target,
        "/v1/native/repository-authorities/7/task-land"
    );
}

#[test]
fn task_land_http_adapter_fixture_roundtrips_submit_get_and_retry() {
    let submit_response = json!({
        "submission_id": "LAND-1",
        "change_id": "C-1",
        "status": "queued"
    });
    let (submit_config, submit_server) = serve_task_workflow_json_once(submit_response.clone());
    let submit_spec_config = submit_config.clone();
    let mut submit_remote = HttpWorkflowCloseoutRemote::new(submit_config)
        .expect("http closeout remote")
        .with_resolved_test_actor(0);
    submit_remote
        .set_bound_change_context(Some("T-1"), Some("C-1"))
        .expect("bound change context");

    assert_eq!(
        submit_remote
            .submit_land("C-1", Some("P-C-1-1"), "main", "merge", Some("repo"))
            .expect("submit land response"),
        json!({
            "submission_id": "LAND-1",
            "change_id": "C-1",
            "change_ref": "T-1/C-1",
            "status": "queued"
        })
    );
    let recorded_submit = submit_server.join().expect("submit fixture server");
    assert_eq!(recorded_submit.method, "POST");
    assert_eq!(
        recorded_submit.target,
        "/v1/native/repository-authorities/7/changes/T-1%2FC-1:submit"
    );
    assert_eq!(
        recorded_submit.body,
        Some(json!({
            "actor_key_index": 0,
            "patchset_id": "P-C-1-1",
            "target_line": "main",
            "mode": "merge",
        }))
    );

    let submit_spec = build_submit_land_request_spec(
        &submit_spec_config,
        "C-1",
        Some("P-C-1-1"),
        "main",
        "merge",
        Some("repo"),
    )
    .unwrap();
    assert_eq!(
        PlanWorkflowJson::stateless().task_workflow_http_request_spec_payload(&submit_spec),
        json!({
            "method": "POST",
            "path": "/v1/native/repository-authorities/7/changes/C-1:submit",
            "url": format!("{}v1/native/repository-authorities/7/changes/C-1:submit", submit_spec_config.base_url),
            "query_pairs": [],
            "headers": {
                "Accept": "application/json",
                "Content-Type": "application/json",
            },
            "body": {
                "patchset_id": "P-C-1-1",
                "target_line": "main",
                "mode": "merge",
            },
            "timeout_ms": 5000,
        })
    );

    let get_response = json!({
        "submission_id": "LAND-1",
        "status": "queued"
    });
    let (get_config, get_server) = serve_task_workflow_json_once(get_response.clone());
    let get_spec_config = get_config.clone();
    let mut get_remote = HttpWorkflowCloseoutRemote::new(get_config)
        .expect("http closeout remote")
        .with_resolved_test_actor(0);

    assert_eq!(
        get_remote
            .get_land("LAND-1", Some("repo"))
            .expect("get land response"),
        get_response
    );
    let recorded_get = get_server.join().expect("get fixture server");
    assert_eq!(recorded_get.method, "GET");
    assert_eq!(
        recorded_get.target,
        "/v1/native/repository-authorities/7/lands/LAND-1"
    );
    assert!(recorded_get.body.is_none());

    let get_spec = build_get_land_request_spec(&get_spec_config, "LAND-1", Some("repo")).unwrap();
    assert_eq!(
        PlanWorkflowJson::stateless().task_workflow_http_request_spec_payload(&get_spec),
        json!({
            "method": "GET",
            "path": "/v1/native/repository-authorities/7/lands/LAND-1",
            "url": format!("{}v1/native/repository-authorities/7/lands/LAND-1", get_spec_config.base_url),
            "query_pairs": [],
            "headers": {"Accept": "application/json"},
            "body": null,
            "timeout_ms": 5000,
        })
    );

    let retry_response = json!({
        "submission_id": "LAND-1",
        "retried": true,
        "status": "queued"
    });
    let (retry_config, retry_server) = serve_task_workflow_json_once(retry_response.clone());
    let retry_spec_config = retry_config.clone();
    let mut retry_remote = HttpWorkflowCloseoutRemote::new(retry_config)
        .expect("http closeout remote")
        .with_resolved_test_actor(0);

    assert_eq!(
        retry_remote
            .retry_land("LAND-1", Some("retry after CI"), Some("repo"))
            .expect("retry land response"),
        retry_response
    );
    let recorded_retry = retry_server.join().expect("retry fixture server");
    assert_eq!(recorded_retry.method, "POST");
    assert_eq!(
        recorded_retry.target,
        "/v1/native/repository-authorities/7/lands/LAND-1:retry"
    );
    assert_eq!(
        recorded_retry.body,
        Some(json!({
            "actor_key_index": 0,
            "reason": "retry after CI",
        }))
    );

    let retry_spec = build_retry_land_request_spec(
        &retry_spec_config,
        "LAND-1",
        Some("retry after CI"),
        Some("repo"),
    )
    .unwrap();
    assert_eq!(
        PlanWorkflowJson::stateless().task_workflow_http_request_spec_payload(&retry_spec),
        json!({
            "method": "POST",
            "path": "/v1/native/repository-authorities/7/lands/LAND-1:retry",
            "url": format!("{}v1/native/repository-authorities/7/lands/LAND-1:retry", retry_spec_config.base_url),
            "query_pairs": [],
            "headers": {
                "Accept": "application/json",
                "Content-Type": "application/json",
            },
            "body": {"reason": "retry after CI"},
            "timeout_ms": 5000,
        })
    );
}

#[test]
fn task_land_http_adapter_submits_through_repository_authority() {
    let (mut config, server) = serve_task_workflow_json_once(json!({
        "submission_id": "LAND-1",
        "change_id": "RCT-1/C-01",
        "task_id": "RCT-1",
        "status": "queued"
    }));
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));
    let mut remote = HttpWorkflowCloseoutRemote::new(config)
        .unwrap()
        .with_resolved_test_actor(0);
    remote
        .submit_land(
            "RCT-1/C-01",
            Some("P-RCT-1/C-01-1"),
            "main",
            "merge",
            Some("legacy-name"),
        )
        .unwrap();

    let recorded = server.join().unwrap();
    assert_eq!(recorded.method, "POST");
    assert_eq!(
        recorded.target,
        "/v1/native/repository-authorities/9/changes/RCT-1%2FC-01:submit"
    );
}

#[test]
fn task_workflow_land_helpers_accept_land_remote_trait() {
    let mut remote = FakeLandRemote;
    let remote_port: &mut dyn TaskWorkflowLandRemote = &mut remote;

    assert_eq!(
        submit_land_with_task_workflow_closeout_remote(
            remote_port,
            "C-1",
            Some("P-C-1-1"),
            "main",
            "merge",
            Some("repo"),
        )
        .unwrap()["submission_id"],
        "LAND-1"
    );
    assert_eq!(
        get_land_with_task_workflow_closeout_remote(remote_port, "LAND-1", Some("repo")).unwrap()
            ["status"],
        "queued"
    );
    assert_eq!(
        retry_land_with_task_workflow_closeout_remote(
            remote_port,
            "LAND-1",
            Some("retry"),
            Some("repo"),
        )
        .unwrap()["retried"],
        true
    );
}

#[test]
fn task_workflow_land_helpers_accept_single_capability_ports() {
    let mut submitter = FakeLandSubmitterPort;
    let mut reader = FakeLandReaderPort;
    let mut retryer = FakeLandRetryerPort;

    assert_eq!(
        submit_land_with_task_workflow_closeout_remote(
            &mut submitter,
            "C-1",
            Some("P-C-1-1"),
            "main",
            "merge",
            Some("repo"),
        )
        .unwrap()["submission_id"],
        "LAND-1"
    );
    assert_eq!(
        get_land_with_task_workflow_closeout_remote(&mut reader, "LAND-1", Some("repo")).unwrap()
            ["status"],
        "queued"
    );
    assert_eq!(
        retry_land_with_task_workflow_closeout_remote(
            &mut retryer,
            "LAND-1",
            Some("retry"),
            Some("repo"),
        )
        .unwrap()["retried"],
        true
    );
}
