use crate::{repository::repository, JsonInput};
use ait_cli::binding_review as service;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn validate_review_patchset_selector(value: String) -> Result<String> {
    service::validate_review_patchset_selector(&value).map_err(Error::from_reason)
}

#[napi]
pub fn validate_public_patchset_argument(value: String) -> Result<String> {
    service::validate_public_patchset_argument(&value).map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct TeamReviewOptions {
    pub patchset_id: Option<String>,
    pub reviewer: Option<String>,
    pub message: Option<String>,
    pub remote_name: Option<String>,
}

fn teamreview(options: Option<TeamReviewOptions>) -> service::TeamReviewOptions {
    let options = options.unwrap_or_default();
    service::TeamReviewOptions {
        patchset_id: options.patchset_id,
        reviewer: options.reviewer,
        message: options.message,
        remote_name: options.remote_name,
    }
}

#[napi(object)]
#[derive(Default)]
pub struct TaskReviewOptions {
    pub patchset_id: Option<String>,
    pub message: Option<String>,
    pub remote_name: Option<String>,
}

fn taskreview(options: Option<TaskReviewOptions>) -> service::TaskReviewOptions {
    let options = options.unwrap_or_default();
    service::TaskReviewOptions {
        patchset_id: options.patchset_id,
        message: options.message,
        remote_name: options.remote_name,
    }
}

#[napi(object)]
#[derive(Default)]
pub struct ReviewRequestOptions {
    pub patchset_id: Option<String>,
    pub note: Option<String>,
    pub remote_name: Option<String>,
}

fn reviewrequest(options: Option<ReviewRequestOptions>) -> service::ReviewRequestOptions {
    let options = options.unwrap_or_default();
    service::ReviewRequestOptions {
        patchset_id: options.patchset_id,
        note: options.note,
        remote_name: options.remote_name,
    }
}

#[napi(object)]
#[derive(Default)]
pub struct CodeReviewOptions {
    pub reviewer: Option<String>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn review_show(
    root: String,
    reference: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    service::inspect_review(&repository(&root)?, &reference, remote_name.as_deref())
        .map_err(Error::from_reason)
}
#[napi]
pub fn review_team_request(
    root: String,
    reference: String,
    reviewer_groups: Vec<String>,
    options: Option<ReviewRequestOptions>,
) -> Result<JsonValue> {
    service::request_team_review(
        &repository(&root)?,
        &reference,
        &reviewer_groups,
        &reviewrequest(options),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn review_task_approve(
    root: String,
    reference: String,
    patchset_id: String,
    message: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    service::approve_task_review(
        &repository(&root)?,
        &reference,
        &patchset_id,
        &message,
        remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn review_code_submit(
    root: String,
    reference: String,
    patchset_id: String,
    message: String,
    options: Option<CodeReviewOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::submit_code_review(
        &repository(&root)?,
        &reference,
        &patchset_id,
        &message,
        options.reviewer.as_deref(),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn review_team_approve(
    root: String,
    reference: String,
    options: Option<TeamReviewOptions>,
) -> Result<JsonValue> {
    service::approve_team_review(&repository(&root)?, &reference, &teamreview(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn review_team_request_changes(
    root: String,
    reference: String,
    options: Option<TeamReviewOptions>,
) -> Result<JsonValue> {
    service::request_team_changes(&repository(&root)?, &reference, &teamreview(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn review_team_comment(
    root: String,
    reference: String,
    options: Option<TeamReviewOptions>,
) -> Result<JsonValue> {
    service::comment_team_review(&repository(&root)?, &reference, &teamreview(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn review_team_defer(
    root: String,
    reference: String,
    options: Option<TeamReviewOptions>,
) -> Result<JsonValue> {
    service::defer_team_review(&repository(&root)?, &reference, &teamreview(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn review_task_request_changes(
    root: String,
    reference: String,
    options: Option<TaskReviewOptions>,
) -> Result<JsonValue> {
    service::request_task_changes(&repository(&root)?, &reference, &taskreview(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn review_task_comment(
    root: String,
    reference: String,
    options: Option<TaskReviewOptions>,
) -> Result<JsonValue> {
    service::comment_task_review(&repository(&root)?, &reference, &taskreview(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn review_task_defer(
    root: String,
    reference: String,
    options: Option<TaskReviewOptions>,
) -> Result<JsonValue> {
    service::defer_task_review(&repository(&root)?, &reference, &taskreview(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_review_team_request(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_team_request(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_team_approve(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_team_approve(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_team_request_changes(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_team_request_changes(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_review_team_comment(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_team_comment(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_team_defer(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_team_defer(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_task_approve(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_task_approve(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_task_request_changes(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_task_request_changes(&payload)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_review_task_comment(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_task_comment(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_task_defer(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_task_defer(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_show(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_show(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_code_submit(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_code_submit(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_review_code_template(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_review_code_template(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn project_review_show(payload: JsonInput) -> JsonValue {
    ait_cli::binding_output::project_review_show(&payload)
}
#[napi]
pub fn project_review_task_operation(operation: JsonInput) -> Result<JsonValue> {
    ait_cli::binding_output::project_review_task_operation(&operation).map_err(Error::from_reason)
}
