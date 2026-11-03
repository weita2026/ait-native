#[test]
fn native_git_binary_cli_recovery_preserves_complete_owners_and_blocks_damaged_payload() {
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    fs::write(source.path().join("code.txt"), b"code\n").unwrap();
    fs::write(source.path().join("README.md"), b"document\r\n\xff\x00").unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "recovery source"]);
    let oid = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let ait = init_empty_ait_git_interop_repo();
    json_output(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
    );
    let root = ait.path().join(".ait/git-interop/v1/binary-v0");
    let mapping_path = "git_commit_mapping.bin";
    let mapping_before = native_git_bytes(ait.path(), mapping_path);
    let payload_path = "git_ref_mapping_payload.bin";
    let payload_before = native_git_bytes(ait.path(), payload_path);
    let plan_root = ait.path().join(".ait/binary-db");
    let plans =
        ["plan.bin", "plan_revision.bin"].map(|name| fs::read(plan_root.join(name)).unwrap());
    let index_path = "git_document_revision.idx";
    native_git_mutate(ait.path(),|root|{
        let mut bytes=fs::read(root.join(mapping_path)).unwrap();bytes.extend_from_slice(&[9;19]);fs::write(root.join(mapping_path),bytes).unwrap();
        let mut bytes=fs::read(root.join(payload_path)).unwrap();bytes.extend_from_slice(b"unreferenced partial payload");fs::write(root.join(payload_path),bytes).unwrap();
        fs::write(root.join(index_path),1u32.to_le_bytes()).unwrap();
    });
    let damaged_mapping = native_git_bytes(ait.path(), mapping_path);
    let damaged_payload = native_git_bytes(ait.path(), payload_path);
    let diagnosed = json_output(ait.path(), &["git", "recover", "--json"]);
    assert_eq!(diagnosed["status"], json!("repairable"));
    assert_eq!(diagnosed["mutated"], json!(false));
    assert_eq!(damaged_mapping, native_git_bytes(ait.path(), mapping_path));
    assert_eq!(damaged_payload, native_git_bytes(ait.path(), payload_path));
    assert_eq!(native_git_bytes(ait.path(),index_path),1u32.to_le_bytes());
    let repaired = json_output(ait.path(), &["git", "recover", "--apply", "--json"]);
    assert_eq!(repaired["status"], json!("repaired"));
    assert_eq!(repaired["mapping_json_count"], json!(0));
    assert_eq!(mapping_before, native_git_bytes(ait.path(), mapping_path));
    assert_eq!(payload_before, native_git_bytes(ait.path(), payload_path));
    assert_eq!(
        plans,
        ["plan.bin", "plan_revision.bin"].map(|name| fs::read(plan_root.join(name)).unwrap())
    );
    let output = TempDir::new().unwrap();
    let export = output.path().join("repaired.git");
    json_output(
        ait.path(),
        &["git", "export", export.to_str().unwrap(), "--json"],
    );
    assert_eq!(oid, fixture_git(&export, &["rev-parse", "HEAD"]));
    fixture_git(&export, &["fsck", "--strict", "--full"]);
    let payload_after_export = native_git_bytes(ait.path(), payload_path);
    native_git_write(ait.path(), payload_path, &payload_after_export[..payload_after_export.len() - 1],);
    let complete_mapping = native_git_bytes(ait.path(), mapping_path);
    let damaged = native_git_bytes(ait.path(), payload_path);
    let blocked =
        command_output_with_env(ait.path(), &["git", "recover", "--apply", "--json"], &[]);
    assert!(!blocked.status.success());
    assert!(
        String::from_utf8_lossy(&blocked.stderr).contains("referenced git_ref_mapping_payload.bin")
    );
    assert_eq!(complete_mapping, native_git_bytes(ait.path(), mapping_path));
    assert_eq!(damaged, native_git_bytes(ait.path(), payload_path));
    assert_eq!(
        plans,
        ["plan.bin", "plan_revision.bin"].map(|name| fs::read(plan_root.join(name)).unwrap())
    );
    assert!(!root.parent().unwrap().join("mappings").exists());
    assert!(!root.parent().unwrap().join("operations").exists());
    native_git_write(ait.path(), payload_path, &payload_after_export);
    let prior_plan_prefix = &plans[0][..plans[0].len() - 48];
    fs::write(plan_root.join("plan.bin"), prior_plan_prefix).unwrap();
    let owner_diagnosis = json_output(ait.path(), &["git", "recover", "--json"]);
    assert_eq!(owner_diagnosis["status"], json!("repairable"));
    assert_eq!(
        fs::read(plan_root.join("plan.bin")).unwrap(),
        prior_plan_prefix
    );
    let owner_repair = json_output(ait.path(), &["git", "recover", "--apply", "--json"]);
    assert_eq!(owner_repair["status"], json!("repaired"));
    assert_eq!(
        plans,
        ["plan.bin", "plan_revision.bin"].map(|name| fs::read(plan_root.join(name)).unwrap())
    );
    assert_eq!(complete_mapping, native_git_bytes(ait.path(), mapping_path));
    assert_eq!(
        json_output(ait.path(), &["git", "recover", "--apply", "--json"])["status"],
        json!("clean")
    );
    let checkpoint_path = "git_operation_checkpoint.bin";
    let checkpoint_before = native_git_bytes(ait.path(), checkpoint_path);
    let mut invalid_checkpoint = checkpoint_before.clone();
    invalid_checkpoint[4 + 28..4 + 32].copy_from_slice(&u32::MAX.to_le_bytes());
    native_git_write(ait.path(), checkpoint_path, &invalid_checkpoint);
    let completed_damage =
        command_output_with_env(ait.path(), &["git", "recover", "--apply", "--json"], &[]);
    assert!(!completed_damage.status.success());
    assert!(String::from_utf8_lossy(&completed_damage.stderr).contains("cursor/owner closure"));
    assert_eq!(native_git_bytes(ait.path(), checkpoint_path), invalid_checkpoint);
    invalid_checkpoint[4] = 0;
    native_git_write(ait.path(), checkpoint_path, &invalid_checkpoint);
    assert_eq!(
        json_output(ait.path(), &["git", "recover", "--apply", "--json"])["status"],
        json!("repaired")
    );
    assert_eq!(native_git_bytes(ait.path(), checkpoint_path)[4], 0);
    let resumed = json_output(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
    );
    assert_eq!(resumed["imported_commit_count"], json!(0));
    assert_eq!(native_git_bytes(ait.path(), mapping_path), complete_mapping);
    assert_eq!(
        plans,
        ["plan.bin", "plan_revision.bin"].map(|name| fs::read(plan_root.join(name)).unwrap())
    );
}

#[cfg(unix)]
#[test]
fn native_git_binary_cli_resumes_a_line_conflict_without_json_checkpoint() {
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    write_file(&source.path().join("code.txt"), "code\n");
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "first"]);
    write_file(&source.path().join("README.md"), "source document\n");
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "MD only"]);
    let ait = init_empty_ait_git_interop_repo();
    write_file(
        &ait.path().join("local.txt"),
        "independent local baseline\n",
    );
    seed_snapshot(ait.path(), "local baseline");
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
    let generation = dry["generation_id"].as_str().unwrap();
    let failed = command_output_with_env(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
        &[],
    );
    assert!(!failed.status.success());
    assert!(String::from_utf8_lossy(&failed.stderr)
        .contains("refuses to overwrite advanced AIT Line main"));
    let root = ait.path().join(".ait/git-interop/v1");
    let mapping = "git_commit_mapping.bin";
    let before = native_git_bytes(ait.path(), mapping);
    let target = TempDir::new().unwrap();
    let incomplete = command_output_with_env(
        ait.path(),
        &[
            "git",
            "export",
            target.path().join("incomplete.git").to_str().unwrap(),
            "--generation",
            generation,
            "--json",
        ],
        &[],
    );
    assert!(!incomplete.status.success());
    assert!(String::from_utf8_lossy(&incomplete.stderr).contains("generation is incomplete"));
    json_output(ait.path(), &["line", "rename", "main", "local", "--json"]);
    let resumed = json_output(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
    );
    assert_eq!(resumed["generation_id"], dry["generation_id"]);
    assert_eq!(resumed["plan_hash"], dry["plan_hash"]);
    assert_eq!(resumed["reused_commit_count"], json!(2));
    assert_eq!(resumed["imported_commit_count"], json!(0));
    assert_eq!(resumed["mutated"], json!(true));
    assert_eq!(before, native_git_bytes(ait.path(), mapping));
    assert!(!root.join("operations").exists());
    assert!(!root.join("mappings").exists());
    let repeated = json_output(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
    );
    assert_eq!(repeated["status"], json!("no_op"));
    assert_eq!(repeated["mutated"], json!(false));
    let pending_target = target.path().join("mirror.git");
    let mirror = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            pending_target.to_str().unwrap(),
            "--direction",
            "outbound",
            "--json",
        ],
    );
    assert_eq!(mirror["status"], json!("completed"));
    assert_eq!(mirror["state"], json!("equal"));
    assert_eq!(mirror["compare_and_swap"], json!(true));
    assert_eq!(mirror["mapping_json_count"], json!(0));
    assert!(pending_target.exists());
    let after = native_git_bytes(ait.path(), "git_commit_mapping.bin");
    assert_eq!(&after[..before.len()], before);
    fixture_git(&pending_target, &["fsck", "--strict", "--full"]);
    assert!(!root.join("operations").exists());
}

#[cfg(unix)]
#[test]
fn native_git_binary_cli_preserves_raw_docs_after_original_odb_is_removed() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    fs::write(source.path().join("run.sh"), b"#!/bin/sh\necho code\n").unwrap();
    fs::set_permissions(
        source.path().join("run.sh"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    let first = b"first\r\n\xff\x00";
    fs::write(source.path().join("README.md"), first).unwrap();
    symlink("README.md", source.path().join("link.MD")).unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "first"]);
    let c1 = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let second = b"second\r\n\x00\xfe";
    fs::write(source.path().join("README.md"), second).unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "MD only"]);
    fixture_git(source.path(), &["mv", "README.md", "Guide.MD"]);
    fs::remove_file(source.path().join("link.MD")).unwrap();
    symlink("Guide.MD", source.path().join("link.MD")).unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "rename"]);
    let head = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let tree = fixture_git(source.path(), &["rev-parse", "HEAD^{tree}"]);
    fixture_git(
        source.path(),
        &["tag", "-a", "tree-tag", &tree, "-m", "tree target"],
    );
    let refs = git_ref_map(source.path());
    let ait = init_empty_ait_git_interop_repo();
    let imported = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(imported["storage"], json!("native_binary"));
    let generation = imported["generation_id"].as_str().unwrap();
    // A private native Plan must never become part of the source export.
    write_file(
        &ait.path().join("docs/private.md"),
        "# Private plan\n\nDo not export this.\n",
    );
    json_output(
        ait.path(),
        &[
            "plan",
            "sync",
            ait.path().join("docs/private.md").to_str().unwrap(),
            "--local",
            "--json",
        ],
    );
    let retained = TempDir::new().unwrap();
    fs::rename(
        source.path().join(".git"),
        retained.path().join("original.git"),
    )
    .unwrap();
    let target = TempDir::new().unwrap();
    let out = target.path().join("export.git");
    let exported = json_output(
        ait.path(),
        &[
            "git",
            "export",
            out.to_str().unwrap(),
            "--generation",
            generation,
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(exported["fsck"], json!("passed"));
    assert_eq!(git_ref_map(&out), refs);
    assert_eq!(fixture_git(&out, &["rev-parse", "HEAD"]), head);
    let old = target.path().join("old");
    json_output(
        ait.path(),
        &[
            "git",
            "restore",
            "--generation",
            generation,
            "--git-object",
            &c1,
            "--target",
            old.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(fs::read(old.join("README.md")).unwrap(), first);
    let latest = target.path().join("latest");
    json_output(
        ait.path(),
        &[
            "git",
            "restore",
            "--generation",
            generation,
            "--git-object",
            &head,
            "--target",
            latest.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(fs::read(latest.join("Guide.MD")).unwrap(), second);
    assert_eq!(
        fs::read_link(latest.join("link.MD")).unwrap(),
        PathBuf::from("Guide.MD")
    );
    assert!(!latest.join("README.md").exists());
    assert!(!latest.join("docs/private.md").exists());
    let selective = target.path().join("selective.git");
    let selected = json_output(
        ait.path(),
        &[
            "git",
            "export",
            selective.to_str().unwrap(),
            "--generation",
            generation,
            "--git-object",
            &c1,
            "--json",
        ],
    );
    assert_eq!(selected["exact_git_object_reuse_count"], json!(1));
    assert_eq!(fixture_git(&selective, &["rev-parse", "HEAD"]), c1);
    assert!(git_ref_map(&selective).is_empty());
    assert!(fixture_git(&selective, &["ls-tree", "-r", "HEAD"]).contains("README.md"));
    let unselected = Command::new("git")
        .current_dir(&selective)
        .args(["cat-file", "-e", &head])
        .output()
        .unwrap();
    assert!(!unselected.status.success());
    let repeated = json_output(
        ait.path(),
        &[
            "git",
            "export",
            selective.to_str().unwrap(),
            "--generation",
            generation,
            "--git-object",
            &c1,
            "--json",
        ],
    );
    assert_eq!(repeated["status"], json!("no_op"));
    assert_eq!(repeated["mutated"], json!(false));
    // A same-OID symbolic HEAD is still different from the completed detached
    // target. Replay must verify binding, not merely return its saved status.
    fixture_git(&selective, &["update-ref", "refs/heads/alias", &c1]);
    fixture_git(&selective, &["symbolic-ref", "HEAD", "refs/heads/alias"]);
    let changed_head = command_output_with_env(
        ait.path(),
        &[
            "git",
            "export",
            selective.to_str().unwrap(),
            "--generation",
            generation,
            "--git-object",
            &c1,
            "--json",
        ],
        &[],
    );
    assert!(!changed_head.status.success());
    assert!(String::from_utf8_lossy(&changed_head.stderr).contains("HEAD binding changed"));
    fixture_git(&selective, &["update-ref", "--no-deref", "HEAD", &c1]);
    fixture_git(&selective, &["update-ref", "-d", "refs/heads/alias"]);
    assert_ne!(
        fs::metadata(latest.join("run.sh"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111,
        0
    );
    write_file(&ait.path().join("native.txt"), "native descendant\n");
    let native_snapshot = seed_snapshot(ait.path(), "source preserving native descendant");
    let mixed = target.path().join("mixed-current.git");
    let mixed_report = json_output(
        ait.path(),
        &["git", "export", mixed.to_str().unwrap(), "--json"],
    );
    assert_eq!(mixed_report["native_commit_count"], json!(1));
    assert_eq!(mixed_report["exact_git_object_reuse_count"], json!(3));
    assert_eq!(
        git_ref_map(&mixed).keys().cloned().collect::<Vec<_>>(),
        vec!["refs/heads/main"]
    );
    assert_eq!(
        fixture_git(&mixed, &["show", "HEAD:native.txt"]),
        "native descendant"
    );
    let raw_md = Command::new("git")
        .current_dir(&mixed)
        .args(["show", "HEAD:Guide.MD"])
        .output()
        .unwrap();
    assert!(raw_md.status.success());
    assert_eq!(raw_md.stdout, second);
    assert_eq!(fixture_git(&mixed, &["show", "HEAD:link.MD"]), "Guide.MD");
    assert!(fixture_git(&mixed, &["ls-tree", "-r", "HEAD"]).contains("120000 blob"));
    assert!(!fixture_git(&mixed, &["ls-tree", "-r", "HEAD"]).contains("docs/private.md"));
    let parents = fixture_git(&mixed, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(parents.split_whitespace().nth(1), Some(head.as_str()));
    assert_eq!(parents.split_whitespace().count(), 2);
    assert_eq!(
        json_output(ait.path(), &["line", "show", "main", "--json"])["head_snapshot_id"],
        json!(native_snapshot)
    );
    let tag_absent = Command::new("git")
        .current_dir(&mixed)
        .args(["cat-file", "-e", "refs/tags/tree-tag"])
        .output()
        .unwrap();
    assert!(!tag_absent.status.success());
    fixture_git(&mixed, &["fsck", "--strict", "--full"]);
    let interop = ait.path().join(".ait/git-interop/v1");
    assert!(!interop.join("mappings").exists());
    assert!(!interop.join("operations").exists());
    assert!(!interop.join("repositories").exists());
    let validation = json_output(ait.path(), &["gc", "validate", "--json"]);
    assert_eq!(validation["needs_attention"], json!(false));
    let mapping_path = "git_commit_mapping.bin";
    let mappings_before_gc = native_git_bytes(ait.path(), mapping_path);
    json_output(ait.path(), &["gc", "prune", "--apply", "--json"]);
    assert_eq!(mappings_before_gc, native_git_bytes(ait.path(), mapping_path));
    let after_gc = target.path().join("after-gc");
    json_output(
        ait.path(),
        &[
            "git",
            "restore",
            "--generation",
            generation,
            "--git-object",
            &head,
            "--target",
            after_gc.to_str().unwrap(),
            "--json",
        ],
    );
    assert_eq!(fs::read(after_gc.join("Guide.MD")).unwrap(), second);
    assert_eq!(
        fs::read_link(after_gc.join("link.MD")).unwrap(),
        PathBuf::from("Guide.MD")
    );
}

#[test]
fn native_git_binary_capture_activation_preserves_original_history_without_odb() {
    use ait_core::binary_db_generation::{
        activate_binary_db_generation, capture_binary_db_generation,
        BinaryDbGenerationActivationOptions, CaptureBinaryDbGenerationOptions,
    };
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    fs::write(source.path().join("code.txt"), b"code\n").unwrap();
    let raw = b"first\r\n\xff\0";
    fs::write(source.path().join("README.md"), raw).unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "original"]);
    let first = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    fs::write(source.path().join("README.md"), b"second\r\n\xff\0").unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "MD only"]);
    let latest = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let ait = init_empty_ait_git_interop_repo();
    json_output(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
    );
    let native = ait.path().join(".ait/binary-db");
    let mappings = "git_commit_mapping.bin";
    let before = [
        native_git_bytes(ait.path(), mappings),
        fs::read(native.join("plan.bin")).unwrap(),
        fs::read(native.join("plan_revision.bin")).unwrap(),
    ];
    let output = TempDir::new().unwrap();
    let generation = output.path().join("generation");
    capture_binary_db_generation(CaptureBinaryDbGenerationOptions {
        repo_root: ait.path().into(),
        output_root: generation.clone(),
        jobs: 2,
    })
    .unwrap();
    let report = activate_binary_db_generation(BinaryDbGenerationActivationOptions {
        repo_root: ait.path().into(),
        generation_root: generation,
        expected_current_authority_fingerprint: None,
    })
    .unwrap();
    assert!(report.retained_previous_authority.is_some());
    assert_eq!(
        before,
        [
            native_git_bytes(ait.path(), mappings),
            fs::read(native.join("plan.bin")).unwrap(),
            fs::read(native.join("plan_revision.bin")).unwrap()
        ]
    );
    fs::rename(
        source.path().join(".git"),
        source.path().join("original-odb-offline"),
    )
    .unwrap();
    let export = output.path().join("captured.git");
    json_output(
        ait.path(),
        &["git", "export", export.to_str().unwrap(), "--json"],
    );
    assert_eq!(latest, fixture_git(&export, &["rev-parse", "HEAD"]));
    assert_eq!(
        raw,
        Command::new("git")
            .current_dir(&export)
            .args(["show", &format!("{}:README.md", first.trim())])
            .output()
            .unwrap()
            .stdout
            .as_slice()
    );
    assert_eq!(
        b"second\r\n\xff\0",
        Command::new("git")
            .current_dir(&export)
            .args(["show", "HEAD:README.md"])
            .output()
            .unwrap()
            .stdout
            .as_slice()
    );
    fixture_git(&export, &["fsck", "--strict", "--full"]);
}

#[test]
fn native_git_mirror_tag_delete_recreate_preserves_physical_slot() {
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Mirror"]);
    fixture_git(source.path(), &["config", "user.email", "mirror@local"]);
    fs::write(source.path().join("code.txt"), b"code\n").unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "source"]);
    fixture_git(source.path(), &["tag", "v1"]);
    let ait = init_empty_ait_git_interop_repo();
    let endpoint = source.path().to_str().unwrap();
    json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            endpoint,
            "--direction",
            "inbound",
            "--json",
        ],
    );
    let tag_path = ait.path().join(".ait/binary-db/tag.bin");
    let initial = fs::read(&tag_path).unwrap();
    fixture_git(source.path(), &["tag", "-d", "v1"]);
    assert_eq!(
        json_output(
            ait.path(),
            &[
                "git",
                "mirror",
                endpoint,
                "--direction",
                "inbound",
                "--json"
            ]
        )["status"],
        json!("completed")
    );
    let dead = fs::read(&tag_path).unwrap();
    assert_eq!(initial.len(), dead.len());
    assert_eq!(dead[4] & 128, 128);
    fixture_git(source.path(), &["tag", "v1"]);
    let recreated = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            endpoint,
            "--direction",
            "inbound",
            "--json",
        ],
    );
    assert_eq!(recreated["status"], json!("completed"), "{recreated:?}");
    let live = fs::read(&tag_path).unwrap();
    assert_eq!(live.len(), initial.len());
    assert_eq!(live[4] & 128, 0);
    assert_eq!(
        json_output(
            ait.path(),
            &[
                "git",
                "mirror",
                endpoint,
                "--direction",
                "inbound",
                "--json"
            ]
        )["status"],
        json!("no_op")
    );
    let tag_before = json_output(ait.path(), &["tag", "show", "v1", "--json"]);
    let tag_oid = fixture_git(source.path(), &["rev-parse", "v1"]);
    fs::write(source.path().join("incremental.txt"), b"incremental\n").unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "after Tag recreation"]);
    json_output(
        ait.path(),
        &[
            "git",
            "import",
            endpoint,
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(
        tag_before,
        json_output(ait.path(), &["tag", "show", "v1", "--json"])
    );
    let output = TempDir::new().unwrap();
    let target = output.path().join("export.git");
    json_output(
        ait.path(),
        &[
            "git",
            "export",
            target.to_str().unwrap(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(tag_oid, fixture_git(&target, &["rev-parse", "v1"]));
    assert_eq!(fs::read(&tag_path).unwrap().len(), initial.len());
}

#[test]
fn native_git_legacy_tag_migration_cli_stages_applies_and_replays() {
    let ait = init_empty_ait_git_interop_repo();
    fs::write(ait.path().join("code.txt"), b"tag source\n").unwrap();
    let snapshot = seed_snapshot(ait.path(), "legacy Tag code");
    let tags = ait.path().join(".ait/refs/tags");
    fs::create_dir_all(&tags).unwrap();
    let old=json!({"schema":1,"name":"legacy","snapshot_id":snapshot,"message":"exact retained message","created_at":"2020-01-01T00:00:00Z"}).to_string().into_bytes();
    fs::write(tags.join("legacy.json"), &old).unwrap();
    let output = TempDir::new().unwrap();
    let stage = output.path().join("generation");
    let stage_text = stage.to_str().unwrap();
    let staged = json_output(
        ait.path(),
        &["git", "migrate", "--staging-root", stage_text, "--json"],
    );
    assert_eq!(staged["status"], json!("staged"));
    assert_eq!(staged["mutated"], json!(false));
    assert_eq!(fs::read(tags.join("legacy.json")).unwrap(), old);
    let applied = json_output(
        ait.path(),
        &[
            "git",
            "migrate",
            "--staging-root",
            stage_text,
            "--apply",
            "--json",
        ],
    );
    assert_eq!(applied["status"], json!("migrated"));
    assert_eq!(applied["runtime_json_writes"], json!(0));
    assert_eq!(
        fs::read(
            ait.path()
                .join(".ait/refs/tags.legacy-git-migration/legacy.json")
        )
        .unwrap(),
        old
    );
    let tag = json_output(ait.path(), &["tag", "show", "legacy", "--json"]);
    assert_eq!(tag["snapshot_id"], json!(snapshot));
    assert_eq!(tag["message"], json!("exact retained message"));
    assert_eq!(
        json_output(
            ait.path(),
            &[
                "git",
                "migrate",
                "--staging-root",
                stage_text,
                "--apply",
                "--json"
            ]
        )["status"],
        json!("no_op")
    );
}

#[test]
fn native_git_mirror_same_snapshot_tag_metadata_is_a_real_outbound_change() {
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Mirror"]);
    fixture_git(source.path(), &["config", "user.email", "mirror@local"]);
    fs::write(source.path().join("code.txt"), b"code\n").unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "source"]);
    fixture_git(source.path(), &["tag", "v1"]);
    let ait = init_empty_ait_git_interop_repo();
    let endpoint = source.path().to_str().unwrap();
    json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            endpoint,
            "--direction",
            "inbound",
            "--json",
        ],
    );
    let old_oid = fixture_git(source.path(), &["rev-parse", "v1"]);
    let snapshot = json_output(ait.path(), &["tag", "show", "v1", "--json"])["snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    json_output(ait.path(), &["tag", "delete", "v1", "--json"]);
    json_output(
        ait.path(),
        &[
            "tag",
            "create",
            "v1",
            "--snapshot",
            &snapshot,
            "--message",
            "new native metadata",
            "--json",
        ],
    );
    let dry = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            endpoint,
            "--direction",
            "bidirectional",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry["outbound_only_count"], json!(1));
    assert_eq!(dry["divergent_count"], json!(0));
    assert_eq!(
        json_output(
            ait.path(),
            &[
                "git",
                "mirror",
                endpoint,
                "--direction",
                "outbound",
                "--json"
            ]
        )["status"],
        json!("completed")
    );
    let new_oid = fixture_git(source.path(), &["rev-parse", "v1"]);
    assert_ne!(old_oid, new_oid);
    assert!(fixture_git(source.path(), &["cat-file", "tag", "v1"]).contains("new native metadata"));
    assert_eq!(
        json_output(
            ait.path(),
            &[
                "git",
                "mirror",
                endpoint,
                "--direction",
                "bidirectional",
                "--json"
            ]
        )["status"],
        json!("no_op")
    );
}

#[test]
fn native_git_mirror_independent_inbound_outbound_changes_and_raw_tags_converge() {
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Mirror"]);
    fixture_git(source.path(), &["config", "user.email", "mirror@local"]);
    fs::write(source.path().join("code.txt"), b"code\n").unwrap();
    fs::write(source.path().join("README.md"), b"raw\r\n\xff\0").unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "source"]);
    let tree = fixture_git(source.path(), &["rev-parse", "HEAD^{tree}"]);
    let blob = fixture_git(source.path(), &["rev-parse", "HEAD:code.txt"]);
    fixture_git(source.path(), &["tag", "raw-tree", &tree]);
    fixture_git(source.path(), &["tag", "raw-blob", &blob]);
    fixture_git(
        source.path(),
        &["tag", "-a", "nested", "raw-tree", "-m", "nested tree Tag"],
    );
    let nested_oid = fixture_git(source.path(), &["rev-parse", "nested"]);
    let ait = init_empty_ait_git_interop_repo();
    let endpoint = source.path().to_str().unwrap();
    assert_eq!(
        json_output(
            ait.path(),
            &[
                "git",
                "mirror",
                endpoint,
                "--direction",
                "inbound",
                "--json"
            ]
        )["status"],
        json!("completed")
    );
    let snapshot = json_output(ait.path(), &["line", "show", "main", "--json"])["head_snapshot_id"]
        .as_str()
        .unwrap()
        .to_string();
    fixture_git(source.path(), &["branch", "from-git"]);
    json_output(
        ait.path(),
        &[
            "line",
            "create",
            "from-ait",
            "--from-snapshot",
            &snapshot,
            "--json",
        ],
    );
    for direction in ["inbound", "outbound"] {
        let blocked = json_output(
            ait.path(),
            &[
                "git",
                "mirror",
                endpoint,
                "--direction",
                direction,
                "--json",
            ],
        );
        assert_eq!(blocked["status"], json!("blocked"));
        assert_eq!(blocked["inbound_only_count"], json!(1));
        assert_eq!(blocked["outbound_only_count"], json!(1));
        assert_eq!(blocked["mutated"], json!(false));
    }
    assert_eq!(
        json_output(
            ait.path(),
            &[
                "git",
                "mirror",
                endpoint,
                "--direction",
                "bidirectional",
                "--json"
            ]
        )["status"],
        json!("completed")
    );
    assert_eq!(
        json_output(
            ait.path(),
            &[
                "git",
                "mirror",
                endpoint,
                "--direction",
                "bidirectional",
                "--json"
            ]
        )["status"],
        json!("no_op")
    );
    assert_eq!(fixture_git(source.path(), &["rev-parse", "raw-tree"]), tree);
    assert_eq!(fixture_git(source.path(), &["rev-parse", "raw-blob"]), blob);
    assert_eq!(
        fixture_git(source.path(), &["rev-parse", "nested"]),
        nested_oid
    );
    assert!(
        fixture_git(source.path(), &["show-ref", "refs/heads/from-ait"])
            .contains("refs/heads/from-ait")
    );
    assert!(
        json_output(ait.path(), &["line", "show", "from-git", "--json"])["head_snapshot_id"]
            .is_string()
    );
    fixture_git(source.path(), &["notes", "add", "-m", "preserved notes"]);
    let blocked = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            endpoint,
            "--direction",
            "bidirectional",
            "--json",
        ],
    );
    assert_eq!(blocked["status"], json!("blocked"));
    assert_eq!(blocked["mutated"], json!(false));
}

#[test]
fn native_git_binary_cli_deduplicates_projected_merge_parents() {
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    write_file(&source.path().join("code.txt"), "initial code\n");
    write_file(&source.path().join("README.md"), "initial docs\n");
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "root"]);
    let root = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    for (branch, path) in [("docs-one", "one.md"), ("docs-two", "two.MD")] {
        fixture_git(source.path(), &["checkout", "-b", branch, &root]);
        write_file(&source.path().join(path), branch);
        fixture_git(source.path(), &["add", "."]);
        fixture_git(source.path(), &["commit", "-m", branch]);
    }
    fixture_git(source.path(), &["checkout", "main"]);
    write_file(&source.path().join("README.md"), "main docs\n");
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "main docs"]);
    fixture_git(
        source.path(),
        &[
            "merge",
            "--no-ff",
            "docs-one",
            "docs-two",
            "-m",
            "docs merge",
        ],
    );
    let collapsed_merge = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let collapsed_raw = fixture_git(source.path(), &["cat-file", "commit", &collapsed_merge]);
    assert_eq!(
        collapsed_raw
            .lines()
            .filter(|line| line.starts_with("parent "))
            .count(),
        3
    );
    fixture_git(source.path(), &["checkout", "-b", "code-change", &root]);
    write_file(&source.path().join("code.txt"), "changed code\n");
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "code change"]);
    let code_change = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    fixture_git(source.path(), &["checkout", "main"]);
    fixture_git(
        source.path(),
        &[
            "merge",
            "--no-ff",
            "code-change",
            "-m",
            "ordered code merge",
        ],
    );
    let head = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let graph = fixture_git(source.path(), &["rev-list", "--parents", "--all"]);
    let refs = git_ref_map(source.path());
    let ait = init_empty_ait_git_interop_repo();
    let imported = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(imported["commit_count"], json!(7));
    let mappings = git_interop_mappings(ait.path())
        .into_iter()
        .map(|(mapping, id, raw)| (fixture_oid_hex(&mapping.object_id), (mapping, id, raw)))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        mappings[&collapsed_merge].0.snapshot_index,
        mappings[&root].0.snapshot_index
    );
    assert_eq!(
        mappings[&collapsed_merge].2,
        collapsed_raw
            .as_bytes()
            .iter()
            .copied()
            .chain(std::iter::once(b'\n'))
            .collect::<Vec<_>>()
    );
    let repo = RepoRuntime::discover_from_path(ait.path()).unwrap();
    let content = repo.binary_db_stores::<1>().content();
    let view = content
        .snapshots()
        .get_snapshot_view(
            &ait_core::binary_db::BinaryDbReadTxn::new(content.db()),
            &mappings[&head].1,
        )
        .unwrap()
        .unwrap();
    assert_eq!(
        view.parent_snapshot_ids,
        vec![mappings[&root].1.clone(), mappings[&code_change].1.clone()]
    );
    assert!(!view.parent_snapshot_ids.contains(&view.snapshot_id));
    let retained = TempDir::new().unwrap();
    fs::rename(
        source.path().join(".git"),
        retained.path().join("source.git"),
    )
    .unwrap();
    let target = TempDir::new().unwrap();
    let out = target.path().join("export.git");
    json_output(
        ait.path(),
        &[
            "git",
            "export",
            out.to_str().unwrap(),
            "--generation",
            imported["generation_id"].as_str().unwrap(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(
        fixture_git(&out, &["rev-list", "--parents", "--all"]),
        graph
    );
    assert_eq!(git_ref_map(&out), refs);
    assert_eq!(fixture_git(&out, &["rev-parse", "HEAD"]), head);
    fixture_git(&out, &["fsck", "--strict", "--full"]);
}

#[cfg(unix)]
#[test]
fn native_git_binary_export_uses_bounded_object_processes() {
    use std::os::unix::fs::PermissionsExt;
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "fixture@example.invalid"],
    );
    for i in 0..128 {
        let mut body = format!("unique-file-{i:03}\n").into_bytes();
        body.extend((0..4096).map(|j| ((j * 31 + i) % 251) as u8));
        fs::write(source.path().join(format!("code-{i:03}")), body).unwrap();
    }
    fs::write(source.path().join("README.md"), b"raw docs\r\n\xff\0").unwrap();
    fixture_git(source.path(), &["add", "."]);
    fixture_git(source.path(), &["commit", "-m", "bounded export"]);
    let head = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let tree = fixture_git(source.path(), &["ls-tree", "-r", "HEAD"]);
    let ait = init_empty_ait_git_interop_repo();
    let imported = json_output(
        ait.path(),
        &["git", "import", source.path().to_str().unwrap(), "--json"],
    );
    let retained = TempDir::new().unwrap();
    fs::rename(
        source.path().join(".git"),
        retained.path().join("source.git"),
    )
    .unwrap();
    let instrumentation = TempDir::new().unwrap();
    let real_git = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(real_git.status.success());
    let real_git = String::from_utf8(real_git.stdout)
        .unwrap()
        .trim()
        .to_string();
    let quoted_git = format!("'{}'", real_git.replace('\'', "'\\''"));
    let wrapper = instrumentation.path().join("git");
    write_file(&wrapper, &format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$AIT_TEST_GIT_CALL_LOG\"\nexec {quoted_git} \"$@\"\n"));
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();
    let calls_file = instrumentation.path().join("git-calls.txt");
    let path = format!(
        "{}:{}",
        instrumentation.path().display(),
        std::env::var("PATH").unwrap()
    );
    let log = calls_file.to_string_lossy();
    let target = TempDir::new().unwrap();
    let out = target.path().join("export.git");
    let started = std::time::Instant::now();
    let exported = json_output_with_env(
        ait.path(),
        &[
            "git",
            "export",
            out.to_str().unwrap(),
            "--generation",
            imported["generation_id"].as_str().unwrap(),
            "--json",
        ],
        &[("PATH", &path), ("AIT_TEST_GIT_CALL_LOG", &log)],
    );
    let seconds = started.elapsed().as_secs_f64();
    assert_eq!(exported["fsck"], json!("passed"));
    assert_eq!(fixture_git(&out, &["rev-parse", "HEAD"]), head);
    assert_eq!(fixture_git(&out, &["ls-tree", "-r", "HEAD"]), tree);
    assert!(!out.join("objects/info/alternates").exists());
    let calls = fs::read_to_string(calls_file).unwrap();
    let cat_file = calls
        .lines()
        .filter(|s| s.split_whitespace().any(|a| a == "cat-file"))
        .count();
    let hash_object = calls
        .lines()
        .filter(|s| s.split_whitespace().any(|a| a == "hash-object"))
        .count();
    eprintln!("bounded export objects={} seconds={seconds:.6} cat_file={cat_file} hash_object={hash_object} total_git_processes={}",exported["object_count"],calls.lines().count());
    assert_eq!(hash_object, 0, "per-object Git hash-object must not return");
    assert!(
        cat_file <= 4,
        "per-object Git cat-file must not return: {cat_file}"
    );
}
