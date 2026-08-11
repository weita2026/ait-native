use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
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

fn make_delegate_script(root: &Path) -> PathBuf {
    let script_path = root.join("fake_delegate.sh");
    let script = r#"#!/bin/sh
if [ -n "${AIT_CLI_DELEGATE_LOG:-}" ]; then
  : > "$AIT_CLI_DELEGATE_LOG"
  for arg in "$@"; do
    printf '%s\n' "$arg" >> "$AIT_CLI_DELEGATE_LOG"
  done
fi
if [ -n "${AIT_CLI_DELEGATE_ENV_LOG:-}" ]; then
  {
    printf 'AIT_REPO_ROOT=%s\n' "${AIT_REPO_ROOT:-}"
    printf 'PYTHONPATH=%s\n' "${PYTHONPATH:-}"
  } > "$AIT_CLI_DELEGATE_ENV_LOG"
fi
if [ -n "${AIT_CLI_DELEGATE_STDOUT:-}" ]; then
  printf '%s' "${AIT_CLI_DELEGATE_STDOUT}"
fi
if [ -n "${AIT_CLI_DELEGATE_STDERR:-}" ]; then
  printf '%s' "${AIT_CLI_DELEGATE_STDERR}" >&2
fi
exit "${AIT_CLI_DELEGATE_EXIT_CODE:-0}"
"#;
    write_file(&script_path, script);
    let mut perms = fs::metadata(&script_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&script_path, perms).unwrap();
    script_path
}

#[test]
fn task_namespace_unknown_subcommand_fails_closed() {
    let repo = make_repo();
    let log_path = repo.path().join("delegate.log");
    let script_path = make_delegate_script(repo.path());

    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .args(["task", "unknown-subcommand", "--title", "Demo", "--json"]);
    cmd.assert().failure().stderr(predicate::str::contains(
        "unrecognized subcommand 'unknown-subcommand'",
    ));
    assert!(!log_path.exists());
}

#[test]
fn workflow_ready_help_is_consumed_by_clap_without_delegate() {
    let repo = make_repo();
    let log_path = repo.path().join("workflow.log");
    let script_path = make_delegate_script(repo.path());

    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .env("AIT_CLI_DELEGATE_LOG", &log_path)
        .args(["workflow", "ready", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains(
            "Show or apply the text-only ready-phase helper for one change before review and remote land.",
        ))
        .stdout(predicate::str::contains("--json").not());
    assert!(!log_path.exists());
}

#[test]
fn text_only_workflow_commands_reject_json_without_parser_hints() {
    let repo = make_repo();
    let script_path = make_delegate_script(repo.path());

    for subcommand in ["ready", "land"] {
        for json_arg in ["--json", "--json=true"] {
            let log_path = repo.path().join(format!("workflow-{subcommand}.log"));
            let mut cmd = Command::cargo_bin("ait-cli").unwrap();
            cmd.current_dir(repo.path())
                .env("AIT_CLI_DELEGATE_BIN", &script_path)
                .env("AIT_CLI_DELEGATE_LOG", &log_path)
                .args(["workflow", subcommand, "RCC-1", json_arg]);
            cmd.assert()
                .failure()
                .stderr(predicate::str::contains(
                    "unexpected argument '--json' found",
                ))
                .stderr(predicate::str::contains("tip:").not())
                .stderr(predicate::str::contains("-- --json").not());
            assert!(!log_path.exists());
        }
    }
}

#[test]
fn workflow_land_help_stays_native_without_top_level_land_or_task_complete() {
    let repo = make_repo();
    let log_path = repo.path().join("workflow-land-local.log");
    let script_path = make_delegate_script(repo.path());

    let mut land_local_help = Command::cargo_bin("ait-cli").unwrap();
    land_local_help
        .current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .env("AIT_CLI_DELEGATE_LOG", &log_path)
        .args(["workflow", "land-local", "--help"]);
    land_local_help
        .assert()
        .success()
        .stdout(predicate::str::contains("--json").not());
    assert!(!log_path.exists());

    let mut workflow_land_help = Command::cargo_bin("ait-cli").unwrap();
    workflow_land_help
        .current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .env("AIT_CLI_DELEGATE_LOG", &log_path)
        .args(["workflow", "land", "--help"]);
    workflow_land_help
        .assert()
        .success()
        .stdout(predicate::str::contains("--apply"))
        .stdout(predicate::str::contains("--json").not());
    assert!(!log_path.exists());

    let mut top_level_land_help = Command::cargo_bin("ait-cli").unwrap();
    top_level_land_help
        .current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .env("AIT_CLI_DELEGATE_LOG", &log_path)
        .args(["land", "--help"]);
    top_level_land_help
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand 'land'"));
    assert!(!log_path.exists());

    let mut task_help = Command::cargo_bin("ait-cli").unwrap();
    task_help
        .current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .env("AIT_CLI_DELEGATE_LOG", &log_path)
        .args(["task", "--help"]);
    task_help
        .assert()
        .success()
        .stdout(predicate::str::contains("complete").not())
        .stdout(predicate::str::contains("land"));
    assert!(!log_path.exists());
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
            "solo_local lands only local draft state",
        ))
        .stdout(predicate::str::contains(
            "final Change finishes a bound Task",
        ))
        .stdout(predicate::str::contains(
            "Remote closeout consumes an already-ready Patchset and leaves Plan state untouched",
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
        .stdout(predicate::str::contains("Write local Plan lineage only"))
        .stdout(predicate::str::contains(
            "Publish the touched local Plan heads",
        ));
}

#[test]
fn workflow_guide_is_native_and_not_delegated() {
    let repo = make_repo();
    let log_path = repo.path().join("workflow-guide.log");
    let script_path = make_delegate_script(repo.path());

    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .env("AIT_CLI_DELEGATE_LOG", &log_path)
        .args(["workflow", "guide", "land"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("ait workflow guide · land"))
        .stdout(predicate::str::contains("task-land-plan-closeout/v1"))
        .stdout(predicate::str::contains(
            "automatic_exact_local_when_final_task_completed",
        ))
        .stdout(predicate::str::contains("separate_after_land"));
    assert!(!log_path.exists());
}

fn assert_unknown_subcommand_fails_closed(
    repo: &TempDir,
    log_name: &str,
    args: &[&str],
    expected_error: &str,
) {
    let log_path = repo.path().join(log_name);
    let script_path = make_delegate_script(repo.path());

    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .args(args);
    cmd.assert()
        .failure()
        .stderr(predicate::str::contains(expected_error));
    assert!(!log_path.exists());
}

#[test]
fn unsupported_snapshot_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        "snapshot.log",
        &["snapshot", "legacy-export", "SNP-1"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn unsupported_change_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        "change.log",
        &["change", "legacy-export", "RC-1"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn standalone_change_create_help_is_native_and_not_delegated() {
    let repo = make_repo();
    let log_path = repo.path().join("change-create.log");
    let script_path = make_delegate_script(repo.path());
    let mut cmd = Command::cargo_bin("ait-cli").unwrap();
    cmd.current_dir(repo.path())
        .env("AIT_CLI_DELEGATE_BIN", &script_path)
        .env("AIT_CLI_DELEGATE_LOG", &log_path)
        .args(["change", "create", "--help"]);
    cmd.assert()
        .success()
        .stdout(predicate::str::contains("--title <TITLE>"))
        .stdout(predicate::str::contains("--base-line <BASE_LINE>"));
    assert!(!log_path.exists());
}

#[test]
fn unsupported_patchset_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        "patchset.log",
        &["patchset", "legacy-export", "--change", "RC-1"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn unsupported_review_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        "review.log",
        &["review", "legacy-export", "RC-1711"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn unsupported_policy_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        "policy.log",
        &["policy", "legacy-export", "RP-1"],
        "unrecognized subcommand 'legacy-export'",
    );
}

#[test]
fn removed_top_level_land_namespace_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        "land.log",
        &["land", "legacy-export", "LAND-1"],
        "unrecognized subcommand 'land'",
    );
}

#[test]
fn unsupported_queue_subcommand_fails_closed() {
    let repo = make_repo();
    assert_unknown_subcommand_fails_closed(
        &repo,
        "queue.log",
        &["queue", "legacy-export"],
        "unrecognized subcommand 'legacy-export'",
    );
}
