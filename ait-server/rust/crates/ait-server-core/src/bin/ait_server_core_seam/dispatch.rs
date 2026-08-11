use super::ci::*;
use super::json_helpers::*;
use super::read_models::*;
use super::scheduler::*;
use super::stores::*;
use super::workflow::*;
use super::*;

const SEAM_COMMANDS: &[&str] = &[
    "handshake",
    "async-job-contract",
    "agent-server-protocol-schema",
    "normalize-agent-server-job",
    "normalize-async-job-payload",
    "retry-delay-seconds-for-job",
    "land-request",
    "identity",
    "plan-revision",
    "scheduler-shape-async-job",
    "scheduler-admit-async-jobs",
    "scheduler-status",
    "workflow-async-runtime",
    "workflow-artifacts",
    "policy-gate",
    "patchset-ci-schedule-admission",
    "patchset-ci-workflow-ready-evidence",
    "patchset-ci-run",
    "patchset-ci-host",
    "repo-ci-run",
    "ci-main-seed-prewarm",
    "ci-command-bundle-run",
    "ci-test-shard-plan",
    "ci-test-shard-prepare",
    "ci-test-shard-run",
    "ci-test-shard-cleanup",
    "repository-ci-runs-read-model",
    "queue-read-model-summary",
    "runtime-metrics-read-model",
    "operator-metrics-read-model",
    "operator-readiness-read-model",
    "authority-map-read-model",
    "reviewer-inbox-read-model",
    "workflow-task-detail-read-model",
    "repository-index-read-model",
    "repository-detail-read-model",
    "repository-worker-status-read-model",
    "server-storage",
];

#[cfg(feature = "legacy-postgres-runtime")]
const LEGACY_SEAM_COMMANDS: &[&str] = &[
    "server-context",
    "patchset-store",
    "policy-store",
    "review-store",
    "worker-queue-kernel",
    "worker-queue-service",
    "postgres-runtime-probe",
];

pub(super) fn run() -> ExitCode {
    let mut args = env::args().skip(1);
    let result = (|| match args.next().as_deref() {
        Some("handshake") => handshake(),
        Some("async-job-contract") => print_json(&async_job_contract()),
        Some("agent-server-protocol-schema") => print_json(&agent_server_protocol_schema_json()),
        Some("normalize-agent-server-job") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            normalize_agent_server_job_command(&payload_json)
        }
        Some("normalize-async-job-payload") => {
            let job_type = required_arg("job_type", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            normalize_async_job_command(&job_type, &payload_json)
        }
        Some("retry-delay-seconds-for-job") => {
            let job_type = required_arg("job_type", args.next())?;
            print_json(&json!({
                "job_type": job_type,
                "retry_delay_seconds": retry_delay_seconds_for_job(&job_type),
            }))
        }
        Some("land-request") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            land_request_command(&operation, &payload_json)
        }
        Some("identity") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            identity_command(&operation, &payload_json)
        }
        Some("plan-revision") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            plan_revision_command(&operation, &payload_json)
        }
        #[cfg(feature = "legacy-postgres-runtime")]
        Some("server-context") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            server_context_command(&operation, &payload_json)
        }
        Some("scheduler-shape-async-job") => {
            let job_type = required_arg("job_type", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            scheduler_shape_async_job_command(&job_type, &payload_json)
        }
        Some("scheduler-admit-async-jobs") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            scheduler_admit_async_jobs_command(&payload_json)
        }
        Some("scheduler-status") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            scheduler_status_command(&payload_json)
        }
        Some("workflow-async-runtime") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            workflow_async_runtime_command(&operation, &payload_json)
        }
        Some("workflow-artifacts") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            workflow_artifacts_command(&operation, &payload_json)
        }
        Some("policy-gate") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            policy_gate_command(&operation, &payload_json)
        }
        #[cfg(feature = "legacy-postgres-runtime")]
        Some("policy-store") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            policy_store_command(&operation, &payload_json)
        }
        #[cfg(feature = "legacy-postgres-runtime")]
        Some("review-store") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            review_store_command(&operation, &payload_json)
        }
        #[cfg(feature = "legacy-postgres-runtime")]
        Some("patchset-store") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            patchset_store_command(&operation, &payload_json)
        }
        #[cfg(feature = "legacy-postgres-runtime")]
        Some("worker-queue-kernel") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            worker_queue_kernel_command(&payload_json)
        }
        #[cfg(feature = "legacy-postgres-runtime")]
        Some("worker-queue-service") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            worker_queue_service_command(&payload_json)
        }
        #[cfg(feature = "legacy-postgres-runtime")]
        Some("postgres-runtime-probe") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            postgres_runtime_probe_command(&payload_json)
        }
        Some("patchset-ci-schedule-admission") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            patchset_ci_schedule_admission_command(&payload_json)
        }
        Some("patchset-ci-workflow-ready-evidence") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            patchset_ci_workflow_ready_evidence_command(&payload_json)
        }
        Some("patchset-ci-run") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            patchset_ci_run_command(&payload_json)
        }
        Some("patchset-ci-host") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            patchset_ci_host_command(&operation, &payload_json)
        }
        Some("repo-ci-run") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            repo_ci_run_command(&payload_json)
        }
        Some("ci-main-seed-prewarm") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            ci_main_seed_prewarm_command(&payload_json)
        }
        Some("ci-command-bundle-run") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            ci_command_bundle_run_command(&payload_json)
        }
        Some("ci-test-shard-plan") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            ci_test_shard_plan_command(&payload_json)
        }
        Some("ci-test-shard-prepare") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            ci_test_shard_prepare_command(&payload_json)
        }
        Some("ci-test-shard-run") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            ci_test_shard_run_command(&payload_json)
        }
        Some("ci-test-shard-cleanup") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            ci_test_shard_cleanup_command(&payload_json)
        }
        Some("repository-ci-runs-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            repository_ci_runs_read_model_command(&payload_json)
        }
        Some("queue-read-model-summary") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            queue_read_model_summary_command(&payload_json)
        }
        Some("runtime-metrics-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            runtime_metrics_read_model_command(&payload_json)
        }
        Some("operator-metrics-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            operator_metrics_read_model_command(&payload_json)
        }
        Some("operator-readiness-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            operator_readiness_read_model_command(&payload_json)
        }
        Some("authority-map-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            authority_map_read_model_command(&payload_json)
        }
        Some("reviewer-inbox-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            reviewer_inbox_read_model_command(&payload_json)
        }
        Some("workflow-task-detail-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            workflow_task_detail_read_model_command(&payload_json)
        }
        Some("repository-index-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            repository_index_read_model_command(&payload_json)
        }
        Some("repository-detail-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            repository_detail_read_model_command(&payload_json)
        }
        Some("repository-worker-status-read-model") => {
            let payload_json = payload_arg("payload_json", args.next())?;
            repository_worker_status_read_model_command(&payload_json)
        }
        Some("server-storage") => {
            let operation = required_arg("operation", args.next())?;
            let payload_json = payload_arg("payload_json", args.next())?;
            server_storage_command(&operation, &payload_json)
        }
        Some(command) => Err(format!(
            "Unsupported ait-server-core seam command: {command}. Expected one of: {}.",
            supported_commands()
        )),
        None => Err(format!(
            "Expected an ait-server-core seam command: {}.",
            supported_commands()
        )),
    })();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(1)
        }
    }
}

fn handshake() -> Result<(), String> {
    print_json(&json!({
        "ready": true,
        "contract_version": SEAM_CONTRACT_VERSION,
        "package_version": env!("CARGO_PKG_VERSION"),
        "capabilities": seam_capabilities(),
        "supported_async_job_types": supported_async_job_types(),
    }))
}

#[cfg(not(feature = "legacy-postgres-runtime"))]
fn seam_capabilities() -> Vec<&'static str> {
    SEAM_CAPABILITIES.to_vec()
}

#[cfg(feature = "legacy-postgres-runtime")]
fn seam_capabilities() -> Vec<&'static str> {
    let mut capabilities = SEAM_CAPABILITIES.to_vec();
    capabilities.extend_from_slice(LEGACY_SEAM_CAPABILITIES);
    capabilities
}

#[cfg(not(feature = "legacy-postgres-runtime"))]
fn supported_commands() -> String {
    SEAM_COMMANDS.join(", ")
}

#[cfg(feature = "legacy-postgres-runtime")]
fn supported_commands() -> String {
    let mut commands = SEAM_COMMANDS.to_vec();
    commands.extend_from_slice(LEGACY_SEAM_COMMANDS);
    commands.join(", ")
}
