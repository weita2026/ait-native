fn init_plain_line_lifecycle_repo() -> TempDir {
    let temp = TempDir::new().expect("line lifecycle temp repo");
    write_file(
        &temp.path().join("src/lib.rs"),
        "pub fn line_lifecycle_fixture() -> &'static str { \"ready\" }\n",
    );
    json_output(temp.path(), &["init", "--json"]);
    json_output(
        temp.path(),
        &[
            "snapshot",
            "create",
            "--message",
            "line lifecycle base",
            "--json",
        ],
    );
    temp
}

fn failed_cli(root: &Path, args: &[&str]) -> String {
    let output = command_output_with_env(root, args, &[]);
    assert!(
        !output.status.success(),
        "command unexpectedly succeeded: {:?}\nstdout:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout)
    );
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn native_line_rename_preserves_identity_and_updates_default_current_pointers() {
    let temp = init_plain_line_lifecycle_repo();
    let root = temp.path();
    let before = json_output(root, &["line", "show", "main", "--json"]);
    let line_id = before["line_id"].as_str().expect("stable line id");
    let head = before["head_snapshot_id"].clone();

    let renamed = json_output(root, &["line", "rename", "main", "trunk", "--json"]);
    assert_eq!(renamed["contract"], json!("line-lifecycle/v1"));
    assert_eq!(renamed["line_id"], json!(line_id));
    assert_eq!(renamed["old_line_name"], json!("main"));
    assert_eq!(renamed["new_line_name"], json!("trunk"));
    assert_eq!(renamed["head_snapshot_id"], head);

    let after = json_output(root, &["line", "show", "trunk", "--json"]);
    assert_eq!(after["line_id"], json!(line_id));
    let config = parse_json_file(root.join(".ait/config.json"));
    assert_eq!(config["default_line"], json!("trunk"));
    assert_eq!(config["default_line_id"], json!(line_id));
    assert_eq!(config["current_line"], json!("trunk"));
    assert_eq!(config["current_line_id"], json!(line_id));
    assert!(failed_cli(root, &["line", "show", "main", "--json"]).contains("Unknown line"));
    assert!(!root.join(".ait/line-lifecycle-transaction.json").exists());
}

#[test]
fn native_line_rename_reconciles_bound_worktree_and_rejects_collisions() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (temp, worktree) = init_worktree_repo(&base_url);
    let root = temp.path();
    let before = json_output(
        &worktree,
        &["line", "show", "feature/rt-1", "--json"],
    );
    let line_id = before["line_id"].as_str().unwrap().to_string();
    let active_change = failed_cli(
        &worktree,
        &["line", "rename", "main", "trunk", "--json"],
    );
    assert!(active_change.contains("active Change(s)"), "{active_change}");
    seed_binary_line(root, "occupied", FIXTURE_BASE_SNAPSHOT_ID);

    let collision = failed_cli(
        &worktree,
        &[
            "line",
            "rename",
            "feature/rt-1",
            "occupied",
            "--json",
        ],
    );
    assert!(collision.contains("Line already exists: occupied"));
    assert_eq!(
        json_output(
            &worktree,
            &["line", "show", "feature/rt-1", "--json"]
        )["line_id"],
        json!(line_id)
    );

    let renamed = json_output(
        &worktree,
        &[
            "line",
            "rename",
            "feature/rt-1",
            "feature/renamed",
            "--json",
        ],
    );
    assert_eq!(renamed["line_id"], json!(line_id));
    let marker = parse_json_file(worktree.join(".ait-worktree.json"));
    assert_eq!(marker["current_line"], json!("feature/renamed"));
    assert_eq!(marker["current_line_id"], json!(line_id));
    let registry = parse_json_file(root.join(".ait/worktrees/rt-1.json"));
    assert_eq!(registry["line_name"], json!("feature/renamed"));
    assert_eq!(registry["line_id"], json!(line_id));
    assert_eq!(
        json_output(
            &worktree,
            &["line", "show", "feature/renamed", "--json"]
        )["line_id"],
        json!(line_id)
    );

    handle.join().unwrap();
}

#[test]
fn native_line_delete_tombstones_ref_preserves_snapshot_and_changes_identity_on_reuse() {
    let temp = init_plain_line_lifecycle_repo();
    let root = temp.path();
    let main = json_output(root, &["line", "show", "main", "--json"]);
    let head = main["head_snapshot_id"].as_str().unwrap();
    let created = json_output(
        root,
        &[
            "line",
            "create",
            "topic/dead",
            "--from-snapshot",
            head,
            "--json",
        ],
    );
    let old_line_id = created["line_id"].as_str().unwrap().to_string();

    let deleted = json_output(
        root,
        &["line", "delete", "topic/dead", "--yes", "--json"],
    );
    assert_eq!(deleted["line_id"], json!(old_line_id));
    assert_eq!(deleted["status"], json!("deleted"));
    assert_eq!(deleted["history_preserved"], json!(true));
    assert_eq!(deleted["tombstone"], json!(true));
    assert_eq!(deleted["snapshots_deleted"], json!(0));
    assert!(failed_cli(root, &["line", "show", "topic/dead", "--json"])
        .contains("Unknown line"));
    assert_eq!(
        json_output(root, &["snapshot", "show", head, "--json"])["snapshot_id"],
        json!(head)
    );

    let recreated = json_output(
        root,
        &[
            "line",
            "create",
            "topic/dead",
            "--from-snapshot",
            head,
            "--json",
        ],
    );
    assert_ne!(recreated["line_id"], json!(old_line_id));
    assert_eq!(recreated["head_snapshot_id"], json!(head));
}

#[test]
fn native_line_delete_rejects_default_current_protected_bound_and_unique_history() {
    let temp = init_plain_line_lifecycle_repo();
    let root = temp.path();
    assert!(failed_cli(root, &["line", "delete", "main", "--yes", "--json"])
        .contains("Default line main cannot be deleted"));

    let head = json_output(root, &["line", "show", "main", "--json"])
        ["head_snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    json_output(
        root,
        &[
            "line",
            "create",
            "review/protected",
            "--from-snapshot",
            &head,
            "--json",
        ],
    );
    assert!(failed_cli(
        root,
        &[
            "line",
            "delete",
            "review/protected",
            "--yes",
            "--json"
        ]
    )
    .contains("Protected review line"));

    json_output(
        root,
        &[
            "line",
            "create",
            "topic/current",
            "--from-snapshot",
            &head,
            "--switch",
            "--json",
        ],
    );
    assert!(failed_cli(
        root,
        &[
            "line",
            "delete",
            "topic/current",
            "--yes",
            "--json"
        ]
    )
    .contains("Current line topic/current cannot be deleted"));
    json_output(root, &["line", "switch", "main", "--json"]);

    json_output(
        root,
        &[
            "line",
            "create",
            "topic/bound",
            "--from-snapshot",
            &head,
            "--json",
        ],
    );
    let bound_worktree = root.join("bound-worktree");
    fs::create_dir_all(&bound_worktree).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join(".ait"), bound_worktree.join(".ait")).unwrap();
    write_file(
        &bound_worktree.join(".ait-worktree.json"),
        &format!(
            "{{\n  \"current_line\": \"topic/bound\",\n  \"repo_root\": \"{}\",\n  \"workspace_root\": \"{}\",\n  \"worktree_name\": \"bound-worktree\"\n}}\n",
            root.display(),
            bound_worktree.display()
        ),
    );
    write_file(
        &root.join(".ait/worktrees/bound-worktree.json"),
        &format!(
            "{{\n  \"name\": \"bound-worktree\",\n  \"path\": \"{}\",\n  \"repo_root\": \"{}\",\n  \"line_name\": \"topic/bound\",\n  \"rebase_state\": \"idle\",\n  \"merge_state\": \"idle\",\n  \"created_at\": \"2026-07-19T00:00:00Z\",\n  \"auto_created_for_task\": false\n}}\n",
            bound_worktree.display(),
            root.display()
        ),
    );
    let bound = failed_cli(
        root,
        &[
            "line",
            "delete",
            "topic/bound",
            "--yes",
            "--json",
        ],
    );
    assert!(bound.contains("still bound"), "{bound}");

    json_output(
        root,
        &[
            "line",
            "create",
            "topic/unique",
            "--from-snapshot",
            &head,
            "--switch",
            "--json",
        ],
    );
    write_file(&root.join("unique-only.txt"), "unique history\n");
    json_output(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "unique line history",
            "--json",
        ],
    );
    json_output(root, &["line", "switch", "main", "--json"]);
    let unique = failed_cli(
        root,
        &[
            "line",
            "delete",
            "topic/unique",
            "--yes",
            "--json",
        ],
    );
    assert!(unique.contains("unique history that is not verified"), "{unique}");
}

#[test]
fn native_line_cleanup_text_groups_protection_and_bounds_representative_rows() {
    let temp = init_repo("https://example.test");
    let root = temp.path();
    for index in 0..25 {
        let line_name = format!("topic/manual-{index:02}");
        json_output(root, &["line", "create", &line_name, "--json"]);
    }

    let output = command_output_with_env(
        root,
        &[
            "line",
            "cleanup",
            "--idle-for",
            "1m",
            "--include-protected",
        ],
        &[],
    );
    assert!(output.status.success());
    let output = String::from_utf8_lossy(&output.stdout);
    assert!(output.contains("protected reasons\ncount\tprotected_reason"));
    assert!(output.contains("line lifecycle is manual_only"));
    assert!(output.contains("protected examples\nline_name\tlifecycle_kind"));
    assert!(output.contains("representative rows"));
    assert!(output.contains(
        "more: ait line cleanup --idle-for 1m --include-protected --all"
    ));

    let bounded = command_output_with_env(
        root,
        &["line", "cleanup", "--idle-for", "1m"],
        &[],
    );
    assert!(bounded.status.success());
    let bounded = String::from_utf8_lossy(&bounded.stdout);
    assert!(bounded.contains(
        "protected detail: ait line cleanup --idle-for 1m --include-protected --all"
    ));
}

#[test]
fn native_line_cleanup_previews_by_default_and_requires_yes_to_archive() {
    let temp = init_plain_line_lifecycle_repo();
    let root = temp.path();
    let head = json_output(root, &["line", "show", "main", "--json"])
        ["head_snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    seed_binary_line(root, "wip/idle-a", &head);
    seed_binary_line(root, "wip/idle-b", &head);

    let preview = json_output(
        root,
        &[
            "line",
            "cleanup",
            "--idle-for",
            "7d",
            "--kind",
            "wip",
            "--limit",
            "1",
            "--include-protected",
            "--json",
        ],
    );
    assert_eq!(preview["mode"], json!("preview"));
    assert_eq!(preview["applied"], json!(false));
    assert_eq!(preview["idle_for"], json!("7d"));
    assert_eq!(preview["cleanup_kind"], json!("wip"));
    assert_eq!(preview["candidate_count"], json!(2));
    assert_eq!(preview["planned_count"], json!(1));
    assert_eq!(preview["archived_count"], json!(0));
    assert!(preview["protected_count"].as_u64().unwrap() >= 1);
    assert_eq!(
        json_output(root, &["line", "show", "wip/idle-a", "--json"])["status"],
        json!("active")
    );

    let applied = json_output(
        root,
        &[
            "line",
            "cleanup",
            "--idle-for",
            "7d",
            "--kind",
            "wip",
            "--limit",
            "1",
            "--yes",
            "--json",
        ],
    );
    assert_eq!(applied["mode"], json!("applied"));
    assert_eq!(applied["applied"], json!(true));
    assert_eq!(applied["archived_count"], json!(1));
    assert_eq!(
        json_output(root, &["line", "show", "wip/idle-a", "--json"])["status"],
        json!("archived")
    );
    assert_eq!(
        json_output(root, &["line", "show", "wip/idle-b", "--json"])["status"],
        json!("active")
    );
}

#[test]
fn native_line_removed_options_fail_before_authority_mutation() {
    let temp = init_plain_line_lifecycle_repo();
    let root = temp.path();
    let before = json_output(root, &["line", "list", "--all", "--json"]);

    for args in [
        vec!["line", "cleanup-candidates"],
        vec!["line", "cleanup", "--older-than", "7d"],
        vec!["line", "cleanup", "--dry-run"],
        vec!["line", "create", "topic/removed", "--restore"],
        vec!["line", "create", "topic/removed", "--force"],
        vec!["line", "merge", "main", "--into", "main"],
    ] {
        let error = failed_cli(root, &args);
        assert!(
            error.contains("unrecognized subcommand") || error.contains("unexpected argument"),
            "{args:?}: {error}"
        );
    }

    for args in [
        vec!["line", "cleanup", "--idle-for", "0d", "--json"],
        vec!["line", "cleanup", "--idle-for=-1d", "--json"],
        vec!["line", "cleanup", "--limit", "0", "--json"],
    ] {
        let error = failed_cli(root, &args);
        assert!(error.contains("greater than zero"), "{args:?}: {error}");
    }
    let malformed = failed_cli(
        root,
        &["line", "cleanup", "--idle-for", "7天", "--json"],
    );
    assert!(malformed.contains("must look like"), "{malformed}");
    let overflow = failed_cli(
        root,
        &[
            "line",
            "cleanup",
            "--idle-for",
            "9223372036854775807d",
            "--json",
        ],
    );
    assert!(overflow.contains("supported duration range"), "{overflow}");

    assert_eq!(
        json_output(root, &["line", "list", "--all", "--json"]),
        before
    );
}

#[test]
fn native_line_help_exposes_the_compact_creation_merge_and_cleanup_contract() {
    let temp = init_plain_line_lifecycle_repo();
    let root = temp.path();

    let help_text = |args: &[&str]| {
        let output = command_output_with_env(root, args, &[]);
        assert!(output.status.success(), "help failed for {args:?}");
        String::from_utf8(output.stdout).unwrap()
    };

    let line_help = help_text(&["line", "--help"]);
    assert!(line_help.contains("cleanup"));
    assert!(!line_help.contains("cleanup-candidates"));

    let create_help = help_text(&["line", "create", "--help"]);
    assert!(create_help.contains("--from-snapshot"));
    assert!(create_help.contains("--switch"));
    assert!(!create_help.contains("--restore"));
    assert!(!create_help.contains("--force"));

    let merge_help = help_text(&["line", "merge", "--help"]);
    assert!(merge_help.contains("--continue"));
    assert!(merge_help.contains("--abort"));
    assert!(!merge_help.contains("--into"));

    let cleanup_help = help_text(&["line", "cleanup", "--help"]);
    for option in [
        "--idle-for",
        "--kind",
        "--limit",
        "--include-protected",
        "--all",
        "--yes",
        "--json",
    ] {
        assert!(cleanup_help.contains(option), "missing {option}: {cleanup_help}");
    }
    assert!(!cleanup_help.contains("--older-than"));
    assert!(!cleanup_help.contains("--dry-run"));
}
