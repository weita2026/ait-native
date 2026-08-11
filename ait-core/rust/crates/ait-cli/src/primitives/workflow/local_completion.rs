use super::*;
use crate::primitives::change_flow::change_local_mark_published_with_change_store;
use crate::primitives::plan_checklist_closeout::plan_sync_request;

fn workflow_completed_local_entry_from_rows(
    task: JsonValue,
    change: JsonValue,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let task_id = required_string_field(&task, "task_id")?;
    let change_id = required_string_field(&change, "change_id")?;
    let target_line = string_field(&change, "base_line").unwrap_or_else(|| "main".to_string());
    let remote_task_id = string_field(&task, "published_task_id").unwrap_or_default();
    let remote_change_id =
        string_field(&change, "published_change_id").unwrap_or_else(|| change_id.clone());
    Ok(json!({
        "status": "ready",
        "task_id": task_id,
        "change_id": change_id,
        "target_line": target_line,
        "remote": remote_name,
        "remote_task_id": remote_task_id,
        "remote_change_id": remote_change_id,
        "state": {
            "routing": {
                "kind": "completed_local",
                "local_task_id": string_field(&task, "task_id"),
                "local_change_id": string_field(&change, "change_id"),
                "remote_task_id": string_field(&task, "published_task_id"),
                "remote_change_id": string_field(&change, "published_change_id").or_else(|| string_field(&change, "change_id")),
                "target_line": target_line,
            },
            "task": task,
            "change": change,
        },
    }))
}

fn workflow_completed_local_entry_if_present(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<Option<JsonValue>, String> {
    let change_store = repo.change_store()?;
    let task_store = repo.task_store()?;
    let matching_changes = workflow_local_change_rows_with_change_store(&change_store)?
        .into_iter()
        .filter(|row| {
            string_field(row, "change_id").as_deref() == Some(change_id)
                || string_field(row, "change_ref").as_deref() == Some(change_id)
        })
        .collect::<Vec<_>>();
    let change = match matching_changes.as_slice() {
        [] => return Ok(None),
        [change] => change.clone(),
        _ => {
            return Err(format!(
                "Local Change identity {change_id:?} is ambiguous across completed Tasks; use the exact `<task-id>/<change-id>` reference."
            ))
        }
    };
    if string_field(&change, "status").as_deref() != Some("landed") {
        return Ok(None);
    }
    let task_id = required_string_field(&change, "task_id")?;
    let task = workflow_local_task_read_with_task_store(&task_store, &task_id)?;
    if string_field(&task, "status").as_deref() != Some("completed") {
        return Ok(None);
    }
    if string_field(&change, "publication_state").as_deref() == Some("published")
        && string_field(&task, "publication_state").as_deref() != Some("published")
    {
        return Err(format!(
            "Local completed change {change_id} is marked published while task {task_id} is not; repair the partial publication state before promotion."
        ));
    }
    workflow_completed_local_entry_from_rows(task, change, remote_name).map(Some)
}

const MAX_HISTORY_PROMOTION_ENTRIES: usize = 64;
const MAX_HISTORY_PROMOTION_SNAPSHOTS_PER_ENTRY: usize = 64;

pub(in crate::primitives) fn workflow_unique_history_plan_artifact_paths(
    paths: impl IntoIterator<Item = String>,
) -> Vec<String> {
    paths
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn workflow_history_epoch_seconds(value: Option<&JsonValue>, label: &str) -> Result<u64, String> {
    if let Some(seconds) = value.and_then(JsonValue::as_u64) {
        return Ok(seconds);
    }
    let seconds = {
        let timestamp = value
            .and_then(JsonValue::as_str)
            .and_then(|value| normalized_text(Some(value)))
            .ok_or_else(|| format!("{label} is missing."))?;
        match timestamp.parse::<u64>() {
            Ok(seconds) => return Ok(seconds),
            Err(_) => DateTime::parse_from_rfc3339(&timestamp.replace('Z', "+00:00"))
                .map_err(|error| format!("{label} is not a valid timestamp: {error}"))?
                .timestamp(),
        }
    };
    u64::try_from(seconds).map_err(|_| format!("{label} predates the Unix epoch."))
}

fn workflow_history_plan_artifact_path(
    repo: &RepoRuntime,
    task: &JsonValue,
) -> Result<Option<String>, String> {
    let plan_id = string_field(task, "plan_id");
    let revision_id = string_field(task, "origin_plan_revision_id");
    match (plan_id.as_deref(), revision_id.as_deref()) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(format!(
            "Local task {} has incomplete Plan linkage.",
            required_string_field(task, "task_id")?
        )),
        (Some(plan_id), Some(revision_id)) => {
            let plan_store = repo
                .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
                .plans();
            let revision = get_plan_revision_by_id_with_plan_store(&plan_store, revision_id)
                .map_err(|error| error.to_string())?;
            if revision.plan_id != plan_id {
                return Err(format!(
                    "Local task {} Plan revision {} belongs to {}, not {}.",
                    required_string_field(task, "task_id")?,
                    revision_id,
                    revision.plan_id,
                    plan_id
                ));
            }
            Ok(Some(revision.artifact_path))
        }
    }
}

fn workflow_history_snapshot_rows(
    repo: &RepoRuntime,
    local_change_ref: &str,
    pre_land_snapshot_id: &str,
    landed_snapshot_id: &str,
) -> Result<Vec<JsonValue>, String> {
    let snapshot_ids = local_snapshot_chain_segment(
        repo,
        pre_land_snapshot_id,
        landed_snapshot_id,
        "workflow ready history promotion",
    )?;
    if snapshot_ids.is_empty() {
        return Err(format!(
            "Completed local Change {local_change_ref} has an empty Land Snapshot boundary."
        ));
    }
    if snapshot_ids.len() > MAX_HISTORY_PROMOTION_SNAPSHOTS_PER_ENTRY {
        return Err(format!(
            "Completed local Change {local_change_ref} contains {} Snapshots; history promotion is bounded to {} per local Land.",
            snapshot_ids.len(),
            MAX_HISTORY_PROMOTION_SNAPSHOTS_PER_ENTRY
        ));
    }
    let workspace_root = repo.workspace_root();
    let snapshot_store =
        repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(&workspace_root)?;
    snapshot_ids
        .into_iter()
        .map(|snapshot_id| {
            let snapshot = snapshot_by_id_with_snapshot_store(&snapshot_store, &snapshot_id)?
                .ok_or_else(|| {
                    format!(
                        "History promotion Snapshot {snapshot_id} disappeared from local storage."
                    )
                })?;
            Ok(json!({
                "snapshot_id": snapshot.snapshot_id,
                "created_at_s": workflow_history_epoch_seconds(
                    Some(&JsonValue::String(snapshot.created_at)),
                    &format!("Snapshot {snapshot_id} created_at"),
                )?,
            }))
        })
        .collect()
}

fn workflow_history_snapshot_parent_ids(snapshot: &JsonValue) -> Result<Vec<String>, String> {
    if let Some(value) = snapshot.get("parent_snapshot_ids") {
        let values = value.as_array().ok_or_else(|| {
            "History promotion Snapshot parent_snapshot_ids must be an array.".to_string()
        })?;
        let mut parents = Vec::with_capacity(values.len());
        let mut seen = BTreeSet::new();
        for value in values {
            let parent = value
                .as_str()
                .and_then(|value| normalized_text(Some(value)))
                .ok_or_else(|| {
                    "History promotion Snapshot parent identity must be a non-empty string."
                        .to_string()
                })?;
            if !seen.insert(parent.clone()) {
                return Err(format!(
                    "History promotion Snapshot contains duplicate parent `{parent}`."
                ));
            }
            parents.push(parent);
        }
        return Ok(parents);
    }
    Ok(string_field(snapshot, "primary_parent_snapshot_id")
        .or_else(|| string_field(snapshot, "parent_snapshot_id"))
        .into_iter()
        .collect())
}

fn workflow_recover_task_owned_pre_land_boundary(
    repo: &RepoRuntime,
    change: &JsonValue,
    landed_snapshot_id: &str,
) -> Result<String, String> {
    let local_change_id = required_string_field(change, "change_id")?;
    let local_change_ref =
        string_field(change, "change_ref").unwrap_or_else(|| local_change_id.clone());
    let local_task_id = required_string_field(change, "task_id")?;
    let fork_snapshot_id = required_string_field(change, "fork_snapshot_id")?;
    if landed_snapshot_id == fork_snapshot_id {
        return Err(format!(
            "Completed local Change {local_change_ref} has a genuinely empty Land boundary at its fork Snapshot `{landed_snapshot_id}`."
        ));
    }
    let task_lines = task_feature_line_candidates(&local_task_id)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut cursor = landed_snapshot_id.to_string();
    let mut seen = BTreeSet::new();

    for _ in 0..MAX_HISTORY_PROMOTION_SNAPSHOTS_PER_ENTRY {
        if !seen.insert(cursor.clone()) {
            return Err(format!(
                "Completed local Change {local_change_ref} empty-boundary recovery found a Snapshot cycle at `{cursor}`."
            ));
        }
        if cursor == fork_snapshot_id {
            return Ok(cursor);
        }
        let snapshot = snapshot_show(repo, &cursor)?;
        let snapshot_kind =
            string_field(&snapshot, "snapshot_kind").unwrap_or_else(|| "line".to_string());
        if snapshot_kind != "line" {
            return Err(format!(
                "Completed local Change {local_change_ref} empty-boundary recovery reached non-Line Snapshot `{cursor}` of kind `{snapshot_kind}`."
            ));
        }
        let snapshot_line = required_string_field(&snapshot, "line_name")?;
        if !task_lines.contains(&snapshot_line) {
            if cursor == landed_snapshot_id {
                return Err(format!(
                    "Completed local Change {local_change_ref} has an empty Land boundary, but landed Snapshot `{cursor}` belongs to `{snapshot_line}` instead of the Task feature Line."
                ));
            }
            if snapshot_distance_if_ancestor(repo, Some(&fork_snapshot_id), Some(&cursor))?
                .is_none()
            {
                return Err(format!(
                    "Completed local Change {local_change_ref} recovered boundary `{cursor}` does not descend from historical fork `{fork_snapshot_id}`."
                ));
            }
            return Ok(cursor);
        }
        let parents = workflow_history_snapshot_parent_ids(&snapshot)?;
        let [parent] = parents.as_slice() else {
            return Err(format!(
                "Completed local Change {local_change_ref} empty-boundary recovery requires exactly one parent for Task-owned Snapshot `{cursor}`; found {}.",
                parents.len()
            ));
        };
        cursor = parent.clone();
    }

    Err(format!(
        "Completed local Change {local_change_ref} empty-boundary recovery exceeds the bounded maximum of {MAX_HISTORY_PROMOTION_SNAPSHOTS_PER_ENTRY} Task-owned Snapshots."
    ))
}

pub(super) fn workflow_effective_pre_land_target_snapshot_id(
    repo: &RepoRuntime,
    change: &JsonValue,
    landed_snapshot_id: &str,
    recorded_pre_land_snapshot_id: &str,
) -> Result<(String, bool), String> {
    if recorded_pre_land_snapshot_id != landed_snapshot_id {
        return Ok((recorded_pre_land_snapshot_id.to_string(), false));
    }
    workflow_recover_task_owned_pre_land_boundary(repo, change, landed_snapshot_id)
        .map(|snapshot_id| (snapshot_id, true))
}

pub(in crate::primitives) fn workflow_local_history_entries(
    repo: &RepoRuntime,
    selected_change_id: &str,
    target_line: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
) -> Result<(Vec<JsonValue>, Vec<String>), String> {
    let change_store = repo.change_store()?;
    let task_store = repo.task_store()?;
    let change_rows = workflow_local_change_rows_with_change_store(&change_store)?;
    let mut current_snapshot_id = revision_snapshot_id.to_string();
    let mut reversed = Vec::new();
    let mut seen_tasks = BTreeSet::new();
    let mut seen_changes = BTreeSet::new();
    let mut plan_artifact_paths = Vec::new();

    while current_snapshot_id != base_snapshot_id {
        if reversed.len() >= MAX_HISTORY_PROMOTION_ENTRIES {
            return Err(format!(
                "History promotion exceeds the bounded maximum of {MAX_HISTORY_PROMOTION_ENTRIES} local Lands."
            ));
        }
        let matching = change_rows
            .iter()
            .filter(|row| {
                string_field(row, "status").as_deref() == Some("landed")
                    && string_field(row, "landed_snapshot_id").as_deref()
                        == Some(current_snapshot_id.as_str())
                    && string_field(row, "target_line")
                        .or_else(|| string_field(row, "base_line"))
                        .as_deref()
                        == Some(target_line)
            })
            .collect::<Vec<_>>();
        let change = match matching.as_slice() {
            [] => {
                return Err(format!(
                    "Local Land history has a gap before Snapshot `{current_snapshot_id}`; no landed Change on `{target_line}` owns that target head."
                ))
            }
            [change] => (*change).clone(),
            _ => {
                return Err(format!(
                    "Local Land history is ambiguous at Snapshot `{current_snapshot_id}`; multiple landed Changes claim the same target head."
                ))
            }
        };
        let local_change_id = required_string_field(&change, "change_id")?;
        let local_task_id = required_string_field(&change, "task_id")?;
        let local_change_ref = string_field(&change, "change_ref")
            .unwrap_or_else(|| format!("{local_task_id}/{local_change_id}"));
        if !seen_changes.insert(local_change_ref.clone()) {
            return Err(format!(
                "Local Land history cycles through Change {local_change_ref}."
            ));
        }
        if !seen_tasks.insert(local_task_id.clone()) {
            return Err(format!(
                "History promotion currently requires one completed Task per local Land; Task {local_task_id} owns more than one included landed Change."
            ));
        }
        let recorded_pre_land_snapshot_id =
            string_field(&change, "pre_land_target_snapshot_id").ok_or_else(|| {
                format!(
                    "Completed local Change {local_change_ref} is missing pre_land_target_snapshot_id."
                )
            })?;
        let (pre_land_snapshot_id, pre_land_boundary_recovered) =
            workflow_effective_pre_land_target_snapshot_id(
                repo,
                &change,
                &current_snapshot_id,
                &recorded_pre_land_snapshot_id,
            )?;
        let task = workflow_local_task_read_with_task_store(&task_store, &local_task_id)?;
        if string_field(&task, "status").as_deref() != Some("completed") {
            return Err(format!(
                "Local Land history Change {local_change_ref} belongs to Task {local_task_id}, which is not completed."
            ));
        }
        let task_is_published =
            string_field(&task, "publication_state").as_deref() == Some("published");
        let change_is_published =
            string_field(&change, "publication_state").as_deref() == Some("published");
        if change_is_published && !task_is_published {
            return Err(format!(
                "Local history Change {local_change_ref} is marked published while Task {local_task_id} is not; repair the invalid publication ordering before promotion."
            ));
        }
        let fork_snapshot_id = string_field(&change, "fork_snapshot_id").ok_or_else(|| {
            format!("Completed local Change {local_change_ref} is missing its historical fork.")
        })?;
        if snapshot_distance_if_ancestor(repo, Some(&fork_snapshot_id), Some(&current_snapshot_id))?
            .is_none()
        {
            return Err(format!(
                "Completed local Change {local_change_ref} landed at `{current_snapshot_id}`, which does not descend from its historical fork `{fork_snapshot_id}`."
            ));
        }
        if let Some(artifact_path) = workflow_history_plan_artifact_path(repo, &task)? {
            plan_artifact_paths.push(artifact_path);
        }
        let snapshots = workflow_history_snapshot_rows(
            repo,
            &local_change_ref,
            &pre_land_snapshot_id,
            &current_snapshot_id,
        )?;
        let landed_at_s = workflow_history_epoch_seconds(
            change.get("landed_at"),
            &format!("Change {local_change_ref} landed_at"),
        )?;
        let landed_snapshot_id = current_snapshot_id.clone();
        let next_snapshot_id = pre_land_snapshot_id.clone();
        reversed.push(json!({
            "local_task_id": local_task_id,
            "local_change_id": local_change_id,
            "local_change_ref": local_change_ref,
            "task": task,
            "change": change,
            "pre_land_target_snapshot_id": pre_land_snapshot_id,
            "pre_land_boundary_source": if pre_land_boundary_recovered {
                "task_owned_snapshot_lineage_recovery"
            } else {
                "land_record"
            },
            "landed_snapshot_id": landed_snapshot_id,
            "landed_at_s": landed_at_s,
            "snapshots": snapshots,
            "already_published": task_is_published,
            "publication_recovery_required": task_is_published && !change_is_published,
        }));
        current_snapshot_id = next_snapshot_id;
    }
    reversed.reverse();
    if reversed.is_empty() {
        return Err("History promotion has no local Land entries.".to_string());
    }
    let final_change_id = reversed
        .last()
        .and_then(|entry| string_field(entry, "local_change_id"))
        .ok_or_else(|| "History promotion final entry has no Change identity.".to_string())?;
    let final_change_ref = reversed
        .last()
        .and_then(|entry| string_field(entry, "local_change_ref"))
        .ok_or_else(|| "History promotion final entry has no Change reference.".to_string())?;
    if final_change_id != selected_change_id && final_change_ref != selected_change_id {
        return Err(format!(
            "Selected local Change {selected_change_id} is not the final Change in the consecutive Land chain; the chain ends at {final_change_ref}."
        ));
    }
    Ok((
        reversed,
        workflow_unique_history_plan_artifact_paths(plan_artifact_paths),
    ))
}

pub(in crate::primitives) fn workflow_final_snapshot_candidate_from_entry(
    entry: &JsonValue,
    local_target_head: Option<&str>,
    remote_target_head: Option<&str>,
    remote_to_revision_distance: Option<i64>,
) -> Result<JsonValue, String> {
    let state = entry.get("state").cloned().unwrap_or_else(|| json!({}));
    let change = state.get("change").cloned().unwrap_or(JsonValue::Null);
    let local_change_id = required_string_field(&change, "change_id")?;
    let target_line = string_field(&change, "base_line").unwrap_or_else(|| "main".to_string());
    let revision_snapshot_id = string_field(&change, "landed_snapshot_id").ok_or_else(|| {
        format!("Completed local change {local_change_id} is missing landed_snapshot_id.")
    })?;
    let local_target_head = normalized_text(local_target_head).ok_or_else(|| {
        format!("Local target line `{target_line}` has no head snapshot to promote.")
    })?;
    if local_target_head != revision_snapshot_id {
        return Err(format!(
            "Completed local change {local_change_id} landed at `{revision_snapshot_id}`, but `{target_line}` is now at `{local_target_head}`. Only the latest completed local change that owns the current target-line head can be promoted; select the change for `{local_target_head}`."
        ));
    }
    let base_snapshot_id = normalized_text(remote_target_head).ok_or_else(|| {
        format!("Remote target line `{target_line}` has no head snapshot; initialize or reconcile the remote line before history promotion.")
    })?;
    let remote_already_contains_revision = base_snapshot_id == revision_snapshot_id;
    if !remote_already_contains_revision && remote_to_revision_distance.is_none() {
        return Err(format!(
            "Final local snapshot `{revision_snapshot_id}` does not descend from remote `{target_line}` head `{base_snapshot_id}`. Pull/reconcile the remote target line and rebase the final local result before promotion."
        ));
    }
    let mut candidate = entry
        .as_object()
        .cloned()
        .ok_or_else(|| "Completed-local promotion entry must be an object.".to_string())?;
    candidate.insert(
        "mode".to_string(),
        JsonValue::String("solo_local_history_promotion".to_string()),
    );
    candidate.insert(
        "base_snapshot_id".to_string(),
        JsonValue::String(base_snapshot_id),
    );
    candidate.insert(
        "revision_snapshot_id".to_string(),
        JsonValue::String(revision_snapshot_id),
    );
    candidate.insert(
        "aggregate_snapshot_count".to_string(),
        JsonValue::Number(remote_to_revision_distance.unwrap_or_default().into()),
    );
    candidate.insert(
        "remote_already_contains_revision".to_string(),
        JsonValue::Bool(remote_already_contains_revision),
    );
    Ok(JsonValue::Object(candidate))
}

fn workflow_same_head_published_remote_change_id(
    entry: &JsonValue,
    revision_snapshot_id: &str,
) -> Result<String, String> {
    let state = entry.get("state").cloned().unwrap_or_else(|| json!({}));
    let task = state.get("task").cloned().unwrap_or(JsonValue::Null);
    let change = state.get("change").cloned().unwrap_or(JsonValue::Null);
    let local_task_id = required_string_field(&task, "task_id")?;
    let local_change_id = required_string_field(&change, "change_id")?;
    let task_published = string_field(&task, "publication_state").as_deref() == Some("published");
    let change_published =
        string_field(&change, "publication_state").as_deref() == Some("published");
    let remote_task_id = string_field(&task, "published_task_id");
    let remote_change_id = string_field(&change, "published_change_id");
    if !task_published
        || !change_published
        || remote_task_id.is_none()
        || remote_change_id.is_none()
    {
        return Err(format!(
            "Remote target Line already equals final local Snapshot \
             `{revision_snapshot_id}`, but completed local Change {local_change_id} and Task \
             {local_task_id} do not have complete canonical remote publication mappings. \
             Refusing to treat this as completed promotion or skip aggregate Patchset CI; the \
             remote target head moved without provable remote Land authority."
        ));
    }
    Ok(remote_change_id.expect("checked above"))
}

pub(in crate::primitives) fn workflow_same_head_remote_land_authority(
    entry: &JsonValue,
    remote_change: Option<&JsonValue>,
    revision_snapshot_id: &str,
) -> Result<JsonValue, String> {
    let remote_change_id =
        workflow_same_head_published_remote_change_id(entry, revision_snapshot_id)?;
    let remote_change = remote_change.ok_or_else(|| {
        format!(
            "Remote target Line already equals final local Snapshot \
             `{revision_snapshot_id}`, but published Change {remote_change_id} could not be read. \
             Refusing to skip aggregate Patchset CI without provable remote Land authority."
        )
    })?;
    let remote_status = string_field(remote_change, "status");
    let remote_landed_snapshot_id = string_field(remote_change, "landed_snapshot_id");
    if remote_status.as_deref() != Some("landed")
        || remote_landed_snapshot_id.as_deref() != Some(revision_snapshot_id)
    {
        return Err(format!(
            "Remote target Line already equals final local Snapshot \
             `{revision_snapshot_id}`, but published Change {remote_change_id} is `{status}` at \
             `{landed_snapshot}`. Refusing to skip aggregate Patchset CI because no exact \
             successful remote Land owns the target-Line head.",
            status = remote_status.as_deref().unwrap_or("unknown"),
            landed_snapshot = remote_landed_snapshot_id.as_deref().unwrap_or("none"),
        ));
    }
    Ok(json!({
        "status": "verified",
        "authority": "remote_landed_change",
        "remote_change_id": remote_change_id,
        "remote_change_status": remote_status,
        "landed_snapshot_id": remote_landed_snapshot_id,
    }))
}

pub(in crate::primitives) fn workflow_final_snapshot_promotion_candidate(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<Option<JsonValue>, String> {
    let root_repo = workflow_root_repo(repo)?;
    let Some(entry) =
        workflow_completed_local_entry_if_present(&root_repo, change_id, remote_name)?
    else {
        return Ok(None);
    };
    let change = entry
        .get("state")
        .and_then(|state| state.get("change"))
        .cloned()
        .unwrap_or(JsonValue::Null);
    let target_line =
        string_field(&change, "base_line").unwrap_or_else(|| root_repo.default_line_name());
    let revision_snapshot_id = string_field(&change, "landed_snapshot_id").ok_or_else(|| {
        format!("Completed local change {change_id} is missing landed_snapshot_id.")
    })?;
    let local_target_head = local_line_head_snapshot_id(&root_repo, &target_line)?;
    let (remote_row, repo_name) = remote_context(&root_repo, remote_name, None)?;
    let mut task_remote = http_task_remote(&root_repo, &remote_row)?;
    let remote_line = task_remote
        .get_line(&repo_name, &target_line)
        .map_err(|err| err.to_string())?;
    let remote_target_head = string_field(&remote_line, "head_snapshot_id");
    let same_head_land_authority =
        if remote_target_head.as_deref() == Some(revision_snapshot_id.as_str()) {
            let remote_change_id =
                workflow_same_head_published_remote_change_id(&entry, &revision_snapshot_id)?;
            let remote_change = task_remote
                .get_change(&remote_change_id, Some(&repo_name))
                .map_err(|err| {
                    format!("Failed to verify remote Land authority for {remote_change_id}: {err}")
                })?;
            Some(workflow_same_head_remote_land_authority(
                &entry,
                Some(&remote_change),
                &revision_snapshot_id,
            )?)
        } else {
            None
        };
    for (label, snapshot_id) in [
        ("final local", Some(revision_snapshot_id.as_str())),
        ("remote base", remote_target_head.as_deref()),
    ] {
        let Some(snapshot_id) = snapshot_id else {
            continue;
        };
        if !local_snapshot_exists(&root_repo, snapshot_id)? {
            return Err(format!(
                "The {label} snapshot `{snapshot_id}` is not present in local storage. Pull/reconcile `{target_line}` from remote `{}` before promotion.",
                remote_row.name
            ));
        }
    }
    let distance = snapshot_distance_if_ancestor(
        &root_repo,
        remote_target_head.as_deref(),
        Some(&revision_snapshot_id),
    )?;
    let mut candidate = workflow_final_snapshot_candidate_from_entry(
        &entry,
        local_target_head.as_deref(),
        remote_target_head.as_deref(),
        distance,
    )?;
    let base_snapshot_id = required_string_field(&candidate, "base_snapshot_id")?;
    let (history_entries, plan_artifact_paths) = if base_snapshot_id == revision_snapshot_id {
        (Vec::new(), Vec::new())
    } else {
        workflow_local_history_entries(
            &root_repo,
            change_id,
            &target_line,
            &base_snapshot_id,
            &revision_snapshot_id,
        )?
    };
    let aggregate_snapshot_count = local_snapshot_chain_segment(
        &root_repo,
        &base_snapshot_id,
        &revision_snapshot_id,
        "workflow ready history promotion",
    )?
    .len();
    if let Some(object) = candidate.as_object_mut() {
        object.insert(
            "mode".to_string(),
            JsonValue::String("solo_local_history_promotion".to_string()),
        );
        object.insert("remote_line".to_string(), remote_line);
        object.insert(
            "remote_name".to_string(),
            JsonValue::String(remote_row.name),
        );
        object.insert("repo_name".to_string(), JsonValue::String(repo_name));
        object.insert(
            "aggregate_snapshot_count".to_string(),
            json!(aggregate_snapshot_count),
        );
        object.insert(
            "history_entry_count".to_string(),
            json!(history_entries.len()),
        );
        object.insert(
            "history_entries".to_string(),
            JsonValue::Array(history_entries),
        );
        object.insert(
            "plan_artifact_paths".to_string(),
            json!(plan_artifact_paths),
        );
        object.insert(
            "same_head_land_authority".to_string(),
            same_head_land_authority.unwrap_or(JsonValue::Null),
        );
    }
    Ok(Some(candidate))
}

pub(in crate::primitives) fn workflow_final_snapshot_promotion_remote_change_id(
    candidate: &JsonValue,
) -> Result<String, String> {
    candidate
        .get("state")
        .and_then(|state| state.get("change"))
        .and_then(|change| {
            string_field(change, "published_change_id")
                .or_else(|| string_field(change, "change_id"))
        })
        .ok_or_else(|| "Final-snapshot promotion candidate is missing change identity.".to_string())
}

pub(in crate::primitives) fn workflow_final_snapshot_promotion_preview(
    candidate: &JsonValue,
) -> Result<JsonValue, String> {
    let state = candidate.get("state").cloned().unwrap_or_else(|| json!({}));
    let task = state.get("task").cloned().unwrap_or(JsonValue::Null);
    let change = state.get("change").cloned().unwrap_or(JsonValue::Null);
    let local_change_id = required_string_field(&change, "change_id")?;
    let local_change_ref = change_reference_from_payload(&change, Some(local_change_id.as_str()))?;
    let target_line = string_field(&change, "base_line").unwrap_or_else(|| "main".to_string());
    let remote_name =
        string_field(candidate, "remote_name").unwrap_or_else(|| "origin".to_string());
    Ok(json!({
        "mode": "solo_local_history_promotion",
        "status": "ready",
        "apply_status": "preview",
        "task_id": string_field(&task, "task_id"),
        "change_id": local_change_id,
        "change_ref": local_change_ref,
        "local_task_id": string_field(&task, "task_id"),
        "local_change_id": string_field(&change, "change_id"),
        "local_change_ref": local_change_ref,
        "remote_task_id": string_field(&task, "published_task_id"),
        "remote_change_id": workflow_final_snapshot_promotion_remote_change_id(candidate)?,
        "routing": state.get("routing").cloned().unwrap_or(JsonValue::Null),
        "task": task,
        "change": change,
        "base_line": {
            "line_name": target_line,
            "head_snapshot_id": candidate.get("base_snapshot_id").cloned().unwrap_or(JsonValue::Null),
        },
        "patchset": JsonValue::Null,
        "history_entry_count": candidate.get("history_entry_count").cloned().unwrap_or(JsonValue::Null),
        "plan_artifact_paths": candidate.get("plan_artifact_paths").cloned().unwrap_or_else(|| json!([])),
        "final_snapshot_promotion": candidate,
        "next_action": {
            "code": "prepare_final_snapshot_promotion",
            "summary": "Promote the consecutive local Task/Change/Snapshot/Land history and run shared CI once on its aggregate Patchset.",
            "detail": format!("Run `ait workflow ready {local_change_ref} --apply --remote {remote_name}`. After it is ready, run `ait task land {local_change_ref} --remote {remote_name}`."),
            "command": format!("ait workflow ready {local_change_ref} --apply --remote {remote_name}"),
        },
    }))
}

fn workflow_sync_history_plan_artifacts(
    repo: &RepoRuntime,
    remote_name: &str,
    candidate: &JsonValue,
) -> Result<Vec<JsonValue>, String> {
    candidate
        .get("plan_artifact_paths")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|value| {
            let artifact_path = value
                .as_str()
                .and_then(|value| normalized_text(Some(value)))
                .ok_or_else(|| {
                    "History promotion Plan artifact path must be a non-empty string.".to_string()
                })?;
            let request = plan_sync_request(repo, &artifact_path, None, Some(remote_name), false)?;
            let sync = execute_plan_sync_command_request_json(&request.to_string())?;
            if sync.get("status").and_then(JsonValue::as_str) != Some("ok") {
                return Err(format!(
                    "History promotion Plan sync for {artifact_path} did not succeed: {}",
                    string_field(&sync, "error")
                        .unwrap_or_else(|| "non-ok Plan sync result".to_string())
                ));
            }
            Ok(json!({
                "artifact_path": artifact_path,
                "result": sync,
            }))
        })
        .collect()
}

fn workflow_history_prepare_entries(
    repo: &RepoRuntime,
    candidate: &JsonValue,
) -> Result<Vec<JsonValue>, String> {
    candidate
        .get("history_entries")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "History promotion candidate is missing history_entries.".to_string())?
        .iter()
        .map(|entry| {
            let task = entry.get("task").ok_or_else(|| {
                "History promotion entry is missing local Task projection.".to_string()
            })?;
            let change = entry.get("change").ok_or_else(|| {
                "History promotion entry is missing local Change projection.".to_string()
            })?;
            let (published_plan_id, published_revision_id, published_plan_item_ref) =
                published_local_task_plan_linkage(repo, task)?;
            Ok(json!({
                "local_task_id": required_string_field(entry, "local_task_id")?,
                "local_change_id": required_string_field(entry, "local_change_id")?,
                "local_change_ref": required_string_field(entry, "local_change_ref")?,
                "task": {
                    "title": required_string_field(task, "title")?,
                    "intent": required_string_field(task, "intent")?,
                    "plan_id": published_plan_id,
                    "origin_plan_revision_id": published_revision_id,
                    "plan_item_ref": published_plan_item_ref,
                },
                "change": {
                    "title": required_string_field(change, "title")?,
                    "base_line": required_string_field(change, "base_line")?,
                    "fork_snapshot_id": required_string_field(change, "fork_snapshot_id")?,
                },
                "pre_land_target_snapshot_id": required_string_field(entry, "pre_land_target_snapshot_id")?,
                "landed_snapshot_id": required_string_field(entry, "landed_snapshot_id")?,
                "landed_at_s": entry.get("landed_at_s").cloned().ok_or_else(|| {
                    "History promotion entry is missing landed_at_s.".to_string()
                })?,
                "snapshots": entry.get("snapshots").cloned().ok_or_else(|| {
                    "History promotion entry is missing Snapshot inventory.".to_string()
                })?,
            }))
        })
        .collect()
}

fn workflow_history_idempotency_key(
    repo: &RepoRuntime,
    target_line: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    entries: &[JsonValue],
) -> Result<String, String> {
    let mut bytes = Vec::new();
    let repository_index = repo.require_repository_index()?.to_string();
    for part in [
        "history-promotion-prepare/v1",
        repository_index.as_str(),
        target_line,
        base_snapshot_id,
        revision_snapshot_id,
    ] {
        bytes.extend_from_slice(part.as_bytes());
        bytes.push(0);
    }
    for entry in entries {
        for field in [
            "local_task_id",
            "local_change_ref",
            "pre_land_target_snapshot_id",
            "landed_snapshot_id",
        ] {
            bytes.extend_from_slice(required_string_field(entry, field)?.as_bytes());
            bytes.push(0);
        }
    }
    Ok(format!("history-promotion:{}", sha256_hex_bytes(&bytes)))
}

pub(in crate::primitives) fn workflow_mark_history_published(
    repo: &RepoRuntime,
    remote_name: &str,
    candidate_entries: &[JsonValue],
    response_entries: &[JsonValue],
) -> Result<Vec<JsonValue>, String> {
    if candidate_entries.len() != response_entries.len() {
        return Err(
            "History promotion response mapping count changed after validation.".to_string(),
        );
    }
    let task_store = repo.task_store()?;
    let change_store = repo.change_store()?;
    candidate_entries
        .iter()
        .zip(response_entries.iter())
        .map(|(local, remote)| {
            let local_task_id = required_string_field(local, "local_task_id")?;
            let local_change_id = required_string_field(local, "local_change_id")?;
            let local_change_ref = required_string_field(local, "local_change_ref")?;
            let remote_task_id = required_string_field(remote, "task_id")?;
            let remote_change_ref = required_string_field(remote, "change_ref")?;
            let task = task_local_mark_published_with_task_store(
                &task_store,
                &local_task_id,
                Some(remote_name),
                Some(&remote_task_id),
            )?;
            let change = change_local_mark_published_with_change_store(
                &change_store,
                &local_change_ref,
                Some(remote_name),
                Some(&remote_change_ref),
                true,
            )?;
            Ok(json!({
                "local_task_id": local_task_id,
                "local_change_id": local_change_id,
                "local_change_ref": local_change_ref,
                "remote_task_id": remote_task_id,
                "remote_change_ref": remote_change_ref,
                "task": task,
                "change": change,
            }))
        })
        .collect()
}

pub(in crate::primitives) fn workflow_prepare_final_snapshot_promotion(
    repo: &RepoRuntime,
    change_id: &str,
    summary: Option<&str>,
    author_mode: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let _prepare_range = perfetto_range!("ait.workflow_ready.history_promotion.prepare");
    let root_repo = workflow_root_repo(repo)?;
    let candidate = {
        let _range = perfetto_range!("ait.workflow_ready.history_promotion.collect");
        workflow_final_snapshot_promotion_candidate(&root_repo, change_id, remote_name)?
            .ok_or_else(|| {
                format!("Local change {change_id} is not a completed history-promotion candidate.")
            })?
    };
    if candidate
        .get("remote_already_contains_revision")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return Err(format!(
            "Remote target already contains the final local snapshot for {change_id}; no aggregate patchset or CI run is needed. Inspect the remote change/land state instead."
        ));
    }
    let state = candidate.get("state").cloned().unwrap_or_else(|| json!({}));
    let task = state.get("task").cloned().unwrap_or(JsonValue::Null);
    let change = state.get("change").cloned().unwrap_or(JsonValue::Null);
    let task_id = required_string_field(&task, "task_id")?;
    let local_change_id = required_string_field(&change, "change_id")?;
    let base_line =
        string_field(&change, "base_line").unwrap_or_else(|| root_repo.default_line_name());
    let base_snapshot_id = required_string_field(&candidate, "base_snapshot_id")?;
    let revision_snapshot_id = required_string_field(&candidate, "revision_snapshot_id")?;
    let (remote_row, repo_name) = remote_context(&root_repo, remote_name, None)?;
    let candidate_entries = candidate
        .get("history_entries")
        .and_then(JsonValue::as_array)
        .cloned()
        .ok_or_else(|| "History promotion candidate has no ordered entries.".to_string())?;
    let plan_sync = {
        let _range = perfetto_range!("ait.workflow_ready.history_promotion.plan_sync");
        workflow_sync_history_plan_artifacts(&root_repo, remote_row.name.as_str(), &candidate)?
    };
    let request_entries = {
        let _range = perfetto_range!("ait.workflow_ready.history_promotion.request_build");
        workflow_history_prepare_entries(&root_repo, &candidate)?
    };
    let snapshot_sync = {
        let _range = perfetto_range!("ait.workflow_ready.history_promotion.snapshot_sync");
        sync_patchset_revision_snapshot(
            &root_repo,
            &remote_row,
            &repo_name,
            &base_line,
            &revision_snapshot_id,
            &base_line,
        )?
    };
    let idempotency_key = workflow_history_idempotency_key(
        &root_repo,
        &base_line,
        &base_snapshot_id,
        &revision_snapshot_id,
        &request_entries,
    )?;
    let request = json!({
        "contract": "history-promotion-prepare/v1",
        "idempotency_key": idempotency_key,
        "target_line": base_line,
        "base_snapshot_id": base_snapshot_id,
        "revision_snapshot_id": revision_snapshot_id,
        "author_mode": root_repo.effective_author_mode(author_mode),
        "summary": summary.unwrap_or("solo-local workflow history promotion"),
        "entries": request_entries,
    });
    let mut closeout_remote = http_closeout_remote(&root_repo, &remote_row)?;
    let prepared = {
        let _range = perfetto_range!("ait.workflow_ready.history_promotion.http");
        TaskWorkflowHistoryPromotionPreparer::prepare_history_promotion(
            &mut closeout_remote,
            &repo_name,
            &request,
        )
        .map_err(|error| error.to_string())?
    };
    let response_entries = prepared
        .get("entries")
        .and_then(JsonValue::as_array)
        .cloned()
        .ok_or_else(|| "History promotion response has no mappings.".to_string())?;
    let publication_mappings = {
        let _range = perfetto_range!("ait.workflow_ready.history_promotion.local_mapping");
        workflow_mark_history_published(
            &root_repo,
            remote_row.name.as_str(),
            &candidate_entries,
            &response_entries,
        )?
    };
    let aggregate = prepared
        .get("aggregate")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "History promotion response is missing aggregate authority.".to_string())?;
    let remote_task_id = aggregate
        .get("task_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "History promotion aggregate is missing task_id.".to_string())?;
    let remote_change_id = aggregate
        .get("change_ref")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "History promotion aggregate is missing change_ref.".to_string())?;
    let patchset_id = aggregate
        .get("patchset_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| "History promotion aggregate is missing patchset_id.".to_string())?;
    let patchset = aggregate
        .get("patchset")
        .cloned()
        .ok_or_else(|| "History promotion aggregate is missing Patchset projection.".to_string())?;
    Ok(json!({
        "mode": "solo_local_history_promotion",
        "routing": state.get("routing").cloned().unwrap_or(JsonValue::Null),
        "local_task_id": task_id,
        "local_change_id": local_change_id,
        "remote_task_id": remote_task_id,
        "remote_change_id": remote_change_id,
        "base_snapshot_id": base_snapshot_id,
        "revision_snapshot_id": revision_snapshot_id,
        "aggregate_snapshot_count": candidate.get("aggregate_snapshot_count").cloned().unwrap_or(JsonValue::Null),
        "history_entry_count": candidate_entries.len(),
        "candidate": candidate,
        "plan_sync": plan_sync,
        "snapshot_sync": snapshot_sync,
        "patchset": patchset,
        "patchset_id": patchset_id,
        "selection": {
            "status": "selected_by_history_prepare",
            "patchset_id": patchset_id,
        },
        "publication_mappings": publication_mappings,
        "history_promotion": prepared,
    }))
}

pub fn workflow_land_completed_local_payload(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let candidate = workflow_final_snapshot_promotion_candidate(repo, change_id, remote_name)?
        .ok_or_else(|| {
            format!("Local change {change_id} is not a completed history-promotion candidate.")
        })?;
    workflow_final_snapshot_promotion_preview(&candidate)
}

#[allow(clippy::too_many_arguments)]
pub fn workflow_land_completed_local_apply<F>(
    _repo: &RepoRuntime,
    change_id: &str,
    _summary: Option<&str>,
    _tests: Option<&str>,
    _lint: Option<&str>,
    _security: Option<&str>,
    _license: Option<&str>,
    _author_mode: Option<&str>,
    _model: Option<&str>,
    _reviewer: Option<&str>,
    _review_message: Option<&str>,
    _target: Option<&str>,
    _mode: &str,
    remote_name: Option<&str>,
    _progress: Option<F>,
) -> Result<JsonValue, String>
where
    F: FnMut(&JsonValue) -> Result<(), String>,
{
    let remote_name = normalized_text(remote_name).unwrap_or_else(|| "origin".to_string());
    Err(format!(
        "Completed local change {change_id} must pass the explicit ready phase before remote land. Run `ait workflow ready {change_id} --apply --remote {remote_name}`, then `ait task land {change_id} --remote {remote_name}`."
    ))
}

pub fn workflow_completed_local_batch_retired_error() -> String {
    COMPLETED_LOCAL_BATCH_RETIRED_ERROR.to_string()
}
