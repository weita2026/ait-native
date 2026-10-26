use super::*;
use crate::task_workflow_http_adapter::{HttpTaskRemote, HttpWorkflowCloseoutRemote};

#[test]
fn bound_change_context_rejects_a_mismatched_derived_reference() {
    let config = TaskWorkflowHttpClientConfig {
        base_url: "http://127.0.0.1:1/".to_string(),
        ..TaskWorkflowHttpClientConfig::default()
    };
    let mut task_remote = HttpTaskRemote::new(config.clone()).unwrap();
    let task_error = task_remote
        .set_bound_change_identity_context(Some("T-1"), Some("C-01"), Some("T-2/C-01"))
        .unwrap_err();
    assert!(task_error.contains("does not match derived `T-1/C-01`"));

    let mut closeout_remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
    let closeout_error = closeout_remote
        .set_bound_change_identity_context(Some("T-1"), Some("C-01"), Some("T-2/C-01"))
        .unwrap_err();
    assert!(closeout_error.contains("does not match derived `T-1/C-01`"));
}

#[test]
fn explicit_change_ref_does_not_inherit_an_unrelated_bound_task() {
    let response = json!({
        "task_id": "T-2",
        "change_id": "T-2/C-01",
        "status": "open"
    });
    let (config, server) = serve_task_workflow_json_once(response);
    let mut remote = HttpTaskRemote::new(config).unwrap();
    remote
        .set_bound_change_identity_context(Some("T-1"), Some("C-01"), Some("T-1/C-01"))
        .unwrap();

    let change = remote.get_change("T-2/C-01", Some("repo")).unwrap();
    assert_eq!(change["task_id"], "T-2");
    assert_eq!(change["change_id"], "C-01");
    assert_eq!(change["change_ref"], "T-2/C-01");

    let request = server.join().unwrap();
    assert_eq!(
        request.target,
        "/v1/native/repository-authorities/7/changes/T-2%2FC-01"
    );
}

#[test]
fn closeout_recovery_scopes_duplicate_short_change_ids_by_repository_and_task() {
    fn submit_target(repo_name: &str, task_id: &str) -> String {
        let response = json!({
            "submission_id": "LAND-SCOPED",
            "status": "succeeded",
            "result": {"target_line": "main"}
        });
        let (config, server) = serve_task_workflow_json_once(response);
        let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
        remote
            .set_bound_change_identity_context(Some(task_id), Some("C-01"), None)
            .unwrap();

        remote
            .submit_land("C-01", Some("P-01"), "main", "direct", Some(repo_name))
            .unwrap();

        let request = server.join().unwrap();
        assert_eq!(request.method, "POST");
        request.target
    }

    let first = submit_target("repo-alpha", "T-ALPHA");
    let second = submit_target("repo-beta", "T-BETA");
    assert_eq!(
        first,
        "/v1/native/repository-authorities/7/changes/T-ALPHA%2FC-01:submit"
    );
    assert_eq!(
        second,
        "/v1/native/repository-authorities/7/changes/T-BETA%2FC-01:submit"
    );
    assert_ne!(first, second);
}

#[test]
fn task_workflow_change_helpers_accept_single_capability_ports() {
    struct FakeChangeCreator;
    impl TaskWorkflowRemoteChangeCreator for FakeChangeCreator {
        fn create_change(
            &mut self,
            repo_name: &str,
            task_id: &str,
            title: &str,
            base_line: &str,
            change_id: Option<&str>,
            fork_snapshot_id: Option<&str>,
            forked_from_line: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            Ok(json!({
                "repo_name": repo_name,
                "task_id": task_id,
                "title": title,
                "base_line": base_line,
                "change_id": change_id,
                "fork_snapshot_id": fork_snapshot_id,
                "forked_from_line": forked_from_line,
            }))
        }
    }

    struct FakeChangeLister;
    impl TaskWorkflowRemoteChangeLister for FakeChangeLister {
        fn list_changes(
            &mut self,
            repo_name: &str,
        ) -> TaskWorkflowHttpClientResult<Vec<JsonValue>> {
            Ok(vec![json!({
                "repo_name": repo_name,
                "change_id": "C-1",
            })])
        }
    }

    struct FakeChangeDetailReader;
    impl TaskWorkflowRemoteChangeDetailReader for FakeChangeDetailReader {
        fn get_change_detail(
            &mut self,
            change_ref: &str,
            repo_name: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            Ok(json!({
                "repo_name": repo_name,
                "change_id": change_ref,
                "detail": true,
            }))
        }
    }

    struct FakeChangeReader;
    impl TaskWorkflowRemoteChangeReader for FakeChangeReader {
        fn get_change(
            &mut self,
            change_ref: &str,
            repo_name: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            Ok(json!({
                "repo_name": repo_name,
                "change_id": change_ref,
            }))
        }
    }

    struct FakeChangeCloser;
    impl TaskWorkflowRemoteChangeCloser for FakeChangeCloser {
        fn close_change(
            &mut self,
            change_ref: &str,
            status: &str,
            repo_name: Option<&str>,
        ) -> TaskWorkflowHttpClientResult<JsonValue> {
            Ok(json!({
                "repo_name": repo_name,
                "change_id": change_ref,
                "status": status,
            }))
        }
    }

    assert_eq!(
        create_change_with_task_workflow_task_remote(
            &mut FakeChangeCreator,
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
        list_changes_with_task_workflow_task_remote(&mut FakeChangeLister, "repo").unwrap()[0]
            ["change_id"],
        "C-1"
    );
    assert_eq!(
        get_change_detail_with_task_workflow_task_remote(
            &mut FakeChangeDetailReader,
            "C-1",
            Some("repo"),
        )
        .unwrap()["detail"],
        true
    );
    assert_eq!(
        get_change_with_task_workflow_task_remote(&mut FakeChangeReader, "C-1", Some("repo"))
            .unwrap()["change_id"],
        "C-1"
    );
    assert_eq!(
        close_change_with_task_workflow_task_remote(
            &mut FakeChangeCloser,
            "C-1",
            "archived",
            Some("repo"),
        )
        .unwrap()["status"],
        "archived"
    );
}
