use ait_server_core::foundation::scheduler::{
    admit_next, scheduler_job_spec_from_async_job_with_policy, SchedulerAdmissionDecision,
    SchedulerDeploymentPosture, SchedulerJobClass, SchedulerPolicy, SchedulerQueuedJob,
    SchedulerRunningJob,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

fn payload(value: JsonValue) -> JsonMap<String, JsonValue> {
    value
        .as_object()
        .cloned()
        .expect("payload should be object")
}

fn test_policy() -> SchedulerPolicy {
    SchedulerPolicy::local_co_resident_10_core_default()
}

fn queued(
    job_id: &str,
    ordinal: usize,
    job_type: &str,
    payload_value: JsonValue,
) -> SchedulerQueuedJob {
    SchedulerQueuedJob {
        job_id: job_id.to_string(),
        spec: scheduler_job_spec_from_async_job_with_policy(
            job_type,
            &payload(payload_value),
            &test_policy(),
        )
        .expect("scheduler job should shape"),
        queued_ordinal: ordinal,
    }
}

fn running(job_id: &str, job_type: &str, payload_value: JsonValue) -> SchedulerRunningJob {
    SchedulerRunningJob {
        job_id: job_id.to_string(),
        spec: scheduler_job_spec_from_async_job_with_policy(
            job_type,
            &payload(payload_value),
            &test_policy(),
        )
        .expect("scheduler job should shape"),
    }
}

#[test]
fn local_co_resident_10_core_policy_exposes_ten_ci_tokens() {
    let policy = SchedulerPolicy::local_co_resident_10_core_default();

    assert_eq!(policy.host_cpu_cores, 10);
    assert_eq!(policy.reserved_local_cpu_cores, 0);
    assert_eq!(policy.global_cpu_tokens, 10);
    assert_eq!(policy.ci_full_shared_cpu_tokens, 10);
    assert_eq!(policy.full_test_cpu_tokens, 10);
    assert_eq!(policy.full_test_job_cpu_tokens, 10);
}

#[test]
fn dedicated_10_core_policy_exposes_ten_ci_tokens() {
    let policy = SchedulerPolicy::dedicated_server_10_core_default();

    assert_eq!(policy.host_cpu_cores, 10);
    assert_eq!(policy.reserved_local_cpu_cores, 0);
    assert_eq!(policy.global_cpu_tokens, 10);
    assert_eq!(policy.ci_full_shared_cpu_tokens, 10);
    assert_eq!(policy.full_test_cpu_tokens, 10);
    assert_eq!(policy.full_test_job_cpu_tokens, 10);
}

#[test]
fn local_co_resident_full_test_override_allows_eight_token_jobs() {
    let policy = SchedulerPolicy::for_host_cpu_cores_with_full_test_job_cpu_tokens(
        10,
        SchedulerDeploymentPosture::LocalCoResident,
        Some(8),
    );
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait-core",
            "patchset_id": "RCP-1",
            "suite_ids": ["full_repo"],
            "revision_snapshot_id": "SNP-1"
        })),
        &policy,
    )
    .expect("patchset full-test suite should shape");

    assert_eq!(policy.global_cpu_tokens, 10);
    assert_eq!(policy.full_test_cpu_tokens, 10);
    assert_eq!(policy.full_test_job_cpu_tokens, 8);
    assert_eq!(spec.job_class, SchedulerJobClass::FullTest);
    assert_eq!(spec.cpu_tokens, 8);
}

#[test]
fn full_test_override_is_capped_to_scheduler_pool() {
    let policy = SchedulerPolicy::for_host_cpu_cores_with_full_test_job_cpu_tokens(
        10,
        SchedulerDeploymentPosture::LocalCoResident,
        Some(99),
    );

    assert_eq!(policy.global_cpu_tokens, 10);
    assert_eq!(policy.full_test_cpu_tokens, 10);
    assert_eq!(policy.full_test_job_cpu_tokens, 10);
}

#[test]
fn local_co_resident_small_host_scales_down_without_starving_server() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(4, SchedulerDeploymentPosture::LocalCoResident);

    assert_eq!(policy.host_cpu_cores, 4);
    assert_eq!(policy.reserved_local_cpu_cores, 0);
    assert_eq!(policy.global_cpu_tokens, 4);
    assert_eq!(policy.ci_full_shared_cpu_tokens, 4);
    assert_eq!(policy.full_test_cpu_tokens, 4);
    assert_eq!(policy.full_test_job_cpu_tokens, 4);
}

#[test]
fn dedicated_server_large_host_expands_ci_and_full_test_budgets() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer);

    assert_eq!(policy.host_cpu_cores, 32);
    assert_eq!(policy.reserved_local_cpu_cores, 0);
    assert_eq!(policy.global_cpu_tokens, 32);
    assert_eq!(policy.ci_full_shared_cpu_tokens, 32);
    assert_eq!(policy.full_test_cpu_tokens, 32);
    assert_eq!(policy.full_test_job_cpu_tokens, 10);
}

#[test]
fn patchset_full_test_consumes_ten_tokens_and_full_pool() {
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "full",
            "revision_snapshot_id": "SNP-1"
        })),
        &test_policy(),
    )
    .expect("patchset full test should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::FullTest);
    assert_eq!(spec.cpu_tokens, 10);
    assert!(spec
        .token_pools
        .contains(&"ci_full_shared_cpu_tokens".to_string()));
    assert!(spec
        .token_pools
        .contains(&"full_test_cpu_tokens".to_string()));
    assert_eq!(
        spec.singleflight_key,
        Some("patchset.ci:RP-1:full:SNP-1".to_string())
    );
}

#[test]
fn main_seed_refresh_uses_scheduler_full_test_tokens() {
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "main-seed.refresh",
        &payload(json!({
            "repo_name": "ait-server",
            "snapshot_id": "SNP-LANDED",
            "patchset_id": "RSEP-1",
            "previous_snapshot_id": "SNP-BASE",
        })),
        &test_policy(),
    )
    .expect("main seed refresh should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::FullTest);
    assert_eq!(spec.cpu_tokens, 10);
    assert!(spec
        .token_pools
        .contains(&"ci_full_shared_cpu_tokens".to_string()));
    assert!(spec
        .token_pools
        .contains(&"full_test_cpu_tokens".to_string()));
    assert_eq!(
        spec.singleflight_key,
        Some("main-seed.refresh:repo:ait-server:SNP-LANDED".to_string())
    );
    assert_eq!(spec.write_keys, vec!["repo:ait-server:main-seed"]);
}

#[test]
fn main_seed_refresh_waits_for_running_same_repo_refresh() {
    let queued = vec![queued(
        "refresh-2",
        0,
        "main-seed.refresh",
        json!({
            "repo_name": "ait-server",
            "snapshot_id": "SNP-2",
            "patchset_id": "RSEP-2",
        }),
    )];
    let running = vec![running(
        "refresh-1",
        "main-seed.refresh",
        json!({
            "repo_name": "ait-server",
            "snapshot_id": "SNP-1",
            "patchset_id": "RSEP-1",
        }),
    )];

    assert!(matches!(
        admit_next(&queued, &running, &test_policy()),
        SchedulerAdmissionDecision::Wait { .. }
    ));
}

#[test]
fn patchset_ci_suite_ids_consume_full_test_tokens_when_any_suite_is_full() {
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_ids": ["preflight", "full"],
            "revision_snapshot_id": "SNP-1"
        })),
        &test_policy(),
    )
    .expect("patchset CI suite_ids should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::FullTest);
    assert_eq!(spec.cpu_tokens, 10);
    assert_eq!(
        spec.singleflight_key,
        Some("patchset.ci:RP-1:preflight+full:SNP-1".to_string())
    );
    assert!(spec
        .write_keys
        .contains(&"patchset:RP-1:ci:preflight".to_string()));
    assert!(spec
        .write_keys
        .contains(&"patchset:RP-1:ci:full".to_string()));
    assert!(spec
        .write_keys
        .contains(&"repo:ait:ci-shard-pool:patchset-ci:preflight".to_string()));
    assert!(spec
        .write_keys
        .contains(&"repo:ait:ci-shard-pool:patchset-ci:full".to_string()));
}

#[test]
fn patchset_ci_all_suite_payload_keeps_low_normal_ci_priority_with_ten_shard_tokens() {
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait-server",
            "patchset_id": "P-SEC-1",
            "suite_ids": ["rust_core"],
            "revision_snapshot_id": "SNP-1"
        })),
        &test_policy(),
    )
    .expect("all-suite patchset CI should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::NormalCi);
    assert_eq!(spec.cpu_tokens, 10);
    assert_eq!(spec.priority, 30);
    assert!(!spec
        .token_pools
        .contains(&"full_test_cpu_tokens".to_string()));
    assert_eq!(
        spec.singleflight_key,
        Some("patchset.ci:P-SEC-1:rust_core:SNP-1".to_string())
    );
    assert!(spec
        .write_keys
        .contains(&"patchset:P-SEC-1:ci:rust_core".to_string()));
    assert!(spec
        .write_keys
        .contains(&"repo:ait-server:ci-shard-pool:patchset-ci:rust_core".to_string()));
}

#[test]
fn tg1_required_consumes_ten_ci_tokens_without_full_test_pool() {
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-1"
        })),
        &test_policy(),
    )
    .expect("TG1 patchset CI should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::NormalCi);
    assert_eq!(spec.cpu_tokens, 10);
    assert!(spec
        .token_pools
        .contains(&"ci_full_shared_cpu_tokens".to_string()));
    assert!(!spec
        .token_pools
        .contains(&"full_test_cpu_tokens".to_string()));
    assert_eq!(spec.priority, 30);
}

#[test]
fn tg1_required_zstd_only_consumes_tg1_ci_tokens_without_full_test_pool() {
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required_zstd_only",
            "revision_snapshot_id": "SNP-1"
        })),
        &test_policy(),
    )
    .expect("TG1 zstd-only patchset CI should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::NormalCi);
    assert_eq!(spec.cpu_tokens, 10);
    assert!(spec
        .token_pools
        .contains(&"ci_full_shared_cpu_tokens".to_string()));
    assert!(!spec
        .token_pools
        .contains(&"full_test_cpu_tokens".to_string()));
    assert_eq!(spec.priority, 30);
}

#[test]
fn tg1_required_scales_down_and_admits_on_small_hosts() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(4, SchedulerDeploymentPosture::LocalCoResident);
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-1"
        })),
        &policy,
    )
    .expect("TG1 patchset CI should shape");

    assert_eq!(spec.cpu_tokens, 4);

    let decision = admit_next(
        &[SchedulerQueuedJob {
            job_id: "tg1".to_string(),
            spec,
            queued_ordinal: 0,
        }],
        &[],
        &policy,
    );
    assert_eq!(
        decision,
        SchedulerAdmissionDecision::Admit {
            job_id: "tg1".to_string(),
        }
    );
}

#[test]
fn patchset_full_test_uses_scaled_dedicated_server_tokens() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer);
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "full",
            "revision_snapshot_id": "SNP-1"
        })),
        &policy,
    )
    .expect("patchset full test should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::FullTest);
    assert_eq!(spec.cpu_tokens, 10);
}

#[test]
fn repo_full_repo_suite_consumes_full_test_pool() {
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "repo.ci",
        &payload(json!({
            "repo_name": "ait",
            "suite_ids": ["full_repo"],
            "plane": "nightly",
            "target_line": "main",
            "snapshot_id": "SNP-FULL"
        })),
        &test_policy(),
    )
    .expect("repo full_repo suite should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::FullTest);
    assert_eq!(spec.cpu_tokens, 10);
    assert!(spec
        .token_pools
        .contains(&"ci_full_shared_cpu_tokens".to_string()));
    assert!(spec
        .token_pools
        .contains(&"full_test_cpu_tokens".to_string()));
    assert_eq!(
        spec.singleflight_key,
        Some("repo.ci:repo:ait:nightly:full_repo:SNP-FULL".to_string())
    );
    assert_eq!(
        spec.write_keys,
        vec![
            "repo:ait:ci:nightly:full_repo".to_string(),
            "repo:ait:ci-shard-pool:repo-ci:nightly:main:full_repo".to_string(),
        ]
    );
}

#[test]
fn repo_full_repo_zstd_only_suite_consumes_full_test_pool() {
    let spec = scheduler_job_spec_from_async_job_with_policy(
        "repo.ci",
        &payload(json!({
            "repo_name": "ait",
            "suite_ids": ["full_repo_zstd_only"],
            "plane": "nightly",
            "target_line": "main",
            "snapshot_id": "SNP-FULL-ZSTD"
        })),
        &test_policy(),
    )
    .expect("repo full_repo_zstd_only suite should shape");

    assert_eq!(spec.job_class, SchedulerJobClass::FullTest);
    assert_eq!(spec.cpu_tokens, 10);
    assert!(spec
        .token_pools
        .contains(&"ci_full_shared_cpu_tokens".to_string()));
    assert!(spec
        .token_pools
        .contains(&"full_test_cpu_tokens".to_string()));
    assert_eq!(
        spec.singleflight_key,
        Some("repo.ci:repo:ait:nightly:full_repo_zstd_only:SNP-FULL-ZSTD".to_string())
    );
    assert_eq!(
        spec.write_keys,
        vec![
            "repo:ait:ci:nightly:full_repo_zstd_only".to_string(),
            "repo:ait:ci-shard-pool:repo-ci:nightly:main:full_repo_zstd_only".to_string(),
        ]
    );
}

#[test]
fn normal_ci_has_priority_over_full_test_inside_shared_budget() {
    let policy = test_policy();
    let queued_jobs = Vec::from([
        queued(
            "full-1",
            0,
            "repo.ci",
            json!({
                "repo_name": "ait",
                "suite_ids": ["full"],
                "plane": "nightly",
                "snapshot_id": "SNP-1"
            }),
        ),
        queued(
            "ci-1",
            1,
            "patchset.ci",
            json!({
                "repo_name": "ait",
                "patchset_id": "RP-1",
                "suite_id": "unit",
                "revision_snapshot_id": "SNP-2"
            }),
        ),
    ]);

    assert_eq!(
        admit_next(&queued_jobs, &[], &policy),
        SchedulerAdmissionDecision::Admit {
            job_id: "ci-1".to_string()
        }
    );
}

#[test]
fn ci_priority_is_below_main_seed_and_above_maintenance() {
    let policy = test_policy();
    let main_seed = queued(
        "main-seed",
        2,
        "main-seed.refresh",
        json!({
            "repo_name": "ait-main",
            "snapshot_id": "SNP-MAIN",
            "patchset_id": "RP-MAIN"
        }),
    );
    let ci = queued(
        "ci",
        1,
        "patchset.ci",
        json!({
            "repo_name": "ait-ci",
            "patchset_id": "RP-CI",
            "suite_id": "unit",
            "revision_snapshot_id": "SNP-CI"
        }),
    );
    let maintenance = queued(
        "maintenance",
        0,
        "content.gc",
        json!({"repo_name": "ait-maintenance"}),
    );

    assert!(main_seed.spec.priority > ci.spec.priority);
    assert!(ci.spec.priority > maintenance.spec.priority);
    assert_eq!(
        admit_next(&[maintenance.clone(), ci.clone(), main_seed], &[], &policy),
        SchedulerAdmissionDecision::Admit {
            job_id: "main-seed".to_string(),
        }
    );
    assert_eq!(
        admit_next(&[maintenance.clone(), ci], &[], &policy),
        SchedulerAdmissionDecision::Admit {
            job_id: "ci".to_string(),
        }
    );
    assert_eq!(
        admit_next(&[maintenance], &[], &policy),
        SchedulerAdmissionDecision::Admit {
            job_id: "maintenance".to_string(),
        }
    );
}

#[test]
fn ci_and_full_test_share_ten_cpu_tokens() {
    let policy = SchedulerPolicy {
        global_cpu_tokens: 16,
        ..test_policy()
    };
    let running_jobs = Vec::from([
        running(
            "ci-1",
            "patchset.ci",
            json!({"repo_name": "ait", "patchset_id": "RP-1", "suite_id": "unit", "revision_snapshot_id": "SNP-1"}),
        ),
        running(
            "ci-2",
            "patchset.ci",
            json!({"repo_name": "ait", "patchset_id": "RP-2", "suite_id": "unit", "revision_snapshot_id": "SNP-2"}),
        ),
        running(
            "ci-3",
            "patchset.ci",
            json!({"repo_name": "ait", "patchset_id": "RP-3", "suite_id": "unit", "revision_snapshot_id": "SNP-3"}),
        ),
        running(
            "ci-4",
            "patchset.ci",
            json!({"repo_name": "ait", "patchset_id": "RP-4", "suite_id": "unit", "revision_snapshot_id": "SNP-4"}),
        ),
        running(
            "ci-5",
            "patchset.ci",
            json!({"repo_name": "ait", "patchset_id": "RP-5", "suite_id": "unit", "revision_snapshot_id": "SNP-5"}),
        ),
        running(
            "ci-6",
            "patchset.ci",
            json!({"repo_name": "ait", "patchset_id": "RP-6", "suite_id": "unit", "revision_snapshot_id": "SNP-6"}),
        ),
    ]);
    let queued_jobs = Vec::from([queued(
        "full-1",
        0,
        "repo.ci",
        json!({
            "repo_name": "ait-full",
            "suite_ids": ["full"],
            "plane": "nightly",
            "snapshot_id": "SNP-7"
        }),
    )]);

    match admit_next(&queued_jobs, &running_jobs, &policy) {
        SchedulerAdmissionDecision::Wait { reason } => {
            assert!(
                reason.contains("global_cpu_tokens")
                    || reason.contains("ci_full_shared_cpu_tokens")
            )
        }
        decision => panic!("expected wait for shared pool, got {decision:?}"),
    }
}

#[test]
fn full_test_shared_budget_blocks_second_distinct_full_test_on_ten_cores() {
    let policy = test_policy();
    let running_jobs = Vec::from([running(
        "full-active",
        "repo.ci",
        json!({
            "repo_name": "ait-a",
            "suite_ids": ["full"],
            "plane": "nightly",
            "snapshot_id": "SNP-1"
        }),
    )]);
    let queued_jobs = Vec::from([queued(
        "full-next",
        0,
        "repo.ci",
        json!({
            "repo_name": "ait-b",
            "suite_ids": ["full"],
            "plane": "nightly",
            "snapshot_id": "SNP-2"
        }),
    )]);

    assert!(matches!(
        admit_next(&queued_jobs, &running_jobs, &policy),
        SchedulerAdmissionDecision::Wait { ref reason }
            if reason.contains("global_cpu_tokens")
                || reason.contains("ci_full_shared_cpu_tokens")
    ));
}

#[test]
fn larger_dedicated_budget_allows_three_distinct_ten_token_full_tests() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer);
    assert_eq!(policy.ci_full_shared_cpu_tokens, 32);
    assert_eq!(policy.full_test_cpu_tokens, 32);
    assert_eq!(policy.full_test_job_cpu_tokens, 10);

    let running_jobs = Vec::from([
        SchedulerRunningJob {
            job_id: "full-active-a".to_string(),
            spec: scheduler_job_spec_from_async_job_with_policy(
                "repo.ci",
                &payload(json!({
                    "repo_name": "ait-a",
                    "suite_ids": ["full"],
                    "plane": "nightly",
                    "snapshot_id": "SNP-1"
                })),
                &policy,
            )
            .expect("scheduler job should shape"),
        },
        SchedulerRunningJob {
            job_id: "full-active-b".to_string(),
            spec: scheduler_job_spec_from_async_job_with_policy(
                "repo.ci",
                &payload(json!({
                    "repo_name": "ait-b",
                    "suite_ids": ["full"],
                    "plane": "nightly",
                    "snapshot_id": "SNP-2"
                })),
                &policy,
            )
            .expect("scheduler job should shape"),
        },
    ]);
    let queued_jobs = Vec::from([SchedulerQueuedJob {
        job_id: "full-next".to_string(),
        spec: scheduler_job_spec_from_async_job_with_policy(
            "repo.ci",
            &payload(json!({
                "repo_name": "ait-c",
                "suite_ids": ["full"],
                "plane": "nightly",
                "snapshot_id": "SNP-3"
            })),
            &policy,
        )
        .expect("scheduler job should shape"),
        queued_ordinal: 0,
    }]);

    assert_eq!(
        admit_next(&queued_jobs, &running_jobs, &policy),
        SchedulerAdmissionDecision::Admit {
            job_id: "full-next".to_string(),
        }
    );
}

#[test]
fn duplicate_full_test_snapshot_attaches_to_active_singleflight() {
    let policy = test_policy();
    let running_jobs = Vec::from([running(
        "full-active",
        "repo.ci",
        json!({
            "repo_name": "ait",
            "suite_ids": ["full"],
            "plane": "nightly",
            "snapshot_id": "SNP-1"
        }),
    )]);
    let queued_jobs = Vec::from([queued(
        "full-duplicate",
        0,
        "repo.ci",
        json!({
            "repo_name": "ait",
            "suite_ids": ["full"],
            "plane": "nightly",
            "snapshot_id": "SNP-1"
        }),
    )]);

    assert_eq!(
        admit_next(&queued_jobs, &running_jobs, &policy),
        SchedulerAdmissionDecision::Attach {
            job_id: "full-duplicate".to_string(),
            active_job_id: "full-active".to_string(),
            singleflight_key: "repo.ci:repo:ait:nightly:full:SNP-1".to_string(),
        }
    );
}

#[test]
fn conflicting_patchset_ci_writes_serialize() {
    let policy = test_policy();
    let running_jobs = Vec::from([running(
        "ci-active",
        "patchset.ci",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "unit",
            "revision_snapshot_id": "SNP-1"
        }),
    )]);
    let queued_jobs = Vec::from([queued(
        "ci-next",
        0,
        "patchset.ci",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "unit",
            "revision_snapshot_id": "SNP-2"
        }),
    )]);

    match admit_next(&queued_jobs, &running_jobs, &policy) {
        SchedulerAdmissionDecision::Wait { reason } => {
            assert!(reason.contains("conflicts with running job ci-active"))
        }
        decision => panic!("expected write-key conflict wait, got {decision:?}"),
    }
}

#[test]
fn same_repo_patchset_ci_suite_shard_pool_serializes_across_patchsets() {
    let policy = test_policy();
    let running_jobs = Vec::from([running(
        "ci-active",
        "patchset.ci",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "rust_core",
            "revision_snapshot_id": "SNP-1"
        }),
    )]);
    let queued_jobs = Vec::from([queued(
        "ci-next",
        0,
        "patchset.ci",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-2",
            "suite_id": "rust_core",
            "revision_snapshot_id": "SNP-2"
        }),
    )]);

    match admit_next(&queued_jobs, &running_jobs, &policy) {
        SchedulerAdmissionDecision::Wait { reason } => {
            assert!(reason.contains("conflicts with running job ci-active"))
        }
        decision => panic!("expected stable shard-pool write-key conflict wait, got {decision:?}"),
    }
}

#[test]
fn different_patchset_ci_suites_can_run_before_attestation_aggregation() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer);
    let running_jobs = Vec::from([running(
        "preflight-active",
        "patchset.ci",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "preflight",
            "revision_snapshot_id": "SNP-1"
        }),
    )]);
    let queued_jobs = Vec::from([queued(
        "tg1-next",
        0,
        "patchset.ci",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-1"
        }),
    )]);

    assert_eq!(
        admit_next(&queued_jobs, &running_jobs, &policy),
        SchedulerAdmissionDecision::Admit {
            job_id: "tg1-next".to_string()
        }
    );
}

#[test]
fn patchset_ci_ready_aggregation_waits_for_running_suite_result() {
    let policy = test_policy();
    let running_jobs = Vec::from([running(
        "preflight-active",
        "patchset.ci",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_id": "preflight",
            "revision_snapshot_id": "SNP-1"
        }),
    )]);
    let queued_jobs = Vec::from([queued(
        "ready-aggregate",
        0,
        "patchset.ci.aggregate",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_ids": ["package_smoke", "preflight", "stable_smoke"],
            "stage": "ready_blocking",
            "revision_snapshot_id": "SNP-1"
        }),
    )]);

    match admit_next(&queued_jobs, &running_jobs, &policy) {
        SchedulerAdmissionDecision::Wait { reason } => {
            assert!(reason.contains("conflicts with running job preflight-active"))
        }
        decision => panic!("expected aggregation to wait for suite result, got {decision:?}"),
    }
}

#[test]
fn patchset_ci_aggregations_serialize_patchset_completion_writes() {
    let policy = test_policy();
    let running_jobs = Vec::from([running(
        "ready-aggregate-active",
        "patchset.ci.aggregate",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_ids": ["package_smoke", "preflight", "stable_smoke"],
            "stage": "ready_blocking",
            "revision_snapshot_id": "SNP-1"
        }),
    )]);
    let queued_jobs = Vec::from([queued(
        "informational-aggregate",
        0,
        "patchset.ci.aggregate",
        json!({
            "repo_name": "ait",
            "patchset_id": "RP-1",
            "suite_ids": ["tg1_required"],
            "stage": "informational",
            "revision_snapshot_id": "SNP-1"
        }),
    )]);

    match admit_next(&queued_jobs, &running_jobs, &policy) {
        SchedulerAdmissionDecision::Wait { reason } => {
            assert!(reason.contains("conflicts with running job ready-aggregate-active"))
        }
        decision => panic!("expected Patchset completion serialization, got {decision:?}"),
    }
}
