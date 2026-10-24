use crate::json_support::{json, JsonValue};

use super::{workflow_remote_action_mutation_receipts, workflow_remote_mutation_receipt};

#[test]
fn mutation_receipt_matches_reference_shape() {
    let result = workflow_remote_mutation_receipt(
        "submit_land",
        "submit_land",
        "direct_response",
        None,
        Some(&json!({"submission_id":"SUB-1","status":"queued","task_id":"RT-1"})),
    )
    .unwrap();
    assert_eq!(
        result,
        json!({
            "action":"submit_land",
            "source_action":"submit_land",
            "delivery":"direct_response",
            "submission_id":"SUB-1",
            "status":"queued",
            "task_id":"RT-1"
        })
    );
}

#[test]
fn action_receipts_expand_publish_selection_and_policy_refresh() {
    let publish = workflow_remote_action_mutation_receipts(
        "publish_patchset",
        &json!({
            "patchset_id":"RP-1",
            "response_recovery":{"state":"recovered"},
            "selection_recovery":{"state":"selected"}
        }),
    )
    .unwrap();
    assert!(matches!(publish, JsonValue::Array(_)));

    let review = workflow_remote_action_mutation_receipts(
        "record_review",
        &json!({
            "review_id":"RV-1",
            "policy_refresh":{"decision":"pass","response_recovery":{"state":"policy"}}
        }),
    )
    .unwrap();
    assert!(matches!(review, JsonValue::Array(_)));
}
