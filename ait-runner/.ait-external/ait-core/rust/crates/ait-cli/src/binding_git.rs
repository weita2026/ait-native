//! Existing Git interoperability services with native CLI workspace locking.
use crate::{primitives, runtime::RepoRuntime, workspace_lock::run_locked_workspace_command};
use ait_core::json_support::JsonValue;
use std::path::Path;

pub fn migrate_git(
    repo: &RepoRuntime,
    staging_root: &Path,
    apply: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli git migrate", || {
        primitives::git_migrate(repo, staging_root, apply)
    })
}

pub fn recover_git(repo: &RepoRuntime, apply: bool) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli git recover", || {
        primitives::git_recover(repo, apply)
    })
}

pub fn import_git(
    repo: &RepoRuntime,
    source: &str,
    all_branches_and_tags: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli git import", || {
        primitives::git_import(repo, source, all_branches_and_tags, dry_run)
    })
}

pub fn export_git(
    repo: &RepoRuntime,
    target: &str,
    all_lines_and_tags: bool,
    dry_run: bool,
    generation: Option<&str>,
    git_object: Option<&str>,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli git export", || {
        primitives::git_export_selection(
            repo,
            target,
            all_lines_and_tags,
            dry_run,
            generation,
            git_object,
        )
    })
}

pub fn restore_git(
    repo: &RepoRuntime,
    generation: &str,
    git_object: &str,
    target: &Path,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli git restore", || {
        primitives::git_restore(repo, generation, git_object, target)
    })
}

pub fn mirror_git(
    repo: &RepoRuntime,
    endpoint: &str,
    direction: &str,
    dry_run: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli git mirror", || {
        primitives::git_mirror(repo, endpoint, direction, dry_run)
    })
}
