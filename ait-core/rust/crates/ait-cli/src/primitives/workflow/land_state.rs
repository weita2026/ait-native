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
) -> Result<Option<JsonValue>, String>
where
    R: TaskWorkflowAttestationReader + ?Sized,
{
    let _range = perfetto_range!("ait.task_land.remote.attestation_read");
    let Some(patchset_id) = patchset_id else {
        return Ok(None);
    };
    match closeout_remote.get_attestation(patchset_id, Some(repo_name), true) {
        Ok(row) => {
            ait_core::attest_json::RemoteAttestationStatement::from_readback(&row)?;
            Ok(Some(row))
        }
        Err(error) if error.remote_status() == Some(404) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
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
        "contract": "ait.server.patchset_ci.readiness.v2",
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

#[cfg(test)]
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
    _include_landing_detail: bool,
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
    let explicit = patchset_id
        .map(|id| {
            workflow_land_patchset_read_with_closeout_remote(closeout_remote, repo_name, id, None)
        })
        .transpose()?;
    let requested = normalized_text(change_id);
    let (task, requested_change) =
        if let Some(reference) = requested.as_deref().filter(|id| !is_task_id(id)) {
            let (change, task) =
                workflow_land_change_task_read_with_task_remote(task_remote, repo_name, reference)?;
            let exact = change_reference_from_payload(&change, Some(reference))?;
            (task, Some(exact))
        } else {
            let task_id = requested
                .clone()
                .or_else(|| {
                    explicit
                        .as_ref()
                        .and_then(|row| string_field(row, "task_id"))
                })
                .ok_or("Task candidate requires exact Task ownership")?;
            let task = task_remote
                .get_task(&task_id, Some(repo_name))
                .map_err(|e| e.to_string())?;
            if string_field(&task, "task_id").as_deref() != Some(&task_id) {
                return Err("Task lookup returned another owner".into());
            }
            (task, None)
        };
    let task_id = required_string_field(&task, "task_id")?;
    let receipt = ait_core::land_json::LandJson::stateless()
        .task_successful_receipt(&task, &task_id, None)?;
    // Historical acceptance is immutable evidence, independent of Task latest.
    if let Some(receipt) = receipt {
        let accepted_id = required_string_field(&receipt, "patchset_id")?;
        let patchset = workflow_land_patchset_read_with_closeout_remote(
            closeout_remote,
            repo_name,
            &accepted_id,
            None,
        )?;
        let change_ref = workflow_validate_candidate_owner(&patchset, &task_id)?;
        if change_ref != string_field(&receipt, "change_ref")
            || requested_change
                .as_ref()
                .is_some_and(|id| Some(id) != change_ref.as_ref())
        {
            return Err("Task receipt has a different exact Change association".into());
        }
        let identity = WorkflowExpectedIdentity {
            repository_index: receipt["repository_index"]
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or("Task receipt Repository missing")?,
            patchset_id: accepted_id,
            base_snapshot_id: required_string_field(&receipt, "base_snapshot_id")?,
            revision_snapshot_id: required_string_field(&receipt, "revision_snapshot_id")?,
        };
        identity.validate_patchset(identity.repository_index, &patchset)?;
        if let Some(explicit) = explicit.as_ref() {
            identity.validate_patchset(identity.repository_index, explicit)?;
        }
        let change = workflow_read_optional_candidate_change(
            task_remote,
            repo_name,
            &task_id,
            change_ref.as_deref(),
        )?;
        let target = required_string_field(&receipt, "target_line")?;
        return Ok(
            json!({"landed":true,"task":task,"change":change,"patchset":patchset,"patchset_source":"accepted_task_receipt",
            "base_line_name":target,"target_line":target,"landing_summary":receipt,
            "resolved_task_id":task_id,"resolved_change_id":string_field(&change,"change_id"),"resolved_change_ref":change_ref}),
        );
    }
    if string_field(&task, "status").as_deref() != Some("active") {
        return Err("Task has no actual successful receipt and is not active; remote acceptance cannot be inferred from terminal Change state".into());
    }
    let current_id = match task.get("current_patchset_id") {
        Some(JsonValue::Null) => None,
        Some(JsonValue::String(id)) if !id.is_empty() && id.trim() == id => Some(id.clone()),
        _ => return Err("Task readback requires nullable current_patchset_id".into()),
    };
    let patchset = current_id
        .as_deref()
        .map(|id| {
            workflow_land_patchset_read_with_closeout_remote(closeout_remote, repo_name, id, None)
        })
        .transpose()?;
    if explicit.is_some() && patchset.is_none() {
        return Err("Explicit Patchset is not the Task's current candidate".into());
    }
    if let Some(current) = patchset.as_ref() {
        if current["withdrawn"].as_bool() == Some(true)
            || current["invalidated"].as_bool() == Some(true)
            || matches!(
                current["status"].as_str(),
                Some("withdrawn" | "invalidated")
            )
        {
            return Err("Task latest candidate is withdrawn or invalidated; an older passing candidate cannot replace it".into());
        }
        if let Some(explicit) = explicit.as_ref() {
            let identity = WorkflowExpectedIdentity {
                repository_index: 0,
                patchset_id: required_string_field(current, "patchset_id")?,
                base_snapshot_id: required_string_field(current, "base_snapshot_id")?,
                revision_snapshot_id: required_string_field(current, "revision_snapshot_id")?,
            };
            identity.validate_patchset(0, explicit)?;
        }
    }
    let resolved_change_ref = patchset
        .as_ref()
        .map(|row| workflow_validate_candidate_owner(row, &task_id))
        .transpose()?
        .flatten();
    if requested_change
        .as_ref()
        .is_some_and(|id| Some(id) != resolved_change_ref.as_ref())
        && patchset.is_some()
    {
        return Err("Task current candidate belongs to another exact revision".into());
    }
    let change = workflow_read_optional_candidate_change(
        task_remote,
        repo_name,
        &task_id,
        resolved_change_ref
            .as_deref()
            .or(requested_change.as_deref()),
    )?;
    let resolved_change_id = string_field(&change, "change_id");
    let base_line_name = "main".to_string();
    let target_line = base_line_name.clone();
    let base_line =
        workflow_land_base_line_read_with_task_remote(task_remote, repo_name, &base_line_name)?;
    let remote_base_snapshot_id = string_field(&base_line, "head_snapshot_id");
    let patchset_source = Some("task_latest".to_string());
    let mut review_summary = workflow_land_review_summary_read_with_closeout_remote(
        closeout_remote,
        repo_name,
        &task_id,
    )?;
    let candidate_id = patchset
        .as_ref()
        .and_then(|row| string_field(row, "patchset_id"));
    let attestation = workflow_land_attestation_read_with_closeout_remote(
        closeout_remote,
        repo_name,
        candidate_id.as_deref(),
    )?;
    if let (Some(row), Some(candidate)) = (attestation.as_ref(), patchset.as_ref()) {
        ait_core::attest_json::RemoteAttestationStatement::validate_for_patchset(row, candidate)?;
    }
    let persisted_ci = workflow_land_persisted_patchset_ci_status(patchset.as_ref());
    let patchset_ci_status = if ready_patchset_is_authoritative
        || persisted_ci
            .as_ref()
            .is_some_and(workflow_land_persisted_patchset_ci_is_completed)
    {
        persisted_ci
    } else {
        workflow_land_patchset_ci_status_read_with_closeout_remote(
            closeout_remote,
            repo_name,
            candidate_id.as_deref(),
        )
    };
    let policy = workflow_land_policy_read_with_closeout_remote(
        closeout_remote,
        repo_name,
        candidate_id.as_deref(),
    );
    let patchset_base_snapshot_id = patchset
        .as_ref()
        .and_then(|row| string_field(row, "base_snapshot_id"));
    let patchset_revision_snapshot_id = patchset
        .as_ref()
        .and_then(|row| string_field(row, "revision_snapshot_id"));
    let landing_summary = None::<JsonValue>;

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
    if let Some(summary) = review_summary.as_object_mut() {
        for field in [
            "human_approvals",
            "human_task_approvals",
            "eligible_human_approvals",
        ] {
            summary.insert(field.into(), review_lane_counts[field].clone());
        }
    }
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

fn workflow_validate_candidate_owner(
    patchset: &JsonValue,
    task_id: &str,
) -> Result<Option<String>, String> {
    ait_core::workflow_expected_identity::validate_patchset_owner_fields(patchset, task_id)
}

fn workflow_read_optional_candidate_change<T: TaskWorkflowRemoteChangeReader + ?Sized>(
    remote: &mut T,
    repo_name: &str,
    task_id: &str,
    reference: Option<&str>,
) -> Result<JsonValue, String> {
    let Some(reference) = reference else {
        return Ok(JsonValue::Null);
    };
    let change = remote
        .get_change(reference, Some(repo_name))
        .map_err(|e| e.to_string())?;
    if string_field(&change, "task_id").as_deref() != Some(task_id)
        || change_reference_from_payload(&change, Some(reference))? != reference
    {
        return Err("Candidate Change lookup returned another owner".into());
    }
    Ok(change)
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
        return Err("Provide a Task ID so Task finish can resolve its work.".to_string());
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
    if workflow_task_review_required(repo) && remote_state["landed"] == false {
        let counts = workflow_review_lane_counts(
            &remote_state["review_summary"],
            remote_state["patchset"]["patchset_id"].as_str(),
        );
        remote_state["task_review_approvals"] = counts["human_task_approvals"].clone();
    }

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
        let head_relation = workflow_patchset_head_relation(
            repo,
            patchset_revision_snapshot_id.as_deref(),
            revision_snapshot_id.as_deref(),
        )?;
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
            head_relation,
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
