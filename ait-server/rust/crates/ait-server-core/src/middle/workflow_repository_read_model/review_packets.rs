use super::helpers::*;
use super::*;

pub(super) fn task_review_packet(
    task: &JsonMap<String, JsonValue>,
    change_rows: &[JsonValue],
    summary: &JsonValue,
    aggregate_diff: &JsonValue,
) -> JsonValue {
    let change_count = value_int(summary, "change_count");
    let open_change_count = value_int(summary, "open_change_count");
    let landed_change_count = value_int(summary, "landed_change_count");
    let patchset_count = value_int(summary, "patchset_count");
    let shared_boundary_crossed = patchset_count > 0 || landed_change_count > 0;
    let mut unresolved_gaps = Vec::new();
    let mut landable_open_changes = 0_i64;

    if change_count == 0 {
        unresolved_gaps.push(json!("No linked change exists yet."));
    }
    for row in change_rows {
        let change_id = value_text_path(row, &["change", "change_id"]).unwrap_or_default();
        if matches!(
            value_text_path(row, &["change", "status"]).as_deref(),
            Some(CHANGE_STATUS_LANDED | CHANGE_STATUS_ARCHIVED)
        ) {
            continue;
        }
        if change_is_landable(row) {
            landable_open_changes += 1;
            continue;
        }
        if row.get("current_patchset").is_none()
            || row.get("current_patchset") == Some(&JsonValue::Null)
        {
            unresolved_gaps.push(json!(format!("{change_id} has no published patchset yet.")));
            continue;
        }
        if row.get("attestation_summary").is_none()
            || row.get("attestation_summary") == Some(&JsonValue::Null)
        {
            unresolved_gaps.push(json!(format!(
                "{change_id} is missing attestation evidence."
            )));
        }
        if !value_bool_path(row, &["freshness", "base_is_fresh"]) {
            unresolved_gaps.push(json!(format!(
                "{change_id} is based on a stale main snapshot."
            )));
        }
        let blocking = value_int_path(row, &["review_summary", "blocking"]);
        if blocking > 0 {
            unresolved_gaps.push(json!(format!("{change_id} has blocking review feedback.")));
        } else if value_int_path(row, &["review_summary", "approvals"]) < 1 {
            unresolved_gaps.push(json!(format!("{change_id} still needs human approval.")));
        }
        let policy_decision = value_text_path(row, &["policy_summary", "decision"])
            .unwrap_or_else(|| "pending".to_string())
            .to_ascii_lowercase();
        if policy_decision != "pass" {
            let missing =
                task_policy_missing_checks(row.get("policy_summary").unwrap_or(&JsonValue::Null));
            let detail = if missing.is_empty() {
                policy_decision
            } else {
                missing.join(", ")
            };
            unresolved_gaps.push(json!(format!(
                "{change_id} is still waiting on policy gates: {detail}."
            )));
        }
    }

    let (acceptance_status, completion_summary, suggested_next_action) = if change_count == 0 {
        (
            "defer",
            "No reviewable implementation has been linked to the task yet.".to_string(),
            "revise",
        )
    } else if !unresolved_gaps.is_empty() {
        (
            "needs_followup",
            format!(
                "{change_count} linked change(s) exist, but {} outcome or readiness gaps still block acceptance.",
                unresolved_gaps.len()
            ),
            "revise",
        )
    } else if open_change_count > 0 {
        (
            "complete",
            format!("{landable_open_changes} linked change(s) are ready for the final land path."),
            "land",
        )
    } else {
        (
            "complete",
            format!("All {landed_change_count} linked change(s) are already landed."),
            if object_text(task, "status").as_deref() == Some("completed") {
                "stop"
            } else {
                "land"
            },
        )
    };
    let effect_summary = if patchset_count > 0 {
        format!(
            "{} unique path(s), {} insertion(s), and {} deletion(s) are currently visible from the task surface.",
            value_int(aggregate_diff, "unique_paths"),
            value_int(aggregate_diff, "insertions"),
            value_int(aggregate_diff, "deletions")
        )
    } else {
        "No published aggregate diff is available yet; the task is still below the shared review surface.".to_string()
    };
    json!({
        "source": "derived_from_workflow_state",
        "intent_summary": object_text(task, "intent").unwrap_or_default(),
        "completion_summary": completion_summary,
        "effect_summary": effect_summary,
        "acceptance_status": acceptance_status,
        "unresolved_gaps": unresolved_gaps,
        "suggested_next_action": suggested_next_action,
        "operator_confirmation_required": object_text(task, "status").as_deref() != Some("completed"),
        "shared_boundary": {
            "crossed": shared_boundary_crossed,
            "state": if shared_boundary_crossed {"shared_workflow"} else {"local_execution"},
        },
    })
}

pub(super) fn code_review_packet(
    change_rows: &[JsonValue],
    summary: &JsonValue,
    aggregate_diff: &JsonValue,
) -> JsonValue {
    let change_count = value_int(summary, "change_count");
    let open_change_count = value_int(summary, "open_change_count");
    let file_entries = value_int(aggregate_diff, "file_entries");
    let touched_components = aggregate_diff
        .get("files")
        .and_then(JsonValue::as_array)
        .into_iter()
        .flatten()
        .filter_map(|file| value_text(file, "path"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(8)
        .map(JsonValue::String)
        .collect::<Vec<_>>();
    let mut regression_concerns = Vec::new();
    if change_count == 0 {
        regression_concerns.push(json!("No linked change exists yet."));
    }
    for row in change_rows {
        let change_id = value_text_path(row, &["change", "change_id"]).unwrap_or_default();
        if matches!(
            value_text_path(row, &["change", "status"]).as_deref(),
            Some(CHANGE_STATUS_LANDED | CHANGE_STATUS_ARCHIVED)
        ) {
            continue;
        }
        if row.get("current_patchset").is_none()
            || row.get("current_patchset") == Some(&JsonValue::Null)
        {
            regression_concerns.push(json!(format!("{change_id} has no published patchset.")));
            continue;
        }
        if row.get("attestation_summary").is_none()
            || row.get("attestation_summary") == Some(&JsonValue::Null)
        {
            regression_concerns.push(json!(format!(
                "{change_id} is missing attestation evidence."
            )));
        }
        if !value_bool_path(row, &["freshness", "base_is_fresh"]) {
            regression_concerns.push(json!(format!(
                "{change_id} is based on a stale base snapshot."
            )));
        }
        if value_int_path(row, &["review_summary", "blocking"]) > 0 {
            regression_concerns.push(json!(format!("{change_id} has blocking review feedback.")));
        }
        let decision = value_text_path(row, &["policy_summary", "decision"])
            .unwrap_or_default()
            .to_ascii_lowercase();
        if decision != "pass" {
            let missing =
                task_policy_missing_checks(row.get("policy_summary").unwrap_or(&JsonValue::Null));
            let detail = if missing.is_empty() {
                decision
            } else {
                missing.join(", ")
            };
            regression_concerns.push(json!(format!(
                "{change_id} is waiting on policy gates: {detail}."
            )));
        }
        for key in ["tests", "lint", "security", "license"] {
            let status = attestation_status(row.get("attestation_summary"), key);
            if matches!(
                status.as_str(),
                "fail" | "failed" | "hard_fail" | "soft_fail"
            ) {
                regression_concerns.push(json!(format!("{change_id} has failing {key} evidence.")));
            }
        }
    }

    let approvals: i64 = change_rows
        .iter()
        .map(|row| value_int_path(row, &["review_summary", "approvals"]))
        .sum();
    let blocking: i64 = change_rows
        .iter()
        .map(|row| value_int_path(row, &["review_summary", "blocking"]))
        .sum();
    let passing_policy_rows = change_rows
        .iter()
        .filter(|row| {
            value_text_path(row, &["policy_summary", "decision"]).as_deref() == Some("pass")
        })
        .count();
    let passing_tests_rows = change_rows
        .iter()
        .filter(|row| attestation_status(row.get("attestation_summary"), "tests") == "pass")
        .count();
    let coverage_summary = if change_count > 0 {
        format!(
            "{passing_tests_rows}/{change_count} change(s) report passing tests; {passing_policy_rows}/{change_count} change(s) currently pass policy; approvals={approvals}; blocking={blocking}; file_entries={file_entries}."
        )
    } else {
        "No reviewable implementation is available yet.".to_string()
    };
    let verdict = if change_count == 0 {
        "needs_fix"
    } else if change_rows.iter().any(change_has_failed_validation) {
        "high_risk"
    } else if change_rows.iter().any(|row| {
        !matches!(
            value_text_path(row, &["change", "status"]).as_deref(),
            Some(CHANGE_STATUS_LANDED | CHANGE_STATUS_ARCHIVED)
        ) && (row.get("current_patchset").is_none()
            || row.get("current_patchset") == Some(&JsonValue::Null)
            || row.get("attestation_summary").is_none()
            || row.get("attestation_summary") == Some(&JsonValue::Null))
    }) {
        "needs_fix"
    } else if !regression_concerns.is_empty() {
        "safe_with_minor_followup"
    } else {
        "safe_to_promote"
    };
    let promotion_recommendation = if verdict == "safe_to_promote" && open_change_count > 0 {
        "land when the task outcome is accepted"
    } else if verdict == "safe_to_promote" {
        "complete the task record after confirming the landed outcome"
    } else if verdict == "safe_with_minor_followup" {
        "clear the remaining follow-up before shared landing"
    } else {
        "revise the implementation before promotion"
    };
    json!({
        "source": "derived_from_change_patchset_policy_state",
        "touched_components": touched_components,
        "risk_summary": coverage_summary,
        "regression_concerns": regression_concerns,
        "coverage_summary": coverage_summary,
        "promotion_recommendation": promotion_recommendation,
        "verdict": verdict,
        "human_checkable": true,
    })
}

pub(super) fn combined_review_recommendation(
    task: &JsonMap<String, JsonValue>,
    task_review: &JsonValue,
    code_review: &JsonValue,
    summary: &JsonValue,
) -> JsonValue {
    let task_verdict =
        value_text(task_review, "acceptance_status").unwrap_or_else(|| "defer".to_string());
    let code_verdict =
        value_text(code_review, "verdict").unwrap_or_else(|| "needs_fix".to_string());
    let shared_boundary_crossed = value_bool_path(task_review, &["shared_boundary", "crossed"]);
    let open_change_count = value_int(summary, "open_change_count");
    let landed_change_count = value_int(summary, "landed_change_count");
    let (action, reason) = if task_verdict == "redirect" {
        (
            "change direction",
            "The task outcome needs redirection even before the technical slice is promoted.",
        )
    } else if task_verdict == "not_accepted" {
        (
            "stop",
            "The current task outcome is not accepted, so the implementation should not advance to landing.",
        )
    } else if matches!(code_verdict.as_str(), "needs_fix" | "high_risk") {
        (
            "revise",
            "Technical safety issues still block promotion or landing.",
        )
    } else if task_verdict == "complete" && code_verdict == "safe_to_promote" {
        (
            if open_change_count > 0 {
                "land"
            } else if object_text(task, "status").as_deref() == Some("completed") {
                "stop"
            } else {
                "land"
            },
            "The task outcome appears complete and the current technical slice looks safe to promote.",
        )
    } else if task_verdict == "complete" {
        (
            "split follow-up task",
            "The outcome appears complete, but a minor technical follow-up should stay visible.",
        )
    } else {
        (
            "revise",
            "The task outcome still needs follow-up before it should be treated as accepted.",
        )
    };
    let boundary_summary = if !shared_boundary_crossed {
        "The task is still in local execution space; no shared patchset has been published yet."
    } else if open_change_count > 0 {
        "The task has crossed into shared workflow and is waiting on the final review/policy/land path."
    } else if landed_change_count > 0 {
        "The shared workflow path has already landed; finish the remaining task-level cleanup honestly."
    } else {
        "The task is in a shared workflow state."
    };
    json!({
        "task_review_verdict": task_verdict,
        "code_review_verdict": code_verdict,
        "action": action,
        "reason": reason,
        "shared_boundary": {
            "crossed": shared_boundary_crossed,
            "state": if shared_boundary_crossed {"shared_workflow"} else {"local_execution"},
            "summary": boundary_summary,
        },
    })
}
