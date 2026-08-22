use super::change_inventory::change_inventory;
use super::filters::*;
use super::gate_state::*;
use super::helpers::*;
use super::index::{patchset_number, QueueIndex};
use super::reviewer_inbox::reviewer_inbox;
use super::*;

pub fn queue_summary_read_model(input: &QueueReadModelInput) -> Result<JsonValue, String> {
    let normalized_status = normalize_task_filter(&input.status)?;
    let selected_tasks = selected_tasks(input, &normalized_status);
    let selected_task_ids = selected_tasks
        .iter()
        .filter_map(|task| object_text(task, "task_id"))
        .collect::<HashSet<_>>();
    let queue_changes = input
        .changes
        .iter()
        .filter(|change| repo_matches(input.repo_name.as_deref(), change))
        .filter(|change| {
            object_text(change, "task_id")
                .map(|task_id| selected_task_ids.contains(&task_id))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let index = QueueIndex::new(input);

    let mut items = selected_tasks
        .iter()
        .map(|task| {
            let task_id = object_text(task, "task_id").unwrap_or_default();
            let mut task_changes = queue_changes
                .iter()
                .filter(|change| {
                    object_text(change, "task_id").as_deref() == Some(task_id.as_str())
                })
                .copied()
                .collect::<Vec<_>>();
            task_changes.sort_by(|left, right| {
                object_text(right, "updated_at").cmp(&object_text(left, "updated_at"))
            });
            task_queue_entry(task, &task_changes, &index)
        })
        .collect::<Vec<_>>();
    items.sort_by(|left, right| {
        let left_priority =
            workflow_priority(value_text_path(left, &["workflow", "state"]).as_deref());
        let right_priority =
            workflow_priority(value_text_path(right, &["workflow", "state"]).as_deref());
        left_priority
            .cmp(&right_priority)
            .then_with(|| value_text(right, "updated_at").cmp(&value_text(left, "updated_at")))
    });

    let mut summary = json!({
        "active": 0,
        "completed": 0,
        "abandoned": 0,
        "later_promotion_excluded": 0,
        "canceled": 0,
        "attention_required": 0,
        "ready_to_land": 0,
        "ready_to_complete": 0,
    });
    for item in &items {
        let task_status = value_text_path(item, &["task", "status"]).unwrap_or_default();
        increment_summary(&mut summary, task_status.as_str());
        let workflow_state = value_text_path(item, &["workflow", "state"]).unwrap_or_default();
        increment_summary(&mut summary, workflow_state.as_str());
    }

    let queue_task_ids = selected_tasks
        .iter()
        .filter_map(|task| object_text(task, "task_id"))
        .collect::<Vec<_>>();
    let mut payload = json!({
        "task_queue": {
            "items": items,
            "count": selected_tasks.len(),
            "filters": {
                "repo_name": input.repo_name,
                "status": normalized_status,
            },
            "summary": summary,
        },
        "reviewer_inbox": reviewer_inbox(input, &index),
        "query_plan": {
            "task_statuses": task_statuses_for_filter(&normalized_status),
            "queue_change_task_ids": queue_task_ids,
            "queue_change_scope": "selected_tasks_only",
            "change_inventory_exclude_statuses": [CHANGE_STATUS_ARCHIVED, CHANGE_STATUS_LANDED],
            "input_counts": {
                "tasks": input.tasks.len(),
                "changes": input.changes.len(),
                "patchsets": input.patchsets.len(),
                "reviews": input.reviews.len(),
                "review_requests": input.review_requests.len(),
                "attestations": input.attestations.len(),
                "policy_decisions": input.policy_decisions.len(),
                "refs": input.refs.len(),
                "ci_statuses": input.ci_statuses.len(),
            },
            "selected_counts": {
                "tasks": selected_tasks.len(),
                "queue_changes": queue_changes.len(),
            },
        },
    });

    if input.include_all_changes {
        payload["change_inventory"] = change_inventory(input, &index);
    }
    Ok(payload)
}

fn task_queue_entry(
    task: &JsonMap<String, JsonValue>,
    task_changes: &[&JsonMap<String, JsonValue>],
    index: &QueueIndex<'_>,
) -> JsonValue {
    let mut open_changes = 0;
    let mut reviewable_changes = 0;
    let mut landed_changes = 0;
    let mut patchset_ids = HashSet::new();
    let mut blocking_reviews = 0;
    let mut missing_attestation = 0;
    let mut tests_pending = 0;
    let mut stale_base = 0;
    let mut policy_pending = 0;
    let mut ready_to_land = 0;
    let mut focus_change: Option<JsonValue> = None;

    for change in task_changes {
        let patchset = index.current_patchset(change);
        let patchset_id = patchset.and_then(|row| object_text(row, "patchset_id"));
        let review = review_summary(change, patchset_id.as_deref(), index);
        let policy = policy_summary(patchset_id.as_deref(), index);
        let attestation = attestation_summary(patchset_id.as_deref(), index);
        let freshness = freshness(change, patchset, index);
        let tests_state =
            effective_validation_state(&policy, attestation.as_ref(), "tests", "require_tests");
        let ci = ci_summary(
            change,
            patchset,
            &review,
            &policy,
            &freshness,
            &tests_state,
            index,
        );
        let change_status = object_text(change, "status").unwrap_or_default();
        let actionable = !matches!(
            change_status.as_str(),
            CHANGE_STATUS_LANDED | CHANGE_STATUS_ARCHIVED
        );
        if actionable {
            open_changes += 1;
        }
        if change_status == CHANGE_STATUS_LANDED {
            landed_changes += 1;
        }
        if REVIEWABLE_CHANGE_STATES.contains(&change_status.as_str()) {
            reviewable_changes += 1;
        }
        if let Some(patchset_id) = patchset_id.as_ref() {
            patchset_ids.insert(patchset_id.clone());
        }
        if actionable && review.blocking > 0 {
            blocking_reviews += 1;
        }
        if actionable && patchset.is_some() && attestation.is_none() {
            missing_attestation += 1;
        }
        if actionable
            && patchset.is_some()
            && !matches!(tests_state.as_str(), "pass" | "not_required")
        {
            tests_pending += 1;
        }
        if actionable
            && patchset.is_some()
            && !freshness
                .get("base_is_fresh")
                .and_then(JsonValue::as_bool)
                .unwrap_or(true)
        {
            stale_base += 1;
        }
        let decision = policy
            .get("decision")
            .and_then(value_to_text)
            .unwrap_or_else(|| "pending".to_string());
        if actionable && patchset.is_some() && decision != "pass" {
            policy_pending += 1;
        }
        if actionable
            && patchset.is_some()
            && decision == "pass"
            && review.blocking == 0
            && matches!(
                change_status.as_str(),
                "review" | "gated" | "approved" | "landable"
            )
        {
            ready_to_land += 1;
        }
        if actionable {
            let mut candidate = task_focus_candidate(
                change,
                patchset,
                &review,
                &policy,
                attestation.as_ref(),
                &freshness,
            );
            let rank = candidate
                .get("rank")
                .and_then(JsonValue::as_i64)
                .unwrap_or(99);
            candidate["change_id"] = json!(object_text(change, "change_id"));
            candidate["title"] = json!(object_text(change, "title"));
            candidate["status"] = json!(change_status);
            candidate["updated_at"] = json!(object_text(change, "updated_at"));
            candidate["patchset_id"] = json!(patchset_id);
            candidate["patchset_number"] = json!(patchset.map(patchset_number));
            candidate["policy_decision"] = json!(decision);
            candidate["tests"] = json!(tests_state);
            candidate["ci_summary"] = ci.unwrap_or(JsonValue::Null);
            let should_replace = focus_change
                .as_ref()
                .and_then(|existing| existing.get("rank"))
                .and_then(JsonValue::as_i64)
                .map(|existing_rank| rank < existing_rank)
                .unwrap_or(true);
            if should_replace {
                focus_change = Some(candidate);
            }
        }
    }

    let task_status = object_text(task, "status").unwrap_or_default();
    let total_changes = task_changes.len();
    let (workflow_state, workflow_reason) = if task_status == TASK_STATUS_COMPLETED {
        ("completed", "Task is already completed.".to_string())
    } else if matches!(
        task_status.as_str(),
        TASK_STATUS_ABANDONED | TASK_STATUS_LEGACY_CANCELED
    ) {
        ("abandoned", "Task is already abandoned.".to_string())
    } else if task_status == TASK_STATUS_LATER_PROMOTION_EXCLUDED {
        (
            TASK_STATUS_LATER_PROMOTION_EXCLUDED,
            "Task is already excluded from later promotion.".to_string(),
        )
    } else if total_changes == 0 {
        ("planning", "No linked changes exist yet.".to_string())
    } else if [
        blocking_reviews,
        missing_attestation,
        tests_pending,
        stale_base,
        policy_pending,
    ]
    .iter()
    .any(|value| *value > 0)
    {
        (
            "attention_required",
            focus_change
                .as_ref()
                .and_then(|candidate| candidate.get("reason"))
                .and_then(value_to_text)
                .unwrap_or_else(|| "At least one linked change needs attention.".to_string()),
        )
    } else if ready_to_land > 0 {
        (
            "ready_to_land",
            format!("{ready_to_land} linked change(s) can land now."),
        )
    } else if open_changes == 0 && landed_changes > 0 {
        (
            "ready_to_complete",
            "All linked changes are landed; the task can complete.".to_string(),
        )
    } else if reviewable_changes > 0 {
        (
            "in_review",
            format!("{reviewable_changes} linked change(s) are in review."),
        )
    } else {
        (
            "in_progress",
            format!("{open_changes} linked change(s) are still in progress."),
        )
    };

    let updated_at = task_changes
        .iter()
        .filter_map(|change| object_text(change, "updated_at"))
        .chain(object_text(task, "created_at"))
        .max();
    let next_action = task_next_action(task, workflow_state, focus_change.as_ref(), total_changes);
    json!({
        "task": task,
        "workflow": {"state": workflow_state, "reason": workflow_reason},
        "primary_gate": focus_change.as_ref().and_then(|candidate| candidate.get("primary_gate")).cloned().unwrap_or(JsonValue::Null),
        "primary_reason": focus_change.as_ref().and_then(|candidate| candidate.get("reason")).cloned().unwrap_or(JsonValue::Null),
        "changes": {
            "total": total_changes,
            "open": open_changes,
            "reviewable": reviewable_changes,
            "landed": landed_changes,
            "patchsets": patchset_ids.len(),
        },
        "attention": {
            "blocking_reviews": blocking_reviews,
            "missing_attestation": missing_attestation,
            "tests_pending": tests_pending,
            "stale_base": stale_base,
            "policy_pending": policy_pending,
            "ready_to_land": ready_to_land,
        },
        "focus_change": focus_change.clone().unwrap_or(JsonValue::Null),
        "ci_summary": focus_change.as_ref().and_then(|candidate| candidate.get("ci_summary")).cloned().unwrap_or(JsonValue::Null),
        "next_action": next_action,
        "updated_at": updated_at,
    })
}

fn task_next_action(
    task: &JsonMap<String, JsonValue>,
    workflow_state: &str,
    focus_change: Option<&JsonValue>,
    total_changes: usize,
) -> JsonValue {
    let task_status = object_text(task, "status").unwrap_or_default();
    if task_status == TASK_STATUS_COMPLETED
        || matches!(
            task_status.as_str(),
            TASK_STATUS_ABANDONED | TASK_STATUS_LEGACY_CANCELED
        )
        || task_status == TASK_STATUS_LATER_PROMOTION_EXCLUDED
    {
        return json!({
            "code": "open_history",
            "label": "Open task history",
            "detail": "Review the completed or closed task context.",
            "change_id": null,
        });
    }
    if total_changes == 0 {
        return json!({
            "code": "create_change",
            "label": "Open task and create the first change",
            "detail": "This task does not have any linked changes yet.",
            "change_id": null,
        });
    }
    if workflow_state == "ready_to_complete" {
        return json!({
            "code": "complete_task",
            "label": "Open task and complete it",
            "detail": "All linked changes are already landed.",
            "change_id": null,
        });
    }
    let Some(focus_change) = focus_change else {
        return json!({
            "code": "open_task",
            "label": "Open task",
            "detail": "Use the task as the main workflow home.",
            "change_id": null,
        });
    };
    json!({
        "code": focus_change.get("action").and_then(value_to_text),
        "label": "Open task",
        "detail": focus_change.get("reason").and_then(value_to_text),
        "change_id": focus_change.get("change_id").cloned().unwrap_or(JsonValue::Null),
    })
}
