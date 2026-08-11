use super::*;
use ait_core::json_support::json;

#[test]
fn launch_plan_fails_closed_when_transport_runtime_is_missing() {
    let plan = plan_agent_cli_launch(AgentCliPlanInput {
        worker_manifest: json!({
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"}
            }
        }),
        expected_concurrent_workers: Some(128),
        rust_worker_binary: None,
        available_rust_transports: Vec::new(),
    });

    assert!(!plan.launch_allowed);
    assert_eq!(plan.blocked_worker_count, 1);
    assert!(!plan.python_worker_execution_allowed);
    assert_eq!(
        plan.workers[0].launch_state,
        AgentWorkerLaunchState::MissingRustTransportRuntime
    );
    assert!(plan.workers[0].argv[0].contains("ait-agent-worker"));
    assert!(!plan
        .workers
        .iter()
        .flat_map(|worker| worker.argv.iter())
        .any(|arg| arg == "python" || arg == "ait_agent.telegram.app"));
    assert!(plan.runtime.capacity.high_concurrency);
}

#[test]
fn launch_plan_emits_rust_worker_argv_when_transport_is_available() {
    let plan = plan_agent_cli_launch(AgentCliPlanInput {
        worker_manifest: json!({
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"}
            }
        }),
        expected_concurrent_workers: Some(1),
        rust_worker_binary: Some("ait-agent".to_string()),
        available_rust_transports: vec![TransportKind::Telegram],
    });

    assert!(plan.launch_allowed);
    assert_eq!(plan.blocked_worker_count, 0);
    assert_eq!(plan.workers[0].launch_state, AgentWorkerLaunchState::Ready);
    assert_eq!(
        plan.workers[0].argv,
        vec![
            "ait-agent",
            "run",
            "--transport",
            "telegram",
            "--worker",
            "main",
            "--event-loop-backend",
            plan.runtime.capacity.backend.label(),
            "--shard",
            "0",
        ]
    );
}
