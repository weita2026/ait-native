use super::*;

pub(super) fn list_lines_json(
    client: &mut pg::Client,
    repo_name: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    let rows = client
        .query(
            "select repo_name, repo_id, line_name, head_snapshot_id, status, archived_at::text as archived_at_text, created_at::text as created_at_text, updated_at::text as updated_at_text from lines where repo_id = $1 order by line_name asc",
            &[&repo.repo_id],
        )
        .map_err(db_internal)?;
    let items = rows
        .into_iter()
        .map(line_json)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(JsonValue::Array(items))
}

pub(super) fn get_line_json(
    client: &mut pg::Client,
    repo_name: &str,
    line_name: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    let row = client
        .query_opt(
            "select repo_name, repo_id, line_name, head_snapshot_id, status, archived_at::text as archived_at_text, created_at::text as created_at_text, updated_at::text as updated_at_text from lines where repo_id = $1 and line_name = $2",
            &[&repo.repo_id, &line_name],
        )
        .map_err(db_internal)?
        .ok_or_else(|| {
            NativeRepositoryError::not_found(format!(
                "Unknown line {line_name} for repository {repo_name}"
            ))
        })?;
    line_json(row)
}

pub(in crate::foundation::native_repositories) fn update_line_json(
    client: &mut pg::Client,
    repo_name: &str,
    line_name: &str,
    request: LineUpdateRequest,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    ensure_zstd_only_repository_flow_allowed(
        client,
        repo_name,
        &repo,
        ZstdOnlyRepositoryFlow::LineUpdate,
    )?;
    let line_name = normalize_required_text(line_name, "line_name")?;
    let head_snapshot_id = normalize_optional_text(request.head_snapshot_id);
    let expected_head_snapshot_id = normalize_optional_text(request.expected_head_snapshot_id);

    if let Some(snapshot_id) = head_snapshot_id.as_deref() {
        let snapshot_repo_name = client
            .query_opt(
                "select repo_name from snapshots where snapshot_id = $1",
                &[&snapshot_id],
            )
            .map_err(db_internal)?
            .map(|row| row.get::<_, String>("repo_name"))
            .ok_or_else(|| {
                NativeRepositoryError::not_found(format!("Unknown snapshot: {snapshot_id}"))
            })?;
        if snapshot_repo_name != repo_name {
            return Err(NativeRepositoryError::not_found(format!(
                "Snapshot {snapshot_id} belongs to repository {snapshot_repo_name}, not {repo_name}"
            )));
        }
    }

    let current = client
        .query_opt(
            "select repo_name, repo_id, line_name, head_snapshot_id, status, archived_at::text as archived_at_text, created_at::text as created_at_text, updated_at::text as updated_at_text from lines where repo_id = $1 and line_name = $2",
            &[&repo.repo_id, &line_name],
        )
        .map_err(db_internal)?;
    let current_head = current
        .as_ref()
        .and_then(|row| row.get::<_, Option<String>>("head_snapshot_id"));
    if let Some(expected) = expected_head_snapshot_id.as_deref() {
        if current_head.as_deref() != Some(expected) {
            return Err(NativeRepositoryError::bad_request(format!(
                "Line {line_name} head advanced before update: expected {expected:?}, got {:?}",
                current_head
            )));
        }
    }
    let timestamp = now_rfc3339();
    match current {
        None => {
            client
                .execute(
                    "insert into lines(repo_name, repo_id, line_name, head_snapshot_id, status, archived_at, created_at, updated_at) values ($1, $2, $3, $4, 'active', null, $5::text::timestamptz, $6::text::timestamptz)",
                    &[&repo_name, &repo.repo_id, &line_name, &head_snapshot_id, &timestamp, &timestamp],
                )
                .map_err(db_internal)?;
        }
        Some(row) => {
            let status: String = row.get("status");
            if status.trim().eq_ignore_ascii_case("archived") {
                return Err(NativeRepositoryError::bad_request(format!(
                    "Line {line_name} is archived and cannot move"
                )));
            }
            client
                .execute(
                    "update lines set head_snapshot_id = $1, updated_at = $2::text::timestamptz where repo_id = $3 and line_name = $4",
                    &[&head_snapshot_id, &timestamp, &repo.repo_id, &line_name],
                )
                .map_err(db_internal)?;
        }
    }
    get_line_json(client, repo_name, &line_name)
}

pub(super) fn close_line_json(
    client: &mut pg::Client,
    repo_name: &str,
    line_name: &str,
    request: LineCloseRequest,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    if request.status.trim() != "archived" {
        return Err(NativeRepositoryError::bad_request(format!(
            "Unsupported line status: {:?}",
            request.status
        )));
    }
    if line_name == repo.default_line {
        return Err(NativeRepositoryError::bad_request(format!(
            "Default line {line_name} cannot be archived"
        )));
    }
    let row = client
        .query_opt(
            "select repo_name, repo_id, line_name, head_snapshot_id, status, archived_at::text as archived_at_text, created_at::text as created_at_text, updated_at::text as updated_at_text from lines where repo_id = $1 and line_name = $2",
            &[&repo.repo_id, &line_name],
        )
        .map_err(db_internal)?
        .ok_or_else(|| {
            NativeRepositoryError::not_found(format!(
                "Unknown line {line_name} for repository {repo_name}"
            ))
        })?;
    let status: String = row.get("status");
    if status.trim().eq_ignore_ascii_case("archived") {
        return line_json(row);
    }
    let timestamp = now_rfc3339();
    client
        .execute(
            "update lines set status = 'archived', archived_at = $1::text::timestamptz, updated_at = $2::text::timestamptz where repo_id = $3 and line_name = $4",
            &[&timestamp, &timestamp, &repo.repo_id, &line_name],
        )
        .map_err(db_internal)?;
    get_line_json(client, repo_name, line_name)
}

pub(super) fn snapshot_existence_json(
    client: &mut pg::Client,
    repo_name: &str,
    request: SnapshotExistsRequest,
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    let normalized = request
        .snapshot_ids
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let unique = normalized
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let present_set = if unique.is_empty() {
        HashSet::new()
    } else {
        let statement = format!(
            "select snapshot_id from snapshots where repo_id = $1 and snapshot_id = any($2)"
        );
        client
            .query(statement.as_str(), &[&repo.repo_id, &unique])
            .map_err(db_internal)?
            .into_iter()
            .map(|row| row.get::<_, String>("snapshot_id"))
            .collect::<HashSet<_>>()
    };
    let present = normalized
        .iter()
        .filter(|value| present_set.contains(value.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let missing = normalized
        .iter()
        .filter(|value| !present_set.contains(value.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    Ok(json!({
        "repo_name": repo_name,
        "checked_snapshots": normalized.len(),
        "present": present,
        "missing": missing,
    }))
}

pub(in crate::foundation::native_repositories) fn validate_existing_snapshot(
    existing: &SnapshotRow,
    repo_name: &str,
    line_name: &str,
    parent_snapshot_id: Option<&str>,
    message: Option<&str>,
    file_count: i64,
    total_bytes: i64,
) -> Result<(), NativeRepositoryError> {
    let mut mismatches = Vec::new();
    if existing.repo_name != repo_name {
        mismatches.push(format!("repository={:?}", existing.repo_name));
    }
    if existing.line_name.as_deref().unwrap_or("main") != line_name {
        mismatches.push(format!("line_name={:?}", existing.line_name));
    }
    if existing.parent_snapshot_id.as_deref() != parent_snapshot_id {
        mismatches.push(format!(
            "parent_snapshot_id={:?}",
            existing.parent_snapshot_id
        ));
    }
    if existing.message.as_deref() != message {
        mismatches.push(format!("message={:?}", existing.message));
    }
    if i64::from(existing.file_count) != file_count {
        mismatches.push(format!("file_count={:?}", existing.file_count));
    }
    if existing.total_bytes != total_bytes {
        mismatches.push(format!("total_bytes={:?}", existing.total_bytes));
    }
    if !mismatches.is_empty() {
        return Err(NativeRepositoryError::bad_request(format!(
            "Snapshot {} already exists with different canonical fields: {}",
            existing.snapshot_id,
            mismatches.join(", ")
        )));
    }
    Ok(())
}

pub(in crate::foundation::native_repositories) fn select_snapshot_row(
    client: &mut pg::Client,
    repo_name: &str,
    snapshot_id: &str,
) -> Result<Option<SnapshotRow>, NativeRepositoryError> {
    let row = client
        .query_opt(
            "select snapshot_id, repo_name, repo_id, parent_snapshot_id, root_tree_pack_id, root_entry_ordinal, manifest_hash, message, line_name, file_count, total_bytes, created_at::text as created_at_text from snapshots where snapshot_id = $1 and repo_name = $2",
            &[&snapshot_id, &repo_name],
        )
        .map_err(db_internal)?;
    Ok(row.map(snapshot_row_from_db))
}

pub(in crate::foundation::native_repositories) fn snapshot_json_from_row(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    row: SnapshotRow,
) -> Result<JsonValue, NativeRepositoryError> {
    let (tree_pack_path, tree_pack_format) = tree_pack_locator_for_id(
        client,
        paths,
        &row.repo_name,
        &row.repo_id,
        &row.root_tree_pack_id,
    )?;
    let root_payload = read_tree_pack_tree_by_ordinal_with_format(
        path_to_string(&tree_pack_path)?.as_str(),
        row.root_entry_ordinal,
        &tree_pack_format,
    )
    .map_err(NativeRepositoryError::internal)?;
    let root_tree_id = root_payload
        .get("tree_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| NativeRepositoryError::internal("root tree payload is missing tree_id"))?;
    Ok(json!({
        "snapshot_id": row.snapshot_id,
        "repo_name": row.repo_name,
        "repo_id": row.repo_id,
        "parent_snapshot_id": row.parent_snapshot_id,
        "root_tree_pack_id": row.root_tree_pack_id,
        "root_entry_ordinal": row.root_entry_ordinal,
        "manifest_hash": row.manifest_hash,
        "manifest_path": tree_pack_manifest_path(stored_path_string(paths, &tree_pack_path)?.as_str(), format!("trees/{root_tree_id}.json").as_str()),
        "message": row.message,
        "line_name": row.line_name,
        "file_count": row.file_count,
        "total_bytes": row.total_bytes,
        "created_at": row.created_at,
        "capabilities": remote_sync_capabilities(),
    }))
}
