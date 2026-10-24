//! Disclose Plan-backed Markdown reachable from a new Task worktree.
//!
//! The Task start refusal and the finish gate both know which paths travel
//! through Plan lineage instead of code Snapshots. Saying so at start time
//! saves the round-trip of learning it from a later refusal.

use super::*;
use crate::primitives::workspace::{
    collect_planning_only_artifact_drift_paths, plan_markdown_head_groups,
    render_planning_only_artifact_drift_error,
};
use ait_core::plan_sync_execution::execute_plan_sync_command_request_json;

const PLAN_BINARY_DB_WRITE_LAYOUT: u32 = 1;
pub(crate) const PLAN_MARKDOWN_PRESYNC_PAYLOAD_KEY: &str = "plan_markdown_presync";

pub(crate) const PLAN_BACKED_MARKDOWN_PAYLOAD_KEY: &str = "plan_backed_markdown";
const PLAN_BACKED_PATHS_SHOWN: usize = 8;

/// Never fails: the Task is already started, so a summary problem is reported
/// inside the object.
pub(crate) fn plan_backed_markdown_summary(repo: &RepoRuntime, edit_root: &Path) -> JsonValue {
    let shared_links = ["docs"]
        .iter()
        .filter(|name| {
            fs::symlink_metadata(edit_root.join(name))
                .map(|metadata| metadata.file_type().is_symlink())
                .unwrap_or(false)
        })
        .map(|name| JsonValue::String((*name).to_string()))
        .collect::<Vec<_>>();
    let tracked = match plan_markdown_head_groups(repo) {
        Ok(groups) => groups.into_keys().collect::<Vec<_>>(),
        Err(error) => {
            return json!({
                "status": "unknown",
                "error": error,
                "shared_links": shared_links,
                "paths": [],
            })
        }
    };
    let reachable = tracked
        .iter()
        .filter(|path| edit_root.join(path).exists())
        .cloned()
        .collect::<Vec<_>>();
    let absent = tracked.len().saturating_sub(reachable.len());
    json!({
        "status": "ok",
        "shared_links": shared_links,
        "paths": reachable,
        "tracked_count": tracked.len(),
        "absent_in_worktree_count": absent,
        "sync_command_template": "ait plan sync <path> --local",
        "detail": "These Markdown paths travel through Plan lineage, not code Snapshots. After editing one, run `ait plan sync <path> --local` before `ait task finish`; the finish gate refuses unsynced drift.",
    })
}

pub(crate) fn plan_backed_markdown_text_lines(payload: &JsonValue) -> Vec<String> {
    let Some(summary) = payload
        .get(PLAN_BACKED_MARKDOWN_PAYLOAD_KEY)
        .and_then(JsonValue::as_object)
    else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    let shared = summary
        .get("shared_links")
        .and_then(JsonValue::as_array)
        .map(|links| {
            links
                .iter()
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for link in &shared {
        lines.push(format!(
            "plan-backed: {link}/ is a shared link to the repository root; edits there are canonical"
        ));
    }
    let paths = summary
        .get("paths")
        .and_then(JsonValue::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(JsonValue::as_str)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if paths.is_empty() {
        if let Some(error) = summary.get("error").and_then(JsonValue::as_str) {
            lines.push(format!("plan-backed: unknown ({error})"));
        }
        return lines;
    }
    let shown = paths
        .iter()
        .take(PLAN_BACKED_PATHS_SHOWN)
        .copied()
        .collect::<Vec<_>>();
    let more = paths.len().saturating_sub(shown.len());
    let mut listed = shown.join(", ");
    if more > 0 {
        listed.push_str(&format!(" and {more} more"));
    }
    lines.push(format!(
        "plan-backed: {} Markdown path(s) sync with `ait plan sync <path> --local` before finish: {listed}",
        paths.len()
    ));
    lines
}

/// The active workspace owns authored Markdown, except when a Task worktree
/// target resolves through a shared link (such as `docs`) into the canonical
/// repository; that file is the canonical file and must sync against the root.
pub(crate) fn plan_sync_root_path(repo: &RepoRuntime, target: &Path) -> PathBuf {
    let workspace_root = repo.workspace_root();
    if !repo.is_worktree() {
        return workspace_root;
    }
    let authoritative_root = repo.authoritative_repo_root();
    let candidate = if target.is_absolute() {
        target.to_path_buf()
    } else {
        workspace_root.join(target)
    };
    let (Ok(resolved), Ok(canonical_workspace), Ok(canonical_root)) = (
        canonicalize_existing_prefix(&candidate),
        workspace_root.canonicalize(),
        authoritative_root.canonicalize(),
    ) else {
        return workspace_root;
    };
    if !resolved.starts_with(&canonical_workspace) && resolved.starts_with(&canonical_root) {
        authoritative_root
    } else {
        workspace_root
    }
}

/// Canonicalize the longest existing prefix so a not-yet-created or deleted
/// file under a symlinked directory still resolves to its real parent.
pub(crate) fn canonicalize_existing_prefix(path: &Path) -> std::io::Result<PathBuf> {
    let mut missing = Vec::new();
    let mut cursor = path.to_path_buf();
    loop {
        match cursor.canonicalize() {
            Ok(mut resolved) => {
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return Ok(resolved);
            }
            Err(error) => {
                let Some(name) = cursor.file_name().map(|name| name.to_os_string()) else {
                    return Err(error);
                };
                missing.push(name);
                if !cursor.pop() {
                    return Err(error);
                }
            }
        }
    }
}

fn presync_request(
    repo: &RepoRuntime,
    root_path: &Path,
    artifact_path: &str,
) -> Result<JsonValue, String> {
    Ok(json!({
        "root_path": root_path,
        "repo_name": repo.repo_name(),
        "repository_index": repo.repository_index(),
        "id_namespace_prefix": repo.id_namespace_prefix(),
        "created_by": repo.actor_identity(),
        "target": artifact_path,
        "plan_ref": JsonValue::Null,
        "prune": false,
        "local": true,
        "remote_name": JsonValue::Null,
        "remote_repo_name": JsonValue::Null,
        "base_url": JsonValue::Null,
        "rebase": false,
        "reconcile": false,
        "plan_storage": repo.plan_binary_db_storage_request::<PLAN_BINARY_DB_WRITE_LAYOUT>()?,
    }))
}

/// Sync every drifted Plan-backed Markdown path reachable from the current
/// workspace before a local finish. Unreachable drift (a deleted file, or a
/// path only the repository root could see) is still refused with the exact
/// command, because syncing it here would hide a decision.
pub(crate) fn presync_plan_markdown_drift(repo: &RepoRuntime) -> Result<JsonValue, String> {
    let drift = collect_planning_only_artifact_drift_paths(repo)?;
    if drift.is_empty() {
        return Ok(json!({"status": "in_sync", "synced": [], "count": 0}));
    }
    let mut synced = Vec::new();
    let mut unreachable = Vec::new();
    for artifact_path in drift {
        let root_path = plan_sync_root_path(repo, Path::new(&artifact_path));
        if !root_path.join(&artifact_path).is_file() {
            unreachable.push((artifact_path, true));
            continue;
        }
        let response = execute_plan_sync_command_request_json(
            &presync_request(repo, &root_path, &artifact_path)?.to_string(),
        )?;
        if response.get("status").and_then(JsonValue::as_str) != Some("ok") {
            let detail = response
                .get("error")
                .map(|error| {
                    error
                        .get("message")
                        .and_then(JsonValue::as_str)
                        .map(ToString::to_string)
                        .unwrap_or_else(|| error.to_string())
                })
                .unwrap_or_else(|| "plan sync returned a non-ok result".to_string());
            return Err(format!(
                "`ait task finish --sync-plan-markdown` could not sync {artifact_path}: {detail} Run `ait plan sync {artifact_path} --local` and retry."
            ));
        }
        synced.push(json!({
            "artifact_path": artifact_path,
            "root_path": root_path,
            "results": response.get("results").cloned().unwrap_or(JsonValue::Null),
        }));
    }
    if !unreachable.is_empty() {
        return Err(render_planning_only_artifact_drift_error(
            "ait task finish --sync-plan-markdown",
            &unreachable,
        ));
    }
    Ok(json!({"status": "synced", "count": synced.len(), "synced": synced}))
}

pub(crate) fn plan_markdown_presync_text_lines(payload: &JsonValue) -> Vec<String> {
    payload
        .get(PLAN_MARKDOWN_PRESYNC_PAYLOAD_KEY)
        .and_then(|value| value.get("synced"))
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| row.get("artifact_path").and_then(JsonValue::as_str))
        .map(|path| format!("plan markdown presync: synced {path}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_lines_name_shared_links_and_bounded_paths() {
        let payload = json!({
            PLAN_BACKED_MARKDOWN_PAYLOAD_KEY: {
                "status": "ok",
                "shared_links": ["docs"],
                "paths": (0..10).map(|index| format!("docs/sprints/card-{index}.md")).collect::<Vec<_>>(),
            }
        });
        let lines = plan_backed_markdown_text_lines(&payload);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].starts_with("plan-backed: docs/ is a shared link"),
            "{lines:?}"
        );
        assert!(lines[1].contains("10 Markdown path(s)"), "{lines:?}");
        assert!(
            lines[1].contains("`ait plan sync <path> --local`"),
            "{lines:?}"
        );
        assert!(lines[1].ends_with(" and 2 more"), "{lines:?}");
        assert!(plan_backed_markdown_text_lines(&json!({})).is_empty());
    }

    #[test]
    fn presync_text_lines_name_each_synced_path() {
        let lines = plan_markdown_presync_text_lines(&json!({
            PLAN_MARKDOWN_PRESYNC_PAYLOAD_KEY: {"synced": [{"artifact_path": "docs/a.md"}]}
        }));
        assert_eq!(lines, vec!["plan markdown presync: synced docs/a.md"]);
        assert!(plan_markdown_presync_text_lines(&json!({})).is_empty());
    }

    #[test]
    fn existing_prefix_canonicalization_keeps_missing_tail() {
        let temp = tempfile::TempDir::new().unwrap();
        let base = temp.path().canonicalize().unwrap();
        let resolved = canonicalize_existing_prefix(&temp.path().join("missing/leaf.md")).unwrap();
        assert_eq!(resolved, base.join("missing/leaf.md"));
    }
}
