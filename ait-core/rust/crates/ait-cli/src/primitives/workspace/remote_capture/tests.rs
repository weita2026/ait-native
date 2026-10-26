use super::*;
use crate::init_surface::{init_repo, InitRequest};
use ait_core::plan_http_client::{PlanHttpClientConfig, RemoteActorIdentity};
use ait_core::server_operational::RepositoryIndex;
use ait_core::task_workflow_http_adapter::HttpTaskRemote;
use std::time::Duration;
use tiny_http::{Response, Server};

#[test]
fn ownership_lookup_treats_remote_404_as_absent_without_weakening_capture_readback() {
    let temp = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("capture".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    for status in [404, 403, 500] {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let worker = std::thread::spawn(move || {
            for _ in 0..3 {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.method().as_str(), "GET");
                assert!(request.url().contains("/snapshots/SNP-000000000001"));
                request
                    .respond(Response::from_string("Unknown snapshot").with_status_code(status))
                    .unwrap();
            }
        });
        let remote = HttpTaskRemote::new(PlanHttpClientConfig {
            base_url: url,
            repository_index: Some(RepositoryIndex::new(0)),
            default_timeout_ms: 1000,
            ..PlanHttpClientConfig::default()
        })
        .unwrap();
        let mut context = CaptureContext {
            remote,
            repo_name: "capture".into(),
            repository_index: 0,
            capabilities: ait_core::remote_sync_backend::RemoteSyncCapabilities::default(),
        };
        let owners = ownership_rows_with_context(&repo, &["SNP-000000000001".into()], &mut context);
        let acknowledgment = read_binding(&repo, &mut context, "SNP-000000000001");
        let missing = capture_missing_remotely(&mut context, "SNP-000000000001");
        worker.join().unwrap();
        assert!(
            acknowledgment.is_err(),
            "capture acknowledgment must remain strict"
        );
        if status == 404 {
            assert!(owners.unwrap().is_empty());
            assert!(missing.unwrap());
        } else {
            assert!(
                owners.is_err(),
                "status {status} must not hide an authority failure"
            );
            assert!(missing.is_err());
        }
    }
}

#[test]
fn existing_capture_requires_exact_acknowledgment_and_never_resolves_a_new_author() {
    let temp = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("capture".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    fs::write(temp.path().join("content.txt"), "original author A").unwrap();
    let mut snapshot = create_local_snapshot(
        temp.path().to_string_lossy().as_ref(),
        "capture",
        "main",
        Some("capture"),
        false,
    )
    .unwrap();
    snapshot["newly_created"] = json!(false);
    let id = required_string_field(&snapshot, "snapshot_id").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let owner = ait_core::workflow_binary_db::SnapshotWorkflowBinding {
        task_id: "RT-1".into(),
        change_id: String::new(),
        line_name: "main".into(),
    };
    let binding = SnapshotCaptureBinding {
        repository_index: 0,
        snapshot_id: id.clone(),
        owner: owner.clone(),
    };
    let mut other = binding.clone();
    other.owner.task_id = "RT-2".into();
    for (acknowledged, succeeds) in [
        (None, false),
        (Some(other), false),
        (Some(binding.clone()), true),
    ] {
        let server = Server::http("127.0.0.1:0").unwrap();
        let url = format!("http://{}", server.server_addr());
        let payload = json!({
            "snapshot_id":id, "line_name":"main", "actor_key_index":7,
            "capture_binding":acknowledged.as_ref().map(SnapshotCaptureBinding::to_value),
        });
        let worker = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(request.method().as_str(), "GET");
            assert!(request.url().contains("/snapshots/"));
            request
                .respond(Response::from_string(payload.to_string()))
                .unwrap();
            assert!(server
                .recv_timeout(Duration::from_millis(30))
                .unwrap()
                .is_none());
        });
        let mut remote = HttpTaskRemote::new(PlanHttpClientConfig {
            base_url: url,
            repository_index: Some(RepositoryIndex::new(0)),
            default_timeout_ms: 1000,
            ..PlanHttpClientConfig::default()
        })
        .unwrap();
        // B's effective identity must not trigger registration or overwrite A
        // while this operation only resumes an acknowledged immutable capture.
        remote.set_actor_registration(
            "capture",
            RemoteActorIdentity::new("B", "B@Example").map_err(|error| error.to_string()),
            Some("R"),
        );
        let mut context = CaptureContext {
            remote,
            repo_name: "capture".into(),
            repository_index: 0,
            capabilities:
                ait_core::remote_sync_backend::RemoteSyncCapabilities::with_zstd_pack_bulk(),
        };
        let result = commit(&repo, &mut context, &snapshot, &owner);
        if succeeds {
            assert_eq!(result.unwrap(), binding.to_value());
        } else {
            assert!(result
                .unwrap_err()
                .contains("exact server-acknowledged capture binding"));
        }
        worker.join().unwrap();
    }
}
