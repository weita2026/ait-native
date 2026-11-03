use crate::{repository::repository, workspace::optional_count, JsonInput};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct BlameOptions {
    pub line: Option<JsonInput>,
    pub start_line: Option<JsonInput>,
    pub end_line: Option<JsonInput>,
    pub snapshot_id: Option<String>,
    pub via_parent_snapshot_id: Option<String>,
    pub patchset_id: Option<String>,
    pub remote_name: Option<String>,
    pub plan_id: Option<String>,
    pub plan_ref: Option<String>,
}

#[napi]
pub fn task_workflow_blame(
    root: String,
    path: String,
    options: Option<BlameOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let request = ait_cli::blame_surface::BlameRequest {
        path,
        line: optional_count(options.line, "line")?,
        start_line: optional_count(options.start_line, "start_line")?,
        end_line: optional_count(options.end_line, "end_line")?,
        snapshot_id: options.snapshot_id,
        via_parent_snapshot_id: options.via_parent_snapshot_id,
        patchset_id: options.patchset_id,
        remote_name: options.remote_name,
        plan_id: options.plan_id,
        plan_ref: options.plan_ref,
    };
    ait_cli::blame_surface::blame(&repository(&root)?, &request).map_err(Error::from_reason)
}

#[napi]
pub fn workspace_diff(root: String, paths: Option<Vec<String>>) -> Result<JsonValue> {
    ait_cli::primitives::workspace_dirty_diff(
        &repository(&root)?,
        &paths.unwrap_or_default(),
        ait_core::object_diff::DEFAULT_SNAPSHOT_DIFF_MAX_BYTES,
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn format_blame(payload: JsonInput) -> String {
    ait_cli::binding_output::format_blame(&payload)
}

#[napi]
pub fn render_workspace_diff_names(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::render_workspace_diff_names(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn render_workspace_diff_stat(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::render_workspace_diff_stat(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn render_workspace_diff_text(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::render_workspace_diff_text(&payload).map_err(Error::from_reason)
}
