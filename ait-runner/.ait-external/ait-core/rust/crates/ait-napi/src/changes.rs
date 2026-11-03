//! Concrete counterparts of retained PyO3 Change, Patchset and governance APIs.
use crate::repository::repository;
use ait_cli::binding_changes as operations;

#[napi]
pub fn project_change_list(payload: crate::JsonInput, all: Option<bool>) -> JsonValue {
    ait_cli::binding_output::project_change_list(&payload, all.unwrap_or(false))
}

#[napi]
pub fn format_change_list(
    payload: crate::JsonInput,
    options: Option<crate::output::TaskListOutputOptions>,
) -> Result<String> {
    let options = options.unwrap_or_default();
    ait_cli::binding_output::format_change_list(
        &payload,
        options.all.unwrap_or(false),
        &ait_cli::binding_services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
    )
    .map_err(Error::from_reason)
}
use ait_cli::primitives;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct ChangeCreateOptions {
    pub base_line: Option<String>,
    pub fork_snapshot: Option<String>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
}

fn scope(options: Option<crate::repository::ScopeOptions>) -> operations::ChangeScope {
    let options = options.unwrap_or_default();
    operations::ChangeScope {
        local: options.local.unwrap_or(false),
        remote_name: options.remote_name,
    }
}

#[napi]
pub fn change_create(
    root: String,
    task_id: String,
    title: String,
    options: Option<ChangeCreateOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    operations::create_change(
        &repository(&root)?,
        &operations::ChangeCreateRequest {
            task_id,
            title,
            base_line: options.base_line,
            fork_snapshot: options.fork_snapshot,
            scope: operations::ChangeScope {
                local: options.local.unwrap_or(false),
                remote_name: options.remote_name,
            },
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn change_show(
    root: String,
    reference: String,
    options: Option<crate::repository::ScopeOptions>,
) -> Result<JsonValue> {
    operations::inspect_change(&repository(&root)?, &reference, &scope(options))
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct ChangeMutationOptions {
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
}

fn mutation(options: Option<ChangeMutationOptions>) -> operations::ChangeMutationOptions {
    let options = options.unwrap_or_default();
    operations::ChangeMutationOptions {
        force: options.force.unwrap_or(false),
        dry_run: options.dry_run.unwrap_or(false),
        scope: operations::ChangeScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
    }
}

#[napi]
pub fn change_revert(
    root: String,
    reference: String,
    options: Option<ChangeMutationOptions>,
) -> Result<JsonValue> {
    operations::revert_change(&repository(&root)?, &reference, &mutation(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn change_replay(
    root: String,
    reference: String,
    onto: Option<String>,
    options: Option<ChangeMutationOptions>,
) -> Result<JsonValue> {
    operations::replay_change(
        &repository(&root)?,
        &reference,
        onto.as_deref(),
        &mutation(options),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn change_close(
    root: String,
    reference: String,
    options: Option<crate::repository::ScopeOptions>,
) -> Result<JsonValue> {
    operations::close_change(&repository(&root)?, &reference, &scope(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn change_publish(
    root: String,
    reference: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    operations::publish_change(&repository(&root)?, &reference, remote_name.as_deref())
        .map_err(Error::from_reason)
}

#[napi]
pub fn patchset_publish(
    root: String,
    reference: String,
    summary: String,
    options: Option<TaskWorkflowPatchsetPublishOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    operations::publish_patchset(
        &repository(&root)?,
        &reference,
        &summary,
        options.author_mode.as_deref(),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn patchset_list(
    root: String,
    reference: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    operations::list_patchsets(&repository(&root)?, &reference, remote_name.as_deref())
        .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowChangeListOptions {
    pub local: Option<bool>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_change_list(
    root: String,
    options: Option<TaskWorkflowChangeListOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::change_list(
        &repository(&root)?,
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowChangeShowOptions {
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[napi]
pub fn task_workflow_change_show(
    root: String,
    change_id: String,
    options: Option<TaskWorkflowChangeShowOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::change_show(
        &repository(&root)?,
        &change_id,
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowChangeRevertOptions {
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[napi]
pub fn task_workflow_change_revert(
    root: String,
    change_id: String,
    options: Option<TaskWorkflowChangeRevertOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::change_revert(
        &repository(&root)?,
        &change_id,
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowChangeReplayOptions {
    pub force: Option<bool>,
    pub dry_run: Option<bool>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[napi]
pub fn task_workflow_change_replay(
    root: String,
    change_id: String,
    onto: String,
    options: Option<TaskWorkflowChangeReplayOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::change_replay(
        &repository(&root)?,
        &change_id,
        &onto,
        options.force.unwrap_or(false),
        options.dry_run.unwrap_or(false),
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowChangeCloseOptions {
    pub local: Option<bool>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_change_close(
    root: String,
    change_id: String,
    options: Option<TaskWorkflowChangeCloseOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::change_close(
        &repository(&root)?,
        &change_id,
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowChangePublishOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_change_publish(
    root: String,
    change_id: String,
    options: Option<TaskWorkflowChangePublishOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::change_publish(
        &repository(&root)?,
        &change_id,
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPatchsetPublishOptions {
    pub author_mode: Option<String>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_patchset_publish(
    root: String,
    change_id: String,
    summary: String,
    options: Option<TaskWorkflowPatchsetPublishOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::patchset_publish(
        &repository(&root)?,
        &change_id,
        &summary,
        options.author_mode.as_deref(),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPatchsetListOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_patchset_list(
    root: String,
    change_id: String,
    options: Option<TaskWorkflowPatchsetListOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::patchset_list(
        &repository(&root)?,
        &change_id,
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPatchsetShowOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_patchset_show(
    root: String,
    patchset_id: String,
    options: Option<TaskWorkflowPatchsetShowOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::patchset_show(
        &repository(&root)?,
        &patchset_id,
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPatchsetSelectOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_patchset_select(
    root: String,
    patchset_id: String,
    options: Option<TaskWorkflowPatchsetSelectOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::patchset_select(
        &repository(&root)?,
        &patchset_id,
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPatchsetPublishExplicitOptions {
    pub author_mode: Option<String>,
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[napi]
pub fn task_workflow_patchset_publish_explicit(
    root: String,
    change_id: String,
    base_snapshot_id: String,
    revision_snapshot_id: String,
    summary: String,
    options: Option<TaskWorkflowPatchsetPublishExplicitOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::patchset_publish_explicit(
        &repository(&root)?,
        &change_id,
        &base_snapshot_id,
        &revision_snapshot_id,
        &summary,
        options.author_mode.as_deref(),
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPatchsetCiStatusOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_patchset_ci_status(
    root: String,
    patchset_id: String,
    options: Option<TaskWorkflowPatchsetCiStatusOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::patchset_ci_status(
        &repository(&root)?,
        &patchset_id,
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPatchsetRerunCiOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_patchset_rerun_ci(
    root: String,
    patchset_id: String,
    options: Option<TaskWorkflowPatchsetRerunCiOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::patchset_rerun_ci(
        &repository(&root)?,
        &patchset_id,
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowReviewTeamApproveOptions {
    pub patchset_id: Option<String>,
    pub reviewer: Option<String>,
    pub message: Option<String>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_review_team_approve(
    root: String,
    change_id: String,
    options: Option<TaskWorkflowReviewTeamApproveOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::review_team_approve(
        &repository(&root)?,
        &change_id,
        options.patchset_id.as_deref(),
        options.reviewer.as_deref(),
        options.message.as_deref(),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowReviewShowOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_review_show(
    root: String,
    change_id: String,
    options: Option<TaskWorkflowReviewShowOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::review_show(
        &repository(&root)?,
        &change_id,
        options.remote_name.as_deref(),
        None,
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowReviewRequestOptions {
    pub patchset_id: Option<String>,
    pub note: Option<String>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_review_request(
    root: String,
    change_id: String,
    reviewer_groups: Vec<String>,
    options: Option<TaskWorkflowReviewRequestOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::review_request(
        &repository(&root)?,
        &change_id,
        options.patchset_id.as_deref(),
        &reviewer_groups,
        options.note.as_deref(),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowReviewTaskApproveOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_review_task_approve(
    root: String,
    change_id: String,
    patchset_id: String,
    message: String,
    options: Option<TaskWorkflowReviewTaskApproveOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::review_task_approve(
        &repository(&root)?,
        &change_id,
        &patchset_id,
        &message,
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowReviewRecordOptions {
    pub blocking: Option<bool>,
    pub patchset_id: Option<String>,
    pub reviewer: Option<String>,
    pub message: Option<String>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_review_record(
    root: String,
    change_id: String,
    action: String,
    options: Option<TaskWorkflowReviewRecordOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::review_record(
        &repository(&root)?,
        &change_id,
        &action,
        options.blocking.unwrap_or(false),
        options.patchset_id.as_deref(),
        options.reviewer.as_deref(),
        options.message.as_deref(),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowReviewCodeSubmitOptions {
    pub remote_name: Option<String>,
    pub reviewer: Option<String>,
}

#[napi]
pub fn task_workflow_review_code_submit(
    root: String,
    change_id: String,
    patchset_id: String,
    message: String,
    options: Option<TaskWorkflowReviewCodeSubmitOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::review_code_submit_with_reviewer(
        &repository(&root)?,
        &change_id,
        &patchset_id,
        &message,
        options.reviewer.as_deref(),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowReviewCodeTemplateOptions {
    pub style: Option<String>,
}

#[napi]
pub fn task_workflow_review_code_template(
    options: Option<TaskWorkflowReviewCodeTemplateOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::review_code_template(options.style.as_deref()).map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowAttestPutOptions {
    pub patchset_id: Option<String>,
    pub change_id: Option<String>,
    pub tests: Option<String>,
    pub lint: Option<String>,
    pub security: Option<String>,
    pub license: Option<String>,
    pub author_mode: Option<String>,
    pub model: Option<String>,
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[napi]
pub fn task_workflow_attest_put(
    root: String,
    options: Option<TaskWorkflowAttestPutOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::attest_put(
        &repository(&root)?,
        options.patchset_id.as_deref(),
        options.change_id.as_deref(),
        options.tests.as_deref(),
        options.lint.as_deref(),
        options.security.as_deref(),
        options.license.as_deref(),
        options.author_mode.as_deref(),
        options.model.as_deref(),
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowAttestShowOptions {
    pub remote_name: Option<String>,
}

#[napi]
pub fn task_workflow_attest_show(
    root: String,
    patchset_id: String,
    options: Option<TaskWorkflowAttestShowOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::attest_show(
        &repository(&root)?,
        &patchset_id,
        options.remote_name.as_deref(),
        None,
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPolicyEvalOptions {
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[napi]
pub fn task_workflow_policy_eval(
    root: String,
    patchset_id: String,
    options: Option<TaskWorkflowPolicyEvalOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::policy_eval(
        &repository(&root)?,
        &patchset_id,
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPolicyShowOptions {
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[napi]
pub fn task_workflow_policy_show(
    root: String,
    patchset_id: String,
    options: Option<TaskWorkflowPolicyShowOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::policy_show(
        &repository(&root)?,
        &patchset_id,
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TaskWorkflowPolicyWaiveOptions {
    pub expires_at: Option<String>,
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

#[napi]
pub fn task_workflow_policy_waive(
    root: String,
    patchset_id: String,
    rule_name: String,
    reason: String,
    options: Option<TaskWorkflowPolicyWaiveOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::policy_waive(
        &repository(&root)?,
        &patchset_id,
        &rule_name,
        &reason,
        options.expires_at.as_deref(),
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

macro_rules! change_text_output {
    ($(($name:ident, $target:path)),* $(,)?) => {$(
        #[napi]
        pub fn $name(payload: crate::JsonInput) -> Result<String> { $target(&payload).map_err(Error::from_reason) }
    )*};
}
change_text_output! {
    (format_patchset_show, ait_cli::binding_output::format_patchset_show),
    (format_patchset_ci_status, ait_cli::binding_output::format_patchset_ci_status),
    (format_patchset_list, ait_cli::binding_output::format_patchset_list),
    (format_change_create, ait_cli::binding_output::format_change_create),
    (format_change_show, ait_cli::binding_output::format_change_show),
    (format_change_revert, ait_cli::binding_output::format_change_revert),
    (format_change_replay, ait_cli::binding_output::format_change_replay),
    (format_change_close, ait_cli::binding_output::format_change_close),
    (format_change_publish, ait_cli::binding_output::format_change_publish),
    (format_patchset_publish, ait_cli::binding_output::format_patchset_publish),
    (format_patchset_select, ait_cli::binding_output::format_patchset_select),
    (format_patchset_rerun_ci, ait_cli::binding_output::format_patchset_rerun_ci),
}
