use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

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
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Invalid init CLI JSON").unwrap()
}

#[test]
fn init_cli_creates_then_reinitializes_the_agent_contract() {
    let temp = TempDir::new().unwrap();
    let expected_repo_name = temp
        .path()
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap()
        .to_string();
    let initialized = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["init", "--json"]),
    );

    assert_eq!(initialized["action"], "initialized");
    assert_eq!(initialized["repo_name"], expected_repo_name);
    assert_eq!(initialized["default_line"], "main");
    assert_eq!(initialized["policy_profile"], "prototype");
    assert_eq!(initialized["default_author_mode"], "ai_with_human_review");
    assert!(initialized["default_model"].is_null());
    assert!(temp.path().join(".ait/config.json").is_file());
    let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
    assert!(agents.contains("<!-- ait:workflow:start -->"));
    assert!(agents.contains("sprint mode: `on`"));
    assert!(temp.path().join("docs/sprints").is_dir());
    for path in ["ait-native.md", "docs/plan.md", "docs/milestone.md"] {
        assert!(!temp.path().join(path).exists(), "unexpected {path}");
    }

    let reinitialized = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["init", "--json"]),
    );

    assert_eq!(reinitialized["action"], "reinitialized");
    assert_eq!(reinitialized["repo_name"], expected_repo_name);
    assert_eq!(reinitialized["default_line"], "main");
    assert_eq!(reinitialized["policy_profile"], "prototype");
    assert_eq!(
        fs::read_to_string(temp.path().join("AGENTS.md"))
            .unwrap()
            .matches("<!-- ait:workflow:start -->")
            .count(),
        1
    );
}

#[test]
fn init_cli_invalid_input_leaves_no_ait_entry() {
    let temp = TempDir::new().unwrap();

    cargo_bin()
        .current_dir(temp.path())
        .args(["init", "--policy-profile", "invalid"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown policy profile"));

    assert!(fs::symlink_metadata(temp.path().join(".ait")).is_err());
}

#[test]
fn init_cli_rejects_retired_bootstrap_options() {
    for (option, value) in [
        ("--name", "demo"),
        ("--default-line", "develop"),
        ("--default-author-mode", "human_only"),
        ("--default-model", "example-model"),
    ] {
        let temp = TempDir::new().unwrap();
        cargo_bin()
            .current_dir(temp.path())
            .args(["init", option, value])
            .assert()
            .failure()
            .stderr(predicate::str::contains("unexpected argument"));
        assert!(fs::symlink_metadata(temp.path().join(".ait")).is_err());
    }
}

#[test]
fn init_cli_help_and_human_output_describe_repository_initialization() {
    cargo_bin()
        .args(["init", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Create or reinitialize a local AIT repository.",
        ))
        .stdout(predicate::str::contains("--policy-profile"))
        .stdout(predicate::str::contains("--repair-existing"))
        .stdout(predicate::str::contains("--name").not())
        .stdout(predicate::str::contains("--default-line").not())
        .stdout(predicate::str::contains("--default-author-mode").not())
        .stdout(predicate::str::contains("--default-model").not())
        .stdout(predicate::str::contains("onboarding").not());

    let temp = TempDir::new().unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "Initialized empty AIT repository in ",
        ));
    cargo_bin()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "Reinitialized existing AIT repository in ",
        ));

    fs::remove_file(temp.path().join(".ait/policy.yaml")).unwrap();
    cargo_bin()
        .current_dir(temp.path())
        .args(["init", "--repair-existing"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "Repaired existing AIT repository in ",
        ))
        .stdout(predicate::str::contains("\n- missing policy.yaml\n"));
}
