use super::*;

pub(super) fn binary_validate_existing_snapshot_payload(
    existing: &JsonValue,
    repo_name: &str,
    line_name: &str,
    parent_snapshot_id: Option<&str>,
    file_count: i64,
    total_bytes: i64,
) -> Result<(), NativeRepositoryError> {
    let existing_repo = binary_json_text(existing, "repo_name").unwrap_or_default();
    if existing_repo != repo_name {
        return Err(NativeRepositoryError::conflict(format!(
            "Snapshot {} belongs to repository {existing_repo}, not {repo_name}",
            binary_snapshot_id(existing).unwrap_or_default()
        )));
    }
    let existing_line = binary_json_text(existing, "line_name").unwrap_or_else(default_main_line);
    if existing_line != line_name {
        return Err(NativeRepositoryError::bad_request(format!(
            "Snapshot {} already exists with line_name={existing_line:?}, not {line_name:?}",
            binary_snapshot_id(existing).unwrap_or_default()
        )));
    }
    let existing_parent = binary_json_text(existing, "parent_snapshot_id");
    if existing_parent.as_deref() != parent_snapshot_id {
        return Err(NativeRepositoryError::bad_request(format!(
            "Snapshot {} already exists with parent_snapshot_id={:?}, not {:?}",
            binary_snapshot_id(existing).unwrap_or_default(),
            existing_parent,
            parent_snapshot_id
        )));
    }
    if existing
        .get("file_count")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        != file_count
    {
        return Err(NativeRepositoryError::bad_request(format!(
            "Snapshot {} already exists with different file_count",
            binary_snapshot_id(existing).unwrap_or_default()
        )));
    }
    if existing
        .get("total_bytes")
        .and_then(JsonValue::as_i64)
        .unwrap_or(0)
        != total_bytes
    {
        return Err(NativeRepositoryError::bad_request(format!(
            "Snapshot {} already exists with different total_bytes",
            binary_snapshot_id(existing).unwrap_or_default()
        )));
    }
    Ok(())
}

pub(super) fn binary_snapshot_export_json(
    value: &JsonValue,
    files: Vec<JsonValue>,
    query: &SnapshotExportQuery,
) -> Result<JsonValue, NativeRepositoryError> {
    let mut snapshot = value.as_object().cloned().ok_or_else(|| {
        NativeRepositoryError::internal("canonical Binary DB snapshot must be a JSON object")
    })?;
    let mut selected_files = Vec::new();
    for file in files {
        let object = file.as_object().ok_or_else(|| {
            NativeRepositoryError::bad_request("Binary DB snapshot file must be an object")
        })?;
        let path =
            required_json_text(object, "path").map_err(NativeRepositoryError::bad_request)?;
        let selected = query.path.as_deref().is_none_or(|filter| {
            path == filter
                || path
                    .strip_prefix(filter)
                    .is_some_and(|rest| rest.starts_with('/'))
        });
        if selected {
            let mut file = object.clone();
            file.remove("content_entry_name");
            selected_files.push(JsonValue::Object(file));
        }
    }
    let total_bytes = selected_files
        .iter()
        .filter_map(|file| file.get("size_bytes").and_then(JsonValue::as_i64))
        .sum::<i64>();
    snapshot.insert("file_count".to_string(), json!(selected_files.len()));
    snapshot.insert("total_bytes".to_string(), json!(total_bytes));
    snapshot.insert("content_included".to_string(), JsonValue::Bool(false));
    snapshot.insert("files".to_string(), JsonValue::Array(selected_files));
    Ok(JsonValue::Object(snapshot))
}
