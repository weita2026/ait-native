use super::*;

pub(in crate::repository_pack_json) fn blob_locator_row_to_value(
    row: &ZstdBulkBlobLocatorRow,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "generation_key", &row.generation_key);
    object.insert("blob_id".to_string(), string_value(row.blob_id.clone()));
    insert_optional_string(&mut object, "sha256", &row.sha256);
    insert_optional_string(&mut object, "storage_path", &row.storage_path);
    insert_optional_string(&mut object, "storage_kind", &row.storage_kind);
    insert_optional_i64(&mut object, "size_bytes", row.size_bytes);
    insert_optional_string(&mut object, "pack_id", &row.pack_id);
    insert_optional_string(&mut object, "pack_entry_name", &row.pack_entry_name);
    insert_optional_string(&mut object, "pack_entry_type", &row.pack_entry_type);
    insert_optional_string(&mut object, "pack_base_blob_id", &row.pack_base_blob_id);
    insert_optional_i64(&mut object, "pack_chain_depth", row.pack_chain_depth);
    insert_optional_string(&mut object, "created_at", &row.created_at);
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn blob_locator_row_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ZstdBulkBlobLocatorRow, String> {
    Ok(ZstdBulkBlobLocatorRow {
        generation_key: opt_string(&object, "generation_key")?,
        blob_id: req_string(&object, "blob_id")?,
        sha256: opt_string(&object, "sha256")?,
        storage_path: opt_string(&object, "storage_path")?,
        storage_kind: opt_string(&object, "storage_kind")?,
        size_bytes: opt_i64(&object, "size_bytes")?,
        pack_id: opt_string(&object, "pack_id")?,
        pack_entry_name: opt_string(&object, "pack_entry_name")?,
        pack_entry_type: opt_string(&object, "pack_entry_type")?,
        pack_base_blob_id: opt_string(&object, "pack_base_blob_id")?,
        pack_chain_depth: opt_i64(&object, "pack_chain_depth")?,
        created_at: opt_string(&object, "created_at")?,
    })
}

pub(in crate::repository_pack_json) fn blob_locator_row_from_value(
    value: JsonValue,
) -> Result<ZstdBulkBlobLocatorRow, String> {
    object_from_value(value, "zstd blob locator row").and_then(blob_locator_row_from_object)
}

pub(in crate::repository_pack_json) fn tree_locator_row_to_value(
    row: &ZstdBulkTreeLocatorRow,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "generation_key", &row.generation_key);
    object.insert("tree_id".to_string(), string_value(row.tree_id.clone()));
    insert_optional_i64(&mut object, "entry_count", row.entry_count);
    insert_optional_string(&mut object, "tree_pack_id", &row.tree_pack_id);
    insert_optional_string(&mut object, "tree_pack_checksum", &row.tree_pack_checksum);
    insert_optional_string(&mut object, "created_at", &row.created_at);
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn tree_locator_row_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ZstdBulkTreeLocatorRow, String> {
    Ok(ZstdBulkTreeLocatorRow {
        generation_key: opt_string(&object, "generation_key")?,
        tree_id: req_string(&object, "tree_id")?,
        entry_count: opt_i64(&object, "entry_count")?,
        tree_pack_id: opt_string(&object, "tree_pack_id")?,
        tree_pack_checksum: opt_string(&object, "tree_pack_checksum")?,
        created_at: opt_string(&object, "created_at")?,
    })
}

pub(in crate::repository_pack_json) fn tree_locator_row_from_value(
    value: JsonValue,
) -> Result<ZstdBulkTreeLocatorRow, String> {
    object_from_value(value, "zstd tree locator row").and_then(tree_locator_row_from_object)
}

pub(in crate::repository_pack_json) fn snapshot_row_to_value(
    row: &ZstdBulkSnapshotRow,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "snapshot_id".to_string(),
        string_value(row.snapshot_id.clone()),
    );
    object.insert(
        "parent_snapshot_ids".to_string(),
        string_vec_value(&row.parent_snapshot_ids),
    );
    insert_optional_string(
        &mut object,
        "primary_parent_snapshot_id",
        &row.primary_parent_snapshot_id,
    );
    insert_optional_string(&mut object, "parent_snapshot_id", &row.parent_snapshot_id);
    insert_optional_string(&mut object, "root_tree_pack_id", &row.root_tree_pack_id);
    insert_optional_i64(&mut object, "root_entry_ordinal", row.root_entry_ordinal);
    insert_optional_string(&mut object, "manifest_hash", &row.manifest_hash);
    insert_optional_string(&mut object, "message", &row.message);
    insert_optional_string(&mut object, "line_name", &row.line_name);
    insert_optional_string(&mut object, "snapshot_kind", &row.snapshot_kind);
    insert_optional_i64(&mut object, "file_count", row.file_count);
    insert_optional_i64(&mut object, "total_bytes", row.total_bytes);
    insert_optional_string(&mut object, "created_at", &row.created_at);
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn snapshot_row_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ZstdBulkSnapshotRow, String> {
    let snapshot_id = req_string(&object, "snapshot_id")?;
    let parent_snapshot_ids = object
        .contains_key("parent_snapshot_ids")
        .then(|| string_vec_from_field(&object, "parent_snapshot_ids"))
        .transpose()?;
    let (parent_snapshot_ids, primary_parent_snapshot_id, parent_snapshot_id) =
        normalize_snapshot_parent_set(
            Some(&snapshot_id),
            parent_snapshot_ids,
            opt_string(&object, "primary_parent_snapshot_id")?,
            opt_string(&object, "parent_snapshot_id")?,
        )?;
    Ok(ZstdBulkSnapshotRow {
        snapshot_id,
        parent_snapshot_ids,
        primary_parent_snapshot_id,
        parent_snapshot_id,
        root_tree_pack_id: opt_string(&object, "root_tree_pack_id")?,
        root_entry_ordinal: opt_i64(&object, "root_entry_ordinal")?,
        manifest_hash: opt_string(&object, "manifest_hash")?,
        message: opt_string(&object, "message")?,
        line_name: opt_string(&object, "line_name")?,
        snapshot_kind: opt_string(&object, "snapshot_kind")?,
        file_count: opt_i64(&object, "file_count")?,
        total_bytes: opt_i64(&object, "total_bytes")?,
        created_at: opt_string(&object, "created_at")?,
    })
}

pub(in crate::repository_pack_json) fn snapshot_row_from_value(
    value: JsonValue,
) -> Result<ZstdBulkSnapshotRow, String> {
    object_from_value(value, "zstd snapshot row").and_then(snapshot_row_from_object)
}

pub(in crate::repository_pack_json) fn line_update_to_value(
    update: &ZstdBulkLineUpdate,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "line_name".to_string(),
        string_value(update.line_name.clone()),
    );
    object.insert(
        "head_snapshot_id".to_string(),
        update
            .head_snapshot_id
            .clone()
            .map(string_value)
            .unwrap_or(JsonValue::Null),
    );
    object.insert(
        "expected_head_snapshot_id".to_string(),
        update
            .expected_head_snapshot_id
            .clone()
            .map(string_value)
            .unwrap_or(JsonValue::Null),
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn line_update_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ZstdBulkLineUpdate, String> {
    Ok(ZstdBulkLineUpdate {
        line_name: req_string(&object, "line_name")?,
        head_snapshot_id: opt_string(&object, "head_snapshot_id")?,
        expected_head_snapshot_id: opt_string(&object, "expected_head_snapshot_id")?,
    })
}

pub(in crate::repository_pack_json) fn line_update_result_to_value(
    update: &ZstdBulkLineUpdateResult,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "line_name", &update.line_name);
    insert_optional_string(&mut object, "head_snapshot_id", &update.head_snapshot_id);
    insert_optional_bool(&mut object, "updated", update.updated);
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn line_update_result_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ZstdBulkLineUpdateResult, String> {
    Ok(ZstdBulkLineUpdateResult {
        line_name: opt_string(&object, "line_name")?,
        head_snapshot_id: opt_string(&object, "head_snapshot_id")?,
        updated: opt_bool(&object, "updated")?,
    })
}

pub(in crate::repository_pack_json) fn remote_line_to_value(
    line: &ZstdBulkRemoteLine,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "repo_name", &line.repo_name);
    insert_optional_string(&mut object, "line_name", &line.line_name);
    insert_optional_string(&mut object, "status", &line.status);
    insert_optional_string(&mut object, "head_snapshot_id", &line.head_snapshot_id);
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn remote_line_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ZstdBulkRemoteLine, String> {
    Ok(ZstdBulkRemoteLine {
        repo_name: opt_string(&object, "repo_name")?,
        line_name: opt_string(&object, "line_name")?,
        status: opt_string(&object, "status")?,
        head_snapshot_id: opt_string(&object, "head_snapshot_id")?,
    })
}

pub(in crate::repository_pack_json) fn inventory_blob_locator_to_value(
    row: &RepositoryBlobLocatorInventoryRow,
) -> Result<JsonValue, String> {
    blob_locator_row_to_value(&ZstdBulkBlobLocatorRow {
        generation_key: None,
        blob_id: row.blob_id.clone(),
        sha256: Some(row.sha256.clone()),
        storage_path: None,
        storage_kind: None,
        size_bytes: Some(row.size_bytes),
        pack_id: Some(row.pack_id.clone()),
        pack_entry_name: Some(row.pack_entry_name.clone()),
        pack_entry_type: Some(row.pack_entry_type.clone()),
        pack_base_blob_id: row.pack_base_blob_id.clone(),
        pack_chain_depth: Some(row.pack_chain_depth),
        created_at: Some(row.created_at.clone()),
    })
}

pub(in crate::repository_pack_json) fn inventory_blob_locator_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<RepositoryBlobLocatorInventoryRow, String> {
    let row = blob_locator_row_from_object(object)?;
    Ok(RepositoryBlobLocatorInventoryRow {
        blob_id: row.blob_id,
        sha256: row.sha256.unwrap_or_default(),
        size_bytes: row.size_bytes.unwrap_or(0),
        pack_id: row.pack_id.unwrap_or_default(),
        pack_entry_name: row.pack_entry_name.unwrap_or_default(),
        pack_entry_type: row.pack_entry_type.unwrap_or_default(),
        pack_base_blob_id: row.pack_base_blob_id,
        pack_chain_depth: row.pack_chain_depth.unwrap_or(0),
        created_at: row.created_at.unwrap_or_default(),
    })
}

pub(in crate::repository_pack_json) fn inventory_tree_locator_to_value(
    row: &RepositoryTreeLocatorInventoryRow,
) -> Result<JsonValue, String> {
    tree_locator_row_to_value(&ZstdBulkTreeLocatorRow {
        generation_key: None,
        tree_id: row.tree_id.clone(),
        entry_count: Some(row.entry_count),
        tree_pack_id: Some(row.tree_pack_id.clone()),
        tree_pack_checksum: Some(row.tree_pack_checksum.clone()),
        created_at: Some(row.created_at.clone()),
    })
}

pub(in crate::repository_pack_json) fn inventory_tree_locator_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<RepositoryTreeLocatorInventoryRow, String> {
    let row = tree_locator_row_from_object(object)?;
    Ok(RepositoryTreeLocatorInventoryRow {
        tree_id: row.tree_id,
        entry_count: row.entry_count.unwrap_or(0),
        tree_pack_id: row.tree_pack_id.unwrap_or_default(),
        tree_pack_checksum: row.tree_pack_checksum.unwrap_or_default(),
        created_at: row.created_at.unwrap_or_default(),
    })
}

pub(in crate::repository_pack_json) fn inventory_snapshot_to_value(
    row: &RepositorySnapshotInventoryRow,
) -> Result<JsonValue, String> {
    snapshot_row_to_value(&ZstdBulkSnapshotRow {
        snapshot_id: row.snapshot_id.clone(),
        parent_snapshot_ids: row.parent_snapshot_ids.clone(),
        primary_parent_snapshot_id: row.primary_parent_snapshot_id.clone(),
        parent_snapshot_id: row.parent_snapshot_id.clone(),
        root_tree_pack_id: Some(row.root_tree_pack_id.clone()),
        root_entry_ordinal: Some(row.root_entry_ordinal),
        manifest_hash: Some(row.manifest_hash.clone()),
        message: row.message.clone(),
        line_name: row.line_name.clone(),
        snapshot_kind: row.snapshot_kind.clone(),
        file_count: Some(row.file_count),
        total_bytes: Some(row.total_bytes),
        created_at: Some(row.created_at.clone()),
    })
}

pub(in crate::repository_pack_json) fn inventory_snapshot_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<RepositorySnapshotInventoryRow, String> {
    let row = snapshot_row_from_object(object)?;
    Ok(RepositorySnapshotInventoryRow {
        snapshot_id: row.snapshot_id,
        parent_snapshot_ids: row.parent_snapshot_ids,
        primary_parent_snapshot_id: row.primary_parent_snapshot_id,
        parent_snapshot_id: row.parent_snapshot_id,
        root_tree_pack_id: row.root_tree_pack_id.unwrap_or_default(),
        root_entry_ordinal: row.root_entry_ordinal.unwrap_or(0),
        manifest_hash: row.manifest_hash.unwrap_or_default(),
        message: row.message,
        line_name: row.line_name,
        snapshot_kind: row.snapshot_kind,
        file_count: row.file_count.unwrap_or(0),
        total_bytes: row.total_bytes.unwrap_or(0),
        created_at: row.created_at.unwrap_or_default(),
    })
}

pub(in crate::repository_pack_json) fn inventory_line_head_to_value(
    row: &RepositoryLineHeadInventoryRow,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert("line_name".to_string(), string_value(row.line_name.clone()));
    insert_optional_string(&mut object, "head_snapshot_id", &row.head_snapshot_id);
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn inventory_line_head_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<RepositoryLineHeadInventoryRow, String> {
    Ok(RepositoryLineHeadInventoryRow {
        line_name: req_string(&object, "line_name")?,
        head_snapshot_id: opt_string(&object, "head_snapshot_id")?,
    })
}
