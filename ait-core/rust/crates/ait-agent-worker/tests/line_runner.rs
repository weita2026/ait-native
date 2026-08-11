use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ait_agent_core::AgentWorkerRuntimeConfig;
use ait_agent_worker::{
    prepare_worker_run_with_env, AgentEventLoopHostWait, DefaultLineHttpTransactionExecutor,
    LineWorkerHttpHandler, WorkerHostEventLoop, WorkerHostRuntime, WorkerHttpHostConfig,
    WorkerHttpHostRuntime, WorkerPathInputs, WorkerRunContext, WorkerRunRequest,
};
use tempfile::tempdir;

#[path = "../../../test_support.rs"]
mod workspace_test_support;

fn agent_worker_binary() -> std::path::PathBuf {
    workspace_test_support::cargo_binary(
        "ait-agent-worker",
        option_env!("CARGO_BIN_EXE_ait-agent-worker"),
    )
}

fn fixture_context() -> (tempfile::TempDir, WorkerRunContext) {
    let temp = tempdir().expect("tempdir");
    fs::create_dir_all(temp.path().join(".ait/agent-runtime")).expect("runtime dir");
    fs::write(
        temp.path().join(".ait/config.json"),
        r#"{"repo_name":"fixture","workflow_mode":"solo_local"}"#,
    )
    .expect("repo config");
    fs::write(
        temp.path().join(".ait/agent-workers.json"),
        r#"{"version":1,"workers":{"line/main":{"kind":"line","name":"main","secret":"manifest-line-secret","token":"manifest-line-token"}}}"#,
    )
    .expect("manifest");
    let context = prepare_worker_run_with_env(
        &WorkerRunRequest {
            transport: "line".to_string(),
            worker: "main".to_string(),
            event_loop_backend: "portable_poll".to_string(),
            shard: "0".to_string(),
        },
        &WorkerPathInputs {
            current_dir: temp.path().to_path_buf(),
            repo_root_override: Some(temp.path().to_path_buf()),
            manifest_path_override: None,
        },
        BTreeMap::new(),
    )
    .expect("worker context");
    (temp, context)
}

fn line_runtime(
    context: &WorkerRunContext,
) -> WorkerHttpHostRuntime<LineWorkerHttpHandler<DefaultLineHttpTransactionExecutor>> {
    let AgentWorkerRuntimeConfig::Line(config) = &context.config else {
        panic!("LINE config");
    };
    let handler =
        LineWorkerHttpHandler::new(config, DefaultLineHttpTransactionExecutor, 2).unwrap();
    WorkerHttpHostRuntime::new(
        WorkerHttpHostConfig {
            expected_path: config.webhook_path.clone(),
            enforce_expected_path: false,
            request_timeout: Duration::from_secs(2),
            ..WorkerHttpHostConfig::default()
        },
        handler,
    )
}

fn exchange(
    context: &WorkerRunContext,
    runtime: &mut WorkerHttpHostRuntime<LineWorkerHttpHandler<DefaultLineHttpTransactionExecutor>>,
    event_loop: &mut dyn WorkerHostEventLoop,
    path: &str,
    signature: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut client =
        TcpStream::connect(runtime.local_addr().expect("listener address")).expect("client");
    write!(
        client,
        "POST {path} HTTP/1.1\r\nHost: localhost\r\nX-Line-Signature: {signature}\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .expect("request head");
    client.write_all(body).expect("request body");
    client.set_nonblocking(true).expect("nonblocking client");

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut response = Vec::new();
    let mut chunk = [0u8; 4_096];
    loop {
        let events = event_loop.wait(Duration::from_millis(5)).expect("poll");
        runtime
            .tick(context, event_loop, &events)
            .expect("LINE host tick");
        loop {
            match client.read(&mut chunk) {
                Ok(0) => return response,
                Ok(read) => response.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => panic!("client read failed: {error}"),
            }
        }
        assert!(Instant::now() < deadline, "LINE response timed out");
    }
}

fn assert_status(response: &[u8], status: u16) {
    assert!(
        response.starts_with(format!("HTTP/1.1 {status} ").as_bytes()),
        "{}",
        String::from_utf8_lossy(response)
    );
}

#[test]
fn production_line_handler_verifies_signature_and_preserves_transaction_http_contract() {
    let (_temp, context) = fixture_context();
    let mut runtime = line_runtime(&context);
    let mut event_loop = AgentEventLoopHostWait::new(&context).expect("event loop");
    runtime
        .start(&context, &mut event_loop)
        .expect("start LINE host");
    let body = br#"{"events":[]}"#;
    let signature = "8+J6Sajy0Fq5kxWJ8zotF99NBMLRY7ytuaULzF50Veg=";

    let success = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/callback",
        signature,
        body,
    );
    assert_status(&success, 200);
    assert!(success.ends_with(br#"{"ok":true,"processed_events":0}"#));

    let rejected_path = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/wrong",
        signature,
        body,
    );
    assert_status(&rejected_path, 404);
    assert!(rejected_path
        .windows(b"Content-Length: 0".len())
        .any(|value| value == b"Content-Length: 0"));
    assert!(rejected_path.ends_with(b"\r\n\r\n"));

    let bad_signature = exchange(
        &context,
        &mut runtime,
        &mut event_loop,
        "/callback",
        "bad-signature",
        body,
    );
    assert_status(&bad_signature, 401);
    let public = String::from_utf8_lossy(&bad_signature);
    assert!(!public.contains("manifest-line-secret"));
    assert!(!public.contains("manifest-line-token"));
    assert!(!public.contains("bad-signature"));

    runtime
        .request_shutdown(&context, &mut event_loop, libc::SIGTERM)
        .expect("stop LINE host");
    assert_eq!(runtime.inflight_work_count(), 0);
    runtime
        .finish_shutdown(&context, &mut event_loop)
        .expect("finish LINE host");
}

#[test]
fn compiled_line_runner_reaches_ready_and_stops_cleanly_cross_platform() {
    let (repo, _context) = fixture_context();
    let reserved = TcpListener::bind(("127.0.0.1", 0)).expect("reserve port");
    let port = reserved.local_addr().expect("reserved address").port();
    drop(reserved);
    let mut child = Command::new(agent_worker_binary())
        .current_dir(repo.path())
        .env("AIT_REPO_ROOT", repo.path())
        .env("AIT_LINE_BIND_HOST", "127.0.0.1")
        .env("AIT_LINE_BIND_PORT", port.to_string())
        .args([
            "run",
            "--transport",
            "line",
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
        .expect("spawn LINE worker");

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut probe = loop {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(stream) => break stream,
            Err(_) => {
                assert!(
                    child.try_wait().expect("poll child").is_none(),
                    "LINE worker exited before readiness"
                );
                assert!(Instant::now() < deadline, "LINE worker did not bind");
                thread::sleep(Duration::from_millis(10));
            }
        }
    };
    probe
        .write_all(b"POST /probe HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
        .expect("probe request");
    probe
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("probe timeout");
    let mut response = Vec::new();
    probe.read_to_end(&mut response).expect("probe response");
    assert_status(&response, 404);
    workspace_test_support::request_worker_shutdown(repo.path(), "line", "main", child.id());
    let status = workspace_test_support::wait_for_child_exit(
        &mut child,
        "LINE worker",
        Duration::from_secs(5),
    );
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("child stderr")
        .read_to_string(&mut stderr)
        .expect("read child stderr");

    assert!(status.success(), "{stderr}");
    for state in ["ready", "stopping", "stopped"] {
        assert!(
            stderr.contains(&format!("\"state\":\"{state}\"")),
            "{stderr}"
        );
    }
    assert!(stderr.contains("\"transport\":\"line\""));
    assert!(stderr.contains("\"python_worker_execution_allowed\":false"));
    assert!(!stderr.contains("manifest-line-secret"));
    assert!(!stderr.contains("manifest-line-token"));
}
