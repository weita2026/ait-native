use super::*;
use crate::runtime::SNAPSHOT_BINARY_DB_WRITE_LAYOUT;
use ait_core::local_snapshot::{LocalSnapshotBlobReadStore, LocalSnapshotTreeReadStore};

pub fn worktree_restore_owned_head(
    repo: &RepoRuntime,
    name: Option<&str>,
    dry_run: bool,
) -> Result<JsonValue, String> {
    guard_no_active_line_merge(repo, name, "restoring the owned worktree head")?;
    let worktree_name = resolve_runtime_worktree_name(repo, name)?;
    let metadata = worktree_metadata_with_defaults(&load_worktree_metadata(repo, &worktree_name)?);
    if metadata_string(&metadata, "rebase_state").as_deref() == Some("conflicted") {
        return Err(format!(
            "Worktree {worktree_name} is in a conflicted rebase. Use `ait worktree rebase --continue` or `--abort`."
        ));
    }
    let worktree_path = required_path_field(&JsonValue::Object(metadata.clone()), "path")?;
    let Some(worktree_repo) = discover_worktree_repo(&worktree_path) else {
        return Err(format!("Worktree is missing or detached: {worktree_name}"));
    };
    let bound_task_id = metadata_string(&metadata, "bound_task_id").ok_or_else(|| {
        format!(
            "Worktree {worktree_name} is not task-bound; `ait worktree restore-owned-head` only supports task worktrees."
        )
    })?;
    let bound_change_id = metadata_string(&metadata, "bound_change_id");
    let current_line_name = worktree_repo.current_line_name()?;
    let current_head_snapshot_id = local_line_head_snapshot_id(&worktree_repo, &current_line_name)?
        .ok_or_else(|| format!("Worktree {worktree_name} has no current line head to restore."))?;
    let fork_snapshot_id = metadata_string(&metadata, "fork_snapshot_id").ok_or_else(|| {
        format!("Worktree {worktree_name} has no registered fork snapshot to restore against.")
    })?;
    let store = snapshot_store(&worktree_repo)?;
    // Restoring one line head intentionally follows its primary-parent
    // history. Reachability and rebase-base selection elsewhere use all
    // parents; this operation needs one unambiguous anchor to restore.
    let chain = snapshot_first_parent_chain(
        &store,
        &current_head_snapshot_id,
        None,
        SnapshotDagLimits::default(),
    )?;
    let Some(fork_index) = chain.iter().position(|value| value == &fork_snapshot_id) else {
        return Err(format!(
            "Current line head `{current_head_snapshot_id}` does not descend from registered fork `{fork_snapshot_id}`. Rebase, recreate, or retarget the bound worktree before running `ait worktree restore-owned-head`."
        ));
    };
    let workspace = workspace_delta_payload(&worktree_repo, Some(&current_head_snapshot_id), None)?;
    if !workspace
        .get("clean")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let changed_paths = json_string_list(workspace.get("changed_paths"));
        let sample = summarize_path_sample(&changed_paths);
        return Err(format!(
            "Worktree {worktree_name} has unsaved changes relative to current head `{current_head_snapshot_id}`: {sample}"
        ));
    }
    let head_segment = chain[fork_index + 1..].to_vec();
    let ownership_rows = snapshot_ownership_rows(&worktree_repo, &head_segment)?;
    let ownership_index = ownership_rows
        .iter()
        .filter_map(|row| {
            string_field(row, "snapshot_id").map(|snapshot_id| (snapshot_id, row.clone()))
        })
        .collect::<BTreeMap<_, _>>();

    let mut restore_anchor_snapshot_id = fork_snapshot_id.clone();
    let mut first_foreign_snapshot_id = None::<String>;
    let mut dropped_snapshots = Vec::new();
    for snapshot_id in &head_segment {
        let ownership = snapshot_owned_by_bound_slice(
            ownership_index.get(snapshot_id),
            snapshot_id,
            &bound_task_id,
            bound_change_id.as_deref(),
            Some(&worktree_name),
        );
        if let Some(foreign_root) = first_foreign_snapshot_id.as_ref() {
            let mut dropped = ownership.clone();
            dropped.insert(
                "reason".to_string(),
                JsonValue::String(
                    ownership
                        .get("reason")
                        .and_then(JsonValue::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| {
                            format!("descends from foreign snapshot {foreign_root}")
                        }),
                ),
            );
            dropped.insert(
                "foreign_root_snapshot_id".to_string(),
                JsonValue::String(foreign_root.clone()),
            );
            dropped_snapshots.push(JsonValue::Object(dropped));
            continue;
        }
        if ownership
            .get("owned")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            restore_anchor_snapshot_id = snapshot_id.clone();
            continue;
        }
        first_foreign_snapshot_id = Some(snapshot_id.clone());
        let mut dropped = ownership.clone();
        dropped.insert(
            "foreign_root_snapshot_id".to_string(),
            JsonValue::String(snapshot_id.clone()),
        );
        dropped_snapshots.push(JsonValue::Object(dropped));
    }

    let mut restore_details = json!({
        "worktree_name": worktree_name,
        "path": worktree_path.to_string_lossy().to_string(),
        "task_id": bound_task_id,
        "change_id": bound_change_id,
        "line_name": current_line_name,
        "dry_run": dry_run,
        "foreign_detected": !dropped_snapshots.is_empty(),
        "fork_snapshot_id": fork_snapshot_id,
        "current_head_snapshot_id_before": current_head_snapshot_id,
        "materialized_snapshot_id_before": worktree_repo
            .config
            .get("materialized_snapshot_id")
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value))),
        "restored_snapshot_id": restore_anchor_snapshot_id,
        "dropped_snapshots": dropped_snapshots,
        "noop": dropped_snapshots.is_empty(),
    });
    if dropped_snapshots.is_empty() {
        let mut summary = worktree_get(repo, Some(&worktree_name), true)?;
        summary
            .as_object_mut()
            .expect("restore owned head summary")
            .insert("restore_owned_head".to_string(), restore_details);
        return Ok(summary);
    }

    let restore_data = restore_workspace_all(
        &worktree_repo,
        Some(&restore_anchor_snapshot_id),
        Some(&current_head_snapshot_id),
        false,
        dry_run,
    )?;
    restore_details
        .as_object_mut()
        .expect("restore owned head details")
        .insert("restore".to_string(), restore_data);
    if dry_run {
        let mut summary = worktree_get(repo, Some(&worktree_name), true)?;
        summary
            .as_object_mut()
            .expect("restore owned head dry-run summary")
            .insert("restore_owned_head".to_string(), restore_details);
        return Ok(summary);
    }

    materialize_worktree_docs_symlink(&repo.authoritative_repo_root(), &worktree_path)?;
    materialize_worktree_cargo_config(&repo.authoritative_repo_root(), &worktree_path)?;
    set_local_line_head(
        &worktree_repo,
        &current_line_name,
        Some(&restore_anchor_snapshot_id),
    )?;
    worktree_repo.set_worktree_materialized_snapshot(Some(&restore_anchor_snapshot_id))?;
    let restored_at = system_event_timestamp();
    let mut updated = load_worktree_metadata(repo, &worktree_name)?;
    updated.insert(
        "line_name".to_string(),
        JsonValue::String(current_line_name.clone()),
    );
    updated.insert("last_used_at".to_string(), JsonValue::String(restored_at));
    save_worktree_metadata(repo, &worktree_name, &updated)?;
    let mut summary = worktree_get(repo, Some(&worktree_name), true)?;
    summary
        .as_object_mut()
        .expect("restore owned head summary")
        .insert("restore_owned_head".to_string(), restore_details);
    Ok(summary)
}

pub fn worktree_preview_rebase(
    repo: &RepoRuntime,
    name: Option<&str>,
    onto_line_name: Option<&str>,
) -> Result<JsonValue, String> {
    let prepared = prepare_worktree_rebase(repo, name, onto_line_name, false)?;
    let mut summary = worktree_get(repo, Some(&prepared.worktree_name), true)?;
    summary
        .as_object_mut()
        .expect("worktree rebase preview summary")
        .insert(
            "rebase".to_string(),
            json!({
                "worktree_name": prepared.worktree_name,
                "path": prepared.worktree_path.to_string_lossy().to_string(),
                "dry_run": true,
                "line_name": prepared.line_name,
                "onto_line_name": prepared.onto_line_name,
                "old_base_snapshot_id": prepared.old_base_snapshot_id,
                "old_head_snapshot_id": prepared.old_head_snapshot_id,
                "new_base_snapshot_id": prepared.new_base_snapshot_id,
                "rewrites_ancestry": prepared.rewrites_ancestry,
                "feature_delta_count": prepared.plan.feature_delta_count,
                "conflict_count": prepared.plan.conflict_paths.len(),
                "conflict_paths": prepared.plan.conflict_paths,
                "apply_write_paths": prepared.plan.apply_write_paths,
                "apply_remove_paths": prepared.plan.apply_remove_paths,
                "files": prepared.plan.files,
                "would_fast_forward": prepared.plan.would_fast_forward,
            }),
        );
    Ok(summary)
}

pub fn worktree_rebase(
    repo: &RepoRuntime,
    name: Option<&str>,
    onto_line_name: Option<&str>,
) -> Result<JsonValue, String> {
    let prepared = prepare_worktree_rebase(repo, name, onto_line_name, false)?;
    let workspace = worktree_get(repo, Some(&prepared.worktree_name), true)?;
    if !workspace
        .get("clean")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let changed_paths = json_string_list(workspace.get("changed_paths"));
        let sample = summarize_path_sample(&changed_paths);
        return Err(format!(
            "Worktree {} has unsaved changes relative to current head `{}`: {}",
            prepared.worktree_name, prepared.old_head_snapshot_id, sample,
        ));
    }
    let materialized_snapshot_id = prepared
        .worktree_repo
        .config
        .get("materialized_snapshot_id")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)));
    if materialized_snapshot_id.as_deref() != Some(prepared.old_head_snapshot_id.as_str()) {
        // The workspace has already been verified clean against the line head
        // above. If only the materialized marker is stale, repair metadata
        // directly instead of restoring over the intentional feature delta.
        prepared
            .worktree_repo
            .set_worktree_materialized_snapshot(Some(&prepared.old_head_snapshot_id))?;
    }
    if prepared.old_base_snapshot_id == prepared.new_base_snapshot_id && !prepared.rewrites_ancestry
    {
        update_worktree_metadata_fields(
            repo,
            &prepared.worktree_name,
            &[
                (
                    "fork_snapshot_id",
                    JsonValue::String(prepared.new_base_snapshot_id.clone()),
                ),
                (
                    "forked_from_line",
                    JsonValue::String(prepared.onto_line_name.clone()),
                ),
                (
                    "target_base_line",
                    JsonValue::String(prepared.onto_line_name.clone()),
                ),
                (
                    "last_retargeted_at",
                    JsonValue::String(system_event_timestamp()),
                ),
                ("rebase_state", JsonValue::String("idle".to_string())),
                ("rebase_conflict_paths", JsonValue::Array(Vec::new())),
            ],
            &[
                "rebase_started_at",
                "rebase_original_head_snapshot_id",
                "rebase_onto_snapshot_id",
            ],
        )?;
        let mut summary = worktree_get(repo, Some(&prepared.worktree_name), true)?;
        summary
            .as_object_mut()
            .expect("worktree noop rebase summary")
            .insert(
                "rebase".to_string(),
                rebase_plan_payload(&prepared, json!({"status": "noop"})),
            );
        return Ok(summary);
    }

    restore_workspace_all(
        &prepared.worktree_repo,
        Some(&prepared.new_base_snapshot_id),
        Some(&prepared.old_head_snapshot_id),
        false,
        false,
    )?;
    prepared
        .worktree_repo
        .set_worktree_materialized_snapshot(Some(&prepared.new_base_snapshot_id))?;
    for entry in &prepared.plan.files {
        let entry_path = required_string_field(entry, "path")?;
        match string_field(entry, "apply_status").as_deref() {
            Some("write") => write_workspace_snapshot_row(
                &prepared.worktree_repo,
                &entry_path,
                entry.get("feature"),
            )?,
            Some("remove") => {
                write_workspace_snapshot_row(&prepared.worktree_repo, &entry_path, None)?
            }
            _ => {}
        }
    }
    if !prepared.plan.conflict_paths.is_empty() {
        let rendered_conflicts = materialize_worktree_rebase_conflicts(&prepared)?;
        update_worktree_metadata_fields(
            repo,
            &prepared.worktree_name,
            &[
                (
                    "target_base_line",
                    JsonValue::String(prepared.onto_line_name.clone()),
                ),
                ("rebase_state", JsonValue::String("conflicted".to_string())),
                (
                    "rebase_started_at",
                    JsonValue::String(system_event_timestamp()),
                ),
                (
                    "rebase_original_head_snapshot_id",
                    JsonValue::String(prepared.old_head_snapshot_id.clone()),
                ),
                (
                    "rebase_onto_snapshot_id",
                    JsonValue::String(prepared.new_base_snapshot_id.clone()),
                ),
                (
                    "rebase_conflict_paths",
                    JsonValue::Array(
                        prepared
                            .plan
                            .conflict_paths
                            .iter()
                            .cloned()
                            .map(JsonValue::String)
                            .collect(),
                    ),
                ),
            ],
            &[],
        )?;
        let mut summary = worktree_get(repo, Some(&prepared.worktree_name), true)?;
        summary
            .as_object_mut()
            .expect("worktree conflicted rebase summary")
            .insert(
                "rebase".to_string(),
                rebase_plan_payload(
                    &prepared,
                    json!({
                        "status": "conflicted",
                        "rendered_conflicts": rendered_conflicts,
                    }),
                ),
            );
        return Ok(summary);
    }

    let new_workspace = workspace_delta_payload(
        &prepared.worktree_repo,
        Some(&prepared.new_base_snapshot_id),
        None,
    )?;
    let new_head_snapshot_id = if new_workspace
        .get("clean")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        prepared.new_base_snapshot_id.clone()
    } else {
        let snapshot = create_snapshot_with_parent(
            &prepared.worktree_repo,
            &format!(
                "Rebase {} onto {}",
                prepared.line_name, prepared.onto_line_name
            ),
            &prepared.new_base_snapshot_id,
        )?;
        required_string_field(&snapshot, "snapshot_id")?
    };
    set_local_line_head(
        &prepared.worktree_repo,
        &prepared.line_name,
        Some(&new_head_snapshot_id),
    )?;
    prepared
        .worktree_repo
        .set_worktree_materialized_snapshot(Some(&new_head_snapshot_id))?;
    update_worktree_metadata_fields(
        repo,
        &prepared.worktree_name,
        &[
            (
                "fork_snapshot_id",
                JsonValue::String(prepared.new_base_snapshot_id.clone()),
            ),
            (
                "forked_from_line",
                JsonValue::String(prepared.onto_line_name.clone()),
            ),
            (
                "target_base_line",
                JsonValue::String(prepared.onto_line_name.clone()),
            ),
            (
                "last_retargeted_at",
                JsonValue::String(system_event_timestamp()),
            ),
            ("rebase_state", JsonValue::String("idle".to_string())),
            ("rebase_conflict_paths", JsonValue::Array(Vec::new())),
        ],
        &[
            "rebase_started_at",
            "rebase_original_head_snapshot_id",
            "rebase_onto_snapshot_id",
        ],
    )?;
    let mut summary = worktree_get(repo, Some(&prepared.worktree_name), true)?;
    summary
        .as_object_mut()
        .expect("worktree rebase summary")
        .insert(
            "rebase".to_string(),
            rebase_plan_payload(
                &prepared,
                json!({
                    "status": "applied",
                    "new_head_snapshot_id": new_head_snapshot_id,
                }),
            ),
        );
    Ok(summary)
}

pub fn worktree_continue_rebase(
    repo: &RepoRuntime,
    name: Option<&str>,
) -> Result<JsonValue, String> {
    let prepared = prepare_worktree_rebase(repo, name, None, true)?;
    if metadata_string(&prepared.metadata, "rebase_state").as_deref() != Some("conflicted") {
        return Err(format!(
            "Worktree {} has no conflicted rebase to continue.",
            prepared.worktree_name
        ));
    }
    let new_base_snapshot_id = metadata_string(&prepared.metadata, "rebase_onto_snapshot_id")
        .ok_or_else(|| {
            format!(
                "Worktree {} is missing rebase state metadata; abort and retry.",
                prepared.worktree_name
            )
        })?;
    let old_head_snapshot_id =
        metadata_string(&prepared.metadata, "rebase_original_head_snapshot_id").ok_or_else(
            || {
                format!(
                    "Worktree {} is missing rebase state metadata; abort and retry.",
                    prepared.worktree_name
                )
            },
        )?;
    let onto_line_name = metadata_string(&prepared.metadata, "target_base_line")
        .or_else(|| metadata_string(&prepared.metadata, "forked_from_line"))
        .ok_or_else(|| {
            format!(
                "Worktree {} is missing rebase state metadata; abort and retry.",
                prepared.worktree_name
            )
        })?;
    let current_delta =
        workspace_delta_payload(&prepared.worktree_repo, Some(&new_base_snapshot_id), None)?;
    let new_head_snapshot_id = if current_delta
        .get("clean")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        new_base_snapshot_id.clone()
    } else {
        let snapshot = create_snapshot_with_parent(
            &prepared.worktree_repo,
            &format!("Rebase {} onto {}", prepared.line_name, onto_line_name),
            &new_base_snapshot_id,
        )?;
        required_string_field(&snapshot, "snapshot_id")?
    };
    set_local_line_head(
        &prepared.worktree_repo,
        &prepared.line_name,
        Some(&new_head_snapshot_id),
    )?;
    prepared
        .worktree_repo
        .set_worktree_materialized_snapshot(Some(&new_head_snapshot_id))?;
    update_worktree_metadata_fields(
        repo,
        &prepared.worktree_name,
        &[
            (
                "fork_snapshot_id",
                JsonValue::String(new_base_snapshot_id.clone()),
            ),
            (
                "forked_from_line",
                JsonValue::String(onto_line_name.clone()),
            ),
            (
                "target_base_line",
                JsonValue::String(onto_line_name.clone()),
            ),
            (
                "last_retargeted_at",
                JsonValue::String(system_event_timestamp()),
            ),
            ("rebase_state", JsonValue::String("idle".to_string())),
            ("rebase_conflict_paths", JsonValue::Array(Vec::new())),
        ],
        &[
            "rebase_started_at",
            "rebase_original_head_snapshot_id",
            "rebase_onto_snapshot_id",
        ],
    )?;
    let mut summary = worktree_get(repo, Some(&prepared.worktree_name), true)?;
    summary
        .as_object_mut()
        .expect("worktree continue rebase summary")
        .insert(
            "rebase".to_string(),
            json!({
                "worktree_name": prepared.worktree_name,
                "path": prepared.worktree_path.to_string_lossy().to_string(),
                "status": "continued",
                "old_head_snapshot_id": old_head_snapshot_id,
                "new_base_snapshot_id": new_base_snapshot_id,
                "new_head_snapshot_id": new_head_snapshot_id,
            }),
        );
    Ok(summary)
}

pub fn worktree_abort_rebase(repo: &RepoRuntime, name: Option<&str>) -> Result<JsonValue, String> {
    let prepared = prepare_worktree_rebase(repo, name, None, true)?;
    if metadata_string(&prepared.metadata, "rebase_state").as_deref() != Some("conflicted") {
        return Err(format!(
            "Worktree {} has no conflicted rebase to abort.",
            prepared.worktree_name
        ));
    }
    let old_head_snapshot_id =
        metadata_string(&prepared.metadata, "rebase_original_head_snapshot_id").ok_or_else(
            || {
                format!(
            "Worktree {} is missing the original head snapshot; manual recovery is required.",
            prepared.worktree_name
        )
            },
        )?;
    let baseline_snapshot_id = prepared
        .worktree_repo
        .config
        .get("materialized_snapshot_id")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)));
    restore_workspace_all(
        &prepared.worktree_repo,
        Some(&old_head_snapshot_id),
        baseline_snapshot_id.as_deref(),
        true,
        false,
    )?;
    set_local_line_head(
        &prepared.worktree_repo,
        &prepared.line_name,
        Some(&old_head_snapshot_id),
    )?;
    prepared
        .worktree_repo
        .set_worktree_materialized_snapshot(Some(&old_head_snapshot_id))?;
    update_worktree_metadata_fields(
        repo,
        &prepared.worktree_name,
        &[
            ("rebase_state", JsonValue::String("idle".to_string())),
            ("rebase_conflict_paths", JsonValue::Array(Vec::new())),
        ],
        &[
            "rebase_started_at",
            "rebase_original_head_snapshot_id",
            "rebase_onto_snapshot_id",
        ],
    )?;
    let mut summary = worktree_get(repo, Some(&prepared.worktree_name), true)?;
    summary
        .as_object_mut()
        .expect("worktree abort rebase summary")
        .insert(
            "rebase".to_string(),
            json!({
                "worktree_name": prepared.worktree_name,
                "path": prepared.worktree_path.to_string_lossy().to_string(),
                "status": "aborted",
                "restored_snapshot_id": old_head_snapshot_id,
            }),
        );
    Ok(summary)
}

#[derive(Clone, Debug)]
pub(in crate::primitives) struct WorktreeRebasePlan {
    pub(in crate::primitives) feature_delta_count: i64,
    pub(in crate::primitives) conflict_paths: Vec<String>,
    pub(in crate::primitives) apply_write_paths: Vec<String>,
    pub(in crate::primitives) apply_remove_paths: Vec<String>,
    pub(in crate::primitives) files: Vec<JsonValue>,
    pub(in crate::primitives) would_fast_forward: bool,
}

#[derive(Clone, Debug)]
pub(in crate::primitives) struct PreparedWorktreeRebase {
    pub(in crate::primitives) worktree_name: String,
    pub(in crate::primitives) metadata: JsonMap<String, JsonValue>,
    pub(in crate::primitives) worktree_path: PathBuf,
    pub(in crate::primitives) worktree_repo: RepoRuntime,
    pub(in crate::primitives) line_name: String,
    pub(in crate::primitives) old_base_snapshot_id: String,
    pub(in crate::primitives) old_head_snapshot_id: String,
    pub(in crate::primitives) new_base_snapshot_id: String,
    pub(in crate::primitives) onto_line_name: String,
    pub(in crate::primitives) rewrites_ancestry: bool,
    pub(in crate::primitives) plan: WorktreeRebasePlan,
}

pub(super) fn update_worktree_metadata_fields(
    repo: &RepoRuntime,
    worktree_name: &str,
    updates: &[(&str, JsonValue)],
    clear_keys: &[&str],
) -> Result<(), String> {
    let mut metadata = load_worktree_metadata(repo, worktree_name)?;
    for key in clear_keys {
        metadata.remove(*key);
    }
    for (key, value) in updates {
        metadata.insert((*key).to_string(), value.clone());
    }
    save_worktree_metadata(repo, worktree_name, &metadata)
}

pub(in crate::primitives) fn set_local_line_head(
    repo: &RepoRuntime,
    line_name: &str,
    snapshot_id: Option<&str>,
) -> Result<(), String> {
    let updated_at = system_event_timestamp();
    let workspace_root = repo.workspace_root();
    repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?
        .set_line_head(line_name, snapshot_id, &updated_at)
        .map(|_| ())
}

pub(super) fn snapshot_owned_by_bound_slice(
    provenance: Option<&JsonValue>,
    snapshot_id: &str,
    expected_task_id: &str,
    expected_change_id: Option<&str>,
    expected_worktree_name: Option<&str>,
) -> JsonMap<String, JsonValue> {
    let provenance_task_id = provenance.and_then(|value| string_field(value, "task_id"));
    let provenance_change_id = provenance.and_then(|value| string_field(value, "change_id"));
    let provenance_worktree_name =
        provenance.and_then(|value| string_field(value, "worktree_name"));
    let base = JsonMap::from_iter([
        (
            "snapshot_id".to_string(),
            JsonValue::String(snapshot_id.to_string()),
        ),
        (
            "owner_task_id".to_string(),
            provenance_task_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "owner_change_id".to_string(),
            provenance_change_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
        (
            "owner_worktree_name".to_string(),
            provenance_worktree_name
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null),
        ),
    ]);
    let mut owned = base.clone();
    match provenance {
        None => {
            owned.insert("owned".to_string(), JsonValue::Bool(false));
            owned.insert(
                "reason".to_string(),
                JsonValue::String("missing workflow provenance".to_string()),
            );
            owned
        }
        Some(_) if provenance_task_id.as_deref() != Some(expected_task_id) => {
            owned.insert("owned".to_string(), JsonValue::Bool(false));
            owned.insert(
                "reason".to_string(),
                JsonValue::String(format!(
                    "task {}",
                    provenance_task_id.as_deref().unwrap_or("none")
                )),
            );
            owned
        }
        Some(_)
            if expected_change_id.is_some()
                && provenance_change_id.as_deref() != expected_change_id
                && provenance_change_id.is_some() =>
        {
            owned.insert("owned".to_string(), JsonValue::Bool(false));
            owned.insert(
                "reason".to_string(),
                JsonValue::String(format!(
                    "change {}",
                    provenance_change_id.as_deref().unwrap_or("none")
                )),
            );
            owned
        }
        Some(_)
            if expected_worktree_name.is_some()
                && provenance_worktree_name.as_deref() != expected_worktree_name
                && provenance_worktree_name.is_some() =>
        {
            owned.insert("owned".to_string(), JsonValue::Bool(false));
            owned.insert(
                "reason".to_string(),
                JsonValue::String(format!(
                    "worktree {}",
                    provenance_worktree_name.as_deref().unwrap_or("none")
                )),
            );
            owned
        }
        Some(_) => {
            owned.insert("owned".to_string(), JsonValue::Bool(true));
            owned.insert("reason".to_string(), JsonValue::Null);
            owned
        }
    }
}

pub(in crate::primitives) fn read_worktree_snapshot_blob_bytes(
    repo: &RepoRuntime,
    row: Option<&JsonValue>,
) -> Result<Vec<u8>, String> {
    let Some(row) = row else {
        return Ok(Vec::new());
    };
    let Some(blob_id) = file_map_row_blob_id(row) else {
        return Ok(Vec::new());
    };
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    store.read_blob_bytes(&blob_id)
}

pub(super) fn decode_worktree_merge_text(data: &[u8]) -> Option<String> {
    if data.contains(&0) {
        return None;
    }
    String::from_utf8(data.to_vec()).ok()
}

pub(super) fn render_worktree_rebase_conflict_text(
    base_text: &str,
    feature_text: &str,
    target_text: &str,
    feature_label: &str,
    target_label: &str,
) -> String {
    format!(
        "<<<<<<< {feature_label}\n{feature_text}||||||| base\n{base_text}=======\n{target_text}>>>>>>> {target_label}\n"
    )
}

pub(in crate::primitives) fn write_workspace_snapshot_row(
    repo: &RepoRuntime,
    path: &str,
    row: Option<&JsonValue>,
) -> Result<(), String> {
    let abs_path = repo.workspace_root().join(path);
    if let Some(row) = row {
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        if abs_path.exists() && abs_path.is_dir() {
            return Err(format!("Cannot restore file over directory: {path}"));
        }
        fs::write(
            &abs_path,
            read_worktree_snapshot_blob_bytes(repo, Some(row))?,
        )
        .map_err(|err| err.to_string())?;
        let mode = parse_mode_bits(file_map_row_mode(row).as_deref())?;
        fs::set_permissions(&abs_path, fs::Permissions::from_mode(mode))
            .map_err(|err| err.to_string())?;
        return Ok(());
    }
    if abs_path.exists() {
        fs::remove_file(&abs_path).map_err(|err| err.to_string())?;
        prune_empty_parent_dirs(&repo.workspace_root(), &abs_path)?;
    }
    Ok(())
}

pub(super) fn write_workspace_text_file(
    repo: &RepoRuntime,
    path: &str,
    text: &str,
) -> Result<(), String> {
    let abs_path = repo.workspace_root().join(path);
    if let Some(parent) = abs_path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    if abs_path.exists() && abs_path.is_dir() {
        return Err(format!("Cannot restore file over directory: {path}"));
    }
    fs::write(&abs_path, text).map_err(|err| err.to_string())
}

pub(super) fn compute_worktree_rebase_plan(
    repo: &RepoRuntime,
    old_base_snapshot_id: &str,
    old_head_snapshot_id: &str,
    new_base_snapshot_id: &str,
) -> Result<WorktreeRebasePlan, String> {
    let workspace_root = repo.workspace_root();
    let store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    let snapshot_file_map = |snapshot_id: &str| {
        store
            .snapshot_tree_file_rows(Some(snapshot_id))
            .map(|rows| {
                rows.into_iter()
                    .map(|row| (row.path.clone(), row))
                    .collect::<BTreeMap<_, _>>()
            })
    };
    let base_files = snapshot_file_map(old_base_snapshot_id)?;
    let head_files = snapshot_file_map(old_head_snapshot_id)?;
    let target_files = snapshot_file_map(new_base_snapshot_id)?;
    let changed_paths = |left: &BTreeMap<String, SnapshotFileRow>,
                         right: &BTreeMap<String, SnapshotFileRow>| {
        left.keys()
            .chain(right.keys())
            .filter(|path| left.get(*path) != right.get(*path))
            .cloned()
            .collect::<BTreeSet<_>>()
    };
    let feature_paths = changed_paths(&base_files, &head_files);
    let target_paths = changed_paths(&base_files, &target_files);
    let all_paths = feature_paths
        .iter()
        .chain(target_paths.iter())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut files = Vec::new();
    let mut apply_write_paths = Vec::new();
    let mut apply_remove_paths = Vec::new();
    let mut conflict_paths = Vec::new();
    let feature_delta_count = feature_paths.len() as i64;
    for path in all_paths {
        let base_row = base_files.get(&path);
        let head_row = head_files.get(&path);
        let target_row = target_files.get(&path);
        let feature_changed = head_row != base_row;
        let target_changed = target_row != base_row;
        let (resolution, apply_status) = if !feature_changed {
            ("target", "unchanged")
        } else if !target_changed {
            if head_row.is_none() {
                ("feature", "remove")
            } else {
                ("feature", "write")
            }
        } else if head_row == target_row {
            ("same_result", "unchanged")
        } else {
            conflict_paths.push(path.clone());
            ("conflict", "conflict")
        };
        if apply_status == "write" {
            apply_write_paths.push(path.clone());
        } else if apply_status == "remove" {
            apply_remove_paths.push(path.clone());
        }
        files.push(json!({
            "path": path,
            "feature_changed": feature_changed,
            "target_changed": target_changed,
            "resolution": resolution,
            "apply_status": apply_status,
            "old": base_row.map(worktree_snapshot_file_row_json).unwrap_or(JsonValue::Null),
            "feature": head_row.map(worktree_snapshot_file_row_json).unwrap_or(JsonValue::Null),
            "target": target_row.map(worktree_snapshot_file_row_json).unwrap_or(JsonValue::Null),
        }));
    }
    Ok(WorktreeRebasePlan {
        feature_delta_count,
        conflict_paths,
        apply_write_paths,
        apply_remove_paths,
        files,
        would_fast_forward: feature_delta_count == 0,
    })
}

fn worktree_snapshot_file_row_json(row: &SnapshotFileRow) -> JsonValue {
    json!({
        "path": row.path,
        "blob_id": row.blob_id,
        "size_bytes": row.size_bytes,
        "mode": row.mode,
        "sha256": row.sha256,
    })
}

pub(in crate::primitives) fn prepare_worktree_rebase(
    repo: &RepoRuntime,
    name: Option<&str>,
    onto_line_name: Option<&str>,
    allow_conflicted_state: bool,
) -> Result<PreparedWorktreeRebase, String> {
    guard_no_active_line_merge(repo, name, "rebasing the worktree")?;
    let worktree_name = resolve_runtime_worktree_name(repo, name)?;
    let metadata = worktree_metadata_with_defaults(&load_worktree_metadata(repo, &worktree_name)?);
    if metadata_string(&metadata, "rebase_state").as_deref() == Some("conflicted")
        && !allow_conflicted_state
    {
        return Err(format!(
            "Worktree {worktree_name} is already in a conflicted rebase. Use --continue or --abort."
        ));
    }
    let worktree_path = required_path_field(&JsonValue::Object(metadata.clone()), "path")?;
    let Some(worktree_repo) = discover_worktree_repo(&worktree_path) else {
        return Err(format!("Worktree is missing or detached: {worktree_name}"));
    };
    let line_name = worktree_repo.current_line_name()?;
    let old_head_snapshot_id = local_line_head_snapshot_id(&worktree_repo, &line_name)?
        .ok_or_else(|| format!("Current line {line_name} has no head snapshot to rebase."))?;
    let onto_line_name = normalized_text(onto_line_name)
        .or_else(|| effective_worktree_target_base_line(&metadata, None))
        .ok_or_else(|| {
            format!("Worktree {worktree_name} has no target base line. Pass --onto <line>.")
        })?;
    let new_base_snapshot_id = local_line_head_snapshot_id(repo, &onto_line_name)?
        .ok_or_else(|| format!("Base line {onto_line_name} has no head snapshot."))?;
    let registered_fork_snapshot_id = metadata_string(&metadata, "fork_snapshot_id");
    let mut old_base_snapshot_id = registered_fork_snapshot_id
        .clone()
        .or_else(|| {
            latest_common_snapshot(repo, &old_head_snapshot_id, Some(&new_base_snapshot_id))
                .ok()
                .flatten()
        })
        .ok_or_else(|| format!("Could not infer a fork snapshot for worktree {worktree_name}."))?;
    if snapshot_distance_if_ancestor(
        repo,
        Some(&old_base_snapshot_id),
        Some(&old_head_snapshot_id),
    )?
    .is_none()
    {
        if let Some(candidate) = recoverable_worktree_rebase_fork(
            repo,
            &old_base_snapshot_id,
            None,
            &old_head_snapshot_id,
            &new_base_snapshot_id,
        )? {
            old_base_snapshot_id = candidate;
        } else {
            return Err(format!(
                "Fork snapshot {old_base_snapshot_id} is not an ancestor of line head {old_head_snapshot_id}; manual recovery is required."
            ));
        }
    }
    if old_base_snapshot_id != new_base_snapshot_id
        && snapshot_distance_if_ancestor(
            repo,
            Some(&old_base_snapshot_id),
            Some(&new_base_snapshot_id),
        )?
        .is_none()
    {
        if let Some(candidate) = recoverable_worktree_rebase_fork(
            repo,
            &old_base_snapshot_id,
            None,
            &old_head_snapshot_id,
            &new_base_snapshot_id,
        )? {
            old_base_snapshot_id = candidate;
        } else {
            return Err(format!(
                "Base line {onto_line_name} no longer descends from fork snapshot {old_base_snapshot_id}; automatic retarget is not safe."
            ));
        }
    }
    let plan = compute_worktree_rebase_plan(
        repo,
        &old_base_snapshot_id,
        &old_head_snapshot_id,
        &new_base_snapshot_id,
    )?;
    let rewrites_ancestry = requires_same_base_ancestry_rewrite(
        registered_fork_snapshot_id.as_deref(),
        &old_base_snapshot_id,
        &old_head_snapshot_id,
        &new_base_snapshot_id,
    );
    Ok(PreparedWorktreeRebase {
        worktree_name,
        metadata,
        worktree_path,
        worktree_repo,
        line_name,
        old_base_snapshot_id,
        old_head_snapshot_id,
        new_base_snapshot_id,
        onto_line_name,
        rewrites_ancestry,
        plan,
    })
}

fn requires_same_base_ancestry_rewrite(
    registered_fork_snapshot_id: Option<&str>,
    computed_old_base_snapshot_id: &str,
    old_head_snapshot_id: &str,
    new_base_snapshot_id: &str,
) -> bool {
    // A divergent registered fork can fall back to the target's common ancestor.
    // Treating that recovered equality as a no-op preserves the divergent parent
    // chain even though the worktree was explicitly retargeted to another line.
    computed_old_base_snapshot_id == new_base_snapshot_id
        && old_head_snapshot_id != new_base_snapshot_id
        && registered_fork_snapshot_id
            .is_some_and(|registered_fork| registered_fork != new_base_snapshot_id)
}

fn valid_bound_change_rebase_fork(
    repo: &RepoRuntime,
    rejected_fork_snapshot_id: &str,
    candidate_fork_snapshot_id: Option<&str>,
    old_head_snapshot_id: &str,
    new_base_snapshot_id: &str,
) -> Result<Option<String>, String> {
    let Some(candidate) = candidate_fork_snapshot_id else {
        return Ok(None);
    };
    if candidate == rejected_fork_snapshot_id
        || snapshot_distance_if_ancestor(repo, Some(candidate), Some(old_head_snapshot_id))?
            .is_none()
        || (candidate != new_base_snapshot_id
            && snapshot_distance_if_ancestor(repo, Some(candidate), Some(new_base_snapshot_id))?
                .is_none())
    {
        return Ok(None);
    }
    Ok(Some(candidate.to_string()))
}

fn recoverable_worktree_rebase_fork(
    repo: &RepoRuntime,
    rejected_fork_snapshot_id: &str,
    bound_change_fork_snapshot_id: Option<&str>,
    old_head_snapshot_id: &str,
    new_base_snapshot_id: &str,
) -> Result<Option<String>, String> {
    if let Some(candidate) = valid_bound_change_rebase_fork(
        repo,
        rejected_fork_snapshot_id,
        bound_change_fork_snapshot_id,
        old_head_snapshot_id,
        new_base_snapshot_id,
    )? {
        return Ok(Some(candidate));
    }
    latest_common_snapshot(repo, old_head_snapshot_id, Some(new_base_snapshot_id))
}

pub(super) fn materialize_worktree_rebase_conflicts(
    prepared: &PreparedWorktreeRebase,
) -> Result<Vec<JsonValue>, String> {
    let mut rendered = Vec::new();
    for entry in &prepared.plan.files {
        if string_field(entry, "resolution").as_deref() != Some("conflict") {
            continue;
        }
        let path = required_string_field(entry, "path")?;
        let base_bytes =
            read_worktree_snapshot_blob_bytes(&prepared.worktree_repo, entry.get("old"))?;
        let feature_bytes =
            read_worktree_snapshot_blob_bytes(&prepared.worktree_repo, entry.get("feature"))?;
        let target_bytes =
            read_worktree_snapshot_blob_bytes(&prepared.worktree_repo, entry.get("target"))?;
        let feature_text = decode_worktree_merge_text(&feature_bytes);
        let target_text = decode_worktree_merge_text(&target_bytes);
        if let (Some(feature_text), Some(target_text)) = (feature_text, target_text) {
            write_workspace_text_file(
                &prepared.worktree_repo,
                &path,
                &render_worktree_rebase_conflict_text(
                    &decode_worktree_merge_text(&base_bytes).unwrap_or_default(),
                    &feature_text,
                    &target_text,
                    &prepared.line_name,
                    &prepared.onto_line_name,
                ),
            )?;
            rendered.push(json!({"path": path, "kind": "text_markers"}));
        } else if entry.get("feature").is_some_and(|value| !value.is_null()) {
            write_workspace_snapshot_row(&prepared.worktree_repo, &path, entry.get("feature"))?;
            rendered.push(json!({"path": path, "kind": "binary_or_non_utf8_feature_version"}));
        } else {
            write_workspace_snapshot_row(&prepared.worktree_repo, &path, entry.get("target"))?;
            rendered.push(json!({"path": path, "kind": "binary_or_non_utf8_target_version"}));
        }
    }
    Ok(rendered)
}

pub(super) fn rebase_plan_payload(
    prepared: &PreparedWorktreeRebase,
    extras: JsonValue,
) -> JsonValue {
    let mut payload = json!({
        "worktree_name": prepared.worktree_name,
        "path": prepared.worktree_path.to_string_lossy().to_string(),
        "line_name": prepared.line_name,
        "onto_line_name": prepared.onto_line_name,
        "old_base_snapshot_id": prepared.old_base_snapshot_id,
        "old_head_snapshot_id": prepared.old_head_snapshot_id,
        "new_base_snapshot_id": prepared.new_base_snapshot_id,
        "rewrites_ancestry": prepared.rewrites_ancestry,
        "feature_delta_count": prepared.plan.feature_delta_count,
        "conflict_count": prepared.plan.conflict_paths.len(),
        "conflict_paths": prepared.plan.conflict_paths,
        "apply_write_paths": prepared.plan.apply_write_paths,
        "apply_remove_paths": prepared.plan.apply_remove_paths,
        "files": prepared.plan.files,
        "would_fast_forward": prepared.plan.would_fast_forward,
    });
    if let (Some(target), Some(extra)) = (payload.as_object_mut(), extras.as_object()) {
        for (key, value) in extra {
            target.insert(key.clone(), value.clone());
        }
    }
    payload
}

pub(super) fn create_snapshot_with_parent(
    repo: &RepoRuntime,
    message: &str,
    parent_snapshot_id: &str,
) -> Result<JsonValue, String> {
    let line_name = repo.current_line_name()?;
    let previous_head_snapshot_id = local_line_head_snapshot_id(repo, &line_name)?;
    let workspace_root = repo.workspace_root();
    let snapshot_store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    set_local_line_head(repo, &line_name, Some(parent_snapshot_id))?;
    let created = snapshot_store.create_snapshot(
        &repo.repo_name(),
        &line_name,
        Some(message),
        repo.is_worktree(),
    );
    let snapshot = match created {
        Ok(snapshot) => snapshot,
        Err(err) => {
            set_local_line_head(repo, &line_name, previous_head_snapshot_id.as_deref())?;
            return Err(err);
        }
    };
    Ok(snapshot)
}

#[cfg(test)]
mod selected_binary_line_tests {
    use super::*;
    use ait_core::line_store::LineStore;
    use ait_core::local_snapshot::{LocalSnapshotReadStore, LocalSnapshotWriteStore};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent");
        }
        fs::write(path, content).expect("write file");
    }

    fn binary_snapshot_repo() -> (TempDir, RepoRuntime) {
        let temp = TempDir::new().expect("repo tempdir");
        let root = temp.path();
        fs::create_dir_all(root.join(".ait")).expect("create .ait");
        write_file(
            &root.join(".ait/config.json"),
            r#"{"repo_name":"fixture-ait","default_line":"main","snapshot_binary_db_storage":"binary"}"#,
        );
        let repo = RepoRuntime::discover_from_path(root).expect("discover runtime");
        repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
            .lines()
            .create_line("main", None, "2026-07-08T00:00:00Z")
            .expect("create Binary DB line");
        (temp, repo)
    }

    #[test]
    fn divergent_registered_fork_rewrites_equal_recovered_base_ancestry() {
        assert!(requires_same_base_ancestry_rewrite(
            Some("SNP-SIDE"),
            "SNP-MAIN",
            "SNP-FEATURE",
            "SNP-MAIN",
        ));
    }

    #[test]
    fn aligned_or_base_only_rebase_remains_a_noop() {
        assert!(!requires_same_base_ancestry_rewrite(
            Some("SNP-MAIN"),
            "SNP-MAIN",
            "SNP-FEATURE",
            "SNP-MAIN",
        ));
        assert!(!requires_same_base_ancestry_rewrite(
            Some("SNP-SIDE"),
            "SNP-MAIN",
            "SNP-MAIN",
            "SNP-MAIN",
        ));
        assert!(!requires_same_base_ancestry_rewrite(
            None,
            "SNP-MAIN",
            "SNP-FEATURE",
            "SNP-MAIN",
        ));
    }

    #[test]
    fn create_snapshot_with_parent_uses_selected_binary_line_head_override() {
        let (_temp, repo) = binary_snapshot_repo();
        let root = repo.workspace_root();
        let snapshot_store = repo
            .local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&root)
            .expect("selected snapshot store");

        write_file(&root.join("file.txt"), "first\n");
        let first = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("first"), false)
            .expect("create first Binary DB snapshot");
        let first_id = required_string_field(&first, "snapshot_id").expect("first snapshot id");

        write_file(&root.join("file.txt"), "second\n");
        let second = snapshot_store
            .create_snapshot("fixture-ait", "main", Some("second"), false)
            .expect("create second Binary DB snapshot");
        let second_id = required_string_field(&second, "snapshot_id").expect("second snapshot id");
        assert_eq!(
            repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
                .lines()
                .line_by_name("main")
                .expect("read Binary DB line")
                .expect("Binary DB line exists")
                .head_snapshot_id
                .as_deref(),
            Some(second_id.as_str())
        );

        write_file(&root.join("file.txt"), "rebased\n");
        let rebased = create_snapshot_with_parent(&repo, "rebased", &first_id)
            .expect("create rebase snapshot");
        let rebased_id =
            required_string_field(&rebased, "snapshot_id").expect("rebased snapshot id");

        assert_eq!(rebased["parent_snapshot_id"], first_id);
        assert_ne!(rebased_id, second_id);
        let read_back = snapshot_store
            .get_snapshot(&rebased_id)
            .expect("read rebased Binary DB snapshot");
        assert_eq!(read_back["parent_snapshot_id"], first_id);
        assert_eq!(
            repo.binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
                .lines()
                .line_by_name("main")
                .expect("read Binary DB line")
                .expect("Binary DB line exists")
                .head_snapshot_id
                .as_deref(),
            Some(rebased_id.as_str())
        );
    }
}
