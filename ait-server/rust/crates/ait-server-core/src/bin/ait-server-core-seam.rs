use ait_server_core::foundation::agent_protocol::{
    agent_server_protocol_schema_json, normalize_agent_server_job_json,
};
use ait_server_core::foundation::ci_command_bundle::ci_command_bundle_run_json;
use ait_server_core::foundation::identity::identity_json;
use ait_server_core::foundation::main_seed_prewarm::ci_main_seed_prewarm_json;
use ait_server_core::foundation::patchset_ci::{
    plan_patchset_ci_dispatch_from_manifest_values,
    workflow_ready_server_evidence_from_manifest_values, PatchsetCiDispatchJob, PatchsetCiJobPlan,
    PatchsetCiPlan,
};
use ait_server_core::foundation::patchset_ci_host::{
    patchset_ci_active_state_json, patchset_ci_completion_json,
    patchset_ci_contract_available_json, patchset_ci_status_summary_json,
    patchset_ci_suite_catalog_json,
};
use ait_server_core::foundation::patchset_ci_runtime::patchset_ci_run_json;
use ait_server_core::foundation::plan_revision::plan_revision_json;
use ait_server_core::foundation::policy_gate::policy_gate_json;
use ait_server_core::foundation::repo_ci_runtime::repo_ci_run_json;
use ait_server_core::foundation::scheduler::{
    admit_next, scheduler_job_spec_from_async_job, scheduler_queued_job_from_async_job_with_policy,
    scheduler_running_job_from_async_job_with_policy, SchedulerAdmissionDecision,
    SchedulerDeploymentPosture, SchedulerJobSpec, SchedulerPolicy, SchedulerQueuedJob,
    SchedulerRunningJob,
};
use ait_server_core::foundation::test_shard_runner::ci_test_shard_run_json;
use ait_server_core::foundation::test_shard_runtime::{
    ci_test_shard_cleanup_json, ci_test_shard_prepare_json,
};
use ait_server_core::foundation::test_shards::ci_test_shard_plan_json;
use ait_server_core::foundation::transport::{
    async_job_contract, land_request_json, normalize_async_job_payload,
    retry_delay_seconds_for_job, supported_async_job_types,
};
use ait_server_core::foundation::workflow_artifacts::workflow_artifacts_json;
use ait_server_core::foundation::workflow_async_runtime::workflow_async_runtime_json;
use ait_server_core::foundation::{pack_substrate, revision_trees};
use ait_server_core::middle::ci_status_read_model::{
    repository_ci_runs_read_model, RepositoryCiRunsInput,
};
use ait_server_core::middle::metrics_read_model::{
    operator_metrics_read_model, operator_readiness_read_model, runtime_metrics_read_model,
    OperatorMetricsInput, RuntimeMetricsInput,
};
use ait_server_core::middle::queue_read_model::{queue_summary_read_model, QueueReadModelInput};
use ait_server_core::middle::secondary_read_model::{
    authority_map_read_model, reviewer_inbox_read_model, AuthorityMapInput, ReviewerInboxInput,
};
use ait_server_core::middle::workflow_repository_read_model::{
    repository_detail_read_model, repository_index_read_model, repository_worker_status_read_model,
    task_workflow_detail_read_model, RepositoryDetailInput, RepositoryIndexInput,
    RepositoryWorkerStatusInput, TaskWorkflowDetailInput,
};
use serde_json::{json, Map as JsonMap, Value as JsonValue};
use std::io::{self, Read};
use std::process::ExitCode;
use std::{env, fs};

const SEAM_CONTRACT_VERSION: &str = "ait-server-core-seam-v1";
const SEAM_CAPABILITIES: &[&str] = &[
    "foundation.agent_protocol.normalize_agent_server_job",
    "foundation.agent_protocol.schema",
    "foundation.transport.async_job_contract",
    "foundation.transport.normalize_async_job_payload",
    "foundation.transport.retry_delay_seconds_for_job",
    "foundation.transport.land_request_payload",
    "foundation.transport.land_freshness_result",
    "foundation.transport.land_snapshot_alignment",
    "foundation.identity.repo_scoped_keys",
    "foundation.identity.row_normalization",
    "foundation.plan_revision.payload_shaping",
    "foundation.plan_revision.plan_link_metadata",
    "foundation.plan_revision.revision_view",
    "foundation.policy_gate.evaluation",
    "foundation.policy_gate.input_fingerprint",
    "foundation.policy_gate.waiver_shaping",
    "foundation.scheduler.shape_async_job",
    "foundation.scheduler.admit_async_jobs",
    "foundation.scheduler.status",
    "server.workflow_async.runtime",
    "server.workflow_async.queue_mode",
    "server.workflow_async.job_payloads",
    "server.workflow_async.patchset_ci_start_plan",
    "server.workflow_async.patchset_publish_policy_followup",
    "server.workflow_artifacts.shaping",
    "server.workflow_artifacts.review_summary",
    "server.patchset_ci.schedule_admission",
    "server.patchset_ci.workflow_ready_evidence",
    "server.patchset_ci.run",
    "server.patchset_ci.contract_available",
    "server.patchset_ci.suite_catalog",
    "server.patchset_ci.tracking_attestation",
    "server.patchset_ci.active_state",
    "server.patchset_ci.status_summary",
    "server.repo_ci.run",
    "server.ci_main_seed.prewarm",
    "server.ci_command_bundle.run",
    "server.ci_test_shard.plan",
    "server.ci_test_shard.prepare",
    "server.ci_test_shard.run",
    "server.ci_test_shard.cleanup",
    "middle.ci_status.repository_ci_runs",
    "middle.queue_read_model.summary",
    "middle.metrics_read_model.runtime_metrics",
    "middle.metrics_read_model.operator_metrics",
    "middle.metrics_read_model.operator_readiness",
    "middle.secondary_read_model.authority_map",
    "middle.secondary_read_model.reviewer_inbox",
    "middle.workflow_repository_read_model.task_detail",
    "middle.workflow_repository_read_model.repository_index",
    "middle.workflow_repository_read_model.repository_detail",
    "middle.workflow_repository_read_model.repository_worker_status",
    "server.storage.build_pack_members",
    "server.storage.write_pack_archive",
    "server.storage.read_pack_index",
    "server.storage.read_pack_entry",
    "server.storage.pack_has_entry",
    "server.storage.summarize_pack_archives",
    "server.storage.build_storage_validation_summary",
    "server.storage.build_tree_pack_members",
    "server.storage.write_tree_pack_archive",
    "server.storage.read_tree_pack_index",
    "server.storage.read_tree_pack_index_without_ordinals",
    "server.storage.read_tree_pack_tree",
    "server.storage.read_tree_pack_tree_by_ordinal",
    "server.storage.tree_pack_contains_blob_ids",
    "server.storage.summarize_tree_pack_archives",
    "server.storage.tree_pack_manifest_path",
    "server.storage.build_tree_records",
    "server.storage.build_snapshot_id",
];

#[path = "ait_server_core_seam/ci.rs"]
mod ci;
#[path = "ait_server_core_seam/dispatch.rs"]
mod dispatch;
#[path = "ait_server_core_seam/json_helpers.rs"]
mod json_helpers;
#[path = "ait_server_core_seam/read_models.rs"]
mod read_models;
#[path = "ait_server_core_seam/scheduler.rs"]
mod scheduler;
#[path = "ait_server_core_seam/stores.rs"]
mod stores;
#[path = "ait_server_core_seam/workflow.rs"]
mod workflow;

fn main() -> ExitCode {
    dispatch::run()
}
