use super::{
    compute_taskable_items, local_plan_publish_shadow, plan_candidates_payload,
    plan_dispatch_summary, plan_items_payload, plan_task_link_indexes, validate_dispatch_legality,
    DispatchPlanInput, DispatchPlanItemInput, DispatchRevisionInput, DispatchTaskInput,
};

fn sample_plan() -> DispatchPlanInput {
    DispatchPlanInput {
        plan_id: Some("PL-1".to_string()),
        title: Some("Demo".to_string()),
        status: Some("draft".to_string()),
        repo_name: Some("ait".to_string()),
        publication_state: Some("published".to_string()),
        published_plan_id: Some("PL-1".to_string()),
        published_head_revision_id: Some("PR-1".to_string()),
        head_revision_id: Some("PR-2".to_string()),
        head_revision: Some(DispatchRevisionInput {
            plan_revision_id: Some("PR-2".to_string()),
            revision_number: Some(2),
            artifact_path: Some("docs/sprints/demo.md".to_string()),
            artifact_selector: Some("demo/root".to_string()),
            artifact_heading: Some("Demo".to_string()),
            publication_state: Some("local_draft".to_string()),
            items: vec![
                DispatchPlanItemInput {
                    plan_item_ref: Some("demo/linked".to_string()),
                    text: "linked".to_string(),
                    checkbox_state: "open".to_string(),
                    heading_path: vec!["Demo".to_string()],
                    line_number: 10,
                },
                DispatchPlanItemInput {
                    plan_item_ref: Some("demo/taskable".to_string()),
                    text: "taskable".to_string(),
                    checkbox_state: "open".to_string(),
                    heading_path: vec!["Demo".to_string()],
                    line_number: 11,
                },
                DispatchPlanItemInput {
                    plan_item_ref: None,
                    text: "missing ref".to_string(),
                    checkbox_state: "open".to_string(),
                    heading_path: vec!["Demo".to_string()],
                    line_number: 12,
                },
                DispatchPlanItemInput {
                    plan_item_ref: Some("demo/done".to_string()),
                    text: "done".to_string(),
                    checkbox_state: "done".to_string(),
                    heading_path: vec!["Demo".to_string()],
                    line_number: 13,
                },
            ],
        }),
    }
}

#[test]
fn plan_items_payload_marks_identity_only_contract() {
    let payload = plan_items_payload(&sample_plan(), None);

    assert!(payload.identity_only);
    assert!(payload.dispatch_validation_required);
    assert_eq!(payload.item_count, 4);
    assert_eq!(
        payload.items[0].plan_item_ref.as_deref(),
        Some("demo/linked")
    );
}

#[test]
fn local_plan_publish_shadow_detects_unpublished_head() {
    let shadow = local_plan_publish_shadow(Some(&sample_plan())).expect("shadow");

    assert!(shadow.unpublished_head);
    assert_eq!(shadow.head_revision_number, Some(2));
    assert_eq!(shadow.head_revision_id.as_deref(), Some("PR-2"));
}

#[test]
fn plan_task_link_indexes_group_by_plan_and_item() {
    let indexes = plan_task_link_indexes(&[
        DispatchTaskInput {
            task_id: Some("RT-1".to_string()),
            title: Some("A".to_string()),
            status: Some("active".to_string()),
            planning_state: Some("planned".to_string()),
            origin_plan_revision_id: Some("PR-1".to_string()),
            plan_drift_state: None,
            plan_id: Some("PL-1".to_string()),
            plan_item_ref: Some("demo/a".to_string()),
        },
        DispatchTaskInput {
            task_id: Some("RT-2".to_string()),
            title: Some("B".to_string()),
            status: Some("completed".to_string()),
            planning_state: Some("planned".to_string()),
            origin_plan_revision_id: Some("PR-1".to_string()),
            plan_drift_state: None,
            plan_id: Some("PL-1".to_string()),
            plan_item_ref: None,
        },
    ]);

    let by_plan = indexes.by_plan.get("PL-1").expect("by_plan");
    assert_eq!(
        by_plan
            .iter()
            .map(|row| row.task_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("RT-1"), Some("RT-2")]
    );
    let by_item = indexes
        .by_item
        .get(&(String::from("PL-1"), String::from("demo/a")))
        .expect("by_item");
    assert_eq!(
        by_item
            .iter()
            .map(|row| row.task_id.as_deref())
            .collect::<Vec<_>>(),
        vec![Some("RT-1")]
    );
}

#[test]
fn plan_dispatch_summary_marks_taskable_and_blocked_items() {
    let summary = plan_dispatch_summary(
        &sample_plan(),
        &[DispatchTaskInput {
            task_id: Some("RT-1".to_string()),
            title: Some("Linked".to_string()),
            status: Some("active".to_string()),
            planning_state: Some("planned".to_string()),
            origin_plan_revision_id: Some("PR-1".to_string()),
            plan_drift_state: None,
            plan_id: Some("PL-1".to_string()),
            plan_item_ref: Some("demo/linked".to_string()),
        }],
        None,
        None,
    );

    assert_eq!(summary.linked_task_count, 1);
    assert_eq!(summary.taskable_item_count, 1);
    assert_eq!(summary.linked_open_item_count, 1);
    assert_eq!(summary.unref_open_item_count, 1);
    let linked_row = summary
        .items
        .iter()
        .find(|item| item.plan_item_ref.as_deref() == Some("demo/linked"))
        .expect("linked row");
    assert_eq!(
        linked_row.taskable_blocker.as_deref(),
        Some("linked_task_exists")
    );
    let taskable_row = summary
        .items
        .iter()
        .find(|item| item.plan_item_ref.as_deref() == Some("demo/taskable"))
        .expect("taskable row");
    assert!(taskable_row.taskable);
}

#[test]
fn compute_taskable_items_returns_only_taskable_rows() {
    let items = compute_taskable_items(
        &sample_plan(),
        &[DispatchTaskInput {
            task_id: Some("RT-1".to_string()),
            title: Some("Linked".to_string()),
            status: Some("active".to_string()),
            planning_state: Some("planned".to_string()),
            origin_plan_revision_id: Some("PR-1".to_string()),
            plan_drift_state: None,
            plan_id: Some("PL-1".to_string()),
            plan_item_ref: Some("demo/linked".to_string()),
        }],
        None,
        None,
    );

    assert_eq!(items.len(), 1);
    assert_eq!(items[0].plan_item_ref.as_deref(), Some("demo/taskable"));
    assert!(items[0].taskable);
}

#[test]
fn validate_dispatch_legality_reports_item_state_and_missing_refs() {
    let tasks = [DispatchTaskInput {
        task_id: Some("RT-1".to_string()),
        title: Some("Linked".to_string()),
        status: Some("active".to_string()),
        planning_state: Some("planned".to_string()),
        origin_plan_revision_id: Some("PR-1".to_string()),
        plan_drift_state: None,
        plan_id: Some("PL-1".to_string()),
        plan_item_ref: Some("demo/linked".to_string()),
    }];

    let taskable =
        validate_dispatch_legality(&sample_plan(), &tasks, Some("demo/taskable"), None, None);
    assert!(taskable.taskable);
    assert_eq!(taskable.taskable_blocker, None);

    let linked =
        validate_dispatch_legality(&sample_plan(), &tasks, Some("demo/linked"), None, None);
    assert!(!linked.taskable);
    assert_eq!(
        linked.taskable_blocker.as_deref(),
        Some("linked_task_exists")
    );

    let missing =
        validate_dispatch_legality(&sample_plan(), &tasks, Some("demo/missing"), None, None);
    assert!(!missing.taskable);
    assert_eq!(
        missing.taskable_blocker.as_deref(),
        Some("plan_item_not_found")
    );

    let blank = validate_dispatch_legality(&sample_plan(), &tasks, Some("   "), None, None);
    assert!(!blank.taskable);
    assert_eq!(
        blank.taskable_blocker.as_deref(),
        Some("missing_requested_plan_item_ref")
    );
}

#[test]
fn plan_candidates_payload_filters_summaries_and_keeps_stable_ordering() {
    let mut alpha = plan_dispatch_summary(&sample_plan(), &[], None, None);
    alpha.title = Some("Alpha".to_string());
    alpha.artifact_path = Some("docs/sprints/alpha.md".to_string());
    alpha.local_unpublished_head = true;

    let mut beta = alpha.clone();
    beta.plan_id = Some("PL-2".to_string());
    beta.title = Some("Beta".to_string());
    beta.artifact_path = Some("docs/sprints/beta.md".to_string());
    beta.open_item_count = 3;
    beta.taskable_item_count = 0;
    beta.taskable_items = Vec::new();
    beta.local_unpublished_head = false;

    let mut gamma = alpha.clone();
    gamma.plan_id = Some("PL-3".to_string());
    gamma.title = Some("Gamma".to_string());
    gamma.artifact_path = Some("docs/sprints/gamma.md".to_string());
    gamma.open_item_count = 2;
    gamma.taskable_item_count = 3;

    let payload = plan_candidates_payload(
        &[beta.clone(), alpha.clone(), gamma.clone()],
        Some("remote"),
        Some("ait"),
        Some("origin"),
        false,
    );

    assert_eq!(payload.scope, "remote");
    assert_eq!(payload.remote.as_deref(), Some("origin"));
    assert_eq!(payload.summary.scanned_plan_count, 3);
    assert_eq!(payload.summary.candidate_plan_count, 2);
    assert_eq!(payload.summary.open_item_count, 8);
    assert_eq!(payload.summary.taskable_item_count, 5);
    assert_eq!(payload.summary.local_unpublished_head_count, 2);
    assert_eq!(payload.candidates.len(), 2);
    assert_eq!(payload.candidates[0].title.as_deref(), Some("Gamma"));
    assert_eq!(payload.candidates[1].title.as_deref(), Some("Alpha"));

    let include_all_payload = plan_candidates_payload(
        &[beta.clone(), alpha, gamma],
        Some("remote"),
        Some("ait"),
        Some("origin"),
        true,
    );
    assert_eq!(include_all_payload.summary.candidate_plan_count, 3);
    assert_eq!(
        include_all_payload.candidates[2].title.as_deref(),
        Some("Beta")
    );
}
