use super::*;
use crate::external_readiness_gate::external_readiness_report_for_repo;

mod apply_action;
mod apply_support;
mod land_state;
mod local_completion;
mod ready_apply;
mod task_land;
mod task_line_closeout;
mod wait_hint;

pub(super) use apply_action::{workflow_land_apply_action, workflow_ready_apply_action};

pub(super) use apply_support::{
    workflow_apply_phase_payload_json, workflow_current_ids, workflow_json_text,
    workflow_nested_text, workflow_progress_emit, workflow_root_text,
    workflow_wait_for_pending_state,
};

#[cfg(test)]
pub(super) use apply_action::{
    workflow_land_evaluate_policy_with_closeout_remote,
    workflow_land_record_attestation_with_closeout_remote,
    workflow_land_record_code_review_summary_with_closeout_remote,
    workflow_land_record_task_review_with_closeout_remote,
    workflow_land_submit_action_with_task_and_closeout_remotes,
    workflow_publish_patchset_action_with_task_and_closeout_remotes,
    workflow_ready_record_attestation_with_closeout_remote,
    workflow_ready_run_patchset_ci_with_closeout_remote,
    workflow_record_attestation_action_with_closeout_remote,
    workflow_record_review_action_with_closeout_remote,
    workflow_run_patchset_ci_action_with_closeout_remote,
};

pub(super) use land_state::{
    workflow_hydrate_land_state, workflow_land_patchset_read_with_closeout_remote,
};

#[cfg(test)]
pub(super) use land_state::{
    workflow_land_attestation_read_with_closeout_remote,
    workflow_land_base_line_read_with_task_remote,
    workflow_land_change_detail_read_with_task_remote,
    workflow_land_change_task_read_with_task_remote,
    workflow_land_patchset_ci_status_read_with_closeout_remote,
    workflow_land_policy_read_with_closeout_remote, workflow_land_remote_state_with_remotes,
    workflow_land_review_summary_read_with_closeout_remote,
    workflow_land_workspace_context_with_status_reader,
};

pub(super) use wait_hint::{
    workflow_maybe_record_ready_wait_hint_sample, workflow_wait_seconds_hint,
};

#[cfg(test)]
pub(super) use wait_hint::{
    workflow_bootstrap_wait_hint_seconds_from_history_with_task_remote,
    workflow_coerce_wait_hint_seconds, workflow_resolve_wait_hint_seconds,
    workflow_wait_hint_change_detail_with_task_remote,
    workflow_wait_hint_change_rows_with_task_remote,
};

use local_completion::workflow_effective_pre_land_target_snapshot_id;

pub(super) use local_completion::{
    workflow_final_snapshot_promotion_candidate, workflow_final_snapshot_promotion_preview,
    workflow_final_snapshot_promotion_remote_change_id, workflow_prepare_final_snapshot_promotion,
};

pub use ready_apply::workflow_ready_apply;

#[cfg(test)]
pub(super) use ready_apply::{
    workflow_ready_ci_pending_wait_state, workflow_ready_ci_poll_wait_state,
};

#[cfg(test)]
pub(super) use local_completion::{
    workflow_final_snapshot_candidate_from_entry,
    workflow_initialize_null_remote_base_with_task_remote, workflow_local_history_entries,
    workflow_mark_history_published, workflow_same_head_remote_land_authority,
    workflow_unique_history_plan_artifact_paths, workflow_unique_history_plan_publications,
};

pub use task_land::{
    task_land_apply, task_land_apply_scoped, task_land_payload, task_land_payload_scoped,
};

#[cfg(test)]
pub(super) use task_land::{
    task_land_atomic_action_result, task_land_atomic_output, task_land_attach_cli_main_seed_sync,
    task_land_attach_plan_checklist_closeout, task_land_defer_bound_cleanup,
    task_land_exact_atomic_reference, task_land_local_change_id_with_change_store,
    task_land_remote_change_id_with_task_remote, task_land_remote_change_read_with_task_remote,
    task_land_remote_change_rows_with_task_remote, task_land_remote_task_read_with_task_remote,
};

pub(super) use task_line_closeout::{
    task_land_attach_bound_line_closeout, task_land_capture_bound_line,
};

#[cfg(test)]
pub(super) use task_line_closeout::{
    task_land_archive_local_bound_line, task_land_archive_remote_bound_line_with_task_remote,
    task_land_selected_patchset_revision_with_closeout_remote,
};

mod bound_worktree;
mod command_hints;
mod local_state;
mod review_state;

pub(super) use bound_worktree::*;
pub(super) use command_hints::*;
pub(super) use local_state::*;
pub(super) use review_state::*;

#[cfg(test)]
mod tests;
pub(super) fn workflow_current_worktree_retarget(
    repo: &RepoRuntime,
    root_repo: &RepoRuntime,
    current_line_name: &str,
    head_snapshot_id: Option<&str>,
    authoritative_target_base_snapshot_id: Option<&str>,
) -> Result<Option<JsonValue>, String> {
    if !repo.is_worktree() {
        return Ok(None);
    }
    let Some(worktree_name) = repo
        .config
        .get("worktree_name")
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))
    else {
        return Ok(None);
    };
    let worktree_name = normalize_worktree_name(&worktree_name)?;
    let metadata = load_worktree_metadata(root_repo, &worktree_name)?;
    let retarget = worktree_retarget_summary_with_authority(
        root_repo,
        &metadata,
        Some(current_line_name),
        head_snapshot_id,
        None,
        authoritative_target_base_snapshot_id,
    )?;
    Ok(Some(JsonValue::Object(retarget)))
}

pub fn workflow_ready_payload(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if let Some(candidate) =
        workflow_final_snapshot_promotion_candidate(repo, change_id, remote_name)?
    {
        let local_change = candidate
            .get("state")
            .and_then(|state| state.get("change"))
            .cloned()
            .unwrap_or(JsonValue::Null);
        if string_field(&local_change, "publication_state").as_deref() == Some("published") {
            let remote_change_id = workflow_final_snapshot_promotion_remote_change_id(&candidate)?;
            let local_change_ref = workflow_completed_local_command_change_ref(&candidate)?;
            if let Ok(remote_state) =
                workflow_ready_remote_payload_with_patchset_authority_and_command_ref(
                    repo,
                    &remote_change_id,
                    remote_name,
                    true,
                    Some(&local_change_ref),
                )
            {
                let has_patchset =
                    workflow_nested_text(&remote_state, "patchset", "patchset_id").is_some();
                let landed = workflow_nested_text(&remote_state, "change", "status").as_deref()
                    == Some("landed");
                if has_patchset || landed {
                    return Ok(remote_state);
                }
            }
        }
        return workflow_final_snapshot_promotion_preview(&candidate);
    }
    workflow_ready_remote_payload(repo, change_id, remote_name)
}

pub(super) fn workflow_ready_remote_payload(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    workflow_ready_remote_payload_with_patchset_authority(repo, change_id, remote_name, false)
}

pub(super) fn workflow_ready_remote_payload_with_patchset_authority(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    ready_patchset_is_authoritative: bool,
) -> Result<JsonValue, String> {
    workflow_ready_remote_payload_with_patchset_authority_and_command_ref(
        repo,
        change_id,
        remote_name,
        ready_patchset_is_authoritative,
        None,
    )
}

fn workflow_ready_remote_payload_with_patchset_authority_and_command_ref(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    ready_patchset_is_authoritative: bool,
    command_change_ref: Option<&str>,
) -> Result<JsonValue, String> {
    let _payload_range = perfetto_range!("ait.workflow_ready.payload");
    let mut full_state = {
        let _range = perfetto_range!("ait.workflow_ready.payload.land_state");
        if ready_patchset_is_authoritative {
            workflow_projected_ready_task_land_state(repo, change_id, remote_name)?
        } else {
            workflow_projected_ready_state(repo, change_id, remote_name)?
        }
    };
    {
        let _range = perfetto_range!("ait.workflow_ready.payload.external_readiness");
        workflow_insert_external_readiness(repo, &mut full_state)?;
    }
    workflow_project_ready_payload(
        repo,
        &full_state,
        change_id,
        remote_name,
        ready_patchset_is_authoritative,
        command_change_ref,
    )
}

fn workflow_project_ready_payload(
    repo: &RepoRuntime,
    full_state: &JsonValue,
    change_id: &str,
    remote_name: Option<&str>,
    ready_patchset_is_authoritative: bool,
    command_change_ref: Option<&str>,
) -> Result<JsonValue, String> {
    let change = full_state.get("change").cloned().unwrap_or(JsonValue::Null);
    let resolved_change_ref = change_reference_from_payload(&change, Some(change_id))
        .unwrap_or_else(|_| change_id.into());
    let command_change_ref =
        normalized_text(command_change_ref).unwrap_or_else(|| resolved_change_ref.clone());
    let base_line = full_state
        .get("base_line")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let base_line_name = string_field(&base_line, "line_name")
        .or_else(|| string_field(&change, "base_line"))
        .unwrap_or_else(|| "main".to_string());
    let facts = {
        let _range = perfetto_range!("ait.workflow_ready.payload.facts");
        workflow_ready_facts(full_state)?
    };
    let command_hints = workflow_ready_command_hints(
        repo,
        command_change_ref.as_str(),
        remote_name,
        full_state.get("patchset"),
        base_line_name.as_str(),
        full_state.get("worktree_retarget"),
    );
    let _range = perfetto_range!("ait.workflow_ready.payload.project");
    project_workflow_ready_read_model(
        &facts,
        &command_hints,
        ready_patchset_is_authoritative,
        ready_patchset_is_authoritative,
        false,
    )
}

fn workflow_ready_patchset_authority_from_state(state: &JsonValue) -> Result<bool, String> {
    let ignore_workspace_authoring = state
        .get("ignore_workspace_authoring")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    let patchset_is_authoritative = state
        .get("patchset_is_authoritative")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if ignore_workspace_authoring != patchset_is_authoritative {
        return Err(
            "Workflow ready state is inconsistent: workspace authoring and Patchset selection disagree."
                .to_string(),
        );
    }
    Ok(patchset_is_authoritative)
}

pub(in crate::primitives) fn workflow_ready_ci_poll_payload_with_closeout_remote<R>(
    repo: &RepoRuntime,
    closeout_remote: &mut R,
    repo_name: &str,
    state: &JsonValue,
    change_id: &str,
    remote_name: Option<&str>,
    command_change_ref: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowPatchsetCiStatusReader + ?Sized,
{
    let _poll_range = perfetto_range!("ait.workflow_ready.ci.poll");
    let patchset_id = workflow_nested_text(state, "patchset", "patchset_id")
        .ok_or_else(|| "Waiting-for-CI state is missing patchset.patchset_id.".to_string())?;
    let patchset_ci_status = {
        let _range = perfetto_range!("ait.workflow_ready.ci.poll_readiness_http");
        change_flow::patchset_ci_readiness_with_closeout_remote(
            closeout_remote,
            &patchset_id,
            repo_name,
            10,
        )?
    };
    let mut refreshed = state
        .as_object()
        .cloned()
        .ok_or_else(|| "Workflow ready CI poll state must be an object.".to_string())?;
    refreshed.insert("patchset_ci_status".to_string(), patchset_ci_status);
    let refreshed = JsonValue::Object(refreshed);
    let ready_patchset_is_authoritative = workflow_ready_patchset_authority_from_state(&refreshed)?;
    workflow_project_ready_payload(
        repo,
        &refreshed,
        change_id,
        remote_name,
        ready_patchset_is_authoritative,
        command_change_ref,
    )
}

fn workflow_completed_local_finish_authority(
    candidate: Option<&JsonValue>,
) -> Result<(Option<String>, bool), String> {
    let Some(candidate) = candidate else {
        return Ok((None, false));
    };
    let publication_state = candidate
        .get("state")
        .and_then(|state| state.get("change"))
        .and_then(|change| string_field(change, "publication_state"));
    if publication_state.as_deref() != Some("published") {
        return Ok((None, false));
    }
    Ok((
        Some(workflow_final_snapshot_promotion_remote_change_id(
            candidate,
        )?),
        true,
    ))
}

fn workflow_completed_local_command_change_ref(candidate: &JsonValue) -> Result<String, String> {
    let change = candidate
        .get("state")
        .and_then(|state| state.get("change"))
        .ok_or_else(|| {
            "Completed-local workflow candidate is missing local Change state.".to_string()
        })?;
    let local_change_id = required_string_field(change, "change_id")?;
    change_reference_from_payload(change, Some(&local_change_id))
}

pub fn workflow_land_payload(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let promotion_candidate =
        workflow_final_snapshot_promotion_candidate(repo, change_id, remote_name)?;
    if let Some(candidate) = promotion_candidate.as_ref() {
        let (remote_change_id, patchset_is_authoritative) =
            workflow_completed_local_finish_authority(Some(candidate))?;
        let Some(remote_change_id) = remote_change_id else {
            return workflow_final_snapshot_promotion_preview(candidate);
        };
        let local_change_ref = workflow_completed_local_command_change_ref(candidate)?;
        return workflow_land_payload_with_workspace_mode(
            repo,
            &remote_change_id,
            remote_name,
            patchset_is_authoritative,
            Some(&local_change_ref),
        );
    }
    workflow_land_payload_with_workspace_mode(repo, change_id, remote_name, false, None)
}

fn workflow_ready_task_land_payload_with_command_ref(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    command_change_ref: Option<&str>,
) -> Result<JsonValue, String> {
    workflow_land_payload_with_workspace_mode(
        repo,
        change_id,
        remote_name,
        true,
        command_change_ref,
    )
}

fn workflow_land_payload_with_workspace_mode(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    ready_patchset_is_authoritative: bool,
    command_change_ref: Option<&str>,
) -> Result<JsonValue, String> {
    let mut full_state = if ready_patchset_is_authoritative {
        workflow_projected_ready_task_land_state(repo, change_id, remote_name)?
    } else {
        workflow_projected_land_state(repo, change_id, remote_name)?
    };
    workflow_insert_external_readiness(repo, &mut full_state)?;
    let change = full_state.get("change").cloned().unwrap_or(JsonValue::Null);
    if string_field(&change, "status").as_deref() == Some("landed") {
        return Ok(full_state);
    }
    let resolved_change_ref = change_reference_from_payload(&change, Some(change_id))
        .unwrap_or_else(|_| change_id.to_string());
    let command_change_ref =
        normalized_text(command_change_ref).unwrap_or_else(|| resolved_change_ref.clone());
    let base_line = full_state
        .get("base_line")
        .cloned()
        .unwrap_or(JsonValue::Null);
    let base_line_name = string_field(&base_line, "line_name")
        .or_else(|| string_field(&change, "base_line"))
        .unwrap_or_else(|| "main".to_string());
    let ready_state = project_workflow_ready_read_model(
        &workflow_ready_facts(&full_state)?,
        &workflow_ready_command_hints(
            repo,
            command_change_ref.as_str(),
            remote_name,
            full_state.get("patchset"),
            base_line_name.as_str(),
            full_state.get("worktree_retarget"),
        ),
        ready_patchset_is_authoritative,
        ready_patchset_is_authoritative,
        false,
    )?;
    let phase_facts = workflow_land_phase_facts(&full_state, &ready_state)?;
    let phase_payload = project_workflow_land_phase_read_model(&phase_facts)?;
    let mut payload = phase_payload
        .as_object()
        .cloned()
        .ok_or_else(|| "workflow finish payload must decode to an object".to_string())?;
    if let Some(full_object) = full_state.as_object() {
        for key in [
            "change",
            "task",
            "patchset",
            "patchset_source",
            "workspace",
            "base_line",
            "review",
            "attestation",
            "policy",
            "task_review",
            "landing_summary",
            "worktree_retarget",
            "freshness",
            "patchset_refresh",
        ] {
            if let Some(value) = full_object.get(key) {
                payload.insert(key.to_string(), value.clone());
            }
        }
    }
    Ok(JsonValue::Object(payload))
}

fn workflow_insert_external_readiness(
    repo: &RepoRuntime,
    state: &mut JsonValue,
) -> Result<(), String> {
    let Some(report) = external_readiness_report_for_repo(repo)? else {
        return Ok(());
    };
    if let Some(object) = state.as_object_mut() {
        object.insert("external_readiness".to_string(), report.to_json_value());
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn workflow_local_land_landed_snapshot_id_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    target_line: &str,
    result: &JsonValue,
    land_result: &JsonValue,
    base_stale_converged_snapshot_id: Option<String>,
) -> Option<String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.local.resolve_landed_snapshot");
    string_field(result, "landed_snapshot_id")
        .or(base_stale_converged_snapshot_id)
        .or_else(|| string_field(land_result, "landed_snapshot_id"))
        .or_else(|| {
            task_remote
                .get_line(repo_name, target_line)
                .ok()
                .and_then(|line| string_field(&line, "head_snapshot_id"))
        })
}

#[cfg(test)]
pub(super) fn workflow_local_land_task_id_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    change_id: &str,
) -> Result<String, String>
where
    R: TaskWorkflowRemoteChangeReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.post_land_change_read");
    let change = task_remote
        .get_change(change_id, Some(repo_name))
        .map_err(|err| err.to_string())?;
    required_string_field(&change, "task_id")
}

#[cfg(test)]
pub(super) fn workflow_attach_local_land_sync_with_task_remote<R>(
    repo: &RepoRuntime,
    task_remote: &mut R,
    repo_name: &str,
    change_id: &str,
    land_result: &JsonValue,
    patchset_revision_snapshot_id: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowRemoteChangeReader + TaskWorkflowLineReader + ?Sized,
{
    let _sync_range = perfetto_range!("ait.task_land.local.sync");
    let mut payload = land_result.clone();
    let status = string_field(land_result, "status")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let result = land_result
        .get("result")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let target_line = string_field(&result, "target_line").unwrap_or_else(|| "main".to_string());
    let base_stale_converged_snapshot_id = if status == "blocked" {
        workflow_base_stale_converged_snapshot_id(land_result, patchset_revision_snapshot_id)
    } else {
        None
    };
    if !matches!(status.as_str(), "succeeded" | "landed")
        && base_stale_converged_snapshot_id.is_none()
    {
        return Ok(payload);
    }
    let landed_snapshot_id = workflow_local_land_landed_snapshot_id_with_task_remote(
        task_remote,
        repo_name,
        &target_line,
        &result,
        land_result,
        base_stale_converged_snapshot_id.clone(),
    );
    let root_repo = workflow_root_repo(repo)?;
    let previous_head_snapshot_id = {
        let _range = perfetto_range!("ait.task_land.local.target_line_read");
        local_line_head_snapshot_id(&root_repo, &target_line)?
    };
    if let Some(landed_snapshot_id) = landed_snapshot_id.as_deref() {
        let _range = perfetto_range!("ait.task_land.local.target_line_update");
        set_local_line_head(&root_repo, &target_line, Some(landed_snapshot_id))?;
    }
    let workspace_restore = {
        let _range = perfetto_range!("ait.task_land.local.workspace_restore");
        workflow_repo_root_restore_after_land(
            repo,
            &target_line,
            previous_head_snapshot_id.as_deref(),
            landed_snapshot_id.as_deref(),
        )?
    };
    let mut local_sync = json!({
        "status": if string_field(&workspace_restore, "status").as_deref() == Some("failed") {
            "failed"
        } else {
            "synced"
        },
        "line": target_line,
        "landed_snapshot_id": landed_snapshot_id,
        "auto_rebase": true,
        "workspace_restore": workspace_restore,
    });
    if base_stale_converged_snapshot_id.is_some() {
        local_sync["status_source"] =
            JsonValue::String("base_stale_target_line_already_at_revision".to_string());
        if let Some(payload_obj) = payload.as_object_mut() {
            payload_obj.insert("base_stale_converged".to_string(), JsonValue::Bool(true));
            if let Some(result_obj) = payload_obj
                .get_mut("result")
                .and_then(JsonValue::as_object_mut)
            {
                if let Some(landed_snapshot_id) = landed_snapshot_id.as_deref() {
                    result_obj.insert(
                        "landed_snapshot_id".to_string(),
                        JsonValue::String(landed_snapshot_id.to_string()),
                    );
                }
                result_obj.insert("base_stale_converged".to_string(), JsonValue::Bool(true));
            }
        }
    }
    let task_id = workflow_local_land_task_id_with_task_remote(task_remote, repo_name, change_id)?;
    let cleanup = json!({
        "status": "deferred",
        "reason": "task_land_main_seed_finalizer",
        "task_id": task_id,
        "detail": "The Task worktree remains available for validated CLI main-seed promotion until final Task status is known.",
    });
    payload["local_sync"] = local_sync.take();
    payload["bound_worktree_cleanup"] = cleanup;
    Ok(payload)
}

pub(super) fn workflow_attach_local_land_sync_from_atomic_response(
    repo: &RepoRuntime,
    task_id: &str,
    land_result: &JsonValue,
    target_line: &str,
    landed_snapshot_id: &str,
) -> Result<JsonValue, String> {
    let _sync_range = perfetto_range!("ait.task_land.local.atomic_response_sync");
    let root_repo = workflow_root_repo(repo)?;
    let previous_head_snapshot_id = {
        let _range = perfetto_range!("ait.task_land.local.target_line_read");
        local_line_head_snapshot_id(&root_repo, target_line)?
    };
    let same_head = previous_head_snapshot_id.as_deref() == Some(landed_snapshot_id);
    let local_head_contains_landed_snapshot = if same_head {
        false
    } else {
        snapshot_distance_if_ancestor(
            &root_repo,
            Some(landed_snapshot_id),
            previous_head_snapshot_id.as_deref(),
        )?
        .is_some()
    };
    let preserve_local_head = same_head || local_head_contains_landed_snapshot;
    if !preserve_local_head {
        let _range = perfetto_range!("ait.task_land.local.target_line_update");
        set_local_line_head(&root_repo, target_line, Some(landed_snapshot_id))?;
    }
    let workspace_restore = if same_head {
        json!({
            "status": "skipped",
            "reason": "already_at_trusted_local_landed_snapshot",
            "line": target_line,
            "snapshot_id": landed_snapshot_id,
        })
    } else if local_head_contains_landed_snapshot {
        json!({
            "status": "skipped",
            "reason": "local_head_already_contains_landed_snapshot",
            "line": target_line,
            "snapshot_id": previous_head_snapshot_id,
            "landed_snapshot_id": landed_snapshot_id,
        })
    } else {
        let _range = perfetto_range!("ait.task_land.local.workspace_restore");
        workflow_repo_root_restore_after_land(
            repo,
            target_line,
            previous_head_snapshot_id.as_deref(),
            Some(landed_snapshot_id),
        )?
    };
    let local_sync = json!({
        "status": if same_head {
            "already_synced"
        } else if local_head_contains_landed_snapshot {
            "local_descendant_preserved"
        } else if string_field(&workspace_restore, "status").as_deref() == Some("failed") {
            "failed"
        } else {
            "synced"
        },
        "line": target_line,
        "landed_snapshot_id": landed_snapshot_id,
        "line_head_snapshot_id": if preserve_local_head {
            previous_head_snapshot_id.clone()
        } else {
            Some(landed_snapshot_id.to_string())
        },
        "auto_rebase": !preserve_local_head,
        "same_head": same_head,
        "local_head_contains_landed_snapshot": local_head_contains_landed_snapshot,
        "source": "task-land-atomic/v1",
        "workspace_restore": workspace_restore,
    });
    let mut payload = land_result.clone();
    let mut result = payload
        .get("result")
        .and_then(JsonValue::as_object)
        .cloned()
        .unwrap_or_default();
    result.insert(
        "target_line".to_string(),
        JsonValue::String(target_line.to_string()),
    );
    result.insert(
        "landed_snapshot_id".to_string(),
        JsonValue::String(landed_snapshot_id.to_string()),
    );
    payload["result"] = JsonValue::Object(result);
    payload["local_sync"] = local_sync;
    payload["bound_worktree_cleanup"] = json!({
        "status": "deferred",
        "reason": "task_land_main_seed_finalizer",
        "task_id": task_id,
        "detail": "The Task worktree remains available until CLI main-seed promotion succeeds.",
    });
    Ok(payload)
}

pub fn workflow_land_apply<F>(
    repo: &RepoRuntime,
    change_id: &str,
    review_message: Option<&str>,
    remote_name: Option<&str>,
    progress: Option<F>,
) -> Result<JsonValue, String>
where
    F: FnMut(&JsonValue) -> Result<(), String>,
{
    let promotion_candidate =
        workflow_final_snapshot_promotion_candidate(repo, change_id, remote_name)?;
    let (resolved_change_id, ready_patchset_is_authoritative, command_change_ref) = if let Some(
        candidate,
    ) =
        promotion_candidate.as_ref()
    {
        let (remote_change_id, patchset_is_authoritative) =
            workflow_completed_local_finish_authority(Some(candidate))?;
        let Some(remote_change_id) = remote_change_id else {
            let remote_name = normalized_text(remote_name).unwrap_or_else(|| "origin".to_string());
            return Err(format!(
                "Completed local change {change_id} must pass the explicit ready phase before reviewer finish. Run `ait workflow ready {change_id} --apply --remote {remote_name}`, then `ait workflow finish {change_id} --apply --remote {remote_name}`."
            ));
        };
        (
            remote_change_id,
            patchset_is_authoritative,
            Some(workflow_completed_local_command_change_ref(candidate)?),
        )
    } else {
        (change_id.to_string(), false, None)
    };
    workflow_land_apply_with_state_mode(
        repo,
        &resolved_change_id,
        review_message,
        remote_name,
        WorkflowLandApplyStateMode {
            ready_patchset_is_authoritative,
            command_change_ref: command_change_ref.as_deref(),
            initial_state: None,
        },
        progress,
    )
}

struct WorkflowLandApplyStateMode<'a> {
    ready_patchset_is_authoritative: bool,
    command_change_ref: Option<&'a str>,
    initial_state: Option<JsonValue>,
}

fn workflow_land_apply_with_state_mode<F>(
    repo: &RepoRuntime,
    change_id: &str,
    review_message: Option<&str>,
    remote_name: Option<&str>,
    mut state_mode: WorkflowLandApplyStateMode<'_>,
    mut progress: Option<F>,
) -> Result<JsonValue, String>
where
    F: FnMut(&JsonValue) -> Result<(), String>,
{
    let mut applied_actions = Vec::new();
    let mut mutation_receipts = Vec::new();
    let mut seen_signatures = BTreeSet::new();
    workflow_progress_emit(
        &mut progress,
        "probing",
        "authoritative_state",
        Some(change_id),
        None,
        None,
        Some("Reading current workflow state before applying requested changes."),
        Some("authoritative_read"),
        None,
        None,
        None,
    )?;
    loop {
        let state = if let Some(state) = state_mode.initial_state.take() {
            state
        } else if state_mode.ready_patchset_is_authoritative {
            workflow_ready_task_land_payload_with_command_ref(
                repo,
                change_id,
                remote_name,
                state_mode.command_change_ref,
            )?
        } else {
            workflow_land_payload(repo, change_id, remote_name)?
        };
        let mut code = workflow_nested_text(&state, "next_action", "code").unwrap_or_default();
        if (code.is_empty() || code == "done" || code == "complete_task")
            && workflow_done_state_needs_converged_land_submit(&state, &applied_actions)
        {
            code = "submit_land".to_string();
        }
        let (current_change_id, current_patchset_id) = workflow_current_ids(&state);
        if code.is_empty() || code == "done" {
            let detail = if applied_actions.is_empty() {
                "Current state already satisfies `task finish`; no change was needed."
            } else {
                "Workflow finish apply completed."
            };
            if applied_actions.is_empty() {
                workflow_progress_emit(
                    &mut progress,
                    "resumed",
                    "done",
                    current_change_id.as_deref(),
                    current_patchset_id.as_deref(),
                    None,
                    Some(detail),
                    Some("authoritative_resume"),
                    None,
                    None,
                    None,
                )?;
            }
            let mut output = state.as_object().cloned().unwrap_or_default();
            output.insert(
                "applied_actions".to_string(),
                JsonValue::Array(applied_actions),
            );
            output.insert(
                "mutation_receipts".to_string(),
                JsonValue::Array(mutation_receipts),
            );
            output.insert(
                "apply_status".to_string(),
                JsonValue::String("done".to_string()),
            );
            output.insert(
                "apply_phase".to_string(),
                workflow_apply_phase_payload_json(
                    if output
                        .get("applied_actions")
                        .and_then(JsonValue::as_array)
                        .is_some_and(|rows| rows.is_empty())
                    {
                        "authoritative_resume"
                    } else {
                        "done"
                    },
                    "done",
                    Some(detail),
                    output
                        .get("applied_actions")
                        .and_then(JsonValue::as_array)
                        .is_some_and(|rows| rows.is_empty()),
                ),
            );
            return Ok(JsonValue::Object(output));
        }
        if matches!(code.as_str(), "waiting_for_ci" | "waiting_for_land") {
            let detail = workflow_nested_text(&state, "next_action", "detail")
                .or_else(|| workflow_nested_text(&state, "next_action", "summary"))
                .unwrap_or_else(|| format!("Workflow finish helper is waiting at `{code}`."));
            let resumed = applied_actions.is_empty();
            workflow_progress_emit(
                &mut progress,
                if resumed { "resumed" } else { "waiting" },
                &code,
                current_change_id.as_deref(),
                current_patchset_id.as_deref(),
                None,
                Some(&detail),
                Some(if resumed {
                    "authoritative_resume"
                } else {
                    "pending_gate"
                }),
                None,
                None,
                None,
            )?;
            let mut output = state.as_object().cloned().unwrap_or_default();
            output.insert(
                "applied_actions".to_string(),
                JsonValue::Array(applied_actions),
            );
            output.insert(
                "mutation_receipts".to_string(),
                JsonValue::Array(mutation_receipts),
            );
            output.insert("apply_status".to_string(), JsonValue::String(code.clone()));
            output.insert(
                "apply_stopped_reason".to_string(),
                JsonValue::String(detail.clone()),
            );
            output.insert(
                "apply_phase".to_string(),
                workflow_apply_phase_payload_json(
                    if resumed {
                        "authoritative_resume"
                    } else {
                        "pending_gate"
                    },
                    &code,
                    Some(&detail),
                    resumed,
                ),
            );
            return Ok(JsonValue::Object(output));
        }
        let signature = format!(
            "{}|{}|{}|{}|{}|{}|{}",
            code,
            workflow_nested_text(&state, "patchset", "patchset_id").unwrap_or_default(),
            workflow_nested_text(&state, "change", "status").unwrap_or_default(),
            workflow_nested_text(&state, "policy", "decision").unwrap_or_default(),
            state
                .get("review")
                .and_then(|value| value.get("approvals"))
                .and_then(JsonValue::as_i64)
                .unwrap_or_default(),
            state
                .get("review")
                .and_then(|value| value.get("blocking"))
                .and_then(JsonValue::as_i64)
                .unwrap_or_default(),
            workflow_nested_text(&state, "landing_summary", "status").unwrap_or_default(),
        );
        if seen_signatures.contains(&signature) {
            let stopped_reason =
                format!("Workflow finish apply made no further progress at `{code}`.");
            workflow_progress_emit(
                &mut progress,
                "stopped",
                &code,
                current_change_id.as_deref(),
                current_patchset_id.as_deref(),
                Some(applied_actions.len() + 1),
                None,
                Some("stopped"),
                Some(&stopped_reason),
                None,
                None,
            )?;
            let mut output = state.as_object().cloned().unwrap_or_default();
            output.insert(
                "applied_actions".to_string(),
                JsonValue::Array(applied_actions),
            );
            output.insert(
                "mutation_receipts".to_string(),
                JsonValue::Array(mutation_receipts),
            );
            output.insert(
                "apply_status".to_string(),
                JsonValue::String("stopped".to_string()),
            );
            output.insert(
                "apply_stopped_reason".to_string(),
                JsonValue::String(stopped_reason.clone()),
            );
            output.insert(
                "apply_phase".to_string(),
                workflow_apply_phase_payload_json("stopped", &code, Some(&stopped_reason), false),
            );
            return Ok(JsonValue::Object(output));
        }
        seen_signatures.insert(signature);
        workflow_progress_emit(
            &mut progress,
            "starting",
            &code,
            current_change_id.as_deref(),
            current_patchset_id.as_deref(),
            Some(applied_actions.len() + 1),
            None,
            Some("mutation_started"),
            None,
            None,
            None,
        )?;
        if matches!(code.as_str(), "submit_land" | "complete_task") {
            let resolved_change_ref = state
                .get("change")
                .filter(|change| change.is_object())
                .map(|change| change_reference_from_payload(change, Some(change_id)))
                .transpose()?
                .unwrap_or_else(|| change_id.to_string());
            let atomic_change_ref = if resolved_change_ref.contains("/C-") {
                resolved_change_ref
            } else {
                workflow_nested_text(&state, "task", "task_id").unwrap_or(resolved_change_ref)
            };
            let atomic_output = task_land_apply(
                repo,
                &atomic_change_ref,
                remote_name,
                None::<fn(&JsonValue) -> Result<(), String>>,
            )?;
            let summary = workflow_applied_action_summary(&json!({
                "code": "submit_land",
                "result": atomic_output.clone(),
            }))
            .unwrap_or_else(|_| "completed atomic Task finish".to_string());
            workflow_progress_emit(
                &mut progress,
                "completed",
                "atomic_task_land",
                current_change_id.as_deref(),
                current_patchset_id.as_deref(),
                Some(applied_actions.len() + 1),
                Some("Atomic Task finish committed the reviewer-approved closeout."),
                Some("mutation_accepted"),
                None,
                None,
                Some(&summary),
            )?;
            return workflow_land_attach_atomic_task_land_history(
                atomic_output,
                applied_actions,
                mutation_receipts,
            );
        }
        let action = workflow_land_apply_action(
            repo,
            &code,
            &state,
            change_id,
            review_message,
            remote_name,
        )?;
        if let Some(stopped_reason) = workflow_root_text(&action, "stopped_reason") {
            workflow_progress_emit(
                &mut progress,
                "stopped",
                &code,
                current_change_id.as_deref(),
                current_patchset_id.as_deref(),
                Some(applied_actions.len() + 1),
                None,
                Some("stopped"),
                Some(&stopped_reason),
                None,
                None,
            )?;
            let mut output = state.as_object().cloned().unwrap_or_default();
            output.insert(
                "applied_actions".to_string(),
                JsonValue::Array(applied_actions),
            );
            output.insert(
                "mutation_receipts".to_string(),
                JsonValue::Array(mutation_receipts),
            );
            output.insert(
                "apply_status".to_string(),
                JsonValue::String("stopped".to_string()),
            );
            output.insert(
                "apply_stopped_reason".to_string(),
                JsonValue::String(stopped_reason.clone()),
            );
            output.insert(
                "apply_phase".to_string(),
                workflow_apply_phase_payload_json("stopped", &code, Some(&stopped_reason), false),
            );
            return Ok(JsonValue::Object(output));
        }
        let result = action
            .get("result")
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        let receipts = workflow_remote_action_mutation_receipts(&code, &result)
            .ok()
            .and_then(|value| value.as_array().cloned())
            .unwrap_or_default();
        mutation_receipts.extend(receipts);
        let summary = workflow_applied_action_summary(&json!({"code": code, "result": result}))
            .unwrap_or_else(|_| format!("completed `{code}`"));
        workflow_progress_emit(
            &mut progress,
            "completed",
            &code,
            current_change_id.as_deref(),
            action
                .get("patchset_id")
                .and_then(JsonValue::as_str)
                .or(current_patchset_id.as_deref()),
            Some(applied_actions.len() + 1),
            None,
            Some("mutation_accepted"),
            None,
            None,
            Some(&summary),
        )?;
        applied_actions.push(json!({"code": code, "result": result}));
    }
}

fn workflow_land_attach_atomic_task_land_history(
    mut output: JsonValue,
    mut reviewer_actions: Vec<JsonValue>,
    mut reviewer_receipts: Vec<JsonValue>,
) -> Result<JsonValue, String> {
    let object = output
        .as_object_mut()
        .ok_or_else(|| "Atomic Task finish output must decode to an object.".to_string())?;
    let reviewer_action_count = reviewer_actions.len();
    reviewer_actions.extend(
        object
            .get("applied_actions")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default(),
    );
    reviewer_receipts.extend(
        object
            .get("mutation_receipts")
            .and_then(JsonValue::as_array)
            .cloned()
            .unwrap_or_default(),
    );
    object.insert(
        "applied_actions".to_string(),
        JsonValue::Array(reviewer_actions),
    );
    object.insert(
        "mutation_receipts".to_string(),
        JsonValue::Array(reviewer_receipts),
    );
    object.insert(
        "reviewer_workflow".to_string(),
        json!({
            "contract": "workflow-land-reviewer-atomic-closeout/v1",
            "reviewer_action_count": reviewer_action_count,
            "finalizer": "task-land-atomic/v1",
        }),
    );
    Ok(output)
}

fn workflow_done_state_needs_converged_land_submit(
    state: &JsonValue,
    applied_actions: &[JsonValue],
) -> bool {
    if applied_actions.iter().any(|action| {
        action
            .get("code")
            .and_then(JsonValue::as_str)
            .is_some_and(|code| code == "submit_land")
    }) {
        return false;
    }
    if state.get("patchset").is_none_or(JsonValue::is_null) {
        return false;
    }
    let Some(landing_summary) = state.get("landing_summary").and_then(JsonValue::as_object) else {
        return false;
    };
    if landing_summary
        .get("status_source")
        .and_then(JsonValue::as_str)
        .is_some_and(|value| {
            matches!(
                value,
                "target_line_already_at_revision" | "target_line_already_contains_revision"
            )
        })
    {
        return true;
    }
    if landing_summary
        .get("target_line_already_at_revision")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    if landing_summary
        .get("target_line_already_contains_revision")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    landing_summary
        .get("result")
        .and_then(JsonValue::as_object)
        .is_some_and(|result| {
            result
                .get("target_line_already_at_revision")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
                || result
                    .get("target_line_already_contains_revision")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
        })
}

fn workflow_land_local(
    repo: &RepoRuntime,
    change_id: &str,
    target: Option<&str>,
    snapshot: Option<&str>,
    snapshot_message: Option<&str>,
) -> Result<JsonValue, String> {
    let resolved_change_id =
        normalized_text(Some(change_id)).ok_or_else(|| "change-id is required".to_string())?;
    let change_store = repo.change_store()?;
    let task_store = repo.task_store()?;
    let change = workflow_local_change_read_with_change_store(&change_store, &resolved_change_id)?;
    let change_status = string_field(&change, "status").unwrap_or_default();
    if change_status == "landed" {
        return Err(format!(
            "Local change {resolved_change_id} is already finished"
        ));
    }
    if matches!(change_status.as_str(), "archived") {
        return Err(format!(
            "Local change {resolved_change_id} is {change_status} and cannot be finished"
        ));
    }
    if !matches!(change_status.as_str(), "draft" | "active") {
        return Err(format!(
            "Local change {resolved_change_id} is {change_status} and cannot be finished"
        ));
    }
    if string_field(&change, "publication_state").as_deref() == Some("published") {
        return Err(format!(
            "Local change {resolved_change_id} has already been published; use `ait task finish` for shared closeout."
        ));
    }
    let local_change_id = required_string_field(&change, "change_id")?;
    let change_ref = change_reference_from_payload(&change, Some(&resolved_change_id))?;
    let task_id = required_string_field(&change, "task_id")?;
    let task = workflow_local_task_read_with_task_store(&task_store, &task_id)?;
    if string_field(&task, "publication_state").as_deref() == Some("published") {
        return Err(format!(
            "Local task {task_id} has already been published; use `ait task finish` for shared closeout."
        ));
    }
    let task_status = string_field(&task, "status").unwrap_or_default();
    if !matches!(task_status.as_str(), "active" | "completed") {
        return Err(format!(
            "Local task {task_id} is {task_status} and cannot be locally finished"
        ));
    }
    let current_line_name = repo.current_line_name()?;
    let mut created_snapshot = None;
    let status = workflow_workspace_status(repo, None, None)?;
    let revision_snapshot_id = if status
        .get("clean")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false)
    {
        let current_line_row = local_line_row(repo, &current_line_name)?;
        normalized_text(snapshot).or_else(|| string_field(&current_line_row, "head_snapshot_id"))
    } else {
        if snapshot.is_some() {
            return Err(
                "Workspace is dirty and `--snapshot` was supplied; clean the workspace or snapshot the current edits first."
                    .to_string(),
            );
        }
        let message = normalized_text(snapshot_message).ok_or_else(|| {
            let changed_count = status
                .get("changed_count")
                .and_then(JsonValue::as_i64)
                .unwrap_or_default();
            let changed_paths = status
                .get("changed_paths")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
                .filter_map(JsonValue::as_str)
                .take(8)
                .collect::<Vec<_>>();
            let changed_paths_hint = if changed_paths.is_empty() {
                String::new()
            } else {
                format!(": {}", changed_paths.join(", "))
            };
            format!(
                "Workspace is dirty ({changed_count} changed{changed_paths_hint}); pass `--message <MESSAGE>` to `ait task finish {change_ref} --local`, or create an intermediate Snapshot first."
            )
        })?;
        let snapshot = snapshot_create(repo, Some(message.as_str()))?;
        let revision_snapshot_id = required_string_field(&snapshot, "snapshot_id")?;
        created_snapshot = Some(snapshot);
        Some(revision_snapshot_id)
    };
    let revision_snapshot_id = revision_snapshot_id.ok_or_else(|| {
        format!(
            "Current line {current_line_name} has no head snapshot; pass --snapshot or create a snapshot first."
        )
    })?;
    if !local_snapshot_exists(repo, &revision_snapshot_id)? {
        return Err(format!("Unknown snapshot: {revision_snapshot_id}"));
    }
    let target_line = normalized_text(target)
        .or_else(|| string_field(&change, "base_line"))
        .unwrap_or_else(|| "main".to_string());
    let target_line_row = local_line_row(repo, &target_line)?;
    let previous_target_head_snapshot_id = string_field(&target_line_row, "head_snapshot_id");
    if previous_target_head_snapshot_id.is_some()
        && snapshot_distance_if_ancestor(
            repo,
            previous_target_head_snapshot_id.as_deref(),
            Some(&revision_snapshot_id),
        )?
        .is_none()
    {
        let guidance = if repo.is_worktree() {
            format!(
                " Run `ait worktree rebase --onto {target_line}` in the bound worktree and retry `ait task finish {change_ref} --local`."
            )
        } else {
            format!(
                " Rebase or retarget the current line onto `{target_line}` before retrying `ait task finish {change_ref} --local`."
            )
        };
        return Err(format!(
            "Local finish target `{target_line}` currently points at `{}`, but selected revision `{revision_snapshot_id}` does not descend from that head.{guidance}",
            previous_target_head_snapshot_id.as_deref().unwrap_or_default()
        ));
    }
    let peer_changes = workflow_local_change_rows_with_change_store(&change_store)?
        .into_iter()
        .filter(|row| {
            string_field(row, "task_id").as_deref() == Some(task_id.as_str())
                && string_field(row, "change_id").as_deref() != Some(local_change_id.as_str())
                && !matches!(
                    string_field(row, "status").unwrap_or_default().as_str(),
                    "landed" | "archived"
                )
        })
        .collect::<Vec<_>>();
    let effective_pre_land_target_snapshot_id = if let Some(previous_target_head_snapshot_id) =
        previous_target_head_snapshot_id.as_deref()
    {
        Some(
            workflow_effective_pre_land_target_snapshot_id(
                repo,
                &change,
                &revision_snapshot_id,
                previous_target_head_snapshot_id,
            )?
            .0,
        )
    } else {
        None
    };
    set_local_line_head(repo, &target_line, Some(&revision_snapshot_id))?;
    workflow_local_change_land_with_change_store(
        &change_store,
        &resolved_change_id,
        &target_line,
        &revision_snapshot_id,
        effective_pre_land_target_snapshot_id.as_deref(),
    )?;
    let resulting_task = if peer_changes.is_empty() {
        workflow_local_task_close_with_task_store(&task_store, &task_id, "completed")?
    } else {
        task
    };
    let repo_root_restore = workflow_repo_root_restore_after_land(
        repo,
        &target_line,
        previous_target_head_snapshot_id.as_deref(),
        Some(&revision_snapshot_id),
    )?;
    let bound_worktree_cleanup =
        if string_field(&repo_root_restore, "status").as_deref() == Some("failed") {
            json!({
                "status": "skipped",
                "reason": "repo_root_restore_failed",
                "task_id": task_id,
            })
        } else {
            workflow_bound_worktree_cleanup_after_local_land(
                repo,
                &task_id,
                string_field(&resulting_task, "status")
                    .unwrap_or_else(|| "completed".to_string())
                    .as_str(),
                "landed",
            )
            .unwrap_or_else(|error| {
                json!({
                    "status": "failed",
                    "reason": "post_land_cleanup_failed",
                    "error": error,
                    "task_id": task_id,
                })
            })
        };
    Ok(json!({
        "change_id": local_change_id,
        "change_ref": change_ref,
        "task_id": task_id,
        "target_line": target_line,
        "line_name": target_line,
        "previous_target_head_snapshot_id": previous_target_head_snapshot_id,
        "recorded_pre_land_target_snapshot_id": effective_pre_land_target_snapshot_id,
        "landed_snapshot_id": revision_snapshot_id,
        "change_status": "landed",
        "task_status": string_field(&resulting_task, "status").unwrap_or_else(|| "completed".to_string()),
        "task_status_before": task_status,
        "open_peer_change_count": peer_changes.len(),
        "current_line": current_line_name,
        "workspace_action": if string_field(&repo_root_restore, "status").as_deref() == Some("restored") {
            "restored"
        } else if string_field(&repo_root_restore, "status").as_deref() == Some("failed") {
            "failed"
        } else {
            "unchanged"
        },
        "auto_snapshot": created_snapshot,
        "repo_root_restore": repo_root_restore,
        "bound_worktree_cleanup": bound_worktree_cleanup,
    }))
}
