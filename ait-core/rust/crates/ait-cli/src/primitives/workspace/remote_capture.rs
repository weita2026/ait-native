use super::*;
use ait_core::repository_pack_json::{SnapshotCaptureBinding, SNAPSHOT_CAPTURE_BINDING_CONTRACT};

#[cfg(test)]
mod tests;

pub(super) struct CaptureContext {
    remote: ait_core::task_workflow_http_adapter::HttpTaskRemote,
    repo_name: String,
    repository_index: u32,
    capabilities: ait_core::remote_sync_backend::RemoteSyncCapabilities,
}

pub(super) fn context(repo: &RepoRuntime) -> Result<CaptureContext, String> {
    let name = snapshot_remote_name(repo)?;
    let (row, repo_name) = remote_context(repo, Some(&name), None)?;
    let current = current_worktree_metadata(repo)?.ok_or("Capture worktree is missing")?;
    let metadata = load_worktree_metadata(repo, &current.name)?;
    if metadata
        .get("capture_remote_url")
        .and_then(JsonValue::as_str)
        != Some(row.url.trim_end_matches('/'))
        || metadata
            .get("capture_repository_index")
            .and_then(JsonValue::as_u64)
            != Some(u64::from(repo.require_repository_index()?.get()))
    {
        return Err(
            "Remote capture authority differs from the Task worktree's pinned Repository authority"
                .to_string(),
        );
    }
    authority_context(repo, &row, &repo_name)
}
fn authority_context(
    repo: &RepoRuntime,
    row: &crate::runtime::RemoteRow,
    repo_name: &str,
) -> Result<CaptureContext, String> {
    let mut remote = http_task_remote(repo, row)?;
    let authority =
        crate::remote_repository::read_remote_repository_authority(repo, &mut remote, repo_name)?;
    let payload = authority.get("repository").unwrap_or(&authority);
    if payload["snapshot_capture_binding_contract"].as_str()
        != Some(SNAPSHOT_CAPTURE_BINDING_CONTRACT)
    {
        return Err("Remote authority does not support durable Snapshot capture binding; update the server before capture".to_string());
    }
    Ok(CaptureContext {
        remote,
        repo_name: repo_name.to_string(),
        repository_index: crate::remote_repository::remote_repository_index(&authority)?,
        capabilities: ait_core::remote_sync_backend::RemoteSyncCapabilities::from_server_payload(
            Some(&authority),
        ),
    })
}

pub(in crate::primitives) fn require_v2_authority(
    repo: &RepoRuntime,
    row: &crate::runtime::RemoteRow,
    repo_name: &str,
) -> Result<(), String> {
    authority_context(repo, row, repo_name).map(|_| ())
}

pub(super) fn require_merge_capability(repo: &RepoRuntime) -> Result<(), String> {
    if !context(repo)?.capabilities.snapshot_dag_v2 {
        return Err("Remote must advertise remote_sync.snapshot_dag.v2 before authoring a multi-parent capture".to_string());
    }
    Ok(())
}

pub(super) fn guard_target(
    repo: &RepoRuntime,
    row: &crate::runtime::RemoteRow,
) -> Result<(), String> {
    let Some(current) = current_worktree_metadata(repo)? else {
        return Ok(());
    };
    let metadata = load_worktree_metadata(repo, &current.name)?;
    let Some(url) = metadata
        .get("capture_remote_url")
        .and_then(JsonValue::as_str)
    else {
        return Ok(());
    };
    if url != row.url.trim_end_matches('/')
        || metadata
            .get("capture_repository_index")
            .and_then(JsonValue::as_u64)
            != Some(u64::from(repo.require_repository_index()?.get()))
    {
        return Err(
            "Publication target differs from the Task worktree's pinned capture authority"
                .to_string(),
        );
    }
    Ok(())
}

pub(super) fn remember_head(
    repo: &RepoRuntime,
    snapshot: &JsonValue,
    expected: Option<&str>,
    owner: &ait_core::workflow_binary_db::SnapshotWorkflowBinding,
) -> Result<(), String> {
    let binding = SnapshotCaptureBinding {
        repository_index: repo.require_repository_index()?.get(),
        snapshot_id: required_string_field(snapshot, "snapshot_id")?,
        owner: owner.clone(),
    };
    let current = current_worktree_metadata(repo)?.ok_or("Capture worktree is missing")?;
    let metadata = load_worktree_metadata(repo, &current.name)?;
    let pending = metadata.get("capture_pending").unwrap_or(&JsonValue::Null);
    if let Some(id) = pending["snapshot_id"].as_str() {
        if Some(id) != snapshot["snapshot_id"].as_str() {
            // A prior acknowledged capture must be resumed before another
            // capture can replace its local recovery expectation.
            let mut ctx = context(repo)?;
            let existence = ctx
                .remote
                .get_remote_snapshots_existence(&ctx.repo_name, &[id.to_string()])
                .map_err(|e| e.to_string())?;
            if json_string_list(existence.get("present"))
                .iter()
                .any(|value| value == id)
                && read_binding(repo, &mut ctx, id)?.is_some()
            {
                return Err(format!(
                    "Resume pending remote capture {id} before creating another Snapshot"
                ));
            }
        } else if pending["expected_head"].as_str() != expected
            || !binding.matches_recovery_value(&pending["binding"])
        {
            return Err("Capture recovery head expectation cannot change".to_string());
        }
    }
    let mut metadata = load_worktree_metadata(repo, &current.name)?;
    metadata.insert(
        "capture_pending".to_string(),
        json!({"snapshot_id":snapshot["snapshot_id"], "expected_head":expected, "binding":binding.to_value()}),
    );
    save_worktree_metadata(repo, &current.name, &metadata)
}

pub(super) fn expected_head(
    repo: &RepoRuntime,
    snapshot: &JsonValue,
) -> Result<Option<String>, String> {
    let current = current_worktree_metadata(repo)?.ok_or("Capture worktree is missing")?;
    let metadata = load_worktree_metadata(repo, &current.name)?;
    let pending = metadata.get("capture_pending").unwrap_or(&JsonValue::Null);
    if pending["snapshot_id"] == snapshot["snapshot_id"] {
        Ok(pending["expected_head"].as_str().map(str::to_string))
    } else {
        Ok(snapshot["parent_snapshot_id"].as_str().map(str::to_string))
    }
}

pub(super) fn clear_pending(repo: &RepoRuntime) -> Result<(), String> {
    let current = current_worktree_metadata(repo)?.ok_or("Capture worktree is missing")?;
    let mut metadata = load_worktree_metadata(repo, &current.name)?;
    metadata.remove("capture_pending");
    save_worktree_metadata(repo, &current.name, &metadata)
}

fn read_binding(
    repo: &RepoRuntime,
    ctx: &mut CaptureContext,
    id: &str,
) -> Result<Option<SnapshotCaptureBinding>, String> {
    let snapshot = ctx
        .remote
        .get_remote_snapshot(&ctx.repo_name, id, false, None)
        .map_err(|e| e.to_string())?;
    binding_from_readback(repo, ctx, id, &snapshot)
}

pub(super) fn capture_missing_remotely(ctx: &mut CaptureContext, id: &str) -> Result<bool, String> {
    match ctx
        .remote
        .get_remote_snapshot(&ctx.repo_name, id, false, None)
    {
        Ok(_) => Ok(false),
        Err(error) if error.remote_status() == Some(404) => Ok(true),
        Err(error) => Err(error.to_string()),
    }
}

fn binding_from_readback(
    repo: &RepoRuntime,
    ctx: &CaptureContext,
    id: &str,
    snapshot: &JsonValue,
) -> Result<Option<SnapshotCaptureBinding>, String> {
    if snapshot["snapshot_id"].as_str() != Some(id) {
        return Err("Remote capture readback returned a different Snapshot".to_string());
    }
    let Some(value) = snapshot.get("capture_binding").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let binding = SnapshotCaptureBinding::from_value(value)?;
    let local = snapshot_store(repo)?
        .snapshot_by_id(id)?
        .ok_or("Capture Snapshot is missing locally")?;
    if binding.repository_index != ctx.repository_index
        || binding.snapshot_id != id
        || binding.owner.line_name != local.line_name
        || snapshot["line_name"].as_str() != Some(local.line_name.as_str())
    {
        return Err(
            "Persisted capture Repository/Snapshot/Line disagrees with local immutable content"
                .to_string(),
        );
    }
    Ok(Some(binding))
}

pub(super) fn commit(
    repo: &RepoRuntime,
    ctx: &mut CaptureContext,
    snapshot: &JsonValue,
    owner: &ait_core::workflow_binary_db::SnapshotWorkflowBinding,
) -> Result<JsonValue, String> {
    let id = snapshot["snapshot_id"]
        .as_str()
        .ok_or("Capture Snapshot ID is missing")?;
    let expected = SnapshotCaptureBinding {
        repository_index: ctx.repository_index,
        snapshot_id: id.to_string(),
        owner: owner.clone(),
    };
    if snapshot["newly_created"].as_bool() != Some(true) {
        // Successful replay preserves the Server's immutable author. A pending
        // Local cursor stores no Actor and cannot prove this uploader authored
        // an earlier, unacknowledged capture.
        if read_binding(repo, ctx, id)?.as_ref() == Some(&expected) {
            return Ok(expected.to_value());
        }
        return Err(format!("Cannot reuse Snapshot {id} as a fresh capture without its exact server-acknowledged capture binding; existing local capture state does not establish authorship."));
    }
    let uploaded = super::super::remote_sync::upload_snapshot_capture(
        repo,
        &mut ctx.remote,
        &ctx.repo_name,
        &expected,
        &ctx.capabilities,
    );
    // A transport failure can happen after durable commit. Exact persisted
    // readback, never the request itself, decides whether capture succeeded.
    let actual = read_binding(repo, ctx, id).map_err(|error| {
        format!(
            "SNAPSHOT_CAPTURE_INCOMPLETE: {id}; local head unchanged; {error}; upload={uploaded:?}"
        )
    })?;
    if actual.as_ref() != Some(&expected) {
        return Err(format!("SNAPSHOT_CAPTURE_INCOMPLETE: {id} has no exact persisted Task/Change capture binding; local head unchanged; upload={uploaded:?}"));
    }
    Ok(expected.to_value())
}

pub(super) fn verify_existing(
    repo: &RepoRuntime,
    ctx: &mut CaptureContext,
    id: &str,
    owner: &ait_core::workflow_binary_db::SnapshotWorkflowBinding,
) -> Result<(), String> {
    let expected = SnapshotCaptureBinding {
        repository_index: ctx.repository_index,
        snapshot_id: id.to_string(),
        owner: owner.clone(),
    };
    if read_binding(repo, ctx, id)?.as_ref() != Some(&expected) {
        return Err("Resume requires an exact existing durable capture binding; unbound history cannot be repaired from current metadata".to_string());
    }
    Ok(())
}

pub(super) fn ownership_rows(repo: &RepoRuntime, ids: &[String]) -> Result<Vec<JsonValue>, String> {
    let remote_name = snapshot_remote_name(repo)?;
    ownership_rows_for_remote(repo, ids, Some(&remote_name))
}
pub(in crate::primitives) fn ownership_rows_for_remote(
    repo: &RepoRuntime,
    ids: &[String],
    remote_name: Option<&str>,
) -> Result<Vec<JsonValue>, String> {
    let (row, name) = remote_context(repo, remote_name, None)?;
    guard_target(repo, &row)?;
    let mut ctx = authority_context(repo, &row, &name)?;
    ownership_rows_with_context(repo, ids, &mut ctx)
}
fn ownership_rows_with_context(
    repo: &RepoRuntime,
    ids: &[String],
    ctx: &mut CaptureContext,
) -> Result<Vec<JsonValue>, String> {
    let mut rows = Vec::new();
    for id in ids {
        let snapshot = match ctx
            .remote
            .get_remote_snapshot(&ctx.repo_name, id, false, None)
        {
            Ok(snapshot) => snapshot,
            // A Local base can precede Remote publication. Absence supplies no
            // ownership; capture acknowledgment still uses strict read_binding.
            Err(error) if error.remote_status() == Some(404) => continue,
            Err(error) => return Err(error.to_string()),
        };
        if let Some(binding) = binding_from_readback(repo, ctx, id, &snapshot)? {
            rows.push(json!({
                "snapshot_id": id, "task_id": binding.owner.task_id,
                "change_id": normalized_text(Some(&binding.owner.change_id)),
                "line_name": binding.owner.line_name, "repository_index": binding.repository_index,
                "ownership_source": "durable_remote_snapshot_binding",
            }));
        }
    }
    Ok(rows)
}
