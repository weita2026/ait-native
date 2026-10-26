//! A mapped Task needs positive Remote ownership evidence. Its incoming Local
//! checkpoints are not a claim that every checkpoint was previously published.
use super::*;
use crate::json_support::parse_value_error_string;
use ait_core::plan_http_client::PlanHttpClientError;

const RECEIPT_PREFIX: &str = "ait-local-land-receipt/v1 ";

fn same_repository(row: &JsonValue, repository: u32) -> Result<(), String> {
    if row
        .get("repository_index")
        .is_some_and(|value| value.as_u64() != Some(u64::from(repository)))
    {
        return Err("Selected origin returned a different Repository authority".into());
    }
    Ok(())
}

fn canonical_change(reference: &str, task: &str) -> bool {
    reference.rsplit_once('/').is_some_and(|(owner, child)| {
        owner == task
            && child
                .strip_prefix("C-")
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| (1..=64).contains(&n) && child == format!("C-{n:02}"))
    })
}

/// Only the existing native missing-Snapshot response denotes absent incoming
/// content. A missing route/Repository, permission error or transport failure
/// never supplies authority and cannot be silently treated as a new Snapshot.
pub(super) fn missing_snapshot(
    error: &PlanHttpClientError,
    repo_name: &str,
    repository: u32,
    id: &str,
) -> bool {
    matches!(error, PlanHttpClientError::RemoteResponse { method, status: 404, detail, .. }
        if method == "GET" && (detail == &format!("Unknown snapshot {id} for repository {repo_name}")
            || detail == &format!("Unknown snapshot {id} for repository {repository}")))
}

/// Returns the unique matching receipt's legacy flag. Without a provenance
/// receipt, the existing exact immutable Snapshot-owner witness is still valid.
fn published_receipt(
    client: &mut PlanHttpClientManager,
    repo_name: &str,
    repository: u32,
    entry: &JsonValue,
    mapped: &str,
) -> Result<Option<bool>, String> {
    let task = client
        .get_task(mapped, Some(repo_name))
        .map_err(|e| e.to_string())?;
    let task = ait_core::workflow_package::package_response_data(&task)?;
    same_repository(task, repository)?;
    if task["task_id"].as_str() != Some(mapped) {
        return Err("Selected origin conflicts with the original publication authority".into());
    }
    let local_task = required_string_field(entry, "local_task_id")?;
    let original = &entry["local_finish"];
    let target = required_string_field(original, "target_line")?;
    let base = required_string_field(original, "pre_land_target_snapshot_id")?;
    let final_id = required_string_field(original, "landed_snapshot_id")?;
    let time = snapshot_seconds(
        original["updated_at"]
            .as_str()
            .ok_or("Original receipt time is missing")?,
    )?;
    let mut found = None;
    let mut seen = BTreeSet::new();
    let listed = client
        .list_patchsets(mapped, Some(repo_name))
        .map_err(|e| e.to_string())?;
    if listed.len() > 64 {
        return Err("Published source Patchset inventory exceeds its Task bound".into());
    }
    for listed in listed {
        same_repository(&listed, repository)?;
        if listed
            .get("task_id")
            .is_some_and(|id| id.as_str() != Some(mapped))
        {
            return Err("Published source Patchset list has a foreign owner".into());
        }
        let id = required_string_field(&listed, "patchset_id")?;
        ait_core::workflow_expected_identity::validate_patchset_reference_owner(&id, mapped, None)?;
        if !seen.insert(id.clone()) {
            return Err("Published source receipt inventory is ambiguous".into());
        }
        let value = client
            .get_patchset(&id, Some(repo_name), None)
            .map_err(|e| e.to_string())?;
        let row = ait_core::workflow_package::package_response_data(&value)?;
        same_repository(row, repository)?;
        if row["patchset_id"].as_str() != Some(id.as_str())
            || row["task_id"].as_str() != Some(mapped)
        {
            return Err("Published source Patchset has a foreign owner".into());
        }
        let summary = row["summary"].as_str().unwrap_or_default();
        let Some(raw) = summary.strip_prefix(RECEIPT_PREFIX) else {
            if row["source_kind"].as_str() == Some("imported_local_land_receipt") {
                return Err("Published source receipt is unparseable".into());
            }
            continue;
        };
        let receipt = parse_value_error_string(raw)
            .map_err(|e| format!("Published source receipt is unparseable: {e}"))?;
        let legacy = match receipt["contract"].as_str() {
            Some("ait-local-land-receipt/v1") => true,
            Some("ait-local-land-receipt/v2") => false,
            _ => return Err("Unsupported published source receipt".into()),
        };
        if row["source_kind"].as_str() != Some("imported_local_land_receipt")
            || !row.get("change_ref").is_some_and(JsonValue::is_null)
            || receipt["source_kind"].as_str() != Some("imported_local_land_receipt")
            || receipt["local_task_id"].as_str() != Some(local_task.as_str())
            || receipt["task_id"].as_str() != Some(mapped)
            || receipt["target_line"].as_str() != Some(target.as_str())
            || receipt["pre_land_target_snapshot_id"].as_str() != Some(base.as_str())
            || receipt["landed_snapshot_id"].as_str() != Some(final_id.as_str())
            || receipt["landed_at_s"].as_u64() != Some(time)
            || row["base_snapshot_id"].as_str() != Some(base.as_str())
            || row["revision_snapshot_id"].as_str() != Some(final_id.as_str())
            || snapshot_seconds(
                row["created_at"]
                    .as_str()
                    .ok_or("Published receipt time is missing")?,
            )? != time
            || row["ci_run_seq"].as_u64() != Some(0)
        {
            return Err(
                "Published source receipt conflicts with the exact original Local receipt".into(),
            );
        }
        if legacy {
            let local = receipt["local_change_ref"]
                .as_str()
                .ok_or("Legacy receipt lacks its source-era Change quotation")?;
            let remote = receipt["change_ref"]
                .as_str()
                .ok_or("Legacy receipt lacks its source-era Change quotation")?;
            if !canonical_change(local, &local_task)
                || !canonical_change(remote, mapped)
                || receipt["local_change_id"].as_str() != local.rsplit_once('/').map(|(_, id)| id)
            {
                return Err("Legacy receipt has inconsistent source-era Change quotations".into());
            }
        } else if ["local_change_id", "local_change_ref", "change_ref"]
            .iter()
            .any(|field| receipt.get(field).is_some_and(|value| !value.is_null()))
        {
            return Err("Published v2 source receipt has unexpected Change associations".into());
        }
        if found.replace(legacy).is_some() {
            return Err("Published source Task has ambiguous original receipts".into());
        }
    }
    Ok(found)
}

pub(super) fn verify_published_source_authority(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    entries: &[JsonValue],
    originals: &BTreeMap<String, Vec<(String, SnapshotWorkflowBinding)>>,
) -> Result<(), String> {
    require_package_origin(remote)?;
    let repo_name = remote.repo_name.clone().unwrap_or_else(|| repo.repo_name());
    let repository = repo.require_repository_index()?.get();
    let mut client = package_client(repo, remote, &repo_name)?;
    let snapshots = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    for entry in entries {
        let Some(mapped) = string_field(&entry["task"], "published_task_id") else {
            continue;
        };
        let task = required_string_field(entry, "local_task_id")?;
        let anchors = originals
            .get(&task)
            .filter(|rows| !rows.is_empty())
            .ok_or("Published Task has no original Snapshot authority; keep publication held")?;
        let receipt = published_receipt(&mut client, &repo_name, repository, entry, &mapped)?;
        let endpoint = required_string_field(&entry["local_finish"], "landed_snapshot_id")?;
        let changes = repo
            .task_store()?
            .changes_for_task(&task)
            .map_err(|e| e.to_string())?;
        let mut resolved = BTreeMap::new();
        for change in &changes {
            if let Some(reference) = string_field(change, "published_change_id") {
                if !canonical_change(&reference, &mapped) {
                    return Err("Published Change has a foreign Task owner".into());
                }
                let value = client
                    .get_change(&reference, Some(&repo_name))
                    .map_err(|e| e.to_string())?;
                let row = ait_core::workflow_package::package_response_data(&value)?;
                same_repository(row, repository)?;
                if row["task_id"].as_str() != Some(mapped.as_str())
                    || row["change_ref"].as_str() != Some(reference.as_str())
                    || row["fork_snapshot_id"] != change["fork_snapshot_id"]
                    || row["title"] != change["title"]
                {
                    return Err("Selected origin has conflicting original Change mappings".into());
                }
                resolved.insert(required_string_field(change, "change_id")?, reference);
            }
        }
        let mut any_witness = false;
        let mut endpoint_witness = false;
        for (id, owner) in anchors {
            let value = match client.get_remote_snapshot(&repo_name, id, false, None) {
                Ok(value) => value,
                Err(error) if missing_snapshot(&error, &repo_name, repository, id) => continue,
                Err(error) => {
                    return Err(format!(
                        "Established origin source authority is unavailable: {error}"
                    ))
                }
            };
            let row = ait_core::workflow_package::package_response_data(&value)?;
            if row["repository_index"].as_u64() != Some(u64::from(repository))
                || row["snapshot_id"].as_str() != Some(id.as_str())
            {
                return Err("Selected origin conflicts with the original publication authority; do not retarget existing mappings".into());
            }
            let remote_owner = row
                .get("workflow_owner")
                .ok_or("Remote Snapshot lacks its typed owner projection")?;
            if !remote_owner.is_null()
                && (remote_owner["task_id"].as_str() != Some(mapped.as_str())
                    || remote_owner
                        .as_object()
                        .is_none_or(|owner| owner.len() != 2))
            {
                return Err("Selected origin conflicts with the original publication authority; do not retarget existing mappings".into());
            }
            let original = snapshot_by_id_with_snapshot_store(&snapshots, id)?
                .ok_or("Original publication Snapshot is unavailable")?;
            let parents = row["parent_snapshot_ids"]
                .as_array()
                .ok_or("Origin Snapshot lacks immutable ordered parents")?
                .iter()
                .map(|id| {
                    id.as_str()
                        .map(str::to_string)
                        .ok_or("Origin Snapshot has an invalid parent")
                })
                .collect::<Result<Vec<_>, _>>()?;
            if row["manifest_hash"].as_str() != Some(original.manifest_hash.as_str())
                || row["line_name"].as_str() != Some(original.line_name.as_str())
                || parents != original.parent_snapshot_ids
                || snapshot_seconds(
                    row["created_at"]
                        .as_str()
                        .ok_or("Origin Snapshot lacks its immutable creation time")?,
                )? != snapshot_seconds(&original.created_at)?
            {
                return Err(
                    "Selected origin conflicts with immutable original Snapshot data".into(),
                );
            }
            // Existing content may still await its first workflow binding.
            // Server preflight validates that incoming claim. It is never a
            // positive witness for this Task's existing publication mapping.
            if remote_owner.is_null() {
                continue;
            }
            let remote_change = remote_owner
                .get("change_ref")
                .ok_or("Original Remote Snapshot lacks exact Change ownership")?;
            if owner.change_id.is_empty() {
                if !remote_change.is_null() {
                    return Err("Published Task-only Snapshot has another Change owner".into());
                }
            } else {
                let reference = remote_change
                    .as_str()
                    .filter(|id| canonical_change(id, &mapped))
                    .ok_or("Published Snapshot lacks its exact Remote Change owner")?;
                let change = changes
                    .iter()
                    .find(|change| change["change_id"].as_str() == Some(owner.change_id.as_str()))
                    .ok_or("Original Change is absent")?;
                if string_field(change, "published_change_id").is_some_and(|id| id != reference)
                    || resolved
                        .insert(owner.change_id.clone(), reference.to_string())
                        .is_some_and(|old| old != reference)
                {
                    return Err("Selected origin has conflicting original Change mappings".into());
                }
            }
            any_witness = true;
            if id == &endpoint {
                if receipt == Some(true) && !remote_change.is_null() {
                    return Err("Legacy receipt endpoint has a noncanonical Change owner".into());
                }
                endpoint_witness = true;
            }
        }
        if !any_witness || (receipt.is_some() && !endpoint_witness) {
            return Err(
                "No eligible exact Remote source ownership witness; keep publication held".into(),
            );
        }
        if resolved.values().collect::<BTreeSet<_>>().len() != resolved.len() {
            return Err("Distinct source Changes collide in the selected origin".into());
        }
    }
    Ok(())
}
