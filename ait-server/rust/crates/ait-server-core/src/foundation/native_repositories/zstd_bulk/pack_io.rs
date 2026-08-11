use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static ZSTD_TEMP_PACK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn put_zstd_bulk_object_pack_bytes(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    pack_id: &str,
    pack_bytes: &[u8],
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    validate_pack_id_segment(pack_id)?;
    if pack_bytes.is_empty() {
        return Err(NativeRepositoryError::bad_request(
            "zstd object pack body is empty",
        ));
    }
    if let Some(existing) = client
        .query_opt(
            "select repo_name, repo_id, pack_path, pack_format, member_count, total_bytes, pack_index_checksum from packs where pack_id = $1",
            &[&pack_id],
        )
        .map_err(db_internal)?
    {
        let pack_format: String = existing.get("pack_format");
        if pack_format != PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {pack_id} already exists with unsupported pack_format {pack_format:?}"
            )));
        }
        let existing_pack_path = zstd_pack_row_path(paths, &existing, pack_id, false)?;
        let existing_bytes = fs::read(&existing_pack_path).map_err(|exc| {
            NativeRepositoryError::internal(format!(
                "failed to read existing zstd object pack `{}`: {exc}",
                path_string(&existing_pack_path)
            ))
        })?;
        if existing_bytes != pack_bytes {
            return Err(NativeRepositoryError::conflict(format!(
                "Object pack {pack_id} already exists with different content"
            )));
        }
        return Ok(json!({
            "repo_name": repo_name,
            "repo_id": repo.repo_id,
            "pack_id": pack_id,
            "pack_format": pack_format,
            "status": "already_present",
            "raw_binary_upload": true,
        }));
    }
    let pack_path = zstd_object_pack_archive_path(paths, pack_id);
    write_verified_zstd_pack_file(
        &pack_path,
        pack_bytes,
        PACK_FORMAT_ZSTD_CHUNKED_V1,
        pack_id,
        false,
    )?;
    let pack_index = read_pack_index_with_format(
        path_to_string(&pack_path)?.as_str(),
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .map_err(NativeRepositoryError::bad_request)?;
    Ok(json!({
        "repo_name": repo_name,
        "repo_id": repo.repo_id,
        "pack_id": pack_id,
        "pack_format": PACK_FORMAT_ZSTD_CHUNKED_V1,
        "member_count": pack_index.get("member_count").cloned().unwrap_or(JsonValue::Null),
        "total_bytes": pack_index.get("total_bytes").cloned().unwrap_or(JsonValue::Null),
        "status": "uploaded",
        "raw_binary_upload": true,
    }))
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn get_zstd_bulk_object_pack_bytes(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    pack_id: &str,
) -> Result<Vec<u8>, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    validate_pack_id_segment(pack_id)?;
    let row = client
        .query_opt(
            "select repo_name, repo_id, pack_path, pack_format, member_count, total_bytes, pack_index_checksum from packs where pack_id = $1",
            &[&pack_id],
        )
        .map_err(db_internal)?
        .ok_or_else(|| {
            NativeRepositoryError::not_found(format!(
                "Unknown zstd object pack {pack_id} for repository {repo_name}"
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
            "Object pack {pack_id} belongs to repository {existing_repo_name}, not {repo_name}"
        )));
    }
    let pack_format: String = row.get("pack_format");
    if pack_format != PACK_FORMAT_ZSTD_CHUNKED_V1 {
        return Err(NativeRepositoryError::bad_request(format!(
            "Object pack {pack_id} has unsupported pack_format {pack_format:?}"
        )));
    }
    let pack_path = zstd_pack_row_path(paths, &row, pack_id, false)?;
    let index = read_pack_index_with_format(path_to_string(&pack_path)?.as_str(), &pack_format)
        .map_err(NativeRepositoryError::bad_request)?;
    let metadata = zstd_pack_metadata_from_row(&row, false);
    validate_zstd_pack_index_metadata(&index, &metadata, pack_id, false)?;
    validate_zstd_pack_index_checksum(
        &pack_path,
        &pack_format,
        optional_json_text(&metadata, "pack_index_checksum").as_deref(),
        pack_id,
        false,
    )?;
    fs::read(&pack_path).map_err(|exc| {
        NativeRepositoryError::internal(format!(
            "failed to read zstd object pack `{}`: {exc}",
            path_string(&pack_path)
        ))
    })
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn put_zstd_bulk_tree_pack_bytes(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    pack_id: &str,
    pack_bytes: &[u8],
) -> Result<JsonValue, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    validate_pack_id_segment(pack_id)?;
    if pack_bytes.is_empty() {
        return Err(NativeRepositoryError::bad_request(
            "zstd tree pack body is empty",
        ));
    }
    if let Some(existing) = client
        .query_opt(
            "select repo_name, repo_id, pack_format from tree_packs where pack_id = $1",
            &[&pack_id],
        )
        .map_err(db_internal)?
    {
        let pack_format: String = existing.get("pack_format");
        if pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} already exists with unsupported pack_format {pack_format:?}"
            )));
        }
        return Ok(json!({
            "repo_name": repo_name,
            "repo_id": repo.repo_id,
            "pack_id": pack_id,
            "pack_format": pack_format,
            "status": "already_present",
            "raw_binary_upload": true,
        }));
    }
    let pack_path = zstd_tree_pack_archive_path(paths, pack_id);
    write_verified_zstd_pack_file(
        &pack_path,
        pack_bytes,
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        pack_id,
        true,
    )?;
    let pack_index = read_tree_pack_index_with_format(
        path_to_string(&pack_path)?.as_str(),
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .map_err(NativeRepositoryError::bad_request)?;
    Ok(json!({
        "repo_name": repo_name,
        "repo_id": repo.repo_id,
        "pack_id": pack_id,
        "pack_format": TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        "tree_count": pack_index.get("tree_count").cloned().unwrap_or(JsonValue::Null),
        "total_bytes": pack_index.get("total_bytes").cloned().unwrap_or(JsonValue::Null),
        "status": "uploaded",
        "raw_binary_upload": true,
    }))
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn get_zstd_bulk_tree_pack_bytes(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo_name: &str,
    pack_id: &str,
) -> Result<Vec<u8>, NativeRepositoryError> {
    let repo = select_repository_row(client, repo_name)?.ok_or_else(|| {
        NativeRepositoryError::not_found(format!("Unknown repository: {repo_name}"))
    })?;
    validate_pack_id_segment(pack_id)?;
    let row = client
        .query_opt(
            "select repo_name, repo_id, pack_path, pack_format, tree_count, total_bytes, pack_index_checksum from tree_packs where pack_id = $1",
            &[&pack_id],
        )
        .map_err(db_internal)?
        .ok_or_else(|| {
            NativeRepositoryError::not_found(format!(
                "Unknown zstd tree pack {pack_id} for repository {repo_name}"
            ))
        })?;
    validate_tree_pack_download_scope(&row, pack_id, &repo.repo_name, &repo.repo_id)?;
    let pack_format: String = row.get("pack_format");
    if pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
        return Err(NativeRepositoryError::bad_request(format!(
            "Tree pack {pack_id} has unsupported pack_format {pack_format:?}"
        )));
    }
    let pack_path = zstd_pack_row_path(paths, &row, pack_id, true)?;
    let index =
        read_tree_pack_index_with_format(path_to_string(&pack_path)?.as_str(), &pack_format)
            .map_err(NativeRepositoryError::bad_request)?;
    let metadata = zstd_pack_metadata_from_row(&row, true);
    validate_zstd_pack_index_metadata(&index, &metadata, pack_id, true)?;
    validate_zstd_pack_index_checksum(
        &pack_path,
        &pack_format,
        optional_json_text(&metadata, "pack_index_checksum").as_deref(),
        pack_id,
        true,
    )?;
    fs::read(&pack_path).map_err(|exc| {
        NativeRepositoryError::internal(format!(
            "failed to read zstd tree pack `{}`: {exc}",
            path_string(&pack_path)
        ))
    })
}

pub(in crate::foundation::native_repositories) fn zstd_pack_index_from_bytes(
    pack_bytes: &[u8],
    pack_id: &str,
    tree_pack: bool,
) -> Result<(JsonValue, Option<String>), NativeRepositoryError> {
    if let Some(index) = uploaded_zstd_pack_index(pack_bytes) {
        let index = normalize_zstd_pack_index_for_remote(index, tree_pack);
        validate_remote_sync_uploaded_zstd_pack_index_metadata(
            &index,
            &JsonMap::new(),
            pack_id,
            tree_pack,
            None,
        )?;
        let checksum = index
            .get("pack_index_checksum")
            .and_then(JsonValue::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string);
        return Ok((index, checksum));
    }

    let temp_path = zstd_temp_pack_path(pack_id, tree_pack)?;
    fs::write(&temp_path, pack_bytes).map_err(|exc| {
        NativeRepositoryError::internal(format!(
            "failed to write temporary zstd pack `{}`: {exc}",
            path_string(&temp_path)
        ))
    })?;
    let pack_path = path_to_string(&temp_path)?;
    let result = if tree_pack {
        let index =
            read_tree_pack_index_with_format(pack_path.as_str(), TREE_PACK_FORMAT_ZSTD_CHUNKED_V1)
                .map_err(NativeRepositoryError::bad_request)?;
        let checksum = read_tree_pack_index_checksum_with_format(
            pack_path.as_str(),
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .map_err(NativeRepositoryError::bad_request)?;
        Ok((index, Some(checksum)))
    } else {
        let index = read_pack_index_with_format(pack_path.as_str(), PACK_FORMAT_ZSTD_CHUNKED_V1)
            .map_err(NativeRepositoryError::bad_request)?;
        let checksum =
            read_pack_index_checksum_with_format(pack_path.as_str(), PACK_FORMAT_ZSTD_CHUNKED_V1)
                .map_err(NativeRepositoryError::bad_request)?;
        Ok((index, Some(checksum)))
    };
    let _ = fs::remove_file(&temp_path);
    let (index, checksum) = result?;
    let index = normalize_zstd_pack_index_for_remote(index, tree_pack);
    validate_remote_sync_uploaded_zstd_pack_index_metadata(
        &index,
        &JsonMap::new(),
        pack_id,
        tree_pack,
        None,
    )?;
    Ok((index, checksum))
}

fn normalize_zstd_pack_index_for_remote(mut index: JsonValue, tree_pack: bool) -> JsonValue {
    let internal_format = if tree_pack {
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1
    } else {
        PACK_FORMAT_ZSTD_CHUNKED_V1
    };
    let remote_format = if tree_pack {
        REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1
    } else {
        REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1
    };
    if index.get("pack_format").and_then(JsonValue::as_str) == Some(internal_format) {
        if let Some(object) = index.as_object_mut() {
            object.insert(
                "pack_format".to_string(),
                JsonValue::String(remote_format.to_string()),
            );
        }
    }
    index
}

fn zstd_temp_pack_path(pack_id: &str, tree_pack: bool) -> Result<PathBuf, NativeRepositoryError> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|exc| NativeRepositoryError::internal(format!("system clock error: {exc}")))?
        .as_nanos();
    Ok(zstd_temp_pack_path_at_nanos(pack_id, tree_pack, nanos))
}

fn zstd_temp_pack_path_at_nanos(pack_id: &str, tree_pack: bool, nanos: u128) -> PathBuf {
    let label = if tree_pack { "tree" } else { "object" };
    let sequence = ZSTD_TEMP_PACK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "ait-server-binary-zstd-{label}-{pack_id}-{}-{nanos}-{sequence}.zstpack",
        std::process::id(),
    ))
}

#[cfg(test)]
mod temp_pack_path_tests {
    use super::*;

    #[test]
    fn same_process_pack_id_and_timestamp_still_get_unique_temp_paths() {
        let first = zstd_temp_pack_path_at_nanos("PCK-000000000001", false, 7);
        let second = zstd_temp_pack_path_at_nanos("PCK-000000000001", false, 7);

        assert_ne!(first, second);
        assert_eq!(
            first.extension().and_then(|value| value.to_str()),
            Some("zstpack")
        );
        assert_eq!(
            second.extension().and_then(|value| value.to_str()),
            Some("zstpack")
        );
    }
}

pub(in crate::foundation::native_repositories) fn binary_zstd_pack_upload_response(
    metadata: &JsonValue,
    repo_name: &str,
    repo_id: &str,
    pack_id: &str,
    label: &str,
    tree_pack: bool,
    status: &str,
) -> Result<JsonValue, NativeRepositoryError> {
    let mut object = JsonMap::new();
    object.insert("repo_name".to_string(), json!(repo_name));
    object.insert("repo_id".to_string(), json!(repo_id));
    object.insert("pack_id".to_string(), json!(pack_id));
    let pack_format = metadata.get("pack_format").cloned().ok_or_else(|| {
        NativeRepositoryError::internal(format!(
            "Binary DB {label} {pack_id} metadata is missing pack_format"
        ))
    })?;
    let expected_pack_format = if tree_pack {
        REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1
    } else {
        REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1
    };
    if pack_format.as_str() != Some(expected_pack_format) {
        return Err(NativeRepositoryError::internal(format!(
            "Binary DB {label} {pack_id} metadata has unsupported pack_format {pack_format}"
        )));
    }
    object.insert("pack_format".to_string(), pack_format);
    let count_field = if tree_pack {
        "tree_count"
    } else {
        "member_count"
    };
    if let Some(count) = metadata.get(count_field) {
        object.insert(count_field.to_string(), count.clone());
    }
    if let Some(total) = metadata.get("total_bytes") {
        object.insert("total_bytes".to_string(), total.clone());
    }
    object.insert("status".to_string(), json!(status));
    object.insert("raw_binary_upload".to_string(), JsonValue::Bool(true));
    object.insert("pack_kind".to_string(), json!(label));
    Ok(JsonValue::Object(object))
}

pub(in crate::foundation::native_repositories) fn binary_zstd_pack_metadata_object(
    value: &JsonValue,
    tree_pack: bool,
) -> JsonMap<String, JsonValue> {
    let mut object = JsonMap::new();
    for field in [
        "pack_format",
        if tree_pack {
            "tree_count"
        } else {
            "member_count"
        },
        "total_bytes",
        "pack_index_checksum",
    ] {
        if let Some(value) = value.get(field) {
            object.insert(field.to_string(), value.clone());
        }
    }
    object
}

fn write_verified_zstd_pack_file(
    pack_path: &Path,
    pack_bytes: &[u8],
    pack_format: &str,
    expected_pack_id: &str,
    tree_pack: bool,
) -> Result<(), NativeRepositoryError> {
    if let Some(parent) = pack_path.parent() {
        fs::create_dir_all(parent).map_err(|exc| {
            NativeRepositoryError::internal(format!(
                "failed to create zstd pack directory `{}`: {exc}",
                path_string(parent)
            ))
        })?;
    }
    let tmp_path = pack_path.with_extension(format!("tmp-{}", std::process::id()));
    {
        let mut file = fs::File::create(&tmp_path).map_err(|exc| {
            NativeRepositoryError::internal(format!(
                "failed to create zstd pack upload `{}`: {exc}",
                path_string(&tmp_path)
            ))
        })?;
        file.write_all(pack_bytes).map_err(|exc| {
            NativeRepositoryError::internal(format!(
                "failed to write zstd pack upload `{}`: {exc}",
                path_string(&tmp_path)
            ))
        })?;
    }
    let index = if tree_pack {
        read_tree_pack_index_with_format(path_to_string(&tmp_path)?.as_str(), pack_format)
    } else {
        read_pack_index_with_format(path_to_string(&tmp_path)?.as_str(), pack_format)
    }
    .map_err(|err| {
        let _ = fs::remove_file(&tmp_path);
        NativeRepositoryError::bad_request(err)
    })?;
    if index.get("pack_id").and_then(JsonValue::as_str) != Some(expected_pack_id) {
        let _ = fs::remove_file(&tmp_path);
        return Err(NativeRepositoryError::bad_request(format!(
            "zstd pack upload pack_id mismatch: path={expected_pack_id}, index={:?}",
            index.get("pack_id")
        )));
    }
    fs::rename(&tmp_path, pack_path).map_err(|exc| {
        let _ = fs::remove_file(&tmp_path);
        NativeRepositoryError::internal(format!(
            "failed to install zstd pack upload `{}`: {exc}",
            path_string(pack_path)
        ))
    })
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn upsert_zstd_object_pack(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo: &RepositoryRow,
    object: &JsonMap<String, JsonValue>,
    pack_id: &str,
) -> Result<(JsonValue, bool), NativeRepositoryError> {
    if let Some(existing) = client
        .query_opt(
            "select repo_name, pack_path, pack_format from packs where pack_id = $1",
            &[&pack_id],
        )
        .map_err(db_internal)?
    {
        let pack_format: String = existing.get("pack_format");
        if pack_format != PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Object pack {pack_id} already exists with unsupported pack_format {pack_format:?}"
            )));
        }
        let pack_path: String = existing.get("pack_path");
        let index = read_pack_index_with_format(
            path_to_string(&runtime_storage_path(paths, &pack_path))?.as_str(),
            PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .map_err(NativeRepositoryError::internal)?;
        validate_zstd_pack_index_metadata(&index, object, pack_id, false)?;
        return Ok((index, false));
    }

    let pack_path = zstd_object_pack_archive_path(paths, pack_id);
    if !pack_path.exists() {
        return Err(NativeRepositoryError::bad_request(format!(
            "zstd object pack {pack_id} was not uploaded before commit"
        )));
    }
    let index = read_pack_index_with_format(
        path_to_string(&pack_path)?.as_str(),
        PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .map_err(NativeRepositoryError::bad_request)?;
    validate_zstd_pack_index_metadata(&index, object, pack_id, false)?;
    let index_object = index.as_object().ok_or_else(|| {
        NativeRepositoryError::bad_request(format!(
            "zstd object pack {pack_id} index must be an object"
        ))
    })?;
    let member_count =
        i32::try_from(required_i64_field(index_object, "member_count")?).map_err(|_| {
            NativeRepositoryError::bad_request(format!(
                "zstd object pack {pack_id} member_count exceeds i32"
            ))
        })?;
    let total_bytes = required_i64_field(index_object, "total_bytes")?;
    let index_entry_name = required_json_text(index_object, "index_entry_name")
        .map_err(NativeRepositoryError::bad_request)?;
    let index_checksum = required_json_text(object, "pack_index_checksum")
        .map_err(NativeRepositoryError::bad_request)?;
    let created_at = optional_json_text(object, "created_at").unwrap_or_else(now_rfc3339);
    let inserted = client
        .execute(
            "insert into packs(pack_id, repo_name, repo_id, status, member_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at) values ($1, $2, $3, 'ready', $4, $5, $6, $7, $8, $9, $10::text::timestamptz) on conflict (pack_id) do nothing",
            &[
                &pack_id,
                &repo.repo_name,
                &repo.repo_id,
                &member_count,
                &total_bytes,
                &stored_path_string(paths, &pack_path)?,
                &PACK_FORMAT_ZSTD_CHUNKED_V1,
                &index_entry_name,
                &index_checksum,
                &created_at,
            ],
        )
        .map_err(db_internal)?
        > 0;
    Ok((index, inserted))
}

#[cfg(feature = "legacy-postgres-runtime")]
fn validate_tree_pack_download_scope(
    row: &postgres::Row,
    pack_id: &str,
    repo_name: &str,
    repo_id: &str,
) -> Result<(), NativeRepositoryError> {
    let existing_repo_name = row.get::<_, Option<String>>("repo_name");
    let existing_repo_id = row.get::<_, Option<String>>("repo_id");
    validate_tree_pack_owner_values(
        existing_repo_name.as_deref(),
        existing_repo_id.as_deref(),
        pack_id,
        repo_name,
        repo_id,
    )
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn object_pack_has_repository_blob_locator(
    client: &mut postgres::Client,
    pack_id: &str,
    repo_name: &str,
    repo_id: &str,
) -> Result<bool, NativeRepositoryError> {
    client
        .query_opt(
            "select blob_id from blob_locators where pack_id = $1 and repo_id = $2 and repo_name = $3 limit 1",
            &[&pack_id, &repo_id, &repo_name],
        )
        .map(|row| row.is_some())
        .map_err(db_internal)
}

fn tree_pack_owner_values_match(
    existing_repo_name: Option<&str>,
    existing_repo_id: Option<&str>,
    repo_name: &str,
    repo_id: &str,
) -> bool {
    existing_repo_id == Some(repo_id) && existing_repo_name == Some(repo_name)
}

pub(in crate::foundation::native_repositories) fn validate_tree_pack_owner_values(
    existing_repo_name: Option<&str>,
    existing_repo_id: Option<&str>,
    pack_id: &str,
    repo_name: &str,
    repo_id: &str,
) -> Result<(), NativeRepositoryError> {
    if tree_pack_owner_values_match(existing_repo_name, existing_repo_id, repo_name, repo_id) {
        return Ok(());
    }
    if existing_repo_name.is_none() || existing_repo_id.is_none() {
        return Err(NativeRepositoryError::conflict(format!(
            "Tree pack {pack_id} is missing repository ownership metadata"
        )));
    }
    if let Some(existing_repo_name) = existing_repo_name {
        return Err(NativeRepositoryError::conflict(format!(
            "Tree pack {pack_id} belongs to repository {existing_repo_name}, not {repo_name}"
        )));
    }
    if let Some(existing_repo_id) = existing_repo_id {
        return Err(NativeRepositoryError::conflict(format!(
            "Tree pack {pack_id} belongs to repository id {existing_repo_id}, not {repo_name}"
        )));
    }
    unreachable!("complete mismatched ownership must have a repository name or id")
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn upsert_zstd_tree_pack(
    client: &mut postgres::Client,
    paths: &ServerRuntimePaths,
    repo: &RepositoryRow,
    object: &JsonMap<String, JsonValue>,
    pack_id: &str,
) -> Result<(JsonValue, bool), NativeRepositoryError> {
    if let Some(existing) = client
        .query_opt(
            "select repo_name, repo_id, pack_path, pack_format from tree_packs where pack_id = $1",
            &[&pack_id],
        )
        .map_err(db_internal)?
    {
        let pack_format: String = existing.get("pack_format");
        if pack_format != TREE_PACK_FORMAT_ZSTD_CHUNKED_V1 {
            return Err(NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} already exists with unsupported pack_format {pack_format:?}"
            )));
        }
        let pack_path: String = existing.get("pack_path");
        let index = read_tree_pack_index_with_format(
            path_to_string(&runtime_storage_path(paths, &pack_path))?.as_str(),
            TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
        )
        .map_err(NativeRepositoryError::internal)?;
        return Ok((index, false));
    }

    let pack_path = zstd_tree_pack_archive_path(paths, pack_id);
    if !pack_path.exists() {
        return Err(NativeRepositoryError::bad_request(format!(
            "zstd tree pack {pack_id} was not uploaded before commit"
        )));
    }
    let index = read_tree_pack_index_with_format(
        path_to_string(&pack_path)?.as_str(),
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
    )
    .map_err(NativeRepositoryError::bad_request)?;
    validate_zstd_pack_index_metadata(&index, object, pack_id, true)?;
    let index_object = index.as_object().ok_or_else(|| {
        NativeRepositoryError::bad_request(format!(
            "zstd tree pack {pack_id} index must be an object"
        ))
    })?;
    let tree_count =
        i32::try_from(required_i64_field(index_object, "tree_count")?).map_err(|_| {
            NativeRepositoryError::bad_request(format!(
                "zstd tree pack {pack_id} tree_count exceeds i32"
            ))
        })?;
    let total_bytes = required_i64_field(index_object, "total_bytes")?;
    let index_entry_name = required_json_text(index_object, "index_entry_name")
        .map_err(NativeRepositoryError::bad_request)?;
    let index_checksum = required_json_text(object, "pack_index_checksum")
        .map_err(NativeRepositoryError::bad_request)?;
    let created_at = optional_json_text(object, "created_at").unwrap_or_else(now_rfc3339);
    let inserted = client
        .execute(
            "insert into tree_packs(pack_id, repo_name, repo_id, status, tree_count, total_bytes, pack_path, pack_format, pack_index_entry_name, pack_index_checksum, created_at) values ($1, $2, $3, 'ready', $4, $5, $6, $7, $8, $9, $10::text::timestamptz) on conflict (pack_id) do nothing",
            &[
                &pack_id,
                &repo.repo_name,
                &repo.repo_id,
                &tree_count,
                &total_bytes,
                &stored_path_string(paths, &pack_path)?,
                &TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                &index_entry_name,
                &index_checksum,
                &created_at,
            ],
        )
        .map_err(db_internal)?
        > 0;
    Ok((index, inserted))
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn zstd_pack_row_path(
    paths: &ServerRuntimePaths,
    row: &postgres::Row,
    pack_id: &str,
    tree_pack: bool,
) -> Result<PathBuf, NativeRepositoryError> {
    let pack_path = row
        .get::<_, Option<String>>("pack_path")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            let label = if tree_pack {
                "tree pack"
            } else {
                "object pack"
            };
            NativeRepositoryError::internal(format!("{label} {pack_id} is missing pack_path"))
        })?;
    Ok(runtime_storage_path(paths, &pack_path))
}

#[cfg(feature = "legacy-postgres-runtime")]
pub(in crate::foundation::native_repositories) fn zstd_pack_metadata_from_row(
    row: &postgres::Row,
    tree_pack: bool,
) -> JsonMap<String, JsonValue> {
    let mut metadata = JsonMap::new();
    metadata.insert(
        "pack_format".to_string(),
        JsonValue::String(row.get::<_, String>("pack_format")),
    );
    metadata.insert(
        if tree_pack {
            "tree_count"
        } else {
            "member_count"
        }
        .to_string(),
        JsonValue::Number(JsonNumber::from(if tree_pack {
            i64::from(row.get::<_, i32>("tree_count"))
        } else {
            i64::from(row.get::<_, i32>("member_count"))
        })),
    );
    metadata.insert(
        "total_bytes".to_string(),
        JsonValue::Number(JsonNumber::from(row.get::<_, i64>("total_bytes"))),
    );
    if let Some(checksum) = row
        .get::<_, Option<String>>("pack_index_checksum")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        metadata.insert(
            "pack_index_checksum".to_string(),
            JsonValue::String(checksum),
        );
    }
    metadata
}

fn validate_zstd_pack_index_checksum(
    pack_path: &Path,
    pack_format: &str,
    expected_checksum: Option<&str>,
    pack_id: &str,
    tree_pack: bool,
) -> Result<(), NativeRepositoryError> {
    let Some(expected_checksum) = expected_checksum
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    let actual_checksum = if tree_pack {
        read_tree_pack_index_checksum_with_format(path_to_string(pack_path)?.as_str(), pack_format)
    } else {
        read_pack_index_checksum_with_format(path_to_string(pack_path)?.as_str(), pack_format)
    }
    .map_err(NativeRepositoryError::bad_request)?;
    if actual_checksum != expected_checksum {
        return Err(NativeRepositoryError::bad_request(format!(
            "zstd pack {pack_id} index checksum mismatch"
        )));
    }
    Ok(())
}

pub(in crate::foundation::native_repositories) fn uploaded_zstd_pack_index(
    pack_bytes: &[u8],
) -> Option<JsonValue> {
    let index: JsonValue = serde_json::from_slice(pack_bytes).ok()?;
    match &index {
        JsonValue::Object(object)
            if object.contains_key("pack_id") && object.contains_key("pack_format") =>
        {
            Some(index)
        }
        _ => None,
    }
}

pub(in crate::foundation::native_repositories) fn uploaded_tree_pack_root_index(
    pack_bytes: &[u8],
) -> Option<JsonValue> {
    let index: JsonValue = serde_json::from_slice(pack_bytes).ok()?;
    index.get("trees")?.as_array()?;
    Some(index)
}

pub(in crate::foundation::native_repositories) fn validate_remote_sync_uploaded_zstd_pack_index_metadata(
    index: &JsonValue,
    object: &JsonMap<String, JsonValue>,
    pack_id: &str,
    tree_pack: bool,
    detected_checksum: Option<&str>,
) -> Result<(), NativeRepositoryError> {
    if index.get("pack_id").and_then(JsonValue::as_str) != Some(pack_id) {
        return Err(NativeRepositoryError::bad_request(format!(
            "zstd pack {pack_id} index pack_id mismatch"
        )));
    }
    let expected_format = if tree_pack {
        REMOTE_SYNC_ZSTD_TREE_PACK_FORMAT_V1
    } else {
        REMOTE_SYNC_ZSTD_OBJECT_PACK_FORMAT_V1
    };
    if index.get("pack_format").and_then(JsonValue::as_str) != Some(expected_format) {
        return Err(NativeRepositoryError::bad_request(format!(
            "zstd pack {pack_id} index pack_format mismatch"
        )));
    }
    if let Some(expected) = optional_i64_field(
        object,
        if tree_pack {
            "tree_count"
        } else {
            "member_count"
        },
    )? {
        let actual = index
            .get(if tree_pack {
                "tree_count"
            } else {
                "member_count"
            })
            .and_then(JsonValue::as_i64)
            .unwrap_or(-1);
        if actual != expected {
            return Err(NativeRepositoryError::bad_request(format!(
                "zstd pack {pack_id} member count mismatch"
            )));
        }
    }
    if let Some(expected) = optional_i64_field(object, "total_bytes")? {
        let actual = index
            .get("total_bytes")
            .and_then(JsonValue::as_i64)
            .unwrap_or(-1);
        if actual != expected {
            return Err(NativeRepositoryError::bad_request(format!(
                "zstd pack {pack_id} total_bytes mismatch"
            )));
        }
    }
    if let Some(expected) = optional_json_text(object, "pack_index_checksum")
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        let actual = detected_checksum
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .or_else(|| {
                index
                    .get("pack_index_checksum")
                    .and_then(JsonValue::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            })
            .unwrap_or_default();
        if actual != expected {
            return Err(NativeRepositoryError::bad_request(format!(
                "zstd pack {pack_id} index checksum mismatch"
            )));
        }
    }
    Ok(())
}
