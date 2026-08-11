use super::*;

pub fn patchset_changed_paths(patchset: &JsonMap<String, JsonValue>) -> Vec<String> {
    let diff_stats = patchset_diff_stats(patchset);
    let Some(paths) = diff_stats.get("paths").and_then(JsonValue::as_object) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut changed = Vec::new();
    for key in ["added", "deleted", "modified"] {
        if let Some(values) = paths.get(key).and_then(JsonValue::as_array) {
            for value in values {
                if let Some(text) = optional_text(Some(value)) {
                    if seen.insert(text.clone()) {
                        changed.push(text);
                    }
                }
            }
        }
    }
    changed.sort();
    changed
}

pub fn requires_code_review_summary(policy_context: &JsonMap<String, JsonValue>) -> bool {
    let configured_requirement = policy_context
        .get("effective_requirements")
        .and_then(JsonValue::as_object)
        .and_then(|requirements| requirements.get("require_code_review_summary"))
        .is_some_and(|value| truthy(Some(value)));
    configured_requirement
        || (optional_text(policy_context.get("content_class")).as_deref() == Some("code_change")
            && optional_text(policy_context.get("author_class")).as_deref() == Some("ai_related"))
}

pub fn dedupe_text_values(value: Option<&JsonValue>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let Some(values) = value.and_then(JsonValue::as_array) else {
        return out;
    };
    for item in values {
        if let Some(text) = optional_text(Some(item)) {
            if seen.insert(text.clone()) {
                out.push(text);
            }
        }
    }
    out
}

fn patchset_diff_stats(patchset: &JsonMap<String, JsonValue>) -> JsonValue {
    if let Some(diff_stats) = patchset.get("diff_stats").filter(|value| value.is_object()) {
        return diff_stats.clone();
    }
    json_loads_or_default(patchset.get("diff_stats_json"), json!({}))
}
