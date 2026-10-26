use super::*;
use ait_core::repository_pack_json::ZstdBulkSnapshotRow;
use ait_core::snapshot_dag::{snapshot_ancestor_closure, snapshot_merge_bases_from_parent_map};
use ait_core::task_workflow_http_adapter::TaskWorkflowSnapshotMetadataReader;

/// Read-time supplementation, never a replacement for a persisted boundary.
pub(super) struct PackageAncestry {
    pub common: String,
    pub remote_ids: BTreeSet<String>,
    pub remote_rows: BTreeMap<String, ZstdBulkSnapshotRow>,
}

impl PackageAncestry {
    pub fn load<R: TaskWorkflowSnapshotMetadataReader + ?Sized>(
        repo: &RepoRuntime,
        remote: &mut R,
        repo_name: &str,
        original: &str,
        base: &str,
    ) -> Result<Self, String> {
        let limits = SnapshotDagLimits {
            max_depth: 65536,
            max_results: 65536,
            ..Default::default()
        };
        let local = repo.local_content_maintenance_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>()?;
        let source = snapshot_ancestor_closure(
            &local,
            &[original.to_string()],
            &BTreeSet::new(),
            SnapshotParentMode::AllParents,
            limits,
        )?;
        let mut parents = source.parent_snapshot_ids;
        let mut remote_ids = BTreeSet::new();
        let mut remote_rows = BTreeMap::new();
        let mut pending = vec![base.to_string()];
        let mut queued = BTreeSet::from([base.to_string()]);
        while let Some(id) = pending.pop() {
            if !remote_ids.insert(id.clone()) {
                continue;
            }
            if remote_ids.len() > limits.max_results {
                return Err("Remote ancestry exceeds the bounded package dependency walk".into());
            }
            let known = {
                let read = local.snapshots().begin_read_txn();
                local
                    .snapshots()
                    .get_snapshot_view(&read, &id)
                    .map_err(|error| error.to_string())?
            };
            let actual = match known.as_ref() {
                Some(row) if !row.record.is_remote_head_history_boundary() => {
                    row.parent_snapshot_ids.clone()
                }
                _ => {
                    let response = remote
                        .get_remote_snapshot(repo_name, &id, false, None)
                        .map_err(|error| error.to_string())?;
                    let value = ait_core::workflow_package::package_response_data(&response)?;
                    if value["snapshot_id"].as_str() != Some(id.as_str())
                        || value["repository_index"].as_u64()
                            != Some(u64::from(repo.require_repository_index()?.get()))
                        || value["repo_name"].as_str() != Some(repo_name)
                        || value["remote_head_history_boundary"].as_bool() != Some(false)
                        || !value["parent_snapshot_ids"].is_array()
                    {
                        return Err(
                            "Remote package ancestry has a mismatched or incomplete Snapshot"
                                .into(),
                        );
                    }
                    let mut decoded = ZstdBulkCommitRequest::from_json_rows(
                        None,
                        None,
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        Vec::new(),
                        vec![value.clone()],
                        None,
                    )?;
                    let row = decoded.snapshots.remove(0);
                    if let Some(known) = &known {
                        if row.manifest_hash.as_deref() != Some(known.manifest_hash.as_str()) {
                            return Err(
                                "Remote ancestry conflicts with immutable local Snapshot content"
                                    .into(),
                            );
                        }
                    }
                    let actual = row.parent_snapshot_ids.clone();
                    remote_rows.insert(id.clone(), row);
                    actual
                }
            };
            for parent in &actual {
                if queued.insert(parent.clone()) {
                    if queued.len() > limits.max_results {
                        return Err("Remote ancestry exceeds its bounded inventory".into());
                    }
                    pending.push(parent.clone());
                }
            }
            parents.insert(id, actual);
        }
        let bases = snapshot_merge_bases_from_parent_map(&parents, original, base, limits)?;
        let [common] = bases.as_slice() else {
            return Err("Package publication requires one exact merge base; absent or multiple incomparable bases are unsupported".into());
        };
        let common = common.clone();
        Ok(Self {
            common,
            remote_ids,
            remote_rows,
        })
    }
}
