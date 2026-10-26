use super::*;

fn file<'a>(path: &'a str, mode: &'a str, content: &'a [u8]) -> SnapshotTreeFile<'a> {
    SnapshotTreeFile {
        path,
        mode,
        content,
    }
}

#[test]
fn exact_tree_removes_seed_leftovers_and_keeps_package_outside_sources() {
    let base = tempfile::tempdir().unwrap();
    let sources = base.path().join("sources");
    fs::create_dir_all(sources.join("obsolete/subdir")).unwrap();
    fs::write(sources.join("obsolete/subdir/old"), b"old").unwrap();
    fs::write(sources.join("removed"), b"removed").unwrap();
    fs::write(base.path().join("package.zst"), b"immutable package").unwrap();
    materialize_exact_snapshot_tree(
        &sources,
        [
            file("src/main.rs", "100644", b"final"),
            file("ci/run", "100755", b"run"),
        ],
    )
    .unwrap();
    assert!(!sources.join("obsolete").exists());
    assert!(!sources.join("removed").exists());
    assert_eq!(fs::read(sources.join("src/main.rs")).unwrap(), b"final");
    assert_eq!(
        fs::read(base.path().join("package.zst")).unwrap(),
        b"immutable package"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(sources.join("ci/run"))
                .unwrap()
                .permissions()
                .mode()
                & 0o7777,
            0o755
        );
    }
    materialize_exact_snapshot_tree(&sources, []).unwrap();
    assert_eq!(fs::read_dir(&sources).unwrap().count(), 0);
}

#[test]
fn conflicting_paths_and_invalid_entries_fail_before_pruning() {
    let base = tempfile::tempdir().unwrap();
    let sentinel = base.path().join("seed");
    fs::write(&sentinel, b"preserve until manifest is valid").unwrap();
    for entries in [
        vec![file("a", "100644", b"a"), file("a/b", "100644", b"b")],
        vec![file("a", "100644", b"a"), file("a", "100644", b"b")],
        vec![file("../escape", "100644", b"no")],
        vec![file("a", "040000", b"not a file")],
        vec![file("a", "120000", b"target\0suffix")],
    ] {
        assert!(materialize_exact_snapshot_tree(base.path(), entries).is_err());
        assert_eq!(
            fs::read(&sentinel).unwrap(),
            b"preserve until manifest is valid"
        );
    }
}

#[test]
fn exact_tree_handles_file_directory_type_changes() {
    let base = tempfile::tempdir().unwrap();
    fs::create_dir(base.path().join("now-file")).unwrap();
    fs::write(base.path().join("now-file/old"), b"old").unwrap();
    fs::write(base.path().join("now-directory"), b"old").unwrap();
    materialize_exact_snapshot_tree(
        base.path(),
        [
            file("now-file", "100644", b"file"),
            file("now-directory/new", "100644", b"nested"),
        ],
    )
    .unwrap();
    assert_eq!(fs::read(base.path().join("now-file")).unwrap(), b"file");
    assert_eq!(
        fs::read(base.path().join("now-directory/new")).unwrap(),
        b"nested"
    );
}

#[test]
fn selected_path_writes_preserve_unrelated_files() {
    let base = tempfile::tempdir().unwrap();
    fs::write(base.path().join("unrelated"), b"keep").unwrap();
    write_snapshot_tree_file(base.path(), "src/selected", "100644", b"selected").unwrap();
    assert_eq!(fs::read(base.path().join("unrelated")).unwrap(), b"keep");
    assert_eq!(
        fs::read(base.path().join("src/selected")).unwrap(),
        b"selected"
    );
}

#[cfg(unix)]
#[test]
fn exact_tree_replaces_seed_symlinks_without_following_them() {
    use std::os::unix::fs::symlink;
    let base = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("sentinel"), b"outside").unwrap();
    symlink(outside.path(), base.path().join("parent")).unwrap();
    symlink(
        outside.path().join("sentinel"),
        base.path().join("now-file"),
    )
    .unwrap();
    fs::write(base.path().join("now-link"), b"old regular file").unwrap();
    materialize_exact_snapshot_tree(
        base.path(),
        [
            file("parent/child", "100644", b"inside"),
            file("now-file", "100644", b"regular"),
            file("now-link", "120000", b"parent/child"),
            file("dangling", "120000", b"missing"),
        ],
    )
    .unwrap();
    assert_eq!(
        fs::read(outside.path().join("sentinel")).unwrap(),
        b"outside"
    );
    assert!(!outside.path().join("child").exists());
    assert!(!fs::symlink_metadata(base.path().join("parent"))
        .unwrap()
        .file_type()
        .is_symlink());
    assert_eq!(fs::read(base.path().join("now-file")).unwrap(), b"regular");
    assert_eq!(
        fs::read_link(base.path().join("now-link")).unwrap(),
        PathBuf::from("parent/child")
    );
    assert_eq!(
        fs::read_link(base.path().join("dangling")).unwrap(),
        PathBuf::from("missing")
    );
}

#[cfg(unix)]
#[test]
fn selected_path_write_rejects_symlink_parent_and_root() {
    use std::os::unix::fs::symlink;
    let base = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), base.path().join("parent")).unwrap();
    assert!(write_snapshot_tree_file(base.path(), "parent/escape", "100644", b"no").is_err());
    assert!(materialize_exact_snapshot_tree(
        &base.path().join("parent"),
        [file("escape", "100644", b"no")]
    )
    .is_err());
    assert!(!outside.path().join("escape").exists());
}

#[cfg(unix)]
#[test]
fn selected_path_replacement_does_not_mutate_old_hardlink_target() {
    let base = tempfile::tempdir().unwrap();
    fs::write(base.path().join("original"), b"original").unwrap();
    fs::hard_link(base.path().join("original"), base.path().join("replaced")).unwrap();
    write_snapshot_tree_file(base.path(), "replaced", "100755", b"new").unwrap();
    assert_eq!(fs::read(base.path().join("original")).unwrap(), b"original");
    assert_eq!(fs::read(base.path().join("replaced")).unwrap(), b"new");
}
