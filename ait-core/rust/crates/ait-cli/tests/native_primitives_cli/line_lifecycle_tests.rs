fn init_plain_line_lifecycle_repo() -> TempDir {
    let temp = TempDir::new().expect("line lifecycle temp repo");
    write_file(
        &temp.path().join("src/lib.rs"),
        "pub fn line_lifecycle_fixture() -> &'static str { \"ready\" }\n",
    );
    json_output(
        temp.path(),
        &[
            "init",
            "--name",
            "line-lifecycle-fixture",
            "--default-line",
            "main",
            "--json",
        ],
    );
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
            "cleanup-candidates",
            "--older-than",
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
        "more: ait line cleanup-candidates --older-than 1m --include-protected --all"
    ));

    let bounded = command_output_with_env(
        root,
        &["line", "cleanup-candidates", "--older-than", "1m"],
        &[],
    );
    assert!(bounded.status.success());
    let bounded = String::from_utf8_lossy(&bounded.stdout);
    assert!(bounded.contains(
        "protected detail: ait line cleanup-candidates --older-than 1m --include-protected --all"
    ));
}
