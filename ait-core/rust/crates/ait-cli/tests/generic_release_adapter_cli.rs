use assert_cmd::Command;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn run_json(root: &Path, args: &[&str]) -> Value {
    let output = Command::cargo_bin("ait-cli")
        .expect("ait-cli binary")
        .current_dir(root)
        .args(args)
        .output()
        .expect("ait-cli command executes");
    assert!(
        output.status.success(),
        "command failed: {:?}\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("ait-cli JSON output")
}

#[test]
fn generic_release_adapter_check_and_build_are_snapshot_derived_without_release_store() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    run_json(root, &["init", "--json"]);

    let output_name = if cfg!(windows) {
        "component-bin.exe"
    } else {
        "component-bin"
    };
    let smoke_path = if cfg!(windows) {
        ".\\component-bin.exe"
    } else {
        "./component-bin"
    };
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let manifest = json!({
        "schema": "ait.release.adapter/v1",
        "package": {
            "name": "adapter-fixture",
            "version": "1.2.3",
            "license_files": [
                {"path": "LICENSE", "role": "license"},
                {"path": "NOTICE", "role": "notice"}
            ]
        },
        "components": [{
            "id": "component",
            "ecosystem": "rust-fixture",
            "working_directory": ".",
            "dependency_files": ["component.rs"],
            "commands": {
                "test": [[rustc, "--version"]],
                "build": [[rustc, "component.rs", "-o", output_name]],
                "smoke": [[smoke_path]]
            },
            "artifacts": [{
                "path": output_name,
                "kind": "native-test-binary"
            }]
        }]
    });
    fs::write(
        root.join("ait-release.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("component.rs"),
        "fn main() { println!(\"adapter smoke ok\"); }\n",
    )
    .unwrap();
    fs::write(root.join("LICENSE"), "fixture license\n").unwrap();
    fs::write(root.join("NOTICE"), "fixture notice\n").unwrap();
    let snapshot = run_json(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "generic adapter fixture",
            "--json",
        ],
    );

    let checked = run_json(
        root,
        &[
            "release",
            "adapter",
            "check",
            "--version",
            "1.2.3",
            "--line",
            "main",
            "--json",
        ],
    );
    assert_eq!(checked["contract"], "ait.release.adapter.receipt/v1");
    assert_eq!(checked["status"], "checked");
    assert_eq!(checked["check_summary"]["decision"], "pass");
    assert_eq!(checked["snapshot_id"], snapshot["snapshot_id"]);
    assert_eq!(checked["authority"]["persistence"], "none");
    assert_eq!(
        checked["authority"]["local_release_authority"],
        "not_activated"
    );

    let built = run_json(
        root,
        &[
            "release",
            "adapter",
            "build",
            "--version",
            "1.2.3",
            "--line",
            "main",
            "--json",
        ],
    );
    assert_eq!(built["release_id"], checked["release_id"]);
    assert_eq!(built["snapshot_id"], checked["snapshot_id"]);
    assert_eq!(built["status"], "built");
    assert!(built.get("target").is_none());
    assert_eq!(
        built["next_action"]["code"],
        "promote_with_ecosystem_adapter"
    );
    let artifacts = built["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 5);
    assert_eq!(
        artifacts
            .iter()
            .filter(|artifact| artifact["role"] == "component-artifact")
            .count(),
        1
    );
    assert_eq!(
        artifacts
            .iter()
            .filter(|artifact| artifact["role"] == "license-material")
            .count(),
        2
    );
    assert_eq!(built["metadata"]["build"]["license_material_count"], 2);
    for artifact in artifacts {
        assert_eq!(artifact["sha256"].as_str().unwrap().len(), 64);
        assert!(artifact["size_bytes"].as_u64().unwrap() > 0);
        assert!(Path::new(artifact["absolute_path"].as_str().unwrap()).is_file());
    }
}

#[test]
fn generic_release_adapter_emits_independent_target_receipts_for_matrix_ci() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    run_json(root, &["init", "--json"]);

    let first_output = "x86_64-unknown-linux-gnu";
    let second_output = "aarch64-apple-darwin";
    let portable_output = "portable";
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let manifest = json!({
        "schema": "ait.release.adapter/v1",
        "package": {"name": "adapter-matrix", "version": "1.2.3"},
        "components": [{
            "id": "component",
            "ecosystem": "rust-fixture",
            "working_directory": ".",
            "dependency_files": ["component.rs"],
            "commands": {
                "prepare": [[rustc, "--version"]],
                "test": [[rustc, "--version"]],
                "build": [[rustc, "component.rs", "-o", "$AIT_RELEASE_TARGET"]]
            },
            "artifacts": [
                {
                    "path": first_output,
                    "kind": "native-test-binary",
                    "target": "x86_64-unknown-linux-gnu"
                },
                {
                    "path": second_output,
                    "kind": "native-test-binary",
                    "target": "aarch64-apple-darwin"
                },
                {
                    "path": portable_output,
                    "kind": "portable-metadata"
                }
            ]
        }]
    });
    fs::write(
        root.join("ait-release.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(root.join("component.rs"), "fn main() {}\n").unwrap();
    run_json(
        root,
        &[
            "snapshot",
            "create",
            "--message",
            "matrix adapter fixture",
            "--json",
        ],
    );

    let first = run_json(
        root,
        &[
            "release",
            "adapter",
            "build",
            "--version",
            "1.2.3",
            "--target",
            "x86_64-unknown-linux-gnu",
            "--json",
        ],
    );
    let second = run_json(
        root,
        &[
            "release",
            "adapter",
            "build",
            "--version",
            "1.2.3",
            "--target",
            "aarch64-apple-darwin",
            "--json",
        ],
    );
    let portable = run_json(
        root,
        &[
            "release",
            "adapter",
            "build",
            "--version",
            "1.2.3",
            "--target",
            "portable",
            "--json",
        ],
    );
    assert_ne!(first["release_id"], second["release_id"]);
    assert_ne!(first["release_id"], portable["release_id"]);
    assert_eq!(first["target"], "x86_64-unknown-linux-gnu");
    assert_eq!(second["target"], "aarch64-apple-darwin");
    assert!(portable.get("target").is_none());
    assert_eq!(portable["artifact_selection"], "portable");
    for (receipt, target) in [
        (&first, "x86_64-unknown-linux-gnu"),
        (&second, "aarch64-apple-darwin"),
    ] {
        let components = receipt["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|artifact| artifact["role"] == "component-artifact")
            .collect::<Vec<_>>();
        assert_eq!(components.len(), 1);
        assert_eq!(components[0]["target"], target);
        assert_eq!(receipt["metadata"]["build"]["declared_artifact_count"], 1);
        let build_command = receipt["metadata"]["build"]["components"][0]["commands"]
            .as_array()
            .unwrap()
            .iter()
            .find(|command| command["phase"] == "build")
            .unwrap();
        assert_eq!(build_command["declared_argv"][3], "$AIT_RELEASE_TARGET");
        assert_eq!(build_command["argv"][3], target);
    }
    let portable_components = portable["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|artifact| artifact["role"] == "component-artifact")
        .collect::<Vec<_>>();
    assert_eq!(portable_components.len(), 1);
    assert!(portable_components[0].get("target").is_none());
    assert_eq!(portable_components[0]["kind"], "portable-metadata");
    assert_eq!(
        portable["metadata"]["build"]["components"][0]["commands"][1]["argv"][3],
        "portable"
    );
}
