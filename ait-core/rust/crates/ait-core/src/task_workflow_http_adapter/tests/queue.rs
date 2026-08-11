use super::*;

#[test]
fn task_workflow_queue_and_change_remote_helpers_accept_narrow_trait_objects() {
    let mut remote = FakeQueueRemote;
    let remote_port: &mut dyn TaskWorkflowQueueRemote = &mut remote;

    assert_eq!(
        read_task_queue_with_task_workflow_task_remote(remote_port, "repo", Some("active"))
            .unwrap()["status"],
        "active"
    );
    assert_eq!(
        read_reviewer_inbox_with_task_workflow_task_remote(remote_port, "repo").unwrap()
            ["reviewer_inbox"],
        true
    );
    assert_eq!(
        read_queue_summary_bundle_with_task_workflow_task_remote(
            remote_port,
            "repo",
            Some("active"),
        )
        .unwrap()["summary"],
        true
    );
    assert_eq!(
        remote_port.list_changes("repo").unwrap()[0]["change_id"],
        "C-1"
    );

    let mut change_remote = FakeChangeRemote;
    let change_remote_port: &mut dyn TaskWorkflowChangeRemote = &mut change_remote;

    assert_eq!(
        create_change_with_task_workflow_task_remote(
            change_remote_port,
            "repo",
            "T-1",
            "title",
            "main",
            Some("C-1"),
            Some("SNP-1"),
            Some("main"),
        )
        .unwrap()["change_id"],
        "C-1"
    );
    assert_eq!(
        list_changes_with_task_workflow_task_remote(change_remote_port, "repo").unwrap()[0]
            ["change_id"],
        "C-1"
    );
    assert_eq!(
        get_change_detail_with_task_workflow_task_remote(change_remote_port, "C-1", Some("repo"),)
            .unwrap()["detail"],
        true
    );
    assert_eq!(
        get_change_with_task_workflow_task_remote(change_remote_port, "C-1", Some("repo")).unwrap()
            ["change_id"],
        "C-1"
    );
    assert_eq!(
        close_change_with_task_workflow_task_remote(
            change_remote_port,
            "C-1",
            "archived",
            Some("repo"),
        )
        .unwrap()["status"],
        "archived"
    );
}

#[test]
fn task_workflow_queue_read_helpers_accept_single_capability_ports() {
    let mut task_queue_reader = FakeTaskQueueReader;
    assert_eq!(
        read_task_queue_with_task_workflow_task_remote(
            &mut task_queue_reader,
            "repo",
            Some("active"),
        )
        .unwrap()["status"],
        "active"
    );

    let mut reviewer_inbox_reader = FakeReviewerInboxReader;
    assert_eq!(
        read_reviewer_inbox_with_task_workflow_task_remote(&mut reviewer_inbox_reader, "repo",)
            .unwrap()["reviewer_inbox"],
        true
    );

    let mut queue_summary_reader = FakeQueueSummaryBundleReader;
    assert_eq!(
        read_queue_summary_bundle_with_task_workflow_task_remote(
            &mut queue_summary_reader,
            "repo",
            Some("active"),
        )
        .unwrap()["summary"],
        true
    );
}
