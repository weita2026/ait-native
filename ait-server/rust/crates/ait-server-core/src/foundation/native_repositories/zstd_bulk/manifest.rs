use super::*;

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn get_zstd_pull_manifest_json(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    request: &RemoteSyncZstdPullManifestRequest,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    ensure_zstd_only_repository_flow_allowed(
        client,
        repo_name,
        &repo,
        ZstdOnlyRepositoryFlow::ZstdImportManifest,
    )?;
    let (snapshots, boundary_snapshot_ids) = postgres_zstd_pull_manifest_snapshots(
        client,
        repo_name,
        &request.head_snapshot_id,
        &request.have_snapshot_ids,
    )?;
    let closure =
        walk_zstd_import_manifest_tree_closure_for_snapshots(client, paths, &repo, &snapshots)?;
    let tree_locator_records =
        zstd_import_manifest_tree_locator_records(client, &closure.tree_ids)?;
    let tree_pack_ids = tree_locator_records
        .iter()
        .map(|record| record.tree_pack_id.clone())
        .collect::<BTreeSet<_>>();
    let (tree_packs, tree_pack_indexes) =
        zstd_import_manifest_tree_pack_rows(client, paths, &repo, &tree_pack_ids)?;
    let tree_locators =
        zstd_import_manifest_tree_locator_rows(&tree_locator_records, &tree_pack_indexes)?;
    let blob_locator_records = zstd_import_manifest_blob_locator_records(
        client,
        &repo.repo_name,
        &repo.repo_id,
        &closure.blob_ids,
    )?;
    let object_pack_ids = blob_locator_records
        .iter()
        .map(|record| record.pack_id.clone())
        .collect::<BTreeSet<_>>();
    let (object_packs, object_pack_indexes) =
        zstd_import_manifest_object_pack_rows(client, paths, &repo, &object_pack_ids)?;
    let blob_locators =
        zstd_import_manifest_blob_locator_rows(&blob_locator_records, &object_pack_indexes)?;
    let snapshot_rows = snapshots
        .iter()
        .map(zstd_import_manifest_snapshot_row)
        .collect::<Vec<_>>();
    Ok(
        RemoteSyncZstdImportManifestJson::stateless().zstd_pull_manifest_response(
            &repo.repo_name,
            &request.head_snapshot_id,
            boundary_snapshot_ids,
            snapshot_rows,
            object_packs,
            tree_packs,
            blob_locators,
            tree_locators,
        ),
    )
}

#[cfg(feature = "legacy-postgres-runtime")]
fn postgres_zstd_pull_manifest_snapshots(
    client: &mut postgres::Client,
    repo_name: &str,
    head_snapshot_id: &str,
    have_snapshot_ids: &BTreeSet<String>,
) -> Result<(Vec<SnapshotRow>, Vec<String>), NativeRepositoryError> {
    let mut pending = vec![head_snapshot_id.to_string()];
    let mut queued = BTreeSet::from([head_snapshot_id.to_string()]);
    let mut boundaries = BTreeSet::new();
    let mut snapshots = BTreeMap::<String, SnapshotRow>::new();
    while let Some(snapshot_id) = pending.pop() {
        if have_snapshot_ids.contains(&snapshot_id) {
            boundaries.insert(snapshot_id);
            continue;
        }
        let snapshot = select_snapshot_row(client, repo_name, &snapshot_id)?.ok_or_else(|| {
            NativeRepositoryError::not_found(format!(
                "Unknown snapshot {snapshot_id} for repository {repo_name}"
            ))
        })?;
        if let Some(parent) = &snapshot.parent_snapshot_id {
            if queued.insert(parent.clone()) {
                if queued.len() > 100_000 {
                    return Err(NativeRepositoryError::bad_request(
                        "Zstd pull manifest ancestry exceeds 100000 snapshots",
                    ));
                }
                pending.push(parent.clone());
            }
        }
        snapshots.insert(snapshot_id, snapshot);
    }

    let snapshot_ids = snapshots.keys().cloned().collect::<BTreeSet<_>>();
    let mut children = BTreeMap::<String, BTreeSet<String>>::new();
    let mut unresolved_parent_counts = BTreeMap::<String, usize>::new();
    for (snapshot_id, snapshot) in &snapshots {
        let unresolved = snapshot
            .parent_snapshot_id
            .as_ref()
            .is_some_and(|parent| snapshot_ids.contains(parent));
        if unresolved {
            children
                .entry(
                    snapshot
                        .parent_snapshot_id
                        .clone()
                        .expect("unresolved parent must exist"),
                )
                .or_default()
                .insert(snapshot_id.clone());
        }
        unresolved_parent_counts.insert(snapshot_id.clone(), usize::from(unresolved));
    }
    let mut ready = unresolved_parent_counts
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(snapshot_id, _)| snapshot_id.clone())
        .collect::<BTreeSet<_>>();
    let mut ordered = Vec::with_capacity(snapshots.len());
    while let Some(snapshot_id) = ready.pop_first() {
        ordered.push(
            snapshots
                .get(&snapshot_id)
                .expect("ordered Snapshot must remain available")
                .clone(),
        );
        if let Some(child_ids) = children.get(&snapshot_id) {
            for child_id in child_ids {
                let count = unresolved_parent_counts
                    .get_mut(child_id)
                    .expect("child Snapshot count must exist");
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.insert(child_id.clone());
                }
            }
        }
    }
    if ordered.len() != snapshots.len() {
        return Err(NativeRepositoryError::internal(
            "Zstd pull manifest Snapshot ancestry contains a cycle",
        ));
    }
    Ok((ordered, boundaries.into_iter().collect()))
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn get_zstd_import_manifest_json(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    ensure_zstd_only_repository_flow_allowed(
        client,
        repo_name,
        &repo,
        ZstdOnlyRepositoryFlow::ZstdImportManifest,
    )?;
    let snapshot_id = normalize_required_text(snapshot_id, "snapshot_id")?;
    let snapshot = match select_snapshot_row(client, repo_name, &snapshot_id)? {
        Some(snapshot) => snapshot,
        None => {
            if let Some(existing_repo_name) = client
                .query_opt(
                    "select repo_name from snapshots where snapshot_id = $1",
                    &[&snapshot_id],
                )
                .map_err(db_internal)?
                .map(|row| row.get::<_, String>("repo_name"))
            {
                return Err(NativeRepositoryError::conflict(format!(
                    "Snapshot {snapshot_id} belongs to repository {existing_repo_name}, not {repo_name}"
                )));
            }
            return Err(NativeRepositoryError::not_found(format!(
                "Unknown snapshot {snapshot_id} for repository {repo_name}"
            )));
        }
    };
    let (root_tree_pack_path, root_tree_pack_format) = tree_pack_locator_for_id(
        client,
        paths,
        &snapshot.repo_name,
        &snapshot.repo_id,
        &snapshot.root_tree_pack_id,
    )?;
    if root_tree_pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
        return Err(NativeRepositoryError::bad_request(format!(
            "Tree pack {} has unsupported pack_format {:?}",
            snapshot.root_tree_pack_id, root_tree_pack_format
        )));
    }
    let root_payload = read_tree_pack_tree_by_ordinal_with_format(
        path_to_string(&root_tree_pack_path)?.as_str(),
        snapshot.root_entry_ordinal,
        &root_tree_pack_format,
    )
    .map_err(NativeRepositoryError::bad_request)?;
    let root_tree_id = root_payload
        .get("tree_id")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| NativeRepositoryError::bad_request("root tree payload is missing tree_id"))?
        .to_string();
    let root_rows = root_payload
        .get("rows")
        .cloned()
        .unwrap_or_else(|| JsonValue::Array(Vec::new()));
    let closure = walk_zstd_import_manifest_tree_closure(
        client,
        paths,
        &repo.repo_name,
        &repo.repo_id,
        &root_tree_id,
        root_rows,
    )?;
    let tree_locator_records =
        zstd_import_manifest_tree_locator_records(client, &closure.tree_ids)?;
    let tree_pack_ids = tree_locator_records
        .iter()
        .map(|record| record.tree_pack_id.clone())
        .collect::<BTreeSet<_>>();
    let (tree_packs, tree_pack_indexes) =
        zstd_import_manifest_tree_pack_rows(client, paths, &repo, &tree_pack_ids)?;
    let tree_locators =
        zstd_import_manifest_tree_locator_rows(&tree_locator_records, &tree_pack_indexes)?;

    let blob_locator_records = zstd_import_manifest_blob_locator_records(
        client,
        &repo.repo_name,
        &repo.repo_id,
        &closure.blob_ids,
    )?;
    let object_pack_ids = blob_locator_records
        .iter()
        .map(|record| record.pack_id.clone())
        .collect::<BTreeSet<_>>();
    let (object_packs, object_pack_indexes) =
        zstd_import_manifest_object_pack_rows(client, paths, &repo, &object_pack_ids)?;
    let blob_locators =
        zstd_import_manifest_blob_locator_rows(&blob_locator_records, &object_pack_indexes)?;

    let snapshot_row = zstd_import_manifest_snapshot_row(&snapshot);
    Ok(
        RemoteSyncZstdImportManifestJson::stateless().zstd_import_manifest_response(
            &repo.repo_name,
            &snapshot_id,
            snapshot_row,
            object_packs,
            tree_packs,
            blob_locators,
            tree_locators,
        ),
    )
}

#[cfg(feature = "legacy-postgres-runtime")]
fn walk_zstd_import_manifest_tree_closure(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    root_tree_id: &str,
    root_rows: JsonValue,
) -> Result<ZstdImportManifestTreeClosure, NativeRepositoryError> {
    let mut cached_rows = BTreeMap::new();
    cached_rows.insert(root_tree_id.to_string(), root_rows);
    walk_zstd_import_manifest_tree_closure_from_roots(
        client,
        paths,
        repo_name,
        repo_id,
        cached_rows,
        BTreeSet::from([root_tree_id.to_string()]),
        vec![root_tree_id.to_string()],
    )
}

#[cfg(feature = "legacy-postgres-runtime")]
fn walk_zstd_import_manifest_tree_closure_for_snapshots(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo: &RepositoryRow,
    snapshots: &[SnapshotRow],
) -> Result<ZstdImportManifestTreeClosure, NativeRepositoryError> {
    let mut cached_rows = BTreeMap::new();
    let mut tree_ids = BTreeSet::new();
    let mut stack = Vec::new();
    for snapshot in snapshots {
        let (root_tree_pack_path, root_tree_pack_format) = tree_pack_locator_for_id(
            client,
            paths,
            &snapshot.repo_name,
            &snapshot.repo_id,
            &snapshot.root_tree_pack_id,
        )?;
        if root_tree_pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {} has unsupported pack_format {:?}",
                snapshot.root_tree_pack_id, root_tree_pack_format
            )));
        }
        let root_payload = read_tree_pack_tree_by_ordinal_with_format(
            path_to_string(&root_tree_pack_path)?.as_str(),
            snapshot.root_entry_ordinal,
            &root_tree_pack_format,
        )
        .map_err(NativeRepositoryError::bad_request)?;
        let root_tree_id = root_payload
            .get("tree_id")
            .and_then(JsonValue::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                NativeRepositoryError::bad_request("root tree payload is missing tree_id")
            })?
            .to_string();
        let root_rows = root_payload
            .get("rows")
            .cloned()
            .unwrap_or_else(|| JsonValue::Array(Vec::new()));
        cached_rows.entry(root_tree_id.clone()).or_insert(root_rows);
        if tree_ids.insert(root_tree_id.clone()) {
            stack.push(root_tree_id);
        }
    }
    walk_zstd_import_manifest_tree_closure_from_roots(
        client,
        paths,
        &repo.repo_name,
        &repo.repo_id,
        cached_rows,
        tree_ids,
        stack,
    )
}

#[cfg(feature = "legacy-postgres-runtime")]
fn walk_zstd_import_manifest_tree_closure_from_roots(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    mut cached_rows: BTreeMap<String, JsonValue>,
    mut tree_ids: BTreeSet<String>,
    mut stack: Vec<String>,
) -> Result<ZstdImportManifestTreeClosure, NativeRepositoryError> {
    let mut blob_ids = BTreeSet::new();
    let mut expanded_tree_pack_paths = BTreeSet::new();
    while let Some(tree_id) = stack.pop() {
        let (tree_pack_path, tree_pack_format) =
            tree_pack_locator_for_tree_id(client, paths, repo_name, repo_id, &tree_id)?;
        if tree_pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree {tree_id} has unsupported tree pack format {tree_pack_format:?}"
            )));
        }
        if expanded_tree_pack_paths.insert(tree_pack_path.clone()) {
            let index = read_tree_pack_index_with_format(
                path_to_string(&tree_pack_path)?.as_str(),
                &tree_pack_format,
            )
            .map_err(NativeRepositoryError::bad_request)?;
            let members = index
                .get("trees")
                .and_then(JsonValue::as_array)
                .ok_or_else(|| {
                    NativeRepositoryError::bad_request(format!(
                        "Tree pack for {tree_id} is missing its trees index"
                    ))
                })?;
            for member in members {
                let member = member.as_object().ok_or_else(|| {
                    NativeRepositoryError::bad_request(format!(
                        "Tree pack for {tree_id} contains a non-object tree index member"
                    ))
                })?;
                let member_tree_id = required_json_text(member, "tree_id")
                    .map_err(NativeRepositoryError::bad_request)?;
                if tree_ids.insert(member_tree_id.clone()) {
                    stack.push(member_tree_id);
                }
            }
        }
        let rows = if let Some(existing) = cached_rows.get(&tree_id) {
            existing.clone()
        } else {
            let rows = read_tree_pack_tree_with_format(
                path_to_string(&tree_pack_path)?.as_str(),
                &tree_id,
                &tree_pack_format,
            )
            .map_err(NativeRepositoryError::bad_request)?;
            cached_rows.insert(tree_id.clone(), rows.clone());
            rows
        };
        let rows = rows.as_array().ok_or_else(|| {
            NativeRepositoryError::bad_request("tree rows payload must be an array")
        })?;
        for row in rows {
            let object = row
                .as_object()
                .ok_or_else(|| NativeRepositoryError::bad_request("tree row must be an object"))?;
            let entry_type = required_json_text(object, "entry_type")
                .map_err(NativeRepositoryError::bad_request)?;
            let target_id = required_json_text(object, "target_id")
                .map_err(NativeRepositoryError::bad_request)?;
            match entry_type.as_str() {
                "tree" => {
                    if tree_ids.insert(target_id.clone()) {
                        stack.push(target_id);
                    }
                }
                "blob" => {
                    blob_ids.insert(target_id);
                }
                _ => {}
            }
        }
    }
    Ok(ZstdImportManifestTreeClosure { tree_ids, blob_ids })
}

#[cfg(feature = "legacy-postgres-runtime")]
fn zstd_import_manifest_object_pack_rows(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo: &RepositoryRow,
    pack_ids: &BTreeSet<String>,
) -> Result<(Vec<JsonValue>, BTreeMap<String, JsonValue>), NativeRepositoryError> {
    let mut rows = Vec::new();
    let mut indexes = BTreeMap::new();
    for pack_id in pack_ids {
        validate_pack_id_segment(pack_id)?;
        let row = client
            .query_opt(
                "select repo_name, repo_id, status, member_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at::text as created_at_text from packs where pack_id = $1",
                &[&pack_id],
            )
            .map_err(db_internal)?
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Blob locator references missing object pack {pack_id}"
                ))
            })?;
        let existing_repo_name: String = row.get("repo_name");
        let existing_repo_id: String = row.get("repo_id");
        if (existing_repo_name != repo.repo_name || existing_repo_id != repo.repo_id)
            && !object_pack_has_repository_blob_locator(
                client,
                pack_id,
                &repo.repo_name,
                &repo.repo_id,
            )?
        {
            return Err(NativeRepositoryError::conflict(format!(
                "Object pack {pack_id} belongs to repository {existing_repo_name}, not {}",
                repo.repo_name
            )));
        }
        let pack_format: String = row.get("pack_format");
        if pack_format != PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {pack_id} has unsupported pack_format {pack_format:?}"
            )));
        }
        let status: String = row.get("status");
        if status != "ready" {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {pack_id} is not ready for zstd manifest export"
            )));
        }
        let pack_path = zstd_pack_row_path(paths, &row, pack_id, false)?;
        let index = read_pack_index_with_format(path_to_string(&pack_path)?.as_str(), &pack_format)
            .map_err(NativeRepositoryError::bad_request)?;
        let metadata = zstd_pack_metadata_from_row(&row, false);
        validate_zstd_pack_index_metadata(&index, &metadata, pack_id, false)?;
        let index_checksum = read_pack_index_checksum_with_format(
            path_to_string(&pack_path)?.as_str(),
            &pack_format,
        )
        .map_err(NativeRepositoryError::bad_request)?;
        if let Some(expected) = row
            .get::<_, Option<String>>("pack_index_checksum")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            if expected != index_checksum {
                return Err(NativeRepositoryError::bad_request(format!(
                    "zstd pack {pack_id} index checksum mismatch"
                )));
            }
        }
        let index_entry_name = row
            .get::<_, Option<String>>("pack_index_entry_name")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                index
                    .get("index_entry_name")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "zstd-chunked-object-index".to_string());
        rows.push(json!({
            "pack_id": pack_id,
            "pack_format": pack_format,
            "member_count": row.get::<_, i32>("member_count"),
            "total_bytes": row.get::<_, i64>("total_bytes"),
            "pack_index_entry_name": index_entry_name,
            "pack_index_checksum": index_checksum,
            "created_at": row.get::<_, String>("created_at_text"),
        }));
        indexes.insert(pack_id.clone(), index);
    }
    Ok((rows, indexes))
}

#[cfg(feature = "legacy-postgres-runtime")]
fn zstd_import_manifest_tree_pack_rows(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    _repo: &RepositoryRow,
    pack_ids: &BTreeSet<String>,
) -> Result<(Vec<JsonValue>, BTreeMap<String, JsonValue>), NativeRepositoryError> {
    let mut rows = Vec::new();
    let mut indexes = BTreeMap::new();
    for pack_id in pack_ids {
        validate_pack_id_segment(pack_id)?;
        let row = client
            .query_opt(
                "select repo_name, repo_id, status, tree_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at::text as created_at_text from tree_packs where pack_id = $1",
                &[&pack_id],
            )
            .map_err(db_internal)?
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Tree locator references missing tree pack {pack_id}"
                ))
            })?;
        let pack_format: String = row.get("pack_format");
        if pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} has unsupported pack_format {pack_format:?}"
            )));
        }
        let status: String = row.get("status");
        if status != "ready" {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} is not ready for zstd manifest export"
            )));
        }
        let pack_path = zstd_pack_row_path(paths, &row, pack_id, true)?;
        let index =
            read_tree_pack_index_with_format(path_to_string(&pack_path)?.as_str(), &pack_format)
                .map_err(NativeRepositoryError::bad_request)?;
        let metadata = zstd_pack_metadata_from_row(&row, true);
        validate_zstd_pack_index_metadata(&index, &metadata, pack_id, true)?;
        let index_checksum = read_tree_pack_index_checksum_with_format(
            path_to_string(&pack_path)?.as_str(),
            &pack_format,
        )
        .map_err(NativeRepositoryError::bad_request)?;
        if let Some(expected) = row
            .get::<_, Option<String>>("pack_index_checksum")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        {
            if expected != index_checksum {
                return Err(NativeRepositoryError::bad_request(format!(
                    "zstd pack {pack_id} index checksum mismatch"
                )));
            }
        }
        let index_entry_name = row
            .get::<_, Option<String>>("pack_index_entry_name")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .or_else(|| {
                index
                    .get("index_entry_name")
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "zstd-chunked-tree-index".to_string());
        rows.push(json!({
            "pack_id": pack_id,
            "pack_format": pack_format,
            "tree_count": row.get::<_, i32>("tree_count"),
            "total_bytes": row.get::<_, i64>("total_bytes"),
            "pack_index_entry_name": index_entry_name,
            "pack_index_checksum": index_checksum,
            "created_at": row.get::<_, String>("created_at_text"),
        }));
        indexes.insert(pack_id.clone(), index);
    }
    Ok((rows, indexes))
}

#[cfg(feature = "legacy-postgres-runtime")]
fn zstd_import_manifest_blob_locator_records(
    client: &mut postgres::Client,
    repo_name: &str,
    repo_id: &str,
    blob_ids: &BTreeSet<String>,
) -> Result<Vec<ZstdImportManifestBlobLocatorRecord>, NativeRepositoryError> {
    let mut rows = Vec::new();
    for blob_id in blob_ids {
        rows.push(zstd_import_manifest_blob_locator_record_from_locator(
            require_blob_locator_for_repo(client, repo_name, repo_id, blob_id)?,
        ));
    }
    Ok(rows)
}

#[cfg(feature = "legacy-postgres-runtime")]
fn zstd_import_manifest_blob_locator_rows(
    records: &[ZstdImportManifestBlobLocatorRecord],
    object_pack_indexes: &BTreeMap<String, JsonValue>,
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    let mut rows = Vec::new();
    for record in records {
        let entry =
            object_pack_entry_for_blob_id(object_pack_indexes, &record.pack_id, &record.blob_id)?;
        if entry.checksum != record.sha256 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {} checksum mismatch for blob {}",
                record.pack_id, record.blob_id
            )));
        }
        let entry_type = record
            .pack_entry_type
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Blob {} is missing pack entry type metadata",
                    record.blob_id
                ))
            })?;
        if entry.entry_type != entry_type {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {} entry_type mismatch for blob {}",
                record.pack_id, record.blob_id
            )));
        }
        if record.pack_base_blob_id != entry.base_blob_id {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {} base blob mismatch for blob {}",
                record.pack_id, record.blob_id
            )));
        }
        let chain_depth = record.pack_chain_depth.ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "Blob {} is missing pack chain depth metadata",
                record.blob_id
            ))
        })?;
        if chain_depth != entry.chain_depth as i64 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {} chain depth mismatch for blob {}",
                record.pack_id, record.blob_id
            )));
        }
        rows.push(json!({
            "blob_id": record.blob_id,
            "sha256": record.sha256,
            "size_bytes": record.size_bytes,
            "pack_id": record.pack_id,
            "pack_entry_name": entry.entry_name,
            "pack_entry_type": entry_type,
            "pack_base_blob_id": entry.base_blob_id,
            "pack_chain_depth": chain_depth,
            "created_at": record.created_at,
        }));
    }
    Ok(rows)
}

#[cfg(feature = "legacy-postgres-runtime")]
fn zstd_import_manifest_tree_locator_records(
    client: &mut postgres::Client,
    tree_ids: &BTreeSet<String>,
) -> Result<Vec<ZstdImportManifestTreeLocatorRecord>, NativeRepositoryError> {
    let mut rows = Vec::new();
    for tree_id in tree_ids {
        let row = client
            .query_opt(
                "select tree_id, entry_count, tree_pack_id, tree_pack_checksum, created_at::text as created_at_text from trees where tree_id = $1",
                &[&tree_id],
            )
            .map_err(db_internal)?
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Snapshot closure references missing tree {tree_id}"
                ))
            })?;
        let tree_pack_id = row
            .get::<_, Option<String>>("tree_pack_id")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Tree {tree_id} is missing zstd tree pack metadata"
                ))
            })?;
        let tree_pack_checksum = row
            .get::<_, Option<String>>("tree_pack_checksum")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                NativeRepositoryError::bad_request(format!(
                    "Tree {tree_id} is missing zstd tree pack checksum"
                ))
            })?;
        rows.push(ZstdImportManifestTreeLocatorRecord {
            tree_id: row.get("tree_id"),
            entry_count: row.get::<_, i32>("entry_count"),
            tree_pack_id,
            tree_pack_checksum,
            created_at: row.get("created_at_text"),
        });
    }
    Ok(rows)
}

#[cfg(feature = "legacy-postgres-runtime")]
fn zstd_import_manifest_tree_locator_rows(
    records: &[ZstdImportManifestTreeLocatorRecord],
    tree_pack_indexes: &BTreeMap<String, JsonValue>,
) -> Result<Vec<JsonValue>, NativeRepositoryError> {
    let mut rows = Vec::new();
    for record in records {
        let entry =
            tree_pack_entry_for_tree_id(tree_pack_indexes, &record.tree_pack_id, &record.tree_id)?;
        if entry.entry_count != record.entry_count as usize {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {} entry_count mismatch for tree {}",
                record.tree_pack_id, record.tree_id
            )));
        }
        if entry.checksum != record.tree_pack_checksum {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {} checksum mismatch for tree {}",
                record.tree_pack_id, record.tree_id
            )));
        }
        rows.push(json!({
            "tree_id": record.tree_id,
            "entry_count": record.entry_count,
            "tree_pack_id": record.tree_pack_id,
            "tree_pack_checksum": record.tree_pack_checksum,
            "created_at": record.created_at,
        }));
    }
    Ok(rows)
}

#[cfg(feature = "legacy-postgres-runtime")]
fn zstd_import_manifest_snapshot_row(snapshot: &SnapshotRow) -> JsonValue {
    json!({
        "snapshot_id": snapshot.snapshot_id,
        "parent_snapshot_id": snapshot.parent_snapshot_id,
        "root_tree_pack_id": snapshot.root_tree_pack_id,
        "root_entry_ordinal": snapshot.root_entry_ordinal,
        "manifest_hash": snapshot.manifest_hash,
        "message": snapshot.message,
        "line_name": snapshot.line_name,
        "snapshot_kind": "line",
        "file_count": snapshot.file_count,
        "total_bytes": snapshot.total_bytes,
        "created_at": snapshot.created_at,
    })
}

pub(in crate::foundation::native_repositories) fn binary_zstd_import_manifest_pack_row(
    value: JsonValue,
    tree_pack: bool,
) -> Result<JsonValue, NativeRepositoryError> {
    let object = value.as_object().ok_or_else(|| {
        NativeRepositoryError::internal("Binary DB zstd pack metadata must be an object")
    })?;
    let pack_id = binary_json_text(&value, "pack_id").ok_or_else(|| {
        NativeRepositoryError::internal("Binary DB zstd pack metadata is missing pack_id")
    })?;
    let count_field = if tree_pack {
        "tree_count"
    } else {
        "member_count"
    };
    let pack_format = binary_json_text(&value, "pack_format").ok_or_else(|| {
        NativeRepositoryError::internal(format!(
            "Binary DB zstd pack {pack_id} metadata is missing pack_format"
        ))
    })?;
    let expected_format = if tree_pack {
        REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1
    } else {
        REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1
    };
    if pack_format != expected_format {
        return Err(NativeRepositoryError::internal(format!(
            "Binary DB zstd pack {pack_id} metadata has unsupported pack_format {pack_format}"
        )));
    }
    let count = required_i64_field(object, count_field)?;
    let total_bytes = required_i64_field(object, "total_bytes")?;
    let index_entry_name = required_json_text(object, "pack_index_entry_name")
        .map_err(NativeRepositoryError::internal)?;
    let index_checksum = required_json_text(object, "pack_index_checksum")
        .map_err(NativeRepositoryError::internal)?;
    let mut row = JsonMap::new();
    row.insert("pack_id".to_string(), JsonValue::String(pack_id));
    row.insert("pack_format".to_string(), JsonValue::String(pack_format));
    row.insert(count_field.to_string(), json!(count));
    row.insert("total_bytes".to_string(), json!(total_bytes));
    row.insert(
        "pack_index_entry_name".to_string(),
        JsonValue::String(index_entry_name),
    );
    row.insert(
        "pack_index_checksum".to_string(),
        JsonValue::String(index_checksum),
    );
    row.insert("created_at".to_string(), binary_created_at_value(&value));
    Ok(JsonValue::Object(row))
}

pub(in crate::foundation::native_repositories) fn binary_zstd_import_manifest_blob_locator_row(
    value: JsonValue,
) -> Result<JsonValue, NativeRepositoryError> {
    let object = value.as_object().ok_or_else(|| {
        NativeRepositoryError::internal("Binary DB blob locator metadata must be an object")
    })?;
    let blob_id = binary_json_text(&value, "blob_id").ok_or_else(|| {
        NativeRepositoryError::internal("Binary DB blob locator metadata is missing blob_id")
    })?;
    let sha256 = required_json_text(object, "sha256").map_err(NativeRepositoryError::internal)?;
    let size_bytes = required_i64_field(object, "size_bytes")?;
    let pack_id = required_json_text(object, "pack_id").map_err(NativeRepositoryError::internal)?;
    let pack_entry_name =
        required_json_text(object, "pack_entry_name").map_err(NativeRepositoryError::internal)?;
    let pack_entry_type =
        required_json_text(object, "pack_entry_type").map_err(NativeRepositoryError::internal)?;
    let pack_chain_depth = required_i64_field(object, "pack_chain_depth")?;
    Ok(json!({
        "blob_id": blob_id,
        "sha256": sha256,
        "size_bytes": size_bytes,
        "pack_id": pack_id,
        "pack_entry_name": pack_entry_name,
        "pack_entry_type": pack_entry_type,
        "pack_base_blob_id": value.get("pack_base_blob_id").cloned().unwrap_or(JsonValue::Null),
        "pack_chain_depth": pack_chain_depth,
        "created_at": binary_created_at_value(&value),
    }))
}

pub(in crate::foundation::native_repositories) fn binary_zstd_import_manifest_tree_locator_row(
    value: JsonValue,
) -> Result<JsonValue, NativeRepositoryError> {
    let object = value.as_object().ok_or_else(|| {
        NativeRepositoryError::internal("Binary DB tree locator metadata must be an object")
    })?;
    let tree_id = binary_json_text(&value, "tree_id").ok_or_else(|| {
        NativeRepositoryError::internal("Binary DB tree locator metadata is missing tree_id")
    })?;
    let entry_count = required_i64_field(object, "entry_count")?;
    let tree_pack_id =
        required_json_text(object, "tree_pack_id").map_err(NativeRepositoryError::internal)?;
    let tree_pack_checksum = required_json_text(object, "tree_pack_checksum")
        .map_err(NativeRepositoryError::internal)?;
    Ok(json!({
        "tree_id": tree_id,
        "entry_count": entry_count,
        "tree_pack_id": tree_pack_id,
        "tree_pack_checksum": tree_pack_checksum,
        "created_at": binary_created_at_value(&value),
    }))
}

pub(in crate::foundation::native_repositories) fn binary_zstd_import_manifest_snapshot_row(
    value: &JsonValue,
) -> Result<JsonValue, NativeRepositoryError> {
    let snapshot_id = binary_snapshot_id(value).ok_or_else(|| {
        NativeRepositoryError::internal("Binary DB zstd snapshot payload is missing snapshot_id")
    })?;
    let parent_snapshot_id = value
        .get("parent_snapshot_id")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let parent_snapshot_ids = value
        .get("parent_snapshot_ids")
        .cloned()
        .unwrap_or_else(|| match parent_snapshot_id.as_str() {
            Some(parent) => json!([parent]),
            None => json!([]),
        });
    Ok(json!({
        "snapshot_id": snapshot_id,
        "parent_snapshot_ids": parent_snapshot_ids,
        "primary_parent_snapshot_id": value
            .get("primary_parent_snapshot_id")
            .cloned()
            .unwrap_or_else(|| parent_snapshot_id.clone()),
        "parent_snapshot_id": parent_snapshot_id,
        "root_tree_pack_id": binary_json_text(value, "root_tree_pack_id").unwrap_or_default(),
        "root_entry_ordinal": value.get("root_entry_ordinal").and_then(JsonValue::as_i64).unwrap_or(0),
        "manifest_hash": binary_json_text(value, "manifest_hash").unwrap_or_default(),
        "message": value.get("message").cloned().unwrap_or(JsonValue::Null),
        "line_name": value.get("line_name").cloned().unwrap_or_else(|| json!(default_main_line())),
        "snapshot_kind": binary_json_text(value, "snapshot_kind").unwrap_or_else(|| "line".to_string()),
        "file_count": value.get("file_count").and_then(JsonValue::as_i64).unwrap_or(0),
        "total_bytes": value.get("total_bytes").and_then(JsonValue::as_i64).unwrap_or(0),
        "created_at": binary_created_at_value(value),
    }))
}

#[cfg(feature = "legacy-postgres-runtime")]
fn zstd_import_manifest_blob_locator_record_from_locator(
    locator: BlobLocatorRow,
) -> ZstdImportManifestBlobLocatorRecord {
    ZstdImportManifestBlobLocatorRecord {
        blob_id: locator.blob_id,
        sha256: locator.sha256,
        size_bytes: locator.size_bytes,
        pack_id: locator.pack_id,
        pack_entry_type: locator.pack_entry_type,
        pack_base_blob_id: locator.pack_base_blob_id,
        pack_chain_depth: locator.pack_chain_depth,
        created_at: locator.created_at,
    }
}

#[cfg(feature = "legacy-postgres-runtime")]
#[derive(Debug, Clone)]
struct ZstdImportManifestTreeClosure {
    tree_ids: BTreeSet<String>,
    blob_ids: BTreeSet<String>,
}

#[cfg(feature = "legacy-postgres-runtime")]
#[derive(Debug, Clone)]
struct ZstdImportManifestBlobLocatorRecord {
    blob_id: String,
    sha256: String,
    size_bytes: i64,
    pack_id: String,
    pack_entry_type: Option<String>,
    pack_base_blob_id: Option<String>,
    pack_chain_depth: Option<i64>,
    created_at: String,
}

#[cfg(feature = "legacy-postgres-runtime")]
#[derive(Debug, Clone)]
struct ZstdImportManifestTreeLocatorRecord {
    tree_id: String,
    entry_count: i32,
    tree_pack_id: String,
    tree_pack_checksum: String,
    created_at: String,
}
