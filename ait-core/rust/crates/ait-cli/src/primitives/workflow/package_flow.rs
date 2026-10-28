use super::local_completion::{
    workflow_completed_local_entry_if_present, workflow_history_prepare_entries,
    workflow_initialize_null_remote_base_with_task_remote, workflow_null_remote_base_from_entry,
    workflow_remote_line_head, workflow_sync_history_plan_artifacts,
};
use super::*;
use crate::primitives::remote_sync::{
    build_zstd_bulk_local_plan, ordered_object_pack_metadata, ordered_tree_pack_metadata,
};
use ait_core::local_snapshot::LocalSnapshotTreeReadStore;
use ait_core::plan_http_client::PlanHttpClientManager;
use ait_core::repository_pack_json::ZstdBulkCommitRequest;
use ait_core::snapshot_rebase::SnapshotFileMap;
use ait_core::workflow_binary_db::SnapshotWorkflowBinding;
use ait_core::workflow_expected_identity::PackageWorkflowExpectedIdentity;
use ait_core::workflow_package::{
    EncodedWorkflowPackage, PackageChangeMappingExpectation, PackageMappingExpectation,
    PackageSnapshotOwnerExpectation, WorkflowPackageManifest,
};

mod source;
use source::recover_source_history;
mod ancestry;
use ancestry::PackageAncestry;
mod authority;
use authority::verify_published_source_authority;
mod apply;
mod replay;
mod retry;
pub use apply::{
    workflow_package_abandon, workflow_package_finish, workflow_package_ready,
    workflow_package_task_review, PackageReadyOptions,
};
pub(crate) use retry::workflow_package_accepted_retry;

pub fn workflow_package_source_requested(
    repo: &RepoRuntime,
    reference: &str,
) -> Result<bool, String> {
    let namespace = ait_core::workflow_primitives::workflow_origin_namespace_prefix(
        "L",
        Some(&repo.id_namespace_prefix()),
    )?;
    Ok(reference
        .split('/')
        .next()
        .is_some_and(|task| task.starts_with(&format!("{namespace}T-"))))
}

#[cfg(test)]
mod tests;

struct PreparedSource {
    manifest: WorkflowPackageManifest,
    package: EncodedWorkflowPackage,
    mappings: Vec<PackageMappingExpectation>,
}

fn package_client(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    repo_name: &str,
) -> Result<PlanHttpClientManager, String> {
    require_package_origin(remote)?;
    let mut client =
        PlanHttpClientManager::new(http_config(repo, remote)).map_err(|error| error.to_string())?;
    client.set_actor_registration(
        repo_name,
        repo.remote_actor_identity(),
        Some(&repo.id_namespace_prefix()),
    );
    Ok(client)
}

fn require_package_origin(remote: &RemoteRow) -> Result<(), String> {
    if remote.name != "origin" {
        return Err("Package publication supports only the established contextual origin authority; aliases or multi-authority publication cannot be recorded locally".into());
    }
    Ok(())
}

fn preflight_source_entries(repo: &RepoRuntime, entries: &[JsonValue]) -> Result<(), String> {
    ait_core::workflow_package::validate_package_source_count(entries.len())?;
    if entries
        .iter()
        .all(|entry| string_field(&entry["task"], "published_task_id").is_some())
    {
        return Err("An already published whole batch requires exact original result readback, not a new publication Job".into());
    }
    let remote_prefix = ait_core::workflow_primitives::workflow_origin_namespace_prefix(
        "R",
        Some(&repo.id_namespace_prefix()),
    )?;
    let projection_prefix_len = format!("import/{remote_prefix}T-{}/base/", u32::MAX).len();
    for entry in entries {
        let task = &entry["task"];
        let task_id = required_string_field(entry, "local_task_id")?;
        let title = required_string_field(task, "title")?;
        let intent = required_string_field(task, "intent")?;
        if title.len().saturating_add(intent.len()).saturating_add(2) > usize::from(u16::MAX) {
            return Err(format!(
                "Source Task {task_id} payload exceeds u16 UTF-8 bytes"
            ));
        }
        let changes = repo
            .task_store()?
            .changes_for_task(&task_id)
            .map_err(|e| e.to_string())?;
        if changes.len() > 64 {
            return Err(format!("Source Task {task_id} exceeds 64 Changes"));
        }
        for change in changes {
            let title = required_string_field(&change, "title")?;
            let base = required_string_field(&change, "base_line")?;
            if title.len() > usize::from(u16::MAX)
                || projection_prefix_len.saturating_add(base.trim().len().saturating_mul(2))
                    > usize::from(u16::MAX)
            {
                return Err(format!("Source Change in {task_id} exceeds publication UTF-8 payload/projected Line bounds"));
            }
        }
    }
    Ok(())
}

fn source_files(repo: &RepoRuntime, id: &str) -> Result<SnapshotFileMap, String> {
    let store = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    Ok(store
        .snapshot_tree_file_rows(Some(id))?
        .into_iter()
        .map(|row| (row.path.clone(), row))
        .collect())
}

fn snapshot_seconds(value: &str) -> Result<u64, String> {
    value.parse().or_else(|_| {
        let time = DateTime::parse_from_rfc3339(value).map_err(|error| error.to_string())?;
        u64::try_from(time.timestamp()).map_err(|_| "Snapshot time precedes the epoch".into())
    })
}

fn prepare_source_history(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    reference: &str,
    author_mode: Option<&str>,
    summary: Option<&str>,
) -> Result<PreparedSource, String> {
    require_package_origin(remote)?;
    let entry = workflow_completed_local_entry_if_present(repo, reference, Some(&remote.name))?
        .ok_or(
            "Unfinished Local Task history cannot be packaged without a real successful receipt",
        )?;
    let receipt = &entry["state"]["local_finish"];
    let target_line = required_string_field(receipt, "target_line")?;
    let original_revision = required_string_field(receipt, "landed_snapshot_id")?;
    let local_head = local_line_head_snapshot_id(repo, &target_line)?
        .ok_or("Fresh package target has no Local head")?;
    if snapshot_distance_if_ancestor(repo, Some(&original_revision), Some(&local_head))?.is_none() {
        return Err(
            "Fresh package ready requires the completed Task in the current Local target history"
                .into(),
        );
    }
    let repo_name = remote.repo_name.clone().unwrap_or_else(|| repo.repo_name());
    let mut task_remote = http_task_remote(repo, remote)?;
    let line = task_remote
        .get_line(&repo_name, &target_line)
        .map_err(|error| error.to_string())?;
    let base = if let Some(base) = workflow_remote_line_head(&line, &target_line)? {
        base
    } else {
        let (base, _) = workflow_null_remote_base_from_entry(repo, &entry)?;
        workflow_initialize_null_remote_base_with_task_remote(
            repo,
            remote,
            &mut task_remote,
            &repo_name,
            &target_line,
            &base,
        )?;
        let confirmed = task_remote
            .get_line(&repo_name, &target_line)
            .map_err(|error| error.to_string())?;
        if workflow_remote_line_head(&confirmed, &target_line)?.as_deref() != Some(base.as_str()) {
            return Err("Remote default Line changed after null-head initialization".into());
        }
        base
    };
    if !remote_sync_snapshot_content_complete_for_repo(repo, &base)? {
        hydrate_remote_snapshot_boundary_for_repo(
            repo,
            &remote.name,
            repo.require_repository_index()?,
            &repo_name,
            &base,
        )?;
    }
    let ancestry = PackageAncestry::load(
        repo,
        &mut task_remote,
        &repo_name,
        &original_revision,
        &base,
    )?;
    prepare_source_history_at_base_with_ancestry(
        repo,
        remote,
        &entry,
        &base,
        author_mode,
        summary,
        Some(&ancestry),
    )
}

#[cfg(test)]
fn prepare_source_history_at_base(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    entry: &JsonValue,
    base: &str,
    author_mode: Option<&str>,
    summary: Option<&str>,
) -> Result<PreparedSource, String> {
    prepare_source_history_at_base_with_ancestry(
        repo,
        remote,
        entry,
        base,
        author_mode,
        summary,
        None,
    )
}

fn prepare_source_history_at_base_with_ancestry(
    repo: &RepoRuntime,
    remote: &RemoteRow,
    entry: &JsonValue,
    base: &str,
    author_mode: Option<&str>,
    summary: Option<&str>,
    ancestry: Option<&PackageAncestry>,
) -> Result<PreparedSource, String> {
    let reference = required_string_field(&entry["state"]["task"], "task_id")?;
    let receipt = &entry["state"]["local_finish"];
    let target_line = required_string_field(receipt, "target_line")?;
    let original_revision = required_string_field(receipt, "landed_snapshot_id")?;
    let common = match ancestry {
        Some(ancestry) => ancestry.common.clone(),
        None => latest_common_snapshot(repo, &original_revision, Some(base))?
            .ok_or("Local history has no common Snapshot with its target Line")?,
    };
    let (entries, _) = local_completion::workflow_local_history_entries(
        repo,
        &reference,
        &target_line,
        &common,
        &original_revision,
    )?;
    preflight_source_entries(repo, &entries)?;
    let tasks = repo.task_store()?;
    let snapshot_store = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    let mut source_tasks = BTreeSet::new();
    let mut previous = common.clone();
    for entry in &entries {
        let task_id = required_string_field(entry, "local_task_id")?;
        for snapshot in entry["snapshots"]
            .as_array()
            .ok_or("Completed history Snapshot inventory missing")?
        {
            let id = required_string_field(snapshot, "snapshot_id")?;
            let owners = tasks
                .snapshot_ownership_rows(std::slice::from_ref(&id))
                .map_err(|error| error.to_string())?;
            let Some(owner) = owners.first().filter(|_| owners.len() == 1) else {
                continue;
            };
            if owner["ownership_source"].as_str() != Some("durable_snapshot_binding") {
                return Err(
                    "Package selection requires exact fixed source Snapshot ownership".into(),
                );
            }
            if owner["task_id"].as_str() != Some(task_id.as_str()) {
                return Err("Completed history Snapshot belongs to another Task".into());
            }
            if snapshot_distance_if_ancestor(repo, Some(&previous), Some(&id))?.is_none() {
                return Err("Independent source branches require an explicit combined candidate before package selection".into());
            }
            let source = snapshot_by_id_with_snapshot_store(&snapshot_store, &id)?
                .ok_or("Source Snapshot disappeared during package selection")?;
            if source.parent_snapshot_ids.len() != 1 {
                return Err("Package selection does not support an owned merge Snapshot; preserve its original DAG and prepare a supported combined candidate explicitly".into());
            }
            source_tasks.insert(task_id.clone());
            previous = id;
        }
    }
    let mut original_sources = BTreeMap::new();
    // Freeze the original successful-receipt cutoff before deriving any replay.
    for entry in &entries {
        let task_id = required_string_field(entry, "local_task_id")?;
        let original = tasks
            .completed_task_source_snapshots(&task_id)
            .map_err(|error| error.to_string())?;
        if !source_tasks.contains(&task_id) {
            return Err(format!(
                "Source Task {task_id} has no exactly owned Snapshot boundary"
            ));
        }
        for change in tasks
            .changes_for_task(&task_id)
            .map_err(|error| error.to_string())?
        {
            let id = required_string_field(&change, "change_id")?;
            if string_field(&change, "published_change_id").is_none()
                && !original.iter().any(|(_, owner)| owner.change_id == id)
            {
                return Err(format!("Source Change {task_id}/{id} has no Snapshot or explicit mapping; publication requires a recoverable source"));
            }
        }
        original_sources.insert(task_id, original);
    }
    verify_published_source_authority(repo, remote, &entries, &original_sources)?;
    let (entries, final_revision) = if common != base {
        replay::prepare_fresh_suffix(
            repo,
            remote,
            &target_line,
            base,
            &common,
            entries,
            &mut original_sources,
            ancestry,
        )?
    } else {
        (entries, original_revision)
    };
    let candidate = json!({"history_entries":entries});
    workflow_sync_history_plan_artifacts(repo, &remote.name, &candidate)?;
    let legacy = workflow_history_prepare_entries(repo, &candidate)?;
    let mut wire = Vec::new();
    let mut mappings = Vec::new();
    let mut content_ids = BTreeSet::from([base.to_string(), common.clone()]);
    if ancestry.is_none() {
        content_ids.extend(local_snapshot_chain_segment(
            repo,
            &common,
            base,
            "workflow package target ancestry",
        )?);
    }
    for (entry, old) in entries.iter().zip(legacy) {
        let task_id = required_string_field(entry, "local_task_id")?;
        let changes = tasks
            .changes_for_task(&task_id)
            .map_err(|error| error.to_string())?;
        let mut change_rows = Vec::new();
        let mut expected_changes = Vec::new();
        let mut snapshots = Vec::new();
        let mut anchors = Vec::new();
        // Receipt endpoints and adopted ancestry are content dependencies. Only
        // exact governed owners are enumerated in this entry's Snapshot list.
        for snapshot in entry["snapshots"]
            .as_array()
            .ok_or("Original Snapshot inventory missing")?
        {
            content_ids.insert(required_string_field(snapshot, "snapshot_id")?);
        }
        for (id, owner) in &original_sources[&task_id] {
            let record = snapshot_by_id_with_snapshot_store(&snapshot_store, id)?
                .ok_or("Original governed Snapshot is missing")?;
            let local_change_ref =
                (!owner.change_id.is_empty()).then(|| format!("{task_id}/{}", owner.change_id));
            content_ids.insert(id.clone());
            snapshots.push(json!({"snapshot_id":id,"created_at_s":snapshot_seconds(&record.created_at)?,"local_change_ref":local_change_ref}));
            anchors.push(PackageSnapshotOwnerExpectation {
                snapshot_id: id.clone(),
                local_change_ref,
            });
        }
        if snapshots.is_empty() {
            return Err(format!(
                "Source Task {task_id} has no exactly owned Snapshot boundary"
            ));
        }
        anchors.sort_by(|left, right| left.snapshot_id.cmp(&right.snapshot_id));
        for change in changes {
            let id = required_string_field(&change, "change_id")?;
            let reference = format!("{task_id}/{id}");
            let mapped = string_field(&change, "published_change_id");
            if mapped.is_none()
                && !anchors
                    .iter()
                    .any(|anchor| anchor.local_change_ref.as_deref() == Some(reference.as_str()))
            {
                return Err(format!("Source Change {reference} has no Snapshot or explicit mapping; publication requires a recoverable source"));
            }
            let fork = required_string_field(&change, "fork_snapshot_id")?;
            content_ids.insert(fork.clone());
            change_rows.push(json!({"local_change_id":id,"local_change_ref":reference,"expected_remote_change_ref":mapped,
                "change":{"title":required_string_field(&change,"title")?,"base_line":required_string_field(&change,"base_line")?,"fork_snapshot_id":fork}}));
            expected_changes.push(PackageChangeMappingExpectation {
                local_change_ref: reference,
                remote_change_ref: mapped,
            });
        }
        let original = &entry["local_finish"];
        let original_base = required_string_field(original, "pre_land_target_snapshot_id")?;
        let original_landed = required_string_field(original, "landed_snapshot_id")?;
        content_ids.extend([original_base.clone(), original_landed.clone()]);
        wire.push(json!({"local_task_id":task_id,"expected_remote_task_id":old["expected_remote_task_id"],"task":old["task"],"changes":change_rows,
            "pre_land_target_snapshot_id":original_base,"landed_snapshot_id":original_landed,"landed_at_s":entry["landed_at_s"],"snapshots":snapshots}));
        mappings.push(PackageMappingExpectation {
            local_task_id: task_id,
            remote_task_id: old["expected_remote_task_id"].as_str().map(str::to_string),
            changes: expected_changes,
            snapshots: anchors,
        });
    }
    // Local-only ancestor metadata must resolve before dependent Snapshot FKs.
    // Existing Remote ancestry is already authoritative and is not recopied.
    let mut remote_ancestors = ancestry
        .map(|value| value.remote_ids.clone())
        .unwrap_or_default();
    let mut pending = if ancestry.is_some() {
        Vec::new()
    } else {
        vec![base.to_string()]
    };
    while let Some(id) = pending.pop() {
        if !remote_ancestors.insert(id.clone()) {
            continue;
        }
        if remote_ancestors.len() > 65536 {
            return Err("Remote ancestry exceeds the bounded package dependency walk".into());
        }
        if let Some(row) = snapshot_by_id_with_snapshot_store(&snapshot_store, &id)? {
            pending.extend(row.parent_snapshot_ids);
        }
    }
    let mut pending = content_ids.iter().cloned().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        let row = snapshot_by_id_with_snapshot_store(&snapshot_store, &id)?
            .ok_or("Package Snapshot FK dependency is missing")?;
        for parent in row.parent_snapshot_ids {
            if !remote_ancestors.contains(&parent) && content_ids.insert(parent.clone()) {
                pending.push(parent);
            }
        }
        if content_ids.len() > 65536 {
            return Err("Package Snapshot dependencies exceed the bounded inventory".into());
        }
    }
    let ids = content_ids.into_iter().collect::<Vec<_>>();
    let plan = build_zstd_bulk_local_plan(repo, &ids, &BTreeSet::new())?;
    let mut body = ZstdBulkCommitRequest::from_json_rows(
        None,
        None,
        ordered_object_pack_metadata(&plan)?,
        ordered_tree_pack_metadata(&plan)?,
        plan.blob_locators.values().cloned().collect(),
        plan.tree_locators.values().cloned().collect(),
        plan.snapshots.values().cloned().collect(),
        None,
    )?;
    if let Some(ancestry) = ancestry {
        for snapshot in &mut body.snapshots {
            if let Some(actual) = ancestry.remote_rows.get(&snapshot.snapshot_id) {
                *snapshot = actual.clone();
            }
        }
    }
    let objects = plan
        .object_packs
        .iter()
        .map(|(id, pack)| {
            Ok((
                id.clone(),
                fs::read(&pack.pack_abs_path).map_err(|error| error.to_string())?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let trees = plan
        .tree_packs
        .iter()
        .map(|(id, pack)| {
            Ok((
                id.clone(),
                fs::read(&pack.pack_abs_path).map_err(|error| error.to_string())?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    let manifest = WorkflowPackageManifest {
        repository_index: repo.require_repository_index()?.get(),
        target_line,
        base_snapshot_id: base.to_string(),
        revision_snapshot_id: final_revision,
        author_mode: repo.effective_author_mode(author_mode),
        summary: summary.map(str::to_string),
        history_entries: wire,
        content: body,
    };
    let package = manifest.encode(&objects, &trees)?;
    Ok(PreparedSource {
        manifest,
        package,
        mappings,
    })
}
