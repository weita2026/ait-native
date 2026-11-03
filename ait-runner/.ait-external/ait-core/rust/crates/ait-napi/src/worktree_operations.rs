use crate::{
    repository::repository,
    workspace::optional_count,
    worktrees::{
        TaskWorkflowWorktreeRecreateOptions, TaskWorkflowWorktreeRestoreOptions,
        TaskWorkflowWorktreeRestoreOwnedHeadOptions,
    },
    JsonInput,
};
use ait_cli::binding_worktrees as service;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn worktree_show(root: String, name: Option<String>) -> Result<JsonValue> {
    service::show_worktree(&repository(&root)?, name.as_deref()).map_err(Error::from_reason)
}

#[napi]
pub fn worktree_path(root: String, name: Option<String>) -> Result<JsonValue> {
    service::open_worktree_path(&repository(&root)?, name.as_deref()).map_err(Error::from_reason)
}

#[napi]
pub fn worktree_restore(
    root: String,
    options: Option<TaskWorkflowWorktreeRestoreOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::restore_worktree(
        &repository(&root)?,
        &service::WorktreeRestoreRequest {
            name: options.name,
            snapshot_id: options.snapshot_id,
            line_name: options.line_name,
            paths: options.paths.unwrap_or_default(),
            force: options.force.unwrap_or(false),
            dry_run: options.dry_run.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorktreeCleanupOptions {
    pub older_than: Option<String>,
    pub cleanup_policy: Option<String>,
    pub allow_manual_only: Option<bool>,
    pub limit: Option<JsonInput>,
    pub dry_run: Option<bool>,
    pub yes: Option<bool>,
}

#[napi]
pub fn worktree_cleanup(
    root: String,
    options: Option<WorktreeCleanupOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::cleanup_worktrees(
        &repository(&root)?,
        &service::WorktreeCleanupRequest {
            older_than: options.older_than,
            cleanup_policy: options.cleanup_policy,
            allow_manual_only: options.allow_manual_only.unwrap_or(false),
            limit: optional_count(options.limit, "limit")?,
            dry_run: options.dry_run.unwrap_or(false),
            yes: options.yes.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorktreePruneOptions {
    pub dry_run: Option<bool>,
    pub yes: Option<bool>,
}

#[napi]
pub fn worktree_prune_stale(
    root: String,
    options: Option<WorktreePruneOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::prune_stale_worktrees(
        &repository(&root)?,
        options.dry_run.unwrap_or(false),
        options.yes.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorktreeSyncOptions {
    pub name: Option<String>,
    pub line_name: Option<String>,
    pub all_worktrees: Option<bool>,
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn worktree_sync(root: String, options: Option<WorktreeSyncOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::sync_worktree(
        &repository(&root)?,
        &service::WorktreeSyncRequest {
            name: options.name,
            line_name: options.line_name,
            all_worktrees: options.all_worktrees.unwrap_or(false),
            force: options.force.unwrap_or(false),
            dry_run: options.dry_run.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn worktree_sync_exit_code(payload: JsonInput, all_worktrees: bool) -> u8 {
    service::worktree_sync_exit_code(&payload, all_worktrees)
}

#[napi]
pub fn worktree_recreate(
    root: String,
    options: Option<TaskWorkflowWorktreeRecreateOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::recreate_worktree(
        &repository(&root)?,
        options.name.as_deref(),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorktreeRecoverTaskOptions {
    pub change_ref: Option<String>,
    pub remote_name: Option<String>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn worktree_recover_task(
    root: String,
    task_id: String,
    options: Option<WorktreeRecoverTaskOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::recover_task_worktree(
        &repository(&root)?,
        &task_id,
        options.change_ref.as_deref(),
        options.remote_name.as_deref(),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn worktree_restore_owned_head(
    root: String,
    options: Option<TaskWorkflowWorktreeRestoreOwnedHeadOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::restore_worktree_owned_head(
        &repository(&root)?,
        options.name.as_deref(),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorktreeRebaseOptions {
    pub name: Option<String>,
    pub onto_line: Option<String>,
    pub continue_rebase: Option<bool>,
    pub abort_rebase: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn worktree_rebase(root: String, options: Option<WorktreeRebaseOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::rebase_worktree(
        &repository(&root)?,
        &service::WorktreeRebaseRequest {
            name: options.name,
            onto_line: options.onto_line,
            continue_rebase: options.continue_rebase.unwrap_or(false),
            abort_rebase: options.abort_rebase.unwrap_or(false),
            dry_run: options.dry_run.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorktreeRemoveOptions {
    pub all_stale: Option<bool>,
    pub delete_path: Option<bool>,
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
    pub yes: Option<bool>,
}

#[napi]
pub fn worktree_remove(
    root: String,
    names: Vec<String>,
    options: Option<WorktreeRemoveOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::remove_worktrees(
        &repository(&root)?,
        &service::WorktreeRemoveRequest {
            names,
            all_stale: options.all_stale.unwrap_or(false),
            delete_path: options.delete_path.unwrap_or(false),
            force: options.force.unwrap_or(false),
            dry_run: options.dry_run.unwrap_or(false),
            yes: options.yes.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_restore(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_restore(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_show(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_show(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_recreate(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_recreate(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_recover_task(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_recover_task(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_restore_owned_head(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_restore_owned_head(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_rebase(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_rebase(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_list(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_list(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_doctor(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_doctor(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_cleanup_candidates(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_cleanup_candidates(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_sync(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_sync(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_prune_stale(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_prune_stale(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_cleanup(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_worktree_cleanup(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_worktree_status(payload: JsonInput, verbose: Option<bool>) -> Result<String> {
    ait_cli::binding_output::format_worktree_status(&payload, verbose.unwrap_or(false))
        .map_err(Error::from_reason)
}
#[napi]
pub fn format_worktree_path(payload: JsonInput, shell_output: Option<bool>) -> Result<String> {
    ait_cli::binding_output::format_worktree_path(&payload, shell_output.unwrap_or(false))
        .map_err(Error::from_reason)
}
#[napi]
pub fn project_worktree_path(payload: JsonInput) -> Result<JsonValue> {
    ait_cli::binding_output::project_worktree_path(&payload).map_err(Error::from_reason)
}
#[napi]
pub fn format_worktree_remove(
    payload: JsonInput,
    all_stale: bool,
    dry_run: bool,
    multiple: bool,
) -> Result<String> {
    ait_cli::binding_output::format_worktree_remove(&payload, all_stale, dry_run, multiple)
        .map_err(Error::from_reason)
}
