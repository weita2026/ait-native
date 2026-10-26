use super::*;

pub(super) fn build_rename_hints(
    old_map: &BTreeMap<String, SnapshotFileRow>,
    new_map: &BTreeMap<String, SnapshotFileRow>,
    added: &[String],
    deleted: &[String],
) -> Vec<RenameHint> {
    let mut deleted_by_blob_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut added_by_blob_id: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for path in deleted {
        if let Some(blob_id) = old_map
            .get(path)
            .and_then(|row| row.blob_id.clone())
            .filter(|value| !value.trim().is_empty())
        {
            deleted_by_blob_id
                .entry(blob_id)
                .or_default()
                .push(path.clone());
        }
    }
    for path in added {
        if let Some(blob_id) = new_map
            .get(path)
            .and_then(|row| row.blob_id.clone())
            .filter(|value| !value.trim().is_empty())
        {
            added_by_blob_id
                .entry(blob_id)
                .or_default()
                .push(path.clone());
        }
    }

    let mut hints = Vec::new();
    for blob_id in deleted_by_blob_id.keys() {
        let Some(old_paths) = deleted_by_blob_id.get(blob_id) else {
            continue;
        };
        let Some(new_paths) = added_by_blob_id.get(blob_id) else {
            continue;
        };
        if old_paths.len() != 1 || new_paths.len() != 1 {
            continue;
        }
        let old_path = old_paths[0].clone();
        let new_path = new_paths[0].clone();
        let old_row = match old_map.get(&old_path) {
            Some(value) => value,
            None => continue,
        };
        let new_row = match new_map.get(&new_path) {
            Some(value) => value,
            None => continue,
        };
        hints.push(RenameHint {
            blob_id: blob_id.clone(),
            old_path: old_path.clone(),
            new_path: new_path.clone(),
            old_parent_path: parent_path(&old_path),
            new_parent_path: parent_path(&new_path),
            size_bytes: new_row.size_bytes.or(old_row.size_bytes).unwrap_or(0),
        });
    }
    hints.sort_by(|left, right| {
        (&left.old_path, &left.new_path, &left.blob_id).cmp(&(
            &right.old_path,
            &right.new_path,
            &right.blob_id,
        ))
    });
    hints
}

pub(super) fn build_directory_move_hints(rename_hints: &[RenameHint]) -> Vec<JsonValue> {
    let mut old_to_new: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut new_to_old: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut grouped: BTreeMap<(String, String), Vec<&RenameHint>> = BTreeMap::new();

    for hint in rename_hints {
        if hint.old_parent_path.is_empty()
            || hint.new_parent_path.is_empty()
            || hint.old_parent_path == hint.new_parent_path
        {
            continue;
        }
        old_to_new
            .entry(hint.old_parent_path.clone())
            .or_default()
            .insert(hint.new_parent_path.clone());
        new_to_old
            .entry(hint.new_parent_path.clone())
            .or_default()
            .insert(hint.old_parent_path.clone());
        grouped
            .entry((hint.old_parent_path.clone(), hint.new_parent_path.clone()))
            .or_default()
            .push(hint);
    }

    let mut hints = Vec::new();
    for ((old_parent, new_parent), matched) in grouped {
        if matched.len() < 2 {
            continue;
        }
        if old_to_new.get(&old_parent).map(|value| value.len()) != Some(1)
            || new_to_old.get(&new_parent).map(|value| value.len()) != Some(1)
        {
            continue;
        }
        let mut renames = matched
            .iter()
            .map(|hint| {
                json!({
                    "old_path": hint.old_path,
                    "new_path": hint.new_path,
                    "blob_id": hint.blob_id,
                })
            })
            .collect::<Vec<_>>();
        renames.sort_by(|left, right| {
            let left_old = left
                .get("old_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let left_new = left
                .get("new_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let right_old = right
                .get("old_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            let right_new = right
                .get("new_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("");
            (left_old, left_new).cmp(&(right_old, right_new))
        });
        hints.push(json!({
            "match_kind": "exact_blob_id_group",
            "old_parent_path": old_parent,
            "new_parent_path": new_parent,
            "rename_count": renames.len(),
            "renames": renames,
        }));
    }
    hints
}

pub(super) fn parent_path(path: &str) -> String {
    match path.rsplit_once('/') {
        Some((prefix, _)) if !prefix.is_empty() => prefix.to_string(),
        _ => ".".to_string(),
    }
}
