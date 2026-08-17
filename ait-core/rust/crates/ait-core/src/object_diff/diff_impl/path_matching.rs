use super::*;

pub fn workspace_diff_from_entries(
    entries: &[WorkspaceDiffEntry],
    baseline_label: &str,
    workspace_label: &str,
    include_text: bool,
    max_bytes: usize,
) -> JsonValue {
    let mut modified = Vec::new();
    let mut missing = Vec::new();
    let mut untracked = Vec::new();
    let mut files = Vec::new();
    let mut total_insertions = 0usize;
    let mut total_deletions = 0usize;

    for entry in entries {
        match entry.status.as_str() {
            "modified" => modified.push(entry.path.clone()),
            "missing" => missing.push(entry.path.clone()),
            "untracked" => untracked.push(entry.path.clone()),
            _ => {}
        }

        let diff = if include_text {
            workspace_entry_text_diff(entry, baseline_label, workspace_label, max_bytes)
        } else {
            TextDiffPayload {
                status: "metadata_only",
                insertions: 0,
                deletions: 0,
                text: None,
            }
        };
        total_insertions += diff.insertions;
        total_deletions += diff.deletions;

        files.push(json!({
            "path": entry.path.clone(),
            "status": entry.status.clone(),
            "old_mode": entry.old_mode.clone(),
            "new_mode": entry.new_mode.clone(),
            "diff": text_diff_json(&diff),
        }));
    }

    json!({
        "modified": modified,
        "missing": missing,
        "untracked": untracked,
        "changed_paths": entries.iter().map(|entry| entry.path.clone()).collect::<Vec<_>>(),
        "files": files,
        "summary": {
            "files_changed": entries.len(),
            "insertions": total_insertions,
            "deletions": total_deletions,
        },
    })
}

pub(super) fn workspace_entry_text_diff(
    entry: &WorkspaceDiffEntry,
    baseline_label: &str,
    workspace_label: &str,
    max_bytes: usize,
) -> TextDiffPayload {
    let old_data = entry.old_bytes.as_deref().unwrap_or(&[]);
    let new_data = entry.new_bytes.as_deref().unwrap_or(&[]);
    if old_data == new_data {
        return TextDiffPayload {
            status: "metadata_only",
            insertions: 0,
            deletions: 0,
            text: None,
        };
    }
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
        &entry.path,
        old_text.as_deref().unwrap_or(""),
        new_text.as_deref().unwrap_or(""),
        Some(baseline_label),
        Some(workspace_label),
    )
}
