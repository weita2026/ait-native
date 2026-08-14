#[test]
fn native_patchset_publish_rejects_bound_worktree_retarget_requirement() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "retarget candidate",
            "--json",
        ],
    );
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
            "--change",
            "RC-1",
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
    assert!(!logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets"));
}

#[test]
fn native_patchset_publish_uses_unchanged_remote_base_when_local_main_is_ahead() {
    let (base_url, log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    let feature_snapshot = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "direct remote candidate",
            "--json",
        ],
    );
    let feature_snapshot_id = feature_snapshot["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();

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
            "--change",
            "RC-1",
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
    assert!(logged
        .iter()
        .any(|row| row.method == "POST"
            && row.url == "/v1/native/repository-authorities/7/changes/RC-1/patchsets"));
}

#[test]
fn native_workflow_ready_retarget_uses_executing_worktree_not_root_binding() {
    let (base_url, _log, state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();

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
    let mut other_metadata: JsonValue =
        parse_json_file(&other_metadata_path);
    other_metadata["fork_snapshot_id"] = JsonValue::String(advanced_main_snapshot_id.clone());
    write_file(
        &other_metadata_path,
        &(encode_json_pretty(&other_metadata) + "\n"),
    );

    let root_config_path = root.join(".ait/config.json");
    let mut root_config: JsonValue =
        parse_json_file(&root_config_path);
    root_config["worktree_name"] = JsonValue::String("rt-other".to_string());
    write_file(
        &root_config_path,
        &(encode_json_pretty(&root_config) + "\n"),
    );

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = workflow_ready_payload(&repo, "RC-1", None).unwrap();
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
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());

    write_file(
        &root.join("src/lib.rs"),
        "pub fn repo_root_version() -> &'static str { \"local only\" }\n",
    );
    let advanced_local_main_snapshot_id = seed_snapshot(root, "advance local main only");

    let repo = RepoRuntime::discover_from_path(&worktree).unwrap();
    let payload = workflow_ready_payload(&repo, "RC-1", None).unwrap();
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
    metadata["last_retargeted_at"] =
        JsonValue::String("2026-07-15T00:00:00Z".to_string());
    write_file(&metadata_path, &(encode_json_pretty(&metadata) + "\n"));

    let payload = json_output(
        &worktree,
        &["worktree", "rebase", "--dry-run", "--json"],
    );
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

    let payload = json_output(
        &worktree,
        &["worktree", "rebase", "--dry-run", "--json"],
    );
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
fn native_patchset_publish_uses_one_zstd_plan_for_suffix_above_remote_head() {
    let (base_url, log, state, handle) = spawn_bounded_snapshot_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let first_snapshot_id = FIXTURE_BASE_SNAPSHOT_ID.to_string();
    state.lock().unwrap().remote_head_snapshot_id = Some(first_snapshot_id.clone());

    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"changed\" }\n",
    );
    let second_snapshot = json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "suffix snapshot",
            "--json",
        ],
    );
    let second_snapshot_id = second_snapshot["snapshot_id"].as_str().unwrap().to_string();

    let patchset = json_output(
        root,
        &[
            "patchset",
            "publish",
            "--change",
            "RC-1",
            "--summary",
            "Bounded suffix publish",
            "--json",
        ],
    );
    assert_eq!(
        patchset["snapshot_sync"]["checked_snapshots"].as_i64(),
        Some(1)
    );
    assert_eq!(
        patchset["snapshot_sync"]["uploaded_snapshots"].as_i64(),
        Some(1)
    );
    assert_eq!(
        patchset["snapshot_sync"]["skipped_snapshots"].as_i64(),
        Some(0)
    );
    assert_eq!(
        patchset["snapshot_sync"]["sync_scope"].as_str(),
        Some("bounded_suffix")
    );
    assert_eq!(
        patchset["snapshot_sync"]["sync_reason"].as_str(),
        Some("remote_head_is_local_ancestor")
    );
    assert_eq!(
        patchset["snapshot_sync"]["bounded_by_snapshot_id"].as_str(),
        Some(first_snapshot_id.as_str())
    );

    handle.join().unwrap();
    let logged = log.lock().unwrap().clone();
    assert!(!logged
        .iter()
        .any(|row| {
            row.method == "POST"
                && row.url == "/v1/native/repository-authorities/7/snapshots:exists"
        }));
    let plan_request = logged
        .iter()
        .find(|row| {
            row.method == "POST"
                && row.url
                    == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/plan"
        })
        .expect("expected authoritative zstd plan request");
    let plan_payload: JsonValue = parse_json(&plan_request.body);
    let snapshot_ids = plan_payload["snapshot_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(snapshot_ids, vec![second_snapshot_id.clone()]);
    assert!(logged.iter().any(|row| row.method == "PUT"
        && row
            .url
            .starts_with("/v1/native/repository-authorities/7/remote-sync/zstd-bulk/object-packs/")));
    assert!(logged.iter().any(|row| row.method == "PUT"
        && row
            .url
            .starts_with("/v1/native/repository-authorities/7/remote-sync/zstd-bulk/tree-packs/")));
    assert!(logged.iter().any(|row| row.method == "POST"
        && row.url == "/v1/native/repository-authorities/7/remote-sync/zstd-bulk/commit"
        && row.body.contains(second_snapshot_id.as_str())));
    assert!(!logged.iter().any(|row| {
        row.method == "PUT"
            && row
                .url
                .starts_with("/v1/native/repository-authorities/7/snapshots/")
    }));
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

    let pushed = json_output(root, &["push", "--line", "main", "--json"]);
    assert_eq!(pushed["remote"].as_str(), Some("origin"));
    assert_eq!(pushed["repo_name"].as_str(), Some("fixture-ait"));
    assert_eq!(pushed["line"].as_str(), Some("main"));
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
        && row
            .url
            .starts_with("/v1/native/repository-authorities/7/remote-sync/zstd-bulk/object-packs/")));
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
fn native_push_phase_timings_and_transfer_metrics_are_opt_in_debug_json() {
    let (base_url, _log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"normal-timing\" }\n",
    );
    seed_snapshot(root, "normal timing push");
    let normal = json_output(root, &["push", "--line", "main", "--json"]);
    assert!(normal.get("phase_timings_ms").is_none());
    assert!(normal.get("remote_sync_metrics").is_none());
    handle.join().unwrap();

    let (base_url, log, state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    state.lock().unwrap().remote_head_snapshot_id = Some(FIXTURE_BASE_SNAPSHOT_ID.to_string());
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"debug-timing\" }\n",
    );
    seed_snapshot(root, "debug timing push");
    let debug = json_output_with_env(
        root,
        &["push", "--line", "main", "--json"],
        &[
            ("AIT_JSON_MODE", "debug"),
            ("AIT_REMOTE_SYNC_PACK_PARALLELISM", "99"),
        ],
    );
    assert!(debug["phase_timings_ms"]["zstd_bulk"].is_object());
    for phase in [
        "local_plan",
        "plan_http",
        "pack_prepare",
        "pack_upload_pipeline",
        "commit_assembly",
        "commit_http",
        "total",
    ] {
        assert!(
            debug["phase_timings_ms"]["zstd_bulk"][phase]
                .as_f64()
                .is_some(),
            "missing push phase timing {phase}"
        );
    }
    assert_eq!(
        debug["remote_sync_metrics"]["remote_round_trips"].as_u64(),
        Some(log.lock().unwrap().len() as u64)
    );
    assert_eq!(
        debug["remote_sync_metrics"]["http_retry_count"].as_u64(),
        Some(0)
    );
    assert!(debug["remote_sync_metrics"]["transferred_pack_bytes"]
        .as_u64()
        .is_some_and(|bytes| bytes > 0));
    assert_eq!(
        debug["remote_sync_metrics"]["pack_parallelism"].as_u64(),
        Some(16)
    );
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
    seed_snapshot(root, "push Perfetto");
    let trace_path = root.join(".ait-runtime/push.perfetto.json");
    fs::create_dir_all(trace_path.parent().unwrap()).unwrap();
    let trace_text = trace_path.to_string_lossy().to_string();
    let _ = json_output_with_env(
        root,
        &["push", "--line", "main", "--json"],
        &[
            ("AIT_JSON_MODE", "debug"),
            ("AIT_PERFETTO_TRACE", trace_text.as_str()),
        ],
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
        assert!(names.contains(expected), "missing Perfetto range {expected}");
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
        row.method == "PUT"
            && row.url == "/v1/native/repository-authorities/7/lines/feature%2Frt-1"
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
    assert_eq!(created["head_snapshot_id"].as_str(), Some(feature_snapshot_id.as_str()));

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
    assert_eq!(pushed["head_snapshot_id"].as_str(), Some(feature_snapshot_id.as_str()));
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
    let (base_url, _log, _state, handle) = spawn_publish_recovery_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let approve = json_output(
        root,
        &[
            "review",
            "team",
            "approve",
            "RC-1",
            "--patchset",
            "RP-1",
            "--json",
        ],
    );

    assert_eq!(approve["action"].as_str(), Some("approve"));
    assert_eq!(approve["change_id"].as_str(), Some("RC-1"));
    handle.join().unwrap();
}

#[test]
fn native_review_namespace_supports_show_request_task_comment_and_template() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let request = json_output(
        root,
        &[
            "review",
            "team",
            "request",
            "RC-1",
            "--group",
            "core",
            "--patchset",
            "RP-1",
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
            "RC-1",
            "--patchset",
            "RP-1",
            "--message",
            "looks fine",
            "--json",
        ],
    );
    assert_eq!(comment["action"].as_str(), Some("task_comment"));

    let show = json_output(root, &["review", "show", "RC-1", "--json"]);
    assert_eq!(show["current_patchset_id"].as_str(), Some("RP-1"));
    assert_eq!(show["approvals"].as_i64(), Some(1));
    assert_eq!(
        show["review_requests"],
        json!([{"patchset_id": "RP-1", "reviewer_group": "core"}])
    );
    assert!(show.get("reviews").is_none());

    let show_debug = json_output_with_env(
        root,
        &["review", "show", "RC-1", "--json"],
        &[("AIT_JSON_MODE", "debug")],
    );
    assert_eq!(
        show_debug["reviews"],
        json!([{
            "reviewer": "Fixture User <fixture@example.com>",
            "patchset_id": "RP-1",
            "action": "task_comment",
            "blocking": false,
            "comment": "looks fine"
        }])
    );

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
    assert!(logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes/RC-1:requestReview"));
    assert!(logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/changes/RC-1/reviews"));
    assert!(logged
        .iter()
        .any(|row| row.method == "GET" && row.url == "/v1/native/repository-authorities/7/changes/RC-1/reviews"));
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

    let show_debug = json_output_with_env(
        root,
        &["policy", "show", "RP-1", "--json"],
        &[("AIT_JSON_MODE", "debug")],
    );
    assert_eq!(show_debug["input_fingerprint"].as_str(), Some("abc"));
    assert_eq!(
        show_debug["checks"],
        json!([{"name": "require_tests", "status": "pass", "message": "ok"}])
    );

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
    assert!(logged
        .iter()
        .any(|row| row.method == "GET" && row.url == "/v1/native/repository-authorities/7/patchsets/RP-1/policy"));
    assert!(logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repository-authorities/7/patchsets/RP-1/waivers"));
}

#[test]
fn native_land_namespace_is_removed_from_cli_surface() {
    let temp = init_repo("https://example.test");
    let root = temp.path();

    let output = cargo_bin()
        .current_dir(root)
        .args(["land", "show", "LAND-1", "--json"])
        .env_remove("AIT_JSON_MODE")
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("unrecognized subcommand 'land'"));
}
