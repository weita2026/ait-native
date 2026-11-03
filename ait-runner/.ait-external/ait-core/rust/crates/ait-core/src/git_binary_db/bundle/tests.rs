use super::*;
use crate::binary_db::{BinaryDbReadTxn, BinaryFileId};
use crate::git_binary_db::GitHistoryStore;

#[test]
#[ignore = "explicit bounded publication profile on an isolated archive copy"]
fn publication_cost_profile() {
    let root = PathBuf::from(
        std::env::var_os("AIT_GIT_BUNDLE_PROFILE_ROOT").expect("isolated profile root"),
    );
    let bundle = GitInteropBundle::open(&root, true).unwrap();
    let input_bytes = members()
        .map(|name| {
            fs::metadata(bundle.decoded_root().join(name))
                .unwrap()
                .len()
        })
        .sum::<u64>();
    let started = std::time::Instant::now();
    crate::binary_db::read_profile::reset();
    for _ in 0..20 {
        bundle.publish_admitted().unwrap();
    }
    println!(
        "BUNDLE_PROFILE={}",
        serde_json::json!({"publications":20,"inner_bytes":input_bytes,"wall_seconds":started.elapsed().as_secs_f64(),"archive_bytes":fs::metadata(root.join(GIT_INTEROP_BUNDLE_PATH)).unwrap().len(),"phase_counters":crate::binary_db::read_profile::snapshot()})
    );
}
pub(super) fn publication_pause(at: &str) {
    if std::env::var("AIT_GIT_BUNDLE_TEST_PAUSE").ok().as_deref() == Some(at) {
        if let Some(marker) = std::env::var_os("AIT_GIT_BUNDLE_TEST_MARKER") {
            fs::write(marker, at).expect("test marker");
            loop {
                std::thread::park();
            }
        }
    }
}

#[test]
fn roundtrip_preserves_all_inner_bytes_and_reader_writer_exclusion() {
    let repo = tempfile::tempdir().unwrap();
    let bundle = GitInteropBundle::open(repo.path(), true).unwrap();
    let legacy = repo.path().join(GIT_INTEROP_LEGACY_DIRECTORY);
    fs::create_dir_all(&legacy).unwrap();
    for name in members() {
        fs::copy(bundle.decoded_root().join(name), legacy.join(name)).unwrap();
    }
    bundle.publish_admitted().unwrap();
    let expected = members()
        .map(|n| (n, fs::read(bundle.decoded_root().join(n)).unwrap()))
        .collect::<Vec<_>>();
    assert!(GitInteropBundle::open(repo.path(), false)
        .unwrap_err()
        .is_retryable_busy());
    let decoded = bundle.decoded_root().to_path_buf();
    drop(bundle);
    assert!(!decoded.exists());
    assert!(legacy_inputs(&legacy).unwrap().is_empty());
    let a = GitInteropBundle::open(repo.path(), false).unwrap();
    let b = GitInteropBundle::open(repo.path(), false).unwrap();
    assert!(GitInteropBundle::open(repo.path(), true)
        .unwrap_err()
        .is_retryable_busy());
    for (name, bytes) in expected {
        assert_eq!(fs::read(a.decoded_root().join(name)).unwrap(), bytes);
    }
    assert!(a.publish_admitted().is_err());
    drop((a, b));
    assert_eq!(
        fs::read_dir(repo.path().join(".ait/git")).unwrap().count(),
        2
    );
}

#[test]
#[cfg(unix)]
fn last_database_clone_releases_lock_while_an_owned_child_is_before_exec() {
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let repo = tempfile::tempdir().unwrap();
    let bundle = GitInteropBundle::open(repo.path(), true).unwrap();
    let db = bundle.database(repo.path(), AuthorityId::new("fork-lock"));
    drop(bundle);
    let (mut ready, child_ready) = UnixStream::pair().unwrap();
    let (mut release, child_release) = UnixStream::pair().unwrap();
    ready
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    child_release
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let launcher = std::thread::spawn(move || -> std::io::Result<_> {
        let ready_fd = child_ready.as_raw_fd();
        let release_fd = child_release.as_raw_fd();
        let mut command = Command::new(std::env::current_exe()?);
        command
            .args([
                "--exact",
                "git_binary_db::bundle::tests::checkpoint_process_child",
            ])
            .env_remove("AIT_GIT_BUNDLE_CHILD_ROOT")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // SAFETY: only async-signal-safe syscalls access the socket descriptors,
        // whose owners stay alive until spawn returns. The socket deadline
        // releases the child if the parent test fails.
        unsafe {
            command.pre_exec(move || {
                let mut byte = 1u8;
                if libc::write(ready_fd, (&byte as *const u8).cast(), 1) != 1
                    || libc::read(release_fd, (&mut byte as *mut u8).cast(), 1) != 1
                {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let spawned = command.spawn();
        drop((child_ready, child_release));
        spawned
    });
    let reached = ready.read_exact(&mut [0u8; 1]);
    let still_exclusive = reached.is_ok()
        && GitInteropBundle::open(repo.path(), false).is_err_and(|error| error.is_retryable_busy());
    drop(db);
    let reopened = reached
        .is_ok()
        .then(|| GitInteropBundle::open(repo.path(), true));
    let released = release.write_all(&[1]);
    let status = launcher.join().unwrap().and_then(|mut child| child.wait());
    reached.unwrap();
    released.unwrap();
    assert!(
        status.unwrap().success(),
        "owned child must exit and be reaped"
    );
    assert!(
        still_exclusive,
        "a live DB clone must retain exclusive ownership"
    );
    reopened
        .unwrap()
        .expect("the last DB clone must release its process lock before child exec");
}

#[test]
fn damaged_or_ambiguous_bundle_never_falls_back() {
    let repo = tempfile::tempdir().unwrap();
    let bundle = GitInteropBundle::open(repo.path(), true).unwrap();
    bundle.publish_admitted().unwrap();
    drop(bundle);
    let archive = repo.path().join(GIT_INTEROP_BUNDLE_PATH);
    let good = fs::read(&archive).unwrap();
    for bad in [
        good[..good.len() - 1].to_vec(),
        {
            let mut b = good.clone();
            b.push(0);
            b
        },
        {
            let mut b = good.clone();
            let n = b.len();
            b[n - 1] ^= 1;
            b
        },
    ] {
        fs::write(&archive, bad).unwrap();
        assert!(GitInteropBundle::open(repo.path(), false).is_err());
    }
    fs::write(&archive, good).unwrap();
    let legacy = repo.path().join(GIT_INTEROP_LEGACY_DIRECTORY);
    fs::create_dir_all(&legacy).unwrap();
    fs::write(legacy.join("git_repository_payload.bin"), [1, 0, 0, 0, 9]).unwrap();
    assert!(GitInteropBundle::open(repo.path(), true)
        .unwrap_err()
        .contains("ambiguous"));
}

#[test]
fn streamed_candidate_comparison_checks_full_bytes_and_external_dependencies_still_sync() {
    use crate::binary_db::BinaryDb;
    let repo = tempfile::tempdir().unwrap();
    let bundle = GitInteropBundle::open(repo.path(), true).unwrap();
    let db = bundle.database(repo.path(), AuthorityId::new("streamed-candidate"));
    GitHistoryStore::open(db.clone())
        .unwrap()
        .append_identity(b"Fixture <fixture@local> 1 +0000")
        .unwrap();
    assert!(db
        .sync_file(&repo.path().join("missing-external-dependency"))
        .is_err());
    let file = bundle.decoded_root().join("git_identity_payload.bin");
    let original = fs::read(&file).unwrap();
    let mut changed = original.clone();
    let at = changed.len() - 1;
    changed[at] ^= 1;
    fs::write(&file, changed).unwrap();
    assert!(decode_to(
        &repo.path().join(GIT_INTEROP_BUNDLE_PATH),
        bundle.decoded_root(),
        true,
        true
    )
    .unwrap_err()
    .contains("inner bytes"));
    fs::write(file, original).unwrap();
}

#[test]
fn native_migration_preserves_populated_records_payloads_indexes_and_content_fks() {
    use crate::content_binary_db::LocalContentBinaryDb;
    use crate::git_binary_db::*;
    let repo = tempfile::tempdir().unwrap();
    let seed = GitInteropBundle::open(repo.path(), true).unwrap();
    let legacy = repo.path().join(GIT_INTEROP_LEGACY_DIRECTORY);
    fs::create_dir_all(&legacy).unwrap();
    for name in members() {
        fs::copy(seed.decoded_root().join(name), legacy.join(name)).unwrap();
    }
    drop(seed);
    let content = LocalContentBinaryDb::<1>::new(
        repo.path().join(".ait/binary-db"),
        repo.path(),
        AuthorityId::new("bundle-content"),
        LocalStateScope::Repository,
    );
    let db = LocalBinaryDbFs::new(
        legacy.as_path(),
        repo.path(),
        AuthorityId::new("legacy"),
        LocalStateScope::Repository,
    )
    .with_declared_bin_paths(GIT_OBJECT_STORE_BIN_PATHS)
    .with_declared_index_paths(GIT_OBJECT_STORE_INDEX_PATHS);
    let objects = LocalGitObjectStore::open(db.clone(), content.clone()).unwrap();
    let repository = objects
        .ensure_repository(
            GitRepositoryKind::ImportSource,
            1,
            [7; 12],
            "preserved original source",
        )
        .unwrap();
    let body = b"exact binary body\r\n\0\xff";
    let oid = git_object_id(GitObjectKind::Blob, body);
    objects
        .put(repository, GitObjectKind::Blob, 1, &oid, body)
        .unwrap();
    let history = GitHistoryStore::open(db.clone()).unwrap();
    let generation = history
        .ensure_generation(GitGenerationRecord {
            export: false,
            source_repository_index: repository,
            target_repository_index_plus1: 0,
            hash: [8; 8],
        })
        .unwrap();
    history
        .append_identity(b"Fixture <fixture@local> 1 +0000")
        .unwrap();
    history
        .append_identity(b"Other <other@local> 2 -0800")
        .unwrap();
    history
        .append_ref(
            "refs/tags/raw",
            GitRefMappingRecord {
                meta: 1,
                object_kind: GitObjectKind::Blob,
                generation_index: generation,
                snapshot_index_plus1: 0,
                target_index_plus1: 0,
                symbolic_target_index_plus1: 0,
                object_id: oid,
                expected_previous: [0; 20],
                created_at_s: 1,
            },
        )
        .unwrap();
    history
        .checkpoint(GitOperationCheckpointRecord {
            status: 1,
            generation_index: generation,
            operation_hash: [9; 8],
            plan_hash: [10; 12],
            next_commit: 0,
            next_ref: 1,
            updated_at_s: 1,
        })
        .unwrap();
    let expected = members()
        .map(|name| (name, fs::read(legacy.join(name)).unwrap()))
        .collect::<Vec<_>>();
    drop((history, objects, db));
    let bundle = GitInteropBundle::open(repo.path(), true).unwrap();
    let db = bundle.database(repo.path(), AuthorityId::new("migrated"));
    let objects = LocalGitObjectStore::open(db.clone(), content.clone()).unwrap();
    let history = GitHistoryStore::open(db.clone()).unwrap();
    let documents = RawGitDocumentStore::open(db.clone(), content.clone()).unwrap();
    validate_git_content_closure(&objects, &history, &documents, &content).unwrap();
    bundle.publish_admitted().unwrap();
    assert_eq!(objects.get(repository, 1, &oid).unwrap().unwrap().1, body);
    for (name, bytes) in &expected {
        assert_eq!(&fs::read(bundle.decoded_root().join(name)).unwrap(), bytes);
    }
    assert!(legacy_inputs(&legacy).unwrap().is_empty());
    drop((documents, objects, history, db, bundle));
    // A crash during retirement may leave any byte-identical subset.
    for (name, bytes) in expected.iter().take(2) {
        fs::write(legacy.join(name), bytes).unwrap();
    }
    let replay = GitInteropBundle::open(repo.path(), true).unwrap();
    replay.publish_admitted().unwrap();
    assert!(legacy_inputs(&legacy).unwrap().is_empty());
}

fn compressed(raw: &[u8]) -> Vec<u8> {
    let mut zstd = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    zstd.include_checksum(true).unwrap();
    zstd.write_all(raw).unwrap();
    zstd.finish().unwrap()
}
fn checksum(header: &mut [u8]) {
    header[148..156].fill(b' ');
    let sum = header.iter().map(|b| u64::from(*b)).sum();
    put_octal(&mut header[148..155], sum).unwrap();
    header[155] = b' ';
}
#[test]
fn archive_inventory_bounds_and_inner_widths_are_enforced() {
    let repo = tempfile::tempdir().unwrap();
    let bundle = GitInteropBundle::open(repo.path(), true).unwrap();
    bundle.publish_admitted().unwrap();
    drop(bundle);
    let path = repo.path().join(GIT_INTEROP_BUNDLE_PATH);
    let good = fs::read(&path).unwrap();
    let tar = zstd::stream::decode_all(good.as_slice()).unwrap();
    let mut invalid = Vec::new();
    for name in [
        "../git_repository.bin",
        "/git_repository.bin",
        "unexpected.bin",
    ] {
        let mut raw = tar.clone();
        raw[..100].fill(0);
        raw[..name.len()].copy_from_slice(name.as_bytes());
        checksum(&mut raw[..512]);
        invalid.push(raw);
    }
    for kind in [b'1', b'2', b'5', b'x', b'L'] {
        let mut raw = tar.clone();
        raw[156] = kind;
        checksum(&mut raw[..512]);
        invalid.push(raw);
    }
    let mut duplicate = tar.clone();
    duplicate[1024..1536].copy_from_slice(&tar[..512]);
    invalid.push(duplicate);
    let mut oversize = tar.clone();
    put_octal(&mut oversize[124..136], GIT_INTEROP_MEMBER_LIMIT + 1).unwrap();
    checksum(&mut oversize[..512]);
    invalid.push(oversize);
    let mut bad_layout = tar.clone();
    bad_layout[512] = 2;
    invalid.push(bad_layout);
    invalid.push(tar[1024..].to_vec());
    invalid.push(tar[..tar.len() - 512].to_vec());
    let mut trailing = tar.clone();
    trailing.extend_from_slice(&[0; 512]);
    invalid.push(trailing);
    for raw in invalid {
        fs::write(&path, compressed(&raw)).unwrap();
        assert!(GitInteropBundle::open(repo.path(), false).is_err());
    }
    fs::write(path, good).unwrap();
}

#[test]
fn checkpoint_process_child() {
    let Some(repo) = std::env::var_os("AIT_GIT_BUNDLE_CHILD_ROOT") else {
        return;
    };
    let repo = PathBuf::from(repo);
    let bundle = GitInteropBundle::open(&repo, true).unwrap();
    let db = bundle.database(&repo, AuthorityId::new("bundle-crash"));
    GitHistoryStore::open(db)
        .unwrap()
        .append_identity(b"Fixture <fixture@local> 1 +0000")
        .unwrap();
}
#[test]
#[cfg(unix)]
fn interrupted_inner_commit_selects_one_complete_archive_in_a_fresh_process() {
    use std::os::unix::process::ExitStatusExt;
    use std::process::{Command, Stdio};
    for (at, count) in [("before-rename", 0), ("after-rename", 1)] {
        let repo = tempfile::tempdir().unwrap();
        let bundle = GitInteropBundle::open(repo.path(), true).unwrap();
        bundle.publish_admitted().unwrap();
        drop(bundle);
        let marker = repo.path().join("paused");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "git_binary_db::bundle::tests::checkpoint_process_child",
                "--nocapture",
            ])
            .env("AIT_GIT_BUNDLE_CHILD_ROOT", repo.path())
            .env("AIT_GIT_BUNDLE_TEST_PAUSE", at)
            .env("AIT_GIT_BUNDLE_TEST_MARKER", &marker)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..400 {
            if marker.exists() || child.try_wait().unwrap().is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        if !marker.exists() {
            let _ = child.kill();
            let _ = child.wait();
            panic!("child missed {at}");
        }
        assert!(GitInteropBundle::open(repo.path(), false)
            .unwrap_err()
            .is_retryable_busy());
        child.kill().unwrap();
        assert_eq!(child.wait().unwrap().signal(), Some(9));
        let bundle = GitInteropBundle::open(repo.path(), false).unwrap();
        let db = bundle.database(repo.path(), AuthorityId::new("fresh"));
        assert_eq!(
            BinaryDbReadTxn::new(&db)
                .record_count(BinaryFileId::new("git_identity.bin", 1, 24))
                .unwrap(),
            count
        );
        drop((db, bundle));
        let cleanup = GitInteropBundle::open(repo.path(), true).unwrap();
        drop(cleanup);
        assert_eq!(
            fs::read_dir(repo.path().join(".ait/git")).unwrap().count(),
            2
        );
    }
}
