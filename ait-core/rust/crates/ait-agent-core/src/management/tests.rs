use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use ait_core::json_support::json;

use crate::supervisor::{
    AgentWorkerLogTail, AgentWorkerLogTailInput, AgentWorkerProcessLogTailPort,
    AgentWorkerProcessStartPort, AgentWorkerProcessStatus, AgentWorkerProcessStatusInput,
    AgentWorkerProcessStatusPort, AgentWorkerProcessStopPort, AgentWorkerRuntimeHealth,
    AgentWorkerStartInput, AgentWorkerStartResult, AgentWorkerStopInput, AgentWorkerStopResult,
};
use crate::AgentEventLoopBackend;

use super::*;

#[derive(Debug, Default)]
struct FakeProcessState {
    running: BTreeMap<String, i64>,
    starts: Vec<Vec<String>>,
    stop_count: usize,
}

#[derive(Clone, Debug, Default)]
struct FakeProcessPort {
    state: Arc<Mutex<FakeProcessState>>,
}

impl FakeProcessPort {
    fn state(&self) -> std::sync::MutexGuard<'_, FakeProcessState> {
        self.state.lock().unwrap()
    }
}

impl AgentWorkerProcessStatusPort for FakeProcessPort {
    fn inspect_worker_process_status(
        &self,
        input: AgentWorkerProcessStatusInput,
    ) -> AgentWorkerProcessStatus {
        let state = self.state();
        let pid = state.running.get(&input.paths.pid_file).copied();
        AgentWorkerProcessStatus {
            running: pid.is_some(),
            pid,
            health: fake_health(pid.is_some()),
            python_worker_execution_allowed: false,
            migration_stage: "test".to_string(),
        }
    }
}

impl AgentWorkerProcessLogTailPort for FakeProcessPort {
    fn read_worker_log_tail(&self, input: AgentWorkerLogTailInput) -> AgentWorkerLogTail {
        AgentWorkerLogTail {
            lines: vec!["first".to_string(), "second".to_string()],
            log_exists: true,
            lines_requested: input.lines.unwrap_or(100),
            python_worker_execution_allowed: false,
            migration_stage: "test".to_string(),
        }
    }
}

impl AgentWorkerProcessStartPort for FakeProcessPort {
    fn start_worker_process(
        &self,
        input: AgentWorkerStartInput,
    ) -> Result<AgentWorkerStartResult, String> {
        let mut state = self.state();
        let pid = 4242 + i64::try_from(state.starts.len()).unwrap();
        state.running.insert(input.paths.pid_file.clone(), pid);
        state.starts.push(input.argv.clone());
        Ok(AgentWorkerStartResult {
            started: true,
            start_state: "started".to_string(),
            running: true,
            pid: Some(pid),
            command: input.argv,
            diagnostic: None,
            health: fake_health(true),
            env_file_seeded: false,
            termination_context_removed: false,
            pid_file_written: true,
            python_worker_execution_allowed: false,
            migration_stage: "test".to_string(),
        })
    }
}

impl AgentWorkerProcessStopPort for FakeProcessPort {
    fn stop_worker_process(
        &self,
        input: AgentWorkerStopInput,
    ) -> Result<AgentWorkerStopResult, String> {
        let mut state = self.state();
        let target_pid = state.running.remove(&input.paths.pid_file);
        let was_running = target_pid.is_some();
        state.stop_count += usize::from(was_running);
        Ok(AgentWorkerStopResult {
            stopped: was_running,
            stop_state: if was_running {
                "stopped"
            } else {
                "not_running"
            }
            .to_string(),
            running: false,
            pid: None,
            target_pid,
            health: fake_health(false),
            termination_context_written: was_running,
            termination_context_removed: !was_running,
            pid_file_removed: was_running,
            python_worker_execution_allowed: false,
            migration_stage: "test".to_string(),
        })
    }
}

#[derive(Clone, Debug)]
struct FakeCapabilityProbe {
    supported: Vec<TransportKind>,
    error: Option<String>,
}

impl AgentWorkerCapabilityProbe for FakeCapabilityProbe {
    fn probe(&self, _worker_binary: &str) -> Result<AgentWorkerCapabilityReport, String> {
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(AgentWorkerCapabilityReport {
            supported_transports: self.supported.clone(),
            python_worker_execution_allowed: false,
        })
    }
}

fn fake_health(running: bool) -> AgentWorkerRuntimeHealth {
    AgentWorkerRuntimeHealth {
        pid_file_exists: running,
        pid_file_readable: running,
        pid_file_valid: running,
        pid_file_state: if running { "running" } else { "missing" }.to_string(),
        log_exists: false,
        log_size_bytes: 0,
        sync_state_exists: false,
        env_exists: false,
        termination_context_exists: false,
    }
}

fn runtime(
    root: &std::path::Path,
    process: FakeProcessPort,
    supported: Vec<TransportKind>,
) -> AgentManagementRuntime<FakeProcessPort, FakeCapabilityProbe> {
    AgentManagementRuntime::with_ports(
        root,
        root.join(".ait/agent-workers.json"),
        "ait-agent-worker",
        BTreeMap::new(),
        process,
        FakeCapabilityProbe {
            supported,
            error: None,
        },
    )
}

#[test]
fn all_transport_management_surfaces_share_manifest_and_redaction_contracts() {
    let temp = tempfile::tempdir().unwrap();
    let process = FakeProcessPort::default();
    let runtime = runtime(temp.path(), process, Vec::new());
    let fixtures = [
        (
            TransportKind::Telegram,
            json!({"kind": "telegram", "name": "main", "token": "tg-secret", "username": "bot"}),
            "token_set",
        ),
        (
            TransportKind::Line,
            json!({"kind": "line", "name": "main", "token": "line-token", "secret": "line-secret"}),
            "secret_set",
        ),
        (
            TransportKind::Discord,
            json!({"kind": "discord", "name": "main", "application_id": "app-1", "bot_token": "discord-secret"}),
            "bot_token_set",
        ),
        (
            TransportKind::Slack,
            json!({"kind": "slack", "name": "main", "app_token": "slack-secret"}),
            "app_token_set",
        ),
    ];

    for (transport, worker, secret_flag) in fixtures {
        let added = runtime.add_worker(worker).unwrap();
        assert_eq!(added["kind"], transport.as_str());
        assert_eq!(added["name"], "main");
        assert_eq!(added[secret_flag], true);
        for secret in ["token", "secret", "bot_token", "app_token"] {
            assert!(added.get(secret).is_none());
        }
        let listed = runtime.list_workers(transport).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["name"], "main");
        let status = runtime.status_workers(transport, Some("main")).unwrap();
        assert_eq!(status["running"], false);
        assert_eq!(status["config_valid"], true);
    }

    for transport in TransportKind::ALL {
        let removed = runtime.remove_worker(transport, "main").unwrap();
        assert_eq!(
            removed,
            json!({"removed": true, "kind": transport.as_str(), "name": "main"})
        );
        assert!(runtime.list_workers(transport).unwrap().is_empty());
    }
}

#[test]
fn start_is_blocked_until_worker_capability_report_names_transport() {
    let temp = tempfile::tempdir().unwrap();
    let process = FakeProcessPort::default();
    let runtime = runtime(temp.path(), process.clone(), Vec::new());
    runtime
        .add_worker(json!({
            "kind": "telegram",
            "name": "main",
            "token": "secret"
        }))
        .unwrap();

    let payload = runtime
        .start_worker(TransportKind::Telegram, "main")
        .unwrap();

    assert_eq!(payload["started"], false);
    assert_eq!(payload["start_state"], "missing_rust_transport_runtime");
    assert_eq!(payload["rust_launch_blocked"], true);
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert!(payload["planned_command"]
        .as_array()
        .unwrap()
        .iter()
        .all(|part| !part.as_str().unwrap().contains("python")));
    assert!(process.state().starts.is_empty());
    assert!(!temp
        .path()
        .join(".ait/agent-runtime/telegram-lifecycle.lock")
        .exists());
}

#[test]
fn foreground_command_reuses_manifest_capability_and_shard_authority() {
    let temp = tempfile::tempdir().unwrap();
    let process = FakeProcessPort::default();
    let runtime = runtime(
        temp.path(),
        process,
        vec![TransportKind::Telegram, TransportKind::Slack],
    );
    runtime
        .add_worker(json!({
            "kind": "slack",
            "name": "primary",
            "app_token": "slack-secret"
        }))
        .unwrap();
    runtime
        .add_worker(json!({
            "kind": "telegram",
            "name": "main",
            "token": "telegram-secret"
        }))
        .unwrap();

    let command = runtime
        .foreground_worker_command(TransportKind::Telegram, "main")
        .unwrap();

    assert_eq!(
        command,
        vec![
            "ait-agent-worker",
            "run",
            "--transport",
            "telegram",
            "--worker",
            "main",
            "--event-loop-backend",
            AgentEventLoopBackend::current_platform_default().label(),
            "--shard",
            "0",
        ]
    );
    assert!(command.iter().all(|part| !part.contains("python")));
    assert!(!command.iter().any(|part| part.contains("telegram-secret")));
}

#[test]
fn foreground_command_fails_closed_for_missing_or_unsupported_workers() {
    let temp = tempfile::tempdir().unwrap();
    let process = FakeProcessPort::default();
    let runtime = runtime(temp.path(), process, Vec::new());
    runtime
        .add_worker(json!({
            "kind": "telegram",
            "name": "main",
            "token": "private-telegram-secret"
        }))
        .unwrap();

    let unsupported = runtime
        .foreground_worker_command(TransportKind::Telegram, "main")
        .unwrap_err();
    assert!(unsupported.contains("refusing Python fallback"));
    assert!(!unsupported.contains("private-telegram-secret"));

    let missing = runtime
        .foreground_worker_command(TransportKind::Telegram, "missing")
        .unwrap_err();
    assert_eq!(missing, "Unknown telegram worker: missing");
}

#[test]
fn foreground_command_redacts_capability_probe_failures() {
    let temp = tempfile::tempdir().unwrap();
    let process = FakeProcessPort::default();
    let runtime = AgentManagementRuntime::with_ports(
        temp.path(),
        temp.path().join(".ait/agent-workers.json"),
        "ait-agent-worker",
        BTreeMap::new(),
        process,
        FakeCapabilityProbe {
            supported: Vec::new(),
            error: Some("stale-binary-private-secret".to_string()),
        },
    );
    runtime
        .add_worker(json!({
            "kind": "telegram",
            "name": "main",
            "token": "private-telegram-secret"
        }))
        .unwrap();

    let error = runtime
        .foreground_worker_command(TransportKind::Telegram, "main")
        .unwrap_err();

    assert!(error.contains("rebuild or reinstall"));
    assert!(error.contains("Refusing Python fallback"));
    assert!(!error.contains("stale-binary-private-secret"));
    assert!(!error.contains("private-telegram-secret"));
}

#[test]
fn supported_worker_starts_stops_restarts_and_reads_logs_through_ports() {
    let temp = tempfile::tempdir().unwrap();
    let process = FakeProcessPort::default();
    let runtime = runtime(temp.path(), process.clone(), vec![TransportKind::Telegram]);
    runtime
        .add_worker(json!({
            "kind": "telegram",
            "name": "main",
            "token": "secret"
        }))
        .unwrap();

    let started = runtime
        .start_worker(TransportKind::Telegram, "main")
        .unwrap();
    assert_eq!(started["started"], true);
    assert_eq!(started["running"], true);
    assert_eq!(started["command"][0], "ait-agent-worker");
    assert_eq!(started["command"][2], "--transport");
    assert_eq!(started["command"][3], "telegram");

    let already_running = runtime
        .start_worker(TransportKind::Telegram, "main")
        .unwrap();
    assert_eq!(already_running["started"], false);
    assert_eq!(process.state().starts.len(), 1);

    let logs = runtime
        .worker_logs(TransportKind::Telegram, "main", 25)
        .unwrap();
    assert_eq!(logs["lines"], json!(["first", "second"]));
    assert_eq!(logs["lines_requested"], 25);

    let restarted = runtime
        .restart_worker(TransportKind::Telegram, "main")
        .unwrap();
    assert_eq!(restarted["stopped"], true);
    assert_eq!(restarted["restarted"], true);
    assert_eq!(process.state().starts.len(), 2);
    assert_eq!(process.state().stop_count, 1);

    let stopped = runtime
        .stop_worker(TransportKind::Telegram, "main")
        .unwrap();
    assert_eq!(stopped["stopped"], true);
    assert_eq!(stopped["stop_state"], "stopped");
    assert_eq!(stopped["running"], false);
}

#[test]
fn supervisor_and_missing_worker_errors_preserve_public_contract() {
    let temp = tempfile::tempdir().unwrap();
    let process = FakeProcessPort::default();
    let runtime = runtime(temp.path(), process, vec![TransportKind::Telegram]);
    runtime
        .add_worker(json!({"kind": "telegram", "name": "a", "token": "one"}))
        .unwrap();
    runtime
        .add_worker(json!({"kind": "telegram", "name": "b", "token": "two"}))
        .unwrap();

    let payload = runtime
        .telegram_supervisor(AgentSupervisorAction::Start, None, None)
        .unwrap();
    assert_eq!(payload["kind"], "telegram-supervisor");
    assert_eq!(payload["worker_count"], 2);
    assert_eq!(payload["running_count"], 2);
    assert_eq!(payload["started_count"], 2);
    assert_eq!(payload["config_valid"], true);

    assert_eq!(
        runtime
            .status_workers(TransportKind::Slack, Some("missing"))
            .unwrap_err(),
        "Unknown slack worker: missing"
    );
}
