use super::*;
use crate::primitives::change_flow::change_create_with_task_remote;

pub(in crate::primitives) fn task_start_change_bootstrap_lineage(
    change: Option<&JsonValue>,
    fallback_base_line_name: &str,
) -> Result<(String, Option<String>), String> {
    let resolved_base_line_name = normalized_text(Some(fallback_base_line_name))
        .unwrap_or_else(|| fallback_base_line_name.to_string());
    let Some(change) = change else {
        return Ok((resolved_base_line_name, None));
    };
    let change_id =
        string_field(change, "change_id").unwrap_or_else(|| "unknown change".to_string());
    let change_base_line =
        string_field(change, "base_line").or_else(|| string_field(change, "forked_from_line"));
    if let Some(change_base_line) = change_base_line.as_deref() {
        if !resolved_base_line_name.is_empty() && change_base_line != resolved_base_line_name {
            return Err(format!(
                "Bound change `{change_id}` forks from `{change_base_line}`, not `{resolved_base_line_name}`."
            ));
        }
    }
    Ok((
        change_base_line.unwrap_or(resolved_base_line_name),
        string_field(change, "fork_snapshot_id"),
    ))
}

pub(in crate::primitives) fn ensure_task_feature_line(
    repo: &RepoRuntime,
    task_id: &str,
    base_line_name: &str,
    base_snapshot_id: Option<&str>,
    fallback_to_local_base_line: bool,
) -> Result<JsonValue, String> {
    let line_name = match task_feature_line_bootstrap_target(repo, task_id)? {
        TaskFeatureLineBootstrapTarget::Existing(line) => return Ok(line),
        TaskFeatureLineBootstrapTarget::Create(line_name) => line_name,
    };
    let resolved_base_snapshot_id = resolve_task_feature_base_snapshot_id(
        repo,
        base_line_name,
        base_snapshot_id,
        fallback_to_local_base_line,
    )?;
    create_local_line(repo, &line_name, resolved_base_snapshot_id.as_deref()).map_err(|err| {
        format!(
            "failed to create the task feature line in Binary DB authority {}: {err}",
            repo.ait_dir.join("binary-db").display()
        )
    })
}

enum TaskFeatureLineBootstrapTarget {
    Existing(JsonValue),
    Create(String),
}

fn resolve_task_feature_base_snapshot_id(
    repo: &RepoRuntime,
    base_line_name: &str,
    base_snapshot_id: Option<&str>,
    fallback_to_local_base_line: bool,
) -> Result<Option<String>, String> {
    match normalized_text(base_snapshot_id) {
        Some(snapshot_id) => Ok(Some(snapshot_id)),
        None if fallback_to_local_base_line => Ok(string_field(
            &local_line_row(repo, base_line_name).map_err(|err| {
                format!("failed to read the base line `{base_line_name}` from selected local snapshot store: {err}")
            })?,
            "head_snapshot_id",
        )),
        None => Ok(None),
    }
}

fn task_feature_line_bootstrap_target(
    repo: &RepoRuntime,
    task_id: &str,
) -> Result<TaskFeatureLineBootstrapTarget, String> {
    let candidates = task_feature_line_candidates(task_id)?;
    let primary = candidates
        .first()
        .cloned()
        .ok_or_else(|| format!("Task {task_id} has no feature Line candidate."))?;
    let mut first_unused = None;
    let mut archived = Vec::new();
    for candidate in candidates {
        match local_line_row(repo, &candidate) {
            Ok(line) => {
                if string_field(&line, "status").as_deref() == Some("archived") {
                    archived.push(required_string_field(&line, "line_name")?);
                    continue;
                }
                return Ok(TaskFeatureLineBootstrapTarget::Existing(line));
            }
            Err(err) if err.contains("Unknown line") => {
                if first_unused.is_none() {
                    first_unused = Some(candidate);
                }
            }
            Err(err) => {
                return Err(format!(
                    "failed to inspect candidate task feature line `{candidate}`: {err}"
                ));
            }
        }
    }
    let line_name = if archived.is_empty() {
        primary
    } else if let Some(first_unused) = first_unused {
        first_unused
    } else {
        return Err(format!(
            "All exact feature Line candidates for Task {task_id} are archived and cannot be reused: {}.",
            archived.join(", ")
        ));
    };
    Ok(TaskFeatureLineBootstrapTarget::Create(line_name))
}

pub(in crate::primitives) fn generate_workflow_id(
    repo: &RepoRuntime,
    family: &str,
) -> Result<String, String> {
    let payload = build_plan_workflow_id_payload_json(
        &json!({
            "family": family,
            "namespace_prefix": repo.id_namespace_prefix(),
        })
        .to_string(),
    )?;
    required_string_field(&payload, "generated_id")
}

#[expect(
    clippy::too_many_arguments,
    reason = "summary inputs map directly to persisted worktree metadata"
)]
pub(in crate::primitives) fn worktree_summary_payload(
    worktree_name: &str,
    worktree_path: &Path,
    alias_path: Option<&Path>,
    repo_root: &Path,
    line_name: &str,
    head_snapshot_id: Option<&str>,
    task_id: &str,
    change_id: Option<&str>,
    root_source: Option<&str>,
    fallback_reason: Option<&str>,
    default_line: Option<&str>,
    seed_snapshot_id: Option<&str>,
    seed_snapshot_total_bytes: Option<i64>,
    main_seed_ram_max_bytes: Option<i64>,
    materialization_source: Option<&str>,
    copy_strategy: Option<&str>,
    main_seed: Option<&JsonValue>,
    fork_snapshot_id: Option<&str>,
    forked_from_line: Option<&str>,
    target_base_line: Option<&str>,
) -> Result<JsonValue, String> {
    let bound_change_id = change_id
        .map(|value| ChangeJson::stateless().canonical_change_id(value))
        .transpose()?;
    let bound_change_ref = bound_change_id
        .as_deref()
        .map(|value| ChangeJson::stateless().rolling_server_change_id(Some(task_id), value))
        .transpose()?;
    let open_path = alias_path.unwrap_or(worktree_path);
    let venv_path = worktree_path.join(".venv");
    let cargo_enabled = cargo_worktree_integration_enabled(repo_root, open_path);
    let cargo_target_dir = cargo_enabled.then(|| {
        worktree_cargo_target_dir(open_path)
            .to_string_lossy()
            .to_string()
    });
    let cargo_build_dir = cargo_enabled.then(|| {
        worktree_cargo_build_dir(open_path)
            .to_string_lossy()
            .to_string()
    });
    let shell_command = task_worktree_shell_command(repo_root, open_path);
    Ok(json!({
        "name": worktree_name,
        "path": worktree_path.to_string_lossy().to_string(),
        "alias_path": alias_path.map(|value| value.to_string_lossy().to_string()),
        "open_path": open_path.to_string_lossy().to_string(),
        "cd_command": format!("cd {}", shell_escape(open_path)),
        "shell_command": shell_command,
        "cargo_target_dir": cargo_target_dir,
        "cargo_build_dir": cargo_build_dir,
        "venv_path": venv_path.exists().then(|| venv_path.to_string_lossy().to_string()),
        "repo_root": repo_root.to_string_lossy().to_string(),
        "registered_line_name": line_name,
        "current_line": line_name,
        "head_snapshot_id": normalized_text(head_snapshot_id),
        "created_at": system_event_timestamp(),
        "exists": true,
        "is_current": false,
        "workspace_status": "clean",
        "status_source": "verified",
        "status_checked_at": system_event_timestamp(),
        "needs_retarget": false,
        "clean": true,
        "changed_count": 0,
        "modified_paths": [],
        "missing_paths": [],
        "untracked_paths": [],
        "bound_task_id": task_id,
        "bound_change_id": bound_change_id,
        "bound_change_ref": bound_change_ref,
        "auto_created_for_task": true,
        "creation_kind": "task_auto_created",
        "cleanup_policy": "after_remote_land",
        "root_source": normalized_text(root_source),
        "fallback_reason": normalized_text(fallback_reason),
        "default_line": normalized_text(default_line),
        "seed_snapshot_id": normalized_text(seed_snapshot_id),
        "seed_snapshot_total_bytes": seed_snapshot_total_bytes,
        "main_seed_ram_max_bytes": main_seed_ram_max_bytes,
        "materialization_source": normalized_text(materialization_source),
        "copy_strategy": normalized_text(copy_strategy),
        "main_seed": main_seed.cloned(),
        "last_used_at": system_event_timestamp(),
        "fork_snapshot_id": normalized_text(fork_snapshot_id),
        "forked_from_line": normalized_text(forked_from_line),
        "target_base_line": normalized_text(target_base_line),
    }))
}

pub(in crate::primitives) fn task_worktree_shell_command(
    repo_root: &Path,
    open_path: &Path,
) -> String {
    let mut commands = vec![format!("cd {}", shell_escape(open_path))];
    if cargo_worktree_integration_enabled(repo_root, open_path) {
        let cargo_target_dir = worktree_cargo_target_dir(open_path);
        commands.push(format!(
            "export CARGO_TARGET_DIR={}",
            shell_escape(&cargo_target_dir)
        ));
        let cargo_build_dir = worktree_cargo_build_dir(open_path);
        commands.push(format!(
            "export CARGO_BUILD_BUILD_DIR={}",
            shell_escape(&cargo_build_dir)
        ));
    }
    commands.join(" && ")
}

pub(in crate::primitives) fn shell_escape(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '-' | '_' | '.' | ':'))
    {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', "'\"'\"'"))
    }
}

pub(in crate::primitives) fn ensure_worktree_runtime_layout(
    repo_root: &Path,
    target_path: &Path,
) -> Result<(), String> {
    if !target_path.is_dir() {
        return Ok(());
    }
    let venv_source = repo_root.join(".venv");
    let venv_target = target_path.join(".venv");
    if venv_source.is_dir() && !path_exists_or_directory_link(&venv_target) {
        create_directory_link(&venv_target, &venv_source)?;
    }
    if target_path.join(APP_DIR).exists()
        && cargo_worktree_integration_enabled(repo_root, target_path)
    {
        let shared_build_root = ensure_repository_shared_cargo_build_dir(repo_root)?;
        fs::create_dir_all(shared_build_root.join("workspaces")).map_err(|err| err.to_string())?;
        fs::create_dir_all(shared_build_root.join(MANAGED_WORKTREE_CARGO_BUILD_DIRNAME))
            .map_err(|err| err.to_string())?;
        fs::create_dir_all(worktree_cargo_target_dir(target_path))
            .map_err(|err| err.to_string())?;
    }
    Ok(())
}

pub(in crate::primitives) fn ensure_repository_shared_cargo_build_dir(
    repo_root: &Path,
) -> Result<PathBuf, String> {
    let ait_dir = repo_root.join(APP_DIR);
    let shared_ait_dir = fs::canonicalize(&ait_dir).unwrap_or(ait_dir);
    let shared_path = shared_ait_dir.join(SHARED_CARGO_BUILD_DIRNAME);
    match fs::metadata(&shared_path) {
        Ok(metadata) if metadata.is_dir() => {
            return Ok(fs::canonicalize(&shared_path).unwrap_or(shared_path));
        }
        Ok(_) => {
            return Err(format!(
                "Repository shared Cargo build path is not a directory: {}",
                shared_path.display()
            ));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "Failed to inspect repository shared Cargo build path {}: {err}",
                shared_path.display()
            ));
        }
    }
    let dangling_link = match fs::symlink_metadata(&shared_path) {
        Ok(metadata) if metadata.file_type().is_symlink() => true,
        Ok(_) => {
            return Err(format!(
                "Repository shared Cargo build path is not a directory link: {}",
                shared_path.display()
            ));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
        Err(err) => {
            return Err(format!(
                "Failed to inspect repository shared Cargo build link {}: {err}",
                shared_path.display()
            ));
        }
    };

    let config = read_json_value(&shared_ait_dir.join("config.json"));
    let configured_memory_root = config
        .get("task_worktree")
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("memory_root"))
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("root"))
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .map(|value| expanduser_path(&value))
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                repo_root.join(path)
            }
        })
        .map(|path| resolve_path_strict_false(&path))
        .filter(|path| path.is_dir());
    let Some(memory_root) = configured_memory_root else {
        if dangling_link {
            return Err(format!(
                "Repository shared Cargo build link is dangling and its configured memory root is unavailable: {}",
                shared_path.display()
            ));
        }
        fs::create_dir_all(&shared_path).map_err(|err| err.to_string())?;
        return Ok(fs::canonicalize(&shared_path).unwrap_or(shared_path));
    };

    let repo_name = config
        .get("repo_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .or_else(|| {
            repo_root
                .file_name()
                .and_then(|value| value.to_str())
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| "repo".to_string());
    let physical_path = repository_ram_cargo_build_dir(&memory_root, &repo_name);
    if dangling_link {
        let link_target = fs::read_link(&shared_path).map_err(|err| {
            format!(
                "Failed to read repository shared Cargo build link {}: {err}",
                shared_path.display()
            )
        })?;
        let resolved_link_target = if link_target.is_absolute() {
            link_target
        } else {
            shared_path
                .parent()
                .unwrap_or_else(|| Path::new(""))
                .join(link_target)
        };
        if resolve_path_strict_false(&resolved_link_target)
            != resolve_path_strict_false(&physical_path)
        {
            return Err(format!(
                "Repository shared Cargo build link points at an unexpected target and cannot be recovered automatically: {} (expected {})",
                shared_path.display(),
                physical_path.display()
            ));
        }
        fs::create_dir_all(&physical_path).map_err(|err| {
            format!(
                "Failed to recreate repository shared Cargo build directory {}: {err}",
                physical_path.display()
            )
        })?;
        return Ok(fs::canonicalize(&shared_path).unwrap_or(physical_path));
    }
    fs::create_dir_all(&physical_path).map_err(|err| err.to_string())?;
    if let Some(parent) = shared_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    create_directory_link(&shared_path, &physical_path)?;
    Ok(physical_path)
}

pub(in crate::primitives) fn materialize_worktree_cargo_config(
    repo_root: &Path,
    target_path: &Path,
) -> Result<(), String> {
    materialize_worktree_cargo_config_for_workspace(repo_root, target_path, target_path)
}

pub(in crate::primitives) fn materialize_worktree_cargo_config_for_workspace(
    repo_root: &Path,
    target_path: &Path,
    projected_workspace_root: &Path,
) -> Result<(), String> {
    if !target_path.is_dir() || !path_exists_or_directory_link(&target_path.join(APP_DIR)) {
        return Ok(());
    }
    if !cargo_worktree_integration_enabled(repo_root, target_path) {
        return Ok(());
    }
    let config_path = target_path.join(WORKTREE_CARGO_CONFIG_RELATIVE_PATH);
    if path_exists_or_directory_link(&config_path) {
        let metadata = fs::symlink_metadata(&config_path).map_err(|err| err.to_string())?;
        if metadata.is_file() && !metadata.file_type().is_symlink() {
            let contents = fs::read_to_string(&config_path).map_err(|err| err.to_string())?;
            if let Some(upgraded) =
                upgrade_generated_worktree_cargo_config_text(projected_workspace_root, &contents)
                    .or_else(|| {
                        upgrade_copied_task_worktree_cargo_config_text(
                            projected_workspace_root,
                            &contents,
                        )
                    })
                    .or_else(|| {
                        upgrade_copied_main_seed_cargo_config_text(
                            projected_workspace_root,
                            &contents,
                        )
                    })
            {
                if upgraded != contents {
                    let mode = portable_mode(&metadata, 0o644);
                    if mode & 0o200 == 0 {
                        set_portable_mode(&config_path, mode | 0o200)
                            .map_err(|err| err.to_string())?;
                    }
                    fs::write(&config_path, upgraded).map_err(|err| err.to_string())?;
                }
            }
        }
        return Ok(());
    }
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    fs::write(
        &config_path,
        generated_worktree_cargo_config_text(projected_workspace_root),
    )
    .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn materialize_worktree_docs_symlink(
    repo_root: &Path,
    target_path: &Path,
) -> Result<(), String> {
    let source_path = repo_root.join("docs");
    if !source_path.is_dir() {
        return Ok(());
    }
    let link_path = target_path.join("docs");
    if path_exists_or_directory_link(&link_path) {
        let resolved = fs::read_link(&link_path).ok();
        if resolved.as_deref() == Some(source_path.as_path()) {
            return Ok(());
        }
        remove_path_entry(&link_path)?;
    }
    create_directory_link(&link_path, &source_path)
}

pub(in crate::primitives) fn materialize_worktree_runtime_layout(
    repo: &RepoRuntime,
    worktree_name: &str,
    target_path: &Path,
    line_name: &str,
    created_at: &str,
    extra_worktree_config: Option<&JsonValue>,
) -> Result<RepoRuntime, String> {
    let shared_ait_dir = repo.authoritative_repo_root().join(".ait");
    fs::create_dir_all(target_path).map_err(|err| err.to_string())?;
    let ait_link = target_path.join(APP_DIR);
    if path_exists_or_directory_link(&ait_link) {
        return Err(format!(
            "Worktree path already contains {APP_DIR}: {}",
            target_path.display()
        ));
    }
    create_directory_link(&ait_link, &shared_ait_dir)?;
    ensure_worktree_runtime_layout(&repo.authoritative_repo_root(), target_path)?;
    let worktree_config_path = target_path.join(WORKTREE_CONFIG_NAME);
    let mut config = json!({
        "worktree_name": worktree_name,
        "current_line": line_name,
        "repo_root": repo.authoritative_repo_root().to_string_lossy().to_string(),
        "workspace_root": target_path.to_string_lossy().to_string(),
        "created_at": created_at,
    });
    if let (Some(extra), Some(map)) = (extra_worktree_config, config.as_object_mut()) {
        if let Some(extra_map) = extra.as_object() {
            for (key, value) in extra_map {
                map.insert(key.clone(), value.clone());
            }
        }
    }
    write_json_pretty(&worktree_config_path, &config)?;
    materialize_worktree_docs_symlink(&repo.authoritative_repo_root(), target_path)?;
    materialize_worktree_cargo_config(&repo.authoritative_repo_root(), target_path)?;
    RepoRuntime::discover_from_path(target_path)
}

pub(in crate::primitives) fn task_start_snapshot_restore(
    worktree_repo: &RepoRuntime,
    target_snapshot_id: Option<&str>,
    baseline_snapshot_id: Option<&str>,
) -> Result<(), String> {
    let Some(snapshot_id) = normalized_text(target_snapshot_id) else {
        return Ok(());
    };
    restore_workspace_all(
        worktree_repo,
        Some(&snapshot_id),
        baseline_snapshot_id,
        true,
        false,
    )?;
    worktree_repo.set_worktree_materialized_snapshot(Some(&snapshot_id))
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments map directly to persisted worktree registration metadata"
)]
pub(in crate::primitives) fn write_worktree_registration(
    repo: &RepoRuntime,
    worktree_name: &str,
    worktree_path: &Path,
    alias_path: Option<&Path>,
    line_name: &str,
    created_at: &str,
    root_source: Option<&str>,
    task_id: &str,
    change_id: Option<&str>,
    head_snapshot_id: Option<&str>,
    fork_snapshot_id: Option<&str>,
    forked_from_line: Option<&str>,
    target_base_line: Option<&str>,
) -> Result<(), String> {
    let bound_change_id = change_id
        .map(|value| ChangeJson::stateless().canonical_change_id(value))
        .transpose()?;
    let bound_change_ref = bound_change_id
        .as_deref()
        .map(|value| ChangeJson::stateless().rolling_server_change_id(Some(task_id), value))
        .transpose()?;
    let metadata_path = worktree_registry_path(repo, worktree_name);
    if let Some(parent) = metadata_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    write_json_pretty(
        &metadata_path,
        &json!({
            "name": worktree_name,
            "path": worktree_path.to_string_lossy().to_string(),
            "alias_path": alias_path.map(|value| value.to_string_lossy().to_string()),
            "line_name": line_name,
            "repo_root": repo.authoritative_repo_root().to_string_lossy().to_string(),
            "created_at": created_at,
            "creation_kind": "task_auto_created",
            "cleanup_policy": "after_remote_land",
            "root_source": normalized_text(root_source),
            "last_used_at": created_at,
            "bound_task_id": task_id,
            "bound_change_id": bound_change_id,
            "bound_change_ref": bound_change_ref,
            "auto_created_for_task": true,
            "fork_snapshot_id": normalized_text(fork_snapshot_id),
            "forked_from_line": normalized_text(forked_from_line),
            "target_base_line": normalized_text(target_base_line),
            "workspace_status_cache": {
                "workspace_status": "clean",
                "clean": true,
                "changed_count": 0,
                "modified_paths": [],
                "missing_paths": [],
                "untracked_paths": [],
                "current_line": line_name,
                "head_snapshot_id": normalized_text(head_snapshot_id),
                "status_checked_at": created_at,
            },
        }),
    )?;
    Ok(())
}

pub(in crate::primitives) fn set_active_root_worktree_binding(
    repo: &RepoRuntime,
    worktree_name: &str,
) -> Result<(), String> {
    if repo.is_worktree() {
        return Ok(());
    }
    update_root_config(repo, |config| {
        config.insert(
            "worktree_name".to_string(),
            JsonValue::String(worktree_name.to_string()),
        );
    })
}

pub(in crate::primitives) fn task_start_bootstrap(
    repo: &RepoRuntime,
    request: TaskStartBootstrapRequest<'_>,
) -> Result<JsonValue, String> {
    task_start_bootstrap_with_progress(repo, request, None)
}

pub fn worktree_recover_task(
    repo: &RepoRuntime,
    task_id: &str,
    change_id: &str,
    remote_name: Option<&str>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    if repo.is_worktree() {
        return Err(format!(
            "Run `ait worktree recover-task` from the authoritative repository root `{}`.",
            repo.authoritative_repo_root().display()
        ));
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    worktree_recover_task_with_task_remote(
        repo,
        &mut task_remote,
        &remote_row.name,
        &repo_name,
        task_id,
        change_id,
        dry_run,
        None,
    )
}

fn cleanup_unregistered_task_worktree_materialization(
    repo: &RepoRuntime,
    worktree_name: &str,
    worktree_path: &Path,
    alias_path: Option<&Path>,
) -> Result<bool, String> {
    if worktree_registry_path(repo, worktree_name).exists() {
        return Ok(false);
    }
    if !path_exists_or_directory_link(worktree_path) {
        return Ok(false);
    }
    if !worktree_path.is_dir() {
        return Err(format!(
            "Refusing to remove unregistered task worktree path because it is not a directory: {}",
            worktree_path.display()
        ));
    }
    let metadata = read_json_value(&worktree_path.join(WORKTREE_CONFIG_NAME));
    let metadata_name = string_field(&metadata, "worktree_name");
    let metadata_repo_root = string_field(&metadata, "repo_root")
        .map(PathBuf::from)
        .map(|path| resolve_path_strict_false(&path));
    let metadata_workspace_root = string_field(&metadata, "workspace_root")
        .map(PathBuf::from)
        .map(|path| resolve_path_strict_false(&path));
    let expected_repo_root = resolve_path_strict_false(&repo.authoritative_repo_root());
    let expected_worktree_path = resolve_path_strict_false(worktree_path);
    if metadata_name.as_deref() != Some(worktree_name)
        || metadata_repo_root.as_deref() != Some(expected_repo_root.as_path())
        || metadata_workspace_root.as_deref() != Some(expected_worktree_path.as_path())
    {
        return Err(format!(
            "Refusing to remove unregistered task worktree path because its ownership metadata does not exactly match `{worktree_name}`: {}",
            worktree_path.display()
        ));
    }
    if let Some(alias_path) = alias_path {
        if path_exists_or_directory_link(alias_path) {
            let alias_target = fs::read_link(alias_path).map_err(|err| {
                format!(
                    "Refusing to remove task worktree alias that is not a readable link {}: {err}",
                    alias_path.display()
                )
            })?;
            if resolve_path_strict_false(&alias_target) != expected_worktree_path {
                return Err(format!(
                    "Refusing to remove task worktree alias because it targets `{}`, not `{}`.",
                    alias_target.display(),
                    worktree_path.display()
                ));
            }
            remove_path_entry(alias_path)?;
        }
    }
    remove_tree_force(worktree_path)?;
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn worktree_recover_task_with_task_remote<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    remote_name: &str,
    repo_name: &str,
    task_id: &str,
    change_id: &str,
    dry_run: bool,
    debug_probe_override: Option<&JsonValue>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteTaskReader + TaskWorkflowRemoteChangeReader + ?Sized,
{
    let task_id = normalized_text(Some(task_id))
        .ok_or_else(|| "Task id is required to recover a worktree.".to_string())?;
    let change_id = normalized_text(Some(change_id))
        .ok_or_else(|| "Change id is required to recover a worktree.".to_string())?;
    if let Some(existing) = bound_task_worktree_metadata(repo, Some(&task_id), Some(&change_id))? {
        return Err(format!(
            "Task `{task_id}` is already registered to worktree `{}`. Enter that worktree or run `ait worktree recreate {}` if its path is missing.",
            existing.name, existing.name
        ));
    }

    let task = task_remote
        .get_task(&task_id, Some(repo_name))
        .map_err(|err| err.to_string())?;
    let returned_task_id = required_string_field(&task, "task_id")?;
    if returned_task_id != task_id {
        return Err(format!(
            "Remote task lookup for `{task_id}` returned unrelated task `{returned_task_id}`."
        ));
    }
    let task_repo_name = required_string_field(&task, "repo_name")?;
    if task_repo_name != repo_name {
        return Err(format!(
            "Remote task `{task_id}` belongs to repository `{task_repo_name}`, not `{repo_name}`."
        ));
    }
    let task_status = required_string_field(&task, "status")?;
    if !matches!(task_status.as_str(), "active" | "draft") {
        return Err(format!(
            "Remote task `{task_id}` has status `{task_status}` and cannot recover an authoring worktree."
        ));
    }

    let change = task_remote
        .get_change(&change_id, Some(repo_name))
        .map_err(|err| err.to_string())?;
    let returned_change_id = required_string_field(&change, "change_id")?;
    let returned_change_ref = change_reference_from_payload(&change, Some(&change_id))?;
    let requested_canonical = canonical_change_id(&change_id)?;
    let returned_canonical = canonical_change_id(&returned_change_id)?;
    if returned_canonical != requested_canonical
        || (change_id != requested_canonical && returned_change_ref != change_id)
    {
        return Err(format!(
            "Remote change lookup for `{change_id}` returned unrelated change `{returned_change_ref}`."
        ));
    }
    let change_task_id = required_string_field(&change, "task_id")?;
    if change_task_id != task_id {
        return Err(format!(
            "Remote change `{change_id}` belongs to task `{change_task_id}`, not `{task_id}`."
        ));
    }
    let change_repo_name = required_string_field(&change, "repo_name")?;
    if change_repo_name != repo_name {
        return Err(format!(
            "Remote change `{change_id}` belongs to repository `{change_repo_name}`, not `{repo_name}`."
        ));
    }
    let change_status = required_string_field(&change, "status")?;
    if !matches!(change_status.as_str(), "draft" | "review") {
        return Err(format!(
            "Remote change `{change_id}` has status `{change_status}` and cannot recover an authoring worktree."
        ));
    }
    let base_line = required_string_field(&change, "base_line")?;
    if let Some(fork_snapshot_id) = string_field(&change, "fork_snapshot_id") {
        if !local_snapshot_exists(repo, &fork_snapshot_id)? {
            return Err(format!(
                "Remote change `{change_id}` forks from `{fork_snapshot_id}`, which is not available in the local Binary snapshot store. Pull the base line before recovering the task worktree."
            ));
        }
    }
    let worktree_name = resolve_next_task_worktree_name(repo, &task_id)?;
    let location = task_worktree_layout::resolve_task_worktree_location_with_debug(
        repo,
        &worktree_name,
        debug_probe_override,
    )?;
    let path = location.target_path.to_string_lossy().to_string();
    let alias_path = location
        .alias_path
        .as_ref()
        .map(|value| value.to_string_lossy().to_string());
    preflight_task_worktree_target(
        repo,
        &worktree_name,
        &location.target_path,
        location.alias_path.as_deref(),
    )?;
    if dry_run {
        return Ok(json!({
            "status": "recovery_planned",
            "task_id": task_id,
            "change_id": change_id,
            "remote_name": remote_name,
            "repo_name": repo_name,
            "name": worktree_name,
            "path": path,
            "alias_path": alias_path,
            "current_line": task_feature_line_name(&task_id)?,
            "base_line": base_line,
            "fork_snapshot_id": string_field(&change, "fork_snapshot_id"),
            "dry_run": true,
        }));
    }

    let bootstrap_result = task_start_bootstrap(
        repo,
        TaskStartBootstrapRequest {
            task: &task,
            change: Some(&change),
            base_line_name: &base_line,
            local: false,
            worktree_name: &worktree_name,
            worktree_path: &path,
            worktree_alias_path: alias_path.as_deref(),
            worktree_root_source: Some(&location.root_source),
            worktree_fallback_reason: location.fallback_reason.as_deref(),
            worktree_default_line: location.default_line.as_deref(),
            worktree_seed_snapshot_id: location.seed_snapshot_id.as_deref(),
            worktree_seed_snapshot_total_bytes: location.seed_snapshot_total_bytes,
            worktree_main_seed_ram_max_bytes: location.main_seed_ram_max_bytes,
        },
    );
    let mut payload = match bootstrap_result {
        Ok(payload) => payload,
        Err(error) => {
            return match cleanup_unregistered_task_worktree_materialization(
                repo,
                &worktree_name,
                &location.target_path,
                location.alias_path.as_deref(),
            ) {
                Ok(_) => Err(error),
                Err(cleanup_error) => Err(format!(
                    "{error} Partial task worktree cleanup also failed: {cleanup_error}"
                )),
            };
        }
    };
    payload
        .as_object_mut()
        .ok_or_else(|| "Recovered worktree payload must be an object.".to_string())?
        .extend(JsonMap::from_iter([
            (
                "status".to_string(),
                JsonValue::String("recovered".to_string()),
            ),
            ("task_id".to_string(), JsonValue::String(task_id)),
            ("change_id".to_string(), JsonValue::String(change_id)),
            (
                "remote_name".to_string(),
                JsonValue::String(remote_name.to_string()),
            ),
            (
                "repo_name".to_string(),
                JsonValue::String(repo_name.to_string()),
            ),
        ]));
    Ok(payload)
}

pub(in crate::primitives) fn emit_task_start_progress(
    progress: Option<&mut TaskStartProgressEmitter<'_>>,
    payload: JsonValue,
) -> Result<(), String> {
    let Some(progress) = progress else {
        return Ok(());
    };
    progress(&payload)
}

fn preflight_task_worktree_target(
    repo: &RepoRuntime,
    worktree_name: &str,
    worktree_path: &Path,
    alias_path: Option<&Path>,
) -> Result<(), String> {
    if path_exists_or_directory_link(worktree_path) {
        if !worktree_path.is_dir() {
            return Err(format!(
                "Worktree path exists and is not a directory: {}",
                worktree_path.display()
            ));
        }
        if fs::read_dir(worktree_path)
            .map_err(|err| err.to_string())?
            .next()
            .is_some()
        {
            return Err(format!(
                "Worktree path must be empty: {}",
                worktree_path.display()
            ));
        }
    }
    if let Some(alias_path) = alias_path {
        if alias_path == worktree_path {
            return Err(
                "Worktree alias path must differ from the canonical worktree path.".to_string(),
            );
        }
        if path_exists_or_directory_link(alias_path) {
            return Err(format!(
                "Worktree alias path is already in use: {}",
                alias_path.display()
            ));
        }
    }
    if worktree_registry_path(repo, worktree_name).exists() {
        return Err(format!("Worktree already exists: {worktree_name}"));
    }
    let registry_dir = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("worktrees");
    if registry_dir.is_dir() {
        for entry in fs::read_dir(&registry_dir).map_err(|err| err.to_string())? {
            let entry = entry.map_err(|err| err.to_string())?;
            let payload = read_json_document(&entry.path());
            let registered_path = payload
                .get("path")
                .and_then(JsonValue::as_str)
                .map(PathBuf::from);
            if registered_path.as_deref() == Some(worktree_path) {
                return Err(format!(
                    "Worktree path is already registered to {}: {}",
                    payload
                        .get("name")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("unknown"),
                    worktree_path.display()
                ));
            }
            if let (Some(existing_alias), Some(expected_alias)) = (
                payload
                    .get("alias_path")
                    .and_then(JsonValue::as_str)
                    .map(PathBuf::from),
                alias_path,
            ) {
                if existing_alias == expected_alias {
                    return Err(format!(
                        "Worktree alias path is already registered to {}: {}",
                        payload
                            .get("name")
                            .and_then(JsonValue::as_str)
                            .unwrap_or("unknown"),
                        expected_alias.display()
                    ));
                }
            }
        }
    }
    Ok(())
}

pub(in crate::primitives) fn task_start_bootstrap_with_progress(
    repo: &RepoRuntime,
    request: TaskStartBootstrapRequest<'_>,
    mut progress: Option<&mut TaskStartProgressEmitter<'_>>,
) -> Result<JsonValue, String> {
    let task_id = required_string_field(request.task, "task_id")?;
    let (resolved_base_line_name, resolved_fork_snapshot_id) =
        task_start_change_bootstrap_lineage(request.change, request.base_line_name)?;
    let fallback_to_local_base_line = request.local || request.change.is_none();
    let worktree_name = normalized_text(Some(request.worktree_name))
        .ok_or_else(|| "Task worktree name is required.".to_string())?;
    let worktree_path = PathBuf::from(request.worktree_path);
    let alias_path = request.worktree_alias_path.map(PathBuf::from);
    preflight_task_worktree_target(repo, &worktree_name, &worktree_path, alias_path.as_deref())?;
    let feature_line = ensure_task_feature_line(
        repo,
        &task_id,
        &resolved_base_line_name,
        resolved_fork_snapshot_id.as_deref(),
        fallback_to_local_base_line,
    )
    .map_err(|err| format!("failed to ensure the task feature line: {err}"))?;
    let feature_line_name = required_string_field(&feature_line, "line_name")?;
    let feature_line_head_snapshot_id = string_field(&feature_line, "head_snapshot_id");
    let created_at = system_event_timestamp();

    let local_base_line_head_snapshot_id = if fallback_to_local_base_line {
        let line = local_line_row(repo, &resolved_base_line_name)
            .map_err(|err| format!("failed to read the base line from local snapshots: {err}"))?;
        string_field(&line, "head_snapshot_id")
    } else {
        None
    };
    let change_id = request
        .change
        .and_then(|change| string_field(change, "change_id"));
    let bootstrap_fork_snapshot_id = resolved_fork_snapshot_id
        .clone()
        .or(local_base_line_head_snapshot_id);
    let default_line_name = repo.default_line_name();
    let default_line_row = local_line_row(repo, &default_line_name).ok();
    let default_line_snapshot_id = default_line_row
        .as_ref()
        .and_then(|row| string_field(row, "head_snapshot_id"));
    let seed_candidate = request
        .worktree_root_source
        .and_then(|root_source| normalized_text(Some(root_source)))
        .filter(|root_source| root_source != "repo_internal_fallback")
        .filter(|_| resolved_base_line_name == default_line_name)
        .filter(|_| feature_line_head_snapshot_id.is_some())
        .filter(|_| feature_line_head_snapshot_id == default_line_snapshot_id)
        .and_then(|root_source| {
            ensure_main_seed_mirror_for_snapshot(
                repo,
                &default_line_name,
                feature_line_head_snapshot_id.as_deref().unwrap_or_default(),
                Some(&root_source),
            )
        });

    let mut materialization_source = "snapshot_restore".to_string();
    let mut copy_strategy: Option<String> = None;
    let mut main_seed_payload: Option<JsonValue> = None;
    if let Some(seed_result) = seed_candidate.as_ref() {
        if seed_result.status == "refreshed" {
            emit_task_start_progress(
                progress.as_deref_mut(),
                json!({
                    "phase": "aligning_main_seed",
                    "line_name": default_line_name,
                    "seed_snapshot_id": feature_line_head_snapshot_id,
                }),
            )?;
        }
    }
    let progress_open_path = alias_path
        .as_ref()
        .unwrap_or(&worktree_path)
        .to_string_lossy()
        .to_string();
    let progress_source = if seed_candidate
        .as_ref()
        .is_some_and(|seed_result| matches!(seed_result.status.as_str(), "aligned" | "refreshed"))
    {
        "main_seed_mirror"
    } else {
        "snapshot_restore"
    };
    emit_task_start_progress(
        progress,
        json!({
            "phase": "materializing_worktree",
            "worktree_name": worktree_name,
            "open_path": progress_open_path,
            "source": progress_source,
        }),
    )?;
    let worktree_repo = if let Some(seed_result) = seed_candidate.clone() {
        if matches!(seed_result.status.as_str(), "aligned" | "refreshed") {
            let copy_result = copy_seed_tree(
                &seed_result.path,
                &worktree_path,
                MAIN_SEED_COPY_EXCLUDE_NAMES,
            );
            match copy_result {
                Ok(strategy) => {
                    copy_strategy = Some(strategy);
                    set_tree_writeable(&worktree_path)
                        .map_err(|err| format!("failed to make copied worktree writable: {err}"))?;
                    materialization_source = "main_seed_mirror".to_string();
                    main_seed_payload = Some(seed_result.to_json());
                    materialize_worktree_runtime_layout(
                        repo,
                        &worktree_name,
                        &worktree_path,
                        &feature_line_name,
                        &created_at,
                        None,
                    )
                    .map_err(|err| {
                        format!("failed to materialize the task worktree runtime layout: {err}")
                    })?
                }
                Err(err) => {
                    if path_exists_or_directory_link(&worktree_path) {
                        remove_tree_force(&worktree_path).map_err(|cleanup_err| {
                            format!(
                                "failed to clean partial copied task worktree after main-seed fallback: {cleanup_err}"
                            )
                        })?;
                    }
                    materialization_source = "snapshot_restore".to_string();
                    let mut fallback_payload = seed_result
                        .to_json()
                        .as_object()
                        .cloned()
                        .unwrap_or_default();
                    fallback_payload.insert(
                        "status".to_string(),
                        JsonValue::String("failed".to_string()),
                    );
                    fallback_payload.insert("fallback_used".to_string(), JsonValue::Bool(true));
                    fallback_payload.insert("fallback_reason".to_string(), JsonValue::String(err));
                    main_seed_payload = Some(JsonValue::Object(fallback_payload));
                    let worktree_repo = materialize_worktree_runtime_layout(
                        repo,
                        &worktree_name,
                        &worktree_path,
                        &feature_line_name,
                        &created_at,
                        None,
                    )
                    .map_err(|err| {
                        format!("failed to materialize the task worktree runtime layout: {err}")
                    })?;
                    task_start_snapshot_restore(
                        &worktree_repo,
                        feature_line_head_snapshot_id.as_deref(),
                        None,
                    )
                    .map_err(|err| {
                        format!("failed to restore the task worktree snapshot state: {err}")
                    })?;
                    worktree_repo
                }
            }
        } else {
            main_seed_payload = Some(seed_result.to_json());
            let worktree_repo = materialize_worktree_runtime_layout(
                repo,
                &worktree_name,
                &worktree_path,
                &feature_line_name,
                &created_at,
                None,
            )
            .map_err(|err| {
                format!("failed to materialize the task worktree runtime layout: {err}")
            })?;
            task_start_snapshot_restore(
                &worktree_repo,
                feature_line_head_snapshot_id.as_deref(),
                None,
            )
            .map_err(|err| format!("failed to restore the task worktree snapshot state: {err}"))?;
            worktree_repo
        }
    } else {
        let worktree_repo = materialize_worktree_runtime_layout(
            repo,
            &worktree_name,
            &worktree_path,
            &feature_line_name,
            &created_at,
            None,
        )
        .map_err(|err| format!("failed to materialize the task worktree runtime layout: {err}"))?;
        task_start_snapshot_restore(
            &worktree_repo,
            feature_line_head_snapshot_id.as_deref(),
            None,
        )
        .map_err(|err| format!("failed to restore the task worktree snapshot state: {err}"))?;
        worktree_repo
    };
    if materialization_source == "main_seed_mirror" {
        worktree_repo
            .set_worktree_materialized_snapshot(feature_line_head_snapshot_id.as_deref())
            .map_err(|err| {
                format!("failed to mark the copied task worktree materialized snapshot: {err}")
            })?;
    }
    materialize_worktree_cargo_config(&repo.authoritative_repo_root(), &worktree_path).map_err(
        |err| format!("failed to finalize the task worktree Cargo configuration: {err}"),
    )?;
    if let Some(alias_path) = alias_path.as_ref() {
        if let Some(parent) = alias_path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "failed to create the task worktree alias parent {}: {err}",
                    parent.display()
                )
            })?;
        }
        create_directory_link(alias_path, &worktree_path)
            .map_err(|err| format!("failed to create the task worktree alias link: {err}"))?;
    }
    write_worktree_registration(
        repo,
        &worktree_name,
        &worktree_path,
        alias_path.as_deref(),
        &feature_line_name,
        &created_at,
        request.worktree_root_source,
        &task_id,
        change_id.as_deref(),
        feature_line_head_snapshot_id.as_deref(),
        bootstrap_fork_snapshot_id.as_deref(),
        Some(&resolved_base_line_name),
        Some(&resolved_base_line_name),
    )
    .map_err(|err| format!("failed to register the task worktree metadata: {err}"))?;
    set_active_root_worktree_binding(repo, &worktree_name)
        .map_err(|err| format!("failed to bind the active root worktree: {err}"))?;
    let mut bootstrap = JsonMap::new();
    bootstrap.insert(
        "worktree".to_string(),
        worktree_summary_payload(
            &worktree_name,
            &worktree_path,
            alias_path.as_deref(),
            &repo.authoritative_repo_root(),
            &feature_line_name,
            feature_line_head_snapshot_id.as_deref(),
            &task_id,
            change_id.as_deref(),
            request.worktree_root_source,
            request.worktree_fallback_reason,
            request.worktree_default_line,
            request.worktree_seed_snapshot_id,
            request.worktree_seed_snapshot_total_bytes,
            request.worktree_main_seed_ram_max_bytes,
            Some(&materialization_source),
            copy_strategy.as_deref(),
            main_seed_payload.as_ref(),
            bootstrap_fork_snapshot_id.as_deref(),
            Some(&resolved_base_line_name),
            Some(&resolved_base_line_name),
        )?,
    );

    Ok(JsonValue::Object(bootstrap))
}

pub(in crate::primitives) fn task_start_remote_base_line_preflight_with_task_remote<R>(
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
    task_remote: &mut R,
    repo_name: &str,
    base_line: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader + TaskWorkflowRepositoryReader + TaskWorkflowZstdPackReader + ?Sized,
{
    let line_row = task_remote
        .get_line(repo_name, base_line)
        .map_err(|err| err.to_string())?;
    let remote_head = string_field(&line_row, "head_snapshot_id");
    if let Some(remote_snapshot_id) = remote_head.as_deref() {
        if !remote_sync_snapshot_content_complete_for_repo(repo, remote_snapshot_id)? {
            let remote_repository = read_remote_repository_authority(repo, task_remote, repo_name)?;
            let remote_sync_capabilities =
                RemoteSyncCapabilities::from_server_payload(Some(&remote_repository));
            hydrate_remote_snapshot_chain_with_task_remote_and_capabilities(
                repo,
                task_remote,
                &remote_row.name,
                repo_name,
                remote_snapshot_id,
                &remote_sync_capabilities,
            )
            .map_err(|err| {
                format!(
                    "Cannot start a remote task: failed to import Remote `{}` line `{base_line}` head `{remote_snapshot_id}` without moving the local Line: {err}",
                    remote_row.name,
                )
            })?;
            if !remote_sync_snapshot_content_complete_for_repo(repo, remote_snapshot_id)? {
                return Err(format!(
                    "Cannot start a remote task: Remote `{}` line `{base_line}` head `{remote_snapshot_id}` remained incomplete after import.",
                    remote_row.name,
                ));
            }
        }
    }
    Ok(line_row)
}

fn create_detached_empty_remote_base_snapshot(
    repo: &RepoRuntime,
    base_line: &str,
) -> Result<JsonValue, String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|err| format!("System clock cannot create empty-base staging identity: {err}"))?
        .as_nanos();
    let staging_parent = repo
        .authoritative_repo_root()
        .join(APP_DIR)
        .join("runtime")
        .join("empty-base-snapshot-workspaces");
    if let Ok(metadata) = fs::symlink_metadata(&staging_parent) {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "Empty-base staging parent is not a physical directory: {}",
                staging_parent.display()
            ));
        }
    }
    fs::create_dir_all(&staging_parent).map_err(|err| {
        format!(
            "Failed to create empty-base staging parent {}: {err}",
            staging_parent.display()
        )
    })?;
    let staging_root = staging_parent.join(format!("{}-{nonce}", std::process::id()));
    fs::create_dir(&staging_root).map_err(|err| {
        format!(
            "Failed to create empty-base staging workspace {}: {err}",
            staging_root.display()
        )
    })?;
    let created = repo
        .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&staging_root)
        .and_then(|store| {
            store.create_detached_empty_snapshot(
                &repo.repo_name(),
                base_line,
                Some("Initialize empty remote base"),
            )
        });
    let cleanup = remove_tree_force(&staging_root);
    match (created, cleanup) {
        (Ok(snapshot), Ok(())) => Ok(snapshot),
        (Err(error), Ok(())) => Err(error),
        (Ok(_), Err(cleanup_error)) => Err(format!(
            "Created an empty-base Snapshot but failed to clean staging workspace {}: {cleanup_error}",
            staging_root.display()
        )),
        (Err(error), Err(cleanup_error)) => Err(format!(
            "{error} Staging cleanup also failed for {}: {cleanup_error}",
            staging_root.display()
        )),
    }
}

pub(in crate::primitives) fn ensure_remote_base_line_snapshot_with_task_remote<R>(
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
    task_remote: &mut R,
    repo_name: &str,
    base_line: &str,
    preflight_line_row: &JsonValue,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader
        + TaskWorkflowRepositoryReader
        + TaskWorkflowZstdPackReader
        + TaskWorkflowZstdPackUploader
        + ?Sized,
{
    if let Some(head_snapshot_id) = string_field(preflight_line_row, "head_snapshot_id") {
        return Ok(json!({
            "line": preflight_line_row,
            "initialized": false,
            "head_snapshot_id": head_snapshot_id,
            "reason": "remote_base_already_initialized",
        }));
    }
    let snapshot = create_detached_empty_remote_base_snapshot(repo, base_line)?;
    let snapshot_id = required_string_field(&snapshot, "snapshot_id")?;
    let remote_repository = read_remote_repository_authority(repo, task_remote, repo_name)?;
    let remote_sync_capabilities =
        RemoteSyncCapabilities::from_server_payload(Some(&remote_repository));
    let initialization =
        super::super::remote_sync::initialize_remote_null_head_line_with_snapshot_via_zstd(
            repo,
            task_remote,
            repo_name,
            base_line,
            &snapshot_id,
            &remote_sync_capabilities,
        );
    match initialization {
        Ok(snapshot_sync) => {
            let line = task_remote
                .get_line(repo_name, base_line)
                .map_err(|err| err.to_string())?;
            if string_field(&line, "head_snapshot_id").as_deref() != Some(snapshot_id.as_str()) {
                return Err(format!(
                    "Remote empty-base initialization returned an unexpected `{base_line}` head."
                ));
            }
            Ok(json!({
                "line": line,
                "initialized": true,
                "head_snapshot_id": snapshot_id,
                "snapshot": snapshot,
                "snapshot_sync": snapshot_sync,
                "remote_repository": remote_repository,
                "reason": "remote_null_head_initialized",
            }))
        }
        Err(initialization_error) => {
            let winner = task_start_remote_base_line_preflight_with_task_remote(
                repo,
                remote_row,
                task_remote,
                repo_name,
                base_line,
            )?;
            let Some(winner_snapshot_id) = string_field(&winner, "head_snapshot_id") else {
                return Err(format!(
                    "Remote empty-base initialization failed and `{base_line}` still has no head: {initialization_error}"
                ));
            };
            if winner_snapshot_id == snapshot_id {
                return Ok(json!({
                    "line": winner,
                    "initialized": true,
                    "head_snapshot_id": winner_snapshot_id,
                    "snapshot": snapshot,
                    "initialization_error": initialization_error,
                    "remote_repository": remote_repository,
                    "reason": "remote_null_head_initialized_after_uncertain_response",
                }));
            }
            Ok(json!({
                "line": winner,
                "initialized": false,
                "head_snapshot_id": winner_snapshot_id,
                "orphaned_snapshot_id": snapshot_id,
                "initialization_error": initialization_error,
                "remote_repository": remote_repository,
                "reason": "remote_null_head_initialized_by_peer",
            }))
        }
    }
}

fn task_start_remote_initial_change_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    task_id: &str,
    title: &str,
    base_line: &str,
    preflight_line_row: &JsonValue,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeCreator + TaskWorkflowLineagePayloadBuilder + ?Sized,
{
    let lineage_payload =
        task_remote.change_lineage_payload(base_line, Some(preflight_line_row))?;
    change_create_with_task_remote(
        task_remote,
        repo_name,
        task_id,
        title,
        base_line,
        None,
        &lineage_payload,
    )
}

pub(in crate::primitives) fn task_start_root_preflight(repo: &RepoRuntime) -> Result<(), String> {
    if repo.is_worktree() {
        return Err(format!(
            "Refusing to run `ait task start` inside existing worktree `{}`. Start new task lineage from the authoritative repository root `{}` so all deterministic guards run before remote task creation.",
            repo.workspace_root().display(),
            repo.authoritative_repo_root().display(),
        ));
    }
    Ok(())
}

pub(in crate::primitives) fn task_start_context_preflight(
    repo: &RepoRuntime,
) -> Result<(), String> {
    task_start_root_preflight(repo)?;
    guard_no_planning_only_artifact_drift(repo, "ait task start")
}

pub(in crate::primitives) fn worktree_remote_change_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    change_ref: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeReader + ?Sized,
{
    task_remote
        .get_change(change_ref, Some(repo_name))
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn worktree_remote_patchset_read_with_closeout_remote<R>(
    closeout_remote: &mut R,
    repo_name: &str,
    patchset_id: &str,
    change_ref: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + ?Sized,
{
    closeout_remote
        .get_patchset(patchset_id, Some(repo_name), Some(change_ref))
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn worktree_remote_patchset_revision_candidate_with_remotes<T, C>(
    repo: &RepoRuntime,
    task_remote: &mut T,
    closeout_remote: &mut C,
    repo_name: &str,
    change_ref: &str,
) -> Result<Option<JsonValue>, String>
where
    T: TaskWorkflowRemoteChangeReader + ?Sized,
    C: TaskWorkflowPatchsetReader + ?Sized,
{
    let remote_change =
        worktree_remote_change_read_with_task_remote(task_remote, repo_name, change_ref)?;
    let Some(patchset_id) = string_field(&remote_change, "selected_patchset_id")
        .or_else(|| string_field(&remote_change, "current_patchset_id"))
    else {
        return Ok(None);
    };
    let patchset = worktree_remote_patchset_read_with_closeout_remote(
        closeout_remote,
        repo_name,
        &patchset_id,
        change_ref,
    )?;
    let Some(snapshot_id) = string_field(&patchset, "revision_snapshot_id") else {
        return Ok(None);
    };
    if !local_snapshot_exists(repo, &snapshot_id)? {
        return Ok(None);
    }
    Ok(Some(json!({
        "source": "remote_patchset_revision",
        "snapshot_id": snapshot_id,
        "available_locally": true,
        "change_id": string_field(&remote_change, "change_id").unwrap_or_else(|| change_ref.to_string()),
        "patchset_id": patchset_id,
    })))
}

pub fn task_start(
    repo: &RepoRuntime,
    title: &str,
    intent: &str,
    local: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    task_start_with_progress(
        repo,
        title,
        intent,
        local,
        remote_name,
        None,
        None,
        None,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn task_start_with_progress(
    repo: &RepoRuntime,
    title: &str,
    intent: &str,
    local: bool,
    remote_name: Option<&str>,
    plan_id: Option<&str>,
    plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
    debug_probe_override: Option<&JsonValue>,
    mut progress: Option<&mut TaskStartProgressEmitter<'_>>,
) -> Result<JsonValue, String> {
    let total_started = Instant::now();
    let context_preflight_started = Instant::now();
    validate_task_start_plan_binding_mode(repo, plan_id, plan_revision_id, plan_item_ref)?;

    let resolved_title =
        normalized_text(Some(title)).ok_or_else(|| "Task title must not be empty.".to_string())?;
    let resolved_intent = normalized_text(Some(intent))
        .ok_or_else(|| "Task intent must not be empty.".to_string())?;
    let resolved_base_line = "main".to_string();
    let resolved_change_title = resolved_title.clone();
    task_start_context_preflight(repo)?;
    let context_preflight_elapsed = elapsed_ms(context_preflight_started);
    let use_local = repo.task_uses_local_scope(local, remote_name)?;
    let remote_base_line_preflight_started = Instant::now();
    let remote_base_line_context = if !use_local {
        let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
        let mut task_remote = http_task_remote(repo, &remote_row)?;
        let line_row = task_start_remote_base_line_preflight_with_task_remote(
            repo,
            &remote_row,
            &mut task_remote,
            &repo_name,
            &resolved_base_line,
        )?;
        let initialized = ensure_remote_base_line_snapshot_with_task_remote(
            repo,
            &remote_row,
            &mut task_remote,
            &repo_name,
            &resolved_base_line,
            &line_row,
        )?;
        let line_row = initialized.get("line").cloned().ok_or_else(|| {
            "Remote base-Line initialization response is missing `line`.".to_string()
        })?;
        Some((remote_row, repo_name, line_row))
    } else {
        None
    };
    let remote_base_line_preflight_elapsed = elapsed_ms(remote_base_line_preflight_started);

    let task_create_started = Instant::now();
    let task = task_create(
        repo,
        &resolved_title,
        &resolved_intent,
        local,
        remote_name,
        plan_id,
        plan_revision_id,
        plan_item_ref,
    )?;
    let task_create_elapsed = elapsed_ms(task_create_started);
    let task_id = required_string_field(&task, "task_id")?;
    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "task_created",
            "task_id": task_id,
        }),
    )?;
    let change_create_started = Instant::now();
    let recovery_scope = if local {
        " --local".to_string()
    } else if let Some(remote_name) = normalized_text(remote_name) {
        format!(" --remote {}", shell_escape(Path::new(&remote_name)))
    } else {
        String::new()
    };
    let recovery_command = format!(
        "ait change create {} --title {} --base-line {}{}",
        shell_escape(Path::new(&task_id)),
        shell_escape(Path::new(&resolved_change_title)),
        shell_escape(Path::new(&resolved_base_line)),
        recovery_scope,
    );
    let change = if use_local {
        crate::primitives::change_flow::change_create_for_worktree_bootstrap(
            repo,
            &task_id,
            &resolved_change_title,
            Some(&resolved_base_line),
            local,
            remote_name,
        )
    } else {
        let (remote_row, repo_name, line_row) =
            remote_base_line_context.as_ref().ok_or_else(|| {
                "Remote Task initial Change is missing its validated main-Line context."
                    .to_string()
            })?;
        let mut task_remote = http_task_remote(repo, remote_row)?;
        task_start_remote_initial_change_with_task_remote(
            &mut task_remote,
            repo_name,
            &task_id,
            &resolved_change_title,
            &resolved_base_line,
            line_row,
        )
    }
    .map_err(|err| {
        format!(
            "Task {task_id} was created, but the initial change could not be created: {err}. Recover without creating another task by running `{recovery_command}`."
        )
    })?;
    let change_create_elapsed = elapsed_ms(change_create_started);
    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "change_created",
            "change_id": string_field(&change, "change_id"),
        }),
    )?;

    let mut payload = task_start_bootstrap_created_records_with_progress(
        repo,
        task,
        Some(change),
        &resolved_base_line,
        use_local,
        debug_probe_override,
        progress,
    )?;
    let object = payload
        .as_object_mut()
        .ok_or_else(|| "task start payload must decode to an object.".to_string())?;
    let bootstrap_timings = object
        .remove("phase_timings_ms")
        .unwrap_or_else(|| json!({}));
    object.insert(
        "phase_timings_ms".to_string(),
        json!({
            "context_preflight": context_preflight_elapsed,
            "remote_base_line_preflight": remote_base_line_preflight_elapsed,
            "task_create": task_create_elapsed,
            "change_create": change_create_elapsed,
            "worktree_location": bootstrap_timings.get("worktree_location").cloned().unwrap_or(JsonValue::Null),
            "worktree_bootstrap": bootstrap_timings.get("worktree_bootstrap").cloned().unwrap_or(JsonValue::Null),
            "total": elapsed_ms(total_started),
        }),
    );
    Ok(payload)
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn task_start_bootstrap_created_records_with_progress(
    repo: &RepoRuntime,
    task: JsonValue,
    change: Option<JsonValue>,
    resolved_base_line: &str,
    use_local: bool,
    debug_probe_override: Option<&JsonValue>,
    mut progress: Option<&mut TaskStartProgressEmitter<'_>>,
) -> Result<JsonValue, String> {
    let total_started = Instant::now();
    let task_id = required_string_field(&task, "task_id")?;
    let change_id = change
        .as_ref()
        .and_then(|row| string_field(row, "change_ref").or_else(|| string_field(row, "change_id")));
    if let Some(existing) =
        bound_task_worktree_metadata(repo, Some(&task_id), change_id.as_deref())?
    {
        let worktree = worktree_get(repo, Some(&existing.name), false)?;
        emit_task_start_progress(
            progress,
            json!({
                "phase": "worktree_ready",
                "worktree_name": worktree.get("name").and_then(JsonValue::as_str),
                "open_path": worktree
                    .get("open_path")
                    .and_then(JsonValue::as_str)
                    .or_else(|| worktree.get("path").and_then(JsonValue::as_str)),
                "reused": true,
            }),
        )?;
        let mut payload = task
            .as_object()
            .cloned()
            .ok_or_else(|| "task start payload must decode to an object.".to_string())?;
        if let Some(change_row) = change {
            payload.insert("change".to_string(), change_row);
        }
        payload.insert("worktree".to_string(), worktree);
        payload.insert("worktree_reused".to_string(), JsonValue::Bool(true));
        payload.insert(
            "phase_timings_ms".to_string(),
            json!({
                "worktree_location": 0.0,
                "worktree_bootstrap": 0.0,
                "total": elapsed_ms(total_started),
            }),
        );
        return Ok(JsonValue::Object(payload));
    }
    let worktree_location_started = Instant::now();
    let worktree_name = resolve_next_task_worktree_name(repo, &task_id)?;
    let worktree_location = task_worktree_layout::resolve_task_worktree_location_with_debug(
        repo,
        &worktree_name,
        debug_probe_override,
    )?;
    let worktree_location_elapsed = elapsed_ms(worktree_location_started);
    let worktree_path_text = worktree_location.target_path.to_string_lossy().to_string();
    let worktree_alias_path_text = worktree_location
        .alias_path
        .as_ref()
        .map(|value| value.to_string_lossy().to_string());
    emit_task_start_progress(
        progress.as_deref_mut(),
        json!({
            "phase": "worktree_bootstrap_started",
            "worktree_name": worktree_name,
            "open_path": worktree_alias_path_text
                .clone()
                .unwrap_or_else(|| worktree_path_text.clone()),
        }),
    )?;
    let worktree_bootstrap_started = Instant::now();
    let bootstrap = task_start_bootstrap_with_progress(
        repo,
        TaskStartBootstrapRequest {
            task: &task,
            change: change.as_ref(),
            base_line_name: resolved_base_line,
            local: use_local,
            worktree_name: &worktree_name,
            worktree_path: &worktree_path_text,
            worktree_alias_path: worktree_alias_path_text.as_deref(),
            worktree_root_source: Some(&worktree_location.root_source),
            worktree_fallback_reason: worktree_location.fallback_reason.as_deref(),
            worktree_default_line: worktree_location.default_line.as_deref(),
            worktree_seed_snapshot_id: worktree_location.seed_snapshot_id.as_deref(),
            worktree_seed_snapshot_total_bytes: worktree_location.seed_snapshot_total_bytes,
            worktree_main_seed_ram_max_bytes: worktree_location.main_seed_ram_max_bytes,
        },
        progress.as_deref_mut(),
    )
    .map_err(|err| match change.as_ref() {
        Some(change_row) => format!(
            "Task {} and change {} were created, but the bound worktree could not be created: {err}",
            task_id,
            string_field(change_row, "change_id").unwrap_or_else(|| "unknown".to_string()),
        ),
        None => format!(
            "Task {task_id} was created, but the bound worktree could not be created: {err}"
        ),
    })?;
    let worktree_bootstrap_elapsed = elapsed_ms(worktree_bootstrap_started);
    if let Some(worktree) = bootstrap.get("worktree").and_then(JsonValue::as_object) {
        emit_task_start_progress(
            progress,
            json!({
                "phase": "worktree_ready",
                "worktree_name": worktree.get("name").and_then(JsonValue::as_str),
                "open_path": worktree
                    .get("open_path")
                    .and_then(JsonValue::as_str)
                    .or_else(|| worktree.get("path").and_then(JsonValue::as_str)),
            }),
        )?;
    }

    let mut payload = task
        .as_object()
        .cloned()
        .ok_or_else(|| "task start payload must decode to an object.".to_string())?;
    if let Some(change_row) = change {
        payload.insert("change".to_string(), change_row);
    }
    match bootstrap {
        JsonValue::Object(map) => payload.extend(map),
        _ => {
            return Err("task start bootstrap payload must decode to an object.".to_string());
        }
    }
    payload.insert(
        "phase_timings_ms".to_string(),
        json!({
            "worktree_location": worktree_location_elapsed,
            "worktree_bootstrap": worktree_bootstrap_elapsed,
            "total": elapsed_ms(total_started),
        }),
    );
    Ok(JsonValue::Object(payload))
}

fn validate_task_start_plan_binding_mode(
    repo: &RepoRuntime,
    plan_id: Option<&str>,
    plan_revision_id: Option<&str>,
    plan_item_ref: Option<&str>,
) -> Result<(), String> {
    let provided = [plan_id, plan_revision_id, plan_item_ref]
        .into_iter()
        .filter(|value| normalized_text(*value).is_some())
        .count();
    if provided != 0 && provided != 3 {
        return Err(
            "Internal task bootstrap requires a complete canonical Plan ID, revision ID, and item ref. Public sprint work must use `ait task start --from <markdown-path>#<item-ref> --intent <intent>`."
                .to_string(),
        );
    }
    if repo.sprint_enabled() {
        if provided == 0 {
            return Err(
                "`ait task start --title` is unavailable while sprint mode is on. Use `ait task start --from <markdown-path>#<item-ref> --intent <intent>`; `--from` owns Plan sync, validation, and canonical binding."
                    .to_string(),
            );
        }
    } else if provided != 0 {
        return Err(
            "Plan-bound task bootstrap is unavailable while sprint mode is off. Use `ait task start --title <title> --intent <intent>` for unbound work, or enable sprint mode first."
                .to_string(),
        );
    }
    Ok(())
}

pub fn task_resolve_worktree_location(
    repo: &RepoRuntime,
    worktree_name: &str,
    debug_probe_override: Option<&JsonValue>,
) -> Result<JsonValue, String> {
    Ok(
        task_worktree_layout::resolve_task_worktree_location_with_debug(
            repo,
            worktree_name,
            debug_probe_override,
        )?
        .to_json(),
    )
}

#[cfg(test)]
mod recovery_cleanup_tests {
    use super::*;
    use tempfile::TempDir;

    fn recovery_cleanup_fixture() -> (TempDir, RepoRuntime, PathBuf, PathBuf) {
        let temp = TempDir::new().expect("fixture tempdir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(repo_root.join(".ait")).expect("create repo .ait");
        fs::write(
            repo_root.join(".ait/config.json"),
            r#"{"repo_name":"fixture-ait","default_line":"main"}"#,
        )
        .expect("write repo config");
        let repo = RepoRuntime::discover_from_path(&repo_root).expect("repo runtime");
        let worktree_path = temp.path().join("managed/rct-recover");
        fs::create_dir_all(&worktree_path).expect("create partial worktree");
        write_json_pretty(
            &worktree_path.join(WORKTREE_CONFIG_NAME),
            &json!({
                "worktree_name": "rct-recover",
                "repo_root": repo_root.to_string_lossy().to_string(),
                "workspace_root": worktree_path.to_string_lossy().to_string(),
                "current_line": "feature/rct-recover"
            }),
        )
        .expect("write ownership metadata");
        fs::write(worktree_path.join("partial.txt"), "partial\n")
            .expect("write partial materialization");
        let alias_path = temp.path().join("aliases/rct-recover");
        fs::create_dir_all(alias_path.parent().expect("alias parent"))
            .expect("create alias parent");
        create_directory_link(&alias_path, &worktree_path).expect("create worktree alias");
        (temp, repo, worktree_path, alias_path)
    }

    #[test]
    fn recovery_cleanup_removes_only_exactly_owned_unregistered_materialization() {
        let (_temp, repo, worktree_path, alias_path) = recovery_cleanup_fixture();

        assert!(cleanup_unregistered_task_worktree_materialization(
            &repo,
            "rct-recover",
            &worktree_path,
            Some(&alias_path),
        )
        .expect("clean exact orphan"));
        assert!(!worktree_path.exists());
        assert!(!path_exists_or_directory_link(&alias_path));
    }

    #[test]
    fn recovery_cleanup_preserves_unmatched_unregistered_directory() {
        let (_temp, repo, worktree_path, alias_path) = recovery_cleanup_fixture();
        let mut metadata = read_json_object_value(&worktree_path.join(WORKTREE_CONFIG_NAME));
        metadata.insert(
            "worktree_name".to_string(),
            JsonValue::String("someone-else".to_string()),
        );
        write_json_pretty(
            &worktree_path.join(WORKTREE_CONFIG_NAME),
            &JsonValue::Object(metadata),
        )
        .expect("forge unmatched ownership metadata");

        let error = cleanup_unregistered_task_worktree_materialization(
            &repo,
            "rct-recover",
            &worktree_path,
            Some(&alias_path),
        )
        .expect_err("unmatched directory must be preserved");
        assert!(error.contains("ownership metadata does not exactly match"));
        assert!(worktree_path.exists());
        assert!(path_exists_or_directory_link(&alias_path));
    }
}
