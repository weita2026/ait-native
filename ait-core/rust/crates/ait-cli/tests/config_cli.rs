use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn cargo_bin() -> Command {
    Command::cargo_bin("ait-cli").unwrap()
}

#[test]
fn compact_config_show_exposes_automatic_task_review_policy_and_reviewer() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .args(["init", "--json"])
        .assert()
        .success();
    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "set", "--user-name", "Alice Example"])
        .assert()
        .success();

    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("task_review: automatic"))
        .stdout(predicate::str::contains("task_review_source: built_in"))
        .stdout(predicate::str::contains(
            "automatic_task_reviewer: Alice Example",
        ));
}

#[test]
fn compact_config_show_exposes_missing_automatic_task_reviewer() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .args(["init", "--json"])
        .assert()
        .success();

    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("task_review: automatic"))
        .stdout(predicate::str::contains("automatic_task_reviewer: <unset>"));
}

#[test]
fn config_show_json_keeps_required_mode_explicit_without_automatic_reviewer() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .args(["init", "--json"])
        .assert()
        .success();
    cargo_bin()
        .current_dir(temp.path())
        .args([
            "config",
            "set",
            "--user-name",
            "Alice Example",
            "--task-review",
            "required",
        ])
        .assert()
        .success();

    cargo_bin()
        .current_dir(temp.path())
        .args(["config", "show", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"value\": \"required\""))
        .stdout(predicate::str::contains("\"automatic_reviewer\": null"));
}
