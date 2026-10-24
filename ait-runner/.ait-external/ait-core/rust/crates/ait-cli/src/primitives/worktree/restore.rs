use super::*;

pub fn worktree_restore(
    repo: &RepoRuntime,
    name: Option<&str>,
    snapshot_id: Option<&str>,
    line_name: Option<&str>,
    paths: &[String],
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    run_locked_workspace_command(repo, "ait-cli worktree restore", || {
        guard_no_active_line_merge(repo, name, "restoring the worktree")?;
        worktree_restore_unlocked(repo, name, snapshot_id, line_name, paths, force, dry_run)
    })
}

pub(in crate::primitives) fn worktree_restore_unlocked(
    repo: &RepoRuntime,
    name: Option<&str>,
    snapshot_id: Option<&str>,
    line_name: Option<&str>,
    paths: &[String],
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    if snapshot_id.is_some() && line_name.is_some() {
        return Err("Choose either snapshot_id or line_name, not both.".to_string());
    }
    if !paths.is_empty() && snapshot_id.is_none() && line_name.is_none() {
        return Err("Selected-path restore requires --snapshot or --line".to_string());
    }
    let target_repo = resolve_worktree_content_repo(repo, name)?;
    let current_line_name = target_repo.current_line_name()?;
    let current_line_row = local_line_row(&target_repo, &current_line_name)?;
    let baseline_snapshot_id = string_field(&current_line_row, "head_snapshot_id");
    let target_line_name = normalized_text(line_name).unwrap_or_else(|| current_line_name.clone());
    let target_snapshot_id = match normalized_text(snapshot_id) {
        Some(value) => Some(value),
        None => local_line_head_snapshot_id(&target_repo, &target_line_name)?,
    };
    if !paths.is_empty() && target_snapshot_id.is_none() {
        return Err(format!(
            "Line {target_line_name} has no head snapshot to restore selected paths from."
        ));
    }

    let mut result = if paths.is_empty() {
        restore_workspace_all(
            &target_repo,
            target_snapshot_id.as_deref(),
            baseline_snapshot_id.as_deref(),
            force,
            dry_run,
        )?
    } else {
        restore_workspace_paths_selected(
            &target_repo,
            target_snapshot_id.as_deref(),
            paths,
            baseline_snapshot_id.as_deref(),
            force,
            dry_run,
        )?
    };

    let result_obj = result
        .as_object_mut()
        .ok_or_else(|| "worktree restore payload must be an object".to_string())?;
    result_obj.insert(
        "repo_name".to_string(),
        JsonValue::String(target_repo.repo_name()),
    );
    result_obj.insert(
        "workspace_root".to_string(),
        JsonValue::String(target_repo.workspace_root().to_string_lossy().to_string()),
    );
    result_obj.insert(
        "worktree_name".to_string(),
        target_repo
            .config
            .get("worktree_name")
            .cloned()
            .unwrap_or(JsonValue::Null),
    );
    result_obj.insert(
        "current_line_before".to_string(),
        JsonValue::String(current_line_name.clone()),
    );
    result_obj.insert(
        "current_line".to_string(),
        JsonValue::String(current_line_name.clone()),
    );
    result_obj.insert(
        "line_name".to_string(),
        JsonValue::String(target_line_name.clone()),
    );
    result_obj.insert(
        "line_head_snapshot_id".to_string(),
        target_snapshot_id
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );

    if !dry_run && paths.is_empty() {
        target_repo.set_worktree_materialized_snapshot(target_snapshot_id.as_deref())?;
    }
    if line_name.is_some() && paths.is_empty() {
        if !dry_run {
            set_runtime_current_line(&target_repo, &target_line_name)?;
        }
        result_obj.insert(
            "current_line".to_string(),
            JsonValue::String(target_line_name.clone()),
        );
    }
    Ok(result)
}

pub fn worktree_sync(
    repo: &RepoRuntime,
    name: Option<&str>,
    line_name: Option<&str>,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    guard_no_active_line_merge(repo, name, "synchronizing the worktree")?;
    let worktree_name = resolve_runtime_worktree_name(repo, name)?;
    let metadata = load_worktree_metadata(repo, &worktree_name)?;
    let worktree_path = required_path_field(&JsonValue::Object(metadata.clone()), "path")?;
    let Some(worktree_repo) = discover_worktree_repo(&worktree_path) else {
        return Err(format!("Worktree is missing or detached: {worktree_name}"));
    };
    let target_line_name = normalized_text(line_name).unwrap_or_else(|| {
        worktree_repo
            .current_line_name()
            .unwrap_or_else(|_| "main".to_string())
    });
    let target_snapshot_id = local_line_head_snapshot_id(&worktree_repo, &target_line_name)?;
    let baseline_snapshot_id = worktree_repo
        .config
        .get("materialized_snapshot_id")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)));
    let mut restore_data = restore_workspace_all(
        &worktree_repo,
        target_snapshot_id.as_deref(),
        baseline_snapshot_id.as_deref(),
        force,
        dry_run,
    )?;
    let current_line_before = worktree_repo.current_line_name()?;
    restore_data
        .as_object_mut()
        .ok_or_else(|| "worktree sync restore payload must be an object".to_string())?
        .extend(JsonMap::from_iter([
            (
                "repo_name".to_string(),
                JsonValue::String(worktree_repo.repo_name()),
            ),
            (
                "current_line_before".to_string(),
                JsonValue::String(current_line_before.clone()),
            ),
            (
                "current_line".to_string(),
                JsonValue::String(current_line_before.clone()),
            ),
            (
                "line_name".to_string(),
                JsonValue::String(target_line_name.clone()),
            ),
            (
                "line_head_snapshot_id".to_string(),
                target_snapshot_id
                    .clone()
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "materialized_snapshot_id_before".to_string(),
                baseline_snapshot_id
                    .clone()
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
        ]));
    if !dry_run {
        materialize_worktree_docs_symlink(&repo.authoritative_repo_root(), &worktree_path)?;
        materialize_worktree_cargo_config(&repo.authoritative_repo_root(), &worktree_path)?;
        worktree_repo.set_worktree_materialized_snapshot(target_snapshot_id.as_deref())?;
        if current_line_before != target_line_name {
            set_worktree_current_line(&worktree_repo, &target_line_name)?;
            restore_data
                .as_object_mut()
                .expect("sync restore payload")
                .insert(
                    "current_line".to_string(),
                    JsonValue::String(target_line_name.clone()),
                );
        }
        let mut updated = load_worktree_metadata(repo, &worktree_name)?;
        updated.insert(
            "line_name".to_string(),
            JsonValue::String(target_line_name.clone()),
        );
        updated.insert(
            "path".to_string(),
            JsonValue::String(worktree_path.to_string_lossy().to_string()),
        );
        updated.insert(
            "repo_root".to_string(),
            JsonValue::String(repo.authoritative_repo_root().to_string_lossy().to_string()),
        );
        updated.insert(
            "last_used_at".to_string(),
            JsonValue::String(system_event_timestamp()),
        );
        save_worktree_metadata(repo, &worktree_name, &updated)?;
        let mut summary = worktree_get(repo, Some(&worktree_name), true)?;
        summary
            .as_object_mut()
            .expect("worktree sync summary")
            .insert("restore".to_string(), restore_data);
        return Ok(summary);
    }
    let mut summary = worktree_get(repo, Some(&worktree_name), true)?;
    summary
        .as_object_mut()
        .expect("worktree sync summary")
        .insert("restore".to_string(), restore_data);
    Ok(summary)
}

pub fn worktree_sync_all(
    repo: &RepoRuntime,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let rows = worktree_list(repo, true)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let requested_count = rows.len();
    let mut synced_rows = Vec::new();
    let mut skipped_rows = Vec::new();
    let mut error_rows = Vec::new();
    for row in rows {
        let Some(name) = string_field(&row, "name") else {
            continue;
        };
        let status = string_field(&row, "workspace_status").unwrap_or_default();
        if matches!(status.as_str(), "missing" | "detached") {
            let mut skipped = row.as_object().cloned().unwrap_or_default();
            skipped.insert(
                "reason".to_string(),
                JsonValue::String("stale_registration".to_string()),
            );
            skipped_rows.push(JsonValue::Object(skipped));
            continue;
        }
        match worktree_sync(repo, Some(&name), None, force, dry_run) {
            Ok(value) => synced_rows.push(value),
            Err(err) => {
                error_rows.push(json!({
                    "name": name,
                    "path": string_field(&row, "path"),
                    "current_line": string_field(&row, "current_line"),
                    "workspace_status": string_field(&row, "workspace_status"),
                    "error": err,
                }));
            }
        }
    }
    Ok(json!({
        "dry_run": dry_run,
        "force": force,
        "target": "all",
        "requested_count": requested_count,
        "synced_count": synced_rows.len(),
        "skipped_count": skipped_rows.len(),
        "error_count": error_rows.len(),
        "ok": error_rows.is_empty(),
        "synced_rows": synced_rows,
        "skipped_rows": skipped_rows,
        "error_rows": error_rows,
    }))
}

pub fn worktree_touch_usage(repo: &RepoRuntime, name: Option<&str>) -> Result<JsonValue, String> {
    let resolved_name = match normalized_text(name) {
        Some(value) => Some(normalize_worktree_name(&value)?),
        None if repo.is_worktree() => Some(resolve_runtime_worktree_name(repo, None)?),
        None => active_root_worktree_binding_name(repo),
    };
    let Some(worktree_name) = resolved_name else {
        return Ok(JsonValue::Null);
    };
    let mut metadata = load_worktree_metadata(repo, &worktree_name)?;
    metadata.insert(
        "last_used_at".to_string(),
        JsonValue::String(system_event_timestamp()),
    );
    save_worktree_metadata(repo, &worktree_name, &metadata)?;
    worktree_get(repo, Some(&worktree_name), false)
}

pub fn worktree_recreate(
    repo: &RepoRuntime,
    name: Option<&str>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let worktree_name = resolve_runtime_worktree_name(repo, name)?;
    let metadata = worktree_metadata_with_defaults(&load_worktree_metadata(repo, &worktree_name)?);
    let summary = worktree_get(repo, Some(&worktree_name), true)?;
    if string_field(&summary, "workspace_status").as_deref() != Some("missing") {
        return Err(format!("Worktree {worktree_name} is not missing."));
    }
    if metadata_string(&metadata, "bound_task_id").is_none() {
        return Err(format!(
            "Worktree {worktree_name} is not task-bound; automatic recreate is only supported for task worktrees."
        ));
    }
    let line_name = string_field(&summary, "registered_line_name")
        .or_else(|| metadata_string(&metadata, "line_name"))
        .ok_or_else(|| format!("Worktree {worktree_name} has no registered line to recreate."))?;

    let mut candidates = Vec::new();
    if let Some(snapshot_id) = local_line_head_snapshot_id(repo, &line_name)? {
        if local_snapshot_exists(repo, &snapshot_id)? {
            candidates.push(json!({
                "source": "current_line_head",
                "snapshot_id": snapshot_id,
                "available_locally": true,
                "line_name": line_name,
            }));
        }
    }
    if let Some(snapshot_id) = metadata_string(&metadata, "fork_snapshot_id") {
        if local_snapshot_exists(repo, &snapshot_id)? {
            candidates.push(json!({
                "source": "fork_snapshot",
                "snapshot_id": snapshot_id,
                "available_locally": true,
                "forked_from_line": metadata_string(&metadata, "forked_from_line"),
            }));
        }
    }
    if let Some(change_ref) = metadata_string(&metadata, "bound_change_ref")
        .or_else(|| metadata_string(&metadata, "bound_change_id"))
    {
        if let Ok((remote_row, repo_name)) = remote_context(repo, None, None) {
            if let Ok(mut task_remote) = http_task_remote(repo, &remote_row) {
                if let Ok(mut closeout_remote) = http_closeout_remote(repo, &remote_row) {
                    if let Ok(Some(candidate)) =
                        worktree_remote_patchset_revision_candidate_with_remotes(
                            repo,
                            &mut task_remote,
                            &mut closeout_remote,
                            &repo_name,
                            &change_ref,
                        )
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
    }
    let mut seen_snapshot_ids = BTreeSet::new();
    candidates.retain(|row| {
        string_field(row, "snapshot_id")
            .is_some_and(|snapshot_id| seen_snapshot_ids.insert(snapshot_id))
    });
    if candidates.is_empty() {
        return Err(format!(
            "Worktree {worktree_name} has no locally available recreate snapshot. Expected one of the bound line head, fork snapshot, or selected patchset revision to still exist."
        ));
    }

    let target_path = required_path_field(&JsonValue::Object(metadata.clone()), "path")?;
    let alias_path = metadata_string(&metadata, "alias_path").map(PathBuf::from);
    if path_exists_or_directory_link(&target_path) {
        return Err(format!(
            "Worktree path is no longer missing: {}",
            target_path.display()
        ));
    }
    if let Some(alias_path) = alias_path.as_ref() {
        if path_exists_or_directory_link(alias_path) && !alias_path.is_symlink() {
            return Err(format!(
                "Worktree alias path is occupied by a non-link entry: {}",
                alias_path.display()
            ));
        }
        if alias_path.is_symlink() && !directory_link_points_at(alias_path, &target_path)? {
            return Err(format!(
                "Worktree alias path points at a different target and cannot be reclaimed automatically: {}",
                alias_path.display()
            ));
        }
    }

    let candidate = candidates
        .first()
        .cloned()
        .ok_or_else(|| "worktree recreate candidate selection unexpectedly failed".to_string())?;
    let chosen_snapshot_id = required_string_field(&candidate, "snapshot_id")?;
    let recreate_payload = json!({
        "name": worktree_name,
        "path": target_path.to_string_lossy().to_string(),
        "alias_path": alias_path.as_ref().map(|value| value.to_string_lossy().to_string()),
        "line_name": line_name,
        "workspace_status_before": string_field(&summary, "workspace_status"),
        "dry_run": dry_run,
        "recreate": {
            "candidate": candidate,
            "candidates": candidates,
            "managed_alias_recreated": alias_path.is_some(),
        },
    });
    if dry_run {
        return Ok(recreate_payload);
    }

    let created_at =
        metadata_string(&metadata, "created_at").unwrap_or_else(system_event_timestamp);
    let worktree_repo = materialize_worktree_runtime_layout(
        repo,
        &worktree_name,
        &target_path,
        &line_name,
        &created_at,
        None,
    )?;
    if let Some(alias_path) = alias_path.as_ref() {
        if path_exists_or_directory_link(alias_path) {
            remove_path_entry(alias_path)?;
        }
        create_directory_link(alias_path, &target_path)?;
    }
    let line_row = match local_line_row(&worktree_repo, &line_name) {
        Ok(row) => row,
        Err(_) => create_local_line(&worktree_repo, &line_name, Some(&chosen_snapshot_id))?,
    };
    let current_head_snapshot_id = string_field(&line_row, "head_snapshot_id");
    if current_head_snapshot_id.is_none()
        || (current_head_snapshot_id.as_deref() != Some(chosen_snapshot_id.as_str())
            && current_head_snapshot_id
                .as_deref()
                .is_some_and(|value| !local_snapshot_exists(repo, value).unwrap_or(false)))
    {
        set_local_line_head(&worktree_repo, &line_name, Some(&chosen_snapshot_id))?;
    }
    set_worktree_current_line(&worktree_repo, &line_name)?;
    restore_workspace_all(&worktree_repo, Some(&chosen_snapshot_id), None, true, false)?;
    materialize_worktree_docs_symlink(&repo.authoritative_repo_root(), &target_path)?;
    materialize_worktree_cargo_config(&repo.authoritative_repo_root(), &target_path)?;
    worktree_repo.set_worktree_materialized_snapshot(Some(&chosen_snapshot_id))?;
    let recreated_at = system_event_timestamp();
    let mut updated = load_worktree_metadata(repo, &worktree_name)?;
    updated.insert(
        "last_used_at".to_string(),
        JsonValue::String(recreated_at.clone()),
    );
    updated.insert(
        "workspace_status_cache".to_string(),
        json!({
            "workspace_status": "clean",
            "clean": true,
            "changed_count": 0,
            "modified_paths": [],
            "missing_paths": [],
            "untracked_paths": [],
            "current_line": line_name,
            "head_snapshot_id": chosen_snapshot_id,
            "status_checked_at": recreated_at,
        }),
    );
    save_worktree_metadata(repo, &worktree_name, &updated)?;
    let mut summary = worktree_get(repo, Some(&worktree_name), true)?;
    summary
        .as_object_mut()
        .expect("worktree recreate summary")
        .extend(JsonMap::from_iter([
            (
                "workspace_status_after".to_string(),
                JsonValue::String("clean".to_string()),
            ),
            (
                "head_snapshot_id".to_string(),
                JsonValue::String(chosen_snapshot_id),
            ),
            (
                "recreate".to_string(),
                recreate_payload
                    .get("recreate")
                    .cloned()
                    .unwrap_or(JsonValue::Null),
            ),
        ]));
    Ok(summary)
}
