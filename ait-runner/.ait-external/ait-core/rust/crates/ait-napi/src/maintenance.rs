use crate::{repository::repository, JsonInput};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn gc_stats(root: String) -> Result<JsonValue> {
    ait_cli::binding_maintenance::gc_stats(&repository(&root)?).map_err(Error::from_reason)
}
#[napi]
pub fn gc_validate(root: String) -> Result<JsonValue> {
    ait_cli::binding_maintenance::gc_validate(&repository(&root)?).map_err(Error::from_reason)
}
#[napi]
pub fn gc_validate_exit_code(payload: JsonInput) -> Result<u8> {
    ait_cli::binding_maintenance::gc_validate_exit_code(&payload).map_err(Error::from_reason)
}
#[napi]
pub fn gc_prune(root: String, apply: Option<bool>) -> Result<JsonValue> {
    ait_cli::binding_maintenance::gc_prune(&repository(&root)?, apply.unwrap_or(false))
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_gc_stats(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_gc_stats(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_gc_validate(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_gc_validate(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_gc_prune(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_gc_prune(&payload).map_err(Error::from_reason)
}
