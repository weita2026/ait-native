use super::*;

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
