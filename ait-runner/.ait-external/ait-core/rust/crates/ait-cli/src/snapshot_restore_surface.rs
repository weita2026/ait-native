use crate::runtime::{RepoRuntime, SNAPSHOT_BINARY_DB_WRITE_LAYOUT};
use crate::workspace_lock::run_locked_workspace_command;
use ait_core::json_support::{json, JsonValue};
use ait_core::local_snapshot::{
    LocalSnapshotBlobReadStore, LocalSnapshotReadStore, LocalSnapshotTreeReadStore,
};
use ait_core::plan_filesystem::is_lineage_only_markdown_artifact_path;
use std::fs;
use std::path::{Component, Path, PathBuf};

const SNAPSHOT_RESTORE_LINES_CONTRACT: &str = "ait.snapshot-restore-lines/v1";

#[derive(Clone, Debug)]
pub struct SnapshotRestoreLinesRequest {
    pub snapshot_id: String,
    pub path: String,
    pub line: Option<usize>,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub apply: bool,
}

pub fn snapshot_restore_lines(
    repo: &RepoRuntime,
    request: &SnapshotRestoreLinesRequest,
) -> Result<JsonValue, String> {
    if request.apply {
        return run_locked_workspace_command(repo, "ait-cli snapshot restore-lines", || {
            snapshot_restore_lines_unlocked(repo, request)
        });
    }
    snapshot_restore_lines_unlocked(repo, request)
}

fn snapshot_restore_lines_unlocked(
    repo: &RepoRuntime,
    request: &SnapshotRestoreLinesRequest,
) -> Result<JsonValue, String> {
    let snapshot_id = require_non_empty(&request.snapshot_id, "Snapshot ID")?;
    let rel_path = normalize_workspace_path(repo, &request.path)?;
    if is_lineage_only_markdown_artifact_path(&rel_path) {
        return Err(format!(
            "Path {rel_path} is a planning-only Markdown file and cannot be restored from a Snapshot."
        ));
    }

    let store = repo.local_snapshot_operation_store::<SNAPSHOT_BINARY_DB_WRITE_LAYOUT>(
        &repo.workspace_root(),
    )?;
    store
        .get_snapshot(&snapshot_id)
        .map_err(|_| format!("Unknown snapshot: {snapshot_id}"))?;
    let row = store
        .snapshot_tree_path_row(&snapshot_id, &rel_path)?
        .ok_or_else(|| format!("Path {rel_path} does not exist in snapshot {snapshot_id}."))?;
    let blob_id = row
        .get("blob_id")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            format!("Snapshot {snapshot_id} path {rel_path} is missing its Blob identity.")
        })?;
    let source_bytes = store.read_blob_bytes(blob_id)?;
    let source_lines =
        decode_text_lines(&source_bytes, &format!("Snapshot {snapshot_id}:{rel_path}"))?;
    let (start_line, end_line) = selected_range(
        source_lines.len(),
        request.line,
        request.start_line,
        request.end_line,
    )?;

    let workspace_path = checked_workspace_file(repo, &rel_path)?;
    let workspace_bytes = fs::read(&workspace_path).map_err(|err| err.to_string())?;
    let workspace_lines =
        decode_text_lines(&workspace_bytes, &format!("Workspace file {rel_path}"))?;
    if end_line > workspace_lines.len() {
        return Err(format!(
            "Workspace file {rel_path} has only {} lines, so range {start_line}-{end_line} cannot be restored safely.",
            workspace_lines.len()
        ));
    }

    let source_selection = &source_lines[start_line - 1..end_line];
    let workspace_selection = &workspace_lines[start_line - 1..end_line];
    let changed_line_count = source_selection
        .iter()
        .zip(workspace_selection)
        .filter(|(source, workspace)| source != workspace)
        .count();
    let would_overwrite_selected_local_edits = changed_line_count > 0;
    let restored_lines = workspace_lines[..start_line - 1]
        .iter()
        .cloned()
        .chain(source_selection.iter().cloned())
        .chain(workspace_lines[end_line..].iter().cloned())
        .collect::<Vec<_>>();

    if request.apply && would_overwrite_selected_local_edits {
        fs::write(&workspace_path, restored_lines.concat()).map_err(|err| err.to_string())?;
    }

    Ok(json!({
        "contract": SNAPSHOT_RESTORE_LINES_CONTRACT,
        "mode": if request.apply { "applied" } else { "preview" },
        "snapshot_id": snapshot_id,
        "path": rel_path,
        "blob_id": blob_id,
        "selected_range": {
            "start": start_line,
            "end": end_line,
        },
        "selected_line_count": end_line - start_line + 1,
        "source_line_count": source_lines.len(),
        "workspace_line_count": workspace_lines.len(),
        "changed_line_count": changed_line_count,
        "would_overwrite_selected_local_edits": would_overwrite_selected_local_edits,
        "unchanged_outside_selected_range": true,
        "creates_snapshot": false,
        "applied": request.apply,
    }))
}

fn normalize_workspace_path(repo: &RepoRuntime, path_value: &str) -> Result<String, String> {
    let raw = path_value.trim();
    if raw.is_empty() {
        return Err("Path is required.".to_string());
    }
    let root = repo
        .workspace_root()
        .canonicalize()
        .map_err(|err| format!("Cannot resolve the current workspace root: {err}"))?;
    let candidate = PathBuf::from(raw);
    let normalized = if candidate.is_absolute() {
        lexical_normalize(&candidate)
    } else {
        lexical_normalize(&root.join(candidate))
    };
    let rel_path = normalized
        .strip_prefix(&root)
        .map_err(|_| format!("Path {raw:?} is outside the current workspace root."))?
        .to_string_lossy()
        .replace('\\', "/");
    if rel_path.is_empty() || rel_path == "." {
        return Err("Choose one file path to restore.".to_string());
    }
    Ok(rel_path)
}

fn checked_workspace_file(repo: &RepoRuntime, rel_path: &str) -> Result<PathBuf, String> {
    let root = repo
        .workspace_root()
        .canonicalize()
        .map_err(|err| format!("Cannot resolve the current workspace root: {err}"))?;
    let path = root.join(rel_path);
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| format!("Workspace file {rel_path} does not exist."))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Workspace file {rel_path} is a symbolic link and cannot be restored safely."
        ));
    }
    if !metadata.is_file() {
        return Err(format!("Path {rel_path} is not a regular workspace file."));
    }
    let canonical = path
        .canonicalize()
        .map_err(|err| format!("Cannot resolve workspace file {rel_path}: {err}"))?;
    if !canonical.starts_with(&root) {
        return Err(format!(
            "Workspace file {rel_path} resolves outside the current workspace root."
        ));
    }
    Ok(path)
}

fn selected_range(
    total_lines: usize,
    line: Option<usize>,
    start_line: Option<usize>,
    end_line: Option<usize>,
) -> Result<(usize, usize), String> {
    if line.is_some() && (start_line.is_some() || end_line.is_some()) {
        return Err("Choose either --line or --start/--end.".to_string());
    }
    let (start_line, end_line) = if let Some(line) = line {
        (Some(line), Some(line))
    } else {
        (start_line, end_line)
    };
    let Some(start_line) = start_line else {
        return Err("Select one line with --line or one range with --start/--end.".to_string());
    };
    let Some(end_line) = end_line else {
        return Err("Provide both --start and --end.".to_string());
    };
    if start_line == 0 || end_line == 0 {
        return Err("Line selections are 1-based and must be positive.".to_string());
    }
    if end_line < start_line {
        return Err("The selected range must have end >= start.".to_string());
    }
    if total_lines == 0 {
        return Err("The selected Snapshot file is empty and has no restorable lines.".to_string());
    }
    if end_line > total_lines {
        return Err(format!(
            "Selected range {start_line}-{end_line} exceeds Snapshot file length {total_lines}."
        ));
    }
    Ok((start_line, end_line))
}

fn decode_text_lines(data: &[u8], label: &str) -> Result<Vec<String>, String> {
    if data.contains(&0) {
        return Err(format!(
            "{label} is binary and cannot be restored as lines."
        ));
    }
    let text = std::str::from_utf8(data)
        .map_err(|_| format!("{label} is not valid UTF-8 text and cannot be restored as lines."))?;
    Ok(text.split_inclusive('\n').map(str::to_string).collect())
}

fn require_non_empty(value: &str, label: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("{label} is required."));
    }
    Ok(value.to_string())
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_range_requires_one_complete_positive_selection() {
        assert_eq!(selected_range(4, Some(2), None, None).unwrap(), (2, 2));
        assert_eq!(selected_range(4, None, Some(2), Some(4)).unwrap(), (2, 4));
        assert!(selected_range(4, None, None, None).is_err());
        assert!(selected_range(4, Some(1), Some(1), Some(1)).is_err());
        assert!(selected_range(4, None, Some(1), None).is_err());
        assert!(selected_range(4, None, Some(0), Some(1)).is_err());
        assert!(selected_range(4, None, Some(3), Some(2)).is_err());
        assert!(selected_range(4, None, Some(1), Some(5)).is_err());
    }

    #[test]
    fn text_lines_preserve_newline_and_non_newline_endings() {
        assert_eq!(
            decode_text_lines(b"one\ntwo\n", "fixture").unwrap(),
            vec!["one\n", "two\n"]
        );
        assert_eq!(
            decode_text_lines(b"one\ntwo", "fixture").unwrap(),
            vec!["one\n", "two"]
        );
        assert!(decode_text_lines(b"one\0two", "fixture").is_err());
        assert!(decode_text_lines(&[0xff], "fixture").is_err());
    }
}
