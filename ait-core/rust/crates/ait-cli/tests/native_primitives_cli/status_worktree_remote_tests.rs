#[test]
fn native_snapshot_create_excludes_markdown_paths_without_planning_only_artifacts() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(&root.join("README.md"), "# Readme\n");
    write_file(
        &root.join("release/guides/LOCAL_QUICKSTART.md"),
        "# Quickstart\n",
    );
    write_file(&root.join("docs/plan.md"), "# Plan\n");
    write_file(&root.join("docs/data.json"), "{\"ok\": true}\n");
    write_file(&root.join("LICENSE"), "license\n");

    let snapshot = json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "exclude markdown",
            "--json",
        ],
    );
    let snapshot_paths = snapshot["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["path"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();

    assert!(snapshot_paths.contains(&"src/lib.rs".to_string()));
    assert!(snapshot_paths.contains(&"LICENSE".to_string()));
    assert!(snapshot_paths.contains(&"docs/data.json".to_string()));
    assert!(!snapshot_paths.contains(&"README.md".to_string()));
    assert!(!snapshot_paths.contains(&"release/guides/LOCAL_QUICKSTART.md".to_string()));
    assert!(!snapshot_paths.contains(&"docs/plan.md".to_string()));
}

#[test]
fn native_snapshot_create_projects_out_retired_task_dag_files() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(
        &root.join("docs/sprints/card.task_graph.json"),
        "{\"nodes\": []}\n",
    );
    write_file(&root.join("src/retained.rs"), "pub fn retained() {}\n");

    let snapshot = json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "ignore retired task DAG file",
            "--json",
        ],
    );
    let snapshot_paths = snapshot["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(snapshot_paths.contains(&"src/retained.rs"));
    assert!(!snapshot_paths.contains(&"docs/sprints/card.task_graph.json"));
}

#[test]
fn native_status_and_diff_filter_markdown_without_status_manifest_cache() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(&root.join("README.md"), "# Readme\n");
    assert!(!root.join(".ait/workspace/status-manifests").exists());

    let status = json_output(root, &["status", "--json"]);
    assert_eq!(status["workspace_status"].as_str(), Some("clean"));
    assert_eq!(status["workspace_changed_paths_sample"], json!([]));
    assert_eq!(status["workspace_missing_count"].as_i64(), Some(0));

    let diff = json_output(root, &["diff", "--json"]);
    assert_eq!(diff["clean"].as_bool(), Some(true));
    assert_eq!(diff["changed_paths"], json!([]));
    assert_eq!(diff["files"], json!([]));
    assert!(!root.join(".ait/workspace/status-manifests").exists());
}

#[test]
fn native_status_reports_repo_summary_with_contract_json() {
    let temp = init_repo("https://example.test");
    let root = temp.path();

    let status = json_output(root, &["status", "--json"]);

    assert_eq!(status["repo_name"].as_str(), Some("fixture-ait"));
    assert_eq!(status["current_line"].as_str(), Some("main"));
    assert_eq!(
        status["head_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(status["workspace_status"].as_str(), Some("clean"));
    assert_eq!(status["workspace_dirty"].as_bool(), Some(false));
    assert_eq!(status["workspace_changed_paths_sample"], json!([]));
    assert_eq!(status["workspace_changed_count"].as_u64(), Some(0));
    assert_eq!(status["remote_count"].as_u64(), Some(1));
    assert_eq!(status["default_remote"].as_str(), Some("origin"));
    assert!(status["snapshot_count"].as_u64().unwrap_or(0) >= 1);
    assert_eq!(status["reconciliation"]["state"], json!("never_observed"));
    assert_eq!(
        status["reconciliation"]["next_command"],
        json!("ait workflow reconcile --remote origin --apply --safe-only")
    );
    assert!(status.get("ignore_policy").is_none());
    assert!(status.get("phase_timings_ms").is_none());
    assert!(status.get("pack_count").is_none());
}

#[test]
fn native_status_and_worktree_text_default_to_agent_decision_facts() {
    let temp = init_repo("https://example.test");
    let root = temp.path();

    let compact = command_output_with_env(root, &["status"], &[]);
    assert!(compact.status.success());
    let compact = String::from_utf8_lossy(&compact.stdout);
    assert!(compact.starts_with("ait status\n"));
    assert!(compact.contains("repo: fixture-ait"));
    assert!(compact.contains(&format!("line: main @ {FIXTURE_BASE_SNAPSHOT_ID}")));
    assert!(compact.contains("workspace: clean"));
    assert!(!compact.contains("snapshots:"));
    assert!(!compact.contains("packed blobs:"));
    assert!(!compact.contains("operational roots:"));
    assert!(!compact.contains("ait-cli"));

    let worktree = command_output_with_env(root, &["worktree", "status"], &[]);
    assert!(worktree.status.success());
    let worktree = String::from_utf8_lossy(&worktree.stdout);
    assert!(worktree.starts_with("ait worktree status\n"));
    assert!(worktree.contains(&format!("line: main @ {FIXTURE_BASE_SNAPSHOT_ID}")));
    assert!(worktree.contains("workspace: clean"));
    assert!(!worktree.contains("worktree_name:"));
    assert!(!worktree.contains("root:"));
    assert!(!worktree.contains("ait-cli"));
}

#[test]
fn native_dirty_status_exposes_truncation_and_skips_the_redundant_status_round_trip() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    for index in 0..12 {
        write_file(
            &root.join(format!("src/changed_{index}.rs")),
            "pub fn changed() {}\n",
        );
    }

    let status = command_output_with_env(root, &["status"], &[]);
    assert!(status.status.success());
    let status = String::from_utf8_lossy(&status.stdout);
    assert!(status.contains("workspace: dirty (12 changed)"));
    assert!(status.contains("shown: 10/12 changed paths"));
    assert!(status.contains("next: ait diff"));
    assert!(!status.contains("next: ait worktree status"));

    let worktree = command_output_with_env(root, &["worktree", "status"], &[]);
    assert!(worktree.status.success());
    let worktree = String::from_utf8_lossy(&worktree.stdout);
    assert!(worktree.contains("next: ait diff"));
}

#[test]
fn native_status_reports_line_hygiene_without_cleanup_scan() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    init_registered_worktree(
        root,
        "rt-fast-status",
        "feature/rt-fast-status",
        Some("RT-FAST"),
        Some("RC-FAST"),
        true,
        Some("after_remote_land"),
    );

    let payload = json_output(root, &["status", "--json"]);
    let line_hygiene = &payload["line_hygiene"];

    assert_eq!(line_hygiene["mode"].as_str(), Some("metadata_only"));
    assert_eq!(line_hygiene["idle_for"], JsonValue::Null);
    assert!(line_hygiene.get("older_than").is_none());
    assert_eq!(line_hygiene["candidate_count"], JsonValue::Null);
    assert_eq!(line_hygiene["protected_count"], JsonValue::Null);
    assert_eq!(line_hygiene["inspected_count"].as_i64(), Some(2));
    assert_eq!(
        line_hygiene["detail_command"].as_str(),
        Some("ait line cleanup --include-protected")
    );
}

#[test]
fn native_line_cleanup_preview_projects_line_usage_contract() {
    let (temp, _worktree) = init_worktree_repo("http://127.0.0.1:1");
    let root = temp.path();

    let payload = json_output(
        root,
        &[
            "line",
            "cleanup",
            "--include-protected",
            "--json",
        ],
    );
    let row = payload["protected"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["line_name"].as_str() == Some("feature/rt-1"))
        .unwrap();
    let usage = &row["usage"];

    let mut usage_keys = usage
        .as_object()
        .expect("line usage")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    usage_keys.sort();
    assert_eq!(
        usage_keys,
        vec![
            "active_change_count",
            "active_change_ids",
            "worktree_count",
            "worktree_names",
        ]
    );
}

#[test]
fn native_worktree_cleanup_candidates_project_binding_contract() {
    let (temp, _worktree) = init_worktree_repo("http://127.0.0.1:1");
    let root = temp.path();

    let payload = json_output(
        root,
        &[
            "worktree",
            "cleanup-candidates",
            "--include-protected",
            "--json",
        ],
    );
    let protected_rows = payload["protected"].as_array().unwrap();
    let candidate_rows = payload["candidates"].as_array().unwrap();
    let row = protected_rows
        .iter()
        .chain(candidate_rows.iter())
        .find(|row| row["name"].as_str() == Some("rt-1"))
        .unwrap();
    let binding_summary = &row["binding_summary"];

    let mut binding_keys = binding_summary
        .as_object()
        .expect("binding summary")
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    binding_keys.sort();
    assert_eq!(
        binding_keys,
        vec![
            "active_root_binding",
            "change_id",
            "change_status",
            "task_id",
            "task_status",
        ]
    );
}

#[test]
fn native_worktree_removal_surfaces_share_yes_and_dry_run_contract() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    let confirmation_error =
        "Pass --yes to apply this destructive worktree operation, or use --dry-run to preview it.";
    let failed = |args: &[&str]| {
        let output = command_output_with_env(root, args, &[]);
        assert!(
            !output.status.success(),
            "command unexpectedly succeeded: {args:?}\nstdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    };

    let cleanup_error = failed(&["worktree", "cleanup", "--json"]);
    assert!(cleanup_error.contains(confirmation_error), "{cleanup_error}");
    let cleanup_preview = json_output(root, &["worktree", "cleanup", "--dry-run", "--json"]);
    assert_eq!(cleanup_preview["dry_run"].as_bool(), Some(true));

    let prune_path = init_registered_worktree(
        root,
        "rt-prune-confirm",
        "feature/rt-prune-confirm",
        None,
        None,
        false,
        Some("manual_only"),
    );
    fs::remove_dir_all(&prune_path).unwrap();
    let prune_registry = root.join(".ait/worktrees/rt-prune-confirm.json");
    let prune_error = failed(&["worktree", "prune-stale", "--json"]);
    assert!(prune_error.contains(confirmation_error), "{prune_error}");
    assert!(prune_registry.is_file());
    let prune_preview = json_output(
        root,
        &["worktree", "prune-stale", "--dry-run", "--json"],
    );
    assert_eq!(prune_preview["dry_run"].as_bool(), Some(true));
    assert!(prune_registry.is_file());
    let pruned = json_output(
        root,
        &["worktree", "prune-stale", "--yes", "--json"],
    );
    assert_eq!(pruned["pruned_count"].as_i64(), Some(1));
    assert!(!prune_registry.exists());

    let remove_path = init_registered_worktree(
        root,
        "rt-remove-confirm",
        "feature/rt-remove-confirm",
        None,
        None,
        false,
        Some("manual_only"),
    );
    let remove_registry = root.join(".ait/worktrees/rt-remove-confirm.json");
    let remove_error = failed(&[
        "worktree",
        "remove",
        "rt-remove-confirm",
        "--delete-path",
        "--force",
        "--json",
    ]);
    assert!(remove_error.contains(confirmation_error), "{remove_error}");
    assert!(remove_path.is_dir());
    assert!(remove_registry.is_file());
    let remove_preview = json_output(
        root,
        &[
            "worktree",
            "remove",
            "rt-remove-confirm",
            "--delete-path",
            "--force",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(remove_preview["dry_run"].as_bool(), Some(true));
    assert!(remove_path.is_dir());
    assert!(remove_registry.is_file());
    let removed = json_output(
        root,
        &[
            "worktree",
            "remove",
            "rt-remove-confirm",
            "--delete-path",
            "--force",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(removed["removed_count"].as_i64(), Some(1));
    assert!(!remove_path.exists());
    assert!(!remove_registry.exists());

    let all_stale_path = init_registered_worktree(
        root,
        "rt-all-stale-confirm",
        "feature/rt-all-stale-confirm",
        None,
        None,
        false,
        Some("manual_only"),
    );
    fs::remove_dir_all(&all_stale_path).unwrap();
    let all_stale_registry = root.join(".ait/worktrees/rt-all-stale-confirm.json");
    let all_stale_error = failed(&["worktree", "remove", "--all-stale", "--json"]);
    assert!(
        all_stale_error.contains(confirmation_error),
        "{all_stale_error}"
    );
    assert!(all_stale_registry.is_file());
    let all_stale_preview = json_output(
        root,
        &[
            "worktree",
            "remove",
            "--all-stale",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(all_stale_preview["dry_run"].as_bool(), Some(true));
    assert!(all_stale_registry.is_file());
    let all_stale_removed = json_output(
        root,
        &[
            "worktree",
            "remove",
            "--all-stale",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(all_stale_removed["pruned_count"].as_i64(), Some(1));
    assert!(!all_stale_registry.exists());
}

#[test]
fn native_worktree_doctor_defaults_to_metadata_and_refreshes_on_demand() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    let worktree_path = init_registered_worktree(
        root,
        "rt-doctor",
        "feature/rt-doctor",
        Some("RT-DOCTOR"),
        Some("RC-DOCTOR"),
        true,
        Some("after_remote_land"),
    );
    write_file(
        &worktree_path.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"ok\" }\n",
    );
    let metadata_path = root.join(".ait/worktrees/rt-doctor.json");
    let before: JsonValue =
        parse_json_file(&metadata_path);
    assert!(before.get("workspace_status_cache").is_none());

    let metadata_payload = json_output(root, &["worktree", "doctor", "--json"]);
    assert_eq!(metadata_payload["refresh_status"].as_bool(), Some(false));
    assert_eq!(metadata_payload["status_mode"].as_str(), Some("metadata"));
    let metadata_row = metadata_payload["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].as_str() == Some("rt-doctor"))
        .unwrap();
    assert_eq!(metadata_row["status_source"].as_str(), Some("unverified"));
    assert_eq!(metadata_row["workspace_status"].as_str(), Some("unknown"));
    let after_metadata: JsonValue =
        parse_json_file(&metadata_path);
    assert!(after_metadata.get("workspace_status_cache").is_none());

    let refreshed_payload = json_output(root, &["worktree", "doctor", "--refresh", "--json"]);
    assert_eq!(refreshed_payload["refresh_status"].as_bool(), Some(true));
    assert_eq!(refreshed_payload["status_mode"].as_str(), Some("verified"));
    let refreshed_row = refreshed_payload["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].as_str() == Some("rt-doctor"))
        .unwrap();
    assert_eq!(refreshed_row["status_source"].as_str(), Some("verified"));
    assert_eq!(refreshed_row["workspace_status"].as_str(), Some("clean"));
    let after_refresh: JsonValue =
        parse_json_file(&metadata_path);
    assert!(after_refresh.get("workspace_status_cache").is_some());
}

#[test]
fn native_worktree_refresh_reports_missing_registered_paths() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    let worktree_path = init_registered_worktree(
        root,
        "rt-missing",
        "feature/rt-missing",
        Some("RT-MISSING"),
        Some("RC-MISSING"),
        true,
        Some("after_remote_land"),
    );
    fs::create_dir_all(root.join(".venv")).unwrap();
    fs::remove_dir_all(&worktree_path).unwrap();

    let list_payload = json_output(root, &["worktree", "list", "--refresh", "--json"]);
    let missing_row = list_payload
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].as_str() == Some("rt-missing"))
        .unwrap();
    assert_eq!(missing_row["exists"].as_bool(), Some(false));
    assert_eq!(missing_row["status_source"].as_str(), Some("verified"));
    assert_eq!(missing_row["workspace_status"].as_str(), Some("missing"));

    let doctor_payload = json_output(root, &["worktree", "doctor", "--refresh", "--json"]);
    assert_eq!(doctor_payload["refresh_status"].as_bool(), Some(true));
    assert_eq!(doctor_payload["missing_count"].as_i64(), Some(1));
    assert_eq!(doctor_payload["stale_count"].as_i64(), Some(1));
    let stale_row = doctor_payload["stale_rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].as_str() == Some("rt-missing"))
        .unwrap();
    assert_eq!(stale_row["workspace_status"].as_str(), Some("missing"));

    let cleanup_payload = json_output(root, &["worktree", "cleanup", "--dry-run", "--json"]);
    assert_eq!(cleanup_payload["dry_run"].as_bool(), Some(true));
    assert_eq!(cleanup_payload["candidate_count"].as_i64(), Some(0));
    assert_eq!(cleanup_payload["planned_count"].as_i64(), Some(0));

    let prune_payload = json_output(root, &["worktree", "prune-stale", "--dry-run", "--json"]);
    assert_eq!(prune_payload["dry_run"].as_bool(), Some(true));
    assert_eq!(prune_payload["pruned_count"].as_i64(), Some(1));
    let pruned_row = prune_payload["pruned_rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].as_str() == Some("rt-missing"))
        .unwrap();
    assert_eq!(pruned_row["workspace_status"].as_str(), Some("missing"));
}

#[test]
fn native_worktree_refresh_treats_status_cache_persistence_as_best_effort() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    let _worktree_path = init_registered_worktree(
        root,
        "rt-cached",
        "feature/rt-cached",
        Some("RT-CACHED"),
        Some("RC-CACHED"),
        true,
        Some("after_remote_land"),
    );
    let metadata_path = root.join(".ait/worktrees/rt-cached.json");
    let mut metadata: JsonValue =
        parse_json_file(&metadata_path);
    metadata["name"] = json!("rt-cache-missing-key");
    write_file(
        &metadata_path,
        &(encode_json_pretty(&metadata) + "\n"),
    );

    let list_payload = json_output(root, &["worktree", "list", "--refresh", "--json"]);
    let cached_row = list_payload
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].as_str() == Some("rt-cache-missing-key"))
        .unwrap();
    assert_eq!(cached_row["exists"].as_bool(), Some(true));
    assert_eq!(cached_row["status_source"].as_str(), Some("verified"));
    assert!(matches!(
        cached_row["workspace_status"].as_str(),
        Some("clean" | "dirty")
    ));
    assert!(!root
        .join(".ait/worktrees/rt-cache-missing-key.json")
        .exists());
}

#[cfg(unix)]
#[test]
fn native_worktree_recreate_accepts_dangling_alias_for_registered_target() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    let worktree_path = init_registered_worktree(
        root,
        "rt-recreate",
        "feature/rt-recreate",
        Some("RT-RECREATE"),
        Some("RC-RECREATE"),
        true,
        Some("after_remote_land"),
    );
    let alias_path = root.join(".ait-worktree-links/rt-recreate");
    fs::create_dir_all(alias_path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&worktree_path, &alias_path).unwrap();
    let metadata_path = root.join(".ait/worktrees/rt-recreate.json");
    let mut metadata: JsonValue =
        parse_json_file(&metadata_path);
    metadata["alias_path"] = JsonValue::String(alias_path.display().to_string());
    write_file(
        &metadata_path,
        &(encode_json_pretty(&metadata) + "\n"),
    );
    fs::remove_dir_all(&worktree_path).unwrap();

    let recreate = json_output(
        root,
        &["worktree", "recreate", "rt-recreate", "--dry-run", "--json"],
    );

    assert_eq!(recreate["name"].as_str(), Some("rt-recreate"));
    assert_eq!(recreate["dry_run"].as_bool(), Some(true));
    assert_eq!(
        recreate["workspace_status_before"].as_str(),
        Some("missing")
    );
    assert_eq!(
        recreate["recreate"]["managed_alias_recreated"].as_bool(),
        Some(true)
    );
}

#[test]
fn native_status_json_ignores_the_retired_debug_environment() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(&root.join(".aitignore"), "local-secrets/.env\n");
    fs::create_dir_all(root.join("local-secrets")).unwrap();
    write_file(&root.join("local-secrets/.env"), "secret\n");

    let normal = command_output_with_env(root, &["status", "--json"], &[]);
    let retired = command_output_with_env(
        root,
        &["status", "--json"],
        &[(concat!("AIT_JSON_", "MODE"), "debug")],
    );
    assert!(normal.status.success());
    assert!(retired.status.success());
    assert_eq!(normal.stdout, retired.stdout);
    let status = parse_json_bytes(&retired.stdout);

    assert_eq!(status["workspace_dirty"].as_bool(), Some(true));
    assert_eq!(
        status["workspace_changed_paths_sample"],
        json!([".aitignore"])
    );
    for internal in [
        "pack_count",
        "packed_blob_count",
        "ignore_policy",
        "phase_timings_ms",
        "binary_db_authority_root",
        "objects_path",
        "refs_path",
    ] {
        assert!(status.get(internal).is_none(), "unexpected {internal}");
    }
}

#[cfg(feature = "perfetto-tracing")]
#[test]
fn native_status_perfetto_trace_names_cover_cache_walk_and_compare() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    let trace_path = root.join(".ait-runtime/status.perfetto.json");
    fs::create_dir_all(trace_path.parent().unwrap()).unwrap();
    let trace_text = trace_path.to_string_lossy().to_string();

    let status = json_output_with_env(
        root,
        &["status", "--json"],
        &[("AIT_PERFETTO_TRACE", trace_text.as_str())],
    );
    assert!(status.get("phase_timings_ms").is_none());
    let trace = parse_json_bytes(&fs::read(&trace_path).expect("status Perfetto trace"));
    let names = trace["traceEvents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["name"].as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "ait.cli.status.command",
        "ait.cli.status.read",
        "ait.cli.status.hash_cache_read",
        "ait.cli.status.workspace_walk",
        "ait.cli.status.metadata_cache_match",
        "ait.cli.workspace_delta.compare",
    ] {
        assert!(names.contains(expected), "missing Perfetto range {expected}");
    }
}

#[test]
fn native_status_treats_exact_generated_worktree_cargo_projection_as_parent_source() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let source_config = "# AIT source policy: canonical Cargo settings; task worktrees receive a managed projection.\n[build]\ntarget-dir = \".ait/cargo-target\"\nbuild-dir = \".ait/cargo-build/canonical\"\n\n[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n";
    write_file(
        &worktree.join(".cargo/config.toml"),
        source_config,
    );

    let snapshot_id = seed_snapshot(&worktree, "source-level cargo config");
    let worktrees = json_output(
        temp.path(),
        &["worktree", "list", "--refresh", "--json"],
    );
    let row = worktrees
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].as_str() == Some("rt-1"))
        .unwrap();
    assert_eq!(row["workspace_status"].as_str(), Some("clean"));

    let cargo_config_path = worktree.join(".cargo/config.toml");
    let generated_config = fs::read_to_string(&cargo_config_path).unwrap();
    assert!(generated_config.starts_with(
        "# Managed by ait: workspace-isolated final artifacts and intermediates.\n"
    ));
    assert!(generated_config.contains("cargo-target/task-workspaces/rt-1"));
    assert!(generated_config.contains("cargo-build/task-workspaces/rt-1"));
    assert!(generated_config.contains("task-workspaces/rt-1"));
    assert!(generated_config.contains(
        "[alias]\nmanaged-test = [\"test\", \"--profile\", \"ait-ci\"]\n"
    ));

    let status = json_output(&worktree, &["status", "--json"]);

    assert_eq!(
        status["head_snapshot_id"].as_str(),
        Some(snapshot_id.as_str())
    );
    assert_eq!(status["workspace_status"].as_str(), Some("clean"));
    assert_eq!(status["workspace_dirty"].as_bool(), Some(false));
    assert_eq!(status["workspace_changed_paths_sample"], json!([]));
    assert_eq!(status["workspace_changed_count"].as_u64(), Some(0));

    let diff = json_output(&worktree, &["diff", "--json"]);
    assert_eq!(diff["clean"].as_bool(), Some(true));
    assert_eq!(diff["changed_paths"], json!([]));
    assert_eq!(diff["files"], json!([]));

    write_file(
        &cargo_config_path,
        &generated_config.replace("managed-test", "manually-edited-test"),
    );
    let edited = json_output(&worktree, &["status", "--json"]);
    assert_eq!(edited["workspace_dirty"].as_bool(), Some(true));
    assert_eq!(edited["workspace_modified_count"].as_u64(), Some(1));
    assert_eq!(
        edited["workspace_changed_paths_sample"],
        json!([".cargo/config.toml"])
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        write_file(&cargo_config_path, &generated_config);
        fs::set_permissions(&cargo_config_path, fs::Permissions::from_mode(0o600)).unwrap();
        let mode_drift = json_output(&worktree, &["status", "--json"]);
        assert_eq!(mode_drift["workspace_dirty"].as_bool(), Some(true));
        assert_eq!(mode_drift["workspace_modified_count"].as_u64(), Some(1));
        assert_eq!(
            mode_drift["workspace_changed_paths_sample"],
            json!([".cargo/config.toml"])
        );
    }

    handle.join().unwrap();
}

#[test]
fn native_remote_list_reports_configured_remotes() {
    let temp = init_repo("https://example.test");
    let root = temp.path();

    let remotes = json_output(root, &["remote", "list", "--json"]);
    let rows = remotes.as_array().unwrap();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["name"].as_str(), Some("origin"));
    assert_eq!(rows[0]["url"].as_str(), Some("https://example.test"));
    assert_eq!(rows[0]["repo_name"].as_str(), Some("fixture-ait"));
    assert_eq!(rows[0]["is_default_push"].as_i64(), Some(1));
    assert_eq!(rows[0]["is_default_pull"].as_i64(), Some(1));
}

fn configure_remote_patch_ci_snapshot(root: &Path) {
    write_file(
        &root.join("ci/patch_ci.json"),
        r#"{
  "schema_version": 1,
  "suites": [
    {
      "schema_version": 1,
      "suite_id": "fixture_unit",
      "display_name": "Fixture Unit",
      "plane": "patchset",
      "default_blocking": true,
      "mode": "gate",
      "purpose": "Validate the fixture before remote land.",
      "runner": {
        "kind": "command_bundle",
        "commands": ["true"]
      },
      "artifacts": {
        "log_path": ".ait/generated/ci/fixture_unit.log"
      }
    }
  ]
}
"#,
    );
    json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "configure fixture Patchset CI",
            "--json",
        ],
    );
}

#[test]
fn native_remote_add_default_persists_remote_config_and_validates_fixed_server_authority() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    let repo_directory_name = root
        .canonicalize()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let config_path = root.join(".ait/config.json");
    let mut config: JsonValue = parse_json_file(&config_path);
    config["workflow_mode"] = json!("solo_local");
    config["workflow_default_scope"] = json!("local");
    config["task_default_scope"] = json!("local");
    config["change_default_scope"] = json!("local");
    write_file(&config_path, &(encode_json_pretty(&config) + "\n"));
    configure_remote_patch_ci_snapshot(root);

    let added = json_output(
        root,
        &["remote", "add", "mirror", &base_url, "--default", "--json"],
    );

    assert_eq!(added["name"].as_str(), Some("mirror"));
    assert_eq!(added["url"].as_str(), Some(base_url.as_str()));
    assert_eq!(added["repo_name"].as_str(), Some(repo_directory_name.as_str()));
    assert_eq!(added["is_default_push"].as_i64(), Some(1));
    assert_eq!(added["is_default_pull"].as_i64(), Some(1));
    assert_eq!(added["patch_ci"]["status"], json!("ready"));
    assert_eq!(added["patch_ci"]["required"], json!(true));
    assert_eq!(
        added["patch_ci"]["manifest_path"],
        json!("ci/patch_ci.json")
    );
    assert_eq!(
        added["patch_ci"]["blocking_suite_ids"],
        json!(["fixture_unit"])
    );
    assert!(added["patch_ci"]["snapshot_id"].as_str().is_some());
    assert_eq!(added["agent_harness"]["status"], json!("synced"));
    assert_eq!(added["agent_harness"]["scope"], json!("local"));
    assert_eq!(
        added["agent_harness"]["plan_sync"]["results"][0]["artifact_path"],
        json!("AGENTS.md")
    );

    let remotes = json_output(root, &["remote", "list", "--json"]);
    let rows = remotes.as_array().unwrap();
    let origin = rows.iter().find(|row| row["name"] == "origin").unwrap();
    let mirror = rows.iter().find(|row| row["name"] == "mirror").unwrap();
    assert_eq!(origin["is_default_push"].as_i64(), Some(0));
    assert_eq!(origin["is_default_pull"].as_i64(), Some(0));
    assert_eq!(mirror["is_default_push"].as_i64(), Some(1));
    assert_eq!(mirror["is_default_pull"].as_i64(), Some(1));

    let config: JsonValue =
        parse_json_file(root.join(".ait/config.json"));
    assert_eq!(config["default_remote"].as_str(), Some("mirror"));

    let logged = log.lock().unwrap().clone();
    let authority_request = logged
        .iter()
        .find(|row| row.method == "GET" && row.url == "/v1/native/repository-authorities/7")
        .expect("remote add should validate the fixed Repository authority");
    assert!(authority_request.body.is_empty());
    assert!(!logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repositories"));
    handle.join().unwrap();
}

#[test]
fn native_remote_add_skips_patch_ci_bootstrap_when_tests_are_disabled() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();
    let repo_directory_name = root
        .canonicalize()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    write_file(
        &root.join(".ait/policy.yaml"),
        "version: 1\npolicy_id: prototype\ndefaults:\n  require_tests: false\n",
    );

    let added = json_output(
        root,
        &["remote", "add", "mirror", &base_url, "--json"],
    );

    assert_eq!(added["patch_ci"]["status"], json!("not_required"));
    assert_eq!(added["patch_ci"]["required"], json!(false));
    assert_eq!(added["repo_name"], json!(repo_directory_name));
    assert!(!root.join("ci/patch_ci.json").exists());
    let logged = log.lock().unwrap();
    assert!(logged.iter().any(|row| {
        row.method == "GET" && row.url == "/v1/native/repository-authorities/7"
    }));
    assert!(!logged
        .iter()
        .any(|row| row.method == "POST" && row.url == "/v1/native/repositories"));
    handle.join().unwrap();
}

#[test]
fn native_remote_add_generates_language_neutral_patch_ci_before_any_remote_contact() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    write_file(
        &root.join("pyproject.toml"),
        "[project]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
    );

    let output = cargo_bin()
        .current_dir(root)
        .args([
            "remote",
            "add",
            "mirror",
            "http://127.0.0.1:1",
            "--default",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Created a language-neutral ci/patch_ci.json starter."),
        "{stderr}"
    );
    assert!(
        stderr.contains("No project manifests were inspected and no validation command was inferred."),
        "{stderr}"
    );
    assert!(
        stderr.contains("Remote registration was not attempted."),
        "{stderr}"
    );
    assert!(stderr.contains("CONFIGURE_PATCHSET_TEST_COMMAND"), "{stderr}");
    assert!(!stderr.contains("python3 -m pytest"), "{stderr}");
    assert!(stderr.contains("ait snapshot create"), "{stderr}");
    assert!(
        stderr.contains("ait remote add mirror http://127.0.0.1:1 --default"),
        "{stderr}"
    );
    assert!(!stderr.contains("--repo-name"), "{stderr}");
    assert!(!stderr.contains("--discard-export"), "{stderr}");

    let generated = parse_json_file(root.join("ci/patch_ci.json"));
    assert_eq!(
        generated["suites"][0]["runner"]["commands"],
        json!(["CONFIGURE_PATCHSET_TEST_COMMAND"])
    );
    let generated_bytes = fs::read(root.join("ci/patch_ci.json")).unwrap();

    let retry = cargo_bin()
        .current_dir(root)
        .args([
            "remote",
            "add",
            "mirror",
            "http://127.0.0.1:1",
            "--default",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!retry.status.success());
    let retry_stderr = String::from_utf8_lossy(&retry.stderr);
    assert!(
        retry_stderr.contains("still contains the generated placeholder"),
        "{retry_stderr}"
    );
    assert!(
        retry_stderr.contains("existing configuration was not overwritten"),
        "{retry_stderr}"
    );
    assert_eq!(
        fs::read(root.join("ci/patch_ci.json")).unwrap(),
        generated_bytes
    );

    let remotes = json_output(root, &["remote", "list", "--json"]);
    assert!(remotes
        .as_array()
        .unwrap()
        .iter()
        .all(|row| row["name"] != "mirror"));
}

#[test]
fn native_remote_add_does_not_persist_when_server_ensure_fails() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    let repo_directory_name = root
        .canonicalize()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    configure_remote_patch_ci_snapshot(root);

    let mut command = cargo_bin();
    let output = command
        .current_dir(root)
        .args([
            "remote",
            "add",
            "mirror",
            "http://127.0.0.1:1",
            "--default",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "Remote repository {repo_directory_name} could not be ensured"
        )),
        "{stderr}"
    );

    let remotes = json_output(root, &["remote", "list", "--json"]);
    let rows = remotes.as_array().unwrap();
    assert!(rows
        .iter()
        .all(|row| row["name"].as_str() != Some("mirror")));
    let origin = rows.iter().find(|row| row["name"] == "origin").unwrap();
    assert_eq!(origin["is_default_push"].as_i64(), Some(1));
    assert_eq!(origin["is_default_pull"].as_i64(), Some(1));

    let config: JsonValue =
        parse_json_file(root.join(".ait/config.json"));
    assert_eq!(config["default_remote"].as_str(), Some("origin"));
}

#[test]
fn native_repo_show_uses_fixed_authority() {
    let (base_url, log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let shown = json_output(root, &["repo", "show", "--json"]);
    assert_eq!(
        shown["contract"].as_str(),
        Some("ait.server.repository-authority.v1")
    );
    assert_eq!(shown["repository"]["repository_index"].as_u64(), Some(7));
    assert_eq!(
        shown["repository"]["repository_name"].as_str(),
        Some("fixture-ait")
    );
    assert_eq!(shown["repository"]["namespace"].as_str(), Some(""));

    let logged = log.lock().unwrap().clone();
    assert!(logged
        .iter()
        .any(|row| row.method == "GET" && row.url == "/v1/native/repository-authorities/7"));
    assert!(!logged.iter().any(|row| row.url.contains("/native/admin/repositories")));
    handle.join().unwrap();
}

#[test]
fn native_repository_text_is_decision_complete_without_inline_json() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let repo = command_output_with_env(root, &["repo", "show"], &[]);
    assert!(repo.status.success());
    let repo = String::from_utf8_lossy(&repo.stdout);
    assert!(repo.contains("ait repo show\n"));
    assert!(repo.contains("repository: fixture-ait (#7)"));
    assert!(repo.contains("storage: valid (0 errors)"));
    assert!(repo.contains("details: ait repo show --json"));
    assert!(!repo.contains("{\""));

    let capabilities = command_output_with_env(root, &["repo", "ci-capabilities"], &[]);
    assert!(capabilities.status.success());
    let capabilities = String::from_utf8_lossy(&capabilities.stdout);
    assert!(capabilities.contains("remote sync: 2/2 required ready"));
    assert!(capabilities.contains("pull manifest unavailable (optional)"));
    assert!(capabilities.contains("decision: Patchset CI submission and zstd remote sync are ready"));
    assert!(capabilities.contains("details: ait repo ci-capabilities --json"));
    assert!(!capabilities.contains("{\""));

    let jobs = command_output_with_env(root, &["repo", "jobs"], &[]);
    assert!(jobs.status.success());
    let jobs = String::from_utf8_lossy(&jobs.stdout);
    assert!(jobs.contains("state: all returned jobs succeeded"));
    assert!(jobs.contains("#77\tpatchset.ci\tsucceeded"));
    assert!(jobs.contains("details: ait repo jobs --limit 50 --json"));
    assert!(!jobs.contains("{\""));

    handle.join().unwrap();
}

#[test]
fn native_auth_whoami_local_fallback_exposes_identity_only() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();

    let whoami = json_output(
        root,
        &["auth", "whoami", "--repo", "fixture-auth", "--json"],
    );

    assert_eq!(whoami["identity"].as_str(), Some("fixture@example.com"));
    assert_eq!(whoami["mode"].as_str(), Some("open"));
    assert_eq!(whoami["repo_name"].as_str(), Some("fixture-auth"));
    assert!(whoami.get("actor_type").is_none());
    assert!(whoami.get("claimed_roles").is_none());
    assert!(whoami.get("claimed_repos").is_none());
    assert!(whoami.get("effective_roles").is_none());
    assert!(whoami.get("effective_repos").is_none());
}

#[test]
fn native_queue_summary_json_covers_empty_local_inventory() {
    let temp = init_repo_without_workflow_rows("http://127.0.0.1:1");
    let root = temp.path();
    disable_default_remote(root);

    let payload = json_output(root, &["queue", "summary", "--json"]);

    assert_eq!(payload["repo_name"].as_str(), Some("fixture-ait"));
    assert!(payload.get("query").is_none());
    assert_eq!(payload["remote"]["configured"].as_bool(), Some(false));
    assert_eq!(payload["remote"]["remote_name"], JsonValue::Null);
    assert_eq!(payload["remote"]["available_remotes"], json!(["origin"]));
    assert_eq!(
        payload["remote"]["error"].as_str(),
        Some(
            "No default remote configured. Set one first, or pass --remote <name> for this queue read."
        )
    );
    assert_eq!(payload["local"]["tasks"], json!([]));
    assert_eq!(payload["local"]["changes"], json!([]));
    assert!(payload["local"].get("all_tasks").is_none());
    assert!(payload["local"].get("all_changes").is_none());
    assert_eq!(payload["local"]["summary"]["task_record_count"], json!(0));
    assert_eq!(payload["local"]["summary"]["change_record_count"], json!(0));
    assert_eq!(payload["summary"]["local_draft_task_count"], json!(0));
    assert_eq!(payload["summary"]["local_draft_change_count"], json!(0));
    assert_eq!(payload["summary"]["workspace_dirty"], json!(false));
}

#[test]
fn native_queue_summary_json_reports_local_binary_workflow_authority_in_solo_remote() {
    let temp = init_repo("http://127.0.0.1:1");
    let root = temp.path();
    disable_default_remote(root);

    let payload = json_output(root, &["queue", "summary", "--json"]);

    assert_eq!(payload["local"]["available"].as_bool(), Some(true));
    assert_eq!(
        payload["local"]["authority"].as_str(),
        Some("local_binary_v0")
    );
    assert_eq!(payload["local"]["summary"]["task_record_count"], json!(0));
    assert_eq!(payload["local"]["summary"]["change_record_count"], json!(0));
    assert_eq!(payload["local"]["summary"]["draft_task_count"], json!(0));
    assert_eq!(payload["local"]["summary"]["published_task_count"], json!(0));
    assert_eq!(payload["local"]["summary"]["draft_change_count"], json!(0));
    assert_eq!(
        payload["local"]["summary"]["published_change_count"],
        json!(0)
    );
    assert_eq!(
        payload["local"]["summary"]["unpublished_change_record_count"],
        json!(0)
    );
    assert_eq!(payload["summary"]["local_draft_task_count"], json!(0));
    assert_eq!(payload["summary"]["local_draft_change_count"], json!(0));
    assert_eq!(payload["local"]["tasks"], json!([]));
    assert_eq!(payload["local"]["changes"], json!([]));
    assert_eq!(
        payload["workspace"]["status"]["clean"].as_bool(),
        Some(true)
    );
}

#[test]
fn native_queue_summary_json_falls_back_when_remote_bundle_is_missing() {
    let (base_url, log, handle) = spawn_queue_summary_fallback_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let payload = json_output(root, &["queue", "summary", "--json"]);

    assert_eq!(payload["remote"]["configured"].as_bool(), Some(true));
    assert_eq!(payload["remote"]["remote_name"].as_str(), Some("origin"));
    assert_eq!(payload["remote"]["error"], JsonValue::Null);
    assert_eq!(payload["remote"]["task_queue"]["count"], json!(2));
    assert_eq!(payload["remote"]["reviewer_inbox"]["count"], json!(1));
    assert_eq!(payload["summary"]["shared_task_count"], json!(2));
    assert_eq!(payload["summary"]["attention_required_count"], json!(1));
    assert_eq!(payload["summary"]["ready_to_land_count"], json!(1));
    assert_eq!(payload["summary"]["reviewer_inbox_count"], json!(1));
    assert!(payload["summary"].get("open_shared_change_count").is_none());
    assert!(payload["remote"].get("changes").is_none());

    handle.join().unwrap();
    let urls = log
        .lock()
        .unwrap()
        .iter()
        .map(|row| row.url.clone())
        .collect::<Vec<_>>();
    assert_eq!(urls.len(), 3);
    assert!(urls[0].starts_with(
        "/v1/native/repository-authorities/7/read/queue-summary?"
    ));
    assert!(urls[1].starts_with(
        "/v1/native/repository-authorities/7/read/task-queue?"
    ));
    assert_eq!(
        urls[2],
        "/v1/native/repository-authorities/7/read/reviewer-inbox"
    );
}

#[test]
fn native_pull_line_imports_remote_snapshot_and_moves_local_line_head() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    let remote_source = "pub fn example() -> &'static str { \"pulled\" }\n";
    json_output(
        remote_root,
        &["line", "create", "pulled", "--switch", "--json"],
    );
    write_file(&remote_root.join("src/lib.rs"), remote_source);
    let remote_snapshot_id = seed_snapshot(remote_root, "remote pulled update");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_snapshot_id);
    let (base_url, log, handle) = spawn_remote_import_server(
        "pulled",
        &remote_snapshot_id,
        remote_zstd,
    );
    let temp = init_repo(&base_url);
    let root = temp.path();
    assert_fixture_repo_is_zstd_only_compatible(root);
    let workspace_before = fs::read_to_string(root.join("src/lib.rs")).unwrap();

    let pulled = json_output(root, &["pull", "--line", "pulled", "--json"]);

    assert_eq!(pulled["mode"].as_str(), Some("line"));
    assert_eq!(pulled["line"].as_str(), Some("pulled"));
    assert_eq!(pulled["relationship"].as_str(), Some("new_remote_line"));
    assert_eq!(pulled["action"].as_str(), Some("fast_forward"));
    assert_eq!(
        pulled["head_snapshot_id"].as_str(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(pulled["imported_snapshots"].as_i64(), Some(1));
    assert_eq!(
        pulled["imported_snapshot_ids"],
        json!([remote_snapshot_id.clone()])
    );
    assert_eq!(pulled["local_line_present"].as_bool(), Some(false));
    assert_eq!(pulled["line_head_updated"].as_bool(), Some(true));
    assert_eq!(pulled["workspace_restored"].as_bool(), Some(false));
    assert!(pulled.get("phase_timings_ms").is_none());
    assert!(pulled.get("remote_sync_metrics").is_none());
    assert_eq!(
        local_line_head(root, "pulled").as_deref(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(
        fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        workspace_before
    );
    let line = json_output(root, &["line", "show", "pulled", "--json"]);
    assert_eq!(
        line["head_snapshot_id"].as_str(),
        Some(remote_snapshot_id.as_str())
    );
    let logged = log.lock().unwrap().clone();
    assert!(logged
        .iter()
        .any(|row| row.method == "GET"
            && row.url == "/v1/native/repository-authorities/7/lines/pulled"));
    assert_zstd_snapshot_download_logged(&logged, &remote_snapshot_id);
    assert!(!logged.iter().any(|row| row.method == "PUT"));
    handle.join().unwrap();
}

#[test]
fn native_pull_json_ignores_the_retired_debug_environment() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    json_output(
        remote_root,
        &["line", "create", "pulled", "--switch", "--json"],
    );
    write_file(
        &remote_root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"pull-debug\" }\n",
    );
    let remote_snapshot_id = seed_snapshot(remote_root, "remote debug pull");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_snapshot_id);
    let (base_url, _log, handle) =
        spawn_remote_import_server("pulled", &remote_snapshot_id, remote_zstd);
    let temp = init_repo(&base_url);
    let output = json_output_with_env(
        temp.path(),
        &["pull", "--line", "pulled", "--json"],
        &[(concat!("AIT_JSON_", "MODE"), "debug")],
    );
    assert_eq!(output["line"].as_str(), Some("pulled"));
    assert!(output.get("phase_timings_ms").is_none());
    assert!(output.get("remote_sync_metrics").is_none());
    handle.join().unwrap();
}

#[cfg(feature = "perfetto-tracing")]
#[test]
fn native_pull_perfetto_trace_names_cover_import_download_and_head_movement() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    json_output(
        remote_root,
        &["line", "create", "pulled", "--switch", "--json"],
    );
    write_file(
        &remote_root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"pull-perfetto\" }\n",
    );
    let remote_snapshot_id = seed_snapshot(remote_root, "remote Perfetto pull");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_snapshot_id);
    let (base_url, _log, handle) =
        spawn_remote_import_server("pulled", &remote_snapshot_id, remote_zstd);
    let temp = init_repo(&base_url);
    let root = temp.path();
    let trace_path = root.join(".ait-runtime/pull.perfetto.json");
    fs::create_dir_all(trace_path.parent().unwrap()).unwrap();
    let trace_text = trace_path.to_string_lossy().to_string();
    let _ = json_output_with_env(
        root,
        &["pull", "--line", "pulled", "--json"],
        &[("AIT_PERFETTO_TRACE", trace_text.as_str())],
    );
    handle.join().unwrap();
    let trace = parse_json_bytes(&fs::read(&trace_path).expect("pull Perfetto trace"));
    let names = trace["traceEvents"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|event| event["name"].as_str())
        .collect::<BTreeSet<_>>();
    for expected in [
        "ait.remote_sync.pull",
        "ait.remote_sync.pull.line_read",
        "ait.remote_sync.pull.import_chain",
        "ait.remote_sync.pull.manifest_ancestry",
        "ait.remote_sync.pull.pack_download_pipeline",
        "ait.remote_sync.pull.metadata_import",
        "ait.remote_sync.pull.ancestry_relationship",
        "ait.remote_sync.pull.head_movement",
    ] {
        assert!(names.contains(expected), "missing Perfetto range {expected}");
    }
}

#[test]
fn native_pull_line_with_restore_materializes_workspace_and_switches_line() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    let remote_source = "pub fn example() -> &'static str { \"restored pull\" }\n";
    json_output(
        remote_root,
        &["line", "create", "pulled", "--switch", "--json"],
    );
    write_file(&remote_root.join("src/lib.rs"), remote_source);
    let remote_snapshot_id = seed_snapshot(remote_root, "remote restored pull");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_snapshot_id);
    let (base_url, log, handle) = spawn_remote_import_server(
        "pulled",
        &remote_snapshot_id,
        remote_zstd,
    );
    let temp = init_repo(&base_url);
    let root = temp.path();
    assert_fixture_repo_is_zstd_only_compatible(root);

    let pulled = json_output(root, &["pull", "--line", "pulled", "--restore", "--json"]);

    assert_eq!(pulled["mode"].as_str(), Some("line"));
    assert_eq!(pulled["line"].as_str(), Some("pulled"));
    assert_eq!(pulled["workspace_restored"].as_bool(), Some(true));
    assert_eq!(pulled["restore_applied"].as_bool(), Some(true));
    assert_eq!(
        pulled["head_snapshot_id"].as_str(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(
        pulled["restore"]["target_snapshot_id"].as_str(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(
        pulled["restore"]["baseline_snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        pulled["restore"]["current_line_before"].as_str(),
        Some("main")
    );
    assert_eq!(pulled["restore"]["current_line"].as_str(), Some("pulled"));
    assert_eq!(
        local_line_head(root, "pulled").as_deref(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(
        fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        remote_source
    );
    let status = json_output(root, &["status", "--json"]);
    assert_eq!(status["current_line"].as_str(), Some("pulled"));
    let logged = log.lock().unwrap().clone();
    assert_zstd_snapshot_download_logged(&logged, &remote_snapshot_id);
    handle.join().unwrap();
}

#[test]
fn native_pull_restore_rejects_dirty_workspace_without_moving_line_head() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    let remote_source = "pub fn example() -> &'static str { \"dirty protected\" }\n";
    write_file(&remote_root.join("src/lib.rs"), remote_source);
    let remote_snapshot_id = seed_snapshot(remote_root, "remote dirty protected");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_snapshot_id);
    let (base_url, log, handle) = spawn_remote_import_server(
        "main",
        &remote_snapshot_id,
        remote_zstd,
    );
    let temp = init_repo(&base_url);
    let root = temp.path();
    assert_fixture_repo_is_zstd_only_compatible(root);
    let line_head_before = local_line_head(root, "main");
    let dirty_source = "pub fn example() -> &'static str { \"local dirty\" }\n";
    write_file(&root.join("src/lib.rs"), dirty_source);

    let output = cargo_bin()
        .current_dir(root)
        .args(["pull", "--line", "main", "--restore", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(&format!(
            "Workspace has unsaved changes relative to {FIXTURE_BASE_SNAPSHOT_ID}"
        )),
        "stderr:\n{stderr}"
    );
    assert_eq!(local_line_head(root, "main"), line_head_before);
    assert_eq!(
        fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        dirty_source
    );
    let logged = log.lock().unwrap().clone();
    assert_zstd_snapshot_download_logged(&logged, &remote_snapshot_id);
    handle.join().unwrap();
}

#[test]
fn native_pull_restore_force_overwrites_dirty_workspace() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    let remote_source = "pub fn example() -> &'static str { \"force restored\" }\n";
    write_file(&remote_root.join("src/lib.rs"), remote_source);
    let remote_snapshot_id = seed_snapshot(remote_root, "remote force restored");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_snapshot_id);
    let (base_url, log, handle) = spawn_remote_import_server(
        "main",
        &remote_snapshot_id,
        remote_zstd,
    );
    let temp = init_repo(&base_url);
    let root = temp.path();
    assert_fixture_repo_is_zstd_only_compatible(root);
    write_file(
        &root.join("src/lib.rs"),
        "pub fn example() -> &'static str { \"local dirty\" }\n",
    );

    let pulled = json_output(
        root,
        &["pull", "--line", "main", "--restore", "--force", "--json"],
    );

    assert_eq!(pulled["workspace_restored"].as_bool(), Some(true));
    assert_eq!(pulled["restore"]["force"].as_bool(), Some(true));
    assert_eq!(
        pulled["restore"]["target_snapshot_id"].as_str(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(
        local_line_head(root, "main").as_deref(),
        Some(remote_snapshot_id.as_str())
    );
    assert_eq!(
        fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        remote_source
    );
    let logged = log.lock().unwrap().clone();
    assert_zstd_snapshot_download_logged(&logged, &remote_snapshot_id);
    handle.join().unwrap();
}
