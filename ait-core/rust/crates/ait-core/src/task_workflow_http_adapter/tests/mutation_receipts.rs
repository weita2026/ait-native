use super::*;

#[test]
fn task_workflow_mutation_receipt_helpers_accept_mutation_receipt_remote_trait() {
    let remote = FakeMutationReceiptRemote;
    let remote_port: &dyn TaskWorkflowMutationReceiptRemote = &remote;

    assert_eq!(
        mutation_receipt_with_task_workflow_closeout_remote(
            remote_port,
            "publish_patchset",
            "workflow ready",
            "remote",
            Some(&json!({"state": "recovered"})),
            Some(&json!({"ok": true})),
        )
        .unwrap()["source_action"],
        "workflow ready"
    );
    assert_eq!(
        action_mutation_receipts_with_task_workflow_closeout_remote(
            remote_port,
            "close_task",
            &json!({"status": "completed"}),
        )
        .unwrap()["code"],
        "close_task"
    );
}

#[test]
fn task_workflow_mutation_receipt_helpers_accept_single_capability_ports() {
    let receipt_builder = FakeMutationReceiptBuilderPort;
    assert_eq!(
        mutation_receipt_with_task_workflow_closeout_remote(
            &receipt_builder,
            "publish_patchset",
            "workflow ready",
            "remote",
            Some(&json!({"state": "recovered"})),
            Some(&json!({"ok": true})),
        )
        .unwrap()["source_action"],
        "workflow ready"
    );

    let action_builder = FakeActionMutationReceiptsBuilderPort;
    assert_eq!(
        action_mutation_receipts_with_task_workflow_closeout_remote(
            &action_builder,
            "close_task",
            &json!({"status": "completed"}),
        )
        .unwrap()["code"],
        "close_task"
    );
}
