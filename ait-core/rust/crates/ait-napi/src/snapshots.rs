use crate::{repository::repository, workflows::WorkflowOperationResult, JsonInput};
use ait_cli::{binding_output as output, binding_services as services, primitives};
use ait_core::json_support::JsonValue;
use napi::{Error, Result};
use napi_derive::napi;

#[napi(object)]
#[derive(Default)]
pub struct SnapshotAncestryOptions {
    pub descendants: Option<bool>,
    pub first_parent: Option<bool>,
    pub max_depth: Option<JsonInput>,
    pub limit: Option<JsonInput>,
}

fn positive_count(input: Option<JsonInput>, default: usize, name: &str) -> Result<usize> {
    match input {
        None => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                Error::from_reason(format!("{name} must be a positive native-size integer."))
            }),
    }
}

#[napi]
pub fn inspect_snapshot(
    root: String,
    selector: String,
    include_delta: Option<bool>,
) -> Result<JsonValue> {
    services::inspect_snapshot(
        &repository(&root)?,
        &selector,
        include_delta.unwrap_or(false),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn project_snapshot_list(payload: JsonInput, all: Option<bool>) -> JsonValue {
    output::project_snapshot_list(&payload, all.unwrap_or(false))
}

#[napi]
pub fn format_snapshot_list(payload: JsonInput, all: Option<bool>) -> Result<String> {
    output::format_snapshot_list(&payload, all.unwrap_or(false)).map_err(Error::from_reason)
}

#[napi]
pub fn format_snapshot_show(inspection: JsonInput, files: Option<bool>) -> Result<String> {
    output::format_snapshot_show(&inspection, files.unwrap_or(false)).map_err(Error::from_reason)
}

#[napi]
pub fn snapshot_ancestry(
    root: String,
    selector: String,
    options: Option<SnapshotAncestryOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let request = services::SnapshotAncestryRequest {
        selector,
        descendants: options.descendants.unwrap_or(false),
        first_parent: options.first_parent.unwrap_or(false),
        max_depth: positive_count(
            options.max_depth,
            primitives::DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_MAX_DEPTH,
            "maxDepth",
        )?,
        limit: positive_count(
            options.limit,
            primitives::DEFAULT_PUBLIC_SNAPSHOT_ANCESTRY_LIMIT,
            "limit",
        )?,
    };
    services::query_snapshot_ancestry(&repository(&root)?, &request).map_err(Error::from_reason)
}

#[napi]
pub fn snapshot_is_ancestor(
    root: String,
    older: String,
    newer: String,
) -> Result<WorkflowOperationResult> {
    services::query_snapshot_is_ancestor(&repository(&root)?, &older, &newer)
        .map(Into::into)
        .map_err(Error::from_reason)
}

#[napi]
pub fn snapshot_merge_base(
    root: String,
    left: String,
    right: String,
    all: Option<bool>,
) -> Result<WorkflowOperationResult> {
    services::query_snapshot_merge_base(&repository(&root)?, &left, &right, all.unwrap_or(false))
        .map(Into::into)
        .map_err(Error::from_reason)
}

#[napi]
pub fn format_snapshot_ancestry(payload: JsonInput, all: Option<bool>) -> Result<String> {
    output::format_snapshot_ancestry(&payload, all.unwrap_or(false)).map_err(Error::from_reason)
}

#[napi]
pub fn format_snapshot_is_ancestor(payload: JsonInput) -> Result<String> {
    output::format_snapshot_is_ancestor(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn format_snapshot_merge_base(payload: JsonInput) -> Result<String> {
    output::format_snapshot_merge_base(&payload).map_err(Error::from_reason)
}

#[napi]
pub fn snapshot_query_error_exit_code() -> u32 {
    u32::from(output::SNAPSHOT_QUERY_ERROR_EXIT_CODE)
}

#[napi]
pub fn format_snapshot_query_error(message: String) -> String {
    output::format_snapshot_query_error(&message)
}

#[napi]
pub fn parse_snapshot_authoring_reference(reference: String) -> Result<String> {
    services::parse_snapshot_authoring_reference(&reference).map_err(Error::from_reason)
}
