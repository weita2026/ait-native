fn bootstrap_git_source() -> TempDir {
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    fs::write(source.path().join("code.txt"), b"code\n").unwrap();
    fs::write(
        source.path().join("README.md"),
        b"exact markdown\r\n\xff\x00",
    )
    .unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "source"]);
    fixture_git(source.path(), &["tag", "-a", "v1", "-m", "raw tag"]);
    source
}

#[test]
fn native_git_import_partitions_mixed_case_markdown_with_non_markdown_alias() {
    use sha2::{Digest, Sha256};
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Fixture"]);
    fixture_git(source.path(), &["config", "user.email", "fixture@example.invalid"]);
    let shared = b"exact aliased markdown\r\n\xff\x00";
    fs::write(source.path().join("a-alias.txt"), shared).unwrap();
    fs::write(source.path().join("Guide.MD"), shared).unwrap();
    fs::write(source.path().join("other.txt"), b"ordinary source body").unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "mixed-case alias"]);
    fixture_git(source.path(), &["tag", "-a", "v1", "-m", "exact tag"]);
    let expected_head = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let expected_refs = git_ref_map(source.path());
    let ait = init_empty_ait_git_interop_repo();
    let imported = json_output(ait.path(), &["git", "import", source.path().to_str().unwrap(), "--all-branches-and-tags", "--json"]);
    assert_eq!(imported["imported_commit_count"], json!(1));
    let digest: [u8; 32] = Sha256::digest(shared).into();
    let blob = ait_core::content_binary_db::blob_id_from_sha256(&digest);
    let packs = fs::read_dir(ait.path().join(".ait/objects/packs")).unwrap();
    let mut matching = 0;
    for pack in packs {
        let path = pack.unwrap().path();
        let index = ait_core::pack_substrate::read_pack_index(path.to_str().unwrap()).unwrap();
        if index["entries"].as_array().unwrap().iter().any(|entry| entry["blob_id"] == json!(blob)) {
            matching += 1;
            assert_eq!(index["chunk_count"], json!(2), "the tiny raw source batch must separate the aliased .MD Blob from other kinds");
        }
    }
    assert_eq!(matching, 1);
    let target = TempDir::new().unwrap();
    let restored = target.path().join("restored");
    json_output(ait.path(), &["git", "restore", "--generation", imported["generation_id"].as_str().unwrap(), "--git-object", &expected_head, "--target", restored.to_str().unwrap(), "--json"]);
    assert_eq!(fs::read(restored.join("Guide.MD")).unwrap(), shared);
    assert_eq!(fs::read(restored.join("a-alias.txt")).unwrap(), shared);
    let exported = target.path().join("export.git");
    json_output(ait.path(), &["git", "export", exported.to_str().unwrap(), "--generation", imported["generation_id"].as_str().unwrap(), "--all-lines-and-tags", "--json"]);
    assert_eq!(git_ref_map(&exported), expected_refs);
    assert_eq!(fixture_git(&exported, &["rev-parse", "HEAD"]), expected_head);
    fixture_git(&exported, &["fsck", "--strict", "--full"]);
}

#[test]
fn native_git_import_bootstrap_replay_fills_missing_setup_without_history_rewrite() {
    let source = bootstrap_git_source();
    let oid = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let refs = git_ref_map(source.path());
    let ait = init_empty_ait_git_interop_repo();
    assert!(!ait.path().join("ci/patch_ci.json").exists());
    for path in [".aitignore", "AGENTS.md", "CLAUDE.md"] {
        fs::remove_file(ait.path().join(path)).unwrap();
    }
    fs::remove_dir(ait.path().join("docs/sprints")).unwrap();
    let report = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(
        report["workspace_bootstrap"]["created_paths"],
        json!([".aitignore", "AGENTS.md", "CLAUDE.md", "docs/sprints"])
    );
    assert!(!ait.path().join("ci").exists());
    let head = json_output(ait.path(), &["status", "--json"])["head_snapshot_id"].clone();
    let inventory = json_output(
        ait.path(),
        &["snapshot", "show", head.as_str().unwrap(), "--json"],
    );
    assert!(!inventory["files"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["path"] == json!("ci/patch_ci.json")));
    let mappings_path = "git_commit_mapping.bin";
    let mappings = native_git_bytes(ait.path(), mappings_path);
    let guide = fs::read(ait.path().join("CLAUDE.md")).unwrap();
    let guide_stamp = fs::metadata(ait.path().join("CLAUDE.md"))
        .unwrap()
        .modified()
        .unwrap();
    for path in ["AGENTS.md", ".aitignore"] {
        fs::remove_file(ait.path().join(path)).unwrap();
    }
    let replay = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(replay["status"], json!("no_op"));
    assert_eq!(replay["mutated"], json!(true));
    assert_eq!(
        replay["workspace_bootstrap"]["source_history_rewritten"],
        json!(false)
    );
    assert_eq!(mappings, native_git_bytes(ait.path(), mappings_path));
    assert_eq!(
        head,
        json_output(ait.path(), &["status", "--json"])["head_snapshot_id"]
    );
    assert_eq!(guide, fs::read(ait.path().join("CLAUDE.md")).unwrap());
    assert_eq!(
        guide_stamp,
        fs::metadata(ait.path().join("CLAUDE.md"))
            .unwrap()
            .modified()
            .unwrap()
    );
    let stable_guide = fs::read(ait.path().join("AGENTS.md")).unwrap();
    let stable_stamp = fs::metadata(ait.path().join("AGENTS.md"))
        .unwrap()
        .modified()
        .unwrap();
    let again = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(again["mutated"], json!(false));
    assert_eq!(
        stable_guide,
        fs::read(ait.path().join("AGENTS.md")).unwrap()
    );
    assert_eq!(
        stable_stamp,
        fs::metadata(ait.path().join("AGENTS.md"))
            .unwrap()
            .modified()
            .unwrap()
    );
    let restored = TempDir::new().unwrap();
    let restored_path = restored.path().join("tree");
    json_output(
        ait.path(),
        &[
            "git",
            "restore",
            "--generation",
            report["generation_id"].as_str().unwrap(),
            "--git-object",
            &oid,
            "--target",
            restored_path.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(
        fs::read(restored_path.join("README.md")).unwrap(),
        b"exact markdown\r\n\xff\x00"
    );
    assert_eq!(fs::read(restored_path.join("code.txt")).unwrap(), b"code\n");
    assert!(!restored_path.join("ci/patch_ci.json").exists());
    let exported = restored.path().join("export.git");
    json_output(
        ait.path(),
        &[
            "git",
            "export",
            exported.to_str().unwrap(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(refs, git_ref_map(&exported));
    assert_eq!(oid, fixture_git(&exported, &["rev-parse", "HEAD"]));
    fixture_git(&exported, &["fsck", "--strict", "--full"]);
}

#[test]
fn native_git_import_bootstrap_custom_ci_in_source_is_preserved() {
    let source = bootstrap_git_source();
    let bytes = b"{\"schema_version\":1,\"suites\":[{\"schema_version\":1,\"suite_id\":\"custom\",\"plane\":\"patchset\",\"mode\":\"gate\",\"default_blocking\":true,\"runner\":{\"kind\":\"command_bundle\",\"commands\":[\"actual-project-check\"]}}]}\n";
    fs::create_dir(source.path().join("ci")).unwrap();
    fs::write(source.path().join("ci/patch_ci.json"), bytes).unwrap();
    fixture_git(source.path(), &["add", "ci/patch_ci.json"]);
    fixture_git(source.path(), &["commit", "-m", "source CI"]);
    let ait = init_empty_ait_git_interop_repo();
    fs::create_dir(ait.path().join("ci")).unwrap();
    fs::write(ait.path().join("ci/patch_ci.json"), bytes).unwrap();
    let config = fs::read(ait.path().join(".ait/config.json")).unwrap();
    let policy = fs::read(ait.path().join(".ait/policy.yaml")).unwrap();
    let first = json_output(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
    );
    assert_eq!(first["workspace_bootstrap"]["status"], json!("completed"));
    let stamp = fs::metadata(ait.path().join("ci/patch_ci.json"))
        .unwrap()
        .modified()
        .unwrap();
    let again = json_output(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
    );
    assert_eq!(again["workspace_bootstrap"]["mutated"], json!(false));
    assert_eq!(
        bytes,
        fs::read(ait.path().join("ci/patch_ci.json"))
            .unwrap()
            .as_slice()
    );
    assert_eq!(
        stamp,
        fs::metadata(ait.path().join("ci/patch_ci.json"))
            .unwrap()
            .modified()
            .unwrap()
    );
    assert_eq!(
        config,
        fs::read(ait.path().join(".ait/config.json")).unwrap()
    );
    assert_eq!(
        policy,
        fs::read(ait.path().join(".ait/policy.yaml")).unwrap()
    );
}

#[test]
fn native_git_import_bootstrap_dry_run_is_inert_and_invalid_setup_blocks_before_history() {
    let source = bootstrap_git_source();
    let ait = init_empty_ait_git_interop_repo();
    let before = fs::read(ait.path().join(".ait/binary-db/snapshot.bin")).ok();
    let dry = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry["mutated"], json!(false));
    assert!(!ait.path().join("ci").exists());
    fs::create_dir(ait.path().join("ci")).unwrap();
    fs::write(
        ait.path().join("AGENTS.md"),
        b"custom\n<!-- ait:workflow:start -->\n",
    )
    .unwrap();
    let rejected = command_output_with_env(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
        &[],
    );
    assert!(!rejected.status.success());
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains("incomplete ait-managed workflow block")
    );
    assert_eq!(
        fs::read(ait.path().join("AGENTS.md")).unwrap(),
        b"custom\n<!-- ait:workflow:start -->\n"
    );
    assert_eq!(
        before,
        fs::read(ait.path().join(".ait/binary-db/snapshot.bin")).ok()
    );
    assert!(json_output(ait.path(), &["status", "--json"])["head_snapshot_id"].is_null());
}

#[test]
fn native_git_import_preserves_invalid_unrelated_ci_without_generating_templates() {
    let source = bootstrap_git_source();
    let ait = init_empty_ait_git_interop_repo();
    fs::create_dir(ait.path().join("ci")).unwrap();
    let path = ait.path().join("ci/patch_ci.json");
    fs::write(&path, b"invalid user-owned CI\n").unwrap();
    let stamp = fs::metadata(&path).unwrap().modified().unwrap();
    for _ in 0..2 {
        json_output(
            ait.path(),
            &["git", "import", source.path().to_str().unwrap(), "--json"],
        );
        assert_eq!(fs::read(&path).unwrap(), b"invalid user-owned CI\n");
        assert_eq!(fs::metadata(&path).unwrap().modified().unwrap(), stamp);
    }
}
