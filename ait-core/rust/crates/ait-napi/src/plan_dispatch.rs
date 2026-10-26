use crate::JsonInput;
use ait_core::{json_support::JsonValue, plan_application::sdk};
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct PlanDispatchOptions {
    pub tasks: Option<JsonInput>,
    pub revision: Option<JsonInput>,
    pub local_shadow: Option<JsonInput>,
}

#[napi]
pub fn plan_items_payload(plan: JsonInput, revision: Option<JsonInput>) -> Result<JsonValue> {
    sdk::plan_items_payload_json(&plan, revision.as_deref()).map_err(Error::from_reason)
}
#[napi]
pub fn local_plan_publish_shadow(plan: Option<JsonInput>) -> Result<JsonValue> {
    sdk::local_plan_publish_shadow_json(plan.as_deref()).map_err(Error::from_reason)
}
#[napi]
pub fn plan_dispatch_summary(
    plan: JsonInput,
    options: Option<PlanDispatchOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    sdk::plan_dispatch_summary_json(
        &plan,
        options.tasks.as_deref(),
        options.revision.as_deref(),
        options.local_shadow.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn compute_taskable_items(
    plan: JsonInput,
    options: Option<PlanDispatchOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    sdk::compute_taskable_items_json(
        &plan,
        options.tasks.as_deref(),
        options.revision.as_deref(),
        options.local_shadow.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn validate_dispatch_legality(
    plan: JsonInput,
    plan_item_ref: Option<String>,
    options: Option<PlanDispatchOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    sdk::validate_dispatch_legality_json(
        &plan,
        options.tasks.as_deref(),
        plan_item_ref.as_deref(),
        options.revision.as_deref(),
        options.local_shadow.as_deref(),
    )
    .map_err(Error::from_reason)
}
#[napi]
pub fn plan_task_link_indexes(tasks: Option<JsonInput>) -> Result<JsonValue> {
    sdk::plan_task_link_indexes_json(tasks.as_deref()).map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct PlanCandidatesOptions {
    pub scope: Option<String>,
    pub repo_name: Option<String>,
    pub remote: Option<String>,
    pub include_all: Option<bool>,
}
#[napi]
pub fn plan_candidates_payload(
    summaries: Option<JsonInput>,
    options: Option<PlanCandidatesOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    sdk::plan_candidates_payload_json(
        summaries.as_deref(),
        options.scope.as_deref(),
        options.repo_name.as_deref(),
        options.remote.as_deref(),
        options.include_all.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}
