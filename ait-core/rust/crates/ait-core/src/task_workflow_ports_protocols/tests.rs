use super::*;
use crate::json_support::json;

#[test]
fn builds_task_tracking_title_contract() {
    let payload = build_task_tracking_title_payload(&json!({
        "task_id": "LT-1",
        "title": "Demo task"
    }))
    .expect("title payload");

    assert_eq!(payload, json!({"title": "LT-1: Demo task"}));
}

#[test]
fn builds_task_tracking_metadata_contract() {
    let payload = build_task_tracking_metadata_payload(
        &json!({
            "task_id": "LT-1",
            "intent": "Ship it"
        }),
        "codex",
        "task_tracking",
    )
    .expect("metadata payload");

    assert_eq!(
        payload,
        json!({
            "author_mode": "codex",
            "tracking_policy": "task_tracking",
            "task_id": "LT-1",
            "objective": "Ship it"
        })
    );
    let mut payload_keys = payload
        .as_object()
        .expect("metadata payload")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    payload_keys.sort();
    assert_eq!(
        payload_keys,
        vec!["author_mode", "objective", "task_id", "tracking_policy"]
    );
}

#[test]
fn task_workflow_payloads_report_exact_required_field_errors() {
    assert_eq!(
        build_linked_change_lookup_payload(Some(&json!([
            {"changes": [{"change_id": "RCC-1"}]}
        ])))
        .expect_err("missing task id"),
        "`task_id` must be a string."
    );
    assert_eq!(
        build_task_tracking_metadata_payload(
            &json!({
                "task_id": "LT-1",
                "intent": "Ship it"
            }),
            "",
            "task_tracking",
        )
        .expect_err("missing author mode"),
        "`author_mode` must be non-empty."
    );
}

#[test]
fn linked_task_and_change_lookup_payloads_roundtrip() {
    let linked_tasks = build_linked_task_lookup_payload(
        Some(&json!([
            {
                "plan_id": "PLAN-1",
                "plan_item_ref": "PLAN-1/item-1",
                "tasks": [{"task_id": "RCT-1"}]
            }
        ])),
        Some(&json!([
            {
                "plan_id": "PLAN-1",
                "tasks": [{"task_id": "RCT-1"}]
            }
        ])),
    )
    .expect("linked tasks");

    assert_eq!(linked_tasks["linked_task_count"], 1);
    assert_eq!(
        linked_tasks["task_links_by_item"][0]["plan_item_ref"],
        "PLAN-1/item-1"
    );
    assert_eq!(
        linked_tasks["tasks_by_plan"][0]["tasks"][0]["task_id"],
        "RCT-1"
    );

    let linked_changes = build_linked_change_lookup_payload(Some(&json!([
        {
            "task_id": "RCT-1",
            "changes": [{"change_id": "RCC-1"}]
        }
    ])))
    .expect("linked changes");

    assert_eq!(linked_changes["linked_change_count"], 1);
    assert_eq!(
        linked_changes["change_links_by_task"][0]["changes"][0]["change_id"],
        "RCC-1"
    );
}

#[test]
fn default_task_tracking_title_uses_generic_label() {
    let payload = build_task_tracking_title_payload(&json!({})).expect("title payload");

    assert_eq!(payload, json!({"title": "Tracked task"}));
}
