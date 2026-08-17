use crate::foundation::async_job_json::WorkerQueueJobJson;
use crate::foundation::db::{
    connect_server_plane, NativePostgresDriver, PostgresConnectionPoolRegistry,
    PostgresDbConnection, PostgresTimeoutScope,
};
use crate::foundation::scheduler::{
    admit_next, scheduler_queued_job_from_async_job_with_policy,
    scheduler_running_job_from_async_job_with_policy, SchedulerAdmissionDecision, SchedulerPolicy,
    SchedulerQueuedJob, SchedulerRunningJob,
};
use crate::foundation::transport::{
    max_attempts_for_job, normalize_async_job_payload, retry_delay_seconds_for_job, row_to_job,
};
use ::postgres::Row;
use chrono::{DateTime, Duration, Utc};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

#[path = "worker_queue/rows.rs"]
mod rows;
pub use rows::{compact_ci_job_result_for_storage, compact_job_result_for_storage, utc_now_string};
#[cfg(test)]
use rows::{postgres_int4, postgres_timestamptz};

#[path = "worker_queue/scheduler_projection.rs"]
mod scheduler_projection;

#[path = "worker_queue/kernel.rs"]
mod kernel;
pub use kernel::{
    worker_queue_job_diagnostics_from_jobs, worker_queue_kernel_json, worker_queue_service_json,
    WorkerQueueClaimCapabilities, WorkerQueueConnection, WorkerQueueConnectionPool,
    WorkerQueueKernel, WorkerQueueReclaimSummary,
};
pub(crate) use kernel::{
    worker_queue_job_diagnostics_from_jobs_impl, worker_queue_kernel_json_impl,
    worker_queue_service_json_impl,
};

#[path = "worker_queue/in_memory.rs"]
mod in_memory;
pub use in_memory::{InMemoryWorkerQueueConnection, InMemoryWorkerQueuePool};

#[path = "worker_queue/postgres.rs"]
mod postgres;
pub use postgres::{PostgresWorkerQueueConnection, PostgresWorkerQueuePool};

#[cfg(test)]
mod tests {
    use super::{
        compact_ci_job_result_for_storage, compact_job_result_for_storage, postgres_int4,
        postgres_timestamptz, InMemoryWorkerQueuePool, WorkerQueueClaimCapabilities,
        WorkerQueueKernel,
    };
    use serde_json::{json, Map as JsonMap};

    #[cfg(feature = "perfetto-tracing")]
    #[test]
    #[ignore = "release-profile Perfetto evidence harness"]
    fn perfetto_enqueue_and_semantic_dedupe_500_by_30() {
        let trace_path = std::env::temp_dir().join(format!(
            "ait-server-worker-queue-perfetto-{}.json",
            std::process::id()
        ));
        for _sample in 0..30 {
            let pool = InMemoryWorkerQueuePool::new(Vec::new());
            let kernel = WorkerQueueKernel::new(pool.clone(), Default::default());
            let range = crate::perfetto_trace::PerfettoRange::for_test(
                "ait.server.worker_queue.perf.normal_enqueue_500",
                trace_path.clone(),
            );
            for _ in 0..500 {
                std::hint::black_box(
                    kernel
                        .enqueue_job(
                            "ait-core",
                            Some("REPO-CORE"),
                            "content.gc",
                            &json!({"repo_name": "ait-core"}),
                            None,
                            Some(3),
                            false,
                            "2026-07-18T08:00:00Z",
                        )
                        .expect("normal enqueue benchmark operation"),
                );
            }
            drop(range);
            assert_eq!(pool.rows().len(), 500);
        }

        for sample in 0..30 {
            let pool = InMemoryWorkerQueuePool::new(Vec::new());
            let kernel = WorkerQueueKernel::new(pool.clone(), Default::default());
            let initial = kernel
                .enqueue_job(
                    "ait-core",
                    Some("REPO-CORE"),
                    "patchset.ci",
                    &json!({
                        "repo_name": "ait-core",
                        "patchset_id": format!("RP-PERF-{sample}"),
                        "suite_ids": ["rust_core"],
                        "runtime_payload": {"workspace_path": "/ram/seed/workspace"},
                    }),
                    None,
                    Some(3),
                    true,
                    "2026-07-18T08:00:00Z",
                )
                .expect("semantic dedupe benchmark seed");
            let range = crate::perfetto_trace::PerfettoRange::for_test(
                "ait.server.worker_queue.perf.semantic_dedupe_500",
                trace_path.clone(),
            );
            for index in 0..500 {
                let duplicate = kernel
                    .enqueue_job(
                        "ait-core",
                        Some("REPO-CORE"),
                        "patchset.ci",
                        &json!({
                            "repo_name": "ait-core",
                            "patchset_id": format!("RP-PERF-{sample}"),
                            "suite_ids": ["rust_core"],
                            "runtime_payload": {
                                "workspace_path": format!("/ram/run-{index}/workspace")
                            },
                        }),
                        None,
                        Some(3),
                        true,
                        "2026-07-18T08:00:00Z",
                    )
                    .expect("semantic dedupe benchmark operation");
                assert_eq!(duplicate["job_id"], initial["job_id"]);
                std::hint::black_box(duplicate);
            }
            drop(range);
            assert_eq!(pool.rows().len(), 1);
        }
        let _ = std::fs::remove_file(trace_path);
    }

    #[test]
    fn postgres_int4_rejects_out_of_range_values() {
        assert_eq!(postgres_int4("max_attempts", 3), Ok(3));
        assert!(postgres_int4("max_attempts", i64::from(i32::MAX) + 1).is_err());
        assert!(postgres_int4("max_attempts", i64::from(i32::MIN) - 1).is_err());
    }

    #[test]
    fn postgres_timestamptz_accepts_rfc3339_and_postgres_text() {
        assert!(postgres_timestamptz("now", "2026-06-28T12:16:57+00:00").is_ok());
        assert!(postgres_timestamptz("now", "2026-06-28 20:16:57+08").is_ok());
    }

    #[test]
    fn durable_ci_result_omits_duplicate_authority_bodies_and_bounds_suite_evidence() {
        let large_links = (0..900)
            .map(|index| json!({"path": format!("src/file-{index}.rs")}))
            .collect::<Vec<_>>();
        let full = json!({
            "contract": "ait.server.patchset_ci.run.v1",
            "patchset_id": "RP-1",
            "change_id": "RC-1",
            "repo_name": "ait-core",
            "tests_status": "pass",
            "blocking_failures": [],
            "suite_results": [{
                "suite_id": "rust_core",
                "status": "pass",
                "blocking": true,
                "artifacts": {"log_path": "/tmp/rust-core.log"},
                "shard_run": {"immutable_links": large_links},
            }],
            "patchset_ci_detail": {"large": "x".repeat(2 * 1024 * 1024)},
            "attestation_update": {"detail": {"large": "x".repeat(2 * 1024 * 1024)}},
        });

        let compact = compact_ci_job_result_for_storage("patchset.ci", &full);
        let encoded = serde_json::to_vec(&compact).unwrap();

        assert_eq!(
            compact["storage_contract"],
            json!("ait.server.worker_queue.ci_result_summary.v1")
        );
        assert_eq!(compact["suite_result_count"], json!(1));
        assert_eq!(
            compact["suite_results"][0]["artifacts"]["log_path"],
            json!("/tmp/rust-core.log")
        );
        assert!(compact["suite_results"][0].get("shard_run").is_none());
        assert!(compact.get("patchset_ci_detail").is_none());
        assert!(compact.get("attestation_update").is_none());
        assert!(
            encoded.len() < 16 * 1024,
            "durable CI result was {} bytes",
            encoded.len()
        );
    }

    #[test]
    fn durable_ci_result_has_a_hard_serialized_size_limit() {
        let oversized_suite_results = (0..64)
            .map(|index| {
                json!({
                    "suite_id": format!("suite-{index}-{}", "s".repeat(8 * 1024)),
                    "display_name": "d".repeat(8 * 1024),
                    "status": "pass",
                    "artifact_path": format!("/tmp/{}/artifact.json", "a".repeat(8 * 1024)),
                    "artifacts": {
                        "summary_json": "j".repeat(8 * 1024),
                        "log_path": "l".repeat(8 * 1024),
                    },
                    "summary": "m".repeat(8 * 1024),
                    "checks": "c".repeat(8 * 1024),
                    "execution": "e".repeat(8 * 1024),
                })
            })
            .collect::<Vec<_>>();
        let full = json!({
            "contract": "ait.server.patchset_ci.run.v1",
            "patchset_id": "RP-BOUND",
            "tests_status": "pass",
            "suite_results": oversized_suite_results,
            "artifacts": {"large": "x".repeat(2 * 1024 * 1024)},
        });

        let compact = compact_ci_job_result_for_storage("patchset.ci", &full);
        let encoded = serde_json::to_vec(&compact).unwrap();

        assert_eq!(compact["suite_result_count"], json!(64));
        assert_eq!(compact["storage_detail_truncated"], json!(true));
        assert!(
            encoded.len() <= 256 * 1024,
            "durable CI result exceeded the hard limit at {} bytes",
            encoded.len()
        );
    }

    #[test]
    fn durable_non_ci_result_preserves_small_contracts_and_bounds_large_evidence() {
        let small = json!({
            "contract": "ait.server.land.main_seed_refresh.v1",
            "status": "updated",
            "snapshot_id": "SNP-SMALL",
            "implementation_specific": {"preserved": true},
        });
        assert_eq!(
            compact_job_result_for_storage("main-seed.refresh", &small),
            small
        );

        let oversized = json!({
            "contract": "ait.server.land.main_seed_refresh.v1",
            "status": "updated",
            "repo_name": "ait",
            "snapshot_id": "SNP-LARGE",
            "source_materialization": {"detail": "x".repeat(1024 * 1024)},
            "revision_snapshot_materialize": {"detail": "y".repeat(512 * 1024)},
        });
        let compact = compact_job_result_for_storage("main-seed.refresh", &oversized);
        let encoded = serde_json::to_vec(&compact).unwrap();

        assert_eq!(
            compact["storage_contract"],
            json!("ait.server.worker_queue.result_summary.v1")
        );
        assert_eq!(compact["job_type"], json!("main-seed.refresh"));
        assert_eq!(compact["status"], json!("updated"));
        assert_eq!(compact["snapshot_id"], json!("SNP-LARGE"));
        assert_eq!(compact["storage_detail_truncated"], json!(true));
        assert!(compact["original_result_bytes"].as_u64().unwrap() > 256 * 1024);
        assert!(compact.get("source_materialization").is_none());
        assert!(compact.get("revision_snapshot_materialize").is_none());
        assert!(
            encoded.len() <= 256 * 1024,
            "durable non-CI result exceeded the hard limit at {} bytes",
            encoded.len()
        );
    }

    #[test]
    fn polled_job_reads_discard_large_raw_bodies_and_preserve_readiness_evidence() {
        let large_detail = "x".repeat(2 * 1024 * 1024);
        let row = JsonMap::from_iter([
            ("job_id".to_string(), json!(77)),
            ("repo_name".to_string(), json!("ait-core")),
            ("repo_id".to_string(), json!("REPO-1")),
            ("job_type".to_string(), json!("patchset.ci")),
            ("state".to_string(), json!("succeeded")),
            (
                "payload_json".to_string(),
                json!(json!({
                    "patchset_id": "RP-1",
                    "suite_ids": ["rust_core"],
                    "runtime_payload": {"materialization": large_detail.clone()}
                })
                .to_string()),
            ),
            (
                "result_json".to_string(),
                json!(json!({
                    "tests_status": "pass",
                    "selected_suite_ids": ["rust_core"],
                    "suite_results": [{"suite_id": "rust_core", "log": large_detail}]
                })
                .to_string()),
            ),
            ("attempt_count".to_string(), json!(1)),
            ("max_attempts".to_string(), json!(3)),
            ("available_at".to_string(), json!("2026-07-14T00:00:00Z")),
            ("locked_at".to_string(), json!(null)),
            ("locked_by".to_string(), json!(null)),
            ("last_error".to_string(), json!(null)),
            ("created_at".to_string(), json!("2026-07-14T00:00:00Z")),
            ("updated_at".to_string(), json!("2026-07-14T00:00:01Z")),
        ]);
        let kernel =
            WorkerQueueKernel::new(InMemoryWorkerQueuePool::new(vec![row]), Default::default());

        let jobs = kernel
            .list_patchset_ci_status_jobs("ait-core", "RP-1", None, 20)
            .unwrap();
        let encoded = serde_json::to_vec(&jobs).unwrap();

        assert_eq!(jobs[0]["result"]["suite_result_count"], json!(1));
        assert!(jobs[0]["payload"].get("runtime_payload").is_none());
        assert!(jobs[0]["result"].get("suite_results").is_none());
        assert!(
            encoded.len() < 4096,
            "status jobs were {} bytes",
            encoded.len()
        );

        let readiness = kernel
            .list_patchset_ci_readiness_jobs("ait-core", "RP-1", None, 20)
            .unwrap();
        let readiness_encoded = serde_json::to_vec(&readiness).unwrap();
        assert_eq!(readiness[0]["state"], json!("succeeded"));
        assert_eq!(readiness[0]["payload"]["patchset_id"], json!("RP-1"));
        assert_eq!(
            readiness[0]["result"],
            json!({
                "tests_status": "pass",
                "selected_suite_ids": ["rust_core"],
                "suite_result_count": 1,
            })
        );
        assert!(
            readiness_encoded.len() < 2048,
            "readiness jobs were {} bytes",
            readiness_encoded.len()
        );

        let listed = kernel.list_jobs(Some("ait-core"), None, 200).unwrap();
        let listed_encoded = serde_json::to_vec(&listed).unwrap();
        assert_eq!(listed[0]["payload"], json!({}));
        assert_eq!(listed[0]["result"], json!({}));
        assert!(
            listed_encoded.len() < 2048,
            "listed jobs were {} bytes",
            listed_encoded.len()
        );
    }

    #[test]
    fn claim_capabilities_keep_native_runner_jobs_out_of_inline_workers() {
        let pool = InMemoryWorkerQueuePool::new(Vec::new());
        let kernel = WorkerQueueKernel::new(pool, Default::default());
        let now = "2026-07-29T08:00:00Z";
        let runner = kernel
            .enqueue_job(
                "ait-core",
                Some("REPO-CORE"),
                "repo.ci",
                &json!({
                    "repo_name": "ait-core",
                    "runtime_payload": {"contract": "ait.runner.native-job.v1"},
                }),
                None,
                Some(3),
                false,
                now,
            )
            .unwrap();
        let inline = kernel
            .enqueue_job(
                "ait-core",
                Some("REPO-CORE"),
                "repo.ci",
                &json!({
                    "repo_name": "ait-core",
                    "runtime_payload": {"contract": "ait.server.repo_ci.run.v1"},
                }),
                None,
                Some(3),
                false,
                now,
            )
            .unwrap();

        let claimed = kernel
            .claim_next_job_with_capabilities(
                "inline-worker",
                now,
                Some("ait-core"),
                &WorkerQueueClaimCapabilities {
                    excluded_runtime_contracts: vec!["ait.runner.native-job.v1".to_string()],
                    ..Default::default()
                },
            )
            .unwrap()
            .unwrap();

        assert_eq!(claimed["job_id"], inline["job_id"]);
        assert_eq!(
            kernel.get_job(runner["job_id"].as_i64().unwrap()).unwrap()["state"],
            json!("queued")
        );
    }

    #[test]
    fn heartbeat_and_terminal_delivery_require_the_claim_owner() {
        let pool = InMemoryWorkerQueuePool::new(Vec::new());
        let kernel = WorkerQueueKernel::new(pool, Default::default());
        let queued = kernel
            .enqueue_job(
                "ait-core",
                Some("REPO-CORE"),
                "content.gc",
                &json!({"repo_name": "ait-core"}),
                None,
                Some(3),
                false,
                "2026-07-29T08:00:00Z",
            )
            .unwrap();
        let job_id = queued["job_id"].as_i64().unwrap();
        kernel
            .claim_job(job_id, "runner-a", "2026-07-29T08:00:01Z", Some("ait-core"))
            .unwrap();

        assert!(kernel
            .heartbeat_job(job_id, "runner-b", "2026-07-29T08:00:02Z")
            .is_err());
        let heartbeat = kernel
            .heartbeat_job(job_id, "runner-a", "2026-07-29T08:00:03Z")
            .unwrap();
        assert_eq!(heartbeat["locked_at"], json!("2026-07-29T08:00:03Z"));
        assert!(kernel
            .complete_job_for_worker(
                job_id,
                &json!({"status": "ok"}),
                "2026-07-29T08:00:04Z",
                Some("runner-b"),
            )
            .is_err());
        let completed = kernel
            .complete_job_for_worker(
                job_id,
                &json!({"status": "ok"}),
                "2026-07-29T08:00:05Z",
                Some("runner-a"),
            )
            .unwrap();
        assert_eq!(completed["state"], json!("succeeded"));
    }
}
