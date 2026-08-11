use super::*;

pub(super) struct ExportResult {
    pub(super) export_path: PathBuf,
    pub(super) manifest_path: PathBuf,
    pub(super) manifest_sha256: String,
    pub(super) manifest: JsonValue,
}

pub(super) fn export_bundle(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    repo_id: &str,
) -> Result<ExportResult, NativeRepositoryError> {
    let export_root = export_root(&service.paths)?;
    let stamp = now_rfc3339()
        .replace(':', "")
        .replace("+00:00", "Z")
        .replace("+08:00", "+0800");
    let export_path = export_root.join(format!("{stamp}__{}__{repo_id}", safe_name(repo_name)));
    fs::create_dir_all(&export_path).map_err(|exc| {
        NativeRepositoryError::internal(format!(
            "failed to create retire export directory `{}`: {exc}",
            export_path.display()
        ))
    })?;

    let mut files = Vec::<WrittenFile>::new();
    let repository = service.with_read(|client| get_repository_json(client, repo_name))?;
    let lines = service.with_read(|client| list_lines_json(client, repo_name))?;
    let refs = line_refs_json(&lines);
    let storage = repository_storage_summary(service, repo_name, repo_id)?;
    files.push(write_json_file(
        &export_path.join("repository.json"),
        &repository,
    )?);
    files.push(write_json_file(
        &export_path.join("content/lines.json"),
        &lines,
    )?);
    files.push(write_json_file(
        &export_path.join("content/refs.json"),
        &refs,
    )?);
    files.push(write_json_file(
        &export_path.join("content/storage.json"),
        &storage,
    )?);

    let (content_tables, snapshot_ids) = service.with_read(|client| {
        Ok::<_, NativeRepositoryError>((
            repo_content_rows(client, repo_name, repo_id)?,
            text_column(
                client,
                "select snapshot_id from snapshots where repo_id = $1 or (repo_id is null and repo_name = $2) order by created_at asc, snapshot_id asc",
                &[&repo_id, &repo_name],
                "snapshot_id",
            )?,
        ))
    })?;
    for (table_name, rows) in &content_tables {
        files.push(write_json_file(
            &export_path.join(format!("content/tables/{table_name}.json")),
            &JsonValue::Array(rows.clone()),
        )?);
    }

    let (control_exports, control_blob_ids) =
        export_control_tables(service, repo_name, repo_id, &export_path, &mut files)?;
    export_control_blobs(
        service,
        repo_name,
        repo_id,
        &control_blob_ids,
        &export_path,
        &mut files,
    )?;

    let mut snapshot_manifest = Vec::<JsonValue>::new();
    for snapshot_id in snapshot_ids {
        let bundle = service.with_read(|client| {
            export_snapshot_json(
                client,
                &service.paths,
                repo_name,
                &snapshot_id,
                SnapshotExportQuery {
                    include_content: false,
                    path: None,
                },
            )
        })?;
        let path = export_path.join(format!("content/snapshots/{snapshot_id}.json"));
        let written = write_json_file(&path, &bundle)?;
        snapshot_manifest.push(json!({
            "snapshot_id": snapshot_id,
            "path": relative_path(&export_path, &path)?,
            "sha256": written.sha256,
            "size_bytes": written.size_bytes
        }));
        files.push(written);
    }
    files.push(write_json_file(
        &export_path.join("content/snapshot_manifest.json"),
        &JsonValue::Array(snapshot_manifest),
    )?);

    let mut manifest_files = files
        .iter()
        .map(|item| {
            Ok(json!({
                "path": relative_path(&export_path, &item.path)?,
                "sha256": item.sha256,
                "size_bytes": item.size_bytes
            }))
        })
        .collect::<Result<Vec<_>, NativeRepositoryError>>()?;
    manifest_files.sort_by_key(|value| {
        value
            .get("path")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_string()
    });
    let manifest = json!({
        "repo_name": repo_name,
        "repo_id": repo_id,
        "generated_at": now_rfc3339(),
        "export_root": path_string(&export_root),
        "export_path": path_string(&export_path),
        "snapshot_count": manifest_files.iter().filter(|item| item.get("path").and_then(JsonValue::as_str).unwrap_or_default().starts_with("content/snapshots/")).count(),
        "control_blob_count": control_blob_ids.len(),
        "content_table_counts": table_counts(&content_tables),
        "control_table_counts": table_counts(&control_exports),
        "files": manifest_files
    });
    let manifest_path = export_path.join("manifest.json");
    let manifest_written = write_json_file(&manifest_path, &manifest)?;
    fs::write(
        export_path.join("manifest.sha256"),
        format!("{}  manifest.json\n", manifest_written.sha256),
    )
    .map_err(|exc| {
        NativeRepositoryError::internal(format!("failed to write manifest checksum: {exc}"))
    })?;
    Ok(ExportResult {
        export_path,
        manifest_path,
        manifest_sha256: manifest_written.sha256,
        manifest,
    })
}

pub(super) fn export_root(paths: &ServerRuntimePaths) -> Result<PathBuf, NativeRepositoryError> {
    let raw = env::var(RETIRE_EXPORT_ROOT_ENV).map_err(|_| {
        NativeRepositoryError::bad_request(format!(
            "{RETIRE_EXPORT_ROOT_ENV} is required for repository retirement"
        ))
    })?;
    let root = PathBuf::from(raw.trim());
    if !root.is_absolute() {
        return Err(NativeRepositoryError::bad_request(format!(
            "{RETIRE_EXPORT_ROOT_ENV} must be an absolute path"
        )));
    }
    let root = root.canonicalize().map_err(|exc| {
        NativeRepositoryError::bad_request(format!(
            "{RETIRE_EXPORT_ROOT_ENV} path does not exist: {} ({exc})",
            root.display()
        ))
    })?;
    if !root.is_dir() {
        return Err(NativeRepositoryError::bad_request(format!(
            "{RETIRE_EXPORT_ROOT_ENV} must point to a directory: {}",
            root.display()
        )));
    }
    let runtime_root = paths
        .root
        .canonicalize()
        .unwrap_or_else(|_| paths.root.clone());
    if root.starts_with(&runtime_root) {
        return Err(NativeRepositoryError::bad_request(format!(
            "{RETIRE_EXPORT_ROOT_ENV} must not be inside the active server runtime root {}",
            runtime_root.display()
        )));
    }
    let probe = root.join(format!(
        ".ait-retire-write-check-{}",
        new_identifier("TMP", "probe")
    ));
    fs::write(&probe, b"ok\n").map_err(|exc| {
        NativeRepositoryError::bad_request(format!(
            "{RETIRE_EXPORT_ROOT_ENV} is not writable: {} ({exc})",
            root.display()
        ))
    })?;
    let _ = fs::remove_file(&probe);
    Ok(root)
}

pub(super) fn safe_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches(['.', '_']);
    if trimmed.is_empty() {
        "repo".to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn line_refs_json(lines: &JsonValue) -> JsonValue {
    let mut refs = JsonMap::new();
    for line in lines.as_array().into_iter().flatten() {
        let Some(line_name) = line.get("line_name").and_then(JsonValue::as_str) else {
            continue;
        };
        let head = line
            .get("head_snapshot_id")
            .cloned()
            .unwrap_or(JsonValue::Null);
        refs.insert(line_name.to_string(), head);
    }
    JsonValue::Object(refs)
}

pub(super) fn repository_storage_summary(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    repo_id: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    service.with_read(|client| {
        let row = client
            .query_one(
                "select
                    (select count(*) from snapshots where repo_id = $1 or (repo_id is null and repo_name = $2))::bigint as snapshot_count,
                    (select count(*) from packs where repo_id = $1 or (repo_id is null and repo_name = $2))::bigint as object_pack_count,
                    (select coalesce(sum(total_bytes), 0) from packs where repo_id = $1 or (repo_id is null and repo_name = $2))::bigint as object_pack_bytes,
                    (select count(*) from tree_packs where repo_id = $1 or (repo_id is null and repo_name = $2))::bigint as tree_pack_count,
                    (select coalesce(sum(total_bytes), 0) from tree_packs where repo_id = $1 or (repo_id is null and repo_name = $2))::bigint as tree_pack_bytes,
                    (select count(*) from blob_locators where repo_id = $1 or (repo_id is null and repo_name = $2))::bigint as blob_locator_count",
                &[&repo_id, &repo_name],
            )
            .map_err(db_internal)?;
        Ok(json!({
            "repo_name": repo_name,
            "repo_id": repo_id,
            "snapshot_count": row.get::<_, i64>("snapshot_count"),
            "object_pack_count": row.get::<_, i64>("object_pack_count"),
            "object_pack_bytes": row.get::<_, i64>("object_pack_bytes"),
            "tree_pack_count": row.get::<_, i64>("tree_pack_count"),
            "tree_pack_bytes": row.get::<_, i64>("tree_pack_bytes"),
            "blob_locator_count": row.get::<_, i64>("blob_locator_count")
        }))
    })
}

pub(super) fn repo_content_rows(
    client: &mut pg::Client,
    repo_name: &str,
    repo_id: &str,
) -> Result<BTreeMap<String, Vec<JsonValue>>, NativeRepositoryError> {
    let mut tables = BTreeMap::new();
    tables.insert(
        "repositories".to_string(),
        query_json_object_rows(
            client,
            "select to_jsonb(t)::text as row_json from repositories t where repo_name = $1 order by repo_name asc",
            &[&repo_name],
        )?,
    );
    for (table, order) in [
        ("lines", "line_name asc"),
        ("snapshots", "created_at asc, snapshot_id asc"),
        ("packs", "created_at asc, pack_id asc"),
        ("tree_packs", "created_at asc, pack_id asc"),
        ("blob_locators", "created_at asc, blob_id asc"),
    ] {
        tables.insert(
            table.to_string(),
            query_json_object_rows(
                client,
                format!(
                    "select to_jsonb(t)::text as row_json from {table} t where repo_id = $1 or (repo_id is null and repo_name = $2) order by {order}"
                )
                .as_str(),
                &[&repo_id, &repo_name],
            )?,
        );
    }
    tables.insert(
        "blobs".to_string(),
        query_json_object_rows(
            client,
            "select to_jsonb(b)::text as row_json from blobs b where b.blob_id in (select blob_id from blob_locators where repo_id = $1 or (repo_id is null and repo_name = $2)) or b.pack_id in (select pack_id from packs where repo_id = $1 or (repo_id is null and repo_name = $2)) order by blob_id asc",
            &[&repo_id, &repo_name],
        )?,
    );
    Ok(tables)
}

pub(super) fn export_control_tables(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    repo_id: &str,
    export_path: &Path,
    files: &mut Vec<WrittenFile>,
) -> Result<(BTreeMap<String, Vec<JsonValue>>, Vec<String>), NativeRepositoryError> {
    let (exports, blob_ids) = service.with_control_read(|client| {
        let plan_ids = repo_plan_ids(client, repo_id, repo_name)?;
        let authority_map_ids = repo_authority_map_ids(client, repo_id, repo_name)?;
        let mut exports = BTreeMap::new();
        for table in REPO_SCOPED_CONTROL_TABLES {
            exports.insert(
                (*table).to_string(),
                table_repo_rows(client, table, repo_id, repo_name)?,
            );
        }
        exports.insert(
            "plan_revisions".to_string(),
            rows_by_ids(
                client,
                "plan_revisions",
                "plan_id",
                &plan_ids,
                "plan_id asc, revision_number asc",
            )?,
        );
        exports.insert(
            "authority_nodes".to_string(),
            rows_by_ids(
                client,
                "authority_nodes",
                "authority_map_id",
                &authority_map_ids,
                "authority_map_id asc, sort_index asc, authority_node_id asc",
            )?,
        );
        exports.insert(
            "authority_mutations".to_string(),
            rows_by_ids(
                client,
                "authority_mutations",
                "authority_map_id",
                &authority_map_ids,
                "authority_map_id asc, created_at asc, mutation_id asc",
            )?,
        );
        let blob_ids = control_blob_ids(&exports);
        Ok((exports, blob_ids))
    })?;
    for (table_name, rows) in &exports {
        files.push(write_json_file(
            &export_path.join(format!("control/tables/{table_name}.json")),
            &JsonValue::Array(rows.clone()),
        )?);
    }
    Ok((exports, blob_ids))
}

pub(super) fn export_control_blobs(
    service: &PostgresNativeRepositoryService,
    repo_name: &str,
    repo_id: &str,
    blob_ids: &[String],
    export_path: &Path,
    files: &mut Vec<WrittenFile>,
) -> Result<(), NativeRepositoryError> {
    let mut manifest = Vec::<JsonValue>::new();
    for blob_id in blob_ids {
        let bytes = service.with_read(|client| {
            blob_bytes_for_global_blob_id(client, &service.paths, repo_name, repo_id, blob_id)
        })?;
        let blob_path = export_path.join(format!("control/blobs/{blob_id}.blob"));
        let written = write_bytes_file(&blob_path, &bytes)?;
        manifest.push(json!({
            "blob_id": blob_id,
            "path": relative_path(export_path, &blob_path)?,
            "sha256": written.sha256,
            "size_bytes": written.size_bytes
        }));
        files.push(written);
    }
    files.push(write_json_file(
        &export_path.join("control/blob_manifest.json"),
        &JsonValue::Array(manifest),
    )?);
    Ok(())
}

pub(super) fn blob_bytes_for_global_blob_id(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    blob_id: &str,
) -> Result<Vec<u8>, NativeRepositoryError> {
    if let Some(bytes) = inline_blob_content_bytes(client, blob_id)? {
        return Ok(bytes);
    }
    if let Ok(bytes) = blob_bytes_for_blob_id(client, paths, repo_name, repo_id, blob_id) {
        return Ok(bytes);
    }
    let blob = select_blob_by_id(client, blob_id)?
        .ok_or_else(|| NativeRepositoryError::not_found(format!("Unknown blob: {blob_id}")))?;
    let pack_id = blob.pack_id.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown blob pack for {blob_id}"))
    })?;
    let (pack_path, pack_format) = pack_locator_for_id(client, paths, &pack_id)?;
    read_pack_entry_with_format(
        path_to_string(&pack_path)?.as_str(),
        format!("blobs/{blob_id}").as_str(),
        None,
        crate::foundation::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH,
        &pack_format,
    )
    .map_err(NativeRepositoryError::internal)
}

pub(super) fn table_counts(tables: &BTreeMap<String, Vec<JsonValue>>) -> JsonValue {
    let mut counts = JsonMap::new();
    for (table, rows) in tables {
        counts.insert(table.clone(), json!(rows.len()));
    }
    JsonValue::Object(counts)
}

pub(super) fn verify_export(
    export_path: &Path,
    manifest_path: &Path,
    manifest_sha256: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let actual_manifest_sha = sha256_path(manifest_path)?;
    if actual_manifest_sha != manifest_sha256 {
        return Err(NativeRepositoryError::internal(format!(
            "Manifest checksum mismatch for {}",
            manifest_path.display()
        )));
    }
    let manifest_text = fs::read_to_string(manifest_path).map_err(|exc| {
        NativeRepositoryError::internal(format!("failed to read manifest: {exc}"))
    })?;
    let manifest: JsonValue = serde_json::from_str(&manifest_text).map_err(|exc| {
        NativeRepositoryError::internal(format!("manifest JSON is invalid: {exc}"))
    })?;
    let mut verified_file_count = 0_u64;
    let mut verified_total_bytes = 0_u64;
    for entry in manifest
        .get("files")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let path = export_path.join(
            entry
                .get("path")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
        );
        if !path.exists() {
            return Err(NativeRepositoryError::internal(format!(
                "Export verification failed; missing file {}",
                path.display()
            )));
        }
        let actual_sha = sha256_path(&path)?;
        let expected_sha = entry
            .get("sha256")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        if actual_sha != expected_sha {
            return Err(NativeRepositoryError::internal(format!(
                "Export verification failed; checksum mismatch for {}",
                path.display()
            )));
        }
        let size = path
            .metadata()
            .map_err(|exc| {
                NativeRepositoryError::internal(format!("failed to stat {}: {exc}", path.display()))
            })?
            .len();
        let expected_size = entry
            .get("size_bytes")
            .and_then(JsonValue::as_u64)
            .unwrap_or(0);
        if size != expected_size {
            return Err(NativeRepositoryError::internal(format!(
                "Export verification failed; size mismatch for {}",
                path.display()
            )));
        }
        verified_file_count += 1;
        verified_total_bytes += size;
    }
    Ok(json!({
        "verified": true,
        "verified_file_count": verified_file_count,
        "verified_total_bytes": verified_total_bytes
    }))
}
