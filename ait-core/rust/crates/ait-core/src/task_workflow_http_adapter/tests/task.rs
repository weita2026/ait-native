use super::*;
use crate::plan_http_client::{
    build_create_change_request_spec, build_create_task_request_spec,
    build_read_task_audit_request_spec,
};
use crate::plan_workflow_json::PlanWorkflowJson;
use crate::task_workflow_http_adapter::HttpTaskRemote;

#[test]
fn task_audit_http_adapter_fixture_roundtrips_request_and_response() {
    let expected_response = json!({
        "task_id": "T-1",
        "target_line": "main",
        "status": "ready",
        "checks": [{"name": "worktree", "status": "pass"}],
        "changes": [{
            "task_id": "T-1",
            "change_id": "T-1/C-01",
            "status": "landed"
        }]
    });
    let (config, server) = serve_task_workflow_json_once(expected_response.clone());
    let spec_config = config.clone();
    let mut remote = HttpTaskRemote::new(config).expect("http task remote");

    let audit = remote
        .read_task_audit("repo", "T-1", "main")
        .expect("task audit response");
    assert_eq!(audit["task_id"], expected_response["task_id"]);
    assert_eq!(audit["checks"], expected_response["checks"]);
    assert_eq!(audit["changes"][0]["change_id"], "C-01");
    assert_eq!(audit["changes"][0]["change_ref"], "T-1/C-01");
    let recorded = server.join().expect("fixture server");
    assert_eq!(recorded.method, "GET");
    assert_eq!(
        recorded.target,
        "/v1/native/repository-authorities/7/read/tasks/T-1/audit?target_line=main"
    );
    assert!(recorded.body.is_none());

    let spec = build_read_task_audit_request_spec(&spec_config, "repo", "T-1", "main").unwrap();
    assert_eq!(
        PlanWorkflowJson::stateless().task_workflow_http_request_spec_payload(&spec),
        json!({
            "method": "GET",
            "path": "/v1/native/repository-authorities/7/read/tasks/T-1/audit",
            "url": format!("{}v1/native/repository-authorities/7/read/tasks/T-1/audit?target_line=main", spec_config.base_url),
            "query_pairs": [{"name": "target_line", "value": "main"}],
            "headers": {"Accept": "application/json"},
            "body": null,
            "timeout_ms": 5000,
        })
    );
}

#[test]
fn task_start_http_adapter_fixture_roundtrips_task_and_change_requests() {
    let task_response = json!({
        "task_id": "T-2",
        "title": "Add JSON fixtures",
        "intent": "Lock adapter JSON",
        "status": "active"
    });
    let (task_config, task_server) = serve_task_workflow_json_once(task_response.clone());
    let task_spec_config = task_config.clone();
    let mut task_remote = HttpTaskRemote::new(task_config).expect("http task remote");

    assert_eq!(
        task_remote
            .create_task(
                "repo",
                "Add JSON fixtures",
                "Lock adapter JSON",
                Some("T-2"),
                Some("PLAN-1"),
                Some("REV-1"),
                Some("1"),
            )
            .expect("create task response"),
        task_response
    );
    let recorded_task = task_server.join().expect("task fixture server");
    assert_eq!(recorded_task.method, "POST");
    assert_eq!(
        recorded_task.target,
        "/v1/native/repository-authorities/7/tasks"
    );
    assert_eq!(
        recorded_task.body,
        Some(json!({
            "title": "Add JSON fixtures",
            "intent": "Lock adapter JSON",
            "task_id": "T-2",
            "plan_id": "PLAN-1",
            "origin_plan_revision_id": "REV-1",
            "plan_item_ref": "1",
        }))
    );

    let task_spec = build_create_task_request_spec(
        &task_spec_config,
        "repo",
        "Add JSON fixtures",
        "Lock adapter JSON",
        Some("T-2"),
        Some("PLAN-1"),
        Some("REV-1"),
        Some("1"),
    )
    .unwrap();
    assert_eq!(
        PlanWorkflowJson::stateless().task_workflow_http_request_spec_payload(&task_spec),
        json!({
            "method": "POST",
            "path": "/v1/native/repository-authorities/7/tasks",
            "url": format!("{}v1/native/repository-authorities/7/tasks", task_spec_config.base_url),
            "query_pairs": [],
            "headers": {
                "Accept": "application/json",
                "Content-Type": "application/json",
            },
            "body": {
                "title": "Add JSON fixtures",
                "intent": "Lock adapter JSON",
                "task_id": "T-2",
                "plan_id": "PLAN-1",
                "origin_plan_revision_id": "REV-1",
                "plan_item_ref": "1",
            },
            "timeout_ms": 5000,
        })
    );

    let change_response = json!({
        "change_id": "C-2",
        "task_id": "T-2",
        "base_line": "main",
        "status": "open"
    });
    let (change_config, change_server) = serve_task_workflow_json_once(change_response.clone());
    let change_spec_config = change_config.clone();
    let mut change_remote = HttpTaskRemote::new(change_config).expect("http task remote");

    assert_eq!(
        change_remote
            .create_change(
                "repo",
                "T-2",
                "Add JSON fixtures",
                "main",
                Some("C-2"),
                Some("SNP-1"),
                Some("main"),
            )
            .expect("create change response"),
        json!({
            "change_id": "C-2",
            "change_ref": "T-2/C-2",
            "task_id": "T-2",
            "base_line": "main",
            "status": "open"
        })
    );
    let recorded_change = change_server.join().expect("change fixture server");
    assert_eq!(recorded_change.method, "POST");
    assert_eq!(
        recorded_change.target,
        "/v1/native/repository-authorities/7/changes"
    );
    assert_eq!(
        recorded_change.body,
        Some(json!({
            "task_id": "T-2",
            "title": "Add JSON fixtures",
            "base_line": "main",
            "change_id": "C-2",
            "fork_snapshot_id": "SNP-1",
            "forked_from_line": "main",
        }))
    );

    let change_spec = build_create_change_request_spec(
        &change_spec_config,
        "repo",
        "T-2",
        "Add JSON fixtures",
        "main",
        Some("C-2"),
        Some("SNP-1"),
        Some("main"),
    )
    .unwrap();
    assert_eq!(
        PlanWorkflowJson::stateless().task_workflow_http_request_spec_payload(&change_spec),
        json!({
            "method": "POST",
            "path": "/v1/native/repository-authorities/7/changes",
            "url": format!("{}v1/native/repository-authorities/7/changes", change_spec_config.base_url),
            "query_pairs": [],
            "headers": {
                "Accept": "application/json",
                "Content-Type": "application/json",
            },
            "body": {
                "task_id": "T-2",
                "title": "Add JSON fixtures",
                "base_line": "main",
                "change_id": "C-2",
                "fork_snapshot_id": "SNP-1",
                "forked_from_line": "main",
            },
            "timeout_ms": 5000,
        })
    );
}

#[test]
fn task_workflow_task_lifecycle_helpers_accept_task_lifecycle_remote_trait() {
    let mut remote = FakeTaskLifecycleRemote;
    let remote_port: &mut dyn TaskWorkflowTaskLifecycleRemote = &mut remote;

    assert_eq!(
        close_task_with_task_workflow_closeout_remote(
            remote_port,
            "T-1",
            "completed",
            Some("repo"),
        )
        .unwrap()["status"],
        "completed"
    );
    assert_eq!(
        restart_task_with_task_workflow_closeout_remote(remote_port, "T-1", Some("repo")).unwrap()
            ["status"],
        "active"
    );
}

#[test]
fn task_workflow_task_lifecycle_helpers_accept_single_capability_ports() {
    let mut closer = FakeRemoteTaskCloserPort;
    let mut restarter = FakeRemoteTaskRestarterPort;

    assert_eq!(
        close_task_with_task_workflow_closeout_remote(
            &mut closer,
            "T-1",
            "completed",
            Some("repo"),
        )
        .unwrap()["status"],
        "completed"
    );
    assert_eq!(
        restart_task_with_task_workflow_closeout_remote(&mut restarter, "T-1", Some("repo"))
            .unwrap()["status"],
        "active"
    );
}

#[test]
fn task_workflow_task_remote_helpers_accept_trait_object() {
    let mut remote = FakeTaskRemote::default();
    let remote_port: &mut dyn TaskWorkflowTaskRemote = &mut remote;

    assert_eq!(
        inspect_client_with_task_workflow_task_remote(remote_port).base_url,
        "https://ait.example"
    );
    assert!(close_client_with_task_workflow_task_remote(remote_port).closed);
    assert_eq!(
        ensure_repository_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "main",
            Some(&json!({"rules": []})),
            Some("NS"),
        )
        .unwrap()["id_namespace_prefix"],
        "NS"
    );
    assert_eq!(
        get_repository_with_task_workflow_task_remote(remote_port, "repo").unwrap()["repo_name"],
        "repo"
    );
    assert_eq!(
        change_lineage_payload_with_task_workflow_task_remote(
            remote_port,
            "main",
            Some(&json!({"head_snapshot_id": "SNP-1"})),
        )
        .unwrap()["fork_snapshot_id"],
        "SNP-1"
    );
    assert_eq!(
        get_line_with_task_workflow_task_remote(remote_port, "repo", "main").unwrap()["line_name"],
        "main"
    );
    assert_eq!(
        list_lines_with_task_workflow_task_remote(remote_port, "repo").unwrap()[0]["line_name"],
        "main"
    );
    assert_eq!(
        get_task_with_task_workflow_task_remote(remote_port, "T-1", Some("repo")).unwrap()
            ["task_id"],
        "T-1"
    );
    assert_eq!(
        list_tasks_with_task_workflow_task_remote(remote_port, "repo").unwrap()[0]["task_id"],
        "T-1"
    );
    assert_eq!(
        read_task_audit_with_task_workflow_task_remote(remote_port, "repo", "T-1", "main").unwrap()
            ["target_line"],
        "main"
    );
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
        create_task_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "title",
            "intent",
            Some("T-2"),
            Some("PLAN-1"),
            Some("REV-1"),
            Some("1"),
        )
        .unwrap()["task_id"],
        "T-2"
    );
    assert_eq!(
        create_change_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "T-2",
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
        list_changes_with_task_workflow_task_remote(remote_port, "repo").unwrap()[0]["change_id"],
        "C-1"
    );
    assert_eq!(
        get_change_detail_with_task_workflow_task_remote(remote_port, "C-1", Some("repo")).unwrap()
            ["detail"],
        true
    );
    assert_eq!(
        get_change_with_task_workflow_task_remote(remote_port, "C-1", Some("repo")).unwrap()
            ["change_id"],
        "C-1"
    );
    assert_eq!(
        close_change_with_task_workflow_task_remote(remote_port, "C-1", "archived", Some("repo"))
            .unwrap()["status"],
        "archived"
    );
    assert_eq!(
        update_remote_line_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "main",
            Some("SNP-2"),
            Some("SNP-1"),
        )
        .unwrap()["head_snapshot_id"],
        "SNP-2"
    );
    assert_eq!(
        close_line_with_task_workflow_task_remote(remote_port, "repo", "feature", "archived")
            .unwrap()["status"],
        "archived"
    );
    assert_eq!(
        get_remote_snapshot_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "SNP-5",
            true,
            Some("a.txt"),
        )
        .unwrap()["path"],
        "a.txt"
    );
    assert_eq!(
        get_remote_snapshots_existence_with_task_workflow_task_remote(
            remote_port,
            "repo",
            &["SNP-7".to_string()],
        )
        .unwrap()["snapshot_ids"][0],
        "SNP-7"
    );
}

#[test]
fn task_workflow_task_record_remote_helpers_accept_narrow_trait_object() {
    let mut remote = FakeTaskRecordRemote;
    let remote_port: &mut dyn TaskWorkflowTaskRecordRemote = &mut remote;

    assert_eq!(
        get_task_with_task_workflow_task_remote(remote_port, "T-1", Some("repo")).unwrap()
            ["task_id"],
        "T-1"
    );
    assert_eq!(
        list_tasks_with_task_workflow_task_remote(remote_port, "repo").unwrap()[0]["task_id"],
        "T-1"
    );
    assert_eq!(
        read_task_audit_with_task_workflow_task_remote(remote_port, "repo", "T-1", "main").unwrap()
            ["target_line"],
        "main"
    );
    assert_eq!(
        create_task_with_task_workflow_task_remote(
            remote_port,
            "repo",
            "title",
            "intent",
            Some("T-2"),
            Some("PLAN-1"),
            Some("REV-1"),
            Some("1"),
        )
        .unwrap()["task_id"],
        "T-2"
    );
}

#[test]
fn task_workflow_task_record_helpers_accept_single_capability_ports() {
    let mut reader = FakeRemoteTaskReaderPort;
    assert_eq!(
        get_task_with_task_workflow_task_remote(&mut reader, "T-1", Some("repo")).unwrap()
            ["task_id"],
        "T-1"
    );

    let mut lister = FakeRemoteTaskListerPort;
    assert_eq!(
        list_tasks_with_task_workflow_task_remote(&mut lister, "repo").unwrap()[0]["task_id"],
        "T-LIST"
    );

    let mut audit_reader = FakeRemoteTaskAuditReaderPort;
    assert_eq!(
        read_task_audit_with_task_workflow_task_remote(&mut audit_reader, "repo", "T-1", "main")
            .unwrap()["target_line"],
        "main"
    );

    let mut creator = FakeRemoteTaskCreatorPort;
    assert_eq!(
        create_task_with_task_workflow_task_remote(
            &mut creator,
            "repo",
            "title",
            "intent",
            Some("T-2"),
            Some("PLAN-1"),
            Some("REV-1"),
            Some("1"),
        )
        .unwrap()["task_id"],
        "T-2"
    );
}
