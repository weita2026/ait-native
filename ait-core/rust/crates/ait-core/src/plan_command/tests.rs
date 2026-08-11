use super::*;
use crate::json_support::json;

#[test]
fn plan_command_payloads_report_exact_required_field_errors() {
    assert_eq!(
        normalize_plan_list_command_request_payload_json("{}").expect_err("missing scope"),
        "Plan application payload field `scope` must be non-empty."
    );
    assert_eq!(
        normalize_plan_revisions_command_request_payload_json(
            &json!({
                "scope": "local",
                "repo_name": "ait",
                "revisions": []
            })
            .to_string()
        )
        .expect_err("missing plan id"),
        "Plan application payload field `plan_id` must be non-empty."
    );
}

#[test]
fn plan_command_payloads_roundtrip_service_projection_shapes() {
    let list_payload = build_plan_list_command_payload_json(
        &json!({
            "scope": "local",
            "repo_name": "ait",
            "plans": [{"plan_id": "PLAN-1"}]
        })
        .to_string(),
    )
    .expect("list command payload");
    assert_eq!(list_payload, json!([{"plan_id": "PLAN-1"}]));

    let show_payload = build_plan_show_command_payload_json(
        &json!({
            "scope": "local",
            "repo_name": "ait",
            "plan": {"plan_id": "PLAN-1"},
            "revision": {"plan_revision_id": "REV-1"}
        })
        .to_string(),
    )
    .expect("show command payload");
    assert_eq!(
        show_payload,
        json!({
            "plan": {"plan_id": "PLAN-1"},
            "revision": {"plan_revision_id": "REV-1"}
        })
    );
}
