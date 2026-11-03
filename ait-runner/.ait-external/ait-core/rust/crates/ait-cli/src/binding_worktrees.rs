//! Native worktree CLI orchestration shared by language bindings.
use crate::{primitives, runtime::RepoRuntime, workspace_lock::run_locked_workspace_command};
use ait_core::json_support::JsonValue;

pub fn show_worktree(repo: &RepoRuntime, name: Option<&str>) -> Result<JsonValue, String> {
    primitives::worktree_show(repo, name, true)
}

pub fn open_worktree_path(repo: &RepoRuntime, name: Option<&str>) -> Result<JsonValue, String> {
    let payload = primitives::worktree_get(repo, name, true)?;
    let touched = primitives::worktree_touch_usage(repo, name)?;
    if touched.is_null() {
        Ok(payload)
    } else {
        Ok(touched)
    }
}

#[derive(Default)]
pub struct WorktreeRestoreRequest {
    pub name: Option<String>,
    pub snapshot_id: Option<String>,
    pub line_name: Option<String>,
    pub paths: Vec<String>,
    pub force: bool,
    pub dry_run: bool,
}

pub fn restore_worktree(
    repo: &RepoRuntime,
    request: &WorktreeRestoreRequest,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli worktree restore", || {
        primitives::worktree_restore(
            repo,
            request.name.as_deref(),
            request.snapshot_id.as_deref(),
            request.line_name.as_deref(),
            &request.paths,
            request.force,
            request.dry_run,
        )
    })
}

pub(crate) const WORKTREE_DESTRUCTIVE_CONFIRMATION_ERROR: &str =
    "Pass --yes to apply this destructive worktree operation, or use --dry-run to preview it.";

pub(crate) fn require_worktree_destructive_confirmation(
    dry_run: bool,
    yes: bool,
) -> Result<(), String> {
    if !dry_run && !yes {
        return Err(WORKTREE_DESTRUCTIVE_CONFIRMATION_ERROR.to_string());
    }
    Ok(())
}

#[derive(Default)]
pub struct WorktreeCleanupRequest {
    pub older_than: Option<String>,
    pub cleanup_policy: Option<String>,
    pub allow_manual_only: bool,
    pub limit: Option<usize>,
    pub dry_run: bool,
    pub yes: bool,
}

pub fn cleanup_worktrees(
    repo: &RepoRuntime,
    request: &WorktreeCleanupRequest,
) -> Result<JsonValue, String> {
    require_worktree_destructive_confirmation(request.dry_run, request.yes)?;
    run_locked_workspace_command(repo, "ait-cli worktree cleanup", || {
        primitives::worktree_cleanup(
            repo,
            request.older_than.as_deref(),
            request.cleanup_policy.as_deref(),
            request.allow_manual_only,
            request.limit,
            request.dry_run,
        )
    })
}

pub fn prune_stale_worktrees(
    repo: &RepoRuntime,
    dry_run: bool,
    yes: bool,
) -> Result<JsonValue, String> {
    require_worktree_destructive_confirmation(dry_run, yes)?;
    primitives::worktree_prune_stale(repo, dry_run)
}

#[derive(Default)]
pub struct WorktreeSyncRequest {
    pub name: Option<String>,
    pub line_name: Option<String>,
    pub all_worktrees: bool,
    pub force: bool,
    pub dry_run: bool,
}

pub fn sync_worktree(
    repo: &RepoRuntime,
    request: &WorktreeSyncRequest,
) -> Result<JsonValue, String> {
    if request.all_worktrees && request.name.is_some() {
        return Err("Choose either a worktree name or --all".to_string());
    }
    if request.all_worktrees && request.line_name.is_some() {
        return Err(
            "--line cannot be combined with --all; each worktree syncs to its own current line"
                .to_string(),
        );
    }
    run_locked_workspace_command(repo, "ait-cli worktree sync", || {
        if request.all_worktrees {
            primitives::worktree_sync_all(repo, request.force, request.dry_run)
        } else {
            primitives::worktree_sync(
                repo,
                request.name.as_deref(),
                request.line_name.as_deref(),
                request.force,
                request.dry_run,
            )
        }
    })
}

pub fn worktree_sync_exit_code(payload: &JsonValue, all_worktrees: bool) -> u8 {
    if all_worktrees
        && !payload
            .get("ok")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true)
    {
        2
    } else {
        0
    }
}

pub fn recreate_worktree(
    repo: &RepoRuntime,
    name: Option<&str>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli worktree recreate", || {
        primitives::worktree_recreate(repo, name, dry_run)
    })
}

pub fn recover_task_worktree(
    repo: &RepoRuntime,
    task_id: &str,
    change_ref: Option<&str>,
    remote_name: Option<&str>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli worktree recover-task", || {
        primitives::worktree_recover_task(
            repo,
            task_id,
            change_ref.unwrap_or(""),
            remote_name,
            dry_run,
        )
    })
}

pub fn restore_worktree_owned_head(
    repo: &RepoRuntime,
    name: Option<&str>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli worktree restore-owned-head", || {
        primitives::worktree_restore_owned_head(repo, name, dry_run)
    })
}

#[derive(Default)]
pub struct WorktreeRebaseRequest {
    pub name: Option<String>,
    pub onto_line: Option<String>,
    pub continue_rebase: bool,
    pub abort_rebase: bool,
    pub dry_run: bool,
}

pub fn rebase_worktree(
    repo: &RepoRuntime,
    request: &WorktreeRebaseRequest,
) -> Result<JsonValue, String> {
    if request.continue_rebase && request.abort_rebase {
        return Err("Choose either --continue or --abort".to_string());
    }
    if request.dry_run && (request.continue_rebase || request.abort_rebase) {
        return Err("--dry-run cannot be combined with --continue or --abort".to_string());
    }
    if request.continue_rebase {
        run_locked_workspace_command(repo, "ait-cli worktree rebase --continue", || {
            primitives::worktree_continue_rebase(repo, request.name.as_deref())
        })
    } else if request.abort_rebase {
        run_locked_workspace_command(repo, "ait-cli worktree rebase --abort", || {
            primitives::worktree_abort_rebase(repo, request.name.as_deref())
        })
    } else {
        run_locked_workspace_command(repo, "ait-cli worktree rebase", || {
            if request.dry_run {
                primitives::worktree_preview_rebase(
                    repo,
                    request.name.as_deref(),
                    request.onto_line.as_deref(),
                )
            } else {
                primitives::worktree_rebase(
                    repo,
                    request.name.as_deref(),
                    request.onto_line.as_deref(),
                )
            }
        })
    }
}

#[derive(Default)]
pub struct WorktreeRemoveRequest {
    pub names: Vec<String>,
    pub all_stale: bool,
    pub delete_path: bool,
    pub force: bool,
    pub dry_run: bool,
    pub yes: bool,
}

pub fn remove_worktrees(
    repo: &RepoRuntime,
    request: &WorktreeRemoveRequest,
) -> Result<JsonValue, String> {
    if request.all_stale && !request.names.is_empty() {
        return Err("Choose either one or more worktree names or --all-stale".to_string());
    }
    if !request.all_stale && request.names.is_empty() {
        return Err("Provide one or more worktree names or use --all-stale".to_string());
    }
    if request.all_stale && request.delete_path {
        return Err("--delete-path cannot be combined with --all-stale".to_string());
    }
    if request.all_stale && request.force {
        return Err("--force cannot be combined with --all-stale".to_string());
    }
    require_worktree_destructive_confirmation(request.dry_run, request.yes)?;
    run_locked_workspace_command(repo, "ait-cli worktree remove", || {
        if request.all_stale {
            primitives::worktree_prune_stale(repo, request.dry_run)
        } else {
            primitives::worktree_remove(
                repo,
                &request.names,
                false,
                request.delete_path,
                request.force,
                request.dry_run,
            )
        }
    })
}
