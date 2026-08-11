#[test]
fn native_snapshot_phase_timings_are_opt_in_debug_json() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"normal-json\" }\n",
    );
    let normal = json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "normal Snapshot JSON",
            "--json",
        ],
    );
    assert!(normal.get("phase_timings_ms").is_none());

    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"debug-json\" }\n",
    );
    let debug = json_output_with_env(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "debug Snapshot JSON",
            "--json",
        ],
        &[("AIT_JSON_MODE", "debug")],
    );
    assert!(debug["phase_timings_ms"].is_object());
    assert_eq!(
        debug["phase_timings_ms"]["hashing_cache"]["state_read"],
        json!("hit")
    );
}

#[cfg(feature = "perfetto-tracing")]
#[test]
fn native_snapshot_perfetto_trace_names_cover_stable_hot_phases() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"perfetto\" }\n",
    );
    let trace_path = root.join("snapshot.perfetto.json");
    let trace_text = trace_path.to_string_lossy().to_string();
    let _ = json_output_with_env(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "Perfetto Snapshot",
            "--json",
        ],
        &[
            ("AIT_JSON_MODE", "debug"),
            ("AIT_PERFETTO_TRACE", trace_text.as_str()),
        ],
    );
    let trace = parse_json_bytes(&fs::read(&trace_path).expect("Perfetto trace"));
    let names = trace["traceEvents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["name"].as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "ait.cli.snapshot_create.command",
        "ait.core.snapshot.repository_discovery",
        "ait.core.snapshot.workspace_scan",
        "ait.core.snapshot.hash_cache_read",
        "ait.core.snapshot.workspace_projection_hash",
        "ait.core.snapshot.tree_build",
        "ait.core.snapshot.blob_lookup",
        "ait.core.snapshot.blob_pack_write",
        "ait.core.snapshot.blob_delta_lookup",
        "ait.core.snapshot.blob_pack_assembly",
        "ait.core.snapshot.blob_pack_archive_write",
        "ait.core.snapshot.blob_pack_metadata_commit",
        "ait.core.snapshot.tree_pack_write",
        "ait.core.snapshot.tree_lookup",
        "ait.core.snapshot.tree_pack_assembly",
        "ait.core.snapshot.tree_pack_archive_write",
        "ait.core.snapshot.tree_pack_metadata_commit",
        "ait.core.snapshot.tree_pack_locator",
        "ait.core.snapshot.metadata_transaction",
        "ait.core.snapshot.hash_cache_write",
    ] {
        assert!(names.contains(expected), "missing Perfetto range {expected}");
    }
}

#[test]
fn native_snapshot_and_remote_primitives_work_end_to_end() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"reviewable\" }\n",
    );
    let snapshot = json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "reviewable snapshot",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap().to_string();
    assert!(snapshot_id.starts_with("SNP-"));

    let patchset = json_output(
        root,
        &[
            "patchset",
            "publish",
            "--change",
            "RC-1",
            "--summary",
            "Native Rust patchset",
            "--json",
        ],
    );
    assert_eq!(patchset["patchset"]["patchset_id"].as_str(), Some("RP-2"));
    let patchset_id = patchset["patchset"]["patchset_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(
        patchset["revision_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(patchset["snapshot_sync"]["line"].as_str(), Some("main"));
    assert_eq!(
        patchset["snapshot_sync"]["line_updated"].as_bool(),
        Some(false)
    );
    assert_eq!(
        patchset["snapshot_sync"]["line_update_skipped_reason"].as_str(),
        Some("current line is the change base line")
    );
    assert_eq!(
        patchset["snapshot_sync"]["head_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );

    let ci_status = json_output(root, &["patchset", "ci-status", &patchset_id, "--json"]);
    assert_eq!(ci_status["tests_status"].as_str(), Some("pass"));

    let rerun = json_output(root, &["patchset", "rerun-ci", &patchset_id, "--json"]);
    assert_eq!(rerun["queued"].as_bool(), Some(true));

    let approve = json_output(
        root,
        &[
            "review",
            "team",
            "approve",
            "RC-1",
            "--patchset",
            &patchset_id,
            "--json",
        ],
    );
    assert_eq!(approve["action"].as_str(), Some("approve"));

    let code_review = json_output(
        root,
        &[
            "review",
            "code",
            "submit",
            "RC-1",
            "--patchset",
            &patchset_id,
            "--message",
            "Reviewed files: src/lib.rs; Findings: none; Risks: low; Tests: cargo test; Recommendation: land",
            "--json",
        ],
    );
    assert_eq!(code_review["action"].as_str(), Some("code_review_summary"));

    let attestation = json_output(
        root,
        &["attest", "put", &patchset_id, "--tests", "pass", "--json"],
    );
    assert_eq!(
        attestation["patchset_id"].as_str(),
        Some(patchset_id.as_str())
    );
    let attestation_show = json_output(root, &["attest", "show", &patchset_id, "--json"]);
    assert_eq!(attestation_show["attestation_id"].as_str(), Some("AT-1"));

    let policy = json_output(root, &["policy", "eval", &patchset_id, "--json"]);
    assert_eq!(policy["decision"].as_str(), Some("pass"));

    let task_land = json_output(
        root,
        &[
            "task", "land", "RT-1", "--target", "main", "--mode", "direct", "--json",
        ],
    );
    assert_eq!(
        task_land["apply_status"].as_str(),
        Some("done"),
        "{}",
        encode_json_pretty(&task_land)
    );
    assert_eq!(
        action_result(&task_land, "submit_land")["status"].as_str(),
        Some("succeeded")
    );
    assert_eq!(
        action_result(&task_land, "complete_task")["status"].as_str(),
        Some("completed")
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets"));
    assert!(logged.iter().any(|row| row.method == "PUT"
        && row.url == format!("/v1/native/repository-authorities/7/patchsets/{patchset_id}/attestation")
        && row.body.contains("\"tests\":\"pass\"")));
    assert!(logged.iter().any(|row| row.method == "GET"
        && row.url == format!("/v1/native/repository-authorities/7/patchsets/{patchset_id}/attestation")));
    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/changes/RC-1/reviews"
        && row.body.contains("\"action\":\"code_review_summary\"")));
    assert!(logged.iter().any(|row| {
        row.method == "POST"
            && row.url == "/v1/native/repository-authorities/7/task-land"
            && row.body.contains("\"contract\":\"task-land-atomic/v1\"")
    }));
    assert!(!logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks/RT-1:close"));
}

#[test]
fn native_snapshot_create_rejects_clean_workspace_without_advancing_head() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let output = cargo_bin()
        .current_dir(root)
        .args([
            "snapshot",
            "create",
            "--message",
            "message-only snapshot",
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
    assert!(stderr.contains("workspace tree is unchanged from parent snapshot"));
    assert_eq!(
        local_line_head(root, "main").as_deref(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    let snapshots = json_output(root, &["snapshot", "list", "--json"]);
    assert_eq!(snapshots.as_array().unwrap().len(), 1);

    drop(temp);
    handle.join().unwrap();
}

#[test]
fn native_patchset_ci_status_human_output_surfaces_reset_notice() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let mut command = cargo_bin();
    let output = command
        .current_dir(root)
        .args(["patchset", "ci-status", "RP-RESET"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("recommended_action"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("rebase_patchset_to_latest_main"),
        "stdout:\n{stdout}"
    );
    assert!(stdout.contains("status_message"), "stdout:\n{stdout}");
    assert!(
        stdout.contains("Patchset CI reset after land moved fixture-ait:main from SNP-A to SNP-B"),
        "stdout:\n{stdout}"
    );

    drop(temp);
    handle.join().unwrap();
}

#[test]
fn native_task_land_closes_when_base_stale_submit_already_moved_target_to_revision() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"already landed\" }\n",
    );
    let revision_snapshot_id = seed_snapshot(root, "already moved land revision");
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.selected_patchset_id = Some("RP-2".to_string());
        guard.selected_patchset_base_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.selected_patchset_revision_snapshot_id = Some(revision_snapshot_id.clone());
        guard.land_submit_base_stale_converged = true;
        guard.omit_landing_summary_after_base_stale_converged = true;
    }

    let task_land = json_output(
        root,
        &[
            "task", "land", "RT-1", "--target", "main", "--mode", "direct", "--json",
        ],
    );

    assert_eq!(
        task_land["apply_status"].as_str(),
        Some("done"),
        "{}",
        encode_json_pretty(&task_land)
    );
    let land_result = action_result(&task_land, "submit_land");
    assert_eq!(land_result["status"].as_str(), Some("succeeded"));
    assert_eq!(
        land_result["result"]["landed_snapshot_id"].as_str(),
        Some(revision_snapshot_id.as_str())
    );
    assert_eq!(
        land_result["local_sync"]["landed_snapshot_id"].as_str(),
        Some(revision_snapshot_id.as_str())
    );
    assert_eq!(task_land["change"]["status"].as_str(), Some("landed"));
    assert_eq!(
        task_land["change"]["landed_snapshot_id"].as_str(),
        Some(revision_snapshot_id.as_str())
    );
    assert_eq!(
        action_result(&task_land, "complete_task")["status"].as_str(),
        Some("completed")
    );
    {
        let guard = state.lock().unwrap();
        assert!(guard.land_submitted);
        assert!(guard.task_completed);
        assert_eq!(
            guard.remote_head_snapshot_id.as_deref(),
            Some(revision_snapshot_id.as_str())
        );
    }

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
    }));
    assert!(!logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/tasks/RT-1:close"));
    assert!(!logged
        .iter()
        .any(|row| { row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets" }));
}

#[test]
fn native_task_land_submits_when_target_line_already_points_at_revision() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"already at revision\" }\n",
    );
    let revision_snapshot_id = seed_snapshot(root, "already at revision land");
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(revision_snapshot_id.clone());
        guard.selected_patchset_id = Some("RP-2".to_string());
        guard.selected_patchset_base_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.selected_patchset_revision_snapshot_id = Some(revision_snapshot_id.clone());
    }

    let task_land = json_output(
        root,
        &[
            "task", "land", "RT-1", "--target", "main", "--mode", "direct", "--json",
        ],
    );

    assert_eq!(
        task_land["apply_status"].as_str(),
        Some("done"),
        "{}",
        encode_json_pretty(&task_land)
    );
    let land_result = action_result(&task_land, "submit_land");
    assert_eq!(land_result["status"].as_str(), Some("succeeded"));
    assert_eq!(
        land_result["result"]["landed_snapshot_id"].as_str(),
        Some(revision_snapshot_id.as_str())
    );
    assert_eq!(task_land["change"]["status"].as_str(), Some("landed"));
    {
        let guard = state.lock().unwrap();
        assert!(guard.land_submitted);
        assert_eq!(
            guard.remote_head_snapshot_id.as_deref(),
            Some(revision_snapshot_id.as_str())
        );
    }

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
    }));
    assert!(!logged
        .iter()
        .any(|row| { row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets" }));
}

#[test]
fn native_task_land_submits_when_target_line_already_contains_revision() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"selected revision\" }\n",
    );
    let selected_revision_snapshot_id = seed_snapshot(root, "selected revision");
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"containing revision\" }\n",
    );
    let containing_snapshot_id = seed_snapshot(root, "containing revision");
    {
        let mut guard = state.lock().unwrap();
        guard.remote_head_snapshot_id = Some(containing_snapshot_id.clone());
        guard.selected_patchset_id = Some("RP-2".to_string());
        guard.selected_patchset_base_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
        guard.selected_patchset_revision_snapshot_id = Some(selected_revision_snapshot_id.clone());
    }

    let task_land = json_output(
        root,
        &[
            "task", "land", "RT-1", "--target", "main", "--mode", "direct", "--json",
        ],
    );

    assert_eq!(
        task_land["apply_status"].as_str(),
        Some("done"),
        "{}",
        encode_json_pretty(&task_land)
    );
    assert_eq!(
        action_result(&task_land, "submit_land")["status"].as_str(),
        Some("succeeded")
    );
    {
        let guard = state.lock().unwrap();
        assert!(guard.land_submitted);
    }

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(logged.iter().any(|row| {
        row.method == "POST" && row.url == "/v1/native/repository-authorities/7/task-land"
    }));
    assert!(!logged
        .iter()
        .any(|row| { row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets" }));
}

#[test]
fn native_snapshot_create_prefers_worktree_workspace_root() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);

    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "worktree snapshot",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap();
    let size_bytes = snapshot_blob_size(&worktree, snapshot_id, "src/lib.rs");
    assert_eq!(
        size_bytes as usize,
        "pub fn worktree_version() -> &'static str { \"worktree override\" }\n".len()
    );
    let worktree_config: JsonValue = parse_json_file(worktree.join(".ait-worktree.json"));
    assert_eq!(
        worktree_config["materialized_snapshot_id"].as_str(),
        Some(snapshot_id)
    );
    handle.join().unwrap();
}

#[test]
fn native_snapshot_revert_and_replay_work_without_snapshot_files_view() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let base_snapshot_id = FIXTURE_BASE_SNAPSHOT_ID.to_string();
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"changed\" }\n",
    );
    write_file(&root.join("notes.txt"), "hello from replay\n");
    let changed_snapshot_id = seed_snapshot(root, "changed snapshot");

    let revert = json_output(
        root,
        &["snapshot", "revert", &changed_snapshot_id, "--json"],
    );
    assert_eq!(revert["affected_path_count"].as_u64(), Some(2));
    assert_eq!(
        fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        "pub fn example() -> &'static str { \"ok\" }\n"
    );
    assert!(!root.join("notes.txt").exists());

    seed_binary_line(root, "main", &base_snapshot_id);
    let replay = json_output(
        root,
        &[
            "snapshot",
            "replay",
            &changed_snapshot_id,
            "--onto",
            "main",
            "--json",
        ],
    );
    assert_eq!(replay["affected_path_count"].as_u64(), Some(2));
    assert_eq!(
        fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        "pub fn example() -> &'static str { \"changed\" }\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("notes.txt")).unwrap(),
        "hello from replay\n"
    );

    handle.join().unwrap();
}

#[test]
fn native_worktree_cli_lists_registered_worktrees_from_repo_root() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();

    let listed = json_output(root, &["worktree", "list", "--json"]);
    let rows = listed.as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"].as_str(), Some("rt-1"));
    assert_eq!(
        rows[0]["path"].as_str(),
        Some(worktree.to_string_lossy().as_ref())
    );

    let shown = json_output(root, &["worktree", "show", "rt-1", "--json"]);
    assert_eq!(shown["name"].as_str(), Some("rt-1"));
    assert_eq!(shown["bound_task_id"].as_str(), Some("RT-1"));
    assert_eq!(shown["bound_change_id"].as_str(), Some("RC-1"));
    assert_eq!(shown["current_line"].as_str(), Some("feature/rt-1"));

    handle.join().unwrap();
}

#[test]
fn native_worktree_cli_path_surface_supports_open_alias() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);

    let path_payload = json_output(&worktree, &["worktree", "path", "--json"]);
    assert_eq!(path_payload["name"].as_str(), Some("rt-1"));
    assert_eq!(
        path_payload["open_path"].as_str(),
        Some(worktree.to_string_lossy().as_ref())
    );

    let open_payload = json_output(&worktree, &["worktree", "open", "--json"]);
    assert_eq!(open_payload["open_path"], path_payload["open_path"]);
    assert_eq!(open_payload["shell_command"], path_payload["shell_command"]);

    handle.join().unwrap();
}

#[test]
fn native_worktree_status_inspects_named_worktree_content() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = worktree.parent().unwrap().to_path_buf();

    let status = json_output(&repo_root, &["worktree", "status", "rt-1", "--json"]);
    assert_eq!(status["is_worktree"].as_bool(), Some(true));
    assert_eq!(status["worktree_name"].as_str(), Some("rt-1"));
    assert_eq!(status["current_line"].as_str(), Some("feature/rt-1"));
    assert!(status["changed_count"].as_u64().unwrap_or(0) > 0);

    handle.join().unwrap();
}

#[test]
fn native_worktree_restore_can_target_named_worktree_from_repo_root() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = worktree.parent().unwrap().to_path_buf();

    let restored = json_output(
        &repo_root,
        &["worktree", "restore", "rt-1", "--force", "--json"],
    );
    assert_eq!(restored["worktree_name"].as_str(), Some("rt-1"));
    assert_eq!(restored["current_line"].as_str(), Some("feature/rt-1"));
    assert_eq!(restored["applied"].as_bool(), Some(true));
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        "pub fn example() -> &'static str { \"ok\" }\n"
    );

    let status = json_output(&repo_root, &["worktree", "status", "rt-1", "--json"]);
    assert_eq!(status["clean"].as_bool(), Some(true));
    assert_eq!(status["changed_count"].as_u64(), Some(0));

    handle.join().unwrap();
}

#[test]
fn native_worktree_restore_reads_target_snapshot_without_status_manifest_cache() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = worktree.parent().unwrap().to_path_buf();

    write_file(
        &repo_root.join("src/lib.rs"),
        "pub fn main_line_version() -> &'static str { \"target\" }\n",
    );
    let target_snapshot_id = seed_snapshot(&repo_root, "main line target");
    let target_manifest_dir = repo_root.join(".ait/workspace/status-manifests");
    assert!(!target_manifest_dir.exists());

    let restored = json_output(
        &repo_root,
        &[
            "worktree", "restore", "rt-1", "--line", "main", "--force", "--json",
        ],
    );
    assert_eq!(restored["worktree_name"].as_str(), Some("rt-1"));
    assert_eq!(restored["current_line"].as_str(), Some("main"));
    assert_eq!(
        restored["line_head_snapshot_id"].as_str(),
        Some(target_snapshot_id.as_str())
    );
    assert!(restored.get("status_manifest").is_none());
    assert!(!target_manifest_dir.exists());
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        "pub fn main_line_version() -> &'static str { \"target\" }\n"
    );

    let status = json_output(&repo_root, &["worktree", "status", "rt-1", "--json"]);
    assert_eq!(
        status["baseline_snapshot_id"].as_str(),
        Some(target_snapshot_id.as_str())
    );
    assert_eq!(status["clean"].as_bool(), Some(true));
    assert_eq!(status["changed_count"].as_u64(), Some(0));

    handle.join().unwrap();
}

#[test]
fn native_snapshot_namespace_supports_list_show_and_diff() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);

    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "worktree snapshot",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap();

    let listed = json_output(&worktree, &["snapshot", "list", "--json"]);
    let listed_rows = listed.as_array().unwrap();
    assert_eq!(listed_rows[0]["snapshot_id"].as_str(), Some(snapshot_id));

    let shown = json_output(&worktree, &["snapshot", "show", snapshot_id, "--json"]);
    assert_eq!(shown["snapshot_id"].as_str(), Some(snapshot_id));
    let shown_paths = shown["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row.get("path").and_then(JsonValue::as_str))
        .collect::<Vec<_>>();
    assert!(shown_paths.contains(&"src/lib.rs"));

    let diff = json_output(
        &worktree,
        &[
            "snapshot",
            "diff",
            FIXTURE_BASE_SNAPSHOT_ID,
            snapshot_id,
            "--json",
        ],
    );
    assert_eq!(
        diff["modified"].as_array().unwrap()[0].as_str(),
        Some("src/lib.rs")
    );
    assert_eq!(
        diff["files"].as_array().unwrap()[0]["status"].as_str(),
        Some("modified")
    );

    let ancestry = json_output(
        &worktree,
        &["snapshot", "ancestry", snapshot_id, "--ancestors", "--json"],
    );
    assert_eq!(ancestry["contract"].as_str(), Some("snapshot-ancestry/v1"));
    assert_eq!(ancestry["direction"].as_str(), Some("ancestors"));
    assert_eq!(ancestry["includes_query_snapshot"].as_bool(), Some(false));
    assert!(ancestry["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["snapshot_id"].as_str() == Some(FIXTURE_BASE_SNAPSHOT_ID)));

    let descendants = json_output(
        &worktree,
        &[
            "snapshot",
            "ancestry",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--descendants",
            "--json",
        ],
    );
    assert_eq!(descendants["direction"].as_str(), Some("descendants"));
    assert!(descendants["snapshots"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["snapshot_id"].as_str() == Some(snapshot_id)));

    let is_ancestor = json_output(
        &worktree,
        &[
            "snapshot",
            "is-ancestor",
            FIXTURE_BASE_SNAPSHOT_ID,
            snapshot_id,
            "--json",
        ],
    );
    assert_eq!(
        is_ancestor["contract"].as_str(),
        Some("snapshot-is-ancestor/v1")
    );
    assert_eq!(is_ancestor["is_ancestor"].as_bool(), Some(true));
    assert_eq!(is_ancestor["distance"].as_u64(), Some(1));

    let false_ancestor = command_output_with_env(
        &worktree,
        &[
            "snapshot",
            "is-ancestor",
            snapshot_id,
            FIXTURE_BASE_SNAPSHOT_ID,
            "--json",
        ],
        &[],
    );
    assert_eq!(false_ancestor.status.code(), Some(1));
    assert_eq!(
        parse_json_bytes(&false_ancestor.stdout)["is_ancestor"].as_bool(),
        Some(false)
    );

    let missing_ancestor = command_output_with_env(
        &worktree,
        &[
            "snapshot",
            "is-ancestor",
            "SNP-DOES-NOT-EXIST",
            snapshot_id,
            "--json",
        ],
        &[],
    );
    assert_eq!(missing_ancestor.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&missing_ancestor.stderr).contains("Error:"));

    let merge_base = json_output(
        &worktree,
        &[
            "snapshot",
            "merge-base",
            FIXTURE_BASE_SNAPSHOT_ID,
            snapshot_id,
            "--all",
            "--json",
        ],
    );
    assert_eq!(
        merge_base["contract"].as_str(),
        Some("snapshot-merge-base/v1")
    );
    assert_eq!(
        merge_base["merge_base_snapshot_ids"][0].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );

    handle.join().unwrap();
}

#[test]
fn native_top_level_diff_reports_repo_root_dirty_text() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"updated\" }\n",
    );

    let diff = json_output(root, &["diff", "--json"]);
    assert_eq!(diff["is_worktree"].as_bool(), Some(false));
    assert_eq!(
        diff["baseline_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        diff["modified"].as_array().unwrap()[0].as_str(),
        Some("src/lib.rs")
    );
    assert_eq!(
        diff["files"].as_array().unwrap()[0]["diff"]["status"].as_str(),
        Some("text")
    );
    assert!(diff["files"].as_array().unwrap()[0]["diff"]["text"]
        .as_str()
        .unwrap()
        .contains("+pub fn example() -> &'static str { \"updated\" }"));

    handle.join().unwrap();
}

#[test]
fn native_top_level_diff_accepts_trailing_path_filters() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"updated\" }\n",
    );
    write_file(&root.join("src/new.rs"), "pub fn added() {}\n");

    let diff = json_output(root, &["diff", "--json", "--", "src/lib.rs"]);
    assert_eq!(diff["path_filters"], json!(["src/lib.rs"]));
    assert_eq!(diff["changed_paths"], json!(["src/lib.rs"]));
    assert_eq!(
        diff["files"].as_array().unwrap()[0]["path"].as_str(),
        Some("src/lib.rs")
    );

    let combined = json_output(
        root,
        &["diff", "--json", "--path", "src/lib.rs", "--", "src/new.rs"],
    );
    assert_eq!(
        combined["path_filters"],
        json!(["src/lib.rs", "src/new.rs"])
    );
    assert_eq!(
        combined["changed_paths"],
        json!(["src/lib.rs", "src/new.rs"])
    );

    handle.join().unwrap();
}

#[test]
fn native_top_level_diff_reports_worktree_dirty_paths_and_name_only() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);

    write_file(&worktree.join("src/new.rs"), "pub fn added() {}\n");

    let diff = json_output(&worktree, &["diff", "--json", "--path", "src"]);
    assert_eq!(diff["is_worktree"].as_bool(), Some(true));
    assert_eq!(diff["worktree_name"].as_str(), Some("rt-1"));
    assert_eq!(diff["baseline_line_name"].as_str(), Some("feature/rt-1"));
    assert!(diff["changed_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row.as_str() == Some("src/lib.rs")));
    assert!(diff["untracked"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row.as_str() == Some("src/new.rs")));

    let mut command = cargo_bin();
    let output = command
        .current_dir(&worktree)
        .args(["diff", "--name-only", "--path", "src/new.rs"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "src/new.rs\n");

    handle.join().unwrap();
}

#[test]
fn native_snapshot_diff_works_without_snapshot_files_view() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);

    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "worktree snapshot",
            "--json",
        ],
    );
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap();

    let diff = json_output(
        &worktree,
        &[
            "snapshot",
            "diff",
            FIXTURE_BASE_SNAPSHOT_ID,
            snapshot_id,
            "--json",
        ],
    );
    assert_eq!(
        diff["modified"].as_array().unwrap()[0].as_str(),
        Some("src/lib.rs")
    );
    assert_eq!(
        diff["files"].as_array().unwrap()[0]["status"].as_str(),
        Some("modified")
    );

    handle.join().unwrap();
}

#[test]
fn native_snapshot_show_defaults_to_bounded_parent_change_evidence() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    for index in 0..25 {
        write_file(
            &root.join(format!("src/generated_{index:02}.rs")),
            &format!("pub const VALUE_{index}: usize = {index};\n"),
        );
    }
    let snapshot_id = seed_snapshot(root, "bounded snapshot evidence");

    let compact = command_output_with_env(root, &["snapshot", "show", &snapshot_id], &[]);
    assert!(compact.status.success());
    let compact = String::from_utf8_lossy(&compact.stdout);
    assert!(compact.contains("ait snapshot show\n"));
    assert!(compact.contains("change: 25 files (25 added)"));
    assert!(compact.contains("changed paths\nstatus\tpath"));
    assert!(compact.contains("shown: 20/25"));
    assert!(compact.contains(&format!(
        "more: ait snapshot diff {FIXTURE_BASE_SNAPSHOT_ID} {snapshot_id}"
    )));
    assert!(compact.contains(&format!(
        "tree: ait snapshot show {snapshot_id} --files"
    )));
    assert!(!compact.contains("blob_id"));

    let files = command_output_with_env(
        root,
        &["snapshot", "show", &snapshot_id, "--files"],
        &[],
    );
    assert!(files.status.success());
    let files = String::from_utf8_lossy(&files.stdout);
    assert!(files.contains("files\npath\tblob_id\tsize_bytes\tmode"));
    assert!(files.contains("src/generated_24.rs"));
    assert!(!files.contains("shown: 20/25"));
}

#[test]
fn native_snapshot_ancestry_bounds_text_and_prints_exact_full_command() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    let mut latest = String::new();
    for index in 0..25 {
        write_file(
            &root.join("src/lib.rs"),
            &format!("pub fn fixture() -> usize {{ {index} }}\n"),
        );
        latest = seed_snapshot(root, &format!("ancestry {index}"));
    }

    let output = command_output_with_env(
        root,
        &[
            "snapshot",
            "ancestry",
            &latest,
            "--ancestors",
            "--max-depth",
            "100",
            "--limit",
            "100",
        ],
        &[],
    );
    assert!(output.status.success());
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("nearest history"));
    assert!(output.contains("shown: 20/25"));
    assert!(output.contains(&format!(
        "more: ait snapshot ancestry {latest} --ancestors --max-depth 100 --limit 100 --all"
    )));
}
