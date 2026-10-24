use crate::json_support::{json, JsonMap as Map, JsonValue};
use std::collections::BTreeMap;

use crate::object_diff::artifact_blob_id;

pub fn artifact_candidates_open(candidates: &JsonValue) -> Result<JsonValue, String> {
    let values = as_array(candidates, "candidates")?;
    Ok(JsonValue::Array(
        values
            .iter()
            .filter_map(|candidate| {
                let candidate_obj = candidate.as_object()?;
                if plan_status_is_historical(
                    candidate_obj.get("status").and_then(JsonValue::as_str),
                ) {
                    return None;
                }
                Some(candidate.clone())
            })
            .collect(),
    ))
}

pub fn plan_artifact_identity(artifact_path: &str, artifact_selector: Option<&str>) -> JsonValue {
    json!({
        "artifact_path": artifact_path,
        "artifact_selector": normalize_optional_text(artifact_selector),
    })
}

pub fn plan_artifact_identity_label(
    artifact_path: &str,
    artifact_selector: Option<&str>,
) -> String {
    match normalize_optional_text(artifact_selector) {
        Some(selector) => format!("{artifact_path} [{selector}]"),
        None => artifact_path.to_string(),
    }
}

pub fn index_plans_by_artifact_path(plans: &JsonValue) -> Result<JsonValue, String> {
    let values = as_array(plans, "plans")?;
    let mut grouped: BTreeMap<String, Vec<JsonValue>> = BTreeMap::new();
    for plan in values {
        let Some(plan_obj) = plan.as_object() else {
            return Err("plans must contain object rows".to_string());
        };
        let empty_head_revision = Map::new();
        let head_revision = object_field(plan_obj, "head_revision").unwrap_or(&empty_head_revision);
        let Some(artifact_path) = normalize_optional_text(
            head_revision
                .get("artifact_path")
                .and_then(JsonValue::as_str),
        ) else {
            continue;
        };
        grouped.entry(artifact_path).or_default().push(plan.clone());
    }
    let mut payload = Map::new();
    for (artifact_path, plans) in grouped {
        payload.insert(artifact_path, JsonValue::Array(plans));
    }
    Ok(JsonValue::Object(payload))
}

pub fn index_plans_by_artifact_identity(plans: &JsonValue) -> Result<JsonValue, String> {
    let values = as_array(plans, "plans")?;
    let mut grouped: BTreeMap<(String, Option<String>), Vec<JsonValue>> = BTreeMap::new();
    for plan in values {
        let Some(plan_obj) = plan.as_object() else {
            return Err("plans must contain object rows".to_string());
        };
        let empty_head_revision = Map::new();
        let head_revision = object_field(plan_obj, "head_revision").unwrap_or(&empty_head_revision);
        let Some(artifact_path) = normalize_optional_text(
            head_revision
                .get("artifact_path")
                .and_then(JsonValue::as_str),
        ) else {
            continue;
        };
        let key = (
            artifact_path,
            normalize_optional_text(
                head_revision
                    .get("artifact_selector")
                    .and_then(JsonValue::as_str),
            ),
        );
        grouped.entry(key).or_default().push(plan.clone());
    }
    Ok(JsonValue::Array(
        grouped
            .into_iter()
            .map(|((artifact_path, artifact_selector), plans)| {
                json!({
                    "artifact_path": artifact_path,
                    "artifact_selector": artifact_selector,
                    "plans": plans,
                })
            })
            .collect(),
    ))
}

pub fn open_generic_plans_matching_blob_id(
    plans: &JsonValue,
    blob_id: &str,
) -> Result<JsonValue, String> {
    let values = as_array(plans, "plans")?;
    let filtered = values
        .iter()
        .filter_map(|plan| {
            let plan_obj = plan.as_object()?;
            if plan_status_is_historical(plan_obj.get("status").and_then(JsonValue::as_str)) {
                return None;
            }
            if plan_head_value(plan_obj, "artifact_selector").is_some() {
                return None;
            }
            if plan_head_value(plan_obj, "artifact_blob_id").as_deref() != Some(blob_id) {
                return None;
            }
            Some(plan.clone())
        })
        .collect::<Vec<_>>();
    Ok(JsonValue::Array(filtered))
}

pub fn open_plans_matching_selector(
    plans: &JsonValue,
    selector: &str,
) -> Result<JsonValue, String> {
    let values = as_array(plans, "plans")?;
    let filtered = values
        .iter()
        .filter_map(|plan| {
            let plan_obj = plan.as_object()?;
            if plan_status_is_historical(plan_obj.get("status").and_then(JsonValue::as_str)) {
                return None;
            }
            if plan_head_value(plan_obj, "artifact_selector").as_deref() != Some(selector) {
                return None;
            }
            Some(plan.clone())
        })
        .collect::<Vec<_>>();
    Ok(JsonValue::Array(filtered))
}

pub fn local_plan_fully_published(plan: &JsonValue) -> Result<bool, String> {
    let plan_obj = as_object(plan, "plan")?;
    let empty_head_revision = Map::new();
    let head_revision = object_field(plan_obj, "head_revision").unwrap_or(&empty_head_revision);
    Ok(plan_obj
        .get("publication_state")
        .and_then(JsonValue::as_str)
        == Some("published")
        && head_revision
            .get("publication_state")
            .and_then(JsonValue::as_str)
            == Some("published")
        && normalize_optional_text(
            plan_obj
                .get("published_head_revision_id")
                .and_then(JsonValue::as_str),
        )
        .is_some())
}

pub fn plan_heads_equivalent(left: &JsonValue, right: &JsonValue) -> Result<bool, String> {
    let left_obj = as_object(left, "left plan")?;
    let right_obj = as_object(right, "right plan")?;
    let empty_left_head = Map::new();
    let empty_right_head = Map::new();
    let left_head = object_field(left_obj, "head_revision").unwrap_or(&empty_left_head);
    let right_head = object_field(right_obj, "head_revision").unwrap_or(&empty_right_head);
    Ok(
        normalize_optional_text(left_obj.get("title").and_then(JsonValue::as_str))
            == normalize_optional_text(right_obj.get("title").and_then(JsonValue::as_str))
            && normalize_optional_text(left_head.get("artifact_path").and_then(JsonValue::as_str))
                == normalize_optional_text(
                    right_head.get("artifact_path").and_then(JsonValue::as_str),
                )
            && normalize_optional_text(
                left_head
                    .get("artifact_selector")
                    .and_then(JsonValue::as_str),
            ) == normalize_optional_text(
                right_head
                    .get("artifact_selector")
                    .and_then(JsonValue::as_str),
            )
            && normalize_optional_text(
                left_head
                    .get("artifact_heading")
                    .and_then(JsonValue::as_str),
            ) == normalize_optional_text(
                right_head
                    .get("artifact_heading")
                    .and_then(JsonValue::as_str),
            )
            && normalize_optional_text(
                left_head
                    .get("artifact_blob_id")
                    .and_then(JsonValue::as_str),
            ) == normalize_optional_text(
                right_head
                    .get("artifact_blob_id")
                    .and_then(JsonValue::as_str),
            )
            && array_field(left_head, "items").unwrap_or_default()
                == array_field(right_head, "items").unwrap_or_default(),
    )
}

pub fn plan_matches_sync_artifact(
    plan: &JsonValue,
    artifact: &JsonValue,
    require_title_match: bool,
) -> Result<bool, String> {
    let plan_obj = as_object(plan, "plan")?;
    let artifact_obj = as_object(artifact, "artifact")?;
    let empty_head_revision = Map::new();
    let head_revision = object_field(plan_obj, "head_revision").unwrap_or(&empty_head_revision);
    let head_selector = normalize_optional_text(
        head_revision
            .get("artifact_selector")
            .and_then(JsonValue::as_str),
    );
    let artifact_selector = normalize_optional_text(
        artifact_obj
            .get("artifact_selector")
            .and_then(JsonValue::as_str),
    );
    let expected_blob_id = artifact_obj
        .get("artifact_body")
        .and_then(JsonValue::as_str)
        .map(artifact_blob_id);
    let title_matches = if require_title_match {
        plan_obj
            .get("title")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            == artifact_obj
                .get("artifact_heading")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
    } else {
        true
    };
    Ok(title_matches
        && head_revision
            .get("artifact_path")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            == artifact_obj
                .get("artifact_path")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
        && head_selector == artifact_selector
        && head_revision
            .get("artifact_heading")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            == artifact_obj
                .get("artifact_heading")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
        && array_field(head_revision, "items").unwrap_or_default()
            == array_field(artifact_obj, "items").unwrap_or_default()
        && expected_blob_id
            .map(|value| {
                head_revision
                    .get("artifact_blob_id")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    == value
            })
            .unwrap_or(true))
}

fn plan_status_is_historical(status: Option<&str>) -> bool {
    normalize_optional_text(status)
        .map(|value| matches!(value.as_str(), "archived" | "superseded"))
        .unwrap_or(false)
}

fn plan_head_value(plan: &Map<String, JsonValue>, key: &str) -> Option<String> {
    object_field(plan, "head_revision")
        .and_then(|head_revision| head_revision.get(key))
        .and_then(JsonValue::as_str)
        .and_then(|value| normalize_optional_text(Some(value)))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn as_object<'a>(value: &'a JsonValue, label: &str) -> Result<&'a Map<String, JsonValue>, String> {
    value
        .as_object()
        .ok_or_else(|| format!("{label} must be an object"))
}

fn object_field<'a>(
    value: &'a Map<String, JsonValue>,
    key: &str,
) -> Option<&'a Map<String, JsonValue>> {
    value.get(key).and_then(JsonValue::as_object)
}

fn as_array<'a>(value: &'a JsonValue, label: &str) -> Result<&'a Vec<JsonValue>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{label} must be a list"))
}

fn array_field(value: &Map<String, JsonValue>, key: &str) -> Option<Vec<JsonValue>> {
    value.get(key).and_then(JsonValue::as_array).cloned()
}

#[cfg(test)]
mod tests;
