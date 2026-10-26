use super::*;
use crate::task_workflow_http_adapter::HttpWorkflowCloseoutRemote;

#[test]
fn task_workflow_patchset_ci_readiness_http_adapter_uses_bounded_projection() {
    let response = json!({
        "contract": "ait.server.patchset_ci.readiness.v1",
        "projection": "readiness",
        "patchset_id": "P-C-1-1",
        "recent_limit_applied": 20
    });
    let (mut config, server) = serve_task_workflow_json_once(response.clone());
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote =
        HttpWorkflowCloseoutRemote::new(config).expect("HTTP workflow closeout remote");

    assert_eq!(
        remote
            .read_patchset_ci_readiness("P-C-1-1", 200, None, true)
            .expect("bounded readiness response"),
        response
    );
    let request = server.join().expect("readiness fixture server");
    assert_eq!(request.method, "GET");
    assert_eq!(
        request.target,
        "/v1/native/repository-authorities/7/read/patchsets/P-C-1-1/ci-status?recent_limit=20&projection=readiness"
    );
    assert_eq!(request.body, None);
}

#[test]
fn task_workflow_patchset_ci_dispatch_uses_repository_index_not_repository_name() {
    let response = json!({
        "patchset_id": "P-C-1-1",
        "queued": true
    });
    let (mut config, server) = serve_task_workflow_json_once(response.clone());
    config.repository_index = Some(crate::server_operational::RepositoryIndex::new(7));
    let mut remote =
        HttpWorkflowCloseoutRemote::new(config).expect("HTTP workflow closeout remote");

    assert_eq!(
        remote
            .run_patchset_ci(
                "P-C-1-1",
                "workflow_ready_apply",
                Some("workflow_ready_foreground"),
                Some("renamed-repository"),
                true,
            )
            .expect("repository-index CI dispatch"),
        response
    );
    let request = server.join().expect("runCi fixture server");
    assert_eq!(request.method, "POST");
    assert_eq!(
        request.target,
        "/v1/native/repository-authorities/7/patchsets/P-C-1-1:runCi"
    );
    assert_eq!(
        request.body,
        Some(json!({
            "trigger": "workflow_ready_apply",
            "execution_profile": "workflow_ready_foreground"
        }))
    );
}

#[test]
fn task_workflow_patchset_helpers_accept_patchset_remote_trait() {
    let mut remote = FakePatchsetRemote;
    let remote_port: &mut dyn TaskWorkflowPatchsetRemote = &mut remote;

    assert_eq!(
        publish_patchset_with_task_workflow_closeout_remote(
            remote_port,
            "C-1",
            "SNP-BASE",
            "SNP-REV",
            "summary",
            "codex",
            Some("repo"),
            true,
        )
        .unwrap()["patchset_id"],
        "P-C-1-1"
    );
    assert_eq!(
        list_patchsets_with_task_workflow_closeout_remote(remote_port, "C-1", Some("repo"))
            .unwrap()[0]["patchset_id"],
        "P-C-1-1"
    );
    assert_eq!(
        get_patchset_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            Some("repo"),
            Some("C-1"),
        )
        .unwrap()["change_ref"],
        "C-1"
    );
    assert_eq!(
        select_patchset_with_task_workflow_closeout_remote(
            remote_port,
            "C-1",
            "P-C-1-1",
            Some("repo"),
            true,
        )
        .unwrap()["selected"],
        true
    );
    assert_eq!(
        run_patchset_ci_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            "workflow_ready_apply",
            Some("foreground"),
            Some("repo"),
            true,
        )
        .unwrap()["queued"],
        true
    );
}

#[test]
fn task_workflow_patchset_helpers_accept_single_capability_ports() {
    let mut lister = FakePatchsetListerPort;
    assert_eq!(
        list_patchsets_with_task_workflow_closeout_remote(&mut lister, "C-1", Some("repo"))
            .unwrap()[0]["patchset_id"],
        "P-C-1-1"
    );

    let mut reader = FakePatchsetReaderPort;
    assert_eq!(
        get_patchset_with_task_workflow_closeout_remote(
            &mut reader,
            "P-C-1-1",
            Some("repo"),
            Some("C-1"),
        )
        .unwrap()["change_ref"],
        "C-1"
    );

    let mut publisher = FakePatchsetPublisherPort;
    assert_eq!(
        publish_patchset_with_task_workflow_closeout_remote(
            &mut publisher,
            "C-1",
            "SNP-BASE",
            "SNP-REV",
            "summary",
            "codex",
            Some("repo"),
            true,
        )
        .unwrap()["revision_snapshot_id"],
        "SNP-REV"
    );

    let mut selector = FakePatchsetSelectorPort;
    assert_eq!(
        select_patchset_with_task_workflow_closeout_remote(
            &mut selector,
            "C-1",
            "P-C-1-1",
            Some("repo"),
            true,
        )
        .unwrap()["selected"],
        true
    );

    let mut ci_runner = FakePatchsetCiRunnerPort;
    assert_eq!(
        run_patchset_ci_with_task_workflow_closeout_remote(
            &mut ci_runner,
            "P-C-1-1",
            "workflow_ready_apply",
            Some("foreground"),
            Some("repo"),
            true,
        )
        .unwrap()["queued"],
        true
    );

    let mut ci_status_reader = FakePatchsetCiStatusReaderPort;
    assert_eq!(
        read_patchset_ci_status_with_task_workflow_closeout_remote(
            &mut ci_status_reader,
            "P-C-1-1",
            10,
            Some("repo"),
            true,
        )
        .unwrap()["tests_status"],
        "pass"
    );
    assert_eq!(
        read_patchset_ci_readiness_with_task_workflow_closeout_remote(
            &mut ci_status_reader,
            "P-C-1-1",
            10,
            Some("repo"),
            true,
        )
        .unwrap()["tests_status"],
        "pass"
    );

    let mut repo_job_lister = FakeRepoJobListerPort;
    assert_eq!(
        list_repo_jobs_with_task_workflow_closeout_remote(
            &mut repo_job_lister,
            "repo",
            None,
            10,
            false,
            300,
        )
        .unwrap()["repo_name"],
        "repo"
    );
}

#[test]
fn task_workflow_patchset_ci_helpers_accept_patchset_ci_remote_trait() {
    let mut remote = FakePatchsetCiRemote;
    let remote_port: &mut dyn TaskWorkflowPatchsetCiRemote = &mut remote;

    assert_eq!(
        read_patchset_ci_status_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            10,
            Some("repo"),
            true,
        )
        .unwrap()["tests_status"],
        "pass"
    );
    assert_eq!(
        read_patchset_ci_readiness_with_task_workflow_closeout_remote(
            remote_port,
            "P-C-1-1",
            10,
            Some("repo"),
            true,
        )
        .unwrap()["tests_status"],
        "pass"
    );
    assert_eq!(
        list_repo_jobs_with_task_workflow_closeout_remote(
            remote_port,
            "repo",
            None,
            10,
            false,
            300,
        )
        .unwrap()["repo_name"],
        "repo"
    );
}
