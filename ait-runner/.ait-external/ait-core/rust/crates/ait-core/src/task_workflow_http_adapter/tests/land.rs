use super::*;
use crate::plan_http_client::{
    build_get_land_request_spec, build_retry_land_request_spec, build_submit_land_request_spec,
    build_submit_task_land_request_spec,
};
use crate::plan_workflow_json::PlanWorkflowJson;
use crate::task_workflow_http_adapter::HttpWorkflowCloseoutRemote;

fn atomic_task_land_response() -> JsonValue {
    json!({
        "contract": "task-land-atomic/v1",
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
        "patchset_id": "RCT-1/C-01/P-01",
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
            "selected_patchset_id": "RCT-1/C-01/P-01"
        },
        "patchset": {
            "patchset_id": "RCT-1/C-01/P-01",
            "revision_snapshot_id": "SNP-2"
        },
        "land": {
            "submission_id": "RCT-1/C-01/L-01",
            "status": "succeeded",
            "target_line": "main",
            "landed_snapshot_id": "SNP-2"
        }
    })
}

#[test]
fn atomic_task_land_http_adapter_uses_one_exact_repository_mutation() {
    let (mut config, server) = serve_task_workflow_json_once(atomic_task_land_response());
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let spec_config = config.clone();
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

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
            "contract": "task-land-atomic/v1",
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
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

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
    let mut response = atomic_task_land_response();
    response["replayed"] = json!(true);
    let (mut config, server) = serve_task_workflow_timeout_then_json(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

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
    let mut response = atomic_task_land_response();
    response["replayed"] = json!(true);
    let (mut config, server) = serve_task_workflow_repeated_timeouts_then_json(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

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
    let mut submit_remote =
        HttpWorkflowCloseoutRemote::new(submit_config).expect("http closeout remote");
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
    let mut get_remote = HttpWorkflowCloseoutRemote::new(get_config).expect("http closeout remote");

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
    let mut retry_remote =
        HttpWorkflowCloseoutRemote::new(retry_config).expect("http closeout remote");

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
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
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
