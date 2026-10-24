#[test]
fn native_tag_namespace_stores_message_and_resolves_snapshot_refs() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let (_temp, worktree) = init_worktree_repo(&base_url);
    let repo_root = worktree.parent().unwrap().to_path_buf();

    let created = json_output(
        &worktree,
        &[
            "tag",
            "create",
            "stable/baseline",
            "--message",
            "known good regression baseline",
            "--json",
        ],
    );
    assert_eq!(created["name"].as_str(), Some("stable/baseline"));
    assert_eq!(
        created["snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        created["message"].as_str(),
        Some("known good regression baseline")
    );
    assert_eq!(created["source_line"].as_str(), Some("feature/rt-1"));

    let tag_path = repo_root.join(".ait/binary-db/tag.bin");
    let tag_bytes = fs::read(&tag_path).unwrap();
    assert_eq!(tag_bytes.len(), 4 + 24);
    assert_eq!(tag_bytes[4], 0);
    let payload = fs::read(repo_root.join(".ait/binary-db/tag_payload.bin")).unwrap();
    assert!(payload
        .windows(b"stable/baseline".len())
        .any(|v| v == b"stable/baseline"));
    assert!(payload
        .windows(b"known good regression baseline".len())
        .any(|v| v == b"known good regression baseline"));
    assert!(!repo_root.join(".ait/refs/tags").exists());

    let shown = json_output(&worktree, &["tag", "show", "stable/baseline", "--json"]);
    assert_eq!(
        shown["snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );

    let listed = json_output(&worktree, &["tag", "list", "--json"]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["name"].as_str(), Some("stable/baseline"));

    let snapshot = json_output(
        &worktree,
        &["snapshot", "show", "stable/baseline", "--json"],
    );
    assert_eq!(
        snapshot["snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );

    write_file(
        &worktree.join("src/lib.rs"),
        "pub fn worktree_version() -> &'static str { \"after baseline\" }\n",
    );
    let next = checkpoint_bound_worktree_fixture(&worktree, "after baseline");
    let next_snapshot_id = next["snapshot_id"].as_str().unwrap();
    let diff = json_output(
        &worktree,
        &[
            "snapshot",
            "diff",
            "stable/baseline",
            next_snapshot_id,
            "--json",
        ],
    );
    assert_eq!(
        diff["modified"].as_array().unwrap()[0].as_str(),
        Some("src/lib.rs")
    );

    let duplicate = cargo_bin()
        .current_dir(&worktree)
        .args([
            "tag",
            "create",
            "stable/baseline",
            "--message",
            "duplicate baseline",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        !duplicate.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&duplicate.stdout),
        String::from_utf8_lossy(&duplicate.stderr)
    );
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("already exists"));

    let preserved = json_output(&worktree, &["tag", "show", "stable/baseline", "--json"]);
    assert_eq!(
        preserved["snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );
    assert_eq!(
        preserved["message"].as_str(),
        Some("known good regression baseline")
    );

    let retired_force = cargo_bin()
        .current_dir(&worktree)
        .args([
            "tag",
            "create",
            "stable/baseline",
            "--snapshot",
            next_snapshot_id,
            "--message",
            "updated regression baseline",
            "--force",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(!retired_force.status.success());
    assert!(
        String::from_utf8_lossy(&retired_force.stderr).contains("unexpected argument '--force'")
    );

    let deleted = json_output(&worktree, &["tag", "delete", "stable/baseline", "--json"]);
    assert_eq!(deleted["deleted"].as_bool(), Some(true));
    assert_eq!(
        deleted["snapshot_id"].as_str(),
        Some(FIXTURE_BASE_SNAPSHOT_ID)
    );

    let missing = cargo_bin()
        .current_dir(&worktree)
        .args(["tag", "show", "stable/baseline", "--json"])
        .output()
        .unwrap();
    assert!(
        !missing.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&missing.stdout),
        String::from_utf8_lossy(&missing.stderr)
    );
    assert!(String::from_utf8_lossy(&missing.stderr).contains("Unknown tag"));

    let tombstone = fs::read(&tag_path).unwrap();
    assert_eq!(tombstone.len(), tag_bytes.len());
    assert_eq!(tombstone[4], 128);
    json_output(
        &worktree,
        &[
            "tag",
            "create",
            "stable/baseline",
            "--snapshot",
            FIXTURE_BASE_SNAPSHOT_ID,
            "--message",
            "recreated",
            "--json",
        ],
    );
    let recreated = fs::read(&tag_path).unwrap();
    assert_eq!(recreated.len(), tag_bytes.len());
    assert_eq!(recreated[4], 0);

    handle.join().unwrap();
}

#[test]
fn native_tag_create_requires_single_line_message_and_existing_snapshot() {
    let (base_url, _log, _state, handle) = spawn_fake_remote();
    let temp = init_repo(&base_url);
    let root = temp.path();

    let multiline = cargo_bin()
        .current_dir(root)
        .args([
            "tag",
            "create",
            "stable/bad-message",
            "--message",
            "line one\nline two",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        !multiline.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&multiline.stdout),
        String::from_utf8_lossy(&multiline.stderr)
    );
    assert!(String::from_utf8_lossy(&multiline.stderr).contains("message must be a single line"));

    let unknown_snapshot = cargo_bin()
        .current_dir(root)
        .args([
            "tag",
            "create",
            "stable/missing",
            "--snapshot",
            "SNP-000000000000",
            "--message",
            "missing snapshot baseline",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        !unknown_snapshot.status.success(),
        "stdout:\n{}\n\nstderr:\n{}",
        String::from_utf8_lossy(&unknown_snapshot.stdout),
        String::from_utf8_lossy(&unknown_snapshot.stderr)
    );
    assert!(String::from_utf8_lossy(&unknown_snapshot.stderr).contains("Unknown snapshot"));

    handle.join().unwrap();
}
