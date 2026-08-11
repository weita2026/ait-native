use super::{
    normalize_artifact_publish_request_payload_json, normalize_plan_remote_request_payload_json,
    normalize_plan_store_read_request_payload_json,
};
use crate::json_support::json;

#[test]
fn plan_store_request_reports_exact_required_field_errors() {
    assert_eq!(
        normalize_plan_store_read_request_payload_json(
            &json!({
                "operation": "get_plan",
                "plan_storage": {"mode": "binary"}
            })
            .to_string()
        )
        .expect_err("missing plan id"),
        "Plan store request `get_plan` must include plan_id."
    );
    assert_eq!(
        normalize_plan_store_read_request_payload_json(
            &json!({
                "operation": "get_revision_by_id",
                "plan_storage": {"mode": "binary"}
            })
            .to_string()
        )
        .expect_err("missing revision id"),
        "Plan store request `get_revision_by_id` must include plan_revision_id."
    );
}

#[test]
fn artifact_publish_request_reports_exact_required_field_errors() {
    assert_eq!(
        normalize_artifact_publish_request_payload_json(
            &json!({
                "plan_revision_id": "REV-1",
                "artifacts": []
            })
            .to_string()
        )
        .expect_err("missing plan id"),
        "Payload field `plan_id` must be a non-empty string."
    );
    assert_eq!(
        normalize_artifact_publish_request_payload_json(
            &json!({
                "plan_id": "PLAN-1",
                "artifacts": []
            })
            .to_string()
        )
        .expect_err("missing revision id"),
        "Payload field `plan_revision_id` must be a non-empty string."
    );
}

#[test]
fn plan_store_read_request_roundtrips_local_sync_shape() {
    let normalized = normalize_plan_store_read_request_payload_json(
        &json!({
            "operation": "list_plans",
            "plan_storage": {"mode": "binary", "authority_root": "/tmp/binary-db"}
        })
        .to_string(),
    )
    .expect("local store request");

    assert_eq!(
        normalized,
        json!({
            "operation": "list_plans",
            "plan_storage": {"mode": "binary", "authority_root": "/tmp/binary-db"},
            "plan_id": null,
            "plan_revision_id": null
        })
    );
}

#[test]
fn normalize_plan_remote_request_payload_preserves_artifact_body_exact_text() {
    let payload = json!({
        "operation": "create_plan",
        "transport": {"base_url": "https://example.test"},
        "repo_name": "housekeeper",
        "title": "Title",
        "artifact_path": "docs/sprints/example.md",
        "artifact_selector": "example/root",
        "artifact_heading": "Title",
        "items": [{"plan_item_ref": "example/item", "text": "Body line"}],
        "artifact_body": "# Title\n\nBody line\n",
    });

    let normalized = normalize_plan_remote_request_payload_json(&payload.to_string())
        .expect("payload should normalize");

    assert_eq!(
        normalized
            .get("artifact_body")
            .and_then(|value| value.as_str()),
        Some("# Title\n\nBody line\n")
    );
}

#[test]
fn plan_remote_request_roundtrips_paired_artifacts() {
    let payload = json!({
        "operation": "put_plan_revision_artifacts",
        "transport": {"base_url": "https://example.test"},
        "plan_id": "PLAN-1",
        "plan_revision_id": "REV-1",
        "artifacts": [
            {"path": "docs/plan.md", "blob_id": "BLB-MD"},
            {"path": "docs/plan.json", "blob_id": "BLB-JSON"}
        ]
    });

    let normalized = normalize_plan_remote_request_payload_json(&payload.to_string())
        .expect("remote artifact request");

    assert_eq!(normalized["operation"], "put_plan_revision_artifacts");
    assert_eq!(normalized["plan_id"], "PLAN-1");
    assert_eq!(normalized["plan_revision_id"], "REV-1");
    assert_eq!(
        normalized["artifacts"],
        json!([
            {"path": "docs/plan.md", "blob_id": "BLB-MD"},
            {"path": "docs/plan.json", "blob_id": "BLB-JSON"}
        ])
    );
}
