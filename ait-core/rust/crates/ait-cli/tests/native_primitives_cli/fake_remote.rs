type RecordedRequestLog = Arc<Mutex<Vec<RecordedRequest>>>;
type SpawnedFakeRemote<S, H = thread::JoinHandle<()>> =
    (String, RecordedRequestLog, Arc<Mutex<S>>, H);

fn spawn_fake_remote() -> SpawnedFakeRemote<FakeRemoteState> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::from_listener(listener, None).unwrap();
    let log = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let log_clone = Arc::clone(&log);
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    let state_clone = Arc::clone(&state);
    let handle = thread::spawn(move || {
        let mut received_request = false;
        loop {
            let idle_timeout = if received_request { 2 } else { 10 };
            let Ok(Some(mut request)) =
                server.recv_timeout(Duration::from_secs(idle_timeout))
            else {
                break;
            };
            received_request = true;
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
    (format!("http://{}", addr), log, state, handle)
}

fn spawn_queue_summary_fallback_remote() -> (
    String,
    Arc<Mutex<Vec<RecordedRequest>>>,
    thread::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::from_listener(listener, None).unwrap();
    let log = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let log_clone = Arc::clone(&log);
    let handle = thread::spawn(move || {
        let mut received_request = false;
        loop {
            let idle_timeout = if received_request { 2 } else { 10 };
            let Ok(Some(mut request)) =
                server.recv_timeout(Duration::from_secs(idle_timeout))
            else {
                break;
            };
            received_request = true;
            let mut body_bytes = Vec::new();
            request.as_reader().read_to_end(&mut body_bytes).unwrap();
            let body = String::from_utf8_lossy(&body_bytes).to_string();
        let url = request.url().to_string();
        let method = request.method().as_str().to_string();
        log_clone.lock().unwrap().push(RecordedRequest {
            method: method.clone(),
            url: url.clone(),
            body,
        });
        let payload = if method == "GET"
            && url.starts_with(
                "/v1/native/repository-authorities/7/read/queue-summary?",
            )
            && url.contains("status=active")
        {
            request
                .respond(
                    Response::from_string(r#"{"error":"missing queue bundle"}"#)
                        .with_status_code(404),
                )
                .unwrap();
            continue;
        } else if method == "GET"
            && url.starts_with(
                "/v1/native/repository-authorities/7/read/task-queue?",
            )
            && url.contains("status=active")
        {
            json!({
                "count": 2,
                "summary": {
                    "attention_required": 1,
                    "ready_to_land": 1,
                    "ready_to_complete": 0
                },
                "items": [{
                    "task_id": "RT-REMOTE",
                    "focus_change": {
                        "change_id": "RC-REMOTE-STALE",
                        "reason": "Remote task queue focus reason."
                    }
                }]
            })
        } else if method == "GET"
            && url == "/v1/native/repository-authorities/7/read/reviewer-inbox"
        {
            json!({
                "count": 1,
                "items": [
                    {
                        "change_id": "RC-REMOTE-READY",
                        "review_state": {"blocking": 0},
                        "freshness": {"base_is_fresh": true},
                        "policy_state": {"decision": "pass"},
                        "attestation": {"completeness": "present"}
                    },
                    {
                        "change_id": "RC-REMOTE-STALE",
                        "review_state": {"blocking": 0},
                        "freshness": {"base_is_fresh": false},
                        "policy_state": {"decision": "pending"},
                        "attestation": {"completeness": "present"}
                    }
                ]
            })
        } else {
            json!({"error": format!("unexpected {method} {url}")})
        };
        let status = if payload.get("error").is_some() {
            404
        } else {
            200
        };
        request
            .respond(
                Response::from_string(encode_json(&payload))
                    .with_status_code(status),
            )
            .unwrap();
        }
    });
    (format!("http://{}", addr), log, handle)
}

fn spawn_remote_import_server(
    line_name: &str,
    head_snapshot_id: &str,
    zstd_fixture: ZstdRemoteImportFixture,
) -> (
    String,
    Arc<Mutex<Vec<RecordedRequest>>>,
    thread::JoinHandle<()>,
) {
    let server = Server::http("127.0.0.1:0").unwrap();
    let addr = server.server_addr();
    let log: Arc<Mutex<Vec<RecordedRequest>>> = Arc::new(Mutex::new(Vec::new()));
    let line_name = line_name.to_string();
    let encoded_line_name = line_name.replace('/', "%2F");
    let line_url = format!("/v1/native/repository-authorities/7/lines/{encoded_line_name}");
    let head_snapshot_id = head_snapshot_id.to_string();
    let zstd_fixture = Arc::new(zstd_fixture);
    let log_clone = log.clone();
    let handle = thread::spawn(move || {
        let mut received_request = false;
        loop {
            let idle_timeout = if received_request { 2 } else { 10 };
            let Ok(Some(mut request)) =
                server.recv_timeout(Duration::from_secs(idle_timeout))
            else {
                break;
            };
            received_request = true;
            let mut body_bytes = Vec::new();
            request.as_reader().read_to_end(&mut body_bytes).unwrap();
            let body = String::from_utf8_lossy(&body_bytes).to_string();
            let method = request.method().as_str().to_string();
            let url = request.url().to_string();
            log_clone.lock().unwrap().push(RecordedRequest {
                method: method.clone(),
                url: url.clone(),
                body: body.clone(),
            });
            const ZSTD_IMPORT_PREFIX: &str =
                "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/";
            if method == "GET" {
                if let Some(snapshot_id) = url.strip_prefix(&format!(
                    "{ZSTD_IMPORT_PREFIX}import-manifests/"
                )) {
                    let response = match zstd_fixture.manifests.get(snapshot_id) {
                        Some(manifest) => Response::from_string(
                            ZstdImportManifestJson::stateless()
                                .encode_string(manifest)
                                .unwrap(),
                        )
                        .with_header(
                            tiny_http::Header::from_bytes(b"Content-Type", b"application/json")
                                .unwrap(),
                        ),
                        None => Response::from_string(format!("unknown snapshot {snapshot_id}"))
                            .with_status_code(404),
                    };
                    request.respond(response).unwrap();
                    continue;
                }
                if let Some(pack_id) =
                    url.strip_prefix(&format!("{ZSTD_IMPORT_PREFIX}object-packs/"))
                {
                    let response = match zstd_fixture.object_packs.get(pack_id) {
                        Some(bytes) => Response::from_data(bytes.clone()).with_header(
                            tiny_http::Header::from_bytes(
                                b"Content-Type",
                                b"application/vnd.ait.zstd-object-pack",
                            )
                            .unwrap(),
                        ),
                        None => Response::from_string(format!("unknown object pack {pack_id}"))
                            .with_status_code(404),
                    };
                    request.respond(response).unwrap();
                    continue;
                }
                if let Some(pack_id) =
                    url.strip_prefix(&format!("{ZSTD_IMPORT_PREFIX}tree-packs/"))
                {
                    let response = match zstd_fixture.tree_packs.get(pack_id) {
                        Some(bytes) => Response::from_data(bytes.clone()).with_header(
                            tiny_http::Header::from_bytes(
                                b"Content-Type",
                                b"application/vnd.ait.zstd-tree-pack",
                            )
                            .unwrap(),
                        ),
                        None => Response::from_string(format!("unknown tree pack {pack_id}"))
                            .with_status_code(404),
                    };
                    request.respond(response).unwrap();
                    continue;
                }
            }
            let payload = match (method.as_str(), url.as_str()) {
                ("GET", "/v1/handshake") => handshake_payload(),
                ("GET", "/v1/native/repository-authorities/7") => {
                    repository_payload("fixture-ait")
                }
                ("GET", candidate) if candidate == line_url.as_str() => json!({
                    "repo_name": "fixture-ait",
                    "line_name": line_name,
                    "head_snapshot_id": head_snapshot_id,
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
                    Response::from_string(encode_json(&payload))
                        .with_status_code(status),
                )
                .unwrap();
        }
    });
    (format!("http://{}", addr), log, handle)
}

fn spawn_publish_recovery_remote() -> SpawnedFakeRemote<RecoveryRemoteState> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::from_listener(listener, None).unwrap();
    let log = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let log_clone = Arc::clone(&log);
    let state = Arc::new(Mutex::new(RecoveryRemoteState::default()));
    let state_clone = Arc::clone(&state);
    let handle = thread::spawn(move || loop {
        let idle_timeout = if log_clone.lock().unwrap().is_empty() {
            10
        } else {
            2
        };
        let Ok(Some(mut request)) = server.recv_timeout(Duration::from_secs(idle_timeout)) else {
            break;
        };
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
        let response = response_for_publish_recovery(&method, &url, &body, &state_clone);
        request.respond(response).unwrap();
    });
    (format!("http://{}", addr), log, state, handle)
}

fn spawn_bounded_snapshot_remote() -> SpawnedFakeRemote<BoundedSnapshotRemoteState> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::from_listener(listener, None).unwrap();
    let log = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let log_clone = Arc::clone(&log);
    let state = Arc::new(Mutex::new(BoundedSnapshotRemoteState::default()));
    let state_clone = Arc::clone(&state);
    let handle = thread::spawn(move || loop {
        let idle_timeout = if log_clone.lock().unwrap().is_empty() {
            10
        } else {
            2
        };
        let Ok(Some(mut request)) = server.recv_timeout(Duration::from_secs(idle_timeout)) else {
            break;
        };
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
        let response = response_for_bounded_snapshot_sync(&method, &url, &body, &state_clone);
        request.respond(response).unwrap();
    });
    (format!("http://{}", addr), log, state, handle)
}

fn spawn_closeout_recovery_remote(
) -> SpawnedFakeRemote<CloseoutRecoveryRemoteState, CloseoutRecoveryServerHandle> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = Server::from_listener(listener, None).unwrap();
    let log = Arc::new(Mutex::new(Vec::<RecordedRequest>::new()));
    let log_clone = Arc::clone(&log);
    let state = Arc::new(Mutex::new(CloseoutRecoveryRemoteState::default()));
    let state_clone = Arc::clone(&state);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);
    let handle = thread::spawn(move || loop {
        if stop_clone.load(Ordering::Acquire) {
            break;
        }
        let Ok(request) = server.recv_timeout(Duration::from_millis(20)) else {
            break;
        };
        let Some(mut request) = request else {
            continue;
        };
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
        if method == "POST" && url == "/v1/native/repository-authorities/7/task-land" {
            let request_state = Arc::clone(&state_clone);
            thread::spawn(move || {
                let response =
                    response_for_closeout_recovery(&method, &url, &body, &request_state);
                let _ = request.respond(response);
            });
            continue;
        }
        let response = response_for_closeout_recovery(&method, &url, &body, &state_clone);
        let _ = request.respond(response);
    });
    (
        format!("http://{}", addr),
        log,
        state,
        CloseoutRecoveryServerHandle {
            stop,
            handle: Some(handle),
        },
    )
}

fn repository_payload(repo_name: &str) -> JsonValue {
    json!({
        "contract": "ait.server.repository-authority.v1",
        "repository": {
            "repository_index": FIXTURE_REPOSITORY_INDEX,
            "repository_name": repo_name,
            "namespace": "",
            "policy_flags": 0,
            "tombstoned": false
        },
        "pack_storage": {
            "contract": "ait.repository.pack_storage.v1",
            "zstd_only_verified": true,
            "object_pack_format": "ait-pack-v3-zstd-chunked",
            "tree_pack_format": "ait-tree-pack-v2-zstd-chunked",
            "object_pack_count": 0,
            "tree_pack_count": 0,
            "zstd_object_pack_count": 0,
            "zstd_tree_pack_count": 0,
            "requires_zstd_remote_sync": true,
            "validation": {"state": "valid", "error_count": 0}
        },
        "ci_capabilities": {
            "native_runner": {
                "contract": "ait.runner.native-job.v3",
                "repository_entrypoint": "ci/run"
            },
            "remote_sync_capabilities": {
                "zstd_pack_bulk": true,
                "zstd_pack_bulk_download": true
            }
        }
    })
}

fn handshake_payload() -> JsonValue {
    json!({
        "ready": true,
        "authority_backend": "binary_v0",
        "contract_version": "ait.agent_server_protocol.v2",
        "supported_async_job_types": ["patchset.ci"],
        "ci_capabilities": {
            "patchset_run_ci_route": true,
            "repository_pack_storage": {
                "contract": "ait.repository.pack_storage.v1",
                "supported": true
            },
            "native_runner": {
                "contract": "ait.runner.native-job.v3",
                "repository_entrypoint": "ci/run"
            },
            "remote_sync_capabilities": {
                "zstd_pack_bulk": true,
                "zstd_pack_bulk_download": true
            }
        },
        "ci_readiness": {
            "runtime_generation": "current"
        }
    })
}

fn zstd_bulk_ids(parsed: &JsonValue, field: &str, id_field: &str) -> Vec<JsonValue> {
    parsed
        .get(field)
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get(id_field).and_then(JsonValue::as_str))
        .map(|id| JsonValue::String(id.to_string()))
        .collect()
}

fn maybe_zstd_bulk_response(
    method: &str,
    url: &str,
    body: &str,
    present_snapshot_id: Option<String>,
    state: Option<&Arc<Mutex<FakeRemoteState>>>,
) -> Option<Response<std::io::Cursor<Vec<u8>>>> {
    const PREFIX: &str = "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/";
    let suffix = url.strip_prefix(PREFIX)?;
    match (method, suffix) {
        ("POST", "plan") => {
            let parsed = parse_json(body);
            let snapshot_ids = parsed
                .get("snapshot_ids")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
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
            Some(json_response(
                200,
                &json!({
                    "repo_name": "fixture-ait",
                    "present_snapshot_ids": present_snapshot_ids,
                    "missing_snapshot_ids": missing_snapshot_ids,
                    "present_object_pack_ids": [],
                    "missing_object_pack_ids": object_pack_ids,
                    "present_tree_pack_ids": [],
                    "missing_tree_pack_ids": tree_pack_ids
                }),
            ))
        }
        ("POST", "commit") => {
            let parsed = parse_json(body);
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
                    if let (Some(state), Some(head_snapshot_id)) = (state, head_snapshot_id.clone())
                    {
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
            Some(json_response(
                200,
                &json!({
                    "repo_name": "fixture-ait",
                    "committed_snapshot_ids": committed_snapshot_ids,
                    "committed_object_pack_ids": committed_object_pack_ids,
                    "committed_tree_pack_ids": committed_tree_pack_ids,
                    "upserted_snapshots": committed_snapshot_ids.len(),
                    "remote_line": remote_line,
                    "line_update": JsonValue::Null
                }),
            ))
        }
        _ if method == "PUT" && suffix.starts_with("object-packs/") => {
            let pack_id = suffix.trim_start_matches("object-packs/");
            Some(json_response(
                200,
                &json!({
                    "repo_name": "fixture-ait",
                    "pack_id": pack_id,
                    "stored": true,
                    "pack_bytes": body.len(),
                    "raw_binary_upload": true
                }),
            ))
        }
        _ if method == "PUT" && suffix.starts_with("tree-packs/") => {
            let pack_id = suffix.trim_start_matches("tree-packs/");
            Some(json_response(
                200,
                &json!({
                    "repo_name": "fixture-ait",
                    "pack_id": pack_id,
                    "stored": true,
                    "pack_bytes": body.len(),
                    "raw_binary_upload": true
                }),
            ))
        }
        _ if method == "GET"
            && (suffix.starts_with("object-packs/") || suffix.starts_with("tree-packs/")) =>
        {
            Some(json_response(
                404,
                &json!({"detail": "repository pack is absent"}),
            ))
        }
        _ => None,
    }
}

fn fake_landing_summary(guard: &FakeRemoteState, selected_patchset_id: &str) -> JsonValue {
    if guard.base_stale_converged_submitted {
        if guard.omit_landing_summary_after_base_stale_converged {
            return JsonValue::Null;
        }
        let target_line_head = guard
            .selected_patchset_revision_snapshot_id
            .clone()
            .unwrap_or_else(|| FIXTURE_REVISION_SNAPSHOT_ID.to_string());
        return json!({
            "submission_id":"LAND-1",
            "patchset_id": selected_patchset_id,
            "status":"blocked",
            "result":{
                "blocker_class":"BASE_STALE",
                "expected_base_snapshot_id": guard
                    .selected_patchset_base_snapshot_id
                    .clone()
                    .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string()),
                "target_line_head": target_line_head
            }
        });
    }
    if guard.land_submitted {
        return json!({
            "submission_id":"LAND-1",
            "patchset_id": selected_patchset_id,
            "status":"succeeded",
            "result":{"line_action":"moved"}
        });
    }
    JsonValue::Null
}

fn fake_atomic_task_land_response(
    request: &JsonValue,
    replayed: bool,
    landed_snapshot_id: &str,
    patchset_id: &str,
    submission_id: &str,
) -> JsonValue {
    json!({
        "contract": "task-land-atomic/v1",
        "repo_name": "fixture-ait",
        "repo_id": "repo-fixture-ait",
        "idempotency_key": request.get("idempotency_key").cloned().unwrap_or(JsonValue::Null),
        "replayed": replayed,
        "status": "succeeded",
        "task_id": "RT-1",
        "task_status": "completed",
        "change_id": "RC-1",
        "change_ref": "RT-1/C-01",
        "change_status": "landed",
        "patchset_id": patchset_id,
        "target_line": request
            .get("target_line")
            .cloned()
            .unwrap_or(JsonValue::String("main".to_string())),
        "landed_snapshot_id": landed_snapshot_id,
        "task": {
            "task_id": "RT-1",
            "status": "completed"
        },
        "change": {
            "task_id": "RT-1",
            "change_id": "RC-1",
            "change_ref": "RT-1/C-01",
            "status": "landed",
            "selected_patchset_id": patchset_id,
            "landed_snapshot_id": landed_snapshot_id
        },
        "patchset": {
            "patchset_id": patchset_id,
            "revision_snapshot_id": landed_snapshot_id
        },
        "land": {
            "submission_id": submission_id,
            "status": "succeeded",
            "target_line": request
                .get("target_line")
                .cloned()
                .unwrap_or(JsonValue::String("main".to_string())),
            "landed_snapshot_id": landed_snapshot_id,
            "result": {
                "target_line": request
                    .get("target_line")
                    .cloned()
                    .unwrap_or(JsonValue::String("main".to_string())),
                "line_action": if replayed { "already_landed" } else { "moved" },
                "landed_snapshot_id": landed_snapshot_id
            }
        }
    })
}

fn fake_atomic_task_start_response(
    body: &str,
    state: &Arc<Mutex<FakeRemoteState>>,
) -> JsonValue {
    let request = parse_json(body);
    let plan_operation = request
        .get("plan")
        .and_then(JsonValue::as_object)
        .expect("atomic task-start plan operation");
    let action = plan_operation
        .get("action")
        .and_then(JsonValue::as_str)
        .expect("atomic task-start action");
    if action == "existing" {
        let mut replay = state
            .lock()
            .unwrap()
            .atomic_task_start
            .clone()
            .expect("atomic task-start replay state");
        replay["replayed"] = JsonValue::Bool(true);
        replay["idempotency_key"] = request["idempotency_key"].clone();
        return replay;
    }

    let plan_id = plan_operation
        .get("plan_id")
        .and_then(JsonValue::as_str)
        .unwrap_or("PR-42");
    let plan_revision_id = if action == "revise" {
        "plan-revision:43"
    } else {
        "plan-revision:42"
    };
    let plan_payload = plan_operation
        .get("payload")
        .and_then(JsonValue::as_object)
        .expect("atomic task-start Plan payload");
    let plan = json!({
        "repo_name": "fixture-ait",
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "head_revision_id": plan_revision_id,
        "title": plan_payload.get("title").cloned().unwrap_or(JsonValue::String("Atomic Plan".to_string())),
        "status": plan_payload.get("status").cloned().unwrap_or(JsonValue::String("draft".to_string())),
        "artifact_path": plan_payload.get("artifact_path").cloned().unwrap_or(JsonValue::Null),
        "artifact_selector": plan_payload.get("artifact_selector").cloned().unwrap_or(JsonValue::Null),
        "artifact_heading": plan_payload.get("artifact_heading").cloned().unwrap_or(JsonValue::Null),
        "items": plan_payload.get("items").cloned().unwrap_or(JsonValue::Array(Vec::new())),
    });
    let task_request = request
        .get("task")
        .and_then(JsonValue::as_object)
        .expect("atomic task-start Task payload");
    let task = json!({
        "repo_name": "fixture-ait",
        "task_id": "RT-ATOMIC",
        "published_task_id": "RT-ATOMIC",
        "title": task_request.get("title").cloned().unwrap_or(JsonValue::Null),
        "intent": task_request.get("intent").cloned().unwrap_or(JsonValue::Null),
        "plan_id": plan_id,
        "origin_plan_revision_id": plan_revision_id,
        "plan_item_ref": request.get("plan_item_ref").cloned().unwrap_or(JsonValue::Null),
        "status": "active",
    });
    let change_request = request
        .get("change")
        .and_then(JsonValue::as_object)
        .expect("atomic task-start Change payload");
    let remote_head_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
    let change = json!({
        "repo_name": "fixture-ait",
        "task_id": "RT-ATOMIC",
        "change_id": "C-01",
        "change_ref": "RT-ATOMIC/C-01",
        "title": change_request.get("title").cloned().unwrap_or(JsonValue::Null),
        "base_line": change_request.get("base_line").cloned().unwrap_or(JsonValue::Null),
        "fork_snapshot_id": remote_head_snapshot_id,
        "forked_from_line": change_request.get("base_line").cloned().unwrap_or(JsonValue::Null),
        "status": "draft",
        "current_patchset_number": 0,
    });
    let response = json!({
        "contract": "task-start-atomic/v1",
        "repo_name": "fixture-ait",
        "repo_id": "repo-fixture-ait",
        "idempotency_key": request.get("idempotency_key").cloned().unwrap_or(JsonValue::Null),
        "replayed": false,
        "plan_action": if action == "revise" { "revised" } else { "created" },
        "plan_id": plan_id,
        "plan_revision_id": plan_revision_id,
        "plan_item_ref": request.get("plan_item_ref").cloned().unwrap_or(JsonValue::Null),
        "plan": plan,
        "task_id": "RT-ATOMIC",
        "task": task,
        "change": change,
    });
    let mut guard = state.lock().unwrap();
    guard.atomic_plan = Some(response["plan"].clone());
    guard.atomic_task_start = Some(response.clone());
    response
}

fn maybe_zstd_download_response(
    method: &str,
    url: &str,
    state: &Arc<Mutex<FakeRemoteState>>,
) -> Option<Response<std::io::Cursor<Vec<u8>>>> {
    if method != "GET" {
        return None;
    }
    const ZSTD_IMPORT_PREFIX: &str =
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/";
    let guard = state.lock().unwrap();
    let fixture = guard.zstd_import_fixture.as_ref()?;
    if let Some(snapshot_id) = url.strip_prefix(&format!("{ZSTD_IMPORT_PREFIX}import-manifests/")) {
        return Some(match fixture.manifests.get(snapshot_id) {
            Some(manifest) => Response::from_string(
                ZstdImportManifestJson::stateless()
                    .encode_string(manifest)
                    .unwrap(),
            )
            .with_header(
                tiny_http::Header::from_bytes(b"Content-Type", b"application/json").unwrap(),
            ),
            None => Response::from_string(format!("unknown snapshot {snapshot_id}"))
                .with_status_code(404),
        });
    }
    if let Some(pack_id) = url.strip_prefix(&format!("{ZSTD_IMPORT_PREFIX}object-packs/")) {
        return Some(match fixture.object_packs.get(pack_id) {
            Some(bytes) => Response::from_data(bytes.clone()).with_header(
                tiny_http::Header::from_bytes(
                    b"Content-Type",
                    b"application/vnd.ait.zstd-object-pack",
                )
                .unwrap(),
            ),
            None => Response::from_string(format!("unknown object pack {pack_id}"))
                .with_status_code(404),
        });
    }
    if let Some(pack_id) = url.strip_prefix(&format!("{ZSTD_IMPORT_PREFIX}tree-packs/")) {
        return Some(match fixture.tree_packs.get(pack_id) {
            Some(bytes) => Response::from_data(bytes.clone()).with_header(
                tiny_http::Header::from_bytes(
                    b"Content-Type",
                    b"application/vnd.ait.zstd-tree-pack",
                )
                .unwrap(),
            ),
            None => Response::from_string(format!("unknown tree pack {pack_id}"))
                .with_status_code(404),
        });
    }
    None
}

fn response_for(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<FakeRemoteState>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    if let Some(response) = maybe_zstd_download_response(method, url, state) {
        return response;
    }
    let present_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
    if let Some(response) = maybe_zstd_bulk_response(method, url, body, present_snapshot_id, Some(state)) {
        return response;
    }
    if method == "GET"
        && (url == "/v1/native/repository-authorities/7/sprints"
            || url.starts_with("/v1/native/repository-authorities/7/sprints?"))
    {
        let plans = state
            .lock()
            .unwrap()
            .atomic_plan
            .clone()
            .into_iter()
            .collect::<Vec<_>>();
        return json_response(200, &JsonValue::Array(plans));
    }
    if method == "GET"
        && url.starts_with("/v1/native/repository-authorities/7/sprints/")
    {
        let suffix = url
            .trim_start_matches("/v1/native/repository-authorities/7/sprints/")
            .to_string();
        let plan_id = suffix.split('/').next().unwrap_or_default();
        let plan = state.lock().unwrap().atomic_plan.clone();
        return match plan {
            Some(plan)
                if plan.get("plan_id").and_then(JsonValue::as_str) == Some(plan_id) =>
            {
                if suffix.ends_with("/revisions") {
                    json_response(200, &JsonValue::Array(vec![plan]))
                } else {
                    json_response(200, &plan)
                }
            }
            _ => json_response(
                400,
                &json!({"detail": "record index out of bounds for file 'plan.bin'"}),
            ),
        };
    }
    if method == "POST"
        && url == "/v1/native/repository-authorities/7/task-start"
        && state.lock().unwrap().fail_atomic_task_start
    {
        return json_response(
            409,
            &json!({"detail": "injected atomic task-start transaction failure"}),
        );
    }
    let payload = match (method, url) {
        ("GET", "/healthz") | ("GET", "/v1/handshake") => handshake_payload(),
        ("GET", "/v1/native/repository-authorities/7") => {
            repository_payload("fixture-ait")
        }
        _ if method == "GET"
            && url.starts_with("/v1/native/repository-authorities/7/worker-jobs") =>
        {
            json!({
                "contract": "ait.server.worker-job.v1",
                "repository_index": 7,
                "jobs": [{
                    "repository_index": 7,
                    "worker_job_index": 77,
                    "job_kind": 7,
                    "job_type": "patchset.ci",
                    "state_kind": 3,
                    "state": "succeeded",
                    "diagnostic_status": "succeeded"
                }],
                "count": 1
            })
        }
        ("POST", "/v1/native/repository-authorities/7/sprints") => {
            let request = parse_json(body);
            let plan = json!({
                "repo_name": "fixture-ait",
                "plan_id": "PR-42",
                "plan_revision_id": "plan-revision:42",
                "head_revision_id": "plan-revision:42",
                "title": request.get("title").cloned().unwrap_or(JsonValue::String("Plan".to_string())),
                "status": request.get("status").cloned().unwrap_or(JsonValue::String("draft".to_string())),
                "summary": request.get("summary").cloned().unwrap_or(JsonValue::Null),
                "artifact_path": request.get("artifact_path").cloned().unwrap_or(JsonValue::Null),
                "artifact_selector": request.get("artifact_selector").cloned().unwrap_or(JsonValue::Null),
                "artifact_heading": request.get("artifact_heading").cloned().unwrap_or(JsonValue::Null),
                "items": request.get("items").cloned().unwrap_or(JsonValue::Array(Vec::new())),
                "source_kind": request.get("source_kind").cloned().unwrap_or(JsonValue::String("manual_edit".to_string())),
            });
            state.lock().unwrap().atomic_plan = Some(plan.clone());
            plan
        }
        ("POST", "/v1/native/repository-authorities/7/task-start") => {
            fake_atomic_task_start_response(body, state)
        }
        ("POST", "/v1/native/repository-authorities/7/task-land") => {
            let request = parse_json(body);
            assert_eq!(
                request.get("contract").and_then(JsonValue::as_str),
                Some("task-land-atomic/v1")
            );
            assert_eq!(
                request.get("task_or_change_ref").and_then(JsonValue::as_str),
                Some("RT-1")
            );
            let mut guard = state.lock().unwrap();
            if guard.force_no_selected_patchset && guard.selected_patchset_id.is_none() {
                return json_response(
                    409,
                    &json!({
                        "detail": "Atomic Task Land requires an existing selected remote patchset; Task RT-1 currently has no selected patchset. Task Land does not publish or synchronize content and does not start or wait for CI."
                    }),
                );
            }
            let replayed = guard.land_submitted && guard.task_completed;
            let patchset_id = guard
                .selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            let selected_revision_snapshot_id = guard
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_FINISHED_SNAPSHOT_ID.to_string());
            let landed_snapshot_id = match (
                guard.remote_head_snapshot_id.as_deref(),
                guard.selected_patchset_base_snapshot_id.as_deref(),
            ) {
                (Some(head), Some(base))
                    if head != base && head != selected_revision_snapshot_id =>
                {
                    head.to_string()
                }
                _ => selected_revision_snapshot_id,
            };
            guard.land_submitted = true;
            guard.task_completed = true;
            guard.remote_head_snapshot_id = Some(landed_snapshot_id.clone());
            fake_atomic_task_land_response(
                &request,
                replayed,
                &landed_snapshot_id,
                &patchset_id,
                "LAND-1",
            )
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1") => {
            let guard = state.lock().unwrap();
            let land_submitted = guard.land_submitted;
            let selected_patchset_id =
                if guard.force_no_selected_patchset && guard.selected_patchset_id.is_none() {
                    None
                } else {
                    Some(
                        guard
                            .selected_patchset_id
                            .clone()
                            .unwrap_or_else(|| "RP-1".to_string()),
                    )
                };
            let has_selected_patchset = selected_patchset_id.is_some();
            let selected_patchset_id_for_summary = selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            let landing_summary = fake_landing_summary(&guard, &selected_patchset_id_for_summary);
            json!({
                "change_id":"RC-1",
                "task_id":"RT-1",
                "title":"Published review change",
                "base_line":"main",
                "fork_snapshot_id": FIXTURE_BASE_SNAPSHOT_ID,
                "forked_from_line":"main",
                "status": if land_submitted { "landed" } else { "active" },
                "publication_state":"published",
                "published_remote_name":"origin",
                "published_change_id":"RC-1",
                "selected_patchset_id": selected_patchset_id,
                "current_patchset_number": if has_selected_patchset { 1 } else { 0 },
                "landing_summary": landing_summary
            })
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RT-1%2FC-01") => {
            let guard = state.lock().unwrap();
            json!({
                "change_id":"C-01",
                "change_ref":"RT-1/C-01",
                "task_id":"RT-1",
                "title":"Task-scoped published review change",
                "base_line":"main",
                "fork_snapshot_id": FIXTURE_BASE_SNAPSHOT_ID,
                "forked_from_line":"main",
                "status":"active",
                "publication_state":"published",
                "published_remote_name":"origin",
                "published_change_id":"C-01",
                "selected_patchset_id":"RT-1/C-01/P-01",
                "current_patchset_number":1,
                "landing_summary": fake_landing_summary(&guard, "RT-1/C-01/P-01")
            })
        }
        ("GET", "/v1/native/repository-authorities/7/read/changes/RC-1") => {
            let guard = state.lock().unwrap();
            let selected_patchset_id =
                if guard.force_no_selected_patchset && guard.selected_patchset_id.is_none() {
                    None
                } else {
                    Some(
                        guard
                            .selected_patchset_id
                            .clone()
                            .unwrap_or_else(|| "RP-1".to_string()),
                    )
                };
            let selected_patchset_id_for_summary = selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            let landing_summary = fake_landing_summary(&guard, &selected_patchset_id_for_summary);
            json!({
                "change_id":"RC-1",
                "task_id":"RT-1",
                "selected_patchset_id": selected_patchset_id,
                "landing_summary": landing_summary
            })
        }
        ("GET", "/v1/native/repository-authorities/7/changes") => {
            let guard = state.lock().unwrap();
            let has_selected_patchset =
                !(guard.force_no_selected_patchset && guard.selected_patchset_id.is_none());
            json!([{
                "change_id":"RC-1",
                "title":"Published review change",
                "base_line":"main",
                "current_patchset_number": if has_selected_patchset { 1 } else { 0 },
                "status":"active"
            }])
        }
        ("POST", "/v1/native/repository-authorities/7/changes") => {
            let parsed: JsonValue = parse_json(body);
            let task_id = parsed
                .get("task_id")
                .and_then(JsonValue::as_str)
                .unwrap_or("RT-REMOTE");
            let contextual_change_id = format!("{task_id}/C-01");
            let change_id = parsed
                .get("change_id")
                .and_then(JsonValue::as_str)
                .unwrap_or(&contextual_change_id);
            json!({
                "change_id": change_id,
                "task_id": parsed.get("task_id").cloned().unwrap_or(JsonValue::Null),
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
            let guard = state.lock().unwrap();
            json!([
                {
                    "repo_name":"fixture-ait",
                    "line_name":"main",
                    "head_snapshot_id": guard.remote_head_snapshot_id
                },
                {
                    "repo_name":"fixture-ait",
                    "line_name":"feature/rt-1",
                    "head_snapshot_id": guard.selected_patchset_revision_snapshot_id
                }
            ])
        }
        ("GET", "/v1/native/repository-authorities/7/lines/feature%2Frt-1") => {
            let revision_snapshot_id = state
                .lock()
                .unwrap()
                .selected_patchset_revision_snapshot_id
                .clone();
            json!({"repo_name":"fixture-ait","line_name":"feature/rt-1","head_snapshot_id": revision_snapshot_id})
        }
        ("PUT", "/v1/native/repository-authorities/7/lines/main") => {
            let parsed: JsonValue = parse_json(body);
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
        ("PUT", "/v1/native/repository-authorities/7/lines/feature%2Frt-1") => {
            let parsed: JsonValue = parse_json(body);
            json!({
                "repo_name":"fixture-ait",
                "line_name":"feature/rt-1",
                "head_snapshot_id": parsed.get("head_snapshot_id").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("POST", "/v1/native/repository-authorities/7/snapshots:exists") => {
            let parsed: JsonValue = parse_json(body);
            let snapshot_ids = parsed
                .get("snapshot_ids")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            let present_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
            let mut present = Vec::new();
            let mut missing = Vec::new();
            for snapshot_id in snapshot_ids {
                if snapshot_id.as_str() == present_snapshot_id.as_deref() {
                    present.push(snapshot_id);
                } else {
                    missing.push(snapshot_id);
                }
            }
            json!({
                "repo_name":"fixture-ait",
                "checked_snapshots": present.len() + missing.len(),
                "present": present,
                "missing": missing
            })
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/patchsets") => {
            let parsed: JsonValue = parse_json(body);
            {
                let mut guard = state.lock().unwrap();
                guard.selected_patchset_id = Some("RP-2".to_string());
                guard.selected_patchset_base_snapshot_id = parsed
                    .get("base_snapshot_id")
                    .and_then(JsonValue::as_str)
                    .map(ToString::to_string);
                guard.selected_patchset_revision_snapshot_id = parsed
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
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1/patchsets") => {
            let guard = state.lock().unwrap();
            if guard.force_no_selected_patchset && guard.selected_patchset_id.is_none() {
                return json_response(200, &json!([]));
            }
            let patchset_id = guard
                .selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            let base_snapshot_id = guard
                .selected_patchset_base_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            let revision_snapshot_id = guard
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_REVISION_SNAPSHOT_ID.to_string());
            json!([{
                "patchset_id": patchset_id,
                "change_id":"RC-1",
                "patchset_number":1,
                "base_snapshot_id": base_snapshot_id,
                "revision_snapshot_id": revision_snapshot_id,
                "publish_state":"published",
                "evaluation_state":"pending"
            }])
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RT-1%2FC-01/patchsets") => {
            let guard = state.lock().unwrap();
            let base_snapshot_id = guard
                .selected_patchset_base_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            let revision_snapshot_id = guard
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_REVISION_SNAPSHOT_ID.to_string());
            json!([{
                "patchset_id":"RT-1/C-01/P-01",
                "change_id":"C-01",
                "change_ref":"RT-1/C-01",
                "patchset_number":1,
                "base_snapshot_id":base_snapshot_id,
                "revision_snapshot_id":revision_snapshot_id,
                "publish_state":"published",
                "evaluation_state":"pending"
            }])
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RT-1%2FC-01%2FP-01") => {
            let guard = state.lock().unwrap();
            let base_snapshot_id = guard
                .selected_patchset_base_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            let revision_snapshot_id = guard
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_REVISION_SNAPSHOT_ID.to_string());
            json!({
                "patchset_id":"RT-1/C-01/P-01",
                "change_id":"C-01",
                "change_ref":"RT-1/C-01",
                "patchset_number":1,
                "base_snapshot_id":base_snapshot_id,
                "revision_snapshot_id":revision_snapshot_id,
                "publish_state":"published",
                "evaluation_state":"pending",
                "summary":"Task-scoped native Rust patchset"
            })
        }
        _ if method == "GET"
            && (url == "/v1/native/repository-authorities/7/patchsets/RP-1"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-1?change_ref=RC-1"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-2"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-2?change_ref=RC-1"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-RESET"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-RESET?change_ref=RC-1") =>
        {
            let guard = state.lock().unwrap();
            let patchset_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/patchsets/")
                .split('?')
                .next()
                .unwrap_or("RP-1");
            let base_snapshot_id = guard
                .selected_patchset_base_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            let revision_snapshot_id = guard
                .selected_patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_REVISION_SNAPSHOT_ID.to_string());
            json!({
                "patchset_id": patchset_id,
                "change_id":"RC-1",
                "patchset_number":1,
                "base_snapshot_id": base_snapshot_id,
                "revision_snapshot_id": revision_snapshot_id,
                "publish_state":"published",
                "evaluation_state":"pending",
                "summary":"Native Rust patchset"
            })
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/1") => {
            json!({"patchset_id":"RP-1","change_id":"RC-1"})
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/1?change_ref=RC-1") => {
            json!({
                "patchset_id":"RP-1",
                "change_id":"RC-1",
                "patchset_number":1,
                "base_snapshot_id": FIXTURE_BASE_SNAPSHOT_ID,
                "revision_snapshot_id":FIXTURE_REVISION_SNAPSHOT_ID,
                "publish_state":"published",
                "evaluation_state":"pending",
                "summary":"Native Rust patchset"
            })
        }
        _ if method == "PUT"
            && url.starts_with("/v1/native/repository-authorities/7/snapshots/") =>
        {
            let snapshot_id = url.rsplit('/').next().unwrap().trim_end_matches(":pack");
            json!({"snapshot_id": snapshot_id, "repo_name":"fixture-ait"})
        }
        _ if method == "GET"
            && (url.starts_with("/v1/native/repository-authorities/7/read/patchsets/RP-1/ci-status")
                || url.starts_with("/v1/native/repository-authorities/7/read/patchsets/RP-2/ci-status")) =>
        {
            let patchset_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/read/patchsets/")
                .split('/')
                .next()
                .unwrap_or("RP-1");
            json!({
                "patchset_id": patchset_id,
                "change_id":"RC-1",
                "tests_status":"pass",
                "latest_job":{"job_id":"JOB-1","state":"succeeded"}
            })
        }
        _ if method == "GET" && url.starts_with("/v1/native/repository-authorities/7/read/patchsets/RP-RESET/ci-status") => {
            json!({
                "patchset_id": "RP-RESET",
                "change_id": "RC-1",
                "tests_status": "pending",
                "recommended_action": "rebase_patchset_to_latest_main",
                "status_notice": {
                    "agent_visible": true,
                    "kind": "patchset_ci_reset_after_land",
                    "message": "Patchset CI reset after land moved fixture-ait:main from SNP-A to SNP-B",
                    "recommended_action": "rebase_patchset_to_latest_main",
                    "severity": "action_required",
                    "tests_status_semantics": "pending_not_test_failure"
                },
                "latest_job": {
                    "job_id": "JOB-RESET",
                    "state": "queued",
                    "diagnostic_status": "retry_pending",
                    "last_error": "Patchset CI reset after land moved fixture-ait:main from SNP-A to SNP-B",
                    "retry_pending": true
                }
            })
        }
        _ if method == "POST"
            && (url == "/v1/native/repository-authorities/7/patchsets/RP-1:runCi"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-2:runCi") =>
        {
            let parsed: JsonValue = parse_json(body);
            let patchset_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/patchsets/")
                .trim_end_matches(":runCi");
            json!({"patchset_id": patchset_id,"queued":true,"trigger": parsed.get("trigger").cloned().unwrap_or(JsonValue::Null)})
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/reviews") => {
            let parsed: JsonValue = parse_json(body);
            json!({
                "change_id":"RC-1",
                "patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                "reviewer": parsed.get("reviewer").cloned().unwrap_or(JsonValue::Null),
                "action": parsed.get("action").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1/reviews") => {
            let patchset_id = state
                .lock()
                .unwrap()
                .selected_patchset_id
                .clone()
                .unwrap_or_else(|| "RP-1".to_string());
            let reviews = if patchset_id == "RP-2" {
                json!([
                    {"reviewer":"Fixture User <fixture@example.com>","patchset_id": patchset_id,"action":"task_approve","blocking":false,"comment":"looks fine"},
                    {"reviewer":"Fixture User <fixture@example.com>","patchset_id": patchset_id,"action":"task_comment","blocking":false,"comment":"looks fine"}
                ])
            } else {
                json!([
                    {"reviewer":"Fixture User <fixture@example.com>","patchset_id": patchset_id,"action":"task_comment","blocking":false,"comment":"looks fine"}
                ])
            };
            json!({
                "change_id":"RC-1",
                "current_patchset_id": patchset_id,
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
                "review_requests":[{"patchset_id": patchset_id,"reviewer_group":"core","note":"Need review"}],
                "reviews": reviews
            })
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1:requestReview") => {
            let parsed: JsonValue = parse_json(body);
            json!({
                "change_id":"RC-1",
                "patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                "requested_groups": parsed.get("reviewer_groups").cloned().unwrap_or(JsonValue::Array(vec![])),
                "status":"requested"
            })
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1:selectPatchset") => {
            let parsed: JsonValue = parse_json(body);
            if let Some(patchset_id) = parsed.get("patchset_id").and_then(JsonValue::as_str) {
                state.lock().unwrap().selected_patchset_id = Some(patchset_id.to_string());
            }
            json!({
                "change_id":"RC-1",
                "selected_patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1:close") => {
            let parsed: JsonValue = parse_json(body);
            json!({
                "change_id":"RC-1",
                "status": parsed.get("status").cloned().unwrap_or(JsonValue::Null)
            })
        }
        _ if method == "PUT"
            && (url == "/v1/native/repository-authorities/7/patchsets/RP-1/attestation"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-2/attestation") =>
        {
            let parsed: JsonValue = parse_json(body);
            let patchset_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/patchsets/")
                .trim_end_matches("/attestation");
            json!({
                "patchset_id": patchset_id,
                "author_mode": parsed.get("author_mode").cloned().unwrap_or(JsonValue::Null),
                "evaluation_summary": parsed.get("evaluation_summary").cloned().unwrap_or(JsonValue::Null)
            })
        }
        _ if method == "GET"
            && (url == "/v1/native/repository-authorities/7/patchsets/RP-1/attestation"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-2/attestation") =>
        {
            let patchset_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/patchsets/")
                .trim_end_matches("/attestation");
            json!({
                "attestation_id":"AT-1",
                "patchset_id": patchset_id,
                "author_mode":"ai_with_human_review",
                "evaluation_summary":{"tests":"pass"},
                "provenance_summary":{"policy_readable":true}
            })
        }
        _ if method == "POST"
            && (url == "/v1/native/repository-authorities/7/patchsets/RP-1:evaluatePolicy"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-2:evaluatePolicy") =>
        {
            let patchset_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/patchsets/")
                .trim_end_matches(":evaluatePolicy");
            json!({"patchset_id": patchset_id,"lane":"assisted","decision":"pass","evaluated_at":"2026-06-08T00:00:00Z"})
        }
        _ if method == "GET"
            && (url == "/v1/native/repository-authorities/7/patchsets/RP-1/policy"
                || url == "/v1/native/repository-authorities/7/patchsets/RP-2/policy") =>
        {
            let patchset_id = url
                .trim_start_matches("/v1/native/repository-authorities/7/patchsets/")
                .trim_end_matches("/policy");
            json!({
                "policy_id":"PO-1",
                "patchset_id": patchset_id,
                "decision":"pass",
                "content_class":"code",
                "author_class":"hybrid",
                "effective_requirements":{"require_tests":true},
                "evaluated_at":"2026-06-08T00:00:00Z",
                "checks":[{"name":"require_tests","status":"pass","message":"ok"}],
                "input_fingerprint":"abc"
            })
        }
        ("POST", "/v1/native/repository-authorities/7/patchsets/RP-1/waivers") => {
            let parsed: JsonValue = parse_json(body);
            json!({
                "waiver_id":"WV-1",
                "patchset_id":"RP-1",
                "rule_name": parsed.get("rule_name").cloned().unwrap_or(JsonValue::Null),
                "reason": parsed.get("reason").cloned().unwrap_or(JsonValue::Null),
                "expires_at": parsed.get("expires_at").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1:submit") => {
            let parsed: JsonValue = parse_json(body);
            let landed_snapshot_id = {
                let mut guard = state.lock().unwrap();
                if guard.land_submit_base_stale_converged {
                    guard.base_stale_converged_submitted = true;
                    guard.remote_head_snapshot_id =
                        guard.selected_patchset_revision_snapshot_id.clone();
                    let target_line_head = guard
                        .selected_patchset_revision_snapshot_id
                        .clone()
                        .unwrap_or_else(|| FIXTURE_FINISHED_SNAPSHOT_ID.to_string());
                    return json_response(
                        200,
                        &json!({
                            "submission_id":"LAND-1",
                            "change_id":"RC-1",
                            "patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                            "status":"blocked",
                            "result":{
                                "blocker_class":"BASE_STALE",
                                "expected_base_snapshot_id": guard
                                    .selected_patchset_base_snapshot_id
                                    .clone()
                                    .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string()),
                                "target_line_head": target_line_head
                            }
                        }),
                    );
                }
                guard.land_submitted = true;
                guard.remote_head_snapshot_id =
                    guard.selected_patchset_revision_snapshot_id.clone();
                guard
                    .selected_patchset_revision_snapshot_id
                    .clone()
                    .unwrap_or_else(|| FIXTURE_FINISHED_SNAPSHOT_ID.to_string())
            };
            json!({
                "submission_id":"LAND-1",
                "change_id":"RC-1",
                "patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                "status":"succeeded",
                "result":{
                    "target_line":"main",
                    "line_action":"moved",
                    "landed_snapshot_id": landed_snapshot_id
                }
            })
        }
        ("GET", "/v1/native/lands/LAND-1")
        | ("GET", "/v1/native/repository-authorities/7/lands/LAND-1")
        | ("GET", "/v1/native/repository-authorities/7/lands/1") => {
            json!({
                "submission_id":"LAND-1",
                "land_seq":1,
                "change_id":"RC-1",
                "patchset_id":"RP-1",
                "target_line":"main",
                "mode":"direct",
                "status":"landed",
                "result":{
                    "target_line":"main",
                    "base_snapshot_id": FIXTURE_BASE_SNAPSHOT_ID,
                    "selected_revision_snapshot_id":FIXTURE_REVISION_SNAPSHOT_ID,
                    "landed_snapshot_id":FIXTURE_FINISHED_SNAPSHOT_ID,
                    "line_action":"moved",
                    "snapshot_action":"selected_patchset_revision",
                    "archived_lines":["feature/rt-1"],
                    "freshness_preflight":{
                        "already_aligned_equivalent":true,
                        "base_is_fresh":false,
                        "target_line":"main",
                        "target_line_head":FIXTURE_FINISHED_SNAPSHOT_ID,
                        "expected_base_snapshot_id": FIXTURE_BASE_SNAPSHOT_ID,
                        "revision_snapshot_id":FIXTURE_REVISION_SNAPSHOT_ID,
                        "target_matches_revision_snapshot":true,
                        "target_matches_revision_tree":true,
                        "checked_at":"2026-06-08T00:00:00Z"
                    },
                    "phase_timings_ms":{"total_process_land":1.2}
                },
                "result_json":"{\"landed_snapshot_id\":\"SNP-A11CE5EED002\"}"
            })
        }
        ("POST", "/v1/native/lands/LAND-1:retry")
        | ("POST", "/v1/native/repository-authorities/7/lands/1:retry") => {
            let parsed: JsonValue = parse_json(body);
            json!({
                "submission_id":"LAND-1",
                "change_id":"RC-1",
                "patchset_id":"RP-1",
                "status":"queued",
                "reason": parsed.get("reason").cloned().unwrap_or(JsonValue::Null)
            })
        }
        ("GET", "/v1/native/repository-authorities/7/tasks/RT-1") => {
            let guard = state.lock().unwrap();
            if guard.land_submitted || guard.task_completed {
                json!({
                    "task_id":"RT-1",
                    "status": if guard.task_completed { "completed" } else { "active" }
                })
            } else {
                json!({
                    "task_id":"RT-1"
                })
            }
        }
        ("GET", "/v1/native/repository-authorities/7/tasks") => {
            json!([
                {
                    "task_id":"RT-1",
                    "title":"Published task",
                    "status":"active",
                    "publication_state":"published"
                }
            ])
        }
        ("POST", "/v1/native/repository-authorities/7/tasks") => {
            let parsed: JsonValue = parse_json(body);
            let mut created = json!({
                "task_id": parsed.get("task_id").cloned().unwrap_or(JsonValue::String("RT-REMOTE".to_string())),
                "published_task_id": parsed.get("task_id").cloned().unwrap_or(JsonValue::String("RT-REMOTE".to_string())),
                "title": parsed.get("title").cloned().unwrap_or(JsonValue::Null),
                "intent": parsed.get("intent").cloned().unwrap_or(JsonValue::Null),
                "repo_name":"fixture-ait"
            });
            for field in ["plan_id", "origin_plan_revision_id", "plan_item_ref"] {
                if let Some(value) = parsed.get(field) {
                    created
                        .as_object_mut()
                        .expect("task create response")
                        .insert(field.to_string(), value.clone());
                }
            }
            created
        }
        ("POST", "/v1/native/repository-authorities/7/tasks/RT-1:close") => {
            let parsed: JsonValue = parse_json(body);
            state.lock().unwrap().task_completed = true;
            json!({"task_id":"RT-1","status": parsed.get("status").cloned().unwrap_or(JsonValue::Null)})
        }
        _ => panic!("unexpected request {method} {url} with body {body}"),
    };
    let body = encode_json_vec(&payload);
    Response::from_data(body).with_header(
        tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
    )
}

fn json_response(status_code: u16, payload: &JsonValue) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(encode_json_vec(payload))
        .with_status_code(status_code)
        .with_header(
            tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        )
}

fn response_for_publish_recovery(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<RecoveryRemoteState>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let present_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
    if let Some(response) = maybe_zstd_bulk_response(method, url, body, present_snapshot_id, None) {
        return response;
    }
    match (method, url) {
        ("GET", "/v1/handshake") => json_response(200, &handshake_payload()),
        ("GET", "/v1/native/repository-authorities/7") => {
            json_response(200, &repository_payload("fixture-ait"))
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1") => {
            json_response(404, &json!({"detail":"lane"}))
        }
        ("GET", "/v1/native/repository-authorities/7/changes") => {
            json_response(
                200,
                &json!([{
                    "change_id":"RC-1",
                    "base_line":"main",
                    "current_patchset_number":0
                }]),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/lines/main") => {
            let remote_head_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
            json_response(
                200,
                &json!({"line_name":"main","head_snapshot_id": remote_head_snapshot_id}),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/snapshots:exists") => {
            let parsed: JsonValue = parse_json(body);
            let snapshot_ids = parsed
                .get("snapshot_ids")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            json_response(
                200,
                &json!({
                    "repo_name":"fixture-ait",
                    "checked_snapshots": snapshot_ids.len(),
                    "present":[],
                    "missing": snapshot_ids
                }),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/patchsets") => {
            let parsed: JsonValue = parse_json(body);
            let mut guard = state.lock().unwrap();
            guard.published_base_snapshot_id = parsed
                .get("base_snapshot_id")
                .and_then(JsonValue::as_str)
                .map(|value| value.to_string());
            guard.published_revision_snapshot_id = parsed
                .get("revision_snapshot_id")
                .and_then(JsonValue::as_str)
                .map(|value| value.to_string());
            json_response(404, &json!({"detail":"lane"}))
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1/patchsets") => {
            let guard = state.lock().unwrap();
            let Some(base_snapshot_id) = guard.published_base_snapshot_id.clone() else {
                return json_response(200, &json!([]));
            };
            let Some(revision_snapshot_id) = guard.published_revision_snapshot_id.clone() else {
                return json_response(200, &json!([]));
            };
            json_response(
                200,
                &json!([{
                    "patchset_id":"RP-REC",
                    "change_id":"RC-1",
                    "patchset_number":1,
                    "base_snapshot_id": base_snapshot_id,
                    "revision_snapshot_id": revision_snapshot_id
                }]),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1") => {
            json_response(200, &json!({"patchset_id":"RP-1","change_id":"RC-1"}))
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/reviews") => {
            let parsed: JsonValue = parse_json(body);
            json_response(
                200,
                &json!({
                    "change_id":"RC-1",
                    "patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                    "reviewer": parsed.get("reviewer").cloned().unwrap_or(JsonValue::Null),
                    "action": parsed.get("action").cloned().unwrap_or(JsonValue::Null)
                }),
            )
        }
        _ if method == "PUT"
            && url.starts_with("/v1/native/repository-authorities/7/snapshots/") =>
        {
            let snapshot_id = url.rsplit('/').next().unwrap().trim_end_matches(":pack");
            json_response(
                200,
                &json!({"snapshot_id": snapshot_id, "repo_name":"fixture-ait"}),
            )
        }
        _ => panic!("unexpected recovery request {method} {url} with body {body}"),
    }
}

fn response_for_bounded_snapshot_sync(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<BoundedSnapshotRemoteState>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let present_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
    if let Some(response) = maybe_zstd_bulk_response(method, url, body, present_snapshot_id, None) {
        return response;
    }
    match (method, url) {
        ("GET", "/v1/handshake") => json_response(200, &handshake_payload()),
        ("GET", "/v1/native/repository-authorities/7") => {
            json_response(200, &repository_payload("fixture-ait"))
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1") => {
            json_response(
                200,
                &json!({"change_id":"RC-1","base_line":"main","selected_patchset_id":"RP-1"}),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/lines/main") => {
            let remote_head_snapshot_id = state.lock().unwrap().remote_head_snapshot_id.clone();
            json_response(
                200,
                &json!({"line_name":"main","head_snapshot_id": remote_head_snapshot_id}),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/snapshots:exists") => {
            let parsed: JsonValue = parse_json(body);
            let snapshot_ids = parsed
                .get("snapshot_ids")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            json_response(
                200,
                &json!({
                    "repo_name":"fixture-ait",
                    "checked_snapshots": snapshot_ids.len(),
                    "present": [],
                    "missing": snapshot_ids,
                }),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/patchsets") => {
            let parsed: JsonValue = parse_json(body);
            json_response(
                200,
                &json!({
                    "patchset_id":"RP-2",
                    "change_id":"RC-1",
                    "base_snapshot_id": parsed.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                    "revision_snapshot_id": parsed.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                    "summary": parsed.get("summary").cloned().unwrap_or(JsonValue::Null),
                    "author_mode": parsed.get("author_mode").cloned().unwrap_or(JsonValue::Null)
                }),
            )
        }
        _ if method == "PUT"
            && url.starts_with("/v1/native/repository-authorities/7/snapshots/") =>
        {
            let snapshot_id = url.rsplit('/').next().unwrap().trim_end_matches(":pack");
            json_response(
                200,
                &json!({"snapshot_id": snapshot_id, "repo_name":"fixture-ait"}),
            )
        }
        _ => panic!("unexpected bounded snapshot request {method} {url} with body {body}"),
    }
}

fn response_for_closeout_recovery(
    method: &str,
    url: &str,
    body: &str,
    state: &Arc<Mutex<CloseoutRecoveryRemoteState>>,
) -> Response<std::io::Cursor<Vec<u8>>> {
    if let Some(response) = maybe_zstd_bulk_response(method, url, body, None, None) {
        return response;
    }
    match (method, url) {
        ("GET", "/v1/handshake") => json_response(200, &handshake_payload()),
        ("GET", "/v1/native/repository-authorities/7") => {
            json_response(200, &repository_payload("fixture-ait"))
        }
        ("POST", "/v1/native/repository-authorities/7/task-land") => {
            let request = parse_json(body);
            assert_eq!(
                request.get("contract").and_then(JsonValue::as_str),
                Some("task-land-atomic/v1")
            );
            assert!(matches!(
                request.get("task_or_change_ref").and_then(JsonValue::as_str),
                Some("RT-1" | "RC-1" | "RT-1/C-01")
            ));
            {
                let guard = state.lock().unwrap();
                if guard.enforce_reviewer_workflow
                    && (!guard.code_review_recorded
                        || !guard.task_review_recorded
                        || !guard.policy_evaluated)
                {
                    return json_response(
                        409,
                        &json!({"detail":"reviewer Workflow Finish gates are incomplete"}),
                    );
                }
            }
            let (
                timeout_before_mutation,
                retryable_busy_after_mutation,
                retryable_busy_while_in_flight,
                starts_in_flight_mutation,
                replayed,
                response_delay,
                fixture_seed,
                landed_snapshot_id,
            ) = {
                let mut guard = state.lock().unwrap();
                guard.land_submit_attempts += 1;
                let idempotency_key = request
                    .get("idempotency_key")
                    .and_then(JsonValue::as_str)
                    .expect("atomic Task Land idempotency_key");
                if let Some(expected) = &guard.atomic_task_land_idempotency_key {
                    assert_eq!(idempotency_key, expected);
                } else {
                    guard.atomic_task_land_idempotency_key = Some(idempotency_key.to_string());
                }
                let replayed = guard.land_submitted && guard.task_completed;
                let timeout_before_mutation = !replayed
                    && (guard.land_boundary == CloseoutMutationBoundary::TimeoutBeforeMutation
                        || guard.task_boundary
                            == CloseoutMutationBoundary::TimeoutBeforeMutation);
                let retryable_busy_after_mutation = !replayed
                    && guard.land_boundary
                        == CloseoutMutationBoundary::RetryableBusyAfterMutation;
                let retryable_busy_while_in_flight = !replayed
                    && guard.land_boundary == CloseoutMutationBoundary::MutationInFlight
                    && guard.land_mutation_in_flight;
                let starts_in_flight_mutation = !replayed
                    && guard.land_boundary == CloseoutMutationBoundary::MutationInFlight
                    && !guard.land_mutation_in_flight;
                if starts_in_flight_mutation {
                    guard.land_mutation_in_flight = true;
                }
                let response_delay = if replayed || retryable_busy_while_in_flight {
                    Duration::ZERO
                } else {
                    guard.land_response_delay.max(guard.task_response_delay)
                };
                if !timeout_before_mutation
                    && !replayed
                    && !retryable_busy_while_in_flight
                    && !starts_in_flight_mutation
                {
                    guard.land_submitted = true;
                    guard.task_completed = true;
                    guard.land_submit_mutations += 1;
                    guard.task_close_attempts += 1;
                    guard.task_close_mutations += 1;
                }
                (
                    timeout_before_mutation,
                    retryable_busy_after_mutation,
                    retryable_busy_while_in_flight,
                    starts_in_flight_mutation,
                    replayed,
                    response_delay,
                    guard.fixture_seed,
                    guard
                        .patchset_revision_snapshot_id
                        .clone()
                        .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string()),
                )
            };
            if retryable_busy_while_in_flight {
                return json_response(
                    503,
                    &json!({
                        "detail": "ait.binary-db.error.v1|retryable_busy|scope=ServerLand|operation=Atomic Task Land mutation in flight",
                        "fixture_seed": fixture_seed
                    }),
                );
            }
            thread::sleep(response_delay);
            if starts_in_flight_mutation {
                let mut guard = state.lock().unwrap();
                if !guard.land_submitted {
                    guard.land_submitted = true;
                    guard.task_completed = true;
                    guard.land_submit_mutations += 1;
                    guard.task_close_attempts += 1;
                    guard.task_close_mutations += 1;
                }
                guard.land_mutation_in_flight = false;
            }
            if timeout_before_mutation {
                return json_response(
                    504,
                    &json!({
                        "detail": "injected atomic Task Land timeout before mutation",
                        "fixture_seed": fixture_seed
                    }),
                );
            }
            if retryable_busy_after_mutation {
                return json_response(
                    503,
                    &json!({
                        "detail": "ait.binary-db.error.v1|retryable_busy|scope=server-content|operation=Atomic Task Land response projection",
                        "fixture_seed": fixture_seed
                    }),
                );
            }
            json_response(
                200,
                &fake_atomic_task_land_response(
                    &request,
                    replayed,
                    &landed_snapshot_id,
                    "RP-1",
                    "LAND-REC",
                ),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1:submit") => {
            let (boundary, response_delay, fixture_seed, landed_snapshot_id) = {
                let mut guard = state.lock().unwrap();
                guard.land_submit_attempts += 1;
                let boundary = guard.land_boundary;
                if boundary == CloseoutMutationBoundary::MutateBeforeResponse
                    && !guard.land_submitted
                {
                    guard.land_submitted = true;
                    guard.land_submit_mutations += 1;
                }
                (
                    boundary,
                    guard.land_response_delay,
                    guard.fixture_seed,
                    guard
                    .patchset_revision_snapshot_id
                    .clone()
                    .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string()),
                )
            };
            thread::sleep(response_delay);
            if boundary == CloseoutMutationBoundary::TimeoutBeforeMutation {
                return json_response(
                    504,
                    &json!({
                        "detail":"injected timeout before land mutation",
                        "fixture_seed": fixture_seed
                    }),
                );
            }
            json_response(
                200,
                &json!({
                    "submission_id":"LAND-REC",
                    "status":"succeeded",
                    "result":{
                        "target_line":"main",
                        "line_action":"moved",
                        "landed_snapshot_id": landed_snapshot_id
                    }
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1") => {
            let submitted = state.lock().unwrap().land_submitted;
            let landing_summary = if submitted {
                json!({
                    "submission_id":"LAND-REC",
                    "status":"succeeded",
                    "result":{
                        "target_line":"main",
                        "line_action":"moved",
                        "landed_snapshot_id": state.lock().unwrap().patchset_revision_snapshot_id.clone().unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string())
                    }
                })
            } else {
                JsonValue::Null
            };
            json_response(
                200,
                &json!({
                    "change_id":"RC-1",
                    "task_id":"RT-1",
                    "title":"Closeout recovery change",
                    "base_line":"main",
                    "status": if submitted { "landed" } else { "approved" },
                    "publication_state":"published",
                    "published_change_id":"RC-1",
                    "selected_patchset_id":"RP-1",
                    "landing_summary": landing_summary
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RT-1") => {
            json_response(
                200,
                &json!({
                    "change_id":"RC-WRONG",
                    "task_id":"RT-WRONG",
                    "title":"Wrong same-sequence change",
                    "base_line":"main",
                    "status":"landed",
                    "publication_state":"published",
                    "published_change_id":"RC-WRONG",
                    "selected_patchset_id":"RP-WRONG",
                    "landing_summary":{
                        "submission_id":"LAND-WRONG",
                        "status":"succeeded",
                        "result":{"landed_snapshot_id":"SNP-WRONG"}
                    }
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/read/changes/RC-1") => {
            let submitted = state.lock().unwrap().land_submitted;
            let landing_summary = if submitted {
                json!({
                    "submission_id":"LAND-REC",
                    "patchset_id":"RP-1",
                    "status":"succeeded",
                    "result":{
                        "target_line":"main",
                        "line_action":"moved",
                        "landed_snapshot_id": state.lock().unwrap().patchset_revision_snapshot_id.clone().unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string())
                    }
                })
            } else {
                JsonValue::Null
            };
            json_response(
                200,
                &json!({
                    "change_id":"RC-1",
                    "task_id":"RT-1",
                    "selected_patchset_id":"RP-1",
                    "landing_summary": landing_summary
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/changes") => {
            let guard = state.lock().unwrap();
            let landing_summary = if guard.land_submitted {
                json!({
                    "submission_id":"LAND-REC",
                    "status":"succeeded",
                    "result":{
                        "target_line":"main",
                        "line_action":"moved",
                        "landed_snapshot_id": guard.patchset_revision_snapshot_id.clone().unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string())
                    }
                })
            } else {
                JsonValue::Null
            };
            json_response(
                200,
                &json!([{
                    "change_id":"RC-1",
                    "task_id":"RT-1",
                    "title":"Closeout recovery change",
                    "base_line":"main",
                    "status": if guard.land_submitted { "landed" } else { "approved" },
                    "publication_state":"published",
                    "published_change_id":"RC-1",
                    "selected_patchset_id":"RP-1",
                    "landing_summary": landing_summary
                }]),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/tasks") => {
            let completed = state.lock().unwrap().task_completed;
            json_response(
                200,
                &json!([{
                    "task_id":"RT-1",
                    "repo_name":"fixture-ait",
                    "title":"Closeout recovery task",
                    "intent":"prove authoritative recovery",
                    "status": if completed { "completed" } else { "active" },
                    "publication_state":"published",
                    "published_task_id":"RT-1"
                }]),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1")
        | ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1?change_ref=RC-1") => {
            let revision_snapshot_id = state
                .lock()
                .unwrap()
                .patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            json_response(
                200,
                &json!({
                    "patchset_id":"RP-1",
                    "change_id":"RC-1",
                    "base_snapshot_id": FIXTURE_BASE_SNAPSHOT_ID,
                    "revision_snapshot_id": revision_snapshot_id,
                    "publish_state":"published",
                    "evaluation_state":"pass",
                    "ci_run_seq": 1,
                    "ci_completed_at_s": 1_783_814_400_u64,
                    "ci": {
                        "run_seq": 1,
                        "completed_at_s": 1_783_814_400_u64,
                        "overall_status": "pass",
                        "tests_status": "pass",
                        "lint_status": "none",
                        "selected_suite_count": 1,
                        "suite_result_count": 1,
                        "blocking_failure_count": 0
                    }
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/lines/main") => {
            let guard = state.lock().unwrap();
            let submitted = guard.land_submitted;
            let landed_snapshot_id = guard
                .patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            json_response(
                200,
                &json!({
                    "line_name":"main",
                    "head_snapshot_id": if submitted { landed_snapshot_id } else { FIXTURE_BASE_SNAPSHOT_ID.to_string() }
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/lines") => {
            let guard = state.lock().unwrap();
            let landed_snapshot_id = guard
                .patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            json_response(
                200,
                &json!([
                    {
                        "line_name":"main",
                        "head_snapshot_id": if guard.land_submitted {
                            landed_snapshot_id.clone()
                        } else {
                            FIXTURE_BASE_SNAPSHOT_ID.to_string()
                        }
                    },
                    {
                        "line_name":"feature/rt-1",
                        "head_snapshot_id": landed_snapshot_id
                    }
                ]),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/lines/feature%2Frt-1") => {
            let revision_snapshot_id = state
                .lock()
                .unwrap()
                .patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            json_response(
                200,
                &json!({
                    "line_name":"feature/rt-1",
                    "head_snapshot_id": revision_snapshot_id
                }),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/lines/feature%2Frt-1:close") => {
            let parsed: JsonValue = parse_json(body);
            assert_eq!(
                parsed.get("status").and_then(JsonValue::as_str),
                Some("archived")
            );
            let revision_snapshot_id = state
                .lock()
                .unwrap()
                .patchset_revision_snapshot_id
                .clone()
                .unwrap_or_else(|| FIXTURE_BASE_SNAPSHOT_ID.to_string());
            json_response(
                200,
                &json!({
                    "repo_name":"fixture-ait",
                    "line_name":"feature/rt-1",
                    "head_snapshot_id":revision_snapshot_id,
                    "status":"archived"
                }),
            )
        }
        ("PUT", "/v1/native/repository-authorities/7/lines/feature%2Frt-1") => {
            let parsed: JsonValue = parse_json(body);
            json_response(
                200,
                &json!({
                    "line_name":"feature/rt-1",
                    "head_snapshot_id": parsed.get("head_snapshot_id").cloned().unwrap_or(JsonValue::Null)
                }),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/snapshots:exists") => {
            let parsed: JsonValue = parse_json(body);
            let snapshot_ids = parsed
                .get("snapshot_ids")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default();
            json_response(
                200,
                &json!({
                    "repo_name":"fixture-ait",
                    "checked_snapshots": snapshot_ids.len(),
                    "present": [],
                    "missing": snapshot_ids,
                }),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/patchsets") => {
            let parsed: JsonValue = parse_json(body);
            if let Some(revision_snapshot_id) = parsed
                .get("revision_snapshot_id")
                .and_then(JsonValue::as_str)
            {
                state.lock().unwrap().patchset_revision_snapshot_id =
                    Some(revision_snapshot_id.to_string());
            }
            json_response(
                200,
                &json!({
                    "patchset_id":"RP-1",
                    "change_id":"RC-1",
                    "base_snapshot_id": parsed.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                    "revision_snapshot_id": parsed.get("revision_snapshot_id").cloned().unwrap_or(JsonValue::Null),
                    "summary": parsed.get("summary").cloned().unwrap_or(JsonValue::Null),
                    "author_mode": parsed.get("author_mode").cloned().unwrap_or(JsonValue::Null)
                }),
            )
        }
        _ if method == "PUT"
            && url.starts_with("/v1/native/repository-authorities/7/snapshots/") =>
        {
            let snapshot_id = url.rsplit('/').next().unwrap().trim_end_matches(":pack");
            json_response(
                200,
                &json!({"snapshot_id": snapshot_id, "repo_name":"fixture-ait"}),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1/reviews") => {
            let parsed: JsonValue = parse_json(body);
            match parsed.get("action").and_then(JsonValue::as_str) {
                Some("code_review_summary") => {
                    state.lock().unwrap().code_review_recorded = true;
                }
                Some("task_approve") => {
                    state.lock().unwrap().task_review_recorded = true;
                }
                _ => {}
            }
            json_response(
                200,
                &json!({
                    "change_id":"RC-1",
                    "patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
                    "reviewer": parsed.get("reviewer").cloned().unwrap_or(JsonValue::Null),
                    "action": parsed.get("action").cloned().unwrap_or(JsonValue::Null)
                }),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/changes/RC-1:selectPatchset") => {
            let parsed: JsonValue = parse_json(body);
            json_response(
                200,
                &json!({
                    "change_id":"RC-1",
                    "selected_patchset_id": parsed.get("patchset_id").cloned().unwrap_or(JsonValue::Null)
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/changes/RC-1/reviews") => {
            let guard = state.lock().unwrap();
            let task_reviews = if guard.task_review_recorded {
                vec![json!({"reviewer":"Fixture User","patchset_id":"RP-1","action":"task_approve","blocking":false,"comment":"looks fine"})]
            } else {
                Vec::new()
            };
            let mut reviews = task_reviews.clone();
            if guard.code_review_recorded {
                reviews.push(json!({
                    "reviewer":"ait-cli",
                    "patchset_id":"RP-1",
                    "action":"code_review_summary",
                    "blocking":false,
                    "comment":"Reviewed files: src/lib.rs; Findings: no blocking findings; Risks: low; Tests: cargo test passed; Recommendation: land."
                }));
            }
            json_response(
                200,
                &json!({
                    "change_id":"RC-1",
                    "current_patchset_id":"RP-1",
                    "approvals":task_reviews.len(),
                    "blocking":0,
                    "comments":0,
                    "task_approvals":task_reviews.len(),
                    "team_approvals":0,
                    "human_approvals":task_reviews.len(),
                    "human_task_approvals":task_reviews.len(),
                    "independent_human_approvals":task_reviews.len(),
                    "independent_task_approvals":task_reviews.len(),
                    "code_review_summaries":if guard.code_review_recorded { 1 } else { 0 },
                    "code_review_summary_reviewers":if guard.code_review_recorded { json!(["ait-cli"]) } else { json!([]) },
                    "review_requests":[],
                    "reviews":reviews
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1/attestation") => {
            json_response(
                200,
                &json!({
                    "attestation_id":"AT-1",
                    "patchset_id":"RP-1",
                    "author_mode":"ai_with_human_review",
                    "evaluation_summary":{"tests":"pass"},
                    "provenance_summary":{"policy_readable":true}
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/patchsets/RP-1/policy") => {
            let policy_evaluated = state.lock().unwrap().policy_evaluated;
            json_response(
                200,
                &json!({
                    "policy_id":"PO-1",
                    "patchset_id":"RP-1",
                    "decision":if policy_evaluated { "pass" } else { "pending" },
                    "content_class":"code",
                    "author_class":"hybrid",
                    "effective_requirements":{"require_tests":true,"require_code_review_summary":true},
                    "evaluated_at":"2026-06-08T00:00:00Z",
                    "checks":[{"name":"require_tests","status":"pass","message":"ok"}],
                    "input_fingerprint":"abc"
                }),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/patchsets/RP-1:evaluatePolicy") => {
            state.lock().unwrap().policy_evaluated = true;
            json_response(
                200,
                &json!({
                    "policy_id":"PO-1",
                    "patchset_id":"RP-1",
                    "decision":"pass",
                    "checks":[{"name":"require_tests","status":"pass","message":"ok"}]
                }),
            )
        }
        _ if method == "GET" && url.starts_with("/v1/native/repository-authorities/7/read/patchsets/RP-1/ci-status") => {
            json_response(
                200,
                &json!({
                    "contract":"ait.server.patchset_ci.readiness.v1",
                    "patchset_id":"RP-1",
                    "change_id":"RC-1",
                    "tests_status":"pass",
                    "has_runnable_evidence":true,
                    "selected_suite_ids":["preflight"],
                    "latest_job":{"job_id":"JOB-1","job_type":"patchset.ci","state":"succeeded"}
                }),
            )
        }
        _ if method == "GET"
            && url.starts_with(
                "/v1/native/repository-authorities/7/read/tasks/RT-1/audit?target_line=main",
            ) =>
        {
            json_response(
                200,
                &json!({
                    "task": {
                        "task_id": "RT-1",
                        "repo_name": "fixture-ait",
                        "status": "active",
                    },
                    "target_line": "main",
                    "target_line_head": FIXTURE_BASE_SNAPSHOT_ID,
                    "summary": {
                        "verdict": "in_progress",
                        "open_changes": 1,
                    },
                    "changes": [{
                        "change": {
                            "change_id": "RC-1",
                            "task_id": "RT-1",
                            "status": "draft",
                        },
                        "target_state": "not_landed",
                    }],
                }),
            )
        }
        ("GET", "/v1/native/repository-authorities/7/tasks/RT-1") => {
            let completed = state.lock().unwrap().task_completed;
            json_response(
                200,
                &json!({
                    "task_id":"RT-1",
                    "repo_name":"fixture-ait",
                    "title":"Closeout recovery task",
                    "intent":"prove authoritative recovery",
                    "status": if completed { "completed" } else { "active" },
                    "publication_state":"published",
                    "published_task_id":"RT-1"
                }),
            )
        }
        ("POST", "/v1/native/repository-authorities/7/tasks/RT-1:close") => {
            let parsed: JsonValue = parse_json(body);
            assert_eq!(
                parsed.get("status").and_then(JsonValue::as_str),
                Some("completed")
            );
            let (boundary, response_delay, fixture_seed) = {
                let mut guard = state.lock().unwrap();
                guard.task_close_attempts += 1;
                let boundary = guard.task_boundary;
                if boundary == CloseoutMutationBoundary::MutateBeforeResponse
                    && !guard.task_completed
                {
                    guard.task_completed = true;
                    guard.task_close_mutations += 1;
                }
                (
                    boundary,
                    guard.task_response_delay,
                    guard.fixture_seed,
                )
            };
            thread::sleep(response_delay);
            if boundary == CloseoutMutationBoundary::TimeoutBeforeMutation {
                return json_response(
                    504,
                    &json!({
                        "detail":"injected timeout before Task mutation",
                        "fixture_seed": fixture_seed
                    }),
                );
            }
            json_response(200, &json!({"task_id":"RT-1","status":"completed"}))
        }
        _ => panic!("unexpected closeout recovery request {method} {url} with body {body}"),
    }
}
