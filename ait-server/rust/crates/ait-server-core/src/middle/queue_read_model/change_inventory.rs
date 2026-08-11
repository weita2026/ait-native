use super::filters::repo_matches;
use super::gate_state::{
    attestation_summary, freshness, policy_summary, review_summary, task_focus_candidate,
};
use super::helpers::*;
use super::index::QueueIndex;
use super::*;

pub(super) fn change_inventory(input: &QueueReadModelInput, index: &QueueIndex<'_>) -> JsonValue {
    let mut items = input
        .changes
        .iter()
        .filter(|change| repo_matches(input.repo_name.as_deref(), *change))
        .filter(|change| {
            !matches!(
                object_text(change, "status").unwrap_or_default().as_str(),
                CHANGE_STATUS_LANDED | CHANGE_STATUS_ARCHIVED
            )
        })
        .map(|change| {
            let patchset = index.current_patchset(change);
            let patchset_id = patchset.and_then(|row| object_text(row, "patchset_id"));
            let review = review_summary(change, patchset_id.as_deref(), index);
            let policy = policy_summary(patchset_id.as_deref(), index);
            let attestation = attestation_summary(patchset_id.as_deref(), index);
            let freshness = freshness(change, patchset, index);
            let candidate = task_focus_candidate(
                change,
                patchset,
                &review,
                &policy,
                attestation.as_ref(),
                &freshness,
            );
            json!({
                "change_id": object_text(change, "change_id"),
                "task_id": object_text(change, "task_id"),
                "title": object_text(change, "title"),
                "repo_name": object_text(change, "repo_name"),
                "status": object_text(change, "status"),
                "patchset_id": patchset_id,
                "reason": candidate.get("reason").and_then(value_to_text),
                "action": candidate.get("action").and_then(value_to_text),
                "primary_gate": candidate.get("primary_gate").cloned().unwrap_or(JsonValue::Null),
                "updated_at": object_text(change, "updated_at"),
            })
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        value_text(right, "updated_at").cmp(&value_text(left, "updated_at"))
    });
    json!({
        "items": items,
        "count": items.len(),
        "filters": {
            "repo_name": input.repo_name,
            "exclude_statuses": [CHANGE_STATUS_ARCHIVED, CHANGE_STATUS_LANDED],
        },
    })
}
