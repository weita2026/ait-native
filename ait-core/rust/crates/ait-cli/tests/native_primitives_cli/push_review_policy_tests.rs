fn set_active_root_worktree(root: &Path, worktree_name: &str) {
    let config_path = root.join(".ait/config.json");
    let mut config = parse_json_file(&config_path);
    config["worktree_name"] = JsonValue::String(worktree_name.to_string());
    write_file(&config_path, &(encode_json_pretty(&config) + "\n"));
}

fn prepare_root_publication_candidate(root: &Path, message: &str) -> String {
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"publication candidate\" }\n",
    );
    seed_snapshot(root, message)
}

#[test]
fn native_workflow_task_input_preserves_remote_scope_and_exact_patchset_publication() {
    for default_scope in ["remote", "local"] {
        let (base_url, log, state, handle) = spawn_fake_remote();
        let temp = init_repo(&base_url);
        let root = temp.path();
        if default_scope == "local" {
            let path = root.join(".ait/config.json");
            let mut config = parse_json_file(&path);
            config["workflow_default_scope"] = json!("local");
            config["task_default_scope"] = json!("local");
            write_file(&path, &encode_json_pretty(&config));
        }
        {
            let mut guard = state.lock().unwrap();
            guard.remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
            guard.force_no_selected_patchset = true;
            guard.change_rows_override = Some(Vec::new());
        }
        let worktree = register_matching_target_worktree(root);
        let revision =
            prepare_worktree_publication_candidate(&worktree, "Task publication checkpoint");
        use_captured_task_publication_fixture(&state, &revision);
        let preview_start = log.lock().unwrap().len();
        for phase in ["ready", "finish"] {
            let preview = cargo_bin()
                .current_dir(root)
                .args(["workflow", phase, "RT-1", "--remote", "origin"])
                .output()
                .unwrap();
            assert!(
                preview.status.success(),
                "{phase}: {}",
                String::from_utf8_lossy(&preview.stderr)
            );
            let text = String::from_utf8_lossy(&preview.stdout);
            assert!(text.contains("RT-1"), "{text}");
            assert!(!text.contains("RC-1"), "{text}");
        }
        assert!(log.lock().unwrap()[preview_start..]
            .iter()
            .all(|row| row.method == "GET"));
        let output = cargo_bin()
            .current_dir(root)
            .args([
                "workflow",
                "ready",
                "RT-1",
                "--remote",
                "origin",
                "--apply",
                "--summary",
                "Task publication",
                "--tests",
                "pass",
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        handle.join().unwrap();
        let logged = log.lock().unwrap();
        let published = logged
            .iter()
            .filter(|row| {
                row.method == "POST"
                    && row.url == "/v1/native/repository-authorities/7/changes/RT-1/patchsets"
            })
            .collect::<Vec<_>>();
        assert_eq!(published.len(), 1);
        let body = parse_json(&published[0].body);
        assert_eq!(body["base_snapshot_id"], FIXTURE_BASE_SNAPSHOT_ID);
        assert_eq!(body["revision_snapshot_id"], revision);
        assert!(!logged.iter().any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes"));
    }
}

#[test]
fn native_workflow_task_input_keeps_exact_candidate_despite_sibling_changes() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let mut rows = (2..=31)
        .map(|n| json!({"task_id": format!("RT-{n}"), "change_id": "C-01", "status": "active"}))
        .collect::<Vec<_>>();
    rows.extend([
        json!({"task_id": "RT-1", "change_id": "C-01", "status": "landed"}),
        json!({"task_id": "RT-1", "change_id": "C-02", "status": "active"}),
        json!({"task_id": "RT-1", "change_id": "C-03", "status": "superseded"}),
        json!({"task_id": "RT-1", "change_id": "C-04", "status": "active"}),
    ]);
    {
        let mut state = state.lock().unwrap();
        state.change_rows_override = Some(rows);
        state.selected_patchset_id = Some("RT-1/P-01".into());
        state.selected_patchset_revision_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.into());
    }
    let before = json_output(temp.path(), &["snapshot", "list", "--all", "--json"]);
    for phase in ["ready", "finish"] {
        let result = json_output(
            temp.path(),
            &["workflow", phase, "RT-1", "--json", "--full"],
        );
        assert_eq!(result["task"]["task_id"], "RT-1");
        assert_eq!(result["patchset"]["patchset_id"], "RT-1/P-01");
        assert!(result["change"].is_null());
    }
    assert_eq!(
        json_output(temp.path(), &["snapshot", "list", "--all", "--json"]),
        before
    );
    handle.join().unwrap();
    assert!(log.lock().unwrap().iter().all(|row| row.method == "GET"));
}

#[test]
fn native_remote_task_snapshot_keeps_exact_binding_and_rejects_completed_task_before_capture() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    state.lock().unwrap().change_rows_override = Some(Vec::new());
    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "RT-1",
            "-m",
            "Remote Task checkpoint",
            "--json",
        ],
    );
    assert_eq!(snapshot["task_id"], "RT-1");
    assert!(snapshot["change_id"].is_null());
    assert_eq!(snapshot["message"], "Remote Task checkpoint");
    let id = snapshot["snapshot_id"].as_str().unwrap();
    {
        let guard = state.lock().unwrap();
        let persisted = &guard.committed_snapshots[id]["capture_binding"];
        assert_eq!(persisted, &snapshot["capture_binding"]);
        assert_eq!(persisted["task_id"], "RT-1");
        assert!(persisted["change_ref"].is_null());
    }
    let before = json_output(temp.path(), &["snapshot", "list", "--all", "--json"]);
    let before_request_count = log.lock().unwrap().len();
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn remains_pending() {}\n",
    );
    state.lock().unwrap().task_completed = true;
    let rejected = command_output_with_env(
        &worktree,
        &[
            "snapshot",
            "create",
            "RT-1",
            "-m",
            "Completed Task cannot capture",
        ],
        &[],
    );
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("exact active Task"));
    assert_eq!(
        json_output(temp.path(), &["snapshot", "list", "--all", "--json"]),
        before
    );
    handle.join().unwrap();
    let requests = log.lock().unwrap();
    assert!(requests[before_request_count..]
        .iter()
        .all(|row| row.method == "GET"));
    assert!(!requests.iter().any(
        |row| row.url == "/v1/native/repository-authorities/7/changes" && row.method == "POST"
    ));
}

fn prepare_worktree_publication_candidate(worktree: &Path, message: &str) -> String {
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"worktree publication candidate\" }\n",
    );
    checkpoint_bound_worktree_fixture(worktree, message)["snapshot_id"].as_str().unwrap().to_string()
}

fn register_publication_bound_worktree(
    root: &Path,
    name: &str,
    line_name: &str,
    task_id: &str,
    change_id: &str,
    materialize: bool,
) -> PathBuf {
    let physical_root = root.canonicalize().unwrap();
    let root = physical_root.as_path();
    let config = parse_json_file(root.join(".ait/config.json"));
    let capture_url = config["remotes"]["origin"]["url"].as_str().unwrap().trim_end_matches('/');
    let external_path = root.join(format!("{name}-external"));
    if materialize {
        fs::create_dir_all(external_path.join("src")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.join(".ait"), external_path.join(".ait")).unwrap();
        write_file(
            &external_path.join(".ait-worktree.json"),
            &(encode_json_pretty(&json!({
                "current_line": line_name,
                "repo_root": root.display().to_string(),
                "workspace_root": external_path.display().to_string(),
                "worktree_name": name,
            })) + "\n"),
        );
        write_file(
            &external_path.join("src/lib.rs"),
            "pub fn example() -> &'static str { \"worktree baseline\" }\n",
        );
    }
    seed_binary_line(root, line_name, FIXTURE_BASE_SNAPSHOT_ID);
    write_file(
        &root.join(".ait/worktrees").join(format!("{name}.json")),
        &(encode_json_pretty(&json!({
            "name": name,
            "path": external_path.display().to_string(),
            "repo_root": root.display().to_string(),
            "line_name": line_name,
            "bound_task_id": task_id,
            "bound_change_id": (!change_id.is_empty()).then_some(change_id),
            "capture_remote_name":"origin","capture_remote_url":capture_url,"capture_repository_index":FIXTURE_REPOSITORY_INDEX,
            "auto_created_for_task": true,
            "fork_snapshot_id": FIXTURE_BASE_SNAPSHOT_ID,
            "forked_from_line": "main",
            "target_base_line": "main",
            "rebase_state": "idle",
            "rebase_conflict_paths": [],
            "created_at": "2026-06-08T00:00:00Z",
            "cleanup_policy": "after_remote_land",
        })) + "\n"),
    );
    external_path
}

fn use_captured_task_publication_fixture(state: &Arc<Mutex<FakeRemoteState>>, snapshot_id: &str) {
    let mut state = state.lock().unwrap();
    let capture = state.committed_snapshots.get(snapshot_id).expect("actual durable capture request").clone();
    assert_eq!(capture["capture_binding"]["task_id"], "RT-1");
    assert!(capture["capture_binding"]["change_ref"].is_null());
    state.durable_task = Some(DurableTaskFixture {capture, ..Default::default()});
}

fn register_unrelated_active_worktree(root: &Path) {
    register_publication_bound_worktree(
        root,
        "rt-other",
        "feature/rt-other",
        "RT-OTHER",
        "RC-OTHER",
        false,
    );
    set_active_root_worktree(root, "rt-other");
}

fn register_matching_target_worktree(root: &Path) -> PathBuf {
    register_publication_bound_worktree(root, "rt-1", "feature/rt-1", "RT-1", "", true)
}

#[test]
fn native_patchset_publish_rejects_unowned_root_candidate_with_unrelated_active_worktree() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    prepare_root_publication_candidate(root, "unrelated direct publication pin");
    register_unrelated_active_worktree(root);

    let output = command_output_with_env(root, &["patchset", "publish", "RT-1", "--summary", "Reject unowned root candidate", "--json"], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Publication requires unique numeric capture ownership"));
    handle.join().unwrap();
    assert!(log.lock().unwrap().iter().all(|row| row.method == "GET"));
}

#[test]
fn native_patchset_publish_from_root_routes_matching_not_active_unrelated_worktree() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    state.lock().unwrap().change_rows_override = Some(Vec::new());
    let matching_worktree = register_matching_target_worktree(root);
    let revision_snapshot_id = prepare_worktree_publication_candidate(
        &matching_worktree,
        "matching direct publication pin",
    );
    use_captured_task_publication_fixture(&state, &revision_snapshot_id);
    register_unrelated_active_worktree(root);

    let patchset = json_output(
        root,
        &[
            "patchset",
            "publish",
            "RT-1",
            "--summary",
            "Require matching worktree",
            "--json",
        ],
    );

    assert_eq!(
        patchset["revision_snapshot_id"].as_str(),
        Some(revision_snapshot_id.as_str())
    );
    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| {
        row.method == "POST"
            && row.url == "/v1/native/repository-authorities/7/changes/RT-1/patchsets"
    }));
}

#[test]
fn native_workflow_ready_rejects_unowned_root_candidate_with_unrelated_active_worktree() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.force_no_selected_patchset = true;
    }
    prepare_root_publication_candidate(root, "unrelated workflow publication pin");
    register_unrelated_active_worktree(root);

    let output = cargo_bin()
        .current_dir(root)
        .args([
            "workflow",
            "ready",
            "RT-1",
            "--apply",
            "--summary",
            "Ignore unrelated workflow worktree",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Publication requires unique numeric capture ownership"));
    handle.join().unwrap();
    assert!(log.lock().unwrap().iter().all(|row| row.method == "GET"));
}

#[test]
fn native_workflow_ready_apply_from_root_routes_matching_not_active_unrelated_worktree() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.force_no_selected_patchset = true;
    }
    state.lock().unwrap().change_rows_override = Some(Vec::new());
    let matching_worktree = register_matching_target_worktree(root);
    let revision = prepare_worktree_publication_candidate(&matching_worktree, "matching workflow publication pin");
    use_captured_task_publication_fixture(&state, &revision);
    register_unrelated_active_worktree(root);

    let output = cargo_bin()
        .current_dir(root)
        .args([
            "workflow",
            "ready",
            "RT-1",
            "--apply",
            "--summary",
            "Require matching workflow worktree",
            "--tests", "pass",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| {
        row.method == "POST"
            && row.url == "/v1/native/repository-authorities/7/changes/RT-1/patchsets"
    }));
}

#[test]
fn native_patchset_publish_rejects_bound_worktree_retarget_requirement() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let snapshot = checkpoint_bound_worktree_fixture(&worktree, "retarget candidate");
    assert!(snapshot["snapshot_id"]
        .as_str()
        .unwrap()
        .starts_with("SNP-"));
    write_file(
        &root.join(".ait/worktrees/rt-1.json"),
        &format!(
            concat!(
                "{{\n",
                "  \"name\": \"rt-1\",\n",
                "  \"path\": \"{}\",\n",
                "  \"repo_root\": \"{}\",\n",
                "  \"line_name\": \"feature/rt-1\",\n",
                "  \"bound_task_id\": \"RT-1\",\n",
                "  \"bound_change_id\": \"RC-1\",\n",
                "  \"auto_created_for_task\": true,\n",
                "  \"fork_snapshot_id\": \"SNP-OLD\",\n",
                "  \"forked_from_line\": \"main\",\n",
                "  \"target_base_line\": \"main\",\n",
                "  \"rebase_state\": \"idle\",\n",
                "  \"rebase_conflict_paths\": []\n",
                "}}\n"
            ),
            worktree.display(),
            root.display(),
        ),
    );

    let output = cargo_bin()
        .current_dir(&worktree)
        .args([
            "patchset",
            "publish",
            "RT-1",
            "--summary",
            "retarget check",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&format!(
        "Current worktree is still based on `SNP-OLD` while `main` moved to `{FIXTURE_BASE_SNAPSHOT_ID}`"
    )));
    assert!(stderr.contains("ait worktree rebase --onto main"));

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(!logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets"));
}

#[test]
fn native_patchset_publish_uses_unchanged_remote_base_when_local_main_is_ahead() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    {
        let mut state = state.lock().unwrap();
        state.change_rows_override = Some(Vec::new());
        state.force_no_selected_patchset = true;
    }
    let feature_snapshot = checkpoint_bound_worktree_fixture(&worktree, "direct remote candidate");
    let feature_snapshot_id = feature_snapshot["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    use_captured_task_publication_fixture(&state, &feature_snapshot_id);

    write_file(
        &root.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"local only\" }\n",
    );
    let local_main_snapshot_id = seed_snapshot(root, "advance local main only");
    assert_ne!(local_main_snapshot_id, feature_snapshot_id);

    let patchset = json_output(
        &worktree,
        &[
            "patchset",
            "publish",
            "RT-1",
            "--summary",
            "Keep direct Remote ancestry",
            "--json",
        ],
    );
    assert_eq!(
        patchset["base_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        patchset["revision_snapshot_id"].as_str(),
        Some(feature_snapshot_id.as_str())
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/changes/RT-1/patchsets"));
}

#[test]
fn native_workflow_ready_retarget_uses_executing_worktree_not_root_binding() {
    let (base_url, _log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    {
        let mut state = state.lock().unwrap();
        state.selected_patchset_id = Some("RT-1/P-01".into());
        state.selected_patchset_revision_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.into());
    }

    write_file(
        &root.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"advanced\" }\n",
    );
    let advanced_main_snapshot_id = seed_snapshot(root, "advance main for retarget");
    state.lock().unwrap().remote_head_snapshot_id = Some(advanced_main_snapshot_id.clone());

    init_registered_worktree(
        root,
        "rt-other",
        "feature/rt-other",
        Some("RT-OTHER"),
        Some("RC-OTHER"),
        true,
        None,
    );
    let other_metadata_path = root.join(".ait/worktrees/rt-other.json");
    let mut other_metadata: JsonValue = parse_json_file(&other_metadata_path);
    other_metadata["fork_snapshot_id"] = JsonValue::String(advanced_main_snapshot_id.clone());
    write_file(
        &other_metadata_path,
        &(encode_json_pretty(&other_metadata) + "\n"),
    );

    let root_config_path = root.join(".ait/config.json");
    let mut root_config: JsonValue = parse_json_file(&root_config_path);
    root_config["worktree_name"] = JsonValue::String("rt-other".to_string());
    write_file(
        &root_config_path,
        &(encode_json_pretty(&root_config) + "\n"),
    );

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = workflow_ready_payload(&repo, "RT-1", None).unwrap();
    let retarget = &payload["worktree_retarget"];

    assert_eq!(
        retarget["line_name"].as_str(),
        Some("feature/rt-1"),
        "{}",
        encode_json_pretty(&payload)
    );
    assert_eq!(
        retarget["fork_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        retarget["target_base_snapshot_id"].as_str(),
        Some(advanced_main_snapshot_id.as_str())
    );
    assert_eq!(retarget["needs_retarget"].as_bool(), Some(true));

    handle.join().unwrap();
}

#[test]
fn native_workflow_ready_uses_remote_base_when_only_local_main_advanced() {
    let (base_url, _log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    {
        let mut state = state.lock().unwrap();
        state.selected_patchset_id = Some("RT-1/P-01".into());
        state.selected_patchset_revision_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.into());
    }
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    write_file(
        &root.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"local only\" }\n",
    );
    let advanced_local_main_snapshot_id = seed_snapshot(root, "advance local main only");

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = workflow_ready_payload(&repo, "RT-1", None).unwrap();
    let retarget = &payload["worktree_retarget"];

    assert_ne!(
        retarget["target_base_snapshot_id"].as_str(),
        Some(advanced_local_main_snapshot_id.as_str())
    );
    assert_eq!(
        retarget["target_base_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        retarget["fork_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(retarget["needs_retarget"].as_bool(), Some(false));

    handle.join().unwrap();
}

#[test]
fn native_workflow_ready_apply_uses_remote_base_when_only_local_main_advanced() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.force_no_selected_patchset = true;
    }
    {
        let mut state = state.lock().unwrap();
        state.change_rows_override = Some(Vec::new());
        state.force_no_selected_patchset = true;
    }
    let feature_snapshot =
        checkpoint_bound_worktree_fixture(&worktree, "remote workflow candidate");
    let feature_snapshot_id = feature_snapshot["snapshot_id"]
        .as_str()
        .expect("feature snapshot id")
        .to_string();
    use_captured_task_publication_fixture(&state, &feature_snapshot_id);

    write_file(
        &root.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"local only\" }\n",
    );
    let advanced_local_main_snapshot_id = seed_snapshot(root, "advance local main only");

    let output = cargo_bin()
        .current_dir(&worktree)
        .args([
            "workflow",
            "ready",
            "RT-1",
            "--apply",
            "--summary",
            "Use authoritative Remote base",
            "--tests",
            "pass",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        local_line_head(root, "main").as_deref(),
        Some(advanced_local_main_snapshot_id.as_str())
    );
    assert_eq!(
        local_line_head(&worktree, "feature/rt-1").as_deref(),
        Some(feature_snapshot_id.as_str())
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let published = logged
        .iter()
        .find(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/changes/RT-1/patchsets"
        })
        .unwrap_or_else(|| panic!("missing Patchset publication: {logged:#?}"));
    let body = parse_json(&published.body);
    assert_eq!(
        body["base_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        body["revision_snapshot_id"].as_str(),
        Some(feature_snapshot_id.as_str())
    );
}

#[test]
fn native_worktree_rebase_recovers_stale_registry_fork_from_bound_change() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();

    let feature_snapshot_id = seed_snapshot(&worktree, "feature work before retarget");
    write_file(
        &root.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"advanced\" }\n",
    );
    let advanced_main_snapshot_id = seed_snapshot(root, "advance main for retarget");

    let metadata_path = root.join(".ait/worktrees/rt-1.json");
    let mut metadata: JsonValue = parse_json_file(&metadata_path);
    metadata["fork_snapshot_id"] = JsonValue::String(advanced_main_snapshot_id.clone());
    metadata["last_retargeted_at"] = JsonValue::String("2026-07-15T00:00:00Z".to_string());
    write_file(&metadata_path, &(encode_json_pretty(&metadata) + "\n"));

    let payload = json_output(&worktree, &["worktree", "rebase", "--dry-run", "--json"]);
    let rebase = &payload["rebase"];
    assert_eq!(
        rebase["old_base_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        rebase["old_head_snapshot_id"].as_str(),
        Some(feature_snapshot_id.as_str())
    );
    assert_eq!(
        rebase["new_base_snapshot_id"].as_str(),
        Some(advanced_main_snapshot_id.as_str())
    );

    handle.join().unwrap();
}

#[test]
fn native_worktree_rebase_recovers_stale_registry_fork_without_local_change_row() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();

    let feature_snapshot_id = seed_snapshot(&worktree, "feature work before remote-only retarget");
    write_file(
        &root.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"advanced remote only\" }\n",
    );
    let advanced_main_snapshot_id = seed_snapshot(root, "advance main for remote-only retarget");

    let metadata_path = root.join(".ait/worktrees/rt-1.json");
    let mut metadata: JsonValue = parse_json_file(&metadata_path);
    metadata["bound_change_id"] = JsonValue::String("RC-REMOTE-ONLY".to_string());
    metadata["fork_snapshot_id"] = JsonValue::String(advanced_main_snapshot_id.clone());
    write_file(&metadata_path, &(encode_json_pretty(&metadata) + "\n"));

    let payload = json_output(&worktree, &["worktree", "rebase", "--dry-run", "--json"]);
    let rebase = &payload["rebase"];
    assert_eq!(
        rebase["old_base_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        rebase["old_head_snapshot_id"].as_str(),
        Some(feature_snapshot_id.as_str())
    );
    assert_eq!(
        rebase["new_base_snapshot_id"].as_str(),
        Some(advanced_main_snapshot_id.as_str())
    );

    handle.join().unwrap();
}

#[test]
fn native_task_capture_uses_one_zstd_plan_and_publication_reuses_acknowledged_suffix() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.change_rows_override = Some(Vec::new());
        guard.force_no_selected_patchset = true;
    }
    let worktree = register_matching_target_worktree(temp.path());
    let snapshot_id = prepare_worktree_publication_candidate(&worktree, "Task suffix capture");
    use_captured_task_publication_fixture(&state, &snapshot_id);
    let capture_end = log.lock().unwrap().len();

    let patchset = json_output(
        &worktree,
        &[
            "patchset",
            "publish",
            "RT-1",
            "--summary",
            "Reuse captured suffix",
            "--json",
        ],
    );
    assert_eq!(patchset["revision_snapshot_id"], snapshot_id);
    assert_eq!(patchset["base_snapshot_id"], FIXTURE_BASE_SNAPSHOT_ID);
    assert!(patchset["change_id"].is_null());
    assert_eq!(patchset["snapshot_sync"]["checked_snapshots"], 1);
    assert_eq!(patchset["snapshot_sync"]["uploaded_snapshots"], 0);
    assert_eq!(patchset["snapshot_sync"]["skipped_snapshots"], 1);
    assert_eq!(patchset["snapshot_sync"]["sync_scope"], "line_only");

    handle.join().unwrap();
    let logged = log.lock().unwrap();
    let captures = &logged[..capture_end];
    let plans = captures
        .iter()
        .filter(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/plan"
        })
        .collect::<Vec<_>>();
    assert_eq!(plans.len(), 1);
    let plan = parse_json(&plans[0].body);
    assert_eq!(
        plan["snapshot_ids"],
        json!([FIXTURE_BASE_SNAPSHOT_ID, snapshot_id])
    );
    for kind in ["object-packs/", "tree-packs/"] {
        assert!(captures.iter().any(|row| row.method == "PUT"
            && row.url.starts_with(&format!(
                "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/{kind}"
            ))));
    }
    let commits = captures
        .iter()
        .filter(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
        })
        .collect::<Vec<_>>();
    assert_eq!(commits.len(), 1);
    let commit = parse_json(&commits[0].body);
    assert_eq!(commit["snapshots"].as_array().unwrap().len(), 1);
    assert_eq!(commit["snapshots"][0]["snapshot_id"], snapshot_id);
    assert_eq!(commit["capture_binding"]["task_id"], "RT-1");
    assert_eq!(commit["capture_binding"]["snapshot_id"], snapshot_id);
    assert!(commit["capture_binding"]["change_ref"].is_null());
    assert!(!logged.iter().any(|row| row.method == "PUT"
        && row
            .url
            .starts_with("/v1/native/repository-authorities/7/snapshots/")));
    let publication = &logged[capture_end..];
    assert!(!publication
        .iter()
        .any(|row| row.url.contains("/zstd-bulk/")));
    assert!(publication.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/snapshots:exists"));
    assert_eq!(
        publication
            .iter()
            .filter(|row| row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/changes/RT-1/patchsets")
            .count(),
        1
    );
}

#[test]
fn native_push_uploads_missing_suffix_and_updates_remote_line() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"pushed\" }\n",
    );
    let snapshot_id = seed_snapshot(root, "push suffix snapshot");
seed_binary_line(root, "feature/rt-1", &snapshot_id);
state.lock().unwrap().selected_patchset_revision_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let pushed = json_output(root, &["push", "--line", "feature/rt-1", "--json"]);
    assert_eq!(pushed["remote"].as_str(), Some("origin"));
    assert_eq!(pushed["repo_name"].as_str(), Some("fixture-ait"));
    assert_eq!(pushed["line"].as_str(), Some("feature/rt-1"));
    assert_eq!(pushed["checked_snapshots"].as_i64(), Some(1));
    assert_eq!(pushed["uploaded_snapshots"].as_i64(), Some(1));
    assert_eq!(pushed["skipped_snapshots"].as_i64(), Some(0));
    assert_eq!(
        pushed["head_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(pushed["sync_scope"].as_str(), Some("bounded_suffix"));
    assert_eq!(
        pushed["sync_reason"].as_str(),
        Some("remote_head_is_local_ancestor")
    );
    assert_eq!(
        pushed["bounded_by_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged
        .iter()
        .any(|row| row.method == "GET" && row.url == "/v1/native/repository-authorities/7"));
    let exists_request = logged
        .iter()
        .find(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/snapshots:exists"
        })
        .expect("expected snapshots:exists request");
    let exists_payload: JsonValue = parse_json(&exists_request.body);
    let snapshot_ids = exists_payload["snapshot_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(snapshot_ids, vec![snapshot_id.clone()]);
    assert_eq!(
        logged
            .iter()
            .filter(|row| {
                row.method == "POST"
                    && row.url
                        == "/v1/native/repository-authorities/7/snapshots:exists"
            })
            .count(),
        1,
        "zstd planning must reuse its own presence result instead of issuing a second existence round trip"
    );

    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/plan"));
    assert!(logged.iter().any(|row| row.method == "PUT"
        && row.url.starts_with(
            "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/object-packs/"
        )));
    assert!(logged.iter().any(|row| row.method == "PUT"
        && row
            .url
            .starts_with("/v1/native/repository-authorities/7/remote-sync/zstd-bulk/tree-packs/")));
    let commit_request = logged
        .iter()
        .find(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
        })
        .expect("expected zstd bulk commit");
    let commit_payload: JsonValue = parse_json(&commit_request.body);
    assert_eq!(commit_payload["line_update"]["line_name"], "feature/rt-1");
    assert!(commit_payload["snapshots"]
        .as_array()
        .expect("zstd commit snapshots")
        .iter()
        .any(|snapshot| snapshot["snapshot_id"].as_str() == Some(snapshot_id.as_str())));
    assert_eq!(
        commit_payload["line_update"]["head_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(
        commit_payload["line_update"]["expected_head_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert!(!logged.iter().any(|row| {
        row.method == "PUT"
            && row
                .url
                .starts_with("/v1/native/repository-authorities/7/snapshots/")
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "PUT" && row.url == "/v1/native/repository-authorities/7/lines/main"
    }));
    assert_eq!(
        state.lock().unwrap().remote_head_snapshot_id.as_deref(),
        Some(snapshot_id.as_str())
    );
}

#[test]
fn native_push_json_ignores_the_retired_debug_environment() {
    let (base_url, _log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"normal-timing\" }\n",
    );
    let snapshot_id = seed_snapshot(root, "normal timing push");
seed_binary_line(root, "feature/rt-1", &snapshot_id);
state.lock().unwrap().selected_patchset_revision_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    let normal = json_output_with_env(
        root,
        &["push", "--line", "feature/rt-1", "--json"],
        &[(concat!("AIT_JSON_", "MODE"), "debug")],
    );
    assert!(normal.get("phase_timings_ms").is_none());
    assert!(normal.get("remote_sync_metrics").is_none());
    handle.join().unwrap();
}

#[cfg(feature = "perfetto-tracing")]
#[test]
fn native_push_perfetto_trace_names_cover_frontier_pack_pipeline_and_commit() {
    let (base_url, _log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"push-perfetto\" }\n",
    );
    let snapshot_id = seed_snapshot(root, "push Perfetto");
seed_binary_line(root, "feature/rt-1", &snapshot_id);
state.lock().unwrap().selected_patchset_revision_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    let trace_path = root.join(".ait-runtime/push.perfetto.json");
    fs::create_dir_all(trace_path.parent().unwrap()).unwrap();
    let trace_text = trace_path.to_string_lossy().to_string();
    let _ = json_output_with_env(
        root,
        &["push", "--line", "feature/rt-1", "--json"],
        &[("AIT_PERFETTO_TRACE", trace_text.as_str())],
    );
    handle.join().unwrap();
    let trace = parse_json_bytes(&fs::read(&trace_path).expect("push Perfetto trace"));
    let names = trace["traceEvents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["name"].as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "ait.remote_sync.push",
        "ait.remote_sync.push.remote_line_read",
        "ait.remote_sync.push.have_want_frontier",
        "ait.remote_sync.push.head_existence_read",
        "ait.remote_sync.push.zstd_bulk",
        "ait.remote_sync.push.pack_assembly",
        "ait.remote_sync.push.plan_http",
        "ait.remote_sync.push.pack_prepare.object",
        "ait.remote_sync.push.pack_prepare.tree",
        "ait.remote_sync.push.pack_upload_pipeline",
        "ait.remote_sync.push.commit_assembly",
        "ait.remote_sync.push.commit_http",
    ] {
        assert!(
            names.contains(expected),
            "missing Perfetto range {expected}"
        );
    }
}

#[test]
fn native_push_skips_snapshot_check_when_remote_head_matches_local_head() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let pushed = json_output(root, &["push", "--line", "main", "--json"]);
    assert_eq!(pushed["remote"].as_str(), Some("origin"));
    assert_eq!(pushed["repo_name"].as_str(), Some("fixture-ait"));
    assert_eq!(pushed["line"].as_str(), Some("main"));
    assert_eq!(pushed["checked_snapshots"].as_i64(), Some(0));
    assert_eq!(pushed["uploaded_snapshots"].as_i64(), Some(0));
    assert_eq!(pushed["skipped_snapshots"].as_i64(), Some(0));
    assert_eq!(
        pushed["head_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(pushed["sync_scope"].as_str(), Some("bounded_suffix"));
    assert_eq!(
        pushed["sync_reason"].as_str(),
        Some("remote_head_matches_local_head")
    );
    assert_eq!(
        pushed["bounded_by_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(!logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/snapshots:exists"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "PUT"
            && row
                .url
                .starts_with("/v1/native/repository-authorities/7/snapshots/")
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "PUT" && row.url == "/v1/native/repository-authorities/7/lines/main"
    }));
    assert!(!logged.iter().any(|row| {
        row.method == "POST"
            && row.url == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/plan"
    }));
}

#[test]
fn native_push_creates_feature_line_from_present_head_without_zstd_commit() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let created = json_output(
        root,
        &[
            "line",
            "create",
            "feature/rt-1",
            "--from-snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--json",
        ],
    );
    assert_eq!(created["line_name"].as_str(), Some("feature/rt-1"));
    assert_eq!(
        created["head_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );

    let pushed = json_output(root, &["push", "--line", "feature/rt-1", "--json"]);
    assert_eq!(pushed["remote"].as_str(), Some("origin"));
    assert_eq!(pushed["repo_name"].as_str(), Some("fixture-ait"));
    assert_eq!(pushed["line"].as_str(), Some("feature/rt-1"));
    assert_eq!(pushed["checked_snapshots"].as_i64(), Some(1));
    assert_eq!(pushed["uploaded_snapshots"].as_i64(), Some(0));
    assert_eq!(pushed["skipped_snapshots"].as_i64(), Some(1));
    assert_eq!(pushed["pushed_snapshots"].as_i64(), Some(0));
    assert_eq!(pushed["sync_scope"].as_str(), Some("line_only"));
    assert_eq!(
        pushed["sync_reason"].as_str(),
        Some("remote_line_missing_head_snapshot_present")
    );
    assert_eq!(
        pushed["head_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        pushed["remote_line"]["head_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let exists_request = logged
        .iter()
        .find(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/snapshots:exists"
        })
        .expect("expected head snapshot existence check");
    let exists_payload: JsonValue = parse_json(&exists_request.body);
    let snapshot_ids = exists_payload["snapshot_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(snapshot_ids, vec![FIXTURE_BASE_SNAPSHOT_ID.to_string()]);
    assert!(logged.iter().any(|row| {
        row.method == "PUT" && row.url == "/v1/native/repository-authorities/7/lines/feature%2Frt-1"
    }));
    assert!(!logged.iter().any(|row| {
        row.url
            .starts_with("/v1/native/repository-authorities/7/remote-sync/zstd-bulk/")
    }));
}

#[test]
fn native_push_updates_stale_feature_line_from_present_head_without_zstd_commit() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"present remote head\" }\n",
    );
    let feature_snapshot_id = seed_snapshot(root, "feature present remote head");
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(feature_snapshot_id.clone());
        guard.selected_patchset_revision_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    }

    let created = json_output(
        root,
        &[
            "line",
            "create",
            "feature/rt-1",
            "--from-snapshot",
            &feature_snapshot_id,
            "--json",
        ],
    );
    assert_eq!(created["line_name"].as_str(), Some("feature/rt-1"));
    assert_eq!(
        created["head_snapshot_id"].as_str(),
        Some(feature_snapshot_id.as_str())
    );

    let pushed = json_output(root, &["push", "--line", "feature/rt-1", "--json"]);
    assert_eq!(pushed["remote"].as_str(), Some("origin"));
    assert_eq!(pushed["repo_name"].as_str(), Some("fixture-ait"));
    assert_eq!(pushed["line"].as_str(), Some("feature/rt-1"));
    assert_eq!(pushed["checked_snapshots"].as_i64(), Some(1));
    assert_eq!(pushed["uploaded_snapshots"].as_i64(), Some(0));
    assert_eq!(pushed["skipped_snapshots"].as_i64(), Some(1));
    assert_eq!(pushed["pushed_snapshots"].as_i64(), Some(0));
    assert_eq!(pushed["sync_scope"].as_str(), Some("line_only"));
    assert_eq!(
        pushed["sync_reason"].as_str(),
        Some("remote_line_stale_head_snapshot_present")
    );
    assert_eq!(
        pushed["remote_head_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        pushed["head_snapshot_id"].as_str(),
        Some(feature_snapshot_id.as_str())
    );
    assert_eq!(
        pushed["remote_line"]["head_snapshot_id"].as_str(),
        Some(feature_snapshot_id.as_str())
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    let exists_request = logged
        .iter()
        .find(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/snapshots:exists"
        })
        .expect("expected head snapshot existence check");
    let exists_payload: JsonValue = parse_json(&exists_request.body);
    let snapshot_ids = exists_payload["snapshot_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(snapshot_ids, vec![feature_snapshot_id.clone()]);
    let line_request = logged
        .iter()
        .find(|row| {
            row.method == "PUT"
                && row.url == "/v1/native/repository-authorities/7/lines/feature%2Frt-1"
        })
        .expect("expected stale remote line CAS update");
    let line_payload: JsonValue = parse_json(&line_request.body);
    assert_eq!(
        line_payload["head_snapshot_id"].as_str(),
        Some(feature_snapshot_id.as_str())
    );
    assert_eq!(
        line_payload["expected_head_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert!(!logged.iter().any(|row| {
        row.url
            .starts_with("/v1/native/repository-authorities/7/remote-sync/zstd-bulk/")
    }));
}

#[test]
fn native_review_team_approve_recovers_change_lookup_via_repo_listing() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    {
        let mut state = state.lock().unwrap();
        state.fail_current_change_read = true;
        state.selected_patchset_id = Some("RT-1/C-01/P-01".into());
        state.change_rows_override = Some(vec![
            json!({"task_id":"RT-1","change_id":"C-01","change_ref":"RT-1/C-01","status":"active","base_line":"main"}),
        ]);
    }
    let temp = init_repo(&base_url);
    let root = temp.path();

    let approve = json_output(
        root,
        &[
            "review",
            "team",
            "approve",
            "RT-1/C-01",
            "--patchset",
            "RT-1/C-01/P-01",
            "--json",
        ],
    );

    assert_eq!(approve["action"].as_str(), Some("approve"));
    assert_eq!(approve["change_id"].as_str(), Some("C-01"));
    handle.join().unwrap();
    let requests = log.lock().unwrap();
    assert!(requests.iter().any(|row| row.method == "GET"
        && row.url == "/v1/native/repository-authorities/7/changes/RT-1%2FC-01"));
    assert!(
        requests
            .iter()
            .any(|row| row.method == "GET"
                && row.url == "/v1/native/repository-authorities/7/changes")
    );
}

#[test]
fn native_review_namespace_supports_distinct_code_task_team_and_template_lanes() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().selected_patchset_id = Some("RT-1/P-01".into());

    let request = json_output(
        root,
        &[
            "review",
            "team",
            "request",
            "RT-1",
            "--group",
            "core",
            "--patchset",
            "RT-1/P-01",
            "--json",
        ],
    );
    assert_eq!(request["status"].as_str(), Some("requested"));
    assert_eq!(request["requested_groups"][0].as_str(), Some("core"));

    let comment = json_output(
        root,
        &[
            "review",
            "task",
            "comment",
            "RT-1",
            "--patchset",
            "RT-1/P-01",
            "--message",
            "looks fine",
            "--json",
        ],
    );
    assert_eq!(comment["action"].as_str(), Some("task_comment"));

    let code_review = json_output(
        root,
        &[
            "review",
            "code",
            "submit",
            "RT-1",
            "--reviewer", "Einstein",
            "--patchset",
            "RT-1/P-01",
            "--message",
            "Reviewed files: src/lib.rs; Findings: none after inspection; Risks: low; Tests: cargo test; Recommendation: pass",
            "--json",
        ],
    );
    assert_eq!(code_review["action"].as_str(), Some("code_review_summary"));
    assert_eq!(code_review["reviewer"].as_str(), Some("Einstein"));
    assert_eq!(code_review["reviewer_kind"].as_str(), Some("bot"));
    assert_eq!(
        code_review["task_review"]["mode"].as_str(),
        Some("automatic")
    );
    assert_eq!(
        code_review["task_review"]["reviewer"].as_str(),
        Some("Einstein")
    );
    assert_eq!(
        code_review["task_review"]["policy_refresh"]["decision"].as_str(),
        Some("pending")
    );

    let show = json_output(root, &["review", "show", "RT-1", "--json"]);
    assert_eq!(show["current_patchset_id"].as_str(), Some("RT-1/P-01"));
    assert_eq!(show["approvals"].as_i64(), Some(1));
    assert_eq!(
        show["review_requests"],
        json!([{"patchset_id": "RT-1/P-01", "reviewer_group": "core"}])
    );
    assert!(show.get("reviews").is_none());

    let show_with_retired_environment = json_output_with_env(
        root,
        &["review", "show", "RT-1", "--json"],
        &[(concat!("AIT_JSON_", "MODE"), "debug")],
    );
    assert_eq!(show_with_retired_environment, show);
    assert!(show_with_retired_environment.get("reviews").is_none());

    let template = json_output(
        root,
        &[
            "review", "code", "template", "--style", "numbered", "--json",
        ],
    );
    assert_eq!(template["style"].as_str(), Some("numbered"));
    assert!(template["template"]
        .as_str()
        .unwrap()
        .starts_with("1. Reviewed files"));

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/changes/RT-1:requestReview"));
    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/changes/RT-1/reviews"));
    let recorded_reviews = logged
        .iter()
        .filter(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/changes/RT-1/reviews"
        })
        .map(|row| parse_json(&row.body))
        .collect::<Vec<_>>();
    assert!(recorded_reviews.iter().any(|review| {
        review["action"] == json!("code_review_summary") && review["reviewer"] == json!("Einstein") && review["reviewer_kind"] == json!("bot")
    }));
    assert!(recorded_reviews.iter().any(|review| {
        review["action"] == json!("task_approve")
            && review["reviewer"] == json!("Einstein")
            && review["reviewer_kind"] == json!("bot")
            && review["comment"]
                == json!("Automatic Task approval authorized by repository `task_review=automatic` policy.")
    }));
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url == "/v1/native/repository-authorities/7/changes/RT-1/reviews"));
}

#[test]
fn native_policy_namespace_supports_show_and_waive() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let show = json_output(root, &["policy", "show", "RP-1", "--json"]);
    assert_eq!(show["decision"].as_str(), Some("pass"));
    assert_eq!(
        show["checks"],
        json!([{"name": "require_tests", "status": "pass"}])
    );
    assert!(show.get("input_fingerprint").is_none());

    let show_with_retired_environment = json_output_with_env(
        root,
        &["policy", "show", "RP-1", "--json"],
        &[(concat!("AIT_JSON_", "MODE"), "debug")],
    );
    assert_eq!(show_with_retired_environment, show);
    assert!(show_with_retired_environment
        .get("input_fingerprint")
        .is_none());

    let waive = json_output(
        root,
        &[
            "policy",
            "waive",
            "RP-1",
            "--rule",
            "require_tests",
            "--reason",
            "compatibility",
            "--json",
        ],
    );
    assert_eq!(waive["waiver_id"].as_str(), Some("WV-1"));
    assert_eq!(waive["rule_name"].as_str(), Some("require_tests"));

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url == "/v1/native/repository-authorities/7/patchsets/RP-1/policy"));
    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/patchsets/RP-1/waivers"));
}

#[test]
fn native_land_namespace_is_removed_from_cli_surface() {
    let temp = init_repo("https://example.test");
    let root = temp.path();

    let output = cargo_bin()
        .current_dir(root)
        .args(["land", "show", "LAND-1", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand 'land'"));
}
