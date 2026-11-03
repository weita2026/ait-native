use super::*;

/// Recover only the original completed source inventory and the exact pinned
/// replay chain. This read never captures, rebases, uploads or selects a Job.
pub(super) fn recover_source_history(
    repo: &RepoRuntime,
    reference: &str,
    remote_name: &str,
    identity: &PackageWorkflowExpectedIdentity,
) -> Result<(String, Vec<PackageMappingExpectation>), String> {
    identity.validate()?;
    if repo.require_repository_index()?.get() != identity.repository_index {
        return Err("Package identity belongs to another Repository".into());
    }
    let selected =
        workflow_completed_local_entry_if_present(repo, reference, Some(remote_name))?
            .ok_or("Exact completed Local source is unavailable; package outcome is unknown")?;
    let receipt = &selected["state"]["local_finish"];
    let target = required_string_field(receipt, "target_line")?;
    let original_final = required_string_field(receipt, "landed_snapshot_id")?;
    let selected_id = required_string_field(&selected["state"]["task"], "task_id")?;
    let tasks = repo.task_store()?;
    let owner = |id: &str| -> Result<(String, Option<String>), String> {
        let rows = tasks
            .snapshot_ownership_rows(&[id.to_string()])
            .map_err(|error| error.to_string())?;
        let row = rows
            .first()
            .filter(|_| rows.len() == 1)
            .ok_or("Exact package source Snapshot owner is unavailable")?;
        if row["ownership_source"].as_str() != Some("durable_snapshot_binding") {
            return Err("Package source Snapshot has no fixed owner".into());
        }
        let task = required_string_field(row, "task_id")?;
        let change = string_field(row, "change_id").map(|id| format!("{task}/{id}"));
        Ok((task, change))
    };
    let snapshots = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    // The sealed tested chain pins the source sequence independently of the
    // deliberately truncated local representation of its Remote base.
    let mut cursor = identity.revision_snapshot_id.clone();
    let mut seen = BTreeSet::new();
    let mut reversed_tasks = Vec::new();
    let mut tested = Vec::new();
    while cursor != identity.base_snapshot_id {
        if !seen.insert(cursor.clone()) || seen.len() > 65536 {
            return Err("Pinned package chain is cyclic or exceeds its bounded inventory".into());
        }
        let snapshot = snapshot_by_id_with_snapshot_store(&snapshots, &cursor)?
            .ok_or("Pinned source Snapshot is unavailable; outcome is unknown")?;
        let (task, change) = owner(&cursor)?;
        if reversed_tasks.is_empty() && task != selected_id {
            return Err("Pinned final Snapshot belongs to a different source Task".into());
        }
        if reversed_tasks.last() != Some(&task) {
            reversed_tasks.push(task.clone());
        }
        tested.push((cursor.clone(), task, change));
        let [parent] = snapshot.parent_snapshot_ids.as_slice() else {
            return Err("Pinned tested history is not one coherent replay chain".into());
        };
        cursor = parent.clone();
    }
    let first = reversed_tasks
        .last()
        .ok_or("Pinned package has no source Task")?;
    let first_entry = workflow_completed_local_entry_if_present(repo, first, Some(remote_name))?
        .ok_or("Original first source receipt is unavailable; outcome is unknown")?;
    if required_string_field(&first_entry["state"]["local_finish"], "target_line")? != target {
        return Err("Pinned source receipts target a different Line".into());
    }
    let recorded_pre = required_string_field(
        &first_entry["state"]["local_finish"],
        "pre_land_target_snapshot_id",
    )?;
    let first_final =
        required_string_field(&first_entry["state"]["local_finish"], "landed_snapshot_id")?;
    let common = if recorded_pre != first_final {
        recorded_pre
    } else if first_entry["state"]["change"].is_null() {
        local_completion::workflow_recover_task_receipt_boundary(repo, first, &first_final)?
    } else {
        local_completion::workflow_effective_pre_land_target_snapshot_id(
            repo,
            &first_entry["state"]["change"],
            &first_final,
            &recorded_pre,
        )?
        .0
    };
    let (entries, _) = local_completion::workflow_local_history_entries(
        repo,
        &selected_id,
        &target,
        &common,
        &original_final,
    )?;
    let mut inventory = entries
        .iter()
        .map(|entry| {
            Ok((
                required_string_field(entry, "local_task_id")?,
                BTreeMap::<String, Option<String>>::new(),
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    for entry in &entries {
        let task_id = required_string_field(entry, "local_task_id")?;
        let original = tasks
            .completed_task_source_snapshots(&task_id)
            .map_err(|error| error.to_string())?;
        if original.is_empty() {
            return Err(
                "Original fixed source Snapshot inventory is unavailable; outcome is unknown"
                    .into(),
            );
        }
        for (id, binding) in original {
            // Selection uses the original fixed-link prefix, never the broad
            // current Task inventory that may include other ready candidates.
            snapshot_by_id_with_snapshot_store(&snapshots, &id)?
                .ok_or("Original source Snapshot is unavailable; outcome is unknown")?;
            let change =
                (!binding.change_id.is_empty()).then(|| format!("{task_id}/{}", binding.change_id));
            inventory
                .get_mut(&task_id)
                .expect("source Task inventoried")
                .insert(id, change);
        }
    }
    for (id, task, change) in tested {
        inventory
            .get_mut(&task)
            .ok_or("Pinned Snapshot belongs to a different source Task")?
            .insert(id, change);
    }
    reversed_tasks.reverse();
    let expected_tasks = entries
        .iter()
        .map(|entry| required_string_field(entry, "local_task_id"))
        .collect::<Result<Vec<_>, _>>()?;
    if reversed_tasks != expected_tasks {
        return Err("Pinned tested chain does not cover the exact original Task sequence".into());
    }
    let mappings = entries
        .iter()
        .map(|entry| {
            let task = &entry["task"];
            let task_id = required_string_field(entry, "local_task_id")?;
            let snapshots = inventory
                .remove(&task_id)
                .expect("source inventoried")
                .into_iter()
                .map(
                    |(snapshot_id, local_change_ref)| PackageSnapshotOwnerExpectation {
                        snapshot_id,
                        local_change_ref,
                    },
                )
                .collect::<Vec<_>>();
            let changes = tasks
                .changes_for_task(&task_id)
                .map_err(|error| error.to_string())?
                .into_iter()
                .map(|change| {
                    let local_change_ref =
                        format!("{task_id}/{}", required_string_field(&change, "change_id")?);
                    let mapped = string_field(&change, "published_change_id");
                    if mapped.is_none()
                        && !snapshots.iter().any(|row| {
                            row.local_change_ref.as_deref() == Some(local_change_ref.as_str())
                        })
                    {
                        return Err(
                            "Source Change has no Snapshot or explicit mapping; outcome is unknown"
                                .into(),
                        );
                    }
                    Ok(PackageChangeMappingExpectation {
                        local_change_ref,
                        remote_change_ref: mapped,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            if snapshots.is_empty() {
                return Err("Exact source Snapshot inventory is empty".into());
            }
            Ok(PackageMappingExpectation {
                local_task_id: task_id,
                remote_task_id: string_field(task, "published_task_id"),
                changes,
                snapshots,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((target, mappings))
}
