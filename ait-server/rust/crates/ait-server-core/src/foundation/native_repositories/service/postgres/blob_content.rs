use super::*;

pub(in crate::foundation::native_repositories) fn blob_bytes_for_blob_id(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    blob_id: &str,
) -> Result<Vec<u8>, NativeRepositoryError> {
    if let Some(bytes) = inline_blob_content_bytes(client, blob_id)? {
        return Ok(bytes);
    }
    let mut visited_blob_ids = HashSet::new();
    blob_bytes_for_blob_id_inner(
        client,
        paths,
        repo_name,
        repo_id,
        blob_id,
        &mut visited_blob_ids,
    )
}

pub(super) fn blob_bytes_for_blob_id_inner(
    client: &mut pg::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    repo_id: &str,
    blob_id: &str,
    visited_blob_ids: &mut HashSet<String>,
) -> Result<Vec<u8>, NativeRepositoryError> {
    if native_blob_resolver_delta_chain_exceeded(visited_blob_ids.len()) {
        return Err(NativeRepositoryError::internal(format!(
            "Pack delta chain depth exceeded for blobs/{blob_id}"
        )));
    }
    if !visited_blob_ids.insert(blob_id.to_string()) {
        return Err(NativeRepositoryError::internal(format!(
            "Cyclic pack delta chain detected for blobs/{blob_id}"
        )));
    }
    let blob =
        require_blob_locator_for_repo(client, repo_name, repo_id, blob_id).map_err(|err| {
            if err.kind == NativeRepositoryErrorKind::BadRequest
                && err
                    .message
                    .contains("Snapshot closure references missing blob")
            {
                NativeRepositoryError::not_found(format!("Unknown blob: {blob_id}"))
            } else {
                err
            }
        })?;
    let pack_id = blob.pack_id;
    let (pack_path, pack_format) = pack_locator_for_id(client, paths, &pack_id)?;
    let mut base_blob_map = BTreeMap::new();
    if blob.pack_entry_type.as_deref() == Some("delta") {
        let base_blob_id = blob.pack_base_blob_id.as_deref().ok_or_else(|| {
            NativeRepositoryError::internal(format!(
                "Blob {blob_id} is missing delta base blob metadata"
            ))
        })?;
        let base_bytes = blob_bytes_for_blob_id_inner(
            client,
            paths,
            repo_name,
            repo_id,
            base_blob_id,
            visited_blob_ids,
        )?;
        base_blob_map.insert(base_blob_id.to_string(), base_bytes);
    }
    let base_blob_resolver = if base_blob_map.is_empty() {
        None
    } else {
        Some(&base_blob_map)
    };
    read_pack_entry_with_format(
        path_to_string(&pack_path)?.as_str(),
        format!("blobs/{blob_id}").as_str(),
        base_blob_resolver,
        crate::foundation::pack_substrate::MAX_DELTA_CHAIN_READ_DEPTH,
        &pack_format,
    )
    .map_err(NativeRepositoryError::internal)
}

pub(in crate::foundation::native_repositories) fn native_blob_resolver_delta_chain_exceeded(
    visited_blob_count: usize,
) -> bool {
    visited_blob_count > NATIVE_BLOB_RESOLVER_MAX_DELTA_CHAIN_DEPTH
}

pub(in crate::foundation::native_repositories) fn select_blob_by_id(
    client: &mut pg::Client,
    blob_id: &str,
) -> Result<Option<BlobRow>, NativeRepositoryError> {
    client
        .query_opt(
            "select blob_id, sha256, pack_id from blobs where blob_id = $1",
            &[&blob_id],
        )
        .map_err(db_internal)
        .map(|row| row.map(blob_row_from_db))
}

pub(in crate::foundation::native_repositories) fn select_blob_locator_for_repo(
    client: &mut pg::Client,
    _repo_name: &str,
    repo_id: &str,
    blob_id: &str,
) -> Result<Option<BlobLocatorRow>, NativeRepositoryError> {
    client
        .query_opt(
            "select blob_id, sha256, size_bytes, coalesce(pack_id, '') as pack_id, pack_entry_type, pack_base_blob_id, pack_chain_depth, created_at::text as created_at_text from blob_locators where blob_id = $1 and repo_id = $2",
            &[&blob_id, &repo_id],
        )
        .map_err(db_internal)
        .map(|row| row.map(blob_locator_row_from_db))
}

pub(in crate::foundation::native_repositories) fn require_blob_locator_for_repo(
    client: &mut pg::Client,
    repo_name: &str,
    repo_id: &str,
    blob_id: &str,
) -> Result<BlobLocatorRow, NativeRepositoryError> {
    let locator =
        select_blob_locator_for_repo(client, repo_name, repo_id, blob_id)?.ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "Snapshot closure references missing blob {blob_id}"
            ))
        })?;
    require_blob_locator_pack_id(locator, repo_name)
}

pub(super) fn require_blob_locator_pack_id(
    locator: BlobLocatorRow,
    repo_name: &str,
) -> Result<BlobLocatorRow, NativeRepositoryError> {
    if locator.pack_id.trim().is_empty() {
        return Err(NativeRepositoryError::bad_request(format!(
            "Blob {} is missing zstd object pack metadata for repository {repo_name}",
            locator.blob_id
        )));
    }
    let entry_type = locator
        .pack_entry_type
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "Blob {} is missing pack entry type metadata for repository {repo_name}",
                locator.blob_id
            ))
        })?;
    let chain_depth = locator.pack_chain_depth.ok_or_else(|| {
        NativeRepositoryError::bad_request(format!(
            "Blob {} is missing pack chain depth metadata for repository {repo_name}",
            locator.blob_id
        ))
    })?;
    match entry_type {
        "full" if locator.pack_base_blob_id.is_none() && chain_depth == 0 => {}
        "delta"
            if locator
                .pack_base_blob_id
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty())
                && chain_depth > 0 => {}
        _ => {
            return Err(NativeRepositoryError::bad_request(format!(
                "Blob {} has inconsistent current pack metadata for repository {repo_name}",
                locator.blob_id
            )));
        }
    }
    Ok(locator)
}

pub(super) fn inline_blob_content_bytes(
    client: &mut pg::Client,
    blob_id: &str,
) -> Result<Option<Vec<u8>>, NativeRepositoryError> {
    let Some(row) = client
        .query_opt(
            "select sha256, size_bytes, content from blob_inline_contents where blob_id = $1",
            &[&blob_id],
        )
        .map_err(db_internal)?
    else {
        return Ok(None);
    };
    let expected_sha256: String = row.get("sha256");
    let expected_size_bytes: i64 = row.get("size_bytes");
    let bytes: Vec<u8> = row.get("content");
    if expected_size_bytes != bytes.len() as i64 {
        return Err(NativeRepositoryError::internal(format!(
            "Inline blob content {blob_id} expected {expected_size_bytes} bytes, got {}",
            bytes.len()
        )));
    }
    let actual_sha256 = sha256_hex(&bytes);
    if actual_sha256 != expected_sha256 {
        return Err(NativeRepositoryError::internal(format!(
            "Inline blob content {blob_id} expected sha256 {expected_sha256}, got {actual_sha256}"
        )));
    }
    Ok(Some(bytes))
}
