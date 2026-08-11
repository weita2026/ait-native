use super::*;
use std::collections::HashSet;

pub(super) fn clean_text(value: Option<&JsonValue>) -> Option<String> {
    let text = value.and_then(JsonValue::as_str).unwrap_or("").trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

pub(super) fn clean_text_value(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => {
            let normalized = text.trim();
            if normalized.is_empty() {
                None
            } else {
                Some(normalized.to_string())
            }
        }
        _ => None,
    }
}

pub(super) fn plan_head_revision_id(plan: &JsonValue) -> Option<String> {
    plan.get("head_revision")
        .and_then(|value| value.get("plan_revision_id"))
        .and_then(clean_text_value)
        .or_else(|| plan.get("head_revision_id").and_then(clean_text_value))
}

pub(super) fn plan_revision_items(revision: &JsonValue) -> Vec<JsonValue> {
    if let Some(items) = revision.get("items").and_then(JsonValue::as_array) {
        return items.to_vec();
    }
    revision
        .get("items_json")
        .and_then(JsonValue::as_str)
        .and_then(|text| serde_json::from_str::<JsonValue>(text).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

pub(super) fn collect_plan_item_refs(items: &[JsonValue], refs: &mut Vec<String>) {
    for item in items {
        if let Some(plan_item_ref) = item.get("plan_item_ref").and_then(clean_text_value) {
            refs.push(plan_item_ref);
        }
        for key in ["items", "children", "tasks"] {
            if let Some(children) = item.get(key).and_then(JsonValue::as_array) {
                collect_plan_item_refs(children, refs);
            }
        }
    }
}

pub(super) fn plan_item_ref_exists(items: &[JsonValue], wanted: &str) -> bool {
    items.iter().any(|item| {
        item.get("plan_item_ref")
            .and_then(clean_text_value)
            .as_deref()
            == Some(wanted)
            || ["items", "children", "tasks"].iter().any(|key| {
                item.get(*key)
                    .and_then(JsonValue::as_array)
                    .is_some_and(|children| plan_item_ref_exists(children, wanted))
            })
    })
}

pub(super) fn find_revision_in_repo<R>(
    runtime: &R,
    repo_name: &str,
    plan_revision_id: &str,
) -> Result<(String, JsonValue), String>
where
    R: ServerRuntimeService + ?Sized,
{
    let plans = runtime.list_plans(repo_name, None)?;
    for plan in plans.as_array().cloned().unwrap_or_default() {
        let Some(plan_id) = plan.get("plan_id").and_then(clean_text_value) else {
            continue;
        };
        if let Ok(revision) = runtime.get_plan_revision(&plan_id, plan_revision_id) {
            return Ok((plan_id, revision));
        }
    }
    Err(format!("Unknown plan revision: {plan_revision_id}"))
}

pub(super) fn resolve_task_plan_linkage_with_runtime<R>(
    runtime: &R,
    repo_name: &str,
    payload: &JsonValue,
) -> Result<JsonValue, String>
where
    R: ServerRuntimeService + ?Sized,
{
    let payload = payload
        .as_object()
        .ok_or_else(|| "task plan linkage payload must be a JSON object.".to_string())?;
    let mut resolved_plan_id = clean_text(payload.get("plan_id"));
    let mut resolved_revision_id = clean_text(payload.get("origin_plan_revision_id"));
    let resolved_plan_item_ref = clean_text(payload.get("plan_item_ref"));

    if resolved_plan_id.is_none() && resolved_revision_id.is_none() {
        if resolved_plan_item_ref.is_some() {
            return Err("plan_item_ref requires plan linkage".to_string());
        }
        return Ok(json!({
            "plan_id": null,
            "origin_plan_revision_id": null,
            "plan_item_ref": null,
        }));
    }

    let mut plan: Option<JsonValue> = None;
    if let Some(plan_id) = resolved_plan_id.as_deref() {
        let value = runtime.get_plan(plan_id)?;
        let actual_repo = value.get("repo_name").and_then(clean_text_value);
        if actual_repo.as_deref() != Some(repo_name) {
            let actual = actual_repo
                .or_else(|| value.get("repo_id").and_then(clean_text_value))
                .unwrap_or_else(|| "unknown".to_string());
            return Err(format!(
                "Plan {plan_id} belongs to repository {actual}, not {repo_name}"
            ));
        }
        plan = Some(value);
    }

    let revision = if let Some(revision_id) = resolved_revision_id.as_deref() {
        if let Some(plan_id) = resolved_plan_id.as_deref() {
            Some(runtime.get_plan_revision(plan_id, revision_id)?)
        } else {
            let (plan_id, revision) = find_revision_in_repo(runtime, repo_name, revision_id)?;
            resolved_plan_id = Some(plan_id);
            Some(revision)
        }
    } else if let Some(plan_value) = plan.as_ref() {
        let head_revision_id = plan_head_revision_id(plan_value).ok_or_else(|| {
            format!(
                "Plan {} has no head revision to link from",
                resolved_plan_id.as_deref().unwrap_or("")
            )
        })?;
        let plan_id = resolved_plan_id
            .as_deref()
            .ok_or_else(|| "resolved plan is missing plan_id".to_string())?;
        resolved_revision_id = Some(head_revision_id.clone());
        Some(runtime.get_plan_revision(plan_id, &head_revision_id)?)
    } else {
        None
    };

    if let (Some(plan_id), Some(revision_value)) = (resolved_plan_id.as_deref(), revision.as_ref())
    {
        if let Some(revision_plan_id) = revision_value.get("plan_id").and_then(clean_text_value) {
            if revision_plan_id != plan_id {
                return Err(format!(
                    "Plan revision {} does not belong to plan {plan_id}",
                    resolved_revision_id.as_deref().unwrap_or("")
                ));
            }
        }
    }

    if let Some(plan_item_ref) = resolved_plan_item_ref.as_deref() {
        let revision_value = revision.as_ref().ok_or_else(|| {
            format!(
                "Unknown plan revision: {}",
                resolved_revision_id.as_deref().unwrap_or("")
            )
        })?;
        let items = plan_revision_items(revision_value);
        if !plan_item_ref_exists(&items, plan_item_ref) {
            let mut known_refs = Vec::new();
            collect_plan_item_refs(&items, &mut known_refs);
            known_refs.sort();
            known_refs.dedup();
            if known_refs.is_empty() {
                return Err(format!(
                    "Plan revision {} does not expose any explicit `[ref: ...]` plan items yet. Add refs to the file-backed plan section before binding a task to one.",
                    resolved_revision_id.as_deref().unwrap_or("")
                ));
            }
            return Err(format!(
                "Plan item ref {plan_item_ref:?} is not present in plan revision {}. Known refs: {}",
                resolved_revision_id.as_deref().unwrap_or(""),
                known_refs.join(", ")
            ));
        }
    }

    Ok(json!({
        "plan_id": resolved_plan_id,
        "origin_plan_revision_id": resolved_revision_id,
        "plan_item_ref": resolved_plan_item_ref,
    }))
}

pub(super) fn list_plan_ids_matching_contains_with_runtime<R>(
    runtime: &R,
    repo_name: &str,
    payload: &JsonValue,
) -> Result<JsonValue, String>
where
    R: ServerRuntimeService + ?Sized,
{
    let payload = payload
        .as_object()
        .ok_or_else(|| "plan contains query payload must be a JSON object.".to_string())?;
    let terms = payload
        .get("contains_terms")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| "contains_terms must be a JSON array.".to_string())?;
    let terms = normalize_plan_contains_terms(terms.iter().filter_map(JsonValue::as_str));
    if terms.is_empty() {
        return Ok(JsonValue::Array(Vec::new()));
    }
    let out = matching_active_plans_with_runtime(runtime, repo_name, &terms)?
        .iter()
        .filter_map(|plan| plan.get("plan_id").and_then(clean_text_value))
        .map(JsonValue::String)
        .collect();
    Ok(JsonValue::Array(out))
}

pub(crate) fn normalize_plan_contains_query(raw: Option<&str>) -> Vec<String> {
    normalize_plan_contains_terms(raw.into_iter().flat_map(|value| value.split(',')))
}

pub(crate) fn read_plan_candidate_inputs_with_runtime<R, W>(
    runtime: &R,
    workflow: &W,
    repo_name: &str,
    contains_terms: &[String],
) -> Result<JsonValue, String>
where
    R: ServerRuntimeService + ?Sized,
    W: ServerWorkflowStore + ?Sized,
{
    let plans = matching_active_plans_with_runtime(runtime, repo_name, contains_terms)?;
    let matched_plan_ids = plans
        .iter()
        .filter_map(|plan| plan.get("plan_id").and_then(clean_text_value))
        .collect::<HashSet<_>>();
    let task_rows = workflow.list_tasks(repo_name)?;
    let tasks = task_rows
        .as_array()
        .ok_or_else(|| "workflow task list must be a JSON array.".to_string())?
        .iter()
        .filter(|task| {
            task.get("plan_id")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .is_some_and(|plan_id| !plan_id.is_empty() && matched_plan_ids.contains(plan_id))
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(json!({
        "repo_name": repo_name,
        "contains_terms": contains_terms,
        "plans": plans,
        "tasks": tasks,
    }))
}

fn normalize_plan_contains_terms<'a>(values: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut terms = Vec::new();
    for value in values {
        let normalized = value.trim().to_ascii_lowercase();
        if !normalized.is_empty() && !terms.contains(&normalized) {
            terms.push(normalized);
        }
    }
    terms
}

fn matching_active_plans_with_runtime<R>(
    runtime: &R,
    repo_name: &str,
    contains_terms: &[String],
) -> Result<Vec<JsonValue>, String>
where
    R: ServerRuntimeService + ?Sized,
{
    let plan_rows = runtime.list_plans(repo_name, None)?;
    let plans = plan_rows
        .as_array()
        .ok_or_else(|| "runtime Plan list must be a JSON array.".to_string())?
        .iter()
        .filter(|plan| !plan_is_historical(plan))
        .filter(|plan| plan_matches_contains_terms(plan, contains_terms))
        .cloned()
        .collect();
    Ok(plans)
}

fn plan_is_historical(plan: &JsonValue) -> bool {
    plan.get("status")
        .and_then(JsonValue::as_str)
        .is_some_and(|status| {
            matches!(
                status.trim().to_ascii_lowercase().as_str(),
                "archived" | "superseded"
            )
        })
}

fn plan_matches_contains_terms(plan: &JsonValue, contains_terms: &[String]) -> bool {
    if contains_terms.is_empty() {
        return true;
    }
    let head_revision = plan.get("head_revision");
    let scalar_fields = [
        plan.get("title"),
        head_revision
            .and_then(|revision| revision.get("artifact_path"))
            .or_else(|| plan.get("head_artifact_path")),
        head_revision
            .and_then(|revision| revision.get("artifact_selector"))
            .or_else(|| plan.get("head_artifact_selector")),
    ];
    if scalar_fields
        .into_iter()
        .flatten()
        .any(|field| json_text_matches(field, contains_terms))
    {
        return true;
    }
    plan_head_items(plan).iter().any(|item| {
        ["plan_item_ref", "text"]
            .into_iter()
            .filter_map(|key| item.get(key))
            .any(|field| json_text_matches(field, contains_terms))
            || item
                .get("heading_path")
                .and_then(JsonValue::as_array)
                .is_some_and(|headings| {
                    headings
                        .iter()
                        .any(|heading| json_text_matches(heading, contains_terms))
                })
    })
}

fn plan_head_items(plan: &JsonValue) -> Vec<JsonValue> {
    let items = plan
        .get("head_revision")
        .map(plan_revision_items)
        .unwrap_or_default();
    if !items.is_empty() {
        return items;
    }
    plan.get("head_revision_items_json")
        .and_then(JsonValue::as_str)
        .and_then(|text| serde_json::from_str::<JsonValue>(text).ok())
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
}

fn json_text_matches(value: &JsonValue, contains_terms: &[String]) -> bool {
    let Some(value) = value.as_str() else {
        return false;
    };
    let field = value.trim().to_ascii_lowercase();
    !field.is_empty()
        && contains_terms.iter().any(|term| {
            let term = term.trim().to_ascii_lowercase();
            !term.is_empty() && field.contains(&term)
        })
}

#[cfg(test)]
mod contains_tests {
    use super::*;

    fn candidate_plan() -> JsonValue {
        json!({
            "plan_id": "PR-7",
            "title": "Release readiness",
            "status": "active",
            "head_artifact_path": "docs/stale.md",
            "head_revision": {
                "artifact_path": "docs/sprints/runtime.md",
                "artifact_selector": "runtime/root",
                "items": [{
                    "plan_item_ref": "RUN-01",
                    "text": "Verify worker drain",
                    "heading_path": ["Runtime", "Queue"]
                }]
            },
            "summary": "unrelated-secret-metadata"
        })
    }

    #[test]
    fn plan_contains_query_normalizes_and_stably_deduplicates_terms() {
        assert_eq!(
            normalize_plan_contains_query(Some(" RUN-01,worker,run-01, ,QUEUE ")),
            vec!["run-01", "worker", "queue"]
        );
        assert!(normalize_plan_contains_query(None).is_empty());
    }

    #[test]
    fn plan_contains_matcher_uses_only_documented_candidate_fields() {
        let plan = candidate_plan();
        assert!(plan_matches_contains_terms(&plan, &[]));
        for term in [
            "release",
            "sprints/runtime",
            "runtime/root",
            "run-01",
            "worker drain",
            "queue",
        ] {
            assert!(
                plan_matches_contains_terms(&plan, &[term.to_string()]),
                "expected {term:?} to match"
            );
        }
        assert!(!plan_matches_contains_terms(
            &plan,
            &["unrelated-secret-metadata".to_string()]
        ));
        assert!(!plan_matches_contains_terms(
            &plan,
            &["docs/stale.md".to_string()]
        ));
    }

    #[test]
    fn historical_plan_statuses_are_not_active_candidates() {
        let mut plan = candidate_plan();
        assert!(!plan_is_historical(&plan));
        plan["status"] = json!("archived");
        assert!(plan_is_historical(&plan));
        plan["status"] = json!("SUPERSEDED");
        assert!(plan_is_historical(&plan));
    }
}
