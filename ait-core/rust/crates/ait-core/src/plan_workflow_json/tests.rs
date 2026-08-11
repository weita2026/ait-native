use super::PlanWorkflowJson;
use crate::json_support::json;

#[test]
fn stateless_wrapper_preserves_plan_task_and_command_contracts() {
    let json = PlanWorkflowJson::stateless();

    let remote_request = json
        .normalize_plan_remote_request_payload_json(
            &json!({
                "operation": "get_revision",
                "transport": {"base_url": "https://example.test"},
                "plan_id": "PLAN-1",
                "plan_revision_id": "REV-1"
            })
            .to_string(),
        )
        .expect("remote request");
    assert_eq!(remote_request["operation"], "get_revision");
    assert_eq!(remote_request["plan_id"], "PLAN-1");

    let task_title = json
        .build_task_tracking_title_payload(&json!({
            "task_id": "RCT-1",
            "title": "Implement"
        }))
        .expect("task title");
    assert_eq!(task_title, json!({"title": "RCT-1: Implement"}));

    let command_payload = json
        .build_plan_list_command_payload_json(
            &json!({
                "scope": "local",
                "repo_name": "ait",
                "plans": [{"plan_id": "PLAN-1"}]
            })
            .to_string(),
        )
        .expect("plan list command payload");
    assert_eq!(command_payload, json!([{"plan_id": "PLAN-1"}]));
}

#[test]
fn stateless_wrapper_projects_http_request_specs_as_stable_json() {
    let json = PlanWorkflowJson::stateless();
    let spec = crate::plan_http_client::PlanHttpRequestSpec {
        method: "GET".to_string(),
        path: "/v1/native/repository-authorities/0/tasks".to_string(),
        url: "https://example.test/v1/native/repository-authorities/0/tasks".to_string(),
        query_pairs: vec![("status".to_string(), "active".to_string())],
        headers: std::collections::BTreeMap::from([(
            "Accept".to_string(),
            "application/json".to_string(),
        )]),
        body: None,
        timeout_ms: 30_000,
    };

    assert_eq!(
        json.task_workflow_http_request_spec_payload(&spec),
        crate::json_support::json!({
            "method": "GET",
            "path": "/v1/native/repository-authorities/0/tasks",
            "url": "https://example.test/v1/native/repository-authorities/0/tasks",
            "query_pairs": [{"name": "status", "value": "active"}],
            "headers": {"Accept": "application/json"},
            "body": null,
            "timeout_ms": 30000,
        })
    );
}
