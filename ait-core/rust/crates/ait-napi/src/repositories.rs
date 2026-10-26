use crate::{repository::repository, JsonInput};
use ait_cli::{repo_surface as service, repository_retirement as retirement};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct RepositoryJobsOptions {
    pub remote_name: Option<String>,
    pub worker_job_index: Option<JsonInput>,
    pub state: Option<String>,
    pub limit: Option<JsonInput>,
}

fn unsigned(value: Option<JsonInput>, field: &str) -> Result<Option<u32>> {
    value
        .map(|value| {
            value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    Error::from_reason(format!("{field} must be an unsigned 32-bit integer."))
                })
        })
        .transpose()
}

fn jobs_request(options: Option<RepositoryJobsOptions>) -> Result<service::RepositoryJobsRequest> {
    let options = options.unwrap_or_default();
    Ok(service::RepositoryJobsRequest {
        remote_name: options.remote_name,
        worker_job_index: unsigned(options.worker_job_index, "worker_job_index")?,
        state: options.state,
        limit: unsigned(options.limit, "limit")?,
    })
}

#[napi]
pub fn repository_show(root: String, remote_name: Option<String>) -> Result<JsonValue> {
    service::inspect_repository(&repository(&root)?, remote_name.as_deref())
        .map_err(Error::from_reason)
}

#[napi]
pub fn repository_jobs(root: String, options: Option<RepositoryJobsOptions>) -> Result<JsonValue> {
    service::repository_jobs(&repository(&root)?, &jobs_request(options)?)
        .map_err(Error::from_reason)
}

#[napi]
pub fn repository_ci_capabilities(root: String, remote_name: Option<String>) -> Result<JsonValue> {
    service::repository_ci_capabilities(&repository(&root)?, remote_name.as_deref())
        .map_err(Error::from_reason)
}

#[napi]
pub fn repository_retire(
    root: String,
    remote_name: Option<String>,
    abort: Option<bool>,
) -> Result<JsonValue> {
    retirement::retire_repository(
        &repository(&root)?,
        &retirement::RepoRetireRequest {
            remote_name,
            abort: abort.unwrap_or(false),
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn repository_restore(root: String, remote_name: Option<String>) -> Result<JsonValue> {
    retirement::restore_repository(
        &repository(&root)?,
        &retirement::RepoRestoreRequest { remote_name },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn format_repository_show(payload: JsonInput, remote_name: Option<String>) -> String {
    service::format_repository_show(&payload, remote_name.as_deref())
}

#[napi]
pub fn format_repository_jobs(
    payload: JsonInput,
    options: Option<RepositoryJobsOptions>,
) -> Result<String> {
    service::format_repository_jobs(&payload, &jobs_request(options)?).map_err(Error::from_reason)
}

#[napi]
pub fn format_repository_ci_capabilities(
    payload: JsonInput,
    remote_name: Option<String>,
) -> String {
    service::format_repository_ci_capabilities(&payload, remote_name.as_deref())
}

#[napi]
pub fn format_repository_retire(payload: JsonInput) -> String {
    service::format_repository_retire(&payload)
}

#[napi]
pub fn format_repository_restore(payload: JsonInput) -> String {
    service::format_repository_restore(&payload)
}
