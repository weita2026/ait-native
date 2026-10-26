use crate::JsonInput;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn format_attest_put(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_attest_put(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_attest_show(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_attest_show(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_policy_eval(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_policy_eval(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_policy_waive(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_policy_waive(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_policy_show(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_policy_show(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn project_policy_show(payload: JsonInput) -> JsonValue {
    ait_cli::binding_output::project_policy_show(&payload)
}
