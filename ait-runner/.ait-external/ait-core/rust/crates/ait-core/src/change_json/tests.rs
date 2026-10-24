use super::ChangeJson;
use crate::json_support::json;
use crate::plan_http_client::PlanHttpClientConfig;
use crate::server_operational::RepositoryIndex;

#[test]
fn change_json_builds_create_change_request_spec() {
    let json = ChangeJson::stateless();
    let config = PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(RepositoryIndex::new(7)),
        default_timeout_ms: 5000,
        ..PlanHttpClientConfig::default()
    };

    let spec = json
        .build_create_change_request_spec(
            &config,
            "repo",
            "RCT-1",
            "Implement wrapper",
            "main",
            Some("RCC-1"),
            Some("SNP-1"),
            Some("feature/base"),
        )
        .expect("create change spec");

    assert_eq!(spec.method, "POST");
    assert_eq!(spec.path, "/v1/native/repository-authorities/7/changes");
    assert_eq!(
        spec.body,
        Some(json!({
            "task_id": "RCT-1",
            "title": "Implement wrapper",
            "base_line": "main",
            "change_id": "RCC-1",
            "fork_snapshot_id": "SNP-1",
            "forked_from_line": "feature/base",
        }))
    );
}

#[test]
fn change_json_builds_read_and_close_request_specs() {
    let json = ChangeJson::stateless();
    let config = PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(RepositoryIndex::new(7)),
        default_timeout_ms: 5000,
        ..PlanHttpClientConfig::default()
    };

    let list = json
        .build_list_changes_request_spec(&config, "repo")
        .expect("list changes spec");
    assert_eq!(list.method, "GET");
    assert_eq!(list.path, "/v1/native/repository-authorities/7/changes");

    let detail = json
        .build_get_change_detail_request_spec(&config, "RCC-1", Some("repo"))
        .expect("change detail spec");
    assert_eq!(detail.method, "GET");
    assert_eq!(
        detail.path,
        "/v1/native/repository-authorities/7/changes/RCC-1"
    );

    let read = json
        .build_get_change_request_spec(&config, "RCC-1", None)
        .expect("change read spec");
    assert_eq!(read.method, "GET");
    assert_eq!(
        read.path,
        "/v1/native/repository-authorities/7/changes/RCC-1"
    );

    let close = json
        .build_close_change_request_spec(&config, "RCC-1", "archived")
        .expect("close change spec");
    assert_eq!(close.method, "POST");
    assert_eq!(
        close.path,
        "/v1/native/repository-authorities/7/changes/RCC-1:close"
    );
    assert_eq!(close.body, Some(json!({ "status": "archived" })));
}

#[test]
fn change_json_prefers_repository_authority_for_closeout_paths() {
    let json = ChangeJson::stateless();
    let config = PlanHttpClientConfig {
        base_url: "https://example.test".to_string(),
        repository_index: Some(RepositoryIndex::new(7)),
        ..PlanHttpClientConfig::default()
    };

    assert_eq!(
        json.build_list_changes_request_spec(&config, "legacy-name")
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/changes"
    );
    assert_eq!(
        json.build_get_change_request_spec(&config, "RCT-1/C-01", Some("legacy-name"))
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/changes/RCT-1%2FC-01"
    );
    assert_eq!(
        json.build_close_change_request_spec(&config, "RCT-1/C-01", "archived")
            .unwrap()
            .path,
        "/v1/native/repository-authorities/7/changes/RCT-1%2FC-01:close"
    );
}

#[test]
fn change_json_owns_linked_change_lookup_payload() {
    let json = ChangeJson::stateless();

    let payload = json
        .build_linked_change_lookup_payload(Some(&json!([
            {
                "task_id": "RCT-1",
                "changes": [{"change_id": "RCC-1"}, {"change_id": "RCC-2"}]
            }
        ])))
        .expect("linked change payload");

    assert_eq!(payload["linked_change_count"], 2);
    assert_eq!(payload["change_links_by_task"][0]["task_id"], "RCT-1");
    assert_eq!(
        payload["change_links_by_task"][0]["changes"][1]["change_id"],
        "RCC-2"
    );
}

#[test]
fn change_json_recovers_land_submission_from_change_state() {
    let json = ChangeJson::stateless();

    let recovered = json
        .recover_land_submission_from_change_state(
            &json!({
                "change_id": "RCC-1",
                "landing_summary": {
                    "status": "succeeded",
                    "submission_id": "LAND-1",
                    "result": {"landed_snapshot_id": "SNP-2"}
                }
            }),
            "RCC-FALLBACK",
        )
        .expect("recovered land submission");

    assert_eq!(recovered["status"], "succeeded");
    assert_eq!(recovered["submission_id"], "LAND-1");
    assert_eq!(recovered["result"]["landed_snapshot_id"], "SNP-2");
    assert_eq!(recovered["response_recovery"]["action"], "submit_land");
    assert_eq!(recovered["response_recovery"]["change_id"], "RCC-1");
}

#[test]
fn change_json_normalizes_rolling_composite_change_ids_without_new_identity_fields() {
    let json = ChangeJson::stateless();
    let normalized = json
        .normalize_remote_change_payload(
            &json!({
                "task_id": "RCT-1029",
                "change_id": "RCT-1029/C-01",
                "status": "draft"
            }),
            Some("RCT-1029"),
        )
        .expect("normalize rolling response");

    assert_eq!(normalized["task_id"], "RCT-1029");
    assert_eq!(normalized["change_id"], "C-01");
    assert_eq!(normalized["change_ref"], "RCT-1029/C-01");
    assert_eq!(
        json.rolling_server_change_id(Some("RCT-1029"), "C-01")
            .expect("rolling server locator"),
        "RCT-1029/C-01"
    );
    assert!(json
        .rolling_server_change_id(None, "C-01")
        .expect_err("short id without task context must fail")
        .contains("refusing an ambiguous repository-wide lookup"));
}

#[test]
fn change_json_normalizes_nested_change_detail_without_flattening_the_envelope() {
    let normalized = ChangeJson::stateless()
        .normalize_remote_change_detail_payload(
            &json!({
                "selected_patchset": {"patchset_id": "RP-1"},
                "change": {
                    "task_id": "RCT-1029",
                    "change_id": "RCT-1029/C-01",
                    "status": "review"
                }
            }),
            Some("RCT-1029"),
        )
        .expect("normalize nested change detail");

    assert_eq!(normalized["selected_patchset"]["patchset_id"], "RP-1");
    assert_eq!(normalized["change"]["change_id"], "C-01");
    assert_eq!(normalized["change"]["change_ref"], "RCT-1029/C-01");
}

#[test]
fn change_json_normalizes_task_audit_change_rows_with_task_context() {
    let normalized = ChangeJson::stateless()
        .normalize_remote_task_audit_payload(
            &json!({
                "task_id": "RCT-1029",
                "changes": [
                    {
                        "task_id": "RCT-1029",
                        "change_id": "RCT-1029/C-01",
                        "status": "landed"
                    },
                    {
                        "change": {
                            "task_id": "RCT-1029",
                            "change_id": "RCT-1029/C-02",
                            "status": "draft"
                        },
                        "readiness": {"state": "pending"}
                    }
                ]
            }),
            "RCT-1029",
        )
        .expect("normalize Task audit");

    assert_eq!(normalized["changes"][0]["change_id"], "C-01");
    assert_eq!(normalized["changes"][0]["change_ref"], "RCT-1029/C-01");
    assert_eq!(normalized["changes"][1]["change"]["change_id"], "C-02");
    assert_eq!(
        normalized["changes"][1]["change"]["change_ref"],
        "RCT-1029/C-02"
    );
    assert_eq!(normalized["changes"][1]["readiness"]["state"], "pending");
}

#[test]
fn change_json_rejects_task_audit_changes_from_another_task() {
    let error = ChangeJson::stateless()
        .normalize_remote_task_audit_payload(
            &json!({
                "task_id": "RCT-1029",
                "changes": [{
                    "task_id": "RCT-OTHER",
                    "change_id": "RCT-OTHER/C-01"
                }]
            }),
            "RCT-1029",
        )
        .expect_err("mismatched Task audit Change must fail closed");

    assert!(error.contains("not expected task `RCT-1029`"));
}

#[test]
fn change_json_rejects_composite_ids_from_an_unrelated_task() {
    let error = ChangeJson::stateless()
        .normalize_remote_change_payload(
            &json!({
                "task_id": "RCT-OTHER",
                "change_id": "RCT-OTHER/C-01"
            }),
            Some("RCT-1029"),
        )
        .expect_err("unrelated task must fail closed");

    assert!(error.contains("not expected task `RCT-1029`"));
}

#[test]
fn change_json_keeps_same_short_id_isolated_by_task_context() {
    let json = ChangeJson::stateless();
    let first = json
        .normalize_remote_change_payload(
            &json!({"task_id": "RCT-1", "change_id": "RCT-1/C-01"}),
            Some("RCT-1"),
        )
        .expect("first task change");
    let second = json
        .normalize_remote_change_payload(
            &json!({"task_id": "RCT-2", "change_id": "RCT-2/C-01"}),
            Some("RCT-2"),
        )
        .expect("second task change");

    assert_eq!(first["change_id"], "C-01");
    assert_eq!(second["change_id"], "C-01");
    assert_eq!(first["change_ref"], "RCT-1/C-01");
    assert_eq!(second["change_ref"], "RCT-2/C-01");
    assert_ne!(first["task_id"], second["task_id"]);
    assert_eq!(
        json.rolling_server_change_id(Some("RCT-1"), "C-01")
            .expect("first task wire id"),
        "RCT-1/C-01"
    );
    assert_eq!(
        json.rolling_server_change_id(Some("RCT-2"), "C-01")
            .expect("second task wire id"),
        "RCT-2/C-01"
    );
}
