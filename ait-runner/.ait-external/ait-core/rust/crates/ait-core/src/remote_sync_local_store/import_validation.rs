use super::*;

pub(super) fn local_object_pack_matches_manifest(
    ctx: &RemoteSyncLocalStoreContext,
    store: &dyn ObjectPackStore,
    pack: &ZstdBulkObjectPackRow,
) -> Result<bool, String> {
    Ok(local_object_pack_validation_stamp(ctx, store, pack)?.is_some())
}

pub(super) fn local_object_pack_validation_stamp(
    ctx: &RemoteSyncLocalStoreContext,
    store: &dyn ObjectPackStore,
    pack: &ZstdBulkObjectPackRow,
) -> Result<Option<LocalPackValidationStamp>, String> {
    let pack_path = default_object_pack_relative_path(&pack.pack_id);
    let db_path = store
        .get_object_pack(&pack.pack_id)?
        .map(|record| record.pack_path)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| pack_path.clone());
    if db_path != pack_path {
        return Ok(None);
    }
    let pack_abs_path = repo_stored_path(ctx, &pack_path);
    if !pack_abs_path.is_file() {
        return Ok(None);
    }
    let before = pack_file_identity(&pack_abs_path)?;
    validate_object_pack_file_matches_manifest(&pack_abs_path, pack)?;
    let after = pack_file_identity(&pack_abs_path)?;
    if before != after {
        return Err(format!(
            "Object pack {} changed while it was being validated.",
            pack.pack_id
        ));
    }
    Ok(Some(LocalPackValidationStamp {
        pack_id: pack.pack_id.clone(),
        pack_path: pack_abs_path,
        expected_index_checksum: required_option_string(
            &pack.pack_index_checksum,
            "object_packs[].pack_index_checksum",
        )?
        .to_string(),
        file_identity: after,
    }))
}

pub(super) fn local_tree_pack_matches_manifest(
    ctx: &RemoteSyncLocalStoreContext,
    store: &dyn TreePackStore,
    pack: &ZstdBulkTreePackRow,
) -> Result<bool, String> {
    Ok(local_tree_pack_validation_stamp(ctx, store, pack)?.is_some())
}

pub(super) fn local_tree_pack_validation_stamp(
    ctx: &RemoteSyncLocalStoreContext,
    store: &dyn TreePackStore,
    pack: &ZstdBulkTreePackRow,
) -> Result<Option<LocalPackValidationStamp>, String> {
    let pack_path = default_tree_pack_relative_path(&pack.pack_id);
    let db_path = store
        .get_tree_pack(&pack.pack_id)?
        .map(|record| record.pack_path)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| pack_path.clone());
    if db_path != pack_path {
        return Ok(None);
    }
    let pack_abs_path = repo_stored_path(ctx, &pack_path);
    if !pack_abs_path.is_file() {
        return Ok(None);
    }
    let before = pack_file_identity(&pack_abs_path)?;
    validate_tree_pack_file_matches_manifest(&pack_abs_path, pack)?;
    let after = pack_file_identity(&pack_abs_path)?;
    if before != after {
        return Err(format!(
            "Tree pack {} changed while it was being validated.",
            pack.pack_id
        ));
    }
    Ok(Some(LocalPackValidationStamp {
        pack_id: pack.pack_id.clone(),
        pack_path: pack_abs_path,
        expected_index_checksum: required_option_string(
            &pack.pack_index_checksum,
            "tree_packs[].pack_index_checksum",
        )?
        .to_string(),
        file_identity: after,
    }))
}

pub(super) fn object_pack_validation_stamp_is_current(
    ctx: &RemoteSyncLocalStoreContext,
    pack: &ZstdBulkObjectPackRow,
    stamp: &LocalPackValidationStamp,
) -> Result<bool, String> {
    pack_validation_stamp_is_current(
        ctx,
        &pack.pack_id,
        &default_object_pack_relative_path(&pack.pack_id),
        required_option_string(
            &pack.pack_index_checksum,
            "object_packs[].pack_index_checksum",
        )?,
        stamp,
    )
}

pub(super) fn tree_pack_validation_stamp_is_current(
    ctx: &RemoteSyncLocalStoreContext,
    pack: &ZstdBulkTreePackRow,
    stamp: &LocalPackValidationStamp,
) -> Result<bool, String> {
    pack_validation_stamp_is_current(
        ctx,
        &pack.pack_id,
        &default_tree_pack_relative_path(&pack.pack_id),
        required_option_string(
            &pack.pack_index_checksum,
            "tree_packs[].pack_index_checksum",
        )?,
        stamp,
    )
}

fn pack_validation_stamp_is_current(
    ctx: &RemoteSyncLocalStoreContext,
    pack_id: &str,
    pack_rel_path: &str,
    expected_index_checksum: &str,
    stamp: &LocalPackValidationStamp,
) -> Result<bool, String> {
    let pack_abs_path = repo_stored_path(ctx, pack_rel_path);
    if stamp.pack_id != pack_id
        || stamp.pack_path != pack_abs_path
        || stamp.expected_index_checksum != expected_index_checksum
        || !pack_abs_path.is_file()
    {
        return Ok(false);
    }
    Ok(pack_file_identity(&pack_abs_path)? == stamp.file_identity)
}

#[cfg(unix)]
fn pack_file_identity(path: &Path) -> Result<String, String> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(path)
        .map_err(|err| format!("failed to stat pack {}: {err}", path.display()))?;
    Ok(format!(
        "{}:{}:{}:{}:{}:{}:{}",
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec()
    ))
}

#[cfg(not(unix))]
fn pack_file_identity(path: &Path) -> Result<String, String> {
    use std::time::UNIX_EPOCH;

    let metadata = fs::metadata(path)
        .map_err(|err| format!("failed to stat pack {}: {err}", path.display()))?;
    let modified = metadata
        .modified()
        .map_err(|err| format!("failed to read pack mtime {}: {err}", path.display()))?
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("pack mtime predates UNIX epoch {}: {err}", path.display()))?;
    Ok(format!(
        "{}:{}:{}",
        metadata.len(),
        modified.as_secs(),
        modified.subsec_nanos()
    ))
}

pub(super) fn validate_object_pack_file_matches_manifest(
    pack_abs_path: &Path,
    pack: &ZstdBulkObjectPackRow,
) -> Result<(), String> {
    let pack_format = required_object_pack_format(pack)?;
    let pack_path = pack_abs_path.to_string_lossy();
    validate_pack_archive_with_format(pack_path.as_ref(), pack_format)?;
    let index = read_pack_index_with_format(pack_path.as_ref(), pack_format)?;
    if index.get("pack_id").and_then(JsonValue::as_str) != Some(pack.pack_id.as_str()) {
        return Err(format!(
            "Downloaded zstd object pack {} index pack_id mismatch.",
            pack.pack_id
        ));
    }
    if index.get("pack_format").and_then(JsonValue::as_str) != Some(pack_format) {
        return Err(format!(
            "Downloaded zstd object pack {} index pack_format mismatch.",
            pack.pack_id
        ));
    }
    compare_i64_field(
        &index,
        "member_count",
        required_i64(pack.member_count, "object_packs[].member_count")?,
        &format!("Downloaded zstd object pack {}", pack.pack_id),
    )?;
    compare_i64_field(
        &index,
        "total_bytes",
        required_i64(pack.total_bytes, "object_packs[].total_bytes")?,
        &format!("Downloaded zstd object pack {}", pack.pack_id),
    )?;
    compare_string_field(
        &index,
        "index_entry_name",
        required_option_string(
            &pack.pack_index_entry_name,
            "object_packs[].pack_index_entry_name",
        )?,
        &format!("Downloaded zstd object pack {}", pack.pack_id),
    )?;
    let expected_checksum = required_option_string(
        &pack.pack_index_checksum,
        "object_packs[].pack_index_checksum",
    )?;
    let actual_checksum = pack_index_checksum_with_format(pack_path.as_ref(), pack_format)?
        .ok_or_else(|| format!("Object pack {} is not a zstd chunked pack.", pack.pack_id))?;
    if actual_checksum != expected_checksum {
        return Err(format!(
            "Downloaded zstd object pack {} index checksum mismatch.",
            pack.pack_id
        ));
    }
    Ok(())
}

pub(super) fn validate_tree_pack_file_matches_manifest(
    pack_abs_path: &Path,
    pack: &ZstdBulkTreePackRow,
) -> Result<(), String> {
    let pack_format = required_tree_pack_format(pack)?;
    let pack_path = pack_abs_path.to_string_lossy();
    validate_tree_pack_archive_with_format(pack_path.as_ref(), pack_format)?;
    let index = read_tree_pack_index_with_format(pack_path.as_ref(), pack_format)?;
    if index.get("pack_id").and_then(JsonValue::as_str) != Some(pack.pack_id.as_str()) {
        return Err(format!(
            "Downloaded zstd tree pack {} index pack_id mismatch.",
            pack.pack_id
        ));
    }
    if index.get("pack_format").and_then(JsonValue::as_str) != Some(pack_format) {
        return Err(format!(
            "Downloaded zstd tree pack {} index pack_format mismatch.",
            pack.pack_id
        ));
    }
    compare_i64_field(
        &index,
        "tree_count",
        required_i64(pack.tree_count, "tree_packs[].tree_count")?,
        &format!("Downloaded zstd tree pack {}", pack.pack_id),
    )?;
    compare_i64_field(
        &index,
        "total_bytes",
        required_i64(pack.total_bytes, "tree_packs[].total_bytes")?,
        &format!("Downloaded zstd tree pack {}", pack.pack_id),
    )?;
    compare_string_field(
        &index,
        "index_entry_name",
        required_option_string(
            &pack.pack_index_entry_name,
            "tree_packs[].pack_index_entry_name",
        )?,
        &format!("Downloaded zstd tree pack {}", pack.pack_id),
    )?;
    let expected_checksum = required_option_string(
        &pack.pack_index_checksum,
        "tree_packs[].pack_index_checksum",
    )?;
    let actual_checksum = tree_pack_index_checksum_with_format(pack_path.as_ref(), pack_format)?
        .ok_or_else(|| format!("Tree pack {} is not a zstd chunked pack.", pack.pack_id))?;
    if actual_checksum != expected_checksum {
        return Err(format!(
            "Downloaded zstd tree pack {} index checksum mismatch.",
            pack.pack_id
        ));
    }
    Ok(())
}

pub(super) fn validate_manifest_locators_against_pack_indexes(
    ctx: &RemoteSyncLocalStoreContext,
    manifest: &ZstdImportManifestPayload,
) -> Result<(), String> {
    let mut object_indexes = BTreeMap::new();
    for pack in &manifest.object_packs {
        let pack_path = repo_stored_path(ctx, &default_object_pack_relative_path(&pack.pack_id));
        object_indexes.insert(
            pack.pack_id.clone(),
            read_pack_index_with_format(
                pack_path.to_string_lossy().as_ref(),
                PACK_FORMAT_ZSTD_CHUNKED_V1,
            )?,
        );
    }
    let mut tree_indexes = BTreeMap::new();
    for pack in &manifest.tree_packs {
        let pack_path = repo_stored_path(ctx, &default_tree_pack_relative_path(&pack.pack_id));
        tree_indexes.insert(
            pack.pack_id.clone(),
            read_tree_pack_index_with_format(
                pack_path.to_string_lossy().as_ref(),
                TREE_PACK_FORMAT_ZSTD_CHUNKED_V1,
            )?,
        );
    }
    for locator in &manifest.blob_locators {
        validate_blob_locator_against_pack_index(locator, &object_indexes)?;
    }
    for locator in &manifest.tree_locators {
        validate_tree_locator_against_pack_index(locator, &tree_indexes)?;
    }
    validate_snapshot_root_against_tree_indexes(manifest, &tree_indexes)?;
    Ok(())
}

pub(super) fn binary_db_object_pack_write_input_from_index(
    ctx: &RemoteSyncLocalStoreContext,
    manifest: &ZstdImportManifestPayload,
    pack: &ZstdBulkObjectPackRow,
) -> Result<BinaryDbObjectPackWriteInput, String> {
    let pack_format = required_object_pack_format(pack)?;
    let pack_rel_path = pack
        .pack_path
        .clone()
        .unwrap_or_else(|| default_object_pack_relative_path(&pack.pack_id));
    let expected_pack_rel_path = default_object_pack_relative_path(&pack.pack_id);
    if pack_rel_path != expected_pack_rel_path {
        return Err(format!(
            "Object pack {} path mismatch: expected {}, got {}.",
            pack.pack_id, expected_pack_rel_path, pack_rel_path
        ));
    }
    let pack_path = repo_stored_path(ctx, &pack_rel_path);
    let index = read_pack_index_with_format(pack_path.to_string_lossy().as_ref(), pack_format)?;
    compare_string_field(&index, "pack_id", &pack.pack_id, "Object pack index")?;
    compare_string_field(&index, "pack_format", pack_format, "Object pack index")?;
    let entries = index
        .get("entries")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("Object pack {} index is missing entries.", pack.pack_id))?;
    let expected_member_count = usize::try_from(required_i64(
        pack.member_count,
        "object_packs[].member_count",
    )?)
    .map_err(|_| format!("Object pack {} member_count overflows usize.", pack.pack_id))?;
    if entries.len() != expected_member_count {
        return Err(format!(
            "Object pack {} index member_count mismatch: expected {}, got {}.",
            pack.pack_id,
            expected_member_count,
            entries.len()
        ));
    }

    let locator_created_at = manifest
        .blob_locators
        .iter()
        .filter_map(|locator| {
            locator
                .created_at
                .as_ref()
                .map(|created_at| (locator.blob_id.as_str(), created_at.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    let pack_created_at = required_option_string(&pack.created_at, "object_packs[].created_at")?;
    let mut members = Vec::with_capacity(entries.len());
    for entry in entries {
        let blob_id = required_object_pack_index_text(entry, "blob_id", &pack.pack_id)?;
        let sha256 = required_object_pack_index_text(entry, "checksum", &pack.pack_id)?;
        let size_bytes = entry
            .get("uncompressed_byte_length")
            .and_then(JsonValue::as_i64)
            .filter(|value| *value >= 0)
            .ok_or_else(|| {
                format!(
                    "Object pack {} blob {} has invalid uncompressed_byte_length.",
                    pack.pack_id, blob_id
                )
            })?;
        let pack_entry_type = required_object_pack_index_text(entry, "entry_type", &pack.pack_id)?;
        let pack_chain_depth = entry
            .get("chain_depth")
            .and_then(JsonValue::as_i64)
            .filter(|value| *value >= 0)
            .ok_or_else(|| {
                format!(
                    "Object pack {} blob {} has invalid chain_depth.",
                    pack.pack_id, blob_id
                )
            })?;
        let pack_base_blob_id =
            normalize_str(entry.get("base_blob_id").and_then(JsonValue::as_str));
        members.push(BinaryDbObjectPackMemberWriteInput {
            created_at: locator_created_at
                .get(blob_id.as_str())
                .copied()
                .unwrap_or(pack_created_at)
                .to_string(),
            blob_id,
            sha256,
            size_bytes,
            pack_entry_type,
            pack_base_blob_id,
            pack_chain_depth,
        });
    }
    Ok(BinaryDbObjectPackWriteInput {
        pack_id: pack.pack_id.clone(),
        pack_rel_path,
        pack_format: pack_format.to_string(),
        member_count: i64::try_from(expected_member_count)
            .map_err(|_| "object pack member count overflows i64".to_string())?,
        total_bytes: required_i64(pack.total_bytes, "object_packs[].total_bytes")?,
        created_at: pack_created_at.to_string(),
        members,
    })
}

fn required_object_pack_index_text(
    value: &JsonValue,
    field: &str,
    pack_id: &str,
) -> Result<String, String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Object pack {pack_id} index entry is missing {field}."))
}

pub(super) fn binary_db_tree_pack_write_inputs(
    ctx: &RemoteSyncLocalStoreContext,
    manifest: &ZstdImportManifestPayload,
    pack: &ZstdBulkTreePackRow,
) -> Result<(BinaryDbTreePackWriteInput, Vec<BinaryDbTreeEntryWriteInput>), String> {
    let pack_format = required_tree_pack_format(pack)?;
    let pack_rel_path = pack
        .pack_path
        .clone()
        .unwrap_or_else(|| default_tree_pack_relative_path(&pack.pack_id));
    let expected_pack_rel_path = default_tree_pack_relative_path(&pack.pack_id);
    if pack_rel_path != expected_pack_rel_path {
        return Err(format!(
            "Tree pack {} path mismatch: expected {}, got {}.",
            pack.pack_id, expected_pack_rel_path, pack_rel_path
        ));
    }
    let pack_abs_path = repo_stored_path(ctx, &pack_rel_path);
    let pack_path = pack_abs_path.to_string_lossy();
    let index = read_tree_pack_index_with_format(pack_path.as_ref(), pack_format)?;
    let index_trees = index
        .get("trees")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("Tree pack {} index is missing trees.", pack.pack_id))?;
    let expected_tree_count =
        usize::try_from(required_i64(pack.tree_count, "tree_packs[].tree_count")?)
            .map_err(|_| format!("Tree pack {} tree_count overflows usize.", pack.pack_id))?;
    if index_trees.len() != expected_tree_count {
        return Err(format!(
            "Tree pack {} index tree_count mismatch: expected {}, got {}.",
            pack.pack_id,
            expected_tree_count,
            index_trees.len()
        ));
    }

    let mut locator_by_tree_id = BTreeMap::new();
    for locator in &manifest.tree_locators {
        if locator_by_tree_id
            .insert(locator.tree_id.to_ascii_lowercase(), locator)
            .is_some()
        {
            return Err(format!(
                "Tree pack {} has duplicate locator for tree {}.",
                pack.pack_id, locator.tree_id
            ));
        }
    }

    let mut index_tree_by_ordinal = BTreeMap::new();
    for tree in index_trees {
        let ordinal = tree
            .get("entry_ordinal")
            .and_then(JsonValue::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| {
                format!(
                    "Tree pack {} index has an invalid entry_ordinal.",
                    pack.pack_id
                )
            })?;
        if index_tree_by_ordinal.insert(ordinal, tree).is_some() {
            return Err(format!(
                "Tree pack {} index has duplicate entry_ordinal {}.",
                pack.pack_id, ordinal
            ));
        }
    }

    let mut archive = TreePackEntryArchive::open_with_format(pack_path.as_ref(), pack_format)?;
    let mut trees = Vec::with_capacity(expected_tree_count);
    let mut entries = Vec::new();
    for ordinal in 0..expected_tree_count {
        let index_tree = index_tree_by_ordinal.get(&ordinal).ok_or_else(|| {
            format!(
                "Tree pack {} index is missing dense entry_ordinal {}.",
                pack.pack_id, ordinal
            )
        })?;
        let tree_id = required_json_text(index_tree, "tree_id", &pack.pack_id)?;
        let entry_count = index_tree
            .get("entry_count")
            .and_then(JsonValue::as_i64)
            .ok_or_else(|| {
                format!(
                    "Tree pack {} tree {} is missing entry_count.",
                    pack.pack_id, tree_id
                )
            })?;
        if entry_count < 0 {
            return Err(format!(
                "Tree pack {} tree {} has negative entry_count.",
                pack.pack_id, tree_id
            ));
        }
        let locator = locator_by_tree_id
            .get(&tree_id.to_ascii_lowercase())
            .ok_or_else(|| {
                format!(
                    "Tree pack {} physical tree {} is missing its selected manifest locator.",
                    pack.pack_id, tree_id
                )
            })?;
        if required_i64(locator.entry_count, "tree_locators[].entry_count")? != entry_count {
            return Err(format!(
                "Tree pack {} tree {} locator entry_count mismatch.",
                pack.pack_id, tree_id
            ));
        }
        compare_string_field(
            index_tree,
            "checksum",
            required_option_string(
                &locator.tree_pack_checksum,
                "tree_locators[].tree_pack_checksum",
            )?,
            &format!("Tree pack {} tree {}", pack.pack_id, tree_id),
        )?;

        let payload = archive.read_tree_by_ordinal(ordinal)?;
        compare_string_field(
            &payload,
            "tree_id",
            &tree_id,
            &format!("Tree pack {} ordinal {} payload", pack.pack_id, ordinal),
        )?;
        compare_i64_field(
            &payload,
            "entry_ordinal",
            i64::try_from(ordinal)
                .map_err(|_| format!("Tree pack ordinal overflows i64: {ordinal}"))?,
            &format!("Tree pack {} tree {} payload", pack.pack_id, tree_id),
        )?;
        let rows = payload
            .get("rows")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| {
                format!(
                    "Tree pack {} tree {} payload is missing rows.",
                    pack.pack_id, tree_id
                )
            })?;
        if i64::try_from(rows.len()).ok() != Some(entry_count) {
            return Err(format!(
                "Tree pack {} tree {} payload entry_count mismatch: expected {}, got {}.",
                pack.pack_id,
                tree_id,
                entry_count,
                rows.len()
            ));
        }
        let mut entry_names = BTreeSet::new();
        for row in rows {
            let row_tree_id = required_json_text(row, "tree_id", &pack.pack_id)?;
            if row_tree_id != tree_id {
                return Err(format!(
                    "Tree pack {} ordinal {} row tree_id mismatch: expected {}, got {}.",
                    pack.pack_id, ordinal, tree_id, row_tree_id
                ));
            }
            let entry_name = required_json_text(row, "entry_name", &pack.pack_id)?;
            if !entry_names.insert(entry_name.clone()) {
                return Err(format!(
                    "Tree pack {} tree {} has duplicate entry name {}.",
                    pack.pack_id, tree_id, entry_name
                ));
            }
            entries.push(BinaryDbTreeEntryWriteInput {
                tree_id: tree_id.clone(),
                entry_name,
                entry_type: required_json_text(row, "entry_type", &pack.pack_id)?,
                target_id: required_json_text(row, "target_id", &pack.pack_id)?,
                mode: required_json_text(row, "mode", &pack.pack_id)?,
            });
        }
        trees.push(BinaryDbTreePackTreeWriteInput {
            tree_id,
            entry_count,
        });
    }
    Ok((
        BinaryDbTreePackWriteInput {
            pack_id: pack.pack_id.clone(),
            pack_rel_path,
            pack_format: pack_format.to_string(),
            tree_count: i64::try_from(expected_tree_count)
                .map_err(|_| "tree pack tree count overflows i64".to_string())?,
            total_bytes: pack
                .total_bytes
                .ok_or_else(|| format!("Tree pack {} is missing total_bytes.", pack.pack_id))?,
            created_at: pack.created_at.clone().unwrap_or_default(),
            trees,
        },
        entries,
    ))
}

fn required_json_text(value: &JsonValue, field: &str, pack_id: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("Tree pack {pack_id} value is missing text field {field}."))
}

pub(super) fn validate_blob_locator_against_pack_index(
    locator: &ZstdBulkBlobLocatorRow,
    object_indexes: &BTreeMap<String, JsonValue>,
) -> Result<(), String> {
    let pack_id = required_option_string(&locator.pack_id, "blob_locators[].pack_id")?;
    let entry_name =
        required_option_string(&locator.pack_entry_name, "blob_locators[].pack_entry_name")?;
    let index = object_indexes.get(pack_id).ok_or_else(|| {
        format!(
            "Blob locator {} references missing pack {pack_id}.",
            locator.blob_id
        )
    })?;
    let entries = index
        .get("entries")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("Object pack {pack_id} index is missing entries."))?;
    let entry = entries
        .iter()
        .find(|entry| entry.get("entry_name").and_then(JsonValue::as_str) == Some(entry_name))
        .ok_or_else(|| {
            format!(
                "Blob locator {} references missing pack entry {entry_name}.",
                locator.blob_id
            )
        })?;
    compare_string_field(entry, "blob_id", &locator.blob_id, "Blob locator")?;
    compare_string_field(
        entry,
        "entry_type",
        required_option_string(&locator.pack_entry_type, "blob_locators[].pack_entry_type")?,
        "Blob locator",
    )?;
    compare_string_field(
        entry,
        "checksum",
        required_option_string(&locator.sha256, "blob_locators[].sha256")?,
        "Blob locator",
    )?;
    compare_i64_field(
        entry,
        "uncompressed_byte_length",
        required_i64(locator.size_bytes, "blob_locators[].size_bytes")?,
        "Blob locator",
    )?;
    compare_i64_field(
        entry,
        "chain_depth",
        required_i64(locator.pack_chain_depth, "blob_locators[].pack_chain_depth")?,
        "Blob locator",
    )?;
    let entry_base = entry.get("base_blob_id").and_then(JsonValue::as_str);
    let locator_base = locator.pack_base_blob_id.as_deref();
    if normalize_str(entry_base) != normalize_str(locator_base) {
        return Err(format!(
            "Blob locator {} base blob mismatch.",
            locator.blob_id
        ));
    }
    Ok(())
}

pub(super) fn validate_tree_locator_against_pack_index(
    locator: &ZstdBulkTreeLocatorRow,
    tree_indexes: &BTreeMap<String, JsonValue>,
) -> Result<(), String> {
    let pack_id = required_option_string(&locator.tree_pack_id, "tree_locators[].tree_pack_id")?;
    let index = tree_indexes.get(pack_id).ok_or_else(|| {
        format!(
            "Tree locator {} references missing pack {pack_id}.",
            locator.tree_id
        )
    })?;
    let trees = index
        .get("trees")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("Tree pack {pack_id} index is missing trees."))?;
    let tree = trees
        .iter()
        .find(|tree| {
            tree.get("tree_id").and_then(JsonValue::as_str) == Some(locator.tree_id.as_str())
        })
        .ok_or_else(|| {
            format!(
                "Tree locator {} references missing tree pack member.",
                locator.tree_id
            )
        })?;
    compare_i64_field(
        tree,
        "entry_count",
        required_i64(locator.entry_count, "tree_locators[].entry_count")?,
        "Tree locator",
    )?;
    compare_string_field(
        tree,
        "checksum",
        required_option_string(
            &locator.tree_pack_checksum,
            "tree_locators[].tree_pack_checksum",
        )?,
        "Tree locator",
    )?;
    Ok(())
}

pub(super) fn validate_snapshot_root_against_tree_indexes(
    manifest: &ZstdImportManifestPayload,
    tree_indexes: &BTreeMap<String, JsonValue>,
) -> Result<(), String> {
    let snapshot = manifest
        .snapshots
        .first()
        .ok_or_else(|| "Zstd import manifest is missing snapshot row.".to_string())?;
    let root_pack_id =
        required_option_string(&snapshot.root_tree_pack_id, "snapshots[].root_tree_pack_id")?;
    let root_ordinal = required_i64(
        snapshot.root_entry_ordinal,
        "snapshots[].root_entry_ordinal",
    )?;
    let index = tree_indexes
        .get(root_pack_id)
        .ok_or_else(|| format!("Snapshot root tree pack {root_pack_id} was not downloaded."))?;
    let trees = index
        .get("trees")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| format!("Tree pack {root_pack_id} index is missing trees."))?;
    let root_tree = trees
        .iter()
        .find(|tree| tree.get("entry_ordinal").and_then(JsonValue::as_i64) == Some(root_ordinal))
        .ok_or_else(|| {
            format!(
                "Snapshot {} root ordinal {root_ordinal} is missing from tree pack {root_pack_id}.",
                snapshot.snapshot_id
            )
        })?;
    let root_tree_id = root_tree
        .get("tree_id")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| {
            format!(
                "Snapshot {} root tree index entry is missing tree_id.",
                snapshot.snapshot_id
            )
        })?;
    if !manifest.tree_locators.iter().any(|locator| {
        locator.tree_id == root_tree_id && locator.tree_pack_id.as_deref() == Some(root_pack_id)
    }) {
        return Err(format!(
            "Snapshot {} root tree locator {root_tree_id} is missing from manifest.",
            snapshot.snapshot_id
        ));
    }
    Ok(())
}

pub(super) fn required_object_pack_format(
    pack: &ZstdBulkObjectPackRow,
) -> Result<&'static str, String> {
    match pack.pack_format {
        Some(PackFormatKind::ZstdChunkedV1) => Ok(PACK_FORMAT_ZSTD_CHUNKED_V1),
        None => Err("Missing required field `object_packs[].pack_format`.".to_string()),
    }
}

pub(super) fn required_tree_pack_format(
    pack: &ZstdBulkTreePackRow,
) -> Result<&'static str, String> {
    match pack.pack_format {
        Some(TreePackFormatKind::ZstdChunkedTreeV1) => Ok(TREE_PACK_FORMAT_ZSTD_CHUNKED_V1),
        None => Err("Missing required field `tree_packs[].pack_format`.".to_string()),
    }
}

pub(super) fn required_i64(value: Option<i64>, field: &str) -> Result<i64, String> {
    let value = value.ok_or_else(|| format!("Missing required field `{field}`."))?;
    if value < 0 {
        return Err(format!("Field `{field}` must not be negative."));
    }
    Ok(value)
}

pub(super) fn required_option_string<'a>(
    value: &'a Option<String>,
    field: &str,
) -> Result<&'a str, String> {
    let value = value
        .as_deref()
        .ok_or_else(|| format!("Missing required field `{field}`."))?;
    if value.trim().is_empty() {
        return Err(format!("Field `{field}` must not be empty."));
    }
    Ok(value)
}

pub(super) fn compare_i64_field(
    value: &JsonValue,
    field: &str,
    expected: i64,
    label: &str,
) -> Result<(), String> {
    let actual = value
        .get(field)
        .and_then(JsonValue::as_i64)
        .ok_or_else(|| format!("{label} is missing numeric field {field}."))?;
    if actual != expected {
        return Err(format!(
            "{label} field {field} mismatch: expected {expected}, got {actual}."
        ));
    }
    Ok(())
}

pub(super) fn compare_string_field(
    value: &JsonValue,
    field: &str,
    expected: &str,
    label: &str,
) -> Result<(), String> {
    let actual = value
        .get(field)
        .and_then(JsonValue::as_str)
        .ok_or_else(|| format!("{label} is missing text field {field}."))?;
    if actual != expected {
        return Err(format!(
            "{label} field {field} mismatch: expected {expected:?}, got {actual:?}."
        ));
    }
    Ok(())
}

pub(super) fn normalize_str(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
