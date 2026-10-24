use ait_cli::runtime::{RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use ait_core::local_snapshot::LocalSnapshotWriteStore;
use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn ait_command() -> Command {
    Command::cargo_bin("ait-cli").expect("ait-cli binary")
}

fn aitk_command() -> Command {
    Command::cargo_bin("aitk").expect("aitk binary")
}

fn file_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn visit(root: &Path, current: &Path, rows: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            let metadata = fs::symlink_metadata(&path).unwrap();
            if metadata.is_dir() {
                visit(root, &path, rows);
            } else if metadata.is_file() {
                rows.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut rows = BTreeMap::new();
    visit(root, root, &mut rows);
    rows
}

#[test]
fn native_aitk_help_and_version_expose_the_gitk_style_repository_model() {
    aitk_command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Run aitk inside the target AIT repository",
        ))
        .stdout(predicate::str::contains("-C <PATH>"))
        .stdout(predicate::str::contains("--json-only"))
        .stdout(predicate::str::contains("Python").not());
    aitk_command()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("aitk {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn json_only_discovers_a_parent_repository_and_changes_no_repository_bytes() {
    let temp = TempDir::new().unwrap();
    ait_command()
        .current_dir(temp.path())
        .args(["init", "--json"])
        .assert()
        .success();
    let nested = temp.path().join("src/nested");
    fs::create_dir_all(&nested).unwrap();
    let before = file_bytes(temp.path());

    let output = aitk_command()
        .current_dir(&nested)
        .args(["--json-only", "--limit", "7"])
        .output()
        .expect("aitk JSON output");
    assert!(
        output.status.success(),
        "aitk failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(payload["schema"], "aitk-history/v1");
    assert_eq!(payload["read_only"], true);
    assert_eq!(
        payload["repository"]["root"],
        temp.path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .as_ref()
    );
    assert_eq!(payload["history"]["limit"], 7);
    assert_eq!(file_bytes(temp.path()), before);
}

#[test]
fn uninitialized_or_invalid_targets_fail_with_actionable_diagnostics() {
    let temp = TempDir::new().unwrap();
    aitk_command()
        .args(["-C", temp.path().to_str().unwrap(), "--json-only"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("no initialized AIT repository"))
        .stderr(predicate::str::contains("ait init"));

    let file = temp.path().join("not-a-directory");
    fs::write(&file, "fixture").unwrap();
    aitk_command()
        .args(["-C", file.to_str().unwrap(), "--json-only"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("-C target is not a directory"));
}

#[test]
fn embedded_ui_diff_transport_loads_one_snapshot_without_mutation() {
    let temp = TempDir::new().unwrap();
    ait_command()
        .current_dir(temp.path())
        .args(["init", "--json"])
        .assert()
        .success();
    fs::write(temp.path().join("hello.txt"), "hello aitk\n").unwrap();
    let repo = RepoRuntime::discover_from_path(temp.path()).unwrap();
    let store = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&repo.workspace_root())
        .unwrap();
    let snapshot = store
        .create_snapshot(&repo.repo_name(), "main", Some("aitk fixture"), false)
        .unwrap();
    let snapshot_id = snapshot["snapshot_id"].as_str().unwrap();
    let before = file_bytes(temp.path());

    aitk_command()
        .args([
            "-C",
            temp.path().to_str().unwrap(),
            "--ui-diff-tsv",
            snapshot_id,
        ])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("aitk-diff-tsv-v1\n"))
        .stdout(predicate::str::contains("path\t"));
    assert_eq!(file_bytes(temp.path()), before);
}
