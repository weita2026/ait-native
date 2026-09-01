use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value as JsonValue, json};

#[test]
fn package_declares_the_owner_selected_apache_license() {
    assert_eq!(env!("CARGO_PKG_LICENSE"), "Apache-2.0");
    assert!(include_str!("../LICENSE").contains("Apache License"));

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let notice = fs::read_to_string(repo_root.join("NOTICE")).expect("NOTICE");
    assert!(notice.contains("ait-runner"));
    assert!(notice.contains("`ait-core` Snapshot selected by `ait-external.lock`"));
    assert_eq!(
        notice
            .matches("----- BEGIN GENERATED THIRD-PARTY NOTICES -----")
            .count(),
        1
    );
    assert!(!notice.contains("/.cargo/registry/"));
    assert!(!notice.contains("/Users/"));
    assert!(!notice.contains("/Volumes/"));

    let lock = fs::read_to_string(repo_root.join("Cargo.lock")).expect("Cargo.lock");
    let lock: toml::Value = toml::from_str(&lock).expect("Cargo.lock should parse");
    for package in lock
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
        .filter(|package| package.get("source").is_some())
    {
        let name = package
            .get("name")
            .and_then(toml::Value::as_str)
            .expect("locked package name");
        let version = package
            .get("version")
            .and_then(toml::Value::as_str)
            .expect("locked package version");
        let row_prefix = format!("{name}\t{version}\t");
        assert!(
            notice.lines().any(|line| line.starts_with(&row_prefix)),
            "NOTICE is missing locked package {name} {version}"
        );
    }

    for relative in ["ait-external.toml", "ait-external.lock"] {
        let pin = fs::read_to_string(repo_root.join(relative)).expect("core pin");
        assert!(
            pin.contains("snapshot = \"SNP-8F6FBEF7B117\""),
            "{relative} must select the corrected core Snapshot"
        );
    }
    let generator =
        fs::read_to_string(repo_root.join("ci/generate_notice.sh")).expect("notice wrapper");
    assert!(generator.contains(".ait-external/ait-core/ci/generate_rust_notice.sh"));
    assert!(generator.contains("--manifest \"$repo_root/Cargo.toml\""));
    assert!(generator.contains("run 'ait external update --locked --validate' first"));
    assert!(!generator.contains("ait external update ait-core --locked"));
}

#[test]
fn execute_reads_typed_stdin_and_leaves_attempt_parent_empty() {
    let source = tempfile::tempdir().expect("source");
    let attempts = tempfile::tempdir().expect("attempts");
    fs::create_dir(source.path().join("ci")).expect("ci directory");
    let script = source.path().join("ci/run.sh");
    fs::write(&script, "#!/bin/sh\nset -eu\nprintf 'cli-ok'\n").expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    let request = json!({
        "contract": "ait.runner.native-job.v3",
        "source": {
            "kind": "local_directory",
            "path": "."
        },
        "command": {
            "argv": ["./ci/run", "patchset"]
        },
        "timeout_ms": 5000
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_ait-runner"))
        .args([
            "execute",
            "--source-root",
            source.path().to_str().expect("source path"),
            "--attempt-root",
            attempts.path().to_str().expect("attempt path"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start CLI");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(request.to_string().as_bytes())
        .expect("write request");
    let output = child.wait_with_output().expect("CLI output");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result: JsonValue = serde_json::from_slice(&output.stdout).expect("result JSON");
    assert_eq!(result["contract"], "ait.runner.native-result.v1");
    assert_eq!(result["status"], "succeeded");
    assert_eq!(result["cleanup"]["attempt_root_removed"], true);
    assert_eq!(
        fs::read_dir(attempts.path())
            .expect("attempt parent")
            .count(),
        0
    );
}

#[test]
fn execute_rejects_shell_text_before_creating_attempt() {
    let source = tempfile::tempdir().expect("source");
    let attempts = tempfile::tempdir().expect("attempts");
    let request = json!({
        "contract": "ait.runner.native-job.v3",
        "source": {
            "kind": "local_directory",
            "path": "."
        },
        "command": {
            "argv": ["sh", "-c", "echo unsafe"]
        }
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_ait-runner"))
        .args([
            "execute",
            "--source-root",
            source.path().to_str().expect("source path"),
            "--attempt-root",
            attempts.path().to_str().expect("attempt path"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start CLI");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(request.to_string().as_bytes())
        .expect("write request");
    let output = child.wait_with_output().expect("CLI output");
    assert!(!output.status.success());
    let error: JsonValue = serde_json::from_slice(&output.stderr).expect("error JSON");
    assert_eq!(error["contract"], "ait.runner.error.v1");
    assert!(error["error"].as_str().unwrap().contains("./ci/run"));
    assert_eq!(
        fs::read_dir(attempts.path())
            .expect("attempt parent")
            .count(),
        0
    );
}

#[test]
fn removed_configuration_environment_cannot_supply_required_cli_arguments() {
    let doctor = Command::new(env!("CARGO_BIN_EXE_ait-runner"))
        .arg("doctor")
        .env("AIT_SERVER_URL", "http://127.0.0.1:1")
        .output()
        .expect("run doctor argument validation");
    assert!(!doctor.status.success());
    assert!(String::from_utf8_lossy(&doctor.stderr).contains("--server"));

    let serve = Command::new(env!("CARGO_BIN_EXE_ait-runner"))
        .args(["serve", "--server", "http://127.0.0.1:1", "--once"])
        .env("AIT_RUNNER_WORKER_ID", "ambient-worker")
        .output()
        .expect("run serve argument validation");
    assert!(!serve.status.success());
    assert!(String::from_utf8_lossy(&serve.stderr).contains("--worker-id"));
}

#[test]
fn child_attempt_root_environment_does_not_configure_the_runner_cli() {
    let source = tempfile::tempdir().expect("source");
    fs::create_dir(source.path().join("ci")).expect("ci directory");
    let script = source.path().join("ci/run.sh");
    fs::write(&script, "#!/bin/sh\nset -eu\nprintf 'explicit-cli-only'\n").expect("script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    let blocked_parent = tempfile::NamedTempFile::new().expect("blocking file");
    let request = json!({
        "contract": "ait.runner.native-job.v3",
        "source": {
            "kind": "local_directory",
            "path": "."
        },
        "command": {
            "argv": ["./ci/run", "patchset"]
        },
        "timeout_ms": 5000
    });
    let mut child = Command::new(env!("CARGO_BIN_EXE_ait-runner"))
        .args([
            "execute",
            "--source-root",
            source.path().to_str().expect("source path"),
        ])
        .env("AIT_RUNNER_ATTEMPT_ROOT", blocked_parent.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start CLI");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(request.to_string().as_bytes())
        .expect("write request");
    let output = child.wait_with_output().expect("CLI output");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(blocked_parent.path().is_file());
}
