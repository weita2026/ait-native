use super::*;

pub(super) fn text_field(obj: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    obj.get(field)
        .and_then(|value| match value {
            JsonValue::String(text) => Some(text.clone()),
            JsonValue::Number(number) => Some(number.to_string()),
            _ => None,
        })
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn row_text(row: &JsonMap<String, JsonValue>, field: &str) -> Option<String> {
    text_field(row, field)
}

pub(super) fn object_rows(
    value: JsonValue,
    label: &str,
) -> Result<Vec<JsonMap<String, JsonValue>>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{label} must be a JSON array"))?
        .iter()
        .map(|value| {
            value
                .as_object()
                .cloned()
                .ok_or_else(|| format!("{label} rows must be JSON objects"))
        })
        .collect()
}

pub(super) fn review_request_rows_from_review(
    review: &JsonMap<String, JsonValue>,
) -> Vec<JsonMap<String, JsonValue>> {
    let action = row_text(review, "action").unwrap_or_default();
    let has_request_shape = action == "request"
        || row_text(review, "status").as_deref() == Some("requested")
        || review.get("reviewer_groups").is_some()
        || review.get("requested_groups").is_some();
    if !has_request_shape {
        return Vec::new();
    }
    let groups = review
        .get("reviewer_groups")
        .or_else(|| review.get("requested_groups"))
        .and_then(JsonValue::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(JsonValue::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .or_else(|| row_text(review, "reviewer_group").map(|value| vec![value]))
        .unwrap_or_default();
    groups
        .into_iter()
        .enumerate()
        .map(|(index, group)| {
            JsonMap::from_iter([
                (
                    "review_request_id".to_string(),
                    json!(format!(
                        "{}:{index}",
                        row_text(review, "review_id").unwrap_or_else(|| "request".to_string())
                    )),
                ),
                (
                    "repo_id".to_string(),
                    review.get("repo_id").cloned().unwrap_or(JsonValue::Null),
                ),
                (
                    "change_id".to_string(),
                    review.get("change_id").cloned().unwrap_or(JsonValue::Null),
                ),
                (
                    "change_ref".to_string(),
                    review.get("change_ref").cloned().unwrap_or(JsonValue::Null),
                ),
                (
                    "patchset_id".to_string(),
                    review
                        .get("patchset_id")
                        .cloned()
                        .unwrap_or(JsonValue::Null),
                ),
                ("reviewer_group".to_string(), json!(group)),
                (
                    "note".to_string(),
                    review
                        .get("note")
                        .cloned()
                        .or_else(|| review.get("comment").cloned())
                        .unwrap_or(JsonValue::Null),
                ),
                (
                    "created_at".to_string(),
                    review.get("created_at").cloned().unwrap_or(JsonValue::Null),
                ),
            ])
        })
        .collect()
}

pub(super) fn normalize_policy_row(value: JsonValue) -> Result<JsonMap<String, JsonValue>, String> {
    let mut row = value
        .as_object()
        .cloned()
        .ok_or_else(|| "policy row must be a JSON object".to_string())?;
    if !row.contains_key("checks_json") {
        let checks = row.get("checks").cloned().unwrap_or_else(|| json!([]));
        row.insert("checks_json".to_string(), json!(checks.to_string()));
    }
    if !row.contains_key("effective_requirements_json") {
        let requirements = row
            .get("effective_requirements")
            .cloned()
            .unwrap_or_else(|| json!({}));
        row.insert(
            "effective_requirements_json".to_string(),
            json!(requirements.to_string()),
        );
    }
    Ok(row)
}

pub(super) fn normalize_attestation_row(
    value: JsonValue,
) -> Result<JsonMap<String, JsonValue>, String> {
    let mut row = value
        .as_object()
        .cloned()
        .ok_or_else(|| "attestation row must be a JSON object".to_string())?;
    if !row.contains_key("evaluation_summary_json") {
        let summary = row
            .get("evaluation_summary")
            .cloned()
            .unwrap_or_else(|| json!({}));
        row.insert(
            "evaluation_summary_json".to_string(),
            json!(summary.to_string()),
        );
    }
    if !row.contains_key("provenance_summary_json") {
        let summary = row
            .get("provenance_summary")
            .cloned()
            .unwrap_or_else(|| json!({}));
        row.insert(
            "provenance_summary_json".to_string(),
            json!(summary.to_string()),
        );
    }
    if !row.contains_key("detail_json") {
        let detail = row.get("detail").cloned().unwrap_or_else(|| json!({}));
        row.insert("detail_json".to_string(), json!(detail.to_string()));
    }
    Ok(row)
}
