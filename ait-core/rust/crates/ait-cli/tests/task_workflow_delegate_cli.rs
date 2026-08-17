use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn make_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(&root.join(".ait/config.json"), r#"{"repo_name":"ait"}"#);
    temp
}

#[test]
fn task_namespace_unknown_subcommand_fails_closed() {
    let repo = make_repo();

    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .args(["task", "unknown-subcommand", "--title", "Demo", "--json"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "unrecognized subcommand 'unknown-subcommand'",
    ));
}

#[test]
fn workflow_ready_help_is_consumed_by_clap_without_delegate() {
    let repo = make_repo();

    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .args(["workflow", "ready", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "every preparation input requires --apply",
        ))
        .stdout(predicate::str::contains("--snapshot-message"))
        .stdout(predicate::str::contains("--author-mode"))
        .stdout(predicate::str::contains("--remote"))
        .stdout(predicate::str::contains("--json").not());
}

#[test]
fn text_only_workflow_commands_reject_json_without_parser_hints() {
    let repo = make_repo();

    for subcommand in ["ready", "land"] {
        for json_arg in ["--json", "--json=true"] {
            let mut cmd = Command::cargo_bin("ait-cli").unwrap();
            cmd.current_dir(repo.path())
                .args(["workflow", subcommand, "RCC-1", json_arg]);
            cmd.assert()
                .failure()
                .stderr(predicate::str::contains(
                    "unexpected argument '--json' found",
                ))
                .stderr(predicate::str::contains("tip:").not())
                .stderr(predicate::str::contains("-- --json").not());
        }
    }
}

#[test]
fn workflow_land_help_stays_native_without_top_level_land_or_task_complete() {
    let repo = make_repo();

    let mut workflow_help = Command::cargo_bin("ait-cli").unwrap();
    workflow_help
        .current_dir(repo.path())
        .args(["workflow", "--help"]);
    workflow_help
        .assert()
        .success()
        .stdout(predicate::str::contains("\n  land-local").not());

    let mut removed_land_local = Command::cargo_bin("ait-cli").unwrap();
    removed_land_local
        .current_dir(repo.path())
        .args(["workflow", "land-local", "--help"]);
    removed_land_local
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "unrecognized subcommand 'land-local'",
        ));

    let mut workflow_land_help = Command::cargo_bin("ait-cli").unwrap();
    workflow_land_help
        .current_dir(repo.path())
        .args(["workflow", "land", "--help"]);
    workflow_land_help
        .assert()
        .success()
        .stdout(predicate::str::contains("remote-only reviewer-owned"))
        .stdout(predicate::str::contains("--apply"))
        .stdout(predicate::str::contains("--review-message"))
        .stdout(predicate::str::contains("--remote"))
        .stdout(predicate::str::contains("--snapshot-message").not())
        .stdout(predicate::str::contains("--summary").not())
        .stdout(predicate::str::contains("--tests").not())
        .stdout(predicate::str::contains("--lint").not())
        .stdout(predicate::str::contains("--security").not())
        .stdout(predicate::str::contains("--license").not())
        .stdout(predicate::str::contains("--author-mode").not())
        .stdout(predicate::str::contains("--model").not())
        .stdout(predicate::str::contains("--reviewer").not())
        .stdout(predicate::str::contains("--target").not())
        .stdout(predicate::str::contains("--mode").not())
        .stdout(predicate::str::contains("--local").not())
        .stdout(predicate::str::contains("--all-completed-local").not())
        .stdout(predicate::str::contains("--json").not());

    let mut top_level_land_help = Command::cargo_bin("ait-cli").unwrap();
    top_level_land_help
        .current_dir(repo.path())
        .args(["land", "--help"]);
    top_level_land_help
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand 'land'"));

    let mut task_help = Command::cargo_bin("ait-cli").unwrap();
    task_help.current_dir(repo.path()).args(["task", "--help"]);
    task_help
        .assert()
        .success()
        .stdout(predicate::str::contains("\n  complete ").not())
        .stdout(predicate::str::contains("land"));
}

#[test]
fn task_land_and_plan_sync_help_publish_the_same_scope_contract() {
    let repo = make_repo();

    let mut task_land = Command::cargo_bin("ait-cli").unwrap();
    task_land
        .current_dir(repo.path())
        .args(["task", "land", "--help"]);
    task_land
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "configured workflow mode selects local or remote authority",
        ))
        .stdout(predicate::str::contains(
            "Local land consumes an existing Snapshot",
        ))
        .stdout(predicate::str::contains(
            "remote land consumes an already-ready selected Patchset",
        ))
        .stdout(predicate::str::contains(
            "Final Task closeout removes the bound worktree",
        ));

    let mut plan_sync = Command::cargo_bin("ait-cli").unwrap();
    plan_sync
        .current_dir(repo.path())
        .args(["plan", "sync", "--help"]);
    plan_sync
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Plan sync never creates a Snapshot or advances a Line",
        ))
        .stdout(predicate::str::contains(
            "solo_local writes local Plan state",
        ))
        .stdout(predicate::str::contains(
            "solo_remote reconciles local Plan lineage and publishes the touched heads",
        ))
        .stdout(predicate::str::contains(
            "--local or --remote overrides the configured scope",
        ));
}

#[test]
fn workflow_guide_is_native_and_not_delegated() {
    let repo = make_repo();

    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .args(["workflow", "guide", "land"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ait workflow guide · land"))
        .stdout(predicate::str::contains("task-land-plan-closeout/v1"))
        .stdout(predicate::str::contains(
            "reviewer-owned exact-Patchset code-review",
        ))
        .stdout(predicate::str::contains(
            "delegate the already-ready final mutation",
        ))
        .stdout(predicate::str::contains("It creates no Review evidence"))
        .stdout(predicate::str::contains(
            "automatic_exact_local_when_final_task_completed",
        ))
        .stdout(predicate::str::contains("separate_after_land"));
}

fn assert_unknown_subcommand_fails_closed(repo: &TempDir, args: &[&str], expected_error: &str) {
    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path()).args(args);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains(expected_error));
}

#[test]
fn unsupported_snapshot_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        &["snapshot", "legacy-export", "SNP-1"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn unsupported_change_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        &["change", "legacy-export", "RC-1"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn standalone_change_create_help_is_native_and_not_delegated() {
    let repo = make_repo();
    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .args(["change", "create", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("--title <TITLE>"))
        .stdout(predicate::str::contains("--base-line <LINE>"))
        .stdout(predicate::str::contains(
            "defaults to the bound worktree target Line",
        ));
}

#[test]
fn unsupported_patchset_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        &["patchset", "legacy-export", "--change", "RC-1"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn unsupported_review_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        &["review", "legacy-export", "RC-1711"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn unsupported_policy_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        &["policy", "legacy-export", "RP-1"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn removed_top_level_land_namespace_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        &["land", "legacy-export", "LAND-1"],
        "unrecognized subcommand 'land'",
    );
}

#[test]
fn unsupported_queue_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        &["queue", "legacy-export"],
        "unrecognized subcommand 'legacy-export'",
    );
}
