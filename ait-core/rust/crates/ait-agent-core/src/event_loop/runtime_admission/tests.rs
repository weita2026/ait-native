use super::*;
use ait_core::json_support::json;

#[test]
fn runtime_admission_grants_epoll_worker_leases_for_high_concurrency() {
    let planned = plan_agent_runtime_admission(AgentRuntimeAdmissionInput {
        worker_manifest: json!({
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"},
                "discord/ops": {"kind": "discord", "application_id": "a", "bot_token": "b"}
            }
        }),
        expected_concurrent_workers: Some(512),
        backend: Some("linux_epoll".to_string()),
        workers_per_shard: None,
        transport_runtime: "rust".to_string(),
        allow_python_fallback: false,
        requested_worker_keys: Vec::new(),
    })
    .unwrap();

    assert_eq!(planned.admission_state, "admitted");
    assert!(planned.launch_allowed);
    assert!(planned.high_concurrency);
    assert!(!planned.python_worker_execution_allowed);
    assert_eq!(planned.selected_worker_count, 2);
    assert_eq!(planned.shard_count, 2);
    assert_eq!(planned.worker_leases[0].lease_contract, LEASE_CONTRACT);
    assert_eq!(planned.worker_leases[0].backend, "linux_epoll");
    assert_eq!(planned.worker_leases[0].token, 1);
    assert!(planned.worker_leases[0].rust_event_loop_required);
    assert!(!planned.worker_leases[0].python_fallback_allowed);
    assert_eq!(planned.shard_admissions[0].lease_count, 2);
    assert_eq!(planned.shard_admissions[0].inflight_limit, 256);
}

#[test]
fn runtime_admission_rejects_python_fallback_and_non_epoll_high_concurrency() {
    let planned = plan_agent_runtime_admission(AgentRuntimeAdmissionInput {
        worker_manifest: json!({
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"}
            }
        }),
        expected_concurrent_workers: Some(128),
        backend: Some("portable_poll".to_string()),
        workers_per_shard: None,
        transport_runtime: "python".to_string(),
        allow_python_fallback: true,
        requested_worker_keys: Vec::new(),
    })
    .unwrap();

    assert_eq!(planned.admission_state, "rejected");
    assert!(!planned.launch_allowed);
    assert!(planned.python_fallback_requested);
    assert!(planned.requires_epoll_for_target_scale);
    assert!(planned
        .rejection_reasons
        .iter()
        .any(|reason| reason.contains("python fallback execution is disabled")));
    assert!(planned
        .rejection_reasons
        .iter()
        .any(|reason| reason.contains("transport_runtime `python` is not allowed")));
    assert!(planned
        .rejection_reasons
        .iter()
        .any(|reason| reason.contains("requires linux_epoll")));
}

#[test]
fn runtime_admission_rejects_unknown_requested_workers() {
    let planned = agent_runtime_admission_plan_json(&json!({
        "worker_manifest": {
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"}
            }
        },
        "backend": "linux_epoll",
        "requested_worker_keys": ["discord/ops"]
    }))
    .unwrap();

    assert_eq!(planned["admission_state"], "rejected");
    assert_eq!(planned["selected_worker_count"], 0);
    assert!(planned["rejection_reasons"][0]
        .as_str()
        .unwrap()
        .contains("requested worker `discord/ops`"));
}

#[test]
fn runtime_admission_json_serializes_contract_shape() {
    let planned = agent_runtime_admission_plan_json(&json!({
        "worker_manifest": {
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"}
            }
        },
        "expected_concurrent_workers": 1,
        "backend": "linux_epoll"
    }))
    .unwrap();

    assert_eq!(planned["migration_stage"], MIGRATION_STAGE);
    assert_eq!(planned["admission_contract"], ADMISSION_CONTRACT);
    assert_eq!(planned["admission_state"], "admitted");
    assert_eq!(
        planned["worker_leases"][0]["lease_contract"],
        LEASE_CONTRACT
    );
    assert_eq!(planned["worker_leases"][0]["worker_key"], "telegram/main");
    assert_eq!(
        planned["worker_leases"][0]["python_fallback_allowed"],
        false
    );
}
