use super::*;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static ZSTD_TEMP_PACK_SEQUENCE: AtomicU64 = AtomicU64::new(0);
const UPLOADED_JSON_PACK_INDEX_MAX_BYTES: u64 = 16 * 1024 * 1024;
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

pub(in crate::foundation::native_repositories) fn zstd_pack_index_from_path(
    pack_path: &Path,
    pack_id: &str,
    tree_pack: bool,
) -> Result<(JsonValue, Option<String>), NativeRepositoryError> {
    let pack_path_text = pack_path.to_str().ok_or_else(|| {
        NativeRepositoryError::internal(format!(
            "zstd pack path is not valid UTF-8: {}",
            pack_path.display()
        ))
    })?;
    let archive_result = if tree_pack {
        read_tree_pack_index_with_format(pack_path_text, TREE_PACK_FORMAT_ZSTD_CHUNKED_V1).and_then(
            |index| {
                read_tree_pack_index_checksum_with_format(
                    pack_path_text,
                    TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
                )
                .map(|checksum| (index, Some(checksum)))
            },
        )
    } else {
        read_pack_index_with_format(pack_path_text, PACK_FORMAT_ZSTD_CHUNKED_V1).and_then(|index| {
            read_pack_index_checksum_with_format(pack_path_text, PACK_FORMAT_ZSTD_CHUNKED_V1)
                .map(|checksum| (index, Some(checksum)))
        })
    };

    let (index, checksum) = match archive_result {
        Ok(value) => value,
        Err(archive_error) => {
            let payload_bytes = fs::metadata(pack_path)
                .map_err(|error| {
                    NativeRepositoryError::internal(format!(
                        "inspect staged zstd pack {}: {error}",
                        pack_path.display()
                    ))
                })?
                .len();
            if payload_bytes > UPLOADED_JSON_PACK_INDEX_MAX_BYTES {
                return Err(NativeRepositoryError::bad_request(archive_error));
            }
            let reader = BufReader::new(File::open(pack_path).map_err(|error| {
                NativeRepositoryError::internal(format!(
                    "open staged zstd pack {}: {error}",
                    pack_path.display()
                ))
            })?);
            let index: JsonValue = serde_json::from_reader(reader)
                .map_err(|_| NativeRepositoryError::bad_request(archive_error.clone()))?;
            if !matches!(
                &index,
                JsonValue::Object(object)
                    if object.contains_key("pack_id") && object.contains_key("pack_format")
            ) {
                return Err(NativeRepositoryError::bad_request(archive_error));
            }
            let checksum = index
                .get("pack_index_checksum")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string);
            (index, checksum)
        }
    };
    let index = normalize_zstd_pack_index_for_remote(index, tree_pack);
    validate_remote_sync_uploaded_zstd_pack_index_metadata(
        &index,
        &JsonMap::new(),
        pack_id,
        tree_pack,
        checksum.as_deref(),
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
