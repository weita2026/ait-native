use std::collections::BTreeSet;

use crate::plan_items::{extract_plan_items, list_plan_section_refs, PlanItem, PlanSectionRef};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedPlan {
    pub markdown_text: String,
    pub plan_ref_count: usize,
    pub item_count: usize,
    pub plan_refs: Vec<PlanSectionRef>,
    pub items: Vec<PlanItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRefIdentityPayload {
    pub plan_ref_count: usize,
    pub plan_refs: Vec<PlanSectionRef>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncPruneDecisionPayload {
    pub scope: String,
    pub tracked_artifact_count: usize,
    pub synced_artifact_count: usize,
    pub retained_paths: Vec<String>,
    pub prune_paths: Vec<String>,
    pub prune_count: usize,
}

pub fn parse_plan_markdown(markdown_text: Option<&str>) -> ParsedPlan {
    let markdown = markdown_text.unwrap_or("").to_string();
    let plan_refs = list_plan_section_refs(Some(markdown.as_str()));
    let items = extract_plan_items(Some(markdown.as_str()));
    ParsedPlan {
        markdown_text: markdown,
        plan_ref_count: plan_refs.len(),
        item_count: items.len(),
        plan_refs,
        items,
    }
}

pub fn extract_plan_refs(parsed_plan: &ParsedPlan) -> PlanRefIdentityPayload {
    PlanRefIdentityPayload {
        plan_ref_count: parsed_plan.plan_ref_count,
        plan_refs: parsed_plan.plan_refs.clone(),
    }
}

pub fn compute_sync_prune_decisions(
    scope: Option<&str>,
    tracked_artifacts: &[String],
    synced_artifacts: &[String],
) -> Result<SyncPruneDecisionPayload, String> {
    let normalized_scope = normalize_scope(scope)?;
    let tracked = normalize_artifact_paths(tracked_artifacts)?;
    let synced = normalize_artifact_paths(synced_artifacts)?;

    let synced_set: BTreeSet<String> = synced.iter().cloned().collect();
    let prune_paths: Vec<String> = tracked
        .iter()
        .filter(|path| !synced_set.contains(path.as_str()))
        .cloned()
        .collect();
    let retained_paths: Vec<String> = tracked
        .iter()
        .filter(|path| synced_set.contains(path.as_str()))
        .cloned()
        .collect();

    Ok(SyncPruneDecisionPayload {
        scope: normalized_scope,
        tracked_artifact_count: tracked.len(),
        synced_artifact_count: synced.len(),
        retained_paths,
        prune_paths: prune_paths.clone(),
        prune_count: prune_paths.len(),
    })
}

fn normalize_scope(value: Option<&str>) -> Result<String, String> {
    let normalized = value.unwrap_or("").trim().to_lowercase();
    match normalized.as_str() {
        "file" | "directory" => Ok(normalized),
        _ => Err(format!(
            "Unsupported sync prune scope: {}. Expected one of: file, directory.",
            if normalized.is_empty() {
                "<empty>"
            } else {
                normalized.as_str()
            }
        )),
    }
}

fn normalize_artifact_paths(values: &[String]) -> Result<Vec<String>, String> {
    let mut normalized = BTreeSet::new();
    for value in values {
        let text = value.trim();
        if text.is_empty() {
            return Err("Artifact paths must be non-empty strings.".to_string());
        }
        normalized.insert(text.to_string());
    }
    Ok(normalized.into_iter().collect())
}

#[cfg(test)]
mod tests;
