use super::*;

pub(in crate::repository_pack_json) fn object_pack_row_to_value(
    row: &ZstdBulkObjectPackRow,
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
    insert_optional_i64(&mut object, "member_count", row.member_count);
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
            .map(object_pack_index_to_value)
            .transpose()?,
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn object_pack_row_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ZstdBulkObjectPackRow, String> {
    Ok(ZstdBulkObjectPackRow {
        generation_key: opt_string(&object, "generation_key")?,
        pack_id: req_string(&object, "pack_id")?,
        repo_name: opt_string(&object, "repo_name")?,
        repo_id: opt_string(&object, "repo_id")?,
        status: opt_string(&object, "status")?,
        pack_format: opt_string(&object, "pack_format")?
            .map(|format| PackFormatKind::from_persisted(&format))
            .transpose()?,
        member_count: opt_i64(&object, "member_count")?,
        total_bytes: opt_i64(&object, "total_bytes")?,
        pack_path: opt_string(&object, "pack_path")?,
        pack_index_entry_name: opt_string(&object, "pack_index_entry_name")?,
        pack_index_checksum: opt_string(&object, "pack_index_checksum")?,
        created_at: opt_string(&object, "created_at")?,
        pack_index: optional_object_value(&object, "pack_index")?
            .map(object_pack_index_from_object)
            .transpose()?,
    })
}

pub(in crate::repository_pack_json) fn object_pack_row_from_value(
    value: JsonValue,
) -> Result<ZstdBulkObjectPackRow, String> {
    object_from_value(value, "zstd object pack row").and_then(object_pack_row_from_object)
}

pub(in crate::repository_pack_json) fn object_pack_index_to_value(
    index: &ObjectPackIndexInventory,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert("pack_id".to_string(), string_value(index.pack_id.clone()));
    object.insert(
        "pack_format".to_string(),
        string_value(index.pack_format.persisted_name()),
    );
    object.insert("member_count".to_string(), number_value(index.member_count));
    object.insert("total_bytes".to_string(), number_value(index.total_bytes));
    object.insert(
        "entries".to_string(),
        object_vec_value(&index.entries, object_pack_index_entry_to_value)?,
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn object_pack_index_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ObjectPackIndexInventory, String> {
    Ok(ObjectPackIndexInventory {
        pack_id: req_string(&object, "pack_id")?,
        pack_format: PackFormatKind::from_persisted(&req_string(&object, "pack_format")?)?,
        member_count: opt_i64(&object, "member_count")?.unwrap_or(0),
        total_bytes: opt_i64(&object, "total_bytes")?.unwrap_or(0),
        entries: object_vec_from_field(&object, "entries", object_pack_index_entry_from_object)?,
    })
}

pub(in crate::repository_pack_json) fn object_pack_index_entry_to_value(
    entry: &ObjectPackIndexEntryInventory,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "entry_name".to_string(),
        string_value(entry.entry_name.clone()),
    );
    object.insert("blob_id".to_string(), string_value(entry.blob_id.clone()));
    object.insert(
        "entry_type".to_string(),
        string_value(entry.entry_type.clone()),
    );
    object.insert("checksum".to_string(), string_value(entry.checksum.clone()));
    insert_optional_string(&mut object, "base_blob_id", &entry.base_blob_id);
    object.insert("chain_depth".to_string(), number_value(entry.chain_depth));
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn object_pack_index_entry_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<ObjectPackIndexEntryInventory, String> {
    Ok(ObjectPackIndexEntryInventory {
        entry_name: req_string(&object, "entry_name")?,
        blob_id: req_string(&object, "blob_id")?,
        entry_type: req_string(&object, "entry_type")?,
        checksum: req_string(&object, "checksum")?,
        base_blob_id: opt_string(&object, "base_blob_id")?,
        chain_depth: opt_i64(&object, "chain_depth")?.unwrap_or(0),
    })
}

pub(in crate::repository_pack_json) fn inventory_object_pack_to_value(
    row: &RepositoryObjectPackInventoryRow,
) -> Result<JsonValue, String> {
    let bulk = ZstdBulkObjectPackRow {
        generation_key: None,
        pack_id: row.pack_id.clone(),
        repo_name: row.repo_name.clone(),
        repo_id: row.repo_id.clone(),
        status: Some(row.status.clone()),
        pack_format: Some(row.pack_format),
        member_count: Some(row.member_count),
        total_bytes: Some(row.total_bytes),
        pack_path: Some(row.pack_path.clone()),
        pack_index_entry_name: Some(row.pack_index_entry_name.clone()),
        pack_index_checksum: Some(row.pack_index_checksum.clone()),
        created_at: Some(row.created_at.clone()),
        pack_index: Some(row.embedded_index.clone()),
    };
    object_pack_row_to_value(&bulk)
}

pub(in crate::repository_pack_json) fn inventory_object_pack_from_object(
    object: JsonMap<String, JsonValue>,
) -> Result<RepositoryObjectPackInventoryRow, String> {
    let row = object_pack_row_from_object(object)?;
    Ok(RepositoryObjectPackInventoryRow {
        pack_id: row.pack_id,
        repo_name: row.repo_name,
        repo_id: row.repo_id,
        status: row.status.unwrap_or_default(),
        pack_format: row
            .pack_format
            .ok_or_else(|| "object pack inventory row requires pack_format.".to_string())?,
        member_count: row.member_count.unwrap_or(0),
        total_bytes: row.total_bytes.unwrap_or(0),
        pack_path: row.pack_path.unwrap_or_default(),
        pack_index_entry_name: row.pack_index_entry_name.unwrap_or_default(),
        pack_index_checksum: row.pack_index_checksum.unwrap_or_default(),
        created_at: row.created_at.unwrap_or_default(),
        embedded_index: row
            .pack_index
            .ok_or_else(|| "object pack inventory row requires pack_index.".to_string())?,
    })
}
