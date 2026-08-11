use super::*;

pub(in crate::foundation::native_repositories) fn json_value_array<'a>(
    value: Option<&'a JsonValue>,
    field: &str,
) -> Result<Vec<&'a JsonValue>, NativeRepositoryError> {
    match value {
        None | Some(JsonValue::Null) => Ok(Vec::new()),
        Some(JsonValue::Array(values)) => Ok(values.iter().collect()),
        _ => Err(NativeRepositoryError::bad_request(format!(
            "zstd bulk `{field}` must be an array"
        ))),
    }
}

pub(in crate::foundation::native_repositories) fn json_text_array(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Vec<String>, NativeRepositoryError> {
    let mut out = Vec::new();
    for item in json_value_array(value, field)? {
        let Some(text) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) else {
            return Err(NativeRepositoryError::bad_request(format!(
                "zstd bulk `{field}` entries must be non-empty strings"
            )));
        };
        out.push(text.to_string());
    }
    Ok(out)
}

pub(in crate::foundation::native_repositories) fn pack_ids_from_array(
    value: Option<&JsonValue>,
    field: &str,
) -> Result<Vec<String>, NativeRepositoryError> {
    let mut out = Vec::new();
    for item in json_value_array(value, field)? {
        if let Some(text) = item.as_str().map(str::trim).filter(|text| !text.is_empty()) {
            out.push(text.to_string());
            continue;
        }
        let object = item.as_object().ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "zstd bulk `{field}` entries must be strings or objects"
            ))
        })?;
        out.push(
            required_json_text(object, "pack_id").map_err(NativeRepositoryError::bad_request)?,
        );
    }
    Ok(out)
}

pub(in crate::foundation::native_repositories) fn json_object<'a>(
    value: &'a JsonValue,
    label: &str,
) -> Result<&'a JsonMap<String, JsonValue>, NativeRepositoryError> {
    value.as_object().ok_or_else(|| {
        NativeRepositoryError::bad_request(format!("zstd bulk `{label}` entry must be an object"))
    })
}

pub(in crate::foundation::native_repositories) fn optional_i64_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<Option<i64>, NativeRepositoryError> {
    object.get(field).map(json_i64).transpose()
}

pub(in crate::foundation::native_repositories) fn required_i64_field(
    object: &JsonMap<String, JsonValue>,
    field: &str,
) -> Result<i64, NativeRepositoryError> {
    object
        .get(field)
        .ok_or_else(|| NativeRepositoryError::bad_request(format!("Field `{field}` is required.")))
        .and_then(json_i64)
}

pub(in crate::foundation::native_repositories) fn validate_pack_id_segment(
    pack_id: &str,
) -> Result<(), NativeRepositoryError> {
    let pack_id = pack_id.trim();
    if pack_id.is_empty()
        || pack_id.contains('/')
        || pack_id.contains('\\')
        || pack_id.contains('\0')
        || pack_id == "."
        || pack_id == ".."
    {
        return Err(NativeRepositoryError::bad_request(format!(
            "invalid zstd pack_id path segment: {pack_id}"
        )));
    }
    Ok(())
}

pub(in crate::foundation::native_repositories) fn validate_zstd_pack_index_metadata(
    index: &JsonValue,
    object: &JsonMap<String, JsonValue>,
    pack_id: &str,
    tree_pack: bool,
) -> Result<(), NativeRepositoryError> {
    if index.get("pack_id").and_then(JsonValue::as_str) != Some(pack_id) {
        return Err(NativeRepositoryError::bad_request(format!(
            "zstd pack {pack_id} index pack_id mismatch"
        )));
    }
    let expected_format = if tree_pack {
        TREE_PACK_FORMAT_ZSTD_CHUNKED_V1
    } else {
        PACK_FORMAT_ZSTD_CHUNKED_V1
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
    Ok(())
}

pub(in crate::foundation::native_repositories) fn validate_object_pack_entry(
    object_pack_indexes: &BTreeMap<String, JsonValue>,
    pack_id: &str,
    blob_id: &str,
    sha256: &str,
    entry_type: &str,
) -> Result<(), NativeRepositoryError> {
    let entry = object_pack_entry_for_blob_id(object_pack_indexes, pack_id, blob_id)?;
    if entry.checksum != sha256 {
        return Err(NativeRepositoryError::bad_request(format!(
            "Object pack {pack_id} checksum mismatch for blob {blob_id}"
        )));
    }
    if entry.entry_type != entry_type {
        return Err(NativeRepositoryError::bad_request(format!(
            "Object pack {pack_id} entry_type mismatch for blob {blob_id}"
        )));
    }
    Ok(())
}

pub(in crate::foundation::native_repositories) fn validate_tree_pack_entry(
    tree_pack_indexes: &BTreeMap<String, JsonValue>,
    pack_id: &str,
    tree_id: &str,
    entry_count: i32,
    checksum: &str,
) -> Result<(), NativeRepositoryError> {
    let entry = tree_pack_entry_for_tree_id(tree_pack_indexes, pack_id, tree_id)?;
    if entry.entry_count != entry_count as usize {
        return Err(NativeRepositoryError::bad_request(format!(
            "Tree pack {pack_id} entry_count mismatch for tree {tree_id}"
        )));
    }
    if entry.checksum != checksum {
        return Err(NativeRepositoryError::bad_request(format!(
            "Tree pack {pack_id} checksum mismatch for tree {tree_id}"
        )));
    }
    Ok(())
}

pub(in crate::foundation::native_repositories) fn validate_root_tree_locator_index(
    index: &JsonValue,
    pack_id: &str,
    root_entry_ordinal: usize,
) -> Result<(), NativeRepositoryError> {
    tree_pack_entry_for_root_ordinal(index, pack_id, root_entry_ordinal).map(|_| ())
}

pub(in crate::foundation::native_repositories) fn object_pack_entry_for_blob_id(
    object_pack_indexes: &BTreeMap<String, JsonValue>,
    pack_id: &str,
    blob_id: &str,
) -> Result<PackIndexEntry, NativeRepositoryError> {
    let index = object_pack_indexes.get(pack_id).ok_or_else(|| {
        NativeRepositoryError::bad_request(format!(
            "Blob {blob_id} references unknown object pack {pack_id}"
        ))
    })?;
    let entries_by_name = ObjectPackIndexJson::stateless()
        .entries_by_name(index)
        .map_err(NativeRepositoryError::bad_request)?;
    entries_by_name
        .into_values()
        .find(|entry| entry.blob_id == blob_id)
        .ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "Object pack {pack_id} is missing blob {blob_id}"
            ))
        })
}

pub(in crate::foundation::native_repositories) fn tree_pack_entry_for_tree_id(
    tree_pack_indexes: &BTreeMap<String, JsonValue>,
    pack_id: &str,
    tree_id: &str,
) -> Result<TreePackIndexEntry, NativeRepositoryError> {
    let index = tree_pack_indexes.get(pack_id).ok_or_else(|| {
        NativeRepositoryError::bad_request(format!(
            "Tree {tree_id} references unknown tree pack {pack_id}"
        ))
    })?;
    let entries_by_id = TreePackIndexJson::stateless()
        .entries_by_id(index)
        .map_err(NativeRepositoryError::bad_request)?;
    entries_by_id
        .into_values()
        .find(|entry| entry.tree_id == tree_id)
        .ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} is missing tree {tree_id}"
            ))
        })
}

pub(in crate::foundation::native_repositories) fn tree_pack_entry_for_root_ordinal(
    index: &JsonValue,
    pack_id: &str,
    root_entry_ordinal: usize,
) -> Result<TreePackIndexEntry, NativeRepositoryError> {
    let entries_by_id = TreePackIndexJson::stateless()
        .entries_by_id(index)
        .map_err(NativeRepositoryError::bad_request)?;
    entries_by_id
        .into_values()
        .find(|entry| entry.entry_ordinal == root_entry_ordinal)
        .ok_or_else(|| {
            NativeRepositoryError::bad_request(format!(
                "Tree pack {pack_id} is missing root entry ordinal {root_entry_ordinal}"
            ))
        })
}
