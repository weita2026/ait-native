//! Materialize Plan-backed Markdown into the canonical repository root after a
//! local Task finish.
//!
//! Plan-backed Markdown travels through Plan revisions, not code Snapshots. A
//! sync from a Task worktree therefore updates the Plan head while the
//! canonical root copy stays at its previous content, and worktree cleanup
//! then discards the only materialized copy. This step writes the Plan head
//! body back into the root whenever the root copy is missing or matches an
//! earlier revision of the same Plan; genuine unsynced root drift is reported
//! with the exact repair command and never overwritten.

use super::*;
use crate::primitives::workspace::plan_markdown_head_groups;
use ait_core::object_diff::artifact_blob_id;

pub(in crate::primitives) const PLAN_MARKDOWN_MATERIALIZATION_PAYLOAD_KEY: &str =
    "plan_markdown_materialization";
const PLAN_BINARY_DB_LAYOUT: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
enum RootCopyState {
    InSync,
    Missing,
    MatchesRevision,
    UnsyncedDrift,
    AmbiguousHeads,
    Unreadable(String),
}

/// One artifact path may carry several Plans (multi-root sprint cards). The
/// root copy is in sync when it matches any active head; it is only
/// materialized when every active head agrees on one body.
fn classify_root_copy(
    current_blob_id: Option<Result<String, String>>,
    head_blob_ids: &[String],
    historical_blob_ids: &[String],
) -> RootCopyState {
    let distinct_heads = head_blob_ids.iter().collect::<BTreeSet<_>>();
    match current_blob_id {
        Some(Ok(blob_id)) if distinct_heads.contains(&blob_id) => RootCopyState::InSync,
        _ if distinct_heads.len() > 1 => RootCopyState::AmbiguousHeads,
        None => RootCopyState::Missing,
        Some(Err(error)) => RootCopyState::Unreadable(error),
        Some(Ok(blob_id)) if historical_blob_ids.contains(&blob_id) => {
            RootCopyState::MatchesRevision
        }
        Some(Ok(_)) => RootCopyState::UnsyncedDrift,
    }
}

fn current_root_blob_id(path: &Path) -> Option<Result<String, String>> {
    if !path.exists() {
        return None;
    }
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Some(Err(format!("{} is not a regular file", path.display()))),
        Err(error) => return Some(Err(error.to_string())),
    }
    Some(
        fs::read_to_string(path)
            .map(|text| artifact_blob_id(&text))
            .map_err(|error| error.to_string()),
    )
}

fn plan_sync_repair_command(artifact_path: &str) -> String {
    format!("ait plan sync {artifact_path} --local")
}

/// Never fails: every problem is reported inside the returned object because
/// the local finish that preceded this step is already applied.
pub(in crate::primitives) fn materialize_plan_markdown_after_local_finish(
    root_repo: &RepoRuntime,
) -> JsonValue {
    match materialize_plan_markdown_inner(root_repo) {
        Ok(result) => result,
        Err(error) => json!({
            "status": "failed",
            "error": error,
            "detail": "Plan-backed Markdown could not be checked against the local Plan heads. The Task finish itself is applied; run `ait plan sync <path> --local` for any Markdown edited in the Task worktree.",
        }),
    }
}

fn materialize_plan_markdown_inner(root_repo: &RepoRuntime) -> Result<JsonValue, String> {
    let root = root_repo.authoritative_repo_root();
    let stores = root_repo.binary_db_stores::<PLAN_BINARY_DB_LAYOUT>();
    let plans = stores.plans();
    let plan_read = plans.begin_read_txn();
    let content = stores.content();
    let blobs = content.blobs();
    let content_read = blobs.begin_read_txn();

    let grouped = plan_markdown_head_groups(root_repo)?
        .into_iter()
        .map(|(artifact_path, heads)| {
            (
                artifact_path,
                heads
                    .into_iter()
                    .filter_map(|head| head.artifact_blob_id.map(|blob| (head.plan_index, blob)))
                    .collect::<Vec<(u32, String)>>(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    let mut materialized = Vec::new();
    let mut drift = Vec::new();
    let mut skipped = Vec::new();
    let mut in_sync_count = 0_u64;
    for (artifact_path, heads) in grouped {
        let head_blob_ids = heads
            .iter()
            .map(|(_, blob_id)| blob_id.clone())
            .collect::<Vec<_>>();
        let absolute = root.join(&artifact_path);
        let current = current_root_blob_id(&absolute);
        let historical = if matches!(current, Some(Ok(_))) {
            let mut known = Vec::new();
            for (plan_index, _) in &heads {
                known.extend(
                    plans
                        .list_plan_revisions(&plan_read, *plan_index)
                        .map_err(|error| error.to_string())?
                        .iter()
                        .filter_map(|revision| revision.payload.artifact_blob_id_text().ok()),
                );
            }
            known
        } else {
            Vec::new()
        };
        match classify_root_copy(current, &head_blob_ids, &historical) {
            RootCopyState::InSync => in_sync_count += 1,
            RootCopyState::AmbiguousHeads => skipped.push(json!({
                "artifact_path": artifact_path,
                "reason": "multiple_plan_heads_disagree",
                "head_blob_ids": head_blob_ids,
                "command": plan_sync_repair_command(&artifact_path),
            })),
            RootCopyState::UnsyncedDrift => drift.push(json!({
                "artifact_path": artifact_path,
                "reason": "root_has_unsynced_drift",
                "command": plan_sync_repair_command(&artifact_path),
            })),
            RootCopyState::Unreadable(error) => skipped.push(json!({
                "artifact_path": artifact_path,
                "reason": "root_copy_unreadable",
                "error": error,
            })),
            state @ (RootCopyState::Missing | RootCopyState::MatchesRevision) => {
                let head_blob_id = &head_blob_ids[0];
                let body = blobs
                    .read_blob_bytes_for_id(&content_read, head_blob_id)
                    .map_err(|error| error.to_string())?;
                let Some(body) = body else {
                    skipped.push(json!({
                        "artifact_path": artifact_path,
                        "reason": "head_blob_missing",
                        "artifact_blob_id": head_blob_id,
                    }));
                    continue;
                };
                if let Some(parent) = absolute.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("Failed to create {}: {error}", parent.display())
                    })?;
                }
                fs::write(&absolute, &body)
                    .map_err(|error| format!("Failed to write {}: {error}", absolute.display()))?;
                materialized.push(json!({
                    "artifact_path": artifact_path,
                    "artifact_blob_id": head_blob_id,
                    "previous_state": if state == RootCopyState::Missing {
                        "missing"
                    } else {
                        "earlier_revision"
                    },
                }));
            }
        }
    }
    let status = if !drift.is_empty() || !skipped.is_empty() {
        "partial"
    } else if materialized.is_empty() {
        "in_sync"
    } else {
        "complete"
    };
    Ok(json!({
        "status": status,
        "in_sync_count": in_sync_count,
        "materialized": materialized,
        "drift": drift,
        "skipped": skipped,
    }))
}

/// Human text lines; empty when nothing was materialized and no drift exists.
pub(crate) fn plan_markdown_materialization_text_lines(payload: &JsonValue) -> Vec<String> {
    let Some(result) = payload
        .get(PLAN_MARKDOWN_MATERIALIZATION_PAYLOAD_KEY)
        .and_then(JsonValue::as_object)
    else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for row in result
        .get("materialized")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(path) = row.get("artifact_path").and_then(JsonValue::as_str) {
            lines.push(format!("plan markdown: materialized {path}"));
        }
    }
    for row in result
        .get("drift")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let path = row
            .get("artifact_path")
            .and_then(JsonValue::as_str)
            .unwrap_or("<path>");
        let command = row
            .get("command")
            .and_then(JsonValue::as_str)
            .unwrap_or_default();
        lines.push(format!(
            "plan markdown drift: {path} kept as-is; sync it with `{command}`"
        ));
    }
    for row in result
        .get("skipped")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
    {
        let path = row
            .get("artifact_path")
            .and_then(JsonValue::as_str)
            .unwrap_or("<path>");
        let reason = row
            .get("reason")
            .and_then(JsonValue::as_str)
            .unwrap_or("unknown");
        lines.push(format!("plan markdown skipped: {path} ({reason})"));
    }
    if let Some(error) = result.get("error").and_then(JsonValue::as_str) {
        lines.push(format!("plan markdown error: {error}"));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_copy_classification_prefers_safety() {
        let head = "BLB-head".to_string();
        let old = "BLB-old".to_string();
        let heads = [head.clone()];
        assert_eq!(
            classify_root_copy(None, &heads, std::slice::from_ref(&old)),
            RootCopyState::Missing
        );
        assert_eq!(
            classify_root_copy(Some(Ok(head.clone())), &heads, &[]),
            RootCopyState::InSync
        );
        assert_eq!(
            classify_root_copy(Some(Ok(old.clone())), &heads, &[head.clone(), old.clone()]),
            RootCopyState::MatchesRevision
        );
        assert_eq!(
            classify_root_copy(
                Some(Ok("BLB-edited".to_string())),
                &heads,
                std::slice::from_ref(&old)
            ),
            RootCopyState::UnsyncedDrift
        );
        assert_eq!(
            classify_root_copy(Some(Err("io".to_string())), &heads, &[]),
            RootCopyState::Unreadable("io".to_string())
        );
    }

    #[test]
    fn multi_plan_paths_are_in_sync_with_any_head_and_never_materialized_when_heads_disagree() {
        let newest = "BLB-newest".to_string();
        let stale = "BLB-stale".to_string();
        let heads = [stale.clone(), newest.clone()];
        assert_eq!(
            classify_root_copy(Some(Ok(newest.clone())), &heads, &[]),
            RootCopyState::InSync
        );
        assert_eq!(
            classify_root_copy(Some(Ok(stale.clone())), &heads, &[]),
            RootCopyState::InSync
        );
        assert_eq!(
            classify_root_copy(
                Some(Ok("BLB-other".to_string())),
                &heads,
                std::slice::from_ref(&stale)
            ),
            RootCopyState::AmbiguousHeads
        );
        assert_eq!(
            classify_root_copy(None, &heads, &[]),
            RootCopyState::AmbiguousHeads
        );
        let same = [newest.clone(), newest.clone()];
        assert_eq!(classify_root_copy(None, &same, &[]), RootCopyState::Missing);
    }

    #[test]
    fn text_lines_render_materialized_and_drift_rows() {
        let payload = json!({
            PLAN_MARKDOWN_MATERIALIZATION_PAYLOAD_KEY: {
                "status": "partial",
                "materialized": [{"artifact_path": "docs/a.md"}],
                "drift": [{"artifact_path": "docs/b.md", "command": "ait plan sync docs/b.md --local"}],
                "skipped": [],
            }
        });
        let lines = plan_markdown_materialization_text_lines(&payload);
        assert_eq!(lines[0], "plan markdown: materialized docs/a.md");
        assert_eq!(
            lines[1],
            "plan markdown drift: docs/b.md kept as-is; sync it with `ait plan sync docs/b.md --local`"
        );
        assert!(plan_markdown_materialization_text_lines(&json!({})).is_empty());
    }
}
