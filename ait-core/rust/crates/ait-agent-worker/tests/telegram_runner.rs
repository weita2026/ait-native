use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use tempfile::tempdir;

#[path = "../../../test_support.rs"]
mod workspace_test_support;

const TELEGRAM_TOKEN: &str = "123456:compiled-telegram-secret";
const WEBHOOK_SECRET: &str = "compiled-webhook-secret";

fn agent_worker_binary() -> std::path::PathBuf {
    workspace_test_support::cargo_binary(
        "ait-agent-worker",
        option_env!("CARGO_BIN_EXE_ait-agent-worker"),
    )
}

fn fixture_repo() -> tempfile::TempDir {
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
            r#"{{"version":1,"workers":{{"telegram/main":{{"kind":"telegram","name":"main","token":"{TELEGRAM_TOKEN}","username":"ait_bot"}}}}}}"#
        ),
    )
    .expect("worker manifest");
    temp
}

fn assert_status(response: &[u8], status: u16) {
    assert!(
        response.starts_with(format!("HTTP/1.1 {status} ").as_bytes()),
        "{}",
        String::from_utf8_lossy(response)
    );
}

#[test]
fn compiled_telegram_webhook_runner_processes_loopback_update_and_stops_cross_platform() {
    let repo = fixture_repo();
    let reserved = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
    let port = reserved.local_addr().expect("reserved address").port();
    drop(reserved);

    let mut child = Command::new(agent_worker_binary())
        .current_dir(repo.path())
        .env("AIT_REPO_ROOT", repo.path())
        .env("AIT_TELEGRAM_MODE", "webhook")
        .env("AIT_TELEGRAM_BIND_HOST", "127.0.0.1")
        .env("AIT_TELEGRAM_BIND_PORT", port.to_string())
        .env("AIT_TELEGRAM_WEBHOOK_PATH", "/telegram")
        .env("AIT_TELEGRAM_WEBHOOK_SECRET", WEBHOOK_SECRET)
        .env("AIT_TELEGRAM_BACKGROUND_SYNC_ENABLED", "false")
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Telegram worker");

    let ready_deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(_) => break,
            Err(_) => {
                assert!(
                    child.try_wait().expect("poll child").is_none(),
                    "Telegram worker exited before readiness"
                );
                assert!(
                    Instant::now() < ready_deadline,
                    "Telegram worker did not bind"
                );
                thread::sleep(Duration::from_millis(10));
            }
        }
    }

    let payload = br#"{"update_id":1}"#;
    let mut client = TcpStream::connect(("127.0.0.1", port)).expect("webhook client");
    write!(
        client,
        "POST /telegram HTTP/1.1\r\nHost: localhost\r\nX-Telegram-Bot-Api-Secret-Token: {WEBHOOK_SECRET}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        payload.len()
    )
    .expect("webhook request head");
    client.write_all(payload).expect("webhook request body");
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("response timeout");
    let mut response = Vec::new();
    client.read_to_end(&mut response).expect("webhook response");
    assert_status(&response, 200);
    assert!(response.ends_with(br#"{"ok":true,"processed_updates":1}"#));

    workspace_test_support::request_worker_shutdown(repo.path(), "telegram", "main", child.id());
    let status = workspace_test_support::wait_for_child_exit(
        &mut child,
        "Telegram worker",
        Duration::from_secs(5),
    );
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_string(&mut stderr)
        .expect("read child stderr");
    let mut stdout = String::new();
    child
        .stdout
        .take()
        .expect("child stdout")
        .read_to_string(&mut stdout)
        .expect("read child stdout");

    assert!(status.success(), "{stderr}");
    assert!(stdout.is_empty(), "{stdout}");
    for state in ["ready", "stopping", "stopped"] {
        assert!(
            stderr.contains(&format!("\"state\":\"{state}\"")),
            "{stderr}"
        );
    }
    assert!(stderr.contains("\"transport\":\"telegram\""));
    assert!(stderr.contains("\"python_worker_execution_allowed\":false"));
    for forbidden in [TELEGRAM_TOKEN, WEBHOOK_SECRET, "python3", "ait_agent."] {
        assert!(!stderr.contains(forbidden), "{stderr}");
    }
}
