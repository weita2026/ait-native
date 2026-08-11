#[test]
fn scheduler_shape_async_job_command_reports_full_test_tokens() {
    let value = stdout_json(&run_seam(&[
        "scheduler-shape-async-job",
        "repo.ci",
        r#"{"repo_name":"ait","suite_ids":["full_repo"],"plane":"nightly","snapshot_id":"SNP-1"}"#,
    ]));

    assert_eq!(value["job_kind"], json!("repo.ci"));
    assert_eq!(value["job_class"], json!("FullTest"));
    assert_eq!(
        value["cpu_tokens"],
        json!(SchedulerPolicy::default().full_test_job_cpu_tokens)
    );
    assert_eq!(
        value["singleflight_key"],
        json!("repo.ci:repo:ait:nightly:full_repo:SNP-1")
    );
    assert_eq!(
        value["token_pools"],
        json!([
            "global_cpu_tokens",
            "ci_full_shared_cpu_tokens",
            "full_test_cpu_tokens"
        ])
    );
}

#[test]
fn scheduler_admit_async_jobs_command_prioritizes_ci_over_full_test() {
    let payload = json!({
        "policy": {"host_cpu_cores": 10, "posture": "local_co_resident"},
        "queued": [
            {
                "job_id": "full-first",
                "job_type": "repo.ci",
                "payload": {
                    "repo_name": "ait-full",
                    "suite_ids": ["full"],
                    "plane": "nightly",
                    "snapshot_id": "SNP-FULL"
                }
            },
            {
                "job_id": "ci-second",
                "job_type": "patchset.ci",
                "payload": {
                    "repo_name": "ait",
                    "patchset_id": "RP-1",
                    "suite_id": "unit",
                    "revision_snapshot_id": "SNP-CI"
                }
            }
        ],
        "running": []
    })
    .to_string();
    let value = stdout_json(&run_seam(&["scheduler-admit-async-jobs", &payload]));

    assert_eq!(value["decision"]["kind"], json!("admit"));
    assert_eq!(value["decision"]["job_id"], json!("ci-second"));
    assert_eq!(value["decision"]["job"]["cpu_tokens"], json!(10));
    assert_eq!(value["policy"]["global_cpu_tokens"], json!(10));
    assert_eq!(value["policy"]["full_test_cpu_tokens"], json!(10));
}

#[test]
fn scheduler_status_command_reports_capacity_usage_and_next_admission() {
    let payload = json!({
        "policy": {"host_cpu_cores": 10, "posture": "local_co_resident"},
        "queued": [
            {
                "job_id": "full-next",
                "job_type": "repo.ci",
                "payload": {
                    "repo_name": "ait-full",
                    "suite_ids": ["full"],
                    "plane": "nightly",
                    "snapshot_id": "SNP-FULL"
                }
            },
            {
                "job_id": "ci-next",
                "job_type": "patchset.ci",
                "payload": {
                    "repo_name": "ait",
                    "patchset_id": "RP-1",
                    "suite_id": "unit",
                    "revision_snapshot_id": "SNP-CI"
                }
            }
        ],
        "running": [{
            "job_id": "full-active",
            "job_type": "repo.ci",
            "payload": {
                "repo_name": "ait-running",
                "suite_ids": ["full"],
                "plane": "nightly",
                "snapshot_id": "SNP-RUNNING"
            }
        }]
    })
    .to_string();
    let value = stdout_json(&run_seam(&["scheduler-status", &payload]));

    assert_eq!(value["status"], json!("running_with_backlog"));
    assert_eq!(value["queued_job_count"], json!(2));
    assert_eq!(value["running_job_count"], json!(1));
    assert_eq!(value["policy"]["global_cpu_tokens"], json!(10));
    assert_eq!(
        value["capacity"]["global_cpu_tokens"],
        json!({"capacity": 10, "used": 10, "available": 0, "over_capacity": false})
    );
    assert_eq!(
        value["capacity"]["full_test_cpu_tokens"],
        json!({"capacity": 10, "used": 10, "available": 0, "over_capacity": false})
    );
    assert_eq!(
        value["thread_pool"]["state_source"],
        json!("scheduler_snapshot_payload")
    );
    assert_eq!(value["thread_pool"]["running_leases"], json!(1));
    assert_eq!(value["thread_pool"]["worker_count"], JsonValue::Null);
    assert_eq!(value["next_admission"]["kind"], json!("wait"));
    assert_eq!(
        value["queued_jobs"][0]["scheduler_job"]["job_class"],
        json!("FullTest")
    );
    assert_eq!(
        value["running_jobs"][0]["scheduler_job"]["cpu_tokens"],
        json!(10)
    );
}

#[test]
fn scheduler_status_command_allows_empty_snapshot() {
    let value = stdout_json(&run_seam(&[
        "scheduler-status",
        r#"{"policy":{"host_cpu_cores":10,"posture":"local_co_resident"}}"#,
    ]));

    assert_eq!(value["status"], json!("idle"));
    assert_eq!(value["queued_job_count"], json!(0));
    assert_eq!(value["running_job_count"], json!(0));
    assert_eq!(value["next_admission"]["kind"], json!("wait"));
    assert_eq!(value["next_admission"]["reason"], json!("no queued jobs"));
    assert_eq!(
        value["capacity"]["ci_full_shared_cpu_tokens"],
        json!({"capacity": 10, "used": 0, "available": 10, "over_capacity": false})
    );
}

#[test]
fn scheduler_admit_async_jobs_command_allows_second_full_test_when_capacity_remains() {
    let payload = json!({
        "policy": {"host_cpu_cores": 32, "posture": "dedicated_server"},
        "queued": [{
            "job_id": "full-next",
            "job_type": "repo.ci",
            "payload": {
                "repo_name": "ait-b",
                "suite_ids": ["full"],
                "plane": "nightly",
                "snapshot_id": "SNP-2"
            }
        }],
        "running": [{
            "job_id": "full-active",
            "job_type": "repo.ci",
            "payload": {
                "repo_name": "ait-a",
                "suite_ids": ["full"],
                "plane": "nightly",
                "snapshot_id": "SNP-1"
            }
        }]
    })
    .to_string();
    let value = stdout_json(&run_seam(&["scheduler-admit-async-jobs", &payload]));

    assert_eq!(value["decision"]["kind"], json!("admit"));
    assert_eq!(value["decision"]["job_id"], json!("full-next"));
    assert_eq!(value["decision"]["job"]["cpu_tokens"], json!(10));
    assert_eq!(value["running_job_count"], json!(1));
}

#[test]
fn scheduler_admit_async_jobs_command_attaches_duplicate_full_test() {
    let payload = json!({
        "policy": {"host_cpu_cores": 10, "posture": "local_co_resident"},
        "queued": [{
            "job_id": "full-duplicate",
            "job_type": "repo.ci",
            "payload": {
                "repo_name": "ait",
                "suite_ids": ["full"],
                "plane": "nightly",
                "snapshot_id": "SNP-1"
            }
        }],
        "running": [{
            "job_id": "full-active",
            "job_type": "repo.ci",
            "payload": {
                "repo_name": "ait",
                "suite_ids": ["full"],
                "plane": "nightly",
                "snapshot_id": "SNP-1"
            }
        }]
    })
    .to_string();
    let value = stdout_json(&run_seam(&["scheduler-admit-async-jobs", &payload]));

    assert_eq!(value["decision"]["kind"], json!("attach"));
    assert_eq!(value["decision"]["job_id"], json!("full-duplicate"));
    assert_eq!(value["decision"]["active_job_id"], json!("full-active"));
    assert_eq!(
        value["decision"]["singleflight_key"],
        json!("repo.ci:repo:ait:nightly:full:SNP-1")
    );
}

#[test]
fn scheduler_admit_async_jobs_command_rejects_invalid_policy() {
    let output = run_seam(&[
        "scheduler-admit-async-jobs",
        r#"{"policy":{"host_cpu_cores":0,"posture":"local_co_resident"},"queued":[]}"#,
    ]);

    assert_failed_with(
        &output,
        "Field `policy.host_cpu_cores` must be a positive integer.",
    );
}

#[test]
fn patchset_ci_schedule_admission_prioritizes_normal_ci_over_full_test() {
    let payload = json!({
        "repo_name": "ait",
        "patchset_id": "RP-1",
        "revision_snapshot_id": "SNP-1",
        "scope": "workflow_ready_foreground",
        "policy": {"host_cpu_cores": 10, "posture": "local_co_resident"},
        "manifests": [
            {"suite_id": "full", "plane": "patchset", "mode": "gate", "default_blocking": true},
            {"suite_id": "preflight", "plane": "patchset", "mode": "gate", "default_blocking": true}
        ],
        "running": []
    })
    .to_string();
    let value = stdout_json(&run_seam(&["patchset-ci-schedule-admission", &payload]));

    assert_eq!(value["decision"]["kind"], json!("admit"));
    assert_eq!(value["decision"]["job"]["suite_id"], json!("preflight"));
    assert_eq!(value["decision"]["admitted_cpu_tokens"], json!(10));

    let queued_jobs = value["queued_jobs"]
        .as_array()
        .expect("queued_jobs should be an array");
    let full_job = queued_jobs
        .iter()
        .find(|job| job["job"]["suite_id"] == json!("full"))
        .expect("full job should be queued");
    assert_eq!(full_job["scheduler_job"]["cpu_tokens"], json!(10));
    assert_eq!(value["blocked_jobs"].as_array().unwrap().len(), 1);
    assert!(value["blocked_jobs"][0]["reason"]
        .as_str()
        .expect("blocked reason should be text")
        .contains("waits for suite results"));
}

#[test]
fn patchset_ci_schedule_admission_attaches_duplicate_full_test() {
    let payload = json!({
        "repo_name": "ait",
        "patchset_id": "RP-1",
        "revision_snapshot_id": "SNP-1",
        "policy": {"host_cpu_cores": 10, "posture": "local_co_resident"},
        "manifests": [
            {"suite_id": "full", "plane": "patchset", "mode": "gate", "default_blocking": true}
        ],
        "running": [{
            "job_id": "running-full",
            "job_type": "patchset.ci",
            "payload": {
                "repo_name": "ait",
                "patchset_id": "RP-1",
                "suite_id": "full",
                "revision_snapshot_id": "SNP-1"
            }
        }]
    })
    .to_string();
    let value = stdout_json(&run_seam(&["patchset-ci-schedule-admission", &payload]));

    assert_eq!(value["decision"]["kind"], json!("attach"));
    assert_eq!(value["decision"]["active_job_id"], json!("running-full"));
    assert_eq!(
        value["decision"]["singleflight_key"],
        json!("patchset.ci:RP-1:full:SNP-1")
    );
}

#[test]
fn patchset_ci_schedule_admission_allows_distinct_full_test_when_capacity_remains() {
    let payload = json!({
        "repo_name": "ait",
        "patchset_id": "RP-1",
        "revision_snapshot_id": "SNP-1",
        "policy": {"host_cpu_cores": 32, "posture": "dedicated_server"},
        "manifests": [
            {"suite_id": "full", "plane": "patchset", "mode": "gate", "default_blocking": true}
        ],
        "running": [{
            "job_id": "other-full",
            "job_type": "patchset.ci",
            "payload": {
                "repo_name": "ait-other",
                "patchset_id": "RP-OTHER",
                "suite_id": "full",
                "revision_snapshot_id": "SNP-OTHER"
            }
        }]
    })
    .to_string();
    let value = stdout_json(&run_seam(&["patchset-ci-schedule-admission", &payload]));

    assert_eq!(value["decision"]["kind"], json!("admit"));
    assert_eq!(value["decision"]["job"]["suite_id"], json!("full"));
    assert_eq!(value["decision"]["admitted_cpu_tokens"], json!(10));
}

#[test]
fn patchset_ci_schedule_admission_releases_aggregation_after_completed_suites() {
    let payload = json!({
        "repo_name": "ait",
        "patchset_id": "RP-1",
        "revision_snapshot_id": "SNP-1",
        "scope": "all",
        "completed_suite_ids": ["preflight"],
        "policy": {"host_cpu_cores": 10, "posture": "local_co_resident"},
        "manifests": [
            {"suite_id": "preflight", "plane": "patchset", "mode": "gate", "default_blocking": true}
        ],
        "running": []
    })
    .to_string();
    let value = stdout_json(&run_seam(&["patchset-ci-schedule-admission", &payload]));

    assert_eq!(value["decision"]["kind"], json!("admit"));
    assert_eq!(
        value["decision"]["job"]["job_type"],
        json!("patchset.ci.aggregate")
    );
    assert_eq!(value["decision"]["job"]["stage"], json!("ready_blocking"));
    assert_eq!(value["blocked_jobs"], json!([]));
}
