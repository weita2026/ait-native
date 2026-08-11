use super::*;
use crate::task_workflow_http_adapter::HttpWorkflowCloseoutRemote;

#[test]
fn review_http_adapter_records_through_repository_authority() {
    let (mut config, server) = serve_task_workflow_json_once(json!({
        "change_id": "C-01",
        "task_id": "RCT-1",
        "patchset_id": "P-RCT-1/C-01-1",
        "reviewer": "reviewer@example.test",
        "action": "approve"
    }));
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(9));
    let mut remote = HttpWorkflowCloseoutRemote::new(config).unwrap();
    remote
        .set_bound_change_context(Some("RCT-1"), Some("C-01"))
        .unwrap();
    remote
        .record_review(
            "C-01",
            "P-RCT-1/C-01-1",
            "reviewer@example.test",
            "approve",
            None,
            false,
            Some("legacy-name"),
            true,
        )
        .unwrap();

    let recorded = server.join().unwrap();
    assert_eq!(recorded.method, "POST");
    assert_eq!(
        recorded.target,
        "/v1/native/repository-authorities/9/changes/RCT-1%2FC-01/reviews"
    );
}

#[test]
fn task_workflow_review_helpers_accept_review_remote_trait() {
    let mut remote = FakeReviewRemote;
    let remote_port: &mut dyn TaskWorkflowReviewRemote = &mut remote;
    let reviewer_groups = vec!["reviewers".to_string(), "owners".to_string()];

    assert_eq!(
        request_review_with_task_workflow_closeout_remote(
            remote_port,
            "C-1",
            "P-C-1-1",
            &reviewer_groups,
            Some("please review"),
            Some("repo"),
            true,
        )
        .unwrap()["requested"],
        true
    );
    assert_eq!(
        record_review_with_task_workflow_closeout_remote(
            remote_port,
            "C-1",
            "P-C-1-1",
            "reviewer@example.com",
            "approve",
            Some("looks good"),
            false,
            Some("repo"),
            true,
        )
        .unwrap()["action"],
        "approve"
    );
    assert_eq!(
        list_reviews_with_task_workflow_closeout_remote(remote_port, "C-1", Some("repo"), true)
            .unwrap()["reviews"][0]["action"],
        "approve"
    );
}

#[test]
fn task_workflow_review_helpers_accept_single_capability_ports() {
    let mut requester = FakeReviewRequesterPort;
    let mut recorder = FakeReviewRecorderPort;
    let mut lister = FakeReviewListerPort;
    let reviewer_groups = vec!["reviewers".to_string(), "owners".to_string()];

    assert_eq!(
        request_review_with_task_workflow_closeout_remote(
            &mut requester,
            "C-1",
            "P-C-1-1",
            &reviewer_groups,
            Some("please review"),
            Some("repo"),
            true,
        )
        .unwrap()["requested"],
        true
    );
    assert_eq!(
        record_review_with_task_workflow_closeout_remote(
            &mut recorder,
            "C-1",
            "P-C-1-1",
            "reviewer@example.com",
            "approve",
            Some("looks good"),
            false,
            Some("repo"),
            true,
        )
        .unwrap()["action"],
        "approve"
    );
    assert_eq!(
        list_reviews_with_task_workflow_closeout_remote(&mut lister, "C-1", Some("repo"), true)
            .unwrap()["reviews"][0]["action"],
        "approve"
    );
}
