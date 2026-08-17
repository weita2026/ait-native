use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("ait-core repository root")
        .to_path_buf()
}

fn contract() -> Value {
    let path = repo_root().join("ci/native_bootstrap_matrix.json");
    serde_json::from_slice(&fs::read(path).expect("bootstrap matrix bytes"))
        .expect("valid bootstrap matrix JSON")
}

#[test]
fn bootstrap_matrix_is_the_exact_six_target_native_set() {
    let contract = contract();
    let targets = contract["targets"].as_array().expect("target rows");
    let observed = targets
        .iter()
        .map(|row| row["target"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();
    let expected = BTreeSet::from([
        "aarch64-apple-darwin".to_string(),
        "x86_64-apple-darwin".to_string(),
        "aarch64-unknown-linux-gnu".to_string(),
        "x86_64-unknown-linux-gnu".to_string(),
        "aarch64-pc-windows-msvc".to_string(),
        "x86_64-pc-windows-msvc".to_string(),
    ]);
    assert_eq!(targets.len(), 6);
    assert_eq!(observed, expected);
    assert_eq!(contract["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(contract["rust_toolchain"], "1.96.0");
    assert_eq!(contract["cargo_profile"], "release");
    assert_eq!(contract["public_identity"], "ait");
    assert_eq!(contract["public_publish"], false);
}

#[test]
fn bootstrap_matrix_records_matching_native_runner_and_floor_facts() {
    let contract = contract();
    let rows = contract["targets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| (row["target"].as_str().unwrap(), row))
        .collect::<BTreeMap<_, _>>();
    let expected = [
        (
            "aarch64-apple-darwin",
            "macos-15",
            "macos",
            "arm64",
            "macos_deployment_target",
            "13.0",
            "",
        ),
        (
            "x86_64-apple-darwin",
            "macos-15-intel",
            "macos",
            "x86_64",
            "macos_deployment_target",
            "13.0",
            "",
        ),
        (
            "aarch64-unknown-linux-gnu",
            "ubuntu-22.04-arm",
            "linux",
            "arm64",
            "glibc",
            "2.35",
            "",
        ),
        (
            "x86_64-unknown-linux-gnu",
            "ubuntu-22.04",
            "linux",
            "x86_64",
            "glibc",
            "2.35",
            "",
        ),
        (
            "aarch64-pc-windows-msvc",
            "windows-11-arm",
            "windows",
            "arm64",
            "windows_build",
            "26100",
            ".exe",
        ),
        (
            "x86_64-pc-windows-msvc",
            "windows-2025",
            "windows",
            "x86_64",
            "windows_build",
            "17763",
            ".exe",
        ),
    ];
    for (target, runner, os, architecture, floor_kind, floor, suffix) in expected {
        let row = rows.get(target).expect("exact bootstrap target row");
        assert_eq!(row["runner"].as_str(), Some(runner));
        assert_eq!(row["os"].as_str(), Some(os));
        assert_eq!(row["architecture"].as_str(), Some(architecture));
        assert_eq!(row["minimum_platform_kind"].as_str(), Some(floor_kind));
        assert_eq!(row["minimum_platform"].as_str(), Some(floor));
        assert_eq!(row["executable_suffix"].as_str(), Some(suffix));
    }
}

#[test]
fn bootstrap_workflow_builds_and_executes_without_publication_or_fallback() {
    let workflow =
        fs::read_to_string(repo_root().join(".github/workflows/ait-native-bootstrap.yml"))
            .expect("bootstrap workflow");
    for target in contract()["targets"].as_array().unwrap() {
        assert!(!workflow.contains(target["target"].as_str().unwrap()));
    }
    assert!(workflow.contains("fromJSON(needs.contract.outputs.matrix)"));
    assert!(workflow.contains("jq -e -f ci/native_bootstrap_matrix.jq"));
    assert!(workflow.contains("rustup run \"${{ needs.contract.outputs.toolchain }}\" cargo build"));
    assert!(workflow.contains("--locked \\\n"));
    assert!(workflow.contains("--release \\\n"));
    assert!(workflow.contains("--target \"${{ matrix.target }}\""));
    assert!(workflow.contains("test \"${{ runner.arch }}\" = \"${expected_runner_arch}\""));
    assert!(workflow.contains("\"${AIT_BOOTSTRAP_BIN}\" --version"));
    assert!(workflow.contains("\"${AIT_BOOTSTRAP_BIN}\" --help"));
    assert!(workflow.contains("\"${AIT_BOOTSTRAP_BIN}\" init --json"));
    assert!(workflow.contains("plan list --json"));
    assert!(workflow.contains("test ! -s \"${smoke}/${stderr_channel}\""));
    assert!(workflow.contains("cp \"${AIT_BOOTSTRAP_SMOKE}\"/*.stdout"));
    assert!(workflow.contains("cp \"${AIT_BOOTSTRAP_SMOKE}\"/*.stderr"));
    assert!(workflow.contains("stdout_and_stderr_captured_separately"));
    assert!(workflow.contains("actions/upload-artifact@v4"));
    assert!(workflow.contains("${{ needs.contract.outputs.artifact_prefix }}-${{ matrix.target }}"));
    assert!(workflow.contains("command -v sha256sum"));
    assert!(workflow.contains("command -v shasum"));
    assert!(!workflow.contains("contents: write"));
    assert!(!workflow.contains("id-token: write"));
    assert!(!workflow.contains("gh release"));
    assert!(!workflow.contains("python"));
    assert!(!workflow.contains("node"));
    assert!(!workflow.contains("curl "));
    assert!(!workflow.contains("wget "));
    assert!(!workflow.contains("init --name"));
    assert!(!workflow.contains("--default-line"));
}
