use crate::binary_db::StoreResult;

use super::views::{PlanHeadView, PlanSummaryView};

#[derive(Clone, Copy, Debug)]
pub struct PlanHeadScanFilter<'a> {
    pub repo_name: Option<&'a str>,
    pub artifact_path: Option<&'a str>,
    pub contains_terms: &'a [String],
    pub active_only: bool,
}

impl<'a> PlanHeadScanFilter<'a> {
    pub const fn all() -> Self {
        Self {
            repo_name: None,
            artifact_path: None,
            contains_terms: &[],
            active_only: false,
        }
    }

    pub const fn active() -> Self {
        Self {
            repo_name: None,
            artifact_path: None,
            contains_terms: &[],
            active_only: true,
        }
    }
}
pub(super) fn sort_plan_summaries(plans: &mut [PlanSummaryView]) {
    plans.sort_by(|left, right| {
        right
            .record
            .updated_at_s
            .cmp(&left.record.updated_at_s)
            .then_with(|| right.record.created_at_s.cmp(&left.record.created_at_s))
            .then_with(|| right.plan_index.cmp(&left.plan_index))
    });
}

pub(super) fn sort_plan_heads(plans: &mut [PlanHeadView]) {
    plans.sort_by(|left, right| {
        right
            .record
            .updated_at_s
            .cmp(&left.record.updated_at_s)
            .then_with(|| right.record.created_at_s.cmp(&left.record.created_at_s))
            .then_with(|| right.plan_index.cmp(&left.plan_index))
    });
}

pub(super) fn plan_head_matches_contains_terms(
    view: &PlanHeadView,
    contains_terms: &[String],
) -> StoreResult<bool> {
    if contains_terms.is_empty() {
        return Ok(true);
    }
    let mut fields = Vec::new();
    fields.push(normalize_search_field(view.payload.title_text()?));
    if let Some(revision) = view.head_revision.as_ref() {
        fields.push(normalize_search_field(
            revision.payload.artifact_path_text()?,
        ));
        fields.push(normalize_search_field(
            revision.payload.artifact_selector_text()?,
        ));
        for item in &revision.items {
            fields.push(normalize_search_field(item.payload.plan_item_ref_text()?));
            fields.push(normalize_search_field(item.payload.text()?));
            fields.extend(
                item.payload
                    .heading_path
                    .iter()
                    .cloned()
                    .map(normalize_search_field),
            );
        }
    }
    Ok(contains_terms.iter().any(|term| {
        let needle = normalize_search_field(term.clone());
        !needle.is_empty()
            && fields
                .iter()
                .any(|field| !field.is_empty() && field.contains(&needle))
    }))
}

fn normalize_search_field(value: String) -> String {
    value.trim().to_ascii_lowercase()
}
