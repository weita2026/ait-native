//! Workspace-only import setup. No Snapshot, ref, Plan or policy mutation.
use crate::{agent_harness, init_surface, runtime::RepoRuntime};
use ait_core::json_support::{json, JsonValue};
use std::fs;

pub(crate) fn preflight(repo: &RepoRuntime) -> Result<(), String> {
    let root = repo.authoritative_repo_root();
    if repo.sprint_enabled() {
        init_surface::validate_real_directory_tree(&root, "docs/sprints")?;
    }
    let ignore = root.join(".aitignore");
    if init_surface::regular_file_state(&ignore, "workspace ignore file")? {
        fs::read_to_string(&ignore)
            .map_err(|e| format!("Invalid or unreadable .aitignore; content preserved: {e}"))?;
    }
    agent_harness::validate_import_guidance(repo)?;
    Ok(())
}

pub(crate) fn complete(repo: &RepoRuntime) -> Result<JsonValue, String> {
    complete_with_observer(repo, |_| Ok(()))
}

fn complete_with_observer(
    repo: &RepoRuntime,
    mut observe: impl FnMut(&str) -> Result<(), String>,
) -> Result<JsonValue, String> {
    // Revalidate after import: another writer may have supplied a custom file.
    preflight(repo)?;
    let root = repo.authoritative_repo_root();
    let mut created = Vec::new();
    let mut preserved = Vec::new();
    let mut note = |path: &str, changed: bool| -> Result<(), String> {
        if changed {
            created.push(path.to_string());
            observe(path)?;
        } else {
            preserved.push(path.to_string());
        }
        Ok(())
    };
    note(
        ".aitignore",
        init_surface::create_workspace_ignore_if_absent(&root)?,
    )?;
    for (path, changed) in agent_harness::create_import_guidance_if_absent(repo)? {
        note(&path, changed)?;
    }
    let sprint_missing = repo.sprint_enabled() && !root.join("docs/sprints").exists();
    agent_harness::ensure_configured_sprint_directory(repo)?;
    if repo.sprint_enabled() {
        note("docs/sprints", sprint_missing)?;
    }
    preflight(repo)?;
    let guidance_sync_required = created.iter().any(|p| p == "AGENTS.md" || p == "CLAUDE.md");
    Ok(json!({
        "contract": "ait.git.import-workspace-bootstrap/v1",
        "root": root,
        "status": "completed",
        "mutated": !created.is_empty(),
        "created_paths": created,
        "preserved_paths": preserved,
        "guidance_plan_sync_required": guidance_sync_required,
        "code_snapshot_created": false,
        "source_history_rewritten": false,
    }))
}

#[cfg(test)]
mod tests;
