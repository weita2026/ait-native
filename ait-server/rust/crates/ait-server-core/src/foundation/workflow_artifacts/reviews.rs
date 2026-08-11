use super::*;

pub const CODE_REVIEW_SUMMARY_ACTION: &str = "code_review_summary";
pub const TASK_REVIEW_APPROVE_ACTION: &str = "task_approve";
pub const TASK_REVIEW_REQUEST_CHANGES_ACTION: &str = "task_request_changes";
pub const TASK_REVIEW_COMMENT_ACTION: &str = "task_comment";
pub const TASK_REVIEW_DEFER_ACTION: &str = "task_defer";
pub const TEAM_REVIEW_APPROVE_ACTION: &str = "approve";
pub const TEAM_REVIEW_REQUEST_CHANGES_ACTION: &str = "request_changes";
pub const TEAM_REVIEW_COMMENT_ACTION: &str = "comment";

pub fn review_decision_lane(action: &str) -> Option<&'static str> {
    match action {
        TASK_REVIEW_APPROVE_ACTION
        | TASK_REVIEW_REQUEST_CHANGES_ACTION
        | TASK_REVIEW_DEFER_ACTION => Some("task"),
        TEAM_REVIEW_APPROVE_ACTION | TEAM_REVIEW_REQUEST_CHANGES_ACTION | "defer" => Some("team"),
        _ => None,
    }
}

pub fn review_summary_from_rows(
    reviews: &[JsonMap<String, JsonValue>],
    patchset_id: Option<&str>,
) -> JsonMap<String, JsonValue> {
    let mut ordered = reviews
        .iter()
        .enumerate()
        .filter(|(_, review)| review_matches_patchset(review, patchset_id))
        .collect::<Vec<_>>();
    ordered.sort_by(|(left_index, left), (right_index, right)| {
        match (review_id_order_value(left), review_id_order_value(right)) {
            (Some(left_id), Some(right_id)) => {
                left_id.cmp(&right_id).then(left_index.cmp(right_index))
            }
            _ => left_index.cmp(right_index),
        }
    });

    let mut latest_decision_by_reviewer_lane: BTreeMap<(String, &'static str), ReviewDecision> =
        BTreeMap::new();
    let mut blocking_count = 0usize;
    let mut comment_count = 0usize;
    let mut structured_code_review_summary_reviewers = BTreeSet::new();
    let mut code_review_summary_count = 0usize;

    for (_, review) in &ordered {
        let action = optional_text(review.get("action")).unwrap_or_default();
        if let Some(decision_lane) = review_decision_lane(&action) {
            latest_decision_by_reviewer_lane.insert(
                (raw_text(review.get("reviewer")), decision_lane),
                ReviewDecision {
                    reviewer: raw_text(review.get("reviewer")),
                    normalized_reviewer: normalized_reviewer(review.get("reviewer")),
                    action: action.clone(),
                },
            );
        }
        if matches!(
            action.as_str(),
            TEAM_REVIEW_REQUEST_CHANGES_ACTION | TASK_REVIEW_REQUEST_CHANGES_ACTION
        ) || truthy(review.get("blocking"))
        {
            blocking_count += 1;
        }
        if matches!(
            action.as_str(),
            TEAM_REVIEW_COMMENT_ACTION | TASK_REVIEW_COMMENT_ACTION
        ) {
            comment_count += 1;
        }
        if action == CODE_REVIEW_SUMMARY_ACTION {
            comment_count += 1;
            if is_structured_code_review_summary_text(review.get("comment")) {
                code_review_summary_count += 1;
                if let Some(reviewer) = normalized_reviewer(review.get("reviewer")) {
                    structured_code_review_summary_reviewers.insert(reviewer);
                }
            }
        }
    }

    let latest_decisions = latest_decision_by_reviewer_lane
        .values()
        .collect::<Vec<_>>();
    let task_approval_count = latest_decisions
        .iter()
        .filter(|decision| decision.action == TASK_REVIEW_APPROVE_ACTION)
        .count();
    let team_approval_count = latest_decisions
        .iter()
        .filter(|decision| decision.action == TEAM_REVIEW_APPROVE_ACTION)
        .count();
    let approval_reviewers = latest_decisions
        .iter()
        .filter(|decision| {
            matches!(
                decision.action.as_str(),
                TASK_REVIEW_APPROVE_ACTION | TEAM_REVIEW_APPROVE_ACTION
            )
        })
        .map(|decision| decision.reviewer.clone())
        .collect::<BTreeSet<_>>();
    let human_approval_reviewers = latest_decisions
        .iter()
        .filter(|decision| {
            matches!(
                decision.action.as_str(),
                TASK_REVIEW_APPROVE_ACTION | TEAM_REVIEW_APPROVE_ACTION
            )
        })
        .filter_map(|decision| human_reviewer(decision.normalized_reviewer.as_deref()))
        .collect::<BTreeSet<_>>();
    let human_task_approval_reviewers = latest_decisions
        .iter()
        .filter(|decision| decision.action == TASK_REVIEW_APPROVE_ACTION)
        .filter_map(|decision| human_reviewer(decision.normalized_reviewer.as_deref()))
        .collect::<BTreeSet<_>>();
    let independent_human_approval_count = human_approval_reviewers
        .difference(&structured_code_review_summary_reviewers)
        .count();
    let independent_task_approval_count = human_task_approval_reviewers
        .difference(&structured_code_review_summary_reviewers)
        .count();

    let mut out = JsonMap::new();
    insert_count(&mut out, "approval_count", approval_reviewers.len());
    insert_count(&mut out, "task_approval_count", task_approval_count);
    insert_count(&mut out, "team_approval_count", team_approval_count);
    insert_count(
        &mut out,
        "human_approval_count",
        human_approval_reviewers.len(),
    );
    insert_count(
        &mut out,
        "independent_human_approval_count",
        independent_human_approval_count,
    );
    insert_count(
        &mut out,
        "human_task_approval_count",
        human_task_approval_reviewers.len(),
    );
    insert_count(
        &mut out,
        "independent_task_approval_count",
        independent_task_approval_count,
    );
    insert_count(
        &mut out,
        "code_review_summary_reviewer_count",
        structured_code_review_summary_reviewers.len(),
    );
    insert_count(&mut out, "blocking_count", blocking_count);
    insert_count(&mut out, "comment_count", comment_count);
    insert_count(
        &mut out,
        "code_review_summary_count",
        code_review_summary_count,
    );
    insert_count(&mut out, "review_count", ordered.len());

    insert_count(&mut out, "approvals", approval_reviewers.len());
    insert_count(&mut out, "task_approvals", task_approval_count);
    insert_count(&mut out, "team_approvals", team_approval_count);
    insert_count(&mut out, "human_approvals", human_approval_reviewers.len());
    insert_count(
        &mut out,
        "independent_human_approvals",
        independent_human_approval_count,
    );
    insert_count(
        &mut out,
        "human_task_approvals",
        human_task_approval_reviewers.len(),
    );
    insert_count(
        &mut out,
        "independent_task_approvals",
        independent_task_approval_count,
    );
    insert_count(
        &mut out,
        "code_review_summary_reviewers",
        structured_code_review_summary_reviewers.len(),
    );
    insert_count(&mut out, "blocking", blocking_count);
    insert_count(&mut out, "comments", comment_count);
    insert_count(&mut out, "code_review_summaries", code_review_summary_count);
    out
}

pub fn is_structured_code_review_summary_text(value: Option<&JsonValue>) -> bool {
    let Some(text) = optional_text(value) else {
        return false;
    };
    let lower = text.to_lowercase();
    [
        "reviewed files",
        "findings",
        "risks",
        "tests",
        "recommendation",
    ]
    .iter()
    .all(|section| lower.contains(section))
}

struct ReviewDecision {
    reviewer: String,
    normalized_reviewer: Option<String>,
    action: String,
}

fn review_matches_patchset(review: &JsonMap<String, JsonValue>, patchset_id: Option<&str>) -> bool {
    patchset_id.is_none() || optional_text(review.get("patchset_id")).as_deref() == patchset_id
}

fn review_id_order_value(review: &JsonMap<String, JsonValue>) -> Option<i64> {
    optional_i64(review.get("review_id")).ok().flatten()
}

fn raw_text(value: Option<&JsonValue>) -> String {
    match value {
        Some(JsonValue::String(text)) => text.clone(),
        Some(JsonValue::Bool(true)) => "True".to_string(),
        Some(JsonValue::Bool(false)) | Some(JsonValue::Null) | None => String::new(),
        Some(JsonValue::Number(number)) => number.to_string(),
        Some(JsonValue::Array(_)) | Some(JsonValue::Object(_)) => value.unwrap().to_string(),
    }
}

fn normalized_reviewer(value: Option<&JsonValue>) -> Option<String> {
    optional_text(value).map(|text| text.to_lowercase())
}

fn human_reviewer(value: Option<&str>) -> Option<String> {
    match value {
        Some(value) if value != "anonymous" => Some(value.to_string()),
        _ => None,
    }
}

fn insert_count(out: &mut JsonMap<String, JsonValue>, key: &str, value: usize) {
    out.insert(key.to_string(), json!(value));
}
