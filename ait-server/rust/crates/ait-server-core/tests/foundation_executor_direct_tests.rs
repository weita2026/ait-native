use std::sync::mpsc;
use std::sync::Arc;

use ait_server_core::foundation::executor::{
    ScheduledExecutorAdmission, ScheduledExecutorPool, ScheduledExecutorSubmission,
};
use ait_server_core::foundation::scheduler::{
    scheduler_job_spec_from_async_job_with_policy, SchedulerAdmissionDecision,
    SchedulerDeploymentPosture, SchedulerPolicy, SchedulerQueuedJob,
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

fn monotonic() -> Arc<dyn Fn() -> f64 + Send + Sync> {
    Arc::new(|| 0.0)
}

fn scheduled_pool(policy: SchedulerPolicy) -> ScheduledExecutorPool {
    ScheduledExecutorPool::new("ait-test-scheduled", monotonic(), policy)
}

fn patchset_ci_spec(
    patchset_id: &str,
    suite_id: &str,
) -> ait_server_core::foundation::scheduler::SchedulerJobSpec {
    scheduler_job_spec_from_async_job_with_policy(
        "patchset.ci",
        &payload(json!({
            "repo_name": "ait",
            "patchset_id": patchset_id,
            "suite_id": suite_id,
            "revision_snapshot_id": format!("SNP-{patchset_id}")
        })),
        &test_policy(),
    )
    .expect("patchset CI spec should shape")
}

fn repo_ci_spec(
    repo_name: &str,
    suite_id: &str,
    snapshot_id: &str,
    policy: &SchedulerPolicy,
) -> ait_server_core::foundation::scheduler::SchedulerJobSpec {
    scheduler_job_spec_from_async_job_with_policy(
        "repo.ci",
        &payload(json!({
            "repo_name": repo_name,
            "suite_ids": [suite_id],
            "plane": "nightly",
            "snapshot_id": snapshot_id
        })),
        policy,
    )
    .expect("repo CI spec should shape")
}

fn queued(
    job_id: &str,
    queued_ordinal: usize,
    spec: ait_server_core::foundation::scheduler::SchedulerJobSpec,
) -> SchedulerQueuedJob {
    SchedulerQueuedJob {
        job_id: job_id.to_string(),
        spec,
        queued_ordinal,
    }
}

fn blocking_job() -> (mpsc::Sender<()>, impl FnOnce() + Send + 'static) {
    let (release, wait) = mpsc::channel();
    (release, move || {
        wait.recv().expect("test should release blocking job");
    })
}

fn wait_submission(submission: ScheduledExecutorSubmission<()>) -> String {
    match submission {
        ScheduledExecutorSubmission::Waiting {
            admission: ScheduledExecutorAdmission::Waiting { job_id: _, reason },
        } => reason,
        _ => panic!("expected waiting admission"),
    }
}

fn attached_submission(submission: ScheduledExecutorSubmission<()>) -> (String, String, String) {
    match submission {
        ScheduledExecutorSubmission::Attached {
            admission:
                ScheduledExecutorAdmission::Attached {
                    job_id,
                    active_job_id,
                    singleflight_key,
                },
        } => (job_id, active_job_id, singleflight_key),
        _ => panic!("expected attached admission"),
    }
}

#[test]
fn scheduled_executor_submits_admitted_ci_and_releases_running_lease() {
    let pool = scheduled_pool(test_policy());
    let submission = pool
        .submit_scheduled("ci-1", patchset_ci_spec("RP-1", "unit"), || "done")
        .expect("scheduled submission should not fail");

    match submission {
        ScheduledExecutorSubmission::Submitted { admission, future } => {
            assert_eq!(
                admission,
                ScheduledExecutorAdmission::Submitted {
                    job_id: "ci-1".to_string(),
                    queue_key: "patchset:RP-1:ci:unit".to_string(),
                    cpu_tokens: 10,
                    token_pools: vec![
                        "global_cpu_tokens".to_string(),
                        "ci_full_shared_cpu_tokens".to_string(),
                    ],
                }
            );
            assert_eq!(future.wait().expect("job should finish"), "done");
        }
        _ => panic!("expected submitted admission"),
    }

    assert_eq!(pool.running_job_count(), 0);
    assert!(pool.wait_for_idle(Some(1.0)));
    pool.stop();
}

#[test]
fn scheduled_executor_blocks_full_test_when_shared_ci_pool_is_exhausted() {
    let policy = SchedulerPolicy {
        global_cpu_tokens: 20,
        ci_full_shared_cpu_tokens: 10,
        ..test_policy()
    };
    let pool = scheduled_pool(policy.clone());
    let mut releases = Vec::new();

    for index in 0..1 {
        let patchset_id = format!("RP-{index}");
        let suite_id = format!("unit-{index}");
        let (release, job) = blocking_job();
        releases.push(release);
        match pool
            .submit_scheduled(
                format!("ci-{index}"),
                patchset_ci_spec(&patchset_id, &suite_id),
                job,
            )
            .expect("CI submission should not fail")
        {
            ScheduledExecutorSubmission::Submitted { .. } => {}
            _ => panic!("expected CI job to submit"),
        }
    }

    assert_eq!(pool.running_job_count(), 1);
    let reason = wait_submission(
        pool.submit_scheduled(
            "full-next",
            repo_ci_spec("ait-full", "full", "SNP-FULL", &policy),
            || (),
        )
        .expect("full-test submission should not fail"),
    );
    assert!(reason.contains("ci_full_shared_cpu_tokens"));
    assert_eq!(pool.worker_count(), 1);

    for release in releases {
        release.send(()).expect("release should send");
    }
    assert!(pool.wait_for_idle(Some(2.0)));
    pool.stop();
}

#[test]
fn scheduled_executor_wait_submission_runs_after_conflicting_lease_releases() {
    let pool = scheduled_pool(test_policy());
    let (release_active, active_job) = blocking_job();
    let active = pool
        .submit_scheduled("ci-active", patchset_ci_spec("RP-1", "unit"), active_job)
        .expect("active CI submission should not fail");
    assert!(matches!(
        active,
        ScheduledExecutorSubmission::Submitted { .. }
    ));

    let waiting_pool = pool.clone();
    let (submission_tx, submission_rx) = mpsc::channel();
    let waiter = std::thread::spawn(move || {
        let submission = waiting_pool.submit_scheduled_wait(
            "ci-waiting",
            patchset_ci_spec("RP-2", "unit"),
            || "waiting-done",
        );
        submission_tx
            .send(submission)
            .expect("waiting submission result should send");
    });

    assert!(matches!(
        submission_rx.recv_timeout(std::time::Duration::from_millis(50)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    release_active.send(()).expect("active release should send");

    let submission = submission_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("waiting submission should resume after lease release")
        .expect("waiting submission should not fail");
    match submission {
        ScheduledExecutorSubmission::Submitted { admission, future } => {
            assert_eq!(
                admission,
                ScheduledExecutorAdmission::Submitted {
                    job_id: "ci-waiting".to_string(),
                    queue_key: "patchset:RP-2:ci:unit".to_string(),
                    cpu_tokens: 10,
                    token_pools: vec![
                        "global_cpu_tokens".to_string(),
                        "ci_full_shared_cpu_tokens".to_string(),
                    ],
                }
            );
            assert_eq!(
                future.wait().expect("waiting CI job should finish"),
                "waiting-done"
            );
        }
        _ => panic!("expected waiting CI job to submit after lease release"),
    }

    waiter
        .join()
        .expect("waiting submission thread should join");
    assert!(pool.wait_for_idle(Some(2.0)));
    pool.stop();
}

#[test]
fn scheduled_executor_arbitrates_normal_ci_ahead_of_full_test() {
    let policy = test_policy();
    let pool = scheduled_pool(policy.clone());
    let queued_jobs = vec![
        queued(
            "full-first",
            0,
            repo_ci_spec("ait", "full", "SNP-FULL", &policy),
        ),
        queued("ci-second", 1, patchset_ci_spec("RP-1", "unit")),
    ];

    assert_eq!(
        pool.admit_next_queued(&queued_jobs),
        SchedulerAdmissionDecision::Admit {
            job_id: "ci-second".to_string(),
        }
    );
}

#[test]
fn scheduled_executor_allows_ci_while_full_test_running_when_shared_capacity_remains() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer);
    let pool = scheduled_pool(policy.clone());
    let (release_full, full_job) = blocking_job();

    match pool
        .submit_scheduled(
            "full-active",
            repo_ci_spec("ait", "full", "SNP-FULL", &policy),
            full_job,
        )
        .expect("full-test submission should not fail")
    {
        ScheduledExecutorSubmission::Submitted { admission, .. } => {
            assert_eq!(
                admission,
                ScheduledExecutorAdmission::Submitted {
                    job_id: "full-active".to_string(),
                    queue_key: "repo:ait:ci:nightly:full".to_string(),
                    cpu_tokens: 10,
                    token_pools: vec![
                        "global_cpu_tokens".to_string(),
                        "ci_full_shared_cpu_tokens".to_string(),
                        "full_test_cpu_tokens".to_string(),
                    ],
                }
            );
        }
        _ => panic!("expected full-test job to submit"),
    }

    match pool
        .submit_scheduled("ci-next", patchset_ci_spec("RP-1", "unit"), || ())
        .expect("CI submission should not fail")
    {
        ScheduledExecutorSubmission::Submitted { future, .. } => {
            future.wait().expect("CI job should finish");
        }
        _ => panic!("expected CI job to submit while full test runs"),
    }

    assert_eq!(pool.running_job_count(), 1);
    release_full.send(()).expect("full release should send");
    assert!(pool.wait_for_idle(Some(2.0)));
    pool.stop();
}

#[test]
fn scheduled_executor_allows_second_distinct_full_test_when_shared_capacity_remains() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer);
    let pool = scheduled_pool(policy.clone());
    let (release_full, full_job) = blocking_job();

    match pool
        .submit_scheduled(
            "full-active",
            repo_ci_spec("ait-a", "full", "SNP-A", &policy),
            full_job,
        )
        .expect("first full-test submission should not fail")
    {
        ScheduledExecutorSubmission::Submitted { .. } => {}
        _ => panic!("expected first full test to submit"),
    }

    let (release_next, next_job) = blocking_job();
    match pool
        .submit_scheduled(
            "full-next",
            repo_ci_spec("ait-b", "full", "SNP-B", &policy),
            next_job,
        )
        .expect("second full-test submission should not fail")
    {
        ScheduledExecutorSubmission::Submitted { admission, .. } => {
            assert_eq!(
                admission,
                ScheduledExecutorAdmission::Submitted {
                    job_id: "full-next".to_string(),
                    queue_key: "repo:ait-b:ci:nightly:full".to_string(),
                    cpu_tokens: 10,
                    token_pools: vec![
                        "global_cpu_tokens".to_string(),
                        "ci_full_shared_cpu_tokens".to_string(),
                        "full_test_cpu_tokens".to_string(),
                    ],
                }
            );
        }
        _ => panic!("expected second full test to submit"),
    }
    assert_eq!(pool.running_job_count(), 2);
    assert_eq!(pool.worker_count(), 2);

    release_next.send(()).expect("next release should send");
    release_full.send(()).expect("full release should send");
    assert!(pool.wait_for_idle(Some(2.0)));
    pool.stop();
}

#[test]
fn scheduled_executor_allows_three_ten_token_full_tests_on_large_host() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer);
    let pool = scheduled_pool(policy.clone());
    let mut releases = Vec::new();

    for (job_id, repo_name, snapshot_id) in [
        ("full-a", "ait-a", "SNP-A"),
        ("full-b", "ait-b", "SNP-B"),
        ("full-c", "ait-c", "SNP-C"),
    ] {
        let (release, job) = blocking_job();
        releases.push(release);
        match pool
            .submit_scheduled(
                job_id,
                repo_ci_spec(repo_name, "full", snapshot_id, &policy),
                job,
            )
            .expect("full-test submission should not fail")
        {
            ScheduledExecutorSubmission::Submitted { admission, .. } => {
                assert_eq!(
                    admission,
                    ScheduledExecutorAdmission::Submitted {
                        job_id: job_id.to_string(),
                        queue_key: format!("repo:{repo_name}:ci:nightly:full"),
                        cpu_tokens: 10,
                        token_pools: vec![
                            "global_cpu_tokens".to_string(),
                            "ci_full_shared_cpu_tokens".to_string(),
                            "full_test_cpu_tokens".to_string(),
                        ],
                    }
                );
            }
            _ => panic!("expected full test to submit"),
        }
    }

    assert_eq!(pool.running_job_count(), 3);
    assert_eq!(pool.worker_count(), 3);

    for release in releases {
        release.send(()).expect("release should send");
    }
    assert!(pool.wait_for_idle(Some(2.0)));
    pool.stop();
}

#[test]
fn scheduled_executor_attaches_duplicate_full_test_to_active_singleflight() {
    let policy = test_policy();
    let pool = scheduled_pool(policy.clone());
    let (release_full, full_job) = blocking_job();

    match pool
        .submit_scheduled(
            "full-active",
            repo_ci_spec("ait", "full", "SNP-FULL", &policy),
            full_job,
        )
        .expect("full-test submission should not fail")
    {
        ScheduledExecutorSubmission::Submitted { .. } => {}
        _ => panic!("expected active full test to submit"),
    }

    let attached = attached_submission(
        pool.submit_scheduled(
            "full-duplicate",
            repo_ci_spec("ait", "full", "SNP-FULL", &policy),
            || (),
        )
        .expect("duplicate full-test submission should not fail"),
    );
    assert_eq!(
        attached,
        (
            "full-duplicate".to_string(),
            "full-active".to_string(),
            "repo.ci:repo:ait:nightly:full:SNP-FULL".to_string(),
        )
    );
    assert_eq!(pool.worker_count(), 1);

    release_full.send(()).expect("full release should send");
    assert!(pool.wait_for_idle(Some(2.0)));
    pool.stop();
}

#[test]
fn scheduled_executor_caps_full_test_tokens_at_ten_on_dedicated_server() {
    let policy =
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer);
    let pool = scheduled_pool(policy.clone());

    match pool
        .submit_scheduled(
            "full-large",
            repo_ci_spec("ait", "full", "SNP-LARGE", &policy),
            || 42,
        )
        .expect("scaled full-test submission should not fail")
    {
        ScheduledExecutorSubmission::Submitted { admission, future } => {
            assert_eq!(
                admission,
                ScheduledExecutorAdmission::Submitted {
                    job_id: "full-large".to_string(),
                    queue_key: "repo:ait:ci:nightly:full".to_string(),
                    cpu_tokens: 10,
                    token_pools: vec![
                        "global_cpu_tokens".to_string(),
                        "ci_full_shared_cpu_tokens".to_string(),
                        "full_test_cpu_tokens".to_string(),
                    ],
                }
            );
            assert_eq!(future.wait().expect("job should finish"), 42);
        }
        _ => panic!("expected scaled full test to submit"),
    }

    pool.stop();
}
