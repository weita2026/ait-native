use super::*;

pub(super) fn spawn_fake_remote() -> FakeRemote {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::from_listener(listener, None).unwrap();
    let log = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let log_clone = Arc::clone(&log);
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    let state_clone = Arc::clone(&state);
    let shutdown = Arc::new(AtomicBool::new(false));
    let shutdown_clone = Arc::clone(&shutdown);
    let handle = thread::spawn(move || {
        while !shutdown_clone.load(Ordering::SeqCst) {
            let request = match server.recv_timeout(Duration::from_millis(100)) {
                Ok(Some(request)) => request,
                Ok(None) => continue,
                Err(_) => break,
            };
            let mut request = request;
            let mut body_bytes = Vec::new();
            request.as_reader().read_to_end(&mut body_bytes).unwrap();
            let body = String::from_utf8_lossy(&body_bytes).to_string();
            let url = request.url().to_string();
            let method = request.method().as_str().to_string();
            log_clone.lock().unwrap().push(RecordedRequest {
                method: method.clone(),
                url: url.clone(),
                body: body.clone(),
            });
            let response = response_for(&method, &url, &body, &state_clone);
            request.respond(response).unwrap();
        }
    });
    FakeRemote {
        base_url: format!("http://{}", addr),
        log,
        state,
        shutdown,
        handle: Some(handle),
    }
}

pub(super) fn response_for(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<FakeRemoteState>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    if let Some(response) = maybe_zstd_bulk_response(method, url, body, state) {
        return response;
    }

    let payload = match (method, url) {
        ("GET", "/healthz") | ("GET", "/v1/handshake") => {
            json!({
                "ci_capabilities": {
                    "patchset_run_ci_route": true,
                    "repo_run_ci_route": true,
                    "repo_ci_runs_route": true,
                    "remote_sync_capabilities": {
                        "zstd_pack_bulk": true
                    }
                },
                "ci_readiness": {
                    "runtime_generation": "current"
                }
            })
        }
        ("GET", "/v1/native/repository-authorities/7") => {
            json!({
                "contract": "ait.server.repository-authority.v1",
                "repository": {
                    "repository_index": 7,
                    "repository_name": "fixture-ait",
                    "namespace": "",
                    "policy_flags": 0,
                    "tombstoned": false
                }
            })
        }
        ("POST", "/v1/native/repository-authorities/7/tasks") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            json!({
                "task_id": parsed.get("task_id").cloned().unwrap_or(JsonValue::String("RT-REMOTE".to_string())),
                "published_task_id": parsed.get("task_id").cloned().unwrap_or(JsonValue::String("RT-REMOTE".to_string())),
                "title": parsed.get("title").cloned().unwrap_or(JsonValue::Null),
                "intent": parsed.get("intent").cloned().unwrap_or(JsonValue::Null),
                "repo_name":"fixture-ait",
                "status":"active"
            })
        }
        _ if method == "POST"
            && url == "/v1/native/repository-authorities/7/history-promotion:prepare" =>
        {
            history_promotion_response(body, state)
        }
        ("GET", "/v1/native/repository-authorities/7/tasks") => {
            let completed = state
                .lock()
                .map(|locked| locked.closed_task_ids.contains("RT-1"))
                .unwrap_or(false);
            json!([{
                "task_id":"RT-1",
                "repo_name":"fixture-ait",
                "title":"Published task",
                "status": if completed { "completed" } else { "active" },
                "publication_state":"published"
            }])
        }
        _ if method == "GET" && url.starts_with("/v1/native/repository-authorities/7/tasks/") => {
            let task_id = url.rsplit('/').next().unwrap_or("RT-1");
            let status = if state
                .lock()
                .map(|locked| locked.closed_task_ids.contains(task_id))
                .unwrap_or(false)
            {
                "completed"
            } else {
                "active"
            };
            json!({"task_id": task_id, "status": status})
        }
        _ if method == "POST"
            && url.starts_with("/v1/native/repository-authorities/7/tasks/")
            && url.ends_with(":close") =>
        {
            let parsed = parse_value_or(body, JsonValue::Null);
            let task_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/tasks/")
                .trim_end_matches(":close");
            if let Ok(mut locked) = state.lock() {
                locked.closed_task_ids.insert(task_id.to_string());
            }
            json!({"task_id": task_id, "status": parsed.get("status").cloned().unwrap_or(JsonValue::Null)})
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1") => {
            let guard = state.lock().unwrap();
            let submitted = guard.last_submitted_change_id.as_deref() == Some("RC-1");
            let selected_patchset_id = guard
                .selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            let landed_snapshot_id = guard
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| "SNP-REV".to_string());
            let landing_summary = if submitted {
                json!({
                    "submission_id":"LAND-1",
                    "change_id":"RC-1",
                    "patchset_id": selected_patchset_id.clone(),
                    "target_line":"main",
                    "status":"landed",
                    "result":{"landed_snapshot_id": landed_snapshot_id.clone()}
                })
            } else {
                JsonValue::Null
            };
            json!({
                "change_id":"RC-1",
                "task_id":"RT-1",
                "title":"Published review change",
                "base_line":"main",
                "fork_snapshot_id":"SNP-000000000001",
                "forked_from_line":"main",
                "status": if submitted { "landed" } else { "active" },
                "landed_snapshot_id": if submitted { JsonValue::String(landed_snapshot_id) } else { JsonValue::Null },
                "pre_land_target_snapshot_id": if submitted { JsonValue::String("SNP-000000000001".to_string()) } else { JsonValue::Null },
                "publication_state":"published",
                "published_remote_name":"origin",
                "published_change_id":"RC-1",
                "selected_patchset_id": selected_patchset_id,
                "landing_summary": landing_summary
            })
        }
        ("GET", "/v1/native/repository-authorities/7/read/changes/RC-1") => {
            let guard = state.lock().unwrap();
            let submitted = guard.last_submitted_change_id.as_deref() == Some("RC-1");
            let selected_patchset_id = guard
                .selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            let landed_snapshot_id = guard
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| "SNP-REV".to_string());
            let landing_summary = if submitted {
                json!({
                    "submission_id":"LAND-1",
                    "change_id":"RC-1",
                    "patchset_id": selected_patchset_id.clone(),
                    "target_line":"main",
                    "status":"landed",
                    "result":{"landed_snapshot_id": landed_snapshot_id.clone()}
                })
            } else {
                JsonValue::Null
            };
            json!({
                "change_id":"RC-1",
                "task_id":"RT-1",
                "status": if submitted { "landed" } else { "active" },
                "landed_snapshot_id": if submitted { JsonValue::String(landed_snapshot_id) } else { JsonValue::Null },
                "pre_land_target_snapshot_id": if submitted { JsonValue::String("SNP-000000000001".to_string()) } else { JsonValue::Null },
                "selected_patchset_id": selected_patchset_id,
                "landing_summary": landing_summary
            })
        }
        _ if method == "GET"
            && url.starts_with("/v1/native/repository-authorities/7/changes/")
            && !url.ends_with("/patchsets")
            && !url.ends_with("/reviews")
            && !url.contains(':') =>
        {
            completed_local_change_response(url, state)
        }
        _ if method == "GET"
            && url.starts_with("/v1/native/repository-authorities/7/read/changes/") =>
        {
            completed_local_change_response(url, state)
        }
        ("GET", "/v1/native/repository-authorities/7/changes") => {
            json!([{
                "change_id":"RC-1",
                "title":"Published review change",
                "base_line":"main",
                "current_patchset_number":1,
                "status":"active"
            }])
        }
        ("POST", "/v1/native/repository-authorities/7/changes") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            let change_id = parsed
                .get("change_id")
                .cloned()
                .unwrap_or(JsonValue::String("C-01".to_string()));
            let task_id = parsed.get("task_id").cloned().unwrap_or(JsonValue::Null);
            if let (Some(change_id), Some(task_id)) = (change_id.as_str(), task_id.as_str()) {
                if let Ok(mut locked) = state.lock() {
                    locked
                        .published_change_task_ids
                        .insert(change_id.to_string(), task_id.to_string());
                }
            }
            json!({
                "change_id": change_id,
                "task_id": task_id,
                "title": parsed.get("title").cloned().unwrap_or(JsonValue::Null),
                "base_line": parsed.get("base_line").cloned().unwrap_or(JsonValue::Null),
                "fork_snapshot_id": parsed.get("fork_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                "forked_from_line": parsed.get("forked_from_line").cloned().unwrap_or(JsonValue::Null),
                "status":"draft",
                "current_patchset_number":0
            })
        }
        ("GET", "/v1/native/repository-authorities/7/lines/main") => {
            let remote_head_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
            json!({"repo_name":"fixture-ait","line_name":"main","head_snapshot_id": remote_head_snapshot_id})
        }
        ("GET", "/v1/native/repository-authorities/7/lines") => {
            let remote_head_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
            json!([{
                "repo_name":"fixture-ait",
                "line_name":"main",
                "head_snapshot_id": remote_head_snapshot_id
            }])
        }
        ("PUT", "/v1/native/repository-authorities/7/lines/main") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            let head_snapshot_id = parsed
                .get("head_snapshot_id")
                .and_then(JsonValue::as_str)
                .map(ToString::to_string);
            state.lock().unwrap().remote_head_snapshot_id = head_snapshot_id.clone();
            json!({
                "repo_name":"fixture-ait",
                "line_name":"main",
                "head_snapshot_id": head_snapshot_id
            })
        }
        _ if method == "GET" && url.starts_with("/v1/native/repository-authorities/7/lines/") => {
            let line_name = url
                .trim_start_matches("/v1/native/repository-authorities/7/lines/")
                .replace("%2F", "/");
            let locked = state.lock().unwrap();
            let head_snapshot_id = locked
                .line_head_snapshot_ids
                .get(&line_name)
                .cloned()
                .or_else(|| locked.selected_patchset_revision_snapshot_id.clone());
            json!({
                "repo_name":"fixture-ait",
                "line_name": line_name.clone(),
                "head_snapshot_id": head_snapshot_id,
                "status": if locked.archived_line_names.contains(&line_name) { "archived" } else { "active" }
            })
        }
        _ if method == "PUT" && url.starts_with("/v1/native/repository-authorities/7/lines/") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            let line_name = url
                .trim_start_matches("/v1/native/repository-authorities/7/lines/")
                .replace("%2F", "/");
            let head_snapshot_id = parsed
                .get("head_snapshot_id")
                .and_then(JsonValue::as_str)
                .map(ToString::to_string);
            if let Some(head_snapshot_id) = head_snapshot_id.clone() {
                state
                    .lock()
                    .unwrap()
                    .line_head_snapshot_ids
                    .insert(line_name.clone(), head_snapshot_id);
            }
            json!({
                "repo_name":"fixture-ait",
                "line_name": line_name,
                "head_snapshot_id": head_snapshot_id,
                "status": "active"
            })
        }
        _ if method == "POST"
            && url.starts_with("/v1/native/repository-authorities/7/lines/")
            && url.ends_with(":close") =>
        {
            let parsed = parse_value_or(body, JsonValue::Null);
            let line_name = url
                .trim_start_matches("/v1/native/repository-authorities/7/lines/")
                .trim_end_matches(":close")
                .replace("%2F", "/");
            let mut locked = state.lock().unwrap();
            locked.archived_line_names.insert(line_name.clone());
            let head_snapshot_id = locked
                .line_head_snapshot_ids
                .get(&line_name)
                .cloned()
                .or_else(|| locked.selected_patchset_revision_snapshot_id.clone());
            json!({
                "repo_name":"fixture-ait",
                "line_name": line_name.clone(),
                "head_snapshot_id": head_snapshot_id,
                "status": parsed.get("status").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("POST", "/v1/native/repository-authorities/7/snapshots:exists") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            let snapshot_ids = parsed
                .get("snapshot_ids")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            json!({
                "repo_name":"fixture-ait",
                "checked_snapshots": snapshot_ids.len(),
                "present":[],
                "missing": snapshot_ids
            })
        }
        _ if method == "PUT"
            && url.starts_with("/v1/native/repository-authorities/7/snapshots/") =>
        {
            let snapshot_id = url.rsplit('/').next().unwrap_or("");
            json!({"snapshot_id": snapshot_id, "repo_name":"fixture-ait"})
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/patchsets") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            {
                let mut locked = state.lock().unwrap();
                locked.selected_change_id = Some("RC-1".to_string());
                locked.selected_patchset_id = Some("RP-2".to_string());
                locked.selected_patchset_base_snapshot_id = parsed
                    .get("base_snapshot_id")
                    .and_then(JsonValue::as_str)
                    .map(ToString::to_string);
                locked.selected_patchset_revision_snapshot_id = parsed
                    .get("revision_snapshot_id")
                    .and_then(JsonValue::as_str)
                    .map(ToString::to_string);
            }
            json!({
                "patchset_id":"RP-2",
                "change_id":"RC-1",
                "base_snapshot_id": parsed.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                "revision_snapshot_id": parsed.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                "summary": parsed.get("summary").cloned().unwrap_or(JsonValue::Null),
                "author_mode": parsed.get("author_mode").cloned().unwrap_or(JsonValue::Null)
            })
        }
        _ if method == "POST"
            && url.starts_with("/v1/native/repository-authorities/7/changes/")
            && url.ends_with("/patchsets") =>
        {
            let parsed = parse_value_or(body, JsonValue::Null);
            let change_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/changes/")
                .trim_end_matches("/patchsets")
                .replace("%2F", "/");
            let change_id = if change_id.contains('/') {
                change_id
            } else {
                format!("RT-REMOTE/{change_id}")
            };
            if let Ok(mut locked) = state.lock() {
                locked.selected_change_id = Some(change_id.clone());
                locked.selected_patchset_id = Some("RP-2".to_string());
                locked.selected_patchset_base_snapshot_id = parsed
                    .get("base_snapshot_id")
                    .and_then(JsonValue::as_str)
                    .map(ToString::to_string);
                locked.selected_patchset_revision_snapshot_id = parsed
                    .get("revision_snapshot_id")
                    .and_then(JsonValue::as_str)
                    .map(ToString::to_string);
            }
            json!({
                "patchset_id":"RP-2",
                "change_id": change_id,
                "base_snapshot_id": parsed.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                "revision_snapshot_id": parsed.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                "summary": parsed.get("summary").cloned().unwrap_or(JsonValue::Null),
                "author_mode": parsed.get("author_mode").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1/patchsets") => {
            let locked = state.lock().unwrap();
            let patchset_id = locked
                .selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            let ci_completed = patchset_id == "RP-1"
                || locked.ci_run_patchset_ids.contains(&patchset_id)
                || !locked.ci_run_required_patchset_ids.contains(&patchset_id);
            let base_snapshot_id = locked
                .selected_patchset_base_snapshot_id
                .clone()
                .unwrap_or_else(|| "SNP-000000000001".to_string());
            let revision_snapshot_id = locked
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| "SNP-REV".to_string());
            json!([with_embedded_patchset_ci(
                json!({
                    "patchset_id": patchset_id,
                    "change_id":"RC-1",
                    "patchset_number":1,
                    "base_snapshot_id": base_snapshot_id,
                    "revision_snapshot_id": revision_snapshot_id,
                    "publish_state":"published",
                    "evaluation_state":"pending"
                }),
                ci_completed
            )])
        }
        _ if method == "GET"
            && url.starts_with("/v1/native/repository-authorities/7/changes/")
            && url.ends_with("/patchsets") =>
        {
            let change_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/changes/")
                .trim_end_matches("/patchsets")
                .replace("%2F", "/");
            let change_id = if change_id.contains('/') {
                change_id
            } else {
                format!("RT-REMOTE/{change_id}")
            };
            let locked = state.lock().unwrap();
            let ci_completed = locked.ci_run_patchset_ids.contains("RP-2")
                || !locked.ci_run_required_patchset_ids.contains("RP-2");
            json!([with_embedded_patchset_ci(
                json!({
                    "patchset_id": locked.selected_patchset_id.clone().unwrap_or_else(|| "RP-2".to_string()),
                    "change_id": change_id,
                    "patchset_number":1,
                    "base_snapshot_id": locked.selected_patchset_base_snapshot_id.clone().unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string()),
                    "revision_snapshot_id": locked.selected_patchset_revision_snapshot_id.clone().unwrap_or_else(|| FIXTURE_FINAL_LOCAL_SNAPSHOT_ID.to_string()),
                    "publish_state":"published",
                    "evaluation_state":"pending"
                }),
                ci_completed
            )])
        }
        _ if method == "GET"
            && (url == "/v1/native/repository-authorities/7/patchsets/RP-1"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-1?change_ref=RC-1"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-2?change_ref=RC-1") =>
        {
            let locked = state.lock().unwrap();
            let patchset_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/patchsets/")
                .split('?')
                .next()
                .unwrap_or("RP-1");
            let ci_completed = patchset_id == "RP-1"
                || locked.ci_run_patchset_ids.contains(patchset_id)
                || !locked.ci_run_required_patchset_ids.contains(patchset_id);
            let base_snapshot_id = locked
                .selected_patchset_base_snapshot_id
                .clone()
                .unwrap_or_else(|| "SNP-000000000001".to_string());
            let revision_snapshot_id = locked
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| "SNP-REV".to_string());
            with_embedded_patchset_ci(
                json!({
                    "patchset_id": patchset_id,
                    "change_id":"RC-1",
                    "patchset_number":1,
                    "base_snapshot_id": base_snapshot_id,
                    "revision_snapshot_id": revision_snapshot_id,
                    "publish_state":"published",
                    "evaluation_state":"pending",
                    "summary":"Native Rust patchset"
                }),
                ci_completed,
            )
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-2") => {
            let locked = state.lock().unwrap();
            let ci_completed = locked.ci_run_patchset_ids.contains("RP-2")
                || !locked.ci_run_required_patchset_ids.contains("RP-2");
            with_embedded_patchset_ci(
                json!({
                    "patchset_id":"RP-2",
                    "change_id": locked.selected_change_id.clone().unwrap_or_else(|| "LC-0002".to_string()),
                    "patchset_number":1,
                    "base_snapshot_id": locked.selected_patchset_base_snapshot_id.clone().unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string()),
                    "revision_snapshot_id": locked.selected_patchset_revision_snapshot_id.clone().unwrap_or_else(|| FIXTURE_FINAL_LOCAL_SNAPSHOT_ID.to_string()),
                    "publish_state":"published",
                    "evaluation_state":"pending",
                    "summary":"Completed local promotion"
                }),
                ci_completed,
            )
        }
        _ if method == "GET"
            && url
                .starts_with("/v1/native/repository-authorities/7/patchsets/RP-2?change_ref=") =>
        {
            let locked = state.lock().unwrap();
            let ci_completed = locked.ci_run_patchset_ids.contains("RP-2")
                || !locked.ci_run_required_patchset_ids.contains("RP-2");
            with_embedded_patchset_ci(
                json!({
                    "patchset_id":"RP-2",
                    "change_id": locked.selected_change_id.clone().unwrap_or_else(|| "LC-0002".to_string()),
                    "patchset_number":1,
                    "base_snapshot_id": locked.selected_patchset_base_snapshot_id.clone().unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string()),
                    "revision_snapshot_id": locked.selected_patchset_revision_snapshot_id.clone().unwrap_or_else(|| FIXTURE_FINAL_LOCAL_SNAPSHOT_ID.to_string()),
                    "publish_state":"published",
                    "evaluation_state":"pending",
                    "summary":"Completed local promotion"
                }),
                ci_completed,
            )
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/1") => {
            with_embedded_patchset_ci(json!({"patchset_id":"RP-1","change_id":"RC-1"}), true)
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/1?change_ref=RC-1") => {
            with_embedded_patchset_ci(
                json!({
                    "patchset_id":"RP-1",
                    "change_id":"RC-1",
                    "patchset_number":1,
                    "base_snapshot_id":"SNP-000000000001",
                    "revision_snapshot_id":"SNP-REV",
                    "publish_state":"published",
                    "evaluation_state":"pending",
                    "summary":"Native Rust patchset"
                }),
                true,
            )
        }
        _ if method == "GET"
            && url.starts_with(
                "/v1/native/repository-authorities/7/read/patchsets/RP-1/ci-status",
            ) =>
        {
            json!({
                "patchset_id":"RP-1",
                "change_id":"RC-1",
                "tests_status":"pass",
                "latest_job":{"job_id":"JOB-1","state":"succeeded"}
            })
        }
        _ if method == "GET"
            && url.starts_with(
                "/v1/native/repository-authorities/7/read/patchsets/RP-2/ci-status",
            ) =>
        {
            let locked = state.lock().unwrap();
            let change_id = locked
                .selected_change_id
                .clone()
                .unwrap_or_else(|| "LC-0002".to_string());
            if locked.ci_run_patchset_ids.contains("RP-2")
                || !locked.ci_run_required_patchset_ids.contains("RP-2")
            {
                json!({
                    "contract":"ait.server.patchset_ci.readiness.v1",
                    "projection":"readiness",
                    "repo_name":"fixture-ait",
                    "patchset_id":"RP-2",
                    "change_id": change_id,
                    "available":true,
                    "has_runnable_evidence":true,
                    "tests_status":"pass",
                    "selected_suite_ids":["rust_core"],
                    "suite_results":[{"suite_id":"rust_core","status":"pass"}],
                    "suite_result_count":1,
                    "blocking_failure_count":0,
                    "recent_limit_applied":10,
                    "latest_job":{"job_id":2,"job_type":"patchset.ci","state":"succeeded"},
                    "recent_jobs":[{"job_id":2,"job_type":"patchset.ci","state":"succeeded"}]
                })
            } else {
                json!({
                    "contract":"ait.server.patchset_ci.readiness.v1",
                    "projection":"readiness",
                    "repo_name":"fixture-ait",
                    "patchset_id":"RP-2",
                    "change_id": change_id,
                    "available":true,
                    "has_runnable_evidence":false,
                    "tests_status":"pending",
                    "selected_suite_ids":[],
                    "suite_results":[],
                    "suite_result_count":0,
                    "blocking_failure_count":0,
                    "recent_limit_applied":10,
                    "latest_job": JsonValue::Null,
                    "recent_jobs": []
                })
            }
        }
        ("POST", "/v1/native/repository-authorities/7/patchsets/RP-1:runCi") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            json!({"patchset_id":"RP-1","queued":true,"trigger": parsed.get("trigger").cloned().unwrap_or(JsonValue::Null)})
        }
        ("POST", "/v1/native/repository-authorities/7/patchsets/RP-2:runCi") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            state
                .lock()
                .unwrap()
                .ci_run_patchset_ids
                .insert("RP-2".to_string());
            json!({"patchset_id":"RP-2","queued":true,"trigger": parsed.get("trigger").cloned().unwrap_or(JsonValue::Null)})
        }
        _ if method == "POST"
            && url.starts_with("/v1/native/repository-authorities/7/changes/")
            && url.ends_with(":selectPatchset") =>
        {
            let parsed = parse_value_or(body, JsonValue::Null);
            let change_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/changes/")
                .trim_end_matches(":selectPatchset")
                .replace("%2F", "/");
            let change_id = if change_id.contains('/') {
                change_id
            } else {
                format!("RT-REMOTE/{change_id}")
            };
            if let Some(patchset_id) = parsed.get("patchset_id").and_then(JsonValue::as_str) {
                let mut locked = state.lock().unwrap();
                locked.selected_patchset_id = Some(patchset_id.to_string());
                locked.selected_change_id = Some(change_id.clone());
            }
            json!({
                "change_id": change_id,
                "selected_patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/reviews") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            let action = parsed
                .get("action")
                .cloned()
                .unwrap_or(JsonValue::String("approve".to_string()));
            json!({
                "change_id":"RC-1",
                "patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                "reviewer": parsed.get("reviewer").cloned().unwrap_or(JsonValue::Null),
                "action": action
            })
        }
        _ if method == "POST"
            && url.starts_with("/v1/native/repository-authorities/7/changes/")
            && url.ends_with("/reviews") =>
        {
            let parsed = parse_value_or(body, JsonValue::Null);
            let change_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/changes/")
                .trim_end_matches("/reviews");
            let action = parsed
                .get("action")
                .cloned()
                .unwrap_or(JsonValue::String("approve".to_string()));
            json!({
                "change_id": change_id,
                "patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                "reviewer": parsed.get("reviewer").cloned().unwrap_or(JsonValue::Null),
                "action": action
            })
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1/reviews") => {
            let selected_patchset_id = state
                .lock()
                .unwrap()
                .selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            json!({
                "change_id":"RC-1",
                "current_patchset_id": selected_patchset_id.clone(),
                "approvals":1,
                "blocking":0,
                "comments":1,
                "task_approvals":1,
                "team_approvals":0,
                "human_approvals":1,
                "human_task_approvals":1,
                "independent_human_approvals":1,
                "independent_task_approvals":1,
                "code_review_summaries":1,
                "code_review_summary_reviewers":["codex"],
                "review_requests":[{"patchset_id": selected_patchset_id.clone(),"reviewer_group":"core","note":"Need review"}],
                "reviews":[
                    {"reviewer":"Fixture User <fixture@example.com>","patchset_id": selected_patchset_id.clone(),"action":"task_approve","blocking":false,"comment":"looks fine"},
                    {"reviewer":"codex","patchset_id": selected_patchset_id,"action":"code_review_summary","blocking":false,"comment":"Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: tg1; Recommendation: land"}
                ]
            })
        }
        _ if method == "GET"
            && url.starts_with("/v1/native/repository-authorities/7/changes/")
            && url.ends_with("/reviews") =>
        {
            let change_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/changes/")
                .trim_end_matches("/reviews");
            json!({
                "change_id": change_id,
                "current_patchset_id":"RP-2",
                "approvals":1,
                "blocking":0,
                "comments":1,
                "task_approvals":1,
                "team_approvals":0,
                "human_approvals":1,
                "human_task_approvals":1,
                "independent_human_approvals":1,
                "independent_task_approvals":1,
                "code_review_summaries":1,
                "code_review_summary_reviewers":["codex"],
                "review_requests":[],
                "reviews":[
                    {"reviewer":"Fixture User <fixture@example.com>","patchset_id":"RP-2","action":"task_approve","blocking":false,"comment":"looks fine"},
                    {"reviewer":"codex","patchset_id":"RP-2","action":"code_review_summary","blocking":false,"comment":"Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: tg1; Recommendation: land"}
                ]
            })
        }
        ("PUT", "/v1/native/repository-authorities/7/patchsets/RP-1/attestation") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            json!({
                "patchset_id":"RP-1",
                "author_mode": parsed.get("author_mode").cloned().unwrap_or(JsonValue::Null),
                "evaluation_summary": parsed.get("evaluation_summary").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1/attestation") => {
            json!({
                "attestation_id":"AT-1",
                "patchset_id":"RP-1",
                "author_mode":"ai_with_human_review",
                "evaluation_summary":{"tests":"pass"},
                "provenance_summary":{"policy_readable":true}
            })
        }
        ("PUT", "/v1/native/repository-authorities/7/patchsets/RP-2/attestation") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            state
                .lock()
                .unwrap()
                .attested_patchset_ids
                .insert("RP-2".to_string());
            json!({
                "patchset_id":"RP-2",
                "author_mode": parsed.get("author_mode").cloned().unwrap_or(JsonValue::Null),
                "evaluation_summary": parsed.get("evaluation_summary").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-2/attestation") => {
            if state.lock().unwrap().attested_patchset_ids.contains("RP-2") {
                json!({
                    "attestation_id":"AT-2",
                    "patchset_id":"RP-2",
                    "author_mode":"ai_with_human_review",
                    "evaluation_summary":{"tests":"pass"},
                    "provenance_summary":{"policy_readable":true}
                })
            } else {
                JsonValue::Null
            }
        }
        ("POST", "/v1/native/repository-authorities/7/patchsets/RP-1:evaluatePolicy") => {
            json!({"patchset_id":"RP-1","lane":"assisted","decision":"pass","evaluated_at":"2026-06-08T00:00:00Z"})
        }
        ("POST", "/v1/native/repository-authorities/7/patchsets/RP-2:evaluatePolicy") => {
            json!({"patchset_id":"RP-2","lane":"assisted","decision":"pass","evaluated_at":"2026-06-08T00:00:00Z"})
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1/policy") => {
            json!({
                "policy_id":"PO-1",
                "patchset_id":"RP-1",
                "decision":"pass",
                "content_class":"code",
                "author_class":"hybrid",
                "effective_requirements":{"require_tests":true},
                "evaluated_at":"2026-06-08T00:00:00Z",
                "checks":[{"name":"require_tests","status":"pass","message":"ok"}],
                "input_fingerprint":"abc"
            })
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-2/policy") => {
            json!({
                "policy_id":"PO-2",
                "patchset_id":"RP-2",
                "decision":"pass",
                "content_class":"code",
                "author_class":"hybrid",
                "effective_requirements":{"require_tests":true},
                "evaluated_at":"2026-06-08T00:00:00Z",
                "checks":[{"name":"require_tests","status":"pass","message":"ok"}],
                "input_fingerprint":"abc"
            })
        }
        _ if method == "POST" && url == "/v1/native/repository-authorities/7/task-land" => {
            atomic_task_land_response(body, state)
        }
        _ if method == "POST"
            && url.starts_with("/v1/native/repository-authorities/7/changes/")
            && url.ends_with(":submit") =>
        {
            let parsed = parse_value_or(body, JsonValue::Null);
            let change_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/changes/")
                .trim_end_matches(":submit")
                .replace("%2F", "/");
            let patchset_id = parsed
                .get("patchset_id")
                .cloned()
                .unwrap_or(JsonValue::Null);
            if let Ok(mut locked) = state.lock() {
                locked.last_submitted_change_id = Some(change_id.to_string());
                locked.last_submitted_patchset_id = patchset_id.as_str().map(ToString::to_string);
                let landed_snapshot_id = locked
                    .selected_patchset_revision_snapshot_id
                    .clone()
                    .unwrap_or_else(|| "SNP-000000000002".to_string());
                locked.remote_head_snapshot_id = Some(landed_snapshot_id);
            }
            json!({
                "submission_id":"LAND-1",
                "change_id": change_id,
                "patchset_id": patchset_id,
                "status":"landed"
            })
        }
        ("GET", "/v1/native/repository-authorities/7/lands/LAND-1")
        | ("GET", "/v1/native/repository-authorities/7/lands/1") => {
            let (change_id, patchset_id, landed_snapshot_id) = state
                .lock()
                .map(|locked| {
                    (
                        locked
                            .last_submitted_change_id
                            .clone()
                            .unwrap_or_else(|| "RC-1".to_string()),
                        locked
                            .last_submitted_patchset_id
                            .clone()
                            .unwrap_or_else(|| "RP-1".to_string()),
                        locked
                            .selected_patchset_revision_snapshot_id
                            .clone()
                            .or_else(|| locked.remote_head_snapshot_id.clone())
                            .unwrap_or_else(|| "SNP-000000000002".to_string()),
                    )
                })
                .unwrap_or_else(|_| {
                    (
                        "RC-1".to_string(),
                        "RP-1".to_string(),
                        "SNP-000000000002".to_string(),
                    )
                });
            json!({
                "submission_id":"LAND-1",
                "land_seq":1,
                "change_id": change_id,
                "patchset_id": patchset_id,
                "target_line":"main",
                "mode":"direct",
                "status":"landed",
                "result":{
                    "target_line":"main",
                    "base_snapshot_id":"SNP-000000000001",
                    "selected_revision_snapshot_id": landed_snapshot_id.clone(),
                    "landed_snapshot_id": landed_snapshot_id.clone(),
                    "line_action":"moved",
                    "snapshot_action":"selected_patchset_revision",
                    "archived_lines":["feature/rt-1"],
                    "freshness_preflight":{
                        "already_aligned_equivalent":true,
                        "base_is_fresh":false,
                        "target_line":"main",
                        "target_line_head": landed_snapshot_id.clone(),
                        "expected_base_snapshot_id":"SNP-000000000001",
                        "revision_snapshot_id": landed_snapshot_id.clone(),
                        "target_matches_revision_snapshot":true,
                        "target_matches_revision_tree":true,
                        "checked_at":"2026-06-08T00:00:00Z"
                    },
                    "phase_timings_ms":{"total_process_land":1.2}
                },
                "landed_snapshot_id": landed_snapshot_id.clone(),
                "result_json": encode_value_or(&json!({"landed_snapshot_id": landed_snapshot_id}), "{}")
            })
        }
        ("GET", "/v1/native/repository-authorities/7/tasks/RT-1") => {
            json!({"task_id":"RT-1"})
        }
        ("POST", "/v1/native/repository-authorities/7/tasks/RT-1:close") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            json!({"task_id":"RT-1","status": parsed.get("status").cloned().unwrap_or(JsonValue::Null)})
        }
        _ => panic!("unexpected request {method} {url} with body {body}"),
    };
    json_response(&payload)
}

fn atomic_task_land_response(body: &str, state: &Arc<Mutex<FakeRemoteState>>) -> JsonValue {
    let parsed = parse_value_or(body, JsonValue::Null);
    let requested_ref = parsed
        .get("task_or_change_ref")
        .and_then(JsonValue::as_str)
        .unwrap_or("RT-1");
    let requested_target = parsed
        .get("target_line")
        .and_then(JsonValue::as_str)
        .unwrap_or("main");
    let idempotency_key = parsed
        .get("idempotency_key")
        .and_then(JsonValue::as_str)
        .unwrap_or("task-land-atomic:fixture");

    let mut locked = state.lock().unwrap();
    let (task_id, change_id, change_ref) =
        if let Some((task_id, change_id)) = requested_ref.split_once('/') {
            (
                task_id.to_string(),
                change_id.to_string(),
                requested_ref.to_string(),
            )
        } else {
            let selected_change = locked
                .selected_change_id
                .clone()
                .unwrap_or_else(|| "RC-1".to_string());
            if let Some((task_id, change_id)) = selected_change.split_once('/') {
                (task_id.to_string(), change_id.to_string(), selected_change)
            } else {
                (
                    requested_ref.to_string(),
                    selected_change.clone(),
                    format!("{requested_ref}/{selected_change}"),
                )
            }
        };
    let patchset_id = locked
        .selected_patchset_id
        .clone()
        .unwrap_or_else(|| "RP-1".to_string());
    let landed_snapshot_id = locked
        .selected_patchset_revision_snapshot_id
        .clone()
        .or_else(|| locked.remote_head_snapshot_id.clone())
        .unwrap_or_else(|| "SNP-000000000002".to_string());
    let replayed = locked.closed_task_ids.contains(&task_id);
    locked.last_submitted_change_id = Some(change_id.clone());
    locked.last_submitted_patchset_id = Some(patchset_id.clone());
    locked.closed_task_ids.insert(task_id.clone());
    if let Some(entries) = locked
        .history_promotion
        .as_ref()
        .and_then(|promotion| promotion.get("entries"))
        .and_then(JsonValue::as_array)
    {
        let history_task_ids = entries
            .iter()
            .filter_map(|entry| string_field(entry, "task_id"))
            .collect::<Vec<_>>();
        locked.closed_task_ids.extend(history_task_ids);
    }
    locked.remote_head_snapshot_id = Some(landed_snapshot_id.clone());
    locked
        .line_head_snapshot_ids
        .insert(requested_target.to_string(), landed_snapshot_id.clone());
    let history_promotion = locked.history_promotion.clone().unwrap_or(JsonValue::Null);

    json!({
        "contract": "task-land-atomic/v1",
        "repo_name": "fixture-ait",
        "repository_index": 7,
        "idempotency_key": idempotency_key,
        "replayed": replayed,
        "status": "succeeded",
        "task_id": task_id,
        "task_status": "completed",
        "change_id": change_id,
        "change_ref": change_ref,
        "change_status": "landed",
        "patchset_id": patchset_id,
        "target_line": requested_target,
        "landed_snapshot_id": landed_snapshot_id,
        "task": {
            "task_id": task_id,
            "status": "completed"
        },
        "change": {
            "task_id": task_id,
            "change_id": change_id,
            "change_ref": change_ref,
            "status": "landed",
            "selected_patchset_id": patchset_id
        },
        "patchset": {
            "patchset_id": patchset_id,
            "revision_snapshot_id": landed_snapshot_id
        },
        "land": {
            "submission_id": format!("{change_ref}/L-01"),
            "status": "succeeded",
            "target_line": requested_target,
            "landed_snapshot_id": landed_snapshot_id
        },
        "history_promotion": history_promotion
    })
}

fn history_promotion_response(body: &str, state: &Arc<Mutex<FakeRemoteState>>) -> JsonValue {
    let parsed = parse_value_or(body, JsonValue::Null);
    let entries = parsed
        .get("entries")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let mappings = entries
        .iter()
        .enumerate()
        .map(|(ordinal, entry)| {
            let task_id = format!("RT-{:04}", ordinal + 1);
            json!({
                "local_task_id": entry.get("local_task_id").cloned().unwrap_or(JsonValue::Null),
                "local_change_id": entry.get("local_change_id").cloned().unwrap_or(JsonValue::Null),
                "local_change_ref": entry.get("local_change_ref").cloned().unwrap_or(JsonValue::Null),
                "task_id": task_id,
                "change_ref": format!("{task_id}/C-01"),
                "receipt_patchset_id": format!("RLP-{:02}", ordinal + 1),
            })
        })
        .collect::<Vec<_>>();
    let final_mapping = mappings.last().cloned().unwrap_or_else(|| json!({}));
    let final_task_id = final_mapping
        .get("task_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("RT-0001");
    let final_change_ref = final_mapping
        .get("change_ref")
        .and_then(JsonValue::as_str)
        .unwrap_or("RT-0001/C-01");
    let response = json!({
        "contract": "history-promotion-prepare/v1",
        "repo_name": "fixture-ait",
        "repository_index": 7,
        "idempotency_key": parsed.get("idempotency_key").cloned().unwrap_or(JsonValue::Null),
        "replayed": false,
        "target_line": parsed.get("target_line").cloned().unwrap_or(JsonValue::Null),
        "base_snapshot_id": parsed.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
        "revision_snapshot_id": parsed.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
        "entries": mappings,
        "aggregate": {
            "task_id": final_task_id,
            "change_ref": final_change_ref,
            "patchset_id": "RP-2",
            "patchset": {
                "patchset_id": "RP-2",
                "source_kind": "history_promotion_aggregate",
                "governance_authority": true,
                "base_snapshot_id": parsed.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                "revision_snapshot_id": parsed.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                "evaluation_state": "pending"
            }
        }
    });
    let mut locked = state.lock().unwrap();
    locked.selected_change_id = Some(final_change_ref.to_string());
    locked.selected_patchset_id = Some("RP-2".to_string());
    locked.selected_patchset_base_snapshot_id = parsed
        .get("base_snapshot_id")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string);
    locked.selected_patchset_revision_snapshot_id = parsed
        .get("revision_snapshot_id")
        .and_then(JsonValue::as_str)
        .map(ToString::to_string);
    locked
        .published_change_task_ids
        .insert("C-01".to_string(), final_task_id.to_string());
    locked.history_promotion = Some(json!({
        "contract": "ait-history-promotion/v1",
        "target_line": parsed.get("target_line").cloned().unwrap_or(JsonValue::Null),
        "base_snapshot_id": parsed.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
        "revision_snapshot_id": parsed.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
        "entries": response["entries"].clone(),
        "aggregate": response["aggregate"].clone(),
    }));
    response
}

fn with_embedded_patchset_ci(mut patchset: JsonValue, completed: bool) -> JsonValue {
    let Some(object) = patchset.as_object_mut() else {
        return patchset;
    };
    let (
        run_seq,
        completed_at_s,
        overall_status,
        tests_status,
        selected_suite_count,
        suite_result_count,
    ) = if completed {
        (1_u64, 1_783_814_400_u64, "pass", "pass", 1_u64, 1_u64)
    } else {
        (0_u64, 0_u64, "none", "none", 0_u64, 0_u64)
    };
    object.insert("ci_run_seq".to_string(), json!(run_seq));
    object.insert("ci_completed_at_s".to_string(), json!(completed_at_s));
    object.insert(
        "ci".to_string(),
        json!({
            "run_seq": run_seq,
            "completed_at_s": completed_at_s,
            "overall_status": overall_status,
            "tests_status": tests_status,
            "lint_status": "none",
            "selected_suite_count": selected_suite_count,
            "suite_result_count": suite_result_count,
            "blocking_failure_count": 0
        }),
    );
    patchset
}

fn completed_local_change_response(url: &str, state: &Arc<Mutex<FakeRemoteState>>) -> JsonValue {
    let encoded_change_id = url.rsplit('/').next().unwrap_or("C-01");
    let change_id = encoded_change_id
        .rsplit("%2F")
        .next()
        .unwrap_or(encoded_change_id);
    let locked = state.lock().unwrap();
    let submitted = locked.last_submitted_change_id.as_deref() == Some(change_id);
    let task_id = locked
        .published_change_task_ids
        .get(change_id)
        .cloned()
        .unwrap_or_else(|| "RT-REMOTE".to_string());
    let patchset_id = locked.selected_patchset_id.clone();
    let base_snapshot_id = locked
        .selected_patchset_base_snapshot_id
        .clone()
        .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
    let landed_snapshot_id = locked
        .selected_patchset_revision_snapshot_id
        .clone()
        .unwrap_or_else(|| FIXTURE_FINAL_LOCAL_SNAPSHOT_ID.to_string());
    json!({
        "change_id": change_id,
        "task_id": task_id,
        "title":"Local completed change",
        "base_line":"main",
        "fork_snapshot_id": base_snapshot_id,
        "forked_from_line":"main",
        "status": if submitted { "landed" } else { "active" },
        "landed_snapshot_id": if submitted { JsonValue::String(landed_snapshot_id.clone()) } else { JsonValue::Null },
        "pre_land_target_snapshot_id": if submitted { JsonValue::String(base_snapshot_id) } else { JsonValue::Null },
        "publication_state":"published",
        "published_remote_name":"origin",
        "published_change_id": change_id,
        "selected_patchset_id": patchset_id.clone(),
        "landing_summary": if submitted {
            json!({
                "submission_id":"LAND-1",
                "change_id": change_id,
                "patchset_id": patchset_id,
                "target_line":"main",
                "status":"landed",
                "result":{"landed_snapshot_id": landed_snapshot_id}
            })
        } else {
            JsonValue::Null
        }
    })
}

pub(super) fn zstd_bulk_ids(parsed: &JsonValue, field: &str, id_field: &str) -> Vec<JsonValue> {
    parsed
        .get(field)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get(id_field).and_then(JsonValue::as_str))
        .map(|id| JsonValue::String(id.to_string()))
        .collect()
}

pub(super) fn maybe_zstd_bulk_response(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<FakeRemoteState>>,
) -> Option<Response<std::io::Cursor<Vec<u8>>>> {
    const PREFIX: &str = "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/";
    let suffix = url.strip_prefix(PREFIX)?;
    match (method, suffix) {
        ("POST", "plan") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            let snapshot_ids = parsed
                .get("snapshot_ids")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            let present_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
            let mut present_snapshot_ids = Vec::new();
            let mut missing_snapshot_ids = Vec::new();
            for snapshot_id in snapshot_ids {
                let is_present = present_snapshot_id
                    .as_deref()
                    .is_some_and(|present| snapshot_id.as_str() == Some(present));
                if is_present {
                    present_snapshot_ids.push(snapshot_id);
                } else {
                    missing_snapshot_ids.push(snapshot_id);
                }
            }
            let object_pack_ids = zstd_bulk_ids(&parsed, "object_packs", "pack_id");
            let tree_pack_ids = zstd_bulk_ids(&parsed, "tree_packs", "pack_id");
            Some(json_response(&json!({
                "repo_name": "fixture-ait",
                "present_snapshot_ids": present_snapshot_ids,
                "missing_snapshot_ids": missing_snapshot_ids,
                "present_object_pack_ids": [],
                "missing_object_pack_ids": object_pack_ids,
                "present_tree_pack_ids": [],
                "missing_tree_pack_ids": tree_pack_ids
            })))
        }
        ("POST", "commit") => {
            let parsed = parse_value_or(body, JsonValue::Null);
            let committed_snapshot_ids = zstd_bulk_ids(&parsed, "snapshots", "snapshot_id");
            let committed_object_pack_ids = zstd_bulk_ids(&parsed, "object_packs", "pack_id");
            let committed_tree_pack_ids = zstd_bulk_ids(&parsed, "tree_packs", "pack_id");
            let remote_line = parsed
                .get("line_update")
                .filter(|value| !value.is_null())
                .map(|line_update| {
                    let head_snapshot_id = line_update
                        .get("head_snapshot_id")
                        .and_then(JsonValue::as_str)
                        .map(ToString::to_string);
                    if let Some(head_snapshot_id) = head_snapshot_id.clone() {
                        state.lock().unwrap().remote_head_snapshot_id = Some(head_snapshot_id);
                    }
                    json!({
                        "repo_name": "fixture-ait",
                        "line_name": line_update
                            .get("line_name")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("main"),
                        "status": "active",
                        "head_snapshot_id": head_snapshot_id
                    })
                })
                .unwrap_or(JsonValue::Null);
            Some(json_response(&json!({
                "repo_name": "fixture-ait",
                "committed_snapshot_ids": committed_snapshot_ids,
                "committed_object_pack_ids": committed_object_pack_ids,
                "committed_tree_pack_ids": committed_tree_pack_ids,
                "upserted_snapshots": committed_snapshot_ids.len(),
                "remote_line": remote_line,
                "line_update": JsonValue::Null
            })))
        }
        _ if method == "PUT" && suffix.starts_with("object-packs/") => {
            let pack_id = suffix.trim_start_matches("object-packs/");
            Some(json_response(&json!({
                "repo_name": "fixture-ait",
                "pack_id": pack_id,
                "stored": true,
                "pack_bytes": body.len(),
                "raw_binary_upload": true
            })))
        }
        _ if method == "PUT" && suffix.starts_with("tree-packs/") => {
            let pack_id = suffix.trim_start_matches("tree-packs/");
            Some(json_response(&json!({
                "repo_name": "fixture-ait",
                "pack_id": pack_id,
                "stored": true,
                "pack_bytes": body.len(),
                "raw_binary_upload": true
            })))
        }
        _ => None,
    }
}

pub(super) fn json_response(payload: &JsonValue) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = encode_value_to_vec(payload, "failed to encode smoke server response").unwrap();
    Response::from_data(body)
        .with_header(Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap())
}
