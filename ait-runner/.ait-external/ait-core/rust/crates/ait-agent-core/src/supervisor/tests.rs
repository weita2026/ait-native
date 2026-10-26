use super::*;
use crate::runtime::{plan_agent_runtime, AgentRuntimePlanInput};
use ait_core::json_support::json;

#[test]
fn groups_workers_by_transport_onto_event_loop_shards() {
    let plan = plan_agent_runtime(AgentRuntimePlanInput {
        worker_manifest: json!({
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"},
                "discord/main": {"kind": "discord", "application_id": "a", "bot_token": "b"}
            }
        }),
        expected_concurrent_workers: Some(2),
    });

    let launches = plan_worker_launches(&plan);

    assert_eq!(launches.len(), 2);
    assert!(launches
        .iter()
        .any(|row| row.transport == TransportKind::Telegram));
    assert!(launches
        .iter()
        .any(|row| row.transport == TransportKind::Discord));
    assert!(launches
        .iter()
        .all(|row| row.event_loop_backend == plan.capacity.backend.label()));
}

#[test]
fn public_worker_payload_redacts_secrets_and_adds_status_facts() {
    let planned = agent_supervisor_public_worker_payload_json(&json!({
        "worker": {
            "kind": "discord",
            "name": "main",
            "token": "telegram-secret",
            "secret": "signing-secret",
            "app_token": "xapp-token",
            "bot_token": "",
            "application_id": "app-1"
        },
        "env_bot_token": "discord-env-token",
        "config": {"version": 2},
        "config_issues": ["bad token field"],
        "paths": {
            "sync_state_path": "/tmp/sync.json",
            "pid_file": "/tmp/worker.pid",
            "log_file": "/tmp/worker.log",
            "env_path": "/tmp/worker.env",
            "termination_context_path": "/tmp/term.json"
        },
        "process_status": {
            "running": true,
            "pid": 1234,
            "health": {"pid_file_state": "running"}
        }
    }))
    .expect("public worker payload");

    assert_eq!(planned["kind"], "discord");
    assert_eq!(planned["name"], "main");
    assert!(planned.get("token").is_none());
    assert!(planned.get("secret").is_none());
    assert!(planned.get("app_token").is_none());
    assert!(planned.get("bot_token").is_none());
    assert_eq!(planned["token_set"], true);
    assert_eq!(planned["secret_set"], true);
    assert_eq!(planned["app_token_set"], true);
    assert_eq!(planned["bot_token_set"], true);
    assert_eq!(planned["bot_token_preview"], "*************oken");
    assert_eq!(planned["config_version"], 2);
    assert_eq!(planned["config_valid"], false);
    assert_eq!(planned["config_issues"][0], "bad token field");
    assert_eq!(planned["running"], true);
    assert_eq!(planned["pid"], 1234);
    assert_eq!(planned["pid_file"], "/tmp/worker.pid");
    assert_eq!(planned["health"]["pid_file_state"], "running");
    assert_eq!(planned["python_worker_execution_allowed"], false);
}
