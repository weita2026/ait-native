use super::*;

pub(super) fn missing_requirements(policy: &JsonValue) -> Vec<String> {
    policy
        .get("checks")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|check| {
            let status = value_text(check, "status").unwrap_or_default();
            if matches!(status.as_str(), "pending" | "hard_fail" | "soft_fail") {
                Some(
                    value_text(check, "name")
                        .or_else(|| value_text(check, "label"))
                        .unwrap_or_else(|| "unknown".to_string()),
                )
            } else {
                None
            }
        })
        .collect()
}

pub(super) fn effective_validation_state(
    policy: &JsonValue,
    attestation: Option<&JsonValue>,
    key: &str,
    requirement_key: &str,
) -> String {
    let required = policy
        .get("effective_requirements")
        .and_then(JsonValue::as_object)
        .and_then(|requirements| requirements.get(requirement_key))
        .and_then(bool_value)
        .unwrap_or(false);
    if !required {
        return "not_required".to_string();
    }
    attestation
        .and_then(|row| row.get("evaluation_summary"))
        .and_then(JsonValue::as_object)
        .and_then(|summary| summary.get(key))
        .and_then(json_value_to_text)
        .unwrap_or_else(|| "pending".to_string())
}

pub(super) fn bool_value(value: &JsonValue) -> Option<bool> {
    value.as_bool().or_else(
        || match value.as_str()?.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        },
    )
}

pub(super) fn matches_filter(actual: Option<&str>, expected: Option<&str>) -> bool {
    match expected {
        None => true,
        Some("missing") => actual.is_none(),
        Some(expected) => actual == Some(expected),
    }
}

pub(super) fn matches_author_class(actual: Option<&str>, expected: Option<&str>) -> bool {
    match expected {
        None => true,
        Some("missing") => actual.is_none(),
        Some("human_only") => actual == Some("human_only"),
        Some("ai_related") => matches!(
            actual,
            Some("human_with_ai_assist" | "ai_with_human_review" | "ai_only_experimental")
        ),
        Some(expected) => actual == Some(expected),
    }
}

pub(super) fn matches_review_filter(
    review_summary: &JsonValue,
    requested_groups: &[String],
    expected: Option<&str>,
) -> bool {
    let Some(expected) = expected else {
        return true;
    };
    let approvals = value_int(review_summary, "approvals");
    let blocking = value_int(review_summary, "blocking");
    let comments = value_int(review_summary, "comments");
    match expected {
        "approved" => approvals > 0 && blocking == 0,
        "needs_approval" => approvals == 0 && blocking == 0,
        "blocking" => blocking > 0,
        "commented" => comments > 0,
        "requested" => !requested_groups.is_empty(),
        _ => false,
    }
}

pub(super) fn repo_matches(repo_name: Option<&str>, row: &JsonMap<String, JsonValue>) -> bool {
    repo_name
        .map(|repo_name| object_text(row, "repo_name").as_deref() == Some(repo_name))
        .unwrap_or(true)
}

pub(super) fn normalize_filter(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    })
}
