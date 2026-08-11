use super::*;

#[test]
fn queue_local_summary_counts_actionable_rows_and_totals() {
    let tasks = vec![
        json!({
            "task_id": "RT-ACTIVE",
            "status": "active",
            "publication_state": "local_draft"
        }),
        json!({
            "task_id": "RT-CLOSED",
            "status": "completed",
            "publication_state": "local_draft"
        }),
        json!({
            "task_id": "RT-PUBLISHED",
            "status": "active",
            "publication_state": "published"
        }),
    ];
    let changes = vec![
        json!({
            "change_id": "LC-ACTIVE",
            "status": "active",
            "publication_state": "local_draft"
        }),
        json!({
            "change_id": "LC-DRAFT",
            "status": "draft",
            "publication_state": "local_draft"
        }),
        json!({
            "change_id": "LC-LANDED",
            "status": "landed",
            "publication_state": "local_draft"
        }),
        json!({
            "change_id": "LC-ARCHIVED",
            "status": "archived",
            "publication_state": "local_draft"
        }),
        json!({
            "change_id": "RC-PUBLISHED",
            "status": "active",
            "publication_state": "published"
        }),
    ];

    let actionable_tasks = queue_actionable_local_tasks(&tasks);
    let actionable_changes = queue_actionable_local_changes(&changes);
    let summary = queue_local_summary(&tasks, &changes);

    assert_eq!(
        actionable_tasks
            .iter()
            .filter_map(|row| string_field(row, "task_id"))
            .collect::<Vec<_>>(),
        vec!["RT-ACTIVE".to_string()]
    );
    assert_eq!(
        actionable_changes
            .iter()
            .filter_map(|row| string_field(row, "change_id"))
            .collect::<Vec<_>>(),
        vec!["LC-ACTIVE".to_string(), "LC-DRAFT".to_string()]
    );
    assert_eq!(summary["task_record_count"], json!(3));
    assert_eq!(summary["change_record_count"], json!(5));
    assert_eq!(summary["draft_task_count"], json!(1));
    assert_eq!(summary["published_task_count"], json!(1));
    assert_eq!(summary["draft_change_count"], json!(2));
    assert_eq!(summary["published_change_count"], json!(1));
    assert_eq!(summary["unpublished_task_record_count"], json!(2));
    assert_eq!(summary["unpublished_change_record_count"], json!(4));
    assert_eq!(summary["active_draft_task_count"], json!(1));
    assert_eq!(summary["open_draft_change_count"], json!(2));
}

#[test]
fn queue_change_inventory_projects_remote_review_state() {
    let change_rows = vec![
        json!({
            "change_id": "RC-NO-PATCH",
            "status": "active",
            "current_patchset_number": 0
        }),
        json!({
            "change_id": "RC-READY",
            "status": "active",
            "current_patchset_number": 1
        }),
        json!({
            "change_id": "RC-BLOCKED",
            "status": "active",
            "current_patchset_number": 1
        }),
        json!({
            "change_id": "RC-FOCUS",
            "status": "active",
            "current_patchset_number": 1
        }),
        json!({
            "change_id": "RC-LANDED",
            "status": "landed",
            "current_patchset_number": 1
        }),
    ];
    let task_items = vec![json!({
        "focus_change": {
            "change_id": "RC-FOCUS",
            "reason": "Task queue supplied a focus reason."
        }
    })];
    let review_items = vec![
        json!({
            "change_id": "RC-READY",
            "review_state": {"blocking": 0},
            "freshness": {"base_is_fresh": true},
            "policy_state": {"decision": "pass"},
            "attestation": {"completeness": "present"}
        }),
        json!({
            "change_id": "RC-BLOCKED",
            "review_state": {"blocking": 1},
            "freshness": {"base_is_fresh": true},
            "policy_state": {"decision": "pass"},
            "attestation": {"completeness": "present"}
        }),
        json!({
            "change_id": "RC-FOCUS",
            "review_state": {"blocking": 0},
            "freshness": {"base_is_fresh": false},
            "policy_state": {"decision": "pending"},
            "attestation": {"completeness": "present"}
        }),
    ];

    let inventory = queue_change_inventory(&change_rows, &task_items, &review_items);

    assert_eq!(inventory.len(), 4);
    let by_id = inventory
        .iter()
        .filter_map(|row| Some((string_field(row, "change_id")?, row)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        by_id["RC-NO-PATCH"]["reason"].as_str(),
        Some("No published patchset exists yet.")
    );
    assert_eq!(by_id["RC-NO-PATCH"]["ready_to_land"].as_bool(), Some(false));
    assert_eq!(by_id["RC-READY"]["reason"].as_str(), Some("Ready to land."));
    assert_eq!(by_id["RC-READY"]["ready_to_land"].as_bool(), Some(true));
    assert_eq!(
        by_id["RC-BLOCKED"]["reason"].as_str(),
        Some("Blocking review feedback is recorded on this change.")
    );
    assert_eq!(by_id["RC-BLOCKED"]["ready_to_land"].as_bool(), Some(false));
    assert_eq!(
        by_id["RC-FOCUS"]["reason"].as_str(),
        Some("Task queue supplied a focus reason.")
    );
    assert_eq!(by_id["RC-FOCUS"]["ready_to_land"].as_bool(), Some(false));
    assert!(!by_id.contains_key("RC-LANDED"));
}

#[test]
fn queue_summary_bundle_missing_detects_native_read_404_only() {
    assert!(queue_summary_bundle_missing(
        "GET /v1/native/repository-authorities/7/read/queue-summary?status=active failed with 404"
    ));
    assert!(!queue_summary_bundle_missing(
        "GET /v1/native/repository-authorities/7/read/queue-summary?status=active failed with 500"
    ));
    assert!(!queue_summary_bundle_missing(
        "GET /v1/native/repository-authorities/7/read/task-queue?status=active failed with 404"
    ));
    assert!(!queue_summary_bundle_missing(
        "GET /v1/native/read/queue-summary?repo_name=fixture-ait failed with 404"
    ));
}
