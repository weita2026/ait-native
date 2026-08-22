use super::*;
use crate::plan_http_client::PlanHttpClientConfig;
use crate::policy_json::PolicyJson;
use crate::task_workflow_http_adapter::HttpWorkflowCloseoutRemote;
use std::collections::BTreeMap;

fn policy_plan_http_config() -> PlanHttpClientConfig {
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
fn policy_json_builds_request_specs_and_waiver_body() {
    let policy_json = PolicyJson::stateless();

    let evaluate = policy_json
        .build_evaluate_policy_request_spec(&policy_plan_http_config(), " LCP-1 ")
        .unwrap();
    assert_eq!(evaluate.method, "POST");
    assert_eq!(
        evaluate.path,
        "/v1/native/repository-authorities/7/patchsets/LCP-1:evaluatePolicy"
    );
    assert_eq!(evaluate.body, Some(json!({})));

    let get = policy_json
        .build_get_policy_request_spec(&policy_plan_http_config(), " LCP-1 ")
        .unwrap();
    assert_eq!(get.method, "GET");
    assert_eq!(
        get.path,
        "/v1/native/repository-authorities/7/patchsets/LCP-1/policy"
    );
    assert!(get.body.is_none());

    let waiver = policy_json
        .build_create_waiver_request_spec(
            &policy_plan_http_config(),
            " LCP-1 ",
            " tests.required ",
            " external gate covers this ",
            Some(" 2026-07-04T00:00:00Z "),
        )
        .unwrap();
    assert_eq!(waiver.method, "POST");
    assert_eq!(
        waiver.path,
        "/v1/native/repository-authorities/7/patchsets/LCP-1/waivers"
    );
    assert_eq!(
        waiver.body,
        Some(json!({
            "rule_name": "tests.required",
            "reason": "external gate covers this",
            "expires_at": "2026-07-04T00:00:00Z",
        }))
    );
    assert_eq!(
        policy_json
            .build_create_waiver_body("tests.required", "covered", None)
            .unwrap()["expires_at"],
        JsonValue::Null
    );
}

#[test]
fn policy_json_prefers_repository_authority_paths() {
    let policy = PolicyJson::stateless();
    let mut config = policy_plan_http_config();
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));

    assert_eq!(
        policy
            .build_evaluate_policy_request_spec(&config, "P-RCT-1/C-01-1")
            .unwrap()
            .path,
        "/v1/native/repository-authorities/9/patchsets/P-RCT-1%2FC-01-1:evaluatePolicy"
    );
    assert_eq!(
        policy
            .build_get_policy_request_spec(&config, "P-RCT-1/C-01-1")
            .unwrap()
            .path,
        "/v1/native/repository-authorities/9/patchsets/P-RCT-1%2FC-01-1/policy"
    );
}

#[test]
fn policy_http_adapter_evaluates_through_repository_authority() {
    let (mut config, server) = serve_task_workflow_json_once(json!({
        "policy_id": "POL-1",
        "patchset_id": "P-RCT-1/C-01-1",
        "decision": "pass"
    }));
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
    remote
        .evaluate_policy("P-RCT-1/C-01-1", Some("legacy-name"), true)
        .unwrap();

    let recorded = server.join().unwrap();
    assert_eq!(recorded.method, "POST");
    assert_eq!(
        recorded.target,
        "/v1/native/repository-authorities/9/patchsets/P-RCT-1%2FC-01-1:evaluatePolicy"
    );
}

#[test]
fn policy_json_delegates_policy_profiles_and_yaml() {
    let policy_json = PolicyJson::stateless();

    let team = policy_json.policy_profile("team").unwrap();
    assert_eq!(team["policy_id"], "team");
    assert_eq!(team["defaults"]["require_attestation"], true);

    let normalized = policy_json
        .normalize_policy(
            Some(&json!({
                "policy_id": "custom",
                "defaults": {"require_tests": "false", "require_lint": "true"},
            })),
            "prototype",
        )
        .unwrap();
    assert_eq!(normalized["policy_id"], "custom");
    assert_eq!(normalized["defaults"]["require_tests"], false);
    assert_eq!(normalized["defaults"]["require_lint"], true);

    let effective = policy_json
        .resolve_effective_policy(
            Some(&team),
            Some("docs_only"),
            Some("human_only"),
            "prototype",
        )
        .unwrap();
    assert_eq!(
        effective["effective_requirements"]["require_tests"],
        JsonValue::Bool(false)
    );

    let yaml = policy_json
        .policy_to_yaml(Some(&team), "prototype")
        .unwrap();
    let reparsed = policy_json.parse_policy_yaml(&yaml, "prototype").unwrap();
    assert_eq!(reparsed["policy_id"], "team");
}

#[test]
fn task_workflow_policy_helpers_accept_policy_remote_trait() {
    let mut remote = FakePolicyRemote;
    let remote_port: &mut dyn TaskWorkflowPolicyRemote = &mut remote;

    assert_eq!(
        evaluate_policy_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            Some("repo"),
            true,
        )
        .unwrap()["evaluation_state"],
        "pass"
    );
    assert_eq!(
        get_policy_with_task_workflow_closeout_remote(remote_port, "P-C-1-1", Some("repo"), true,)
            .unwrap()["policy"],
        true
    );
    assert_eq!(
        create_waiver_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            "tests",
            "covered by external gate",
            Some("2026-07-04T00:00:00Z"),
            Some("repo"),
            true,
        )
        .unwrap()["waived"],
        true
    );
}

#[test]
fn task_workflow_policy_helpers_accept_single_capability_ports() {
    let mut evaluator = FakePolicyEvaluatorPort;
    let mut reader = FakePolicyReaderPort;
    let mut waiver_creator = FakePolicyWaiverCreatorPort;

    assert_eq!(
        evaluate_policy_with_task_workflow_closeout_remote(
            &mut evaluator,
            "P-C-1-1",
            Some("repo"),
            true,
        )
        .unwrap()["evaluation_state"],
        "pass"
    );
    assert_eq!(
        get_policy_with_task_workflow_closeout_remote(&mut reader, "P-C-1-1", Some("repo"), true,)
            .unwrap()["policy"],
        true
    );
    assert_eq!(
        create_waiver_with_task_workflow_closeout_remote(
            &mut waiver_creator,
            "P-C-1-1",
            "tests",
            "covered by external gate",
            Some("2026-07-04T00:00:00Z"),
            Some("repo"),
            true,
        )
        .unwrap()["waived"],
        true
    );
}
