use super::*;

#[test]
fn task_workflow_line_remote_helpers_accept_narrow_trait_object() {
    let mut remote = FakeLineRemote;
    let remote_port: &mut dyn TaskWorkflowLineRemote = &mut remote;

    assert_eq!(
        change_lineage_payload_with_task_workflow_task_remote(
            remote_port,
            "main",
            Some(&json!({"head_snapshot_id": "SNP-BASE"})),
        )
        .unwrap()["fork_snapshot_id"],
        "SNP-BASE"
    );
    assert_eq!(
        get_line_with_task_workflow_task_remote(remote_port, "repo", "main").unwrap()
            ["head_snapshot_id"],
        "SNP-LINE"
    );
    assert_eq!(
        list_lines_with_task_workflow_task_remote(remote_port, "repo").unwrap()[0]["line_name"],
        "main"
    );
    assert_eq!(
        update_remote_line_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "main",
            Some("SNP-NEW"),
            Some("SNP-OLD"),
        )
        .unwrap()["expected_head_snapshot_id"],
        "SNP-OLD"
    );
    assert_eq!(
        close_line_with_task_workflow_task_remote(remote_port, "repo", "main", "archived").unwrap()
            ["status"],
        "archived"
    );
}

#[test]
fn task_workflow_line_helpers_accept_single_capability_ports() {
    let lineage_builder = FakeLineagePayloadBuilder;
    assert_eq!(
        change_lineage_payload_with_task_workflow_task_remote(
            &lineage_builder,
            "main",
            Some(&json!({"head_snapshot_id": "SNP-BASE"})),
        )
        .unwrap()["fork_snapshot_id"],
        "SNP-BASE"
    );

    let mut line_reader = FakeLineReader;
    assert_eq!(
        get_line_with_task_workflow_task_remote(&mut line_reader, "repo", "main").unwrap()
            ["head_snapshot_id"],
        "SNP-LINE"
    );

    let mut line_lister = FakeLineLister;
    assert_eq!(
        list_lines_with_task_workflow_task_remote(&mut line_lister, "repo").unwrap()[0]
            ["line_name"],
        "main"
    );

    let mut line_updater = FakeLineHeadUpdater;
    assert_eq!(
        update_remote_line_with_task_workflow_task_remote(
            &mut line_updater,
            "repo",
            "main",
            Some("SNP-NEW"),
            Some("SNP-OLD"),
        )
        .unwrap()["expected_head_snapshot_id"],
        "SNP-OLD"
    );

    let mut line_closer = FakeLineCloser;
    assert_eq!(
        close_line_with_task_workflow_task_remote(&mut line_closer, "repo", "main", "archived")
            .unwrap()["status"],
        "archived"
    );
}
