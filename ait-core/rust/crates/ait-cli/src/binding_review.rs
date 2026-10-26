//! Concrete native review operations, including exact package review selectors.
use crate::{primitives::*, runtime::RepoRuntime};
use ait_core::json_support::{json, JsonValue};

pub fn validate_public_patchset_argument(value: &str) -> Result<String, String> {
    let patchset_id = value.trim();
    if patchset_id.is_empty() {
        return Err("Patchset ID must be non-empty".into());
    }
    if patchset_id.chars().all(|ch| ch.is_ascii_digit()) {
        return Err("public Patchset reference required; use TASK_ID/P-##".into());
    }
    Ok(patchset_id.to_string())
}

pub fn validate_review_patchset_selector(value: &str) -> Result<String, String> {
    if let Some(path) = value.trim().strip_prefix('@') {
        if value.trim() != value || !std::path::Path::new(path).is_absolute() {
            return Err("Package --patchset selector requires @<absolute-path> without surrounding whitespace".into());
        }
        return Ok(value.to_string());
    }
    validate_public_patchset_argument(value)
}

#[derive(Clone, Debug, Default)]
pub struct TeamReviewOptions {
    pub patchset_id: Option<String>,
    pub reviewer: Option<String>,
    pub message: Option<String>,
    pub remote_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TaskReviewOptions {
    pub patchset_id: Option<String>,
    pub message: Option<String>,
    pub remote_name: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ReviewRequestOptions {
    pub patchset_id: Option<String>,
    pub note: Option<String>,
    pub remote_name: Option<String>,
}

fn require_team(repo: &RepoRuntime) -> Result<(), String> {
    if repo.team_review_enabled() {
        Ok(())
    } else {
        Err("`ait review team ...` is only available when Agent Teams is enabled.".into())
    }
}

pub fn inspect_review(
    repo: &RepoRuntime,
    reference: &str,
    remote_name: Option<&str>,
) -> Result<JsonValue, String> {
    let reference = resolve_task_remote_change_input(repo, reference, remote_name)?;
    review_show(repo, &reference, remote_name, None)
}

pub fn request_team_review(
    repo: &RepoRuntime,
    reference: &str,
    groups: &[String],
    options: &ReviewRequestOptions,
) -> Result<JsonValue, String> {
    require_team(repo)?;
    let remote = options.remote_name.as_deref();
    let (reference, patchset) = resolve_task_remote_revision_input(
        repo,
        reference,
        options.patchset_id.as_deref(),
        remote,
    )?;
    review_request(
        repo,
        &reference,
        Some(&patchset),
        groups,
        options.note.as_deref(),
        remote,
    )
}

pub fn approve_team_review(
    repo: &RepoRuntime,
    reference: &str,
    options: &TeamReviewOptions,
) -> Result<JsonValue, String> {
    require_team(repo)?;
    let remote = options.remote_name.as_deref();
    let (reference, patchset) = resolve_task_remote_revision_input(
        repo,
        reference,
        options.patchset_id.as_deref(),
        remote,
    )?;
    review_team_approve(
        repo,
        &reference,
        Some(&patchset),
        options.reviewer.as_deref(),
        options.message.as_deref(),
        remote,
    )
}

fn record_team_review(
    repo: &RepoRuntime,
    reference: &str,
    options: &TeamReviewOptions,
    action: &str,
    blocking: bool,
) -> Result<JsonValue, String> {
    require_team(repo)?;
    let remote = options.remote_name.as_deref();
    let (reference, patchset) = resolve_task_remote_revision_input(
        repo,
        reference,
        options.patchset_id.as_deref(),
        remote,
    )?;
    review_record(
        repo,
        &reference,
        action,
        blocking,
        Some(&patchset),
        options.reviewer.as_deref(),
        options.message.as_deref(),
        remote,
    )
}

pub fn request_team_changes(
    repo: &RepoRuntime,
    reference: &str,
    options: &TeamReviewOptions,
) -> Result<JsonValue, String> {
    record_team_review(repo, reference, options, "request_changes", true)
}

pub fn comment_team_review(
    repo: &RepoRuntime,
    reference: &str,
    options: &TeamReviewOptions,
) -> Result<JsonValue, String> {
    record_team_review(repo, reference, options, "comment", false)
}

pub fn defer_team_review(
    repo: &RepoRuntime,
    reference: &str,
    options: &TeamReviewOptions,
) -> Result<JsonValue, String> {
    record_team_review(repo, reference, options, "defer", false)
}

#[derive(Clone, Copy)]
enum TaskReviewAction {
    Approve,
    RequestChanges,
    Comment,
    Defer,
}

impl TaskReviewAction {
    fn name(self) -> &'static str {
        match self {
            Self::Approve => "task_approve",
            Self::RequestChanges => "task_request_changes",
            Self::Comment => "task_comment",
            Self::Defer => "task_defer",
        }
    }
}

fn task_review(
    repo: &RepoRuntime,
    reference: &str,
    selector: Option<&str>,
    message: Option<&str>,
    remote: Option<&str>,
    action: TaskReviewAction,
) -> Result<JsonValue, String> {
    if let Some(identity) = selector
        .map(crate::main_app::read_package_review_identity)
        .transpose()?
        .flatten()
    {
        let payload = workflow_package_task_review(
            repo,
            reference,
            &identity,
            action.name(),
            message,
            remote,
        )?;
        return Ok(json!({"payload":payload,"package_review":true}));
    }
    let (reference, patchset) =
        resolve_task_remote_revision_input(repo, reference, selector, remote)?;
    let payload = match action {
        TaskReviewAction::Approve => review_task_approve(
            repo,
            &reference,
            &patchset,
            message.ok_or("Task approval requires a message")?,
            remote,
        )?,
        _ => review_task_record(
            repo,
            &reference,
            action.name(),
            matches!(action, TaskReviewAction::RequestChanges),
            Some(&patchset),
            message,
            remote,
        )?,
    };
    Ok(json!({"payload":payload,"package_review":false}))
}

pub fn approve_task_review(
    repo: &RepoRuntime,
    reference: &str,
    patchset: &str,
    message: &str,
    remote: Option<&str>,
) -> Result<JsonValue, String> {
    task_review(
        repo,
        reference,
        Some(patchset),
        Some(message),
        remote,
        TaskReviewAction::Approve,
    )
}

pub fn request_task_changes(
    repo: &RepoRuntime,
    reference: &str,
    options: &TaskReviewOptions,
) -> Result<JsonValue, String> {
    task_review(
        repo,
        reference,
        options.patchset_id.as_deref(),
        options.message.as_deref(),
        options.remote_name.as_deref(),
        TaskReviewAction::RequestChanges,
    )
}

pub fn comment_task_review(
    repo: &RepoRuntime,
    reference: &str,
    options: &TaskReviewOptions,
) -> Result<JsonValue, String> {
    task_review(
        repo,
        reference,
        options.patchset_id.as_deref(),
        options.message.as_deref(),
        options.remote_name.as_deref(),
        TaskReviewAction::Comment,
    )
}

pub fn defer_task_review(
    repo: &RepoRuntime,
    reference: &str,
    options: &TaskReviewOptions,
) -> Result<JsonValue, String> {
    task_review(
        repo,
        reference,
        options.patchset_id.as_deref(),
        options.message.as_deref(),
        options.remote_name.as_deref(),
        TaskReviewAction::Defer,
    )
}

pub fn submit_code_review(
    repo: &RepoRuntime,
    reference: &str,
    patchset: &str,
    message: &str,
    reviewer: Option<&str>,
    remote: Option<&str>,
) -> Result<JsonValue, String> {
    let (reference, patchset) =
        resolve_task_remote_revision_input(repo, reference, Some(patchset), remote)?;
    review_code_submit_with_reviewer(repo, &reference, &patchset, message, reviewer, remote)
}
