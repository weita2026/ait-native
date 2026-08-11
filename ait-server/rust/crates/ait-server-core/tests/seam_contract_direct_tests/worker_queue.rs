#[test]
fn worker_queue_service_requires_configured_dsn() {
    assert_failed_with(
        &run_seam_without_postgres_dsn(&[
            "worker-queue-service",
            r#"{"operation":"list-jobs","repo_name":"ait-server"}"#,
        ]),
        "PostgreSQL backend requested but AIT_NATIVE_SERVER_POSTGRES_DSN is not configured",
    );
}

#[test]
fn worker_queue_service_rejects_fake_postgres_dsn() {
    assert_failed_with(
        &run_seam(&[
            "worker-queue-service",
            r#"{"operation":"list-jobs","repo_name":"ait-server","dsn":"fake-postgres:///tmp/ait"}"#,
        ]),
        "fake-postgres is no longer supported",
    );
}

#[test]
fn worker_queue_kernel_command_claims_through_scheduler_and_connection_pool() {
    let payload = json!({
        "operation": "claim-next-job",
        "now": "2026-06-27T13:01:00+08:00",
        "worker_id": "worker-1",
        "repo_name": "housekeeper",
        "jobs": [
            {
                "job_id": 1,
                "repo_name": "housekeeper",
                "job_type": "repo.ci",
                "state": "queued",
                "payload_json": json!({
                    "repo_name": "housekeeper",
                    "suite_ids": ["full_repo"],
                    "plane": "nightly",
                    "snapshot_id": "SNP-FULL"
                }).to_string(),
                "result_json": "{}",
                "attempt_count": 0,
                "max_attempts": 3,
                "available_at": "2026-06-27T13:00:00+08:00",
                "locked_at": null,
                "locked_by": null,
                "last_error": null
            },
            {
                "job_id": 2,
                "repo_name": "housekeeper",
                "job_type": "patchset.ci",
                "state": "queued",
                "payload_json": json!({
                    "repo_name": "housekeeper",
                    "patchset_id": "RP-1",
                    "suite_id": "unit",
                    "revision_snapshot_id": "SNP-CI"
                }).to_string(),
                "result_json": "{}",
                "attempt_count": 0,
                "max_attempts": 3,
                "available_at": "2026-06-27T13:00:00+08:00",
                "locked_at": null,
                "locked_by": null,
                "last_error": null
            }
        ]
    })
    .to_string();

    let value = stdout_json(&run_seam(&["worker-queue-kernel", &payload]));

    assert_eq!(
        value["contract"],
        json!("ait.server.worker_queue.kernel.v1")
    );
    assert_eq!(value["operation"], json!("claim-next-job"));
    assert_eq!(value["claimed_job"]["job_id"], json!(2));
    assert_eq!(
        value["claimed_job"]["scheduler_admission"]["decision"]["kind"],
        json!("admit")
    );
    assert_eq!(value["connection_pool"]["checkout_count"], json!(1));
    assert_eq!(value["jobs"][0]["state"], json!("queued"));
    assert_eq!(value["jobs"][1]["state"], json!("running"));
}

#[test]
fn worker_queue_kernel_command_claims_specific_job_for_handoff() {
    let payload = json!({
        "operation": "claim-job",
        "now": "2026-06-27T13:01:00+08:00",
        "worker_id": "ait-patchset-ci-rp-1-manual",
        "repo_name": "housekeeper",
        "job_id": 2,
        "jobs": [
            {
                "job_id": 1,
                "repo_name": "housekeeper",
                "job_type": "repo.ci",
                "state": "queued",
                "payload_json": json!({
                    "repo_name": "housekeeper",
                    "suite_ids": ["full_repo"],
                    "plane": "nightly",
                    "snapshot_id": "SNP-FULL"
                }).to_string(),
                "result_json": "{}",
                "attempt_count": 0,
                "max_attempts": 3,
                "available_at": "2026-06-27T13:00:00+08:00",
                "locked_at": null,
                "locked_by": null,
                "last_error": null
            },
            {
                "job_id": 2,
                "repo_name": "housekeeper",
                "job_type": "patchset.ci",
                "state": "queued",
                "payload_json": json!({
                    "repo_name": "housekeeper",
                    "patchset_id": "RP-1",
                    "suite_id": "unit",
                    "revision_snapshot_id": "SNP-CI"
                }).to_string(),
                "result_json": "{}",
                "attempt_count": 0,
                "max_attempts": 3,
                "available_at": "2026-06-27T13:00:00+08:00",
                "locked_at": null,
                "locked_by": null,
                "last_error": null
            }
        ]
    })
    .to_string();

    let value = stdout_json(&run_seam(&["worker-queue-kernel", &payload]));

    assert_eq!(value["operation"], json!("claim-job"));
    assert_eq!(value["claimed_job"]["job_id"], json!(2));
    assert_eq!(value["claimed_job"]["state"], json!("running"));
    assert_eq!(
        value["claimed_job"]["locked_by"],
        json!("ait-patchset-ci-rp-1-manual")
    );
    assert_eq!(value["jobs"][0]["state"], json!("queued"));
    assert_eq!(value["jobs"][1]["state"], json!("running"));
}
