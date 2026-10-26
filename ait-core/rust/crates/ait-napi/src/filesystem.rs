//! Existing Plan filesystem and repository-JSON SDK operations.
use crate::JsonInput;
use ait_core::{json_support::JsonValue, plan_filesystem as filesystem, repo_state_json as state};
use napi::{bindgen_prelude::Buffer, Error, Result};
use napi_derive::napi;

fn filesystem_error(error: filesystem::PlanFilesystemError) -> Error {
    match error {
        filesystem::PlanFilesystemError::Invalid(message)
        | filesystem::PlanFilesystemError::NotFound(message)
        | filesystem::PlanFilesystemError::MissingEntry(message)
        | filesystem::PlanFilesystemError::Io(message) => Error::from_reason(message),
    }
}

#[napi]
pub fn plan_filesystem_normalize_markdown_artifact_path(path_value: String) -> String {
    filesystem::normalize_markdown_artifact_path(&path_value)
}
#[napi]
pub fn plan_filesystem_is_markdown_artifact_path(path_value: String) -> bool {
    filesystem::is_markdown_artifact_path(&path_value)
}
#[napi]
pub fn plan_filesystem_is_lineage_only_markdown_artifact_path(path_value: String) -> bool {
    filesystem::is_lineage_only_markdown_artifact_path(&path_value)
}
#[napi]
pub fn plan_filesystem_path_is_projected_out_for_workspace(
    repo_root: String,
    rel_path: String,
    is_worktree: Option<bool>,
) -> bool {
    filesystem::path_is_projected_out_for_workspace(
        &repo_root,
        &rel_path,
        is_worktree.unwrap_or(false),
    )
}
#[napi]
pub fn plan_filesystem_workspace_path_is_ignored(
    repo_root: String,
    path_value: String,
    ignore_rules_text: Option<String>,
) -> Result<bool> {
    filesystem::workspace_path_is_ignored(&repo_root, &path_value, ignore_rules_text.as_deref())
        .map_err(filesystem_error)
}

#[napi(object)]
#[derive(Default)]
pub struct VisibleWorkspaceOptions {
    pub ignore_rules_text: Option<String>,
    pub runtime_root: Option<String>,
}

#[napi]
pub fn plan_filesystem_list_visible_workspace_paths(
    repo_root: String,
    options: Option<VisibleWorkspaceOptions>,
) -> Result<Vec<String>> {
    let options = options.unwrap_or_default();
    filesystem::list_visible_workspace_paths(
        &repo_root,
        options.ignore_rules_text.as_deref(),
        options.runtime_root.as_deref(),
    )
    .map_err(filesystem_error)
}

#[napi]
pub fn plan_filesystem_list_visible_markdown_artifact_paths(
    repo_root: String,
    options: Option<VisibleWorkspaceOptions>,
) -> Result<Vec<String>> {
    let options = options.unwrap_or_default();
    filesystem::list_visible_markdown_artifact_paths(
        &repo_root,
        options.ignore_rules_text.as_deref(),
        options.runtime_root.as_deref(),
    )
    .map_err(filesystem_error)
}

#[napi]
pub fn plan_filesystem_read_utf8_text_file(path_value: String) -> Result<String> {
    filesystem::read_utf8_text_file(&path_value).map_err(filesystem_error)
}

#[napi]
pub fn plan_filesystem_read_json_file(path_value: String) -> Result<JsonValue> {
    filesystem::read_json_file(&path_value).map_err(filesystem_error)
}

#[napi]
pub fn plan_filesystem_read_binary_file(path_value: String) -> Result<Buffer> {
    filesystem::read_binary_file(&path_value)
        .map(Buffer::from)
        .map_err(filesystem_error)
}
#[napi]
pub fn plan_filesystem_zip_archive_has_member(
    path_value: String,
    entry_name: String,
) -> Result<bool> {
    filesystem::zip_archive_has_member(&path_value, &entry_name).map_err(filesystem_error)
}
#[napi]
pub fn plan_filesystem_read_zip_archive_member(
    path_value: String,
    entry_name: String,
) -> Result<Buffer> {
    filesystem::read_zip_archive_member(&path_value, &entry_name)
        .map(Buffer::from)
        .map_err(filesystem_error)
}
#[napi]
pub fn plan_filesystem_resolve_repo_artifact_path(
    repo_root: String,
    path_value: String,
    allow_missing: Option<bool>,
) -> Result<JsonValue> {
    filesystem::resolve_repo_artifact_path(&repo_root, &path_value, allow_missing.unwrap_or(false))
        .map_err(filesystem_error)
}

#[napi]
pub fn repo_state_read_repo_config_json(path_value: String) -> Result<JsonValue> {
    state::read_repo_config_json_file(&path_value).map_err(Error::from_reason)
}
#[napi]
pub fn repo_state_write_repo_config_json(path_value: String, payload: JsonInput) -> Result<()> {
    state::write_repo_config_json_file(&path_value, &payload).map_err(Error::from_reason)
}

#[napi]
pub fn repo_state_read_worktree_config_json(path_value: String) -> Result<JsonValue> {
    state::read_worktree_config_json_file(&path_value).map_err(Error::from_reason)
}
#[napi]
pub fn repo_state_write_worktree_config_json(path_value: String, payload: JsonInput) -> Result<()> {
    state::write_worktree_config_json_file(&path_value, &payload).map_err(Error::from_reason)
}

#[napi]
pub fn repo_state_read_worktree_metadata_json(path_value: String) -> Result<JsonValue> {
    state::read_worktree_metadata_json_file(&path_value).map_err(Error::from_reason)
}
#[napi]
pub fn repo_state_write_worktree_metadata_json(
    path_value: String,
    payload: JsonInput,
) -> Result<()> {
    state::write_worktree_metadata_json_file(&path_value, &payload).map_err(Error::from_reason)
}
