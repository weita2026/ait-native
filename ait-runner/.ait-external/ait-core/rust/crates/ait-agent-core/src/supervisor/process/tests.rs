use super::*;
use ait_core::json_support::json;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "ait-agent-supervisor-process-{label}-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn test_paths(root: &Path) -> AgentWorkerProcessPaths {
    AgentWorkerProcessPaths {
        pid_file: root.join("worker.pid").to_string_lossy().into_owned(),
        log_file: root.join("worker.log").to_string_lossy().into_owned(),
        sync_state_path: root.join("sync.json").to_string_lossy().into_owned(),
        env_path: root.join("worker.env").to_string_lossy().into_owned(),
        termination_context_path: root.join("termination.json").to_string_lossy().into_owned(),
    }
}

#[cfg(unix)]
fn wait_for_file_containing(path: &Path, needle: &str) -> String {
    for _ in 0..100 {
        if let Ok(body) = fs::read_to_string(path) {
            if body.contains(needle) {
                return body;
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "timed out waiting for {} to contain {needle:?}",
        path.display()
    );
}

#[test]
fn process_status_reports_pid_file_and_runtime_health() {
    let root = temp_dir("status");
    let pid_file = root.join("worker.pid");
    let log_file = root.join("worker.log");
    let sync_state_path = root.join("sync.json");
    let env_path = root.join("worker.env");
    let termination_context_path = root.join("termination.json");
    fs::write(&pid_file, format!("{}\n", std::process::id())).unwrap();
    fs::write(&log_file, "boot\n").unwrap();
    fs::write(&sync_state_path, "{}\n").unwrap();
    fs::write(&env_path, "KEY=value\n").unwrap();

    let status = inspect_worker_process_status(AgentWorkerProcessStatusInput {
        paths: AgentWorkerProcessPaths {
            pid_file: pid_file.to_string_lossy().into_owned(),
            log_file: log_file.to_string_lossy().into_owned(),
            sync_state_path: sync_state_path.to_string_lossy().into_owned(),
            env_path: env_path.to_string_lossy().into_owned(),
            termination_context_path: termination_context_path.to_string_lossy().into_owned(),
        },
    });

    assert!(status.running);
    assert_eq!(status.pid, Some(std::process::id() as i64));
    assert_eq!(status.health.pid_file_state, "running");
    assert!(status.health.pid_file_exists);
    assert!(status.health.pid_file_readable);
    assert!(status.health.pid_file_valid);
    assert!(status.health.log_exists);
    assert_eq!(status.health.log_size_bytes, 5);
    assert!(status.health.sync_state_exists);
    assert!(status.health.env_exists);
    assert!(!status.health.termination_context_exists);
    assert!(!status.python_worker_execution_allowed);
}

#[test]
fn process_status_reports_invalid_pid_file() {
    let root = temp_dir("invalid");
    let pid_file = root.join("worker.pid");
    fs::write(&pid_file, "not-a-pid\n").unwrap();

    let status = inspect_worker_process_status(AgentWorkerProcessStatusInput {
        paths: AgentWorkerProcessPaths {
            pid_file: pid_file.to_string_lossy().into_owned(),
            log_file: root.join("worker.log").to_string_lossy().into_owned(),
            sync_state_path: root.join("sync.json").to_string_lossy().into_owned(),
            env_path: root.join("worker.env").to_string_lossy().into_owned(),
            termination_context_path: root.join("termination.json").to_string_lossy().into_owned(),
        },
    });

    assert!(!status.running);
    assert_eq!(status.pid, None);
    assert_eq!(status.health.pid_file_state, "invalid");
    assert!(status.health.pid_file_exists);
    assert!(status.health.pid_file_readable);
    assert!(!status.health.pid_file_valid);
}

#[test]
#[cfg(target_os = "linux")]
fn linux_proc_state_parser_handles_spaces_and_parentheses_in_process_names() {
    assert_eq!(
        linux_proc_process_state("42 (worker name (sidecar)) S 1 2 3"),
        Some("S")
    );
    assert_eq!(linux_proc_process_state("42 malformed"), None);
}

#[test]
fn log_tail_returns_bounded_tail_lines_and_missing_log_metadata() {
    let root = temp_dir("logs");
    let log_file = root.join("worker.log");
    fs::write(&log_file, "line 1\nline 2\nline 3\nline 4\n").unwrap();

    let tail = read_worker_log_tail(AgentWorkerLogTailInput {
        log_file: log_file.to_string_lossy().into_owned(),
        lines: Some(2),
    });

    assert_eq!(tail.lines, vec!["line 3", "line 4"]);
    assert!(tail.log_exists);
    assert_eq!(tail.lines_requested, 2);
    assert!(!tail.python_worker_execution_allowed);

    let missing = read_worker_log_tail(AgentWorkerLogTailInput {
        log_file: root.join("missing.log").to_string_lossy().into_owned(),
        lines: Some(2),
    });
    assert!(missing.lines.is_empty());
    assert!(!missing.log_exists);
    assert_eq!(missing.lines_requested, 2);
}

#[test]
fn process_status_json_matches_python_contract_keys() {
    let root = temp_dir("json");
    let pid_file = root.join("worker.pid");
    fs::write(&pid_file, "0\n").unwrap();
    let payload = inspect_worker_process_status_json(&json!({
        "paths": {
            "pid_file": pid_file,
            "log_file": root.join("worker.log"),
            "sync_state_path": root.join("sync.json"),
            "env_path": root.join("worker.env"),
            "termination_context_path": root.join("termination.json")
        }
    }))
    .unwrap();

    assert_eq!(payload["running"], false);
    assert_eq!(payload["pid"], JsonValue::Null);
    assert_eq!(payload["health"]["pid_file_state"], "invalid");
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert_eq!(payload["migration_stage"], MIGRATION_STAGE);
}

#[test]
#[cfg(unix)]
fn start_contract_uses_typed_manifest_environment_and_writes_pid_file() {
    let root = temp_dir("start-telegram");
    let capture_env_path = root.join("captured.env");
    let termination_context_path = root.join("termination.json");
    fs::write(&termination_context_path, "{}\n").unwrap();
    let mut parent_env = BTreeMap::new();
    parent_env.insert(
        "AIT_CAPTURE_ENV".to_string(),
        capture_env_path.to_string_lossy().into_owned(),
    );
    parent_env.insert("PATH".to_string(), "/usr/bin:/bin".to_string());

    let result = start_worker_process(AgentWorkerStartInput {
        repo_root: root.to_string_lossy().into_owned(),
        paths: test_paths(&root),
        worker: AgentWorkerStartSpec {
            transport: TransportKind::Telegram,
            name: "main".to_string(),
            token: Some("123456:secret-token".to_string()),
            username: Some("ait_main_bot".to_string()),
            secret: None,
            app_token: None,
            bot_token: None,
            application_id: None,
        },
        argv: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "/usr/bin/env > \"$AIT_CAPTURE_ENV\"; exec /bin/sleep 30".to_string(),
        ],
        parent_env,
    })
    .unwrap();

    assert!(result.started);
    assert_eq!(result.start_state, "started");
    assert!(result.running);
    assert!(result.pid.is_some());
    assert_eq!(result.command[0], "/bin/sh");
    assert!(!result.env_file_seeded);
    assert!(result.termination_context_removed);
    assert!(result.pid_file_written);
    assert!(!PathBuf::from(&test_paths(&root).termination_context_path).exists());
    assert!(!result.python_worker_execution_allowed);

    let captured_env = wait_for_file_containing(&capture_env_path, "AIT_REPO_ROOT=");
    assert!(captured_env.contains("AIT_REPO_ROOT="));
    assert!(!captured_env.contains("123456:secret-token"));
    assert!(!root.join("worker.env").exists());

    let stop = stop_worker_process(AgentWorkerStopInput {
        paths: test_paths(&root),
        reason: Some("test_cleanup".to_string()),
        worker_name: Some("main".to_string()),
        stop_timeout_seconds: Some(2.0),
        kill_grace_seconds: Some(0.2),
    })
    .unwrap();
    assert!(stop.stopped);
}

#[test]
fn worker_start_env_preserves_parent_and_binds_only_the_current_repo_root() {
    let root = temp_dir("provider-env-defaults");
    let parent_env = BTreeMap::from([("PARENT_KEEP".to_string(), "value".to_string())]);
    let env = build_worker_start_env(&root, parent_env);

    assert_eq!(env["PARENT_KEEP"], "value");
    assert_eq!(env[names::AIT_REPO_ROOT], root.to_string_lossy());
    assert_eq!(env.len(), 2);
}

#[test]
fn start_contract_returns_already_running_without_spawn() {
    let root = temp_dir("start-running");
    let paths = test_paths(&root);
    fs::write(&paths.pid_file, format!("{}\n", std::process::id())).unwrap();

    let result = start_worker_process(AgentWorkerStartInput {
        repo_root: root.to_string_lossy().into_owned(),
        paths,
        worker: AgentWorkerStartSpec {
            transport: TransportKind::Slack,
            name: "sidecar".to_string(),
            token: None,
            username: None,
            secret: None,
            app_token: Some("xapp-token".to_string()),
            bot_token: None,
            application_id: None,
        },
        argv: vec!["/bin/does-not-run".to_string()],
        parent_env: BTreeMap::new(),
    })
    .unwrap();

    assert!(!result.started);
    assert_eq!(result.start_state, "already_running");
    assert!(result.running);
    assert_eq!(result.pid, Some(std::process::id() as i64));
    assert!(!result.pid_file_written);
    assert!(!result.python_worker_execution_allowed);
}

#[test]
fn start_contract_leaves_transport_validation_to_the_typed_worker_manifest() {
    let root = temp_dir("start-missing-env");
    let paths = test_paths(&root);
    let result = start_worker_process(AgentWorkerStartInput {
        repo_root: root.to_string_lossy().into_owned(),
        paths: paths.clone(),
        worker: AgentWorkerStartSpec {
            transport: TransportKind::Line,
            name: "main".to_string(),
            token: Some("line-token".to_string()),
            username: None,
            secret: None,
            app_token: None,
            bot_token: None,
            application_id: None,
        },
        argv: vec!["/bin/sleep".to_string(), "30".to_string()],
        parent_env: BTreeMap::new(),
    })
    .expect("the process layer must not duplicate typed manifest validation");

    assert!(result.started);
    let stop = stop_worker_process(AgentWorkerStopInput {
        paths,
        reason: Some("test_cleanup".to_string()),
        worker_name: Some("main".to_string()),
        stop_timeout_seconds: Some(2.0),
        kill_grace_seconds: Some(0.2),
    })
    .unwrap();
    assert!(stop.stopped);
}

#[test]
fn start_contract_json_matches_python_consumption_keys() {
    let root = temp_dir("start-json");
    let payload = start_worker_process_json(&json!({
        "repo_root": root,
        "paths": {
            "pid_file": root.join("worker.pid"),
            "log_file": root.join("worker.log"),
            "sync_state_path": root.join("sync.json"),
            "env_path": root.join("worker.env"),
            "termination_context_path": root.join("termination.json")
        },
        "worker": {"kind": "telegram", "name": "main", "token": "t"},
        "argv": []
    }))
    .unwrap();

    assert_eq!(payload["started"], false);
    assert_eq!(payload["start_state"], "rust_launch_command_missing");
    assert_eq!(payload["running"], false);
    assert_eq!(payload["pid"], JsonValue::Null);
    assert_eq!(payload["pid_file_written"], false);
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert_eq!(payload["migration_stage"], MIGRATION_STAGE);
}

#[test]
fn stop_contract_cleans_stale_pid_and_termination_context() {
    let root = temp_dir("stop-stale");
    let pid_file = root.join("worker.pid");
    let termination_context_path = root.join("termination.json");
    fs::write(&pid_file, "0\n").unwrap();
    fs::write(&termination_context_path, "{}\n").unwrap();

    let result = stop_worker_process(AgentWorkerStopInput {
        paths: AgentWorkerProcessPaths {
            pid_file: pid_file.to_string_lossy().into_owned(),
            log_file: root.join("worker.log").to_string_lossy().into_owned(),
            sync_state_path: root.join("sync.json").to_string_lossy().into_owned(),
            env_path: root.join("worker.env").to_string_lossy().into_owned(),
            termination_context_path: termination_context_path.to_string_lossy().into_owned(),
        },
        reason: Some("test_stop".to_string()),
        worker_name: Some("main".to_string()),
        stop_timeout_seconds: Some(0.0),
        kill_grace_seconds: Some(0.0),
    })
    .unwrap();

    assert!(!result.stopped);
    assert_eq!(result.stop_state, "not_running");
    assert!(!result.running);
    assert_eq!(result.pid, None);
    assert_eq!(result.target_pid, None);
    assert!(result.pid_file_removed);
    assert!(result.termination_context_removed);
    assert!(!result.termination_context_written);
    assert!(!pid_file.exists());
    assert!(!termination_context_path.exists());
    assert!(!result.python_worker_execution_allowed);
}

#[test]
#[cfg(unix)]
fn stop_contract_writes_context_signals_child_and_removes_pid_file() {
    let root = temp_dir("stop-child");
    let pid_file = root.join("worker.pid");
    let termination_context_path = root.join("termination.json");
    let mut child = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = child.id() as i64;
    fs::write(&pid_file, format!("{pid}\n")).unwrap();

    let result = stop_worker_process(AgentWorkerStopInput {
        paths: AgentWorkerProcessPaths {
            pid_file: pid_file.to_string_lossy().into_owned(),
            log_file: root.join("worker.log").to_string_lossy().into_owned(),
            sync_state_path: root.join("sync.json").to_string_lossy().into_owned(),
            env_path: root.join("worker.env").to_string_lossy().into_owned(),
            termination_context_path: termination_context_path.to_string_lossy().into_owned(),
        },
        reason: Some("cli_telegram_stop".to_string()),
        worker_name: Some("main".to_string()),
        stop_timeout_seconds: Some(2.0),
        kill_grace_seconds: Some(0.2),
    })
    .unwrap();
    let _ = child.wait();

    assert!(result.stopped);
    assert!(matches!(result.stop_state.as_str(), "stopped" | "killed"));
    assert_eq!(result.target_pid, Some(pid));
    assert!(!result.running);
    assert_eq!(result.pid, None);
    assert!(result.pid_file_removed);
    assert!(result.termination_context_written);
    assert!(!pid_file.exists());
    let context: JsonValue = crate::json_support::parse_value(
        &fs::read_to_string(&termination_context_path).unwrap(),
        "Invalid termination context JSON",
    )
    .unwrap();
    assert_eq!(context["pid"], pid);
    assert_eq!(context["reason"], "cli_telegram_stop");
    assert_eq!(context["worker_name"], "main");
    assert!(context["issued_at"]
        .as_str()
        .unwrap_or_default()
        .contains('T'));
    assert!(context["issued_by_pid"].as_u64().unwrap_or_default() > 0);
    assert!(!result.python_worker_execution_allowed);
}

#[test]
#[cfg(windows)]
fn stop_contract_writes_context_and_force_terminates_windows_child() {
    let root = temp_dir("stop-windows-child");
    let paths = test_paths(&root);
    let mut child = Command::new("cmd")
        .args(["/C", "ping -n 30 127.0.0.1 >NUL"])
        .spawn()
        .expect("spawn Windows child");
    let pid = i64::from(child.id());
    fs::write(&paths.pid_file, format!("{pid}\n")).unwrap();

    let result = stop_worker_process(AgentWorkerStopInput {
        paths: paths.clone(),
        reason: Some("windows_test_stop".to_string()),
        worker_name: Some("main".to_string()),
        stop_timeout_seconds: Some(0.0),
        kill_grace_seconds: Some(2.0),
    })
    .unwrap();
    let _ = child.wait();

    assert!(result.stopped);
    assert!(matches!(result.stop_state.as_str(), "stopped" | "killed"));
    assert_eq!(result.target_pid, Some(pid));
    assert!(!result.running);
    assert!(result.termination_context_written);
    assert!(result.pid_file_removed);
    assert!(!Path::new(&paths.pid_file).exists());
    let context: JsonValue = crate::json_support::parse_value(
        &fs::read_to_string(&paths.termination_context_path).unwrap(),
        "Invalid Windows termination context JSON",
    )
    .unwrap();
    assert_eq!(context["pid"], pid);
    assert_eq!(context["reason"], "windows_test_stop");
}

#[test]
fn stop_contract_json_matches_python_consumption_keys() {
    let root = temp_dir("stop-json");
    let pid_file = root.join("worker.pid");
    fs::write(&pid_file, "0\n").unwrap();
    let payload = stop_worker_process_json(&json!({
        "paths": {
            "pid_file": pid_file,
            "log_file": root.join("worker.log"),
            "sync_state_path": root.join("sync.json"),
            "env_path": root.join("worker.env"),
            "termination_context_path": root.join("termination.json")
        },
        "reason": "cli_stop",
        "worker_name": "main",
        "stop_timeout_seconds": 0.0,
        "kill_grace_seconds": 0.0
    }))
    .unwrap();

    assert_eq!(payload["stopped"], false);
    assert_eq!(payload["stop_state"], "not_running");
    assert_eq!(payload["running"], false);
    assert_eq!(payload["pid"], JsonValue::Null);
    assert_eq!(payload["health"]["pid_file_state"], "missing");
    assert_eq!(payload["pid_file_removed"], true);
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert_eq!(payload["migration_stage"], MIGRATION_STAGE);
}

#[derive(Debug)]
struct FakeProcessPort;

fn fake_running_health() -> AgentWorkerRuntimeHealth {
    AgentWorkerRuntimeHealth {
        pid_file_exists: true,
        pid_file_readable: true,
        pid_file_valid: true,
        pid_file_state: "running".to_string(),
        log_exists: true,
        log_size_bytes: 7,
        sync_state_exists: true,
        env_exists: true,
        termination_context_exists: false,
    }
}

fn fake_stopped_health() -> AgentWorkerRuntimeHealth {
    AgentWorkerRuntimeHealth {
        pid_file_exists: false,
        pid_file_readable: false,
        pid_file_valid: false,
        pid_file_state: "missing".to_string(),
        log_exists: true,
        log_size_bytes: 7,
        sync_state_exists: true,
        env_exists: true,
        termination_context_exists: true,
    }
}

impl AgentWorkerProcessStatusPort for FakeProcessPort {
    fn inspect_worker_process_status(
        &self,
        input: AgentWorkerProcessStatusInput,
    ) -> AgentWorkerProcessStatus {
        AgentWorkerProcessStatus {
            running: true,
            pid: Some(42),
            health: AgentWorkerRuntimeHealth {
                pid_file_exists: input.paths.pid_file == "/tmp/fake.pid",
                ..fake_running_health()
            },
            python_worker_execution_allowed: false,
            migration_stage: MIGRATION_STAGE.to_string(),
        }
    }
}

impl AgentWorkerProcessLogTailPort for FakeProcessPort {
    fn read_worker_log_tail(&self, input: AgentWorkerLogTailInput) -> AgentWorkerLogTail {
        AgentWorkerLogTail {
            lines: vec![input.log_file],
            log_exists: true,
            lines_requested: input.lines.unwrap_or(DEFAULT_LOG_TAIL_LINES),
            python_worker_execution_allowed: false,
            migration_stage: MIGRATION_STAGE.to_string(),
        }
    }
}

impl AgentWorkerProcessStartPort for FakeProcessPort {
    fn start_worker_process(
        &self,
        input: AgentWorkerStartInput,
    ) -> Result<AgentWorkerStartResult, String> {
        Ok(AgentWorkerStartResult {
            started: true,
            start_state: "started".to_string(),
            running: true,
            pid: Some(42),
            command: input.argv,
            diagnostic: None,
            health: fake_running_health(),
            env_file_seeded: false,
            termination_context_removed: false,
            pid_file_written: true,
            python_worker_execution_allowed: false,
            migration_stage: MIGRATION_STAGE.to_string(),
        })
    }
}

impl AgentWorkerProcessStopPort for FakeProcessPort {
    fn stop_worker_process(
        &self,
        input: AgentWorkerStopInput,
    ) -> Result<AgentWorkerStopResult, String> {
        Ok(AgentWorkerStopResult {
            stopped: true,
            stop_state: input.reason.unwrap_or_else(|| "stopped".to_string()),
            running: false,
            pid: None,
            target_pid: Some(42),
            health: fake_stopped_health(),
            termination_context_written: true,
            termination_context_removed: false,
            pid_file_removed: true,
            python_worker_execution_allowed: false,
            migration_stage: MIGRATION_STAGE.to_string(),
        })
    }
}

#[derive(Debug)]
struct StatusOnlyProcessPort;

impl AgentWorkerProcessStatusPort for StatusOnlyProcessPort {
    fn inspect_worker_process_status(
        &self,
        input: AgentWorkerProcessStatusInput,
    ) -> AgentWorkerProcessStatus {
        AgentWorkerProcessStatus {
            running: true,
            pid: Some(7),
            health: AgentWorkerRuntimeHealth {
                pid_file_exists: input.paths.pid_file == "/tmp/status-only.pid",
                ..fake_running_health()
            },
            python_worker_execution_allowed: false,
            migration_stage: MIGRATION_STAGE.to_string(),
        }
    }
}

#[derive(Debug)]
struct LogTailOnlyProcessPort;

impl AgentWorkerProcessLogTailPort for LogTailOnlyProcessPort {
    fn read_worker_log_tail(&self, input: AgentWorkerLogTailInput) -> AgentWorkerLogTail {
        AgentWorkerLogTail {
            lines: vec![input.log_file],
            log_exists: true,
            lines_requested: input.lines.unwrap_or(DEFAULT_LOG_TAIL_LINES),
            python_worker_execution_allowed: false,
            migration_stage: MIGRATION_STAGE.to_string(),
        }
    }
}

#[derive(Debug)]
struct StartOnlyProcessPort;

impl AgentWorkerProcessStartPort for StartOnlyProcessPort {
    fn start_worker_process(
        &self,
        input: AgentWorkerStartInput,
    ) -> Result<AgentWorkerStartResult, String> {
        Ok(AgentWorkerStartResult {
            started: true,
            start_state: "started".to_string(),
            running: true,
            pid: Some(9),
            command: input.argv,
            diagnostic: None,
            health: fake_running_health(),
            env_file_seeded: false,
            termination_context_removed: false,
            pid_file_written: true,
            python_worker_execution_allowed: false,
            migration_stage: MIGRATION_STAGE.to_string(),
        })
    }
}

#[derive(Debug)]
struct StopOnlyProcessPort;

impl AgentWorkerProcessStopPort for StopOnlyProcessPort {
    fn stop_worker_process(
        &self,
        input: AgentWorkerStopInput,
    ) -> Result<AgentWorkerStopResult, String> {
        Ok(AgentWorkerStopResult {
            stopped: true,
            stop_state: input.reason.unwrap_or_else(|| "stopped".to_string()),
            running: false,
            pid: None,
            target_pid: Some(9),
            health: fake_stopped_health(),
            termination_context_written: true,
            termination_context_removed: false,
            pid_file_removed: true,
            python_worker_execution_allowed: false,
            migration_stage: MIGRATION_STAGE.to_string(),
        })
    }
}

fn fake_process_paths(pid_file: &str) -> AgentWorkerProcessPaths {
    AgentWorkerProcessPaths {
        pid_file: pid_file.to_string(),
        log_file: "/tmp/fake.log".to_string(),
        sync_state_path: "/tmp/fake-sync.json".to_string(),
        env_path: "/tmp/fake.env".to_string(),
        termination_context_path: "/tmp/fake-termination.json".to_string(),
    }
}

fn fake_start_input(paths: AgentWorkerProcessPaths, argv: Vec<String>) -> AgentWorkerStartInput {
    AgentWorkerStartInput {
        repo_root: "/tmp".to_string(),
        paths,
        worker: AgentWorkerStartSpec {
            transport: TransportKind::Slack,
            name: "main".to_string(),
            token: None,
            username: None,
            secret: None,
            app_token: Some("xapp".to_string()),
            bot_token: None,
            application_id: None,
        },
        argv,
        parent_env: BTreeMap::new(),
    }
}

#[test]
fn agent_worker_process_helpers_accept_single_capability_ports() {
    let paths = fake_process_paths("/tmp/status-only.pid");

    let status = inspect_worker_process_status_with_port(
        &StatusOnlyProcessPort,
        AgentWorkerProcessStatusInput {
            paths: paths.clone(),
        },
    );
    assert!(status.running);
    assert_eq!(status.pid, Some(7));
    assert!(status.health.pid_file_exists);

    let tail = read_worker_log_tail_with_port(
        &LogTailOnlyProcessPort,
        AgentWorkerLogTailInput {
            log_file: "/tmp/single-capability.log".to_string(),
            lines: Some(2),
        },
    );
    assert_eq!(tail.lines, vec!["/tmp/single-capability.log"]);
    assert_eq!(tail.lines_requested, 2);

    let start = start_worker_process_with_port(
        &StartOnlyProcessPort,
        fake_start_input(paths.clone(), vec!["ait-agent-worker".to_string()]),
    )
    .unwrap();
    assert!(start.started);
    assert_eq!(start.pid, Some(9));
    assert_eq!(start.command, vec!["ait-agent-worker"]);

    let stop = stop_worker_process_with_port(
        &StopOnlyProcessPort,
        AgentWorkerStopInput {
            paths,
            reason: Some("single_capability_stop".to_string()),
            worker_name: Some("main".to_string()),
            stop_timeout_seconds: None,
            kill_grace_seconds: None,
        },
    )
    .unwrap();
    assert!(stop.stopped);
    assert_eq!(stop.stop_state, "single_capability_stop");
    assert_eq!(stop.target_pid, Some(9));
}

#[test]
fn agent_worker_process_port_helpers_accept_trait_object() {
    let port_impl = FakeProcessPort;
    let port: &dyn AgentWorkerProcessPort = &port_impl;
    let paths = fake_process_paths("/tmp/fake.pid");

    let status = inspect_worker_process_status_with_port(
        port,
        AgentWorkerProcessStatusInput {
            paths: paths.clone(),
        },
    );
    assert!(status.running);
    assert_eq!(status.pid, Some(42));

    let tail = read_worker_log_tail_with_port(
        port,
        AgentWorkerLogTailInput {
            log_file: "/tmp/fake.log".to_string(),
            lines: Some(1),
        },
    );
    assert_eq!(tail.lines, vec!["/tmp/fake.log"]);
    assert_eq!(tail.lines_requested, 1);

    let start = start_worker_process_with_port(
        port,
        fake_start_input(paths.clone(), vec!["ait-agent-worker".to_string()]),
    )
    .unwrap();
    assert!(start.started);
    assert_eq!(start.command, vec!["ait-agent-worker"]);

    let stop = stop_worker_process_with_port(
        port,
        AgentWorkerStopInput {
            paths,
            reason: Some("stopped_by_test".to_string()),
            worker_name: Some("main".to_string()),
            stop_timeout_seconds: None,
            kill_grace_seconds: None,
        },
    )
    .unwrap();
    assert!(stop.stopped);
    assert_eq!(stop.stop_state, "stopped_by_test");
}
