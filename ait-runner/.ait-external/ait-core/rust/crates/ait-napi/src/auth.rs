use crate::{repository::repository, JsonInput};
use ait_cli::auth_surface as service;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct AuthRemoteOptions {
    pub remote_name: Option<String>,
    pub repo_name: Option<String>,
}

fn request(options: Option<AuthRemoteOptions>) -> service::AuthRemoteRequest {
    let options = options.unwrap_or_default();
    service::AuthRemoteRequest {
        remote_name: options.remote_name,
        repo_name: options.repo_name,
    }
}

#[napi]
pub fn task_workflow_auth_whoami(
    root: String,
    options: Option<AuthRemoteOptions>,
) -> Result<JsonValue> {
    service::auth_whoami(&repository(&root)?, &request(options)).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_auth_bindings(
    root: String,
    options: Option<AuthRemoteOptions>,
) -> Result<JsonValue> {
    service::auth_bindings(&repository(&root)?, &request(options)).map_err(Error::from_reason)
}

#[napi]
pub fn task_workflow_auth_grant(
    root: String,
    actor_identity: String,
    roles: Vec<String>,
    options: Option<AuthRemoteOptions>,
) -> Result<JsonValue> {
    let request = request(options);
    service::auth_grant(
        &repository(&root)?,
        &service::AuthGrantRequest {
            remote_name: request.remote_name,
            repo_name: request.repo_name,
            actor_identity,
            roles,
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn format_auth_whoami(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_auth_whoami(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_auth_grant(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_auth_grant(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_auth_bindings(payload: JsonInput) -> Result<String> {
    ait_cli::binding_output::format_auth_bindings(&payload).map_err(Error::from_reason)
}
