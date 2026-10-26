use crate::binding_services::{
    attach_automatic_reconciliation, run_automatic_reconciliation_locked,
    task_land_automatic_trigger, validate_scope, workflow_payload_task_id,
};
use ait_cli::auth_surface::{
    auth_bindings as auth_bindings_cmd, auth_grant as auth_grant_cmd,
    auth_whoami as auth_whoami_cmd, AuthGrantRequest, AuthRemoteRequest,
};
use ait_cli::blame_surface::{blame as blame_cmd, render_human_blame, BlameRequest};
use ait_cli::config_surface::{
    config_show as config_show_cmd, config_unset as config_unset_cmd, ConfigSetRequest,
    ConfigUnsetKey,
};
use ait_cli::doctor_surface::{
    doctor_memory_root, doctor_plan_authority, doctor_plan_authority_for_repository,
    doctor_runtime_root, render_doctor_text,
};
use ait_cli::external_surface::{
    external_link as external_link_cmd,
    external_status as external_status_cmd, external_unlink as external_unlink_cmd,
    external_update as external_update_cmd, render_external_text,
};
#[cfg(test)]
use ait_cli::init_surface::init_repo as init_cmd;
use ait_cli::init_surface::InitRequest;
use ait_cli::perfetto_range;
use ait_cli::post_finish_hooks::{
    post_finish_hooks_text_lines, pre_finish_hooks_text_lines, task_finish_exit_code,
    POST_FINISH_HOOKS_PAYLOAD_KEY,
};
use ait_cli::primitives::{
    attest_put, attest_show as attest_show_cmd, change_list as change_list_cmd, line_archive,
    line_cleanup, line_create, line_delete, line_merge, line_rename, line_show, line_show_remote,
    line_switch, patchset_ci_status as patchset_ci_status_cmd,
    patchset_rerun_ci as patchset_rerun_ci_cmd, patchset_select as patchset_select_cmd,
    patchset_show as patchset_show_cmd, plan_backed_markdown_text_lines,
    plan_markdown_materialization_text_lines, plan_markdown_presync_text_lines,
    plan_sync_root_path, policy_eval, policy_show, policy_waive,
    queue_summary as queue_summary_cmd, repo_status as repo_status_cmd,
    resolve_task_scoped_execution_repo, review_code_template, run_task_scoped_workspace_command,
    snapshot_list, stash_list, stash_show, task_abandon, task_audit, task_list, task_show,
    validate_workflow_expected_identity, workflow_finish_apply_guarded_with_reviewer,
    workflow_identity_payload, workflow_land_payload,
    workflow_ready_apply_guarded_with_ci_wait_hook, workflow_ready_payload,
    workflow_reconcile_automatic_best_effort, workspace_dirty_diff, worktree_cleanup_candidates,
    worktree_doctor, worktree_list, worktree_status, AutomaticReconciliationScope,
    AutomaticReconciliationTrigger, DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_LIMIT,
    DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_MAX_DEPTH,
};


use ait_cli::remote_surface::{
    remote_add as remote_add_cmd, remote_list as remote_list_cmd, RemoteAddRequest,
};
use ait_cli::render::print_json;
use ait_cli::repo_surface::{
    render_repo_command_text, repo_command as repo_command_cmd, RepoCommandRequest,
};
use ait_cli::runner_hint::{
    attach_runner_hint_best_effort, ci_diagnostic_text, ci_runner_once_text,
    no_runner_hint_outcome, run_ci_once, runner_hint_text, CI_RUNNER_ONCE_PAYLOAD_KEY,
    RUNNER_HINT_PAYLOAD_KEY,
};
use ait_cli::runtime::RepoRuntime;
use ait_cli::snapshot_restore_surface::{snapshot_restore_lines, SnapshotRestoreLinesRequest};
use ait_cli::tag_surface::{
    tag_create as tag_create_cmd, tag_delete as tag_delete_cmd, tag_list as tag_list_cmd,
    tag_show as tag_show_cmd, TagCreateRequest,
};
use ait_cli::task_land_contract::{
    task_land_scope_contract_json, PLAN_SYNC_COMMAND_ABOUT, TASK_FINISH_COMMAND_ABOUT,
    TASK_LAND_CONTRACT_VERSION,
};
use ait_cli::workspace_lock::run_locked_workspace_command;

use ait_core::current_source_cache::{
    current_source_binary_is_fresh_json, current_source_extension_is_fresh_json,
    current_source_native_cache_contract_json as current_source_native_cache_contract_cmd,
    prune_current_source_native_caches_json, release_current_source_native_cache_lease_json,
    validate_current_source_cli_bootstrap, CurrentSourceBinaryFreshnessRequest,
    CurrentSourceCliBootstrapRequest, CurrentSourceExtensionFreshnessRequest,
    CurrentSourceNativeCachePruneRequest, CurrentSourceNativeCacheRequest,
    CURRENT_SOURCE_CACHE_BUILD_STALE_SECONDS, CURRENT_SOURCE_CACHE_IDLE_TTL_SECONDS,
    CURRENT_SOURCE_CACHE_MAX_BYTES,
};
use ait_core::external::update::ExternalUpdateOptions;
use ait_core::json_support::{json, JsonMap, JsonValue};
use ait_core::object_diff::DEFAULT_SNAPSHOT_DIFF_MAX_BYTES;
use ait_core::plan_command_execution::{
    execute_plan_candidates_command_request_json, execute_plan_inspect_command_request_json,
    execute_plan_items_command_request_json, execute_plan_list_command_request_json,
    execute_plan_revisions_command_request_json, execute_plan_show_command_request_json,
};
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;
use clap::error::ContextKind;
use clap::{ArgAction, ArgGroup, Args, Parser, Subcommand, ValueEnum};
use std::env;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
