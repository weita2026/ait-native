//! Pure Plan and manifest capabilities retained by the PyO3 SDK.
use crate::{workspace::optional_count, JsonInput};
use ait_core::{
    json_support::{json, JsonMap, JsonValue},
    plan_foundation::{self, ParsedPlan},
    plan_items::{self, NormalizedPlanItemSeed, PlanItem, PlanSectionRef},
};
use napi::{bindgen_prelude::Buffer, Error, Result, Status};
use napi_derive::napi;
use std::collections::{BTreeMap, HashMap};

fn input_error(message: impl Into<String>) -> Error {
    Error::new(Status::InvalidArg, message.into())
}

pub(crate) fn text_field(row: &JsonMap<String, JsonValue>, field: &str) -> Result<Option<String>> {
    match row.get(field) {
        None | Some(JsonValue::Null) => Ok(None),
        Some(JsonValue::String(value)) => Ok(
            ait_core::text_normalization::normalize_optional_text(Some(value)),
        ),
        _ => Err(input_error(format!("{field} must be a string"))),
    }
}

fn count_field(row: &JsonMap<String, JsonValue>, field: &str, default: usize) -> Result<usize> {
    match row.get(field) {
        None | Some(JsonValue::Null) => Ok(default),
        Some(value) => value
            .as_u64()
            .and_then(|v| usize::try_from(v).ok())
            .ok_or_else(|| {
                input_error(format!("{field} must be a nonnegative native-size integer"))
            }),
    }
}

pub(crate) fn plan_item_json(item: PlanItem) -> JsonValue {
    json!({"plan_item_ref":item.plan_item_ref,"text":item.text,"checkbox_state":item.checkbox_state.as_str(),"heading_path":item.heading_path,"line_number":item.line_number})
}

fn section_ref_json(item: PlanSectionRef) -> JsonValue {
    json!({"plan_ref":item.plan_ref,"heading_title":item.heading_title,"heading_level":item.heading_level,"line_number":item.line_number})
}

pub(crate) fn parse_plan_item_seeds(
    value: Option<&JsonValue>,
) -> Result<Vec<NormalizedPlanItemSeed>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let rows = value
        .as_array()
        .ok_or_else(|| input_error("Plan items must be a list."))?;
    let seeds = rows
        .iter()
        .map(|value| {
            let row = value
                .as_object()
                .ok_or_else(|| input_error("Plan items must be objects."))?;
            let plan_item_ref = text_field(row, "plan_item_ref")?
                .ok_or_else(|| input_error("Plan items must include plan_item_ref."))?;
            let heading_path = match row.get("heading_path") {
                None | Some(JsonValue::Null) => Vec::new(),
                Some(value) => value
                    .as_array()
                    .ok_or_else(|| {
                        input_error(format!(
                            "Plan item {plan_item_ref} heading_path must be a list."
                        ))
                    })?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(|value| {
                                ait_core::text_normalization::normalize_optional_text(Some(value))
                            })
                            .ok_or_else(|| input_error("heading_path entries must be strings"))
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .flatten()
                    .collect(),
            };
            let line_number = match row.get("line_number") {
                None | Some(JsonValue::Null) => 0,
                Some(value) => value.as_i64().ok_or_else(|| {
                    input_error(format!(
                        "Plan item {plan_item_ref} line_number must be an integer."
                    ))
                })?,
            };
            Ok(NormalizedPlanItemSeed {
                plan_item_ref,
                text: text_field(row, "text")?.unwrap_or_default(),
                checkbox_state: text_field(row, "checkbox_state")?
                    .unwrap_or_else(|| "none".to_string()),
                heading_path,
                line_number,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(seeds)
}

pub(crate) fn parse_plan_items(value: Option<&JsonValue>) -> Result<Vec<PlanItem>> {
    plan_items::normalize_plan_items(&parse_plan_item_seeds(value)?).map_err(Error::from_reason)
}

fn parse_section_refs(value: Option<&JsonValue>) -> Result<Vec<PlanSectionRef>> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    value
        .as_array()
        .ok_or_else(|| input_error("Parsed plan refs must be a list."))?
        .iter()
        .map(|value| {
            let row = value
                .as_object()
                .ok_or_else(|| input_error("Parsed plan refs must be objects."))?;
            Ok(PlanSectionRef {
                plan_ref: text_field(row, "plan_ref")?
                    .ok_or_else(|| input_error("Parsed plan refs must include plan_ref."))?,
                heading_title: text_field(row, "heading_title")?.unwrap_or_default(),
                heading_level: count_field(row, "heading_level", 0)?,
                line_number: count_field(row, "line_number", 0)?,
            })
        })
        .collect()
}

#[napi]
pub fn parse_plan_markdown(markdown: Option<String>) -> JsonValue {
    let parsed = plan_foundation::parse_plan_markdown(markdown.as_deref());
    json!({"markdown_text":parsed.markdown_text,"plan_ref_count":parsed.plan_ref_count,"item_count":parsed.item_count,"plan_refs":parsed.plan_refs.into_iter().map(section_ref_json).collect::<Vec<_>>(),"items":parsed.items.into_iter().map(plan_item_json).collect::<Vec<_>>()})
}

#[napi]
pub fn extract_plan_refs(parsed_plan: JsonInput) -> Result<JsonValue> {
    let row = parsed_plan
        .as_object()
        .ok_or_else(|| input_error("Parsed plan must be an object."))?;
    let plan_refs = parse_section_refs(row.get("plan_refs"))?;
    let items = parse_plan_items(row.get("items"))?;
    let parsed = ParsedPlan {
        markdown_text: text_field(row, "markdown_text")?.unwrap_or_default(),
        plan_ref_count: count_field(row, "plan_ref_count", plan_refs.len())?,
        item_count: count_field(row, "item_count", items.len())?,
        plan_refs,
        items,
    };
    let refs = plan_foundation::extract_plan_refs(&parsed);
    Ok(
        json!({"plan_ref_count":refs.plan_ref_count,"plan_refs":refs.plan_refs.into_iter().map(section_ref_json).collect::<Vec<_>>()}),
    )
}

#[napi]
pub fn compute_sync_prune_decisions(
    scope: Option<String>,
    tracked_artifacts: Option<Vec<String>>,
    synced_artifacts: Option<Vec<String>>,
) -> Result<JsonValue> {
    let result = plan_foundation::compute_sync_prune_decisions(
        scope.as_deref(),
        &tracked_artifacts.unwrap_or_default(),
        &synced_artifacts.unwrap_or_default(),
    )
    .map_err(Error::from_reason)?;
    Ok(
        json!({"scope":result.scope,"tracked_artifact_count":result.tracked_artifact_count,"synced_artifact_count":result.synced_artifact_count,"retained_paths":result.retained_paths,"prune_paths":result.prune_paths,"prune_count":result.prune_count}),
    )
}

#[napi(object)]
#[derive(Default)]
pub struct ManifestIdentityOptions {
    pub old_snapshot_id: Option<String>,
    pub new_snapshot_id: Option<String>,
}
#[napi]
pub fn diff_snapshot_manifests(
    old_files: JsonInput,
    new_files: JsonInput,
    options: Option<ManifestIdentityOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    ait_core::object_diff::diff_snapshot_manifests(
        &old_files,
        &new_files,
        options.old_snapshot_id.as_deref(),
        options.new_snapshot_id.as_deref(),
    )
    .map_err(Error::from_reason)
}

#[napi(object)]
#[derive(Default)]
pub struct ManifestTextDiffOptions {
    pub old_snapshot_id: Option<String>,
    pub new_snapshot_id: Option<String>,
    pub blob_bytes_by_id: Option<HashMap<String, Buffer>>,
    pub include_text: Option<bool>,
    pub max_bytes: Option<JsonInput>,
}
#[napi]
pub fn snapshot_diff_from_manifests(
    old_files: JsonInput,
    new_files: JsonInput,
    options: Option<ManifestTextDiffOptions>,
) -> Result<JsonValue> {
    let options = options.unwrap_or_default();
    let blobs: BTreeMap<String, Vec<u8>> = options
        .blob_bytes_by_id
        .unwrap_or_default()
        .into_iter()
        .map(|(id, bytes)| (id, bytes.to_vec()))
        .collect();
    ait_core::object_diff::snapshot_diff_from_manifests(
        &old_files,
        &new_files,
        &blobs,
        options.old_snapshot_id.as_deref(),
        options.new_snapshot_id.as_deref(),
        options.include_text.unwrap_or(false),
        optional_count(options.max_bytes, "maxBytes")?
            .unwrap_or(ait_core::object_diff::DEFAULT_SNAPSHOT_DIFF_MAX_BYTES),
    )
    .map_err(Error::from_reason)
}

#[napi]
pub fn extract_plan_items(markdown: Option<String>) -> JsonValue {
    JsonValue::Array(
        plan_items::extract_plan_items(markdown.as_deref())
            .into_iter()
            .map(plan_item_json)
            .collect(),
    )
}
#[napi]
pub fn list_plan_section_refs(markdown: Option<String>) -> JsonValue {
    JsonValue::Array(
        plan_items::list_plan_section_refs(markdown.as_deref())
            .into_iter()
            .map(section_ref_json)
            .collect(),
    )
}
#[napi]
pub fn extract_plan_section(markdown: Option<String>, plan_ref: Option<String>) -> JsonValue {
    plan_items::extract_plan_section(markdown.as_deref(), plan_ref.as_deref()).map(|section| json!({
        "plan_ref":section.plan_ref,"heading_title":section.heading_title,"heading_level":section.heading_level,"line_number":section.line_number,
        "section_markdown":section.section_markdown,"items":section.items.into_iter().map(plan_item_json).collect::<Vec<_>>(),
    })).unwrap_or(JsonValue::Null)
}
#[napi]
pub fn find_plan_item(markdown: Option<String>, plan_item_ref: Option<String>) -> JsonValue {
    plan_items::find_plan_item(markdown.as_deref(), plan_item_ref.as_deref())
        .map(plan_item_json)
        .unwrap_or(JsonValue::Null)
}
#[napi]
pub fn normalize_plan_items(items: Option<JsonInput>) -> Result<JsonValue> {
    Ok(JsonValue::Array(
        parse_plan_items(items.as_deref())?
            .into_iter()
            .map(plan_item_json)
            .collect(),
    ))
}
#[napi]
pub fn find_plan_item_in_items(
    items: Option<JsonInput>,
    plan_item_ref: Option<String>,
) -> Result<JsonValue> {
    let item = plan_items::find_plan_item_in_items(
        &parse_plan_item_seeds(items.as_deref())?,
        plan_item_ref.as_deref(),
    )
    .map_err(Error::from_reason)?;
    Ok(item.map(plan_item_json).unwrap_or(JsonValue::Null))
}
#[napi]
pub fn normalize_task_workflow_http_compatibility(payload_json: String) -> Result<JsonValue> {
    ait_core::task_workflow_http_adapter::normalize_task_workflow_http_compatibility_payload_json(
        &payload_json,
    )
    .map_err(|error| Error::from_reason(error.to_string()))
}
#[napi]
pub fn normalize_task_workflow_http_readiness(payload_json: String) -> Result<JsonValue> {
    ait_core::task_workflow_http_adapter::normalize_task_workflow_http_readiness_payload_json(
        &payload_json,
    )
    .map_err(|error| Error::from_reason(error.to_string()))
}
