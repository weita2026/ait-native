use super::*;

#[test]
fn bounded_acceptance_init_without_agent_session() {
    let Some(root) = std::env::var_os("AIT_GIT_ACCEPTANCE_INIT_ROOT") else {
        return;
    };
    let payload = crate::init_surface::init_repo(&crate::init_surface::InitRequest {
        root: PathBuf::from(root),
        name: Some("git-bundle-acceptance".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    println!(
        "AIT_ACCEPTANCE_INIT={}",
        encode_value(&payload, "acceptance init").unwrap()
    );
}
use crate::init_surface::{init_repo, InitRequest};
use ait_core::tag_store::FilesystemTagStore;

fn git(path: &Path, args: &[&str]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}
fn authority_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut result = BTreeMap::new();
    for path in fs::read_dir(root.join(".ait/binary-db")).unwrap() {
        let path = path.unwrap().path();
        if path.is_file() {
            result.insert(
                path.file_name().unwrap().to_str().unwrap().into(),
                fs::read(path).unwrap(),
            );
        }
    }
    result
}
fn legacy_fixture(temp: &Path) -> RepoRuntime {
    let source = temp.join("source");
    let ait = temp.join("ait");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&ait).unwrap();
    git(&source, &["init", "-b", "main"]);
    git(&source, &["config", "user.name", "Legacy"]);
    git(&source, &["config", "user.email", "legacy@local"]);
    fs::write(source.join("code.txt"), b"code\n").unwrap();
    fs::write(source.join("README.md"), b"first\r\n\xff\0").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-m", "first"]);
    let first = String::from_utf8(git(&source, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_string();
    fs::write(source.join("README.md"), b"second\r\n\xfe\0").unwrap();
    git(&source, &["add", "."]);
    git(&source, &["commit", "-m", "MD only"]);
    let latest = String::from_utf8(git(&source, &["rev-parse", "HEAD"]))
        .unwrap()
        .trim()
        .to_string();
    git(&source, &["tag", "v1", &first]);
    init_repo(&InitRequest {
        root: ait.clone(),
        name: Some("legacy".into()),
        default_line: "main".into(),
        policy_profile: "prototype".into(),
        default_author_mode: "ai_with_human_review".into(),
        default_model: None,
        repair_existing: false,
    })
    .unwrap();
    let repo = RepoRuntime::discover_from_path(&ait).unwrap();
    let content = repo.binary_db_stores::<1>().content();
    let code = content.ensure_blob_bytes_batch(&[b"code\n"]).unwrap()[0];
    let view = content
        .create_git_code_snapshot(
            "legacy",
            "legacy-fixture",
            &[],
            Some("legacy code"),
            &[("code.txt".into(), code, 0o644)],
            &mut RawTreeSession::default(),
        )
        .unwrap();
    repo.line_store()
        .unwrap()
        .compare_and_swap_line_head(
            "main",
            None,
            Some(&view.snapshot_id),
            "2020-01-01T00:00:00Z",
        )
        .unwrap();
    let source_info = inspect_source(source.to_str().unwrap()).unwrap();
    let store = InteropStore::new(&repo);
    let retained = store.retained_repository(&source_info.fingerprint);
    prepare_retained_repository(&retained, &source_info).unwrap();
    let mut reader = GitImportObjectReader::new(&retained).unwrap();
    for oid in [&first, &latest] {
        let commit = read_commit_data(&retained, oid, &mut reader).unwrap();
        store.write_mapping(json!({"kind":"commit","direction":"import","created_at":"2020-01-01T00:00:00Z","generation_id":source_info.generation_id,"source_repository_fingerprint":source_info.fingerprint,"git_object_format":"sha1","git_object_id":oid,"git_tree_object_id":commit.tree_object_id,"snapshot_id":view.snapshot_id,"ait_root_tree_id":view.root_tree_id,"parent_git_object_ids":commit.parent_object_ids,"parent_snapshot_ids":[],"author":identity_json(&commit.author),"committer":identity_json(&commit.committer),"message_base64":bytes_base64(&commit.message_bytes),"raw_commit_base64":bytes_base64(&commit.raw_bytes),"file_modes":tree_entries_json(&commit.files),"signed":false,"lfs_pointer_count":0,"imported_unchanged":true})).unwrap();
    }
    for row in [
        json!({"kind":"ref","git_ref_name":"refs/heads/main","git_object_id":latest,"snapshot_id":view.snapshot_id,"ait_line_name":"main"}),
        json!({"kind":"tag","git_ref_name":"refs/tags/v1","git_object_id":first,"snapshot_id":view.snapshot_id,"ait_tag_name":"v1"}),
        json!({"kind":"symbolic_ref","git_ref_name":"HEAD","git_symbolic_target":"refs/heads/main","git_object_id":latest}),
    ] {
        let mut row = row;
        row["direction"] = json!("import");
        row["created_at"] = json!("2020-01-01T00:00:00Z");
        row["generation_id"] = json!(source_info.generation_id);
        row["source_repository_fingerprint"] = json!(source_info.fingerprint);
        row["git_object_format"] = json!("sha1");
        store.write_mapping(row).unwrap();
    }
    FilesystemTagStore::new(ait.to_str().unwrap())
        .unwrap()
        .create_tag(
            &new_tag_record(
                "v1",
                &view.snapshot_id,
                "retained metadata",
                "2020-01-01T00:00:00Z",
            )
            .unwrap(),
        )
        .unwrap();
    store
        .write_operation(
            "legacy-operation",
            &operation_checkpoint(
                "import",
                "legacy-operation",
                &source_info.generation_id,
                "legacy-plan",
                "completed",
                2,
                2,
                Some(json!({"source":source_info.source_identity})),
            ),
        )
        .unwrap();
    assert!(!exists(&repo));
    let revisions = fs::read(ait.join(".ait/binary-db/plan_revision.bin")).unwrap();
    assert!(revisions[4..].chunks_exact(56).all(|row| row[0] & 32 == 0));
    repo
}

#[test]
fn legacy_migration_preserves_raw_history_native_heads_and_offline_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let repo = legacy_fixture(temp.path());
    let canonical = repo.authoritative_repo_root();
    let before = authority_bytes(&canonical);
    let old_input = inputs(&repo).unwrap();
    let source = temp.path().join("source");
    let oid = git(&source, &["rev-parse", "HEAD"]);
    let stage = temp.path().join("generation");
    assert_eq!(
        migrate(&repo, &stage, false).unwrap()["status"],
        json!("staged")
    );
    assert_eq!(authority_bytes(&canonical), before);
    assert!(!exists(&repo));
    let result = migrate(&repo, &stage, true).unwrap();
    assert_eq!(result["status"], json!("migrated"));
    assert_eq!(result["runtime_json_writes"], json!(0));
    assert_eq!(inputs(&repo).unwrap().digest, old_input.digest);
    let after = authority_bytes(&canonical);
    for name in ["snapshot.bin", "line.bin", "line_payload.bin"] {
        assert_eq!(after.get(name), before.get(name), "{name}");
    }
    assert!(canonical.join(ARCHIVED_TAGS).is_dir());
    assert!(!canonical.join(".ait/refs/tags").exists());
    let tag = crate::tag_surface::tag_store(&repo)
        .unwrap()
        .tag_by_name("v1")
        .unwrap()
        .unwrap();
    assert_eq!(tag.message, "retained metadata");
    assert_eq!(tag.created_at, "2020-01-01T00:00:00Z");
    assert_eq!(
        migrate(&repo, &stage, true).unwrap()["status"],
        json!("no_op")
    );
    assert_eq!(authority_bytes(&canonical), after);
    fs::rename(
        source.join(".git"),
        temp.path().join("original-odb.offline"),
    )
    .unwrap();
    let retained = InteropStore::new(&repo).root.join("repositories");
    fs::rename(&retained, temp.path().join("retained-odb.offline")).unwrap();
    let output = temp.path().join("roundtrip.git");
    exports::current(&repo, output.to_str().unwrap(), true, false).unwrap();
    assert_eq!(git(&output, &["rev-parse", "HEAD"]), oid);
    assert_eq!(
        git(&output, &["show", "HEAD:README.md"]),
        b"second\r\n\xfe\0"
    );
    assert_eq!(git(&output, &["show", "v1:README.md"]), b"first\r\n\xff\0");
    git(&output, &["fsck", "--strict", "--full"]);
    assert_eq!(inputs(&repo).unwrap().digest, old_input.digest);
    assert!(
        fs::read_dir(repo.authoritative_repo_root().join(".ait/git"))
            .unwrap()
            .all(|e| e.unwrap().path().extension().is_none_or(|e| e != "json"))
    );
}

#[test]
fn legacy_migration_source_export_preserves_frozen_refs_and_symbolic_head() {
    let temp = tempfile::tempdir().unwrap();
    let repo = legacy_fixture(temp.path());
    let source = temp.path().join("source");
    let before = authority_bytes(&repo.authoritative_repo_root());
    let expected_refs = git(&source, &["show-ref"]);
    let expected_head = git(&source, &["symbolic-ref", "HEAD"]);
    let generation = inputs(&repo).unwrap().mappings[0]["generation_id"]
        .as_str()
        .unwrap()
        .to_string();
    migrate(&repo, &temp.path().join("generation"), true).unwrap();
    fs::rename(source.join(".git"), temp.path().join("source.offline")).unwrap();
    let retained = InteropStore::new(&repo).root.join("repositories");
    fs::rename(retained, temp.path().join("retained.offline")).unwrap();

    let output = temp.path().join("source-roundtrip.git");
    exports::source(
        &repo,
        output.to_str().unwrap(),
        Some(&generation),
        None,
        false,
    )
    .unwrap();
    assert_eq!(git(&output, &["show-ref"]), expected_refs);
    assert_eq!(git(&output, &["symbolic-ref", "HEAD"]), expected_head);
    assert_eq!(
        git(&output, &["show", "HEAD:README.md"]),
        b"second\r\n\xfe\0"
    );
    assert_eq!(git(&output, &["show", "v1:README.md"]), b"first\r\n\xff\0");
    git(&output, &["fsck", "--strict", "--full"]);
    let after = authority_bytes(&repo.authoritative_repo_root());
    for name in ["snapshot.bin", "line.bin", "line_payload.bin"] {
        assert_eq!(after.get(name), before.get(name), "{name}");
    }

    let (_, history, _, _) = stores(&repo).unwrap();
    let refs = history.refs().unwrap();
    let selected = select(&history, Some(&generation)).unwrap().0;
    for name in ["refs/heads/main", "refs/tags/v1"] {
        assert!(refs
            .iter()
            .any(|(_, n, r)| { n == name && r.generation_index == selected && r.meta & 12 == 0 }));
        assert!(refs
            .iter()
            .any(|(_, n, r)| { n == name && r.generation_index == selected && r.meta & 12 == 12 }));
    }
    let head = refs
        .iter()
        .find(|(_, n, r)| n == "HEAD" && r.generation_index == selected)
        .unwrap();
    let target = refs
        .iter()
        .find(|(ordinal, _, _)| ordinal + 1 == head.2.symbolic_target_index_plus1)
        .unwrap();
    assert_eq!(target.1, "refs/heads/main");
    assert_eq!(target.2.meta & 12, 0);
}

#[test]
fn legacy_migration_rejects_changed_input_without_canonical_writes() {
    let temp = tempfile::tempdir().unwrap();
    let repo = legacy_fixture(temp.path());
    let stage = temp.path().join("generation");
    migrate(&repo, &stage, false).unwrap();
    let before = authority_bytes(&repo.authoritative_repo_root());
    let path = direct_files(&InteropStore::new(&repo).mappings_root())
        .unwrap()
        .remove(0);
    let old = fs::read(&path).unwrap();
    let mut row = parse_value(std::str::from_utf8(&old).unwrap(), "fixture").unwrap();
    row["created_at"] = json!("2020-01-02T00:00:00Z");
    fs::write(&path, encode_value(&row, "fixture").unwrap()).unwrap();
    assert!(migrate(&repo, &stage, true)
        .unwrap_err()
        .contains("input changed"));
    assert_eq!(authority_bytes(&repo.authoritative_repo_root()), before);
    assert!(!exists(&repo));
    fs::write(&path, old).unwrap();
    migrate(&repo, &stage, true).unwrap();
}

#[test]
fn legacy_migration_admits_export_ref_aliases_without_old_export_commit_rows() {
    let temp = tempfile::tempdir().unwrap();
    let repo = legacy_fixture(temp.path());
    let store = InteropStore::new(&repo);
    let mappings = store.load_mappings().unwrap();
    let branch = mappings
        .iter()
        .find(|r| json_text(r, "kind") == Some("ref"))
        .unwrap();
    let source_fp = sha256_prefixed(
        "ASR",
        format!(
            "ait-repository-fingerprint/v1\n{}\n{}\n",
            repo.repo_name(),
            repo.authoritative_repo_root().to_string_lossy()
        )
        .as_bytes(),
        24,
    );
    let generation = "GIT-EXP-0123456789ABCDEF";
    let source = temp.path().join("source");
    let generated = store.generated_repository();
    ensure_bare_repository(&generated, "sha1").unwrap();
    git_repo_bytes(
        &generated,
        ["fetch", source.to_str().unwrap(), "main:refs/heads/main"],
    )
    .unwrap();
    let target = temp.path().join("old-export.git");
    ensure_bare_repository(&target, "sha1").unwrap();
    let target = target.canonicalize().unwrap();
    let target_fp = sha256_prefixed(
        "GTR",
        format!(
            "git-target-fingerprint/v1\nsha1\n{}\n",
            target.to_string_lossy()
        )
        .as_bytes(),
        24,
    );
    store.write_mapping(json!({"kind":"ref","direction":"export","created_at":"2020-01-01T00:00:00Z","generation_id":generation,"source_repository_fingerprint":source_fp,"target_repository_fingerprint":target_fp,"git_object_format":"sha1","git_ref_name":"refs/heads/main","git_object_id":branch["git_object_id"],"snapshot_id":branch["snapshot_id"],"ait_kind":"line","ait_name":"main","expected_previous_git_object_id":null})).unwrap();
    store
        .write_operation(
            "legacy-export",
            &operation_checkpoint(
                "export",
                "legacy-export",
                generation,
                "legacy-export-plan",
                "completed",
                1,
                1,
                Some(json!({"target":target})),
            ),
        )
        .unwrap();
    let old = inputs(&repo).unwrap().digest;
    migrate(&repo, &temp.path().join("generation"), true).unwrap();
    let (_, history, _, _) = stores(&repo).unwrap();
    let generations = history.generations().unwrap();
    let exported = generations
        .iter()
        .position(|g| g.hash == bytes::<8>(generation).unwrap())
        .unwrap() as u32;
    assert!(history.generation_is_complete(exported).unwrap());
    assert_eq!(
        history
            .commit_mappings()
            .iter()
            .filter(|r| r.generation_index == exported)
            .count(),
        1
    );
    assert_eq!(inputs(&repo).unwrap().digest, old);
    drop(history);
    assert_eq!(
        migrate(&repo, &temp.path().join("generation"), true).unwrap()["status"],
        json!("no_op")
    );
}

#[test]
fn legacy_migration_converts_tag_only_with_exact_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let repo = legacy_fixture(temp.path());
    let store = InteropStore::new(&repo);
    fs::rename(
        store.mappings_root(),
        temp.path().join("unused-legacy-mappings"),
    )
    .unwrap();
    fs::rename(
        store.root.join("operations"),
        temp.path().join("unused-legacy-operations"),
    )
    .unwrap();
    let before = authority_bytes(&repo.authoritative_repo_root());
    let old = inputs(&repo).unwrap().digest;
    migrate(&repo, &temp.path().join("generation"), true).unwrap();
    let after = authority_bytes(&repo.authoritative_repo_root());
    for (name, bytes) in before {
        assert_eq!(after.get(&name), Some(&bytes), "{name}");
    }
    assert_eq!(inputs(&repo).unwrap().digest, old);
    assert_eq!(
        crate::tag_surface::tag_store(&repo)
            .unwrap()
            .tag_by_name("v1")
            .unwrap()
            .unwrap()
            .message,
        "retained metadata"
    );
    assert_eq!(
        migrate(&repo, &temp.path().join("generation"), true).unwrap()["status"],
        json!("no_op")
    );
}

#[test]
fn legacy_migration_converts_native_export_commit_rows_and_preserves_source_json() {
    let temp = tempfile::tempdir().unwrap();
    let repo = legacy_fixture(temp.path());
    let store = InteropStore::new(&repo);
    let mut commit = store
        .load_mappings()
        .unwrap()
        .into_iter()
        .find(|r| json_text(r, "kind") == Some("commit"))
        .unwrap();
    let source = temp.path().join("source");
    let generated = store.generated_repository();
    ensure_bare_repository(&generated, "sha1").unwrap();
    git_repo_bytes(
        &generated,
        ["fetch", source.to_str().unwrap(), "main:refs/heads/main"],
    )
    .unwrap();
    let target = temp.path().join("old-native-export.git");
    ensure_bare_repository(&target, "sha1").unwrap();
    let target = target.canonicalize().unwrap();
    let source_fp = sha256_prefixed(
        "ASR",
        format!(
            "ait-repository-fingerprint/v1\n{}\n{}\n",
            repo.repo_name(),
            repo.authoritative_repo_root().to_string_lossy()
        )
        .as_bytes(),
        24,
    );
    let target_fp = sha256_prefixed(
        "GTR",
        format!(
            "git-target-fingerprint/v1\nsha1\n{}\n",
            target.to_string_lossy()
        )
        .as_bytes(),
        24,
    );
    let generation = "GIT-EXP-1123456789ABCDEF";
    fs::rename(
        store.mappings_root(),
        temp.path().join("retained-unused-import-json"),
    )
    .unwrap();
    commit["direction"] = json!("export");
    commit["generation_id"] = json!(generation);
    commit["source_repository_fingerprint"] = json!(source_fp);
    store.write_mapping(commit.clone()).unwrap();
    store.write_mapping(json!({"kind":"ref","direction":"export","created_at":"2020-01-01T00:00:00Z","generation_id":generation,"source_repository_fingerprint":source_fp,"target_repository_fingerprint":target_fp,"git_object_format":"sha1","git_ref_name":"refs/heads/main","git_object_id":commit["git_object_id"],"snapshot_id":commit["snapshot_id"],"ait_kind":"line","ait_name":"main","expected_previous_git_object_id":null})).unwrap();
    store
        .write_operation(
            "legacy-native-export",
            &operation_checkpoint(
                "export",
                "legacy-native-export",
                generation,
                "old-native-plan",
                "completed",
                1,
                1,
                Some(json!({"target":target})),
            ),
        )
        .unwrap();
    let old = inputs(&repo).unwrap().digest;
    let before = authority_bytes(&repo.authoritative_repo_root());
    migrate(&repo, &temp.path().join("generation"), true).unwrap();
    let (objects, history, documents, _) = stores(&repo).unwrap();
    let row = history.commit_mappings().pop().unwrap();
    assert_eq!(
        documents
            .document_set_files(row.document_set_tree_index)
            .unwrap()
            .len(),
        1
    );
    let source_index =
        history.generations().unwrap()[row.generation_index as usize].source_repository_index;
    assert_eq!(
        objects.repository_identity(source_index).unwrap().0,
        GitRepositoryKind::AitSource
    );
    assert_eq!(inputs(&repo).unwrap().digest, old);
    let after = authority_bytes(&repo.authoritative_repo_root());
    for name in ["snapshot.bin", "line.bin", "line_payload.bin"] {
        assert_eq!(before.get(name), after.get(name));
    }
}

#[cfg(unix)]
#[test]
fn legacy_migration_rejects_staging_alias_before_external_writes() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let repo = legacy_fixture(temp.path());
    let stage = temp.path().join("generation");
    migrate(&repo, &stage, false).unwrap();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"unchanged").unwrap();
    let local = stage.join("local");
    fs::rename(&local, stage.join("retained-local")).unwrap();
    symlink(&outside, &local).unwrap();
    let before = authority_bytes(&repo.authoritative_repo_root());
    let error = migrate(&repo, &stage, true).unwrap_err();
    assert!(error.contains("alias or nonregular entry"), "{error}");
    assert_eq!(authority_bytes(&repo.authoritative_repo_root()), before);
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"unchanged");
    assert!(!outside.join(".locks").exists());
}

#[test]
fn legacy_migration_rejects_unrepresentable_input_and_keeps_authority() {
    for case in ["format", "identity", "raw", "parents", "tag_fraction"] {
        let temp = tempfile::tempdir().unwrap();
        let repo = legacy_fixture(temp.path());
        let before = authority_bytes(&repo.authoritative_repo_root());
        let store = InteropStore::new(&repo);
        let path = if case == "identity" {
            direct_files(&store.root.join("operations"))
                .unwrap()
                .remove(0)
        } else if case == "tag_fraction" {
            direct_files(&repo.authoritative_repo_root().join(".ait/refs/tags"))
                .unwrap()
                .remove(0)
        } else {
            direct_files(&store.mappings_root())
                .unwrap()
                .into_iter()
                .find(|p| {
                    json_text(
                        &parse_value(&fs::read_to_string(p).unwrap(), "fixture").unwrap(),
                        "kind",
                    ) == Some("commit")
                })
                .unwrap()
        };
        let mut row = parse_value(&fs::read_to_string(&path).unwrap(), "fixture").unwrap();
        match case {
            "format" => row["git_object_format"] = json!("sha256"),
            "identity" => row["result"]["source"] = json!("different-full-source"),
            "raw" => row["raw_commit_base64"] = json!(bytes_base64(b"wrong raw commit")),
            "parents" => {
                row["parent_git_object_ids"] = json!(["0000000000000000000000000000000000000000"])
            }
            _ => row["created_at"] = json!("2020-01-01T00:00:00.001Z"),
        }
        fs::write(&path, encode_value(&row, "fixture").unwrap()).unwrap();
        let error = migrate(&repo, &temp.path().join("generation"), true).unwrap_err();
        assert!(!error.is_empty(), "{case}");
        assert_eq!(
            authority_bytes(&repo.authoritative_repo_root()),
            before,
            "{case}"
        );
        assert!(!exists(&repo), "{case}");
        assert!(path.exists(), "{case}");
    }
}

#[test]
fn legacy_migration_process_child() {
    let Some(temp) = std::env::var_os("AIT_GIT_LEGACY_TEST_ROOT") else {
        return;
    };
    let temp = PathBuf::from(temp);
    let repo = RepoRuntime::discover_from_path(&temp.join("ait")).unwrap();
    migrate(&repo, &temp.join("generation"), true).unwrap();
    panic!("migration child must stop at its owned test boundary");
}

#[cfg(unix)]
#[test]
fn legacy_migration_sigkill_replays_mapping_and_tag_archive_publication() {
    use std::os::unix::process::ExitStatusExt;
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let repo = legacy_fixture(temp.path());
    let canonical = repo.authoritative_repo_root();
    let old = authority_bytes(&canonical);
    let input_digest = inputs(&repo).unwrap().digest;
    let mut stage_mapping = Vec::new();
    for (env, name) in [
        ("AIT_GIT_LEGACY_TEST_PAUSE", "mapping-paused"),
        ("AIT_GIT_LEGACY_ARCHIVE_TEST_PAUSE", "archive-paused"),
    ] {
        let marker = temp.path().join(name);
        let mut child = Child(Command::new(std::env::current_exe().unwrap()).args(["--exact","primitives::git_interop::native::migration::tests::legacy_migration_process_child","--nocapture"]).env("AIT_GIT_LEGACY_TEST_ROOT",temp.path()).env(env,&marker).stdin(Stdio::null()).spawn().unwrap());
        for _ in 0..400 {
            if marker.exists() || child.0.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        assert!(marker.exists(), "{env}: migration failed before boundary");
        child.0.kill().unwrap();
        assert_eq!(child.0.wait().unwrap().signal(), Some(9));
        let mapping_path = temp
            .path()
            .join("generation/.ait/git-interop/v1/binary-v0/git_commit_mapping.bin");
        if name == "mapping-paused" {
            stage_mapping = fs::read(&mapping_path).unwrap();
            assert_eq!(authority_bytes(&canonical), old);
            assert!(!exists(&repo));
        } else {
            assert!(exists(&repo));
            assert!(canonical.join(".ait/refs/tags").exists());
            assert!(crate::tag_surface::tag_store(&repo).is_err());
            let bundle = GitInteropBundle::open(&temp.path().join("generation"), false).unwrap();
            assert!(
                fs::read(bundle.decoded_root().join("git_commit_mapping.bin"))
                    .unwrap()
                    .starts_with(&stage_mapping)
            );
        }
        assert_eq!(inputs(&repo).unwrap().digest, input_digest);
    }
    let fresh = RepoRuntime::discover_from_path(&canonical).unwrap();
    migrate(&fresh, &temp.path().join("generation"), true).unwrap();
    assert_eq!(
        migrate(&fresh, &temp.path().join("generation"), true).unwrap()["status"],
        json!("no_op")
    );
    assert_eq!(inputs(&fresh).unwrap().digest, input_digest);
    assert!(crate::tag_surface::tag_store(&fresh)
        .unwrap()
        .tag_by_name("v1")
        .unwrap()
        .is_some());
    println!("two actual migration SIGKILL phases resumed; complete mapping prefix, original Snapshot/Line authority and offline JSON/ODB retained; no new runtime JSON");
}
