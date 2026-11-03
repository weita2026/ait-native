use super::*;
use crate::primitives::workflow::workflow_registered_worktree_metadata_rows;
use crate::primitives::worktree::{load_worktree_metadata, remove_one_worktree_after_task_abandon};
use crate::workspace_lock::WorkspaceCommandLock;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) struct PreparedCleanup {
    task_id: String,
    target: Option<Target>,
    _writer: Option<WorkspaceCommandLock>,
}

struct Target {
    name: String,
    path: PathBuf,
    protected: Option<String>,
}

fn physical_path(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("Task worktree path is not absolute".into());
    }
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            path.canonicalize().map_err(|e| e.to_string())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or("Task worktree path has no parent")?;
            Ok(parent
                .canonicalize()
                .map_err(|e| e.to_string())?
                .join(path.file_name().ok_or("Task worktree path has no name")?))
        }
        _ => Err("Task worktree path is aliased or is not a physical directory".into()),
    }
}

fn same_root(value: Option<String>, expected: &Path) -> bool {
    value
        .and_then(|v| Path::new(&v).canonicalize().ok())
        .is_some_and(|p| p == expected)
}

fn validate_change(root: &RepoRuntime, task: &str, row: &JsonValue) -> Result<(), String> {
    for key in ["bound_change_ref", "bound_change_id"] {
        if let Some(change) = string_field(row, key) {
            let reference = if change.starts_with("C-") {
                format!("{task}/{change}")
            } else {
                change
            };
            let actual = change_local_read_with_change_store(&root.change_store()?, &reference)?;
            if string_field(&actual, "task_id").as_deref() != Some(task) {
                return Err("Task worktree has a foreign native Change owner".into());
            }
        }
    }
    Ok(())
}

fn target(root: &RepoRuntime, task: &str, name: &str) -> Result<Target, String> {
    let metadata = load_worktree_metadata(root, name)?;
    let row = JsonValue::Object(metadata.clone());
    if metadata_string(&metadata, "bound_task_id").as_deref() != Some(task) {
        return Err("Task worktree registration owner changed".into());
    }
    let authority = root
        .authoritative_repo_root()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !same_root(metadata_string(&metadata, "repo_root"), &authority) {
        return Err("Task worktree registration belongs to another repository".into());
    }
    let path = physical_path(&required_path_field(&row, "path")?)?;
    if authority.starts_with(&path)
        || path.starts_with(authority.join("docs"))
        || path.starts_with(authority.join(APP_DIR))
    {
        return Err("Task worktree path aliases canonical/shared repository data".into());
    }
    validate_change(root, task, &row)?;
    let marker = path.join(WORKTREE_CONFIG_NAME);
    let overlay = match fs::symlink_metadata(&marker) {
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            let text = fs::read_to_string(&marker).map_err(|e| e.to_string())?;
            let overlay = crate::json_support::parse_value(&text, "Task worktree overlay")?;
            // Native overlays carry physical identity; Task ownership lives in
            // the registry. Reject a conflicting optional overlay binding.
            if string_field(&overlay, "bound_task_id").is_some_and(|owner| owner != task)
                || string_field(&overlay, "worktree_name").as_deref() != Some(name)
                || !same_root(string_field(&overlay, "repo_root"), &authority)
                || physical_path(&required_path_field(&overlay, "workspace_root")?)? != path
            {
                return Err(
                    "Task worktree overlay conflicts with its exact registered owner/root".into(),
                );
            }
            validate_change(root, task, &overlay)?;
            Some(overlay)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Removal keeps the overlay until the last files. Without it, only
            // an absent/empty root or the exact authority link can be resumed.
            if path.is_dir() {
                for entry in fs::read_dir(&path).map_err(|e| e.to_string())? {
                    let entry = entry.map_err(|e| e.to_string())?;
                    if entry.file_name() != APP_DIR {
                        return Err("Task worktree lost its owner overlay but still contains files; cleanup is blocked".into());
                    }
                }
            }
            None
        }
        _ => return Err("Task worktree owner overlay is aliased or unavailable".into()),
    };
    let ait_link = path.join(APP_DIR);
    match fs::symlink_metadata(&ait_link) {
        Ok(meta) if meta.file_type().is_symlink() => {
            if ait_link.canonicalize().map_err(|e| e.to_string())?
                != authority
                    .join(APP_DIR)
                    .canonicalize()
                    .map_err(|e| e.to_string())?
            {
                return Err("Task worktree authority link points to another repository".into());
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err("Task worktree authority entry is not its shared link".into()),
    }
    if let Some(alias) = metadata_string(&metadata, "alias_path").map(PathBuf::from) {
        match fs::symlink_metadata(&alias) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let destination = fs::read_link(&alias).map_err(|e| e.to_string())?;
                let destination = if destination.is_absolute() {
                    destination
                } else {
                    alias
                        .parent()
                        .ok_or("Task alias has no parent")?
                        .join(destination)
                };
                if physical_path(&destination)? != path {
                    return Err("Task worktree alias points to another root".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err("Task worktree alias is not a symlink".into()),
        }
    }
    let mut protected = None;
    for state in [
        metadata_string(&metadata, "merge_state"),
        metadata_string(&metadata, "rebase_state"),
        overlay
            .as_ref()
            .and_then(|v| string_field(v, "merge_state")),
        overlay
            .as_ref()
            .and_then(|v| string_field(v, "rebase_state")),
    ]
    .into_iter()
    .flatten()
    {
        if state != "idle" {
            protected = Some(format!(
                "Task worktree has an active merge/rebase ({state})"
            ));
        }
    }
    if metadata
        .get("auto_created_for_task")
        .and_then(JsonValue::as_bool)
        != Some(true)
        || metadata_string(&metadata, "creation_kind").as_deref() != Some("task_auto_created")
        || matches!(
            metadata_string(&metadata, "cleanup_policy").as_deref(),
            Some("manual_only" | "never")
        )
    {
        protected = Some("Task worktree has manual/protected cleanup ownership".into());
    } else {
        match metadata_string(&metadata, "root_source").as_deref() {
            Some("explicit") => {
                protected = Some(
                    "Task worktree was supplied explicitly; automatic deletion is not authorized"
                        .into(),
                )
            }
            Some(
                "configured_ephemeral_root"
                | "repo_internal_fallback"
                | "linux_xdg_runtime_dir"
                | "linux_dev_shm"
                | "linux_tmpfs"
                | "linux_memory_root"
                | "windows_ramdisk"
                | "macos_ram_volume",
            ) => {}
            _ => return Err("Task worktree has an unresolved managed-root source".into()),
        }
    }
    Ok(Target {
        name: name.into(),
        path,
        protected,
    })
}

pub(super) fn prepare(
    root: &RepoRuntime,
    caller: &RepoRuntime,
    task: &str,
) -> Result<PreparedCleanup, String> {
    let expected_name = task_bound_worktree_name(task)?;
    let rows = workflow_registered_worktree_metadata_rows(root)?;
    let registry = root.authoritative_repo_root().join(".ait/worktrees");
    if registry.is_dir() {
        let count = fs::read_dir(registry)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .iter()
            .filter(|entry| entry.path().extension().is_some_and(|v| v == "json"))
            .count();
        if count != rows.len() {
            return Err(
                "Task abandonment cannot resolve an incomplete worktree registration inventory"
                    .into(),
            );
        }
    }
    // A stale foreign/null registry owner cannot hide this Task's valid overlay.
    let authority = root
        .authoritative_repo_root()
        .canonicalize()
        .map_err(|e| e.to_string())?;
    for (name, metadata) in &rows {
        if metadata_string(metadata, "bound_task_id").as_deref() == Some(task) {
            continue;
        }
        if *name == expected_name {
            return Err("Task worktree registration conflicts with its native Task owner".into());
        }
        if let Some(path) = metadata_string(metadata, "path") {
            let marker = Path::new(&path).join(WORKTREE_CONFIG_NAME);
            if fs::symlink_metadata(&marker)
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
            {
                let overlay = read_json_value(&marker);
                if string_field(&overlay, "bound_task_id").as_deref() == Some(task)
                    && same_root(string_field(&overlay, "repo_root"), &authority)
                {
                    return Err(
                        "Task worktree overlay conflicts with its registration owner".into(),
                    );
                }
            }
        }
    }
    let owned = rows
        .iter()
        .filter(|(_, metadata)| metadata_string(metadata, "bound_task_id").as_deref() == Some(task))
        .collect::<Vec<_>>();
    if owned.len() > 1 {
        return Err("Task abandonment has multiple registered roots; resolve exact ownership before closing".into());
    }
    let Some((name, _)) = owned.first() else {
        if current_worktree_metadata(caller)?
            .is_some_and(|m| m.bound_task_id.as_deref() == Some(task) || m.name == expected_name)
        {
            return Err(
                "Task worktree has no exact registration; resolve ownership before closing".into(),
            );
        }
        return Ok(PreparedCleanup {
            task_id: task.into(),
            target: None,
            _writer: None,
        });
    };
    if *name != expected_name {
        return Err("Task abandonment has a conflicting native worktree name".into());
    }
    let initial = target(root, task, name)?;
    for (other_name, metadata) in &rows {
        if other_name == name {
            continue;
        }
        if let Some(other_path) = metadata_string(metadata, "path") {
            if physical_path(Path::new(&other_path)).ok().as_ref() == Some(&initial.path) {
                return Err("Task worktree root is also registered to another owner".into());
            }
        }
    }
    let writer = WorkspaceCommandLock::try_acquire_workspace(
        root,
        &initial.path,
        "ait task abandon owned worktree",
    )?;
    let verified = target(root, task, name)?;
    if verified.path != initial.path {
        return Err("Task worktree root changed while acquiring its writer lock".into());
    }
    Ok(PreparedCleanup {
        task_id: task.into(),
        target: Some(verified),
        _writer: Some(writer),
    })
}

#[cfg(test)]
#[cfg(unix)]
mod tests;

impl PreparedCleanup {
    pub(super) fn guard_pending_finish(&self, root: &RepoRuntime) -> Result<(), String> {
        if let Some(target) = &self.target {
            root.task_store()?
                .guard_no_pending_local_finish(
                    &self.task_id,
                    &target.path.join(".ait-worktree/cursor.bin"),
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub(super) fn finish(&self, root: &RepoRuntime) -> Result<JsonValue, String> {
        let task = task_local_read_with_task_store(&root.task_store()?, &self.task_id)?;
        if string_field(&task, "status").as_deref() != Some("abandoned") {
            return Err("Task worktree cleanup requires authoritative abandonment".into());
        }
        let Some(captured) = &self.target else {
            return Ok(json!({"status":"already_removed","task_id":self.task_id}));
        };
        let verified = target(root, &self.task_id, &captured.name)?;
        if verified.path != captured.path {
            return Err("Task worktree root changed after abandonment".into());
        }
        if let Some(reason) = verified.protected {
            return Err(reason);
        }
        self.guard_pending_finish(root)?;
        let mut configuration = crate::config_surface::begin_root_config_mutation(root)?;
        let removed =
            remove_one_worktree_after_task_abandon(root, &captured.name, &mut configuration)?;
        Ok(json!({"status":"removed","task_id":self.task_id,"worktree":removed}))
    }
}
