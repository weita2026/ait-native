use assert_cmd::Command;
use serde_json::Value;

fn command_in(root: &std::path::Path) -> Command {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli build artifact");
    command.current_dir(root);
    command
}

fn init_external_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("temporary external fixture");
    command_in(root.path())
        .args(["init", "--json"])
        .assert()
        .success();
    std::fs::write(
        root.path().join("ait-external.toml"),
        r#"[[external]]
name = "ait-db"
repo_name = "ait-db"
repository_index = 1
remote = "origin"
line = "main"
snapshot = "SNP-MISSING"
materialize_to = ".ait-external/ait-db"
license = "Apache-2.0"
"#,
    )
    .expect("external manifest");
    root
}

#[test]
fn external_doctor_fail_on_blocking_preserves_json_report_and_returns_two() {
    let root = init_external_fixture();

    let diagnostic = command_in(root.path())
        .args(["external", "doctor", "--json"])
        .output()
        .expect("diagnostic doctor");
    assert!(diagnostic.status.success());
    let diagnostic_payload: Value =
        serde_json::from_slice(&diagnostic.stdout).expect("diagnostic JSON");
    assert_eq!(diagnostic_payload["release_ready"], false);
    assert_eq!(diagnostic_payload["summary"]["release_blocking"], 2);

    let blocking = command_in(root.path())
        .args(["external", "doctor", "--fail-on-blocking", "--json"])
        .output()
        .expect("blocking doctor");
    assert_eq!(blocking.status.code(), Some(2));
    assert!(blocking.stderr.is_empty());
    let blocking_payload: Value = serde_json::from_slice(&blocking.stdout).expect("blocking JSON");
    assert_eq!(blocking_payload, diagnostic_payload);

    let blocking_text = command_in(root.path())
        .args(["external", "doctor", "--fail-on-blocking"])
        .output()
        .expect("blocking text doctor");
    assert_eq!(blocking_text.status.code(), Some(2));
    assert!(blocking_text.stderr.is_empty());
    let blocking_text = String::from_utf8(blocking_text.stdout).expect("blocking doctor text");
    assert!(blocking_text.contains("ait external doctor"));
    assert!(blocking_text.contains("release_ready: false"));
}

#[test]
fn external_doctor_fail_on_blocking_succeeds_for_an_empty_external_set() {
    let root = tempfile::tempdir().expect("temporary empty external fixture");
    command_in(root.path())
        .args(["init", "--json"])
        .assert()
        .success();

    let output = command_in(root.path())
        .args(["external", "doctor", "--fail-on-blocking", "--json"])
        .output()
        .expect("ready doctor");

    assert!(output.status.success());
    let payload: Value = serde_json::from_slice(&output.stdout).expect("ready JSON");
    assert_eq!(payload["release_ready"], true);
}

#[test]
fn external_update_invalid_selection_combinations_exit_two_before_repo_discovery() {
    let root = tempfile::tempdir().expect("temporary parser fixture");
    let invalid = [
        &["external", "update", "ait-db"][..],
        &["external", "update", "--to", "SNP-DB-NEW"][..],
        &["external", "update", "--latest"][..],
        &[
            "external",
            "update",
            "ait-db",
            "--to",
            "SNP-DB-NEW",
            "--latest",
        ][..],
        &["external", "update", "ait-db", "--locked"][..],
    ];

    for args in invalid {
        let output = command_in(root.path())
            .args(args)
            .output()
            .expect("invalid external update");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        let stderr = String::from_utf8(output.stderr).expect("Clap error text");
        assert!(
            stderr.contains("Usage: ait external update"),
            "{args:?}: {stderr}"
        );
    }
}
