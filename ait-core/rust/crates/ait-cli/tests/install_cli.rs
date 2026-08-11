use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

fn cargo_bin() -> Command {
    Command::cargo_bin("ait-cli").unwrap()
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

fn temp_dir_outside_repo() -> TempDir {
    let system_tmp = Path::new("/tmp");
    if system_tmp.is_dir() {
        tempfile::Builder::new()
            .prefix("ait-install-cli-test-")
            .tempdir_in(system_tmp)
            .unwrap()
    } else {
        TempDir::new().unwrap()
    }
}

#[test]
fn install_cli_initializes_repo_with_sprint_enabled_by_default() {
    let temp = temp_dir_outside_repo();

    let payload = output_json(cargo_bin().current_dir(temp.path()).args([
        "install",
        "--user-name",
        "Ada Lovelace",
        "--user-email",
        "ada@example.test",
        "--json",
    ]));

    assert_eq!(payload["repository"]["state"], "initialized_repo");
    assert_eq!(payload["mode"]["requested_mode"], "solo_local");
    assert_eq!(payload["identity"]["user_name"], "Ada Lovelace");
    assert_eq!(payload["identity"]["user_email"], "ada@example.test");
    assert_eq!(payload["sprint"]["value"], "on");
    assert_eq!(payload["sprint"]["plan_task_binding_mode"], "required");

    let config = fs::read_to_string(temp.path().join(".ait/config.json")).unwrap();
    assert!(config.contains("\"workflow_mode\": \"solo_local\""));
    assert!(config.contains("\"user_name\": \"Ada Lovelace\""));
    assert!(config.contains("\"user_email\": \"ada@example.test\""));
    assert!(config.contains("\"sprint\": \"on\""));
    assert!(config.contains("\"mode\": \"required\""));
    assert!(temp.path().join("AGENTS.md").is_file());
    assert!(temp.path().join("docs/sprints").is_dir());
    assert!(!temp.path().join("ait-native.md").exists());
    assert!(!temp.path().join("docs/plan.md").exists());
    assert!(!temp.path().join("docs/milestone.md").exists());
}

#[test]
fn install_cli_no_sprint_disables_plan_task_binding() {
    let temp = temp_dir_outside_repo();

    let payload = output_json(cargo_bin().current_dir(temp.path()).args([
        "install",
        "--no-sprint",
        "--json",
    ]));

    assert_eq!(payload["sprint"]["value"], "off");
    assert_eq!(payload["sprint"]["plan_task_binding_mode"], "off");

    let config = fs::read_to_string(temp.path().join(".ait/config.json")).unwrap();
    assert!(config.contains("\"sprint\": \"off\""));
    assert!(config.contains("\"mode\": \"off\""));
    assert!(temp.path().join("AGENTS.md").is_file());
    assert!(!temp.path().join("docs").exists());
    assert!(!temp.path().join("ait-native.md").exists());
}

#[test]
fn install_cli_configures_an_existing_initialized_agent_contract() {
    let temp = temp_dir_outside_repo();
    output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["init", "--json"]),
    );
    let agents_path = temp.path().join("AGENTS.md");
    let mut agents = fs::read_to_string(&agents_path).unwrap();
    agents.push_str("\nKeep this repository rule.\n");
    fs::write(&agents_path, agents).unwrap();
    assert!(temp.path().join("docs/sprints").is_dir());

    let payload = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["install", "--json"]),
    );

    assert_eq!(payload["repository"]["state"], "existing_repo");
    assert!(fs::read_to_string(&agents_path)
        .unwrap()
        .contains("Keep this repository rule."));
    assert!(!temp.path().join("ait-native.md").exists());
    assert!(!temp.path().join("docs/plan.md").exists());
    assert!(!temp.path().join("docs/milestone.md").exists());
    assert!(temp.path().join("docs/sprints").is_dir());
}

#[test]
fn install_cli_rerun_preserves_existing_mode_and_sprint() {
    let temp = temp_dir_outside_repo();

    output_json(cargo_bin().current_dir(temp.path()).args([
        "install",
        "--mode",
        "remote",
        "--server-setup",
        "skip",
        "--no-sprint",
        "--json",
    ]));
    let payload = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["install", "--json"]),
    );

    assert_eq!(payload["mode"]["requested_mode"], "solo_remote");
    assert_eq!(payload["mode"]["effective_mode"], "solo_remote");
    assert_eq!(payload["mode"]["source"], "existing_repository");
    assert_eq!(payload["sprint"]["value"], "off");
    assert_eq!(payload["sprint"]["source"], "existing_repository");

    let config = fs::read_to_string(temp.path().join(".ait/config.json")).unwrap();
    assert!(config.contains("\"workflow_mode\": \"solo_remote\""));
    assert!(config.contains("\"sprint\": \"off\""));
    assert!(!temp.path().join("docs").exists());
}

#[test]
fn install_cli_remote_admission_failure_preserves_local_workflow_defaults() {
    let temp = temp_dir_outside_repo();

    let output = cargo_bin()
        .current_dir(temp.path())
        .args([
            "install",
            "--mode",
            "remote",
            "--server-setup",
            "connect",
            "--server-url",
            "http://127.0.0.1:1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Remote registration was not attempted")
    );
    let config = fs::read_to_string(temp.path().join(".ait/config.json")).unwrap();
    assert!(!config.contains("\"workflow_mode\": \"solo_remote\""));
    let effective = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["config", "show", "--json"]),
    );
    assert_eq!(effective["workflow_mode"]["value"], "solo_local");
    assert_eq!(effective["sprint"]["value"], "on");
    assert!(temp.path().join("ci/patch_ci.json").is_file());
}

#[test]
fn install_cli_remote_dry_run_does_not_claim_health() {
    let temp = temp_dir_outside_repo();
    output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["install", "--json"]),
    );

    let payload = output_json(cargo_bin().current_dir(temp.path()).args([
        "install",
        "--mode",
        "remote",
        "--server-setup",
        "connect",
        "--server-url",
        "http://127.0.0.1:1",
        "--remote-name",
        "unreachable",
        "--dry-run",
        "--json",
    ]));

    assert_eq!(payload["server"]["classification"], "configured_unverified");
    assert_ne!(payload["server"]["classification"], "healthy");
}

#[test]
fn install_cli_reads_transport_secret_from_environment() {
    let temp = temp_dir_outside_repo();

    let payload = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .env("AIT_TELEGRAM_BOT_TOKEN", "environment-placeholder-secret")
            .args(["install", "--attach", "telegram", "--json"]),
    );

    assert_eq!(payload["transport_actions"][0]["action"], "created");
    assert_eq!(payload["worker_manifest"]["storage"], "plaintext_json");
    let manifest = fs::read_to_string(temp.path().join(".ait/agent-workers.json")).unwrap();
    assert!(manifest.contains("environment-placeholder-secret"));
    #[cfg(unix)]
    assert_eq!(
        fs::metadata(temp.path().join(".ait/agent-workers.json"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
}
