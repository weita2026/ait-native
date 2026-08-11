use super::helpers::*;
use super::index::{patchset_number, QueueIndex};
use super::*;

#[derive(Debug, Clone)]
pub(super) struct ReviewSummary {
    pub(super) approvals: usize,
    pub(super) blocking: usize,
    pub(super) comments: usize,
    pub(super) review_requests: Vec<JsonValue>,
}

pub(super) fn review_summary(
    change: &JsonMap<String, JsonValue>,
    patchset_id: Option<&str>,
    index: &QueueIndex<'_>,
) -> ReviewSummary {
    let change_key = index.change_key(change);
    let mut approvals = HashSet::new();
    let mut blocking = 0;
    let mut comments = 0;
    for review in index
        .reviews_by_change_key
        .get(change_key.as_deref().unwrap_or_default())
        .into_iter()
        .flatten()
    {
        if patchset_id.is_some() && object_text(review, "patchset_id").as_deref() != patchset_id {
            continue;
        }
        let action = object_text(review, "action")
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(action.as_str(), "approve" | "task_approve" | "team_approve") {
            approvals
                .insert(object_text(review, "reviewer").unwrap_or_else(|| "anonymous".to_string()));
        }
        if matches!(action.as_str(), "request_changes" | "task_request_changes")
            || review
                .get("blocking")
                .and_then(JsonValue::as_bool)
                .unwrap_or(false)
            || review
                .get("blocking")
                .and_then(JsonValue::as_i64)
                .unwrap_or(0)
                > 0
        {
            blocking += 1;
        }
        if matches!(
            action.as_str(),
            "comment" | "task_comment" | "code_review_summary"
        ) {
            comments += 1;
        }
    }
    let review_requests = index
        .review_requests_by_change_key
        .get(change_key.as_deref().unwrap_or_default())
        .into_iter()
        .flatten()
        .filter(|request| {
            patchset_id.is_none() || object_text(request, "patchset_id").as_deref() == patchset_id
        })
        .map(|request| JsonValue::Object((*request).clone()))
        .collect();
    ReviewSummary {
        approvals: approvals.len(),
        blocking,
        comments,
        review_requests,
    }
}

pub(super) fn policy_summary(patchset_id: Option<&str>, index: &QueueIndex<'_>) -> JsonValue {
    let Some(patchset_id) = patchset_id else {
        return json!({"decision": "pending", "checks": []});
    };
    let Some(row) = index.policies_by_patchset.get(patchset_id) else {
        return json!({"patchset_id": patchset_id, "decision": "pending", "checks": []});
    };
    let checks = parse_json_field(row, "checks_json").unwrap_or_else(|| json!([]));
    let effective_requirements = parse_json_field(row, "effective_requirements_json")
        .unwrap_or_else(|| {
            row.get("effective_requirements")
                .cloned()
                .unwrap_or_else(|| json!({}))
        });
    json!({
        "patchset_id": patchset_id,
        "decision": object_text(row, "decision").unwrap_or_else(|| "pending".to_string()),
        "checks": checks,
        "effective_requirements": effective_requirements,
    })
}

pub(super) fn attestation_summary(
    patchset_id: Option<&str>,
    index: &QueueIndex<'_>,
) -> Option<JsonValue> {
    let patchset_id = patchset_id?;
    let row = index.attestations_by_patchset.get(patchset_id)?;
    let evaluation_summary =
        parse_json_field(row, "evaluation_summary_json").unwrap_or_else(|| {
            row.get("evaluation_summary")
                .cloned()
                .unwrap_or_else(|| json!({}))
        });
    let provenance_summary =
        parse_json_field(row, "provenance_summary_json").unwrap_or_else(|| {
            row.get("provenance_summary")
                .cloned()
                .unwrap_or_else(|| json!({}))
        });
    Some(json!({
        "patchset_id": patchset_id,
        "author_mode": object_text(row, "author_mode"),
        "evaluation_summary": evaluation_summary,
        "provenance_summary": provenance_summary,
        "updated_at": object_text(row, "updated_at"),
    }))
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
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    if !required {
        return "not_required".to_string();
    }
    attestation
        .and_then(|row| row.get("evaluation_summary"))
        .and_then(JsonValue::as_object)
        .and_then(|summary| summary.get(key))
        .and_then(value_to_text)
        .unwrap_or_else(|| "pending".to_string())
}

pub(super) fn freshness(
    change: &JsonMap<String, JsonValue>,
    patchset: Option<&JsonMap<String, JsonValue>>,
    index: &QueueIndex<'_>,
) -> JsonValue {
    let Some(patchset) = patchset else {
        return json!({"base_is_fresh": false, "current_base_head": null});
    };
    let repo_name = object_text(change, "repo_name").unwrap_or_default();
    let base_line = object_text(change, "base_line").unwrap_or_else(|| "main".to_string());
    let current_base_head = index
        .refs_by_repo_line
        .get(&(repo_name, base_line))
        .cloned();
    let base_snapshot_id = object_text(patchset, "base_snapshot_id");
    json!({
        "base_is_fresh": current_base_head.is_none() || current_base_head == base_snapshot_id,
        "current_base_head": current_base_head,
    })
}

pub(super) fn ci_summary(
    change: &JsonMap<String, JsonValue>,
    patchset: Option<&JsonMap<String, JsonValue>>,
    review: &ReviewSummary,
    policy: &JsonValue,
    freshness: &JsonValue,
    tests_state: &str,
    index: &QueueIndex<'_>,
) -> Option<JsonValue> {
    let patchset = patchset?;
    let patchset_id = object_text(patchset, "patchset_id")?;
    let ci_row = index.ci_by_patchset.get(&patchset_id);
    let tests_status = ci_row
        .and_then(|row| object_text(row, "tests_status"))
        .unwrap_or_else(|| tests_state.to_string());
    Some(json!({
        "patchset_id": patchset_id,
        "patchset_number": patchset_number(patchset),
        "tests_status": tests_status,
        "remote_land_gate": remote_land_gate_state(change, patchset, review, policy, freshness, &tests_status),
    }))
}

fn remote_land_gate_state(
    change: &JsonMap<String, JsonValue>,
    _patchset: &JsonMap<String, JsonValue>,
    review: &ReviewSummary,
    policy: &JsonValue,
    freshness: &JsonValue,
    tests_state: &str,
) -> &'static str {
    if review.blocking > 0
        || !freshness
            .get("base_is_fresh")
            .and_then(JsonValue::as_bool)
            .unwrap_or(true)
    {
        return "blocked";
    }
    let decision = policy
        .get("decision")
        .and_then(value_to_text)
        .unwrap_or_else(|| "pending".to_string())
        .to_ascii_lowercase();
    let change_status = object_text(change, "status")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if decision == "pass" && REVIEWABLE_CHANGE_STATES.contains(&change_status.as_str()) {
        return "pass";
    }
    if matches!(
        decision.as_str(),
        "hard_fail" | "soft_fail" | "fail" | "failed"
    ) || tests_state == "fail"
    {
        return "blocked";
    }
    "pending"
}

pub(super) fn task_focus_candidate(
    change: &JsonMap<String, JsonValue>,
    patchset: Option<&JsonMap<String, JsonValue>>,
    review: &ReviewSummary,
    policy: &JsonValue,
    attestation: Option<&JsonValue>,
    freshness: &JsonValue,
) -> JsonValue {
    let tests_state = effective_validation_state(policy, attestation, "tests", "require_tests");
    let decision = policy
        .get("decision")
        .and_then(value_to_text)
        .unwrap_or_else(|| "pending".to_string());
    let change_status = object_text(change, "status").unwrap_or_default();
    let (rank, action, reason, primary_gate) = if change_status == "draft" && patchset.is_none() {
        (
            0,
            "publish_patchset",
            "No published patchset exists yet.",
            None,
        )
    } else if review.blocking > 0 {
        (
            1,
            "address_blocking_review",
            "Blocking review feedback is recorded on this change.",
            Some("review"),
        )
    } else if patchset.is_none() {
        (
            2,
            "publish_patchset",
            "Publish a patchset so the task has a reviewable surface.",
            None,
        )
    } else if attestation.is_none() {
        (
            3,
            "record_attestation",
            "Attestation is missing for the current patchset.",
            Some("attestation"),
        )
    } else if !matches!(tests_state.as_str(), "pass" | "not_required") {
        (
            4,
            "complete_validation",
            "Tests are pending for the current patchset.",
            Some("ci"),
        )
    } else if !freshness
        .get("base_is_fresh")
        .and_then(JsonValue::as_bool)
        .unwrap_or(true)
    {
        (
            5,
            "refresh_patchset",
            "The base line moved after this patchset was published.",
            Some("freshness"),
        )
    } else if decision != "pass" {
        (
            7,
            "satisfy_policy",
            "Policy evaluation is still pending.",
            Some("policy"),
        )
    } else if matches!(
        change_status.as_str(),
        "review" | "gated" | "approved" | "landable"
    ) {
        (8, "land_change", "This change is ready for landing.", None)
    } else {
        (
            10,
            "inspect_change",
            "Inspect the linked change from the task page.",
            None,
        )
    };
    json!({
        "rank": rank,
        "action": action,
        "reason": reason,
        "primary_gate": primary_gate,
    })
}
