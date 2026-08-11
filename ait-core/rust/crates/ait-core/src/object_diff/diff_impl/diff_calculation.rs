use super::*;

pub(super) fn snapshot_diff_from_tree_result(
    mut tree_result: SnapshotTreeDiffState,
    blob_bytes_by_id: &BTreeMap<String, Vec<u8>>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    let rename_hints = build_rename_hints(
        &tree_result.old_rows,
        &tree_result.new_rows,
        &tree_result.added,
        &tree_result.deleted,
    );
    let directory_move_hints = build_directory_move_hints(&rename_hints);
    let files_changed = tree_result.added.len()
        + tree_result.deleted.len()
        + tree_result.modified.len()
        + tree_result.mode_changed.len();
    let mut total_insertions = 0usize;
    let mut total_deletions = 0usize;
    if include_text {
        for file_row in tree_result.file_entries.iter_mut() {
            let Some(file_map) = file_row.as_object_mut() else {
                return Err("snapshot diff file row must be an object".to_string());
            };
            let status = string_field(file_map, "status")?;
            if status != "modified" {
                file_map.insert(
                    "diff".to_string(),
                    text_diff_json(&TextDiffPayload {
                        status: "metadata_only",
                        insertions: 0,
                        deletions: 0,
                        text: None,
                    }),
                );
                continue;
            }
            let path = string_field(file_map, "path")?;
            let old_row = tree_result
                .old_rows
                .get(&path)
                .ok_or_else(|| format!("missing modified path `{path}` in old snapshot tree"))?;
            let new_row = tree_result
                .new_rows
                .get(&path)
                .ok_or_else(|| format!("missing modified path `{path}` in new snapshot tree"))?;
            let text_diff = maybe_add_text_diff_from_blob_bytes(
                blob_bytes_by_id,
                &path,
                old_row,
                new_row,
                old_snapshot_id,
                new_snapshot_id,
                max_bytes,
            );
            total_insertions += text_diff.insertions;
            total_deletions += text_diff.deletions;
            file_map.insert("diff".to_string(), text_diff_json(&text_diff));
        }
    }

    Ok(json!({
        "old_snapshot_id": old_snapshot_id,
        "new_snapshot_id": new_snapshot_id,
        "added": tree_result.added,
        "deleted": tree_result.deleted,
        "modified": tree_result.modified,
        "mode_changed": tree_result.mode_changed,
        "rename_hints": rename_hints.into_iter().map(rename_hint_json).collect::<Vec<_>>(),
        "directory_move_hints": directory_move_hints,
        "files": tree_result.file_entries,
        "summary": {
            "files_changed": files_changed,
            "insertions": total_insertions,
            "deletions": total_deletions,
            "old_snapshot_id": old_snapshot_id,
            "new_snapshot_id": new_snapshot_id,
        },
    }))
}

pub(super) fn diff_snapshot_trees<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    old_root_tree_id: Option<&str>,
    new_root_tree_id: Option<&str>,
) -> Result<SnapshotTreeDiffState, String> {
    let mut tree_cache = BTreeMap::new();
    let mut state = SnapshotTreeDiffState::default();
    diff_tree_entries(
        snapshot_reader,
        old_root_tree_id,
        new_root_tree_id,
        "",
        &mut tree_cache,
        &mut state,
    )?;
    Ok(state)
}

pub(super) fn diff_tree_entries<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    old_tree_id: Option<&str>,
    new_tree_id: Option<&str>,
    prefix: &str,
    tree_cache: &mut BTreeMap<String, BTreeMap<String, SnapshotTreeEntry>>,
    state: &mut SnapshotTreeDiffState,
) -> Result<(), String> {
    if old_tree_id == new_tree_id && old_tree_id.is_some() {
        return Ok(());
    }
    let old_entries = match old_tree_id {
        Some(tree_id) => load_tree_entries(snapshot_reader, tree_id, tree_cache)?,
        None => BTreeMap::new(),
    };
    let new_entries = match new_tree_id {
        Some(tree_id) => load_tree_entries(snapshot_reader, tree_id, tree_cache)?,
        None => BTreeMap::new(),
    };
    let names = old_entries
        .keys()
        .chain(new_entries.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for name in names {
        let path = join_tree_path(prefix, &name);
        match (old_entries.get(&name), new_entries.get(&name)) {
            (None, Some(new_entry)) => {
                collect_added_entry(snapshot_reader, new_entry, &path, tree_cache, state)?
            }
            (Some(old_entry), None) => {
                collect_deleted_entry(snapshot_reader, old_entry, &path, tree_cache, state)?
            }
            (Some(old_entry), Some(new_entry)) => {
                match (old_entry.entry_type.as_str(), new_entry.entry_type.as_str()) {
                    ("tree", "tree") => {
                        diff_tree_entries(
                            snapshot_reader,
                            Some(&old_entry.target_id),
                            Some(&new_entry.target_id),
                            &path,
                            tree_cache,
                            state,
                        )?;
                    }
                    ("blob", "blob") => {
                        record_blob_change(&path, old_entry, new_entry, state)?;
                    }
                    _ => {
                        collect_deleted_entry(
                            snapshot_reader,
                            old_entry,
                            &path,
                            tree_cache,
                            state,
                        )?;
                        collect_added_entry(snapshot_reader, new_entry, &path, tree_cache, state)?;
                    }
                }
            }
            (None, None) => {}
        }
    }
    Ok(())
}

pub(super) fn collect_added_entry<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    entry: &SnapshotTreeEntry,
    path: &str,
    tree_cache: &mut BTreeMap<String, BTreeMap<String, SnapshotTreeEntry>>,
    state: &mut SnapshotTreeDiffState,
) -> Result<(), String> {
    if entry.entry_type == "tree" {
        let rows = collect_subtree_rows(snapshot_reader, &entry.target_id, path, tree_cache)?;
        for row in rows.into_values() {
            push_added_row(state, row);
        }
        return Ok(());
    }
    push_added_row(state, snapshot_file_row_from_tree_entry(path, entry)?);
    Ok(())
}

pub(super) fn collect_deleted_entry<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    entry: &SnapshotTreeEntry,
    path: &str,
    tree_cache: &mut BTreeMap<String, BTreeMap<String, SnapshotTreeEntry>>,
    state: &mut SnapshotTreeDiffState,
) -> Result<(), String> {
    if entry.entry_type == "tree" {
        let rows = collect_subtree_rows(snapshot_reader, &entry.target_id, path, tree_cache)?;
        for row in rows.into_values() {
            push_deleted_row(state, row);
        }
        return Ok(());
    }
    push_deleted_row(state, snapshot_file_row_from_tree_entry(path, entry)?);
    Ok(())
}

pub(super) fn collect_subtree_rows<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    tree_id: &str,
    prefix: &str,
    tree_cache: &mut BTreeMap<String, BTreeMap<String, SnapshotTreeEntry>>,
) -> Result<BTreeMap<String, SnapshotFileRow>, String> {
    let mut rows = BTreeMap::new();
    collect_tree_file_rows(snapshot_reader, tree_id, prefix, tree_cache, &mut rows)?;
    Ok(rows)
}

pub(super) fn collect_tree_file_rows<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    tree_id: &str,
    prefix: &str,
    tree_cache: &mut BTreeMap<String, BTreeMap<String, SnapshotTreeEntry>>,
    rows: &mut BTreeMap<String, SnapshotFileRow>,
) -> Result<(), String> {
    let entries = load_tree_entries(snapshot_reader, tree_id, tree_cache)?;
    for (name, entry) in entries {
        let path = join_tree_path(prefix, &name);
        if entry.entry_type == "tree" {
            collect_tree_file_rows(snapshot_reader, &entry.target_id, &path, tree_cache, rows)?;
            continue;
        }
        rows.insert(
            path.clone(),
            snapshot_file_row_from_tree_entry(&path, &entry)?,
        );
    }
    Ok(())
}

pub(super) fn load_tree_entries<S: SnapshotReader + ?Sized>(
    snapshot_reader: &S,
    tree_id: &str,
    tree_cache: &mut BTreeMap<String, BTreeMap<String, SnapshotTreeEntry>>,
) -> Result<BTreeMap<String, SnapshotTreeEntry>, String> {
    if let Some(entries) = tree_cache.get(tree_id) {
        return Ok(entries.clone());
    }
    let payload = snapshot_reader
        .read_tree_payload(tree_id)?
        .ok_or_else(|| format!("missing tree payload `{tree_id}`"))?;
    let entries = parse_tree_entries(tree_id, &payload)?;
    tree_cache.insert(tree_id.to_string(), entries.clone());
    Ok(entries)
}

pub(super) fn parse_tree_entries(
    tree_id: &str,
    payload: &JsonValue,
) -> Result<BTreeMap<String, SnapshotTreeEntry>, String> {
    let rows = match payload {
        JsonValue::Object(map) => map
            .get("entries")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| format!("tree `{tree_id}` payload is missing `entries`"))?,
        JsonValue::Array(rows) => rows,
        _ => {
            return Err(format!(
                "tree `{tree_id}` payload must be an object with `entries` or a row array"
            ))
        }
    };
    let mut out = BTreeMap::new();
    for row in rows {
        let JsonValue::Object(row_obj) = row else {
            return Err(format!("tree `{tree_id}` entry rows must be objects"));
        };
        let entry_name = required_text_field(row_obj, "entry_name")
            .or_else(|_| required_text_field(row_obj, "name"))?;
        if out.contains_key(&entry_name) {
            return Err(format!(
                "tree `{tree_id}` contains duplicate entry `{entry_name}`"
            ));
        }
        let entry_type = required_text_field(row_obj, "entry_type")
            .or_else(|_| required_text_field(row_obj, "type"))?;
        let mode_raw = row_obj.get("mode").cloned().unwrap_or(JsonValue::Null);
        let mode_int = if entry_type == "tree" {
            0
        } else {
            to_mode_int(&mode_raw)?
        };
        out.insert(
            entry_name,
            SnapshotTreeEntry {
                entry_type,
                target_id: required_text_field(row_obj, "target_id")?,
                size_bytes: match row_obj.get("size_bytes") {
                    Some(value) => parse_optional_i64(value)?,
                    None => None,
                },
                mode_int,
                mode_raw,
            },
        );
    }
    Ok(out)
}

pub(super) fn snapshot_file_row_from_tree_entry(
    path: &str,
    entry: &SnapshotTreeEntry,
) -> Result<SnapshotFileRow, String> {
    if entry.entry_type != "blob" {
        return Err(format!(
            "tree entry `{path}` must be a blob to build a file-row payload"
        ));
    }
    Ok(SnapshotFileRow {
        path: path.to_string(),
        blob_id: Some(entry.target_id.clone()),
        size_bytes: entry.size_bytes,
        mode_raw: entry.mode_raw.clone(),
        mode_int: entry.mode_int,
    })
}

pub(super) fn size_bytes_changed(old_size: Option<i64>, new_size: Option<i64>) -> bool {
    match (old_size, new_size) {
        (Some(old_size), Some(new_size)) => old_size != new_size,
        _ => false,
    }
}

pub(super) fn record_blob_change(
    path: &str,
    old_entry: &SnapshotTreeEntry,
    new_entry: &SnapshotTreeEntry,
    state: &mut SnapshotTreeDiffState,
) -> Result<(), String> {
    let old_row = snapshot_file_row_from_tree_entry(path, old_entry)?;
    let new_row = snapshot_file_row_from_tree_entry(path, new_entry)?;
    let mode_only = old_row.mode_int != new_row.mode_int && old_row.blob_id == new_row.blob_id;
    let content_changed = old_row.blob_id != new_row.blob_id
        || size_bytes_changed(old_row.size_bytes, new_row.size_bytes);
    if !(mode_only || content_changed) {
        return Ok(());
    }
    let status = if mode_only {
        state.mode_changed.push(path.to_string());
        "mode_changed"
    } else {
        state.modified.push(path.to_string());
        "modified"
    };
    state.old_rows.insert(path.to_string(), old_row.clone());
    state.new_rows.insert(path.to_string(), new_row.clone());
    state.file_entries.push(json!({
        "path": path,
        "status": status,
        "old": file_row_payload(Some(&old_row)),
        "new": file_row_payload(Some(&new_row)),
    }));
    Ok(())
}

pub(super) fn push_added_row(state: &mut SnapshotTreeDiffState, row: SnapshotFileRow) {
    let path = row.path.clone();
    state.added.push(path.clone());
    state.new_rows.insert(path.clone(), row.clone());
    state.file_entries.push(json!({
        "path": path,
        "status": "added",
        "old": file_row_payload(None),
        "new": file_row_payload(Some(&row)),
    }));
}

pub(super) fn push_deleted_row(state: &mut SnapshotTreeDiffState, row: SnapshotFileRow) {
    let path = row.path.clone();
    state.deleted.push(path.clone());
    state.old_rows.insert(path.clone(), row.clone());
    state.file_entries.push(json!({
        "path": path,
        "status": "deleted",
        "old": file_row_payload(Some(&row)),
        "new": file_row_payload(None),
    }));
}

pub(super) fn join_tree_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

pub fn diff_snapshot_manifests(
    old_files: &JsonValue,
    new_files: &JsonValue,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
) -> Result<JsonValue, String> {
    SnapshotJson::stateless().diff_snapshot_manifests(
        old_files,
        new_files,
        old_snapshot_id,
        new_snapshot_id,
    )
}

pub(crate) fn diff_snapshot_manifests_impl(
    old_files: &JsonValue,
    new_files: &JsonValue,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
) -> Result<JsonValue, String> {
    let old_map = coerce_snapshot_manifest(old_files)?;
    let new_map = coerce_snapshot_manifest(new_files)?;
    let old_keys = old_map.keys().cloned().collect::<BTreeSet<_>>();
    let new_keys = new_map.keys().cloned().collect::<BTreeSet<_>>();

    let added = new_keys.difference(&old_keys).cloned().collect::<Vec<_>>();
    let deleted = old_keys.difference(&new_keys).cloned().collect::<Vec<_>>();
    let mut modified = Vec::new();
    let mut mode_changed = Vec::new();
    let mut file_entries = Vec::new();

    for path in &added {
        let new_row = new_map
            .get(path)
            .ok_or_else(|| format!("missing added path `{path}` in new snapshot manifest"))?;
        file_entries.push(json!({
            "path": path,
            "status": "added",
            "old": file_row_payload(None),
            "new": file_row_payload(Some(new_row)),
        }));
    }

    for path in &deleted {
        let old_row = old_map
            .get(path)
            .ok_or_else(|| format!("missing deleted path `{path}` in old snapshot manifest"))?;
        file_entries.push(json!({
            "path": path,
            "status": "deleted",
            "old": file_row_payload(Some(old_row)),
            "new": file_row_payload(None),
        }));
    }

    for path in old_keys.intersection(&new_keys) {
        let old_row = old_map
            .get(path)
            .ok_or_else(|| format!("missing shared path `{path}` in old snapshot manifest"))?;
        let new_row = new_map
            .get(path)
            .ok_or_else(|| format!("missing shared path `{path}` in new snapshot manifest"))?;
        let mode_only = old_row.mode_int != new_row.mode_int && old_row.blob_id == new_row.blob_id;
        let content_changed = old_row.blob_id != new_row.blob_id
            || size_bytes_changed(old_row.size_bytes, new_row.size_bytes);

        let status = if mode_only {
            mode_changed.push(path.clone());
            "mode_changed"
        } else if content_changed {
            modified.push(path.clone());
            "modified"
        } else {
            continue;
        };

        file_entries.push(json!({
            "path": path,
            "status": status,
            "old": file_row_payload(Some(old_row)),
            "new": file_row_payload(Some(new_row)),
        }));
    }

    let rename_hints = build_rename_hints(&old_map, &new_map, &added, &deleted);
    let directory_move_hints = build_directory_move_hints(&rename_hints);
    let files_changed = added.len() + deleted.len() + modified.len() + mode_changed.len();

    Ok(json!({
        "old_snapshot_id": old_snapshot_id,
        "new_snapshot_id": new_snapshot_id,
        "added": added,
        "deleted": deleted,
        "modified": modified,
        "mode_changed": mode_changed,
        "rename_hints": rename_hints.into_iter().map(rename_hint_json).collect::<Vec<_>>(),
        "directory_move_hints": directory_move_hints,
        "files": file_entries,
        "summary": {
            "files_changed": files_changed,
            "insertions": 0,
            "deletions": 0,
        },
    }))
}

pub fn snapshot_diff_from_manifests(
    old_files: &JsonValue,
    new_files: &JsonValue,
    blob_bytes_by_id: &BTreeMap<String, Vec<u8>>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    SnapshotJson::stateless().snapshot_diff_from_manifests(
        old_files,
        new_files,
        blob_bytes_by_id,
        old_snapshot_id,
        new_snapshot_id,
        include_text,
        max_bytes,
    )
}

pub(crate) fn snapshot_diff_from_manifests_impl(
    old_files: &JsonValue,
    new_files: &JsonValue,
    blob_bytes_by_id: &BTreeMap<String, Vec<u8>>,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    include_text: bool,
    max_bytes: usize,
) -> Result<JsonValue, String> {
    let old_map = coerce_snapshot_manifest(old_files)?;
    let new_map = coerce_snapshot_manifest(new_files)?;
    let mut result =
        diff_snapshot_manifests_impl(old_files, new_files, old_snapshot_id, new_snapshot_id)?;
    let Some(root) = result.as_object_mut() else {
        return Err("snapshot diff payload must be an object".to_string());
    };
    {
        let Some(summary) = root.get_mut("summary").and_then(JsonValue::as_object_mut) else {
            return Err("snapshot diff payload summary must be an object".to_string());
        };
        summary.insert(
            "old_snapshot_id".to_string(),
            optional_string_json(old_snapshot_id),
        );
        summary.insert(
            "new_snapshot_id".to_string(),
            optional_string_json(new_snapshot_id),
        );
    }

    if !include_text {
        return Ok(result);
    }

    let Some(files) = root.get_mut("files").and_then(JsonValue::as_array_mut) else {
        return Err("snapshot diff payload files must be a list".to_string());
    };
    let mut total_insertions = 0usize;
    let mut total_deletions = 0usize;

    for file_row in files.iter_mut() {
        let Some(file_map) = file_row.as_object_mut() else {
            return Err("snapshot diff file row must be an object".to_string());
        };
        let status = string_field(file_map, "status")?;
        if status != "modified" {
            file_map.insert(
                "diff".to_string(),
                text_diff_json(&TextDiffPayload {
                    status: "metadata_only",
                    insertions: 0,
                    deletions: 0,
                    text: None,
                }),
            );
            continue;
        }
        let path = string_field(file_map, "path")?;
        let old_row = old_map
            .get(&path)
            .ok_or_else(|| format!("missing modified path `{path}` in old snapshot manifest"))?;
        let new_row = new_map
            .get(&path)
            .ok_or_else(|| format!("missing modified path `{path}` in new snapshot manifest"))?;
        let text_diff = maybe_add_text_diff_from_blob_bytes(
            blob_bytes_by_id,
            &path,
            old_row,
            new_row,
            old_snapshot_id,
            new_snapshot_id,
            max_bytes,
        );
        total_insertions += text_diff.insertions;
        total_deletions += text_diff.deletions;
        file_map.insert("diff".to_string(), text_diff_json(&text_diff));
    }

    let Some(summary) = root.get_mut("summary").and_then(JsonValue::as_object_mut) else {
        return Err("snapshot diff payload summary must be an object".to_string());
    };
    summary.insert(
        "insertions".to_string(),
        JsonValue::Number(Number::from(total_insertions as u64)),
    );
    summary.insert(
        "deletions".to_string(),
        JsonValue::Number(Number::from(total_deletions as u64)),
    );
    Ok(result)
}
