use super::*;
use ait_core::json_support::json;

#[test]
fn runtime_plan_disallows_python_worker_execution() {
    let plan = plan_agent_runtime(AgentRuntimePlanInput {
        worker_manifest: json!({
            "version": 1,
            "workers": {
                "telegram/main": {"kind": "telegram", "token": "t"}
            }
        }),
        expected_concurrent_workers: Some(128),
    });

    assert_eq!(plan.total_configured_workers, 1);
    assert_eq!(plan.expected_concurrent_workers, 128);
    assert!(!plan.python_worker_execution_allowed);
    assert!(plan.capacity.high_concurrency);
}
