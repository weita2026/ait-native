use crate::JsonInput;
use ait_cli::binding_binary_db as service;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;
use std::path::PathBuf;

#[napi]
pub fn binary_db_agent_pool_50(
    repo_root: String,
    generation_root: String,
    activate: Option<bool>,
) -> Result<JsonValue> {
    service::convert_agent_pool_50(
        PathBuf::from(repo_root),
        PathBuf::from(generation_root),
        activate.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn binary_db_upgrade_u64_seconds(
    repo_root: String,
    generation_root: String,
    source_time_width: String,
    activate: Option<bool>,
) -> Result<JsonValue> {
    service::upgrade_u64_seconds(
        PathBuf::from(repo_root),
        PathBuf::from(generation_root),
        source_time_width,
        activate.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn format_binary_db_agent_pool_50(payload: JsonInput) -> String {
    service::format_agent_pool_50(&payload)
}
#[napi]
pub fn format_binary_db_upgrade_u64_seconds(payload: JsonInput) -> String {
    service::format_upgrade_u64_seconds(&payload)
}
