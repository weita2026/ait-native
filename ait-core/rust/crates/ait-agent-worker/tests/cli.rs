use std::fs;
#[cfg(unix)]
use std::io::{Read, Write};
#[cfg(unix)]
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::sync::mpsc;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use std::time::{SystemTime, UNIX_EPOCH};

use ait_core::json_support::{json, JsonCodec, JsonValue};
use assert_cmd::Command;
#[cfg(unix)]
use hmac::{Hmac, Mac};
use ring::signature::{Ed25519KeyPair, KeyPair};
#[cfg(unix)]
use sha2::Sha256;
use tempfile::tempdir;

const SLACK_SIGNING_SECRET: &str = "cli-slack-signing-secret";
const SLACK_RAW_COMMAND: &str = "team_id=T1&channel_id=C1&user_id=U1&command=%2Fait&text=hello&response_url=https%3A%2F%2Fhooks.slack.test%2Fcli-secret&trigger_id=trig-cli";
const DISCORD_PUBLIC_KEY: &str = "03a107bff3ce10be1d70dd18e74bc09967e4d6309ba50d5f1ddc8664125531b8";
const DISCORD_RAW_PING: &str = r#"{"type":1}"#;

fn worker_command() -> Command {
    let mut command = Command::cargo_bin("ait-agent-worker").expect("worker binary");
    for name in [
        "AIT_REPO_ROOT",
        "AIT_AGENT_CONFIG_PATH",
        "AIT_SLACK_SIGNING_SECRET",
        "SLACK_SIGNING_SECRET",
        "AIT_DISCORD_PUBLIC_KEY",
        "DISCORD_PUBLIC_KEY",
        "AIT_AGENT_CODEX_BIN",
        "AIT_CHAT_CODEX_BIN",
        "AIT_TELEGRAM_MODE",
        "AIT_TELEGRAM_BIND_HOST",
        "AIT_TELEGRAM_BIND_PORT",
        "AIT_TELEGRAM_WEBHOOK_PATH",
    ] {
        command.env_remove(name);
    }
    command
}

fn fixture_repo() -> tempfile::TempDir {
    let temp = tempdir().expect("tempdir");
    fs::create_dir(temp.path().join(".ait")).expect("ait dir");
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        r#"{
  "version": 1,
  "workers": {
    "telegram/main": {
      "kind": "telegram",
      "name": "main",
      "token": "must-not-leak"
    }
  }
}
"#,
    )
    .expect("manifest");
    temp
}

fn slack_fixture_repo(local_reply: Option<&JsonValue>) -> tempfile::TempDir {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    let local_reply = local_reply
        .map(|value| format!(",\n      \"local_reply\": {value}"))
        .unwrap_or_default();
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        format!(
            r#"{{
  "version": 1,
  "workers": {{
    "slack/main": {{
      "kind": "slack",
      "name": "main",
      "signing_secret": "{SLACK_SIGNING_SECRET}",
      "ack_text": "queued by native worker"{local_reply}
    }}
  }}
}}
"#
        ),
    )
    .expect("manifest");
    temp
}

fn discord_fixture_repo() -> tempfile::TempDir {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        format!(
            r#"{{
  "version": 1,
  "workers": {{
    "discord/main": {{
      "kind": "discord",
      "name": "main",
      "application_id": "123456789012345678",
      "public_key": "{DISCORD_PUBLIC_KEY}"
    }}
  }}
}}
"#
        ),
    )
    .expect("manifest");
    temp
}

#[cfg(unix)]
fn current_slack_signature(raw_payload: &str) -> (String, String) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_secs()
        .to_string();
    let mut mac = Hmac::<Sha256>::new_from_slice(SLACK_SIGNING_SECRET.as_bytes()).expect("HMAC");
    mac.update(format!("v0:{timestamp}:{raw_payload}").as_bytes());
    let digest = mac.finalize().into_bytes();
    let signature = format!(
        "v0={}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    (timestamp, signature)
}

fn discord_signature(raw_payload: &str) -> (String, String) {
    let timestamp = "1714990000".to_string();
    let seed = (0_u8..32).collect::<Vec<_>>();
    let key_pair = Ed25519KeyPair::from_seed_unchecked(&seed).expect("Ed25519 test key");
    let public_key = key_pair
        .public_key()
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(public_key, DISCORD_PUBLIC_KEY);
    let signature = key_pair
        .sign(format!("{timestamp}{raw_payload}").as_bytes())
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    (timestamp, signature)
}

#[test]
fn help_surfaces_are_available() {
    worker_command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicates::str::contains("capabilities"))
        .stdout(predicates::str::contains("run"))
        .stdout(predicates::str::contains("slack-command"))
        .stdout(predicates::str::contains("discord-interaction"))
        .stdout(predicates::str::contains("reply-provider"));
    worker_command()
        .args(["run", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--event-loop-backend"))
        .stdout(predicates::str::contains("--shard"));
    worker_command()
        .args(["slack-command", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--worker"));
    worker_command()
        .args(["discord-interaction", "--help"])
        .assert()
        .success()
        .stdout(predicates::str::contains("--worker"));
}

#[test]
fn reply_provider_returns_a_versioned_failure_for_malformed_input() {
    let output = worker_command()
        .arg("reply-provider")
        .write_stdin("not-json")
        .output()
        .expect("reply provider output");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let payload =
        JsonCodec::parse_slice_with_error_prefix(&output.stdout, "native reply provider response")
            .expect("reply provider JSON");
    assert_eq!(
        payload["contract"],
        "ait.agent.gateway_reply_provider_response.v1"
    );
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["error"]["kind"], "provider_request_invalid");
}

#[test]
#[cfg(unix)]
fn reply_provider_executes_a_sessionless_gateway_turn_with_command_telemetry() {
    let repo = fixture_repo();
    let codex = repo.path().join("codex-fixture");
    fs::write(
        &codex,
        concat!(
            "#!/bin/sh\n",
            "cat >/dev/null\n",
            "printf '%s\\n' ",
            "'{\"type\":\"thread.started\",\"thread_id\":\"019c-gateway-thread\"}' ",
            "'{\"type\":\"turn.started\"}' ",
            "'{\"type\":\"item.started\",\"item\":{\"id\":\"command-1\",\"type\":\"command_execution\",\"command\":\"ait task audit RCT-1\",\"status\":\"in_progress\"}}' ",
            "'{\"type\":\"item.completed\",\"item\":{\"id\":\"command-1\",\"type\":\"command_execution\",\"command\":\"ait task audit RCT-1\",\"exit_code\":0,\"status\":\"completed\"}}' ",
            "'{\"type\":\"item.completed\",\"item\":{\"id\":\"message-1\",\"type\":\"agent_message\",\"text\":\"gateway reply\"}}' ",
            "'{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":5,\"output_tokens\":2}}'\n",
        ),
    )
    .expect("fake Codex executable");
    let mut permissions = fs::metadata(&codex)
        .expect("fake Codex metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&codex, permissions).expect("fake Codex permissions");
    let request = json!({
        "contract": "ait.agent.gateway_reply_provider_request.v1",
        "repository": {
            "repo_root": repo.path().to_string_lossy(),
            "repo_name": "fixture",
        },
        "surface": {"name": "telegram"},
        "conversation": {"key": "telegram:fixture-chat"},
        "provider_thread": null,
        "input": {"text": "hello"},
        "settings": {"codex_program": codex.to_string_lossy()},
    });

    let output = worker_command()
        .current_dir(repo.path())
        .arg("reply-provider")
        .write_stdin(request.to_string())
        .output()
        .expect("gateway reply provider output");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let payload =
        JsonCodec::parse_slice_with_error_prefix(&output.stdout, "gateway provider response")
            .expect("gateway provider JSON");
    assert_eq!(
        payload["contract"],
        "ait.agent.gateway_reply_provider_response.v1"
    );
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["reply"]["text"], "gateway reply");
    assert_eq!(payload["reply"]["turn_telemetry"]["command_count"], 1);
    assert_eq!(payload["reply"]["turn_telemetry"]["ait_command_count"], 1);
    assert_eq!(
        payload["reply"]["turn_telemetry"]["ait_commands"][0]["command_path"],
        "task audit"
    );
    assert_eq!(
        payload["reply"]["turn_analysis"]["provider_thread"]["conversation_key"],
        "telegram:fixture-chat"
    );
    assert_eq!(payload["reply"]["turn_analysis"]["thread_mode"], "started");
    assert!(payload["reply"]["turn_analysis"]
        .get("session_mode")
        .is_none());
    assert!(payload["reply"]["turn_analysis"]["provider_thread"]
        .get("session_id")
        .is_none());
}

#[test]
fn discord_interaction_outputs_raw_signed_ping_response_without_python_wrapper() {
    let repo = discord_fixture_repo();
    let (timestamp, signature) = discord_signature(DISCORD_RAW_PING);
    let output = worker_command()
        .current_dir(repo.path())
        .args([
            "discord-interaction",
            "--worker",
            "main",
            "--signature",
            &signature,
            "--signature-timestamp",
            &timestamp,
        ])
        .write_stdin(DISCORD_RAW_PING)
        .output()
        .expect("Discord interaction output");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let payload = JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Discord response JSON")
        .expect("Discord response JSON");
    assert_eq!(payload, json!({"type": 1}));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(DISCORD_PUBLIC_KEY));
    assert!(!stdout.contains("python"));
}

#[test]
fn discord_interaction_rejects_invalid_signature_without_stdout_or_secret_leak() {
    let repo = discord_fixture_repo();
    let output = worker_command()
        .current_dir(repo.path())
        .args([
            "discord-interaction",
            "--worker",
            "main",
            "--signature",
            "invalid",
            "--signature-timestamp",
            "1714990000",
        ])
        .write_stdin(DISCORD_RAW_PING)
        .output()
        .expect("Discord interaction rejection");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let payload = JsonCodec::parse_slice_with_error_prefix(&output.stderr, "Discord error JSON")
        .expect("Discord error JSON");
    assert_eq!(payload["code"], "discord_interaction_signature_invalid");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains(DISCORD_PUBLIC_KEY));
    assert!(!stderr.contains(DISCORD_RAW_PING));
}

#[test]
#[cfg(unix)]
fn slack_command_executes_a_native_local_turn_through_the_external_provider_seam() {
    let (response_url, request_rx, server) = serve_slack_response_once();
    let encoded_response_url = response_url.replace(':', "%3A").replace('/', "%2F");
    let raw_command = format!(
        "team_id=T1&channel_id=C1&user_id=U1&command=%2Fait&text=hello&response_url={encoded_response_url}&trigger_id=trig-cli-local"
    );
    let (timestamp, signature) = current_slack_signature(&raw_command);
    let provider_response = json!({
        "contract": "ait.agent.gateway_reply_provider_response.v1",
        "ok": true,
        "reply": {
            "text": "native local reply",
            "model": "fixture-provider",
            "usage": {"input_tokens": 2, "output_tokens": 3},
        },
    });
    let local_reply = json!({
        "program": "/bin/echo",
        "args": [provider_response.to_string()],
        "timeout_seconds": 2,
    });
    let repo = slack_fixture_repo(Some(&local_reply));
    let output = worker_command()
        .current_dir(repo.path())
        .args([
            "slack-command",
            "--worker",
            "main",
            "--signature",
            &signature,
            "--signature-timestamp",
            &timestamp,
        ])
        .write_stdin(raw_command.clone())
        .output()
        .expect("Slack command output");

    let delivered_request = request_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("Slack response URL request");
    server.join().expect("Slack response URL server");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let payload = JsonCodec::parse_slice_with_error_prefix(&output.stdout, "Slack command JSON")
        .expect("Slack command JSON");
    assert_eq!(
        payload["contract"],
        "ait.agent.worker.slack-command-once.v1"
    );
    assert_eq!(payload["response"]["text"], "queued by native worker");
    assert_eq!(
        payload["command_job"]["command_job_state"], "processed",
        "{payload}\n{delivered_request}"
    );
    assert_eq!(payload["command_job"]["turn_ok"], true);
    assert_eq!(payload["command_job"]["delivered"], true);
    assert_eq!(payload["command_job"]["recorded"], true);
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert!(delivered_request.starts_with("POST /reply HTTP/1.1"));
    assert!(delivered_request.contains("native local reply"));
    assert!(repo
        .path()
        .join(".ait/agent-runtime/slack-main-sync.json")
        .is_file());
    let stdout = String::from_utf8_lossy(&output.stdout);
    for forbidden in [
        SLACK_SIGNING_SECRET,
        response_url.as_str(),
        raw_command.as_str(),
    ] {
        assert!(!stdout.contains(forbidden));
    }
}

#[cfg(unix)]
fn serve_slack_response_once() -> (String, mpsc::Receiver<String>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("Slack response server");
    let url = format!("http://{}/reply", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("Slack response connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .expect("Slack response read timeout");
        let request = read_http_request(&mut stream);
        sender
            .send(request)
            .expect("capture Slack response request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
            )
            .expect("write Slack response");
    });
    (url, receiver, handle)
}

#[cfg(unix)]
fn read_http_request(stream: &mut std::net::TcpStream) -> String {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 1024];
    while let Ok(read) = stream.read(&mut chunk) {
        if read == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..read]);
        if http_request_complete(&buffer) {
            break;
        }
    }
    String::from_utf8_lossy(&buffer).to_string()
}

#[cfg(unix)]
fn http_request_complete(buffer: &[u8]) -> bool {
    let Some(header_end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
        return false;
    };
    let headers = String::from_utf8_lossy(&buffer[..header_end]).to_ascii_lowercase();
    let content_length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    buffer.len() >= header_end + 4 + content_length
}

#[test]
fn slack_command_rejects_invalid_signature_without_stdout_or_secret_leak() {
    let repo = slack_fixture_repo(None);
    let output = worker_command()
        .current_dir(repo.path())
        .args([
            "slack-command",
            "--worker",
            "main",
            "--signature",
            "v0=invalid",
            "--signature-timestamp",
            "1714990000",
        ])
        .write_stdin(SLACK_RAW_COMMAND)
        .output()
        .expect("Slack command rejection");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let payload = JsonCodec::parse_slice_with_error_prefix(&output.stderr, "Slack error JSON")
        .expect("Slack error JSON");
    assert_eq!(payload["code"], "slack_command_signature_invalid");
    let stderr = String::from_utf8_lossy(&output.stderr);
    for forbidden in [SLACK_SIGNING_SECRET, "hooks.slack.test", SLACK_RAW_COMMAND] {
        assert!(!stderr.contains(forbidden));
    }
}

#[test]
fn capabilities_are_reported_by_the_binary_without_environment_allowlist() {
    let output = worker_command()
        .args(["capabilities", "--json"])
        .output()
        .expect("capability output");

    assert!(output.status.success());
    let payload = JsonCodec::parse_slice_with_error_prefix(&output.stdout, "capability JSON")
        .expect("capability JSON");
    assert_eq!(payload["contract"], "ait.agent.worker.capabilities.v1");
    assert_eq!(
        payload["supported_transports"],
        JsonValue::Array(vec![
            JsonValue::String("telegram".to_string()),
            JsonValue::String("discord".to_string()),
            JsonValue::String("slack".to_string()),
            JsonValue::String("line".to_string()),
        ])
    );
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert_eq!(
        payload["transport_capabilities"].as_array().map(Vec::len),
        Some(4)
    );
}

#[test]
fn direct_worker_run_argv_loads_the_manifest_backed_worker() {
    let repo = fixture_repo();
    fs::write(
        repo.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    fs::write(
        repo.path().join(".ait/agent-workers.json"),
        r#"{"version":1,"workers":{"telegram/main":{"kind":"telegram","name":"main","token":"must-not-leak","mode":"webhook","bind_port":70000}}}"#,
    )
    .expect("worker manifest");
    let output = worker_command()
        .current_dir(repo.path())
        .args([
            "run",
            "--transport",
            "telegram",
            "--worker",
            "main",
            "--event-loop-backend",
            "portable_poll",
            "--shard",
            "0",
        ])
        .output()
        .expect("worker output");

    assert_eq!(output.status.code(), Some(3));
    let payload =
        JsonCodec::parse_slice_with_error_prefix(&output.stderr, "error JSON").expect("error JSON");
    assert_eq!(payload["code"], "telegram_worker_bind_port_invalid");
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("must-not-leak"));
}

#[test]
fn telegram_stdin_webhook_executes_from_manifest_backed_worker_argv() {
    let repo = fixture_repo();
    fs::write(
        repo.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    let output = worker_command()
        .current_dir(repo.path())
        .args([
            "run",
            "--transport",
            "telegram",
            "--worker",
            "main",
            "--event-loop-backend",
            "portable_poll",
            "--shard",
            "0",
            "--console-mode",
            "webhook",
        ])
        .write_stdin(r#"{"update_id":42}"#)
        .output()
        .expect("worker output");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let payload = JsonCodec::parse_slice_with_error_prefix(&output.stdout, "webhook JSON")
        .expect("webhook JSON");
    assert_eq!(
        payload["contract"],
        "ait.agent.worker.telegram_webhook_once.v1"
    );
    assert_eq!(payload["worker"], "main");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["response"]["processed_updates"], 1);
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("must-not-leak"));
}

#[test]
fn telegram_stdin_webhook_rejects_malformed_input_secret_safely() {
    let repo = fixture_repo();
    fs::write(
        repo.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    let raw = "{private-parser-secret";

    let output = worker_command()
        .current_dir(repo.path())
        .args([
            "run",
            "--transport",
            "telegram",
            "--worker",
            "main",
            "--event-loop-backend",
            "portable_poll",
            "--shard",
            "0",
            "--console-mode",
            "webhook",
        ])
        .write_stdin(raw)
        .output()
        .expect("worker output");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let payload =
        JsonCodec::parse_slice_with_error_prefix(&output.stderr, "error JSON").expect("error JSON");
    assert_eq!(payload["code"], "telegram_webhook_input_invalid");
    assert_eq!(payload["python_worker_execution_allowed"], false);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains(raw));
    assert!(!stderr.contains("must-not-leak"));
}

#[test]
fn unknown_transport_worker_backend_and_shard_fail_before_runner_execution() {
    let repo = fixture_repo();
    let cases = [
        (
            ["unknown", "main", "portable_poll", "0"],
            "unknown_transport",
        ),
        (
            ["telegram", "missing", "portable_poll", "0"],
            "unknown_worker",
        ),
        (
            ["telegram", "main", "not-a-backend", "0"],
            "unknown_event_loop_backend",
        ),
        (
            ["telegram", "main", "portable_poll", "bad"],
            "invalid_shard_index",
        ),
        (
            ["telegram", "main", "portable_poll", "1"],
            "invalid_shard_assignment",
        ),
    ];

    for (values, expected_code) in cases {
        let output = worker_command()
            .current_dir(repo.path())
            .args([
                "run",
                "--transport",
                values[0],
                "--worker",
                values[1],
                "--event-loop-backend",
                values[2],
                "--shard",
                values[3],
            ])
            .output()
            .expect("worker output");
        assert!(!output.status.success());
        let payload = JsonCodec::parse_slice_with_error_prefix(&output.stderr, "error JSON")
            .expect("error JSON");
        assert_eq!(payload["code"], expected_code);
        assert_eq!(payload["python_worker_execution_allowed"], false);
    }
}
