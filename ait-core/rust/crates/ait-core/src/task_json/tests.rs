use super::TaskJson;
use crate::json_support::json;
use crate::plan_http_client::PlanHttpClientConfig;
use crate::server_operational::RepositoryIndex;

#[test]
fn task_json_builds_create_task_request_spec() {
    let json = TaskJson::stateless();
    let config = PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(RepositoryIndex::new(7)),
        default_timeout_ms: 5000,
        ..PlanHttpClientConfig::default()
    };

    let spec = json
        .build_create_task_request_spec(
            &config,
            "repo",
            "Add JSON fixtures",
            "Lock adapter JSON",
            Some("T-2"),
            Some("PLAN-1"),
            Some("REV-1"),
            Some("1"),
        )
        .expect("create task spec");

    assert_eq!(spec.method, "POST");
    assert_eq!(spec.path, "/v1/native/repository-authorities/7/tasks");
    assert_eq!(
        spec.body,
        Some(json!({
            "title": "Add JSON fixtures",
            "intent": "Lock adapter JSON",
            "task_id": "T-2",
            "plan_id": "PLAN-1",
            "origin_plan_revision_id": "REV-1",
            "plan_item_ref": "1",
        }))
    );
}

#[test]
fn task_json_prefers_repository_authority_for_closeout_reads_and_writes() {
    let json = TaskJson::stateless();
    let authority_config = PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(RepositoryIndex::new(7)),
        ..PlanHttpClientConfig::default()
    };

    let get = json
        .build_get_task_request_spec(&authority_config, "RCT-1", Some("legacy-name"))
        .unwrap();
    assert_eq!(get.path, "/v1/native/repository-authorities/7/tasks/RCT-1");
    let close = json
        .build_close_task_request_spec(&authority_config, "RCT-1", "completed")
        .unwrap();
    assert_eq!(
        close.path,
        "/v1/native/repository-authorities/7/tasks/RCT-1:close"
    );

    let legacy_config = PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        ..PlanHttpClientConfig::default()
    };
    assert!(json
        .build_get_task_request_spec(&legacy_config, "RCT-1", Some("repo"))
        .unwrap_err()
        .to_string()
        .contains("repository_index is required"));
    assert!(json
        .build_close_task_request_spec(&legacy_config, "RCT-1", "completed")
        .unwrap_err()
        .to_string()
        .contains("repository_index is required"));
}

#[test]
fn task_json_owns_tracking_and_linked_task_payloads() {
    let json = TaskJson::stateless();
    let title = json
        .build_task_tracking_title_payload(&json!({
            "task_id": "LT-1",
            "title": "Demo task"
        }))
        .expect("title payload");
    assert_eq!(title, json!({"title": "LT-1: Demo task"}));

    let linked = json
        .build_linked_task_lookup_payload(
            Some(&json!([{
                "plan_id": "PLAN-1",
                "plan_item_ref": "PLAN-1/item-1",
                "tasks": [{"task_id": "RCT-1"}]
            }])),
            Some(&json!([{
                "plan_id": "PLAN-1",
                "tasks": [{"task_id": "RCT-1"}]
            }])),
        )
        .expect("linked tasks");
    assert_eq!(linked["linked_task_count"], 1);
}

#[test]
fn task_json_recovers_closed_task_response() {
    let json = TaskJson::stateless();
    assert_eq!(
        json.recover_closed_task_from_state(
            &json!({
                "task_id": "T-1",
                "status": "completed"
            }),
            "fallback",
        ),
        Some(json!({
            "task_id": "T-1",
            "status": "completed",
            "response_recovery": {
                "action": "complete_task",
                "state": "recovered_from_remote_task_status",
                "task_id": "T-1",
            }
        }))
    );
}
