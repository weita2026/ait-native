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

    let tag_path = repo_root
        .join(".ait")
        .join("refs")
        .join("tags")
        .join(format!("{}.json", encode_ref_name("stable/baseline")));
    let tag_text = fs::read_to_string(tag_path).unwrap();
    assert!(tag_text.contains(&format!(
        "\"snapshot_id\": \"{FIXTURE_BASE_SNAPSHOT_ID}\""
    )));
    assert!(tag_text.contains("\"message\": \"known good regression baseline\""));

    let shown = json_output(&worktree, &["tag", "show", "stable/baseline", "--json"]);
    assert_eq!(shown["snapshot_id"].as_str(), Some(FIXTURE_BASE_SNAPSHOT_ID));

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
    let next = json_output(
        &worktree,
        &["snapshot", "create", "--message", "after baseline", "--json"],
    );
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

    let replaced = json_output(
        &worktree,
        &[
            "tag",
            "create",
            "stable/baseline",
            "--snapshot",
            next_snapshot_id,
            "--message",
            "updated regression baseline",
            "--force",
            "--json",
        ],
    );
    assert_eq!(replaced["snapshot_id"].as_str(), Some(next_snapshot_id));
    assert_eq!(
        replaced["message"].as_str(),
        Some("updated regression baseline")
    );

    let deleted = json_output(&worktree, &["tag", "delete", "stable/baseline", "--json"]);
    assert_eq!(deleted["deleted"].as_bool(), Some(true));
    assert_eq!(deleted["snapshot_id"].as_str(), Some(next_snapshot_id));

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
