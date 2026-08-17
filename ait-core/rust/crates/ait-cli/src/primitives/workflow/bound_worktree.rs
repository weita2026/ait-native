use super::*;
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;
use ait_core::local_snapshot::LocalSnapshotTreeReadStore;

type RegisteredWorktreeMetadataRow = (String, JsonMap<String, JsonValue>);

pub(in crate::primitives) fn workflow_registered_worktree_metadata_rows(
    repo: &RepoRuntime,
) -> Result<Vec<RegisteredWorktreeMetadataRow>, String> {
    let registry_dir = repo
        .authoritative_repo_root()
        .join(".ait")
        .join("worktrees");
    if !registry_dir.is_dir() {
        return Ok(Vec::new());
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
        let metadata = worktree_metadata_with_defaults(obj);
        let fallback_name = path.file_stem().and_then(|value| value.to_str());
        let Some(worktree_name) =
            metadata_string(&metadata, "name").or_else(|| normalized_text(fallback_name))
        else {
            continue;
        };
        rows.push((worktree_name, metadata));
    }
    Ok(rows)
}

pub(in crate::primitives) fn workflow_sort_bound_worktree_rows(rows: &mut [JsonValue]) {
    rows.sort_by_key(|row| {
        (
            row.get("auto_created_for_task")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            string_field(row, "created_at").unwrap_or_default(),
            string_field(row, "name").unwrap_or_default(),
        )
    });
    rows.reverse();
}

pub(in crate::primitives) fn workflow_find_bound_task_worktree(
    root_repo: &RepoRuntime,
    task_id: &str,
) -> Result<Option<JsonValue>, String> {
    let Some(task_id) = normalized_text(Some(task_id)) else {
        return Ok(None);
    };
    let candidate_names = workflow_registered_worktree_metadata_rows(root_repo)?
        .into_iter()
        .filter(|(_, metadata)| {
            metadata_string(metadata, "bound_task_id").as_deref() == Some(task_id.as_str())
                && metadata
                    .get("auto_created_for_task")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
        })
        .map(|(worktree_name, _)| worktree_name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut rows = Vec::new();
    for worktree_name in candidate_names {
        match worktree_get(root_repo, Some(&worktree_name), false) {
            Ok(row) => rows.push(row),
            Err(err) if err.starts_with("Unknown worktree:") => continue,
            Err(err) => return Err(err),
        }
    }
    if rows.is_empty() {
        rows = worktree_list(root_repo, false)?
            .as_array()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|row| {
                workflow_nested_text(row, "binding_summary", "task_id")
                    .or_else(|| string_field(row, "bound_task_id"))
                    .as_deref()
                    == Some(task_id.as_str())
                    && row
                        .get("auto_created_for_task")
                        .and_then(JsonValue::as_bool)
                        .unwrap_or(false)
            })
            .collect();
    }
    workflow_sort_bound_worktree_rows(&mut rows);
    Ok(rows.into_iter().next())
}

pub(in crate::primitives) fn workflow_find_bound_task_worktree_metadata(
    root_repo: &RepoRuntime,
    task_id: &str,
) -> Result<Option<JsonValue>, String> {
    let Some(task_id) = normalized_text(Some(task_id)) else {
        return Ok(None);
    };
    let mut rows = workflow_registered_worktree_metadata_rows(root_repo)?
        .into_iter()
        .filter_map(|(worktree_name, mut metadata)| {
            if metadata_string(&metadata, "bound_task_id").as_deref() != Some(task_id.as_str())
                || !metadata
                    .get("auto_created_for_task")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
            {
                return None;
            }
            metadata
                .entry("name".to_string())
                .or_insert_with(|| JsonValue::String(worktree_name));
            Some(JsonValue::Object(metadata))
        })
        .collect::<Vec<_>>();
    workflow_sort_bound_worktree_rows(&mut rows);
    Ok(rows.into_iter().next())
}

pub(in crate::primitives) fn workflow_repo_root_restore_after_land(
    repo: &RepoRuntime,
    target_line: &str,
    previous_head_snapshot_id: Option<&str>,
    target_snapshot_id: Option<&str>,
) -> Result<JsonValue, String> {
    let root_repo = workflow_root_repo(repo)?;
    let default_line = root_repo.default_line_name();
    let mut payload = json!({
        "default_line": default_line,
        "line": target_line,
        "workspace_root": root_repo.workspace_root().to_string_lossy().to_string(),
        "previous_head_snapshot_id": previous_head_snapshot_id,
        "target_snapshot_id": target_snapshot_id,
        "auto_rebase": true,
    });
    if target_line != default_line {
        payload["status"] = JsonValue::String("skipped".to_string());
        payload["reason"] = JsonValue::String("target_not_default_line".to_string());
        return Ok(payload);
    }
    let landed_diff_paths = if let (Some(previous), Some(target)) = (
        normalized_text(previous_head_snapshot_id),
        normalized_text(target_snapshot_id),
    ) {
        let workspace_root = root_repo.workspace_root();
        let store = root_repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
        store
            .snapshot_tree_path_delta(Some(previous.as_str()), Some(target.as_str()))?
            .affected_paths
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    payload["landed_diff_paths"] = JsonValue::Array(
        landed_diff_paths
            .iter()
            .cloned()
            .map(JsonValue::String)
            .collect(),
    );
    payload["workspace_read_scope"] = JsonValue::String("landed_diff_paths".to_string());
    payload["outside_path_policy"] = json!({
        "enumerated": false,
        "read": false,
        "written": false,
        "reason": "exact_landed_delta_only",
    });
    let restored = restore_workspace_paths_selected(
        &root_repo,
        target_snapshot_id,
        &landed_diff_paths,
        previous_head_snapshot_id,
        true,
        false,
    )?;
    if let Some(restored_obj) = restored.as_object() {
        for (key, value) in restored_obj {
            payload[key] = value.clone();
        }
    }
    payload["status"] = JsonValue::String("restored".to_string());
    payload["main_seed_sync"] = json!({
        "status": "pending",
        "reason": "task_land_cli_seed_finalizer",
        "line_name": target_line,
        "default_line": default_line,
        "target_snapshot_id": target_snapshot_id,
        "detail": "Task land will update the CLI-owned main seed after final Task status is known; the server-owned refresh remains asynchronous.",
    });
    Ok(payload)
}

pub(in crate::primitives) fn workflow_bound_worktree_cleanup_after_local_land(
    repo: &RepoRuntime,
    task_id: &str,
    task_status: &str,
    change_status: &str,
) -> Result<JsonValue, String> {
    if task_status != "completed" {
        return Ok(json!({
            "status": "skipped",
            "reason": "task_not_completed",
            "task_id": task_id,
            "task_status": task_status,
        }));
    }
    if change_status != "landed" {
        return Ok(json!({
            "status": "skipped",
            "reason": "change_not_landed",
            "task_id": task_id,
            "change_status": change_status,
        }));
    }
    let root_repo = workflow_root_repo(repo)?;
    let Some(bound_worktree) = workflow_find_bound_task_worktree(&root_repo, task_id)? else {
        return Ok(json!({
            "status": "skipped",
            "reason": "no_bound_worktree",
            "task_id": task_id,
        }));
    };
    let worktree_name = required_string_field(&bound_worktree, "name")?;
    let removed = remove_one_worktree(&root_repo, &worktree_name, true, false)?;
    Ok(json!({
        "status": "removed",
        "task_id": task_id,
        "worktree": removed,
    }))
}
