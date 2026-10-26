use super::*;

pub(in crate::repository_pack_json) fn tree_pack_row_to_value(
    row: &ZstdBulkTreePackRow,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    insert_optional_string(&mut object, "generation_key", &row.generation_key);
    object.insert("pack_id".to_string(), string_value(row.pack_id.clone()));
    insert_optional_string(&mut object, "repo_name", &row.repo_name);
    insert_optional_string(&mut object, "repo_id", &row.repo_id);
    insert_optional_string(&mut object, "status", &row.status);
    if let Some(format) = row.pack_format {
        object.insert(
            "pack_format".to_string(),
            string_value(format.persisted_name()),
        );
    }
    insert_optional_i64(&mut object, "tree_count", row.tree_count);
    insert_optional_i64(&mut object, "total_bytes", row.total_bytes);
    insert_optional_string(&mut object, "pack_path", &row.pack_path);
    insert_optional_string(
        &mut object,
        "pack_index_entry_name",
        &row.pack_index_entry_name,
    );
    insert_optional_string(&mut object, "pack_index_checksum", &row.pack_index_checksum);
    insert_optional_string(&mut object, "created_at", &row.created_at);
    insert_optional_value(
        &mut object,
        "pack_index",
        row.pack_index
            .as_ref()
            .map(tree_pack_index_to_value)
            .transpose()?,
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn tree_pack_row_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ZstdBulkTreePackRow, String> {
    Ok(ZstdBulkTreePackRow {
        generation_key: opt_string(&object, "generation_key")?,
        pack_id: req_string(&object, "pack_id")?,
        repo_name: opt_string(&object, "repo_name")?,
        repo_id: opt_string(&object, "repo_id")?,
        status: opt_string(&object, "status")?,
        pack_format: opt_string(&object, "pack_format")?
            .map(|format| TreePackFormatKind::from_persisted(&format))
            .transpose()?,
        tree_count: opt_i64(&object, "tree_count")?,
        total_bytes: opt_i64(&object, "total_bytes")?,
        pack_path: opt_string(&object, "pack_path")?,
        pack_index_entry_name: opt_string(&object, "pack_index_entry_name")?,
        pack_index_checksum: opt_string(&object, "pack_index_checksum")?,
        created_at: opt_string(&object, "created_at")?,
        pack_index: optional_object_value(&object, "pack_index")?
            .map(tree_pack_index_from_object)
            .transpose()?,
    })
}

pub(in crate::repository_pack_json) fn tree_pack_row_from_value(
    value: JsonValue,
) -> Result<ZstdBulkTreePackRow, String> {
    object_from_value(value, "zstd tree pack row").and_then(tree_pack_row_from_object)
}

pub(in crate::repository_pack_json) fn tree_pack_index_to_value(
    index: &TreePackIndexInventory,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert("pack_id".to_string(), string_value(index.pack_id.clone()));
    object.insert(
        "pack_format".to_string(),
        string_value(index.pack_format.persisted_name()),
    );
    object.insert("tree_count".to_string(), number_value(index.tree_count));
    object.insert("total_bytes".to_string(), number_value(index.total_bytes));
    object.insert(
        "trees".to_string(),
        object_vec_value(&index.trees, tree_pack_index_entry_to_value)?,
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn tree_pack_index_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<TreePackIndexInventory, String> {
    Ok(TreePackIndexInventory {
        pack_id: req_string(&object, "pack_id")?,
        pack_format: TreePackFormatKind::from_persisted(&req_string(&object, "pack_format")?)?,
        tree_count: opt_i64(&object, "tree_count")?.unwrap_or(0),
        total_bytes: opt_i64(&object, "total_bytes")?.unwrap_or(0),
        trees: object_vec_from_field(&object, "trees", tree_pack_index_entry_from_object)?,
    })
}

pub(in crate::repository_pack_json) fn tree_pack_index_entry_to_value(
    entry: &TreePackIndexEntryInventory,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert("tree_id".to_string(), string_value(entry.tree_id.clone()));
    object.insert(
        "entry_ordinal".to_string(),
        number_value(entry.entry_ordinal),
    );
    object.insert("entry_count".to_string(), number_value(entry.entry_count));
    object.insert("checksum".to_string(), string_value(entry.checksum.clone()));
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn tree_pack_index_entry_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<TreePackIndexEntryInventory, String> {
    Ok(TreePackIndexEntryInventory {
        tree_id: req_string(&object, "tree_id")?,
        entry_ordinal: opt_i64(&object, "entry_ordinal")?.unwrap_or(0),
        entry_count: opt_i64(&object, "entry_count")?.unwrap_or(0),
        checksum: req_string(&object, "checksum")?,
    })
}

pub(in crate::repository_pack_json) fn inventory_tree_pack_to_value(
    row: &RepositoryTreePackInventoryRow,
) -> Result<JsonValue, String> {
    let bulk = ZstdBulkTreePackRow {
        generation_key: None,
        pack_id: row.pack_id.clone(),
        repo_name: row.repo_name.clone(),
        repo_id: row.repo_id.clone(),
        status: Some(row.status.clone()),
        pack_format: Some(row.pack_format),
        tree_count: Some(row.tree_count),
        total_bytes: Some(row.total_bytes),
        pack_path: Some(row.pack_path.clone()),
        pack_index_entry_name: Some(row.pack_index_entry_name.clone()),
        pack_index_checksum: Some(row.pack_index_checksum.clone()),
        created_at: Some(row.created_at.clone()),
        pack_index: Some(row.embedded_index.clone()),
    };
    tree_pack_row_to_value(&bulk)
}

pub(in crate::repository_pack_json) fn inventory_tree_pack_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<RepositoryTreePackInventoryRow, String> {
    let row = tree_pack_row_from_object(object)?;
    Ok(RepositoryTreePackInventoryRow {
        pack_id: row.pack_id,
        repo_name: row.repo_name,
        repo_id: row.repo_id,
        status: row.status.unwrap_or_default(),
        pack_format: row
            .pack_format
            .ok_or_else(|| "tree pack inventory row requires pack_format.".to_string())?,
        tree_count: row.tree_count.unwrap_or(0),
        total_bytes: row.total_bytes.unwrap_or(0),
        pack_path: row.pack_path.unwrap_or_default(),
        pack_index_entry_name: row.pack_index_entry_name.unwrap_or_default(),
        pack_index_checksum: row.pack_index_checksum.unwrap_or_default(),
        created_at: row.created_at.unwrap_or_default(),
        embedded_index: row
            .pack_index
            .ok_or_else(|| "tree pack inventory row requires pack_index.".to_string())?,
    })
}
