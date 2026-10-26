use crate::repository::{repository, ScopeOptions};
use crate::JsonInput;
use ait_cli::binding_services as services;
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;
use std::path::PathBuf;

#[napi(object)]
#[derive(Default)]
pub struct PlanQueryOptions {
    pub revision: Option<String>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
}

#[napi(object)]
#[derive(Default)]
pub struct PlanCandidatesOptions {
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub include_all: Option<bool>,
    pub contains: Option<String>,
}

#[napi(object)]
#[derive(Default)]
pub struct PlanSyncOptions {
    pub plan_ref: Option<String>,
    pub prune: Option<bool>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
    pub reconcile: Option<bool>,
}

fn scope(options: Option<ScopeOptions>) -> services::PlanScope {
    let options = options.unwrap_or_default();
    services::PlanScope {
        local: options.local.unwrap_or(false),
        remote_name: options.remote_name,
    }
}

fn selection(plan_id: String, options: Option<PlanQueryOptions>) -> services::PlanSelection {
    let options = options.unwrap_or_default();
    services::PlanSelection {
        plan_id,
        revision: options.revision,
        scope: services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
    }
}

#[napi]
pub fn plan_list(root: String, options: Option<ScopeOptions>) -> Result<JsonValue> {
    services::list_plans(&repository(&root)?, &scope(options)).map_err(Error::from_reason)
}

#[napi]
pub fn plan_show(
    root: String,
    plan_id: String,
    options: Option<PlanQueryOptions>,
) -> Result<JsonValue> {
    services::show_plan(&repository(&root)?, &selection(plan_id, options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn plan_revisions(
    root: String,
    plan_id: String,
    options: Option<ScopeOptions>,
) -> Result<JsonValue> {
    services::plan_revisions(&repository(&root)?, &plan_id, &scope(options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn plan_items(
    root: String,
    plan_id: String,
    options: Option<PlanQueryOptions>,
) -> Result<JsonValue> {
    services::plan_items(&repository(&root)?, &selection(plan_id, options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn plan_inspect(
    root: String,
    plan_id: String,
    options: Option<PlanQueryOptions>,
) -> Result<JsonValue> {
    services::inspect_plan(&repository(&root)?, &selection(plan_id, options))
        .map_err(Error::from_reason)
}

#[napi]
pub fn plan_candidates(root: String, options: Option<PlanCandidatesOptions>) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let request = services::PlanCandidatesRequest {
        scope: services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
        include_all: options.include_all.unwrap_or(false),
        contains: options.contains,
    };
    services::plan_candidates(&repository(&root)?, &request).map_err(Error::from_reason)
}

#[napi]
pub fn plan_sync(
    root: String,
    target: String,
    options: Option<PlanSyncOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let request = services::PlanSyncRequest {
        target: PathBuf::from(target),
        plan_ref: options.plan_ref,
        prune: options.prune.unwrap_or(false),
        scope: services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
        reconcile: options.reconcile.unwrap_or(false),
    };
    services::sync_plan(&repository(&root)?, &request).map_err(Error::from_reason)
}

#[napi]
pub fn plan_sync_error(payload: JsonInput) -> Option<String> {
    ait_cli::binding_output::plan_sync_error(&payload)
}

#[napi(object)]
#[derive(Default)]
pub struct PlanListOutputOptions {
    pub all: Option<bool>,
    pub local: Option<bool>,
    pub remote_name: Option<String>,
}

#[napi]
pub fn format_plan_list(
    payload: JsonInput,
    options: Option<PlanListOutputOptions>,
) -> Result<String> {
    let options = options.unwrap_or_default();
    ait_cli::binding_output::format_plan_list(
        &payload,
        options.all.unwrap_or(false),
        &services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn format_plan_revisions(
    payload: JsonInput,
    plan_id: String,
    options: Option<PlanListOutputOptions>,
) -> Result<String> {
    let options = options.unwrap_or_default();
    ait_cli::binding_output::format_plan_revisions(
        &payload,
        &plan_id,
        options.all.unwrap_or(false),
        &services::PlanScope {
            local: options.local.unwrap_or(false),
            remote_name: options.remote_name,
        },
    )
    .map_err(Error::from_reason)
}

macro_rules! plan_text {
    ($(($name:ident, $target:path)),* $(,)?) => {$(
        #[napi]
        pub fn $name(payload: JsonInput) -> Result<String> { $target(&payload).map_err(Error::from_reason) }
    )*};
}

plan_text! {
    (format_plan_show, ait_cli::binding_output::format_plan_show),
    (format_plan_items, ait_cli::binding_output::format_plan_items),
    (format_plan_candidates, ait_cli::binding_output::format_plan_candidates),
    (format_plan_inspect, ait_cli::binding_output::format_plan_inspect),
    (format_plan_sync, ait_cli::binding_output::format_plan_sync),
}
