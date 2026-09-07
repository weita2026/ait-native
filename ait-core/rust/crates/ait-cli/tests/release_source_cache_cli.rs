#![cfg(unix)]

use assert_cmd::Command;
use serde_json::Value;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;

fn run_json(root: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("ait-cli")
        .unwrap()
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn source_cache_records_bootstrap_policy_through_a_completed_task_before_remote_registration() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let destination = root.join("ait-core");
    let wrapper = root.join("ait-bootstrap");
    fs::write(
        &wrapper,
        "#!/bin/sh\nset -eu\n\
         if [ \"${1:-}\" = remote ] && [ \"${2:-}\" = add ]; then\n\
           printf '%s\\n' 'bootstrap reached remote registration' >&2\n\
           exit 86\n\
         fi\n\
         exec \"$AIT_RELEASE_TEST_REAL_BIN\" \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o755)).unwrap();

    // Run the real source-cache script and CLI until the first network boundary.
    // The old root-level Snapshot command fails before this sentinel is reached.
    let output = std::process::Command::new("bash")
        .arg(source.join("ci/release_source_cache.sh"))
        .arg(&wrapper)
        .args([
            "ait-core",
            "0",
            "C",
            "SNP-0123456789AB",
            "1.1.1",
            "Apache-2.0",
        ])
        .arg(source.join("ci/patch_ci.json"))
        .arg(&destination)
        .env("AIT_RELEASE_SERVER_URL", "http://127.0.0.1:9")
        .env(
            "AIT_RELEASE_TEST_REAL_BIN",
            assert_cmd::cargo::cargo_bin("ait-cli"),
        )
        .env("TMPDIR", &root)
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(86),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("bootstrap reached remote registration")
    );

    let line = run_json(
        &destination,
        &["line", "show", "release-bootstrap", "--json"],
    );
    let snapshot_id = line["head_snapshot_id"].as_str().unwrap();
    let snapshot = run_json(&destination, &["snapshot", "show", snapshot_id, "--json"]);
    assert_eq!(snapshot["message"], "Release source-cache bootstrap policy");
    let paths: Vec<_> = snapshot["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"ci/patch_ci.json"));
    assert!(paths.contains(&"ait-external.toml"));
    let parent_id = snapshot["parent_snapshot_ids"][0].as_str().unwrap();
    let parent = run_json(&destination, &["snapshot", "show", parent_id, "--json"]);
    assert_eq!(parent["message"], "Initialize Task base");
    let parent_paths: Vec<_> = parent["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert!(parent_paths.contains(&"ci/patch_ci.json"));
    assert!(!parent_paths.contains(&"ait-external.toml"));
    let tasks = run_json(&destination, &["task", "list", "--all", "--json"]);
    let tasks = tasks.as_array().unwrap();
    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0]["status"], "completed");
    assert!(run_json(&destination, &["remote", "list", "--json"])
        .as_array()
        .unwrap()
        .is_empty());
}
