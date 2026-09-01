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

const HISTORY_PROMOTION_STAGE_ENTRY_COUNT: usize = 64;
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

pub(in crate::primitives) fn workflow_unique_history_plan_publications(
    publications: impl IntoIterator<Item = (String, String)>,
) -> Result<Vec<(String, String)>, String> {
    let mut unique = BTreeMap::new();
    for (plan_id, artifact_path) in publications {
        if let Some(existing_path) = unique.get(&plan_id) {
            if existing_path != &artifact_path {
                return Err(format!(
                    "History promotion Plan {plan_id} resolves to conflicting head artifact paths {existing_path} and {artifact_path}."
                ));
            }
            continue;
        }
        unique.insert(plan_id, artifact_path);
    }
    Ok(unique.into_iter().collect())
}

fn workflow_history_plan_publications(
    repo: &RepoRuntime,
    candidate: &JsonValue,
) -> Result<Vec<(String, String)>, String> {
    let entries = candidate
        .get("history_entries")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "History promotion candidate is missing history_entries.".to_string())?;
    let plan_store = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .plans();
    let mut publications = Vec::new();
    for entry in entries {
        let task = entry
            .get("task")
            .ok_or_else(|| "History promotion entry is missing local Task data.".to_string())?;
        let plan_id = string_field(task, "plan_id");
        let revision_id = string_field(task, "origin_plan_revision_id");
        match (plan_id.as_deref(), revision_id.as_deref()) {
            (None, None) => continue,
            (Some(_), None) | (None, Some(_)) => {
                return Err(format!(
                    "Local task {} has incomplete Plan linkage.",
                    required_string_field(task, "task_id")?
                ))
            }
            (Some(plan_id), Some(revision_id)) => {
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
                let plan = get_plan_with_plan_store(&plan_store, plan_id)
                    .map_err(|error| error.to_string())?;
                let artifact_path = plan
                    .head_revision
                    .as_ref()
                    .map(|head| head.artifact_path.clone())
                    .filter(|path| !path.trim().is_empty())
                    .ok_or_else(|| {
                        format!("Local history Plan {plan_id} has no head artifact path.")
                    })?;
                publications.push((plan_id.to_string(), artifact_path));
            }
        }
    }
    workflow_unique_history_plan_publications(publications)
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

/// Walk an unowned pre-land boundary back to the nearest owned Snapshot or
/// the promotion base. Only single-parent `line`-kind Snapshots on the target
/// Line are adoptable; they upload as ancestry of the following landed Change
/// and are never replayed as a remote head. Every other unowned boundary
/// keeps the fail-closed gap error.
fn workflow_adopt_unowned_direct_boundary(
    repo: &RepoRuntime,
    owned_boundaries: &BTreeMap<String, Vec<&JsonValue>>,
    local_change_ref: &str,
    target_line: &str,
    base_snapshot_id: &str,
    boundary_snapshot_id: &str,
) -> Result<(String, Vec<String>), String> {
    let mut cursor = boundary_snapshot_id.to_string();
    let mut adopted = Vec::new();
    for _ in 0..=MAX_HISTORY_PROMOTION_SNAPSHOTS_PER_ENTRY {
        if cursor == base_snapshot_id || owned_boundaries.contains_key(&cursor) {
            return Ok((cursor, adopted));
        }
        let snapshot = snapshot_show(repo, &cursor)?;
        let snapshot_kind =
            string_field(&snapshot, "snapshot_kind").unwrap_or_else(|| "line".to_string());
        let snapshot_line = required_string_field(&snapshot, "line_name")?;
        if snapshot_kind != "line" || snapshot_line != target_line {
            return Err(format!(
                "Local Land history has a gap before Snapshot `{cursor}`; no landed Change on `{target_line}` owns that target head, and the unowned boundary is not an adoptable direct `{target_line}` Snapshot (kind `{snapshot_kind}`, line `{snapshot_line}`)."
            ));
        }
        let parents = workflow_history_snapshot_parent_ids(&snapshot)?;
        let [parent] = parents.as_slice() else {
            return Err(format!(
                "Local Land history has a gap before Snapshot `{cursor}`; adopting an unowned direct boundary requires exactly one parent, found {}.",
                parents.len()
            ));
        };
        adopted.push(cursor.clone());
        cursor = parent.clone();
    }
    Err(format!(
        "Local Land history adoption before Change {local_change_ref} exceeds the bounded maximum of {MAX_HISTORY_PROMOTION_SNAPSHOTS_PER_ENTRY} unowned boundary Snapshots."
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
    let mut landed_changes_by_snapshot = BTreeMap::<String, Vec<&JsonValue>>::new();
    for row in &change_rows {
        if string_field(row, "status").as_deref() != Some("landed")
            || string_field(row, "target_line")
                .or_else(|| string_field(row, "base_line"))
                .as_deref()
                != Some(target_line)
        {
            continue;
        }
        if let Some(landed_snapshot_id) = string_field(row, "landed_snapshot_id") {
            landed_changes_by_snapshot
                .entry(landed_snapshot_id)
                .or_default()
                .push(row);
        }
    }
    let mut current_snapshot_id = revision_snapshot_id.to_string();
    let mut reversed = Vec::new();
    let mut seen_tasks = BTreeSet::new();
    let mut seen_changes = BTreeSet::new();
    let mut plan_artifact_paths = Vec::new();

    while current_snapshot_id != base_snapshot_id {
        let change = match landed_changes_by_snapshot
            .get(&current_snapshot_id)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
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
        let (pre_land_snapshot_id, adopted_boundary_snapshot_ids) =
            workflow_adopt_unowned_direct_boundary(
                repo,
                &landed_changes_by_snapshot,
                &local_change_ref,
                target_line,
                base_snapshot_id,
                &pre_land_snapshot_id,
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
            "adopted_boundary_snapshot_ids": adopted_boundary_snapshot_ids,
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
    local_target_head_contains_revision: bool,
    remote_target_head: Option<&str>,
    null_remote_base_snapshot_id: Option<&str>,
    remote_to_revision_distance: Option<i64>,
) -> Result<JsonValue, String> {
    let state = entry.get("state").cloned().unwrap_or_else(|| json!({}));
    let task = state.get("task").cloned().unwrap_or(JsonValue::Null);
    let change = state.get("change").cloned().unwrap_or(JsonValue::Null);
    let local_change_id = required_string_field(&change, "change_id")?;
    let target_line = string_field(&change, "base_line").unwrap_or_else(|| "main".to_string());
    let revision_snapshot_id = string_field(&change, "landed_snapshot_id").ok_or_else(|| {
        format!("Completed local change {local_change_id} is missing landed_snapshot_id.")
    })?;
    let local_target_head = normalized_text(local_target_head).ok_or_else(|| {
        format!("Local target line `{target_line}` has no head snapshot to promote.")
    })?;
    let local_target_head_is_revision = local_target_head == revision_snapshot_id;
    let published_descendant_resume = !local_target_head_is_revision
        && local_target_head_contains_revision
        && string_field(&task, "publication_state").as_deref() == Some("published")
        && string_field(&change, "publication_state").as_deref() == Some("published");
    if !local_target_head_is_revision && !published_descendant_resume {
        return Err(format!(
            "Completed local change {local_change_id} landed at `{revision_snapshot_id}`, but `{target_line}` is now at `{local_target_head}`. Only the latest completed local change that owns the current target-line head can be promoted; an already-published Change may resume only when Snapshot ancestry proves that the current head contains its published revision. Select the change for `{local_target_head}` or repair the divergent publication state."
        ));
    }
    let remote_head_initialization_required = normalized_text(remote_target_head).is_none();
    let base_snapshot_id = normalized_text(remote_target_head)
        .or_else(|| normalized_text(null_remote_base_snapshot_id))
        .ok_or_else(|| {
            format!(
                "Remote target line `{target_line}` has no head snapshot and completed local change {local_change_id} has no admissible pre-land bootstrap boundary."
            )
        })?;
    if remote_head_initialization_required && base_snapshot_id == revision_snapshot_id {
        return Err(format!(
            "Refusing to initialize null remote `{target_line}` directly at final local Snapshot `{revision_snapshot_id}`; completed local change {local_change_id} requires an earlier pre-land bootstrap boundary."
        ));
    }
    let remote_already_contains_revision =
        !remote_head_initialization_required && base_snapshot_id == revision_snapshot_id;
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
        "local_target_head_snapshot_id".to_string(),
        JsonValue::String(local_target_head),
    );
    candidate.insert(
        "local_target_head_is_revision".to_string(),
        JsonValue::Bool(local_target_head_is_revision),
    );
    candidate.insert(
        "local_target_head_contains_revision".to_string(),
        JsonValue::Bool(!local_target_head_is_revision && local_target_head_contains_revision),
    );
    candidate.insert(
        "published_descendant_resume".to_string(),
        JsonValue::Bool(published_descendant_resume),
    );
    candidate.insert(
        "aggregate_snapshot_count".to_string(),
        JsonValue::Number(remote_to_revision_distance.unwrap_or_default().into()),
    );
    candidate.insert(
        "remote_already_contains_revision".to_string(),
        JsonValue::Bool(remote_already_contains_revision),
    );
    candidate.insert(
        "remote_head_initialization_required".to_string(),
        JsonValue::Bool(remote_head_initialization_required),
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
             {local_task_id} do not have complete remote publication records. \
             Refusing to treat this as completed promotion or skip aggregate Patchset CI; the \
             remote target head moved without a matching successful remote Land."
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
             Aggregate Patchset CI cannot be skipped without a matching successful remote Land."
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

pub(in crate::primitives) fn workflow_initialize_null_remote_base_with_task_remote<R>(
    repo: &RepoRuntime,
    remote_row: &RemoteRow,
    task_remote: &mut R,
    repo_name: &str,
    target_line: &str,
    base_snapshot_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader
        + TaskWorkflowRepositoryReader
        + TaskWorkflowZstdPackUploader
        + ?Sized,
{
    if target_line != repo.default_line_name() {
        return Err(format!(
            "Completed-local null-head initialization only admits the configured default Line `{}`; got `{target_line}`.",
            repo.default_line_name()
        ));
    }
    let preflight_line = super::super::remote_sync::remote_sync_line_read_with_task_remote(
        task_remote,
        repo_name,
        target_line,
    )?;
    if let Some(current_head_snapshot_id) = string_field(&preflight_line, "head_snapshot_id") {
        if current_head_snapshot_id == base_snapshot_id {
            return Ok(json!({
                "status": "already_initialized",
                "reason": "remote_null_head_promotion_base_already_selected",
                "line": preflight_line,
                "head_snapshot_id": current_head_snapshot_id,
                "snapshot_sync": JsonValue::Null,
            }));
        }
        return Err(format!(
            "Cannot prepare completed-local promotion: Remote `{}` target Line `{target_line}` moved from null to `{current_head_snapshot_id}` while this promotion selected pre-land base `{base_snapshot_id}`. Refusing to create Remote publication authority from a different base.",
            remote_row.name,
        ));
    }

    let remote_repository = read_remote_repository_authority(repo, task_remote, repo_name)?;
    let remote_sync_capabilities =
        RemoteSyncCapabilities::from_server_payload(Some(&remote_repository));
    let initialization =
        super::super::remote_sync::initialize_remote_null_head_line_with_snapshot_via_zstd(
            repo,
            task_remote,
            repo_name,
            target_line,
            base_snapshot_id,
            &remote_sync_capabilities,
        );
    match initialization {
        Ok(snapshot_sync) => {
            let line = super::super::remote_sync::remote_sync_line_read_with_task_remote(
                task_remote,
                repo_name,
                target_line,
            )?;
            let initialized_head_snapshot_id = string_field(&line, "head_snapshot_id");
            if initialized_head_snapshot_id.as_deref() != Some(base_snapshot_id) {
                return Err(format!(
                    "Completed-local null-head initialization returned `{}` for Remote `{}` target Line `{target_line}` instead of selected pre-land base `{base_snapshot_id}`.",
                    initialized_head_snapshot_id.as_deref().unwrap_or("null"),
                    remote_row.name,
                ));
            }
            Ok(json!({
                "status": "initialized",
                "reason": "remote_null_head_seeded_from_completed_local_pre_land_base",
                "line": line,
                "head_snapshot_id": base_snapshot_id,
                "snapshot_sync": snapshot_sync,
                "remote_repository": remote_repository,
            }))
        }
        Err(initialization_error) => {
            let winner = super::super::remote_sync::remote_sync_line_read_with_task_remote(
                task_remote,
                repo_name,
                target_line,
            )
            .map_err(|readback_error| {
                format!(
                    "Completed-local null-head initialization failed: {initialization_error} Readback of Remote `{}` target Line `{target_line}` also failed: {readback_error}",
                    remote_row.name,
                )
            })?;
            let winner_snapshot_id = string_field(&winner, "head_snapshot_id");
            if winner_snapshot_id.as_deref() == Some(base_snapshot_id) {
                return Ok(json!({
                    "status": "initialized",
                    "reason": "remote_null_head_seeded_from_completed_local_pre_land_base_after_uncertain_response",
                    "line": winner,
                    "head_snapshot_id": base_snapshot_id,
                    "snapshot_sync": JsonValue::Null,
                    "initialization_error": initialization_error,
                    "remote_repository": remote_repository,
                }));
            }
            Err(format!(
                "Cannot prepare completed-local promotion: Remote `{}` target Line `{target_line}` was initialized concurrently at `{}` while this promotion selected pre-land base `{base_snapshot_id}`. Refusing to create Remote Task, Change, Patchset, or publication mappings from a different base. Initial synchronization failed: {initialization_error}",
                remote_row.name,
                winner_snapshot_id.as_deref().unwrap_or("null"),
            ))
        }
    }
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
    let (null_remote_base_snapshot_id, null_remote_base_recovered) = if remote_target_head.is_none()
    {
        if target_line != root_repo.default_line_name() {
            return Err(format!(
                    "Completed-local null-head promotion only admits the configured default Line `{}`; got `{target_line}`.",
                    root_repo.default_line_name()
                ));
        }
        let recorded_pre_land_snapshot_id =
                string_field(&change, "pre_land_target_snapshot_id").ok_or_else(|| {
                    format!(
                        "Completed local change {change_id} is missing pre_land_target_snapshot_id required to initialize null remote `{target_line}`."
                    )
                })?;
        let (base_snapshot_id, recovered) = workflow_effective_pre_land_target_snapshot_id(
            &root_repo,
            &change,
            &revision_snapshot_id,
            &recorded_pre_land_snapshot_id,
        )?;
        (Some(base_snapshot_id), Some(recovered))
    } else {
        (None, None)
    };
    let same_head_land_authority =
        if remote_target_head.as_deref() == Some(revision_snapshot_id.as_str()) {
            let remote_change_id =
                workflow_same_head_published_remote_change_id(&entry, &revision_snapshot_id)?;
            let remote_change = task_remote
                .get_change(&remote_change_id, Some(&repo_name))
                .map_err(|err| {
                    format!("Failed to verify the remote Land for {remote_change_id}: {err}")
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
        (
            "remote initialization base",
            null_remote_base_snapshot_id.as_deref(),
        ),
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
        remote_target_head
            .as_deref()
            .or(null_remote_base_snapshot_id.as_deref()),
        Some(&revision_snapshot_id),
    )?;
    let local_target_head_contains_revision = match local_target_head.as_deref() {
        Some(local_head) if local_head != revision_snapshot_id => snapshot_distance_if_ancestor(
            &root_repo,
            Some(&revision_snapshot_id),
            Some(local_head),
        )?
        .is_some(),
        _ => false,
    };
    let mut candidate = workflow_final_snapshot_candidate_from_entry(
        &entry,
        local_target_head.as_deref(),
        local_target_head_contains_revision,
        remote_target_head.as_deref(),
        null_remote_base_snapshot_id.as_deref(),
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
        object.insert(
            "remote_base_initialization_source".to_string(),
            match null_remote_base_recovered {
                Some(true) => JsonValue::String("task_owned_snapshot_lineage_recovery".to_string()),
                Some(false) => JsonValue::String("land_record".to_string()),
                None => JsonValue::Null,
            },
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
            "detail": format!("Run `ait workflow ready {local_change_ref} --apply --remote {remote_name}`. After it is ready, hand the selected Patchset to a reviewer running `ait workflow finish {local_change_ref} --apply --remote {remote_name}`."),
            "command": format!("ait workflow ready {local_change_ref} --apply --remote {remote_name}"),
        },
    }))
}

fn workflow_sync_history_plan_artifacts(
    repo: &RepoRuntime,
    remote_name: &str,
    candidate: &JsonValue,
) -> Result<Vec<JsonValue>, String> {
    workflow_history_plan_publications(repo, candidate)?
        .into_iter()
        .map(|(plan_id, artifact_path)| {
            let mut request =
                plan_sync_request(repo, &artifact_path, None, Some(remote_name), false)?;
            let request_object = request.as_object_mut().ok_or_else(|| {
                "History promotion Plan publication request must be an object.".to_string()
            })?;
            request_object.insert("rebase".to_string(), JsonValue::Bool(false));
            request_object.insert(
                "history_publish_plan_id".to_string(),
                JsonValue::String(plan_id.clone()),
            );
            let sync = execute_plan_sync_command_request_json(&request.to_string())?;
            if sync.get("status").and_then(JsonValue::as_str) != Some("ok") {
                return Err(format!(
                    "History promotion exact Plan publication for {plan_id} ({artifact_path}) did not succeed: {}",
                    string_field(&sync, "error")
                        .unwrap_or_else(|| "non-ok Plan sync result".to_string())
                ));
            }
            Ok(json!({
                "plan_id": plan_id,
                "artifact_path": artifact_path,
                "result": sync,
            }))
        })
        .collect()
}

type WorkflowRemotePlanLinkage = (Option<String>, Option<String>, Option<String>);

fn workflow_remote_plan_linkage_for_local_task(
    repo: &RepoRuntime,
    task: &JsonValue,
) -> Result<WorkflowRemotePlanLinkage, String> {
    let resolved_plan_id = string_field(task, "plan_id");
    let resolved_revision_id = string_field(task, "origin_plan_revision_id");
    let resolved_plan_item_ref = string_field(task, "plan_item_ref");
    let mode = repo
        .config
        .get("plan_task_binding_mode")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
        .unwrap_or_default();
    if resolved_plan_id.is_none() && resolved_revision_id.is_none() {
        if mode == "required" {
            return Err(
                "Required Plan/Task binding needs every local draft Task to keep its Plan link before remote promotion.".to_string(),
            );
        }
        if resolved_plan_item_ref.is_some() {
            return Err(
                "Local task plan metadata is incomplete: `plan_item_ref` requires plan linkage."
                    .to_string(),
            );
        }
        return Ok((None, None, None));
    }
    if matches!(mode.as_str(), "strict" | "required") && resolved_plan_item_ref.is_none() {
        return Err(
            "Strict or required plan/task binding requires `plan_item_ref` for remote promotion."
                .to_string(),
        );
    }
    let plan_store = repo
        .binary_db_stores::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()
        .plans();
    let linkage = resolve_reconciled_plan_publish_linkage_with_plan_store(
        &plan_store,
        resolved_plan_id.as_deref(),
        resolved_revision_id.as_deref(),
    )
    .map_err(|err| err.to_string())?;
    let resolved_plan_id = linkage.plan_id;
    let published_plan_id = linkage.published_plan_id.ok_or_else(|| {
        format!(
            "Local task {} is linked to unpublished local plan {resolved_plan_id}. Publish the plan first.",
            required_string_field(task, "task_id").unwrap_or_else(|_| "unknown task".to_string())
        )
    })?;
    let resolved_revision_id = linkage.plan_revision_id.ok_or_else(|| {
        format!(
            "Local task {} is linked to local plan {resolved_plan_id} without a stored revision id.",
            required_string_field(task, "task_id").unwrap_or_else(|_| "unknown task".to_string())
        )
    })?;
    let published_revision_id = linkage.published_plan_revision_id.ok_or_else(|| {
        format!(
            "Local task {} is linked to unpublished local plan revision {resolved_revision_id}. Publish the plan revision first.",
            required_string_field(task, "task_id").unwrap_or_else(|_| "unknown task".to_string())
        )
    })?;
    Ok((
        Some(published_plan_id),
        Some(published_revision_id),
        resolved_plan_item_ref,
    ))
}

pub(super) fn workflow_history_prepare_entries(
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
                "History promotion entry is missing local Task data.".to_string()
            })?;
            let change = entry.get("change").ok_or_else(|| {
                "History promotion entry is missing local Change data.".to_string()
            })?;
            let (expected_remote_task_id, expected_remote_change_ref) =
                workflow_expected_history_publication_ids(entry)?;
            let (published_plan_id, published_revision_id, published_plan_item_ref) =
                workflow_remote_plan_linkage_for_local_task(repo, task)?;
            Ok(json!({
                "local_task_id": required_string_field(entry, "local_task_id")?,
                "local_change_id": required_string_field(entry, "local_change_id")?,
                "local_change_ref": required_string_field(entry, "local_change_ref")?,
                "expected_remote_task_id": expected_remote_task_id,
                "expected_remote_change_ref": expected_remote_change_ref,
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

fn workflow_expected_history_publication_ids(
    entry: &JsonValue,
) -> Result<(Option<String>, Option<String>), String> {
    let task = entry
        .get("task")
        .ok_or_else(|| "History promotion entry is missing local Task data.".to_string())?;
    let change = entry
        .get("change")
        .ok_or_else(|| "History promotion entry is missing local Change data.".to_string())?;
    let local_task_id = required_string_field(entry, "local_task_id")?;
    let local_change_ref = required_string_field(entry, "local_change_ref")?;
    let task_is_published = string_field(task, "publication_state").as_deref() == Some("published");
    let change_is_published =
        string_field(change, "publication_state").as_deref() == Some("published");
    if change_is_published && !task_is_published {
        return Err(format!(
            "Local history Change {local_change_ref} is published while Task {local_task_id} is not."
        ));
    }
    let expected_remote_task_id = task_is_published
        .then(|| {
            string_field(task, "published_task_id").ok_or_else(|| {
                format!(
                    "Published local history Task {local_task_id} has no exact Remote Task identity."
                )
            })
        })
        .transpose()?;
    let expected_remote_change_ref = change_is_published
        .then(|| {
            string_field(change, "published_change_id").ok_or_else(|| {
                format!(
                    "Published local history Change {local_change_ref} has no exact Remote Change identity."
                )
            })
        })
        .transpose()?;
    if let (Some(remote_task_id), Some(remote_change_ref)) = (
        expected_remote_task_id.as_deref(),
        expected_remote_change_ref.as_deref(),
    ) {
        let remote_owner = remote_change_ref
            .rsplit_once('/')
            .map(|(owner, _)| owner)
            .ok_or_else(|| {
                format!("Published Remote Change identity {remote_change_ref} has no Task owner.")
            })?;
        if remote_owner != remote_task_id {
            return Err(format!(
                "Published history mapping disagrees on Remote ownership: Task {remote_task_id}, Change {remote_change_ref}."
            ));
        }
    }
    Ok((expected_remote_task_id, expected_remote_change_ref))
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

fn workflow_staged_history_promotion_id(
    repo: &RepoRuntime,
    target_line: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    entries: &[JsonValue],
) -> Result<String, String> {
    let mut bytes = Vec::new();
    let repository_index = repo.require_repository_index()?.to_string();
    for part in [
        "history-promotion-prepare/v2",
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
    Ok(format!("history-promotion-v2:{}", sha256_hex_bytes(&bytes)))
}

fn workflow_history_stage_idempotency_key(
    promotion_id: &str,
    stage_ordinal: u64,
    stage_base_snapshot_id: &str,
    stage_revision_snapshot_id: &str,
    entries: &[JsonValue],
) -> Result<String, String> {
    let mut bytes = Vec::new();
    for part in [
        "history-promotion-stage/v1",
        promotion_id,
        stage_base_snapshot_id,
        stage_revision_snapshot_id,
    ] {
        bytes.extend_from_slice(part.as_bytes());
        bytes.push(0);
    }
    bytes.extend_from_slice(&stage_ordinal.to_le_bytes());
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
    Ok(format!(
        "history-promotion-stage:{}",
        sha256_hex_bytes(&bytes)
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn workflow_staged_history_prepare_request(
    promotion_id: &str,
    target_line: &str,
    base_snapshot_id: &str,
    revision_snapshot_id: &str,
    stage_ordinal: u64,
    total_entry_count: u64,
    previous_stage_patchset_id: Option<&str>,
    author_mode: &str,
    summary: &str,
    entries: &[JsonValue],
) -> Result<JsonValue, String> {
    let stage_start = stage_ordinal
        .checked_mul(HISTORY_PROMOTION_STAGE_ENTRY_COUNT as u64)
        .ok_or_else(|| "History promotion stage ordinal overflow.".to_string())?;
    if total_entry_count == 0 || stage_start >= total_entry_count {
        return Err("History promotion stage starts beyond its entry inventory.".to_string());
    }
    let expected_entry_count =
        (total_entry_count - stage_start).min(HISTORY_PROMOTION_STAGE_ENTRY_COUNT as u64);
    if entries.len() as u64 != expected_entry_count {
        return Err(format!(
            "History promotion stage {stage_ordinal} requires {expected_entry_count} entries, got {}.",
            entries.len()
        ));
    }
    let final_stage = stage_start + expected_entry_count == total_entry_count;
    if (stage_ordinal == 0) != previous_stage_patchset_id.is_none() {
        return Err(
            "History promotion's recorded predecessor does not match the stage number.".to_string(),
        );
    }
    let stage_base_snapshot_id = entries
        .first()
        .and_then(|entry| string_field(entry, "pre_land_target_snapshot_id"))
        .ok_or_else(|| "History promotion stage has no base Snapshot boundary.".to_string())?;
    let stage_revision_snapshot_id = entries
        .last()
        .and_then(|entry| string_field(entry, "landed_snapshot_id"))
        .ok_or_else(|| "History promotion stage has no revision Snapshot boundary.".to_string())?;
    if (stage_ordinal == 0 && stage_base_snapshot_id != base_snapshot_id)
        || (final_stage && stage_revision_snapshot_id != revision_snapshot_id)
    {
        return Err(
            "History promotion stage does not match its global Snapshot boundary.".to_string(),
        );
    }
    let idempotency_key = workflow_history_stage_idempotency_key(
        promotion_id,
        stage_ordinal,
        &stage_base_snapshot_id,
        &stage_revision_snapshot_id,
        entries,
    )?;
    Ok(json!({
        "contract": "history-promotion-prepare/v2",
        "promotion_id": promotion_id,
        "idempotency_key": idempotency_key,
        "target_line": target_line,
        "base_snapshot_id": base_snapshot_id,
        "revision_snapshot_id": revision_snapshot_id,
        "stage_ordinal": stage_ordinal,
        "stage_base_snapshot_id": stage_base_snapshot_id,
        "stage_revision_snapshot_id": stage_revision_snapshot_id,
        "previous_stage_patchset_id": previous_stage_patchset_id,
        "total_entry_count": total_entry_count,
        "final_stage": final_stage,
        "author_mode": author_mode,
        "summary": summary,
        "entries": entries,
    }))
}

pub(in crate::primitives) fn workflow_mark_history_published(
    repo: &RepoRuntime,
    remote_name: &str,
    candidate_entries: &[JsonValue],
    response_entries: &[JsonValue],
) -> Result<Vec<JsonValue>, String> {
    let validated =
        workflow_validate_history_publication_response(candidate_entries, response_entries)?;
    let task_store = repo.task_store()?;
    let change_store = repo.change_store()?;
    validated
        .into_iter()
        .map(|mapping| {
            let task = task_local_mark_published_with_task_store(
                &task_store,
                &mapping.local_task_id,
                Some(remote_name),
                Some(&mapping.remote_task_id),
            )?;
            let change = change_local_mark_published_with_change_store(
                &change_store,
                &mapping.local_change_ref,
                Some(remote_name),
                Some(&mapping.remote_change_ref),
                true,
            )?;
            Ok(json!({
                "local_task_id": mapping.local_task_id,
                "local_change_id": mapping.local_change_id,
                "local_change_ref": mapping.local_change_ref,
                "remote_task_id": mapping.remote_task_id,
                "remote_change_ref": mapping.remote_change_ref,
                "task": task,
                "change": change,
            }))
        })
        .collect()
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct ValidatedHistoryPublicationMapping {
    local_task_id: String,
    local_change_id: String,
    local_change_ref: String,
    remote_task_id: String,
    remote_change_ref: String,
}

pub(super) fn workflow_validate_history_publication_response(
    candidate_entries: &[JsonValue],
    response_entries: &[JsonValue],
) -> Result<Vec<ValidatedHistoryPublicationMapping>, String> {
    if candidate_entries.len() != response_entries.len() {
        return Err(
            "History promotion response mapping count changed after validation.".to_string(),
        );
    }
    let mut remote_task_ids = std::collections::BTreeSet::new();
    let mut remote_change_refs = std::collections::BTreeSet::new();
    candidate_entries
        .iter()
        .zip(response_entries.iter())
        .map(|(local, remote)| {
            let local_task_id = required_string_field(local, "local_task_id")?;
            let local_change_id = required_string_field(local, "local_change_id")?;
            let local_change_ref = required_string_field(local, "local_change_ref")?;
            for (field, expected) in [
                ("local_task_id", local_task_id.as_str()),
                ("local_change_id", local_change_id.as_str()),
                ("local_change_ref", local_change_ref.as_str()),
            ] {
                let actual = required_string_field(remote, field)?;
                if actual != expected {
                    return Err(format!(
                        "History promotion response {field} {actual} does not match requested identity {expected}."
                    ));
                }
            }
            let remote_task_id = required_string_field(remote, "task_id")?;
            let remote_change_ref = required_string_field(remote, "change_ref")?;
            let (remote_owner, remote_child) = remote_change_ref.rsplit_once('/').ok_or_else(|| {
                format!(
                    "History promotion response Change {remote_change_ref} has no Remote Task owner."
                )
            })?;
            if remote_owner != remote_task_id || remote_child.is_empty() {
                return Err(format!(
                    "History promotion response Change {remote_change_ref} is not owned by Remote Task {remote_task_id}."
                ));
            }
            let (expected_remote_task_id, expected_remote_change_ref) =
                workflow_expected_history_publication_ids(local)?;
            if expected_remote_task_id
                .as_deref()
                .is_some_and(|expected| expected != remote_task_id)
                || expected_remote_change_ref
                    .as_deref()
                    .is_some_and(|expected| expected != remote_change_ref)
            {
                return Err(format!(
                    "History promotion response attempts to replace the immutable publication mapping for {local_change_ref}."
                ));
            }
            if !remote_task_ids.insert(remote_task_id.clone()) {
                return Err(format!(
                    "History promotion response repeats Remote Task {remote_task_id}."
                ));
            }
            if !remote_change_refs.insert(remote_change_ref.clone()) {
                return Err(format!(
                    "History promotion response repeats Remote Change {remote_change_ref}."
                ));
            }
            Ok(ValidatedHistoryPublicationMapping {
                local_task_id,
                local_change_id,
                local_change_ref,
                remote_task_id,
                remote_change_ref,
            })
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
    let (remote_row, repo_name) = remote_context(&root_repo, remote_name, None)?;
    let publication_remote_name = contextual_publication_remote_name(remote_row.name.as_str())?;
    let mut candidate = {
        let _range = perfetto_range!("ait.workflow_ready.history_promotion.collect");
        workflow_final_snapshot_promotion_candidate(&root_repo, change_id, remote_name)?
            .ok_or_else(|| {
                format!("Local change {change_id} is not a completed history-promotion candidate.")
            })?
    };
    let mut remote_base_initialization = None;
    if candidate
        .get("remote_head_initialization_required")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let selected_base_snapshot_id = required_string_field(&candidate, "base_snapshot_id")?;
        let selected_revision_snapshot_id =
            required_string_field(&candidate, "revision_snapshot_id")?;
        let selected_target_line = candidate
            .get("state")
            .and_then(|state| state.get("change"))
            .and_then(|change| string_field(change, "base_line"))
            .unwrap_or_else(|| root_repo.default_line_name());
        let initialization = {
            let _range =
                perfetto_range!("ait.workflow_ready.history_promotion.remote_base_initialize");
            let mut task_remote = http_task_remote(&root_repo, &remote_row)?;
            workflow_initialize_null_remote_base_with_task_remote(
                &root_repo,
                &remote_row,
                &mut task_remote,
                &repo_name,
                &selected_target_line,
                &selected_base_snapshot_id,
            )?
        };
        let mut revalidated = workflow_final_snapshot_promotion_candidate(
            &root_repo,
            change_id,
            remote_name,
        )?
        .ok_or_else(|| {
            format!(
                "Completed local change {change_id} stopped being a history-promotion candidate after Remote base initialization."
            )
        })?;
        let revalidated_base_snapshot_id = required_string_field(&revalidated, "base_snapshot_id")?;
        let revalidated_revision_snapshot_id =
            required_string_field(&revalidated, "revision_snapshot_id")?;
        if revalidated
            .get("remote_head_initialization_required")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
            || revalidated_base_snapshot_id != selected_base_snapshot_id
            || revalidated_revision_snapshot_id != selected_revision_snapshot_id
        {
            return Err(format!(
                "Remote `{}` target Line `{selected_target_line}` changed while completed-local promotion initialized base `{selected_base_snapshot_id}`. Revalidated authority is `{revalidated_base_snapshot_id}` -> `{revalidated_revision_snapshot_id}`; refusing to prepare history from stale authority.",
                remote_row.name,
            ));
        }
        revalidated
            .as_object_mut()
            .ok_or_else(|| "Revalidated history promotion candidate is malformed.".to_string())?
            .insert(
                "remote_base_initialization".to_string(),
                initialization.clone(),
            );
        candidate = revalidated;
        remote_base_initialization = Some(initialization);
    }
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
    let mut closeout_remote = http_closeout_remote(&root_repo, &remote_row)?;
    let effective_author_mode = root_repo.effective_author_mode(author_mode);
    let display_summary = summary.unwrap_or("solo-local workflow history promotion");
    let (prepared, publication_mappings) = if request_entries.len()
        <= HISTORY_PROMOTION_STAGE_ENTRY_COUNT
    {
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
            "author_mode": effective_author_mode,
            "summary": display_summary,
            "entries": request_entries,
        });
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
                publication_remote_name,
                &candidate_entries,
                &response_entries,
            )?
        };
        (prepared, publication_mappings)
    } else {
        let promotion_id = workflow_staged_history_promotion_id(
            &root_repo,
            &base_line,
            &base_snapshot_id,
            &revision_snapshot_id,
            &request_entries,
        )?;
        let total_entry_count = u64::try_from(request_entries.len())
            .map_err(|_| "History promotion entry count exceeds u64.".to_string())?;
        let mut previous_stage_patchset_id = None;
        let mut final_prepared = None;
        let mut publication_mappings = Vec::with_capacity(request_entries.len());
        let mut remote_task_ids = BTreeSet::new();
        let mut remote_change_refs = BTreeSet::new();
        let mut receipt_patchset_ids = BTreeSet::new();

        for (stage_ordinal, stage_entries) in request_entries
            .chunks(HISTORY_PROMOTION_STAGE_ENTRY_COUNT)
            .enumerate()
        {
            let stage_start = stage_ordinal
                .checked_mul(HISTORY_PROMOTION_STAGE_ENTRY_COUNT)
                .ok_or_else(|| "History promotion stage offset overflow.".to_string())?;
            let stage_end = stage_start
                .checked_add(stage_entries.len())
                .ok_or_else(|| "History promotion stage boundary overflow.".to_string())?;
            let final_stage = stage_end == request_entries.len();
            let stage_ordinal = u64::try_from(stage_ordinal)
                .map_err(|_| "History promotion stage ordinal exceeds u64.".to_string())?;
            let request = workflow_staged_history_prepare_request(
                &promotion_id,
                &base_line,
                &base_snapshot_id,
                &revision_snapshot_id,
                stage_ordinal,
                total_entry_count,
                previous_stage_patchset_id.as_deref(),
                &effective_author_mode,
                display_summary,
                stage_entries,
            )?;
            let stage_prepared = {
                let _range = perfetto_range!("ait.workflow_ready.history_promotion.http");
                TaskWorkflowHistoryPromotionPreparer::prepare_history_promotion(
                    &mut closeout_remote,
                    &repo_name,
                    &request,
                )
                .map_err(|error| error.to_string())?
            };
            let response_entries = stage_prepared
                .get("entries")
                .and_then(JsonValue::as_array)
                .cloned()
                .ok_or_else(|| "Staged history promotion response has no mappings.".to_string())?;
            for mapping in &response_entries {
                for (field, seen) in [
                    ("task_id", &mut remote_task_ids),
                    ("change_ref", &mut remote_change_refs),
                    ("receipt_patchset_id", &mut receipt_patchset_ids),
                ] {
                    let identity = required_string_field(mapping, field)?;
                    if !seen.insert(identity.clone()) {
                        return Err(format!(
                                "Staged history promotion repeats Remote {field} `{identity}` across stages."
                            ));
                    }
                }
            }
            let marked = {
                let _range = perfetto_range!("ait.workflow_ready.history_promotion.local_mapping");
                workflow_mark_history_published(
                    &root_repo,
                    publication_remote_name,
                    &candidate_entries[stage_start..stage_end],
                    &response_entries,
                )?
            };
            publication_mappings.extend(marked);
            let stage_patchset_id = stage_prepared
                .get("stage")
                .and_then(|stage| stage.get("patchset_id"))
                .and_then(JsonValue::as_str)
                .ok_or_else(|| {
                    "Staged history promotion response is missing stage Patchset identity."
                        .to_string()
                })?
                .to_string();
            previous_stage_patchset_id = Some(stage_patchset_id);
            if final_stage {
                final_prepared = Some(stage_prepared);
            }
        }
        (
            final_prepared.ok_or_else(|| {
                "Staged history promotion completed without a final aggregate result.".to_string()
            })?,
            publication_mappings,
        )
    };
    let aggregate = prepared
        .get("aggregate")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "History promotion response is missing its aggregate result.".to_string())?;
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
        .ok_or_else(|| "History promotion aggregate is missing Patchset data.".to_string())?;
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
        "remote_base_initialization": remote_base_initialization,
    }))
}
