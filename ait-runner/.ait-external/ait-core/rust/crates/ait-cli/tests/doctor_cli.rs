use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn cargo_bin() -> Command {
    Command::cargo_bin("ait-cli").unwrap()
}

fn write_file(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut file = fs::File::create(path).unwrap();
    file.write_all(content.as_bytes()).unwrap();
}

fn output_json(command: &mut Command) -> JsonValue {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "command failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid CLI JSON").unwrap()
}

#[test]
fn doctor_plan_authority_runs_without_repo() {
    let temp = TempDir::new().unwrap();
    let retired_backend_selector = ["AIT", "PLAN", "CORE", "BACKEND"].join("_");
    let retired_extension_selector = ["AIT", "RUST", "EXT", "MODULE"].join("_");
    let payload = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .env(retired_backend_selector, "python")
            .env(retired_extension_selector, "untrusted_extension")
            .args(["doctor", "plan-authority", "--json"]),
    );
    assert_eq!(payload["selected_backend"], "rust");
    assert_eq!(payload["compatibility"], "compatible");
    assert_eq!(payload["rust_authority_ready"], true);
    assert_eq!(payload["extension_module"], "ait_py");
    assert_eq!(payload["env"].as_object().unwrap().len(), 0);
    assert_eq!(
        payload["extension_plan_contract_version"],
        "plan-foundation-v7"
    );
    assert_eq!(payload["missing_exports"].as_array().unwrap().len(), 0);
    assert_eq!(payload["repository_inspected"], false);
    assert!(payload["repository_authority"].is_null());
}

#[test]
fn doctor_plan_authority_reports_repairable_repository_damage_without_mutating_it() {
    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    let authority = temp.path().join(".ait/binary-db");
    for name in [
        "plan.bin",
        "plan_payload.bin",
        "plan_revision.bin",
        "plan_revision_payload.bin",
        "plan_item.bin",
        "plan_item_payload.bin",
    ] {
        let _ = fs::remove_file(authority.join(name));
    }
    fs::write(authority.join("plan_payload.bin"), []).unwrap();

    let payload = output_json(cargo_bin().current_dir(temp.path()).args([
        "doctor",
        "plan-authority",
        "--json",
    ]));
    assert_eq!(payload["compatibility"], "compatible");
    assert_eq!(payload["repository_inspected"], true);
    assert_eq!(payload["repository_ready"], false);
    assert_eq!(payload["repository_authority"]["state"], "repairable");
    assert_eq!(
        payload["repository_authority"]["recommended_action"],
        "retry_for_safe_automatic_recovery"
    );
    assert!(authority.join("plan_payload.bin").exists());
    assert_eq!(
        fs::metadata(authority.join("plan_payload.bin"))
            .unwrap()
            .len(),
        0,
        "doctor must remain read-only"
    );
}

#[test]
fn doctor_memory_root_rejects_malformed_repository_configuration() {
    let temp = TempDir::new().unwrap();
    fs::create_dir_all(temp.path().join(".ait")).unwrap();
    let kind = if cfg!(target_os = "macos") {
        "macos_ram_volume"
    } else if cfg!(target_os = "windows") {
        "windows_ramdisk"
    } else {
        "linux_memory_root"
    };
    write_file(
        &temp.path().join(".ait/config.json"),
        &format!(
            r#"{{
  "repo_name": "fixture-ait",
  "default_line": "main",
  "task_worktree": {{
    "memory_root": {{
      "kind": "{kind}",
      "root": "relative/ram",
      "volume_name": "ram",
      "sector_count": 1024
    }}
  }}
}}"#
        ),
    );
    let output = cargo_bin()
        .current_dir(temp.path())
        .arg("doctor")
        .arg("memory-root")
        .arg("--json")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("task_worktree.memory_root.root must be an absolute path"),
        "stderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn doctor_runtime_root_reports_inside_repo_as_snapshot_protected_warning() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    fs::create_dir_all(root.join(".ait")).unwrap();
    write_file(
        &root.join(".ait/config.json"),
        r#"{"repo_name":"fixture-ait","default_line":"main"}"#,
    );
    write_file(&root.join(".aitignore"), "runtime-data\n");

    let runtime_root = root.join("runtime-data");
    let payload = output_json(
        cargo_bin()
            .current_dir(root)
            .env("AIT_RUNTIME_DATA", &runtime_root)
            .args(["doctor", "runtime-root", "--json"]),
    );
    assert_eq!(payload["state"], "warn");
    assert_eq!(payload["inside_repo"], true);
    assert_eq!(payload["snapshot_ignored"], true);
    assert_eq!(payload["protected_from_snapshots"], true);
    assert_eq!(payload["runtime_root_relative_to_repo"], "runtime-data");
}

#[test]
fn doctor_help_exposes_only_configuration_driven_diagnostics() {
    cargo_bin()
        .args(["doctor", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("memory-root"))
        .stdout(predicate::str::contains("runtime-root"))
        .stdout(predicate::str::contains("plan-authority"))
        .stdout(predicate::str::contains("plan-authority-wheel").not());

    for (subcommand, effective_source) in [
        ("memory-root", "task_worktree.memory_root"),
        ("runtime-root", "AIT_RUNTIME_DATA"),
    ] {
        cargo_bin()
            .args(["doctor", subcommand, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains("--json"))
            .stdout(predicate::str::contains(effective_source));
    }

    cargo_bin()
        .args(["doctor", "plan-authority", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains(
            "fixed Rust-native Plan storage contract",
        ));
}

#[test]
fn doctor_rejects_every_retired_option_and_command_before_discovery() {
    let temp = TempDir::new().unwrap();
    for args in [
        vec!["doctor", "memory-root", "--ensure"],
        vec!["doctor", "runtime-root", "--server-data", "/tmp/runtime"],
        vec!["doctor", "plan-authority", "--backend", "rust"],
    ] {
        cargo_bin()
            .current_dir(temp.path())
            .args(args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("unexpected argument"))
            .stderr(predicate::str::contains("Not an ait repository").not());
    }

    cargo_bin()
        .current_dir(temp.path())
        .args(["doctor", "plan-authority-wheel"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unrecognized subcommand"))
        .stderr(predicate::str::contains("Not an ait repository").not());
    assert!(fs::symlink_metadata(temp.path().join(".ait")).is_err());
}
