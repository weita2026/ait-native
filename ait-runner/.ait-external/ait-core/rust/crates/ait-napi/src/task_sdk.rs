//! Retained lower-level Task SDK helpers, distinct from CLI finish orchestration.
use crate::{
    repository::{repository, ScopeOptions},
    workspace::native_count,
    JsonInput,
};
use ait_cli::primitives;
use ait_core::{json_support::JsonValue, task_workflow_shared_foundation as facts};
use napi::{bindgen_prelude::Buffer, Error, Result};
use napi_derive::napi;

fn i64_value(value: JsonInput, name: &str) -> Result<i64> {
    value
        .as_i64()
        .ok_or_else(|| Error::from_reason(format!("{name} must fit in i64.")))
}
#[napi]
pub fn task_workflow_timestamp_facts(now: Option<String>) -> Result<JsonValue> {
    facts::task_workflow_timestamp_facts(now.as_deref()).map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct SequenceFactsOptions {
    pub namespace_prefix: Option<String>,
    pub width: Option<JsonInput>,
}
#[napi]
pub fn task_workflow_sequence_identity_facts(
    family: String,
    number: JsonInput,
    options: Option<SequenceFactsOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    facts::task_workflow_sequence_identity_facts(
        &family,
        i64_value(number, "number")?,
        options.namespace_prefix.as_deref(),
        native_count(options.width, 4, "width")?,
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct WorkflowIdFactsOptions {
    pub namespace_prefix: Option<String>,
    pub timestamp_ms: Option<JsonInput>,
    pub randomness_hex: Option<String>,
}
#[napi]
pub fn task_workflow_workflow_id_facts(
    family: String,
    options: Option<WorkflowIdFactsOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    facts::task_workflow_workflow_id_facts(
        &family,
        options.namespace_prefix.as_deref(),
        options
            .timestamp_ms
            .map(|v| i64_value(v, "timestamp_ms"))
            .transpose()?,
        options.randomness_hex.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_runtime_selection_facts(overrides: Option<JsonInput>) -> Result<JsonValue> {
    facts::task_workflow_runtime_selection_facts(overrides.as_deref()).map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_read_utf8_text_file(path_value: String) -> Result<String> {
    facts::task_workflow_read_utf8_text_file(&path_value).map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_read_json_file(path_value: String) -> Result<JsonValue> {
    facts::task_workflow_read_json_file(&path_value).map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_read_binary_file(path_value: String) -> Result<Buffer> {
    facts::task_workflow_read_binary_file(&path_value)
        .map(Buffer::from)
        .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_resolve_repo_artifact_path(
    repo_root: String,
    path_value: String,
    allow_missing: Option<bool>,
) -> Result<JsonValue> {
    facts::task_workflow_resolve_repo_artifact_path(
        &repo_root,
        &path_value,
        allow_missing.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_remote_change_lineage_payload(
    base_line: String,
    line_row: Option<JsonInput>,
) -> Result<JsonValue> {
    ait_core::task_remote::task_remote_change_lineage_payload(&base_line, line_row.as_deref())
        .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_resolve_task_worktree_location(
    root: String,
    worktree_name: String,
    debug_probe_override: Option<JsonInput>,
) -> Result<JsonValue> {
    primitives::task_resolve_worktree_location(
        &repository(&root)?,
        &worktree_name,
        debug_probe_override.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_resolve_main_seed_mirror_location(
    root: String,
    seed_name: String,
) -> Result<JsonValue> {
    primitives::task_resolve_main_seed_mirror_location(&repository(&root)?, &seed_name)
        .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct EnsureSeedMirrorOptions {
    pub force_refresh: Option<bool>,
    pub line_name: Option<String>,
}
#[napi]
pub fn task_workflow_ensure_main_seed_mirror(
    root: String,
    options: Option<EnsureSeedMirrorOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::task_ensure_main_seed_mirror(
        &repository(&root)?,
        options.force_refresh.unwrap_or(false),
        options.line_name.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_ensure_status_manifest(
    root: String,
    snapshot_id: String,
) -> Result<JsonValue> {
    primitives::ensure_status_manifest(&repository(&root)?, &snapshot_id)
        .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_queue_summary(root: String, remote_name: Option<String>) -> Result<JsonValue> {
    primitives::queue_summary(&repository(&root)?, remote_name.as_deref())
        .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_task_audit(
    root: String,
    task_id: String,
    options: Option<ScopeOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::task_audit(
        &repository(&root)?,
        &task_id,
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_task_abandon(
    root: String,
    task_id: String,
    options: Option<ScopeOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::task_abandon(
        &repository(&root)?,
        &task_id,
        options.local.unwrap_or(false),
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_task_land(
    root: String,
    task_or_change_id: String,
    options: Option<ScopeOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let repo = repository(&root)?;
    let local = repo
        .task_uses_local_scope(
            options.local.unwrap_or(false),
            options.remote_name.as_deref(),
        )
        .map_err(Error::from_reason)?;
    primitives::task_land_apply_scoped(
        &repo,
        &task_or_change_id,
        local,
        options.remote_name.as_deref(),
        None,
        None::<fn(&JsonValue) -> std::result::Result<(), String>>,
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
pub struct LandSubmitOptions {
    pub patchset_id: Option<String>,
    pub target_line: String,
    pub mode: String,
    pub remote_name: Option<String>,
}
#[napi]
pub fn task_workflow_land_submit(
    root: String,
    change_id: String,
    options: LandSubmitOptions,
) -> Result<JsonValue> {
    primitives::land_submit(
        &repository(&root)?,
        &change_id,
        options.patchset_id.as_deref(),
        &options.target_line,
        &options.mode,
        options.remote_name.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi(object)]
#[derive(Default)]
pub struct LandReceiptOptions {
    pub reason: Option<String>,
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}
#[napi]
pub fn task_workflow_land_show(
    root: String,
    submission_id: String,
    options: Option<LandReceiptOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::land_show(
        &repository(&root)?,
        &submission_id,
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn task_workflow_land_retry(
    root: String,
    submission_id: String,
    options: Option<LandReceiptOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    primitives::land_retry(
        &repository(&root)?,
        &submission_id,
        options.reason.as_deref(),
        options.remote_name.as_deref(),
        options.repo_name.as_deref(),
    )
    .map_err(Error::from_reason)
}
