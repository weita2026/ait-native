use super::*;

#[test]
fn task_workflow_client_helpers_accept_lifecycle_trait() {
    let mut remote = FakeClientLifecycleRemote::default();
    let remote_port: &mut dyn TaskWorkflowHttpClientRemote = &mut remote;

    assert_eq!(
        inspect_client_with_task_workflow_task_remote(remote_port).base_url,
        "https://ait.example"
    );
    assert!(close_client_with_task_workflow_task_remote(remote_port).closed);
    assert!(remote.closed);

    let mut closeout_remote = FakeClientLifecycleRemote::default();
    let closeout_port: &mut dyn TaskWorkflowHttpClientRemote = &mut closeout_remote;

    assert_eq!(
        inspect_client_with_task_workflow_closeout_remote(closeout_port).base_url,
        "https://ait.example"
    );
    assert!(close_client_with_task_workflow_closeout_remote(closeout_port).closed);
    assert!(closeout_remote.closed);
}

#[test]
fn task_workflow_client_helpers_accept_single_capability_ports() {
    let inspector = FakeClientInspectorPort;
    assert_eq!(
        inspect_client_with_task_workflow_task_remote(&inspector).base_url,
        "https://ait.example"
    );
    assert_eq!(
        inspect_client_with_task_workflow_closeout_remote(&inspector).base_url,
        "https://ait.example"
    );

    let mut closer = FakeClientCloserPort::default();
    assert!(close_client_with_task_workflow_task_remote(&mut closer).closed);
    assert!(closer.closed);

    let mut closeout_closer = FakeClientCloserPort::default();
    assert!(close_client_with_task_workflow_closeout_remote(&mut closeout_closer).closed);
    assert!(closeout_closer.closed);
}
