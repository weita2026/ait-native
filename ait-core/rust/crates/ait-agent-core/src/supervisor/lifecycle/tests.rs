use super::*;
use ait_core::json_support::json;
use std::fs;
use std::path::{Path, PathBuf};

fn input(operation: AgentWorkerLifecycleOperation) -> AgentWorkerLifecyclePlanInput {
    AgentWorkerLifecyclePlanInput {
        repo_root: "/repo".to_string(),
        operation,
        worker: AgentWorkerLifecycleSpec {
            transport: TransportKind::Telegram,
            name: "main".to_string(),
            sync_state_path: None,
            pid_file: None,
            log_file: None,
            env_path: None,
            termination_context_path: None,
        },
        runtime_root: None,
        stop_timeout_seconds: None,
        kill_grace_seconds: None,
    }
}

fn temp_repo_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "ait-agent-lifecycle-{label}-{}-{}",
        std::process::id(),
        unique_nonce()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create temp repo root");
    root
}

fn acquire_input(
    repo_root: &Path,
    transport: TransportKind,
    action: &str,
) -> AgentWorkerLifecycleLockAcquireInput {
    AgentWorkerLifecycleLockAcquireInput {
        repo_root: path_to_string(repo_root),
        transport,
        action: action.to_string(),
        runtime_root: None,
    }
}

#[test]
fn supervisor_lifecycle_plans_default_worker_runtime_paths() {
    let plan = plan_worker_supervisor_lifecycle(input(AgentWorkerLifecycleOperation::Start))
        .expect("lifecycle plan");

    assert_eq!(plan.worker_key, "telegram/main");
    assert_eq!(plan.paths.runtime_root, "/repo/.ait/agent-runtime");
    assert_eq!(
        plan.paths.sync_state_path,
        "/repo/.ait/agent-runtime/telegram-main-sync.json"
    );
    assert_eq!(
        plan.paths.pid_file,
        "/repo/.ait/agent-runtime/telegram-main.pid"
    );
    assert_eq!(
        plan.paths.log_file,
        "/repo/.ait/agent-runtime/telegram-main.log"
    );
    assert_eq!(plan.paths.env_path, "/repo/.ait/agent-runtime/telegram.env");
    assert_eq!(
        plan.paths.termination_context_path,
        "/repo/.ait/agent-runtime/telegram-main-termination.json"
    );
    assert_eq!(
        plan.paths.lifecycle_lock_path,
        "/repo/.ait/agent-runtime/telegram-lifecycle.lock"
    );
    assert_eq!(plan.directories_to_create, vec!["/repo/.ait/agent-runtime"]);
    assert!(plan.lock_required);
    assert!(plan.status_probe_required);
    assert!(plan.clear_termination_context_before_start);
    assert!(!plan.write_termination_context_before_stop);
    assert!(plan.spawn_worker);
    assert!(!plan.python_worker_execution_allowed);
}

#[test]
fn supervisor_lifecycle_resolves_relative_overrides_against_repo_root() {
    let mut request = input(AgentWorkerLifecycleOperation::Stop);
    request.worker.transport = TransportKind::Discord;
    request.worker.name = "side car!".to_string();
    request.worker.pid_file = Some("runtime/worker.pid".to_string());
    request.worker.log_file = Some("runtime/worker.log".to_string());
    request.worker.env_path = Some("/var/tmp/discord.env".to_string());
    request.runtime_root = Some(".custom-agent-runtime".to_string());

    let plan = plan_worker_supervisor_lifecycle(request).expect("lifecycle plan");

    assert_eq!(plan.worker_key, "discord/side car!");
    assert_eq!(plan.paths.runtime_root, "/repo/.custom-agent-runtime");
    assert_eq!(
        plan.paths.sync_state_path,
        "/repo/.custom-agent-runtime/discord-side-car-sync.json"
    );
    assert_eq!(plan.paths.pid_file, "/repo/runtime/worker.pid");
    assert_eq!(plan.paths.log_file, "/repo/runtime/worker.log");
    assert_eq!(plan.paths.env_path, "/var/tmp/discord.env");
    assert_eq!(
        plan.paths.termination_context_path,
        "/repo/.custom-agent-runtime/discord-side-car-termination.json"
    );
    assert!(plan.lock_required);
    assert!(plan.write_termination_context_before_stop);
    assert!(plan.remove_pid_file_on_successful_stop);
    assert!(!plan.spawn_worker);
}

#[test]
fn supervisor_lifecycle_restart_plan_models_stop_then_start_metadata() {
    let plan = plan_worker_supervisor_lifecycle(input(AgentWorkerLifecycleOperation::Restart))
        .expect("lifecycle plan");

    assert!(plan.lock_required);
    assert!(plan.clear_termination_context_before_start);
    assert!(plan.write_termination_context_before_stop);
    assert!(plan.remove_pid_file_on_successful_stop);
    assert!(plan.spawn_worker);
    assert_eq!(plan.stop_timeout_seconds, 10.0);
    assert_eq!(plan.kill_grace_seconds, 2.0);
}

#[test]
fn supervisor_lifecycle_status_plan_does_not_take_lifecycle_lock() {
    let plan = plan_worker_supervisor_lifecycle(input(AgentWorkerLifecycleOperation::Status))
        .expect("lifecycle plan");

    assert!(!plan.lock_required);
    assert!(plan.status_probe_required);
    assert!(!plan.clear_termination_context_before_start);
    assert!(!plan.write_termination_context_before_stop);
    assert!(!plan.remove_pid_file_on_successful_stop);
    assert!(!plan.spawn_worker);
}

#[test]
fn supervisor_lifecycle_json_contract_rejects_bad_requests() {
    let err = plan_worker_supervisor_lifecycle_json(&json!({
        "repo_root": "/repo",
        "operation": "start",
        "worker": {"kind": "mastodon", "name": "main"}
    }))
    .expect_err("unsupported transport must fail");
    assert!(err.contains("invalid ait-agent supervisor lifecycle request"));

    let err = plan_worker_supervisor_lifecycle_json(&json!({
        "repo_root": "/repo",
        "operation": "start",
        "worker": {"kind": "telegram", "name": "bad/name"}
    }))
    .expect_err("worker names with slash must fail");
    assert!(err.contains("worker.name must not contain"));
}

#[test]
fn supervisor_lifecycle_json_contract_serializes_fail_closed_flags() {
    let payload = plan_worker_supervisor_lifecycle_json(&json!({
        "repo_root": "/repo",
        "operation": "start",
        "worker": {"transport": "line", "name": "main"},
        "stop_timeout_seconds": 1.5,
        "kill_grace_seconds": 0.25
    }))
    .expect("json lifecycle plan");

    assert_eq!(payload["worker_key"], "line/main");
    assert_eq!(payload["transport"], "line");
    assert_eq!(
        payload["paths"]["env_path"],
        "/repo/.ait/agent-runtime/line.env"
    );
    assert_eq!(payload["python_worker_execution_allowed"], false);
    assert_eq!(
        payload["migration_stage"],
        "rust_agent_supervisor_lifecycle_contract"
    );
    assert_eq!(payload["stop_timeout_seconds"], 1.5);
    assert_eq!(payload["kill_grace_seconds"], 0.25);
}

#[test]
fn supervisor_lifecycle_lock_acquire_writes_exclusive_lock_and_release_removes_it() {
    let repo_root = temp_repo_root("acquire-release");
    let acquire =
        acquire_worker_lifecycle_lock(acquire_input(&repo_root, TransportKind::Telegram, "start"))
            .expect("acquire lifecycle lock");
    let lock_path = PathBuf::from(&acquire.lifecycle_lock_path);

    assert!(acquire.acquired);
    assert_eq!(acquire.action, "start");
    assert_eq!(acquire.transport, TransportKind::Telegram);
    assert!(!acquire.lock_token.is_empty());
    assert_eq!(acquire.lock_pid, std::process::id() as i64);
    assert!(!acquire.python_worker_execution_allowed);
    assert_eq!(
        acquire.migration_stage,
        "rust_agent_supervisor_lifecycle_lock_contract"
    );
    assert!(lock_path.exists());

    let busy =
        acquire_worker_lifecycle_lock(acquire_input(&repo_root, TransportKind::Telegram, "stop"))
            .expect_err("second acquire should be busy");
    assert!(busy.contains("telegram lifecycle lock is busy"));

    let release = release_worker_lifecycle_lock(AgentWorkerLifecycleLockReleaseInput {
        lifecycle_lock_path: acquire.lifecycle_lock_path.clone(),
        lock_token: acquire.lock_token.clone(),
    })
    .expect("release lifecycle lock");
    assert!(release.released);
    assert!(release.lock_present);
    assert_eq!(release.lock_token, acquire.lock_token);
    assert!(!lock_path.exists());

    let _ = fs::remove_dir_all(repo_root);
}

#[test]
fn supervisor_lifecycle_lock_removes_dead_pid_stale_lock_before_acquire() {
    let repo_root = temp_repo_root("stale");
    let repo_root_text = path_to_string(&repo_root);
    let lock_path = supervisor_lifecycle_lock_path(&repo_root_text, TransportKind::Discord, None)
        .expect("lifecycle lock path");
    fs::create_dir_all(lock_path.parent().expect("lock parent")).expect("create lock parent");
    fs::write(
        &lock_path,
        r#"{"action":"restart","lock_token":"stale","pid":999999999999}"#,
    )
    .expect("write stale lock");

    let acquire =
        acquire_worker_lifecycle_lock(acquire_input(&repo_root, TransportKind::Discord, "restart"))
            .expect("acquire after stale lock");

    assert!(acquire.acquired);
    assert!(acquire.stale_lock_removed);
    assert_ne!(acquire.lock_token, "stale");
    assert_eq!(acquire.lifecycle_lock_path, path_to_string(&lock_path));

    let release = release_worker_lifecycle_lock(AgentWorkerLifecycleLockReleaseInput {
        lifecycle_lock_path: acquire.lifecycle_lock_path,
        lock_token: acquire.lock_token,
    })
    .expect("release replacement lock");
    assert!(release.released);

    let _ = fs::remove_dir_all(repo_root);
}

#[test]
fn supervisor_lifecycle_lock_release_rejects_mismatched_token() {
    let repo_root = temp_repo_root("token-mismatch");
    let acquire =
        acquire_worker_lifecycle_lock(acquire_input(&repo_root, TransportKind::Slack, "stop"))
            .expect("acquire lifecycle lock");
    let lock_path = PathBuf::from(&acquire.lifecycle_lock_path);

    let err = release_worker_lifecycle_lock(AgentWorkerLifecycleLockReleaseInput {
        lifecycle_lock_path: acquire.lifecycle_lock_path.clone(),
        lock_token: "wrong-token".to_string(),
    })
    .expect_err("mismatched token should fail");
    assert!(err.contains("lifecycle lock token mismatch"));
    assert!(lock_path.exists());

    let release = release_worker_lifecycle_lock(AgentWorkerLifecycleLockReleaseInput {
        lifecycle_lock_path: acquire.lifecycle_lock_path,
        lock_token: acquire.lock_token,
    })
    .expect("release matching token");
    assert!(release.released);
    assert!(!lock_path.exists());

    let _ = fs::remove_dir_all(repo_root);
}
