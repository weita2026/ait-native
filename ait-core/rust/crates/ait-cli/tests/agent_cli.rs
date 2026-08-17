use std::fs;
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::process::Output;

use ait_core::environment_contract::names;
use ait_core::json_support::{JsonCodec, JsonValue};
use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn agent_command(root: &Path) -> Command {
    let mut command = Command::cargo_bin("ait-agent").expect("ait-agent binary");
    isolate_environment(&mut command, root);
    command
}

fn ait_cli_command(root: &Path) -> Command {
    let mut command = Command::cargo_bin("ait-cli").expect("ait-cli binary");
    isolate_environment(&mut command, root);
    command
}

fn isolate_environment(command: &mut Command, root: &Path) {
    for name in [
        names::AIT_AGENT_CONFIG_PATH,
        names::AIT_AGENT_RUST_WORKER_BINARY,
    ] {
        command.env_remove(name);
    }
    command.current_dir(root).env(names::AIT_REPO_ROOT, root);
}

fn run_json(root: &Path, args: &[&str]) -> JsonValue {
    let output = agent_command(root)
        .args(args)
        .output()
        .expect("ait-agent output");
    assert_success(&output, args);
    JsonCodec::parse_slice_with_error_prefix(&output.stdout, "ait-agent JSON")
        .expect("ait-agent JSON")
}

fn assert_success(output: &Output, args: &[&str]) {
    assert!(
        output.status.success(),
        "ait-agent {args:?} failed: status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn fixture_root() -> TempDir {
    tempfile::tempdir().expect("tempdir")
}

#[test]
fn native_agent_help_is_available_from_both_rust_binaries() {
    let root = fixture_root();
    for transport in ["telegram", "line", "discord", "slack"] {
        agent_command(root.path())
            .arg("--help")
            .assert()
            .success()
            .stdout(predicate::str::contains(transport));
        ait_cli_command(root.path())
            .args(["agent", "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains(transport));
    }
    agent_command(root.path())
        .args(["telegram", "logs", "main", "--lines", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "lines must be between 1 and 10000",
        ));
}

#[test]
fn all_transport_crud_is_persisted_by_rust_and_public_output_is_redacted() {
    let root = fixture_root();
    let log_path = root.path().join("telegram.log");
    fs::write(&log_path, "first\nsecond\nthird\n").expect("log fixture");
    let log_path_text = log_path.to_string_lossy().into_owned();
    let fixtures: [(&str, Vec<&str>, &str, &str); 4] = [
        (
            "telegram",
            vec![
                "main",
                "--token",
                "telegram-full-secret",
                "--username",
                "bot",
                "--log-file",
                &log_path_text,
                "--json",
            ],
            "telegram-full-secret",
            "token_set",
        ),
        (
            "line",
            vec![
                "main",
                "--token",
                "line-full-token",
                "--secret",
                "line-full-secret",
                "--json",
            ],
            "line-full-secret",
            "secret_set",
        ),
        (
            "discord",
            vec![
                "main",
                "--application-id",
                "app-123",
                "--bot-token",
                "discord-full-secret",
                "--json",
            ],
            "discord-full-secret",
            "bot_token_set",
        ),
        (
            "slack",
            vec!["main", "--app-token", "slack-full-secret", "--json"],
            "slack-full-secret",
            "app_token_set",
        ),
    ];

    for (transport, add_args, secret, secret_flag) in fixtures {
        let mut args = vec![transport, "add"];
        args.extend(add_args);
        let output = agent_command(root.path())
            .args(&args)
            .output()
            .expect("add output");
        assert_success(&output, &args);
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(!stdout.contains(secret), "secret leaked for {transport}");
        let added =
            JsonCodec::parse_slice_with_error_prefix(&output.stdout, "add JSON").expect("add JSON");
        assert_eq!(added["kind"], transport);
        assert_eq!(added["name"], "main");
        assert_eq!(added[secret_flag], true);

        let listed = run_json(root.path(), &[transport, "list", "--json"]);
        assert_eq!(listed.as_array().map(Vec::len), Some(1));
        assert_eq!(listed[0]["name"], "main");
        assert!(!listed.to_string().contains(secret));

        let status = run_json(root.path(), &[transport, "status", "main", "--json"]);
        assert_eq!(status["running"], false);
        assert_eq!(status["config_valid"], true);
        assert_eq!(status["python_worker_execution_allowed"], false);
        assert!(!status.to_string().contains(secret));
    }

    let logs = run_json(
        root.path(),
        &["telegram", "logs", "main", "--lines", "2", "--json"],
    );
    assert_eq!(
        logs["lines"],
        ait_core::json_support::json!(["second", "third"])
    );
    assert_eq!(logs["lines_requested"], 2);

    let manifest_path = root.path().join(".ait/agent-workers.json");
    let manifest = JsonCodec::parse_slice_with_error_prefix(
        &fs::read(&manifest_path).expect("manifest"),
        "manifest JSON",
    )
    .expect("manifest JSON");
    assert_eq!(
        manifest["workers"]["telegram/main"]["token"],
        "telegram-full-secret"
    );
    assert_eq!(
        manifest["workers"]["line/main"]["secret"],
        "line-full-secret"
    );
    assert_eq!(
        manifest["workers"]["discord/main"]["bot_token"],
        "discord-full-secret"
    );
    assert_eq!(
        manifest["workers"]["slack/main"]["app_token"],
        "slack-full-secret"
    );

    for transport in ["telegram", "line", "discord", "slack"] {
        let removed = run_json(root.path(), &[transport, "remove", "main", "--json"]);
        assert_eq!(removed["removed"], true);
        assert!(run_json(root.path(), &[transport, "list", "--json"])
            .as_array()
            .expect("worker list")
            .is_empty());
    }
}

#[test]
#[cfg(unix)]
fn start_and_supervisor_probe_rust_capabilities_and_never_fall_back_to_python() {
    let root = fixture_root();
    run_json(
        root.path(),
        &[
            "telegram",
            "add",
            "main",
            "--token",
            "telegram-full-secret",
            "--json",
        ],
    );
    let worker = fake_worker_without_transport_support(root.path());
    let sentinel = root.path().join("unexpected-worker-run");

    let output = agent_command(root.path())
        .env(names::AIT_AGENT_RUST_WORKER_BINARY, &worker)
        .env("FAKE_WORKER_RUN_SENTINEL", &sentinel)
        .args(["telegram", "start", "main", "--json"])
        .output()
        .expect("start output");
    assert_success(&output, &["telegram", "start", "main", "--json"]);
    let payload =
        JsonCodec::parse_slice_with_error_prefix(&output.stdout, "start JSON").expect("start JSON");
    assert_eq!(payload["started"], false);
    assert_eq!(payload["start_state"], "missing_rust_transport_runtime");
    assert_eq!(payload["rust_launch_blocked"], true);
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert!(payload["planned_command"]
        .as_array()
        .expect("planned command")
        .iter()
        .all(|part| !part.as_str().unwrap_or_default().contains("python")));
    assert!(!sentinel.exists(), "worker run command must remain blocked");

    let supervisor = agent_command(root.path())
        .env(names::AIT_AGENT_RUST_WORKER_BINARY, &worker)
        .env("FAKE_WORKER_RUN_SENTINEL", &sentinel)
        .args([
            "telegram",
            "supervisor",
            "run",
            "--once",
            "--interval-seconds",
            "1",
            "--json",
        ])
        .output()
        .expect("supervisor output");
    assert_success(
        &supervisor,
        &[
            "telegram",
            "supervisor",
            "run",
            "--once",
            "--interval-seconds",
            "1",
            "--json",
        ],
    );
    let supervisor_payload =
        JsonCodec::parse_slice_with_error_prefix(&supervisor.stdout, "supervisor JSON")
            .expect("supervisor JSON");
    assert_eq!(supervisor_payload["action"], "run");
    assert_eq!(supervisor_payload["cycle"], 1);
    assert_eq!(
        supervisor_payload["workers"][0]["start_state"],
        "missing_rust_transport_runtime"
    );
    assert!(
        !sentinel.exists(),
        "supervisor must not execute unsupported worker"
    );
}

#[test]
fn ait_cli_agent_namespace_dispatches_without_repository_discovery() {
    let root = fixture_root();
    let output = ait_cli_command(root.path())
        .args([
            "agent",
            "slack",
            "add",
            "primary",
            "--app-token",
            "slack-full-secret",
            "--json",
        ])
        .output()
        .expect("ait-cli agent output");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = JsonCodec::parse_slice_with_error_prefix(&output.stdout, "ait-cli agent JSON")
        .expect("ait-cli agent JSON");
    assert_eq!(payload["kind"], "slack");
    assert_eq!(payload["name"], "primary");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("slack-full-secret"));

    agent_command(root.path())
        .args(["slack", "status", "missing", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown slack worker: missing"));
}

#[test]
#[cfg(unix)]
fn telegram_foreground_defaults_to_main_and_preserves_native_worker_exit_status() {
    let root = fixture_root();
    run_json(
        root.path(),
        &[
            "telegram",
            "add",
            "main",
            "--token",
            "telegram-full-secret",
            "--json",
        ],
    );
    let worker = fake_worker_with_telegram_support(root.path());
    let args_path = root.path().join("worker-args");

    agent_command(root.path())
        .env(names::AIT_AGENT_RUST_WORKER_BINARY, &worker)
        .env("FAKE_WORKER_ARGS", &args_path)
        .env("FAKE_WORKER_EXIT_CODE", "23")
        .arg("telegram")
        .assert()
        .code(23);

    let args = fs::read_to_string(&args_path).expect("worker args");
    assert!(args.starts_with("run\n--transport\ntelegram\n--worker\nmain\n"));
    assert!(args.contains("\n--event-loop-backend\n"));
    assert!(args.contains("\n--shard\n0\n"));
    assert!(!args.contains("python"));

    ait_cli_command(root.path())
        .env(names::AIT_AGENT_RUST_WORKER_BINARY, &worker)
        .env("FAKE_WORKER_ARGS", &args_path)
        .env("FAKE_WORKER_EXIT_CODE", "24")
        .args(["agent", "telegram"])
        .assert()
        .code(24);
}

#[test]
#[cfg(unix)]
fn telegram_foreground_accepts_explicit_worker_and_stdin_webhook_mode() {
    let root = fixture_root();
    run_json(
        root.path(),
        &[
            "telegram",
            "add",
            "secondary",
            "--token",
            "telegram-full-secret",
            "--json",
        ],
    );
    let worker = fake_worker_with_telegram_support(root.path());
    let args_path = root.path().join("worker-args");
    let stdin_path = root.path().join("worker-stdin");
    let update = r#"{"update_id":42,"message":{"text":"private-update"}}"#;

    agent_command(root.path())
        .env(names::AIT_AGENT_RUST_WORKER_BINARY, &worker)
        .env("FAKE_WORKER_ARGS", &args_path)
        .env("FAKE_WORKER_STDIN", &stdin_path)
        .args(["telegram", "--worker", "secondary", "--mode", "webhook"])
        .write_stdin(update)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"contract\":\"fake.telegram.webhook.v1\"",
        ));

    let args = fs::read_to_string(args_path).expect("worker args");
    assert!(args.starts_with("run\n--transport\ntelegram\n--worker\nsecondary\n"));
    assert!(args.ends_with("--console-mode\nwebhook\n"));
    assert_eq!(
        fs::read_to_string(stdin_path).expect("worker stdin"),
        update
    );
}

#[test]
fn telegram_foreground_rejects_invalid_modes_and_missing_workers() {
    let root = fixture_root();
    agent_command(root.path())
        .args(["telegram", "--mode", "python"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value 'python'"));
    agent_command(root.path())
        .arg("telegram")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Unknown telegram worker: main"))
        .stderr(predicate::str::contains("Python fallback").not());
}

#[cfg(unix)]
fn fake_worker_without_transport_support(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = root.join("fake-ait-agent-worker");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" = "capabilities" ]; then
  printf '%s\n' '{"contract":"ait.agent.worker.capabilities.v1","supported_transports":[],"python_worker_execution_allowed":false}'
  exit 0
fi
: > "$FAKE_WORKER_RUN_SENTINEL"
exit 97
"#,
    )
    .expect("fake worker");
    let mut permissions = fs::metadata(&path)
        .expect("fake worker metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("fake worker permissions");
    path
}

#[cfg(unix)]
fn fake_worker_with_telegram_support(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let path = root.join("fake-ait-agent-worker-supported");
    fs::write(
        &path,
        r#"#!/bin/sh
if [ "$1" = "capabilities" ]; then
  printf '%s\n' '{"contract":"ait.agent.worker.capabilities.v1","supported_transports":["telegram"],"python_worker_execution_allowed":false}'
  exit 0
fi
printf '%s\n' "$@" > "$FAKE_WORKER_ARGS"
if [ -n "$FAKE_WORKER_STDIN" ]; then
  cat > "$FAKE_WORKER_STDIN"
  printf '%s\n' '{"contract":"fake.telegram.webhook.v1","ok":true}'
fi
exit "${FAKE_WORKER_EXIT_CODE:-0}"
"#,
    )
    .expect("fake worker");
    let mut permissions = fs::metadata(&path)
        .expect("fake worker metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).expect("fake worker permissions");
    path
}
