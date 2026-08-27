use super::*;

pub(in crate::primitives) fn workflow_land_patchset_read_with_closeout_remote<R>(
    closeout_remote: &mut R,
    repo_name: &str,
    patchset_id: &str,
    change_ref: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.patchset_read");
    closeout_remote
        .get_patchset(patchset_id, Some(repo_name), change_ref)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_land_policy_read_with_closeout_remote<R>(
    closeout_remote: &mut R,
    repo_name: &str,
    patchset_id: Option<&str>,
) -> Option<JsonValue>
where
    R: TaskWorkflowPolicyReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.policy_read");
    let patchset_id = patchset_id?;
    closeout_remote
        .get_policy(patchset_id, Some(repo_name), true)
        .ok()
}

pub(in crate::primitives) fn workflow_land_attestation_read_with_closeout_remote<R>(
    closeout_remote: &mut R,
    repo_name: &str,
    patchset_id: Option<&str>,
) -> Option<JsonValue>
where
    R: TaskWorkflowAttestationReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.attestation_read");
    let patchset_id = patchset_id?;
    closeout_remote
        .get_attestation(patchset_id, Some(repo_name), true)
        .ok()
}

pub(in crate::primitives) fn workflow_land_patchset_ci_status_read_with_closeout_remote<R>(
    closeout_remote: &mut R,
    repo_name: &str,
    patchset_id: Option<&str>,
) -> Option<JsonValue>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowPatchsetCiStatusReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.ci_readiness_read");
    let patchset_id = patchset_id?;
    super::change_flow::patchset_ci_readiness_with_closeout_remote(
        closeout_remote,
        patchset_id,
        repo_name,
        10,
    )
    .ok()
}

fn workflow_land_persisted_patchset_ci_status(patchset: Option<&JsonValue>) -> Option<JsonValue> {
    let patchset = patchset.and_then(JsonValue::as_object)?;
    let ci = patchset.get("ci").and_then(JsonValue::as_object)?;
    let ci_run_seq = patchset
        .get("ci_run_seq")
        .and_then(JsonValue::as_u64)
        .or_else(|| ci.get("run_seq").and_then(JsonValue::as_u64))
        .unwrap_or_default();
    let ci_completed_at_s = patchset
        .get("ci_completed_at_s")
        .and_then(JsonValue::as_u64)
        .or_else(|| ci.get("completed_at_s").and_then(JsonValue::as_u64))
        .unwrap_or_default();
    let suite_result_count = ci
        .get("suite_result_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default();
    let blocking_failure_count = ci
        .get("blocking_failure_count")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default();
    Some(json!({
        "contract": "ait.server.patchset_ci.readiness.v1",
        "projection": "embedded_patchset",
        "patchset_id": patchset.get("patchset_id").cloned().unwrap_or(JsonValue::Null),
        "ci_run_seq": ci_run_seq,
        "ci_completed_at_s": ci_completed_at_s,
        "tests_status": ci.get("tests_status").cloned().unwrap_or(JsonValue::Null),
        "overall_status": ci.get("overall_status").cloned().unwrap_or(JsonValue::Null),
        "lint_status": ci.get("lint_status").cloned().unwrap_or(JsonValue::Null),
        "selected_suite_count": ci.get("selected_suite_count").cloned().unwrap_or(JsonValue::from(0)),
        "suite_result_count": suite_result_count,
        "blocking_failure_count": blocking_failure_count,
        "has_runnable_evidence": ci_completed_at_s > 0
            && (suite_result_count > 0 || blocking_failure_count > 0),
        "latest_job": JsonValue::Null,
        "recent_jobs": [],
    }))
}

fn workflow_land_persisted_patchset_ci_is_completed(status: &JsonValue) -> bool {
    status
        .get("ci_run_seq")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default()
        > 0
        && status
            .get("ci_completed_at_s")
            .and_then(JsonValue::as_u64)
            .unwrap_or_default()
            > 0
        && status
            .get("has_runnable_evidence")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
}

pub(in crate::primitives) fn workflow_land_review_summary_read_with_closeout_remote<R>(
    closeout_remote: &mut R,
    repo_name: &str,
    change_id: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowReviewLister + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.review_list");
    closeout_remote
        .list_reviews(change_id, Some(repo_name), true)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_land_change_task_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    change_id: &str,
) -> Result<(JsonValue, JsonValue), String>
where
    R: TaskWorkflowRemoteChangeReader + TaskWorkflowRemoteTaskReader + ?Sized,
{
    let change = {
        let _range = perfetto_range!("ait.task_land.remote.change_read");
        task_remote
            .get_change(change_id, Some(repo_name))
            .map_err(|err| err.to_string())?
    };
    let task = {
        let _range = perfetto_range!("ait.task_land.remote.task_read");
        task_remote
            .get_task(&required_string_field(&change, "task_id")?, Some(repo_name))
            .map_err(|err| err.to_string())?
    };
    Ok((change, task))
}

pub(in crate::primitives) fn workflow_land_base_line_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    base_line_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowLineReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.target_line_read");
    task_remote
        .get_line(repo_name, base_line_name)
        .map_err(|err| err.to_string())
}

pub(in crate::primitives) fn workflow_land_change_detail_read_with_task_remote<R>(
    task_remote: &mut R,
    repo_name: &str,
    change_id: &str,
) -> JsonValue
where
    R: TaskWorkflowRemoteChangeDetailReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.change_detail_read");
    task_remote
        .get_change_detail(change_id, Some(repo_name))
        .unwrap_or_else(|_| json!({}))
}

fn workflow_target_line_contains_revision_landing_summary(
    change_id: &str,
    target_line: &str,
    patchset: Option<&JsonValue>,
    remote_base_snapshot_id: Option<&str>,
    patchset_revision_snapshot_id: Option<&str>,
) -> Option<JsonValue> {
    let remote_base_snapshot_id = remote_base_snapshot_id?;
    let patchset_revision_snapshot_id = patchset_revision_snapshot_id?;
    if remote_base_snapshot_id == patchset_revision_snapshot_id {
        return None;
    }
    let mut result = json!({
        "landed_snapshot_id": remote_base_snapshot_id,
        "selected_revision_snapshot_id": patchset_revision_snapshot_id,
        "target_line_head": remote_base_snapshot_id,
        "line_action": "already_contains_selected_patchset_revision",
        "target_line_already_contains_revision": true,
    });
    let mut summary = json!({
        "status": "landed",
        "status_source": "target_line_already_contains_revision",
        "target_line": target_line,
        "target_line_already_contains_revision": true,
        "change_id": change_id,
        "result": result.take(),
    });
    if let Some(patchset_id) = patchset.and_then(|value| string_field(value, "patchset_id")) {
        summary["patchset_id"] = JsonValue::String(patchset_id);
    }
    Some(summary)
}

#[cfg(test)]
pub(in crate::primitives) fn workflow_land_remote_state_with_remotes<T, C>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    repo_name: &str,
    change_id: Option<&str>,
    patchset_id: Option<&str>,
    ready_patchset_is_authoritative: bool,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowLineReader
        + TaskWorkflowRemoteTaskReader
        + ?Sized,
    C: TaskWorkflowPatchsetReader
        + TaskWorkflowReviewLister
        + TaskWorkflowAttestationReader
        + TaskWorkflowPatchsetCiStatusReader
        + TaskWorkflowPolicyReader
        + ?Sized,
{
    workflow_land_remote_state_with_remotes_and_detail_mode(
        task_remote,
        closeout_remote,
        repo_name,
        change_id,
        patchset_id,
        ready_patchset_is_authoritative,
        true,
    )
}

fn workflow_land_remote_state_with_remotes_and_detail_mode<T, C>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    repo_name: &str,
    change_id: Option<&str>,
    patchset_id: Option<&str>,
    ready_patchset_is_authoritative: bool,
    include_landing_detail: bool,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowLineReader
        + TaskWorkflowRemoteTaskReader
        + ?Sized,
    C: TaskWorkflowPatchsetReader
        + TaskWorkflowReviewLister
        + TaskWorkflowAttestationReader
        + TaskWorkflowPatchsetCiStatusReader
        + TaskWorkflowPolicyReader
        + ?Sized,
{
    if change_id.is_none() && patchset_id.is_none() {
        return Err("Provide CHANGE_ID so Task finish can resolve a Change.".to_string());
    }
    let resolved_patchset_id = normalized_text(patchset_id);
    let mut requested_change = normalized_text(change_id);
    let mut explicit_patchset = None;
    if let Some(ref patchset_ref) = resolved_patchset_id {
        let patchset = workflow_land_patchset_read_with_closeout_remote(
            closeout_remote,
            repo_name,
            patchset_ref,
            None,
        )?;
        requested_change = requested_change
            .or_else(|| string_field(&patchset, "change_ref"))
            .or_else(|| string_field(&patchset, "change_id"));
        explicit_patchset = Some(patchset);
    }
    let requested_change = requested_change
        .ok_or_else(|| "Could not resolve a Change for Task finish.".to_string())?;
    let (change, task) =
        workflow_land_change_task_read_with_task_remote(task_remote, repo_name, &requested_change)?;
    let resolved_change_id = required_string_field(&change, "change_id")?;
    let resolved_change_ref =
        change_reference_from_payload(&change, Some(requested_change.as_str()))?;
    let requested_canonical = canonical_change_id(&requested_change)?;
    if requested_canonical != resolved_change_id
        || (requested_change != requested_canonical && requested_change != resolved_change_ref)
    {
        return Err(format!(
            "Remote change lookup for `{requested_change}` returned unrelated change `{resolved_change_ref}`."
        ));
    }
    if explicit_patchset.as_ref().is_some_and(|patchset| {
        !payload_belongs_to_change(patchset, &resolved_change_id, &resolved_change_ref)
    }) {
        return Err(format!(
            "Patchset {} does not belong to change {}.",
            explicit_patchset
                .as_ref()
                .and_then(|value| string_field(value, "patchset_id"))
                .unwrap_or_default(),
            resolved_change_ref
        ));
    }
    let selected_patchset_id = resolved_patchset_id
        .clone()
        .or_else(|| string_field(&change, "selected_patchset_id"))
        .or_else(|| string_field(&change, "current_patchset_id"));
    let patchset_source = if resolved_patchset_id.is_some() {
        Some("explicit".to_string())
    } else if change.get("selected_patchset_id").is_some() {
        Some("selected".to_string())
    } else {
        Some("current".to_string())
    };

    let base_line_name = string_field(&change, "base_line").unwrap_or_else(|| "main".to_string());
    let target_line = base_line_name.clone();
    if string_field(&change, "status").as_deref() == Some("landed") {
        let landed_patchset = explicit_patchset.or_else(|| {
            selected_patchset_id.as_ref().map(|value| {
                json!({
                    "patchset_id": value,
                    "change_id": resolved_change_id,
                    "change_ref": resolved_change_ref,
                })
            })
        });
        return Ok(json!({
            "landed": true,
            "change": change,
            "task": task,
            "patchset": landed_patchset,
            "patchset_source": if landed_patchset.is_some() { patchset_source } else { None::<String> },
            "base_line_name": base_line_name,
            "target_line": target_line,
            "resolved_change_id": resolved_change_id,
            "resolved_change_ref": resolved_change_ref,
        }));
    }

    let patchset = if explicit_patchset.is_some() {
        explicit_patchset
    } else if let Some(selected_patchset_id) = selected_patchset_id.as_deref() {
        Some(workflow_land_patchset_read_with_closeout_remote(
            closeout_remote,
            repo_name,
            selected_patchset_id,
            Some(&resolved_change_ref),
        )?)
    } else {
        None
    };
    if patchset.as_ref().is_some_and(|value| {
        !payload_belongs_to_change(value, &resolved_change_id, &resolved_change_ref)
    }) {
        return Err(format!(
            "Patchset {} does not belong to change {}.",
            patchset
                .as_ref()
                .and_then(|value| string_field(value, "patchset_id"))
                .unwrap_or_default(),
            resolved_change_ref
        ));
    }
    let base_line =
        workflow_land_base_line_read_with_task_remote(task_remote, repo_name, &base_line_name)?;
    let remote_base_snapshot_id = string_field(&base_line, "head_snapshot_id");
    let review_summary = workflow_land_review_summary_read_with_closeout_remote(
        closeout_remote,
        repo_name,
        &resolved_change_ref,
    )?;
    let attestation_patchset_id = patchset
        .as_ref()
        .and_then(|value| string_field(value, "patchset_id"));
    let attestation = workflow_land_attestation_read_with_closeout_remote(
        closeout_remote,
        repo_name,
        attestation_patchset_id.as_deref(),
    );
    let persisted_patchset_ci_status =
        workflow_land_persisted_patchset_ci_status(patchset.as_ref());
    let patchset_ci_status = if ready_patchset_is_authoritative
        || persisted_patchset_ci_status
            .as_ref()
            .is_some_and(workflow_land_persisted_patchset_ci_is_completed)
    {
        persisted_patchset_ci_status
    } else {
        workflow_land_patchset_ci_status_read_with_closeout_remote(
            closeout_remote,
            repo_name,
            attestation_patchset_id.as_deref(),
        )
    };
    let policy_patchset_id = patchset
        .as_ref()
        .and_then(|value| string_field(value, "patchset_id"));
    let policy = workflow_land_policy_read_with_closeout_remote(
        closeout_remote,
        repo_name,
        policy_patchset_id.as_deref(),
    );
    let patchset_base_snapshot_id = patchset
        .as_ref()
        .and_then(|value| string_field(value, "base_snapshot_id"));
    let patchset_revision_snapshot_id = patchset
        .as_ref()
        .and_then(|value| string_field(value, "revision_snapshot_id"));
    let change_detail = if ready_patchset_is_authoritative || !include_landing_detail {
        json!({})
    } else {
        workflow_land_change_detail_read_with_task_remote(
            task_remote,
            repo_name,
            &resolved_change_ref,
        )
    };
    let landing_summary = workflow_target_line_converged_landing_summary(
        workflow_relevant_landing_summary(change_detail.get("landing_summary"), patchset.as_ref()),
        patchset.as_ref(),
        &resolved_change_id,
        &target_line,
        remote_base_snapshot_id.as_deref(),
        patchset_base_snapshot_id.as_deref(),
        patchset_revision_snapshot_id.as_deref(),
    );
    if workflow_change_effectively_landed(&change, landing_summary.as_ref()) {
        let landed_patchset = patchset.clone().or_else(|| {
            selected_patchset_id.as_ref().map(|value| {
                json!({
                    "patchset_id": value,
                    "change_id": resolved_change_id,
                    "change_ref": resolved_change_ref,
                })
            })
        });
        return Ok(json!({
            "landed": true,
            "change": workflow_project_landed_change(&change, landing_summary.as_ref()),
            "task": task,
            "patchset": landed_patchset,
            "patchset_source": if landed_patchset.is_some() { patchset_source } else { None::<String> },
            "base_line_name": base_line_name,
            "target_line": target_line,
            "landing_summary": landing_summary,
            "resolved_change_id": resolved_change_id,
            "resolved_change_ref": resolved_change_ref,
        }));
    }

    let mut tests_state = ait_core::attest_json::AttestJson::stateless()
        .tests_state_from_attestation(attestation.as_ref())
        .unwrap_or_default();
    if tests_state.is_empty() {
        if let Some(policy_checks) = policy
            .as_ref()
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("checks"))
            .and_then(JsonValue::as_array)
        {
            for check in policy_checks {
                if string_field(check, "name").as_deref() == Some("tests") {
                    tests_state = string_field(check, "status").unwrap_or_default();
                    break;
                }
            }
        }
    }

    let base_is_fresh = patchset.is_none()
        || patchset_base_snapshot_id.as_deref() == remote_base_snapshot_id.as_deref();
    let review_lane_counts = workflow_review_lane_counts(
        &review_summary,
        patchset
            .as_ref()
            .and_then(|value| string_field(value, "patchset_id"))
            .as_deref(),
    );
    let review_blocking = review_lane_counts
        .get("blocking")
        .and_then(JsonValue::as_i64)
        .unwrap_or_default();
    let review_approvals = review_lane_counts
        .get("approvals")
        .and_then(JsonValue::as_i64)
        .unwrap_or_default();
    let task_review_approvals = review_lane_counts
        .get("task_approvals")
        .and_then(JsonValue::as_i64)
        .unwrap_or_default();
    let team_review_approvals = review_lane_counts
        .get("team_approvals")
        .and_then(JsonValue::as_i64)
        .unwrap_or_default();
    let code_review_summary_count = workflow_code_review_summary_count(
        &review_summary,
        patchset
            .as_ref()
            .and_then(|value| string_field(value, "patchset_id"))
            .as_deref(),
    );
    let policy_decision = policy
        .as_ref()
        .and_then(|value| string_field(value, "decision"))
        .unwrap_or_else(|| "pending".to_string());
    let requires_code_review_summary = workflow_requires_code_review_summary(
        patchset.as_ref(),
        attestation.as_ref(),
        policy.as_ref(),
    );
    let landing_status = landing_summary
        .as_ref()
        .and_then(|value| string_field(value, "status"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let landing_submission_id = landing_summary
        .as_ref()
        .and_then(|value| string_field(value, "submission_id"));
    let landing_result = landing_summary
        .as_ref()
        .and_then(|value| value.get("result"))
        .cloned()
        .filter(JsonValue::is_object)
        .unwrap_or_else(|| json!({}));
    let landing_blocker_class = landing_result
        .get("blocker_class")
        .and_then(JsonValue::as_str)
        .map(|value| value.trim().to_ascii_uppercase())
        .unwrap_or_default();
    let stale_policy_blocker_cleared = landing_status == "blocked"
        && landing_blocker_class == "POLICY_BLOCKED"
        && policy_decision == "pass";

    Ok(json!({
        "landed": false,
        "change": change,
        "task": task,
        "patchset": patchset,
        "patchset_source": if patchset.is_some() { patchset_source } else { None::<String> },
        "base_line_name": base_line_name,
        "target_line": target_line,
        "remote_base_snapshot_id": remote_base_snapshot_id,
        "review_summary": review_summary,
        "attestation": attestation,
        "patchset_ci_status": patchset_ci_status,
        "policy": policy,
        "landing_summary": landing_summary,
        "tests_state": tests_state,
        "patchset_base_snapshot_id": patchset_base_snapshot_id,
        "patchset_revision_snapshot_id": patchset_revision_snapshot_id,
        "base_is_fresh": base_is_fresh,
        "review_blocking": review_blocking,
        "review_approvals": review_approvals,
        "task_review_approvals": task_review_approvals,
        "team_review_approvals": team_review_approvals,
        "code_review_summary_count": code_review_summary_count,
        "policy_decision": policy_decision,
        "requires_code_review_summary": requires_code_review_summary,
        "landing_status": landing_status,
        "landing_submission_id": landing_submission_id,
        "landing_result": landing_result,
        "landing_blocker_class": landing_blocker_class,
        "stale_policy_blocker_cleared": stale_policy_blocker_cleared,
        "resolved_change_id": resolved_change_id,
        "resolved_change_ref": resolved_change_ref,
    }))
}

pub(in crate::primitives) fn workflow_land_workspace_context(
    repo: &RepoRuntime,
    ignore_workspace_authoring: bool,
) -> Result<(JsonValue, String, Option<String>), String> {
    workflow_land_workspace_context_with_status_reader(repo, ignore_workspace_authoring, |repo| {
        workflow_workspace_status(repo, None, None)
    })
}

pub(in crate::primitives) fn workflow_land_workspace_context_with_status_reader<F>(
    repo: &RepoRuntime,
    ignore_workspace_authoring: bool,
    read_workspace_status: F,
) -> Result<(JsonValue, String, Option<String>), String>
where
    F: FnOnce(&RepoRuntime) -> Result<JsonValue, String>,
{
    if !ignore_workspace_authoring {
        let workspace = read_workspace_status(repo)?;
        let current_line_name = required_string_field(&workspace, "current_line")?;
        let current_line_info = local_line_row(repo, &current_line_name)?;
        let revision_snapshot_id = string_field(&current_line_info, "head_snapshot_id")
            .or_else(|| string_field(&workspace, "baseline_snapshot_id"));
        return Ok((workspace, current_line_name, revision_snapshot_id));
    }

    let current_line_name = repo.current_line_name()?;
    let current_line_info = local_line_row(repo, &current_line_name)?;
    let revision_snapshot_id = string_field(&current_line_info, "head_snapshot_id");
    Ok((
        json!({
            "repo_name": repo.repo_name(),
            "workspace_root": repo.workspace_root().to_string_lossy().to_string(),
            "is_worktree": repo.is_worktree(),
            "current_line": current_line_name,
            "baseline_source": "current_line_head",
            "baseline_line_name": current_line_name,
            "baseline_snapshot_id": revision_snapshot_id,
            "clean": JsonValue::Null,
            "changed_count": JsonValue::Null,
            "changed_paths": [],
            "evaluation": "skipped",
            "reason": "ready_patchset_is_authoritative",
            "read_scope": "line_and_bound_worktree_metadata_only",
        }),
        current_line_name,
        revision_snapshot_id,
    ))
}

pub(in crate::primitives) fn workflow_hydrate_land_state(
    repo: &RepoRuntime,
    change_id: Option<&str>,
    patchset_id: Option<&str>,
    remote_name: Option<&str>,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
    include_landing_detail: bool,
) -> Result<JsonValue, String> {
    let _hydrate_range = perfetto_range!("ait.task_land.state_hydration");
    if change_id.is_none() && patchset_id.is_none() {
        return Err("Provide CHANGE_ID so Task finish can resolve a Change.".to_string());
    }
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    let mut remote_state = {
        let _range = perfetto_range!("ait.task_land.remote_state_hydration");
        workflow_land_remote_state_with_remotes_and_detail_mode(
            &mut task_remote,
            &mut closeout_remote,
            &repo_name,
            change_id,
            patchset_id,
            patchset_is_authoritative,
            include_landing_detail,
        )?
    };

    let (workspace, current_line_name, revision_snapshot_id) = {
        let _range = perfetto_range!("ait.task_land.local.workspace_context");
        workflow_land_workspace_context(repo, ignore_workspace_authoring)?
    };

    let landed = remote_state
        .get("landed")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if !landed {
        let patchset = remote_state
            .get("patchset")
            .cloned()
            .filter(JsonValue::is_object);
        let base_line_name = required_string_field(&remote_state, "base_line_name")?;
        let remote_base_snapshot_id = string_field(&remote_state, "remote_base_snapshot_id");
        let patchset_base_snapshot_id = string_field(&remote_state, "patchset_base_snapshot_id");
        let patchset_revision_snapshot_id =
            string_field(&remote_state, "patchset_revision_snapshot_id");
        let target_line_contains_patchset_revision =
            if remote_base_snapshot_id.as_deref() != patchset_revision_snapshot_id.as_deref() {
                snapshot_distance_if_ancestor(
                    repo,
                    patchset_revision_snapshot_id.as_deref(),
                    remote_base_snapshot_id.as_deref(),
                )?
                .is_some()
            } else {
                false
            };
        if target_line_contains_patchset_revision {
            if let Some(landing_summary) = workflow_target_line_contains_revision_landing_summary(
                &required_string_field(&remote_state, "resolved_change_id")?,
                &base_line_name,
                patchset.as_ref(),
                remote_base_snapshot_id.as_deref(),
                patchset_revision_snapshot_id.as_deref(),
            ) {
                let object = remote_state
                    .as_object_mut()
                    .ok_or_else(|| "Workflow finish remote state must be an object.".to_string())?;
                object.insert("landed".to_string(), JsonValue::Bool(true));
                object.insert("landing_summary".to_string(), landing_summary);
            }
        }
        let base_is_fresh = remote_state
            .get("base_is_fresh")
            .and_then(JsonValue::as_bool)
            .unwrap_or_else(|| {
                patchset.is_none()
                    || patchset_base_snapshot_id.as_deref() == remote_base_snapshot_id.as_deref()
            });
        let workspace_matches_patchset = if patchset_is_authoritative && patchset.is_some() {
            Some(true)
        } else if patchset.is_none() || revision_snapshot_id.is_none() {
            None
        } else {
            Some(revision_snapshot_id == patchset_revision_snapshot_id)
        };
        let root_repo = workflow_root_repo(repo)?;
        let worktree_retarget = workflow_current_worktree_retarget(
            repo,
            &root_repo,
            &current_line_name,
            revision_snapshot_id.as_deref(),
            remote_base_snapshot_id.as_deref(),
        )?;
        let patchset_refresh = workflow_patchset_refresh_context(
            patchset.as_ref(),
            worktree_retarget.as_ref(),
            &base_line_name,
            remote_base_snapshot_id.as_deref(),
            patchset_base_snapshot_id.as_deref(),
            patchset_revision_snapshot_id.as_deref(),
            revision_snapshot_id.as_deref(),
            base_is_fresh,
            workspace_matches_patchset,
        );
        let object = remote_state
            .as_object_mut()
            .ok_or_else(|| "Workflow finish remote state must be an object.".to_string())?;
        object.insert(
            "workspace_matches_patchset".to_string(),
            json!(workspace_matches_patchset),
        );
        object.insert("worktree_retarget".to_string(), json!(worktree_retarget));
        object.insert("patchset_refresh".to_string(), json!(patchset_refresh));
    }

    let object = remote_state
        .as_object_mut()
        .ok_or_else(|| "Workflow finish remote state must be an object.".to_string())?;
    object.insert("workspace".to_string(), workspace);
    object.insert(
        "current_line_name".to_string(),
        JsonValue::String(current_line_name),
    );
    object.insert(
        "revision_snapshot_id".to_string(),
        json!(revision_snapshot_id),
    );
    object.insert(
        "ignore_workspace_authoring".to_string(),
        JsonValue::Bool(ignore_workspace_authoring),
    );
    object.insert(
        "patchset_is_authoritative".to_string(),
        JsonValue::Bool(patchset_is_authoritative),
    );
    object.insert(
        "resolved_remote_name".to_string(),
        JsonValue::String(remote_row.name),
    );
    Ok(remote_state)
}
