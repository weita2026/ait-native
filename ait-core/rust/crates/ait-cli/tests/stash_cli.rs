use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn ait_cli() -> Command {
    Command::cargo_bin("ait-cli").unwrap()
}

fn run_json(root: &Path, args: &[&str]) -> JsonValue {
    let output = ait_cli()
        .current_dir(root)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    JsonCodec::parse_slice_with_error_prefix(&output, "Invalid stash CLI JSON").unwrap()
}

#[test]
fn native_stash_cli_preserves_the_complete_binary_db_lifecycle() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let work = root.join("work.txt");
    fs::write(&work, "base\n").unwrap();

    run_json(root, &["init", "--name", "stash-cli", "--json"]);
    let base = run_json(root, &["snapshot", "create", "--message", "base", "--json"]);
    let base_snapshot_id = base["snapshot_id"].as_str().unwrap().to_string();

    ait_cli()
        .current_dir(root)
        .args(["stash", "save", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Workspace is already clean; stash save requires local changes to park.",
        ));

    fs::write(&work, "parked\n").unwrap();
    let saved = run_json(
        root,
        &["stash", "save", "--message", "parked work", "--json"],
    );
    let stash_id = saved["stash_id"].as_str().unwrap().to_string();
    assert_eq!(saved["snapshot_kind"], "stash");
    assert_eq!(saved["workspace_cleared"], true);
    assert_eq!(saved["line_head_snapshot_id_before"], base_snapshot_id);
    assert_eq!(saved["line_head_snapshot_id_after"], base_snapshot_id);
    assert_eq!(fs::read_to_string(&work).unwrap(), "base\n");

    let listed = run_json(root, &["stash", "list", "--json"]);
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["stash_id"], stash_id);
    let shown = run_json(root, &["stash", "show", &stash_id, "--json"]);
    assert_eq!(shown["message"], "parked work");

    let applied = run_json(root, &["stash", "apply", &stash_id, "--json"]);
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["dropped"], false);
    assert_eq!(fs::read_to_string(&work).unwrap(), "parked\n");
    assert_eq!(
        run_json(root, &["stash", "list", "--json"])
            .as_array()
            .unwrap()
            .len(),
        1
    );

    ait_cli()
        .current_dir(root)
        .args(["stash", "pop", &stash_id, "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    let popped = run_json(root, &["stash", "pop", &stash_id, "--force", "--json"]);
    assert_eq!(popped["applied"], true);
    assert_eq!(popped["dropped"], true);
    assert!(run_json(root, &["stash", "list", "--json"])
        .as_array()
        .unwrap()
        .is_empty());

    let second = run_json(
        root,
        &[
            "stash",
            "save",
            "--message",
            "drop only",
            "--keep-workspace",
            "--json",
        ],
    );
    let second_id = second["stash_id"].as_str().unwrap();
    assert_eq!(second["workspace_cleared"], false);
    assert_eq!(fs::read_to_string(&work).unwrap(), "parked\n");
    let dropped = run_json(root, &["stash", "drop", second_id, "--json"]);
    assert_eq!(dropped["dropped"], true);
    assert!(run_json(root, &["stash", "list", "--json"])
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(fs::read_to_string(&work).unwrap(), "parked\n");

    assert!(root.join(".ait/binary-db/stash.bin").exists());
}

#[test]
fn native_stash_cli_help_and_missing_errors_are_self_owned() {
    ait_cli()
        .args(["stash", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("save"))
        .stdout(predicate::str::contains("list"))
        .stdout(predicate::str::contains("show"))
        .stdout(predicate::str::contains("apply"))
        .stdout(predicate::str::contains("pop"))
        .stdout(predicate::str::contains("drop"));

    let temp = TempDir::new().unwrap();
    run_json(temp.path(), &["init", "--name", "stash-errors", "--json"]);
    ait_cli()
        .current_dir(temp.path())
        .args(["stash", "show", "STH-MISSING", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown stash: STH-MISSING"));
}
