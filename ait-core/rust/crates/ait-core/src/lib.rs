#![recursion_limit = "256"]

#[cfg(test)]
#[path = "../../../test_support.rs"]
mod workspace_test_support;

#[cfg(feature = "perfetto-tracing")]
#[macro_export]
macro_rules! perfetto_range {
    ($name:literal) => {
        $crate::perfetto_trace::PerfettoRange::new($name)
    };
}

#[cfg(not(feature = "perfetto-tracing"))]
#[macro_export]
macro_rules! perfetto_range {
    ($name:literal) => {
        ()
    };
}

#[cfg(feature = "perfetto-tracing")]
#[doc(hidden)]
pub mod perfetto_trace;

// Shared foundation traits and concrete kernels proven through the `plan`
// wave. These are cross-domain only where the seam is already honest.
pub mod agent_local_workflow_backend;
pub mod attest_json;
pub mod benchmark_usage;
pub mod binary_db;
pub mod binary_db_generation;
pub mod change_json;
pub mod change_store;
pub mod config_runtime;
pub mod content_binary_db;
pub mod content_store;
pub mod current_source_cache;
pub mod diagnostics;
pub mod external;
pub mod file_io;
pub mod json_support;
pub mod land_json;
pub mod line_binary_db;
pub mod line_store;
pub mod local_content_gc;
pub mod local_snapshot;
pub mod object_diff;
pub mod object_diff_ports;
pub mod pack_substrate;
pub mod patchset_json;
pub mod plan_filesystem;
pub mod plan_foundation;
pub mod plan_http_client;
pub mod plan_http_contracts;
pub mod plan_pack_substrate;
// Keep protocol normalizers behind the stable crate-level module path.
pub mod plan_ports_protocols;
pub mod plan_provenance;
pub mod plan_store;
pub mod plan_workflow_json;
pub mod policy;
pub mod policy_json;
pub mod ref_names;
pub mod remote_store;
pub mod remote_sync_backend;
pub mod remote_sync_local_store;
pub mod repo_state_json;
pub mod repo_status_store;
pub mod repository_pack_json;
pub mod repository_pack_policy;
pub mod runtime_binding_state;
pub mod runtime_roots;
pub mod server_operational;
pub mod server_repo_retire;
pub mod shared_foundation;
pub mod snapshot_dag;
pub mod snapshot_json;
pub mod snapshot_merge;
pub mod snapshot_store;
pub mod stash_binary_db;
pub mod stash_store;
pub mod tag_store;
pub mod task_close;
pub mod task_json;
pub mod task_lifecycle;
pub mod task_remote;
pub mod task_store;
pub mod task_workflow_http_adapter;
pub mod task_workflow_ports_protocols;
pub mod task_workflow_remote_traits;
pub mod task_workflow_shared_foundation;
mod task_workflow_shared_foundation_facts;
mod task_workflow_shared_foundation_local;
pub mod task_workflow_store;
pub mod task_workflow_store_traits;
pub mod text_normalization;
pub mod time_identity;
pub mod toml_support;
pub mod transport_envelope;
pub mod worker_manifest;
pub mod workspace_hash_cache;
// Durable local task/change authority used by solo-local workflows.
pub mod workflow_binary_db;
mod workflow_closeout_command_hints;
mod workflow_closeout_decision;
pub mod workflow_closeout_facts;
mod workflow_closeout_model_support;
mod workflow_closeout_projection;
pub mod workflow_closeout_read_model;
pub mod workflow_closeout_remote;
pub mod workflow_closeout_views;
pub mod workflow_event_store;
pub mod workflow_primitives;
pub mod workflow_release_store;
pub mod workflow_tier;

// Compatibility wrapper modules that preserve the existing `plan_*` symbol
// surface while shared-foundation ownership moves to the generic modules above.
pub mod plan_config_runtime;
pub mod plan_diagnostics;
pub mod plan_time_identity;

// `plan` domain/application/orchestration modules remain concrete until a
// second domain proves the same abstraction honestly exists.
pub mod plan_application;
mod plan_artifact_matching;
pub mod plan_binary_db;
pub mod plan_blob_diff;
pub mod plan_command;
pub mod plan_command_execution;
pub mod plan_dispatch;
pub mod plan_items;
pub mod plan_sync_execution;
