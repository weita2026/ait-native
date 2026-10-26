use std::fs;

use tempfile::tempdir;

use crate::json_support::{json, JsonValue};

use super::*;

fn repository_target(root: &Path) -> JsonValue {
    json!({
        "mode": "local",
        "workflow_mode": "solo_local",
        "repo_root": root.to_string_lossy(),
        "repo_name": "fixture-repo",
    })
}

fn request(root: &Path, operation: &str, arguments: JsonValue) -> JsonValue {
    json!({
        "operation": operation,
        "target": repository_target(root),
        "arguments": arguments,
    })
}

#[test]
fn agent_local_workflow_backend_fails_closed_for_every_supported_operation() {
    let temp = tempdir().expect("tempdir");
    fs::create_dir(temp.path().join(".ait")).expect("create .ait");
    let cases = [
        ("read_task_queue", json!({})),
        ("read_task", json!({"task_id": "T-1"})),
        ("read_change", json!({"change_id": "C-1"})),
        (
            "read_task_audit",
            json!({"task_id": "T-1", "target_line": "main"}),
        ),
    ];

    for (operation, arguments) in cases {
        let response =
            agent_local_workflow_backend_execute_json(&request(temp.path(), operation, arguments))
                .expect("classified response");
        assert_eq!(response["contract"], AGENT_LOCAL_WORKFLOW_BACKEND_CONTRACT);
        assert_eq!(response["operation"], operation);
        assert_eq!(response["ok"], false);
        assert_eq!(response["retryable"], false);
        assert_eq!(response["message"], LOCAL_WORKFLOW_AUTHORITY_ERROR);
        assert_eq!(response["error"]["kind"], "unsupported_authority");
        assert_eq!(response["error"]["message"], LOCAL_WORKFLOW_AUTHORITY_ERROR);
        assert!(response.get("payload").is_none());
    }
    assert!(fs::read_dir(temp.path().join(".ait"))
        .expect("read .ait")
        .next()
        .is_none());
}

#[test]
fn agent_local_workflow_backend_validates_operation_target_and_arguments() {
    let temp = tempdir().expect("tempdir");
    fs::create_dir(temp.path().join(".ait")).expect("create .ait");

    let unsupported =
        agent_local_workflow_backend_execute_json(&request(temp.path(), "write_task", json!({})))
            .expect_err("unsupported operation");
    assert!(unsupported.contains("Unsupported local workflow backend operation"));

    let wrong_mode = json!({
        "operation": "read_task_queue",
        "target": {
            "mode": "remote",
            "workflow_mode": "solo_remote",
            "repo_root": temp.path().to_string_lossy(),
            "repo_name": "fixture-repo",
        },
        "arguments": {},
    });
    assert!(agent_local_workflow_backend_execute_json(&wrong_mode)
        .expect_err("wrong mode")
        .contains("target mode must be `local`"));

    let missing_task = request(temp.path(), "read_task", json!({}));
    assert!(agent_local_workflow_backend_execute_json(&missing_task)
        .expect_err("missing task")
        .contains("arguments.task_id"));

    let missing_repo = request(
        Path::new("/definitely/missing/ait-repo"),
        "read_task_queue",
        json!({}),
    );
    assert!(agent_local_workflow_backend_execute_json(&missing_repo)
        .expect_err("missing repo")
        .contains("is not a directory"));
}

#[test]
fn agent_local_workflow_backend_source_has_no_retired_authority_dependencies() {
    let source = include_str!("mod.rs");
    for forbidden in [
        "task_store".to_string(),
        "change_store".to_string(),
        "workflow_event_store".to_string(),
        "workflow_release_store".to_string(),
        "agent_session_store".to_string(),
        "plan_http_client".to_string(),
    ] {
        assert!(
            !source.contains(&forbidden),
            "forbidden dependency: {forbidden}"
        );
    }
}
