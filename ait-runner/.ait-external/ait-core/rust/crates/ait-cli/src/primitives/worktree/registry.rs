use super::*;

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror explicit existing-worktree binding metadata"
)]
pub fn worktree_bind_existing(
    repo: &RepoRuntime,
    worktree_name: &str,
    task_id: Option<&str>,
    change_id: Option<&str>,
    auto_created_for_task: bool,
    target_base_line: Option<&str>,
    fork_snapshot_id: Option<&str>,
    forked_from_line: Option<&str>,
) -> Result<JsonValue, String> {
    let resolved_name = normalize_worktree_name(worktree_name)?;
    let mut metadata = load_worktree_metadata(repo, &resolved_name)?;
    if let Some(value) = normalized_text(task_id) {
        metadata.insert("bound_task_id".to_string(), JsonValue::String(value));
    }
    if let Some(value) = normalized_text(change_id) {
        let canonical = ChangeJson::stateless().canonical_change_id(&value)?;
        let change_ref = ChangeJson::stateless().rolling_server_change_id(task_id, &canonical)?;
        metadata.insert("bound_change_id".to_string(), JsonValue::String(canonical));
        metadata.insert(
            "bound_change_ref".to_string(),
            JsonValue::String(change_ref),
        );
    }
    metadata.insert(
        "auto_created_for_task".to_string(),
        JsonValue::Bool(auto_created_for_task),
    );
    if auto_created_for_task {
        metadata.insert(
            "creation_kind".to_string(),
            JsonValue::String("task_auto_created".to_string()),
        );
        metadata.insert(
            "cleanup_policy".to_string(),
            JsonValue::String("after_remote_land".to_string()),
        );
    }
    if let Some(value) = normalized_text(target_base_line) {
        metadata.insert("target_base_line".to_string(), JsonValue::String(value));
    }
    if let Some(value) = normalized_text(fork_snapshot_id) {
        metadata.insert("fork_snapshot_id".to_string(), JsonValue::String(value));
    }
    if let Some(value) = normalized_text(forked_from_line) {
        metadata.insert("forked_from_line".to_string(), JsonValue::String(value));
    }
    metadata.insert(
        "last_used_at".to_string(),
        JsonValue::String(system_event_timestamp()),
    );
    save_worktree_metadata(repo, &resolved_name, &metadata)?;
    worktree_get(repo, Some(worktree_name), true)
}

pub(in crate::primitives) fn normalize_worktree_name(name: &str) -> Result<String, String> {
    let value = name.trim();
    if value.is_empty() {
        return Err("Worktree name must not be empty.".to_string());
    }
    if value.contains('/') || value.contains('\\') {
        return Err("Worktree name must not contain path separators.".to_string());
    }
    if matches!(value, "." | "..") {
        return Err("Worktree name must not be '.' or '..'.".to_string());
    }
    Ok(value.to_string())
}

pub(in crate::primitives) fn resolve_runtime_worktree_name(
    repo: &RepoRuntime,
    name: Option<&str>,
) -> Result<String, String> {
    if let Some(value) = normalized_text(name) {
        return normalize_worktree_name(&value);
    }
    if let Some(value) = repo
        .config
        .get("worktree_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
    {
        return normalize_worktree_name(&value);
    }
    let root_config = read_json_object_value(
        &repo
            .authoritative_repo_root()
            .join(".ait")
            .join("config.json"),
    );
    if let Some(value) = root_config
        .get("worktree_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
    {
        return normalize_worktree_name(&value);
    }
    Err("Worktree name is required outside a worktree context.".to_string())
}

pub(in crate::primitives) fn normalize_worktree_creation_kind(
    value: Option<&str>,
    default: &str,
) -> String {
    match normalized_text(value).as_deref() {
        Some(
            "task_auto_created" | "manual_add" | "bootstrap_helper" | "land_helper" | "scratch",
        ) => normalized_text(value).unwrap_or_else(|| default.to_string()),
        _ => default.to_string(),
    }
}

pub(in crate::primitives) fn default_worktree_cleanup_policy(creation_kind: &str) -> &'static str {
    match creation_kind {
        "task_auto_created" => "after_remote_land",
        "bootstrap_helper" | "land_helper" | "scratch" => "after_idle",
        _ => "manual_only",
    }
}

pub(in crate::primitives) fn normalize_worktree_cleanup_policy(
    value: Option<&str>,
    default: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(text) = normalized_text(value) else {
        return Ok(default.map(str::to_string));
    };
    match text.as_str() {
        "manual_only" | "after_remote_land" | "after_task_complete" | "after_idle" | "never" => {
            Ok(Some(text))
        }
        _ => Err(format!("Unsupported worktree cleanup_policy: {text}")),
    }
}

fn normalize_idle_duration(
    value: Option<&str>,
    option_name: &str,
    require_positive: bool,
) -> Result<(ChronoDuration, String), String> {
    let text = normalized_text(value).unwrap_or_else(|| "7d".to_string());
    let syntax_error = || format!("`{option_name}` must look like `7d`, `12h`, or `30m`.");
    let (unit_offset, unit) = text.char_indices().last().ok_or_else(&syntax_error)?;
    let unit = unit.to_ascii_lowercase();
    let count_text = &text[..unit_offset];
    let count = count_text.parse::<i64>().map_err(|_| syntax_error())?;
    if require_positive && count <= 0 {
        return Err(format!(
            "`{option_name}` must be greater than zero, such as `7d`, `12h`, or `30m`."
        ));
    }
    let delta = match unit {
        'd' => ChronoDuration::try_days(count),
        'h' => ChronoDuration::try_hours(count),
        'm' => ChronoDuration::try_minutes(count),
        _ => return Err(syntax_error()),
    }
    .ok_or_else(|| format!("`{option_name}` is outside the supported duration range."))?;
    Ok((delta, format!("{count}{unit}")))
}

pub(in crate::primitives) fn normalize_worktree_older_than(
    value: Option<&str>,
) -> Result<(ChronoDuration, String), String> {
    normalize_idle_duration(value, "--older-than", false)
}

pub(in crate::primitives) fn normalize_line_idle_for(
    value: Option<&str>,
) -> Result<(ChronoDuration, String), String> {
    normalize_idle_duration(value, "--idle-for", true)
}

pub(in crate::primitives) fn load_worktree_metadata(
    repo: &RepoRuntime,
    worktree_name: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    let payload = read_json_value(&worktree_registry_path(repo, worktree_name));
    let Some(obj) = payload.as_object() else {
        return Err(format!("Unknown worktree: {worktree_name}"));
    };
    if obj.is_empty() {
        return Err(format!("Unknown worktree: {worktree_name}"));
    }
    Ok(obj.clone())
}

pub(in crate::primitives) fn save_worktree_metadata(
    repo: &RepoRuntime,
    worktree_name: &str,
    payload: &JsonMap<String, JsonValue>,
) -> Result<(), String> {
    write_json_pretty(
        &worktree_registry_path(repo, worktree_name),
        &JsonValue::Object(payload.clone()),
    )
}

pub(in crate::primitives) fn worktree_metadata_with_defaults(
    payload: &JsonMap<String, JsonValue>,
) -> JsonMap<String, JsonValue> {
    let mut out = payload.clone();
    let auto_created = out
        .get("auto_created_for_task")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let creation_default = if auto_created {
        "task_auto_created"
    } else {
        "manual_add"
    };
    let creation_kind = normalize_worktree_creation_kind(
        out.get("creation_kind").and_then(JsonValue::as_str),
        creation_default,
    );
    let cleanup_policy = normalize_worktree_cleanup_policy(
        out.get("cleanup_policy").and_then(JsonValue::as_str),
        Some(default_worktree_cleanup_policy(&creation_kind)),
    )
    .ok()
    .flatten()
    .unwrap_or_else(|| default_worktree_cleanup_policy(&creation_kind).to_string());
    out.insert(
        "creation_kind".to_string(),
        JsonValue::String(creation_kind),
    );
    out.insert(
        "cleanup_policy".to_string(),
        JsonValue::String(cleanup_policy),
    );
    if let Some(last_used_at) =
        metadata_string(payload, "last_used_at").or_else(|| metadata_string(payload, "created_at"))
    {
        out.insert("last_used_at".to_string(), JsonValue::String(last_used_at));
    }
    let bound_task_id = metadata_string(payload, "bound_task_id");
    let raw_bound_change_id = metadata_string(payload, "bound_change_id");
    if let Some(canonical) = raw_bound_change_id
        .as_deref()
        .and_then(|value| ChangeJson::stateless().canonical_change_id(value).ok())
    {
        out.insert(
            "bound_change_id".to_string(),
            JsonValue::String(canonical.clone()),
        );
        if let Some(change_ref) = ChangeJson::stateless()
            .rolling_server_change_id(bound_task_id.as_deref(), &canonical)
            .ok()
            .or_else(|| metadata_string(payload, "bound_change_ref"))
        {
            out.insert(
                "bound_change_ref".to_string(),
                JsonValue::String(change_ref),
            );
        }
    }
    for key in [
        "fork_snapshot_id",
        "forked_from_line",
        "target_base_line",
        "last_retargeted_at",
        "rebase_started_at",
        "rebase_original_head_snapshot_id",
        "rebase_onto_snapshot_id",
        "bound_task_status",
        "bound_change_status",
    ] {
        match metadata_string(payload, key) {
            Some(value) => {
                out.insert(key.to_string(), JsonValue::String(value));
            }
            None => {
                out.insert(key.to_string(), JsonValue::Null);
            }
        }
    }
    if out.get("target_base_line").is_some_and(JsonValue::is_null) {
        if let Some(value) = metadata_string(payload, "forked_from_line") {
            out.insert("target_base_line".to_string(), JsonValue::String(value));
        }
    }
    let rebase_state =
        metadata_string(payload, "rebase_state").unwrap_or_else(|| "idle".to_string());
    let rebase_state = if matches!(rebase_state.as_str(), "idle" | "conflicted") {
        rebase_state
    } else {
        "idle".to_string()
    };
    out.insert("rebase_state".to_string(), JsonValue::String(rebase_state));
    out.insert(
        "rebase_conflict_paths".to_string(),
        JsonValue::Array(
            payload
                .get("rebase_conflict_paths")
                .and_then(JsonValue::as_array)
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|value| value.as_str().and_then(|text| normalized_text(Some(text))))
                .map(JsonValue::String)
                .collect(),
        ),
    );
    out
}

pub(in crate::primitives) fn metadata_string(
    payload: &JsonMap<String, JsonValue>,
    key: &str,
) -> Option<String> {
    payload
        .get(key)
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
}

type WorkspaceStatusCacheValue = (
    String,
    Option<bool>,
    Option<i64>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(in crate::primitives) fn status_cache_value(
    payload: &JsonMap<String, JsonValue>,
) -> Option<WorkspaceStatusCacheValue> {
    let cache = payload.get("workspace_status_cache")?.as_object()?;
    let workspace_status =
        metadata_string(cache, "workspace_status").unwrap_or_else(|| "unknown".to_string());
    let workspace_status = match workspace_status.as_str() {
        "clean" | "dirty" | "missing" | "detached" | "unknown" => workspace_status,
        _ => "unknown".to_string(),
    };
    let clean =
        cache
            .get("clean")
            .and_then(JsonValue::as_bool)
            .or(match workspace_status.as_str() {
                "clean" => Some(true),
                "dirty" => Some(false),
                _ => None,
            });
    let changed_count = cache.get("changed_count").and_then(JsonValue::as_i64);
    let modified_paths = json_string_list(cache.get("modified_paths"));
    let missing_paths = json_string_list(cache.get("missing_paths"));
    let untracked_paths = json_string_list(cache.get("untracked_paths"));
    Some((
        workspace_status,
        clean,
        changed_count,
        modified_paths,
        missing_paths,
        untracked_paths,
        metadata_string(cache, "current_line"),
        metadata_string(cache, "head_snapshot_id"),
        metadata_string(cache, "status_checked_at"),
    ))
}

pub(in crate::primitives) fn json_string_list(value: Option<&JsonValue>) -> Vec<String> {
    value
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| entry.as_str().and_then(|text| normalized_text(Some(text))))
        .collect()
}

pub(in crate::primitives) fn required_path_field(
    value: &JsonValue,
    key: &str,
) -> Result<PathBuf, String> {
    let text = required_string_field(value, key)?;
    Ok(PathBuf::from(text))
}

pub(in crate::primitives) fn discover_worktree_repo(path: &Path) -> Option<RepoRuntime> {
    if !path.is_dir() {
        return None;
    }
    if !path.join(WORKTREE_CONFIG_NAME).exists()
        || !path_exists_or_directory_link(&path.join(APP_DIR))
    {
        return None;
    }
    RepoRuntime::discover_from_path(path).ok()
}

pub(in crate::primitives) fn active_root_worktree_binding_name(
    repo: &RepoRuntime,
) -> Option<String> {
    let root_config = read_json_object_value(
        &repo
            .authoritative_repo_root()
            .join(".ait")
            .join("config.json"),
    );
    root_config
        .get("worktree_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .and_then(|value| normalize_worktree_name(&value).ok())
}

#[cfg(test)]
pub(in crate::primitives) fn worktree_local_task_for_worktree_with_task_store<S>(
    task_store: &S,
    task_id: Option<&str>,
) -> Result<Option<JsonValue>, String>
where
    S: TaskWorkflowTaskLister + ?Sized,
{
    let Some(task_id) = normalized_text(task_id) else {
        return Ok(None);
    };
    let rows =
        list_tasks_with_task_workflow_task_store(task_store).map_err(|err| err.to_string())?;
    Ok(rows
        .into_iter()
        .find(|row| string_field(row, "task_id").as_deref() == Some(task_id.as_str())))
}

#[cfg(test)]
pub(in crate::primitives) fn worktree_local_change_for_worktree_with_change_store<S>(
    change_store: &S,
    change_id: Option<&str>,
) -> Result<Option<JsonValue>, String>
where
    S: TaskWorkflowChangeLister + ?Sized,
{
    let Some(change_id) = normalized_text(change_id) else {
        return Ok(None);
    };
    let rows = list_changes_with_task_workflow_change_store(change_store)
        .map_err(|err| err.to_string())?;
    Ok(rows
        .into_iter()
        .find(|row| string_field(row, "change_id").as_deref() == Some(change_id.as_str())))
}

pub(in crate::primitives) fn snapshot_distance_if_ancestor(
    repo: &RepoRuntime,
    ancestor_snapshot_id: Option<&str>,
    snapshot_id: Option<&str>,
) -> Result<Option<i64>, String> {
    let store = snapshot_store(repo)?;
    snapshot_distance_if_ancestor_with_snapshot_store(&store, ancestor_snapshot_id, snapshot_id)
}

pub(in crate::primitives) fn snapshot_distance_if_ancestor_with_snapshot_store<S>(
    snapshot_store: &S,
    ancestor_snapshot_id: Option<&str>,
    snapshot_id: Option<&str>,
) -> Result<Option<i64>, String>
where
    S: SnapshotStore + ?Sized,
{
    snapshot_distance_if_ancestor_with_snapshot_store_and_cache(
        snapshot_store,
        ancestor_snapshot_id,
        snapshot_id,
        &mut SnapshotAncestorDistanceCache::default(),
    )
}

pub(in crate::primitives) fn snapshot_distance_if_ancestor_with_snapshot_store_and_cache<S>(
    snapshot_store: &S,
    ancestor_snapshot_id: Option<&str>,
    snapshot_id: Option<&str>,
    cache: &mut SnapshotAncestorDistanceCache,
) -> Result<Option<i64>, String>
where
    S: SnapshotStore + ?Sized,
{
    let Some(ancestor_snapshot_id) = normalized_text(ancestor_snapshot_id) else {
        return Ok(None);
    };
    let Some(snapshot_id) = normalized_text(snapshot_id) else {
        return Ok(None);
    };
    if ancestor_snapshot_id == snapshot_id {
        return Ok(Some(0));
    }
    Ok(snapshot_ancestor_distance_with_cache(
        snapshot_store,
        &ancestor_snapshot_id,
        &snapshot_id,
        SnapshotDagLimits::default(),
        cache,
    )?
    .map(|distance| distance as i64))
}

pub(in crate::primitives) fn effective_worktree_target_base_line(
    metadata: &JsonMap<String, JsonValue>,
    local_change: Option<&JsonValue>,
) -> Option<String> {
    metadata_string(metadata, "target_base_line")
        .or_else(|| local_change.and_then(|value| string_field(value, "base_line")))
        .or_else(|| metadata_string(metadata, "forked_from_line"))
}

pub(in crate::primitives) fn set_worktree_current_line(
    repo: &RepoRuntime,
    line_name: &str,
) -> Result<(), String> {
    let Some(worktree_config_path) = repo.worktree_config_path.as_ref() else {
        return Ok(());
    };
    let mut config = read_json_object_value(worktree_config_path);
    config.insert(
        "current_line".to_string(),
        JsonValue::String(line_name.to_string()),
    );
    config.entry("repo_root".to_string()).or_insert_with(|| {
        JsonValue::String(repo.authoritative_repo_root().to_string_lossy().to_string())
    });
    config
        .entry("workspace_root".to_string())
        .or_insert_with(|| JsonValue::String(repo.workspace_root().to_string_lossy().to_string()));
    write_json_pretty(worktree_config_path, &JsonValue::Object(config.clone()))?;
    if let Some(worktree_name) = config
        .get("worktree_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
    {
        let mut metadata = load_worktree_metadata(repo, &worktree_name)?;
        metadata.insert(
            "line_name".to_string(),
            JsonValue::String(line_name.to_string()),
        );
        metadata.insert(
            "path".to_string(),
            JsonValue::String(repo.workspace_root().to_string_lossy().to_string()),
        );
        metadata.insert(
            "repo_root".to_string(),
            JsonValue::String(repo.authoritative_repo_root().to_string_lossy().to_string()),
        );
        save_worktree_metadata(repo, &worktree_name, &metadata)?;
    }
    Ok(())
}

pub(in crate::primitives) fn set_runtime_current_line(
    repo: &RepoRuntime,
    line_name: &str,
) -> Result<(), String> {
    if repo.is_worktree() {
        return set_worktree_current_line(repo, line_name);
    }
    update_root_config(repo, |config| {
        config.insert(
            "current_line".to_string(),
            JsonValue::String(line_name.to_string()),
        );
    })
}
