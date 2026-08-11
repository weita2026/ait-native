use super::*;

pub(super) fn json_optional_string(value: Option<&str>) -> JsonValue {
    value
        .map(|value| JsonValue::String(value.to_string()))
        .unwrap_or(JsonValue::Null)
}

pub(super) fn summary_matches_contains_terms(
    summary: &PlanDispatchSummary,
    contains_terms: &[String],
) -> bool {
    if contains_terms.is_empty() {
        return true;
    }
    let mut fields: Vec<String> = vec![
        summary
            .title
            .clone()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase(),
        summary
            .artifact_path
            .clone()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase(),
        summary
            .artifact_selector
            .clone()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase(),
    ];
    for item in &summary.items {
        fields.push(
            item.plan_item_ref
                .clone()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase(),
        );
        fields.push(item.text.trim().to_ascii_lowercase());
        fields.extend(
            item.heading_path
                .iter()
                .map(|heading| heading.trim().to_ascii_lowercase()),
        );
    }
    contains_terms.iter().any(|term| {
        let needle = term.trim().to_ascii_lowercase();
        !needle.is_empty()
            && fields
                .iter()
                .any(|field| !field.is_empty() && field.contains(&needle))
    })
}

pub(super) fn json_optional_i64(value: Option<i64>) -> JsonValue {
    value
        .map(|value| JsonValue::Number(value.into()))
        .unwrap_or(JsonValue::Null)
}

pub(super) fn json_usize(value: usize) -> JsonValue {
    JsonValue::Number(JsonNumber::from(value as u64))
}

pub(super) fn sync_action_count(results: &[JsonMap<String, JsonValue>], action: &str) -> usize {
    results
        .iter()
        .filter(|entry| entry.get("action").and_then(JsonValue::as_str) == Some(action))
        .count()
}
