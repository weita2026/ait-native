use super::*;

#[test]
fn remote_worktree_retarget_uses_authoritative_change_without_local_change_record() {
    let repo_tmp = tempdir().expect("repo tempdir");
    init_repo(&InitRequest {
        root: repo_tmp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_tmp.path()).expect("repo runtime");
    let metadata = JsonMap::from_iter([
        (
            "bound_change_id".to_string(),
            JsonValue::String("RC-REMOTE".to_string()),
        ),
        (
            "target_base_line".to_string(),
            JsonValue::String("main".to_string()),
        ),
    ]);
    let authoritative_change = json!({
        "change_id": "RC-REMOTE",
        "base_line": "main",
    });

    let summary = worktree_retarget_summary_with_change(
        &repo,
        &metadata,
        Some("feature/remote"),
        None,
        Some(&authoritative_change),
    )
    .expect("authoritative remote change must not require a local change record");

    assert_eq!(summary["target_base_line"], json!("main"));
}

#[test]
fn workflow_land_action_rejects_every_ready_preparation_action() {
    let repo_tmp = tempdir().expect("repo tempdir");
    init_repo(&InitRequest {
        root: repo_tmp.path().to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let repo = RepoRuntime::discover_from_path(repo_tmp.path()).expect("repo runtime");
    let state = json!({});

    for code in [
        "snapshot_create",
        "publish_patchset",
        "refresh_patchset",
        "record_attestation",
        "run_patchset_ci",
    ] {
        let result = workflow_land_apply_action(&repo, code, &state, "RC-1", None, None)
            .expect("land preparation action should stop without mutation");
        let reason = result["stopped_reason"].as_str().expect("stopped reason");
        assert!(reason.contains("does not"), "{code}: {reason}");
    }
}

#[test]
fn task_audit_payload_normalizes_remote_read_model_schema() {
    let payload = json!({
        "task_id": "RT-1",
        "target_line": "main",
        "target_line_head": "SNP-MAIN",
        "task": {
            "task_id": "RT-1",
            "title": "Remote task",
            "status": "active"
        },
        "summary": {
            "verdict": "in_progress",
            "open_changes": 1,
            "landed_changes": 0,
            "total_changes": 1
        },
        "verdict": {
            "code": "in_progress",
            "status": "in_progress"
        },
        "changes": []
    });

    let normalized = normalize_task_audit_payload(payload, "main");

    assert_eq!(normalized["workflow"]["state"], json!("in_progress"));
    assert_eq!(
        normalized["workflow"]["reason"],
        json!("1 open change(s) are still linked to this task.")
    );
    assert_eq!(normalized["queue_workflow"], normalized["workflow"]);
    assert_eq!(normalized["target"]["line_name"], json!("main"));
    assert_eq!(normalized["target"]["head_snapshot_id"], json!("SNP-MAIN"));
    assert_eq!(
        normalized["task_land_contract"]["version"],
        json!("task-land-plan-closeout/v1")
    );
    assert_eq!(
        normalized["task_land_closeout"]["status"],
        json!("pending_open_changes")
    );
    assert_eq!(
        normalized["task_land_closeout"]["plan_closeout_policy"],
        json!("separate_after_land")
    );
}

#[test]
fn completed_remote_task_audit_overrides_stale_ready_to_close_without_assuming_plan_sync() {
    let payload = json!({
        "task_id": "RCT-8",
        "target_line": "main",
        "task": {
            "task_id": "RCT-8",
            "status": "completed",
            "plan_id": "PR-8",
            "plan_item_ref": "release/fix"
        },
        "summary": {
            "verdict": "ready_to_close",
            "open_changes": 0,
            "landed_changes": 1,
            "total_changes": 1
        },
        "verdict": {
            "code": "ready_to_close",
            "status": "ready_to_close"
        },
        "changes": []
    });

    let normalized = normalize_task_audit_payload(payload, "main");

    assert_eq!(normalized["workflow"]["state"], "task_completed");
    assert_eq!(normalized["summary"]["verdict"], "task_completed");
    assert_eq!(normalized["verdict"]["status"], "task_completed");
    assert_eq!(
        normalized["task_land_closeout"]["status"],
        "plan_closeout_unverified"
    );
    assert_eq!(
        normalized["task_land_closeout"]["recovery"]["code"],
        "inspect_bound_plan"
    );
}

fn spawn_wait_hint_history_remote() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind remote");
    let addr = listener.local_addr().expect("remote addr");
    let server = Server::from_listener(listener, None).expect("remote server");
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(2)) else {
                break;
            };
            let method = request.method().as_str().to_string();
            let url = request.url().to_string();
            let payload = match (method.as_str(), url.as_str()) {
                ("GET", "/v1/native/repository-authorities/7/changes") => json!([{
                    "change_id": "RC-HIST",
                    "status": "landed",
                    "current_patchset_number": 1,
                    "selected_patchset_number": 1,
                    "landed_at": "2026-06-20T00:05:00Z"
                }]),
                ("GET", "/v1/native/repository-authorities/7/changes/RC-HIST") => json!({
                    "selected_patchset": {
                        "patchset_id": "RP-HIST",
                        "created_at": "2026-06-20T00:00:00Z"
                    },
                    "patchset_ci_status": {
                        "ci_completed_at_s": 1_781_913_720_u64
                    },
                    "change": {
                        "change_id": "RC-HIST",
                        "landed_at": "2026-06-20T00:05:00Z"
                    }
                }),
                _ => json!({"error": format!("unexpected {method} {url}")}),
            };
            let status = if payload.get("error").is_some() {
                404
            } else {
                200
            };
            request
                .respond(
                    Response::from_data(
                        crate::json_support::encode_value_to_vec_error_string(&payload)
                            .expect("encode payload"),
                    )
                    .with_status_code(status)
                    .with_header(
                        tiny_http::Header::from_bytes(
                            &b"Content-Type"[..],
                            &b"application/json"[..],
                        )
                        .expect("content-type"),
                    ),
                )
                .expect("respond");
        }
    });
    (format!("http://{addr}"), handle)
}

fn spawn_workflow_ready_run_ci_remote(
) -> (String, Arc<Mutex<Vec<JsonValue>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind remote");
    let addr = listener.local_addr().expect("remote addr");
    let server = Server::from_listener(listener, None).expect("remote server");
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let bodies_for_thread = Arc::clone(&bodies);
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let Ok(Some(mut request)) = server.recv_timeout(Duration::from_secs(2)) else {
                break;
            };
            let method = request.method().as_str().to_string();
            let url = request.url().to_string();
            let mut body_bytes = Vec::new();
            request
                .as_reader()
                .read_to_end(&mut body_bytes)
                .expect("read request body");
            let body = String::from_utf8_lossy(&body_bytes).to_string();
            let payload = match (method.as_str(), url.as_str()) {
                ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1") => {
                    bodies_for_thread.lock().expect("body log").push(json!({
                        "method": method,
                        "url": url,
                        "body": JsonValue::Null,
                    }));
                    json!({
                        "patchset_id": "RP-1",
                        "change_id": "RC-1",
                        "repo_name": "fixture-ait"
                    })
                }
                ("POST", "/v1/native/repository-authorities/7/patchsets/RP-1:runCi") => {
                    let parsed: JsonValue =
                        crate::json_support::parse_value(&body, "Invalid request JSON")
                            .unwrap_or_else(|_| json!({}));
                    bodies_for_thread.lock().expect("body log").push(json!({
                        "method": method,
                        "url": url,
                        "body": parsed,
                    }));
                    json!({
                        "patchset_id": "RP-1",
                        "queued": false,
                        "tests_status": "pass",
                        "execution_profile": PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND,
                        "selected_suite_ids": [
                            "preflight",
                            "stable_smoke",
                            "tg1_required"
                        ],
                        "suite_results": [
                            {
                                "suite_id": "preflight",
                                "runner_kind": "rust_server_ci",
                                "status": "pass"
                            },
                            {
                                "suite_id": "stable_smoke",
                                "runner_kind": "rust_server_ci",
                                "status": "pass"
                            },
                            {
                                "suite_id": "tg1_required",
                                "runner_kind": "rust_server_tg1_required",
                                "status": "pass",
                                "tg1_required_summary": {
                                    "status": "pass",
                                    "validation_status": "pass",
                                    "live_count": 24,
                                    "minimum_count": 24
                                }
                            }
                        ]
                    })
                }
                _ => json!({"error": format!("unexpected {method} {url}")}),
            };
            let status = if payload.get("error").is_some() {
                500
            } else {
                200
            };
            request
                .respond(
                    Response::from_string(payload.to_string())
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .expect("content-type"),
                        ),
                )
                .expect("respond");
        }
    });
    (format!("http://{addr}"), bodies, handle)
}

fn spawn_workflow_ready_attestation_remote(
) -> (String, Arc<Mutex<Vec<JsonValue>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind remote");
    let addr = listener.local_addr().expect("remote addr");
    let server = Server::from_listener(listener, None).expect("remote server");
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let bodies_for_thread = Arc::clone(&bodies);
    let handle = thread::spawn(move || {
        for _ in 0..2 {
            let Ok(Some(mut request)) = server.recv_timeout(Duration::from_secs(2)) else {
                break;
            };
            let method = request.method().as_str().to_string();
            let url = request.url().to_string();
            let mut body_bytes = Vec::new();
            request
                .as_reader()
                .read_to_end(&mut body_bytes)
                .expect("read request body");
            let body = String::from_utf8_lossy(&body_bytes).to_string();
            let payload = match (method.as_str(), url.as_str()) {
                ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1") => {
                    bodies_for_thread.lock().expect("body log").push(json!({
                        "method": method,
                        "url": url,
                        "body": JsonValue::Null,
                    }));
                    json!({
                        "patchset_id": "RP-1",
                        "change_id": "RC-1",
                        "repo_name": "fixture-ait"
                    })
                }
                ("PUT", "/v1/native/repository-authorities/7/patchsets/RP-1/attestation") => {
                    let parsed: JsonValue =
                        crate::json_support::parse_value(&body, "Invalid request JSON")
                            .unwrap_or_else(|_| json!({}));
                    bodies_for_thread.lock().expect("body log").push(json!({
                        "method": method,
                        "url": url,
                        "body": parsed,
                    }));
                    json!({
                        "attestation_id": "AT-RP-1",
                        "patchset_id": "RP-1",
                        "author_mode": "ai_with_human_review",
                        "evaluation_summary": parsed.get("evaluation_summary").cloned().unwrap_or(JsonValue::Null)
                    })
                }
                _ => json!({"error": format!("unexpected {method} {url}")}),
            };
            let status = if payload.get("error").is_some() {
                500
            } else {
                200
            };
            request
                .respond(
                    Response::from_string(payload.to_string())
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .expect("content-type"),
                        ),
                )
                .expect("respond");
        }
    });
    (format!("http://{addr}"), bodies, handle)
}

fn spawn_workflow_ready_done_remote(
    revision_snapshot_id: &str,
) -> (String, Arc<Mutex<Vec<JsonValue>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind remote");
    let addr = listener.local_addr().expect("remote addr");
    let server = Server::from_listener(listener, None).expect("remote server");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let requests_for_thread = Arc::clone(&requests);
    let revision_snapshot_id = revision_snapshot_id.to_string();
    let handle = thread::spawn(move || {
        for _ in 0..9 {
            let Ok(Some(mut request)) = server.recv_timeout(Duration::from_secs(2)) else {
                break;
            };
            let method = request.method().as_str().to_string();
            let url = request.url().to_string();
            let path = url.split('?').next().unwrap_or(url.as_str()).to_string();
            let mut body_bytes = Vec::new();
            request
                .as_reader()
                .read_to_end(&mut body_bytes)
                .expect("read request body");
            let body = String::from_utf8_lossy(&body_bytes).to_string();
            let body_value = if body.trim().is_empty() {
                JsonValue::Null
            } else {
                crate::json_support::parse_value(&body, "Invalid request JSON")
                    .unwrap_or_else(|_| json!(body))
            };
            requests_for_thread
                .lock()
                .expect("request log")
                .push(json!({
                    "method": method,
                    "url": url,
                    "body": body_value,
                }));
            let payload = match (method.as_str(), path.as_str()) {
                ("GET", "/v1/native/repository-authorities/7/changes/RC-DONE") => json!({
                    "change_id": "RC-DONE",
                    "task_id": "RT-DONE",
                    "repo_name": "fixture-ait",
                    "status": "review",
                    "base_line": "main",
                    "selected_patchset_id": "RP-DONE",
                    "current_patchset_id": "RP-DONE",
                    "change": {
                        "change_id": "RC-DONE",
                        "status": "review"
                    }
                }),
                ("GET", "/v1/native/repository-authorities/7/tasks/RT-DONE") => json!({
                    "task_id": "RT-DONE",
                    "repo_name": "fixture-ait",
                    "status": "active",
                    "title": "Ready done"
                }),
                ("GET", "/v1/native/repository-authorities/7/patchsets/RP-DONE") => json!({
                    "patchset_id": "RP-DONE",
                    "change_id": "RC-DONE",
                    "repo_name": "fixture-ait",
                    "base_snapshot_id": revision_snapshot_id.clone(),
                    "revision_snapshot_id": revision_snapshot_id.clone(),
                    "author_mode": "ai_with_human_review",
                    "summary": "ready done",
                    "created_at": "2026-06-20T00:00:00Z"
                }),
                ("GET", "/v1/native/repository-authorities/7/lines/main") => json!({
                    "line_name": "main",
                    "repo_name": "fixture-ait",
                    "status": "active",
                    "head_snapshot_id": revision_snapshot_id.clone()
                }),
                ("GET", "/v1/native/repository-authorities/7/changes/RC-DONE/reviews") => json!({
                    "change_id": "RC-DONE",
                    "current_patchset_id": "RP-DONE",
                    "approvals": 1,
                    "blocking": 0,
                    "reviews": [{
                        "review_id": "RR-TASK-DONE",
                        "patchset_id": "RP-DONE",
                        "reviewer": "Ready Reviewer",
                        "action": "task_approve",
                        "blocking": false
                    }]
                }),
                ("GET", "/v1/native/repository-authorities/7/patchsets/RP-DONE/attestation") => {
                    json!({
                        "attestation_id": "AT-RP-DONE",
                        "patchset_id": "RP-DONE",
                        "author_mode": "ai_with_human_review",
                        "evaluation_summary": {
                            "tests": "pass",
                            "lint": "pass"
                        }
                    })
                }
                ("GET", "/v1/native/repository-authorities/7/read/patchsets/RP-DONE/ci-status") => {
                    json!({
                        "available": true,
                        "patchset_id": "RP-DONE",
                        "tests_status": "pass",
                        "latest_job": null,
                        "recent_jobs": []
                    })
                }
                ("GET", "/v1/native/repository-authorities/7/patchsets/RP-DONE/policy") => json!({
                    "patchset_id": "RP-DONE",
                    "decision": "pass",
                    "checks": [
                        {"name": "tests", "status": "pass"},
                        {"name": "lint", "status": "pass"}
                    ]
                }),
                _ => json!({"error": format!("unexpected {method} {url}")}),
            };
            let status = if payload.get("error").is_some() {
                500
            } else {
                200
            };
            request
                .respond(
                    Response::from_string(payload.to_string())
                        .with_status_code(status)
                        .with_header(
                            tiny_http::Header::from_bytes(
                                &b"Content-Type"[..],
                                &b"application/json"[..],
                            )
                            .expect("content-type"),
                        ),
                )
                .expect("respond");
        }
    });
    (format!("http://{addr}"), requests, handle)
}

#[test]
fn workflow_ready_apply_resumes_done_authoritative_state() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    fs::write(repo_root.join("ready.txt"), "done").expect("ready file");
    let snapshot_repo = RepoRuntime::discover_from_path(repo_root).expect("snapshot repo runtime");
    let revision_snapshot = snapshot_create(&snapshot_repo, Some("ready revision"))
        .expect("create ready revision snapshot");
    let revision_snapshot_id =
        required_string_field(&revision_snapshot, "snapshot_id").expect("revision snapshot id");
    let (base_url, requests, handle) = spawn_workflow_ready_done_remote(&revision_snapshot_id);
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"fixture-ait","repository_index":7,"default_line":"main","current_line":"main","default_remote":"origin"}"#,
    );
    write_remote_config(repo_root, &base_url);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let mut progress_events = Vec::new();

    let result = workflow_ready_apply(
        &repo,
        "RC-DONE",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("origin"),
        Some(|event: &JsonValue| {
            progress_events.push(event.clone());
            Ok(())
        }),
    )
    .expect("ready apply done resume");

    assert_eq!(result["next_action"]["code"], json!("done"));
    assert_eq!(result["apply_status"], json!("done"));
    assert_eq!(result["applied_actions"], json!([]));
    assert_eq!(result["mutation_receipts"], json!([]));
    assert_eq!(
        result["apply_phase"],
        json!({
            "phase": "authoritative_resume",
            "code": "done",
            "detail": "Authoritative state already satisfies `workflow ready --apply`; no new mutation was needed.",
            "resumed_from_authoritative_state": true
        })
    );
    assert_eq!(
        progress_events
            .iter()
            .map(|event| {
                (
                    event["status"].as_str().unwrap_or_default().to_string(),
                    event["code"].as_str().unwrap_or_default().to_string(),
                    event["phase"].as_str().unwrap_or_default().to_string(),
                )
            })
            .collect::<Vec<_>>(),
        vec![
            (
                "probing".to_string(),
                "authoritative_state".to_string(),
                "authoritative_read".to_string(),
            ),
            (
                "resumed".to_string(),
                "done".to_string(),
                "authoritative_resume".to_string(),
            ),
        ]
    );
    assert_eq!(progress_events[1]["change_id"], json!("RC-DONE"));
    assert_eq!(progress_events[1]["patchset_id"], json!("RP-DONE"));

    handle.join().expect("remote thread");
    let logged = requests.lock().expect("request log");
    assert_eq!(logged.len(), 9);
    assert!(logged.iter().all(|entry| entry["method"] == json!("GET")));
    assert!(logged.iter().all(|entry| {
        let url = entry["url"].as_str().unwrap_or_default();
        !url.contains(":runCi")
            && !url.contains(":evaluatePolicy")
            && !url.contains(":land")
            && !url.contains(":select")
            && !url.contains("/read/changes/")
    }));
    assert!(logged.iter().any(|entry| {
        entry["url"]
            .as_str()
            .unwrap_or_default()
            .starts_with("/v1/native/repository-authorities/7/patchsets/RP-DONE?change_ref=RC-DONE")
    }));
}

#[test]
fn workflow_ready_ci_action_calls_remote_run_ci() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let (base_url, bodies, handle) = spawn_workflow_ready_run_ci_remote();
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"fixture-ait","repository_index":7,"default_line":"main","current_line":"main","default_remote":"origin"}"#,
    );
    write_remote_config(repo_root, &base_url);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let state = json!({
        "change": {"change_id": "RC-1", "repo_name": "fixture-ait"},
        "patchset": {
            "patchset_id": "RP-1",
            "change_id": "RC-1",
            "base_snapshot_id": "SNP-SEED",
            "revision_snapshot_id": "SNP-REV",
            "author_mode": "ai_with_human_review"
        }
    });

    let result = workflow_ready_apply_action(
        &repo,
        "run_patchset_ci",
        &state,
        "RC-1",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .expect("ready CI action");

    assert_eq!(
        result["result"]["execution_profile"].as_str(),
        Some(PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND)
    );
    assert_eq!(result["result"]["queued"].as_bool(), Some(false));
    assert_eq!(
        result["result"]["selected_suite_ids"],
        json!(["preflight", "stable_smoke", "tg1_required"])
    );
    assert_eq!(
        result["result"]["suite_results"][2]["tg1_required_summary"]["live_count"],
        json!(24)
    );
    handle.join().expect("remote thread");
    let logged = bodies.lock().expect("body log");
    assert_eq!(
        logged
            .iter()
            .map(|entry| entry["url"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![
            "/v1/native/repository-authorities/7/patchsets/RP-1",
            "/v1/native/repository-authorities/7/patchsets/RP-1:runCi",
        ]
    );
    assert_eq!(
        logged[1]["body"]["trigger"].as_str(),
        Some("workflow_ready_apply")
    );
    assert_eq!(
        logged[1]["body"]["execution_profile"].as_str(),
        Some(PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND)
    );
}

#[test]
fn workflow_ready_record_attestation_defaults_tests_without_persisting_ci_evidence() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    init_repo(&InitRequest {
        root: repo_root.to_path_buf(),
        name: Some("fixture-ait".to_string()),
        default_line: "main".to_string(),
        policy_profile: "prototype".to_string(),
        default_author_mode: "ai_with_human_review".to_string(),
        default_model: None,
        repair_existing: false,
    })
    .expect("init repo");
    let (base_url, bodies, handle) = spawn_workflow_ready_attestation_remote();
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"fixture-ait","repository_index":7,"default_line":"main","current_line":"main","default_remote":"origin"}"#,
    );
    write_remote_config(repo_root, &base_url);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let state = json!({
        "change": {"change_id": "RC-1", "repo_name": "fixture-ait"},
        "patchset": {
            "patchset_id": "RP-1",
            "change_id": "RC-1",
            "base_snapshot_id": "SNP-SEED",
            "revision_snapshot_id": "SNP-REV",
            "author_mode": "ai_with_human_review"
        },
        "patchset_ci_status": {
            "contract": "ait.server.patchset_ci.readiness.v1",
            "patchset_id": "RP-1",
            "tests_status": "pass",
            "has_runnable_evidence": true,
            "selected_suite_ids": ["preflight"],
            "suite_result_count": 1,
            "blocking_failure_count": 0,
            "latest_job": {
                "job_id": 43,
                "job_type": "patchset.ci",
                "state": "succeeded"
            }
        }
    });

    let result = workflow_ready_apply_action(
        &repo,
        "record_attestation",
        &state,
        "RC-1",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    )
    .expect("ready attestation action");

    assert_eq!(
        result["result"]["evaluation_summary"]["tests"].as_str(),
        Some("pass")
    );
    handle.join().expect("remote thread");
    let logged = bodies.lock().expect("body log");
    assert_eq!(
        logged
            .iter()
            .map(|entry| entry["url"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![
            "/v1/native/repository-authorities/7/patchsets/RP-1",
            "/v1/native/repository-authorities/7/patchsets/RP-1/attestation",
        ]
    );
    assert_eq!(
        logged[1]["body"]["evaluation_summary"]["tests"].as_str(),
        Some("pass")
    );
    let mut body_keys = logged[1]["body"]
        .as_object()
        .expect("attestation body")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    body_keys.sort();
    assert_eq!(
        body_keys,
        vec![
            "author_mode",
            "detail",
            "evaluation_summary",
            "provenance_summary",
        ]
    );
    assert_eq!(
        logged[1]["body"]["provenance_summary"]["evidence_readiness"].as_str(),
        Some("partial")
    );
    assert_eq!(
        logged[1]["body"]["detail"]["minimum_evidence"]["required_fields"],
        json!(["model_name"])
    );
    assert!(logged[1]["body"]["detail"].get("patchset_ci").is_none());
}

#[test]
fn workflow_wait_hint_bootstraps_ready_cache_from_remote_history() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    let (base_url, handle) = spawn_wait_hint_history_remote();
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"fixture-ait","repository_index":7,"default_remote":"origin"}"#,
    );
    write_remote_config(repo_root, &base_url);
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");

    let seconds = workflow_resolve_wait_hint_seconds(
        &repo,
        "ready",
        &json!({"resolved_remote_name":"origin"}),
    )
    .expect("resolve wait hint");

    assert_eq!(seconds, Some(120));
    let config = read_json_object_value(&repo_root.join(".ait/config.json"));
    assert_eq!(
        workflow_coerce_wait_hint_seconds(config.get(WORKFLOW_READY_POLL_SECONDS_KEY)),
        Some(120)
    );
    handle.join().expect("remote thread");
}

#[test]
fn workflow_ready_wait_hint_sample_updates_cached_ema() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"fixture-ait","workflow_ready_poll_seconds":60}"#,
    );
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let final_state = json!({"next_action":{"code":"done"}});
    let applied_actions = vec![
        json!({"code":"publish_patchset","result":{}}),
        json!({"code":"run_patchset_ci","result":{}}),
    ];

    let updated =
        workflow_maybe_record_ready_wait_hint_sample(&repo, &final_state, &applied_actions, 120.0)
            .expect("record wait hint");

    assert_eq!(updated, Some(90));
    let config = read_json_object_value(&repo_root.join(".ait/config.json"));
    assert_eq!(
        workflow_coerce_wait_hint_seconds(config.get(WORKFLOW_READY_POLL_SECONDS_KEY)),
        Some(90)
    );
}

fn write_line_state(root: &Path, line_name: &str, head_snapshot_id: Option<&str>) -> String {
    let repo = RepoRuntime::discover_from_path(root).expect("repo runtime");
    let lines = repo.line_store().expect("Binary DB line store");
    if lines.line_by_name(line_name).expect("read line").is_some() {
        lines
            .set_line_head(line_name, head_snapshot_id, "2026-06-20T00:00:00Z")
            .expect("update Binary DB line");
    } else {
        lines
            .create_line(line_name, head_snapshot_id, "2026-06-20T00:00:00Z")
            .expect("create Binary DB line");
    }
    if let Some(snapshot_id) = head_snapshot_id {
        return snapshot_id.to_string();
    }
    let snapshot = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(root)
        .expect("Binary DB snapshot coordinator")
        .create_snapshot("fixture-ait", line_name, Some("seed snapshot"), false)
        .expect("create Binary DB snapshot");
    required_string_field(&snapshot, "snapshot_id").expect("snapshot id")
}

#[expect(
    clippy::too_many_arguments,
    reason = "test fixture fields mirror persisted worktree registration metadata"
)]
fn write_worktree_registration(
    root: &Path,
    name: &str,
    line_name: &str,
    fork_snapshot_id: &str,
    bound_task_id: Option<&str>,
    bound_change_id: Option<&str>,
    auto_created_for_task: bool,
    cleanup_policy: Option<&str>,
) -> PathBuf {
    let worktree_root = root.join("managed").join(name);
    fs::create_dir_all(&worktree_root).expect("worktree dir");
    std::os::unix::fs::symlink(root.join(".ait"), worktree_root.join(".ait"))
        .expect("symlink .ait");
    fs::write(
        worktree_root.join(".ait-worktree.json"),
        format!(
            r#"{{"repo_root":"{}","workspace_root":"{}","worktree_name":"{}","current_line":"{}"}}"#,
            root.display(),
            worktree_root.display(),
            name,
            line_name
        ),
    )
    .expect("worktree config");
    let mut payload = JsonMap::from_iter([
        ("name".to_string(), JsonValue::String(name.to_string())),
        (
            "path".to_string(),
            JsonValue::String(worktree_root.to_string_lossy().to_string()),
        ),
        (
            "repo_root".to_string(),
            JsonValue::String(root.to_string_lossy().to_string()),
        ),
        (
            "line_name".to_string(),
            JsonValue::String(line_name.to_string()),
        ),
        (
            "fork_snapshot_id".to_string(),
            JsonValue::String(fork_snapshot_id.to_string()),
        ),
        (
            "forked_from_line".to_string(),
            JsonValue::String("main".to_string()),
        ),
        (
            "target_base_line".to_string(),
            JsonValue::String("main".to_string()),
        ),
        (
            "auto_created_for_task".to_string(),
            JsonValue::Bool(auto_created_for_task),
        ),
        (
            "created_at".to_string(),
            JsonValue::String("2026-06-20T00:00:00Z".to_string()),
        ),
    ]);
    if let Some(task_id) = bound_task_id {
        payload.insert(
            "bound_task_id".to_string(),
            JsonValue::String(task_id.to_string()),
        );
    }
    if let Some(change_id) = bound_change_id {
        payload.insert(
            "bound_change_id".to_string(),
            JsonValue::String(change_id.to_string()),
        );
    }
    if let Some(policy) = cleanup_policy {
        payload.insert(
            "cleanup_policy".to_string(),
            JsonValue::String(policy.to_string()),
        );
    }
    fs::create_dir_all(root.join(".ait").join("worktrees")).expect("worktree registry dir");
    write_json_pretty(
        &root
            .join(".ait")
            .join("worktrees")
            .join(format!("{name}.json")),
        &JsonValue::Object(payload),
    )
    .expect("write worktree registration");
    worktree_root
}

#[test]
fn task_worktree_shell_command_is_language_neutral_without_source_policy() {
    let repo_root = Path::new("/tmp/language-neutral-repo");
    let open_path = Path::new("/tmp/rct-0002");
    let shell_command = task_worktree_shell_command(repo_root, open_path);

    assert_eq!(shell_command, "cd /tmp/rct-0002");
    assert!(!shell_command.contains("CARGO_TARGET_DIR"));
    assert!(!shell_command.contains("CARGO_BUILD_BUILD_DIR"));
    assert!(!shell_command.contains("PYTHONPATH"));
    assert!(!shell_command.contains("export PATH="));
}

#[test]
fn task_worktree_shell_command_preserves_explicit_cargo_source_policy() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    fs::create_dir_all(repo_root.join(".cargo")).expect("Cargo config parent");
    fs::write(
        repo_root.join(".cargo/config.toml"),
        "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n",
    )
    .expect("Cargo source policy");
    let open_path = repo_root.join("managed/rct-0002");

    let shell_command = task_worktree_shell_command(repo_root, &open_path);

    assert!(shell_command.contains("export CARGO_TARGET_DIR="));
    assert!(shell_command.contains("export CARGO_BUILD_BUILD_DIR="));
}

#[test]
fn language_neutral_worktree_layout_ignores_cargo_manifest_without_source_policy() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"generic-fixture"}"#);
    fs::write(repo_root.join("Cargo.toml"), "[workspace]\n").expect("Cargo manifest");
    let worktree = repo_root.join("managed/task-one");
    fs::create_dir_all(&worktree).expect("worktree dir");
    std::os::unix::fs::symlink(repo_root.join(".ait"), worktree.join(".ait"))
        .expect("shared .ait link");

    ensure_worktree_runtime_layout(repo_root, &worktree).expect("language-neutral runtime layout");
    materialize_worktree_cargo_config(repo_root, &worktree)
        .expect("language-neutral Cargo materialization");

    assert!(!repo_root.join(".ait/cargo-build").exists());
    assert!(!repo_root.join(".ait/cargo-target").exists());
    assert!(!worktree.join(".cargo/config.toml").exists());
}

#[test]
fn worktree_summary_is_language_neutral_without_build_policy() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"ait-core","workflow_mode":"solo_remote","default_line":"main","current_line":"main"}"#,
    );
    write_line_state(repo_root, "feature/rct-0002", None);

    let worktree_root = repo_root.join("managed").join("rct-0002");
    fs::create_dir_all(&worktree_root).expect("worktree dir");
    std::os::unix::fs::symlink(repo_root.join(".ait"), worktree_root.join(".ait"))
        .expect("symlink .ait");
    fs::write(
        worktree_root.join(".ait-worktree.json"),
        format!(
            r#"{{"repo_root":"{}","workspace_root":"{}","worktree_name":"rct-0002","current_line":"feature/rct-0002"}}"#,
            repo_root.display(),
            worktree_root.display()
        ),
    )
    .expect("worktree config");
    let alias_path = repo_root.join(".ait-worktree-links").join("rct-0002");
    fs::create_dir_all(alias_path.parent().expect("alias parent")).expect("alias parent");
    std::os::unix::fs::symlink(&worktree_root, &alias_path).expect("worktree alias");

    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let payload = JsonMap::from_iter([
        (
            "name".to_string(),
            JsonValue::String("rct-0002".to_string()),
        ),
        (
            "path".to_string(),
            JsonValue::String(worktree_root.to_string_lossy().to_string()),
        ),
        (
            "alias_path".to_string(),
            JsonValue::String(
                repo_root
                    .join(".ait-worktree-links")
                    .join("rct-0002")
                    .to_string_lossy()
                    .to_string(),
            ),
        ),
        (
            "line_name".to_string(),
            JsonValue::String("feature/rct-0002".to_string()),
        ),
        (
            "repo_root".to_string(),
            JsonValue::String(repo_root.to_string_lossy().to_string()),
        ),
    ]);

    let summary =
        worktree_summary_from_metadata(&repo, &payload, false, false).expect("worktree summary");
    let status_summary = worktree_summary_from_metadata_for_repo_status(&repo, &payload)
        .expect("repository status worktree summary");
    assert!(summary.get("retarget").is_some());
    assert!(status_summary.get("retarget").is_none());
    assert!(status_summary.get("feature_ahead_count").is_none());
    let full_hygiene = worktree_doctor_from_rows(vec![summary.clone()]).expect("full hygiene");
    let status_hygiene = worktree_doctor_from_rows(vec![status_summary]).expect("status hygiene");
    for key in [
        "total_count",
        "current_count",
        "clean_count",
        "dirty_count",
        "missing_count",
        "detached_count",
        "protected_count",
        "safe_auto_remove_count",
        "safe_cleanup_candidate_count",
        "manual_review_candidate_count",
        "healthy",
        "stale_count",
    ] {
        assert_eq!(
            status_hygiene.get(key),
            full_hygiene.get(key),
            "metadata-only hygiene mismatch for {key}"
        );
    }
    let obj = summary.as_object().expect("summary object");
    let shell_command = obj
        .get("shell_command")
        .and_then(JsonValue::as_str)
        .expect("shell command");
    let cd_command = obj
        .get("cd_command")
        .and_then(JsonValue::as_str)
        .expect("cd command");

    assert_eq!(
        cd_command,
        format!(
            "cd {}",
            shell_escape(&repo_root.join(".ait-worktree-links").join("rct-0002"))
        )
    );
    assert_eq!(shell_command, cd_command);
    assert!(!shell_command.contains("CARGO_TARGET_DIR"));
    assert!(!shell_command.contains("CARGO_BUILD_BUILD_DIR"));
    assert!(!shell_command.contains("PYTHONPATH"));
    assert!(!shell_command.contains("export PATH="));
    assert!(obj["cargo_target_dir"].is_null());
    assert!(obj["cargo_build_dir"].is_null());
}

#[test]
fn repository_worktrees_share_ram_root_with_cargo_workspace_isolation() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    let memory_root = repo_root.join("AIT_RAM");
    fs::create_dir_all(&memory_root).expect("memory root");
    write_runtime_config(
        repo_root,
        &format!(
            r#"{{"repo_name":"Repo.Name","task_worktree":{{"memory_root":{{"kind":"macos_ram_volume","root":"{}"}}}}}}"#,
            memory_root.display()
        ),
    );
    let shared = ensure_repository_shared_cargo_build_dir(repo_root)
        .expect("repository shared cargo build dir");
    let expected = memory_root
        .join("ait-runtime")
        .join("cargo-build")
        .join("repo-name");
    assert_eq!(
        fs::canonicalize(&shared).expect("canonical shared path"),
        fs::canonicalize(&expected).expect("canonical physical path")
    );
    assert!(fs::symlink_metadata(repo_root.join(".ait/cargo-build"))
        .expect("shared build link")
        .file_type()
        .is_symlink());
    assert_eq!(
        ensure_repository_shared_cargo_build_dir(repo_root)
            .expect("preserve valid repository shared Cargo build link"),
        fs::canonicalize(&expected).expect("canonical valid shared build link")
    );

    let expected_root = fs::canonicalize(&expected)
        .expect("canonical expected build root")
        .to_path_buf();
    let expected_canonical = expected_root.join(CANONICAL_CARGO_BUILD_DIRNAME);
    assert_eq!(worktree_cargo_build_dir(repo_root), expected_canonical);
    for name in ["task-one", "task-two"] {
        let worktree = repo_root.join("managed").join(name);
        fs::create_dir_all(&worktree).expect("worktree");
        std::os::unix::fs::symlink(repo_root.join(".ait"), worktree.join(".ait"))
            .expect("shared .ait link");
        fs::write(
            worktree.join(WORKTREE_CONFIG_NAME),
            json!({"worktree_name": name}).to_string(),
        )
        .expect("worktree marker");
        assert_eq!(
            worktree_cargo_build_dir(&worktree),
            expected_root
                .join(MANAGED_WORKTREE_CARGO_BUILD_DIRNAME)
                .join(name)
        );
    }
}

#[test]
fn repository_cargo_build_setup_recovers_exact_managed_dangling_ram_link() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    let memory_root = repo_root.join("AIT_RAM");
    fs::create_dir_all(&memory_root).expect("memory root");
    write_runtime_config(
        repo_root,
        &format!(
            r#"{{"repo_name":"fixture","task_worktree":{{"memory_root":{{"kind":"macos_ram_volume","root":"{}"}}}}}}"#,
            memory_root.display()
        ),
    );
    let shared_path = repo_root.join(".ait/cargo-build");
    let expected = memory_root.join("ait-runtime/cargo-build/fixture");
    create_directory_link(&shared_path, &expected).expect("managed dangling build link");
    assert!(!expected.exists());

    let resolved = ensure_repository_shared_cargo_build_dir(repo_root)
        .expect("recover exact managed dangling Cargo build link");

    assert!(expected.is_dir());
    assert_eq!(
        resolved,
        fs::canonicalize(&expected).expect("canonical recovered build target")
    );
    assert!(fs::symlink_metadata(&shared_path)
        .expect("recovered shared build link")
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_link(&shared_path).expect("read recovered shared build link"),
        expected
    );
}

#[test]
fn repository_cargo_build_setup_rejects_unexpected_dangling_ram_link() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    let memory_root = repo_root.join("AIT_RAM");
    fs::create_dir_all(&memory_root).expect("memory root");
    write_runtime_config(
        repo_root,
        &format!(
            r#"{{"repo_name":"fixture","task_worktree":{{"memory_root":{{"kind":"macos_ram_volume","root":"{}"}}}}}}"#,
            memory_root.display()
        ),
    );
    let shared_path = repo_root.join(".ait/cargo-build");
    let unexpected = memory_root.join("unexpected/missing");
    create_directory_link(&shared_path, &unexpected).expect("unexpected dangling build link");

    let error = ensure_repository_shared_cargo_build_dir(repo_root)
        .expect_err("unexpected dangling Cargo build link must fail closed");

    assert!(error.contains("unexpected target"));
    assert!(!memory_root.join("ait-runtime/cargo-build/fixture").exists());
    assert!(!unexpected.exists());
    assert_eq!(
        fs::read_link(&shared_path).expect("preserved unexpected shared build link"),
        unexpected
    );
}

#[test]
fn repository_cargo_build_setup_preserves_existing_directory_and_custom_config() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    let memory_root = repo_root.join("AIT_RAM");
    fs::create_dir_all(&memory_root).expect("memory root");
    write_runtime_config(
        repo_root,
        &format!(
            r#"{{"repo_name":"fixture","task_worktree":{{"memory_root":{{"kind":"macos_ram_volume","root":"{}"}}}}}}"#,
            memory_root.display()
        ),
    );
    let existing = repo_root.join(".ait/cargo-build");
    fs::create_dir_all(&existing).expect("existing build dir");
    fs::write(existing.join("keep"), "keep\n").expect("existing marker");

    let resolved = ensure_repository_shared_cargo_build_dir(repo_root)
        .expect("preserve existing cargo build dir");

    assert_eq!(
        resolved,
        fs::canonicalize(&existing).expect("canonical existing dir")
    );
    assert!(!fs::symlink_metadata(&existing)
        .expect("existing build dir metadata")
        .file_type()
        .is_symlink());
    assert_eq!(
        fs::read_to_string(existing.join("keep")).expect("existing marker"),
        "keep\n"
    );
    assert!(!memory_root.join("ait-runtime/cargo-build/fixture").exists());

    let worktree = repo_root.join("managed/custom");
    fs::create_dir_all(worktree.join(".cargo")).expect("custom config parent");
    std::os::unix::fs::symlink(repo_root.join(".ait"), worktree.join(".ait"))
        .expect("shared .ait link");
    let custom_config = worktree.join(".cargo/config.toml");
    fs::write(&custom_config, "[build]\njobs = 3\n").expect("custom config");
    materialize_worktree_cargo_config(repo_root, &worktree).expect("preserve custom config");
    assert_eq!(
        fs::read_to_string(custom_config).expect("custom config contents"),
        "[build]\njobs = 3\n"
    );
}

#[test]
fn managed_worktree_cargo_config_upgrades_legacy_build_path() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"fixture"}"#);
    fs::create_dir_all(repo_root.join(".cargo")).expect("cargo config parent");
    fs::write(
        repo_root.join(".cargo/config.toml"),
        "# Managed by ait: stable final artifacts, worktree-local intermediates.\n[build]\njobs = 8\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \"rust/target\"\n",
    )
    .expect("legacy managed config");

    materialize_worktree_cargo_config(repo_root, repo_root).expect("upgrade managed config");

    let upgraded =
        fs::read_to_string(repo_root.join(".cargo/config.toml")).expect("upgraded config");
    assert!(upgraded
        .starts_with("# Managed by ait: workspace-isolated final artifacts and intermediates.\n"));
    assert!(upgraded.contains("jobs = 8\n"));
    assert!(upgraded.contains(&format!(
        "build-dir = \"{}\"\n",
        worktree_cargo_build_dir(repo_root).display()
    )));
}

#[test]
fn managed_worktree_cargo_config_upgrades_repository_shared_build_path() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"fixture"}"#);
    fs::create_dir_all(repo_root.join(".cargo")).expect("cargo config parent");
    let shared_ait_dir =
        fs::canonicalize(repo_root.join(".ait")).expect("canonical shared .ait directory");
    let target_dir = shared_ait_dir.join("cargo-target");
    let previous_build_dir = shared_ait_dir.join("cargo-build");
    fs::write(
        repo_root.join(".cargo/config.toml"),
        format!(
            "# Managed by ait: stable final artifacts, repository-shared intermediates.\n[build]\ntarget-dir = \"{}\"\nbuild-dir = \"{}\"\n",
            target_dir.display(),
            previous_build_dir.display(),
        ),
    )
    .expect("repository-shared managed config");

    materialize_worktree_cargo_config(repo_root, repo_root).expect("upgrade managed config");

    let upgraded =
        fs::read_to_string(repo_root.join(".cargo/config.toml")).expect("upgraded config");
    assert_eq!(upgraded, generated_worktree_cargo_config_text(repo_root));
    assert!(upgraded.contains("cargo-build/canonical"));
}

#[test]
fn managed_worktree_cargo_config_upgrades_hash_template_and_uses_exact_task_cache() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"fixture"}"#);
    let worktree = repo_root.join("managed/task-one");
    fs::create_dir_all(worktree.join(".cargo")).expect("Cargo config parent");
    std::os::unix::fs::symlink(repo_root.join(".ait"), worktree.join(".ait"))
        .expect("shared .ait link");
    fs::write(
        worktree.join(WORKTREE_CONFIG_NAME),
        json!({"worktree_name": "task-one"}).to_string(),
    )
    .expect("worktree marker");
    let source_config =
        "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/workspaces/{workspace-path-hash}\"\n\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n";
    fs::write(worktree.join(".cargo/config.toml"), source_config).expect("source Cargo config");
    assert!(!matches_generated_worktree_cargo_config_text(
        &worktree,
        source_config
    ));

    materialize_worktree_cargo_config(repo_root, &worktree).expect("upgrade managed config");

    let upgraded =
        fs::read_to_string(worktree.join(".cargo/config.toml")).expect("upgraded config");
    assert!(matches_generated_worktree_cargo_config_text(
        &worktree, &upgraded
    ));
    assert!(upgraded.contains(&format!(
        "target-dir = \"{}\"\n",
        worktree_cargo_target_dir(&worktree).display()
    )));
    assert!(upgraded.contains(&format!(
        "build-dir = \"{}\"\n",
        worktree_cargo_build_dir(&worktree).display()
    )));
    assert!(upgraded.contains("[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n"));
    assert!(!upgraded.contains("{workspace-path-hash}"));
}

#[test]
fn managed_worktree_cargo_config_upgrades_shared_final_artifact_projection() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"fixture"}"#);
    let worktree = repo_root.join("managed/task-one");
    fs::create_dir_all(worktree.join(".cargo")).expect("Cargo config parent");
    std::os::unix::fs::symlink(repo_root.join(".ait"), worktree.join(".ait"))
        .expect("shared .ait link");
    fs::write(
        worktree.join(WORKTREE_CONFIG_NAME),
        json!({"worktree_name": "task-one"}).to_string(),
    )
    .expect("worktree marker");
    fs::write(
        worktree.join(".cargo/config.toml"),
        format!(
            "# Managed by ait: stable final artifacts, workspace-isolated intermediates.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \"{}\"\n",
            worktree_cargo_build_dir(&worktree).display()
        ),
    )
    .expect("shared final-artifact projection");

    materialize_worktree_cargo_config(repo_root, &worktree)
        .expect("upgrade shared final-artifact projection");

    let upgraded =
        fs::read_to_string(worktree.join(".cargo/config.toml")).expect("upgraded config");
    assert_eq!(upgraded, generated_worktree_cargo_config_text(&worktree));
    assert!(upgraded
        .starts_with("# Managed by ait: workspace-isolated final artifacts and intermediates."));
    assert!(upgraded.contains("cargo-target/task-workspaces/task-one"));
}

#[test]
fn managed_worktrees_use_distinct_final_artifact_and_intermediate_directories() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(repo_root, r#"{"repo_name":"fixture"}"#);
    let first = repo_root.join("managed/task-one");
    let second = repo_root.join("managed/task-two");
    for (path, name) in [(&first, "task-one"), (&second, "task-two")] {
        fs::create_dir_all(path.join(".cargo")).expect("Cargo config parent");
        std::os::unix::fs::symlink(repo_root.join(".ait"), path.join(".ait"))
            .expect("shared .ait link");
        fs::write(
            path.join(WORKTREE_CONFIG_NAME),
            json!({"worktree_name": name}).to_string(),
        )
        .expect("worktree marker");
    }

    let first_target = worktree_cargo_target_dir(&first);
    let second_target = worktree_cargo_target_dir(&second);
    let first_build = worktree_cargo_build_dir(&first);
    let second_build = worktree_cargo_build_dir(&second);

    assert_ne!(first_target, second_target);
    assert_ne!(first_build, second_build);
    assert!(first_target.ends_with("cargo-target/task-workspaces/task-one"));
    assert!(second_target.ends_with("cargo-target/task-workspaces/task-two"));
    assert!(first_build.ends_with("cargo-build/task-workspaces/task-one"));
    assert!(second_build.ends_with("cargo-build/task-workspaces/task-two"));
    assert!(generated_worktree_cargo_config_text(&first)
        .contains(&format!("target-dir = \"{}\"", first_target.display())));
    assert!(generated_worktree_cargo_config_text(&second)
        .contains(&format!("target-dir = \"{}\"", second_target.display())));
}

#[test]
fn managed_worktree_cargo_config_retargets_copied_main_seed_projection() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"fixture","default_line":"main"}"#,
    );
    let seed = repo_root.join("managed/main-seed");
    let worktree = repo_root.join("managed/task-one");
    for (path, name) in [(&seed, "main-seed"), (&worktree, "task-one")] {
        fs::create_dir_all(path.join(".cargo")).expect("Cargo config parent");
        std::os::unix::fs::symlink(repo_root.join(".ait"), path.join(".ait"))
            .expect("shared .ait link");
        fs::write(
            path.join(WORKTREE_CONFIG_NAME),
            json!({"worktree_name": name}).to_string(),
        )
        .expect("worktree marker");
    }
    let copied = format!(
        "{}\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n",
        generated_worktree_cargo_config_text(&seed).trim_end()
    );
    fs::write(worktree.join(".cargo/config.toml"), &copied).expect("copied seed Cargo config");
    let config_path = worktree.join(".cargo/config.toml");
    set_portable_mode(&config_path, 0o444).expect("readonly copied config");
    assert!(
        !matches_generated_worktree_cargo_config_text(&worktree, &copied),
        "a projection for another worktree must not be hidden before retargeting"
    );

    materialize_worktree_cargo_config(repo_root, &worktree).expect("retarget copied seed config");

    let upgraded =
        fs::read_to_string(worktree.join(".cargo/config.toml")).expect("retargeted config");
    assert!(matches_generated_worktree_cargo_config_text(
        &worktree, &upgraded
    ));
    assert!(upgraded.contains(&format!(
        "target-dir = \"{}\"\n",
        worktree_cargo_target_dir(&worktree).display()
    )));
    assert!(upgraded.contains(&format!(
        "build-dir = \"{}\"\n",
        worktree_cargo_build_dir(&worktree).display()
    )));
    assert!(upgraded.contains("[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n"));
    assert!(!upgraded.contains("task-workspaces/main-seed"));
    assert_ne!(
        portable_mode(
            &fs::metadata(&config_path).expect("retargeted config metadata"),
            0o644,
        ) & 0o200,
        0,
        "retargeted task-worktree projection must be owner-writable"
    );
}

#[test]
fn managed_worktree_cargo_config_retargets_legacy_shared_final_main_seed_projection() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"fixture","default_line":"main"}"#,
    );
    let worktree = repo_root.join("managed/task-one");
    fs::create_dir_all(worktree.join(".cargo")).expect("Cargo config parent");
    std::os::unix::fs::symlink(repo_root.join(".ait"), worktree.join(".ait"))
        .expect("shared .ait link");
    fs::write(
        worktree.join(WORKTREE_CONFIG_NAME),
        json!({"worktree_name": "task-one"}).to_string(),
    )
    .expect("worktree marker");
    let copied_seed_build_dir = fs::canonicalize(repo_root.join(".ait"))
        .expect("canonical shared .ait")
        .join("cargo-build/task-workspaces/main-seed");
    let copied = format!(
        "# Managed by ait: stable final artifacts, workspace-isolated intermediates.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \"{}\"\n\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n",
        copied_seed_build_dir.display()
    );
    fs::write(worktree.join(".cargo/config.toml"), &copied)
        .expect("copied legacy seed Cargo config");

    materialize_worktree_cargo_config(repo_root, &worktree)
        .expect("retarget copied legacy seed config");

    let upgraded =
        fs::read_to_string(worktree.join(".cargo/config.toml")).expect("retargeted config");
    assert!(matches_generated_worktree_cargo_config_text(
        &worktree, &upgraded
    ));
    assert!(upgraded.contains(&format!(
        "target-dir = \"{}\"\n",
        worktree_cargo_target_dir(&worktree).display()
    )));
    assert!(upgraded.contains(&format!(
        "build-dir = \"{}\"\n",
        worktree_cargo_build_dir(&worktree).display()
    )));
    assert!(upgraded.contains("[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n"));
    assert!(!upgraded.contains("task-workspaces/main-seed"));
}

#[test]
fn workflow_bound_worktree_lookup_does_not_refresh_unrelated_worktree_metadata() {
    let repo_tmp = tempdir().expect("repo tempdir");
    let repo_root = repo_tmp.path();
    write_runtime_config(
        repo_root,
        r#"{"repo_name":"fixture-ait","workflow_mode":"solo_remote","default_line":"main","current_line":"main"}"#,
    );
    let base_snapshot_id = write_line_state(repo_root, "main", None);
    write_line_state(repo_root, "feature/rt-1", Some(&base_snapshot_id));
    write_line_state(repo_root, "feature/rt-extra", Some(&base_snapshot_id));
    write_worktree_registration(
        repo_root,
        "rt-1",
        "feature/rt-1",
        &base_snapshot_id,
        Some("RT-1"),
        Some("RC-1"),
        true,
        Some("after_remote_land"),
    );
    write_worktree_registration(
        repo_root,
        "rt-extra",
        "feature/rt-extra",
        &base_snapshot_id,
        None,
        None,
        false,
        Some("manual_only"),
    );
    let repo = RepoRuntime::discover_from_path(repo_root).expect("repo runtime");
    let extra_metadata_path = repo_root.join(".ait/worktrees/rt-extra.json");
    let before = read_json_value(&extra_metadata_path);
    assert!(before.get("workspace_status_cache").is_none());

    let bound = workflow_find_bound_task_worktree(&repo, "RT-1")
        .expect("bound worktree lookup")
        .expect("bound worktree row");

    assert_eq!(string_field(&bound, "name").as_deref(), Some("rt-1"));
    let after = read_json_value(&extra_metadata_path);
    assert!(after.get("workspace_status_cache").is_none());
}
