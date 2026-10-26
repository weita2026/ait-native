use super::*;
use crate::plan_http_client::build_prepare_history_promotion_request_spec;
use crate::task_workflow_http_adapter::HttpWorkflowCloseoutRemote;

fn history_request() -> JsonValue {
    json!({
        "contract": "history-promotion-prepare/v1",
        "idempotency_key": "history-promotion:stable",
        "target_line": "main",
        "base_snapshot_id": "SNP-0",
        "revision_snapshot_id": "SNP-1",
        "author_mode": "ai_with_human_review",
        "summary": "promote local history",
        "entries": [{
            "local_task_id": "LCT-1",
            "local_change_id": "C-01",
            "local_change_ref": "LCT-1/C-01",
            "task": {
                "title": "task",
                "intent": "intent",
                "plan_id": "RP-1",
                "origin_plan_revision_id": "RPR-1",
                "plan_item_ref": "item/1"
            },
            "change": {
                "title": "change",
                "base_line": "main",
                "fork_snapshot_id": "SNP-0"
            },
            "pre_land_target_snapshot_id": "SNP-0",
            "landed_snapshot_id": "SNP-1",
            "landed_at_s": 100,
            "snapshots": [{"snapshot_id": "SNP-1", "created_at_s": 99}]
        }]
    })
}

fn history_response() -> JsonValue {
    json!({
        "contract": "history-promotion-prepare/v1",
        "repo_name": "repo",
        "repository_index": 7,
        "idempotency_key": "history-promotion:stable",
        "replayed": false,
        "target_line": "main",
        "base_snapshot_id": "SNP-0",
        "revision_snapshot_id": "SNP-1",
        "entries": [{
            "local_task_id": "LCT-1",
            "local_change_id": "C-01",
            "local_change_ref": "LCT-1/C-01",
            "task_id": "RCT-1",
            "change_ref": "RCT-1/C-01",
            "receipt_patchset_id": "RCT-1/C-01/P-01"
        }],
        "aggregate": {
            "task_id": "RCT-1",
            "change_ref": "RCT-1/C-01",
            "patchset_id": "RCT-1/C-01/P-02",
            "patchset": {
                "patchset_id": "RCT-1/C-01/P-02",
                "base_snapshot_id": "SNP-0",
                "revision_snapshot_id": "SNP-1",
                "source_kind": "history_promotion_aggregate",
                "governance_authority": true
            }
        }
    })
}

fn staged_history_request(final_stage: bool) -> JsonValue {
    let mut request = history_request();
    request["contract"] = json!("history-promotion-prepare/v2");
    request["promotion_id"] = json!("history-promotion-v2:stable");
    request["idempotency_key"] = json!(if final_stage {
        "history-promotion-stage:final"
    } else {
        "history-promotion-stage:first"
    });
    request["revision_snapshot_id"] = json!("SNP-65");
    request["stage_ordinal"] = json!(if final_stage { 1 } else { 0 });
    request["stage_base_snapshot_id"] = json!(if final_stage { "SNP-64" } else { "SNP-0" });
    request["stage_revision_snapshot_id"] = json!(if final_stage { "SNP-65" } else { "SNP-64" });
    request["previous_stage_patchset_id"] = if final_stage {
        json!("RCT-64/C-01/P-02")
    } else {
        JsonValue::Null
    };
    request["total_entry_count"] = json!(65);
    request["final_stage"] = json!(final_stage);
    request["entries"][0]["local_task_id"] = json!(if final_stage { "LCT-65" } else { "LCT-64" });
    request["entries"][0]["local_change_ref"] = json!(if final_stage {
        "LCT-65/C-01"
    } else {
        "LCT-64/C-01"
    });
    request["entries"][0]["pre_land_target_snapshot_id"] =
        request["stage_base_snapshot_id"].clone();
    request["entries"][0]["landed_snapshot_id"] = request["stage_revision_snapshot_id"].clone();
    request
}

fn staged_history_response(final_stage: bool) -> JsonValue {
    let request = staged_history_request(final_stage);
    let remote_task_id = if final_stage { "RCT-65" } else { "RCT-64" };
    let stage_patchset_id = format!("{remote_task_id}/C-01/P-02");
    let stage = json!({
        "task_id": remote_task_id,
        "change_ref": format!("{remote_task_id}/C-01"),
        "patchset_id": stage_patchset_id,
        "patchset": {
            "patchset_id": stage_patchset_id,
            "base_snapshot_id": "SNP-0",
            "revision_snapshot_id": if final_stage { "SNP-65" } else { "SNP-64" },
            "source_kind": if final_stage {
                "history_promotion_aggregate"
            } else {
                "history_promotion_stage"
            },
            "governance_authority": final_stage
        }
    });
    json!({
        "contract": "history-promotion-prepare/v2",
        "repo_name": "repo",
        "repository_index": 7,
        "promotion_id": request["promotion_id"],
        "idempotency_key": request["idempotency_key"],
        "replayed": false,
        "target_line": "main",
        "base_snapshot_id": "SNP-0",
        "revision_snapshot_id": "SNP-65",
        "stage_ordinal": request["stage_ordinal"],
        "stage_base_snapshot_id": request["stage_base_snapshot_id"],
        "stage_revision_snapshot_id": request["stage_revision_snapshot_id"],
        "previous_stage_patchset_id": request["previous_stage_patchset_id"],
        "total_entry_count": 65,
        "final_stage": final_stage,
        "entries": [{
            "local_task_id": request["entries"][0]["local_task_id"],
            "local_change_id": "C-01",
            "local_change_ref": request["entries"][0]["local_change_ref"],
            "task_id": remote_task_id,
            "change_ref": format!("{remote_task_id}/C-01"),
            "receipt_patchset_id": format!("{remote_task_id}/C-01/P-01")
        }],
        "stage": stage,
        "aggregate": if final_stage { stage } else { JsonValue::Null }
    })
}

#[test]
fn history_promotion_uses_one_repository_authority_request() {
    let request = history_request();
    let (mut config, server) = serve_task_workflow_json_once(history_response());
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let spec_config = config.clone();
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

    let response = remote
        .prepare_history_promotion("repo", &request)
        .expect("history prepare");
    assert_eq!(
        response["aggregate"]["patchset"]["source_kind"],
        "history_promotion_aggregate"
    );

    let recorded = server.join().unwrap();
    assert_eq!(recorded.method, "POST");
    assert_eq!(
        recorded.target,
        "/v1/native/repository-authorities/7/history-promotion:prepare"
    );
    assert_eq!(recorded.body, Some(request.clone()));

    let spec =
        build_prepare_history_promotion_request_spec(&spec_config, "repo", &request).unwrap();
    assert_eq!(spec.body, Some(request));
    assert_eq!(
        spec.path,
        "/v1/native/repository-authorities/7/history-promotion:prepare"
    );
}

#[test]
fn staged_history_promotion_validates_intermediate_and_final_authority() {
    for final_stage in [false, true] {
        let request = staged_history_request(final_stage);
        let (mut config, server) =
            serve_task_workflow_json_once(staged_history_response(final_stage));
        config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
        let spec_config = config.clone();
        let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

        let response = remote
            .prepare_history_promotion("repo", &request)
            .expect("validate staged history response");
        assert_eq!(response["contract"], "history-promotion-prepare/v2");
        assert_eq!(response["final_stage"], final_stage);
        assert_eq!(response["aggregate"].is_object(), final_stage);
        assert_eq!(
            response["stage"]["patchset"]["governance_authority"],
            final_stage
        );

        let recorded = server.join().unwrap();
        assert_eq!(recorded.body, Some(request.clone()));
        let spec =
            build_prepare_history_promotion_request_spec(&spec_config, "repo", &request).unwrap();
        assert_eq!(spec.body, Some(request));
    }
}

#[test]
fn staged_history_promotion_rejects_early_or_divergent_aggregate_authority() {
    let request = staged_history_request(false);
    let mut response = staged_history_response(false);
    response["aggregate"] = response["stage"].clone();
    let (mut config, server) = serve_task_workflow_json_once(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
    let error = remote
        .prepare_history_promotion("repo", &request)
        .expect_err("intermediate aggregate authority must fail closed");
    assert!(error.to_string().contains("must not expose aggregate"));
    server.join().unwrap();

    let request = staged_history_request(true);
    let mut response = staged_history_response(true);
    response["aggregate"]["patchset_id"] = json!("RCT-65/C-01/P-99");
    let (mut config, server) = serve_task_workflow_json_once(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
    let error = remote
        .prepare_history_promotion("repo", &request)
        .expect_err("final stage and aggregate divergence must fail closed");
    assert!(error.to_string().contains("disagree"));
    server.join().unwrap();
}

#[test]
fn history_promotion_timeout_retries_the_identical_request() {
    let request = history_request();
    let mut response = history_response();
    response["replayed"] = json!(true);
    let (mut config, server) = serve_task_workflow_timeout_then_json(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

    let response = remote
        .prepare_history_promotion("repo", &request)
        .expect("idempotent history retry");
    assert_eq!(response["replayed"], true);

    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].target, requests[1].target);
    assert_eq!(requests[0].body, requests[1].body);
    assert_eq!(requests[0].body, Some(request));
}

#[test]
fn history_promotion_rejects_mapping_reordering() {
    let request = history_request();
    let mut response = history_response();
    response["entries"][0]["local_change_id"] = json!("C-02");
    let (mut config, server) = serve_task_workflow_json_once(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

    let error = remote
        .prepare_history_promotion("repo", &request)
        .expect_err("mapping mismatch must fail closed");
    assert!(error.to_string().contains("local_change_id"));
    server.join().unwrap();
}

#[test]
fn history_promotion_rejects_duplicate_or_non_authoritative_remote_mappings() {
    let mut request = history_request();
    let second_request_entry = request["entries"][0].clone();
    request["entries"]
        .as_array_mut()
        .unwrap()
        .push(second_request_entry);
    request["entries"][1]["local_task_id"] = json!("LCT-2");
    request["entries"][1]["local_change_ref"] = json!("LCT-2/C-01");
    let mut response = history_response();
    response["entries"].as_array_mut().unwrap().push(json!({
        "local_task_id": "LCT-2",
        "local_change_id": "C-01",
        "local_change_ref": "LCT-2/C-01",
        "task_id": "RCT-1",
        "change_ref": "RCT-1/C-01",
        "receipt_patchset_id": "RCT-1/C-01/P-01"
    }));
    let (mut config, server) = serve_task_workflow_json_once(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();

    let duplicate_error = remote
        .prepare_history_promotion("repo", &request)
        .expect_err("duplicate canonical mapping must fail closed");
    assert!(duplicate_error.to_string().contains("repeats canonical"));
    server.join().unwrap();

    let request = history_request();
    let mut response = history_response();
    response["aggregate"]["patchset"]["governance_authority"] = json!(false);
    let (mut config, server) = serve_task_workflow_json_once(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
    let authority_error = remote
        .prepare_history_promotion("repo", &request)
        .expect_err("aggregate without governance authority must fail closed");
    assert!(authority_error.to_string().contains("governance authority"));
    server.join().unwrap();

    let request = history_request();
    let mut response = history_response();
    response["aggregate"]["patchset"]["revision_snapshot_id"] = json!("SNP-OTHER");
    let (mut config, server) = serve_task_workflow_json_once(response);
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
    let revision_error = remote
        .prepare_history_promotion("repo", &request)
        .expect_err("aggregate revision drift must fail closed");
    assert!(revision_error.to_string().contains("revision_snapshot_id"));
    server.join().unwrap();
}
