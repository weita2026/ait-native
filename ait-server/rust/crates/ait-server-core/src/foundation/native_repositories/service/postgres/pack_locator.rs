use super::*;

pub(in crate::foundation::native_repositories) fn walk_tree_rows(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    root_tree_id: &str,
    root_rows: JsonValue,
    path_filter: Option<&str>,
) -> Result<Vec<SnapshotFileEntry>, NativeRepositoryError> {
    let mut cached_rows = BTreeMap::new();
    cached_rows.insert(root_tree_id.to_string(), root_rows);
    let mut stack = vec![(String::new(), root_tree_id.to_string())];
    let mut entries = Vec::new();
    while let Some((prefix, tree_id)) = stack.pop() {
        let rows = if let Some(existing) = cached_rows.get(&tree_id) {
            existing.clone()
        } else {
            let (tree_pack_path, tree_pack_format) =
                tree_pack_locator_for_tree_id(client, paths, repo_name, repo_id, &tree_id)?;
            let rows = read_tree_pack_tree_with_format(
                path_to_string(&tree_pack_path)?.as_str(),
                &tree_id,
                &tree_pack_format,
            )
            .map_err(NativeRepositoryError::internal)?;
            cached_rows.insert(tree_id.clone(), rows.clone());
            rows
        };
        let rows = rows
            .as_array()
            .ok_or_else(|| NativeRepositoryError::internal("tree rows payload must be an array"))?;
        for row in rows {
            let object = row
                .as_object()
                .ok_or_else(|| NativeRepositoryError::internal("tree row must be an object"))?;
            let entry_name = required_json_text(object, "entry_name")
                .map_err(NativeRepositoryError::internal)?;
            let entry_type = required_json_text(object, "entry_type")
                .map_err(NativeRepositoryError::internal)?;
            let target_id =
                required_json_text(object, "target_id").map_err(NativeRepositoryError::internal)?;
            let full_path = format!("{prefix}{entry_name}");
            if entry_type == "tree" {
                stack.push((format!("{full_path}/"), target_id));
                continue;
            }
            if entry_type != "blob" {
                continue;
            }
            if let Some(filter) = path_filter {
                if filter != full_path {
                    continue;
                }
            }
            let size_bytes = object
                .get("size_bytes")
                .map(json_i64)
                .transpose()?
                .unwrap_or(0_i64);
            let mode =
                required_json_text(object, "mode").map_err(NativeRepositoryError::internal)?;
            let blob = select_blob_by_id(client, &target_id)?.ok_or_else(|| {
                NativeRepositoryError::internal(format!("missing blob metadata for {}", target_id))
            })?;
            entries.push(SnapshotFileEntry {
                path: full_path,
                blob_id: target_id,
                size_bytes,
                mode,
                sha256: blob.sha256,
            });
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

pub(in crate::foundation::native_repositories) fn tree_pack_locator_for_tree_id(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    tree_id: &str,
) -> Result<(PathBuf, String), NativeRepositoryError> {
    let row = client
        .query_opt(
            "select t.tree_pack_id, tp.repo_name, tp.repo_id, tp.pack_path, tp.pack_format from trees t join tree_packs tp on tp.pack_id = t.tree_pack_id where t.tree_id = $1 and tp.repo_name = $2 and tp.repo_id = $3 and coalesce(t.tree_pack_id, '') <> ''",
            &[&tree_id, &repo_name, &repo_id],
        )
        .map_err(db_internal)?
        .ok_or_else(|| {
            NativeRepositoryError::internal(format!(
                "Tree {tree_id} is missing tree-pack metadata required for snapshot traversal."
            ))
        })?;
    let pack_path_text: String = row.get("pack_path");
    let pack_format: String = row.get("pack_format");
    Ok((runtime_storage_path(paths, &pack_path_text), pack_format))
}

pub(super) fn pack_locator_for_id(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    pack_id: &str,
) -> Result<(PathBuf, String), NativeRepositoryError> {
    let row = client
        .query_opt(
            "select pack_path, pack_format from packs where pack_id = $1",
            &[&pack_id],
        )
        .map_err(db_internal)?
        .ok_or_else(|| {
            NativeRepositoryError::internal(format!("Missing pack metadata for {pack_id}"))
        })?;
    Ok((
        runtime_storage_path(paths, &row.get::<_, String>("pack_path")),
        row.get("pack_format"),
    ))
}

pub(in crate::foundation::native_repositories) fn tree_pack_locator_for_id(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    pack_id: &str,
) -> Result<(PathBuf, String), NativeRepositoryError> {
    let row = client
        .query_opt(
            "select repo_name, repo_id, pack_path, pack_format from tree_packs where pack_id = $1 and repo_name = $2 and repo_id = $3",
            &[&pack_id, &repo_name, &repo_id],
        )
        .map_err(db_internal)?
        .ok_or_else(|| {
            NativeRepositoryError::internal(format!("Missing tree pack metadata for {pack_id}"))
        })?;
    Ok((
        runtime_storage_path(paths, &row.get::<_, String>("pack_path")),
        row.get("pack_format"),
    ))
}
