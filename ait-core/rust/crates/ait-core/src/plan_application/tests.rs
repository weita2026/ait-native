use super::{
    build_plan_candidates_service_payload_json, build_plan_inspect_service_payload_json,
    build_plan_items_service_payload_json, build_plan_list_service_payload_json,
    build_plan_revisions_service_payload_json, build_plan_show_service_payload_json,
    build_plan_sync_service_payload_json, normalize_plan_candidates_service_request_payload_json,
    normalize_plan_list_service_request_payload_json,
};
use crate::json_support::{json, JsonValue};

fn sample_plan() -> JsonValue {
    json!({
        "plan_id": "PL-1",
        "title": "Demo",
        "status": "draft",
        "repo_name": "ait",
        "publication_state": "published",
        "published_plan_id": "PL-1",
        "published_head_revision_id": "PR-1",
        "head_revision_id": "PR-2",
        "head_revision": {
            "plan_revision_id": "PR-2",
            "revision_number": 2,
            "artifact_path": "docs/sprints/demo.md",
            "artifact_selector": "demo/root",
            "artifact_heading": "Demo",
            "publication_state": "local_draft",
            "items": [
                {
                    "plan_item_ref": "demo/linked",
                    "text": "linked",
                    "checkbox_state": "open",
                    "heading_path": ["Demo"],
                    "line_number": 10
                },
                {
                    "plan_item_ref": "demo/taskable",
                    "text": "taskable",
                    "checkbox_state": "open",
                    "heading_path": ["Demo"],
                    "line_number": 11
                }
            ]
        }
    })
}

fn sample_tasks() -> JsonValue {
    json!([
        {
            "task_id": "RT-1",
            "title": "Linked task",
            "status": "active",
            "planning_state": "planned",
            "origin_plan_revision_id": "PR-2",
            "plan_drift_state": null,
            "plan_id": "PL-1",
            "plan_item_ref": "demo/linked"
        }
    ])
}

#[test]
fn list_request_normalization_requires_query_scope() {
    let error = normalize_plan_list_service_request_payload_json(
        r#"{"scope":"directory","repo_name":"ait","plans":[]}"#,
    )
    .expect_err("invalid scope should fail");
    assert!(error.contains("must be one of"));
}

#[test]
fn list_show_and_revisions_build_round_trip() {
    let list_payload = build_plan_list_service_payload_json(
        &json!({
            "scope": "local",
            "repo_name": "ait",
            "plans": [sample_plan()],
        })
        .to_string(),
    )
    .expect("list payload");
    assert_eq!(list_payload["plans"][0]["plan_id"], "PL-1");

    let show_payload = build_plan_show_service_payload_json(
        &json!({
            "scope": "local",
            "repo_name": "ait",
            "plan": sample_plan(),
            "revision": sample_plan()["head_revision"],
        })
        .to_string(),
    )
    .expect("show payload");
    assert_eq!(show_payload["revision"]["plan_revision_id"], "PR-2");

    let revisions_payload = build_plan_revisions_service_payload_json(
        &json!({
            "scope": "remote",
            "repo_name": "ait",
            "remote": "origin",
            "plan_id": "PL-1",
            "revisions": [sample_plan()["head_revision"]],
        })
        .to_string(),
    )
    .expect("revisions payload");
    assert_eq!(
        revisions_payload["revisions"][0]["plan_revision_id"],
        "PR-2"
    );
}

#[test]
fn items_builder_keeps_identity_contract() {
    let payload = build_plan_items_service_payload_json(
        &json!({
            "scope": "local",
            "repo_name": "ait",
            "plan": sample_plan(),
            "revision": sample_plan()["head_revision"],
        })
        .to_string(),
    )
    .expect("items payload");
    assert_eq!(payload["plan"]["plan_id"], "PL-1");
    assert_eq!(payload["plan"]["item_count"], 2);
    assert_eq!(payload["plan"]["identity_only"], true);
    assert_eq!(payload["plan"]["dispatch_validation_required"], true);
}

#[test]
fn candidates_and_inspect_build_dispatch_summaries() {
    let request = json!({
        "scope": "remote",
        "repo_name": "ait",
        "remote": "origin",
        "plans": [sample_plan()],
        "tasks": sample_tasks(),
        "include_all": true,
        "local_shadow_index": {
            "PL-1": {
                "plan_id": "PL-1",
                "publication_state": "published",
                "head_publication_state": "local_draft",
                "head_revision_id": "PR-2",
                "head_revision_number": 2,
                "published_plan_id": "PL-1",
                "published_head_revision_id": "PR-1",
                "unpublished_head": true
            }
        }
    });
    let normalized = normalize_plan_candidates_service_request_payload_json(&request.to_string())
        .expect("normalized request");
    assert_eq!(
        normalized["local_shadow_index"]["PL-1"]["unpublished_head"],
        true
    );

    let candidates = build_plan_candidates_service_payload_json(&request.to_string())
        .expect("candidate payload");
    assert_eq!(candidates["summary"]["candidate_plan_count"], 1);
    assert_eq!(candidates["candidates"][0]["linked_task_count"], 1);
    assert_eq!(candidates["candidates"][0]["local_unpublished_head"], true);

    let inspect = build_plan_inspect_service_payload_json(
        &json!({
            "scope": "remote",
            "repo_name": "ait",
            "remote": "origin",
            "plan": sample_plan(),
            "tasks": sample_tasks(),
            "local_shadow": request["local_shadow_index"]["PL-1"],
        })
        .to_string(),
    )
    .expect("inspect payload");
    assert_eq!(inspect["plan"]["taskable_item_count"], 1);
    assert_eq!(inspect["plan"]["linked_open_item_count"], 1);
}

#[test]
fn candidates_contains_matches_linked_item_without_changing_taskability() {
    let mut plan = sample_plan();
    plan["head_revision"]["items"] = json!([{
        "plan_item_ref": "linked-only/ref",
        "text": "Linked-only work",
        "checkbox_state": "open",
        "heading_path": ["Linked-only heading"],
        "line_number": 10
    }]);
    let tasks = json!([{
        "task_id": "RT-1",
        "title": "Completed linked task",
        "status": "completed",
        "planning_state": "planned",
        "origin_plan_revision_id": "PR-2",
        "plan_id": "PL-1",
        "plan_item_ref": "linked-only/ref"
    }]);
    let mut request = json!({
        "scope": "remote",
        "repo_name": "ait",
        "remote": "origin",
        "plans": [plan],
        "tasks": tasks,
        "include_all": true,
        "contains_terms": ["LINKED-ONLY/REF"],
        "local_shadow_index": {}
    });

    let included = build_plan_candidates_service_payload_json(&request.to_string())
        .expect("include-all linked candidate payload");
    assert_eq!(included["summary"]["scanned_plan_count"], 1);
    assert_eq!(included["summary"]["candidate_plan_count"], 1);
    assert_eq!(included["candidates"][0]["taskable_item_count"], 0);
    assert_eq!(included["candidates"][0]["linked_task_count"], 1);

    request["include_all"] = json!(false);
    let taskable_only = build_plan_candidates_service_payload_json(&request.to_string())
        .expect("taskable-only linked candidate payload");
    assert_eq!(taskable_only["summary"]["scanned_plan_count"], 1);
    assert_eq!(taskable_only["summary"]["candidate_plan_count"], 0);
}

#[test]
fn candidates_contains_matches_done_item_text_and_heading_path() {
    let mut plan = sample_plan();
    plan["head_revision"]["items"] = json!([{
        "plan_item_ref": "done-only/ref",
        "text": "Retired transport cleanup",
        "checkbox_state": "done",
        "heading_path": ["Unique Completion Heading"],
        "line_number": 10
    }]);
    for term in ["TRANSPORT CLEANUP", "completion heading"] {
        let request = json!({
            "scope": "remote",
            "repo_name": "ait",
            "remote": "origin",
            "plans": [plan.clone()],
            "tasks": [],
            "include_all": true,
            "contains_terms": [term],
            "local_shadow_index": {}
        });
        let candidates = build_plan_candidates_service_payload_json(&request.to_string())
            .expect("done-item candidate payload");
        assert_eq!(
            candidates["summary"]["candidate_plan_count"], 1,
            "expected {term:?} to match the done item"
        );
        assert_eq!(candidates["candidates"][0]["done_item_count"], 1);
        assert_eq!(candidates["candidates"][0]["taskable_item_count"], 0);
    }
}

#[test]
fn sync_builder_computes_summary_and_optional_fields() {
    let payload = build_plan_sync_service_payload_json(
        &json!({
            "target": "docs/sprints",
            "scope": "directory",
            "mode": "local_publish",
            "status": "partial_success",
            "results": [
                {"action": "created", "plan_id": "PL-1"},
                {"action": "updated", "plan_id": "PL-2"},
                {"action": "unchanged", "plan_id": "PL-2"},
                {"action": "pruned", "plan_id": "PL-3"}
            ],
            "adoptions": [{"plan_id": "PL-1"}],
            "publish_results": [{"plan_id": "PL-1"}],
            "artifact_results": [{"artifact_path": "docs/sprints/demo.evidence.json"}],
            "advisory": {"plan_ids": ["PL-1", "PL-2"]},
            "error": {"message": "boom", "stage": "sync"}
        })
        .to_string(),
    )
    .expect("sync payload");
    assert_eq!(payload["summary"]["created_count"], 1);
    assert_eq!(payload["summary"]["updated_count"], 1);
    assert_eq!(payload["summary"]["unchanged_count"], 1);
    assert_eq!(payload["summary"]["pruned_count"], 1);
    assert_eq!(payload["summary"]["adopted_count"], 1);
    assert_eq!(payload["task_start_advisory"]["plan_ids"][0], "PL-1");
    assert_eq!(payload["error"]["stage"], "sync");
}
