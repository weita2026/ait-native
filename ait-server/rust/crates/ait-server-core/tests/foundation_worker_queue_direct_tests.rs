#![cfg(feature = "legacy-postgres-runtime")]

use ait_server_core::foundation::async_job_json::WorkerQueueJobJson;
use ait_server_core::foundation::scheduler::{SchedulerDeploymentPosture, SchedulerPolicy};
use ait_server_core::foundation::worker_queue::{
    worker_queue_service_json, InMemoryWorkerQueuePool, WorkerQueueKernel,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};

fn row(value: JsonValue) -> JsonMap<String, JsonValue> {
    value.as_object().cloned().expect("row should be an object")
}

fn queued_job(job_id: i64, job_type: &str, payload: JsonValue) -> JsonMap<String, JsonValue> {
    row(json!({
        "job_id": job_id,
        "repo_name": payload.get("repo_name").and_then(JsonValue::as_str).unwrap_or("housekeeper"),
        "job_type": job_type,
        "state": "queued",
        "payload_json": payload.to_string(),
        "result_json": "{}",
        "attempt_count": 0,
        "max_attempts": 3,
        "available_at": "2026-06-27T13:00:00+08:00",
        "locked_at": null,
        "locked_by": null,
        "last_error": null,
        "created_at": "2026-06-27T13:00:00+08:00",
        "updated_at": "2026-06-27T13:00:00+08:00"
    }))
}

fn running_job(job_id: i64, job_type: &str, payload: JsonValue) -> JsonMap<String, JsonValue> {
    let mut row = queued_job(job_id, job_type, payload);
    row.insert("state".to_string(), json!("running"));
    row.insert("attempt_count".to_string(), json!(1));
    row.insert("locked_at".to_string(), json!("2026-06-27T13:00:00+08:00"));
    row.insert("locked_by".to_string(), json!("worker-1"));
    row
}

fn succeeded_job(
    job_id: i64,
    job_type: &str,
    payload: JsonValue,
    result: JsonValue,
) -> JsonMap<String, JsonValue> {
    let mut row = queued_job(job_id, job_type, payload);
    row.insert("state".to_string(), json!("succeeded"));
    row.insert("result_json".to_string(), json!(result.to_string()));
    row
}

fn with_repo_id(mut row: JsonMap<String, JsonValue>, repo_id: &str) -> JsonMap<String, JsonValue> {
    row.insert("repo_id".to_string(), json!(repo_id));
    row
}

fn kernel(pool: InMemoryWorkerQueuePool) -> WorkerQueueKernel<InMemoryWorkerQueuePool> {
    WorkerQueueKernel::new(
        pool,
        SchedulerPolicy::for_host_cpu_cores(10, SchedulerDeploymentPosture::LocalCoResident),
    )
}

#[test]
fn enqueue_list_and_dedupe_are_owned_by_worker_queue_service() {
    let pool = InMemoryWorkerQueuePool::new(Vec::new());
    let kernel = kernel(pool.clone());

    let first = kernel
        .enqueue_job(
            "housekeeper",
            Some("REPO-1"),
            "content.gc",
            &json!({"repo_name": "housekeeper"}),
            None,
            None,
            true,
            "2026-06-27T13:01:00+08:00",
        )
        .expect("enqueue should succeed");
    let duplicate = kernel
        .enqueue_job(
            "housekeeper",
            Some("REPO-1"),
            "content.gc",
            &json!({"repo_name": "housekeeper"}),
            None,
            None,
            true,
            "2026-06-27T13:02:00+08:00",
        )
        .expect("duplicate enqueue should return active job");

    assert_eq!(first["job_id"], duplicate["job_id"]);
    assert_eq!(first["deduplicated"], json!(false));
    assert_eq!(duplicate["deduplicated"], json!(true));
    assert_eq!(first["repo_id"], json!("REPO-1"));
    assert_eq!(first["payload"]["prune_unreferenced"], json!(true));
    assert_eq!(first["max_attempts"], json!(3));
    assert_eq!(pool.rows().len(), 1);

    let listed = kernel
        .list_jobs(Some("housekeeper"), Some("queued"), 10)
        .expect("list should succeed");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["job_type"], json!("content.gc"));
}

#[test]
fn patchset_enqueue_deduplicates_semantic_identity_not_ephemeral_runtime_paths() {
    let pool = InMemoryWorkerQueuePool::new(Vec::new());
    let kernel = kernel(pool.clone());

    let first = kernel
        .enqueue_job(
            "ait-core",
            Some("REPO-CORE"),
            "patchset.ci",
            &json!({
                "repo_name": "ait-core",
                "patchset_id": "RCT-1/C-01/P-01",
                "suite_ids": ["rust_core"],
                "runtime_payload": {"workspace_path": "/ram/run-1/workspace"}
            }),
            None,
            Some(3),
            true,
            "2026-07-17T10:00:00Z",
        )
        .expect("first patchset job should enqueue");
    let duplicate = kernel
        .enqueue_job(
            "ait-core",
            Some("REPO-CORE"),
            "patchset.ci",
            &json!({
                "repo_name": "ait-core",
                "patchset_id": "RCT-1/C-01/P-01",
                "suite_ids": ["rust_core"],
                "runtime_payload": {"workspace_path": "/ram/run-2/workspace"}
            }),
            None,
            Some(3),
            true,
            "2026-07-17T10:00:01Z",
        )
        .expect("same patchset should reuse active queue row");
    let distinct_suite = kernel
        .enqueue_job(
            "ait-core",
            Some("REPO-CORE"),
            "patchset.ci",
            &json!({
                "repo_name": "ait-core",
                "patchset_id": "RCT-1/C-01/P-01",
                "suite_ids": ["tg1_required"],
                "runtime_payload": {"workspace_path": "/ram/run-3/workspace"}
            }),
            None,
            Some(3),
            true,
            "2026-07-17T10:00:02Z",
        )
        .expect("different suite selection must remain a distinct queue row");

    assert_eq!(duplicate["job_id"], first["job_id"]);
    assert_eq!(duplicate["deduplicated"], json!(true));
    assert_ne!(distinct_suite["job_id"], first["job_id"]);
    assert_eq!(distinct_suite["deduplicated"], json!(false));
    assert_eq!(pool.rows().len(), 2);
    assert_eq!(
        duplicate["payload"]["runtime_payload"]["workspace_path"],
        json!("/ram/run-1/workspace")
    );
}

#[test]
fn later_success_reconciles_only_older_same_patchset_queue_rows() {
    let pool = InMemoryWorkerQueuePool::new(vec![
        queued_job(
            1,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-SAME"}),
        ),
        succeeded_job(
            2,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-SAME"}),
            json!({"tests_status": "pass"}),
        ),
        queued_job(
            3,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-OTHER"}),
        ),
        queued_job(
            4,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-SAME"}),
        ),
        with_repo_id(
            queued_job(
                5,
                "patchset.ci",
                json!({"repo_name": "ait-core", "patchset_id": "P-REPO-SCOPE"}),
            ),
            "REPO-A",
        ),
        with_repo_id(
            succeeded_job(
                6,
                "patchset.ci",
                json!({"repo_name": "ait-core", "patchset_id": "P-REPO-SCOPE"}),
                json!({"tests_status": "pass"}),
            ),
            "REPO-B",
        ),
    ]);
    let kernel = kernel(pool.clone());

    let reconciled = kernel
        .reconcile_superseded_patchset_ci_jobs(Some("ait-core"), None, "2026-07-17T10:01:00Z")
        .expect("reconciliation should succeed");

    assert_eq!(
        reconciled
            .iter()
            .map(|job| job["job_id"].as_i64().unwrap())
            .collect::<Vec<_>>(),
        vec![1]
    );
    let rows = pool.rows();
    assert_eq!(rows[0]["state"], json!("succeeded"));
    assert_eq!(rows[2]["state"], json!("queued"));
    assert_eq!(rows[3]["state"], json!("queued"));
    assert_eq!(rows[4]["state"], json!("queued"));
}

#[test]
fn periodic_stale_reclaim_also_reconciles_historical_queued_duplicates() {
    let pool = InMemoryWorkerQueuePool::new(vec![
        queued_job(
            1,
            "patchset.ci",
            json!({
                "repo_name": "ait-core",
                "patchset_id": "P-SAME",
                "suite_ids": ["rust_core"],
                "runtime_payload": {"workspace_path": "/ram/old/workspace"}
            }),
        ),
        succeeded_job(
            2,
            "patchset.ci",
            json!({
                "repo_name": "ait-core",
                "patchset_id": "P-SAME",
                "suite_ids": ["rust_core"],
                "runtime_payload": {"workspace_path": "/ram/new/workspace"}
            }),
            json!({"tests_status": "pass"}),
        ),
        queued_job(
            3,
            "patchset.ci",
            json!({
                "repo_name": "ait-core",
                "patchset_id": "P-SAME",
                "suite_ids": ["tg1_required"],
                "runtime_payload": {"workspace_path": "/ram/distinct/workspace"}
            }),
        ),
    ]);
    let kernel = kernel(pool.clone());

    let summary = kernel
        .reclaim_stale_jobs(
            "2026-07-17T10:00:00Z",
            "2026-07-17T10:01:00Z",
            Some("ait-core"),
        )
        .expect("periodic maintenance should reconcile queued duplicates");

    assert_eq!(summary.stale_count, 0);
    assert_eq!(summary.reconciled_queued_job_ids, vec![1]);
    assert_eq!(pool.rows()[0]["state"], json!("succeeded"));
    assert_eq!(pool.rows()[2]["state"], json!("queued"));
}

#[test]
fn successful_completion_atomically_supersedes_older_same_patchset_queue_row() {
    let pool = InMemoryWorkerQueuePool::new(vec![
        queued_job(
            1,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-SAME"}),
        ),
        running_job(
            2,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-SAME"}),
        ),
        queued_job(
            3,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-OTHER"}),
        ),
    ]);
    let kernel = kernel(pool.clone());

    let completed = kernel
        .complete_job(2, &json!({"tests_status": "pass"}), "2026-07-17T10:01:00Z")
        .expect("completion should reconcile older duplicate");

    assert_eq!(completed["superseded_job_ids"], json!([1]));
    let rows = pool.rows();
    assert_eq!(rows[0]["state"], json!("succeeded"));
    assert_eq!(rows[1]["state"], json!("succeeded"));
    assert_eq!(rows[2]["state"], json!("queued"));
}

#[test]
fn stale_same_patchset_lease_is_superseded_but_valid_retry_remains_queued() {
    let pool = InMemoryWorkerQueuePool::new(vec![
        running_job(
            1,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-SAME"}),
        ),
        succeeded_job(
            2,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-SAME"}),
            json!({"tests_status": "pass"}),
        ),
        running_job(
            3,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "P-RETRY"}),
        ),
    ]);
    let kernel = kernel(pool.clone());

    let summary = kernel
        .reclaim_stale_jobs(
            "2026-06-27T13:01:00+08:00",
            "2026-07-17T10:02:00Z",
            Some("ait-core"),
        )
        .expect("lease reclaim should succeed");

    assert_eq!(summary.stale_count, 2);
    assert_eq!(summary.superseded_job_ids, vec![1]);
    assert_eq!(summary.requeued_job_ids, vec![3]);
    assert!(summary.failed_job_ids.is_empty());
    let rows = pool.rows();
    assert_eq!(rows[0]["state"], json!("succeeded"));
    assert_eq!(rows[2]["state"], json!("queued"));
    assert_eq!(
        rows[2]["last_error"],
        json!("Worker lease expired; job returned to queue")
    );
}

#[test]
fn patchset_ci_job_listing_filters_before_shaping_unrelated_runtime_payloads() {
    let pool = InMemoryWorkerQueuePool::new(vec![
        queued_job(
            1,
            "patchset.ci",
            json!({"repo_name": "ait-server", "patchset_id": "RSEP-A"}),
        ),
        running_job(
            2,
            "patchset.ci.aggregate",
            json!({"repo_name": "ait-server", "patchset_id": "RSEP-A"}),
        ),
        queued_job(
            3,
            "patchset.ci",
            json!({
                "repo_name": "ait-server",
                "patchset_id": "RSEP-B",
                "runtime_payload": {"unrelated": "must not be selected"}
            }),
        ),
        queued_job(
            4,
            "repo.ci",
            json!({"repo_name": "ait-server", "patchset_id": "RSEP-A"}),
        ),
        queued_job(
            5,
            "patchset.ci",
            json!({"repo_name": "ait-core", "patchset_id": "RSEP-A"}),
        ),
    ]);
    let kernel = kernel(pool);

    let all = kernel
        .list_patchset_ci_jobs("ait-server", "RSEP-A", None, 10)
        .expect("patchset-scoped listing");
    assert_eq!(
        all.iter()
            .map(|job| job["job_id"].as_i64().expect("job id"))
            .collect::<Vec<_>>(),
        vec![2, 1]
    );

    let queued = kernel
        .list_patchset_ci_jobs("ait-server", "RSEP-A", Some("queued"), 10)
        .expect("state-filtered patchset listing");
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0]["job_id"], json!(1));
}

#[test]
fn patchset_ci_readiness_listing_preserves_bounded_gate_verdict() {
    let large_detail = "x".repeat(2 * 1024 * 1024);
    let pool = InMemoryWorkerQueuePool::new(vec![succeeded_job(
        4734,
        "patchset.ci",
        json!({
            "repo_name": "ait-server",
            "patchset_id": "RSET-0520/C-01/P-01",
            "suite_ids": ["rust_core"],
            "runtime_payload": {"materialization": large_detail.clone()}
        }),
        json!({
            "tests_status": "fail",
            "selected_suite_ids": ["rust_core"],
            "blocking_failures": ["rust_core"],
            "suite_results": [{
                "suite_id": "rust_core",
                "status": "fail",
                "log": large_detail
            }],
            "attestation_update": {"detail": "must not survive readiness shaping"}
        }),
    )]);
    let kernel = kernel(pool);

    let jobs = kernel
        .list_patchset_ci_readiness_jobs("ait-server", "RSET-0520/C-01/P-01", None, 10)
        .expect("bounded readiness listing");

    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0]["result"]["tests_status"], json!("fail"));
    assert_eq!(
        jobs[0]["result"]["selected_suite_ids"],
        json!(["rust_core"])
    );
    assert_eq!(jobs[0]["result"]["blocking_failures"], json!(["rust_core"]));
    assert_eq!(jobs[0]["result"]["suite_result_count"], json!(1));
    assert_eq!(jobs[0]["result"]["blocking_failure_count"], json!(1));
    assert!(jobs[0]["payload"].get("runtime_payload").is_none());
    assert!(jobs[0]["result"].get("suite_results").is_none());
    assert!(jobs[0]["result"].get("attestation_update").is_none());
    assert!(serde_json::to_vec(&jobs[0]).unwrap().len() < 4096);
}

#[test]
fn worker_queue_service_json_exposes_durable_operations_without_row_payloads() {
    let pool = InMemoryWorkerQueuePool::new(Vec::new());
    let kernel = kernel(pool.clone());

    let enqueued = worker_queue_service_json(
        &kernel,
        &json!({
            "operation": "enqueue-job",
            "repo_name": "housekeeper",
            "repo_id": "REPO-1",
            "job_type": "content.pack",
            "payload": {"repo_name": "housekeeper", "repack": false},
            "dedupe_active": false,
            "now": "2026-06-27T13:01:00+08:00"
        }),
    )
    .expect("service enqueue should succeed");
    assert_eq!(
        enqueued["contract"],
        json!("ait.server.worker_queue.service.v1")
    );
    assert_eq!(enqueued["job"]["payload"]["max_members"], JsonValue::Null);

    let claimed = worker_queue_service_json(
        &kernel,
        &json!({
            "operation": "claim-next-job",
            "worker_id": "worker-1",
            "repo_name": "housekeeper",
            "now": "2026-06-27T13:02:00+08:00"
        }),
    )
    .expect("service claim should succeed");
    assert_eq!(claimed["claimed_job"]["state"], json!("running"));
    assert_eq!(claimed["claimed_job"]["locked_by"], json!("worker-1"));

    let completed = worker_queue_service_json(
        &kernel,
        &json!({
            "operation": "complete-job",
            "job_id": 1,
            "result": {"status": "pass"},
            "now": "2026-06-27T13:03:00+08:00"
        }),
    )
    .expect("service complete should succeed");
    assert_eq!(completed["job"]["state"], json!("succeeded"));
    assert_eq!(completed["job"]["result"]["status"], json!("pass"));
}

#[test]
fn worker_queue_job_json_wrapper_preserves_service_state_result_and_retry_shapes() {
    let pool = InMemoryWorkerQueuePool::new(Vec::new());
    let kernel = kernel(pool.clone());
    let json = WorkerQueueJobJson::stateless();

    let enqueued = json
        .service_json(
            &kernel,
            &json!({
                "operation": "enqueue-job",
                "repo_name": "housekeeper",
                "repo_id": "REPO-1",
                "job_type": "content.pack",
                "payload": {"repo_name": "housekeeper", "repack": false},
                "dedupe_active": false,
                "now": "2026-06-27T13:01:00+08:00"
            }),
        )
        .expect("enqueue");
    assert_eq!(
        enqueued["contract"],
        json!("ait.server.worker_queue.service.v1")
    );
    assert_eq!(enqueued["job"]["state"], json!("queued"));
    assert_eq!(enqueued["job"]["payload"]["max_members"], JsonValue::Null);
    assert_eq!(enqueued["job"]["attempts_remaining"], json!(3));

    let claimed = json
        .service_json(
            &kernel,
            &json!({
                "operation": "claim-next-job",
                "worker_id": "worker-1",
                "repo_name": "housekeeper",
                "now": "2026-06-27T13:02:00+08:00"
            }),
        )
        .expect("claim");
    assert_eq!(claimed["claimed_job"]["state"], json!("running"));
    assert_eq!(claimed["claimed_job"]["locked_by"], json!("worker-1"));

    let failed = json
        .service_json(
            &kernel,
            &json!({
                "operation": "fail-job",
                "job_id": 1,
                "error": "transient failure",
                "retryable": true,
                "retry_available_at": "2026-06-27T13:05:00+08:00",
                "now": "2026-06-27T13:03:00+08:00"
            }),
        )
        .expect("fail");
    assert_eq!(failed["job"]["state"], json!("queued"));
    assert_eq!(failed["job"]["retry_pending"], json!(true));
    assert_eq!(failed["job"]["last_error"], json!("transient failure"));
    assert_eq!(
        failed["job"]["next_retry_at"],
        json!("2026-06-27T13:05:00+08:00")
    );
    assert_eq!(failed["job"]["retry_delay_seconds"], json!(3));

    let completed = json
        .service_json(
            &kernel,
            &json!({
                "operation": "complete-job",
                "job_id": 1,
                "result": {"status": "pass"},
                "now": "2026-06-27T13:06:00+08:00"
            }),
        )
        .expect("complete");
    assert_eq!(completed["job"]["state"], json!("succeeded"));
    assert_eq!(completed["job"]["result"], json!({"status": "pass"}));
}

#[test]
fn worker_queue_job_json_wrapper_preserves_kernel_reclaim_and_diagnostics_shapes() {
    let json = WorkerQueueJobJson::stateless();
    let diagnostics = json
        .job_diagnostics_from_jobs(
            Some("housekeeper"),
            300,
            10,
            "2026-06-27T13:06:00+08:00",
            vec![
                queued_job(
                    1,
                    "content.gc",
                    json!({"repo_name": "housekeeper", "prune_unreferenced": true}),
                ),
                running_job(
                    2,
                    "patchset.ci",
                    json!({
                        "repo_name": "housekeeper",
                        "patchset_id": "RP-1",
                        "suite_id": "tg1_required",
                        "revision_snapshot_id": "SNP-CI"
                    }),
                ),
            ],
        )
        .expect("diagnostics");
    assert_eq!(diagnostics["repo_name"], json!("housekeeper"));
    assert_eq!(diagnostics["job_count"], json!(2));
    assert_eq!(diagnostics["stale_running_jobs"], json!(1));
    assert_eq!(diagnostics["stale_job_ids"], json!([2]));
    assert_eq!(diagnostics["recommended_action"], json!("reclaim_stale"));

    let reclaimed = json
        .kernel_json(&json!({
            "operation": "reclaim-stale-jobs",
            "now": "2026-06-27T13:06:00+08:00",
            "repo_name": "housekeeper",
            "stale_cutoff": "2026-06-27T13:05:00+08:00",
            "jobs": [
                {
                    "job_id": 2,
                    "repo_name": "housekeeper",
                    "job_type": "content.gc",
                    "state": "running",
                    "payload_json": "{\"repo_name\":\"housekeeper\"}",
                    "result_json": "{}",
                    "attempt_count": 1,
                    "max_attempts": 3,
                    "available_at": "2026-06-27T13:00:00+08:00",
                    "locked_at": "2026-06-27T13:00:00+08:00",
                    "locked_by": "worker-1",
                    "last_error": null,
                    "created_at": "2026-06-27T13:00:00+08:00",
                    "updated_at": "2026-06-27T13:00:00+08:00"
                }
            ]
        }))
        .expect("reclaim");
    assert_eq!(
        reclaimed["contract"],
        json!("ait.server.worker_queue.kernel.v1")
    );
    assert_eq!(reclaimed["stale_count"], json!(1));
    assert_eq!(reclaimed["requeued_job_ids"], json!([2]));
    assert_eq!(reclaimed["failed_job_ids"], json!([]));
}

#[test]
fn worker_queue_job_json_wrapper_preserves_exact_error_text() {
    let pool = InMemoryWorkerQueuePool::new(vec![running_job(
        1,
        "patchset.ci",
        json!({
            "repo_name": "housekeeper",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-CI"
        }),
    )]);
    let kernel = kernel(pool);
    let json = WorkerQueueJobJson::stateless();

    assert_eq!(
        json.service_json(&kernel, &json!({"operation": "get-job"}))
            .expect_err("missing job id"),
        "worker queue kernel payload requires integer `job_id`."
    );
    assert_eq!(
        json.service_json(
            &kernel,
            &json!({
                "operation": "claim-job",
                "job_id": 1,
                "worker_id": "worker-2",
                "repo_name": "housekeeper",
                "now": "2026-06-27T13:02:00+08:00"
            }),
        )
        .expect_err("stale claim"),
        "Cannot claim job 1: expected queued state, got `running`."
    );
    assert_eq!(
        json.kernel_json(&json!({
            "operation": "delete-job",
            "now": "2026-06-27T13:02:00+08:00",
            "jobs": []
        }))
        .expect_err("unsupported operation"),
        "Unsupported worker queue kernel operation `delete-job`. Expected one of: claim-next-job, claim-job, heartbeat-job, complete-job, fail-job, reclaim-stale-jobs."
    );
}

#[test]
fn worker_queue_service_json_reports_job_diagnostics_from_rust_service() {
    let mut delayed = queued_job(
        1,
        "content.gc",
        json!({"repo_name": "housekeeper", "prune_unreferenced": true}),
    );
    delayed.insert("attempt_count".to_string(), json!(1));
    delayed.insert("last_error".to_string(), json!("transient failure"));
    delayed.insert(
        "available_at".to_string(),
        json!("2026-06-27T13:10:00+08:00"),
    );
    let stale = running_job(
        2,
        "patchset.ci",
        json!({
            "repo_name": "housekeeper",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-CI",
        }),
    );
    let mut exhausted = queued_job(
        3,
        "repo.ci",
        json!({
            "repo_name": "housekeeper",
            "suite_ids": ["full_repo"],
            "plane": "nightly",
            "snapshot_id": "SNP-FULL",
        }),
    );
    exhausted.insert("state".to_string(), json!("failed"));
    exhausted.insert("attempt_count".to_string(), json!(3));
    exhausted.insert("last_error".to_string(), json!("hard failure"));
    let pool = InMemoryWorkerQueuePool::new(vec![delayed, stale, exhausted]);
    let kernel = kernel(pool);

    let response = worker_queue_service_json(
        &kernel,
        &json!({
            "operation": "job-diagnostics",
            "repo_name": "housekeeper",
            "stale_after_seconds": 300,
            "limit": 10,
            "now": "2026-06-27T13:06:00+08:00",
        }),
    )
    .expect("service diagnostics should succeed");
    let diagnostics = &response["diagnostics"];

    assert_eq!(response["operation"], json!("job-diagnostics"));
    assert_eq!(diagnostics["job_count"], json!(3));
    assert_eq!(diagnostics["stale_running_jobs"], json!(1));
    assert_eq!(diagnostics["stale_job_ids"], json!([2]));
    assert_eq!(diagnostics["delayed_retry_jobs"], json!(1));
    assert_eq!(diagnostics["delayed_retry_job_ids"], json!([1]));
    assert_eq!(diagnostics["exhausted_jobs"], json!(1));
    assert_eq!(diagnostics["exhausted_job_ids"], json!([3]));
    assert_eq!(diagnostics["failed_jobs"], json!(1));
    assert_eq!(diagnostics["recommended_action"], json!("reclaim_stale"));
    assert_eq!(diagnostics["state_summary"]["failed"], json!(1));
    assert_eq!(diagnostics["job_type_summary"]["patchset.ci"], json!(1));
    assert_eq!(diagnostics["recent_jobs"].as_array().unwrap().len(), 3);
}

#[test]
fn diagnostics_find_old_queued_work_hidden_behind_recent_successes() {
    let mut rows = vec![queued_job(
        1,
        "patchset.ci",
        json!({
            "repo_name": "housekeeper",
            "patchset_id": "RP-OLD",
            "revision_snapshot_id": "SNP-OLD",
        }),
    )];
    rows.extend((2..=26).map(|job_id| {
        succeeded_job(
            job_id,
            "repo.ci",
            json!({
                "repo_name": "housekeeper",
                "suite_ids": ["full_repo"],
                "plane": "nightly",
                "snapshot_id": format!("SNP-{job_id}"),
            }),
            json!({"tests_status": "pass"}),
        )
    }));
    let pool = InMemoryWorkerQueuePool::new(rows);
    let kernel = kernel(pool);

    let public_page = kernel
        .list_jobs(Some("housekeeper"), None, 100)
        .expect("generic list should remain bounded");
    assert_eq!(public_page.len(), 20);
    assert!(public_page.iter().all(|job| job["state"] == "succeeded"));

    let response = worker_queue_service_json(
        &kernel,
        &json!({
            "operation": "job-diagnostics",
            "repo_name": "housekeeper",
            "stale_after_seconds": 300,
            "limit": 100,
            "now": "2026-06-27T13:06:00+08:00",
        }),
    )
    .expect("state-aware diagnostics should succeed");
    let diagnostics = &response["diagnostics"];

    assert_eq!(
        diagnostics["diagnostic_projection"],
        json!("state_aware_compact")
    );
    assert_eq!(diagnostics["state_summary"]["queued"], json!(1));
    assert_eq!(diagnostics["retryable_job_ids"], json!([1]));
    assert_eq!(diagnostics["recommended_action"], json!("monitor_workers"));
    assert_eq!(diagnostics["recent_job_count"], json!(20));
    assert_eq!(diagnostics["recent_jobs"].as_array().unwrap().len(), 20);
}

#[test]
fn worker_queue_diagnostics_reports_main_seed_refresh_recovery() {
    let mut delayed_refresh = queued_job(
        1,
        "main-seed.refresh",
        json!({
            "repo_name": "housekeeper",
            "snapshot_id": "SNP-LANDED",
            "patchset_id": "P-LAND",
            "previous_snapshot_id": "SNP-BASE",
        }),
    );
    delayed_refresh.insert("attempt_count".to_string(), json!(1));
    delayed_refresh.insert(
        "last_error".to_string(),
        json!("Main-seed prewarm failed: patchset-ci-prewarm-001 failed"),
    );
    delayed_refresh.insert(
        "available_at".to_string(),
        json!("2026-06-27T13:10:00+08:00"),
    );
    let mut exhausted_refresh = queued_job(
        2,
        "main-seed.refresh",
        json!({
            "repo_name": "housekeeper",
            "snapshot_id": "SNP-NEWER",
            "patchset_id": "P-LAND-2",
            "previous_snapshot_id": "SNP-LANDED",
        }),
    );
    exhausted_refresh.insert("state".to_string(), json!("failed"));
    exhausted_refresh.insert("attempt_count".to_string(), json!(3));
    exhausted_refresh.insert(
        "last_error".to_string(),
        json!("Main-seed prewarm failed after retries"),
    );
    let pool = InMemoryWorkerQueuePool::new(vec![delayed_refresh, exhausted_refresh]);
    let kernel = kernel(pool);

    let response = worker_queue_service_json(
        &kernel,
        &json!({
            "operation": "job-diagnostics",
            "repo_name": "housekeeper",
            "stale_after_seconds": 300,
            "limit": 10,
            "now": "2026-06-27T13:06:00+08:00",
        }),
    )
    .expect("service diagnostics should succeed");
    let diagnostics = &response["diagnostics"];

    assert_eq!(diagnostics["recommended_action"], json!("inspect_failed"));
    assert_eq!(diagnostics["main_seed_refresh"]["job_count"], json!(2));
    assert_eq!(
        diagnostics["main_seed_refresh"]["delayed_retry_jobs"],
        json!(1)
    );
    assert_eq!(
        diagnostics["main_seed_refresh"]["delayed_retry_job_ids"],
        json!([1])
    );
    assert_eq!(diagnostics["main_seed_refresh"]["failed_jobs"], json!(1));
    assert_eq!(
        diagnostics["main_seed_refresh"]["failed_job_ids"],
        json!([2])
    );
    assert_eq!(diagnostics["main_seed_refresh"]["exhausted_jobs"], json!(1));
    assert_eq!(
        diagnostics["main_seed_refresh"]["requires_attention"],
        json!(true)
    );
    assert_eq!(diagnostics["main_seed_refresh_job_count"], json!(2));
    assert_eq!(diagnostics["main_seed_refresh_failed_job_ids"], json!([2]));
}

#[test]
fn worker_queue_service_claims_specific_job_for_background_handoff() {
    let pool = InMemoryWorkerQueuePool::new(vec![
        queued_job(
            1,
            "repo.ci",
            json!({
                "repo_name": "housekeeper",
                "suite_ids": ["full_repo"],
                "plane": "nightly",
                "snapshot_id": "SNP-FULL",
            }),
        ),
        queued_job(
            2,
            "patchset.ci",
            json!({
                "repo_name": "housekeeper",
                "patchset_id": "RP-1",
                "suite_id": "tg1_required",
                "revision_snapshot_id": "SNP-CI",
            }),
        ),
    ]);
    let kernel = kernel(pool.clone());

    let claimed = worker_queue_service_json(
        &kernel,
        &json!({
            "operation": "claim-job",
            "job_id": 2,
            "worker_id": "ait-patchset-ci-rp-1-manual",
            "repo_name": "housekeeper",
            "now": "2026-06-27T13:02:00+08:00"
        }),
    )
    .expect("specific service claim should succeed");

    assert_eq!(claimed["claimed_job"]["job_id"], json!(2));
    assert_eq!(claimed["claimed_job"]["state"], json!("running"));
    assert_eq!(
        claimed["claimed_job"]["locked_by"],
        json!("ait-patchset-ci-rp-1-manual")
    );
    assert_eq!(
        claimed["claimed_job"]["locked_at"],
        json!("2026-06-27T13:02:00+08:00")
    );
    assert_eq!(pool.rows()[0]["state"], json!("queued"));
    assert_eq!(pool.rows()[1]["state"], json!("running"));
}

#[test]
fn worker_queue_service_rejects_specific_claim_for_non_queued_job() {
    let pool = InMemoryWorkerQueuePool::new(vec![running_job(
        1,
        "patchset.ci",
        json!({
            "repo_name": "housekeeper",
            "patchset_id": "RP-1",
            "suite_id": "tg1_required",
            "revision_snapshot_id": "SNP-CI",
        }),
    )]);
    let kernel = kernel(pool);

    let error = worker_queue_service_json(
        &kernel,
        &json!({
            "operation": "claim-job",
            "job_id": 1,
            "worker_id": "worker-2",
            "repo_name": "housekeeper",
            "now": "2026-06-27T13:02:00+08:00"
        }),
    )
    .expect_err("claiming another worker's running job should fail");

    assert!(error.contains("expected queued state"));
}

#[test]
fn claim_uses_connection_pool_and_scheduler_prioritizes_ci_over_full_test() {
    let pool = InMemoryWorkerQueuePool::new(vec![
        queued_job(
            1,
            "repo.ci",
            json!({
                "repo_name": "housekeeper",
                "suite_ids": ["full_repo"],
                "plane": "nightly",
                "snapshot_id": "SNP-FULL",
            }),
        ),
        queued_job(
            2,
            "patchset.ci",
            json!({
                "repo_name": "housekeeper",
                "patchset_id": "RP-1",
                "suite_id": "tg1_required",
                "revision_snapshot_id": "SNP-CI",
            }),
        ),
    ]);

    let claimed = kernel(pool.clone())
        .claim_next_job("worker-1", "2026-06-27T13:01:00+08:00", Some("housekeeper"))
        .expect("claim should succeed")
        .expect("job should be claimed");

    assert_eq!(claimed["job_id"], json!(2));
    assert_eq!(claimed["admitted_cpu_tokens"], json!(10));
    assert_eq!(
        claimed["scheduler_admission"]["decision"]["kind"],
        json!("admit")
    );
    assert_eq!(pool.rows()[0]["state"], json!("queued"));
    assert_eq!(pool.rows()[1]["state"], json!("running"));
    assert_eq!(pool.stats()["checkout_count"], json!(1));
}

#[test]
fn full_test_claim_carries_scheduler_cpu_tokens_for_sharding() {
    let pool = InMemoryWorkerQueuePool::new(vec![queued_job(
        1,
        "repo.ci",
        json!({
            "repo_name": "housekeeper",
            "suite_ids": ["full_repo"],
            "plane": "nightly",
            "snapshot_id": "SNP-FULL",
        }),
    )]);

    let claimed = kernel(pool)
        .claim_next_job("worker-1", "2026-06-27T13:01:00+08:00", Some("housekeeper"))
        .expect("claim should succeed")
        .expect("full-test should be claimed");

    assert_eq!(claimed["job_id"], json!(1));
    assert_eq!(claimed["admitted_cpu_tokens"], json!(10));
    assert_eq!(
        claimed["scheduler_admission"]["decision"]["job"]["token_pools"],
        json!([
            "global_cpu_tokens",
            "ci_full_shared_cpu_tokens",
            "full_test_cpu_tokens"
        ])
    );
}

#[test]
fn duplicate_full_test_attaches_before_next_scheduler_admission() {
    let duplicate_payload = json!({
        "repo_name": "housekeeper",
        "suite_ids": ["full_repo"],
        "plane": "nightly",
        "snapshot_id": "SNP-FULL",
    });
    let pool = InMemoryWorkerQueuePool::new(vec![
        running_job(1, "repo.ci", duplicate_payload.clone()),
        queued_job(2, "repo.ci", duplicate_payload),
        queued_job(
            3,
            "patchset.ci",
            json!({
                "repo_name": "housekeeper",
                "patchset_id": "RP-1",
                "suite_id": "stable_smoke",
                "revision_snapshot_id": "SNP-CI",
            }),
        ),
    ]);

    let claimed = WorkerQueueKernel::new(
        pool.clone(),
        SchedulerPolicy::for_host_cpu_cores(32, SchedulerDeploymentPosture::DedicatedServer),
    )
    .claim_next_job("worker-2", "2026-06-27T13:01:00+08:00", Some("housekeeper"))
    .expect("claim should succeed")
    .expect("normal ci should be claimed after duplicate attach");

    let rows = pool.rows();
    assert_eq!(claimed["job_id"], json!(3));
    assert_eq!(rows[1]["state"], json!("succeeded"));
    let attached_result: JsonValue =
        serde_json::from_str(rows[1]["result_json"].as_str().unwrap()).unwrap();
    assert_eq!(attached_result["status"], json!("attached"));
    assert_eq!(attached_result["scheduler"]["active_job_id"], json!("1"));
}

#[test]
fn complete_fail_and_reclaim_are_owned_by_worker_queue_kernel() {
    let pool = InMemoryWorkerQueuePool::new(vec![
        queued_job(
            1,
            "content.pack",
            json!({"repo_name": "housekeeper", "repack": false}),
        ),
        running_job(
            2,
            "content.gc",
            json!({"repo_name": "housekeeper", "prune_unreferenced": true}),
        ),
        running_job(
            3,
            "content.gc",
            json!({"repo_name": "housekeeper", "prune_unreferenced": true}),
        ),
    ]);
    {
        let mut rows = pool.rows();
        rows[2].insert("attempt_count".to_string(), json!(3));
    }
    let kernel = kernel(pool.clone());

    let claimed = kernel
        .claim_next_job("worker-1", "2026-06-27T13:01:00+08:00", Some("housekeeper"))
        .expect("claim should succeed")
        .expect("content pack should claim by fifo fallback");
    assert_eq!(claimed["job_id"], json!(1));

    let completed = kernel
        .complete_job(1, &json!({"status": "pass"}), "2026-06-27T13:02:00+08:00")
        .expect("complete should succeed");
    assert_eq!(completed["state"], json!("succeeded"));
    assert_eq!(completed["result"], json!({"status": "pass"}));

    let failed = kernel
        .fail_job(
            2,
            "transient failure",
            true,
            Some("2026-06-27T13:03:00+08:00"),
            "2026-06-27T13:02:30+08:00",
        )
        .expect("fail should succeed");
    assert_eq!(failed["state"], json!("queued"));
    assert_eq!(failed["retry_delay_seconds"], json!(3));
    assert_eq!(failed["next_retry_at"], json!("2026-06-27T13:03:00+08:00"));

    let summary = kernel
        .reclaim_stale_jobs(
            "2026-06-27T13:01:00+08:00",
            "2026-06-27T13:04:00+08:00",
            Some("housekeeper"),
        )
        .expect("reclaim should succeed");
    assert_eq!(summary.stale_count, 1);
    assert_eq!(summary.requeued_job_ids, vec![3]);
    assert_eq!(summary.failed_job_ids, Vec::<i64>::new());
}

#[test]
fn ci_completion_persists_only_bounded_operational_result() {
    let pool = InMemoryWorkerQueuePool::new(vec![running_job(
        1,
        "patchset.ci",
        json!({"repo_name": "ait-core", "patchset_id": "P-LARGE"}),
    )]);
    let kernel = kernel(pool.clone());
    let large_detail = "x".repeat(2 * 1024 * 1024);

    let completed = kernel
        .complete_job(
            1,
            &json!({
                "tests_status": "pass",
                "selected_suite_ids": ["rust_core"],
                "suite_results": [{"suite_id": "rust_core", "log": large_detail}],
                "attestation_update": {"detail": "must not be returned"}
            }),
            "2026-07-14T09:45:00Z",
        )
        .expect("complete should succeed");

    let encoded = serde_json::to_vec(&completed).unwrap();
    assert_eq!(completed["state"], json!("succeeded"));
    assert_eq!(completed["result"]["tests_status"], json!("pass"));
    assert_eq!(completed["result"]["suite_result_count"], json!(1));
    assert!(completed["result"].get("suite_results").is_none());
    assert!(completed["result"].get("attestation_update").is_none());
    assert!(
        encoded.len() < 4096,
        "completion ack was {} bytes",
        encoded.len()
    );

    let persisted = pool.rows();
    let persisted_result: serde_json::Value =
        serde_json::from_str(persisted[0]["result_json"].as_str().unwrap()).unwrap();
    assert_eq!(
        persisted_result["storage_contract"],
        json!("ait.server.worker_queue.ci_result_summary.v1")
    );
    assert_eq!(persisted_result["suite_result_count"], json!(1));
    assert_eq!(
        persisted_result["suite_results"][0]["suite_id"],
        json!("rust_core")
    );
    assert!(persisted_result["suite_results"][0].get("log").is_none());
    assert!(persisted_result.get("attestation_update").is_none());
    assert!(persisted[0]["result_json"].as_str().unwrap().len() < 16 * 1024);

    let detail = kernel
        .get_job(1)
        .expect("explicit job detail should succeed");
    assert_eq!(
        detail["result"]["suite_results"][0]["suite_id"],
        "rust_core"
    );
    assert!(detail["result"]["suite_results"][0].get("log").is_none());
    assert!(serde_json::to_vec(&detail).unwrap().len() < 32 * 1024);
}

#[test]
fn non_ci_completion_preserves_its_result_contract() {
    let pool = InMemoryWorkerQueuePool::new(vec![running_job(
        1,
        "main-seed.refresh",
        json!({"repo_name": "ait-core", "snapshot_id": "SNP-1"}),
    )]);
    let kernel = kernel(pool.clone());
    let result = json!({
        "status": "complete",
        "implementation_specific": {"preserved": true},
    });

    kernel
        .complete_job(1, &result, "2026-07-14T09:45:00Z")
        .expect("complete should succeed");

    let persisted: serde_json::Value =
        serde_json::from_str(pool.rows()[0]["result_json"].as_str().unwrap()).unwrap();
    assert_eq!(persisted, result);
}

#[test]
fn oversized_non_ci_completion_persists_only_a_bounded_summary() {
    let pool = InMemoryWorkerQueuePool::new(vec![running_job(
        1,
        "main-seed.refresh",
        json!({"repo_name": "ait", "snapshot_id": "SNP-LARGE"}),
    )]);
    let kernel = kernel(pool.clone());
    let result = json!({
        "contract": "ait.server.land.main_seed_refresh.v1",
        "status": "updated",
        "repo_name": "ait",
        "snapshot_id": "SNP-LARGE",
        "source_materialization": {"detail": "x".repeat(1024 * 1024)},
        "revision_snapshot_materialize": {"detail": "y".repeat(512 * 1024)},
    });

    kernel
        .complete_job(1, &result, "2026-07-17T05:30:00Z")
        .expect("complete should succeed");

    let persisted = pool.rows();
    let persisted_text = persisted[0]["result_json"].as_str().unwrap();
    let persisted_result: serde_json::Value = serde_json::from_str(persisted_text).unwrap();
    assert!(persisted_text.len() <= 256 * 1024);
    assert_eq!(
        persisted_result["storage_contract"],
        json!("ait.server.worker_queue.result_summary.v1")
    );
    assert_eq!(persisted_result["job_type"], json!("main-seed.refresh"));
    assert_eq!(persisted_result["storage_detail_truncated"], json!(true));
    assert!(persisted_result.get("source_materialization").is_none());
    assert!(persisted_result
        .get("revision_snapshot_materialize")
        .is_none());
}
