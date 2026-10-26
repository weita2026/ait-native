use super::*;
use ait_core::json_support::json;

#[test]
fn reactor_plan_uses_epoll_shards_for_high_concurrency_target() {
    let planned = plan_agent_event_loop_reactor(AgentReactorPlanInput {
        worker_manifest: json!({
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"},
                "discord/ops": {"kind": "discord", "application_id": "a", "bot_token": "b"}
            }
        }),
        expected_concurrent_workers: Some(513),
        backend: Some("linux_epoll".to_string()),
        workers_per_shard: None,
    })
    .unwrap();

    assert_eq!(planned.backend, "linux_epoll");
    assert_eq!(planned.workers_per_shard, 256);
    assert_eq!(planned.shard_count, 3);
    assert!(planned.high_concurrency);
    assert!(!planned.requires_epoll_for_target_scale);
    assert!(planned.launch_allowed);
    assert!(!planned.python_worker_execution_allowed);
    assert_eq!(planned.reactor_shards[0].workers.len(), 2);
    assert_eq!(planned.reactor_shards[0].workers[0].token, 1);
    assert_eq!(planned.reactor_shards[0].workers[1].token, 2);
    assert_eq!(
        crate::json_support::encode_to_value(
            &planned.transport_counts,
            "Failed to encode transport counts",
        )
        .unwrap(),
        json!([
            {"transport": "telegram", "configured_workers": 1},
            {"transport": "discord", "configured_workers": 1},
            {"transport": "slack", "configured_workers": 0},
            {"transport": "line", "configured_workers": 0}
        ])
    );
}

#[test]
fn reactor_plan_fails_closed_for_high_concurrency_portable_poll() {
    let planned = plan_agent_event_loop_reactor(AgentReactorPlanInput {
        worker_manifest: json!({
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"}
            }
        }),
        expected_concurrent_workers: Some(128),
        backend: Some("portable_poll".to_string()),
        workers_per_shard: None,
    })
    .unwrap();

    assert_eq!(planned.backend, "portable_poll");
    assert_eq!(planned.shard_count, 4);
    assert!(planned.high_concurrency);
    assert!(planned.requires_epoll_for_target_scale);
    assert!(!planned.launch_allowed);
    assert!(planned
        .diagnostics
        .iter()
        .any(|line| line.contains("requires linux_epoll")));
}

#[test]
fn reactor_plan_json_rejects_unknown_backend() {
    let err = agent_event_loop_reactor_plan_json(&json!({"backend": "select"})).unwrap_err();

    assert!(err.contains("invalid ait-agent event-loop backend"));
}

#[test]
fn reactor_plan_json_serializes_contract_shape() {
    let planned = agent_event_loop_reactor_plan_json(&json!({
        "worker_manifest": {
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"}
            }
        },
        "expected_concurrent_workers": 2,
        "backend": "linux_epoll",
        "workers_per_shard": 1
    }))
    .unwrap();

    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(planned["driver_contract"], DRIVER_CONTRACT);
    assert_eq!(
        planned["reactor_shards"][0]["workers"][0]["worker_key"],
        "telegram/main"
    );
    assert_eq!(planned["reactor_shards"][0]["workers"][0]["token"], 1);
    assert_eq!(planned["reactor_shards"][1]["worker_count"], 0);
}
