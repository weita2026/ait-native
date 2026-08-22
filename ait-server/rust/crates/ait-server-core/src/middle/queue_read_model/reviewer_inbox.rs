use super::filters::repo_matches;
use super::gate_state::{attestation_summary, freshness, policy_summary, review_summary};
use super::helpers::*;
use super::index::{patchset_number, QueueIndex};
use super::*;

pub(super) fn reviewer_inbox(input: &QueueReadModelInput, index: &QueueIndex<'_>) -> JsonValue {
    let mut items = input
        .changes
        .iter()
        .filter(|change| repo_matches(input.repo_name.as_deref(), change))
        .filter(|change| {
            object_text(change, "status")
                .map(|status| REVIEWABLE_CHANGE_STATES.contains(&status.as_str()))
                .unwrap_or(false)
        })
        .map(|change| {
            let patchset = index.current_patchset(change);
            let patchset_id = patchset.and_then(|row| object_text(row, "patchset_id"));
            let review = review_summary(change, patchset_id.as_deref(), index);
            let policy = policy_summary(patchset_id.as_deref(), index);
            let attestation = attestation_summary(patchset_id.as_deref(), index);
            let freshness = freshness(change, patchset, index);
            json!({
                "change_id": object_text(change, "change_id"),
                "title": object_text(change, "title"),
                "repo": object_text(change, "repo_name"),
                "base_line": object_text(change, "base_line"),
                "task": {"task_id": object_text(change, "task_id")},
                "change_status": object_text(change, "status"),
                "current_patchset": {
                    "patchset_id": patchset_id,
                    "patchset_number": patchset.map(patchset_number).unwrap_or(0),
                },
                "review_state": {
                    "approvals": review.approvals,
                    "blocking": review.blocking,
                    "comments": review.comments,
                },
                "policy_state": {
                    "decision": policy.get("decision").cloned().unwrap_or_else(|| json!("pending")),
                },
                "freshness": freshness,
                "attestation": {
                    "completeness": if attestation.is_some() {"summary_present"} else {"missing"},
                },
                "requested_groups": review.review_requests,
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
        "filters": {"repo_name": input.repo_name},
    })
}
