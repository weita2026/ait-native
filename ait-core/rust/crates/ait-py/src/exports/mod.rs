use ait_agent_core::{
    agent_management_binding_json,
    event_loop::{
        agent_telegram_message_delivery_execute_json, agent_telegram_turn_input_plan_json,
        agent_telegram_workflow_notification_format_json, agent_telegram_workflow_query_plan_json,
    },
    language_binding_info_json,
    transport_config::agent_env_file_load_json,
    web_runtime::agent_web_runtime_execute_json,
};
use ait_agent_worker::{
    agent_worker_capabilities_binding_json, agent_worker_transaction_binding_json,
};
use ait_cli::auth_surface::{
    auth_bindings as rust_task_workflow_auth_bindings, auth_grant as rust_task_workflow_auth_grant,
    auth_whoami as rust_task_workflow_auth_whoami, AuthGrantRequest, AuthRemoteRequest,
};
use ait_cli::blame_surface::{blame as rust_task_workflow_blame, BlameRequest};
use ait_cli::config_surface::{
    config_set_from_payload as rust_task_workflow_config_set,
    config_show as rust_task_workflow_config_show,
};
use ait_cli::doctor_surface::{
    doctor_plan_authority as rust_task_workflow_doctor_plan_authority,
    doctor_plan_authority_wheel as rust_task_workflow_doctor_plan_authority_wheel,
    doctor_postgres as rust_task_workflow_doctor_postgres,
    doctor_runtime_root as rust_task_workflow_doctor_runtime_root,
    postgres_schema_checks as rust_task_workflow_postgres_schema_checks,
};
use ait_cli::init_surface::{init_repo as rust_task_workflow_init, InitRequest};
use ait_cli::install_surface::install_from_payload as rust_task_workflow_install;
use ait_cli::primitives as task_workflow_primitives;
use ait_cli::release_surface as task_workflow_release_surface;
use ait_cli::remote_surface::{
    remote_add_from_payload as rust_task_workflow_remote_add,
    remote_list as rust_task_workflow_remote_list,
};
use ait_cli::repo_surface::repo_command_from_payload as rust_task_workflow_repo_command;
use ait_cli::runtime::RepoRuntime as TaskWorkflowRepoRuntime;
use ait_cli::test_surface::{
    test_run_full as rust_task_workflow_test_run_full, TestRunFullRequest,
};
use ait_cli::workspace_lock::{
    workspace_command_lock_path as rust_workspace_command_lock_path,
    WorkspaceCommandLock as RustWorkspaceCommandLock,
};
use ait_core::benchmark_usage::{
    extract_codex_usage_bundle_jsonl as rust_extract_codex_usage_bundle_jsonl,
    extract_codex_usage_jsonl as rust_extract_codex_usage_jsonl,
};
use ait_core::config_runtime::{
    build_plan_runtime_selection_facts_json, normalize_plan_runtime_compatibility_payload_json,
    normalize_plan_runtime_doctor_payload_json, normalize_plan_runtime_readiness_payload_json,
    normalize_plan_runtime_selection_facts_payload_json,
    normalize_plan_runtime_selection_request_payload_json,
};
use ait_core::current_source_cache::{
    current_core_source_fingerprint as rust_current_core_source_fingerprint,
    current_server_source_fingerprint as rust_current_server_source_fingerprint,
    current_source_native_cache_contract_json, CurrentSourceNativeCacheRequest,
};
use ait_core::diagnostics::{
    build_plan_backend_identity_facts_json, build_plan_diagnostics_compatibility_status_json,
    build_plan_diagnostics_doctor_facts_json, build_plan_diagnostics_readiness_status_json,
    build_plan_storage_readiness_facts_json, build_plan_wheel_status_facts_json,
    normalize_plan_backend_identity_payload_json,
    normalize_plan_diagnostics_compatibility_payload_json,
    normalize_plan_diagnostics_doctor_payload_json,
    normalize_plan_diagnostics_readiness_payload_json,
    normalize_plan_diagnostics_request_payload_json, normalize_plan_wheel_status_payload_json,
};
use ait_core::json_support::{json, JsonMap as Map, JsonNumber as Number, JsonValue};
use ait_core::object_diff::{
    artifact_blob_id, diff_snapshot_manifests, snapshot_diff_from_manifests,
    DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
};
use ait_core::pack_substrate as plan_pack_substrate;
use ait_core::plan_application::{
    build_plan_candidates_service_payload_json, build_plan_inspect_service_payload_json,
    build_plan_items_service_payload_json, build_plan_list_service_payload_json,
    build_plan_revisions_service_payload_json, build_plan_show_service_payload_json,
    build_plan_sync_service_payload_json, normalize_plan_candidates_service_request_payload_json,
    normalize_plan_inspect_service_request_payload_json,
    normalize_plan_items_service_request_payload_json,
    normalize_plan_list_service_request_payload_json,
    normalize_plan_revisions_service_request_payload_json,
    normalize_plan_show_service_request_payload_json,
    normalize_plan_sync_service_request_payload_json,
};
use ait_core::plan_blob_diff::{
    artifact_candidates_open, index_plans_by_artifact_identity, index_plans_by_artifact_path,
    local_plan_fully_published, open_generic_plans_matching_blob_id, open_plans_matching_selector,
    plan_artifact_identity, plan_artifact_identity_label, plan_heads_equivalent,
    plan_matches_sync_artifact,
};
use ait_core::plan_command::{
    build_plan_candidates_command_payload_json, build_plan_inspect_command_payload_json,
    build_plan_items_command_payload_json, build_plan_list_command_payload_json,
    build_plan_revisions_command_payload_json, build_plan_show_command_payload_json,
    build_plan_sync_command_payload_json, normalize_plan_candidates_command_request_payload_json,
    normalize_plan_inspect_command_request_payload_json,
    normalize_plan_items_command_request_payload_json,
    normalize_plan_list_command_request_payload_json,
    normalize_plan_revisions_command_request_payload_json,
    normalize_plan_show_command_request_payload_json,
    normalize_plan_sync_command_request_payload_json,
};
use ait_core::plan_command_execution::{
    execute_plan_candidates_command_request_json, execute_plan_inspect_command_request_json,
    execute_plan_items_command_request_json, execute_plan_list_command_request_json,
    execute_plan_revisions_command_request_json, execute_plan_show_command_request_json,
};
use ait_core::plan_dispatch::{
    compute_taskable_items, local_plan_publish_shadow, plan_candidates_payload,
    plan_dispatch_summary, plan_items_payload, plan_task_link_indexes, validate_dispatch_legality,
    DispatchLegalityDecision, DispatchPlanInput, DispatchPlanItemInput, DispatchRevisionInput,
    DispatchSummaryItem, DispatchTaskInput, LinkedTaskSummary, LocalPlanPublishShadow,
    PlanCandidatesAggregateSummary, PlanCandidatesPayload, PlanDispatchSummary, PlanItemsPayload,
    PlanTaskLinkIndexes,
};
use ait_core::plan_filesystem::{
    is_lineage_only_markdown_artifact_path, is_markdown_artifact_path,
    list_visible_markdown_artifact_paths, list_visible_workspace_paths,
    normalize_markdown_artifact_path, path_is_projected_out_for_workspace, read_binary_file,
    read_json_file, read_utf8_text_file, read_zip_archive_member, resolve_repo_artifact_path,
    workspace_path_is_ignored, zip_archive_has_member, PlanFilesystemError,
};
use ait_core::plan_foundation::{
    compute_sync_prune_decisions, extract_plan_refs, parse_plan_markdown, ParsedPlan,
    PlanRefIdentityPayload, SyncPruneDecisionPayload,
};
use ait_core::plan_http_client::{
    append_planning_session_event as append_http_planning_session_event,
    close_planning_session as close_http_planning_session, create_plan as create_http_plan,
    create_planning_session as create_http_planning_session, get_plan as get_http_plan,
    get_plan_revision as get_http_plan_revision, get_planning_session as get_http_planning_session,
    join_planning_session as join_http_planning_session,
    list_plan_ids_matching_contains as list_http_plan_ids_matching_contains,
    list_plan_revisions as list_http_plan_revisions,
    list_planning_session_events as list_http_planning_session_events,
    list_planning_sessions as list_http_planning_sessions, list_plans as list_http_plans,
    promote_planning_session as promote_http_planning_session,
    put_plan_revision_artifacts as put_http_plan_revision_artifacts,
    resolve_task_plan_linkage as resolve_http_task_plan_linkage, revise_plan as revise_http_plan,
    update_plan_status as update_http_plan_status, PlanHttpClientConfig, PlanHttpClientError,
    PlanHttpClientManager, PlanHttpClientStats,
};
use ait_core::plan_http_contracts::validate_planning_session_join_payload_json;
use ait_core::plan_items::{
    extract_plan_items, extract_plan_section, find_plan_item, find_plan_item_in_items,
    list_plan_section_refs, normalize_plan_items, NormalizedPlanItemSeed, PlanItem, PlanSection,
};
use ait_core::plan_ports_protocols::{
    normalize_artifact_publish_request_payload_json,
    normalize_artifact_resolver_request_payload_json, normalize_linked_task_lookup_payload_json,
    normalize_plan_config_runtime_facts_payload_json,
    normalize_plan_connection_manager_stats_payload_json,
    normalize_plan_remote_request_payload_json, normalize_plan_remote_transport_payload_json,
    normalize_plan_store_read_request_payload_json,
};
use ait_core::plan_provenance::{
    build_plan_revision_provenance_payload_json, normalize_plan_revision_provenance_payload_json,
};
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
use ait_core::policy as rust_policy;
use ait_core::ref_names as rust_ref_names;
use ait_core::repo_state_json::{
    read_repo_config_json_file as rust_read_repo_config_json_file,
    read_worktree_config_json_file as rust_read_worktree_config_json_file,
    read_worktree_metadata_json_file as rust_read_worktree_metadata_json_file,
    write_repo_config_json_file as rust_write_repo_config_json_file,
    write_worktree_config_json_file as rust_write_worktree_config_json_file,
    write_worktree_metadata_json_file as rust_write_worktree_metadata_json_file,
};
use ait_core::repository_pack_json::{
    ZstdBulkCommitRequestJson, ZstdBulkCommitResponseJson, ZstdBulkPlanRequestJson,
    ZstdBulkPlanResponseJson, ZstdImportManifestJson, ZstdPackUploadResponseJson,
};
use ait_core::runtime_binding_state::{
    default_runtime_binding_state_payload_json, normalize_runtime_binding_state_document_json,
    runtime_binding_state_ir_version, runtime_binding_state_schema_json,
};
use ait_core::runtime_roots as rust_runtime_roots;
use ait_core::server_repo_retire::project_repo_retire_runtime_blockers as rust_project_repo_retire_runtime_blockers;
use ait_core::task_close::{resolve_task_close, TaskCloseRequest, TaskCloseScope};
use ait_core::task_lifecycle::build_task_audit_verdict_payload as build_rust_task_audit_verdict_payload;
use ait_core::task_remote::task_remote_change_lineage_payload as rust_task_remote_change_lineage_payload;
use ait_core::task_workflow_http_adapter::{
    normalize_task_workflow_http_compatibility_payload_json,
    normalize_task_workflow_http_readiness_payload_json, HttpTaskRemote,
    HttpWorkflowCloseoutRemote, TaskWorkflowHttpClientManager,
};
use ait_core::task_workflow_ports_protocols::{
    build_linked_change_lookup_payload as build_rust_linked_change_lookup_payload,
    build_linked_task_lookup_payload as build_rust_linked_task_lookup_payload,
    build_task_tracking_metadata_payload as build_rust_task_tracking_metadata_payload,
    build_task_tracking_title_payload as build_rust_task_tracking_title_payload,
};
use ait_core::task_workflow_shared_foundation::{
    task_workflow_read_binary_file as rust_task_workflow_read_binary_file,
    task_workflow_read_json_file as rust_task_workflow_read_json_file,
    task_workflow_read_utf8_text_file as rust_task_workflow_read_utf8_text_file,
    task_workflow_resolve_repo_artifact_path as rust_task_workflow_resolve_repo_artifact_path,
    task_workflow_runtime_selection_facts as build_rust_task_workflow_runtime_selection_facts,
    task_workflow_sequence_identity_facts as build_rust_task_workflow_sequence_identity_facts,
    task_workflow_timestamp_facts as build_rust_task_workflow_timestamp_facts,
    task_workflow_workflow_id_facts as build_rust_task_workflow_workflow_id_facts,
};
use ait_core::time_identity::{
    build_plan_sequence_identity_payload_json, build_plan_temporal_ordering_payload_json,
    build_plan_timestamp_payload_json, build_plan_workflow_id_payload_json,
    normalize_plan_sequence_identity_payload_json,
    normalize_plan_sequence_identity_request_payload_json,
    normalize_plan_temporal_ordering_payload_json, normalize_plan_timestamp_payload_json,
    normalize_plan_timestamp_request_payload_json, normalize_plan_workflow_id_payload_json,
    normalize_plan_workflow_id_request_payload_json,
};
use ait_core::transport_envelope::{
    build_transport_binding_metadata_json, build_transport_event_envelope_json,
    build_transport_reply_envelope_json, compact_transport_event_envelope_json,
    compact_transport_reply_envelope_json, transport_envelope_ir_version,
    transport_envelope_schema_json,
};
use ait_core::worker_manifest::{
    default_worker_manifest_config_json, normalize_worker_manifest_document_json,
    select_telegram_worker_json, worker_manifest_ir_version, worker_manifest_schema_json,
};
use ait_core::workflow_closeout_facts::{
    workflow_land_full_facts as build_rust_workflow_land_full_facts,
    workflow_land_phase_facts as build_rust_workflow_land_phase_facts,
    workflow_landed_facts as build_rust_workflow_landed_facts,
    workflow_ready_facts as build_rust_workflow_ready_facts,
};
use ait_core::workflow_closeout_read_model::{
    project_workflow_land_full_read_model as project_rust_workflow_land_full_read_model,
    project_workflow_land_phase_read_model as project_rust_workflow_land_phase_read_model,
    project_workflow_landed_read_model as project_rust_workflow_landed_read_model,
    project_workflow_ready_read_model as project_rust_workflow_ready_read_model,
};
use ait_core::workflow_closeout_remote::{
    workflow_remote_action_mutation_receipts as rust_workflow_remote_action_mutation_receipts,
    workflow_remote_mutation_receipt as rust_workflow_remote_mutation_receipt,
};
use ait_core::workflow_closeout_views::{
    workflow_applied_action_summary as rust_workflow_applied_action_summary,
    workflow_apply_phase_payload as rust_workflow_apply_phase_payload,
    workflow_apply_phase_summary as rust_workflow_apply_phase_summary,
    workflow_mutation_receipt_summary as rust_workflow_mutation_receipt_summary,
};
use ait_core::workflow_primitives::{
    derive_patchset_id, generate_namespaced_sequence_id, generate_workflow_id,
    normalize_id_namespace_prefix, publication_state_has_unpublished_head, publication_state_value,
    task_status_details, task_status_value, workflow_error_envelope, workflow_id_matches,
    workflow_id_matches_any_namespace_prefix, workflow_id_namespace_prefix_candidates,
    workflow_id_namespace_prefix_for_value, workflow_id_token, workflow_id_tokens,
    workflow_mode_value, workflow_origin_namespace_prefix, workflow_success_envelope,
    WorkflowResultEnvelope, WorkflowStatusDetails, DEFAULT_ID_NAMESPACE_PREFIX,
    LOCAL_WORKFLOW_ID_NAMESPACE_PREFIX, REMOTE_WORKFLOW_ID_NAMESPACE_PREFIX, WORKFLOW_ID_FAMILIES,
    WORKFLOW_TASK_CHANGE_ORIGIN_NAMESPACE_PREFIXES,
};
use pyo3::exceptions::{PyFileNotFoundError, PyKeyError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyAny, PyBool, PyBytes, PyDict, PyList, PyTuple};
use pyo3::wrap_pyfunction;
use sha2::{Digest, Sha256};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

mod task_workflow_shell_support;

use self::task_workflow_shell_support::{
    effective_task_review as rust_effective_task_review,
    task_review_enabled as rust_task_review_enabled,
    workflow_closeout_wait_hint as rust_workflow_closeout_wait_hint,
    workflow_land_command_hints as rust_workflow_land_command_hints,
    workflow_ready_command_hints as rust_workflow_ready_command_hints,
};

include!("remote_clients.rs");
include!("agent_runtime.rs");
include!("plan_http.rs");
include!("task_workflow.rs");
include!("plan_store.rs");
include!("workflow_policy.rs");
include!("plan_filesystem_storage.rs");
include!("render_parse.rs");
include!("registration.rs");

#[cfg(test)]
mod callback_boundary_tests {
    const EXPORT_SOURCES: &[(&str, &str)] = &[
        ("agent_runtime.rs", include_str!("agent_runtime.rs")),
        (
            "plan_filesystem_storage.rs",
            include_str!("plan_filesystem_storage.rs"),
        ),
        ("plan_http.rs", include_str!("plan_http.rs")),
        ("plan_store.rs", include_str!("plan_store.rs")),
        ("registration.rs", include_str!("registration.rs")),
        ("remote_clients.rs", include_str!("remote_clients.rs")),
        ("render_parse.rs", include_str!("render_parse.rs")),
        ("task_workflow.rs", include_str!("task_workflow.rs")),
        (
            "task_workflow_shell_support.rs",
            include_str!("task_workflow_shell_support.rs"),
        ),
        ("workflow_policy.rs", include_str!("workflow_policy.rs")),
    ];

    #[test]
    fn pyo3_exports_have_no_callable_invocation_surface() {
        for (path, source) in EXPORT_SOURCES {
            for forbidden in [".call(", ".call0(", ".call1(", ".call_method"] {
                assert!(
                    !source.contains(forbidden),
                    "{path} contains executable PyO3 boundary `{forbidden}`"
                );
            }
        }
    }

    #[test]
    fn retired_callback_abi_cannot_return() {
        for (path, source) in EXPORT_SOURCES {
            for forbidden in [
                "DeferredReplyClientEpollRuntime",
                "DeferredReplyClientEpollWatchScheduler",
                "PyDeferredReplyRuntimeCallbacks",
                "PyTelegramDispatchFutureForgetCallback",
                "ait_agent_telegram_dispatch_runtime_execute",
                "ait_agent_telegram_submission_runtime_execute",
                "ait_agent_telegram_logical_turn_runtime_execute",
                "ait_agent_telegram_submission_callback_slot",
                "resolve_base_blob",
                "progress_callback",
                "task_start_with_progress",
                "PySnapshotReader",
                "PyBlobReader",
                "PyObjectReader",
                "snapshot_manifest_from_object_reader",
                "snapshot_diff_from_reader_ports",
                "snapshot_diff_from_object_reader_ports",
            ] {
                assert!(
                    !source.contains(forbidden),
                    "{path} contains retired callback ABI `{forbidden}`"
                );
            }
        }
    }

    #[test]
    fn native_stash_mutations_do_not_return_to_the_pyo3_abi() {
        let source = include_str!("task_workflow.rs");
        for retired in [
            "task_workflow_stash_save",
            "task_workflow_stash_apply",
            "task_workflow_stash_pop",
        ] {
            assert!(
                !source.contains(retired),
                "task_workflow.rs contains retired stash mutation export `{retired}`"
            );
        }
        for retained in [
            "task_workflow_stash_list",
            "task_workflow_stash_show",
            "task_workflow_stash_drop",
        ] {
            assert!(
                source.contains(retained),
                "task_workflow.rs lost required direct Rust read/drop export `{retained}`"
            );
        }
    }
}

#[cfg(test)]
mod telegram_message_delivery_export_tests {
    use super::{json, validate_telegram_message_delivery_export};

    #[test]
    fn telegram_message_delivery_export_rejects_exposure_flags_and_wrong_identity() {
        let delivery = json!({
            "contract": "ait_agent_core.event_loop.TelegramMessageDeliveryExecution.v1",
            "migration_stage": "rust_agent_telegram_message_delivery_execution",
            "stage": "execute",
            "python_message_delivery_allowed": false,
            "python_message_formatting_allowed": false,
            "raw_api_result_exposed": false,
            "telegram_description_exposed": false,
            "token_bearing_url_exposed": false,
            "chat_id_exposed": false,
            "formatted_text_exposed": false,
            "plain_text_exposed": false,
        });
        assert!(validate_telegram_message_delivery_export(&delivery).is_ok());

        let mut wrong_identity = delivery.clone();
        wrong_identity["migration_stage"] = json!("python-delivery-secret");
        assert!(validate_telegram_message_delivery_export(&wrong_identity).is_err());
        let mut leaked_text = delivery;
        leaked_text["plain_text_exposed"] = json!(true);
        assert!(validate_telegram_message_delivery_export(&leaked_text).is_err());
    }
}
