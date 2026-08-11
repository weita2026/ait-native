use super::*;

pub(in crate::repository_pack_json) fn pull_manifest_request_to_value(
    request: &ZstdPullManifestRequest,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "contract".to_string(),
        string_value(request.contract.clone()),
    );
    object.insert(
        "head_snapshot_id".to_string(),
        string_value(request.head_snapshot_id.clone()),
    );
    object.insert(
        "have_snapshot_ids".to_string(),
        string_vec_value(&request.have_snapshot_ids),
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn pull_manifest_request_from_value(
    value: JsonValue,
) -> Result<ZstdPullManifestRequest, String> {
    let object = object_from_value(value, "zstd pull manifest request")?;
    Ok(ZstdPullManifestRequest {
        contract: req_string(&object, "contract")?,
        head_snapshot_id: req_string(&object, "head_snapshot_id")?,
        have_snapshot_ids: string_vec_from_field(&object, "have_snapshot_ids")?,
    })
}

pub(in crate::repository_pack_json) fn validate_pull_manifest_request(
    request: &ZstdPullManifestRequest,
) -> Result<(), String> {
    if request.contract != ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME {
        return Err(format!(
            "Zstd pull manifest request contract must be `{ZSTD_PULL_MANIFEST_REQUEST_CONTRACT_NAME}`."
        ));
    }
    validate_nonempty(&request.head_snapshot_id, "head snapshot id")?;
    validate_unique_nonempty_ids(&request.have_snapshot_ids, "have snapshot id")
}

pub(in crate::repository_pack_json) fn pull_manifest_to_value(
    payload: &ZstdPullManifestPayload,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "contract".to_string(),
        string_value(payload.contract.clone()),
    );
    object.insert(
        "repo_name".to_string(),
        string_value(payload.repo_name.clone()),
    );
    object.insert(
        "head_snapshot_id".to_string(),
        string_value(payload.head_snapshot_id.clone()),
    );
    object.insert(
        "boundary_snapshot_ids".to_string(),
        string_vec_value(&payload.boundary_snapshot_ids),
    );
    object.insert(
        "snapshots".to_string(),
        object_vec_value(&payload.snapshots, snapshot_row_to_value)?,
    );
    object.insert(
        "object_packs".to_string(),
        object_vec_value(&payload.object_packs, object_pack_row_to_value)?,
    );
    object.insert(
        "tree_packs".to_string(),
        object_vec_value(&payload.tree_packs, tree_pack_row_to_value)?,
    );
    object.insert(
        "blob_locators".to_string(),
        object_vec_value(&payload.blob_locators, blob_locator_row_to_value)?,
    );
    object.insert(
        "tree_locators".to_string(),
        object_vec_value(&payload.tree_locators, tree_locator_row_to_value)?,
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn pull_manifest_from_value(
    value: JsonValue,
) -> Result<ZstdPullManifestPayload, String> {
    let object = object_from_value(value, "zstd pull manifest payload")?;
    Ok(ZstdPullManifestPayload {
        contract: req_string(&object, "contract")?,
        repo_name: req_string(&object, "repo_name")?,
        head_snapshot_id: req_string(&object, "head_snapshot_id")?,
        boundary_snapshot_ids: string_vec_from_field(&object, "boundary_snapshot_ids")?,
        snapshots: object_vec_from_field(&object, "snapshots", snapshot_row_from_object)?,
        object_packs: object_vec_from_field(&object, "object_packs", object_pack_row_from_object)?,
        tree_packs: object_vec_from_field(&object, "tree_packs", tree_pack_row_from_object)?,
        blob_locators: object_vec_from_field(
            &object,
            "blob_locators",
            blob_locator_row_from_object,
        )?,
        tree_locators: object_vec_from_field(
            &object,
            "tree_locators",
            tree_locator_row_from_object,
        )?,
    })
}

pub(in crate::repository_pack_json) fn validate_pull_manifest(
    payload: &ZstdPullManifestPayload,
) -> Result<(), String> {
    if payload.contract != ZSTD_PULL_MANIFEST_CONTRACT_NAME {
        return Err(format!(
            "Zstd pull manifest contract must be `{ZSTD_PULL_MANIFEST_CONTRACT_NAME}`."
        ));
    }
    validate_nonempty(&payload.repo_name, "repository name")?;
    validate_nonempty(&payload.head_snapshot_id, "head snapshot id")?;
    validate_unique_nonempty_ids(&payload.boundary_snapshot_ids, "boundary snapshot id")?;
    let boundary_ids = payload
        .boundary_snapshot_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();

    let mut snapshot_ids = BTreeSet::new();
    for row in &payload.snapshots {
        validate_zstd_import_manifest_snapshot_row(row)?;
        if !snapshot_ids.insert(row.snapshot_id.clone()) {
            return Err(format!(
                "Zstd pull manifest contains duplicate snapshot {}.",
                row.snapshot_id
            ));
        }
        if boundary_ids.contains(&row.snapshot_id) {
            return Err(format!(
                "Zstd pull manifest snapshot {} is also declared as a boundary.",
                row.snapshot_id
            ));
        }
        for parent in &row.parent_snapshot_ids {
            if !snapshot_ids.contains(parent) && !boundary_ids.contains(parent) {
                return Err(format!(
                    "Zstd pull manifest snapshot {} appears before missing parent {} or omits its boundary.",
                    row.snapshot_id, parent
                ));
            }
        }
    }
    match payload.snapshots.last() {
        Some(snapshot) if snapshot.snapshot_id != payload.head_snapshot_id => {
            return Err(format!(
                "Zstd pull manifest final snapshot {} must match requested head {}.",
                snapshot.snapshot_id, payload.head_snapshot_id
            ));
        }
        None if !boundary_ids.contains(&payload.head_snapshot_id) => {
            return Err(
                "Empty Zstd pull manifest must declare the requested head as a boundary."
                    .to_string(),
            );
        }
        _ => {}
    }

    let mut object_pack_ids = BTreeSet::new();
    for row in &payload.object_packs {
        validate_zstd_import_manifest_object_pack_row(row)?;
        if !object_pack_ids.insert(row.pack_id.clone()) {
            return Err(format!(
                "Zstd pull manifest contains duplicate object pack {}.",
                row.pack_id
            ));
        }
    }
    let mut tree_pack_ids = BTreeSet::new();
    for row in &payload.tree_packs {
        validate_zstd_import_manifest_tree_pack_row(row)?;
        if !tree_pack_ids.insert(row.pack_id.clone()) {
            return Err(format!(
                "Zstd pull manifest contains duplicate tree pack {}.",
                row.pack_id
            ));
        }
    }
    let required_root_pack_ids = payload
        .snapshots
        .iter()
        .map(|snapshot| {
            required_optional_string(&snapshot.root_tree_pack_id, "snapshots[].root_tree_pack_id")
                .map(str::to_string)
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    for root_pack_id in &required_root_pack_ids {
        if !tree_pack_ids.contains(root_pack_id) {
            return Err(format!(
                "Zstd pull manifest snapshot root tree pack `{root_pack_id}` is missing from tree_packs."
            ));
        }
    }

    let mut blob_ids = BTreeSet::new();
    for row in &payload.blob_locators {
        validate_zstd_import_manifest_blob_locator_row(row)?;
        if !blob_ids.insert(row.blob_id.clone()) {
            return Err(format!(
                "Zstd pull manifest contains duplicate blob locator {}.",
                row.blob_id
            ));
        }
        let pack_id = required_optional_string(&row.pack_id, "blob_locators[].pack_id")?;
        if !object_pack_ids.contains(pack_id) {
            return Err(format!(
                "Zstd pull manifest blob locator references missing object pack `{pack_id}`."
            ));
        }
    }
    let mut tree_ids = BTreeSet::new();
    let mut located_tree_pack_ids = BTreeSet::new();
    for row in &payload.tree_locators {
        validate_zstd_import_manifest_tree_locator_row(row)?;
        if !tree_ids.insert(row.tree_id.clone()) {
            return Err(format!(
                "Zstd pull manifest contains duplicate tree locator {}.",
                row.tree_id
            ));
        }
        let pack_id = required_optional_string(&row.tree_pack_id, "tree_locators[].tree_pack_id")?;
        if !tree_pack_ids.contains(pack_id) {
            return Err(format!(
                "Zstd pull manifest tree locator references missing tree pack `{pack_id}`."
            ));
        }
        located_tree_pack_ids.insert(pack_id.to_string());
    }
    for root_pack_id in required_root_pack_ids {
        if !located_tree_pack_ids.contains(&root_pack_id) {
            return Err(format!(
                "Zstd pull manifest snapshot root tree pack `{root_pack_id}` is missing from tree_locators."
            ));
        }
    }
    Ok(())
}

fn validate_unique_nonempty_ids(values: &[String], field: &str) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        validate_nonempty(value, field)?;
        if !seen.insert(value.clone()) {
            return Err(format!("Duplicate {field}: {value}."));
        }
    }
    Ok(())
}

pub(in crate::repository_pack_json) fn import_manifest_to_value(
    payload: &ZstdImportManifestPayload,
) -> Result<JsonValue, String> {
    let mut object = JsonMap::new();
    object.insert(
        "contract".to_string(),
        string_value(payload.contract.clone()),
    );
    object.insert(
        "repo_name".to_string(),
        string_value(payload.repo_name.clone()),
    );
    object.insert(
        "snapshot_id".to_string(),
        string_value(payload.snapshot_id.clone()),
    );
    object.insert(
        "snapshots".to_string(),
        object_vec_value(&payload.snapshots, snapshot_row_to_value)?,
    );
    object.insert(
        "object_packs".to_string(),
        object_vec_value(&payload.object_packs, object_pack_row_to_value)?,
    );
    object.insert(
        "tree_packs".to_string(),
        object_vec_value(&payload.tree_packs, tree_pack_row_to_value)?,
    );
    object.insert(
        "blob_locators".to_string(),
        object_vec_value(&payload.blob_locators, blob_locator_row_to_value)?,
    );
    object.insert(
        "tree_locators".to_string(),
        object_vec_value(&payload.tree_locators, tree_locator_row_to_value)?,
    );
    object.insert(
        "line_update".to_string(),
        payload
            .line_update
            .as_ref()
            .map(line_update_to_value)
            .transpose()?
            .unwrap_or(JsonValue::Null),
    );
    Ok(JsonValue::Object(object))
}

pub(in crate::repository_pack_json) fn import_manifest_from_value(
    value: JsonValue,
) -> Result<ZstdImportManifestPayload, String> {
    let object = object_from_value(value, "zstd import manifest payload")?;
    Ok(ZstdImportManifestPayload {
        contract: req_string(&object, "contract")?,
        repo_name: req_string(&object, "repo_name")?,
        snapshot_id: req_string(&object, "snapshot_id")?,
        snapshots: object_vec_from_field(&object, "snapshots", snapshot_row_from_object)?,
        object_packs: object_vec_from_field(&object, "object_packs", object_pack_row_from_object)?,
        tree_packs: object_vec_from_field(&object, "tree_packs", tree_pack_row_from_object)?,
        blob_locators: object_vec_from_field(
            &object,
            "blob_locators",
            blob_locator_row_from_object,
        )?,
        tree_locators: object_vec_from_field(
            &object,
            "tree_locators",
            tree_locator_row_from_object,
        )?,
        line_update: optional_object_value(&object, "line_update")?
            .map(line_update_from_object)
            .transpose()?,
    })
}

pub(in crate::repository_pack_json) fn validate_import_manifest(
    payload: &ZstdImportManifestPayload,
) -> Result<(), String> {
    if payload.contract != ZSTD_IMPORT_MANIFEST_CONTRACT_NAME {
        return Err(format!(
            "Zstd import manifest contract must be `{ZSTD_IMPORT_MANIFEST_CONTRACT_NAME}`."
        ));
    }
    validate_nonempty(&payload.repo_name, "repository name")?;
    validate_nonempty(&payload.snapshot_id, "snapshot id")?;
    if payload.snapshots.len() != 1 {
        return Err("Zstd import manifest must contain exactly one snapshot row.".to_string());
    }
    let snapshot = &payload.snapshots[0];
    validate_zstd_import_manifest_snapshot_row(snapshot)?;
    if snapshot.snapshot_id != payload.snapshot_id {
        return Err(
            "Zstd import manifest snapshot row must match requested snapshot id.".to_string(),
        );
    }
    let mut object_pack_ids = BTreeSet::new();
    for row in &payload.object_packs {
        validate_zstd_import_manifest_object_pack_row(row)?;
        object_pack_ids.insert(row.pack_id.clone());
    }
    let mut tree_pack_ids = BTreeSet::new();
    for row in &payload.tree_packs {
        validate_zstd_import_manifest_tree_pack_row(row)?;
        tree_pack_ids.insert(row.pack_id.clone());
    }
    let root_tree_pack_id =
        required_optional_string(&snapshot.root_tree_pack_id, "snapshots[].root_tree_pack_id")?;
    if !tree_pack_ids.contains(root_tree_pack_id) {
        return Err(format!(
            "Zstd import manifest snapshot root tree pack `{root_tree_pack_id}` is missing from tree_packs."
        ));
    }
    let mut has_root_tree_locator = false;
    for row in &payload.blob_locators {
        validate_zstd_import_manifest_blob_locator_row(row)?;
        let pack_id = required_optional_string(&row.pack_id, "blob_locators[].pack_id")?;
        if !object_pack_ids.contains(pack_id) {
            return Err(format!(
                "Zstd import manifest blob locator references missing object pack `{pack_id}`."
            ));
        }
    }
    for row in &payload.tree_locators {
        validate_zstd_import_manifest_tree_locator_row(row)?;
        let pack_id = required_optional_string(&row.tree_pack_id, "tree_locators[].tree_pack_id")?;
        if !tree_pack_ids.contains(pack_id) {
            return Err(format!(
                "Zstd import manifest tree locator references missing tree pack `{pack_id}`."
            ));
        }
        if pack_id == root_tree_pack_id {
            has_root_tree_locator = true;
        }
    }
    if !has_root_tree_locator {
        return Err(format!(
            "Zstd import manifest snapshot root tree pack `{root_tree_pack_id}` is missing from tree_locators."
        ));
    }
    if let Some(update) = &payload.line_update {
        validate_nonempty(&update.line_name, "line update line name")?;
        if let Some(snapshot_id) = &update.head_snapshot_id {
            validate_nonempty(snapshot_id, "line update head snapshot id")?;
        }
        if let Some(snapshot_id) = &update.expected_head_snapshot_id {
            validate_nonempty(snapshot_id, "line update expected head snapshot id")?;
        }
    }
    Ok(())
}

pub(crate) fn validate_zstd_import_manifest_object_pack_row(
    row: &ZstdBulkObjectPackRow,
) -> Result<(), String> {
    validate_no_manifest_field(&row.generation_key, "object_packs[].generation_key")?;
    validate_no_manifest_field(&row.repo_name, "object_packs[].repo_name")?;
    validate_no_manifest_field(&row.repo_id, "object_packs[].repo_id")?;
    validate_no_manifest_field(&row.status, "object_packs[].status")?;
    validate_no_manifest_field(&row.pack_path, "object_packs[].pack_path")?;
    if row.pack_index.is_some() {
        return Err(
            "Zstd import manifest object_packs[] must not include embedded pack_index.".to_string(),
        );
    }
    validate_nonempty(&row.pack_id, "object pack id")?;
    match row.pack_format {
        Some(PackFormatKind::ZstdChunkedV1) => {}
        None => return Err("Missing required field `object_packs[].pack_format`.".to_string()),
    }
    required_nonnegative_i64(row.member_count, "object_packs[].member_count")?;
    required_nonnegative_i64(row.total_bytes, "object_packs[].total_bytes")?;
    required_optional_string(
        &row.pack_index_entry_name,
        "object_packs[].pack_index_entry_name",
    )?;
    required_optional_string(
        &row.pack_index_checksum,
        "object_packs[].pack_index_checksum",
    )?;
    required_optional_string(&row.created_at, "object_packs[].created_at")?;
    Ok(())
}

pub(crate) fn validate_zstd_import_manifest_tree_pack_row(
    row: &ZstdBulkTreePackRow,
) -> Result<(), String> {
    validate_no_manifest_field(&row.generation_key, "tree_packs[].generation_key")?;
    validate_no_manifest_field(&row.repo_name, "tree_packs[].repo_name")?;
    validate_no_manifest_field(&row.repo_id, "tree_packs[].repo_id")?;
    validate_no_manifest_field(&row.status, "tree_packs[].status")?;
    validate_no_manifest_field(&row.pack_path, "tree_packs[].pack_path")?;
    if row.pack_index.is_some() {
        return Err(
            "Zstd import manifest tree_packs[] must not include embedded pack_index.".to_string(),
        );
    }
    validate_nonempty(&row.pack_id, "tree pack id")?;
    match row.pack_format {
        Some(TreePackFormatKind::ZstdChunkedTreeV1) => {}
        None => return Err("Missing required field `tree_packs[].pack_format`.".to_string()),
    }
    required_nonnegative_i64(row.tree_count, "tree_packs[].tree_count")?;
    required_nonnegative_i64(row.total_bytes, "tree_packs[].total_bytes")?;
    required_optional_string(
        &row.pack_index_entry_name,
        "tree_packs[].pack_index_entry_name",
    )?;
    required_optional_string(&row.pack_index_checksum, "tree_packs[].pack_index_checksum")?;
    required_optional_string(&row.created_at, "tree_packs[].created_at")?;
    Ok(())
}

pub(in crate::repository_pack_json) fn validate_zstd_import_manifest_blob_locator_row(
    row: &ZstdBulkBlobLocatorRow,
) -> Result<(), String> {
    validate_no_manifest_field(&row.generation_key, "blob_locators[].generation_key")?;
    validate_no_manifest_field(&row.storage_path, "blob_locators[].storage_path")?;
    validate_no_manifest_field(&row.storage_kind, "blob_locators[].storage_kind")?;
    validate_nonempty(&row.blob_id, "blob id")?;
    required_optional_string(&row.sha256, "blob_locators[].sha256")?;
    required_nonnegative_i64(row.size_bytes, "blob_locators[].size_bytes")?;
    required_optional_string(&row.pack_id, "blob_locators[].pack_id")?;
    required_optional_string(&row.pack_entry_name, "blob_locators[].pack_entry_name")?;
    required_optional_string(&row.pack_entry_type, "blob_locators[].pack_entry_type")?;
    if let Some(base_blob_id) = &row.pack_base_blob_id {
        validate_nonempty(base_blob_id, "blob locator base blob id")?;
    }
    required_nonnegative_i64(row.pack_chain_depth, "blob_locators[].pack_chain_depth")?;
    required_optional_string(&row.created_at, "blob_locators[].created_at")?;
    Ok(())
}

pub(in crate::repository_pack_json) fn validate_zstd_import_manifest_tree_locator_row(
    row: &ZstdBulkTreeLocatorRow,
) -> Result<(), String> {
    validate_no_manifest_field(&row.generation_key, "tree_locators[].generation_key")?;
    validate_nonempty(&row.tree_id, "tree id")?;
    required_nonnegative_i64(row.entry_count, "tree_locators[].entry_count")?;
    required_optional_string(&row.tree_pack_id, "tree_locators[].tree_pack_id")?;
    required_optional_string(
        &row.tree_pack_checksum,
        "tree_locators[].tree_pack_checksum",
    )?;
    required_optional_string(&row.created_at, "tree_locators[].created_at")?;
    Ok(())
}

pub(crate) fn validate_zstd_import_manifest_snapshot_row(
    row: &ZstdBulkSnapshotRow,
) -> Result<(), String> {
    validate_nonempty(&row.snapshot_id, "snapshot id")?;
    crate::snapshot_store::validate_snapshot_parent_set(
        Some(&row.snapshot_id),
        &row.parent_snapshot_ids,
        row.primary_parent_snapshot_id.as_deref(),
        row.parent_snapshot_id.as_deref(),
    )?;
    required_optional_string(&row.root_tree_pack_id, "snapshots[].root_tree_pack_id")?;
    required_nonnegative_i64(row.root_entry_ordinal, "snapshots[].root_entry_ordinal")?;
    required_optional_string(&row.manifest_hash, "snapshots[].manifest_hash")?;
    if let Some(message) = &row.message {
        validate_nonempty(message, "snapshot message")?;
    }
    if let Some(line_name) = &row.line_name {
        validate_nonempty(line_name, "snapshot line name")?;
    }
    required_optional_string(&row.snapshot_kind, "snapshots[].snapshot_kind")?;
    required_nonnegative_i64(row.file_count, "snapshots[].file_count")?;
    required_nonnegative_i64(row.total_bytes, "snapshots[].total_bytes")?;
    required_optional_string(&row.created_at, "snapshots[].created_at")?;
    Ok(())
}

pub(in crate::repository_pack_json) fn validate_no_manifest_field(
    value: &Option<String>,
    field: &str,
) -> Result<(), String> {
    if value.is_some() {
        Err(format!("Zstd import manifest must not include `{field}`."))
    } else {
        Ok(())
    }
}

pub(in crate::repository_pack_json) fn required_optional_string<'a>(
    value: &'a Option<String>,
    field: &str,
) -> Result<&'a str, String> {
    let value = value
        .as_ref()
        .ok_or_else(|| format!("Missing required field `{field}`."))?;
    validate_nonempty(value, field)?;
    Ok(value)
}

pub(in crate::repository_pack_json) fn required_nonnegative_i64(
    value: Option<i64>,
    field: &str,
) -> Result<i64, String> {
    let value = value.ok_or_else(|| format!("Missing required field `{field}`."))?;
    if value < 0 {
        Err(format!("Field `{field}` must be non-negative."))
    } else {
        Ok(value)
    }
}
