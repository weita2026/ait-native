use ait_cli::perfetto_range;
use ait_cli::auth_surface::{
    auth_bindings as auth_bindings_cmd, auth_grant as auth_grant_cmd,
    auth_whoami as auth_whoami_cmd, AuthGrantRequest, AuthRemoteRequest,
};
use ait_cli::blame_surface::{blame as blame_cmd, render_human_blame, BlameRequest};
use ait_cli::snapshot_restore_surface::{
    snapshot_restore_lines, SnapshotRestoreLinesRequest,
};
use ait_cli::config_surface::{
    config_set as config_set_cmd, config_show as config_show_cmd,
    config_unset as config_unset_cmd, ConfigSetRequest, ConfigUnsetKey,
};
use ait_cli::doctor_surface::{
    doctor_memory_root, doctor_plan_authority, doctor_plan_authority_for_repository,
    doctor_runtime_root, render_doctor_text,
};
use ait_cli::external_surface::{
    external_doctor as external_doctor_cmd, external_link as external_link_cmd,
    external_status as external_status_cmd, external_unlink as external_unlink_cmd,
    external_update as external_update_cmd, render_external_text,
};
use ait_cli::init_surface::{init_repo as init_cmd, render_human_init, InitRequest};
use ait_cli::primitives::{
    attest_put, attest_show as attest_show_cmd, change_close as change_close_cmd,
    change_create as change_create_cmd, change_list as change_list_cmd,
    change_publish as change_publish_cmd, change_replay as change_replay_cmd,
    change_revert as change_revert_cmd, change_show as change_show_cmd, git_export as git_export_cmd,
    git_import as git_import_cmd, git_mirror as git_mirror_cmd,
    line_archive, line_cleanup, line_create, line_delete, line_list, line_merge, line_rename,
    line_show, line_switch, patchset_ci_status as patchset_ci_status_cmd,
    patchset_list as patchset_list_cmd,
    patchset_publish, patchset_rerun_ci as patchset_rerun_ci_cmd,
    patchset_select as patchset_select_cmd, patchset_show as patchset_show_cmd, policy_eval,
    policy_show, policy_waive, pull as pull_cmd, push as push_cmd,
    queue_summary as queue_summary_cmd, repo_status as repo_status_cmd, review_code_submit,
    review_code_template, review_record, review_request, review_show, review_task_approve,
    review_task_record, review_team_approve, snapshot_ancestry, snapshot_create, snapshot_diff,
    snapshot_is_ancestor_query, snapshot_list, snapshot_merge_base_query, snapshot_replay,
    snapshot_revert, snapshot_show, stash_apply, stash_drop, stash_list, stash_pop, stash_save,
    stash_show, task_abandon, task_audit, task_land_apply_scoped, task_list, task_show,
    task_start_from_with_progress, task_start_with_progress,
    resolve_task_scoped_execution_repo, run_task_scoped_workspace_command,
    workflow_reconcile_apply, workflow_reconcile_automatic,
    workflow_reconcile_automatic_best_effort, workflow_reconcile_inventory,
    workflow_land_apply, workflow_land_payload,
    workflow_ready_apply, workflow_ready_payload,
    workspace_dirty_diff,
    worktree_abort_rebase, worktree_cleanup,
    worktree_cleanup_candidates, worktree_continue_rebase, worktree_doctor, worktree_get,
    worktree_list, worktree_preview_rebase, worktree_prune_stale, worktree_rebase,
    worktree_recover_task, worktree_recreate, worktree_remove, worktree_restore,
    worktree_restore_owned_head,
    worktree_status, worktree_sync, worktree_sync_all, worktree_touch_usage,
    AutomaticReconciliationScope, AutomaticReconciliationTrigger, SnapshotAncestryDirection,
    DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_LIMIT, DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_MAX_DEPTH,
};
use ait_cli::release_surface::{
    family_candidate_exists, family_manifest_exists, family_release_build,
    family_release_candidate_create, family_release_candidate_create_from_public_source,
    family_release_check, family_release_package, family_release_promote,
    family_release_publish_error, family_release_show, release_adapter_build_for_target,
    release_adapter_check_for_target,
    release_build_with_native_inputs as release_build_cmd, release_native_source,
    release_candidate_create as release_candidate_create_cmd, release_check as release_check_cmd,
    release_formula_with_python as release_formula_cmd,
    release_native_bundle as release_native_bundle_cmd,
    release_publish as release_publish_cmd, release_show as release_show_cmd, render_release_text,
    NativeSourceRequest, FAMILY_RELEASE_PROFILE,
};
use ait_cli::remote_surface::{
    remote_add as remote_add_cmd, remote_list as remote_list_cmd, RemoteAddRequest,
};
use ait_cli::remote_head_recovery::{
    recover_remote_head, RemoteHeadRecoveryContext, RemoteHeadRecoveryRequest,
};
use ait_cli::render::{print_json, print_key_values, print_list};
use ait_cli::repo_surface::{
    render_repo_command_text, repo_command as repo_command_cmd, RepoCommandRequest,
};
use ait_cli::runtime::{RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use ait_core::local_content_gc::{
    LocalContentOrphanPackPruneStore, LocalContentStatsStore, LocalContentValidationStore,
};
use ait_cli::tag_surface::{
    resolve_snapshot_ref as resolve_snapshot_ref_cmd, tag_create as tag_create_cmd,
    tag_delete as tag_delete_cmd, tag_list as tag_list_cmd, tag_show as tag_show_cmd,
    TagCreateRequest,
};
use ait_cli::task_land_contract::{
    task_land_exit_code, task_land_scope_contract_json, PLAN_SYNC_COMMAND_ABOUT,
    TASK_FINISH_COMMAND_ABOUT, TASK_LAND_CONTRACT_VERSION,
};
use ait_cli::workspace_lock::run_locked_workspace_command;
use ait_core::binary_db_generation::{
    activate_binary_db_generation, stage_binary_db_u64_second_upgrade,
    BinaryDbGenerationActivationOptions, StageBinaryDbU64SecondUpgradeOptions,
};
use ait_core::current_source_cache::{
    current_core_source_fingerprint as current_core_source_fingerprint_cmd,
    current_core_source_mtime_ns as current_core_source_mtime_ns_cmd,
    current_server_source_fingerprint as current_server_source_fingerprint_cmd,
    current_server_source_mtime_ns as current_server_source_mtime_ns_cmd,
    current_source_binary_is_fresh_json, current_source_extension_is_fresh_json,
    current_source_native_cache_contract_json as current_source_native_cache_contract_cmd,
    current_source_native_cache_paths, prune_current_source_native_caches_json,
    register_current_source_native_cache_lease_for_owner_json,
    register_current_source_native_cache_lease_json,
    release_current_source_native_cache_lease_json,
    seed_current_source_native_cache_from_canonical_json,
    validate_current_source_cli_bootstrap, write_current_source_native_cache_manifest_json,
    CurrentSourceBinaryFreshnessRequest, CurrentSourceCliBootstrapRequest,
    CurrentSourceExtensionFreshnessRequest, CurrentSourceNativeCacheCanonicalSeedRequest,
    CurrentSourceNativeCacheManifestRequest, CurrentSourceNativeCachePruneRequest,
    CurrentSourceNativeCacheRequest, CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS,
    CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS, CURRENT_SOURCE_CACHE_MAX_BYTES,
};
use ait_core::object_diff::DEFAULT_SNAPSHOT_DIFF_MAX_BYTES;
use ait_core::external::update::ExternalUpdateOptions;
use ait_core::plan_command_execution::{
    execute_plan_candidates_command_request_json, execute_plan_inspect_command_request_json,
    execute_plan_items_command_request_json, execute_plan_list_command_request_json,
    execute_plan_revisions_command_request_json, execute_plan_show_command_request_json,
};
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
use clap::error::ContextKind;
use clap::{ArgAction, ArgGroup, Args, Parser, Subcommand, ValueEnum};
use ait_core::json_support::{json, JsonMap, JsonValue};
use std::env;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
