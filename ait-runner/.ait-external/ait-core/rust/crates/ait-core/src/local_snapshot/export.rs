use super::*;

pub fn export_snapshot_source_manifest_with_store<S>(
    snapshot_id: &str,
    repo_name: &str,
    storage: &S,
) -> Result<JsonValue, String>
where
    S: SnapshotStore + LocalSnapshotTreeReadStore + ?Sized,
{
    let snapshot_id = require_non_empty(snapshot_id, "snapshot_id")?;
    let repo_name = require_non_empty(repo_name, "repo_name")?;
    let mut snapshot_obj =
        snapshot_source_manifest_object_from_store(storage, &snapshot_id, &repo_name)?;
    snapshot_obj.insert(
        "manifest_path".to_string(),
        JsonValue::String(storage.snapshot_tree_manifest_path(&snapshot_id)?),
    );

    let files = storage
        .snapshot_tree_file_rows(Some(&snapshot_id))?
        .into_iter()
        .map(|row| {
            json!({
                "path": row.path,
                "blob_id": row.blob_id,
                "size_bytes": row.size_bytes,
                "mode": row.mode,
                "sha256": row.sha256,
            })
        })
        .collect();
    snapshot_obj.insert("files".to_string(), JsonValue::Array(files));
    Ok(JsonValue::Object(snapshot_obj))
}

fn snapshot_source_manifest_object_from_store<S>(
    storage: &S,
    snapshot_id: &str,
    repo_name: &str,
) -> Result<JsonMap<String, JsonValue>, String>
where
    S: SnapshotStore + ?Sized,
{
    let snapshot = storage
        .snapshot_by_id(snapshot_id)?
        .ok_or_else(|| format!("Unknown snapshot: {snapshot_id}"))?;
    snapshot_source_manifest_object_from_record(snapshot, repo_name)
}

fn snapshot_source_manifest_object_from_record(
    snapshot: SnapshotRecord,
    repo_name: &str,
) -> Result<JsonMap<String, JsonValue>, String> {
    let snapshot = json!({
        "snapshot_id": snapshot.snapshot_id,
        "repo_name": repo_name,
        "parent_snapshot_ids": snapshot.parent_snapshot_ids,
        "primary_parent_snapshot_id": snapshot.primary_parent_snapshot_id,
        "parent_snapshot_id": snapshot.parent_snapshot_id,
        "root_tree_pack_id": snapshot.root_tree_pack_id,
        "root_entry_ordinal": snapshot.root_entry_ordinal,
        "manifest_hash": snapshot.manifest_hash,
        "message": snapshot.message,
        "line_name": snapshot.line_name,
        "snapshot_kind": snapshot.snapshot_kind,
        "file_count": snapshot.file_count,
        "total_bytes": snapshot.total_bytes,
        "created_at": snapshot.created_at,
    });
    snapshot
        .as_object()
        .cloned()
        .ok_or_else(|| "snapshot source manifest payload must be an object".to_string())
}
