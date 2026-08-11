use super::api::{NativeRepositoryError, SnapshotManifestFileEntry};
use super::service::{
    blob_bytes_for_blob_id, ensure_zstd_only_repository_flow_allowed, path_string, path_to_string,
    select_repository_row, select_snapshot_row, tree_pack_locator_for_id, walk_tree_rows,
    ServerRuntimePaths, SnapshotFileEntry, SnapshotRow, ZstdOnlyRepositoryFlow,
};
use crate::foundation::pack_substrate::read_tree_pack_tree_by_ordinal_with_format;
use serde_json::{json, Number as JsonNumber, Value as JsonValue};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub(super) fn materialize_snapshot_json(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    snapshot_id: &str,
    destination: &Path,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    ensure_zstd_only_repository_flow_allowed(
        client,
        repo_name,
        &repo,
        ZstdOnlyRepositoryFlow::SnapshotMaterialize,
    )?;
    let (snapshot, file_entries) = snapshot_file_entries(client, paths, repo_name, snapshot_id)?;
    materialize_snapshot_entries_json(
        client,
        paths,
        repo_name,
        snapshot_id,
        destination,
        snapshot,
        file_entries,
        None,
    )
}

pub(super) fn materialize_snapshot_paths_json(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    snapshot_id: &str,
    destination: &Path,
    relative_paths: &[PathBuf],
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    ensure_zstd_only_repository_flow_allowed(
        client,
        repo_name,
        &repo,
        ZstdOnlyRepositoryFlow::SnapshotMaterialize,
    )?;
    let (snapshot, file_entries) = snapshot_file_entries(client, paths, repo_name, snapshot_id)?;
    let selected = relative_paths
        .iter()
        .map(|path| {
            let text = path_to_string(path)?;
            validated_materialized_relative_path(&text)?;
            Ok(text)
        })
        .collect::<Result<Vec<_>, NativeRepositoryError>>()?;
    let mut selected_entries = BTreeMap::<String, SnapshotFileEntry>::new();
    for entry in file_entries {
        if selected
            .iter()
            .any(|path| snapshot_entry_matches_selected_path(entry.path.as_str(), path.as_str()))
        {
            selected_entries.insert(entry.path.clone(), entry);
        }
    }
    materialize_snapshot_entries_json(
        client,
        paths,
        repo_name,
        snapshot_id,
        destination,
        snapshot,
        selected_entries.into_values().collect(),
        Some(selected),
    )
}

pub(super) fn materialize_snapshot_manifest_entries_json(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    snapshot_id: &str,
    destination: &Path,
    entries: &[SnapshotManifestFileEntry],
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    ensure_zstd_only_repository_flow_allowed(
        client,
        repo_name,
        &repo,
        ZstdOnlyRepositoryFlow::SnapshotMaterialize,
    )?;
    let snapshot = select_snapshot_row(client, repo_name, snapshot_id)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!(
            "Unknown snapshot {snapshot_id} for repository {repo_name}"
        ))
    })?;
    fs::create_dir_all(destination).map_err(|exc| {
        NativeRepositoryError::internal(format!(
            "failed to create snapshot materialization destination `{}`: {exc}",
            path_string(destination)
        ))
    })?;
    let mut written_files = 0_i64;
    let mut written_bytes = 0_i64;
    let mut selected = Vec::with_capacity(entries.len());
    for entry in entries {
        let relative_path = validated_materialized_relative_path(&entry.path)?;
        selected.push(entry.path.clone());
        let output_path = destination.join(&relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|exc| {
                NativeRepositoryError::internal(format!(
                    "failed to create snapshot materialized parent `{}`: {exc}",
                    path_string(parent)
                ))
            })?;
        }
        let bytes = blob_bytes_for_blob_id(
            client,
            paths,
            &snapshot.repo_name,
            &snapshot.repo_id,
            &entry.blob_id,
        )?;
        if !entry.sha256.trim().is_empty() {
            let actual_sha256 = super::service::sha256_hex(&bytes);
            if actual_sha256 != entry.sha256 {
                return Err(NativeRepositoryError::internal(format!(
                    "snapshot manifest row `{}` expected blob {} sha256 {}, got {}",
                    entry.path, entry.blob_id, entry.sha256, actual_sha256
                )));
            }
        }
        fs::write(&output_path, &bytes).map_err(|exc| {
            NativeRepositoryError::internal(format!(
                "failed to write snapshot materialized file `{}`: {exc}",
                path_string(&output_path)
            ))
        })?;
        #[cfg(unix)]
        if let Some(mode) = snapshot_file_permissions_mode(&entry.mode) {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&output_path, fs::Permissions::from_mode(mode)).map_err(|exc| {
                NativeRepositoryError::internal(format!(
                    "failed to set snapshot materialized file mode `{}`: {exc}",
                    path_string(&output_path)
                ))
            })?;
        }
        written_files += 1;
        written_bytes += bytes.len() as i64;
    }
    Ok(json!({
        "contract": "ait.server.native_repository.snapshot_materialize.v1",
        "repo_name": repo_name,
        "snapshot_id": snapshot_id,
        "destination_path": path_string(destination),
        "file_count": written_files,
        "total_bytes": written_bytes,
        "root_tree_pack_id": snapshot.root_tree_pack_id,
        "root_entry_ordinal": snapshot.root_entry_ordinal,
        "selected_paths": selected,
        "selected_path_count": entries.len(),
        "selected_path_materialization": true,
        "source": "snapshot_manifest_entries",
        "content_transport": "inline_blob_read_model_or_pack_to_filesystem",
        "json_snapshot_payload": false,
    }))
}

fn snapshot_file_entries(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<(SnapshotRow, Vec<SnapshotFileEntry>), NativeRepositoryError> {
    let snapshot = select_snapshot_row(client, repo_name, snapshot_id)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!(
            "Unknown snapshot {snapshot_id} for repository {repo_name}"
        ))
    })?;
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
    let file_entries = walk_tree_rows(
        client,
        paths,
        &snapshot.repo_name,
        &snapshot.repo_id,
        &root_tree_id,
        root_rows,
        None,
    )?;
    Ok((snapshot, file_entries))
}

fn materialize_snapshot_entries_json(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    snapshot_id: &str,
    destination: &Path,
    snapshot: SnapshotRow,
    file_entries: Vec<SnapshotFileEntry>,
    selected_paths: Option<Vec<String>>,
) -> Result<JsonValue, NativeRepositoryError> {
    fs::create_dir_all(destination).map_err(|exc| {
        NativeRepositoryError::internal(format!(
            "failed to create snapshot materialization destination `{}`: {exc}",
            path_string(destination)
        ))
    })?;
    let mut written_files = 0_i64;
    let mut written_bytes = 0_i64;
    for entry in file_entries {
        let relative_path = validated_materialized_relative_path(&entry.path)?;
        let output_path = destination.join(&relative_path);
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent).map_err(|exc| {
                NativeRepositoryError::internal(format!(
                    "failed to create snapshot materialized parent `{}`: {exc}",
                    path_string(parent)
                ))
            })?;
        }
        let bytes = blob_bytes_for_blob_id(
            client,
            paths,
            &snapshot.repo_name,
            &snapshot.repo_id,
            &entry.blob_id,
        )?;
        fs::write(&output_path, &bytes).map_err(|exc| {
            NativeRepositoryError::internal(format!(
                "failed to write snapshot materialized file `{}`: {exc}",
                path_string(&output_path)
            ))
        })?;
        #[cfg(unix)]
        if let Some(mode) = snapshot_file_permissions_mode(&entry.mode) {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&output_path, fs::Permissions::from_mode(mode)).map_err(|exc| {
                NativeRepositoryError::internal(format!(
                    "failed to set snapshot materialized file mode `{}`: {exc}",
                    path_string(&output_path)
                ))
            })?;
        }
        written_files += 1;
        written_bytes += bytes.len() as i64;
    }
    let mut payload = json!({
        "contract": "ait.server.native_repository.snapshot_materialize.v1",
        "repo_name": repo_name,
        "snapshot_id": snapshot_id,
        "destination_path": path_string(destination),
        "file_count": written_files,
        "total_bytes": written_bytes,
        "root_tree_pack_id": snapshot.root_tree_pack_id,
        "root_entry_ordinal": snapshot.root_entry_ordinal,
        "content_transport": "direct_pack_to_filesystem",
        "json_snapshot_payload": false,
    });
    if let Some(selected_paths) = selected_paths {
        let object = payload
            .as_object_mut()
            .expect("snapshot materialize payload should be an object");
        object.insert(
            "selected_paths".to_string(),
            JsonValue::Array(
                selected_paths
                    .iter()
                    .cloned()
                    .map(JsonValue::String)
                    .collect(),
            ),
        );
        object.insert(
            "selected_path_count".to_string(),
            JsonValue::Number(JsonNumber::from(selected_paths.len() as u64)),
        );
        object.insert(
            "selected_path_materialization".to_string(),
            JsonValue::Bool(true),
        );
    }
    Ok(payload)
}

fn snapshot_entry_matches_selected_path(entry_path: &str, selected_path: &str) -> bool {
    entry_path == selected_path || entry_path.starts_with(format!("{selected_path}/").as_str())
}

fn validated_materialized_relative_path(path: &str) -> Result<PathBuf, NativeRepositoryError> {
    let relative = PathBuf::from(path);
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(NativeRepositoryError::bad_request(format!(
            "snapshot materialized path `{path}` must be relative"
        )));
    }
    for component in relative.components() {
        match component {
            std::path::Component::Normal(_) | std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                return Err(NativeRepositoryError::bad_request(format!(
                    "snapshot materialized path `{path}` escapes destination"
                )));
            }
        }
    }
    Ok(relative)
}

fn snapshot_file_permissions_mode(mode: &str) -> Option<u32> {
    let text = mode.trim().trim_start_matches("0o");
    u32::from_str_radix(text, 8)
        .ok()
        .map(|value| value & 0o7777)
}
