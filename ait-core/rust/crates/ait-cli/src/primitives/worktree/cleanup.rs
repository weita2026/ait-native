use super::*;

pub fn worktree_cleanup_candidates(
    repo: &RepoRuntime,
    older_than: Option<&str>,
    cleanup_policy: Option<&str>,
    include_protected: bool,
    allow_manual_only: bool,
) -> Result<JsonValue, String> {
    let normalized_policy = normalize_worktree_cleanup_policy(cleanup_policy, None)?;
    let (_, older_than_label) = normalize_worktree_older_than(older_than)?;
    let rows = worktree_list(repo, false)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut candidates = Vec::new();
    let mut protected = Vec::new();
    let mut stale_rows = Vec::new();
    let mut inspected_count = 0_i64;
    let mut protected_count = 0_i64;

    for row in rows {
        let Some(_row_obj) = row.as_object() else {
            continue;
        };
        let Some(name) = string_field(&row, "name") else {
            continue;
        };
        let refreshed = worktree_get(repo, Some(&name), true)?;
        let Some(refreshed_obj) = refreshed.as_object() else {
            continue;
        };
        if normalized_policy.as_deref().is_some()
            && string_field(&refreshed, "cleanup_policy").as_deref() != normalized_policy.as_deref()
        {
            continue;
        }
        inspected_count += 1;
        let decision = worktree_cleanup_decision(
            repo,
            refreshed_obj,
            string_field(&refreshed, "workspace_status")
                .unwrap_or_else(|| "missing".to_string())
                .as_str(),
            refreshed
                .get("is_current")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false),
            Some(&older_than_label),
            allow_manual_only,
        )?;
        let mut enriched = refreshed_obj.clone();
        for (key, value) in decision {
            enriched.insert(key, value);
        }
        let enriched_value = JsonValue::Object(enriched.clone());
        match string_field(&JsonValue::Object(enriched.clone()), "cleanup_class").as_deref() {
            Some("stale") => stale_rows.push(JsonValue::Object(enriched)),
            Some("safe_auto_remove" | "safe_cleanup_candidate")
                if enriched_value
                    .get("cleanup_candidate")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false) =>
            {
                candidates.push(JsonValue::Object(enriched));
            }
            Some("protected") => {
                protected_count += 1;
                if include_protected {
                    protected.push(JsonValue::Object(enriched));
                }
            }
            _ => {}
        }
    }

    candidates.sort_by(|left, right| {
        cleanup_candidate_sort_key(left).cmp(&cleanup_candidate_sort_key(right))
    });
    protected.sort_by(|left, right| {
        (
            string_field(left, "protected_reason").unwrap_or_default(),
            string_field(left, "name").unwrap_or_default(),
        )
            .cmp(&(
                string_field(right, "protected_reason").unwrap_or_default(),
                string_field(right, "name").unwrap_or_default(),
            ))
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

    Ok(json!({
        "older_than": older_than_label,
        "cleanup_policy": normalized_policy,
        "include_protected": include_protected,
        "allow_manual_only": allow_manual_only,
        "inspected_count": inspected_count,
        "candidate_count": candidates.len(),
        "protected_count": protected_count,
        "stale_count": stale_rows.len(),
        "candidates": candidates,
        "protected": if include_protected { protected } else { Vec::new() },
        "stale_rows": stale_rows,
    }))
}

pub fn worktree_cleanup(
    repo: &RepoRuntime,
    older_than: Option<&str>,
    cleanup_policy: Option<&str>,
    allow_manual_only: bool,
    limit: Option<usize>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    let payload =
        worktree_cleanup_candidates(repo, older_than, cleanup_policy, false, allow_manual_only)?;
    let candidates = payload
        .get("candidates")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let selected = match limit {
        Some(value) => candidates.into_iter().take(value).collect::<Vec<_>>(),
        None => candidates,
    };
    let planned_rows = selected
        .iter()
        .map(|row| {
            json!({
                "name": string_field(row, "name"),
                "path": string_field(row, "path"),
                "current_line": string_field(row, "current_line")
                    .or_else(|| string_field(row, "registered_line_name")),
                "cleanup_class": string_field(row, "cleanup_class"),
                "cleanup_policy": string_field(row, "cleanup_policy"),
                "cleanup_reason": string_field(row, "cleanup_reason"),
                "deleted_path": true,
                "force": row.get("force_remove_dirty").and_then(JsonValue::as_bool).unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    if dry_run {
        return Ok(json!({
            "dry_run": true,
            "older_than": payload.get("older_than").cloned().unwrap_or(JsonValue::Null),
            "cleanup_policy": payload.get("cleanup_policy").cloned().unwrap_or(JsonValue::Null),
            "allow_manual_only": allow_manual_only,
            "candidate_count": payload.get("candidate_count").and_then(JsonValue::as_i64).unwrap_or(0),
            "planned_count": planned_rows.len(),
            "planned_rows": planned_rows,
            "removed_count": 0,
            "removed_rows": [],
        }));
    }
    let mut removed_rows = Vec::new();
    for row in selected {
        let name = required_string_field(&row, "name")?;
        let force = row
            .get("force_remove_dirty")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        removed_rows.push(remove_one_worktree(repo, &name, true, force)?);
    }
    Ok(json!({
        "dry_run": false,
        "older_than": payload.get("older_than").cloned().unwrap_or(JsonValue::Null),
        "cleanup_policy": payload.get("cleanup_policy").cloned().unwrap_or(JsonValue::Null),
        "allow_manual_only": allow_manual_only,
        "candidate_count": payload.get("candidate_count").and_then(JsonValue::as_i64).unwrap_or(0),
        "planned_count": planned_rows.len(),
        "planned_rows": planned_rows,
        "removed_count": removed_rows.len(),
        "removed_rows": removed_rows,
    }))
}

pub fn worktree_prune_stale(repo: &RepoRuntime, dry_run: bool) -> Result<JsonValue, String> {
    let rows = worktree_list(repo, true)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut pruned_rows = Vec::new();
    let mut live_count = 0_i64;
    for row in rows {
        let status = string_field(&row, "workspace_status").unwrap_or_default();
        if matches!(status.as_str(), "missing" | "detached") {
            pruned_rows.push(row);
        } else {
            live_count += 1;
        }
    }
    if !dry_run {
        for row in &pruned_rows {
            if let Some(name) = string_field(row, "name") {
                let _ = remove_one_worktree(repo, &name, false, false)?;
            }
        }
    }
    Ok(json!({
        "dry_run": dry_run,
        "pruned_count": pruned_rows.len(),
        "remaining_count": if dry_run { live_count + pruned_rows.len() as i64 } else { live_count },
        "pruned_rows": pruned_rows,
    }))
}

pub fn worktree_remove(
    repo: &RepoRuntime,
    names: &[String],
    all_stale: bool,
    delete_path: bool,
    force: bool,
    dry_run: bool,
) -> Result<JsonValue, String> {
    if all_stale {
        return worktree_prune_stale(repo, dry_run);
    }
    if names.is_empty() {
        return Err("Provide at least one worktree name.".to_string());
    }
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();
    for raw in names {
        let normalized = normalize_worktree_name(raw)?;
        if seen.insert(normalized.clone()) {
            ordered.push(normalized);
        }
    }
    for name in &ordered {
        guard_no_active_line_merge(repo, Some(name), "removing the worktree")?;
    }
    let removals = ordered
        .iter()
        .map(|name| preflight_worktree_removal(repo, name, force))
        .collect::<Result<Vec<_>, _>>()?;
    let planned_rows = removals
        .iter()
        .map(|removal| {
            json!({
                "name": string_field(removal, "name"),
                "path": string_field(removal, "path"),
                "workspace_status": string_field(removal, "workspace_status"),
                "deleted_path": delete_path,
            })
        })
        .collect::<Vec<_>>();
    if dry_run {
        return Ok(json!({
            "dry_run": true,
            "planned_count": planned_rows.len(),
            "planned_rows": planned_rows,
            "removed_count": 0,
            "removed_rows": [],
        }));
    }
    let removed_rows = ordered
        .iter()
        .map(|name| remove_one_worktree(repo, name, delete_path, force))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({
        "dry_run": false,
        "planned_count": planned_rows.len(),
        "planned_rows": planned_rows,
        "removed_count": removed_rows.len(),
        "removed_rows": removed_rows,
    }))
}

pub(in crate::primitives) fn worktree_cleanup_reason(
    creation_kind: &str,
    cleanup_policy: &str,
    older_than_label: &str,
) -> String {
    match cleanup_policy {
        "after_idle" => {
            let noun = if matches!(creation_kind, "bootstrap_helper" | "land_helper") {
                "helper worktree"
            } else {
                "worktree"
            };
            format!("clean {noun} idle for {older_than_label}")
        }
        "after_task_complete" => {
            "clean task-complete worktree eligible for explicit cleanup".to_string()
        }
        "after_remote_land" => {
            "clean task-bound worktree eligible for auto-remove after remote finish".to_string()
        }
        _ => format!("cleanup policy {cleanup_policy}"),
    }
}

pub(in crate::primitives) fn worktree_cleanup_decision(
    repo: &RepoRuntime,
    payload: &JsonMap<String, JsonValue>,
    status_label: &str,
    is_current: bool,
    older_than: Option<&str>,
    allow_manual_only: bool,
) -> Result<JsonMap<String, JsonValue>, String> {
    let metadata = worktree_metadata_with_defaults(payload);
    let creation_kind =
        metadata_string(&metadata, "creation_kind").unwrap_or_else(|| "manual_add".to_string());
    let cleanup_policy =
        metadata_string(&metadata, "cleanup_policy").unwrap_or_else(|| "manual_only".to_string());
    let last_used_at = metadata_string(&metadata, "last_used_at");
    let (older_than_delta, older_than_label) = normalize_worktree_older_than(older_than)?;
    let bound_task_id = metadata_string(&metadata, "bound_task_id");
    let bound_change_id = metadata_string(&metadata, "bound_change_id");
    let mut binding_summary = JsonMap::from_iter([
        ("active_root_binding".to_string(), JsonValue::Bool(false)),
        (
            "task_id".to_string(),
            bound_task_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        ("task_status".to_string(), JsonValue::Null),
        (
            "change_id".to_string(),
            bound_change_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        ("change_status".to_string(), JsonValue::Null),
    ]);

    if status_label == "missing" {
        return Ok(JsonMap::from_iter([
            (
                "creation_kind".to_string(),
                JsonValue::String(creation_kind),
            ),
            (
                "cleanup_policy".to_string(),
                JsonValue::String(cleanup_policy),
            ),
            (
                "last_used_at".to_string(),
                last_used_at
                    .clone()
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "cleanup_class".to_string(),
                JsonValue::String("stale".to_string()),
            ),
            ("cleanup_candidate".to_string(), JsonValue::Bool(false)),
            (
                "cleanup_reason".to_string(),
                JsonValue::String("missing worktree path".to_string()),
            ),
            ("protected_reason".to_string(), JsonValue::Null),
            (
                "manual_review_candidate".to_string(),
                JsonValue::Bool(false),
            ),
            ("manual_review_reason".to_string(), JsonValue::Null),
            ("force_remove_dirty".to_string(), JsonValue::Bool(false)),
            (
                "older_than".to_string(),
                JsonValue::String(older_than_label),
            ),
            (
                "binding_summary".to_string(),
                JsonValue::Object(binding_summary),
            ),
        ]));
    }
    if status_label == "detached" {
        return Ok(JsonMap::from_iter([
            (
                "creation_kind".to_string(),
                JsonValue::String(creation_kind),
            ),
            (
                "cleanup_policy".to_string(),
                JsonValue::String(cleanup_policy),
            ),
            (
                "last_used_at".to_string(),
                last_used_at
                    .clone()
                    .map(JsonValue::String)
                    .unwrap_or(JsonValue::Null),
            ),
            (
                "cleanup_class".to_string(),
                JsonValue::String("stale".to_string()),
            ),
            ("cleanup_candidate".to_string(), JsonValue::Bool(false)),
            (
                "cleanup_reason".to_string(),
                JsonValue::String("detached worktree layout".to_string()),
            ),
            ("protected_reason".to_string(), JsonValue::Null),
            (
                "manual_review_candidate".to_string(),
                JsonValue::Bool(false),
            ),
            ("manual_review_reason".to_string(), JsonValue::Null),
            ("force_remove_dirty".to_string(), JsonValue::Bool(false)),
            (
                "older_than".to_string(),
                JsonValue::String(older_than_label),
            ),
            (
                "binding_summary".to_string(),
                JsonValue::Object(binding_summary),
            ),
        ]));
    }

    let worktree_name = normalize_worktree_name(
        metadata_string(&metadata, "name")
            .as_deref()
            .unwrap_or("worktree"),
    )?;
    let active_root_binding =
        active_root_worktree_binding_name(repo) == Some(worktree_name.clone());
    let task_status = metadata_string(&metadata, "bound_task_status");
    let change_status = metadata_string(&metadata, "bound_change_status");
    binding_summary.insert(
        "active_root_binding".to_string(),
        JsonValue::Bool(active_root_binding),
    );
    binding_summary.insert(
        "task_status".to_string(),
        task_status
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );
    binding_summary.insert(
        "change_status".to_string(),
        change_status
            .clone()
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    );

    let task_closed = matches!(
        task_status.as_deref(),
        Some("completed" | "abandoned" | "later_promotion_excluded" | "canceled")
    );
    let change_closed = matches!(change_status.as_deref(), Some("landed" | "archived"));
    let active_task_binding = bound_task_id.is_some() && !task_closed;
    let active_change_binding = bound_change_id.is_some() && !change_closed;
    let closed_task_cleanup_candidate = bound_task_id.is_some()
        && matches!(
            task_status.as_deref(),
            Some("abandoned" | "canceled" | "later_promotion_excluded")
        );

    let clean = payload.get("clean").and_then(JsonValue::as_bool);
    let mut protected_reason = None::<String>;
    let mut manual_review_candidate = false;
    let mut manual_review_reason = None::<String>;
    let mut force_remove_dirty = false;
    if is_current {
        protected_reason = Some("current worktree".to_string());
    } else if clean.is_none() {
        protected_reason = Some("workspace status not verified".to_string());
    } else if closed_task_cleanup_candidate {
        force_remove_dirty = clean == Some(false);
    } else if active_root_binding {
        protected_reason = Some("active root-worktree binding target".to_string());
    } else if clean == Some(false) {
        protected_reason = Some("dirty worktree".to_string());
    } else if active_task_binding {
        protected_reason = Some("active task-bound worktree".to_string());
    } else if active_change_binding && !closed_task_cleanup_candidate {
        protected_reason = Some("active change-bound worktree".to_string());
    } else if cleanup_policy == "manual_only" {
        manual_review_candidate = true;
        manual_review_reason =
            Some("clean manual worktree requires explicit manual-only cleanup opt-in".to_string());
        if !allow_manual_only {
            protected_reason = Some("cleanup policy manual_only".to_string());
        }
    } else if cleanup_policy == "never" {
        protected_reason = Some("cleanup policy never".to_string());
    }

    let mut cleanup_class = "protected".to_string();
    let mut cleanup_candidate = false;
    let mut cleanup_reason = None::<String>;
    if protected_reason.is_none() {
        let idle_long_enough =
            Utc::now() - coerce_worktree_datetime(last_used_at.as_deref()) >= older_than_delta;
        if closed_task_cleanup_candidate {
            cleanup_class = "safe_auto_remove".to_string();
            cleanup_candidate = true;
            cleanup_reason = Some(
                if task_status.as_deref() == Some("later_promotion_excluded") {
                    "later-promotion-excluded task-bound worktree eligible for auto-remove"
                        .to_string()
                } else {
                    "abandoned task-bound worktree eligible for auto-remove".to_string()
                },
            );
        } else if cleanup_policy == "after_idle" {
            if idle_long_enough {
                cleanup_class = "safe_cleanup_candidate".to_string();
                cleanup_candidate = true;
                cleanup_reason = Some(worktree_cleanup_reason(
                    &creation_kind,
                    &cleanup_policy,
                    &older_than_label,
                ));
            } else {
                protected_reason = Some(format!("idle threshold {older_than_label} not reached"));
            }
        } else if cleanup_policy == "after_task_complete" {
            if bound_task_id.is_none() {
                protected_reason = Some(
                    "cleanup policy after_task_complete requires task-bound worktree".to_string(),
                );
            } else if task_closed && (bound_change_id.is_none() || change_closed) {
                cleanup_class = "safe_cleanup_candidate".to_string();
                cleanup_candidate = true;
                cleanup_reason = Some(worktree_cleanup_reason(
                    &creation_kind,
                    &cleanup_policy,
                    &older_than_label,
                ));
            } else {
                protected_reason = Some("task completion cleanup is not ready yet".to_string());
            }
        } else if cleanup_policy == "after_remote_land" {
            if creation_kind != "task_auto_created"
                && !metadata
                    .get("auto_created_for_task")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
            {
                protected_reason = Some(
                    "cleanup policy after_remote_land only applies to auto-created task worktrees"
                        .to_string(),
                );
            } else if bound_task_id.is_some()
                && task_closed
                && (bound_change_id.is_none() || change_closed)
            {
                cleanup_class = "safe_auto_remove".to_string();
                cleanup_candidate = true;
                cleanup_reason = Some(worktree_cleanup_reason(
                    &creation_kind,
                    &cleanup_policy,
                    &older_than_label,
                ));
            } else {
                protected_reason = Some("waiting for remote finish cleanup event".to_string());
            }
        } else if cleanup_policy == "manual_only" && allow_manual_only {
            cleanup_class = "safe_cleanup_candidate".to_string();
            cleanup_candidate = true;
            cleanup_reason =
                Some("clean manual worktree selected for explicit manual-only cleanup".to_string());
        } else {
            protected_reason = Some(format!(
                "cleanup policy {cleanup_policy} is not a candidate path"
            ));
        }
    }

    Ok(JsonMap::from_iter([
        (
            "creation_kind".to_string(),
            JsonValue::String(creation_kind),
        ),
        (
            "cleanup_policy".to_string(),
            JsonValue::String(cleanup_policy),
        ),
        (
            "last_used_at".to_string(),
            last_used_at
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "cleanup_class".to_string(),
            JsonValue::String(cleanup_class),
        ),
        (
            "cleanup_candidate".to_string(),
            JsonValue::Bool(cleanup_candidate),
        ),
        (
            "cleanup_reason".to_string(),
            cleanup_reason
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "protected_reason".to_string(),
            protected_reason
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "manual_review_candidate".to_string(),
            JsonValue::Bool(manual_review_candidate),
        ),
        (
            "manual_review_reason".to_string(),
            manual_review_reason
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "force_remove_dirty".to_string(),
            JsonValue::Bool(force_remove_dirty),
        ),
        (
            "older_than".to_string(),
            JsonValue::String(older_than_label),
        ),
        (
            "binding_summary".to_string(),
            JsonValue::Object(binding_summary),
        ),
    ]))
}

pub(in crate::primitives) fn cleanup_candidate_sort_key(
    value: &JsonValue,
) -> (i32, DateTime<Utc>, String) {
    (
        if string_field(value, "cleanup_class").as_deref() == Some("safe_auto_remove") {
            0
        } else {
            1
        },
        coerce_worktree_datetime(string_field(value, "last_used_at").as_deref()),
        string_field(value, "name").unwrap_or_default(),
    )
}

pub(in crate::primitives) fn coerce_worktree_datetime(value: Option<&str>) -> DateTime<Utc> {
    let Some(text) = normalized_text(value) else {
        return DateTime::parse_from_rfc3339("1970-01-01T00:00:00+00:00")
            .map(|value| value.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now());
    };
    DateTime::<FixedOffset>::parse_from_rfc3339(&text)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| {
            DateTime::parse_from_rfc3339("1970-01-01T00:00:00+00:00")
                .map(|value| value.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now())
        })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WorktreeRemovalWorkspaceStatusPolicy {
    Inspect,
    SkipForAuthoritativeTaskLand,
}

fn read_worktree_removal_workspace_status(
    worktree_repo: &RepoRuntime,
) -> Result<JsonValue, String> {
    workspace_delta_payload(
        worktree_repo,
        local_line_head_snapshot_id(worktree_repo, &worktree_repo.current_line_name()?)?.as_deref(),
        None,
    )
}

fn preflight_worktree_removal_with_status_reader<F>(
    repo: &RepoRuntime,
    name: &str,
    force: bool,
    workspace_status_policy: WorktreeRemovalWorkspaceStatusPolicy,
    read_workspace_status: F,
) -> Result<JsonValue, String>
where
    F: FnOnce(&RepoRuntime) -> Result<JsonValue, String>,
{
    if workspace_status_policy == WorktreeRemovalWorkspaceStatusPolicy::SkipForAuthoritativeTaskLand
        && !force
    {
        return Err(
            "Authoritative Task-finish worktree cleanup requires forced removal.".to_string(),
        );
    }
    let worktree_name = resolve_runtime_worktree_name(repo, Some(name))?;
    let metadata = load_worktree_metadata(repo, &worktree_name)?;
    let worktree_path = required_path_field(&JsonValue::Object(metadata.clone()), "path")?;
    if repo.workspace_root().canonicalize().ok() == worktree_path.canonicalize().ok() {
        return Err("Cannot remove the current worktree from inside itself.".to_string());
    }
    let worktree_status = match workspace_status_policy {
        WorktreeRemovalWorkspaceStatusPolicy::Inspect => {
            match discover_worktree_repo(&worktree_path) {
                Some(worktree_repo) => Some(read_workspace_status(&worktree_repo)?),
                None => None,
            }
        }
        WorktreeRemovalWorkspaceStatusPolicy::SkipForAuthoritativeTaskLand => None,
    };
    if let Some(status) = worktree_status.as_ref() {
        if !status
            .get("clean")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
            && !force
        {
            let sample = summarize_path_sample(&json_string_list(status.get("changed_paths")));
            return Err(format!(
                "Worktree {worktree_name} has unsaved changes: {sample}. Use --force to remove it."
            ));
        }
    }
    let mut removal = json!({
        "name": worktree_name,
        "path": worktree_path.to_string_lossy().to_string(),
        "metadata_path": worktree_registry_path(repo, &worktree_name).to_string_lossy().to_string(),
        "workspace_status": if workspace_status_policy == WorktreeRemovalWorkspaceStatusPolicy::SkipForAuthoritativeTaskLand {
            "not_evaluated"
        } else if worktree_status.as_ref().is_none_or(|value| value.get("clean").and_then(JsonValue::as_bool).unwrap_or(false)) {
            "clean"
        } else {
            "dirty"
        },
    });
    if workspace_status_policy == WorktreeRemovalWorkspaceStatusPolicy::SkipForAuthoritativeTaskLand
    {
        removal["workspace_status_evaluation"] = JsonValue::String("skipped".to_string());
        removal["workspace_status_reason"] =
            JsonValue::String("ready_remote_task_land_is_authoritative".to_string());
        removal["workspace_read_scope"] =
            JsonValue::String("bound_worktree_metadata_only".to_string());
    }
    Ok(removal)
}

fn preflight_worktree_removal_with_policy(
    repo: &RepoRuntime,
    name: &str,
    force: bool,
    workspace_status_policy: WorktreeRemovalWorkspaceStatusPolicy,
) -> Result<JsonValue, String> {
    preflight_worktree_removal_with_status_reader(
        repo,
        name,
        force,
        workspace_status_policy,
        read_worktree_removal_workspace_status,
    )
}

pub(in crate::primitives) fn preflight_worktree_removal(
    repo: &RepoRuntime,
    name: &str,
    force: bool,
) -> Result<JsonValue, String> {
    preflight_worktree_removal_with_policy(
        repo,
        name,
        force,
        WorktreeRemovalWorkspaceStatusPolicy::Inspect,
    )
}

pub(in crate::primitives) fn remove_one_worktree(
    repo: &RepoRuntime,
    name: &str,
    delete_path: bool,
    force: bool,
) -> Result<JsonValue, String> {
    remove_one_worktree_with_policy(
        repo,
        name,
        delete_path,
        force,
        WorktreeRemovalWorkspaceStatusPolicy::Inspect,
    )
}

pub(in crate::primitives) fn remove_one_worktree_after_authoritative_task_land(
    repo: &RepoRuntime,
    name: &str,
) -> Result<JsonValue, String> {
    remove_one_worktree_with_policy(
        repo,
        name,
        true,
        true,
        WorktreeRemovalWorkspaceStatusPolicy::SkipForAuthoritativeTaskLand,
    )
}

fn copy_worktree_removal_workspace_evaluation(source: &JsonValue, target: &mut JsonValue) {
    let Some(target_obj) = target.as_object_mut() else {
        return;
    };
    for key in [
        "workspace_status_evaluation",
        "workspace_status_reason",
        "workspace_read_scope",
    ] {
        if let Some(value) = source.get(key) {
            target_obj.insert(key.to_string(), value.clone());
        }
    }
}

fn worktree_cargo_lock_paths(cache_path: &Path) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let root_lease = cache_path.join(".ait-ci-build-lease");
    if root_lease.is_file() {
        paths.push(root_lease);
    }
    let mut entries = fs::read_dir(cache_path)
        .map_err(|err| err.to_string())?
        .map(|entry| entry.map_err(|err| err.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if !entry.file_type().map_err(|err| err.to_string())?.is_dir() {
            continue;
        }
        for lock_name in [".cargo-build-lock", ".cargo-lock", ".cargo-artifact-lock"] {
            let candidate = entry.path().join(lock_name);
            if candidate.is_file() {
                paths.push(candidate);
            }
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

pub(in crate::primitives) fn cleanup_registered_worktree_cargo_build_dir(
    worktree_path: &Path,
    worktree_name: &str,
) -> Result<JsonValue, String> {
    let Some(cache_path) = registered_worktree_cargo_build_dir(worktree_path, worktree_name) else {
        return Ok(json!({
            "status": "not_managed",
            "removed": false,
            "path": JsonValue::Null,
            "target_path": JsonValue::Null,
            "target_removed": false,
        }));
    };
    let target_path = registered_worktree_cargo_target_dir(worktree_path, worktree_name)
        .ok_or_else(|| {
            format!("Task Cargo target directory is not bound to worktree {worktree_name}.")
        })?;
    let cache_path_text = cache_path.to_string_lossy().to_string();
    let target_path_text = target_path.to_string_lossy().to_string();
    let cache_exists = path_exists_or_directory_link(&cache_path);
    let target_exists = path_exists_or_directory_link(&target_path);
    if !cache_exists && !target_exists {
        return Ok(json!({
            "status": "absent",
            "removed": false,
            "path": cache_path_text,
            "target_path": target_path_text,
            "target_removed": false,
        }));
    }
    for (path, label, exists) in [
        (&cache_path, "build cache", cache_exists),
        (&target_path, "target artifact directory", target_exists),
    ] {
        if !exists {
            continue;
        }
        let metadata = fs::symlink_metadata(path).map_err(|err| err.to_string())?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "Refusing to remove a Task Cargo {label} that is not a physical directory: {}",
                path.display()
            ));
        }
    }

    let mut locks = Vec::new();
    let mut lock_paths = Vec::new();
    if cache_exists {
        lock_paths.extend(worktree_cargo_lock_paths(&cache_path)?);
    }
    if target_exists {
        lock_paths.extend(worktree_cargo_lock_paths(&target_path)?);
    }
    lock_paths.sort();
    lock_paths.dedup();
    for lock_path in lock_paths {
        let file = match OpenOptions::new().read(true).write(true).open(&lock_path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "Failed to inspect Task Cargo build-cache lock {}: {error}",
                    lock_path.display()
                ));
            }
        };
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => locks.push(file),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(format!(
                    "Refusing to close Task worktree {worktree_name} while its Cargo target/build state is active through {}.",
                    lock_path.display()
                ));
            }
            Err(error) => {
                return Err(format!(
                    "Failed to lock Task Cargo target/build state for {} through {}: {error}",
                    worktree_name,
                    lock_path.display()
                ));
            }
        }
    }
    if target_exists {
        remove_tree_force(&target_path)?;
    }
    if cache_exists {
        remove_tree_force(&cache_path)?;
    }
    drop(locks);
    Ok(json!({
        "status": "removed",
        "removed": true,
        "path": cache_path_text,
        "target_path": target_path_text,
        "target_removed": target_exists,
    }))
}

pub(in crate::primitives) fn finalize_promoted_worktree_registration(
    repo: &RepoRuntime,
    worktree_name: &str,
    original_path: &Path,
    promoted_path: &Path,
    cargo_build_cache_cleanup: JsonValue,
) -> Result<JsonValue, String> {
    let registry_path = worktree_registry_path(repo, worktree_name);
    if !registry_path.is_file() {
        return Ok(json!({
            "name": worktree_name,
            "path": original_path.to_string_lossy().to_string(),
            "promoted_path": promoted_path.to_string_lossy().to_string(),
            "status": "already_consumed",
            "removed": true,
            "deleted_path": false,
            "cargo_build_cache_cleanup": cargo_build_cache_cleanup,
        }));
    }
    let metadata = load_worktree_metadata(repo, worktree_name)?;
    let registered_path = required_path_field(&JsonValue::Object(metadata.clone()), "path")?;
    if resolve_path_strict_false(&registered_path) != resolve_path_strict_false(original_path) {
        return Err(format!(
            "Refusing to consume worktree registration {worktree_name}: registered path {} does not match promoted source {}.",
            registered_path.display(),
            original_path.display()
        ));
    }
    let alias_path = metadata_string(&metadata, "alias_path").map(PathBuf::from);
    if let Some(alias_path) = alias_path.as_ref() {
        if path_exists_or_directory_link(alias_path) {
            remove_path_entry(alias_path)?;
        }
    }
    fs::remove_file(&registry_path).map_err(|err| err.to_string())?;
    update_root_config(repo, |config| {
        if config.get("worktree_name").and_then(JsonValue::as_str) == Some(worktree_name) {
            config.remove("worktree_name");
        }
    })?;
    Ok(json!({
        "name": worktree_name,
        "path": original_path.to_string_lossy().to_string(),
        "promoted_path": promoted_path.to_string_lossy().to_string(),
        "alias_path": alias_path.map(|value| value.to_string_lossy().to_string()),
        "status": "promoted_and_consumed",
        "removed": true,
        "deleted_path": false,
        "cargo_build_cache_cleanup": cargo_build_cache_cleanup,
    }))
}

fn remove_one_worktree_with_policy(
    repo: &RepoRuntime,
    name: &str,
    delete_path: bool,
    force: bool,
    workspace_status_policy: WorktreeRemovalWorkspaceStatusPolicy,
) -> Result<JsonValue, String> {
    let removal =
        preflight_worktree_removal_with_policy(repo, name, force, workspace_status_policy)?;
    let worktree_name = required_string_field(&removal, "name")?;
    let worktree_path = required_path_field(&removal, "path")?;
    let metadata = load_worktree_metadata(repo, &worktree_name)?;
    let alias_path = metadata_string(&metadata, "alias_path").map(PathBuf::from);
    let marker_path = worktree_path.join(WORKTREE_CONFIG_NAME);
    let ait_link = worktree_path.join(APP_DIR);
    let cargo_build_cache_cleanup = if delete_path {
        cleanup_registered_worktree_cargo_build_dir(&worktree_path, &worktree_name)?
    } else {
        json!({
            "status": "not_requested",
            "removed": false,
            "path": JsonValue::Null,
        })
    };
    if marker_path.exists() {
        fs::remove_file(&marker_path).map_err(|err| err.to_string())?;
    }
    if path_exists_or_directory_link(&ait_link) {
        remove_path_entry(&ait_link)?;
    }
    if delete_path && worktree_path.exists() {
        let mut children = fs::read_dir(&worktree_path)
            .map_err(|err| err.to_string())?
            .map(|entry| {
                entry
                    .map(|value| value.path())
                    .map_err(|err| err.to_string())
            })
            .collect::<Result<Vec<_>, _>>()?;
        children.sort_by(|left, right| right.cmp(left));
        for child in children {
            remove_tree_force(&child)?;
        }
        if worktree_path.exists() {
            fs::remove_dir(&worktree_path).map_err(|err| err.to_string())?;
        }
    }
    if let Some(alias_path) = alias_path.as_ref() {
        if path_exists_or_directory_link(alias_path) {
            remove_path_entry(alias_path)?;
        }
    }
    fs::remove_file(worktree_registry_path(repo, &worktree_name)).map_err(|err| err.to_string())?;
    update_root_config(repo, |config| {
        if config.get("worktree_name").and_then(JsonValue::as_str) == Some(worktree_name.as_str()) {
            config.remove("worktree_name");
        }
    })?;
    let mut output = json!({
        "name": worktree_name,
        "path": worktree_path.to_string_lossy().to_string(),
        "alias_path": alias_path.map(|value| value.to_string_lossy().to_string()),
        "removed": true,
        "deleted_path": delete_path,
        "workspace_status": string_field(&removal, "workspace_status"),
        "cargo_build_cache_cleanup": cargo_build_cache_cleanup,
    });
    copy_worktree_removal_workspace_evaluation(&removal, &mut output);
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::init_surface::{init_repo, InitRequest};
    use std::cell::Cell;
    use tempfile::tempdir;

    #[test]
    fn metadata_cleanup_decision_does_not_probe_workflow_retired_backend() {
        let temp = tempdir().expect("repo tempdir");
        let repo_root = temp.path().join("repo");
        fs::create_dir_all(repo_root.join(APP_DIR)).expect("create authority directory");
        fs::write(
            repo_root.join(APP_DIR).join("config.json"),
            json!({
                "repo_name": "fixture-ait",
                "default_line": "main",
                "current_line": "main",
            })
            .to_string(),
        )
        .expect("write config");
        let repo = RepoRuntime::discover_from_path(&repo_root).expect("repo runtime");
        let metadata = json!({
            "name": "rct-1",
            "path": repo_root.join("rct-1").to_string_lossy().to_string(),
            "bound_task_id": "RCT-1",
            "bound_change_id": "RCT-1/C-01",
            "auto_created_for_task": true,
            "creation_kind": "task_auto_created",
            "cleanup_policy": "after_remote_land",
            "created_at": "2026-07-15T00:00:00Z",
        });
        let decision = worktree_cleanup_decision(
            &repo,
            metadata.as_object().expect("metadata object"),
            "unknown",
            false,
            None,
            false,
        )
        .expect("metadata-only cleanup decision");

        assert_eq!(decision["cleanup_candidate"], json!(false));
        assert_eq!(decision["binding_summary"]["task_id"], json!("RCT-1"));
        assert_eq!(decision["binding_summary"]["task_status"], JsonValue::Null);
        assert_eq!(
            decision["binding_summary"]["change_status"],
            JsonValue::Null
        );
    }

    #[test]
    fn authoritative_task_land_removal_preflight_skips_workspace_status_reader() {
        let temp = tempdir().expect("repo tempdir");
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("fixture-ait".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .expect("init repo");
        let repo = RepoRuntime::discover_from_path(temp.path()).expect("repo runtime");
        let worktree_name = "rt-authoritative";
        let worktree_path = temp.path().join(worktree_name);
        fs::create_dir_all(&worktree_path).expect("create worktree path");
        std::os::unix::fs::symlink(temp.path().join(APP_DIR), worktree_path.join(APP_DIR))
            .expect("link worktree authority");
        fs::write(
            worktree_path.join(WORKTREE_CONFIG_NAME),
            json!({
                "current_line": "feature/rt-authoritative",
                "repo_root": temp.path().to_string_lossy().to_string(),
                "workspace_root": worktree_path.to_string_lossy().to_string(),
                "worktree_name": worktree_name,
            })
            .to_string(),
        )
        .expect("write worktree marker");
        fs::write(
            worktree_registry_path(&repo, worktree_name),
            json!({
                "name": worktree_name,
                "path": worktree_path.to_string_lossy().to_string(),
                "repo_root": temp.path().to_string_lossy().to_string(),
            })
            .to_string(),
        )
        .expect("write worktree metadata");

        let inspected = Cell::new(false);
        let error = preflight_worktree_removal_with_status_reader(
            &repo,
            worktree_name,
            false,
            WorktreeRemovalWorkspaceStatusPolicy::Inspect,
            |_| {
                inspected.set(true);
                Ok(json!({
                    "clean": false,
                    "changed_paths": ["src/lib.rs"],
                }))
            },
        )
        .expect_err("ordinary removal must protect dirty worktrees");
        assert!(inspected.get());
        assert!(error.contains("unsaved changes: src/lib.rs"));

        let inspected = Cell::new(false);
        let removal = preflight_worktree_removal_with_status_reader(
            &repo,
            worktree_name,
            true,
            WorktreeRemovalWorkspaceStatusPolicy::SkipForAuthoritativeTaskLand,
            |_| {
                inspected.set(true);
                Err("full workspace status reader must not run".to_string())
            },
        )
        .expect("authoritative task land preflight");
        assert!(!inspected.get());
        assert_eq!(removal["workspace_status"], json!("not_evaluated"));
        assert_eq!(removal["workspace_status_evaluation"], json!("skipped"));
        assert_eq!(
            removal["workspace_status_reason"],
            json!("ready_remote_task_land_is_authoritative")
        );
        assert_eq!(
            removal["workspace_read_scope"],
            json!("bound_worktree_metadata_only")
        );
    }

    fn write_removable_task_worktree(
        repo: &RepoRuntime,
        worktree_name: &str,
        worktree_path: &Path,
    ) {
        fs::create_dir_all(worktree_path).expect("create worktree path");
        std::os::unix::fs::symlink(
            repo.authoritative_repo_root().join(APP_DIR),
            worktree_path.join(APP_DIR),
        )
        .expect("link worktree authority");
        fs::write(
            worktree_path.join(WORKTREE_CONFIG_NAME),
            json!({
                "current_line": format!("feature/{worktree_name}"),
                "repo_root": repo.authoritative_repo_root().to_string_lossy().to_string(),
                "workspace_root": worktree_path.to_string_lossy().to_string(),
                "worktree_name": worktree_name,
            })
            .to_string(),
        )
        .expect("write worktree marker");
        fs::write(
            worktree_registry_path(repo, worktree_name),
            json!({
                "name": worktree_name,
                "path": worktree_path.to_string_lossy().to_string(),
                "repo_root": repo.authoritative_repo_root().to_string_lossy().to_string(),
            })
            .to_string(),
        )
        .expect("write worktree metadata");
    }

    #[test]
    fn terminal_worktree_removal_reclaims_exact_idle_cargo_build_cache() {
        let temp = tempdir().expect("repo tempdir");
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("fixture-ait".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .expect("init repo");
        let repo = RepoRuntime::discover_from_path(temp.path()).expect("repo runtime");
        let worktree_name = "rt-idle-cache";
        let worktree_path = temp.path().join(worktree_name);
        write_removable_task_worktree(&repo, worktree_name, &worktree_path);
        let cache_path = registered_worktree_cargo_build_dir(&worktree_path, worktree_name)
            .expect("registered cache path");
        let target_path = registered_worktree_cargo_target_dir(&worktree_path, worktree_name)
            .expect("registered target path");
        let profile_path = cache_path.join("release");
        fs::create_dir_all(&profile_path).expect("cache profile");
        fs::write(profile_path.join(".cargo-build-lock"), "").expect("Cargo build lock");
        fs::write(profile_path.join("intermediate"), "bytes").expect("cache artifact");
        let target_profile_path = target_path.join("release");
        fs::create_dir_all(&target_profile_path).expect("target profile");
        fs::write(target_profile_path.join(".cargo-artifact-lock"), "")
            .expect("Cargo artifact lock");
        fs::write(target_profile_path.join("ait-cli"), "binary").expect("Task final artifact");

        let removal = remove_one_worktree_after_authoritative_task_land(&repo, worktree_name)
            .expect("terminal worktree removal");

        assert_eq!(
            removal["cargo_build_cache_cleanup"]["status"],
            json!("removed")
        );
        assert_eq!(removal["cargo_build_cache_cleanup"]["removed"], json!(true));
        assert_eq!(
            removal["cargo_build_cache_cleanup"]["target_removed"],
            json!(true)
        );
        assert!(!cache_path.exists());
        assert!(!target_path.exists());
        assert!(!worktree_path.exists());
    }

    #[test]
    fn terminal_worktree_removal_fails_closed_with_active_profile_lock_and_retries() {
        let temp = tempdir().expect("repo tempdir");
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("fixture-ait".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .expect("init repo");
        let repo = RepoRuntime::discover_from_path(temp.path()).expect("repo runtime");
        let worktree_name = "rt-active-cache";
        let worktree_path = temp.path().join(worktree_name);
        write_removable_task_worktree(&repo, worktree_name, &worktree_path);
        let cache_path = registered_worktree_cargo_build_dir(&worktree_path, worktree_name)
            .expect("registered cache path");
        let profile_path = cache_path.join("ait-ci");
        fs::create_dir_all(&profile_path).expect("cache profile");
        let lock_path = profile_path.join(".cargo-build-lock");
        fs::write(&lock_path, "").expect("Cargo build lock");
        let active_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("open active lock");
        FileExt::lock_exclusive(&active_lock).expect("hold active Cargo lock");

        let error = remove_one_worktree_after_authoritative_task_land(&repo, worktree_name)
            .expect_err("active Cargo cache must block terminal worktree removal");

        assert!(error.contains("while its Cargo target/build state is active"));
        assert!(cache_path.exists());
        assert!(worktree_path.exists());
        assert!(worktree_registry_path(&repo, worktree_name).is_file());
        FileExt::unlock(&active_lock).expect("release active Cargo lock");

        let removal = remove_one_worktree_after_authoritative_task_land(&repo, worktree_name)
            .expect("retry terminal worktree removal");
        assert_eq!(
            removal["cargo_build_cache_cleanup"]["status"],
            json!("removed")
        );
        assert!(!cache_path.exists());
        assert!(!worktree_path.exists());
        assert!(!worktree_registry_path(&repo, worktree_name).exists());
    }

    #[test]
    fn terminal_worktree_removal_fails_closed_with_active_target_lock_and_retries() {
        let temp = tempdir().expect("repo tempdir");
        init_repo(&InitRequest {
            root: temp.path().to_path_buf(),
            name: Some("fixture-ait".to_string()),
            default_line: "main".to_string(),
            policy_profile: "prototype".to_string(),
            default_author_mode: "ai_with_human_review".to_string(),
            default_model: None,
            repair_existing: false,
        })
        .expect("init repo");
        let repo = RepoRuntime::discover_from_path(temp.path()).expect("repo runtime");
        let worktree_name = "rt-active-target";
        let worktree_path = temp.path().join(worktree_name);
        write_removable_task_worktree(&repo, worktree_name, &worktree_path);
        let target_path = registered_worktree_cargo_target_dir(&worktree_path, worktree_name)
            .expect("registered target path");
        let profile_path = target_path.join("release");
        fs::create_dir_all(&profile_path).expect("target profile");
        let lock_path = profile_path.join(".cargo-artifact-lock");
        fs::write(&lock_path, "").expect("Cargo artifact lock");
        let active_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("open active lock");
        FileExt::lock_exclusive(&active_lock).expect("hold active Cargo artifact lock");

        let error = remove_one_worktree_after_authoritative_task_land(&repo, worktree_name)
            .expect_err("active Cargo target must block terminal worktree removal");

        assert!(error.contains("while its Cargo target/build state is active"));
        assert!(target_path.exists());
        assert!(worktree_path.exists());
        assert!(worktree_registry_path(&repo, worktree_name).is_file());
        FileExt::unlock(&active_lock).expect("release active Cargo artifact lock");

        let removal = remove_one_worktree_after_authoritative_task_land(&repo, worktree_name)
            .expect("retry terminal worktree removal");
        assert_eq!(
            removal["cargo_build_cache_cleanup"]["target_removed"],
            json!(true)
        );
        assert!(!target_path.exists());
        assert!(!worktree_path.exists());
        assert!(!worktree_registry_path(&repo, worktree_name).exists());
    }
}
