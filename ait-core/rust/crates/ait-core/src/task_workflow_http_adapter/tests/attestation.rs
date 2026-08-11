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
    let evaluation = json!({"tests": "pass"});
    let provenance = json!({"policy_readable": true});
    let detail = json!({"minimum_evidence": {"policy_readable": true}});

    let put = attest
        .build_put_attestation_request_spec(
            &plan_http_config(),
            " RCP-1 ",
            " ai_with_human_review ",
            &evaluation,
            &provenance,
            &detail,
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
            "evaluation_summary": evaluation,
            "provenance_summary": provenance,
            "detail": detail,
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
            &json!({}),
            &json!({}),
            &json!({}),
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

#[test]
fn attestation_http_adapter_writes_through_repository_authority() {
    let (mut config, server) = serve_task_workflow_json_once(json!({
        "attestation_id": "ATT-1",
        "patchset_id": "P-RCT-1/C-01-1"
    }));
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
    remote
        .put_attestation(
            "P-RCT-1/C-01-1",
            "ai_with_human_review",
            &json!({"tests": "pass"}),
            &json!({}),
            &json!({}),
            Some("legacy-name"),
            true,
        )
        .unwrap();

    let recorded = server.join().unwrap();
    assert_eq!(recorded.method, "PUT");
    assert_eq!(
        recorded.target,
        "/v1/native/repository-authorities/9/patchsets/P-RCT-1%2FC-01-1/attestation"
    );
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
fn attest_json_normalizes_payload_boundaries() {
    let attest = AttestJson::stateless();
    let payload = attest
        .normalize_attestation_payload_json(r#"{"attestation_id":"AT-1"}"#)
        .unwrap();
    assert_eq!(payload["attestation_id"], json!("AT-1"));
    assert!(attest
        .normalize_attestation_payload_json(r#"[{"attestation_id":"AT-1"}]"#)
        .is_err());
    assert!(attest
        .normalize_evaluation_summary_payload(&json!(["not", "object"]))
        .is_err());
}

#[test]
fn attest_json_extracts_field_helpers_and_tests_state() {
    let attest = AttestJson::stateless();
    let payload = json!({
        "attestation_id": " AT-1 ",
        "patchset_id": " RCP-1 ",
        "author_mode": " ai_with_human_review ",
        "evaluation_summary": {
            "tests": " pass "
        }
    });

    assert_eq!(
        attest.optional_attestation_id(&payload).as_deref(),
        Some("AT-1")
    );
    assert_eq!(
        attest.optional_patchset_id(&payload).as_deref(),
        Some("RCP-1")
    );
    assert_eq!(
        attest.optional_author_mode(&payload).as_deref(),
        Some("ai_with_human_review")
    );
    assert_eq!(
        attest
            .tests_state_from_attestation(Some(&payload))
            .as_deref(),
        Some("pass")
    );
    assert_eq!(attest.tests_state_from_attestation(None), None);
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
            &json!({"state": "pass"}),
            &json!({"source": "test"}),
            &json!({"detail": true}),
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
            &json!({"state": "pass"}),
            &json!({"source": "test"}),
            &json!({"detail": true}),
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
