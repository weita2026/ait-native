use super::*;

pub fn review_team_approve(
    repo: &RepoRuntime,
    change_id: &str,
    patchset_id: Option<&str>,
    reviewer: Option<&str>,
    message: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    if !repo.team_review_enabled() {
        return Err(
            "`ait review team ...` is only available when `workflow_mode=team_remote`.".to_string(),
        );
    }
    let resolved_reviewer = repo.reviewer_identity(reviewer).ok_or_else(|| {
        "No reviewer identity available. Pass --reviewer or configure user_name/user_email."
            .to_string()
    })?;
    record_review(
        repo,
        change_id,
        patchset_id,
        &resolved_reviewer,
        "approve",
        normalized_text(message),
        false,
        remote_name,
    )
}

pub fn review_show(
    repo: &RepoRuntime,
    change_id: &str,
    remote_name: Option<&str>,
    repo_name_override: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, repo_name_override)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    review_show_with_closeout_remote(&mut closeout_remote, change_id, &repo_name)
}

pub(in crate::primitives) fn review_show_with_closeout_remote<R>(
    closeout_remote: &mut R,
    change_id: &str,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowReviewLister + ?Sized,
{
    closeout_remote
        .list_reviews(change_id, Some(repo_name), false)
        .map_err(|err| err.to_string())
}

pub fn review_request(
    repo: &RepoRuntime,
    change_id: &str,
    patchset_id: Option<&str>,
    reviewer_groups: &[String],
    note: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    if patchset_id.is_some() {
        let resolved_patchset_id = resolve_patchset_argument(
            repo,
            &mut closeout_remote,
            patchset_id,
            None,
            Some(&repo_name),
            remote_name,
        )?;
        return review_request_with_closeout_remote(
            &mut closeout_remote,
            change_id,
            &resolved_patchset_id,
            reviewer_groups,
            note,
            &repo_name,
        );
    }
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    review_request_flow_with_task_and_closeout_remotes(
        &mut task_remote,
        &mut closeout_remote,
        change_id,
        None,
        reviewer_groups,
        note,
        &repo_name,
    )
}

pub(in crate::primitives) fn review_request_flow_with_task_and_closeout_remotes<T, C>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    change_id: &str,
    patchset_id: Option<&str>,
    reviewer_groups: &[String],
    note: Option<&str>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader + ?Sized,
    C: TaskWorkflowPatchsetReader
        + TaskWorkflowPatchsetLister
        + TaskWorkflowReviewRequester
        + ?Sized,
{
    let resolved_patchset_id = resolve_patchset_argument_with_task_and_closeout_remotes(
        task_remote,
        closeout_remote,
        patchset_id,
        Some(change_id),
        Some(repo_name),
    )?;
    review_request_with_closeout_remote(
        closeout_remote,
        change_id,
        &resolved_patchset_id,
        reviewer_groups,
        note,
        repo_name,
    )
}

pub(in crate::primitives) fn review_request_with_closeout_remote<R>(
    closeout_remote: &mut R,
    change_id: &str,
    patchset_id: &str,
    reviewer_groups: &[String],
    note: Option<&str>,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowReviewRequester + ?Sized,
{
    closeout_remote
        .request_review(
            change_id,
            patchset_id,
            reviewer_groups,
            note,
            Some(repo_name),
            false,
        )
        .map_err(|err| err.to_string())
}

pub fn review_task_approve(
    repo: &RepoRuntime,
    change_id: &str,
    patchset_id: Option<&str>,
    reviewer: Option<&str>,
    message: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let resolved_reviewer = repo.reviewer_identity(reviewer).ok_or_else(|| {
        "No reviewer identity available. Pass --reviewer or configure user_name/user_email."
            .to_string()
    })?;
    record_review(
        repo,
        change_id,
        patchset_id,
        &resolved_reviewer,
        "task_approve",
        normalized_text(message),
        false,
        remote_name,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "arguments mirror the review command contract"
)]
pub fn review_record(
    repo: &RepoRuntime,
    change_id: &str,
    action: &str,
    blocking: bool,
    patchset_id: Option<&str>,
    reviewer: Option<&str>,
    message: Option<&str>,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let resolved_action = normalized_text(Some(action))
        .ok_or_else(|| "Review action must be a non-empty string.".to_string())?;
    let resolved_reviewer = repo.reviewer_identity(reviewer).ok_or_else(|| {
        "No reviewer identity available. Pass --reviewer or configure user_name/user_email."
            .to_string()
    })?;
    record_review(
        repo,
        change_id,
        patchset_id,
        &resolved_reviewer,
        &resolved_action,
        normalized_text(message),
        blocking,
        remote_name,
    )
}

pub fn review_code_submit(
    repo: &RepoRuntime,
    change_id: &str,
    verdict: &str,
    patchset_id: Option<&str>,
    reviewer: Option<&str>,
    message: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let normalized_verdict = normalized_text(Some(verdict))
        .unwrap_or_else(|| "pass".to_string())
        .replace('_', "-")
        .to_lowercase();
    if !matches!(
        normalized_verdict.as_str(),
        "pass" | "request-changes" | "defer"
    ) {
        return Err("--verdict must be one of: pass, request-changes, defer.".to_string());
    }
    let missing = missing_code_review_summary_sections(message);
    if !missing.is_empty() {
        return Err(format!(
            "Code review summary is missing sections with non-placeholder content: {}.",
            missing.join(", ")
        ));
    }
    let resolved_reviewer = repo
        .ai_code_review_reviewer_identity(reviewer)
        .ok_or_else(|| {
            "No AI code review reviewer identity available. The signer must come from the executing agent basename; `--reviewer` and human reviewer config do not override that lane.".to_string()
        })?;
    let action = if normalized_verdict == "defer" {
        "code_review_defer"
    } else {
        "code_review_summary"
    };
    let blocking = normalized_verdict == "request-changes";
    record_review(
        repo,
        change_id,
        patchset_id,
        &resolved_reviewer,
        action,
        Some(message.to_string()),
        blocking,
        remote_name,
    )
}

pub fn review_code_template(style: Option<&str>) -> Result<JsonValue, String> {
    let normalized_style = normalized_text(style)
        .unwrap_or_else(|| "numbered".to_string())
        .to_ascii_lowercase();
    let template =
        match normalized_style.as_str() {
            "inline" => CODE_REVIEW_SUMMARY_TEMPLATE,
            "numbered" => CODE_REVIEW_SUMMARY_NUMBERED_TEMPLATE,
            _ => return Err(
                "Unknown code review summary template style. Expected one of: inline, numbered."
                    .to_string(),
            ),
        };
    Ok(json!({
        "style": normalized_style,
        "template": template,
        "hint_command": CODE_REVIEW_SUMMARY_TEMPLATE_HINT_COMMAND,
    }))
}

#[expect(
    clippy::too_many_arguments,
    reason = "review persistence keeps each policy field explicit"
)]
fn record_review(
    repo: &RepoRuntime,
    change_id: &str,
    patchset_id: Option<&str>,
    reviewer: &str,
    action: &str,
    comment: Option<String>,
    blocking: bool,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let (remote_row, repo_name) = remote_context(repo, remote_name, None)?;
    let mut closeout_remote = http_closeout_remote(repo, &remote_row)?;
    if patchset_id.is_some() {
        let resolved_patchset_id = resolve_patchset_argument(
            repo,
            &mut closeout_remote,
            patchset_id,
            None,
            Some(&repo_name),
            remote_name,
        )?;
        return review_record_with_closeout_remote(
            &mut closeout_remote,
            change_id,
            &resolved_patchset_id,
            reviewer,
            action,
            comment.as_deref(),
            blocking,
            &repo_name,
        );
    }
    let mut task_remote = http_task_remote(repo, &remote_row)?;
    review_record_flow_with_task_and_closeout_remotes(
        &mut task_remote,
        &mut closeout_remote,
        change_id,
        None,
        reviewer,
        action,
        comment.as_deref(),
        blocking,
        &repo_name,
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::primitives) fn review_record_flow_with_task_and_closeout_remotes<T, C>(
    task_remote: &mut T,
    closeout_remote: &mut C,
    change_id: &str,
    patchset_id: Option<&str>,
    reviewer: &str,
    action: &str,
    comment: Option<&str>,
    blocking: bool,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    T: TaskWorkflowRemoteChangeReader
        + TaskWorkflowRemoteChangeDetailReader
        + TaskWorkflowRemoteChangeLister
        + ?Sized,
    C: TaskWorkflowPatchsetReader
        + TaskWorkflowPatchsetLister
        + TaskWorkflowReviewRecorder
        + ?Sized,
{
    let resolved_patchset_id = resolve_patchset_argument_with_task_and_closeout_remotes(
        task_remote,
        closeout_remote,
        patchset_id,
        Some(change_id),
        Some(repo_name),
    )?;
    review_record_with_closeout_remote(
        closeout_remote,
        change_id,
        &resolved_patchset_id,
        reviewer,
        action,
        comment,
        blocking,
        repo_name,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "remote review persistence mirrors the closeout contract"
)]
pub(in crate::primitives) fn review_record_with_closeout_remote<R>(
    closeout_remote: &mut R,
    change_id: &str,
    patchset_id: &str,
    reviewer: &str,
    action: &str,
    comment: Option<&str>,
    blocking: bool,
    repo_name: &str,
) -> Result<JsonValue, String>
where
    R: TaskWorkflowReviewRecorder + ?Sized,
{
    closeout_remote
        .record_review(
            change_id,
            patchset_id,
            reviewer,
            action,
            comment,
            blocking,
            Some(repo_name),
            false,
        )
        .map_err(|err| err.to_string())
}
