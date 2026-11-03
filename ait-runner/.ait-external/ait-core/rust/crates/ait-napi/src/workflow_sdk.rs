//! Retained workflow facts, projections, presentation and direct primitives.
use crate::{remotes::RemoteMutationReceiptInput, repository::repository, JsonInput};
use ait_cli::primitives;
use ait_core::{
    json_support::JsonValue, task_workflow_ports_protocols as tracking,
    workflow_closeout_facts as facts, workflow_closeout_remote as receipts,
    workflow_closeout_views as views,
};
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn build_linked_task_lookup_payload(
    task_links_by_item_rows: Option<JsonInput>,
    tasks_by_plan_rows: Option<JsonInput>,
) -> Result<JsonValue> {
    tracking::build_linked_task_lookup_payload(
        task_links_by_item_rows.as_deref(),
        tasks_by_plan_rows.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn build_linked_change_lookup_payload(
    change_links_by_task_rows: Option<JsonInput>,
) -> Result<JsonValue> {
    tracking::build_linked_change_lookup_payload(change_links_by_task_rows.as_deref())
        .map_err(Error::from_reason)
}
#[napi]
pub fn build_task_tracking_title_payload(task: JsonInput) -> Result<JsonValue> {
    tracking::build_task_tracking_title_payload(&task).map_err(Error::from_reason)
}
#[napi]
pub fn build_task_tracking_metadata_payload(
    task: JsonInput,
    author_mode: String,
    tracking_policy: String,
) -> Result<JsonValue> {
    tracking::build_task_tracking_metadata_payload(&task, &author_mode, &tracking_policy)
        .map_err(Error::from_reason)
}
#[napi]
pub fn build_task_audit_verdict_payload(
    task: JsonInput,
    change_rows: JsonInput,
    target_line: String,
) -> Result<JsonValue> {
    ait_core::task_lifecycle::build_task_audit_verdict_payload(&task, &change_rows, &target_line)
        .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_landed_facts(state: JsonInput) -> Result<JsonValue> {
    facts::workflow_landed_facts(&state).map_err(Error::from_reason)
}
#[napi]
pub fn workflow_land_full_facts(state: JsonInput) -> Result<JsonValue> {
    facts::workflow_land_full_facts(&state).map_err(Error::from_reason)
}
#[napi]
pub fn workflow_ready_facts(state: JsonInput) -> Result<JsonValue> {
    facts::workflow_ready_facts(&state).map_err(Error::from_reason)
}
#[napi]
pub fn workflow_land_phase_facts(state: JsonInput, ready_state: JsonInput) -> Result<JsonValue> {
    facts::workflow_land_phase_facts(&state, &ready_state).map_err(Error::from_reason)
}
#[napi]
pub fn project_workflow_land_phase_read_model(
    state: JsonInput,
    ready_state: JsonInput,
) -> Result<JsonValue> {
    let facts =
        facts::workflow_land_phase_facts(&state, &ready_state).map_err(Error::from_reason)?;
    ait_core::workflow_closeout_read_model::project_workflow_land_phase_read_model(&facts)
        .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_remote_mutation_receipt(input: RemoteMutationReceiptInput) -> Result<JsonValue> {
    receipts::workflow_remote_mutation_receipt(
        &input.action,
        &input.source_action,
        &input.delivery,
        input.response_recovery.as_deref(),
        input.result.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_remote_action_mutation_receipts(
    code: String,
    result: JsonInput,
) -> Result<JsonValue> {
    receipts::workflow_remote_action_mutation_receipts(&code, &result).map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct ApplyPhaseOptions {
    pub detail: Option<String>,
    pub resumed_from_authoritative_state: Option<bool>,
}
#[napi]
pub fn workflow_apply_phase_payload(
    phase: String,
    code: String,
    options: Option<ApplyPhaseOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    views::workflow_apply_phase_payload(
        &phase,
        &code,
        options.detail.as_deref(),
        options.resumed_from_authoritative_state.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_apply_phase_summary(apply_phase: JsonInput) -> Result<Option<String>> {
    views::workflow_apply_phase_summary(&apply_phase).map_err(Error::from_reason)
}
#[napi]
pub fn workflow_mutation_receipt_summary(receipt: JsonInput) -> Result<String> {
    views::workflow_mutation_receipt_summary(&receipt).map_err(Error::from_reason)
}
#[napi]
pub fn workflow_applied_action_summary(action: JsonInput) -> Result<String> {
    views::workflow_applied_action_summary(&action).map_err(Error::from_reason)
}
#[napi]
pub fn workflow_ready_payload_direct(
    root: String,
    change_id: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    primitives::workflow_ready_payload(&repository(&root)?, &change_id, remote_name.as_deref())
        .map_err(Error::from_reason)
}
#[napi]
pub fn workflow_land_payload_direct(
    root: String,
    change_id: String,
    remote_name: Option<String>,
) -> Result<JsonValue> {
    primitives::workflow_land_payload(&repository(&root)?, &change_id, remote_name.as_deref())
        .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct WorkflowReadyDirectOptions {
    pub snapshot_message: Option<String>,
    pub summary: Option<String>,
    pub tests: Option<String>,
    pub lint: Option<String>,
    pub security: Option<String>,
    pub license: Option<String>,
    pub author_mode: Option<String>,
    pub model: Option<String>,
    pub remote_name: Option<String>,
}
#[napi]
pub fn workflow_ready_apply_direct(
    root: String,
    change_id: String,
    options: Option<WorkflowReadyDirectOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::workflow_ready_apply(
        &repository(&root)?,
        &change_id,
        options.snapshot_message.as_deref(),
        options.summary.as_deref(),
        options.tests.as_deref(),
        options.lint.as_deref(),
        options.security.as_deref(),
        options.license.as_deref(),
        options.author_mode.as_deref(),
        options.model.as_deref(),
        options.remote_name.as_deref(),
        None::<fn(&JsonValue) -> std::result::Result<(), String>>,
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct WorkflowLandDirectOptions {
    pub review_message: Option<String>,
    pub remote_name: Option<String>,
    pub reviewer: Option<String>,
}
#[napi]
pub fn workflow_land_apply_direct(
    root: String,
    change_id: String,
    options: Option<WorkflowLandDirectOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::workflow_finish_apply_guarded_with_reviewer(
        &repository(&root)?,
        &change_id,
        options.review_message.as_deref(),
        options.reviewer.as_deref(),
        options.remote_name.as_deref(),
        None,
        None::<fn(&JsonValue) -> std::result::Result<(), String>>,
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct WorkflowProjectionContext {
    pub root: Option<String>,
    pub config_path: Option<String>,
    pub worktree_config_path: Option<String>,
    pub workspace_dir: Option<String>,
    pub task_review: Option<bool>,
}
impl From<WorkflowProjectionContext> for ait_core::workflow_context_sdk::WorkflowContextData {
    fn from(value: WorkflowProjectionContext) -> Self {
        fn path(value: Option<String>) -> Option<std::path::PathBuf> {
            value
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .map(std::path::PathBuf::from)
        }
        Self {
            root: path(value.root),
            config_path: path(value.config_path),
            worktree_config_path: path(value.worktree_config_path),
            workspace_dir: path(value.workspace_dir),
            task_review: value.task_review,
        }
    }
}
#[napi]
pub fn project_workflow_landed_read_model(
    context: WorkflowProjectionContext,
    state: JsonInput,
) -> Result<JsonValue> {
    ait_core::workflow_context_sdk::project_workflow_landed_read_model_for_context(
        &ait_core::workflow_context_sdk::WorkflowContextData::from(context),
        &state,
    )
    .map_err(|error| Error::from_reason(error.to_string()))
}
#[napi]
pub fn project_workflow_land_full_read_model(
    context: WorkflowProjectionContext,
    state: JsonInput,
    apply_owned_continuation: bool,
) -> Result<JsonValue> {
    ait_core::workflow_context_sdk::project_workflow_land_full_read_model_for_context(
        &ait_core::workflow_context_sdk::WorkflowContextData::from(context),
        &state,
        apply_owned_continuation,
    )
    .map_err(|error| Error::from_reason(error.to_string()))
}
#[napi]
pub fn project_workflow_ready_read_model(
    context: WorkflowProjectionContext,
    state: JsonInput,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
    apply_owned_continuation: bool,
) -> Result<JsonValue> {
    ait_core::workflow_context_sdk::project_workflow_ready_read_model_for_context(
        &ait_core::workflow_context_sdk::WorkflowContextData::from(context),
        &state,
        ignore_workspace_authoring,
        patchset_is_authoritative,
        apply_owned_continuation,
    )
    .map_err(|error| Error::from_reason(error.to_string()))
}
