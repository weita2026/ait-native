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
        .stdout(predicate::str::contains("run-ci"))
        .stdout(predicate::str::contains("ci-capabilities"))
        .stdout(predicate::str::contains("storage").not())
        .stdout(predicate::str::contains("validate").not())
        .stdout(predicate::str::contains("optimize").not())
        .stdout(predicate::str::contains("reconcile").not());
}

#[test]
fn removed_maintenance_commands_are_absent_instead_of_hidden() {
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
