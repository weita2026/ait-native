use super::api::{NativeRepositoryError, SnapshotExportQuery};
use super::service::{
    path_to_string, select_snapshot_row, snapshot_json_from_row, tree_pack_locator_for_id,
    walk_tree_rows, ServerRuntimePaths,
};
use crate::foundation::pack_substrate::read_tree_pack_tree_by_ordinal_with_format;
use serde_json::{json, Value as JsonValue};

pub(super) fn export_snapshot_json(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    snapshot_id: &str,
    query: SnapshotExportQuery,
) -> Result<JsonValue, NativeRepositoryError> {
    let snapshot = select_snapshot_row(client, repo_name, snapshot_id)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!(
            "Unknown snapshot {snapshot_id} for repository {repo_name}"
        ))
    })?;
    let metadata = snapshot_json_from_row(client, paths, snapshot.clone())?;
    let metadata_object = metadata
        .as_object()
        .cloned()
        .ok_or_else(|| NativeRepositoryError::internal("snapshot payload must be an object"))?;
    let (root_tree_pack_path, root_tree_pack_format) = tree_pack_locator_for_id(
        client,
        paths,
        &snapshot.repo_name,
        &snapshot.repo_id,
        &snapshot.root_tree_pack_id,
    )?;
    let root_payload = read_tree_pack_tree_by_ordinal_with_format(
        path_to_string(&root_tree_pack_path)?.as_str(),
        snapshot.root_entry_ordinal,
        &root_tree_pack_format,
    )
    .map_err(NativeRepositoryError::internal)?;
    let root_tree_id = root_payload
        .get("tree_id")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .to_string();
    let root_rows = root_payload
        .get("rows")
        .cloned()
        .unwrap_or_else(|| JsonValue::Array(Vec::new()));
    let files = walk_tree_rows(
        client,
        paths,
        &snapshot.repo_name,
        &snapshot.repo_id,
        &root_tree_id,
        root_rows,
        query.path.as_deref(),
    )?
    .into_iter()
    .map(|entry| {
        json!({
            "path": entry.path,
            "blob_id": entry.blob_id,
            "size_bytes": entry.size_bytes,
            "mode": entry.mode,
            "sha256": entry.sha256,
        })
    })
    .collect::<Vec<_>>();
    let mut payload = metadata_object;
    payload.insert(
        "repo_name".to_string(),
        JsonValue::String(repo_name.to_string()),
    );
    payload.insert("content_included".to_string(), JsonValue::Bool(false));
    payload.insert("files".to_string(), JsonValue::Array(files));
    Ok(JsonValue::Object(payload))
}
