use super::*;

pub fn worktree_get(
    repo: &RepoRuntime,
    name: Option<&str>,
    refresh_status: bool,
) -> Result<JsonValue, String> {
    let worktree_name = resolve_runtime_worktree_name(repo, name)?;
    let metadata = load_worktree_metadata(repo, &worktree_name)?;
    worktree_summary_from_metadata(repo, &metadata, refresh_status, refresh_status)
}

pub(in crate::primitives) fn resolve_worktree_content_repo(
    repo: &RepoRuntime,
    name: Option<&str>,
) -> Result<RepoRuntime, String> {
    let Some(requested_name) = normalized_text(name) else {
        return Ok(repo.clone());
    };
    let worktree_name = normalize_worktree_name(&requested_name)?;
    let metadata = load_worktree_metadata(repo, &worktree_name)?;
    let worktree_path = required_path_field(&JsonValue::Object(metadata), "path")?;
    discover_worktree_repo(&worktree_path)
        .ok_or_else(|| format!("Worktree is missing or detached: {worktree_name}"))
}

pub fn worktree_status(
    repo: &RepoRuntime,
    name: Option<&str>,
    snapshot_id: Option<&str>,
    line_name: Option<&str>,
) -> Result<JsonValue, String> {
    let target_repo = resolve_worktree_content_repo(repo, name)?;
    workflow_workspace_status(&target_repo, snapshot_id, line_name)
}

pub fn worktree_list(repo: &RepoRuntime, refresh_status: bool) -> Result<JsonValue, String> {
    worktree_list_with_retarget(repo, refresh_status, true)
}

fn worktree_list_with_retarget(
    repo: &RepoRuntime,
    refresh_status: bool,
    include_retarget: bool,
) -> Result<JsonValue, String> {
    let registry_dir = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("worktrees");
    if !registry_dir.is_dir() {
        return Ok(JsonValue::Array(Vec::new()));
    }
    let mut paths = fs::read_dir(&registry_dir)
        .map_err(|err| err.to_string())?
        .map(|entry| {
            entry
                .map(|value| value.path())
                .map_err(|err| err.to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut rows = Vec::new();
    for path in paths {
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let payload = read_json_value(&path);
        let Some(obj) = payload.as_object() else {
            continue;
        };
        if obj.is_empty() {
            continue;
        }
        rows.push(worktree_summary_from_metadata_with_retarget(
            repo,
            obj,
            refresh_status,
            refresh_status,
            include_retarget,
        )?);
    }
    Ok(JsonValue::Array(rows))
}

pub(in crate::primitives) fn repo_status_worktree_hygiene(
    repo: &RepoRuntime,
) -> Result<JsonValue, String> {
    let rows = worktree_list_with_retarget(repo, false, false)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    worktree_doctor_from_rows(rows)
}

pub fn worktree_doctor(repo: &RepoRuntime, refresh_status: bool) -> Result<JsonValue, String> {
    let rows = worktree_list(repo, refresh_status)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut payload = worktree_doctor_from_rows(rows)?;
    if let Some(obj) = payload.as_object_mut() {
        obj.insert(
            "refresh_status".to_string(),
            JsonValue::Bool(refresh_status),
        );
        obj.insert(
            "status_mode".to_string(),
            JsonValue::String(
                if refresh_status {
                    "verified"
                } else {
                    "metadata"
                }
                .to_string(),
            ),
        );
    }
    Ok(payload)
}

pub(in crate::primitives) fn worktree_retarget_summary(
    repo: &RepoRuntime,
    metadata: &JsonMap<String, JsonValue>,
    current_line_name: Option<&str>,
    head_snapshot_id: Option<&str>,
) -> Result<JsonMap<String, JsonValue>, String> {
    worktree_retarget_summary_with_change(repo, metadata, current_line_name, head_snapshot_id, None)
}

pub(in crate::primitives) fn worktree_retarget_summary_with_change(
    repo: &RepoRuntime,
    metadata: &JsonMap<String, JsonValue>,
    current_line_name: Option<&str>,
    head_snapshot_id: Option<&str>,
    authoritative_change: Option<&JsonValue>,
) -> Result<JsonMap<String, JsonValue>, String> {
    let snapshot_store = snapshot_store(repo)?;
    let mut distance_cache = SnapshotAncestorDistanceCache::default();
    let mut snapshot_distance = |ancestor_snapshot_id: Option<&str>, snapshot_id: Option<&str>| {
        snapshot_distance_if_ancestor_with_snapshot_store_and_cache(
            &snapshot_store,
            ancestor_snapshot_id,
            snapshot_id,
            &mut distance_cache,
        )
    };
    let target_base_line = effective_worktree_target_base_line(metadata, authoritative_change);
    let target_base_snapshot_id = match target_base_line.as_deref() {
        Some(line_name) => local_line_head_snapshot_id(repo, line_name)?,
        None => None,
    };
    let mut fork_snapshot_id = metadata_string(metadata, "fork_snapshot_id")
        .or_else(|| authoritative_change.and_then(|value| string_field(value, "fork_snapshot_id")));
    let change_fork_snapshot_id =
        authoritative_change.and_then(|value| string_field(value, "fork_snapshot_id"));
    if let (
        Some(metadata_fork),
        Some(change_fork),
        Some(head_snapshot_id),
        Some(target_base_snapshot_id),
    ) = (
        fork_snapshot_id.clone(),
        change_fork_snapshot_id.clone(),
        normalized_text(head_snapshot_id),
        target_base_snapshot_id.clone(),
    ) {
        if metadata_fork != change_fork
            && snapshot_distance(Some(&metadata_fork), Some(&target_base_snapshot_id))?.is_none()
            && snapshot_distance(Some(&change_fork), Some(&head_snapshot_id))?.is_some()
            && (change_fork == target_base_snapshot_id
                || snapshot_distance(Some(&change_fork), Some(&target_base_snapshot_id))?.is_some())
        {
            fork_snapshot_id = Some(change_fork);
        }
    }
    if fork_snapshot_id.is_none() {
        if let Some(head_snapshot_id) = normalized_text(head_snapshot_id) {
            fork_snapshot_id = latest_common_snapshot(
                repo,
                &head_snapshot_id,
                target_base_snapshot_id.as_deref(),
            )?;
        }
    }
    let needs_retarget = target_base_line.is_some()
        && fork_snapshot_id.is_some()
        && target_base_snapshot_id.is_some()
        && fork_snapshot_id != target_base_snapshot_id;
    Ok(JsonMap::from_iter([
        (
            "target_base_line".to_string(),
            target_base_line
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "target_base_snapshot_id".to_string(),
            target_base_snapshot_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "fork_snapshot_id".to_string(),
            fork_snapshot_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "forked_from_line".to_string(),
            metadata_string(metadata, "forked_from_line")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "line_name".to_string(),
            normalized_text(current_line_name)
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "needs_retarget".to_string(),
            JsonValue::Bool(needs_retarget),
        ),
        (
            "feature_ahead_count".to_string(),
            snapshot_distance(fork_snapshot_id.as_deref(), head_snapshot_id)?
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "base_behind_count".to_string(),
            snapshot_distance(
                fork_snapshot_id.as_deref(),
                target_base_snapshot_id.as_deref(),
            )?
            .map(JsonValue::from)
            .unwrap_or(JsonValue::Null),
        ),
        (
            "rebase_state".to_string(),
            metadata_string(metadata, "rebase_state")
                .map(JsonValue::String)
                .unwrap_or_else(|| JsonValue::String("idle".to_string())),
        ),
        (
            "rebase_started_at".to_string(),
            metadata_string(metadata, "rebase_started_at")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "rebase_original_head_snapshot_id".to_string(),
            metadata_string(metadata, "rebase_original_head_snapshot_id")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "rebase_onto_snapshot_id".to_string(),
            metadata_string(metadata, "rebase_onto_snapshot_id")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "rebase_conflict_paths".to_string(),
            JsonValue::Array(
                metadata
                    .get("rebase_conflict_paths")
                    .and_then(JsonValue::as_array)
                    .cloned()
                    .unwrap_or_default(),
            ),
        ),
        (
            "last_retargeted_at".to_string(),
            metadata_string(metadata, "last_retargeted_at")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
    ]))
}

pub(in crate::primitives) fn worktree_summary_from_metadata(
    repo: &RepoRuntime,
    payload: &JsonMap<String, JsonValue>,
    refresh_status: bool,
    persist_status_cache: bool,
) -> Result<JsonValue, String> {
    worktree_summary_from_metadata_with_retarget(
        repo,
        payload,
        refresh_status,
        persist_status_cache,
        true,
    )
}

#[cfg(test)]
pub(in crate::primitives) fn worktree_summary_from_metadata_for_repo_status(
    repo: &RepoRuntime,
    payload: &JsonMap<String, JsonValue>,
) -> Result<JsonValue, String> {
    worktree_summary_from_metadata_with_retarget(repo, payload, false, false, false)
}

fn worktree_summary_from_metadata_with_retarget(
    repo: &RepoRuntime,
    payload: &JsonMap<String, JsonValue>,
    refresh_status: bool,
    persist_status_cache: bool,
    include_retarget: bool,
) -> Result<JsonValue, String> {
    let metadata = worktree_metadata_with_defaults(payload);
    let worktree_name = metadata_string(&metadata, "name")
        .ok_or_else(|| "Worktree metadata is missing name.".to_string())?;
    let worktree_path = metadata_string(&metadata, "path").map(PathBuf::from);
    if refresh_status {
        if let Some(path) = worktree_path.as_deref() {
            ensure_worktree_runtime_layout(&repo.authoritative_repo_root(), path)?;
            materialize_worktree_cargo_config(&repo.authoritative_repo_root(), path)?;
        }
    }
    let worktree_layout_present = worktree_path.as_ref().is_some_and(|path| {
        path.is_dir()
            && path.join(WORKTREE_CONFIG_NAME).exists()
            && path_exists_or_directory_link(&path.join(APP_DIR))
    });
    let worktree_repo = if refresh_status {
        worktree_path.as_deref().and_then(discover_worktree_repo)
    } else {
        None
    };
    let mut current_line_name = metadata_string(&metadata, "line_name");
    let cached_status = status_cache_value(&metadata);
    let mut head_snapshot_id = None::<String>;
    let mut status_label = "missing".to_string();
    let mut status_source = "verified".to_string();
    let mut status_checked_at = None::<String>;
    let mut clean = None::<bool>;
    let mut changed_count = None::<i64>;
    let mut modified_paths = Vec::new();
    let mut missing_paths = Vec::new();
    let mut untracked_paths = Vec::new();

    if let Some(worktree_repo) = worktree_repo.as_ref() {
        let verified_status = (|| -> Result<(String, Option<String>, JsonValue), String> {
            let verified_current_line = worktree_repo.current_line_name()?;
            let verified_head_snapshot =
                local_line_head_snapshot_id(worktree_repo, verified_current_line.as_str())?;
            let status =
                workspace_delta_payload(worktree_repo, verified_head_snapshot.as_deref(), None)?;
            Ok((verified_current_line, verified_head_snapshot, status))
        })();
        match verified_status {
            Ok((verified_current_line, verified_head_snapshot, status)) => {
                current_line_name = Some(verified_current_line);
                head_snapshot_id = verified_head_snapshot;
                status_label = if status
                    .get("clean")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
                {
                    "clean".to_string()
                } else {
                    "dirty".to_string()
                };
                status_source = "verified".to_string();
                status_checked_at = Some(system_event_timestamp());
                clean = status.get("clean").and_then(JsonValue::as_bool);
                changed_count = status.get("changed_count").and_then(JsonValue::as_i64);
                modified_paths = json_string_list(status.get("modified_paths"));
                missing_paths = json_string_list(status.get("missing_paths"));
                untracked_paths = json_string_list(status.get("untracked_paths"));
            }
            Err(_) => {
                if head_snapshot_id.is_none() {
                    if let Some(line_name) = current_line_name.as_deref() {
                        head_snapshot_id = local_line_head_snapshot_id(repo, line_name)?;
                    }
                }
                if let Some((
                    cached_workspace_status,
                    cached_clean,
                    cached_changed_count,
                    cached_modified_paths,
                    cached_missing_paths,
                    cached_untracked_paths,
                    cached_current_line,
                    cached_head_snapshot_id,
                    cached_status_checked_at,
                )) = cached_status.clone()
                {
                    if cached_current_line == current_line_name
                        && cached_head_snapshot_id == head_snapshot_id
                    {
                        status_label = cached_workspace_status;
                        status_source = "cached".to_string();
                        status_checked_at = cached_status_checked_at;
                        clean = cached_clean;
                        changed_count = cached_changed_count;
                        modified_paths = cached_modified_paths;
                        missing_paths = cached_missing_paths;
                        untracked_paths = cached_untracked_paths;
                    } else {
                        status_label = "unknown".to_string();
                        status_source = "unverified".to_string();
                    }
                } else {
                    status_label = "unknown".to_string();
                    status_source = "unverified".to_string();
                }
            }
        }
    } else if worktree_layout_present {
        let local_cfg =
            read_json_object_value(&worktree_path.clone().unwrap().join(WORKTREE_CONFIG_NAME));
        current_line_name = local_cfg
            .get("current_line")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value)))
            .or(current_line_name);
        if let Some(line_name) = current_line_name.as_deref() {
            head_snapshot_id = local_line_head_snapshot_id(repo, line_name)?;
        }
        if let Some((
            cached_workspace_status,
            cached_clean,
            cached_changed_count,
            cached_modified_paths,
            cached_missing_paths,
            cached_untracked_paths,
            cached_current_line,
            cached_head_snapshot_id,
            cached_status_checked_at,
        )) = cached_status.clone()
        {
            if cached_current_line == current_line_name
                && cached_head_snapshot_id == head_snapshot_id
            {
                status_label = cached_workspace_status;
                status_source = "cached".to_string();
                status_checked_at = cached_status_checked_at;
                clean = cached_clean;
                changed_count = cached_changed_count;
                modified_paths = cached_modified_paths;
                missing_paths = cached_missing_paths;
                untracked_paths = cached_untracked_paths;
            } else {
                status_label = "unknown".to_string();
                status_source = "unverified".to_string();
            }
        } else {
            status_label = "unknown".to_string();
            status_source = "unverified".to_string();
        }
    } else if worktree_path.as_ref().is_some_and(|path| path.is_dir()) {
        status_label = "detached".to_string();
    } else if let Some(line_name) = current_line_name.as_deref() {
        head_snapshot_id = local_line_head_snapshot_id(repo, line_name)?;
    }

    let is_current = worktree_path.as_ref().is_some_and(|path| {
        path.exists() && path.canonicalize().ok() == repo.workspace_root().canonicalize().ok()
    });
    let decision = worktree_cleanup_decision(
        repo,
        &JsonMap::from_iter(
            metadata.clone().into_iter().chain([
                (
                    "path".to_string(),
                    worktree_path
                        .as_ref()
                        .map(|value| JsonValue::String(value.to_string_lossy().to_string()))
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "clean".to_string(),
                    clean.map(JsonValue::Bool).unwrap_or(JsonValue::Null),
                ),
            ]),
        ),
        &status_label,
        is_current,
        None,
        false,
    )?;
    let retarget = if include_retarget {
        Some(worktree_retarget_summary(
            repo,
            &metadata,
            current_line_name.as_deref(),
            head_snapshot_id.as_deref(),
        )?)
    } else {
        None
    };

    if persist_status_cache
        && status_source == "verified"
        && matches!(
            status_label.as_str(),
            "clean" | "dirty" | "missing" | "detached"
        )
    {
        if let Ok(mut updated) = load_worktree_metadata(repo, &worktree_name) {
            updated.insert(
                "workspace_status_cache".to_string(),
                json!({
                    "workspace_status": status_label,
                    "clean": clean,
                    "changed_count": changed_count,
                    "modified_paths": modified_paths,
                    "missing_paths": missing_paths,
                    "untracked_paths": untracked_paths,
                    "current_line": current_line_name,
                    "head_snapshot_id": head_snapshot_id,
                    "status_checked_at": status_checked_at.clone().unwrap_or_else(system_event_timestamp),
                }),
            );
            let _ = save_worktree_metadata(repo, &worktree_name, &updated);
        }
    } else if persist_status_cache && status_source == "unverified" {
        if let Ok(mut updated) = load_worktree_metadata(repo, &worktree_name) {
            updated.remove("workspace_status_cache");
            let _ = save_worktree_metadata(repo, &worktree_name, &updated);
        }
    }

    let mut summary = JsonMap::from_iter([
        ("name".to_string(), JsonValue::String(worktree_name.clone())),
        (
            "path".to_string(),
            worktree_path
                .as_ref()
                .map(|value| JsonValue::String(value.to_string_lossy().to_string()))
                .unwrap_or(JsonValue::Null),
        ),
        (
            "alias_path".to_string(),
            metadata_string(&metadata, "alias_path")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "open_path".to_string(),
            metadata_string(&metadata, "alias_path")
                .or_else(|| {
                    worktree_path
                        .as_ref()
                        .map(|value| value.to_string_lossy().to_string())
                })
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "venv_path".to_string(),
            worktree_path
                .as_ref()
                .map(|value| value.join(".venv"))
                .filter(|value| value.exists())
                .map(|value| JsonValue::String(value.to_string_lossy().to_string()))
                .unwrap_or(JsonValue::Null),
        ),
        (
            "repo_root".to_string(),
            metadata_string(&metadata, "repo_root")
                .unwrap_or_else(|| repo.authoritative_repo_root().to_string_lossy().to_string())
                .into(),
        ),
        (
            "registered_line_name".to_string(),
            metadata_string(&metadata, "line_name")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "current_line".to_string(),
            current_line_name
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "head_snapshot_id".to_string(),
            head_snapshot_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "created_at".to_string(),
            metadata_string(&metadata, "created_at")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "exists".to_string(),
            JsonValue::Bool(worktree_path.as_ref().is_some_and(|path| path.is_dir())),
        ),
        ("is_current".to_string(), JsonValue::Bool(is_current)),
        (
            "workspace_status".to_string(),
            JsonValue::String(status_label.clone()),
        ),
        (
            "status_source".to_string(),
            JsonValue::String(status_source.clone()),
        ),
        (
            "status_checked_at".to_string(),
            status_checked_at
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "clean".to_string(),
            clean.map(JsonValue::Bool).unwrap_or(JsonValue::Null),
        ),
        (
            "changed_count".to_string(),
            changed_count
                .map(JsonValue::from)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "modified_paths".to_string(),
            JsonValue::Array(
                modified_paths
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        ),
        (
            "missing_paths".to_string(),
            JsonValue::Array(
                missing_paths
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        ),
        (
            "untracked_paths".to_string(),
            JsonValue::Array(
                untracked_paths
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        ),
        (
            "bound_task_id".to_string(),
            metadata_string(&metadata, "bound_task_id")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "bound_change_id".to_string(),
            metadata_string(&metadata, "bound_change_id")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "bound_change_ref".to_string(),
            metadata_string(&metadata, "bound_change_ref")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "bound_task_status".to_string(),
            metadata_string(&metadata, "bound_task_status")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "bound_change_status".to_string(),
            metadata_string(&metadata, "bound_change_status")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "auto_created_for_task".to_string(),
            JsonValue::Bool(
                metadata
                    .get("auto_created_for_task")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
            ),
        ),
        (
            "root_source".to_string(),
            metadata_string(&metadata, "root_source")
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
    ]);
    let open_path = metadata_string(&metadata, "alias_path").or_else(|| {
        worktree_path
            .as_ref()
            .map(|value| value.to_string_lossy().to_string())
    });
    let effective_line_name = current_line_name
        .clone()
        .or_else(|| metadata_string(&metadata, "line_name"));
    let cargo_enabled = open_path.as_deref().is_some_and(|path| {
        cargo_worktree_integration_enabled(&repo.authoritative_repo_root(), Path::new(path))
    });
    let cargo_open_path = cargo_enabled.then_some(open_path.as_deref()).flatten();
    summary.insert(
        "cd_command".to_string(),
        open_path
            .as_deref()
            .map(|value| JsonValue::String(format!("cd {}", shell_escape(Path::new(value)))))
            .unwrap_or(JsonValue::Null),
    );
    summary.insert(
        "shell_command".to_string(),
        match (open_path.as_deref(), effective_line_name.as_deref()) {
            (Some(path), Some(line_name)) => JsonValue::String(task_worktree_shell_command(
                &repo.authoritative_repo_root(),
                Path::new(path),
                &worktree_name,
                line_name,
            )),
            _ => JsonValue::Null,
        },
    );
    summary.insert(
        "cargo_target_dir".to_string(),
        cargo_open_path
            .map(|value| {
                JsonValue::String(
                    worktree_cargo_target_dir(Path::new(value))
                        .to_string_lossy()
                        .to_string(),
                )
            })
            .unwrap_or(JsonValue::Null),
    );
    summary.insert(
        "cargo_build_dir".to_string(),
        cargo_open_path
            .map(|value| {
                JsonValue::String(
                    worktree_cargo_build_dir(Path::new(value))
                        .to_string_lossy()
                        .to_string(),
                )
            })
            .unwrap_or(JsonValue::Null),
    );
    summary.extend(decision.clone());
    summary.insert(
        "cleanup".to_string(),
        json!({
            "class": decision.get("cleanup_class").cloned().unwrap_or(JsonValue::Null),
            "candidate": decision.get("cleanup_candidate").cloned().unwrap_or(JsonValue::Null),
            "reason": decision.get("cleanup_reason").cloned().unwrap_or(JsonValue::Null),
            "protected_reason": decision.get("protected_reason").cloned().unwrap_or(JsonValue::Null),
            "manual_review_candidate": decision.get("manual_review_candidate").cloned().unwrap_or(JsonValue::Null),
            "manual_review_reason": decision.get("manual_review_reason").cloned().unwrap_or(JsonValue::Null),
            "older_than": decision.get("older_than").cloned().unwrap_or(JsonValue::Null),
        }),
    );
    if let Some(retarget) = retarget {
        summary.extend(retarget.clone());
        summary.insert("retarget".to_string(), JsonValue::Object(retarget));
    }
    let merge_state = metadata_string(&metadata, "merge_state").unwrap_or_else(|| "idle".into());
    let merge_conflict_paths = metadata
        .get("merge_conflict_paths")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let merge = json!({
        "state": merge_state,
        "started_at": metadata_string(&metadata, "merge_started_at"),
        "target_line": metadata_string(&metadata, "merge_target_line"),
        "source_line": metadata_string(&metadata, "merge_source_line"),
        "target_snapshot_id": metadata_string(&metadata, "merge_target_snapshot_id"),
        "source_snapshot_id": metadata_string(&metadata, "merge_source_snapshot_id"),
        "pre_workspace_snapshot_id": metadata_string(&metadata, "merge_pre_workspace_snapshot_id"),
        "base_snapshot_ids": metadata.get("merge_base_snapshot_ids").cloned().unwrap_or_else(|| JsonValue::Array(Vec::new())),
        "conflict_paths": merge_conflict_paths,
        "conflict_kinds": metadata.get("merge_conflict_kinds").cloned().unwrap_or_else(|| JsonValue::Object(JsonMap::new())),
    });
    summary.insert("merge_state".to_string(), merge["state"].clone());
    summary.insert(
        "merge_conflict_count".to_string(),
        JsonValue::from(
            merge["conflict_paths"]
                .as_array()
                .map_or(0_i64, |paths| paths.len() as i64),
        ),
    );
    summary.insert("merge".to_string(), merge);
    Ok(JsonValue::Object(summary))
}

pub(in crate::primitives) fn worktree_doctor_from_rows(
    rows: Vec<JsonValue>,
) -> Result<JsonValue, String> {
    let total_count = rows.len() as i64;
    let mut current_count = 0_i64;
    let mut clean_count = 0_i64;
    let mut dirty_count = 0_i64;
    let mut missing_count = 0_i64;
    let mut detached_count = 0_i64;
    let mut protected_count = 0_i64;
    let mut safe_auto_remove_count = 0_i64;
    let mut safe_cleanup_candidate_count = 0_i64;
    let mut manual_review_candidate_count = 0_i64;
    let mut stale_rows = Vec::new();
    let mut cleanup_candidate_rows = Vec::new();
    let mut manual_review_rows = Vec::new();

    for row in &rows {
        if row
            .get("is_current")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            current_count += 1;
        }
        match string_field(row, "workspace_status").as_deref() {
            Some("clean") => clean_count += 1,
            Some("dirty") => dirty_count += 1,
            Some("missing") => {
                missing_count += 1;
                stale_rows.push(row.clone());
            }
            Some("detached") => {
                detached_count += 1;
                stale_rows.push(row.clone());
            }
            _ => {}
        }
        match string_field(row, "cleanup_class").as_deref() {
            Some("protected") => protected_count += 1,
            Some("safe_auto_remove") => {
                safe_auto_remove_count += 1;
                cleanup_candidate_rows.push(row.clone());
            }
            Some("safe_cleanup_candidate") => {
                safe_cleanup_candidate_count += 1;
                cleanup_candidate_rows.push(row.clone());
            }
            _ => {}
        }
        if row
            .get("manual_review_candidate")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            manual_review_candidate_count += 1;
            manual_review_rows.push(row.clone());
        }
    }
    cleanup_candidate_rows.sort_by(|left, right| {
        cleanup_candidate_sort_key(left).cmp(&cleanup_candidate_sort_key(right))
    });
    stale_rows.sort_by(|left, right| {
        (
            string_field(left, "workspace_status").unwrap_or_default(),
            string_field(left, "name").unwrap_or_default(),
        )
            .cmp(&(
                string_field(right, "workspace_status").unwrap_or_default(),
                string_field(right, "name").unwrap_or_default(),
            ))
    });
    manual_review_rows.sort_by(|left, right| {
        (
            string_field(left, "manual_review_reason").unwrap_or_default(),
            string_field(left, "name").unwrap_or_default(),
        )
            .cmp(&(
                string_field(right, "manual_review_reason").unwrap_or_default(),
                string_field(right, "name").unwrap_or_default(),
            ))
    });
    Ok(json!({
        "total_count": total_count,
        "current_count": current_count,
        "clean_count": clean_count,
        "dirty_count": dirty_count,
        "missing_count": missing_count,
        "detached_count": detached_count,
        "protected_count": protected_count,
        "safe_auto_remove_count": safe_auto_remove_count,
        "safe_cleanup_candidate_count": safe_cleanup_candidate_count,
        "manual_review_candidate_count": manual_review_candidate_count,
        "healthy": missing_count == 0 && detached_count == 0,
        "stale_count": stale_rows.len(),
        "stale_rows": stale_rows,
        "cleanup_candidate_rows": cleanup_candidate_rows,
        "manual_review_rows": manual_review_rows,
        "rows": rows,
    }))
}
