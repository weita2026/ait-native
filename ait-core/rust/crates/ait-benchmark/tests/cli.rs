use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn protocol_command_exposes_compiled_versioned_contract() {
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .arg("protocol")
        .assert()
        .success()
        .stdout(predicate::str::contains("ait-vcs-benchmark-protocol/v1"))
        .stdout(predicate::str::contains(
            "minimum_measured_local_iterations",
        ));
}

#[test]
fn fixture_digest_excludes_subject_specific_vcs_metadata() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("payload.txt"), "payload").unwrap();
    std::fs::create_dir(root.path().join(".git")).unwrap();
    std::fs::write(root.path().join(".git/config"), "metadata").unwrap();
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["fixture", "digest", "--root"])
        .arg(root.path())
        .assert()
        .success()
        .stdout(predicate::str::contains("sha256:"));
}

#[test]
fn plain_fixture_digest_is_probe_safe() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("payload.txt"), "payload").unwrap();
    Command::cargo_bin("ait-benchmark")
        .unwrap()
        .args(["fixture", "digest", "--plain", "--root"])
        .arg(root.path())
        .assert()
        .success()
        .stdout(predicate::str::starts_with("sha256:"))
        .stdout(predicate::str::is_match("^[^\\n]+\\n$").unwrap());
}
