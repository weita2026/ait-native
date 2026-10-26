use super::*;

pub(super) fn file_row_payload(row: Option<&SnapshotFileRow>) -> JsonValue {
    match row {
        Some(row) => json!({
            "blob_id": row.blob_id,
            "size_bytes": row.size_bytes,
            "mode": row.mode_raw,
        }),
        None => json!({
            "blob_id": JsonValue::Null,
            "size_bytes": JsonValue::Null,
            "mode": JsonValue::Null,
        }),
    }
}

pub(super) fn rename_hint_json(hint: RenameHint) -> JsonValue {
    json!({
        "match_kind": "exact_blob_id",
        "blob_id": hint.blob_id,
        "old_path": hint.old_path,
        "new_path": hint.new_path,
        "old_parent_path": hint.old_parent_path,
        "new_parent_path": hint.new_parent_path,
        "size_bytes": hint.size_bytes,
    })
}

pub(in crate::object_diff) fn maybe_add_text_diff_from_blob_bytes(
    blob_bytes_by_id: &BTreeMap<String, Vec<u8>>,
    path: &str,
    old_row: &SnapshotFileRow,
    new_row: &SnapshotFileRow,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
    max_bytes: usize,
) -> TextDiffPayload {
    let Some(old_blob_id) = old_row.blob_id.as_ref() else {
        return TextDiffPayload {
            status: "unavailable",
            insertions: 0,
            deletions: 0,
            text: None,
        };
    };
    let Some(new_blob_id) = new_row.blob_id.as_ref() else {
        return TextDiffPayload {
            status: "unavailable",
            insertions: 0,
            deletions: 0,
            text: None,
        };
    };
    let Some(old_data) = blob_bytes_by_id.get(old_blob_id) else {
        return TextDiffPayload {
            status: "unavailable",
            insertions: 0,
            deletions: 0,
            text: None,
        };
    };
    let Some(new_data) = blob_bytes_by_id.get(new_blob_id) else {
        return TextDiffPayload {
            status: "unavailable",
            insertions: 0,
            deletions: 0,
            text: None,
        };
    };
    let (old_is_text, old_text, old_reason) = safe_decode_text(old_data, max_bytes);
    if !old_is_text {
        return TextDiffPayload {
            status: old_reason.unwrap_or("binary"),
            insertions: 0,
            deletions: 0,
            text: None,
        };
    }
    let (new_is_text, new_text, new_reason) = safe_decode_text(new_data, max_bytes);
    if !new_is_text {
        return TextDiffPayload {
            status: new_reason.unwrap_or("binary"),
            insertions: 0,
            deletions: 0,
            text: None,
        };
    }
    build_text_diff(
        path,
        old_text.as_deref().unwrap_or(""),
        new_text.as_deref().unwrap_or(""),
        old_snapshot_id,
        new_snapshot_id,
    )
}

pub(in crate::object_diff) fn safe_decode_text(
    data: &[u8],
    max_bytes: usize,
) -> (bool, Option<String>, Option<&'static str>) {
    if data.len() > max_bytes {
        return (false, None, Some("too_large"));
    }
    if data.contains(&0) {
        return (false, None, Some("binary"));
    }
    match String::from_utf8(data.to_vec()) {
        Ok(text) => (true, Some(text), None),
        Err(_) => (false, None, Some("binary")),
    }
}

pub(super) fn build_text_diff(
    path: &str,
    old_text: &str,
    new_text: &str,
    old_snapshot_id: Option<&str>,
    new_snapshot_id: Option<&str>,
) -> TextDiffPayload {
    let diff = TextDiff::from_lines(old_text, new_text);
    let from_label = format!("{}:{path}", old_snapshot_id.unwrap_or("?"));
    let to_label = format!("{}:{path}", new_snapshot_id.unwrap_or("?"));
    let mut insertions = 0usize;
    let mut deletions = 0usize;
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => insertions += 1,
            ChangeTag::Delete => deletions += 1,
            ChangeTag::Equal => {}
        }
    }
    let text = diff
        .unified_diff()
        .context_radius(3)
        .header(&from_label, &to_label)
        .to_string();
    TextDiffPayload {
        status: "text",
        insertions,
        deletions,
        text: Some(text),
    }
}

pub(super) fn text_diff_json(value: &TextDiffPayload) -> JsonValue {
    json!({
        "status": value.status,
        "insertions": value.insertions,
        "deletions": value.deletions,
        "text": value.text,
    })
}
