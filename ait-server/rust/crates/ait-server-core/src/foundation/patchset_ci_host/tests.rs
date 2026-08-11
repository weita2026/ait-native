use serde_json::json;

use super::{
    patchset_ci_active_state_json, patchset_ci_completion_json,
    patchset_ci_contract_available_json, patchset_ci_status_summary_json,
    patchset_ci_suite_catalog_json,
};

#[test]
fn contract_available_ignores_non_patch_ci_catalog_paths() {
    let value = patchset_ci_contract_available_json(&json!({
        "snapshot_paths": ["README.md", "./ci/not_patch_ci.json"]
    }))
    .unwrap();
    assert_eq!(value, json!({"available": false}));
}

#[test]
fn contract_available_detects_single_patch_ci_catalog() {
    let value = patchset_ci_contract_available_json(&json!({
        "snapshot_paths": ["README.md", "./ci/patch_ci.json"]
    }))
    .unwrap();
    assert_eq!(value, json!({"available": true}));
}

#[test]
fn suite_catalog_ignores_non_patch_ci_catalog_paths() {
    let value = patchset_ci_suite_catalog_json(&json!({
        "snapshot_files": [
            {
                "path": "README.md",
                "content": "# docs"
            },
            {
                "path": "./ci/not_patch_ci.json",
                "content": r#"{
                        "suite_id": "preflight",
                        "display_name": "Preflight",
                        "plane": "patchset",
                        "mode": "gate",
                        "default_blocking": true,
                        "runner": {"kind": "command_bundle", "commands": ["cargo test"]}
                    }"#
            }
        ]
    }))
    .unwrap();

    assert_eq!(
        value["contract"],
        json!("ait.server.patchset_ci.suite_catalog.v1")
    );
    assert_eq!(value["suite_count"], json!(0));
    assert_eq!(value["catalog_paths"], json!([]));
    assert_eq!(value["suites"], json!([]));
}

#[test]
fn suite_catalog_loads_patch_ci_catalog() {
    let value = patchset_ci_suite_catalog_json(&json!({
        "snapshot_files": [
            {
                "path": "ci/patch_ci.json",
                "content": r#"{
                        "schema_version": 1,
                        "suites": [
                            {
                                "suite_id": "rust_core",
                                "plane": "patchset",
                                "mode": "gate",
                                "default_blocking": true,
                                "runner": {"kind": "command_bundle", "commands": ["cargo test"]}
                            }
                        ]
                    }"#
            }
        ]
    }))
    .unwrap();

    assert_eq!(value["suite_count"], json!(1));
    assert_eq!(value["catalog_paths"], json!(["ci/patch_ci.json"]));
    assert_eq!(value["suites"][0]["suite_id"], json!("rust_core"));
    assert_eq!(
        value["suites"][0]["_artifact_path"],
        json!("ci/patch_ci.json")
    );
}

#[test]
fn suite_catalog_rejects_retired_inline_binary_files() {
    let retired_binary_key = ["content", "base64"].join("_");
    let mut snapshot_file = json!({
        "path": "ci/patch_ci.json"
    });
    snapshot_file[retired_binary_key.as_str()] = json!("e30=");

    let error = patchset_ci_suite_catalog_json(&json!({
        "snapshot_files": [snapshot_file]
    }))
    .expect_err("retired inline binary snapshot materialization should fail closed");

    assert!(error.contains("pack-backed materialization"));
}

#[test]
fn suite_catalog_ignores_configured_catalog_path() {
    let value = patchset_ci_suite_catalog_json(&json!({
        "snapshot_files": [
            {
                "path": "ci/config.contract.json",
                "content": r#"{"suite_manifest_path": "ci/custom/catalog.json"}"#
            },
            {
                "path": "ci/custom/catalog.json",
                "content": r#"[
                        {
                            "suite_id": "custom_gate",
                            "plane": "patchset",
                            "mode": "gate",
                            "default_blocking": true,
                            "runner": {"kind": "command_bundle", "commands": ["cargo test"]}
                        }
                    ]"#
            }
        ]
    }))
    .unwrap();

    assert_eq!(value["suite_count"], json!(0));
    assert_eq!(value["catalog_paths"], json!([]));
    assert_eq!(value["suites"], json!([]));
}

#[test]
fn completion_shapes_patchset_ci_state_without_attestation_payload() {
    let value = patchset_ci_completion_json(&json!({
        "patchset": {
            "patchset_id": "RP-1",
            "ci_run_seq": 3,
            "base_snapshot_id": "SNP-BASE",
            "revision_snapshot_id": "SNP-REV",
        },
        "suites": [
            {"suite_id": "preflight", "default_blocking": true},
            {"suite_id": "cargo_fmt", "default_blocking": true}
        ],
        "tests_status": "fail",
        "job_state": "succeeded",
        "suite_results": [
            {"suite_id": "preflight", "status": "fail"},
            {"suite_id": "cargo_fmt", "status": "pass"}
        ],
        "blocking_failures": ["preflight"]
    }))
    .unwrap();

    assert_eq!(value["patchset_id"], json!("RP-1"));
    assert_eq!(value["ci_run_seq"], json!(3));
    assert_eq!(value["selected_suite_count"], json!(2));
    assert_eq!(value["suite_result_count"], json!(2));
    assert_eq!(value["blocking_failure_count"], json!(1));
    assert_eq!(value["overall_status"], json!("fail"));
    assert_eq!(value["tests_status"], json!("fail"));
    assert_eq!(value["lint_status"], json!("pass"));
    assert_eq!(value.as_object().unwrap().len(), 8);
    assert!(value.get("job_state").is_none());
    assert!(value.get("suite_results").is_none());
    assert!(value.get("selected_suite_ids").is_none());
    assert!(value.get("detail").is_none());
    assert!(value.get("evaluation_summary").is_none());
}

#[test]
fn completion_rejects_nonterminal_job_state() {
    let error = patchset_ci_completion_json(&json!({
        "patchset": {"patchset_id": "RP-1", "ci_run_seq": 3},
        "suites": [{"suite_id": "preflight"}],
        "tests_status": "pending",
        "job_state": "queued",
        "suite_results": [],
        "blocking_failures": []
    }))
    .expect_err("queued state must not mutate the Patchset CI completion block");

    assert!(error.contains("requires succeeded job_state"));
}

#[test]
fn active_state_requires_matching_profile_and_live_inline_thread() {
    let active = patchset_ci_active_state_json(&json!({
        "patchset_id": "RP-1",
        "requested_execution_profile": "full",
        "queue_mode": "inline",
        "inline_thread_alive": true,
        "patchset_ci": {
            "job_state": "running",
            "tests_status": "pending",
            "execution_profile": "full",
            "trigger": "manual_rerun"
        }
    }))
    .unwrap();
    assert_eq!(active["active_state"]["patchset_id"], json!("RP-1"));

    let inactive = patchset_ci_active_state_json(&json!({
        "patchset_id": "RP-1",
        "requested_execution_profile": "workflow_ready_foreground",
        "queue_mode": "inline",
        "inline_thread_alive": true,
        "patchset_ci": {
            "job_state": "running",
            "tests_status": "pending",
            "execution_profile": "full"
        }
    }))
    .unwrap();
    assert!(inactive["active_state"].is_null());
}

#[test]
fn status_summary_uses_latest_job_fallback_and_tg1_summary() {
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-1",
        "change_id": "RC-1",
        "repo_name": "ait",
        "ci_completed_at_s": 1_783_814_400_u64,
        "embedded_patchset_ci": {
            "run_seq": 1,
            "completed_at_s": 1_783_814_400_u64,
            "overall_status": "pass",
            "tests_status": "pass",
            "selected_suite_count": 2,
            "suite_result_count": 2,
            "blocking_failure_count": 0
        },
        "jobs": [{
            "job_id": 11,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "payload": {"patchset_id": "RP-1"},
            "result": {
                "tests_status": "pass",
                "selected_suite_ids": ["preflight", "tg1_required"],
                "suite_results": [
                    {
                        "suite_id": "tg1_required",
                        "status": "pass",
                        "tg1_required_summary": {
                            "status": "pass",
                            "live_count": 24,
                            "minimum_count": 24
                        }
                    }
                ]
            }
        }],
        "recent_limit": 10
    }))
    .unwrap();

    assert_eq!(value["available"], json!(true));
    assert_eq!(value["ci_run_seq"], json!(1));
    assert_eq!(value["latest_job"]["job_id"], json!(11));
    assert_eq!(value["tg1_required"]["live_count"], json!(24));
    assert_eq!(value["rerun"]["cli"], json!("ait patchset rerun-ci RP-1"));
}

#[test]
fn status_summary_readiness_projection_omits_large_diagnostic_bodies() {
    let large_detail = "x".repeat(2 * 1024 * 1024);
    let request = json!({
        "patchset_id": "RP-READINESS",
        "change_id": "RC-READINESS",
        "repo_name": "ait-core",
        "projection": "readiness",
        "embedded_patchset_ci": {
            "run_seq": 7,
            "completed_at_s": 1_783_814_400_u64,
            "overall_status": "pass",
            "tests_status": "pass",
            "lint_status": "none",
            "selected_suite_count": 1,
            "suite_result_count": 1,
            "blocking_failure_count": 0
        },
        "jobs": [{
            "job_id": 73,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "diagnostic_status": "succeeded",
            "payload": {
                "patchset_id": "RP-READINESS",
                "suite_ids": ["rust_core"],
                "runtime_payload": {"materialization": large_detail.clone()}
            },
            "result": {
                "tests_status": "pass",
                "selected_suite_ids": ["rust_core"],
                "suite_results": [{
                    "suite_id": "rust_core",
                    "status": "pass",
                    "log": large_detail.clone()
                }],
                "attestation_update": {"detail": large_detail}
            }
        }],
        "recent_limit": 1
    });

    let value = patchset_ci_status_summary_json(&request).unwrap();
    let encoded = serde_json::to_vec(&value).unwrap();

    assert_eq!(
        value["contract"],
        json!("ait.server.patchset_ci.readiness.v1")
    );
    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["ci_run_seq"], json!(7));
    assert_eq!(value["suite_result_count"], json!(1));
    assert_eq!(value["latest_job"]["job_id"], json!(73));
    assert_eq!(
        value["latest_job"]["selected_suite_ids"],
        json!(["rust_core"])
    );
    assert!(value["latest_job"].get("payload").is_none());
    assert!(value["latest_job"].get("result").is_none());
    assert!(value.get("suite_results").is_none());
    assert!(
        encoded.len() < 4096,
        "readiness response was {} bytes",
        encoded.len()
    );

    let mut default_request = request.clone();
    default_request
        .as_object_mut()
        .unwrap()
        .remove("projection");
    for _ in 0..32 {
        let default_value = patchset_ci_status_summary_json(&default_request).unwrap();
        let default_encoded = serde_json::to_vec(&default_value).unwrap();
        assert_eq!(default_value["detail_bounded"], json!(true));
        assert!(default_value["latest_job"]["payload"]
            .get("runtime_payload")
            .is_none());
        assert!(default_value["latest_job"]["result"]
            .get("suite_results")
            .is_none());
        assert!(default_value["latest_job"]["result"]
            .get("attestation_update")
            .is_none());
        assert!(default_value["latest_job"]["suite_results"][0]
            .get("log")
            .is_none());
        assert!(
            default_encoded.len() < 32 * 1024,
            "bounded diagnostic response was {} bytes",
            default_encoded.len()
        );
    }
}

#[test]
fn status_summary_caps_recent_jobs_before_response_shaping() {
    let jobs = (1..=40)
        .rev()
        .map(|job_id| {
            json!({
                "job_id": job_id,
                "job_type": "patchset.ci",
                "state": "succeeded",
                "payload": {"patchset_id": "RP-LIMIT", "suite_ids": ["rust_core"]},
                "result": {"tests_status": "pass", "suite_result_count": 1}
            })
        })
        .collect::<Vec<_>>();
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-LIMIT",
        "change_id": "RC-LIMIT",
        "repo_name": "ait-core",
        "jobs": jobs,
        "recent_limit": 1_000_000
    }))
    .unwrap();

    assert_eq!(value["recent_limit_applied"], json!(20));
    assert_eq!(value["recent_jobs"].as_array().unwrap().len(), 20);
}

#[test]
fn status_summary_rejects_unknown_projection() {
    let error = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-READINESS",
        "change_id": "RC-READINESS",
        "repo_name": "ait-core",
        "projection": "logs"
    }))
    .unwrap_err();

    assert!(error.contains("Unsupported patchset CI status projection `logs`"));
}

#[test]
fn status_summary_does_not_treat_attached_only_job_as_pass() {
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-3",
        "change_id": "RC-3",
        "repo_name": "ait-core",
        "jobs": [{
            "job_id": 2625,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "diagnostic_status": "succeeded",
            "attempt_count": 1,
            "max_attempts": 3,
            "attempts_remaining": 2,
            "payload": {
                "patchset_id": "RP-3",
                "repo_name": "ait-core",
                "revision_snapshot_id": "SNP-REV",
                "suite_ids": ["rust_core"]
            },
            "result": {
                "status": "attached",
                "executor": {
                    "kind": "attached",
                    "job_id": "2625",
                    "active_job_id": "2623",
                    "singleflight_key": "patchset.ci:RP-3:rust_core:SNP-REV"
                }
            }
        }],
        "recent_limit": 10
    }))
    .unwrap();

    assert_eq!(value["tests_status"], json!("pending"));
    assert_eq!(value["latest_job"]["tests_status"], json!("pending"));
    assert_eq!(value["latest_job"]["suite_results"], json!([]));
    assert!(value["ci_completed_at_s"].is_null());
}

#[test]
fn readiness_summary_does_not_treat_resultless_succeeded_job_as_pass() {
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-RESULTLESS",
        "change_id": "RC-RESULTLESS",
        "repo_name": "ait-server",
        "projection": "readiness",
        "jobs": [{
            "job_id": 4734,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "payload": {
                "patchset_id": "RP-RESULTLESS",
                "suite_ids": ["rust_core"]
            },
            "result": {}
        }],
        "recent_limit": 1
    }))
    .unwrap();

    assert_eq!(value["tests_status"], json!("pending"));
    assert_eq!(value["latest_job"]["tests_status"], json!("pending"));
    assert_eq!(value["suite_result_count"], json!(0));
    assert_eq!(value["has_runnable_evidence"], json!(false));
}

#[test]
fn readiness_summary_does_not_treat_postgres_pass_as_completed_patchset_state() {
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-UNCOMMITTED",
        "change_id": "RC-UNCOMMITTED",
        "repo_name": "ait-server",
        "projection": "readiness",
        "embedded_patchset_ci": {
            "run_seq": 1,
            "completed_at_s": 0,
            "overall_status": "none",
            "tests_status": "none",
            "lint_status": "none",
            "selected_suite_count": 0,
            "suite_result_count": 0,
            "blocking_failure_count": 0
        },
        "jobs": [{
            "job_id": 5054,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "payload": {
                "patchset_id": "RP-UNCOMMITTED",
                "suite_ids": ["rust_core"]
            },
            "result": {
                "tests_status": "pass",
                "selected_suite_ids": ["rust_core"],
                "suite_result_count": 1,
                "blocking_failure_count": 0
            }
        }],
        "recent_limit": 1
    }))
    .unwrap();

    assert_eq!(value["tests_status"], json!("pending"));
    assert_eq!(value["ci_run_seq"], json!(1));
    assert!(value["ci_completed_at_s"].is_null());
    assert_eq!(value["suite_result_count"], json!(1));
    assert_eq!(value["has_runnable_evidence"], json!(false));
}

#[test]
fn readiness_summary_preserves_explicit_blocking_ci_failure() {
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-FAIL",
        "change_id": "RC-FAIL",
        "repo_name": "ait-server",
        "projection": "readiness",
        "jobs": [{
            "job_id": 4734,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "payload": {
                "patchset_id": "RP-FAIL",
                "suite_ids": ["rust_core"]
            },
            "result": {
                "tests_status": "fail",
                "selected_suite_ids": ["rust_core"],
                "blocking_failures": ["rust_core"],
                "suite_result_count": 1,
                "blocking_failure_count": 1
            }
        }],
        "recent_limit": 1
    }))
    .unwrap();

    assert_eq!(value["tests_status"], json!("fail"));
    assert_eq!(value["latest_job"]["tests_status"], json!("fail"));
    assert_eq!(value["suite_result_count"], json!(1));
    assert_eq!(value["blocking_failure_count"], json!(1));
    assert_eq!(value["has_runnable_evidence"], json!(false));
}

#[test]
fn status_summary_ignores_non_patchset_ci_jobs() {
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-4",
        "change_id": "RC-4",
        "repo_name": "ait-core",
        "jobs": [
            {
                "job_id": 31,
                "job_type": "main-seed.refresh",
                "state": "succeeded",
                "payload": {
                    "patchset_id": "RP-4",
                    "repo_name": "ait-core"
                },
                "result": {
                    "status": "ok"
                }
            },
            {
                "job_id": 30,
                "job_type": "patchset.ci",
                "state": "succeeded",
                "payload": {
                    "patchset_id": "RP-4",
                    "repo_name": "ait-core",
                    "suite_ids": ["rust_core"]
                },
                "result": {
                    "tests_status": "pass",
                    "suite_results": [
                        {"suite_id": "rust_core", "status": "pass"}
                    ]
                }
            }
        ],
        "recent_limit": 10
    }))
    .unwrap();

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["latest_job"]["job_id"], json!(30));
    assert_eq!(value["latest_job"]["job_type"], json!("patchset.ci"));
    assert_eq!(value["recent_jobs"].as_array().unwrap().len(), 1);
}

#[test]
fn status_summary_includes_patchset_ci_aggregate_jobs() {
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-6",
        "change_id": "RC-6",
        "repo_name": "ait-core",
        "jobs": [
            {
                "job_id": 52,
                "job_type": "patchset.ci.aggregate",
                "state": "succeeded",
                "payload": {
                    "patchset_id": "RP-6",
                    "repo_name": "ait-core",
                    "suite_ids": ["rust_core"],
                    "stage": "ready_blocking"
                },
                "result": {
                    "tests_status": "pass",
                    "selected_suite_ids": ["rust_core"],
                    "blocking_failures": [],
                    "suite_results": [
                        {"suite_id": "rust_core", "status": "pass", "blocking": true}
                    ]
                }
            },
            {
                "job_id": 51,
                "job_type": "patchset.ci",
                "state": "succeeded",
                "payload": {
                    "patchset_id": "RP-6",
                    "repo_name": "ait-core",
                    "suite_ids": ["rust_core"]
                },
                "result": {
                    "status": "attached"
                }
            }
        ],
        "recent_limit": 10
    }))
    .unwrap();

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["latest_job"]["job_id"], json!(52));
    assert_eq!(
        value["latest_job"]["job_type"],
        json!("patchset.ci.aggregate")
    );
    assert_eq!(value["recent_jobs"].as_array().unwrap().len(), 2);
}

#[test]
fn status_summary_prefers_embedded_patchset_state_over_landed_stale_rerun() {
    let value = patchset_ci_status_summary_json(&json!({
        "patchset_id": "RP-5",
        "change_id": "RC-5",
        "repo_name": "ait-server",
        "ci_completed_at_s": 1_783_299_264_u64,
        "embedded_patchset_ci": {
            "run_seq": 1,
            "completed_at_s": 1_783_299_264_u64,
            "overall_status": "pass",
            "tests_status": "pass",
            "selected_suite_count": 1,
            "suite_result_count": 1,
            "blocking_failure_count": 0
        },
        "jobs": [{
            "job_id": 44,
            "job_type": "patchset.ci",
            "state": "succeeded",
            "payload": {
                "patchset_id": "RP-5",
                "repo_name": "ait-server"
            },
            "result": {
                "tests_status": "fail",
                "blocking_failures": [
                    {
                        "kind": "BASE_STALE_AFTER_LAND",
                        "patchset_id": "RP-5"
                    }
                ]
            }
        }],
        "recent_limit": 10
    }))
    .unwrap();

    assert_eq!(value["tests_status"], json!("pass"));
    assert_eq!(value["blocking_failures"], json!([]));
    assert_eq!(value["suite_results"], json!([]));
    assert_eq!(value["suite_result_count"], json!(1));
    assert_eq!(value["latest_job"]["tests_status"], json!("fail"));
}

#[test]
fn status_summary_surfaces_reset_after_land_notice() {
    let value = patchset_ci_status_summary_json(&json!({
            "patchset_id": "RP-2",
            "change_id": "RC-2",
            "repo_name": "ait-core",
            "jobs": [{
                "job_id": 1334,
                "job_type": "patchset.ci",
                "state": "queued",
                "diagnostic_status": "retry_pending",
                "last_error": "Patchset CI reset after land moved ait-core:main from SNP-OLD to SNP-NEW",
                "retry_pending": true,
                "attempt_count": 0,
                "max_attempts": 3,
                "attempts_remaining": 3,
                "payload": {
                    "patchset_id": "RP-2",
                    "change_id": "RC-2",
                    "repo_name": "ait-core"
                },
                "result": {}
            }],
            "recent_limit": 10
        }))
        .unwrap();

    assert_eq!(value["tests_status"], json!("pending"));
    assert_eq!(
        value["latest_job"]["last_error"],
        json!("Patchset CI reset after land moved ait-core:main from SNP-OLD to SNP-NEW")
    );
    assert_eq!(value["latest_job"]["retry_pending"], json!(true));
    assert_eq!(
        value["status_notice"]["kind"],
        json!("patchset_ci_reset_after_land")
    );
    assert_eq!(
        value["status_notice"]["recommended_action"],
        json!("rebase_patchset_to_latest_main")
    );
    assert_eq!(
        value["recommended_action"],
        json!("rebase_patchset_to_latest_main")
    );
    assert_eq!(
        value["status_notice"]["tests_status_semantics"],
        json!("pending_not_test_failure")
    );
}
