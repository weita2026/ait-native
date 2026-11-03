//! Fresh, unsealed preparation only. Recovery reads the pinned tested chain.
use super::*;
use ait_core::snapshot_rebase::{
    plan_snapshot_history_rebase, SnapshotHistoryBoundary, SnapshotReplayParent,
};

type SourceOwners = BTreeMap<String, Vec<(String, SnapshotWorkflowBinding)>>;

fn stale() -> String {
    "BASE_STALE: No exact accepted publication prefix permits fresh replay. Reconcile Local history explicitly; sealed candidates retain their original history.".into()
}

/// Original imported receipts and fixed Remote owners prove the source maps.
/// The final prefix Task's successful native finish proves acceptance, rather
/// than a prepared/failed candidate or an equal-content Snapshot.
fn accepted_prefix(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    target: &str,
    base: &str,
    prefix: &[JsonValue],
    ancestry: Option<&PackageAncestry>,
) -> Result<(), String> {
    let repo_name = remote.repo_name.clone().unwrap_or_else(|| repo.repo_name());
    let repository = u64::from(repo.require_repository_index()?.get());
    let namespace = ait_core::workflow_primitives::workflow_origin_namespace_prefix(
        "R",
        Some(&repo.id_namespace_prefix()),
    )?;
    let mut client = package_client(repo, remote, &repo_name)?;
    let tasks = repo.task_store()?;
    for entry in prefix {
        let mapped = required_string_field(&entry["task"], "published_task_id")?;
        if !mapped.starts_with(&format!("{namespace}T-"))
            || entry["already_published"].as_bool() != Some(true)
            || entry["publication_recovery_required"].as_bool() == Some(true)
        {
            return Err("Published prefix has incomplete or foreign Local mappings".into());
        }
        let mut mapped_changes = BTreeMap::new();
        for change in tasks
            .changes_for_task(&required_string_field(entry, "local_task_id")?)
            .map_err(|error| error.to_string())?
        {
            let reference = string_field(&change, "published_change_id")
                .ok_or("Published prefix Change mapping is incomplete")?;
            mapped_changes.insert(required_string_field(&change, "change_id")?, reference);
        }
        if authority::published_receipt(&mut client, &repo_name, repository as u32, entry, &mapped)?
            .is_none()
        {
            return Err("Published prefix lacks its exact original Remote receipt".into());
        }
        // Ordinary publication may bring missing original checkpoints into an
        // already mapped Task. Excluding that Task is stricter: every original
        // fixed source at its receipt cutoff must already have its exact owner.
        let sources = tasks
            .completed_task_source_snapshots(&required_string_field(entry, "local_task_id")?)
            .map_err(|error| error.to_string())?;
        if sources.is_empty() {
            return Err("Published prefix has no original source inventory".into());
        }
        for (id, owner) in sources {
            let response = client
                .get_remote_snapshot(&repo_name, &id, false, None)
                .map_err(|error| {
                    format!("Published prefix Snapshot proof is unavailable: {error}")
                })?;
            let row = ait_core::workflow_package::package_response_data(&response)?;
            let expected_change = if owner.change_id.is_empty() {
                None
            } else {
                Some(
                    mapped_changes
                        .get(&owner.change_id)
                        .ok_or("Published prefix Change mapping is incomplete")?
                        .as_str(),
                )
            };
            let actual = &row["workflow_owner"];
            if row["repository_index"].as_u64() != Some(repository)
                || row["snapshot_id"].as_str() != Some(id.as_str())
                || actual.as_object().is_none_or(|owner| owner.len() != 2)
                || actual["task_id"].as_str() != Some(mapped.as_str())
                || actual.get("change_ref").is_none()
                || actual["change_ref"].as_str() != expected_change
                || (expected_change.is_none() && !actual["change_ref"].is_null())
            {
                return Err(
                    "Published prefix Snapshot mapping is incomplete or contradictory".into(),
                );
            }
        }
    }
    let last = prefix.last().ok_or_else(stale)?;
    let mapped = required_string_field(&last["task"], "published_task_id")?;
    let response = client
        .get_task(&mapped, Some(&repo_name))
        .map_err(|error| error.to_string())?;
    let task = ait_core::workflow_package::package_response_data(&response)?;
    let receipt = &task["successful_land"];
    if task["repository_index"].as_u64() != Some(repository)
        || task["task_id"].as_str() != Some(mapped.as_str())
        || task["status"].as_str() != Some("completed")
        || receipt["status"].as_str() != Some("succeeded")
    {
        return Err("Published prefix has no exact accepted Remote finish".into());
    }
    let submission = required_string_field(receipt, "submission_id")?;
    let patchset = required_string_field(receipt, "patchset_id")?;
    ait_core::workflow_expected_identity::validate_patchset_reference_owner(
        &patchset, &mapped, None,
    )?;
    let ordinal = submission
        .strip_prefix(&format!("{mapped}/L-"))
        .and_then(|value| value.parse::<u8>().ok())
        .filter(|ordinal| (1..=64).contains(ordinal))
        .ok_or("Accepted prefix finish has a foreign submission owner")?;
    if submission != format!("{mapped}/L-{ordinal:02}") {
        return Err("Accepted prefix finish has an invalid submission ordinal".into());
    }
    let land = client
        .get_land(&submission, Some(&repo_name))
        .map_err(|error| error.to_string())?;
    let land = ait_core::workflow_package::package_response_data(&land)?;
    let candidate = client
        .get_patchset(&patchset, Some(&repo_name), None)
        .map_err(|error| error.to_string())?;
    let candidate = ait_core::workflow_package::package_response_data(&candidate)?;
    for row in [receipt, land] {
        if row["task_id"].as_str() != Some(mapped.as_str())
            || row["status"].as_str() != Some("succeeded")
            || row["target_line"].as_str() != Some(target)
            || row["submission_id"].as_str() != Some(submission.as_str())
            || row["patchset_id"].as_str() != Some(patchset.as_str())
            || row
                .get("repository_index")
                .is_some_and(|id| id.as_u64() != Some(repository))
            || [
                "base_snapshot_id",
                "revision_snapshot_id",
                "landed_snapshot_id",
                "pre_land_target_snapshot_id",
                "change_id",
                "change_ref",
                "mode",
            ]
            .iter()
            .any(|field| row.get(field) != receipt.get(field))
        {
            return Err("Accepted prefix native finish projections conflict".into());
        }
    }
    if candidate["task_id"].as_str() != Some(mapped.as_str())
        || candidate["patchset_id"].as_str() != Some(patchset.as_str())
        || candidate["base_snapshot_id"] != receipt["base_snapshot_id"]
        || candidate["revision_snapshot_id"] != receipt["revision_snapshot_id"]
        || candidate
            .get("repository_index")
            .is_some_and(|id| id.as_u64() != Some(repository))
    {
        return Err("Accepted prefix Patchset differs from its native finish".into());
    }
    let accepted = required_string_field(receipt, "landed_snapshot_id")?;
    let contained = match ancestry {
        Some(ancestry) => ancestry.remote_ids.contains(&accepted),
        None => snapshot_distance_if_ancestor(repo, Some(&accepted), Some(base))?.is_some(),
    };
    if !contained {
        return Err("Accepted prefix finish is outside selected Remote target ancestry".into());
    }
    Ok(())
}

/// Validate every boundary, conflict, projection bound and retry owner before
/// calling the restricted writer. No Local Line or original receipt is changed.
#[allow(clippy::too_many_arguments)]
pub(super) fn prepare_fresh_suffix(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    target: &str,
    base: &str,
    common: &str,
    mut entries: Vec<JsonValue>,
    originals: &mut SourceOwners,
    ancestry: Option<&PackageAncestry>,
) -> Result<(Vec<JsonValue>, String), String> {
    let split = entries
        .iter()
        .position(|entry| string_field(&entry["task"], "published_task_id").is_none())
        .ok_or_else(stale)?;
    if entries[split..].iter().any(|entry| {
        string_field(&entry["task"], "published_task_id").is_some()
            || entry["already_published"].as_bool() == Some(true)
    }) {
        return Err("Published source mappings are not one complete history prefix".into());
    }
    let tasks = repo.task_store()?;
    let mut prefix = entries[..split].to_vec();
    if prefix.is_empty() {
        // When R descends from the original prefix, the merge base itself may
        // already be its receipt endpoint and thus absent from collected entries.
        let owners = tasks
            .snapshot_ownership_rows(&[common.to_string()])
            .map_err(|error| error.to_string())?;
        let owner = owners
            .first()
            .filter(|_| owners.len() == 1)
            .filter(|owner| owner["ownership_source"].as_str() == Some("durable_snapshot_binding"))
            .ok_or_else(stale)?;
        let id = required_string_field(owner, "task_id")?;
        let original = workflow_completed_local_entry_if_present(repo, &id, Some(&remote.name))?
            .ok_or_else(stale)?;
        let original_receipt = &original["state"]["local_finish"];
        if original_receipt["landed_snapshot_id"].as_str() != Some(common)
            || original_receipt["target_line"].as_str() != Some(target)
            || string_field(&original["state"]["task"], "published_task_id").is_none()
        {
            return Err(stale());
        }
        let start = required_string_field(original_receipt, "pre_land_target_snapshot_id")?;
        let (proof, _) =
            local_completion::workflow_local_history_entries(repo, &id, target, &start, common)?;
        let sources = BTreeMap::from([(
            id.clone(),
            tasks
                .completed_task_source_snapshots(&id)
                .map_err(|error| error.to_string())?,
        )]);
        verify_published_source_authority(repo, remote, &proof, &sources)?;
        prefix = proof;
    }
    accepted_prefix(repo, remote, target, base, &prefix, ancestry)?;
    entries.drain(..split);
    preflight_source_entries(repo, &entries)?;
    let snapshots = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    let mut boundaries = Vec::new();
    let mut additions = BTreeMap::<String, usize>::new();
    for entry in &entries {
        let task = required_string_field(entry, "local_task_id")?;
        if tasks
            .changes_for_task(&task)
            .map_err(|error| error.to_string())?
            .iter()
            .any(|change| string_field(change, "published_change_id").is_some())
        {
            return Err("Unpublished suffix has a contradictory Remote Change mapping".into());
        }
        let mut previous = required_string_field(entry, "pre_land_target_snapshot_id")?;
        for snapshot in entry["snapshots"]
            .as_array()
            .ok_or("Missing replay source history")?
        {
            let id = required_string_field(snapshot, "snapshot_id")?;
            let owner = originals[&task]
                .iter()
                .find(|(owned, _)| owned == &id)
                .map(|(_, owner)| owner.clone())
                .ok_or("Replay boundary lacks its exact original fixed owner")?;
            let row = snapshot_by_id_with_snapshot_store(&snapshots, &id)?
                .ok_or("Replay source Snapshot is unavailable")?;
            if row.parent_snapshot_ids != [previous.clone()] || row.line_name != owner.line_name {
                return Err("Replay source history has an unowned gap or merge".into());
            }
            boundaries.push(SnapshotHistoryBoundary {
                base_snapshot_id: previous.clone(),
                snapshot_id: id.clone(),
                owner,
                base_files: source_files(repo, &previous)?,
                files: source_files(repo, &id)?,
                metadata: row,
            });
            previous = id;
            *additions.entry(task.clone()).or_default() += 1;
        }
        if additions.get(&task).copied().unwrap_or_default() + originals[&task].len() > 64 {
            return Err(format!(
                "Source Task {task} exceeds 64 original plus replay Snapshots"
            ));
        }
    }
    let target_files = source_files(repo, base)?;
    let steps = plan_snapshot_history_rebase(base, &target_files, &boundaries)
        .map_err(|error| format!("Fresh history replay held: {error:?}"))?;
    // Identify exact existing retry results without authoring raw trees merely
    // to discover their IDs. A missing predecessor makes all later steps new.
    let existing = snapshots.list_line_snapshots()?;
    let mut effective = Some(base.to_string());
    let mut needed = BTreeMap::<String, usize>::new();
    for step in &steps {
        for path in step.files.keys() {
            if path
                .split('/')
                .scan(String::new(), |prefix, part| {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(part);
                    Some(prefix.clone())
                })
                .any(|prefix| prefix != *path && step.files.contains_key(&prefix))
            {
                return Err(format!(
                    "History rebase conflict between file and directory at {path}"
                ));
            }
        }
        if let SnapshotReplayParent::Existing(parent) = &step.parent {
            effective = Some(parent.clone());
        }
        let mut reused = None;
        if let Some(parent) = &effective {
            for row in existing.iter().filter(|row| {
                row.parent_snapshot_ids == [parent.clone()]
                    && row.line_name == step.source.metadata.line_name
                    && row.message == step.source.metadata.message
            }) {
                if source_files(repo, &row.snapshot_id)? != step.files {
                    continue;
                }
                let owners = tasks
                    .snapshot_ownership_rows(std::slice::from_ref(&row.snapshot_id))
                    .map_err(|error| error.to_string())?;
                if owners.len() != 1
                    || owners[0]["ownership_source"].as_str() != Some("durable_snapshot_binding")
                    || owners[0]["task_id"].as_str() != Some(step.source.owner.task_id.as_str())
                    || owners[0]["change_id"].as_str()
                        != (!step.source.owner.change_id.is_empty())
                            .then_some(step.source.owner.change_id.as_str())
                {
                    return Err("Existing replay result has a foreign fixed owner".into());
                }
                if reused.replace(row.snapshot_id.clone()).is_some() {
                    return Err("Replay retry identity is ambiguous".into());
                }
            }
        }
        effective = reused;
        if effective.is_none() {
            *needed.entry(step.source.owner.task_id.clone()).or_default() += 1;
        }
    }
    tasks
        .validate_replay_snapshot_capacity(&needed)
        .map_err(|error| error.to_string())?;
    let repo_name = remote.repo_name.clone().unwrap_or_else(|| repo.repo_name());
    let mut client = package_client(repo, remote, &repo_name)?;
    let line = client
        .get_line(&repo_name, target)
        .map_err(|error| error.to_string())?;
    if workflow_remote_line_head(&line, target)?.as_deref() != Some(base) {
        return Err(
            "Remote target advanced during fresh replay preflight; keep this preparation held"
                .into(),
        );
    }
    let content = repo.local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()?;
    let results = content.replay_completed_snapshot_history(
        &repo.repo_name(),
        &repo.id_namespace_prefix(),
        &steps,
    )?;
    let final_id = results
        .last()
        .ok_or("Replay produced no final Snapshot")?
        .snapshot_id
        .clone();
    for (step, result) in steps.iter().zip(results) {
        let owners = originals
            .get_mut(&step.source.owner.task_id)
            .expect("source inventoried");
        if !owners.iter().any(|(id, _)| id == &result.snapshot_id) {
            owners.push((result.snapshot_id, step.source.owner.clone()));
        }
    }
    Ok((entries, final_id))
}
