use super::helpers::*;
use super::review_packets::{
    code_review_packet, combined_review_recommendation, task_review_packet,
};
use super::*;

pub fn task_workflow_detail_read_model(
    input: &TaskWorkflowDetailInput,
) -> Result<JsonValue, String> {
    if input.task.contains_key("current_patchset_id") {
        return task_candidate_detail(input);
    }
    let task_id = required_text(&input.task, "task_id")?;
    let task_repo = required_text(&input.task, "repo_name")?;
    let index = WorkflowDetailIndex::new(input);
    let mut task_changes = input
        .changes
        .iter()
        .filter(|change| object_text(change, "task_id").as_deref() == Some(task_id.as_str()))
        .filter(|change| object_text(change, "repo_name").as_deref() == Some(task_repo.as_str()))
        .collect::<Vec<_>>();
    task_changes.sort_by(|left, right| {
        object_text(left, "created_at").cmp(&object_text(right, "created_at"))
    });

    let mut change_rows = Vec::new();
    let mut aggregate_files = Vec::new();
    let mut patchset_ids = BTreeSet::new();
    let mut change_ids = BTreeSet::new();
    let mut insertions_total = 0_i64;
    let mut deletions_total = 0_i64;
    let mut unique_paths = BTreeSet::new();

    for change in task_changes {
        let change_id = object_text(change, "change_id").unwrap_or_default();
        change_ids.insert(change_id.clone());
        let current_patchset = index.current_patchset(change).map(json_object);
        let display_patchset = current_patchset.clone();
        let display_patchset_id = display_patchset
            .as_ref()
            .and_then(|patchset| value_text(patchset, "patchset_id"));
        if let Some(patchset_id) = display_patchset_id.as_ref() {
            patchset_ids.insert(patchset_id.clone());
        }
        let current_patchset_id = current_patchset
            .as_ref()
            .and_then(|patchset| value_text(patchset, "patchset_id"));
        let review_summary = index.review_summary(&change_id);
        let policy_summary = index.policy_summary(current_patchset_id.as_deref());
        let attestation_summary = index.attestation_summary(current_patchset_id.as_deref());
        let landing_summary = index.latest_land_summary(&change_id);
        let base_head = current_patchset.as_ref().and_then(|_| {
            index.line_head(
                &task_repo,
                object_text(change, "base_line")
                    .as_deref()
                    .unwrap_or("main"),
            )
        });
        let freshness = json!({
            "base_is_fresh": current_patchset
                .as_ref()
                .and_then(|patchset| value_text(patchset, "base_snapshot_id"))
                .zip(base_head.clone())
                .map(|(base, head)| base == head)
                .unwrap_or(false),
            "current_base_head": base_head,
        });
        let delta = display_patchset_id
            .as_deref()
            .and_then(|patchset_id| index.delta_for_patchset(patchset_id));

        if let Some(delta_value) = delta.as_ref() {
            for file_row in delta_value
                .get("files")
                .and_then(JsonValue::as_array)
                .into_iter()
                .flatten()
            {
                let mut file = file_row.as_object().cloned().unwrap_or_default();
                file.insert("change_id".to_string(), json!(change_id));
                file.insert(
                    "change_title".to_string(),
                    change.get("title").cloned().unwrap_or(JsonValue::Null),
                );
                if let Some(patchset) = display_patchset.as_ref() {
                    file.insert(
                        "patchset_id".to_string(),
                        patchset
                            .get("patchset_id")
                            .cloned()
                            .unwrap_or(JsonValue::Null),
                    );
                    file.insert(
                        "patchset_number".to_string(),
                        patchset
                            .get("patchset_number")
                            .cloned()
                            .unwrap_or(JsonValue::Null),
                    );
                }
                insertions_total += file.get("insertions").and_then(int_value).unwrap_or(0);
                deletions_total += file.get("deletions").and_then(int_value).unwrap_or(0);
                if let Some(path) = object_text(&file, "path") {
                    unique_paths.insert(path);
                }
                aggregate_files.push(JsonValue::Object(file));
            }
        }

        change_rows.push(json!({
            "change": JsonValue::Object(change.clone()),
            "current_patchset": current_patchset,
            "display_patchset": display_patchset,
            "review_summary": review_summary,
            "policy_summary": policy_summary,
            "attestation_summary": attestation_summary,
            "landing_summary": landing_summary,
            "freshness": freshness,
            "delta": delta,
        }));
    }

    let summary = json!({
        "change_count": change_rows.len(),
        "open_change_count": change_rows.iter().filter(|row| {
            !matches!(
                value_text_path(row, &["change", "status"]).as_deref(),
                Some(CHANGE_STATUS_LANDED | CHANGE_STATUS_ARCHIVED)
            )
        }).count(),
        "landed_change_count": change_rows.iter().filter(|row| {
            value_text_path(row, &["change", "status"]).as_deref() == Some(CHANGE_STATUS_LANDED)
        }).count(),
        "patchset_count": patchset_ids.len(),
    });
    let aggregate_diff = json!({
        "change_count": change_rows.len(),
        "patchset_count": patchset_ids.len(),
        "file_entries": aggregate_files.len(),
        "unique_paths": unique_paths.len(),
        "insertions": insertions_total,
        "deletions": deletions_total,
        "files": aggregate_files,
    });
    let task_review = task_review_packet(&input.task, &change_rows, &summary, &aggregate_diff);
    let code_review = code_review_packet(&change_rows, &summary, &aggregate_diff);
    let combined_recommendation =
        combined_review_recommendation(&input.task, &task_review, &code_review, &summary);
    let workflow_context = workflow_context(
        "task",
        "task",
        &task_id,
        object_text(&input.task, "title").as_deref().unwrap_or(""),
    );
    Ok(json!({
        "task": JsonValue::Object(input.task.clone()),
        "repository": JsonValue::Object(input.repository.clone()),
        "changes": change_rows,
        "workflow_context": workflow_context,
        "summary": summary,
        "aggregate_diff": aggregate_diff,
        "task_review": task_review,
        "code_review": code_review,
        "combined_recommendation": combined_recommendation,
        "timeline": index.task_timeline(&task_id, &change_ids, &patchset_ids),
    }))
}

fn task_candidate_detail(input: &TaskWorkflowDetailInput) -> Result<JsonValue, String> {
    let task_id = required_text(&input.task, "task_id")?;
    let repo = required_text(&input.task, "repo_name")?;
    let index = WorkflowDetailIndex::new(input);
    let patchsets = input
        .patchsets
        .iter()
        .filter(|row| {
            object_text(row, "task_id").as_deref() == Some(task_id.as_str())
                && object_text(row, "repo_name").as_deref() == Some(repo.as_str())
        })
        .collect::<Vec<_>>();
    let find = |id: Option<String>| {
        id.and_then(|id| {
            patchsets
                .iter()
                .find(|row| object_text(row, "patchset_id").as_deref() == Some(id.as_str()))
                .copied()
        })
    };
    let current = find(object_text(&input.task, "current_patchset_id"));
    let receipt = input
        .task
        .get("successful_land")
        .filter(|value| value["status"] == "succeeded");
    let accepted = find(receipt.and_then(|value| value_text(value, "patchset_id")));
    let completed = object_text(&input.task, "status").as_deref() == Some("completed");
    let abandoned = matches!(
        object_text(&input.task, "status").as_deref(),
        Some("abandoned" | "canceled")
    );
    let terminal = completed || abandoned;
    let display = if completed && receipt.is_some() {
        accepted
    } else {
        current
    };
    let id = display.and_then(|row| object_text(row, "patchset_id"));
    let policy = index.policy_summary(id.as_deref());
    let attestation = index.attestation_summary(id.as_deref());
    let reviews = input
        .reviews
        .iter()
        .filter(|row| object_text(row, "patchset_id").as_deref() == id.as_deref() && id.is_some())
        .cloned()
        .collect::<Vec<_>>();
    let review_counts = review_summary_from_rows(&reviews, id.as_deref());
    let review = json!({"summary":review_counts,"approvals":review_counts.get("approval_count"),
        "blocking":review_counts.get("blocking_count"),"comments":review_counts.get("comment_count"),"reviews":reviews});
    let target = receipt
        .and_then(|row| value_text(row, "target_line"))
        .or_else(|| object_text(&input.repository, "default_line"));
    let head = target
        .as_deref()
        .and_then(|line| index.line_head(&repo, line));
    let fresh = display
        .and_then(|row| object_text(row, "base_snapshot_id"))
        .zip(head.clone())
        .map(|(base, head)| base == head);
    let delta = id.as_deref().and_then(|id| index.delta_for_patchset(id));
    let files = delta
        .as_ref()
        .and_then(|row| row.get("files"))
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    let insertions = files
        .iter()
        .filter_map(|row| row.get("insertions").and_then(int_value))
        .sum::<i64>();
    let deletions = files
        .iter()
        .filter_map(|row| row.get("deletions").and_then(int_value))
        .sum::<i64>();
    let paths = files
        .iter()
        .filter_map(|row| value_text(row, "path"))
        .collect::<BTreeSet<_>>();
    let changes = input
        .changes
        .iter()
        .filter(|row| {
            object_text(row, "task_id").as_deref() == Some(task_id.as_str())
                && object_text(row, "repo_name").as_deref() == Some(repo.as_str())
        })
        .collect::<Vec<_>>();
    let change_ids = changes
        .iter()
        .filter_map(|row| object_text(row, "change_id"))
        .collect::<BTreeSet<_>>();
    let patchset_ids = patchsets
        .iter()
        .filter_map(|row| object_text(row, "patchset_id"))
        .collect::<BTreeSet<_>>();
    let landed = changes
        .iter()
        .filter(|row| object_text(row, "status").as_deref() == Some("landed"))
        .count();
    let open = changes
        .iter()
        .filter(|row| {
            !matches!(
                object_text(row, "status").as_deref(),
                Some("landed" | "archived" | "superseded" | "abandoned")
            )
        })
        .count();
    let mut gaps = Vec::new();
    if completed && receipt.is_none() {
        gaps.push("A successful remote receipt is missing.");
    }
    if display.is_none() {
        gaps.push("No exact Task candidate is available.");
    }
    if display.is_some_and(|row| {
        row.get("governance_authority").and_then(JsonValue::as_bool) == Some(false)
    }) {
        gaps.push("Review the aggregate Task candidate.");
    }
    if !terminal {
        if display.is_some_and(|row| {
            row.get("withdrawn").and_then(JsonValue::as_bool) == Some(true)
                || row.get("invalidated").and_then(JsonValue::as_bool) == Some(true)
                || object_text(row, "publish_state").as_deref() == Some("superseded")
        }) {
            gaps.push("The current Task candidate is unavailable.");
        }
        if attestation.is_none() {
            gaps.push("Attestation is pending.");
        }
        if policy["decision"] != "pass" {
            gaps.push("Validation is pending.");
        }
        if review["blocking"].as_i64().unwrap_or(0) > 0 {
            gaps.push("Review changes are requested.");
        }
        if fresh == Some(false) {
            gaps.push("The candidate base differs from the target Line head.");
        }
        if object_text(&input.task, "workflow_state").as_deref() != Some("ready_to_land") {
            gaps.push("Review and validation of the current Task candidate are pending.");
        }
    }
    let ready = !abandoned && gaps.is_empty() && display.is_some();
    let action = if abandoned || completed && receipt.is_some() {
        "stop"
    } else if ready {
        "land"
    } else {
        "revise"
    };
    let summary = json!({"change_count":changes.len(),"open_change_count":open,"landed_change_count":landed,"patchset_count":patchsets.len()});
    let aggregate = json!({"change_count":changes.len(),"patchset_count":usize::from(display.is_some()),"file_entries":files.len(),
        "unique_paths":paths.len(),"insertions":insertions,"deletions":deletions,"files":files});
    let boundary = json!({"crossed":!patchsets.is_empty(),"state":if patchsets.is_empty(){"local_execution"}else{"shared_workflow"}});
    let task_review = json!({"source":"derived_from_workflow_state","intent_summary":object_text(&input.task,"intent"),
        "completion_summary":if completed&&receipt.is_some(){"The Task has an accepted remote receipt."}else if ready{"The current Task candidate is ready."}else{"The current Task candidate requires review or validation."},
        "effect_summary":format!("{} paths, {insertions} insertions, {deletions} deletions.",paths.len()),
        "acceptance_status":if ready{"complete"}else{"defer"},"unresolved_gaps":gaps,"suggested_next_action":action,
        "operator_confirmation_required":!terminal,"shared_boundary":boundary});
    let code_review = json!({"source":"derived_from_task_candidate_policy_state","touched_components":paths.into_iter().take(8).collect::<Vec<_>>(),
        "regression_concerns":gaps,"coverage_summary":"Review and validation are scoped to the exact Task candidate.",
        "risk_summary":if ready{"Candidate gates passed."}else{"Candidate gates are pending."},
        "promotion_recommendation":action,"verdict":if ready{"safe_to_promote"}else{"needs_fix"},"human_checkable":true});
    Ok(
        json!({"task":input.task,"repository":input.repository,"changes":changes.into_iter().map(|change|json!({"change":change})).collect::<Vec<_>>(),
        "current_patchset":current,"accepted_patchset":accepted,"display_patchset":display,"successful_land":receipt,
        "review_summary":review,"policy_summary":policy,"attestation_summary":attestation,"freshness":{"base_is_fresh":fresh,"current_base_head":head},
        "delta":delta,"workflow_context":workflow_context("task","task",&task_id,object_text(&input.task,"title").as_deref().unwrap_or("")),
        "summary":summary,"aggregate_diff":aggregate,"task_review":task_review,"code_review":code_review,
        "combined_recommendation":{"task_review_verdict":if ready{"complete"}else{"defer"},"code_review_verdict":if ready{"safe_to_promote"}else{"needs_fix"},
            "action":action,"reason":if ready{"The exact Task candidate has passed its gates."}else{"The exact Task candidate has pending gates."},"shared_boundary":boundary},
        "timeline":index.task_timeline(&task_id,&change_ids,&patchset_ids)}),
    )
}
