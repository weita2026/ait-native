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
fn stale_tasks_are_active_rows_older_than_the_threshold() {
    let now = chrono::DateTime::parse_from_rfc3339("2026-09-08T00:00:00+00:00")
        .unwrap()
        .with_timezone(&chrono::Utc);
    let rows = vec![
        json!({"task_id": "LT-1", "status": "active", "updated_at": "2026-07-01T00:00:00Z"}),
        json!({"task_id": "LT-2", "status": "active", "updated_at": "2026-09-01T00:00:00Z"}),
        json!({"task_id": "LT-3", "status": "completed", "updated_at": "2026-01-01T00:00:00Z"}),
        json!({"task_id": "LT-4", "status": "active", "created_at": "2026-06-01T00:00:00Z"}),
        json!({"task_id": "LT-5", "status": "active", "updated_at": "not a date"}),
    ];
    assert_eq!(
        queue_stale_task_ids(&rows, now, QUEUE_STALE_TASK_DAYS),
        vec!["LT-1", "LT-4"]
    );
    assert!(queue_stale_task_ids(&rows, now, 365).is_empty());
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
