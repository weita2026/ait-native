//! Retained pure artifact matching SDK functions.
use crate::JsonInput;
use ait_core::{json_support::JsonValue, plan_blob_diff as artifact};
use napi::{Error, Result};
use napi_derive::napi;

#[napi]
pub fn artifact_blob_id(markdown: String) -> String {
    artifact::artifact_blob_id(&markdown)
}
#[napi]
pub fn plan_artifact_identity(
    artifact_path: String,
    artifact_selector: Option<String>,
) -> JsonValue {
    artifact::plan_artifact_identity(&artifact_path, artifact_selector.as_deref())
}
#[napi]
pub fn plan_artifact_identity_label(
    artifact_path: String,
    artifact_selector: Option<String>,
) -> String {
    artifact::plan_artifact_identity_label(&artifact_path, artifact_selector.as_deref())
}

#[napi]
pub fn artifact_candidates_open(candidates: JsonInput) -> Result<JsonValue> {
    artifact::artifact_candidates_open(&candidates).map_err(Error::from_reason)
}

#[napi]
pub fn index_plans_by_artifact_path(plans: JsonInput) -> Result<JsonValue> {
    artifact::index_plans_by_artifact_path(&plans).map_err(Error::from_reason)
}

#[napi]
pub fn index_plans_by_artifact_identity(plans: JsonInput) -> Result<JsonValue> {
    artifact::index_plans_by_artifact_identity(&plans).map_err(Error::from_reason)
}

#[napi]
pub fn local_plan_fully_published(plan: JsonInput) -> Result<bool> {
    artifact::local_plan_fully_published(&plan).map_err(Error::from_reason)
}

#[napi]
pub fn open_generic_plans_matching_blob_id(plans: JsonInput, blob_id: String) -> Result<JsonValue> {
    artifact::open_generic_plans_matching_blob_id(&plans, &blob_id).map_err(Error::from_reason)
}

#[napi]
pub fn open_plans_matching_selector(plans: JsonInput, selector: String) -> Result<JsonValue> {
    artifact::open_plans_matching_selector(&plans, &selector).map_err(Error::from_reason)
}

#[napi]
pub fn plan_heads_equivalent(left: JsonInput, right: JsonInput) -> Result<bool> {
    artifact::plan_heads_equivalent(&left, &right).map_err(Error::from_reason)
}
#[napi]
pub fn plan_matches_sync_artifact(
    plan: JsonInput,
    artifact_value: JsonInput,
    require_title_match: Option<bool>,
) -> Result<bool> {
    artifact::plan_matches_sync_artifact(
        &plan,
        &artifact_value,
        require_title_match.unwrap_or(true),
    )
    .map_err(Error::from_reason)
}
