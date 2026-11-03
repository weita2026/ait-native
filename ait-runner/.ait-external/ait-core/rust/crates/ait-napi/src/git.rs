use crate::{repository::repository, JsonInput};
use ait_cli::binding_git as service;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;
use std::path::Path;

#[napi]
pub fn git_migrate(root: String, staging_root: String, apply: Option<bool>) -> Result<JsonValue> {
    service::migrate_git(
        &repository(&root)?,
        Path::new(&staging_root),
        apply.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn git_recover(root: String, apply: Option<bool>) -> Result<JsonValue> {
    service::recover_git(&repository(&root)?, apply.unwrap_or(false)).map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct GitImportOptions {
    pub all_branches_and_tags: Option<bool>,
    pub dry_run: Option<bool>,
}

#[napi]
pub fn git_import(
    root: String,
    source: String,
    options: Option<GitImportOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::import_git(
        &repository(&root)?,
        &source,
        options.all_branches_and_tags.unwrap_or(false),
        options.dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct GitExportOptions {
    pub all_lines_and_tags: Option<bool>,
    pub dry_run: Option<bool>,
    pub generation: Option<String>,
    pub git_object: Option<String>,
}

#[napi]
pub fn git_export(
    root: String,
    target: String,
    options: Option<GitExportOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    service::export_git(
        &repository(&root)?,
        &target,
        options.all_lines_and_tags.unwrap_or(false),
        options.dry_run.unwrap_or(false),
        options.generation.as_deref(),
        options.git_object.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn git_restore(
    root: String,
    generation: String,
    git_object: String,
    target: String,
) -> Result<JsonValue> {
    service::restore_git(
        &repository(&root)?,
        &generation,
        &git_object,
        Path::new(&target),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn git_mirror(
    root: String,
    endpoint: String,
    direction: String,
    dry_run: Option<bool>,
) -> Result<JsonValue> {
    service::mirror_git(
        &repository(&root)?,
        &endpoint,
        &direction,
        dry_run.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn format_git_migrate(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_git_migrate(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_git_recover(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_git_recover(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_git_import(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_git_import(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_git_export(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_git_export(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_git_restore(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_git_restore(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_git_mirror(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_git_mirror(&payload).map_err(Error::from_reason)
}
