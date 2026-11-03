#[path = "support/native_home.rs"]
mod native_home;
use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn cargo_bin() -> Command {
    let mut c = Command::cargo_bin("ait-cli").unwrap();
    c.env("CODEX_HOME", native_home::path())
        .env_remove("CODEX_THREAD_ID")
        .env_remove("CODEX_SESSION_ID");
    c
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
#[cfg(target_os = "linux")]
fn init_cli_detects_linux_language_once_and_config_clear_does_not_redetect() {
    let temp = TempDir::new().unwrap();
    let initialized = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .env_remove("LC_ALL")
            .env_remove("LC_MESSAGES")
            .env("LANG", "en_US.UTF-8")
            .env("LANGUAGE", "ja_JP:ko_KR")
            .args(["init", "--json"]),
    );
    assert_eq!(initialized["plan_language"]["value"], "ja-JP");
    let reinitialized = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .env("LC_ALL", "ko_KR")
            .env("LANGUAGE", "ko_KR")
            .args(["init", "--json"]),
    );
    assert_eq!(reinitialized["plan_language"]["value"], "ja-JP");
    let unset = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .env("LC_ALL", "zh_TW.UTF-8")
            .env("LANGUAGE", "zh_TW")
            .args(["config", "unset", "plan-language", "--json"]),
    );
    assert_eq!(unset["plan_language"]["value"], "en");
    assert_eq!(unset["plan_language"]["source"], "built_in");
    let c_locale = TempDir::new().unwrap();
    let initialized = output_json(
        cargo_bin()
            .current_dir(c_locale.path())
            .env("LC_ALL", "C.UTF-8")
            .env("LANGUAGE", "zh_TW")
            .args(["init", "--json"]),
    );
    assert_eq!(initialized["plan_language"]["value"], "en");
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
    let ignore = fs::read_to_string(temp.path().join(".aitignore")).unwrap();
    assert!(!ignore.is_empty());
    assert!(ignore.lines().all(|line| line.starts_with('#')));
    let agents = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
    assert!(agents.contains("<!-- ait:workflow:start -->"));
    assert!(agents.contains("Route: sprint=`on`; scope=`local`"));
    assert!(agents.contains("plan-binding=`required`"));
    assert!(agents.contains("ait task start --from <sprint-card-path>#<exact-ref> --intent"));
    assert!(agents.contains("Work only in the returned `edit_root`"));
    assert!(agents.contains("otherwise omit it and use the returned `edit_root`"));
    let claude = fs::read_to_string(temp.path().join("CLAUDE.md")).unwrap();
    assert!(claude.contains("<!-- ait:workflow:start -->"));
    assert!(claude.contains("Route: sprint=`on`; scope=`local`"));
    assert!(claude.contains("ait task start --from <sprint-card-path>#<exact-ref> --intent"));
    assert!(claude.contains("--edit-root <absolute-path>"));
    assert!(claude.contains("&& cd <absolute-path>"));

    assert!(claude.contains("Do not omit `--edit-root`"));
    assert!(!claude.contains("@AGENTS.md"));
    for guidance in [&agents, &claude] {
        assert!(guidance.contains("review `.aitignore`\nin `edit_root`"));
        assert!(guidance.contains("first build/test or Snapshot-producing finish"));
    }
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
        fs::read_to_string(temp.path().join(".aitignore")).unwrap(),
        ignore
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("AGENTS.md"))
            .unwrap()
            .matches("<!-- ait:workflow:start -->")
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("CLAUDE.md"))
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
    assert!(fs::symlink_metadata(temp.path().join(".aitignore")).is_err());
}

#[test]
fn init_cli_preserves_empty_ignore_and_does_not_scaffold_on_reinit() {
    let temp = TempDir::new().unwrap();
    let ignore = temp.path().join(".aitignore");
    fs::write(&ignore, b"").unwrap();
    output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["init", "--json"]),
    );
    assert!(fs::read(&ignore).unwrap().is_empty());
    fs::remove_file(&ignore).unwrap();
    let result = output_json(
        cargo_bin()
            .current_dir(temp.path())
            .args(["init", "--json"]),
    );
    assert_eq!(result["action"], "reinitialized");
    assert!(!ignore.exists());
}

#[test]
fn fresh_init_ignore_scaffold_survives_the_first_task_worktree_and_finish() {
    let temp = TempDir::new().unwrap();
    let parent = temp.path().canonicalize().unwrap();
    let root = parent.join("repo");
    let edit_root = parent.join("task");
    fs::create_dir(&root).unwrap();
    output_json(cargo_bin().current_dir(&root).args(["init", "--json"]));
    let ignore = fs::read(root.join(".aitignore")).unwrap();
    output_json(
        cargo_bin()
            .current_dir(&root)
            .args(["config", "set", "--sprint", "off", "--json"]),
    );
    let started = output_json(cargo_bin().current_dir(&root).args([
        "task",
        "start",
        "--title",
        "First code",
        "--intent",
        "Verify init scaffold in the first Task",
        "--edit-root",
        edit_root.to_str().unwrap(),
        "--local",
        "--json",
    ]));
    assert_eq!(fs::read(edit_root.join(".aitignore")).unwrap(), ignore);
    fs::write(edit_root.join("data.csv"), "input,value\nfirst,1\n").unwrap();
    output_json(cargo_bin().current_dir(&edit_root).args([
        "task",
        "finish",
        started["task_id"].as_str().unwrap(),
        "--message",
        "Keep project data with comment-only ignore rules",
        "--local",
        "--json",
    ]));
    assert_eq!(fs::read(root.join(".aitignore")).unwrap(), ignore);
    assert_eq!(
        fs::read_to_string(root.join("data.csv")).unwrap(),
        "input,value\nfirst,1\n"
    );
    assert!(!edit_root.exists());
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
