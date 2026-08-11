use super::*;

#[test]
fn task_workflow_repository_helpers_accept_single_capability_ports() {
    let mut ensurer = FakeRepositoryEnsurerPort;
    let mut reader = FakeRepositoryReaderPort;

    assert_eq!(
        ensure_repository_with_task_workflow_task_remote(
            &mut ensurer,
            "repo",
            "main",
            Some(&json!({"policy_id": "test"})),
            Some("NS"),
        )
        .unwrap()["id_namespace_prefix"],
        "NS"
    );
    assert_eq!(
        get_repository_with_task_workflow_task_remote(&mut reader, "repo").unwrap()["default_line"],
        "reader-only"
    );
}

#[test]
fn task_workflow_repository_remote_helpers_accept_narrow_trait_object() {
    let mut remote = FakeRepositoryRemote;
    let remote_port: &mut dyn TaskWorkflowRepositoryRemote = &mut remote;

    assert_eq!(
        ensure_repository_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "main",
            Some(&json!({"policy_id": "test"})),
            Some("NS"),
        )
        .unwrap()["id_namespace_prefix"],
        "NS"
    );
    assert_eq!(
        get_repository_with_task_workflow_task_remote(remote_port, "repo").unwrap()["default_line"],
        "main"
    );
}
