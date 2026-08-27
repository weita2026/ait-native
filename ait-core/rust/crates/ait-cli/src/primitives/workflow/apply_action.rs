use super::*;

fn workflow_publish_base_authority(
    state: &JsonValue,
    target: Option<&str>,
) -> Result<(String, String), String> {
    let change_base_line = workflow_nested_text(state, "change", "base_line");
    let projected_base_line = workflow_nested_text(state, "base_line", "line_name");
    if let (Some(change_line), Some(projected_line)) =
        (change_base_line.as_deref(), projected_base_line.as_deref())
    {
        if change_line != projected_line {
            return Err(format!(
                "Remote workflow base is inconsistent: Change base Line `{change_line}` does not match resolved base Line `{projected_line}`. Refresh workflow state before publishing."
            ));
        }
    }
    let base_line = change_base_line
        .or(projected_base_line)
        .or_else(|| normalized_text(target))
        .ok_or_else(|| {
            "Remote workflow state has no authoritative base Line. Refresh workflow state before publishing."
                .to_string()
        })?;

    let projected_snapshot_id = workflow_nested_text(state, "base_line", "head_snapshot_id");
    let freshness_snapshot_id = workflow_nested_text(state, "freshness", "remote_base_snapshot_id");
    if let (Some(projected), Some(freshness)) = (
        projected_snapshot_id.as_deref(),
        freshness_snapshot_id.as_deref(),
    ) {
        if projected != freshness {
            return Err(format!(
                "Remote workflow base is inconsistent for Line `{base_line}`: resolved head `{projected}` does not match the freshness check `{freshness}`. Refresh workflow state before publishing."
            ));
        }
    }
    let snapshot_id = projected_snapshot_id
        .or(freshness_snapshot_id)
        .ok_or_else(|| {
            format!(
                "Remote workflow base line `{base_line}` has no authoritative head Snapshot. Refresh workflow state before publishing."
            )
        })?;
    Ok((base_line, snapshot_id))
}

pub(in crate::primitives) fn workflow_auto_rebase_current_worktree_before_publish(
    repo: &RepoRuntime,
    state: &JsonValue,
    target: Option<&str>,
) -> Result<Option<JsonValue>, String> {
    if !repo.is_worktree() {
        return Ok(None);
    }
    let (base_line, authoritative_base_snapshot_id) =
        workflow_publish_base_authority(state, target)?;
    let prepared = prepare_worktree_rebase_to_snapshot(
        repo,
        None,
        &base_line,
        &authoritative_base_snapshot_id,
    )?;
    if prepared.old_base_snapshot_id == prepared.new_base_snapshot_id && !prepared.rewrites_ancestry
    {
        return Ok(None);
    }
    let worktree_name = prepared.worktree_name.clone();
    let line_name = prepared.line_name.clone();
    let old_base_snapshot_id = prepared.old_base_snapshot_id.clone();
    let old_head_snapshot_id = prepared.old_head_snapshot_id.clone();
    let new_base_snapshot_id = prepared.new_base_snapshot_id.clone();
    let feature_delta_count = prepared.plan.feature_delta_count;

    let rebase = apply_prepared_worktree_rebase(repo, prepared)?;
    let rebase_payload = rebase
        .get("rebase")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or(JsonValue::Null);
    if rebase_payload
        .get("status")
        .and_then(JsonValue::as_str)
        .is_some_and(|value| value == "conflicted")
    {
        let conflict_paths = json_string_list(rebase_payload.get("conflict_paths"));
        let sample = summarize_path_sample(&conflict_paths);
        return Err(format!(
            "Automatic worktree rebase before publishing conflicted on {sample}. Resolve the conflict with `ait worktree rebase --continue` or abort it with `ait worktree rebase --abort` before retrying `ait task finish`."
        ));
    }
    let rebase_status = rebase_payload
        .get("status")
        .and_then(JsonValue::as_str)
        .unwrap_or("rebased")
        .to_string();

    Ok(Some(json!({
        "status": rebase_status,
        "worktree_name": worktree_name,
        "line_name": line_name,
        "onto_line_name": base_line,
        "old_base_snapshot_id": old_base_snapshot_id,
        "old_head_snapshot_id": old_head_snapshot_id,
        "new_base_snapshot_id": new_base_snapshot_id,
        "feature_delta_count": feature_delta_count,
        "rebase": rebase_payload,
    })))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn workflow_publish_patchset_action_with_task_and_closeout_remotes<T, C>(
    repo: &RepoRuntime,
    task_remote: &mut T,
    closeout_remote: &mut C,
    remote_name: &str,
    repo_name: &str,
    change_id: &str,
    summary: &str,
    author_mode: &str,
    auto_rebase: Option<JsonValue>,
    phase_label: &str,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowLineReader
        + TaskWorkflowLineHeadUpdater
        + TaskWorkflowRepositoryReader
        + TaskWorkflowSnapshotExistenceReader
        + TaskWorkflowZstdPackUploader
        + ?Sized,
    C: TaskWorkflowPatchsetPublisher + TaskWorkflowPatchsetSelector + ?Sized,
{
    let _publish_range = perfetto_range!("ait.workflow_ready.publish");
    let mut result = {
        let _range = perfetto_range!("ait.workflow_ready.publish.remote_flow");
        super::change_flow::patchset_publish_flow_with_task_and_closeout_remotes(
            repo,
            task_remote,
            closeout_remote,
            remote_name,
            repo_name,
            change_id,
            summary,
            author_mode,
        )?
    };
    let published_patchset_id = workflow_nested_text(&result, "patchset", "patchset_id")
        .ok_or_else(|| {
            format!("Workflow {phase_label} apply could not resolve the published patchset id.")
        })?;
    if let Some(auto_rebase) = auto_rebase.clone() {
        if let Some(result_obj) = result.as_object_mut() {
            result_obj.insert("auto_rebase".to_string(), auto_rebase);
        }
    }
    let selection = {
        let _range = perfetto_range!("ait.workflow_ready.publish.select");
        super::change_flow::patchset_select_with_closeout_remote(
            closeout_remote,
            change_id,
            &published_patchset_id,
            repo_name,
        )?
    };
    let mut action = json!({
        "result": result,
        "patchset_id": Some(published_patchset_id),
        "selection": selection,
    });
    if let Some(auto_rebase) = auto_rebase {
        action["auto_rebase"] = auto_rebase;
    }
    Ok(action)
}

pub(in crate::primitives) fn workflow_ready_run_patchset_ci_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetCiRunner + ?Sized,
{
    let _range = perfetto_range!("ait.workflow_ready.ci.dispatch");
    workflow_run_patchset_ci_action_with_closeout_remote(
        closeout_remote,
        patchset,
        repo_name,
        "Workflow ready apply could not resolve a patchset for CI.",
        "workflow_ready_apply",
        Some(PATCHSET_CI_PROFILE_WORKFLOW_READY_FOREGROUND),
    )
}

pub(in crate::primitives) fn workflow_run_patchset_ci_action_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    repo_name: &str,
    missing_patchset_message: &str,
    trigger: &str,
    execution_profile: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetCiRunner + ?Sized,
{
    let patchset_id = string_field(patchset, "patchset_id")
        .ok_or_else(|| missing_patchset_message.to_string())?;
    Ok(json!({
        "result": super::change_flow::patchset_run_ci_with_closeout_remote(
            closeout_remote,
            &patchset_id,
            trigger,
            execution_profile,
            repo_name,
        )?
    }))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn workflow_ready_record_attestation_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: &str,
    model_name: Option<String>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowAttestationWriter + ?Sized,
{
    let _range = perfetto_range!("ait.workflow_ready.attestation.write");
    workflow_record_attestation_action_with_closeout_remote(
        closeout_remote,
        patchset,
        tests,
        lint,
        security,
        license,
        author_mode,
        model_name,
        repo_name,
        "Workflow ready apply could not resolve a patchset for attestation.",
        Some("pass"),
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn workflow_record_attestation_action_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: &str,
    model_name: Option<String>,
    repo_name: &str,
    missing_patchset_message: &str,
    default_tests: Option<&str>,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowAttestationWriter + ?Sized,
{
    let patchset_id = string_field(patchset, "patchset_id")
        .ok_or_else(|| missing_patchset_message.to_string())?;
    let resolved_patchset_id =
        super::change_flow::resolve_patchset_id(closeout_remote, &patchset_id, Some(repo_name))?;
    let resolved_tests = normalized_text(tests).or_else(|| default_tests.map(str::to_string));
    let attest_json = ait_core::attest_json::AttestJson::stateless();
    let evaluation_summary =
        attest_json.build_evaluation_summary(resolved_tests.as_deref(), lint, security, license);
    let (provenance_summary, detail) =
        attest_json.build_minimum_provenance(author_mode, model_name.as_deref())?;
    Ok(json!({
        "result": super::change_flow::attestation_put_with_closeout_remote(
            closeout_remote,
            &resolved_patchset_id,
            author_mode,
            &evaluation_summary,
            &provenance_summary,
            &detail,
            repo_name,
        )?
    }))
}

#[expect(
    clippy::too_many_arguments,
    reason = "ready action dispatcher keeps workflow facts and remote ports explicit"
)]
pub(in crate::primitives) fn workflow_ready_apply_action(
    repo: &RepoRuntime,
    code: &str,
    state: &JsonValue,
    change_id: &str,
    snapshot_message: Option<&str>,
    summary: Option<&str>,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: Option<&str>,
    model: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let patchset = state.get("patchset").cloned().unwrap_or(JsonValue::Null);
    match code {
        "snapshot_create" => {
            let _range = perfetto_range!("ait.workflow_ready.action.snapshot_create");
            Ok(json!({
                "result": snapshot_create(
                    repo,
                    Some(snapshot_message.unwrap_or("reviewable snapshot")),
                )?,
            }))
        }
        "publish_patchset" | "refresh_patchset" => {
            let _range = perfetto_range!("ait.workflow_ready.action.publish_patchset");
            {
                let _range = perfetto_range!("ait.workflow_ready.publish.guard");
                guard_no_planning_only_artifact_drift(repo, "ait workflow ready")?;
            }
            let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
            let mut task_remote = http_task_remote(repo, &remote_row)?;
            let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
            let auto_rebase = if repo.is_worktree() {
                let _range = perfetto_range!("ait.workflow_ready.publish.auto_rebase");
                let (base_line, expected_snapshot_id) =
                    workflow_publish_base_authority(state, None)?;
                let verified_line = task_start_remote_base_line_preflight_with_task_remote(
                    repo,
                    &remote_row,
                    &mut task_remote,
                    &repo_name,
                    &base_line,
                )
                .map_err(|err| {
                    format!(
                        "Cannot prepare remote workflow publication against `{base_line}` at `{expected_snapshot_id}`: {err}"
                    )
                })?;
                let verified_snapshot_id = string_field(&verified_line, "head_snapshot_id");
                if verified_snapshot_id.as_deref() != Some(expected_snapshot_id.as_str()) {
                    return Err(format!(
                        "Remote base line `{base_line}` changed after workflow state was read: expected `{expected_snapshot_id}`, found `{}`. Retry the workflow command so publication uses one current authoritative base.",
                        verified_snapshot_id.as_deref().unwrap_or("none"),
                    ));
                }
                workflow_auto_rebase_current_worktree_before_publish(repo, state, None)?
            } else {
                None
            };
            let resolved_author_mode = repo.effective_author_mode(author_mode);
            workflow_publish_patchset_action_with_task_and_closeout_remotes(
                repo,
                &mut task_remote,
                &mut closeout_remote,
                &remote_row.name,
                &repo_name,
                change_id,
                summary.unwrap_or("review summary"),
                &resolved_author_mode,
                auto_rebase,
                "ready",
            )
        }
        "record_attestation" => {
            let _range = perfetto_range!("ait.workflow_ready.action.record_attestation");
            {
                let _range = perfetto_range!("ait.workflow_ready.attestation.guard");
                guard_no_planning_only_artifact_drift(repo, "ait workflow ready")?;
            }
            let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
            let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
            let resolved_author_mode = repo.effective_author_mode(author_mode);
            let resolved_model_name = repo.effective_model_name(model);
            workflow_ready_record_attestation_with_closeout_remote(
                &mut closeout_remote,
                &patchset,
                tests,
                lint,
                security,
                license,
                &resolved_author_mode,
                resolved_model_name,
                &repo_name,
            )
        }
        "run_patchset_ci" => {
            let _range = perfetto_range!("ait.workflow_ready.action.run_patchset_ci");
            let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
            let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
            workflow_ready_run_patchset_ci_with_closeout_remote(
                &mut closeout_remote,
                &patchset,
                &repo_name,
            )
        }
        _ => Ok(json!({
            "stopped_reason": format!("Workflow ready apply does not own `{code}`; reviewer actions continue through `ait workflow finish`."),
        })),
    }
}

fn workflow_land_patchset_id(patchset: &JsonValue, message: &str) -> Result<String, String> {
    string_field(patchset, "patchset_id").ok_or_else(|| message.to_string())
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(in crate::primitives) fn workflow_land_record_attestation_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    tests: Option<&str>,
    lint: Option<&str>,
    security: Option<&str>,
    license: Option<&str>,
    author_mode: &str,
    model_name: Option<String>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowAttestationWriter + ?Sized,
{
    workflow_record_attestation_action_with_closeout_remote(
        closeout_remote,
        patchset,
        tests,
        lint,
        security,
        license,
        author_mode,
        model_name,
        repo_name,
        "Workflow finish apply could not resolve a patchset for attestation.",
        None,
    )
}

pub(in crate::primitives) fn workflow_land_record_task_review_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    change_id: &str,
    reviewer: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader
        + TaskWorkflowReviewRecorder
        + TaskWorkflowPolicyEvaluator
        + ?Sized,
{
    let (resolved_patchset_id, mut result) =
        workflow_record_review_action_result_with_closeout_remote(
            closeout_remote,
            patchset,
            change_id,
            reviewer,
            "task_approve",
            Some(AUTOMATIC_TASK_APPROVAL_COMMENT),
            false,
            repo_name,
            "Workflow finish apply could not resolve the patchset for task approval.",
        )?;
    let policy_refresh = {
        let _range = perfetto_range!("ait.workflow.review_policy_refresh");
        super::change_flow::policy_eval_with_closeout_remote(
            closeout_remote,
            &resolved_patchset_id,
            repo_name,
        )?
    };
    if let Some(result_obj) = result.as_object_mut() {
        result_obj.insert("policy_refresh".to_string(), policy_refresh);
    }
    Ok(json!({ "result": result }))
}

#[allow(clippy::too_many_arguments)]
fn workflow_record_review_action_result_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    change_id: &str,
    reviewer: &str,
    action: &str,
    comment: Option<&str>,
    blocking: bool,
    repo_name: &str,
    missing_patchset_message: &str,
) -> Result<(String, JsonValue), String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowReviewRecorder + ?Sized,
{
    let patchset_id = workflow_land_patchset_id(patchset, missing_patchset_message)?;
    let resolved_patchset_id = {
        let _range = perfetto_range!("ait.task_land.remote.review_patchset_resolve");
        super::change_flow::resolve_patchset_id(closeout_remote, &patchset_id, Some(repo_name))?
    };
    let result = {
        let _range = perfetto_range!("ait.task_land.remote.review_record");
        super::change_flow::review_record_with_closeout_remote(
            closeout_remote,
            change_id,
            &resolved_patchset_id,
            reviewer,
            action,
            comment,
            blocking,
            repo_name,
        )?
    };
    Ok((resolved_patchset_id, result))
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn workflow_record_review_action_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    change_id: &str,
    reviewer: &str,
    action: &str,
    comment: Option<&str>,
    blocking: bool,
    repo_name: &str,
    missing_patchset_message: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowReviewRecorder + ?Sized,
{
    let (_, result) = workflow_record_review_action_result_with_closeout_remote(
        closeout_remote,
        patchset,
        change_id,
        reviewer,
        action,
        comment,
        blocking,
        repo_name,
        missing_patchset_message,
    )?;
    Ok(json!({ "result": result }))
}

pub(in crate::primitives) fn workflow_land_record_code_review_summary_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    change_id: &str,
    reviewer: &str,
    review_message: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPatchsetReader + TaskWorkflowReviewRecorder + ?Sized,
{
    let missing = missing_code_review_summary_sections(review_message);
    if !missing.is_empty() {
        return Err(format!(
            "Code review summary is missing sections with non-placeholder content: {}.",
            missing.join(", ")
        ));
    }
    workflow_record_review_action_with_closeout_remote(
        closeout_remote,
        patchset,
        change_id,
        reviewer,
        "code_review_summary",
        Some(review_message),
        false,
        repo_name,
        "Workflow finish apply could not resolve the patchset for code review summary.",
    )
}

pub(in crate::primitives) fn workflow_land_evaluate_policy_with_closeout_remote<R>(
    closeout_remote: &mut R,
    patchset: &JsonValue,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowPolicyEvaluator + ?Sized,
{
    let patchset_id = workflow_land_patchset_id(
        patchset,
        "Workflow finish apply could not resolve a patchset for policy evaluation.",
    )?;
    Ok(json!({
        "result": super::change_flow::policy_eval_with_closeout_remote(
            closeout_remote,
            &patchset_id,
            repo_name,
        )?
    }))
}

#[allow(clippy::too_many_arguments)]
#[cfg(test)]
pub(in crate::primitives) fn workflow_land_submit_action_with_task_and_closeout_remotes<T, C, G>(
    repo: &RepoRuntime,
    task_remote: &mut T,
    closeout_remote: &mut C,
    repo_name: &str,
    change_id: &str,
    patchset: &JsonValue,
    target: Option<&str>,
    mode: &str,
    patchset_revision_snapshot_id: Option<&str>,
    resolve_change_task_id: bool,
    guard: G,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + TaskWorkflowLineReader
        + ?Sized,
    C: TaskWorkflowPatchsetReader + TaskWorkflowLandSubmitter + ?Sized,
    G: FnOnce(Option<&str>, &str) -> Result<(), String>,
{
    let _submit_range = perfetto_range!("ait.task_land.submit_flow");
    let patchset_id = string_field(patchset, "patchset_id");
    let target_line = normalized_text(target).unwrap_or_else(|| "main".to_string());
    let resolved_patchset_revision_snapshot_id = patchset_revision_snapshot_id
        .map(str::to_string)
        .or_else(|| string_field(patchset, "revision_snapshot_id"));
    let result = {
        let _range = perfetto_range!("ait.task_land.remote.land_submit_flow");
        super::change_flow::land_submit_flow_with_task_and_closeout_remotes(
            task_remote,
            closeout_remote,
            repo_name,
            change_id,
            patchset_id.as_deref(),
            &target_line,
            mode,
            resolve_change_task_id,
            guard,
        )?
    };
    let synced_result = {
        let _range = perfetto_range!("ait.task_land.local.land_sync");
        workflow_attach_local_land_sync_with_task_remote(
            repo,
            task_remote,
            repo_name,
            change_id,
            &result,
            resolved_patchset_revision_snapshot_id.as_deref(),
        )?
    };
    Ok(json!({"result": synced_result}))
}

pub(in crate::primitives) fn workflow_land_apply_action(
    repo: &RepoRuntime,
    code: &str,
    state: &JsonValue,
    change_id: &str,
    review_message: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let _action_range = perfetto_range!("ait.workflow_land.workflow_action");
    let patchset = state.get("patchset").cloned().unwrap_or(JsonValue::Null);
    match code {
        "snapshot_create" | "publish_patchset" | "refresh_patchset" | "record_attestation"
        | "run_patchset_ci" => Ok(json!({
            "stopped_reason": format!(
                "Workflow finish does not own `{code}`. Run `ait workflow ready <change-id> --apply` explicitly before land."
            ),
        })),
        "record_review" => {
            let _range = perfetto_range!("ait.workflow_land.action.record_review");
            if workflow_task_review_required(repo) {
                return Ok(json!({
                    "stopped_reason": "Task review is required. Record the explicit `ait review task approve` action shown by the workflow decision, then rerun workflow finish."
                }));
            }
            workflow_land_patchset_id(
                &patchset,
                "Workflow finish apply could not resolve the patchset for task approval.",
            )?;
            let resolved_reviewer = repo.task_review_reviewer_identity().ok_or_else(|| {
                "Workflow finish apply needs `ait config` `user_name` before it can auto-record task approval."
                    .to_string()
            })?;
            let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
            let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
            workflow_land_record_task_review_with_closeout_remote(
                &mut closeout_remote,
                &patchset,
                change_id,
                &resolved_reviewer,
                &repo_name,
            )
        }
        "record_code_review_summary" => {
            let _range = perfetto_range!("ait.workflow_land.action.record_code_review_summary");
            workflow_land_patchset_id(
                &patchset,
                "Workflow finish apply could not resolve the patchset for code review summary.",
            )?;
            let resolved_reviewer = repo.ai_code_review_reviewer_identity().ok_or_else(|| {
                "Workflow finish apply needs a reviewer identity before it can record code review evidence.".to_string()
            })?;
            let review_message = normalized_text(review_message).ok_or_else(|| {
                "Workflow finish apply needs --review-message containing the code review summary before it can record code review evidence.".to_string()
            })?;
            let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
            let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
            workflow_land_record_code_review_summary_with_closeout_remote(
                &mut closeout_remote,
                &patchset,
                change_id,
                &resolved_reviewer,
                &review_message,
                &repo_name,
            )
        }
        "evaluate_policy" => {
            let _range = perfetto_range!("ait.task_land.action.evaluate_policy");
            let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
            let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
            workflow_land_evaluate_policy_with_closeout_remote(
                &mut closeout_remote,
                &patchset,
                &repo_name,
            )
        }
        "submit_land" | "complete_task" => Ok(json!({
            "stopped_reason": "Workflow finish final closeout must be executed by its atomic Task Land boundary.",
        })),
        "workflow_ready" => Ok(json!({
            "stopped_reason": workflow_nested_text(state, "next_action", "detail")
                .or_else(|| workflow_nested_text(state, "next_action", "summary"))
                .unwrap_or_else(|| "Workflow finish apply stopped because the change still needs `workflow ready` first.".to_string()),
        })),
        "land_blocked" => Ok(json!({
            "stopped_reason": workflow_nested_text(state, "next_action", "detail")
                .or_else(|| workflow_nested_text(state, "next_action", "summary"))
                .unwrap_or_else(|| "Workflow finish apply stopped because land preflight is blocked.".to_string()),
        })),
        "address_blocking_review" => Ok(json!({
            "stopped_reason": "Workflow finish apply stopped because blocking review feedback still needs manual resolution.",
        })),
        _ => Ok(json!({
            "stopped_reason": format!("Workflow finish apply does not support automatic `{code}`."),
        })),
    }
}

#[cfg(test)]
mod remote_base_authority_tests {
    use super::*;

    #[test]
    fn publish_base_authority_requires_one_consistent_remote_head() {
        let state = json!({
            "change": {"base_line": "main"},
            "base_line": {
                "line_name": "main",
                "head_snapshot_id": "SNP-REMOTE"
            },
            "freshness": {"remote_base_snapshot_id": "SNP-REMOTE"}
        });

        assert_eq!(
            workflow_publish_base_authority(&state, None).unwrap(),
            ("main".to_string(), "SNP-REMOTE".to_string())
        );
    }

    #[test]
    fn publish_base_authority_rejects_line_or_snapshot_disagreement() {
        let line_error = workflow_publish_base_authority(
            &json!({
                "change": {"base_line": "main"},
                "base_line": {
                    "line_name": "release",
                    "head_snapshot_id": "SNP-REMOTE"
                }
            }),
            None,
        )
        .unwrap_err();
        assert!(line_error.contains("does not match resolved base Line"));

        let snapshot_error = workflow_publish_base_authority(
            &json!({
                "change": {"base_line": "main"},
                "base_line": {
                    "line_name": "main",
                    "head_snapshot_id": "SNP-ONE"
                },
                "freshness": {"remote_base_snapshot_id": "SNP-TWO"}
            }),
            None,
        )
        .unwrap_err();
        assert!(snapshot_error.contains("does not match the freshness check"));
    }

    #[test]
    fn publish_base_authority_rejects_a_null_remote_head() {
        let error = workflow_publish_base_authority(
            &json!({
                "change": {"base_line": "main"},
                "base_line": {"line_name": "main", "head_snapshot_id": null},
                "freshness": {"remote_base_snapshot_id": null}
            }),
            None,
        )
        .unwrap_err();

        assert!(error.contains("has no authoritative head Snapshot"));
    }
}
