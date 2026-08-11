use super::*;

pub(super) const CHECKED_IN_CI_CONTRACT_PATH: &str = "ci/config.contract.json";

pub fn suite_manifest_catalog_path(
    ci_config: &JsonMap<String, JsonValue>,
    manifest: Option<&JsonMap<String, JsonValue>>,
) -> Option<String> {
    if let Some(explicit_path) = optional_text(ci_config.get("suite_manifest_path")) {
        return Some(explicit_path);
    }
    manifest
        .is_some_and(|manifest| manifest.contains_key("ci/patch_ci.json"))
        .then(|| "ci/patch_ci.json".to_string())
}

pub fn coerce_suite_catalog_payload(payload: &JsonValue, catalog_path: &str) -> JsonValue {
    let raw_payload = payload
        .as_object()
        .and_then(|object| object.get("suites"))
        .unwrap_or(payload);
    let Some(entries) = raw_payload.as_array() else {
        return json!({});
    };
    let mut suites = JsonMap::new();
    for entry in entries {
        let Some(entry_object) = entry.as_object() else {
            continue;
        };
        let Some(suite_id) = optional_text(entry_object.get("suite_id")) else {
            continue;
        };
        let mut suite = entry_object.clone();
        suite.insert("_artifact_path".to_string(), json!(catalog_path));
        suites.insert(suite_id, JsonValue::Object(suite));
    }
    JsonValue::Object(suites)
}

pub fn patchset_rollout_suite_ids(
    suites_by_id: &JsonMap<String, JsonValue>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let patchset_suite_ids = suites_by_id
        .iter()
        .filter_map(|(suite_id, suite)| {
            (suite
                .as_object()
                .and_then(|object| optional_text(object.get("plane")))
                .as_deref()
                == Some("patchset"))
            .then(|| suite_id.clone())
        })
        .collect::<Vec<_>>();
    let required_suite_ids = patchset_suite_ids
        .iter()
        .filter(|suite_id| {
            suites_by_id
                .get(*suite_id)
                .and_then(JsonValue::as_object)
                .and_then(|suite| suite.get("default_blocking"))
                .is_some_and(|value| truthy(Some(value)))
        })
        .cloned()
        .collect::<Vec<_>>();
    let required_set = required_suite_ids.iter().cloned().collect::<HashSet<_>>();
    let informational_suite_ids = patchset_suite_ids
        .iter()
        .filter(|suite_id| !required_set.contains(*suite_id))
        .cloned()
        .collect::<Vec<_>>();
    (
        patchset_suite_ids,
        required_suite_ids,
        informational_suite_ids,
    )
}

pub fn ci_rollout_summary_message(rollout_context: &JsonMap<String, JsonValue>) -> String {
    let phase = optional_i64(rollout_context.get("phase"))
        .ok()
        .flatten()
        .unwrap_or(0);
    let required = backtick_join_or_none(dedupe_text_values(
        rollout_context.get("required_patchset_suites"),
    ));
    let informational = backtick_join_or_none(dedupe_text_values(
        rollout_context.get("informational_patchset_suites"),
    ));
    let promotion_candidates = rollout_context
        .get("promotion_candidates")
        .and_then(JsonValue::as_object);
    let mut future_labels = Vec::new();
    for phase_name in ["phase1", "phase2"] {
        let items =
            dedupe_text_values(promotion_candidates.and_then(|value| value.get(phase_name)));
        if !items.is_empty() {
            future_labels.push(format!("{phase_name}: {}", backtick_join(items)));
        }
    }
    let future_message = if future_labels.is_empty() {
        String::new()
    } else {
        format!(
            " Future promotions are modeled as {}.",
            future_labels.join(", ")
        )
    };
    format!(
        "CI rollout phase {phase} blocks {required} and keeps {informational} visible as non-blocking surfaces.{future_message}"
    )
}

pub fn ci_rollout_patchset_suite_checks(
    rollout_context: &JsonMap<String, JsonValue>,
) -> Vec<JsonValue> {
    let suite_results_by_id = rollout_context
        .get("suite_results_by_id")
        .and_then(JsonValue::as_object);
    let mut checks = Vec::new();
    for suite_id in dedupe_text_values(rollout_context.get("required_patchset_suites")) {
        checks.push(suite_entry(&suite_id, true, suite_results_by_id));
    }
    for suite_id in dedupe_text_values(rollout_context.get("informational_patchset_suites")) {
        checks.push(suite_entry(&suite_id, false, suite_results_by_id));
    }
    checks
}

fn suite_entry(
    suite_id: &str,
    blocking: bool,
    suite_results_by_id: Option<&JsonMap<String, JsonValue>>,
) -> JsonValue {
    let result = suite_results_by_id
        .and_then(|results| results.get(suite_id))
        .and_then(JsonValue::as_object);
    let label = format!("Patchset CI suite `{suite_id}`");
    let Some(result) = result else {
        if blocking {
            return json!({
                "name": format!("ci_patchset_suite_{suite_id}"),
                "label": label,
                "status": "pending",
                "message": format!("Required patchset suite `{suite_id}` has not produced CI evidence for this patchset."),
            });
        }
        return json!({
            "name": format!("ci_patchset_suite_{suite_id}"),
            "label": label,
            "status": "not_required",
            "message": format!("Informational patchset suite `{suite_id}` is visible in rollout status but is not blocking for the current phase."),
        });
    };
    let status = optional_text(result.get("status"))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let (check_status, message) = if status == "pass" {
        (
            "pass",
            if blocking {
                format!("Required patchset suite `{suite_id}` passed.")
            } else {
                format!("Informational patchset suite `{suite_id}` passed.")
            },
        )
    } else if blocking {
        (
            "hard_fail",
            format!("Required patchset suite `{suite_id}` failed."),
        )
    } else {
        (
            "optional_fail",
            format!("Informational patchset suite `{suite_id}` failed; rollout keeps the red baseline visible without blocking land in the current phase."),
        )
    };
    json!({
        "name": format!("ci_patchset_suite_{suite_id}"),
        "label": label,
        "status": check_status,
        "message": message,
    })
}

fn backtick_join_or_none(items: Vec<String>) -> String {
    if items.is_empty() {
        "none".to_string()
    } else {
        backtick_join(items)
    }
}

fn backtick_join(items: Vec<String>) -> String {
    items
        .into_iter()
        .map(|item| format!("`{item}`"))
        .collect::<Vec<_>>()
        .join(", ")
}
