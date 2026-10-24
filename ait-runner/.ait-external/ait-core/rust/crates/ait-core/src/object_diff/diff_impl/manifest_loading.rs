use super::*;

pub(crate) fn snapshot_manifest_from_object_reader_impl<R: ObjectReader + ?Sized>(
    object_reader: &R,
    snapshot_id: &str,
) -> Result<JsonValue, String> {
    let snapshot_reader = ObjectBackedSnapshotReader::new(object_reader);
    let payload = snapshot_reader
        .read_snapshot_payload(snapshot_id)?
        .ok_or_else(|| format!("snapshot `{snapshot_id}` was not found"))?;
    if let Some(file_map) =
        snapshot_manifest_from_payload_with_tree_reader(&snapshot_reader, snapshot_id, &payload)?
    {
        return Ok(file_map);
    }
    normalize_snapshot_object_payload(snapshot_id, payload)
}

pub fn snapshot_diff_from_readers<S: SnapshotReader + ?Sized, B: BlobReader + ?Sized>(
    snapshot_reader: &S,
    blob_reader: Option<&B>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    SnapshotJson::stateless().snapshot_diff_from_readers(
        snapshot_reader,
        blob_reader,
        old_snapshot_id,
        new_snapshot_id,
        include_text,
        max_bytes,
    )
}

pub(crate) fn snapshot_diff_from_readers_impl<
    S: SnapshotReader + ?Sized,
    B: BlobReader + ?Sized,
>(
    snapshot_reader: &S,
    blob_reader: Option<&B>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    if let Some(payload) = try_snapshot_diff_from_tree_reader(
        snapshot_reader,
        blob_reader,
        old_snapshot_id,
        new_snapshot_id,
        include_text,
        max_bytes,
    )? {
        return Ok(payload);
    }
    let old_files = match old_snapshot_id {
        Some(snapshot_id) => snapshot_reader.read_snapshot_manifest(snapshot_id)?,
        None => JsonValue::Object(Map::new()),
    };
    let new_files = match new_snapshot_id {
        Some(snapshot_id) => snapshot_reader.read_snapshot_manifest(snapshot_id)?,
        None => JsonValue::Object(Map::new()),
    };
    let blob_bytes_by_id = if include_text {
        let blob_reader = blob_reader
            .ok_or_else(|| "blob_reader is required when include_text is true".to_string())?;
        collect_blob_bytes_for_modified_snapshot_manifests(blob_reader, &old_files, &new_files)?
    } else {
        BTreeMap::new()
    };
    snapshot_diff_from_manifests_impl(
        &old_files,
        &new_files,
        &blob_bytes_by_id,
        old_snapshot_id,
        new_snapshot_id,
        include_text,
        max_bytes,
    )
}

pub fn snapshot_diff_from_object_reader<O: ObjectReader + ?Sized, B: BlobReader + ?Sized>(
    object_reader: &O,
    blob_reader: Option<&B>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    SnapshotJson::stateless().snapshot_diff_from_object_reader(
        object_reader,
        blob_reader,
        old_snapshot_id,
        new_snapshot_id,
        include_text,
        max_bytes,
    )
}

pub(crate) fn snapshot_diff_from_object_reader_impl<
    O: ObjectReader + ?Sized,
    B: BlobReader + ?Sized,
>(
    object_reader: &O,
    blob_reader: Option<&B>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    let snapshot_reader = ObjectBackedSnapshotReader::new(object_reader);
    snapshot_diff_from_readers_impl(
        &snapshot_reader,
        blob_reader,
        old_snapshot_id,
        new_snapshot_id,
        include_text,
        max_bytes,
    )
}

pub(super) fn try_snapshot_diff_from_tree_reader<
    S: SnapshotReader + ?Sized,
    B: BlobReader + ?Sized,
>(
    snapshot_reader: &S,
    blob_reader: Option<&B>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: usize,
) -> Result<Option<JsonValue>, String> {
    let old_root = resolve_snapshot_tree_root(snapshot_reader, old_snapshot_id)?;
    let new_root = resolve_snapshot_tree_root(snapshot_reader, new_snapshot_id)?;
    let uses_tree_authority = old_root.is_some() || new_root.is_some();
    if !uses_tree_authority {
        return Ok(None);
    }
    let tree_result =
        diff_snapshot_trees(snapshot_reader, old_root.as_deref(), new_root.as_deref())?;
    let blob_bytes_by_id = if include_text {
        let blob_reader = blob_reader
            .ok_or_else(|| "blob_reader is required when include_text is true".to_string())?;
        collect_blob_bytes_for_modified_rows(
            blob_reader,
            &tree_result.old_rows,
            &tree_result.new_rows,
            &tree_result.modified,
        )?
    } else {
        BTreeMap::new()
    };
    Ok(Some(snapshot_diff_from_tree_result(
        tree_result,
        &blob_bytes_by_id,
        old_snapshot_id,
        new_snapshot_id,
        include_text,
        max_bytes,
    )?))
}

pub(super) fn resolve_snapshot_tree_root<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    snapshot_id: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(snapshot_id) = snapshot_id else {
        return Ok(None);
    };
    if let Some(root_payload) = snapshot_reader.read_snapshot_root_tree_payload(snapshot_id)? {
        let (root_tree_id, _) = snapshot_tree_payload_entries(&root_payload)?;
        return Ok(Some(root_tree_id));
    }
    Ok(None)
}

pub(super) fn snapshot_manifest_from_payload_with_tree_reader<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    snapshot_id: &str,
    payload: &JsonValue,
) -> Result<Option<JsonValue>, String> {
    if let Some(root_payload) = snapshot_reader.read_snapshot_root_tree_payload(snapshot_id)? {
        let (root_tree_id, root_entries) = snapshot_tree_payload_entries(&root_payload)?;
        let mut tree_cache = BTreeMap::new();
        tree_cache.insert(root_tree_id.clone(), root_entries);
        let mut rows = BTreeMap::new();
        collect_tree_file_rows(
            snapshot_reader,
            &root_tree_id,
            "",
            &mut tree_cache,
            &mut rows,
        )?;
        return Ok(Some(render_snapshot_manifest(&rows)));
    }
    let _ = payload;
    Ok(None)
}

pub(super) fn render_snapshot_manifest(rows: &BTreeMap<String, SnapshotFileRow>) -> JsonValue {
    JsonValue::Object(Map::from_iter(rows.iter().map(|(path, row)| {
        (
            path.clone(),
            json!({
                "path": row.path,
                "blob_id": row.blob_id,
                "size_bytes": row.size_bytes,
                "mode": row.mode_raw,
            }),
        )
    })))
}

pub(super) fn snapshot_tree_payload_entries(
    payload: &JsonValue,
) -> Result<(String, BTreeMap<String, SnapshotTreeEntry>), String> {
    let Some(payload_obj) = payload.as_object() else {
        return Err("tree payload must be an object".to_string());
    };
    let root_tree_id = payload_obj
        .get("tree_id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "tree payload is missing tree_id".to_string())?;
    let rows = payload_obj
        .get("rows")
        .or_else(|| payload_obj.get("entries"))
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "tree payload is missing rows".to_string())?;
    let entries = rows
        .iter()
        .map(|row| {
            let Some(row_obj) = row.as_object() else {
                return Err("tree payload row must be an object".to_string());
            };
            let entry_type = row_obj
                .get("entry_type")
                .or_else(|| row_obj.get("type"))
                .and_then(JsonValue::as_str)
                .map(str::to_string)
                .ok_or_else(|| "tree payload row is missing entry_type".to_string())?;
            let mode_raw = row_obj.get("mode").cloned().unwrap_or(JsonValue::Null);
            Ok((
                row_obj
                    .get("entry_name")
                    .or_else(|| row_obj.get("name"))
                    .and_then(JsonValue::as_str)
                    .map(str::to_string)
                    .ok_or_else(|| "tree payload row is missing entry_name".to_string())?,
                SnapshotTreeEntry {
                    entry_type: entry_type.clone(),
                    target_id: row_obj
                        .get("target_id")
                        .and_then(JsonValue::as_str)
                        .map(str::to_string)
                        .ok_or_else(|| "tree payload row is missing target_id".to_string())?,
                    size_bytes: row_obj.get("size_bytes").and_then(JsonValue::as_i64),
                    mode_raw: mode_raw.clone(),
                    mode_int: if entry_type == "tree" {
                        0
                    } else {
                        to_mode_int(&mode_raw)?
                    },
                },
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok((root_tree_id, BTreeMap::from_iter(entries)))
}

pub(super) fn collect_blob_bytes_by_id<B: BlobReader + ?Sized>(
    blob_reader: &B,
    blob_ids: BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    blob_reader.read_blob_bytes_batch(&blob_ids.into_iter().collect::<Vec<_>>())
}

pub(super) fn insert_row_blob_id(blob_ids: &mut BTreeSet<String>, row: &SnapshotFileRow) {
    if let Some(blob_id) = row.blob_id.as_ref() {
        if !blob_id.trim().is_empty() {
            blob_ids.insert(blob_id.clone());
        }
    }
}

pub(super) fn collect_blob_bytes_for_modified_rows<B: BlobReader + ?Sized>(
    blob_reader: &B,
    old_rows: &BTreeMap<String, SnapshotFileRow>,
    new_rows: &BTreeMap<String, SnapshotFileRow>,
    modified_paths: &[String],
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let mut blob_ids = BTreeSet::new();
    for path in modified_paths {
        let old_row = old_rows
            .get(path)
            .ok_or_else(|| format!("missing modified path `{path}` in old snapshot tree"))?;
        let new_row = new_rows
            .get(path)
            .ok_or_else(|| format!("missing modified path `{path}` in new snapshot tree"))?;
        insert_row_blob_id(&mut blob_ids, old_row);
        insert_row_blob_id(&mut blob_ids, new_row);
    }
    collect_blob_bytes_by_id(blob_reader, blob_ids)
}

pub(in crate::object_diff) fn coerce_snapshot_manifest(
    value: &JsonValue,
) -> Result<BTreeMap<String, SnapshotFileRow>, String> {
    match value {
        JsonValue::Object(values) => values
            .iter()
            .map(|(path, row)| {
                let JsonValue::Object(row_obj) = row else {
                    return Err(
                        "snapshot manifest mapping values must be row dictionaries".to_string()
                    );
                };
                let mut normalized = row_obj.clone();
                normalized
                    .entry("path".to_string())
                    .or_insert_with(|| JsonValue::String(path.clone()));
                let row = parse_snapshot_file_row(&JsonValue::Object(normalized))?;
                Ok((path.clone(), row))
            })
            .collect(),
        JsonValue::Array(values) => {
            let mut out = BTreeMap::new();
            for row in values {
                let parsed = parse_snapshot_file_row(row)?;
                out.insert(parsed.path.clone(), parsed);
            }
            Ok(out)
        }
        _ => Err("snapshot files must be a dict[path, row] map or list of rows".to_string()),
    }
}

pub(super) fn normalize_snapshot_object_payload(
    snapshot_id: &str,
    payload: JsonValue,
) -> Result<JsonValue, String> {
    match payload {
        JsonValue::Object(mut map) => {
            if let Some(files) = map.remove("files") {
                coerce_snapshot_manifest(&files)?;
                return Ok(files);
            }
            let value = JsonValue::Object(map);
            coerce_snapshot_manifest(&value).map_err(|error| {
                format!("snapshot `{snapshot_id}` object payload is not a valid snapshot manifest: {error}")
            })?;
            Ok(value)
        }
        JsonValue::Array(_) => {
            coerce_snapshot_manifest(&payload).map_err(|error| {
                format!("snapshot `{snapshot_id}` object payload is not a valid snapshot manifest: {error}")
            })?;
            Ok(payload)
        }
        _ => Err(format!(
            "snapshot object payload for `{snapshot_id}` must be a snapshot-manifest object or row list"
        )),
    }
}

pub(super) fn collect_blob_bytes_for_modified_snapshot_manifests<B: BlobReader + ?Sized>(
    blob_reader: &B,
    old_files: &JsonValue,
    new_files: &JsonValue,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    let old_map = coerce_snapshot_manifest(old_files)?;
    let new_map = coerce_snapshot_manifest(new_files)?;
    let mut blob_ids = BTreeSet::new();
    for (path, old_row) in &old_map {
        let Some(new_row) = new_map.get(path) else {
            continue;
        };
        let content_changed = old_row.blob_id != new_row.blob_id
            || size_bytes_changed(old_row.size_bytes, new_row.size_bytes);
        if content_changed {
            insert_row_blob_id(&mut blob_ids, old_row);
            insert_row_blob_id(&mut blob_ids, new_row);
        }
    }
    collect_blob_bytes_by_id(blob_reader, blob_ids)
}

pub(super) fn parse_snapshot_file_row(value: &JsonValue) -> Result<SnapshotFileRow, String> {
    let JsonValue::Object(row) = value else {
        return Err("snapshot file rows must be mapping objects".to_string());
    };
    let path = row
        .get("path")
        .and_then(JsonValue::as_str)
        .map(|value| value.to_string())
        .ok_or_else(|| "snapshot file row is missing string path".to_string())?;
    let size_bytes = match row.get("size_bytes") {
        Some(value) => parse_optional_i64(value)?,
        None => None,
    };
    let mode_raw = row.get("mode").cloned().unwrap_or(JsonValue::Null);
    Ok(SnapshotFileRow {
        path,
        blob_id: row
            .get("blob_id")
            .and_then(JsonValue::as_str)
            .map(|value| value.to_string()),
        size_bytes,
        mode_int: to_mode_int(&mode_raw)?,
        mode_raw,
    })
}

pub(super) fn parse_optional_i64(value: &JsonValue) -> Result<Option<i64>, String> {
    match value {
        JsonValue::Null => Ok(None),
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .map(Some)
            .ok_or_else(|| "size_bytes must be an integer".to_string()),
        JsonValue::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            trimmed
                .parse::<i64>()
                .map(Some)
                .map_err(|_| "size_bytes must be an integer".to_string())
        }
        _ => Err("size_bytes must be an integer".to_string()),
    }
}

pub(in crate::object_diff) fn to_mode_int(value: &JsonValue) -> Result<i64, String> {
    match value {
        JsonValue::Null => Ok(0),
        JsonValue::Number(number) => number
            .as_i64()
            .or_else(|| number.as_u64().and_then(|value| i64::try_from(value).ok()))
            .ok_or_else(|| "mode must be an integer".to_string()),
        JsonValue::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Ok(0);
            }
            if let Some(stripped) = trimmed
                .strip_prefix("0o")
                .or_else(|| trimmed.strip_prefix("0O"))
            {
                return i64::from_str_radix(stripped, 8)
                    .map_err(|_| "mode must be parseable".to_string());
            }
            if let Some(stripped) = trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
            {
                return i64::from_str_radix(stripped, 16)
                    .map_err(|_| "mode must be parseable".to_string());
            }
            trimmed
                .parse::<i64>()
                .or_else(|_| i64::from_str_radix(trimmed, 8))
                .map_err(|_| "mode must be parseable".to_string())
        }
        _ => Err("mode must be an integer or string".to_string()),
    }
}
