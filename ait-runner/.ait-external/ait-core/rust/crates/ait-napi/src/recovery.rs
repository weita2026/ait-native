use crate::JsonInput;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;
use std::path::Path;

#[napi(object)]
#[derive(Default)]
pub struct RemoteHeadRecoveryOptions {
    pub remote_name: Option<String>,
    pub jobs: Option<JsonInput>,
    pub apply: Option<bool>,
}
#[napi]
pub fn remote_recover_head(
    root: String,
    options: Option<RemoteHeadRecoveryOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let jobs = match options.jobs {
        None => 8,
        Some(value) => value
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=64).contains(n))
            .ok_or_else(|| Error::from_reason("jobs must be an integer from 1 through 64"))?,
    };
    ait_cli::binding_services::recover_remote_head(
        Path::new(&root),
        &ait_cli::remote_head_recovery::RemoteHeadRecoveryRequest {
            remote_name: options.remote_name,
            jobs,
            apply: options.apply.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn format_remote_recover_head(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_remote_recover_head(&payload).map_err(Error::from_reason)
}
