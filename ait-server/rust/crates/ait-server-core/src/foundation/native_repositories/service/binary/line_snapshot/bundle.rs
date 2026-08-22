use super::*;
use crate::foundation::server_content_binary_db::ServerBinaryTreeReadCache;

impl<D> BinaryDbNativeRepositoryService<D>
where
    D: ServerRemoteBinaryDb + BinaryDbIndexAppender + Clone,
{
    pub(in super::super) fn snapshot_files_for_value(
        &self,
        value: &JsonValue,
    ) -> Result<Vec<JsonValue>, NativeRepositoryError> {
        let root_tree_pack_id = binary_json_text(value, "root_tree_pack_id").ok_or_else(|| {
            NativeRepositoryError::internal("canonical snapshot is missing root_tree_pack_id")
        })?;
        let root_entry_ordinal = value
            .get("root_entry_ordinal")
            .and_then(JsonValue::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                NativeRepositoryError::internal(
                    "canonical snapshot has an invalid root_entry_ordinal",
                )
            })?;
        self.snapshot_files_for_root(&root_tree_pack_id, root_entry_ordinal)
    }

    fn snapshot_files_for_root(
        &self,
        root_tree_pack_id: &str,
        root_entry_ordinal: u32,
    ) -> Result<Vec<JsonValue>, NativeRepositoryError> {
        let read = BinaryDbReadTxn::new_bounded_for_scope(&self.db, BinaryDbReadScope::CONTENT);
        let content = self.repository_content();
        let root_pack = content
            .tree_pack_with_read(&read, root_tree_pack_id)
            .map_err(binary_native_repository_store_error)?
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "root tree pack {root_tree_pack_id} is not present in the Binary DB content schema"
                ))
            })?;
        let root_tree = content
            .tree_for_pack_entry_ordinal_with_read(&read, &root_pack, root_entry_ordinal)
            .map_err(binary_native_repository_store_error)?;
        let mut files = BTreeMap::new();
        let mut visited = BTreeSet::new();
        let mut tree_read_cache = ServerBinaryTreeReadCache::default();
        collect_snapshot_tree_files(
            &content,
            &read,
            &root_tree.tree_id,
            "",
            &mut visited,
            &mut files,
            &mut tree_read_cache,
        )?;
        Ok(files.into_values().collect())
    }

    pub(in super::super) fn materialize_canonical_snapshot(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        selected_paths: Option<&[PathBuf]>,
    ) -> Result<JsonValue, NativeRepositoryError> {
        let snapshot = self.latest_snapshot_value(repo_name, snapshot_id)?;
        let files = self.snapshot_files_for_value(&snapshot)?;
        let selected = selected_paths
            .map(|paths| {
                paths
                    .iter()
                    .map(|path| {
                        let text = path.to_str().ok_or_else(|| {
                            NativeRepositoryError::bad_request(
                                "snapshot selected path is not valid UTF-8",
                            )
                        })?;
                        validated_materialized_relative_path(text)?;
                        Ok(text.to_string())
                    })
                    .collect::<Result<Vec<_>, NativeRepositoryError>>()
            })
            .transpose()?;
        self.materialize_canonical_files(
            repo_name,
            snapshot_id,
            destination,
            &snapshot,
            &files,
            selected.as_deref(),
            "schema_defined_tree",
        )
    }

    pub(in super::super) fn materialize_canonical_manifest_entries(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        entries: &[SnapshotManifestFileEntry],
    ) -> Result<JsonValue, NativeRepositoryError> {
        let snapshot = self.latest_snapshot_value(repo_name, snapshot_id)?;
        let files = self.snapshot_files_for_value(&snapshot)?;
        let canonical = files
            .iter()
            .filter_map(|file| {
                file.get("path")
                    .and_then(JsonValue::as_str)
                    .map(|path| (path.to_string(), file))
            })
            .collect::<BTreeMap<_, _>>();
        let mut selected = Vec::with_capacity(entries.len());
        for entry in entries {
            validated_materialized_relative_path(&entry.path)?;
            let file = canonical.get(&entry.path).ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "snapshot manifest path {} is not present in canonical tree entries",
                    entry.path
                ))
            })?;
            if file.get("blob_id").and_then(JsonValue::as_str) != Some(entry.blob_id.as_str())
                || file.get("size_bytes").and_then(JsonValue::as_i64) != Some(entry.size_bytes)
                || file.get("mode").and_then(JsonValue::as_str) != Some(entry.mode.as_str())
                || (!entry.sha256.trim().is_empty()
                    && file.get("sha256").and_then(JsonValue::as_str)
                        != Some(entry.sha256.as_str()))
            {
                return Err(NativeRepositoryError::bad_request(format!(
                    "snapshot manifest row {} disagrees with canonical Binary DB tree/blob records",
                    entry.path
                )));
            }
            selected.push(entry.path.clone());
        }
        self.materialize_canonical_files(
            repo_name,
            snapshot_id,
            destination,
            &snapshot,
            &files,
            Some(&selected),
            "schema_defined_manifest_entries",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn materialize_canonical_files(
        &self,
        repo_name: &str,
        snapshot_id: &str,
        destination: &Path,
        snapshot: &JsonValue,
        files: &[JsonValue],
        selected_paths: Option<&[String]>,
        source: &str,
    ) -> Result<JsonValue, NativeRepositoryError> {
        std::fs::create_dir_all(destination).map_err(|error| {
            NativeRepositoryError::internal(format!(
                "failed to create snapshot destination {}: {error}",
                destination.display()
            ))
        })?;
        let mut written_files = 0_u64;
        let mut written_bytes = 0_u64;
        let mut read_session = BinaryBlobReadSession::default();
        for file in files {
            let object = file.as_object().ok_or_else(|| {
                NativeRepositoryError::internal("canonical snapshot file must be an object")
            })?;
            let path =
                required_json_text(object, "path").map_err(NativeRepositoryError::internal)?;
            if selected_paths.is_some_and(|selected| {
                !selected
                    .iter()
                    .any(|value| path == *value || path.starts_with(&format!("{value}/")))
            }) {
                continue;
            }
            let relative = validated_materialized_relative_path(&path)?;
            let blob_id =
                required_json_text(object, "blob_id").map_err(NativeRepositoryError::internal)?;
            let bytes = self.read_binary_blob_content_with_session(&blob_id, &mut read_session)?;
            validate_canonical_file_bytes(object, &bytes)?;
            let output = destination.join(relative);
            if let Some(parent) = output.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    NativeRepositoryError::internal(format!(
                        "failed to create snapshot parent {}: {error}",
                        parent.display()
                    ))
                })?;
            }
            if output
                .symlink_metadata()
                .is_ok_and(|metadata| metadata.file_type().is_symlink())
            {
                return Err(NativeRepositoryError::bad_request(format!(
                    "snapshot output {} is a symbolic link",
                    output.display()
                )));
            }
            std::fs::write(&output, &bytes).map_err(|error| {
                NativeRepositoryError::internal(format!(
                    "failed to write snapshot file {}: {error}",
                    output.display()
                ))
            })?;
            #[cfg(unix)]
            if let Some(mode) = object
                .get("mode")
                .and_then(JsonValue::as_str)
                .and_then(snapshot_file_permissions_mode)
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&output, std::fs::Permissions::from_mode(mode)).map_err(
                    |error| {
                        NativeRepositoryError::internal(format!(
                            "failed to set snapshot file mode {}: {error}",
                            output.display()
                        ))
                    },
                )?;
            }
            written_files += 1;
            written_bytes = written_bytes
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| NativeRepositoryError::internal("materialized byte overflow"))?;
        }
        Ok(json!({
            "contract": "ait.server.native_repository.snapshot_materialize.v1",
            "repo_name": repo_name,
            "repo_id": self.repo_id(),
            "snapshot_id": snapshot_id,
            "destination_path": destination.to_string_lossy(),
            "file_count": written_files,
            "total_bytes": written_bytes,
            "root_tree_pack_id": snapshot.get("root_tree_pack_id").cloned().unwrap_or(JsonValue::Null),
            "root_entry_ordinal": snapshot.get("root_entry_ordinal").cloned().unwrap_or(json!(0)),
            "selected_paths": selected_paths.unwrap_or(&[]),
            "selected_path_count": selected_paths.map_or(0, <[String]>::len),
            "selected_path_materialization": selected_paths.is_some(),
            "source": source,
            "content_transport": "schema_defined_object_pack_members",
            "json_snapshot_payload": false,
        }))
    }
}

fn collect_snapshot_tree_files<D>(
    content: &ServerBinaryRepositoryContentStore<D>,
    read: &BinaryDbReadTxn<'_, D>,
    tree_id: &str,
    prefix: &str,
    visited: &mut BTreeSet<String>,
    files: &mut BTreeMap<String, JsonValue>,
    tree_read_cache: &mut ServerBinaryTreeReadCache,
) -> Result<(), NativeRepositoryError>
where
    D: ServerRemoteBinaryDb + Clone,
{
    if !visited.insert(tree_id.to_string()) {
        return Err(NativeRepositoryError::internal(format!(
            "cycle detected while reading Binary DB tree {tree_id}"
        )));
    }
    let result = (|| {
        for entry in content
            .tree_entries_with_read_cache(read, tree_id, tree_read_cache)
            .map_err(binary_native_repository_store_error)?
        {
            let path = if prefix.is_empty() {
                entry.entry_name.clone()
            } else {
                format!("{prefix}/{}", entry.entry_name)
            };
            if entry.entry_type == "tree" {
                collect_snapshot_tree_files(
                    content,
                    read,
                    &entry.target_id,
                    &path,
                    visited,
                    files,
                    tree_read_cache,
                )?;
                continue;
            }
            let file = json!({
                "path": path,
                "blob_id": entry.target_id,
                "size_bytes": entry.size_bytes,
                "mode": entry.mode,
                "sha256": entry.sha256,
            });
            if files.insert(path.clone(), file).is_some() {
                return Err(NativeRepositoryError::internal(format!(
                    "duplicate Binary DB snapshot path {path}"
                )));
            }
        }
        Ok(())
    })();
    visited.remove(tree_id);
    result
}

fn validate_canonical_file_bytes(
    file: &JsonMap<String, JsonValue>,
    bytes: &[u8],
) -> Result<(), NativeRepositoryError> {
    let path = file
        .get("path")
        .and_then(JsonValue::as_str)
        .unwrap_or("<unknown>");
    if file.get("size_bytes").and_then(JsonValue::as_u64) != Some(bytes.len() as u64) {
        return Err(NativeRepositoryError::internal(format!(
            "canonical snapshot file {path} size disagrees with blob content"
        )));
    }
    if let Some(expected) = file.get("sha256").and_then(JsonValue::as_str) {
        let actual = sha256_hex(bytes);
        if expected != actual {
            return Err(NativeRepositoryError::internal(format!(
                "canonical snapshot file {path} checksum disagrees with blob content"
            )));
        }
    }
    Ok(())
}

fn validated_materialized_relative_path(path: &str) -> Result<PathBuf, NativeRepositoryError> {
    let relative = PathBuf::from(path);
    if relative.as_os_str().is_empty() || relative.is_absolute() {
        return Err(NativeRepositoryError::bad_request(format!(
            "snapshot materialized path {path:?} must be relative"
        )));
    }
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(NativeRepositoryError::bad_request(format!(
            "snapshot materialized path {path:?} escapes destination"
        )));
    }
    Ok(relative)
}

#[cfg(unix)]
fn snapshot_file_permissions_mode(mode: &str) -> Option<u32> {
    let text = mode.trim().trim_start_matches("0o");
    u32::from_str_radix(text, 8)
        .ok()
        .map(|value| value & 0o7777)
}
