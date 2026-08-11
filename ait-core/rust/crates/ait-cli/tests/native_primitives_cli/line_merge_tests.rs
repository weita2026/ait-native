use ait_core::local_snapshot::LocalSnapshotWriteStore;

fn prepare_divergent_text_lines(
    worktree: &Path,
    target_content: &str,
    source_content: &str,
) -> (String, String) {
    write_file(&worktree.join("src/lib.rs"), target_content);
    let target = json_output(
        worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "target side",
            "--json",
        ],
    );
    let target_snapshot_id = target["snapshot_id"].as_str().unwrap().to_string();

    let repo_root = worktree.parent().expect("fixture repo root");
    seed_binary_line(repo_root, "feature/source", FIXTURE_BASE_SNAPSHOT_ID);
    json_output(
        worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--force",
            "--json",
        ],
    );
    write_file(&worktree.join("src/lib.rs"), source_content);
    let repo = RepoRuntime::discover_from_path(worktree).expect("worktree runtime");
    let store = repo
        .local_snapshot_operation_store::<1>(worktree)
        .expect("snapshot operation store");
    let source = store
        .create_snapshot_with_parents(
            &repo.repo_name(),
            "feature/source",
            &[FIXTURE_BASE_SNAPSHOT_ID.to_string()],
            Some("source side"),
            true,
        )
        .expect("source snapshot");
    let source_snapshot_id = source["snapshot_id"].as_str().unwrap().to_string();
    json_output(
        worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            &target_snapshot_id,
            "--force",
            "--json",
        ],
    );
    (target_snapshot_id, source_snapshot_id)
}

fn merge_worktree_metadata(repo_root: &Path) -> JsonValue {
    parse_json_file(repo_root.join(".ait/worktrees/rt-1.json"))
}

#[test]
fn native_line_merge_distinguishes_fast_forward_equal_and_already_contains() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = temp.path();
    seed_binary_line(repo_root, "feature/source", FIXTURE_BASE_SNAPSHOT_ID);
    json_output(
        &worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--force",
            "--json",
        ],
    );
    write_file(&worktree.join("source-only.txt"), "source ahead\n");
    let repo = RepoRuntime::discover_from_path(&worktree).expect("worktree runtime");
    let source = repo
        .local_snapshot_operation_store::<1>(&worktree)
        .unwrap()
        .create_snapshot_with_parents(
            &repo.repo_name(),
            "feature/source",
            &[FIXTURE_BASE_SNAPSHOT_ID.to_string()],
            Some("source ahead"),
            true,
        )
        .expect("source snapshot");
    let source_snapshot_id = source["snapshot_id"].as_str().unwrap().to_string();
    json_output(
        &worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--force",
            "--json",
        ],
    );

    let fast_forward = json_output(
        &worktree,
        &["line", "merge", "feature/source", "--json"],
    );
    assert_eq!(fast_forward["status"], json!("fast_forward"));
    assert_eq!(fast_forward["merge_snapshot_created"], json!(false));
    assert!(fast_forward["merge_snapshot_id"].is_null());
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(source_snapshot_id.as_str())
    );
    assert_eq!(
        fs::read_to_string(worktree.join("source-only.txt")).unwrap(),
        "source ahead\n"
    );

    let equal = json_output(
        &worktree,
        &["line", "merge", "feature/source", "--json"],
    );
    assert_eq!(equal["status"], json!("already_equal"));
    assert_eq!(equal["merge_snapshot_created"], json!(false));

    write_file(&worktree.join("target-only.txt"), "target ahead\n");
    let target = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "target ahead",
            "--json",
        ],
    );
    let target_snapshot_id = target["snapshot_id"].as_str().unwrap().to_string();
    let already_contains = json_output(
        &worktree,
        &["line", "merge", "feature/source", "--json"],
    );
    assert_eq!(
        already_contains["status"],
        json!("already_contains_source")
    );
    assert_eq!(already_contains["merge_snapshot_created"], json!(false));
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(target_snapshot_id.as_str())
    );

    handle.join().unwrap();
}

#[test]
fn native_line_merge_creates_ordered_two_parent_snapshot_for_clean_divergence() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = temp.path();

    write_file(
        &worktree.join("target-only.txt"),
        "content from the target side\n",
    );
    let target = json_output(
        &worktree,
        &[
            "snapshot",
            "create",
            "--message",
            "target side",
            "--json",
        ],
    );
    let target_snapshot_id = target["snapshot_id"].as_str().unwrap().to_string();

    seed_binary_line(repo_root, "feature/source", FIXTURE_BASE_SNAPSHOT_ID);
    json_output(
        &worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--force",
            "--json",
        ],
    );
    write_file(
        &worktree.join("source-only.txt"),
        "content from the source side\n",
    );
    let repo = RepoRuntime::discover_from_path(&worktree).expect("worktree runtime");
    let store = repo
        .local_snapshot_operation_store::<1>(&worktree)
        .expect("snapshot operation store");
    let source = store
        .create_snapshot_with_parents(
            &repo.repo_name(),
            "feature/source",
            &[FIXTURE_BASE_SNAPSHOT_ID.to_string()],
            Some("source side"),
            true,
        )
        .expect("source snapshot");
    let source_snapshot_id = source["snapshot_id"].as_str().unwrap().to_string();
    json_output(
        &worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            &target_snapshot_id,
            "--force",
            "--json",
        ],
    );

    let merged = json_output(
        &worktree,
        &[
            "line",
            "merge",
            "feature/source",
            "--message",
            "clean two-parent merge",
            "--json",
        ],
    );
    assert_eq!(merged["status"].as_str(), Some("merged"));
    assert_eq!(
        merged["parent_snapshot_ids"],
        json!([target_snapshot_id, source_snapshot_id])
    );
    let merge_snapshot_id = merged["merge_snapshot_id"]
        .as_str()
        .expect("merge snapshot id");
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(merge_snapshot_id)
    );

    let shown = json_output(
        &worktree,
        &["snapshot", "show", merge_snapshot_id, "--json"],
    );
    assert_eq!(
        shown["parent_snapshot_ids"],
        merged["parent_snapshot_ids"]
    );
    let merged_repo = RepoRuntime::discover_from_path(&worktree).expect("merged runtime");
    let paths = merged_repo
        .local_snapshot_operation_store::<1>(&worktree)
        .unwrap()
        .snapshot_tree_file_rows(Some(merge_snapshot_id))
        .unwrap()
        .into_iter()
        .map(|row| row.path)
        .collect::<BTreeSet<_>>();
    assert!(paths.contains("target-only.txt"));
    assert!(paths.contains("source-only.txt"));
    assert_eq!(merge_worktree_metadata(repo_root)["merge_state"], json!("idle"));

    handle.join().unwrap();
}

#[test]
fn native_line_merge_conflict_preserves_heads_blocks_bypass_and_continues() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = temp.path();
    let (target_snapshot_id, source_snapshot_id) = prepare_divergent_text_lines(
        &worktree,
        "pub fn side() -> &'static str { \"target\" }\n",
        "pub fn side() -> &'static str { \"source\" }\n",
    );

    let conflicted = json_output(
        &worktree,
        &["line", "merge", "feature/source", "--json"],
    );
    assert_eq!(conflicted["status"].as_str(), Some("conflicted"));
    assert_eq!(conflicted["conflict_paths"], json!(["src/lib.rs"]));
    assert_eq!(conflicted["conflict_kinds"]["src/lib.rs"], json!("text"));
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(target_snapshot_id.as_str())
    );
    assert_eq!(
        local_line_head(repo_root, "feature/source").as_deref(),
        Some(source_snapshot_id.as_str())
    );
    let marker = fs::read_to_string(worktree.join("src/lib.rs")).unwrap();
    assert!(marker.contains("<<<<<<< AIT target:"));
    assert!(marker.contains(">>>>>>> AIT source:"));

    let metadata = merge_worktree_metadata(repo_root);
    assert_eq!(metadata["merge_state"], json!("conflicted"));
    assert_eq!(metadata["merge_target_snapshot_id"], json!(target_snapshot_id));
    assert_eq!(metadata["merge_source_snapshot_id"], json!(source_snapshot_id));
    assert_eq!(
        metadata["merge_pre_workspace_snapshot_id"],
        metadata["merge_target_snapshot_id"]
    );
    let shown_worktree = json_output(&worktree, &["worktree", "show", "--json"]);
    assert_eq!(shown_worktree["merge_state"], json!("conflicted"));
    assert_eq!(shown_worktree["merge_conflict_count"], json!(1));
    assert_eq!(shown_worktree["merge"]["target_line"], json!("feature/rt-1"));
    assert_eq!(
        shown_worktree["merge"]["source_line"],
        json!("feature/source")
    );

    for args in [
        vec!["snapshot", "create", "--message", "bypass", "--json"],
        vec![
            "worktree",
            "restore",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--force",
            "--json",
        ],
        vec!["line", "switch", "main", "--json"],
    ] {
        let output = command_output_with_env(&worktree, &args, &[]);
        assert!(!output.status.success(), "{args:?} unexpectedly succeeded");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("active line merge"),
            "stderr for {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let unresolved = command_output_with_env(
        &worktree,
        &["line", "merge", "--continue", "--json"],
        &[],
    );
    assert!(!unresolved.status.success());
    assert!(String::from_utf8_lossy(&unresolved.stderr).contains("conflicts remain unresolved"));

    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn side() -> &'static str { \"resolved\" }\n",
    );
    seed_binary_line(repo_root, "feature/source", FIXTURE_BASE_SNAPSHOT_ID);
    let moved_parent = command_output_with_env(
        &worktree,
        &["line", "merge", "--continue", "--json"],
        &[],
    );
    assert!(!moved_parent.status.success());
    assert!(String::from_utf8_lossy(&moved_parent.stderr).contains("feature/source moved"));
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(target_snapshot_id.as_str())
    );
    seed_binary_line(repo_root, "feature/source", &source_snapshot_id);
    let continued = json_output(
        &worktree,
        &[
            "line",
            "merge",
            "--continue",
            "--message",
            "resolved merge",
            "--json",
        ],
    );
    assert_eq!(continued["status"].as_str(), Some("continued"));
    assert_eq!(
        continued["parent_snapshot_ids"],
        json!([target_snapshot_id, source_snapshot_id])
    );
    assert_eq!(merge_worktree_metadata(repo_root)["merge_state"], json!("idle"));
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        "pub fn side() -> &'static str { \"resolved\" }\n"
    );

    handle.join().unwrap();
}

#[test]
fn native_line_merge_abort_restores_exact_target_workspace_without_moving_head() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = temp.path();
    let target_content = "pub fn side() -> &'static str { \"target bytes\" }\n";
    let (target_snapshot_id, _source_snapshot_id) = prepare_divergent_text_lines(
        &worktree,
        target_content,
        "pub fn side() -> &'static str { \"source bytes\" }\n",
    );

    let conflicted = json_output(
        &worktree,
        &["line", "merge", "feature/source", "--json"],
    );
    assert_eq!(conflicted["status"].as_str(), Some("conflicted"));
    write_file(&worktree.join("introduced-during-merge.txt"), "remove me\n");

    let aborted = json_output(&worktree, &["line", "merge", "--abort", "--json"]);
    assert_eq!(aborted["status"].as_str(), Some("aborted"));
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(target_snapshot_id.as_str())
    );
    assert_eq!(
        fs::read_to_string(worktree.join("src/lib.rs")).unwrap(),
        target_content
    );
    assert!(!worktree.join("introduced-during-merge.txt").exists());
    let metadata = merge_worktree_metadata(repo_root);
    assert_eq!(metadata["merge_state"], json!("idle"));
    assert!(metadata.get("merge_target_snapshot_id").is_none());
    assert!(metadata.get("merge_plan").is_none());

    handle.join().unwrap();
}

#[test]
fn native_pull_local_ahead_imports_without_moving_the_line_or_workspace() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    let remote_zstd =
        zstd_remote_import_fixture_from_repo(remote_root, FIXTURE_BASE_SNAPSHOT_ID);
    let (base_url, _log, handle) =
        spawn_remote_import_server("main", FIXTURE_BASE_SNAPSHOT_ID, remote_zstd);
    let temp = init_repo(&base_url);
    let root = temp.path();
    write_file(&root.join("local-only.txt"), "local ahead\n");
    let local_head = seed_snapshot(root, "local ahead");
    let workspace_before = fs::read(root.join("local-only.txt")).unwrap();

    let pulled = json_output(root, &["pull", "--line", "main", "--json"]);

    assert_eq!(pulled["relationship"], json!("local_ahead"));
    assert_eq!(pulled["action"], json!("imported_only"));
    assert_eq!(pulled["line_head_updated"], json!(false));
    assert_eq!(pulled["workspace_restored"], json!(false));
    assert_eq!(
        local_line_head(root, "main").as_deref(),
        Some(local_head.as_str())
    );
    assert_eq!(fs::read(root.join("local-only.txt")).unwrap(), workspace_before);
    handle.join().unwrap();
}

#[test]
fn native_pull_remote_ahead_fast_forwards_then_equal_is_a_noop() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    write_file(&remote_root.join("remote-only.txt"), "remote ahead\n");
    let remote_head = seed_snapshot(remote_root, "remote ahead");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_head);
    let (base_url, _log, handle) =
        spawn_remote_import_server("main", &remote_head, remote_zstd);
    let temp = init_repo(&base_url);
    let root = temp.path();
    let workspace_before = fs::read(root.join("src/lib.rs")).unwrap();

    let fast_forward = json_output(root, &["pull", "--line", "main", "--json"]);
    assert_eq!(fast_forward["relationship"], json!("remote_ahead"));
    assert_eq!(fast_forward["action"], json!("fast_forward"));
    assert_eq!(fast_forward["line_head_updated"], json!(true));
    assert_eq!(
        local_line_head(root, "main").as_deref(),
        Some(remote_head.as_str())
    );
    assert_eq!(fs::read(root.join("src/lib.rs")).unwrap(), workspace_before);
    assert!(!root.join("remote-only.txt").exists());

    let equal = json_output(root, &["pull", "--line", "main", "--json"]);
    assert_eq!(equal["relationship"], json!("equal"));
    assert_eq!(equal["action"], json!("none"));
    assert_eq!(equal["line_head_updated"], json!(false));
    assert_eq!(equal["imported_snapshots"], json!(0));
    assert!(!root.join("remote-only.txt").exists());
    handle.join().unwrap();
}

#[test]
fn native_pull_divergence_without_strategy_imports_then_fails_closed() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    json_output(
        remote_root,
        &[
            "line",
            "create",
            "feature/rt-1",
            "--switch",
            "--restore",
            "--json",
        ],
    );
    write_file(&remote_root.join("remote-only.txt"), "remote side\n");
    let remote_head = seed_snapshot(remote_root, "remote divergent side");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_head);
    let (base_url, _log, handle) =
        spawn_remote_import_server("feature/rt-1", &remote_head, remote_zstd);
    let (temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = temp.path();
    json_output(
        &worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--force",
            "--json",
        ],
    );
    write_file(&worktree.join("local-only.txt"), "local side\n");
    let target_head = json_output(
        &worktree,
        &["snapshot", "create", "--message", "local divergent side", "--json"],
    )["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();

    let output = command_output_with_env(
        &worktree,
        &["pull", "--line", "feature/rt-1", "--json"],
        &[],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("have diverged"), "{stderr}");
    assert!(stderr.contains("ait pull --remote origin --line feature/rt-1 --merge --restore"));
    assert!(stderr.contains("ait worktree rebase --onto remote/origin/feature/rt-1"));
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(target_head.as_str())
    );
    assert!(worktree.join("local-only.txt").exists());
    assert!(!worktree.join("remote-only.txt").exists());
    let imported = json_output(&worktree, &["snapshot", "show", &remote_head, "--json"]);
    assert_eq!(imported["snapshot_id"], json!(remote_head));
    handle.join().unwrap();
}

#[test]
fn native_pull_merge_creates_ordered_two_parent_snapshot_for_divergence() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    json_output(
        remote_root,
        &[
            "line",
            "create",
            "feature/rt-1",
            "--switch",
            "--restore",
            "--json",
        ],
    );
    write_file(&remote_root.join("remote-only.txt"), "remote side\n");
    let remote_head = seed_snapshot(remote_root, "remote clean side");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_head);
    let (base_url, _log, handle) =
        spawn_remote_import_server("feature/rt-1", &remote_head, remote_zstd);
    let (temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = temp.path();
    json_output(
        &worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--force",
            "--json",
        ],
    );
    write_file(&worktree.join("local-only.txt"), "local side\n");
    let target_head = json_output(
        &worktree,
        &["snapshot", "create", "--message", "local clean side", "--json"],
    )["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();

    let pulled = json_output(
        &worktree,
        &["pull", "--line", "feature/rt-1", "--merge", "--json"],
    );

    assert_eq!(pulled["relationship"], json!("divergent"));
    assert_eq!(pulled["action"], json!("merged"));
    assert_eq!(pulled["status"], json!("merged"));
    assert_eq!(
        pulled["parent_snapshot_ids"],
        json!([target_head, remote_head])
    );
    let merge_head = pulled["merge_snapshot_id"].as_str().unwrap();
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(merge_head)
    );
    assert!(worktree.join("local-only.txt").exists());
    assert!(worktree.join("remote-only.txt").exists());
    handle.join().unwrap();
}

#[test]
fn native_pull_merge_conflict_is_resumable_without_a_synthetic_source_line() {
    let remote_temp = init_repo("https://example.test");
    let remote_root = remote_temp.path();
    json_output(
        remote_root,
        &[
            "line",
            "create",
            "feature/rt-1",
            "--switch",
            "--restore",
            "--json",
        ],
    );
    write_file(
        &remote_root.join("src/lib.rs"),
        "pub fn side() -> &'static str { \"remote\" }\n",
    );
    let remote_head = seed_snapshot(remote_root, "remote conflicting side");
    let remote_zstd = zstd_remote_import_fixture_from_repo(remote_root, &remote_head);
    let (base_url, _log, handle) =
        spawn_remote_import_server("feature/rt-1", &remote_head, remote_zstd);
    let (temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = temp.path();
    json_output(
        &worktree,
        &[
            "worktree",
            "restore",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--force",
            "--json",
        ],
    );
    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn side() -> &'static str { \"local\" }\n",
    );
    let target_head = json_output(
        &worktree,
        &["snapshot", "create", "--message", "local conflicting side", "--json"],
    )["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();

    let pulled = json_output(
        &worktree,
        &["pull", "--line", "feature/rt-1", "--merge", "--json"],
    );
    assert_eq!(pulled["action"], json!("merge_conflicted"));
    assert_eq!(pulled["status"], json!("conflicted"));
    assert_eq!(
        local_line_head(repo_root, "feature/rt-1").as_deref(),
        Some(target_head.as_str())
    );
    let metadata = merge_worktree_metadata(repo_root);
    assert_eq!(metadata["merge_source_line"], json!("origin/feature/rt-1"));
    assert!(metadata.get("merge_source_verification_line").is_none());
    assert!(local_line_head(repo_root, "remote/origin/feature/rt-1").is_none());

    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn side() -> &'static str { \"resolved\" }\n",
    );
    let continued = json_output(&worktree, &["line", "merge", "--continue", "--json"]);
    assert_eq!(continued["status"], json!("continued"));
    assert_eq!(
        continued["parent_snapshot_ids"],
        json!([target_head, remote_head])
    );
    handle.join().unwrap();
}
