use super::*;
use crate::init_surface::{init_repo, InitRequest};
use crate::primitives::workflow::task_land_attach_plan_checklist_closeout;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use std::time::Duration;
use tiny_http::{Response, Server};

#[test]
fn remote_finish_closes_exact_item_and_retries_failed_publication() {
    remote_sync_fixture(false, false, false);
}

#[test]
fn remote_finish_closes_exact_item_without_projection_taskable_hint() {
    remote_sync_fixture(false, false, true);
}

#[test]
fn normal_plan_sync_rebases_advanced_remote_without_flag_and_is_idempotent() {
    remote_sync_fixture(true, false, false);
}

#[test]
fn team_log_remote_sync_retries_and_preserves_checked_sprint() {
    remote_sync_fixture(false, true, false);
}

#[test]
fn team_log_and_automatic_rebase_work_together() {
    remote_sync_fixture(true, true, false);
}

fn remote_sync_fixture(automatic: bool, team: bool, omit_taskable_hint: bool) {
    let temp = tempfile::tempdir().unwrap();
    init_repo(&InitRequest {
        root: temp.path().to_path_buf(),
        name: Some("demo".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let artifact = "docs/sprints/remote.md";
    let original = "# Remote [plan-ref: remote/root]\n\n- [ ] Ship [ref: remote/ship]\n- [ ] Other [ref: remote/other]\n";
    let original = if team {
        original.replace("- [ ] Ship", "- [x] Ship")
    } else {
        original.to_string()
    };
    let original = original.as_str();
    fs::create_dir_all(temp.path().join("docs/sprints")).unwrap();
    fs::write(temp.path().join(artifact), original).unwrap();
    let mut repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let synced = execute_plan_sync_command_request_json(
        &plan_sync_request(&repo, artifact, Some("remote/root"), None, false)
            .unwrap()
            .to_string(),
    )
    .unwrap();
    let plan_id = synced["results"][0]["plan_id"]
        .as_str()
        .unwrap()
        .to_string();
    let revision_id = synced["results"][0]["plan_revision_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut shown = execute_plan_show_command_request_json(
        &plan_show_request(&repo, &plan_id, Some(&revision_id), None)
            .unwrap()
            .to_string(),
    )
    .unwrap();
    if omit_taskable_hint {
        for key in ["revision", "plan"] {
            let revision = if key == "revision" {
                &mut shown["revision"]
            } else {
                &mut shown["plan"]["head_revision"]
            };
            for item in revision["items"].as_array_mut().unwrap() {
                item.as_object_mut().unwrap().remove("taskable_hint");
            }
        }
    }
    let state = Arc::new(Mutex::new((
        shown["plan"].clone(),
        vec![shown["revision"].clone()],
        Vec::<String>::new(),
    )));
    let failing = Arc::new(AtomicBool::new(true));
    let stop = Arc::new(AtomicBool::new(false));
    let server = Server::http("127.0.0.1:0").unwrap();
    let url = format!("http://{}", server.server_addr());
    let shared = state.clone();
    let fail = failing.clone();
    let stopped = stop.clone();
    let worker = std::thread::spawn(move || {
        while !stopped.load(Ordering::SeqCst) {
            let Some(mut request) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                continue;
            };
            let method = request.method().as_str().to_string();
            let url = request.url().split('?').next().unwrap().to_string();
            let mut bytes = Vec::new();
            request.as_reader().read_to_end(&mut bytes).unwrap();
            let mut state = shared.lock().unwrap();
            state.2.push(format!("{method} {url}"));
            let prefix = "/v1/native/repository-authorities/7/sprints";
            let mut status = 200;
            let authority = json!({"repository_index":7,"repository_name":"demo","namespace":"","policy_flags":0});
            let payload = if method == "GET" && url == "/v1/native/repository-authorities/7" {
                authority
            } else if method == "POST" && url == "/v1/native/repository-authorities" {
                let body: JsonValue = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(body["repository_index"], 7);
                assert_eq!(body["user_name"], "Plan Fixture");
                assert_eq!(body["user_email"], "plan-fixture@example.test");
                json!({"repository":authority,"actor_key_index":9})
            } else if method == "GET" && url.ends_with("/reviews") {
                json!({"reviews":[{"patchset_id":"RCT-1/C-01/P-01","action":"code_review_summary","review_id":"RCT-1/C-01/P-01/R-01"}]})
            } else if method == "GET" && url.ends_with("/tasks") {
                json!([])
            } else if method == "GET" && url == prefix {
                json!([state.0.clone()])
            } else if method == "GET" && url.starts_with(prefix) {
                if url.ends_with("/revisions") {
                    json!(state.1.clone())
                } else if url.contains("/revisions/") {
                    let id = url
                        .rsplit('/')
                        .next()
                        .unwrap()
                        .replace("%3A", ":")
                        .replace("%3a", ":");
                    state
                        .1
                        .iter()
                        .find(|r| r["plan_revision_id"] == id)
                        .cloned()
                        .unwrap_or(JsonValue::Null)
                } else {
                    state.0.clone()
                }
            } else if method == "GET" && url.contains("/remote-sync/") {
                status = 404;
                json!({"detail":"pack absent"})
            } else if method == "PUT" && url.contains("/remote-sync/") {
                json!({"pack_id":url.rsplit('/').next().unwrap(),"stored":true,"pack_bytes":bytes.len(),"raw_binary_upload":true})
            } else if method == "POST" && url.ends_with("/remote-sync/zstd-bulk/commit") {
                let body: JsonValue = serde_json::from_slice(&bytes).unwrap();
                let ids = |key: &str, field: &str| {
                    body[key]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| r[field].clone())
                        .collect::<Vec<_>>()
                };
                json!({"repo_name":"demo","committed_snapshot_ids":ids("snapshots","snapshot_id"),"committed_object_pack_ids":ids("object_packs","pack_id"),"committed_tree_pack_ids":ids("tree_packs","pack_id"),"upserted_snapshots":0,"remote_line":null,"line_update":null})
            } else if method == "POST" && url.starts_with(prefix) {
                let body: JsonValue = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(body["actor_key_index"], 9);
                if fail.load(Ordering::SeqCst) {
                    status = 503;
                    json!({"detail":"injected publication failure"})
                } else {
                    let mut revision: JsonValue = serde_json::from_slice(&bytes).unwrap();
                    let id = format!("plan-revision:{}", 100 + state.1.len());
                    revision["plan_id"] = state.0["plan_id"].clone();
                    revision["plan_revision_id"] = json!(id);
                    revision["revision_number"] = json!(state.1.len() + 1);
                    revision["parent_plan_revision_id"] = state.0["head_revision_id"].clone();
                    state.1.push(revision.clone());
                    state.0["head_revision_id"] = json!(id);
                    state.0["head_revision"] = revision;
                    state.0.clone()
                }
            } else {
                status = 404;
                json!({"detail":format!("unexpected {method} {url}")})
            };
            request
                .respond(
                    Response::from_string(payload.to_string())
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes("Content-Type", "application/json")
                                .unwrap(),
                        ),
                )
                .unwrap();
        }
    });
    repo.config.insert("repository_index".into(), json!(7));
    repo.config
        .insert("default_remote".into(), json!("wrong-default"));
    repo.config.insert(
        "remotes".into(),
        json!({"review-server":{"remote_id":1,"url":url,"repo_name":"demo"}}),
    );
    repo.config.insert("agent_teams".into(), json!(team));
    repo.config
        .insert("user_name".into(), json!("Plan Fixture"));
    repo.config
        .insert("user_email".into(), json!("plan-fixture@example.test"));
    if team {
        for key in [
            "workflow_default_scope",
            "task_default_scope",
            "change_default_scope",
        ] {
            repo.config.insert(key.into(), json!("remote"));
        }
    }
    fs::write(
        repo.ait_dir.join("config.json"),
        json!(repo.config).to_string(),
    )
    .unwrap();
    let mut output = json!({"patchset_id":"RCT-1/C-01/P-01","apply_status":"done","task":{"task_id":"RCT-1","status":"completed","plan_id":plan_id,"origin_plan_revision_id":revision_id,"plan_item_ref":"remote/ship"}});
    task_land_attach_plan_checklist_closeout(&repo, &mut output, false, Some("review-server"));
    let first = output.clone();
    let after_failure = fs::read_to_string(temp.path().join(artifact)).unwrap();
    assert!(collect_planning_only_artifact_drift_paths(&repo)
        .unwrap()
        .is_empty());
    failing.store(false, Ordering::SeqCst);
    task_land_attach_plan_checklist_closeout(&repo, &mut output, false, Some("review-server"));
    assert!(collect_planning_only_artifact_drift_paths(&repo)
        .unwrap()
        .is_empty());
    let second = output.clone();
    let revisions_after_recovery = state.lock().unwrap().1.len();
    task_land_attach_plan_checklist_closeout(&repo, &mut output, false, Some("review-server"));
    if automatic {
        let local_body =
            fs::read_to_string(temp.path().join(artifact)).unwrap() + "\nLocal author update.\n";
        {
            let mut state = state.lock().unwrap();
            let mut head = state.1.last().unwrap().clone();
            head["parent_plan_revision_id"] = state.0["head_revision_id"].clone();
            head["plan_revision_id"] = json!("plan-revision:900");
            head["revision_number"] = json!(state.1.len() + 1);
            head["artifact_blob_id"] = json!(artifact_blob_id("remote advanced body"));
            state.0["head_revision_id"] = head["plan_revision_id"].clone();
            state.0["head_revision"] = head.clone();
            state.1.push(head);
        }
        fs::write(temp.path().join(artifact), &local_body).unwrap();
        let mut request = plan_sync_request(
            &repo,
            artifact,
            Some("remote/root"),
            Some("review-server"),
            false,
        )
        .unwrap();
        request.as_object_mut().unwrap().remove("rebase");
        let rebased = execute_plan_sync_command_request_json(&request.to_string()).unwrap();
        let again = execute_plan_sync_command_request_json(&request.to_string()).unwrap();
        stop.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        assert_eq!(rebased["status"], "ok", "{rebased}");
        assert_eq!(rebased["publish_results"][0]["rebased"], true, "{rebased}");
        assert_eq!(again["status"], "ok", "{again}");
        let state = state.lock().unwrap();
        assert_eq!(
            state.1.len(),
            revisions_after_recovery + 2,
            "successful recovery, remote advance, one replay; repeat sync adds nothing"
        );
        assert_eq!(
            state.0["head_revision"]["artifact_blob_id"],
            json!(artifact_blob_id(&local_body))
        );
        assert!(state
            .1
            .iter()
            .any(|r| r["plan_revision_id"] == "plan-revision:900"));
        assert_eq!(
            fs::read_to_string(temp.path().join(artifact)).unwrap(),
            local_body
        );
        return;
    }
    stop.store(true, Ordering::SeqCst);

    worker.join().unwrap();
    if team {
        assert_eq!(first["team_finish_log"]["status"], "pending", "{first}");
        assert_eq!(second["team_finish_log"]["status"], "synced", "{second}");
        assert_eq!(output["team_finish_log"]["status"], "synced", "{output}");
        let body = fs::read_to_string(temp.path().join(artifact)).unwrap();
        assert_eq!(body.matches("Reviewed and remote finished:").count(), 1);
        assert!(body.contains("- [x] Ship"));
        assert!(body.contains("- [ ] Other"));
        assert_eq!(body, after_failure);
        assert_eq!(output["closeout_status"], "complete", "{output}");
        let state = state.lock().unwrap();
        assert_eq!(
            state.1.len(),
            2,
            "retry must publish exactly one log revision"
        );
        assert_eq!(
            state.0["head_revision"]["artifact_blob_id"],
            json!(artifact_blob_id(&body))
        );
        return;
    }
    assert_eq!(first["closeout_status"], "partial", "{first}");
    assert!(
        first["plan_checklist_closeout"]["error"]
            .as_str()
            .unwrap()
            .contains("injected publication failure"),
        "{first}"
    );
    assert_eq!(after_failure, original);
    assert_eq!(
        second["plan_checklist_closeout"]["status"],
        "synced",
        "{second}; requests: {:?}",
        state.lock().unwrap().2
    );
    assert_eq!(second["closeout_status"], "complete");
    assert_eq!(
        output["plan_checklist_closeout"]["reason"], "already_done_resynced",
        "{output}"
    );
    assert_eq!(output["closeout_status"], "complete");
    let body = fs::read_to_string(temp.path().join(artifact)).unwrap();
    assert!(body.contains("- [x] Ship"));
    assert!(body.contains("- [ ] Other"));
    let state = state.lock().unwrap();
    assert_eq!(
        state.1.len(),
        revisions_after_recovery,
        "Successful closeout replay must not publish another revision"
    );
    assert_eq!(
        state.0["head_revision"]["artifact_blob_id"],
        json!(artifact_blob_id(&body))
    );
    assert!(
        state
            .2
            .iter()
            .any(|r| r.starts_with("POST ") && r.contains("/sprints/")),
        "{:?}",
        state.2
    );
    assert!(!state
        .2
        .iter()
        .any(|r| r.contains("/tasks/") || r.contains("/land")));
}
