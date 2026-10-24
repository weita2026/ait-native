use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn public_cli_identity_help_is_ait() {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Usage: ait <COMMAND>"))
        .stdout(predicate::str::contains(
            "AIT native local repository and workflow tool.",
        ))
        .stdout(predicate::str::contains(
            "Start, inspect, check, finish, or abandon Tasks locally or on a remote.",
        ))
        .stdout(predicate::str::contains("[alias: branch]"))
        .stdout(predicate::str::contains(
            "commit    Create an AIT Snapshot using Git-friendly commit naming.",
        ))
        .stdout(predicate::str::contains("audit, land").not())
        .stdout(predicate::str::contains("\n  ci-host ").not())
        .stdout(predicate::str::contains("\n  install ").not())
        .stdout(predicate::str::contains("\n  agent ").not())
        .stdout(predicate::str::contains("\n  test ").not())
        .stdout(predicate::str::contains("current-source-cache").not())
        .stdout(predicate::str::contains("ait-cli").not());
}

#[test]
fn public_cli_identity_version_is_package_version() {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("ait {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn public_agent_identity_version_is_package_version() {
    let mut command = Command::cargo_bin("ait-agent").expect("ait-agent build artifact");
    command
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("ait-agent {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn removed_install_command_is_absent_instead_of_hidden() {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command
        .arg("install")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));
}

#[test]
fn public_cli_identity_errors_use_ait_usage() {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command
        .arg("not-a-command")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Usage: ait <COMMAND>"))
        .stderr(predicate::str::contains("ait-cli").not());
}

#[test]
fn public_repo_help_contains_only_server_backed_commands() {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command
        .args(["repo", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("show"))
        .stdout(predicate::str::contains("jobs"))
        .stdout(predicate::str::contains("ci-capabilities"))
        .stdout(predicate::str::contains("run-ci").not())
        .stdout(predicate::str::contains("ci-runs").not())
        .stdout(predicate::str::contains("\n  storage").not())
        .stdout(predicate::str::contains("validate").not())
        .stdout(predicate::str::contains("optimize").not())
        .stdout(predicate::str::contains("reconcile").not());
}

#[test]
fn public_doctor_help_contains_only_retained_diagnostics() {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command
        .args(["doctor", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("memory-root"))
        .stdout(predicate::str::contains("runtime-root"))
        .stdout(predicate::str::contains("plan-authority"))
        .stdout(predicate::str::contains("postgres").not())
        .stdout(predicate::str::contains("plan-authority-wheel").not());
}

#[test]
fn removed_maintenance_commands_are_absent_instead_of_hidden() {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command
        .arg("test")
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"));

    for (namespace, removed) in [
        ("gc", "pack"),
        ("gc", "optimize"),
        ("repo", "storage"),
        ("repo", "validate"),
        ("repo", "pack"),
        ("repo", "optimize"),
        ("repo", "gc"),
        ("repo", "metrics"),
        ("repo", "readiness"),
        ("repo", "reconcile"),
        ("repo", "run-ci"),
        ("repo", "ci-runs"),
        ("doctor", "postgres"),
        ("doctor", "plan-authority-wheel"),
        ("plan", "audit-receipts"),
        ("binary-db", "repair-content-indexes"),
    ] {
        let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
        command
            .args([namespace, removed])
            .assert()
            .failure()
            .stderr(predicate::str::contains("unrecognized subcommand"));
    }
}
