use ait_server_core::foundation::patchset_ci::{
    plan_patchset_ci, plan_patchset_ci_dispatch_from_manifest_values,
    plan_patchset_ci_from_manifest_values, workflow_ready_server_evidence_from_manifest_values,
    PatchsetSuiteManifest,
};
use serde_json::{json, Value as JsonValue};

fn passing_tg1_summary(live_count: i64, minimum_count: i64) -> JsonValue {
    json!({
        "status": "pass",
        "validation_status": "pass",
        "live_count": live_count,
        "minimum_count": minimum_count,
        "scheduler": {
            "authority": "server_scheduler",
            "thread_pool_owner": "server",
            "requested_cpu_tokens": 10,
            "admitted_cpu_tokens": 10,
            "runner_parallelism_source": "scheduler_admitted_cpu_tokens"
        },
        "lifecycle": {
            "init_policy": "once_per_run",
            "prewarm_policy": "main_seed_once_per_run",
            "prewarm_once": true,
            "finish_policy": "once_per_run",
            "finish_report_count": 1,
            "cleanup_policy": "all_tests_reclaimed_no_dirty",
            "cleanup_status": "cleaned"
        },
        "thread_pool_shards": {
            "shard_count": 10,
            "shards": []
        },
        "cleanup": {
            "status": "cleaned",
            "policy": "all_tests_reclaimed_no_dirty"
        }
    })
}

#[test]
fn tg1_required_patchset_gate_is_ready_critical_even_when_manifest_is_not_default_blocking() {
    let plan = plan_patchset_ci(&[
        PatchsetSuiteManifest {
            suite_id: "stable_smoke".to_string(),
            display_name: None,
            plane: "patchset".to_string(),
            mode: "gate".to_string(),
            default_blocking: true,
            purpose: None,
            artifact_path: None,
            runner: json!({}),
        },
        PatchsetSuiteManifest {
            suite_id: "tg1_required".to_string(),
            display_name: None,
            plane: "patchset".to_string(),
            mode: "gate".to_string(),
            default_blocking: false,
            purpose: None,
            artifact_path: None,
            runner: json!({}),
        },
        PatchsetSuiteManifest {
            suite_id: "package_smoke".to_string(),
            display_name: None,
            plane: "patchset".to_string(),
            mode: "gate".to_string(),
            default_blocking: true,
            purpose: None,
            artifact_path: None,
            runner: json!({}),
        },
        PatchsetSuiteManifest {
            suite_id: "preflight".to_string(),
            display_name: None,
            plane: "patchset".to_string(),
            mode: "gate".to_string(),
            default_blocking: true,
            purpose: None,
            artifact_path: None,
            runner: json!({}),
        },
    ])
    .expect("patchset CI plan should build");

    assert_eq!(
        plan.selected_suite_ids,
        vec![
            "package_smoke".to_string(),
            "preflight".to_string(),
            "stable_smoke".to_string(),
            "tg1_required".to_string(),
        ]
    );
    assert_eq!(
        plan.ready_critical_suite_ids,
        vec![
            "package_smoke".to_string(),
            "preflight".to_string(),
            "stable_smoke".to_string(),
            "tg1_required".to_string(),
        ]
    );
    assert!(plan.informational_suite_ids.is_empty());
    assert!(plan
        .ready_critical_suite_ids
        .contains(&"tg1_required".to_string()));
    assert_eq!(plan.ready_aggregation.stage, "ready_blocking");
    assert!(plan.ready_aggregation.updates_tests_status);
    assert!(plan.informational_aggregation.is_none());

    assert_eq!(plan.workflow_ready_foreground_jobs.len(), 5);
    assert_eq!(
        plan.workflow_ready_foreground_jobs
            .iter()
            .map(|job| (
                job.job_type.as_str(),
                job.suite_id.as_deref(),
                job.stage.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("patchset.ci", Some("package_smoke"), None),
            ("patchset.ci", Some("preflight"), None),
            ("patchset.ci", Some("stable_smoke"), None),
            ("patchset.ci", Some("tg1_required"), None),
            ("patchset.ci.aggregate", None, Some("ready_blocking")),
        ]
    );
    assert!(plan
        .workflow_ready_foreground_jobs
        .iter()
        .all(|job| job.workflow_ready_foreground));
    assert!(plan
        .workflow_ready_foreground_jobs
        .iter()
        .any(|job| job.suite_ids.contains(&"tg1_required".to_string())));

    assert!(plan.background_jobs.is_empty());
}

#[test]
fn patchset_ci_plan_filters_non_patchset_gate_suites() {
    let plan = plan_patchset_ci_from_manifest_values(&[
        json!({
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true
        }),
        json!({
            "suite_id": "recent_regression",
            "plane": "patchset",
            "mode": "diagnostic",
            "default_blocking": false
        }),
        json!({
            "suite_id": "full_repo",
            "plane": "nightly",
            "mode": "gate",
            "default_blocking": true
        }),
    ])
    .expect("manifest values should parse");

    assert_eq!(plan.selected_suite_ids, vec!["preflight".to_string()]);
    assert_eq!(plan.blocking_suite_ids, vec!["preflight".to_string()]);
    assert!(plan.informational_suite_ids.is_empty());
    assert!(plan.informational_aggregation.is_none());
    assert_eq!(
        plan.workflow_ready_foreground_jobs
            .iter()
            .map(|job| (
                job.job_type.as_str(),
                job.suite_id.as_deref(),
                job.stage.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("patchset.ci", Some("preflight"), None),
            ("patchset.ci.aggregate", None, Some("ready_blocking")),
        ]
    );
    assert!(plan.background_jobs.is_empty());
}

#[test]
fn tg1_only_patchset_gate_still_blocks_workflow_ready() {
    let plan = plan_patchset_ci(&[PatchsetSuiteManifest {
        suite_id: "tg1_required".to_string(),
        display_name: None,
        plane: "patchset".to_string(),
        mode: "gate".to_string(),
        default_blocking: false,
        purpose: None,
        artifact_path: None,
        runner: json!({}),
    }])
    .expect("tg1-only plan should build");

    assert_eq!(
        plan.ready_critical_suite_ids,
        vec!["tg1_required".to_string()]
    );
    assert_eq!(
        plan.workflow_ready_foreground_jobs
            .iter()
            .map(|job| (
                job.job_type.as_str(),
                job.suite_id.as_deref(),
                job.stage.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("patchset.ci", Some("tg1_required"), None),
            ("patchset.ci.aggregate", None, Some("ready_blocking")),
        ]
    );
    assert!(plan.background_jobs.is_empty());
}

#[test]
fn patchset_ci_dispatch_blocks_aggregation_until_suite_results_exist() {
    let request = json!({
        "repo_name": "ait",
        "patchset_id": "RP-1",
        "revision_snapshot_id": "SNP-1",
        "scope": "all"
    });
    let dispatch = plan_patchset_ci_dispatch_from_manifest_values(
        &[json!({
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true
        })],
        request.as_object().expect("request should be an object"),
    )
    .expect("dispatch plan should build");

    assert_eq!(dispatch.queued_jobs.len(), 1);
    assert_eq!(
        dispatch.queued_jobs[0].job.suite_id.as_deref(),
        Some("preflight")
    );
    assert_eq!(dispatch.blocked_jobs.len(), 1);
    assert_eq!(
        dispatch.blocked_jobs[0].job.stage.as_deref(),
        Some("ready_blocking")
    );
    assert!(dispatch.blocked_jobs[0]
        .reason
        .contains("waits for suite results: preflight"));
}

#[test]
fn patchset_ci_dispatch_skips_completed_suites_and_releases_aggregation() {
    let request = json!({
        "repo_name": "ait",
        "patchset_id": "RP-1",
        "revision_snapshot_id": "SNP-1",
        "scope": "all",
        "completed_suite_ids": ["preflight"]
    });
    let dispatch = plan_patchset_ci_dispatch_from_manifest_values(
        &[json!({
            "suite_id": "preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true
        })],
        request.as_object().expect("request should be an object"),
    )
    .expect("dispatch plan should build");

    assert_eq!(dispatch.queued_jobs.len(), 1);
    assert_eq!(
        dispatch.queued_jobs[0].job.job_type,
        "patchset.ci.aggregate"
    );
    assert_eq!(
        dispatch.queued_jobs[0].payload["suite_ids"],
        json!(["preflight"])
    );
    assert!(dispatch.blocked_jobs.is_empty());
}

#[test]
fn workflow_ready_server_evidence_requires_tg1_and_rejects_python_runners() {
    let manifests = vec![
        json!({
            "suite_id": "preflight",
            "display_name": "Preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {"kind": "server_builtin"}
        }),
        json!({
            "suite_id": "tg1_required",
            "display_name": "TG-1 Required",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": false,
            "runner": {"kind": "server_tg1_required"}
        }),
    ];
    let evidence = workflow_ready_server_evidence_from_manifest_values(
        &manifests,
        &json!({
            "preflight": {
                "runner_kind": "rust_server_ci",
                "status": "pass"
            },
            "tg1_required": {
                "runner_kind": "rust_server_tg1_required",
                "status": "pass",
                "tg1_required_summary": passing_tg1_summary(33, 33)
            }
        }),
    )
    .expect("server evidence should aggregate");

    assert_eq!(evidence["tests_status"], json!("pass"));
    assert_eq!(
        evidence["selected_suite_ids"],
        json!(["preflight", "tg1_required"])
    );
    assert_eq!(
        evidence["suite_results"][0]["runner_kind"],
        json!("rust_server_ci")
    );
    assert_eq!(
        evidence["server_ci_gate"]["python_server_ci_executor"],
        json!(false)
    );
    assert_eq!(evidence["server_ci_gate"]["tg1_required"], json!(true));
    assert_eq!(
        evidence["suite_results"][1]["tg1_required_summary"]["live_count"],
        json!(33)
    );

    let missing_tg1 = workflow_ready_server_evidence_from_manifest_values(
        &manifests,
        &json!({
            "preflight": {
                "runner_kind": "rust_server_ci",
                "status": "pass"
            }
        }),
    )
    .expect_err("missing tg1 evidence should fail closed");
    assert!(missing_tg1.contains("tg1_required"));

    let python_runner = workflow_ready_server_evidence_from_manifest_values(
        &manifests,
        &json!({
            "preflight": {
                "runner_kind": "python_command_bundle",
                "status": "pass"
            },
            "tg1_required": {
                "runner_kind": "rust_server_tg1_required",
                "status": "pass",
                "tg1_required_summary": passing_tg1_summary(33, 33)
            }
        }),
    )
    .expect_err("python runners should be rejected");
    assert!(python_runner.contains("not allowed"));

    let insufficient_tg1 = workflow_ready_server_evidence_from_manifest_values(
        &manifests,
        &json!({
            "preflight": {
                "runner_kind": "rust_server_ci",
                "status": "pass"
            },
            "tg1_required": {
                "runner_kind": "rust_server_tg1_required",
                "status": "pass",
                "tg1_required_summary": passing_tg1_summary(32, 33)
            }
        }),
    )
    .expect_err("insufficient tg1 membership should fail closed");
    assert!(insufficient_tg1.contains("expected at least 33"));
}

#[test]
fn workflow_ready_server_evidence_rejects_command_bundle_tg1_without_server_summary() {
    let manifests = vec![
        json!({
            "suite_id": "preflight",
            "display_name": "Preflight",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": true,
            "runner": {"kind": "server_builtin"}
        }),
        json!({
            "suite_id": "tg1_required",
            "display_name": "TG-1 Required",
            "plane": "patchset",
            "mode": "gate",
            "default_blocking": false,
            "runner": {"kind": "command_bundle", "commands": ["printf ran"]}
        }),
    ];

    let error = workflow_ready_server_evidence_from_manifest_values(
        &manifests,
        &json!({
            "preflight": {
                "runner_kind": "rust_server_ci",
                "status": "pass"
            },
            "tg1_required": {
                "runner_kind": "rust_server_ci",
                "status": "pass"
            }
        }),
    )
    .expect_err("command-bundle tg1 evidence should fail closed");

    assert!(error.contains("server_tg1_required"));
}

#[test]
fn patchset_ci_plan_rejects_empty_selected_suite_id() {
    let error = plan_patchset_ci(&[PatchsetSuiteManifest {
        suite_id: " ".to_string(),
        display_name: None,
        plane: "patchset".to_string(),
        mode: "gate".to_string(),
        default_blocking: true,
        purpose: None,
        artifact_path: None,
        runner: json!({}),
    }])
    .expect_err("empty suite id should fail");

    assert_eq!(
        error,
        "patchset CI suite manifest requires `suite_id`.".to_string()
    );
}
