//! Retained PyO3 worktree capabilities, delegated to existing Rust primitives.
use crate::{repository::repository, workspace::native_count, JsonInput};
use ait_cli::primitives;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeGetOptions {
    pub name: Option<String>,
    pub refresh_status: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_get(
    root: String,
    options: Option<TaskWorkflowWorktreeGetOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_get(
        &repository(&root)?,
        options.name.as_deref(),
        options.refresh_status.unwrap_or(true),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeStatusOptions {
    pub name: Option<String>,
    pub snapshot_id: Option<String>,
    pub line_name: Option<String>,
}

#[napi]
pub fn task_workflow_worktree_status(
    root: String,
    options: Option<TaskWorkflowWorktreeStatusOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_status(
        &repository(&root)?,
        options.name.as_deref(),
        options.snapshot_id.as_deref(),
        options.line_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeRestoreOptions {
    pub name: Option<String>,
    pub snapshot_id: Option<String>,
    pub line_name: Option<String>,
    pub paths: Option<Vec<String>>,
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_restore(
    root: String,
    options: Option<TaskWorkflowWorktreeRestoreOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_restore(
        &repository(&root)?,
        options.name.as_deref(),
        options.snapshot_id.as_deref(),
        options.line_name.as_deref(),
        &options.paths.unwrap_or_default(),
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeListOptions {
    pub refresh_status: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_list(
    root: String,
    options: Option<TaskWorkflowWorktreeListOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_list(&repository(&root)?, options.refresh_status.unwrap_or(true))
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeTouchUsageOptions {
    pub name: Option<String>,
}

#[napi]
pub fn task_workflow_worktree_touch_usage(
    root: String,
    options: Option<TaskWorkflowWorktreeTouchUsageOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_touch_usage(&repository(&root)?, options.name.as_deref())
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeDoctorOptions {
    pub refresh_status: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_doctor(
    root: String,
    options: Option<TaskWorkflowWorktreeDoctorOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_doctor(&repository(&root)?, options.refresh_status.unwrap_or(true))
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeCleanupCandidatesOptions {
    pub older_than: Option<String>,
    pub cleanup_policy: Option<String>,
    pub include_protected: Option<bool>,
    pub allow_manual_only: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_cleanup_candidates(
    root: String,
    options: Option<TaskWorkflowWorktreeCleanupCandidatesOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_cleanup_candidates(
        &repository(&root)?,
        options.older_than.as_deref(),
        options.cleanup_policy.as_deref(),
        options.include_protected.unwrap_or(false),
        options.allow_manual_only.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeCleanupOptions {
    pub older_than: Option<String>,
    pub cleanup_policy: Option<String>,
    pub allow_manual_only: Option<bool>,
    pub limit: Option<JsonInput>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_cleanup(
    root: String,
    options: Option<TaskWorkflowWorktreeCleanupOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_cleanup(
        &repository(&root)?,
        options.older_than.as_deref(),
        options.cleanup_policy.as_deref(),
        options.allow_manual_only.unwrap_or(false),
        options
            .limit
            .map(|v| native_count(Some(v), 0, "limit"))
            .transpose()?,
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreePruneStaleOptions {
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_prune_stale(
    root: String,
    options: Option<TaskWorkflowWorktreePruneStaleOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_prune_stale(&repository(&root)?, options.dry_run.unwrap_or(false))
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeRemoveOptions {
    pub all_stale: Option<bool>,
    pub delete_path: Option<bool>,
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_remove(
    root: String,
    names: Vec<String>,
    options: Option<TaskWorkflowWorktreeRemoveOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_remove(
        &repository(&root)?,
        &names,
        options.all_stale.unwrap_or(false),
        options.delete_path.unwrap_or(false),
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeBindExistingOptions {
    pub task_id: Option<String>,
    pub change_id: Option<String>,
    pub auto_created_for_task: Option<bool>,
    pub target_base_line: Option<String>,
    pub fork_snapshot_id: Option<String>,
    pub forked_from_line: Option<String>,
}

#[napi]
pub fn task_workflow_worktree_bind_existing(
    root: String,
    worktree_name: String,
    options: Option<TaskWorkflowWorktreeBindExistingOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_bind_existing(
        &repository(&root)?,
        &worktree_name,
        options.task_id.as_deref(),
        options.change_id.as_deref(),
        options.auto_created_for_task.unwrap_or(false),
        options.target_base_line.as_deref(),
        options.fork_snapshot_id.as_deref(),
        options.forked_from_line.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeSyncOptions {
    pub name: Option<String>,
    pub line_name: Option<String>,
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_sync(
    root: String,
    options: Option<TaskWorkflowWorktreeSyncOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_sync(
        &repository(&root)?,
        options.name.as_deref(),
        options.line_name.as_deref(),
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeSyncAllOptions {
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_sync_all(
    root: String,
    options: Option<TaskWorkflowWorktreeSyncAllOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_sync_all(
        &repository(&root)?,
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeRecreateOptions {
    pub name: Option<String>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_recreate(
    root: String,
    options: Option<TaskWorkflowWorktreeRecreateOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_recreate(
        &repository(&root)?,
        options.name.as_deref(),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeRestoreOwnedHeadOptions {
    pub name: Option<String>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn task_workflow_worktree_restore_owned_head(
    root: String,
    options: Option<TaskWorkflowWorktreeRestoreOwnedHeadOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_restore_owned_head(
        &repository(&root)?,
        options.name.as_deref(),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreePreviewRebaseOptions {
    pub name: Option<String>,
    pub onto_line_name: Option<String>,
}

#[napi]
pub fn task_workflow_worktree_preview_rebase(
    root: String,
    options: Option<TaskWorkflowWorktreePreviewRebaseOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_preview_rebase(
        &repository(&root)?,
        options.name.as_deref(),
        options.onto_line_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeRebaseOptions {
    pub name: Option<String>,
    pub onto_line_name: Option<String>,
}

#[napi]
pub fn task_workflow_worktree_rebase(
    root: String,
    options: Option<TaskWorkflowWorktreeRebaseOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_rebase(
        &repository(&root)?,
        options.name.as_deref(),
        options.onto_line_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeContinueRebaseOptions {
    pub name: Option<String>,
}

#[napi]
pub fn task_workflow_worktree_continue_rebase(
    root: String,
    options: Option<TaskWorkflowWorktreeContinueRebaseOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_continue_rebase(&repository(&root)?, options.name.as_deref())
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowWorktreeAbortRebaseOptions {
    pub name: Option<String>,
}

#[napi]
pub fn task_workflow_worktree_abort_rebase(
    root: String,
    options: Option<TaskWorkflowWorktreeAbortRebaseOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::worktree_abort_rebase(&repository(&root)?, options.name.as_deref())
        .map_err(Error::from_reason)
}
