use super::helpers::*;
use super::review_packets::{
    code_review_packet, combined_review_recommendation, task_review_packet,
};
use super::*;

pub fn task_workflow_detail_read_model(
    input: &TaskWorkflowDetailInput,
) -> Result<JsonValue, String> {
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
        let selected_patchset = index.selected_patchset(change).map(json_object);
        let display_patchset = selected_patchset
            .clone()
            .or_else(|| current_patchset.clone());
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
            "selected_patchset": selected_patchset,
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
