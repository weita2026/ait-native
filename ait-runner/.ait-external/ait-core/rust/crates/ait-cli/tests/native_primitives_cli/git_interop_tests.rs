// This observer wraps Git with a POSIX shell; functional Git cases remain portable.
#[cfg(unix)]
#[test]
fn native_git_import_repeated_objects_use_two_persistent_batch_processes() {
    use std::os::unix::fs::PermissionsExt;
    let source = TempDir::new().unwrap();
    fixture_git(source.path(), &["init", "-b", "main"]);
    fixture_git(source.path(), &["config", "user.name", "Batch Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "batch@example.com"],
    );
    for index in 0..32 {
        write_file(
            &source.path().join(format!("unchanged-{index}.txt")),
            "same immutable blob\n",
        );
    }
    let pointer = "version https://git-lfs.github.com/spec/v1\noid sha256:fixture\nsize 123\n";
    write_file(&source.path().join("pointer-a"), pointer);
    write_file(&source.path().join("pointer-b"), pointer);
    fs::write(source.path().join("binary"), (0..=255).collect::<Vec<u8>>()).unwrap();
    for index in 0..8 {
        write_file(&source.path().join("counter"), &format!("{index}\n"));
        fixture_git(source.path(), &["add", "."]);
        fixture_git(
            source.path(),
            &["commit", "-m", &format!("revision {index}")],
        );
    }
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
    let ait = init_empty_ait_git_interop_repo();
    let report = json_output_with_env(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--dry-run",
            "--json",
        ],
        &[("PATH", &path), ("AIT_TEST_GIT_CALL_LOG", &log)],
    );
    assert_eq!(report["status"], json!("dry_run"));
    assert_eq!(report["commit_count"], json!(8));
    assert_eq!(report["mutated"], json!(false));
    assert_eq!(
        git_import_classification(&report, "lfs_pointers")["count"],
        json!(16)
    );
    let calls = fs::read_to_string(calls_file).unwrap();
    let cat_file_calls = calls
        .lines()
        .filter(|line| line.split_whitespace().any(|arg| arg == "cat-file"))
        .collect::<Vec<_>>();
    assert_eq!(
        cat_file_calls.len(),
        2,
        "per-file cat-file processes must not return: {calls}"
    );
    assert!(cat_file_calls
        .iter()
        .any(|line| line.ends_with("cat-file --batch")));
    assert!(cat_file_calls
        .iter()
        .any(|line| line.ends_with("cat-file --batch-check")));
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains(" ls-tree -r -z "))
            .count(),
        8
    );
    assert!(!ait.path().join(".ait/git-interop/v1").exists());
}

fn fixture_git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .expect("execute fixture git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("fixture Git metadata should be UTF-8")
        .trim()
        .to_string()
}

fn fixture_git_with_stdin(root: &Path, args: &[&str], input: &[u8]) -> String {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new("git")
        .current_dir(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("execute fixture git with stdin");
    child
        .stdin
        .as_mut()
        .expect("fixture git stdin")
        .write_all(input)
        .expect("write fixture git stdin");
    let output = child.wait_with_output().expect("wait for fixture git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("fixture Git metadata should be UTF-8")
        .trim()
        .to_string()
}

fn public_git_graph(root: &Path) -> String {
    let refs = fixture_git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname)",
            "refs/heads",
            "refs/tags",
        ],
    );
    let mut args = vec!["rev-list", "--topo-order", "--parents"];
    args.extend(refs.lines());
    fixture_git(root, &args)
}

fn configure_git_fixture_platform(root: &Path) {
    fixture_git(
        root,
        &[
            "config",
            "core.filemode",
            if cfg!(unix) { "true" } else { "false" },
        ],
    );
    fixture_git(root, &["config", "core.symlinks", "true"]);
}

fn set_git_fixture_executable(root: &Path, relative: &str, executable: bool) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            root.join(relative),
            fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
        )
        .unwrap();
    }
    // Git index mode remains authoritative on filesystems without POSIX execute bits.
    fixture_git(root, &["add", "--", relative]);
    fixture_git(
        root,
        &[
            "update-index",
            if executable {
                "--chmod=+x"
            } else {
                "--chmod=-x"
            },
            "--",
            relative,
        ],
    );
}

fn git_fixture_file_symlink(target: &str, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("fixture file symlink");
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(target, link)
        .expect("fixture requires Windows file symlink support");
}

fn assert_git_checkout_mode(root: &Path, relative: &str, executable: bool) {
    let entry = fixture_git(root, &["ls-files", "--stage", "--", relative]);
    assert_eq!(
        entry.split_whitespace().next(),
        Some(if executable { "100755" } else { "100644" })
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(root.join(relative))
                .unwrap()
                .permissions()
                .mode()
                & 0o111
                != 0,
            executable
        );
    }
}

fn build_git_interop_fixture() -> TempDir {
    let source = TempDir::new().expect("source Git fixture");
    fixture_git(source.path(), &["init", "-b", "main"]);
    configure_git_fixture_platform(source.path());
    fixture_git(source.path(), &["config", "user.name", "Git Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "git-fixture@example.com"],
    );
    write_file(&source.path().join("README.md"), "root\n");
    write_file(
        &source.path().join("scripts/run.sh"),
        "#!/bin/sh\necho executable\n",
    );
    set_git_fixture_executable(source.path(), "scripts/run.sh", true);
    write_file(
        &source.path().join("資料/說明.txt"),
        "Unicode path 與訊息\n",
    );
    fs::write(source.path().join("binary.dat"), [0_u8, 1, 2, 0xff, 0, 9]).unwrap();
    git_fixture_file_symlink("README.md", &source.path().join("README.link"));
    fixture_git(source.path(), &["add", "--all"]);
    fixture_git(source.path(), &["commit", "-m", "根提交 root"]);
    fixture_git(
        source.path(),
        &["commit", "--allow-empty", "-m", "empty commit"],
    );
    fixture_git(source.path(), &["tag", "lightweight"]);
    fixture_git(
        source.path(),
        &["tag", "-a", "release/v1", "-m", "annotated release"],
    );

    fixture_git(source.path(), &["switch", "-c", "feature/topic"]);
    write_file(&source.path().join("feature.txt"), "feature side\n");
    fixture_git(source.path(), &["add", "feature.txt"]);
    fixture_git(source.path(), &["commit", "-m", "feature commit"]);
    fixture_git(source.path(), &["switch", "main"]);
    write_file(&source.path().join("main.txt"), "main side\n");
    fixture_git(source.path(), &["add", "main.txt"]);
    fixture_git(source.path(), &["commit", "-m", "main commit"]);
    fixture_git(
        source.path(),
        &["merge", "--no-ff", "feature/topic", "-m", "merge feature"],
    );
    source
}

fn build_git_golden_history_matrix_fixture() -> TempDir {
    let source = TempDir::new().expect("golden Git fixture");
    fixture_git(source.path(), &["init", "-b", "main"]);
    configure_git_fixture_platform(source.path());
    fixture_git(source.path(), &["config", "user.name", "Golden Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "golden-fixture@example.com"],
    );
    write_file(&source.path().join("README.md"), "golden root\n");
    write_file(&source.path().join("conflict.txt"), "base\n");
    write_file(
        &source.path().join("scripts/run.sh"),
        "#!/bin/sh\necho golden\n",
    );
    set_git_fixture_executable(source.path(), "scripts/run.sh", true);
    write_file(&source.path().join("資料/說明.txt"), "Unicode golden\n");
    write_file(
        &source.path().join("move/from/file.txt"),
        "directory move\n",
    );
    write_file(&source.path().join("rename-me.txt"), "file rename\n");
    write_file(
        &source.path().join("large.bin"),
        "version https://git-lfs.github.com/spec/v1\noid sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\nsize 123456\n",
    );
    fs::write(source.path().join("binary.dat"), [0_u8, 0xff, 4, 0, 9]).unwrap();
    git_fixture_file_symlink("README.md", &source.path().join("README.link"));
    fixture_git(source.path(), &["add", "--all"]);
    fixture_git(source.path(), &["commit", "-m", "根提交 golden root"]);
    fixture_git(
        source.path(),
        &["commit", "--allow-empty", "-m", "golden empty commit"],
    );
    fixture_git(source.path(), &["tag", "lightweight"]);
    fixture_git(
        source.path(),
        &["tag", "-a", "release/v1", "-m", "annotated golden release"],
    );

    fs::rename(
        source.path().join("rename-me.txt"),
        source.path().join("renamed.txt"),
    )
    .unwrap();
    fs::rename(
        source.path().join("move/from"),
        source.path().join("move/to"),
    )
    .unwrap();
    set_git_fixture_executable(source.path(), "scripts/run.sh", false);
    fixture_git(source.path(), &["add", "--all"]);
    fixture_git(
        source.path(),
        &[
            "commit",
            "-m",
            "rename file move directory and remove executable bit",
        ],
    );

    fixture_git(source.path(), &["switch", "-c", "clean-side"]);
    write_file(&source.path().join("clean-side.txt"), "clean side\n");
    fixture_git(source.path(), &["add", "clean-side.txt"]);
    fixture_git(source.path(), &["commit", "-m", "clean side"]);
    fixture_git(source.path(), &["switch", "main"]);
    write_file(&source.path().join("clean-main.txt"), "clean main\n");
    fixture_git(source.path(), &["add", "clean-main.txt"]);
    fixture_git(source.path(), &["commit", "-m", "clean main"]);
    fixture_git(
        source.path(),
        &["merge", "--no-ff", "clean-side", "-m", "clean merge"],
    );

    fixture_git(source.path(), &["switch", "-c", "conflict-side"]);
    write_file(&source.path().join("conflict.txt"), "side\n");
    fixture_git(source.path(), &["add", "conflict.txt"]);
    fixture_git(source.path(), &["commit", "-m", "conflict side"]);
    fixture_git(source.path(), &["switch", "main"]);
    write_file(&source.path().join("conflict.txt"), "main\n");
    fixture_git(source.path(), &["add", "conflict.txt"]);
    fixture_git(source.path(), &["commit", "-m", "conflict main"]);
    let conflict = Command::new("git")
        .current_dir(source.path())
        .args(["merge", "--no-ff", "conflict-side", "-m", "must conflict"])
        .output()
        .expect("run conflicting merge");
    assert!(!conflict.status.success());
    write_file(
        &source.path().join("conflict.txt"),
        "resolved main + side\n",
    );
    fixture_git(source.path(), &["add", "conflict.txt"]);
    fixture_git(source.path(), &["commit", "-m", "resolved conflict merge"]);
    let resolved_merge = fixture_git(source.path(), &["rev-parse", "HEAD"]);

    let criss_base = resolved_merge.clone();
    fixture_git(source.path(), &["switch", "-c", "criss-a", &criss_base]);
    write_file(&source.path().join("criss-a.txt"), "A\n");
    fixture_git(source.path(), &["add", "criss-a.txt"]);
    fixture_git(source.path(), &["commit", "-m", "criss A"]);
    let criss_a = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let tree_a = fixture_git(source.path(), &["rev-parse", "HEAD^{tree}"]);
    fixture_git(source.path(), &["switch", "-c", "criss-b", &criss_base]);
    write_file(&source.path().join("criss-b.txt"), "B\n");
    fixture_git(source.path(), &["add", "criss-b.txt"]);
    fixture_git(source.path(), &["commit", "-m", "criss B"]);
    let criss_b = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let tree_b = fixture_git(source.path(), &["rev-parse", "HEAD^{tree}"]);
    let criss_left = fixture_git_with_stdin(
        source.path(),
        &["commit-tree", &tree_a, "-p", &criss_a, "-p", &criss_b],
        b"criss-cross left\n",
    );
    let criss_right = fixture_git_with_stdin(
        source.path(),
        &["commit-tree", &tree_b, "-p", &criss_b, "-p", &criss_a],
        b"criss-cross right\n",
    );
    fixture_git(
        source.path(),
        &["update-ref", "refs/heads/criss/left", &criss_left],
    );
    fixture_git(
        source.path(),
        &["update-ref", "refs/heads/criss/right", &criss_right],
    );
    fixture_git(source.path(), &["switch", "main"]);
    fixture_git(source.path(), &["branch", "-D", "criss-a", "criss-b"]);
    assert_eq!(
        fixture_git(
            source.path(),
            &["merge-base", "--all", &criss_left, &criss_right]
        )
        .lines()
        .count(),
        2
    );
    let main_head = fixture_git(source.path(), &["rev-parse", "HEAD"]);
    let main_tree = fixture_git(source.path(), &["rev-parse", "HEAD^{tree}"]);
    let signed_commit_raw = format!(
        concat!(
            "tree {main_tree}\n",
            "parent {main_head}\n",
            "author Signed Fixture <signed@example.com> 1760000000 +0000\n",
            "committer Signed Fixture <signed@example.com> 1760000000 +0000\n",
            "gpgsig -----BEGIN PGP SIGNATURE-----\n",
            " fake-signature\n",
            " -----END PGP SIGNATURE-----\n\n",
            "raw signed commit\n"
        ),
        main_tree = main_tree,
        main_head = main_head,
    );
    let signed_commit = fixture_git_with_stdin(
        source.path(),
        &["hash-object", "-t", "commit", "-w", "--stdin"],
        signed_commit_raw.as_bytes(),
    );
    fixture_git(
        source.path(),
        &["update-ref", "refs/heads/signed", &signed_commit],
    );
    let signed_tag_raw = format!(
        concat!(
            "object {signed_commit}\n",
            "type commit\n",
            "tag signed/v1\n",
            "tagger Signed Fixture <signed@example.com> 1760000001 +0000\n\n",
            "raw signed tag\n",
            "-----BEGIN PGP SIGNATURE-----\n",
            "fake-tag-signature\n",
            "-----END PGP SIGNATURE-----\n"
        ),
        signed_commit = signed_commit,
    );
    let signed_tag = fixture_git_with_stdin(
        source.path(),
        &["hash-object", "-t", "tag", "-w", "--stdin"],
        signed_tag_raw.as_bytes(),
    );
    fixture_git(
        source.path(),
        &["update-ref", "refs/tags/signed/v1", &signed_tag],
    );

    fixture_git(source.path(), &["branch", "legacy", "main"]);
    fixture_git(source.path(), &["branch", "-m", "legacy", "renamed/legacy"]);
    fixture_git(source.path(), &["branch", "deleted", "main"]);
    fixture_git(source.path(), &["branch", "-D", "deleted"]);
    fixture_git(
        source.path(),
        &["notes", "add", "-m", "golden note", "main"],
    );
    fixture_git(source.path(), &["fsck", "--full", "--no-dangling"]);
    source
}

fn init_empty_ait_git_interop_repo() -> TempDir {
    let temp = TempDir::new().expect("AIT Git interop fixture");
    json_output(temp.path(), &["init", "--json"]);
    temp
}

fn git_ref_map(root: &Path) -> BTreeMap<String, String> {
    fixture_git(
        root,
        &[
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads",
            "refs/tags",
        ],
    )
    .lines()
    .map(|line| {
        let (name, object_id) = line.split_once(' ').expect("ref row");
        (name.to_string(), object_id.to_string())
    })
    .collect()
}

fn git_import_classification<'a>(payload: &'a JsonValue, kind: &str) -> &'a JsonValue {
    payload["classifications"]
        .as_array()
        .expect("Git import classifications")
        .iter()
        .find(|row| row["kind"] == json!(kind))
        .unwrap_or_else(|| panic!("missing Git import classification {kind}"))
}

fn git_interop_mappings(
    root: &Path,
) -> Vec<(
    ait_core::git_binary_db::GitCommitMappingRecord,
    String,
    Vec<u8>,
)> {
    use ait_core::binary_db::{AuthorityId, BinaryDbReadTxn};
    use ait_core::git_binary_db::*;
    let repo = RepoRuntime::discover_from_path(root).unwrap();
    let content = repo.binary_db_stores::<1>().content();
    let db = GitInteropBundle::open(root, false)
        .unwrap()
        .database(root, AuthorityId::new("fixture"));
    let objects = LocalGitObjectStore::open(db.clone(), content.clone()).unwrap();
    let history = GitHistoryStore::open(db).unwrap();
    let generations = history.generations().unwrap();
    let read = BinaryDbReadTxn::new(content.db());
    history
        .commit_mappings()
        .into_iter()
        .map(|mapping| {
            let snapshot = content
                .snapshots()
                .snapshot_view_at(&read, mapping.snapshot_index)
                .unwrap();
            let body = objects
                .get(
                    generations[mapping.generation_index as usize].source_repository_index,
                    1,
                    &mapping.object_id,
                )
                .unwrap()
                .unwrap()
                .1;
            (mapping, snapshot.snapshot_id, body)
        })
        .collect()
}
fn fixture_oid_hex(id: &[u8]) -> String {
    id.iter().map(|v| format!("{v:02x}")).collect()
}

#[test]
fn native_git_import_export_roundtrip_reuses_objects_and_preserves_git_semantics() {
    let source = build_git_interop_fixture();
    let source_refs = git_ref_map(source.path());
    let original_main = source_refs["refs/heads/main"].clone();
    let feature_head = source_refs["refs/heads/feature/topic"].clone();
    let source_graph = fixture_git(
        source.path(),
        &["rev-list", "--all", "--topo-order", "--parents"],
    );
    let source_commit_count = source_graph.lines().count();
    let ait = init_empty_ait_git_interop_repo();

    let dry_run = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry_run["status"], json!("dry_run"));
    assert_eq!(dry_run["commit_count"], json!(source_commit_count));
    assert_eq!(dry_run["line_count"], json!(2));
    assert_eq!(dry_run["tag_count"], json!(2));
    assert_eq!(dry_run["mutated"], json!(false));
    assert!(!ait.path().join(".ait/git-interop/v1").exists());

    let imported = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(imported["status"], json!("completed"));
    assert_eq!(imported["plan_hash"], dry_run["plan_hash"]);
    assert_eq!(imported["classifications"], dry_run["classifications"]);
    assert_eq!(imported["commit_count"], json!(source_commit_count));
    assert_eq!(
        imported["imported_commit_count"],
        json!(source_commit_count)
    );
    assert_eq!(imported["line_count"], json!(2));
    assert_eq!(imported["tag_count"], json!(2));
    assert_eq!(imported["head_symbolic_ref"], json!("refs/heads/main"));
    assert_eq!(imported["symbolic_head_mapped"], json!(true));
    assert!(
        json_output(ait.path(), &["line", "show", "main", "--json"])["head_snapshot_id"]
            .is_string()
    );
    assert!(
        json_output(ait.path(), &["line", "show", "feature/topic", "--json"])["head_snapshot_id"]
            .is_string()
    );
    assert_eq!(
        json_output(ait.path(), &["tag", "list", "--json"])
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let target_parent = TempDir::new().expect("export target parent");
    let target = target_parent.path().join("escape.git");
    let exported = json_output(
        ait.path(),
        &[
            "git",
            "export",
            target.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(exported["status"], json!("completed"));
    assert_eq!(exported["fsck"], json!("passed"));
    assert_eq!(exported["head_symbolic_ref"], json!("refs/heads/main"));
    assert_eq!(
        exported["exact_git_object_reuse_count"],
        json!(source_commit_count)
    );
    assert_eq!(exported["native_commit_count"], json!(0));
    assert_eq!(git_ref_map(&target), source_refs);
    assert_eq!(
        fixture_git(&target, &["symbolic-ref", "HEAD"]),
        "refs/heads/main"
    );
    assert_eq!(
        fixture_git(&target, &["rev-list", "--all", "--topo-order", "--parents"]),
        source_graph
    );
    fixture_git(&target, &["fsck", "--full", "--no-dangling"]);

    let checkout = target_parent.path().join("checkout");
    fixture_git(
        target_parent.path(),
        &[
            "clone",
            "--config",
            "core.symlinks=true",
            target.to_string_lossy().as_ref(),
            checkout.to_string_lossy().as_ref(),
        ],
    );
    assert_eq!(
        fs::read(checkout.join("binary.dat")).unwrap(),
        [0_u8, 1, 2, 0xff, 0, 9]
    );
    assert_eq!(
        fs::read_to_string(checkout.join("資料/說明.txt")).unwrap(),
        "Unicode path 與訊息\n"
    );
    assert!(fs::symlink_metadata(checkout.join("README.link"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_git_checkout_mode(&checkout, "scripts/run.sh", true);

    let replayed_import = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(replayed_import["status"], json!("no_op"));
    assert_eq!(replayed_import["replayed"], json!(true));
    let replayed_export = json_output(
        ait.path(),
        &[
            "git",
            "export",
            target.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(replayed_export["status"], json!("no_op"));
    assert_eq!(replayed_export["replayed"], json!(true));

    write_file(
        &source.path().join("incremental.txt"),
        "incremental history\n",
    );
    fixture_git(source.path(), &["add", "incremental.txt"]);
    fixture_git(source.path(), &["commit", "-m", "incremental commit"]);
    let refreshed_source_refs = git_ref_map(source.path());
    let incrementally_imported = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(incrementally_imported["status"], json!("completed"));
    assert_eq!(incrementally_imported["imported_commit_count"], json!(1));

    fixture_git(
        &target,
        &[
            "update-ref",
            "refs/heads/main",
            feature_head.as_str(),
            original_main.as_str(),
        ],
    );
    let refused = command_output_with_env(
        ait.path(),
        &[
            "git",
            "export",
            target.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
        &[],
    );
    assert!(!refused.status.success());
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(
        stderr.contains("refuses to overwrite refs/heads/main"),
        "{stderr}"
    );
    assert_eq!(
        fixture_git(&target, &["rev-parse", "refs/heads/main"]),
        feature_head
    );

    fixture_git(
        &target,
        &[
            "update-ref",
            "refs/heads/main",
            original_main.as_str(),
            feature_head.as_str(),
        ],
    );
    let automatically_resumed_export = json_output(
        ait.path(),
        &[
            "git",
            "export",
            target.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(automatically_resumed_export["status"], json!("completed"));
    assert_eq!(automatically_resumed_export["fsck"], json!("passed"));
    assert_eq!(git_ref_map(&target), refreshed_source_refs);
    fixture_git(&target, &["fsck", "--full", "--no-dangling"]);

    let mapping_count = git_interop_mappings(ait.path()).len();
    assert!(mapping_count > source_commit_count);
    assert!(!ait.path().join(".ait/git-interop/v1/mappings").exists());
}

#[test]
fn native_git_import_automatically_resumes_the_same_immutable_plan() {
    let source = build_git_interop_fixture();
    fixture_git(source.path(), &["branch", "-m", "main", "git-main"]);

    let ait = init_empty_ait_git_interop_repo();
    write_file(&ait.path().join("local-only.txt"), "local history\n");
    let local_snapshot = seed_snapshot(ait.path(), "local history");
    json_output(
        ait.path(),
        &[
            "tag",
            "create",
            "lightweight",
            "--snapshot",
            local_snapshot.as_str(),
            "--message",
            "local conflicting tag",
            "--json",
        ],
    );

    let interrupted = command_output_with_env(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
        &[],
    );
    assert!(!interrupted.status.success());
    let stderr = String::from_utf8_lossy(&interrupted.stderr);
    assert!(
        stderr.contains("refuses to replace AIT tag lightweight"),
        "{stderr}"
    );

    let dry = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--all-branches-and-tags",
            "--dry-run",
            "--json",
        ],
    );
    let operation_id = dry["operation_id"].as_str().unwrap().to_string();
    let checkpoint_path = "git_operation_checkpoint.bin";
    let interrupted_checkpoint = native_git_bytes(ait.path(), checkpoint_path);
    let refs_path = "git_ref_mapping.bin";
    let frozen_refs = native_git_bytes(ait.path(), refs_path);
    assert_eq!(interrupted_checkpoint.len(), 4 + 44);
    assert_eq!(interrupted_checkpoint[4], 0);
    assert_eq!(
        u32::from_le_bytes(interrupted_checkpoint[36..40].try_into().unwrap()),
        2
    );
    assert!(!ait.path().join(".ait/git-interop/v1/operations").exists());
    let mut mismatched_checkpoint = interrupted_checkpoint.clone();
    mismatched_checkpoint[20..32].fill(255);
    native_git_write(ait.path(), checkpoint_path, &mismatched_checkpoint);
    let mismatched = command_output_with_env(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--all-branches-and-tags",
            "--json",
        ],
        &[],
    );
    assert!(!mismatched.status.success());
    assert!(String::from_utf8_lossy(&mismatched.stderr)
        .contains("does not match the current immutable plan"));
    assert_eq!(
        mismatched_checkpoint,
        native_git_bytes(ait.path(), checkpoint_path)
    );
    native_git_write(ait.path(), checkpoint_path, &interrupted_checkpoint);

    json_output(ait.path(), &["tag", "delete", "lightweight", "--json"]);
    let resumed = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(resumed["status"], json!("completed"));
    assert_eq!(resumed["operation_id"], json!(operation_id));
    assert_eq!(resumed["tag_count"], json!(2));
    let resumed_refs = native_git_bytes(ait.path(), refs_path);
    assert_eq!(&resumed_refs[..frozen_refs.len()], frozen_refs.as_slice());
    assert!(
        json_output(ait.path(), &["line", "show", "git-main", "--json"])["head_snapshot_id"]
            .is_string()
    );
    assert_eq!(
        json_output(ait.path(), &["tag", "list", "--json"])
            .as_array()
            .expect("imported tags")
            .len(),
        2
    );
}

#[test]
fn native_git_import_reports_sha256_as_an_explicit_unsupported_capability() {
    let source = TempDir::new().expect("SHA-256 Git fixture");
    let init = Command::new("git")
        .current_dir(source.path())
        .args(["init", "--object-format=sha256", "-b", "main"])
        .output()
        .expect("probe SHA-256 Git support");
    if !init.status.success() {
        return;
    }
    fixture_git(source.path(), &["config", "user.name", "SHA Fixture"]);
    fixture_git(
        source.path(),
        &["config", "user.email", "sha-fixture@example.com"],
    );
    write_file(&source.path().join("README.md"), "sha256\n");
    fixture_git(source.path(), &["add", "README.md"]);
    fixture_git(source.path(), &["commit", "-m", "sha256 root"]);

    let ait = init_empty_ait_git_interop_repo();
    let report = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(report["status"], json!("blocked"));
    assert_eq!(report["git_object_format"], json!("sha256"));
    assert_eq!(
        report["blockers"][0]["kind"],
        json!("unsupported_object_format")
    );
    assert_eq!(report["mutated"], json!(false));
    assert!(!ait.path().join(".ait/git-interop/v1").exists());

    let blocked = command_output_with_env(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--json",
        ],
        &[],
    );
    assert!(!blocked.status.success());
    assert!(String::from_utf8_lossy(&blocked.stderr).contains("supports sha1 only"));

    let export_report = json_output(
        ait.path(),
        &[
            "git",
            "export",
            source.path().to_string_lossy().as_ref(),
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(export_report["status"], json!("blocked"));
    assert_eq!(export_report["git_object_format"], json!("sha256"));
    assert_eq!(export_report["mutated"], json!(false));
}

#[test]
fn native_git_roundtrip_preserves_an_imported_nondefault_symbolic_head() {
    let source = build_git_interop_fixture();
    fixture_git(source.path(), &["switch", "feature/topic"]);
    let source_refs = git_ref_map(source.path());
    let ait = init_empty_ait_git_interop_repo();
    let imported = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(
        imported["head_symbolic_ref"],
        json!("refs/heads/feature/topic")
    );

    let targets = TempDir::new().expect("symbolic HEAD export target");
    let target = targets.path().join("escape.git");
    let exported = json_output(
        ait.path(),
        &[
            "git",
            "export",
            target.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(
        exported["head_symbolic_ref"],
        json!("refs/heads/feature/topic")
    );
    assert_eq!(
        fixture_git(&target, &["symbolic-ref", "HEAD"]),
        "refs/heads/feature/topic"
    );
    assert_eq!(git_ref_map(&target), source_refs);
}

#[test]
fn native_git_import_fails_closed_for_replace_refs_and_submodules() {
    let replace_source = build_git_interop_fixture();
    let replace_refs = git_ref_map(replace_source.path());
    fixture_git(
        replace_source.path(),
        &[
            "replace",
            replace_refs["refs/heads/feature/topic"].as_str(),
            replace_refs["refs/heads/main"].as_str(),
        ],
    );
    let replace_ait = init_empty_ait_git_interop_repo();
    let replace_report = json_output(
        replace_ait.path(),
        &[
            "git",
            "import",
            replace_source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(replace_report["status"], json!("blocked"));
    assert!(replace_report["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["kind"] == json!("replace_refs")));
    assert!(!replace_ait.path().join(".ait/git-interop/v1").exists());
    let replace_blocked = command_output_with_env(
        replace_ait.path(),
        &[
            "git",
            "import",
            replace_source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
        &[],
    );
    assert!(!replace_blocked.status.success());
    assert!(String::from_utf8_lossy(&replace_blocked.stderr).contains("replace ref(s)"));

    let submodule_source = build_git_interop_fixture();
    let gitlink_object = fixture_git(submodule_source.path(), &["rev-parse", "HEAD"]);
    fixture_git(
        submodule_source.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{gitlink_object},vendor/dependency"),
        ],
    );
    fixture_git(submodule_source.path(), &["commit", "-m", "add gitlink"]);
    let submodule_ait = init_empty_ait_git_interop_repo();
    let submodule_report = json_output(
        submodule_ait.path(),
        &[
            "git",
            "import",
            submodule_source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(submodule_report["status"], json!("blocked"));
    assert!(submodule_report["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|row| row["kind"] == json!("submodules")));
    let blocked_line = json_output(submodule_ait.path(), &["line", "show", "main", "--json"]);
    assert_eq!(blocked_line["head_snapshot_id"], JsonValue::Null);

    let submodule_blocked = command_output_with_env(
        submodule_ait.path(),
        &[
            "git",
            "import",
            submodule_source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
        &[],
    );
    assert!(!submodule_blocked.status.success());
    assert!(String::from_utf8_lossy(&submodule_blocked.stderr)
        .contains("stopped before changing any AIT Snapshot or ref"));
    assert_eq!(
        json_output(submodule_ait.path(), &["line", "show", "main", "--json"])["head_snapshot_id"],
        JsonValue::Null
    );
    assert!(!submodule_ait
        .path()
        .join(".ait/git-interop/v1/mappings")
        .exists());
}

#[test]
fn native_ait_snapshots_export_deterministically_and_roundtrip_back_through_ait() {
    let ait = init_empty_ait_git_interop_repo();
    write_file(&ait.path().join("native.txt"), "native root\n");
    write_file(&ait.path().join("bin/tool.sh"), "#!/bin/sh\necho native\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            ait.path().join("bin/tool.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }
    git_fixture_file_symlink("native.txt", &ait.path().join("native.link"));
    let root_snapshot = seed_snapshot(ait.path(), "native root");

    write_file(&ait.path().join("native.txt"), "native second\n");
    fs::write(ait.path().join("payload.bin"), [0_u8, 0xff, 7, 0]).unwrap();
    let second_snapshot = seed_snapshot(ait.path(), "native second");
    assert_ne!(root_snapshot, second_snapshot);
    json_output(
        ait.path(),
        &[
            "tag",
            "create",
            "native/v2",
            "--snapshot",
            second_snapshot.as_str(),
            "--message",
            "native deterministic tag",
            "--json",
        ],
    );

    let targets = TempDir::new().expect("native export targets");
    let first = targets.path().join("first.git");
    let second = targets.path().join("second.git");
    let dry_target = targets.path().join("dry-run.git");
    let dry_export = json_output(
        ait.path(),
        &[
            "git",
            "export",
            dry_target.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry_export["status"], json!("dry_run"));
    assert_eq!(dry_export["native_commit_count"], json!(2));
    assert_eq!(dry_export["mutated"], json!(false));
    assert!(!dry_target.exists());
    assert!(!ait.path().join(".ait/git-interop/v1").exists());

    let first_export = json_output(
        ait.path(),
        &[
            "git",
            "export",
            first.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(first_export["status"], json!("completed"));
    assert_eq!(first_export["native_commit_count"], json!(2));
    assert_eq!(first_export["exact_git_object_reuse_count"], json!(0));
    assert_eq!(first_export["head_symbolic_ref"], json!("refs/heads/main"));

    let second_export = json_output(
        ait.path(),
        &[
            "git",
            "export",
            second.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(second_export["status"], json!("completed"));
    assert_eq!(git_ref_map(&first), git_ref_map(&second));
    assert_eq!(
        fixture_git(&first, &["rev-list", "--all", "--parents"]),
        fixture_git(&second, &["rev-list", "--all", "--parents"])
    );
    assert_eq!(
        fixture_git(&first, &["cat-file", "-p", "refs/tags/native/v2"]),
        fixture_git(&second, &["cat-file", "-p", "refs/tags/native/v2"])
    );
    fixture_git(&first, &["fsck", "--full", "--no-dangling"]);
    fixture_git(&second, &["fsck", "--full", "--no-dangling"]);

    let imported = init_empty_ait_git_interop_repo();
    let import_result = json_output(
        imported.path(),
        &[
            "git",
            "import",
            first.to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(import_result["status"], json!("completed"));
    assert_eq!(import_result["commit_count"], json!(2));

    let third = targets.path().join("third.git");
    let third_export = json_output(
        imported.path(),
        &[
            "git",
            "export",
            third.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(third_export["exact_git_object_reuse_count"], json!(2));
    assert_eq!(third_export["native_commit_count"], json!(0));
    assert_eq!(git_ref_map(&first), git_ref_map(&third));
    assert_eq!(
        fixture_git(&first, &["rev-list", "--all", "--parents"]),
        fixture_git(&third, &["rev-list", "--all", "--parents"])
    );
    fixture_git(&third, &["fsck", "--full", "--no-dangling"]);
}

#[test]
fn native_git_roundtrip_golden_history_matrix_preserves_supported_semantics() {
    let source = build_git_golden_history_matrix_fixture();
    let source_refs = git_ref_map(source.path());
    let source_graph = public_git_graph(source.path());
    let source_commit_ids = source_graph
        .lines()
        .map(|line| line.split_whitespace().next().expect("commit graph row"))
        .collect::<BTreeSet<_>>();
    let source_main_tree = fixture_git(source.path(), &["ls-tree", "-r", "refs/heads/main"]);
    let resolved_merge = fixture_git(
        source.path(),
        &[
            "log",
            "--all",
            "--format=%H",
            "--grep=^resolved conflict merge$",
            "-1",
        ],
    );
    let resolved_merge_parents = fixture_git(
        source.path(),
        &["rev-list", "--parents", "-n", "1", &resolved_merge],
    );
    assert_eq!(resolved_merge_parents.split_whitespace().count(), 3);
    let source_criss_bases = fixture_git(
        source.path(),
        &[
            "merge-base",
            "--all",
            "refs/heads/criss/left",
            "refs/heads/criss/right",
        ],
    )
    .lines()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    assert_eq!(source_criss_bases.len(), 2);
    let source_note_refs = fixture_git(
        source.path(),
        &["for-each-ref", "--format=%(refname)", "refs/notes"],
    );
    assert_eq!(source_note_refs, "refs/notes/commits");
    assert!(source_refs.contains_key("refs/heads/renamed/legacy"));
    assert!(!source_refs.contains_key("refs/heads/legacy"));
    assert!(!source_refs.contains_key("refs/heads/deleted"));

    let ait = init_empty_ait_git_interop_repo();
    let dry_run = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry_run["commit_count"], json!(source_commit_ids.len()));
    assert_eq!(dry_run["status"], json!("blocked"));
    for (kind, count, disposition) in [
        ("signed_commits", 1, "preserved_raw_unverified"),
        ("signed_tags", 1, "preserved_raw_unverified"),
        ("notes_refs", 1, "unsupported"),
    ] {
        let row = git_import_classification(&dry_run, kind);
        assert_eq!(row["count"], json!(count));
        assert_eq!(row["disposition"], json!(disposition));
    }
    let lfs = git_import_classification(&dry_run, "lfs_pointers");
    assert!(lfs["count"].as_u64().unwrap() > 0);
    assert_eq!(lfs["disposition"], json!("pointer_content_preserved"));
    assert_eq!(dry_run["mutated"], json!(false));
    assert!(!ait.path().join(".ait/git-interop/v1").exists());

    let rejected_notes = command_output_with_env(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_str().unwrap(),
            "--all-branches-and-tags",
            "--json",
        ],
        &[],
    );
    assert!(!rejected_notes.status.success());
    assert!(
        String::from_utf8_lossy(&rejected_notes.stderr).contains("notes ref(s) are unsupported")
    );
    assert!(!ait.path().join(".ait/git-interop/v1").exists());
    fixture_git(source.path(), &["update-ref", "-d", "refs/notes/commits"]);

    let imported = json_output(
        ait.path(),
        &[
            "git",
            "import",
            source.path().to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(imported["status"], json!("completed"));
    assert_eq!(imported["commit_count"], json!(source_commit_ids.len()));
    assert_eq!(imported["line_count"], json!(7));
    assert_eq!(imported["tag_count"], json!(3));

    let commit_mappings = git_interop_mappings(ait.path());
    assert_eq!(commit_mappings.len(), source_commit_ids.len());
    for object_id in &source_commit_ids {
        let (mapping, snapshot_id, raw) = commit_mappings
            .iter()
            .find(|(m, _, _)| fixture_oid_hex(&m.object_id) == **object_id)
            .unwrap_or_else(|| panic!("missing mapping for {object_id}"));
        assert!(snapshot_id.starts_with("SNP-"));
        assert!(mapping.mapping_meta & 8 != 0);
        let original = Command::new("git")
            .current_dir(source.path())
            .args(["cat-file", "commit", object_id])
            .output()
            .unwrap();
        assert!(original.status.success());
        assert_eq!(*raw, original.stdout);
        assert_eq!(
            fixture_oid_hex(&mapping.tree_object_id),
            fixture_git(
                source.path(),
                &["rev-parse", &format!("{object_id}^{{tree}}")]
            )
        );
    }
    let signed_commit = source_refs["refs/heads/signed"].as_str();
    assert!(
        commit_mappings
            .iter()
            .find(|(m, _, _)| fixture_oid_hex(&m.object_id) == signed_commit)
            .unwrap()
            .0
            .mapping_meta
            & 1
            != 0
    );
    assert!(!ait.path().join(".ait/git-interop/v1/mappings").exists());

    let targets = TempDir::new().expect("golden export targets");
    let target = targets.path().join("golden.git");
    let exported = json_output(
        ait.path(),
        &[
            "git",
            "export",
            target.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(exported["status"], json!("completed"));
    assert_eq!(exported["fsck"], json!("passed"));
    assert_eq!(
        exported["exact_git_object_reuse_count"],
        json!(source_commit_ids.len())
    );
    assert_eq!(exported["native_commit_count"], json!(0));
    assert_eq!(git_ref_map(&target), source_refs);
    assert_eq!(public_git_graph(&target), source_graph);
    assert_eq!(
        fixture_git(&target, &["ls-tree", "-r", "refs/heads/main"]),
        source_main_tree
    );
    assert_eq!(
        fixture_git(
            &target,
            &["rev-list", "--parents", "-n", "1", &resolved_merge]
        ),
        resolved_merge_parents
    );
    let target_criss_bases = fixture_git(
        &target,
        &[
            "merge-base",
            "--all",
            "refs/heads/criss/left",
            "refs/heads/criss/right",
        ],
    )
    .lines()
    .map(str::to_string)
    .collect::<BTreeSet<_>>();
    assert_eq!(target_criss_bases, source_criss_bases);
    assert!(fixture_git(
        &target,
        &["for-each-ref", "--format=%(refname)", "refs/notes"]
    )
    .is_empty());
    for object_id in &source_commit_ids {
        assert_eq!(
            fixture_git(&target, &["cat-file", "-p", object_id]),
            fixture_git(source.path(), &["cat-file", "-p", object_id])
        );
    }
    for tag in ["release/v1", "signed/v1"] {
        assert_eq!(
            fixture_git(&target, &["cat-file", "-p", &format!("refs/tags/{tag}")]),
            fixture_git(
                source.path(),
                &["cat-file", "-p", &format!("refs/tags/{tag}")]
            )
        );
    }
    fixture_git(&target, &["fsck", "--full", "--no-dangling"]);

    let checkout = targets.path().join("checkout");
    fixture_git(
        targets.path(),
        &[
            "clone",
            "--config",
            "core.symlinks=true",
            target.to_string_lossy().as_ref(),
            checkout.to_string_lossy().as_ref(),
        ],
    );
    assert_eq!(
        fs::read(checkout.join("binary.dat")).unwrap(),
        [0_u8, 0xff, 4, 0, 9]
    );
    assert_eq!(
        fs::read_to_string(checkout.join("資料/說明.txt")).unwrap(),
        "Unicode golden\n"
    );
    assert!(fs::symlink_metadata(checkout.join("README.link"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_git_checkout_mode(&checkout, "scripts/run.sh", false);
    assert_eq!(
        fs::read_to_string(checkout.join("renamed.txt")).unwrap(),
        "file rename\n"
    );
    assert_eq!(
        fs::read_to_string(checkout.join("move/to/file.txt")).unwrap(),
        "directory move\n"
    );
    assert!(fs::read_to_string(checkout.join("large.bin"))
        .unwrap()
        .starts_with("version https://git-lfs.github.com/spec/v1\n"));
    assert!(!checkout.join("rename-me.txt").exists());
    assert!(!checkout.join("move/from").exists());

    let second_ait = init_empty_ait_git_interop_repo();
    let second_import = json_output(
        second_ait.path(),
        &[
            "git",
            "import",
            target.to_string_lossy().as_ref(),
            "--all-branches-and-tags",
            "--json",
        ],
    );
    assert_eq!(second_import["status"], json!("completed"));
    assert_eq!(
        second_import["commit_count"],
        json!(source_commit_ids.len())
    );
    let second_commit_ids = git_interop_mappings(second_ait.path())
        .into_iter()
        .map(|(mapping, _, _)| fixture_oid_hex(&mapping.object_id))
        .collect::<BTreeSet<_>>();
    assert_eq!(
        second_commit_ids,
        source_commit_ids
            .iter()
            .map(|object_id| (*object_id).to_string())
            .collect::<BTreeSet<_>>()
    );
    let second_target = targets.path().join("golden-second.git");
    let second_export = json_output(
        second_ait.path(),
        &[
            "git",
            "export",
            second_target.to_string_lossy().as_ref(),
            "--all-lines-and-tags",
            "--json",
        ],
    );
    assert_eq!(second_export["status"], json!("completed"));
    assert_eq!(git_ref_map(&second_target), source_refs);
    assert_eq!(public_git_graph(&second_target), source_graph);
    fixture_git(&second_target, &["fsck", "--full", "--no-dangling"]);
}

#[test]
fn native_git_mirror_reconciles_one_sided_changes_and_stops_divergence() {
    let source = build_git_interop_fixture();
    fixture_git(source.path(), &["branch", "doomed"]);
    let ait = init_empty_ait_git_interop_repo();

    let dry_inbound = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "inbound",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(dry_inbound["status"], json!("dry_run"));
    assert_eq!(dry_inbound["state"], json!("inbound_only"));
    assert_eq!(dry_inbound["inbound_only_count"], json!(5));
    assert_eq!(dry_inbound["mutated"], json!(false));
    assert!(!ait.path().join(".ait/git-interop/v1").exists());

    let initial = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "inbound",
            "--json",
        ],
    );
    assert_eq!(initial["status"], json!("completed"));
    assert_eq!(initial["state"], json!("equal"));
    assert_eq!(initial["compare_and_swap"], json!(true));
    assert_eq!(initial["force_updated"], json!(false));

    let replay = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "inbound",
            "--json",
        ],
    );
    assert_eq!(replay["status"], json!("no_op"));
    assert_eq!(replay["state"], json!("equal"));
    assert_eq!(replay["mutated"], json!(false));

    fixture_git(
        source.path(),
        &["branch", "-m", "feature/topic", "feature/renamed"],
    );
    fixture_git(source.path(), &["branch", "-D", "doomed"]);
    let renamed = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "inbound",
            "--json",
        ],
    );
    assert_eq!(renamed["status"], json!("completed"));
    assert_eq!(renamed["state"], json!("equal"));
    assert!(
        json_output(ait.path(), &["line", "show", "feature/renamed", "--json"])["head_snapshot_id"]
            .is_string()
    );
    for removed in ["feature/topic", "doomed"] {
        let output = command_output_with_env(ait.path(), &["line", "show", removed, "--json"], &[]);
        assert!(!output.status.success(), "{removed} should be tombstoned");
    }

    write_file(&source.path().join("from-git.txt"), "inbound update\n");
    fixture_git(source.path(), &["add", "from-git.txt"]);
    fixture_git(source.path(), &["commit", "-m", "inbound mirror update"]);
    let inbound_plan = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "bidirectional",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(inbound_plan["inbound_only_count"], json!(1));
    assert_eq!(inbound_plan["divergent_count"], json!(0));
    let inbound = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "bidirectional",
            "--json",
        ],
    );
    assert_eq!(inbound["status"], json!("completed"));
    assert_eq!(inbound["state"], json!("equal"));

    write_file(&ait.path().join("from-ait.txt"), "outbound update\n");
    let ait_outbound_snapshot = seed_snapshot(ait.path(), "outbound mirror update");
    let outbound_plan = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "bidirectional",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(outbound_plan["outbound_only_count"], json!(1));
    assert_eq!(outbound_plan["divergent_count"], json!(0));
    let outbound = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "bidirectional",
            "--json",
        ],
    );
    assert_eq!(outbound["status"], json!("completed"));
    assert_eq!(outbound["state"], json!("equal"));
    let outbound_git_head = fixture_git(source.path(), &["rev-parse", "refs/heads/main"]);
    assert_eq!(
        json_output(ait.path(), &["line", "show", "main", "--json"])["head_snapshot_id"],
        json!(ait_outbound_snapshot)
    );

    fixture_git(source.path(), &["reset", "--hard", "refs/heads/main"]);
    write_file(&source.path().join("git-diverged.txt"), "git divergence\n");
    fixture_git(source.path(), &["add", "git-diverged.txt"]);
    fixture_git(source.path(), &["commit", "-m", "git divergence"]);
    let divergent_git_head = fixture_git(source.path(), &["rev-parse", "refs/heads/main"]);
    assert_ne!(divergent_git_head, outbound_git_head);
    write_file(&ait.path().join("ait-diverged.txt"), "ait divergence\n");
    let divergent_ait_snapshot = seed_snapshot(ait.path(), "ait divergence");

    let blocked = json_output(
        ait.path(),
        &[
            "git",
            "mirror",
            source.path().to_string_lossy().as_ref(),
            "--direction",
            "bidirectional",
            "--json",
        ],
    );
    assert_eq!(blocked["status"], json!("blocked"));
    assert_eq!(blocked["state"], json!("divergent"));
    assert_eq!(blocked["divergent_count"], json!(1));
    assert_eq!(blocked["requires_decision"], json!(true));
    assert_eq!(blocked["mutated"], json!(false));
    assert_eq!(
        fixture_git(source.path(), &["rev-parse", "refs/heads/main"]),
        divergent_git_head
    );
    assert_eq!(
        json_output(ait.path(), &["line", "show", "main", "--json"])["head_snapshot_id"],
        json!(divergent_ait_snapshot)
    );
}
