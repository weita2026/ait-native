use super::*;

fn mock_response(
    state: &Arc<Mutex<FakeRemoteState>>,
    method: &str,
    path: &str,
    body: JsonValue,
) -> (u16, JsonValue) {
    let response = response_for(method, path, &body.to_string(), state);
    let status = response.status_code().0;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut response.into_reader(), &mut text).unwrap();
    (status, parse_value_error_string(&text).unwrap())
}

fn actor_registration() -> JsonValue {
    json!({"repository_index":7,"repository_name":"fixture-ait","namespace":"","policy_flags":0,"user_name":" Alice ","user_email":" A@Example "})
}

#[test]
fn mock_actor_registration_preserves_exact_repository_and_paired_zero_valid_identity() {
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    let request = actor_registration();
    let (status, first) = mock_response(
        &state,
        "POST",
        "/v1/native/repository-authorities",
        request.clone(),
    );
    assert_eq!(status, 200);
    assert_eq!(first["actor_key_index"], 0);
    let (_, authority) = mock_response(
        &state,
        "GET",
        "/v1/native/repository-authorities/7",
        JsonValue::Null,
    );
    assert_eq!(first["repository"], authority["repository"]);
    let mut same = request.clone();
    same["user_name"] = json!("Alice");
    same["user_email"] = json!("A@Example");
    assert_eq!(
        mock_response(
            &state,
            "POST",
            "/v1/native/repository-authorities",
            same.clone()
        )
        .1["actor_key_index"],
        0
    );
    same["user_name"] = json!("alice");
    assert_eq!(
        mock_response(
            &state,
            "POST",
            "/v1/native/repository-authorities",
            same.clone()
        )
        .1["actor_key_index"],
        1
    );
    same["user_email"] = json!("a@example");
    assert_eq!(
        mock_response(&state, "POST", "/v1/native/repository-authorities", same).1
            ["actor_key_index"],
        2
    );
}

#[test]
fn mock_actor_registration_rejects_invalid_inputs_before_effects() {
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    for (field, value) in [
        ("repository_index", json!(8)),
        ("repository_index", JsonValue::Null),
        ("repository_name", json!("another")),
        ("namespace", json!("R")),
        ("policy_flags", json!(1)),
        ("user_name", json!(" ")),
        ("user_email", JsonValue::Null),
        ("user_email", json!("x".repeat(256))),
        ("unexpected", json!(true)),
    ] {
        let mut request = actor_registration();
        request[field] = value;
        assert_ne!(
            mock_response(&state, "POST", "/v1/native/repository-authorities", request).0,
            200,
            "{field}"
        );
        assert!(state.lock().unwrap().actor_keys.is_empty());
        assert!(state.lock().unwrap().created_task_ids.is_empty());
    }
}

#[test]
fn mock_current_patchset_validation_rejects_foreign_owners_without_selection_writes() {
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    {
        let mut locked = state.lock().unwrap();
        locked.patchset_task_id = Some("RT-2".into());
        locked.selected_patchset_id = Some("RT-2/P-01".into());
    }
    for (owner, candidate) in [("RT-3", "RT-2/P-01"), ("RT-2", "RT-2/P-02")] {
        assert_eq!(
            mock_response(
                &state,
                "POST",
                &format!("/v1/native/repository-authorities/7/changes/{owner}:selectPatchset"),
                json!({"patchset_id":candidate})
            )
            .0,
            409
        );
    }
    assert_eq!(
        mock_response(
            &state,
            "GET",
            "/v1/native/repository-authorities/7/patchsets/RT-2%2FP-01?change_ref=RT-3",
            JsonValue::Null
        )
        .0,
        409
    );
    let locked = state.lock().unwrap();
    assert_eq!(locked.selected_patchset_id.as_deref(), Some("RT-2/P-01"));
    assert!(locked.selected_change_id.is_none());
    assert!(locked.created_change_refs.is_empty());
}

#[test]
fn mock_patchset_readback_retains_the_published_author_mode() {
    for author_mode in ["ai_only_experimental", "ai_with_human_review"] {
        let state = Arc::new(Mutex::new(FakeRemoteState::default()));
        state.lock().unwrap().created_task_ids.insert("RT-2".into());
        let (status, published) = mock_response(
            &state,
            "POST",
            "/v1/native/repository-authorities/7/changes/RT-2/patchsets",
            json!({
                "base_snapshot_id":"SNP-BASE",
                "revision_snapshot_id":"SNP-REV",
                "author_mode":author_mode
            }),
        );
        assert_eq!(status, 200);
        assert_eq!(published["author_mode"], author_mode);
        for path in [
            "/v1/native/repository-authorities/7/changes/RT-2/patchsets",
            "/v1/native/repository-authorities/7/patchsets/RT-2%2FP-01",
            "/v1/native/repository-authorities/7/patchsets/RT-2%2FP-01?change_ref=RT-2",
        ] {
            let (status, response) = mock_response(&state, "GET", path, JsonValue::Null);
            assert_eq!(status, 200);
            let patchset = if response.is_array() {
                &response[0]
            } else {
                &response
            };
            assert_eq!(patchset["author_mode"], author_mode, "{path}");
        }
    }
}

#[test]
fn mock_missing_snapshot_uses_the_exact_native_authority_response() {
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    let (status, response) = mock_response(
        &state,
        "GET",
        "/v1/native/repository-authorities/7/snapshots/SNP-ABSENT?include_content=false",
        JsonValue::Null,
    );
    assert_eq!(status, 404);
    assert_eq!(
        response["detail"],
        "Unknown snapshot SNP-ABSENT for repository fixture-ait"
    );
    assert!(state.lock().unwrap().snapshots.is_empty());
}

#[test]
fn mock_task_only_finish_requires_real_candidate_evidence_and_keeps_change_null() {
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    {
        let mut locked = state.lock().unwrap();
        locked.created_task_ids.insert("RT-2".into());
        locked.patchset_task_id = Some("RT-2".into());
        locked.selected_patchset_id = Some("RT-2/P-01".into());
        locked.selected_patchset_base_snapshot_id = Some("SNP-BASE".into());
        locked.selected_patchset_revision_snapshot_id = Some("SNP-REV".into());
        locked.remote_head_snapshot_id = Some("SNP-BASE".into());
    }
    let request = json!({"task_or_change_ref":"RT-2","target_line":"main","idempotency_key":"finish-fixture"});
    for action in [
        "ci",
        "attestation",
        "policy",
        "approve",
        "code_review_summary",
        "task_approve",
    ] {
        assert_eq!(
            mock_response(
                &state,
                "POST",
                "/v1/native/repository-authorities/7/task-land",
                request.clone()
            )
            .0,
            409
        );
        let mut locked = state.lock().unwrap();
        assert!(locked.task_receipts.is_empty());
        assert!(locked.closed_task_ids.is_empty());
        assert_eq!(locked.remote_head_snapshot_id.as_deref(), Some("SNP-BASE"));
        match action {
            "ci" => {
                locked.ci_run_patchset_ids.insert("RT-2/P-01".into());
            }
            "attestation" => {
                locked.attested_patchset_ids.insert("RT-2/P-01".into());
            }
            "policy" => {
                locked.evaluated_patchset_ids.insert("RT-2/P-01".into());
            }
            _ => locked
                .review_events
                .push(json!({"action":action,"patchset_id":"RT-2/P-01"})),
        }
    }
    let (status, response) = mock_response(
        &state,
        "POST",
        "/v1/native/repository-authorities/7/task-land",
        request,
    );
    assert_eq!(status, 200);
    assert!(response["change"].is_null());
    assert!(response["change_ref"].is_null());
    let receipt = ait_core::land_json::LandJson::stateless()
        .task_successful_receipt(&response["task"], "RT-2", Some(7))
        .unwrap()
        .unwrap();
    assert_eq!(receipt["patchset_id"], "RT-2/P-01");
    assert!(state.lock().unwrap().created_change_refs.is_empty());
}

#[test]
fn mock_null_head_cas_rejects_competing_target_before_snapshot_effects() {
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    state.lock().unwrap().remote_head_snapshot_id = Some("SNP-PEER".into());
    let (status, _) = mock_response(
        &state,
        "POST",
        "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit",
        json!({
            "snapshots":[{"snapshot_id":"SNP-BASE","line_name":"main"}],
            "line_update":{"line_name":"main","head_snapshot_id":"SNP-BASE","expected_head_snapshot_id":null}
        }),
    );
    assert_eq!(status, 409);
    let locked = state.lock().unwrap();
    assert!(locked.snapshots.is_empty());
    assert_eq!(locked.remote_head_snapshot_id.as_deref(), Some("SNP-PEER"));
}

fn write_release_exec_fixture(root: &Path) {
    fs::create_dir_all(root.join("src/ait/cli/commands")).unwrap();
    fs::create_dir_all(root.join("ci")).unwrap();
    fs::write(
        root.join("pyproject.toml"),
        "[project.scripts]\nait = \"ait.cli_entrypoint:main\"\n",
    )
    .unwrap();
    fs::write(
        root.join("src/ait/cli_entrypoint.py"),
        "NATIVE = {\"release\"}\nos.execvpe(binary, argv, env)\nfrom .cli import app\n",
    )
    .unwrap();
    fs::write(
        root.join("src/ait/cli/app_surfaces.py"),
        "# no release route\n",
    )
    .unwrap();
    fs::write(
        root.join("src/ait/cli/native_namespace_command.py"),
        "NATIVE_WORKFLOW_GATE_NAMESPACES = set()\n",
    )
    .unwrap();
    fs::write(
        root.join("src/ait/cli/commands/bootstrap.py"),
        "PRIMARY_COMMAND_MODULES = {}\n",
    )
    .unwrap();
    fs::write(
        root.join("ci/patch_ci.json"),
        r#"{"schema_version":1,"suites":[{"runner":{"kind":"test_discovery_sharded","build_args":["test","--test","patchset_ci_runner","--no-run"]}}]}"#,
    )
    .unwrap();
}

#[test]
fn release_authority_guard_requires_console_exec_and_native_smoke() {
    let temp = TempDir::new().unwrap();
    write_release_exec_fixture(temp.path());

    assert_release_python_authority_retired(temp.path()).unwrap();
}

#[test]
fn release_authority_guard_rejects_returning_python_namespace_bridge() {
    let temp = TempDir::new().unwrap();
    write_release_exec_fixture(temp.path());
    fs::write(
        temp.path().join("src/ait/cli/app_surfaces.py"),
        "_register_native_namespace_command(\"release\")\n",
    )
    .unwrap();

    let error = assert_release_python_authority_retired(temp.path()).unwrap_err();
    assert!(error.contains("still registered in the Python CLI application"));
}

#[test]
fn release_authority_guard_rejects_retired_python_module() {
    let temp = TempDir::new().unwrap();
    write_release_exec_fixture(temp.path());
    fs::write(
        temp.path().join("src/ait/cli/commands/release.py"),
        "# retired\n",
    )
    .unwrap();

    let error = assert_release_python_authority_retired(temp.path()).unwrap_err();
    assert!(error.contains("src/ait/cli/commands/release.py"));
}

#[test]
fn fake_remote_survives_fixture_setup_idle_window() {
    let mut remote = spawn_fake_remote();

    thread::sleep(Duration::from_millis(2200));
    let response = fake_remote_get(&remote.base_url, "/healthz");

    assert!(response.starts_with("HTTP/1.1 200"));
    assert!(response.contains("patchset_run_ci_route"));
    remote.stop().unwrap();
}

fn fake_remote_get(base_url: &str, path: &str) -> String {
    let address = base_url
        .strip_prefix("http://")
        .expect("fake remote base url should be http");
    let mut stream =
        std::net::TcpStream::connect(address).expect("fake remote should accept connections");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    std::io::Write::write_all(&mut stream, request.as_bytes())
        .expect("fake remote request should be written");
    let mut response = String::new();
    std::io::Read::read_to_string(&mut stream, &mut response)
        .expect("fake remote response should be read");
    response
}

fn snapshot_remote_call(
    state: &Arc<Mutex<FakeRemoteState>>,
    method: &str,
    suffix: &str,
    payload: JsonValue,
) -> (u16, JsonValue) {
    let response = response_for(
        method,
        &format!("/v1/native/repository-authorities/7/{suffix}"),
        &encode_value_or(&payload, "{}"),
        state,
    );
    let status = response.status_code().0;
    let mut text = String::new();
    std::io::Read::read_to_string(&mut response.into_reader(), &mut text).unwrap();
    (status, parse_value_error_string(&text).unwrap())
}

fn snapshot_remote_state() -> Arc<Mutex<FakeRemoteState>> {
    let state = Arc::new(Mutex::new(FakeRemoteState::default()));
    for task in ["RT-2", "RT-3"] {
        assert_eq!(
            snapshot_remote_call(&state, "POST", "tasks", json!({"task_id": task})).0,
            200
        );
        for change in ["C-01", "C-02"] {
            assert_eq!(
                snapshot_remote_call(
                    &state,
                    "POST",
                    "changes",
                    json!({"task_id": task, "change_id": change})
                )
                .0,
                200
            );
        }
    }
    state
}

fn snapshot_capture_request(id: &str) -> JsonValue {
    json!({
        "contract": "ait.remote_sync.zstd_bulk.capture.v1",
        "snapshots": [{"snapshot_id": id, "line_name": "feature/rt-2"}],
        "capture_binding": {
            "contract": SNAPSHOT_CAPTURE_BINDING_CONTRACT,
            "repository_index": 7,
            "snapshot_id": id,
            "task_id": "RT-2",
            "change_ref": "RT-2/C-01",
            "line_name": "feature/rt-2",
        }
    })
}

const SNAPSHOT_COMMIT_ROUTE: &str = "remote-sync/zstd-bulk/commit";

#[test]
fn fake_remote_capture_accepts_task_only_without_creating_change() {
    let state = snapshot_remote_state();
    let id = "SNP-000000000012";
    let mut request = snapshot_capture_request(id);
    request["capture_binding"]["change_ref"] = JsonValue::Null;
    let before = state.lock().expect("state").created_change_refs.clone();
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, request.clone()).0,
        200
    );
    assert_eq!(state.lock().expect("state").created_change_refs, before);
    let read = snapshot_remote_call(
        &state,
        "GET",
        &format!("snapshots/{id}?include_content=false"),
        JsonValue::Null,
    )
    .1;
    assert_eq!(read["capture_binding"], request["capture_binding"]);
    request["capture_binding"]["change_ref"] = json!("RT-2/C-01");
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, request).0,
        409
    );
}

#[test]
fn fake_remote_capture_readback_and_presence_preserve_exact_owner() {
    let state = snapshot_remote_state();
    let id = "SNP-000000000002";
    let request = snapshot_capture_request(id);
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, request.clone()).0,
        200
    );
    let read = || {
        snapshot_remote_call(
            &state,
            "GET",
            &format!("snapshots/{id}?include_content=false"),
            JsonValue::Null,
        )
        .1
    };
    assert_eq!(read()["capture_binding"], request["capture_binding"]);
    assert_eq!(read()["line_name"], "feature/rt-2");
    assert_eq!(
        snapshot_remote_call(
            &state,
            "POST",
            "snapshots:exists",
            json!({"snapshot_ids": [id]})
        )
        .1["present"],
        json!([id])
    );
    assert_eq!(
        snapshot_remote_call(
            &state,
            "POST",
            "remote-sync/zstd-bulk/plan",
            json!({"snapshot_ids": [id]})
        )
        .1["present_snapshot_ids"],
        json!([id])
    );

    // Exact replay and ordinary publication must retain acknowledged ownership.
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, request.clone()).0,
        200
    );
    let mut ordinary = request.clone();
    ordinary["contract"] = JsonValue::Null;
    ordinary["capture_binding"] = JsonValue::Null;
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, ordinary).0,
        200
    );
    assert_eq!(read()["capture_binding"], request["capture_binding"]);
    for (field, value) in [
        ("change_ref", json!("RT-2/C-02")),
        ("worktree_name", json!("another-worktree")),
        ("line_name", json!("another-line")),
        ("repository_index", json!(8)),
    ] {
        let mut changed = request.clone();
        changed["capture_binding"][field] = value;
        assert_eq!(
            snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, changed).0,
            409,
            "{field}"
        );
        assert_eq!(read()["capture_binding"], request["capture_binding"]);
    }
    let mut other_task = request.clone();
    other_task["capture_binding"]["task_id"] = json!("RT-3");
    other_task["capture_binding"]["change_ref"] = json!("RT-3/C-01");
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, other_task).0,
        409
    );
    assert_eq!(
        snapshot_remote_call(
            &state,
            "POST",
            SNAPSHOT_COMMIT_ROUTE,
            snapshot_capture_request("SNP-000000000003")
        )
        .0,
        200
    );
    assert!(state.lock().unwrap().remote_head_snapshot_id.is_none());
}

#[test]
fn fake_remote_rejects_incomplete_capture_without_partial_state_or_legacy_backfill() {
    let state = snapshot_remote_state();
    let id = "SNP-000000000002";
    let request = snapshot_capture_request(id);
    for (field, value) in [
        ("change_ref", json!("RT-3/C-01")),
        ("change_ref", json!("LT-2/C-01")),
        ("change_ref", json!("C-01")),
        ("change_ref", json!("RT-2/C-03")),
        ("task_id", json!("RT-UNKNOWN")),
        ("worktree_name", json!("")),
        ("line_name", json!("wrong-line")),
        ("repository_index", json!(8)),
    ] {
        let mut invalid = request.clone();
        invalid["capture_binding"][field] = value;
        assert_eq!(
            snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, invalid).0,
            409,
            "{field}"
        );
    }
    let mut missing = request.clone();
    missing["capture_binding"] = JsonValue::Null;
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, missing.clone()).0,
        409
    );
    let mut incomplete_batch = request.clone();
    incomplete_batch["snapshots"]
        .as_array_mut()
        .unwrap()
        .push(json!({"snapshot_id": "SNP-000000000003"}));
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, incomplete_batch).0,
        409
    );
    assert!(state.lock().unwrap().snapshots.is_empty());
    assert!(state.lock().unwrap().snapshot_capture_bindings.is_empty());
    assert_eq!(
        snapshot_remote_call(&state, "GET", &format!("snapshots/{id}"), JsonValue::Null).0,
        404
    );

    // Old ordinary content remains readable but cannot gain present-day ownership.
    missing["contract"] = JsonValue::Null;
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, missing).0,
        200
    );
    assert_eq!(
        snapshot_remote_call(&state, "POST", SNAPSHOT_COMMIT_ROUTE, request).0,
        409
    );
    let unbound = snapshot_remote_call(&state, "GET", &format!("snapshots/{id}"), JsonValue::Null);
    assert_eq!(unbound.0, 200);
    assert!(unbound.1["capture_binding"].is_null());
}

#[test]
fn markdown_link_checker_detects_missing_local_links() {
    let temp = TempDir::new().unwrap();
    let docs = temp.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("source.md"), "[missing](./missing.md)\n").unwrap();

    let issues = find_broken_links(temp.path()).unwrap();

    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, PathBuf::from("docs/source.md"));
    assert_eq!(issues[0].line_number, 1);
}

#[cfg(unix)]
#[test]
fn markdown_link_checker_does_not_follow_directory_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let workspace = temp.path().join("workspace");
    let docs = workspace.join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("source.md"), "[missing](./missing.md)\n").unwrap();

    let external_docs = temp.path().join("external-docs");
    fs::create_dir_all(&external_docs).unwrap();
    fs::write(
        external_docs.join("external.md"),
        "[external missing](./missing.md)\n",
    )
    .unwrap();
    symlink(&external_docs, workspace.join("linked-docs")).unwrap();

    let expected = workspace.canonicalize().unwrap().join("docs/source.md");
    assert_eq!(iter_markdown_files(&workspace).unwrap(), vec![expected]);
    let issues = find_broken_links(&workspace).unwrap();
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].path, PathBuf::from("docs/source.md"));
}

#[test]
fn markdown_link_checker_ignores_external_anchors_and_fences() {
    let temp = TempDir::new().unwrap();
    let docs = temp.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("target.md"), "# Target\n").unwrap();
    fs::write(
        docs.join("source.md"),
        "[target](./target.md)\n[anchor](#local-anchor)\n[external](https://example.com/path)\n```\n[fenced](./missing.md)\n```\n",
    )
    .unwrap();

    assert!(find_broken_links(temp.path()).unwrap().is_empty());
}

#[test]
fn markdown_link_checker_skips_missing_sprint_targets_when_surface_is_absent() {
    let temp = TempDir::new().unwrap();
    let docs = temp.path().join("docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(docs.join("index.md"), "[card](./sprints/card.md)\n").unwrap();

    assert!(find_broken_links(temp.path()).unwrap().is_empty());
}

#[test]
fn markdown_link_checker_uses_source_root_targets_when_worktree_omits_authored_root_files() {
    let temp = TempDir::new().unwrap();
    let source_root = temp.path().join("source");
    fs::create_dir_all(source_root.join("release")).unwrap();
    fs::write(source_root.join("README.md"), "# Root README\n").unwrap();
    fs::write(
        source_root.join("release/HOMEBREW_TAP.md"),
        "# Homebrew tap\n",
    )
    .unwrap();

    let worktree_release = temp.path().join("release/guides");
    fs::create_dir_all(&worktree_release).unwrap();
    fs::write(
        temp.path().join(".ait-worktree.json"),
        format!(
            "{{\"repo_root\":\"{}\"}}\n",
            source_root.display().to_string().replace('\\', "\\\\")
        ),
    )
    .unwrap();
    fs::write(
        worktree_release.join("LOCAL_QUICKSTART.md"),
        "[homebrew](../HOMEBREW_TAP.md)\n[readme](../../README.md)\n",
    )
    .unwrap();

    assert!(find_broken_links(temp.path()).unwrap().is_empty());
}

#[test]
fn markdown_link_checker_rebases_sibling_repo_links_against_source_root() {
    let temp = TempDir::new().unwrap();
    let source_root = temp.path().join("source");
    fs::create_dir_all(source_root.join("../ait-core")).unwrap();
    fs::create_dir_all(source_root.join("docs/rust")).unwrap();
    fs::write(
        source_root.join("docs/rust/plan.md"),
        "[kernel](../../../ait-core/kernel.rs)\n",
    )
    .unwrap();
    fs::write(
        source_root.join("../ait-core/kernel.rs"),
        "pub fn ok() {}\n",
    )
    .unwrap();

    let worktree_docs = temp.path().join("docs/rust");
    fs::create_dir_all(&worktree_docs).unwrap();
    fs::write(
        temp.path().join(".ait-worktree.json"),
        format!("{{\"repo_root\":\"{}\"}}\n", source_root.display()),
    )
    .unwrap();
    fs::write(
        worktree_docs.join("plan.md"),
        "[kernel](../../../ait-core/kernel.rs)\n",
    )
    .unwrap();

    assert!(find_broken_links(temp.path()).unwrap().is_empty());
}
