#[test]
fn handshake_advertises_contract_version_and_capabilities() {
    let value = stdout_json(&run_seam(&["handshake"]));
    assert_eq!(value.get("ready"), Some(&json!(true)));
    assert_eq!(
        value.get("contract_version"),
        Some(&json!("ait-server-core-seam-v1"))
    );
    assert_eq!(value.get("package_version"), Some(&json!("0.1.0")));

    let capabilities = value
        .get("capabilities")
        .and_then(JsonValue::as_array)
        .expect("capabilities should be an array");
    assert!(capabilities.contains(&json!(
        "foundation.agent_protocol.normalize_agent_server_job"
    )));
    assert!(capabilities.contains(&json!("foundation.agent_protocol.schema")));
    assert!(capabilities.contains(&json!("foundation.transport.async_job_contract")));
    assert!(capabilities.contains(&json!("foundation.transport.normalize_async_job_payload")));
    assert!(capabilities.contains(&json!("foundation.transport.retry_delay_seconds_for_job")));
    assert!(capabilities.contains(&json!("foundation.transport.land_request_payload")));
    assert!(capabilities.contains(&json!("foundation.transport.land_freshness_result")));
    assert!(capabilities.contains(&json!("foundation.transport.land_snapshot_alignment")));
    assert!(capabilities.contains(&json!("foundation.identity.repo_scoped_keys")));
    assert!(capabilities.contains(&json!("foundation.identity.row_normalization")));
    assert!(capabilities.contains(&json!("foundation.plan_revision.payload_shaping")));
    assert!(capabilities.contains(&json!("foundation.plan_revision.plan_link_metadata")));
    assert!(capabilities.contains(&json!("foundation.plan_revision.revision_view")));
    assert!(capabilities.contains(&json!("foundation.policy_gate.evaluation")));
    assert!(capabilities.contains(&json!("foundation.policy_gate.input_fingerprint")));
    assert!(capabilities.contains(&json!("foundation.policy_gate.waiver_shaping")));
    assert!(capabilities.contains(&json!("foundation.scheduler.shape_async_job")));
    assert!(capabilities.contains(&json!("foundation.scheduler.admit_async_jobs")));
    assert!(capabilities.contains(&json!("foundation.scheduler.status")));
    assert!(capabilities.contains(&json!("server.workflow_async.runtime")));
    assert!(capabilities.contains(&json!("server.workflow_async.queue_mode")));
    assert!(capabilities.contains(&json!("server.workflow_async.job_payloads")));
    assert!(capabilities.contains(&json!("server.workflow_async.patchset_ci_start_plan")));
    assert!(capabilities.contains(&json!(
        "server.workflow_async.patchset_publish_policy_followup"
    )));
    assert!(capabilities.contains(&json!("server.workflow_artifacts.shaping")));
    assert!(capabilities.contains(&json!("server.workflow_artifacts.review_summary")));
    assert!(capabilities.contains(&json!("server.patchset_ci.schedule_admission")));
    assert!(capabilities.contains(&json!("server.patchset_ci.workflow_ready_evidence")));
    assert!(capabilities.contains(&json!("server.patchset_ci.run")));
    assert!(capabilities.contains(&json!("server.patchset_ci.contract_available")));
    assert!(capabilities.contains(&json!("server.patchset_ci.suite_catalog")));
    assert!(capabilities.contains(&json!("server.patchset_ci.tracking_attestation")));
    assert!(capabilities.contains(&json!("server.patchset_ci.active_state")));
    assert!(capabilities.contains(&json!("server.patchset_ci.status_summary")));
    assert!(capabilities.contains(&json!("server.repo_ci.run")));
    assert!(capabilities.contains(&json!("server.ci_main_seed.prewarm")));
    assert!(capabilities.contains(&json!("server.ci_command_bundle.run")));
    assert!(capabilities.contains(&json!("server.ci_test_shard.plan")));
    assert!(capabilities.contains(&json!("server.ci_test_shard.prepare")));
    assert!(capabilities.contains(&json!("server.ci_test_shard.run")));
    assert!(capabilities.contains(&json!("server.ci_test_shard.cleanup")));
    assert!(capabilities.contains(&json!("middle.ci_status.repository_ci_runs")));
    assert!(!capabilities
        .iter()
        .any(|capability| capability.as_str().is_some_and(|value| value.contains("task_graph"))));
    assert!(capabilities.contains(&json!("middle.queue_read_model.summary")));
    assert!(capabilities.contains(&json!("middle.metrics_read_model.runtime_metrics")));
    assert!(capabilities.contains(&json!("middle.metrics_read_model.operator_metrics")));
    assert!(capabilities.contains(&json!("middle.metrics_read_model.operator_readiness")));
    assert!(capabilities.contains(&json!("middle.workflow_repository_read_model.task_detail")));
    assert!(capabilities.contains(&json!(
        "middle.workflow_repository_read_model.repository_index"
    )));
    assert!(capabilities.contains(&json!(
        "middle.workflow_repository_read_model.repository_detail"
    )));
    assert!(capabilities.contains(&json!(
        "middle.workflow_repository_read_model.repository_worker_status"
    )));
    assert!(capabilities.contains(&json!("middle.secondary_read_model.authority_map")));
    assert!(capabilities.contains(&json!("middle.secondary_read_model.reviewer_inbox")));
    assert!(capabilities.contains(&json!("server.storage.tree_pack_contains_blob_ids")));

    for legacy_capability in [
        "foundation.server_context.path_shaping",
        "foundation.server_context.directory_bootstrap",
        "server.patchset_store.postgres",
        "server.policy_store.postgres",
        "server.review_store.postgres",
        "server.postgres.connection_driver",
        "server.postgres.runtime_probe",
        "server.worker_queue.kernel",
        "server.worker_queue.service",
    ] {
        assert!(
            !capabilities.contains(&json!(legacy_capability)),
            "retired PostgreSQL capability must stay absent: {legacy_capability}"
        );
    }

    let job_types = value
        .get("supported_async_job_types")
        .and_then(JsonValue::as_array)
        .expect("supported async job types should be an array");
    assert!(job_types.contains(&json!("agent.turn.submit")));
    assert!(job_types.contains(&json!("content.gc")));
    assert!(job_types.contains(&json!("land.process")));
}
