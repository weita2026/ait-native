use super::*;

pub(in crate::primitives) fn workflow_json_bool_toggle(value: Option<&JsonValue>) -> Option<bool> {
    match value {
        Some(JsonValue::Bool(value)) => Some(*value),
        Some(JsonValue::String(text)) => match text.trim().to_ascii_lowercase().as_str() {
            "on" | "true" | "yes" | "1" => Some(true),
            "off" | "false" | "no" | "0" => Some(false),
            _ => None,
        },
        Some(JsonValue::Number(value)) => value.as_i64().map(|numeric| numeric != 0),
        _ => None,
    }
}

pub(in crate::primitives) fn workflow_task_review_required(repo: &RepoRuntime) -> bool {
    workflow_json_bool_toggle(repo.config.get("task_review")).unwrap_or(false)
}

pub(in crate::primitives) fn workflow_effective_task_review(repo: &RepoRuntime) -> JsonValue {
    if repo.config.contains_key("task_review") {
        if let Some(value) = workflow_json_bool_toggle(repo.config.get("task_review")) {
            return json!({"value": value, "source": "repo_config"});
        }
    }
    json!({"value": false, "source": "built_in"})
}

pub(in crate::primitives) fn workflow_root_repo(repo: &RepoRuntime) -> Result<RepoRuntime, String> {
    RepoRuntime::discover_from_path(&repo.authoritative_repo_root())
}

pub(in crate::primitives) fn workflow_patchset_changed_paths(
    patchset: Option<&JsonValue>,
) -> Vec<String> {
    let Some(patchset) = patchset.and_then(JsonValue::as_object) else {
        return Vec::new();
    };
    let Some(diff_stats) = patchset.get("diff_stats").and_then(JsonValue::as_object) else {
        return Vec::new();
    };
    let Some(paths) = diff_stats.get("paths").and_then(JsonValue::as_object) else {
        return Vec::new();
    };
    let mut changed_paths = Vec::new();
    for key in ["added", "deleted", "modified"] {
        if let Some(rows) = paths.get(key).and_then(JsonValue::as_array) {
            for row in rows {
                if let Some(path) = row.as_str().and_then(|value| normalized_text(Some(value))) {
                    changed_paths.push(path);
                }
            }
        }
    }
    changed_paths.sort();
    changed_paths.dedup();
    changed_paths
}

pub(in crate::primitives) fn workflow_code_review_summary_count(
    review_summary: &JsonValue,
    patchset_id: Option<&str>,
) -> i64 {
    let current_patchset_id = string_field(review_summary, "current_patchset_id");
    if let Some(reviews) = review_summary.get("reviews").and_then(JsonValue::as_array) {
        return reviews
            .iter()
            .filter(|review| {
                review.is_object()
                    && patchset_id.is_none_or(|value| {
                        string_field(review, "patchset_id").as_deref() == Some(value)
                    })
                    && string_field(review, "action").as_deref() == Some("code_review_summary")
                    && string_field(review, "comment").is_some_and(|value| {
                        missing_code_review_summary_sections(&value).is_empty()
                    })
            })
            .count() as i64;
    }
    if review_summary.get("code_review_summaries").is_some()
        && patchset_id.is_none_or(|value| current_patchset_id.as_deref() == Some(value))
    {
        return review_summary
            .get("code_review_summaries")
            .and_then(JsonValue::as_i64)
            .unwrap_or_default();
    }
    0
}

pub(in crate::primitives) fn workflow_review_decision_lane(action: &str) -> Option<&'static str> {
    match action {
        "task_approve" | "task_request_changes" | "task_defer" => Some("task"),
        "approve" | "request_changes" | "defer" => Some("team"),
        _ => None,
    }
}

pub(in crate::primitives) fn workflow_review_lane_counts(
    review_summary: &JsonValue,
    patchset_id: Option<&str>,
) -> JsonValue {
    if let Some(reviews) = review_summary.get("reviews").and_then(JsonValue::as_array) {
        let mut latest_by_reviewer_lane: BTreeMap<(String, String), JsonValue> = BTreeMap::new();
        let mut blocking_count = 0i64;
        for review in reviews {
            if !review.is_object() {
                continue;
            }
            if patchset_id
                .is_some_and(|value| string_field(review, "patchset_id").as_deref() != Some(value))
            {
                continue;
            }
            let action = string_field(review, "action").unwrap_or_default();
            if let Some(decision_lane) = workflow_review_decision_lane(&action) {
                latest_by_reviewer_lane.insert(
                    (
                        string_field(review, "reviewer").unwrap_or_default(),
                        decision_lane.to_string(),
                    ),
                    review.clone(),
                );
            }
            if matches!(action.as_str(), "request_changes" | "task_request_changes")
                || review
                    .get("blocking")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false)
            {
                blocking_count += 1;
            }
        }
        let task_approvals = latest_by_reviewer_lane
            .values()
            .filter(|review| string_field(review, "action").as_deref() == Some("task_approve"))
            .count() as i64;
        let team_approvals = latest_by_reviewer_lane
            .values()
            .filter(|review| string_field(review, "action").as_deref() == Some("approve"))
            .count() as i64;
        let approval_reviewers = latest_by_reviewer_lane
            .values()
            .filter(|&review| {
                matches!(
                    string_field(review, "action").as_deref(),
                    Some("task_approve") | Some("approve")
                )
            })
            .map(|review| string_field(review, "reviewer").unwrap_or_default())
            .collect::<BTreeSet<_>>();
        return json!({
            "task_approvals": task_approvals,
            "team_approvals": team_approvals,
            "human_approvals": approval_reviewers.len() as i64,
            "eligible_human_approvals": approval_reviewers.len() as i64,
            "human_task_approvals": task_approvals,
            "eligible_task_approvals": task_approvals,
            "approvals": approval_reviewers.len() as i64,
            "blocking": blocking_count,
            "current_patchset_id": patchset_id,
            "reviews": reviews.clone(),
        });
    }
    json!({
        "task_approvals": review_summary.get("task_approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "team_approvals": review_summary.get("team_approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "human_approvals": review_summary.get("human_approvals").and_then(JsonValue::as_i64).or_else(|| review_summary.get("approvals").and_then(JsonValue::as_i64)).unwrap_or_default(),
        "eligible_human_approvals": review_summary.get("approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "human_task_approvals": review_summary.get("human_task_approvals").and_then(JsonValue::as_i64).or_else(|| review_summary.get("task_approvals").and_then(JsonValue::as_i64)).unwrap_or_default(),
        "eligible_task_approvals": review_summary.get("task_approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "approvals": review_summary.get("approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "blocking": review_summary.get("blocking").and_then(JsonValue::as_i64).unwrap_or_default(),
        "current_patchset_id": string_field(review_summary, "current_patchset_id"),
        "reviews": review_summary.get("reviews").cloned().unwrap_or(JsonValue::Array(Vec::new())),
    })
}

pub(in crate::primitives) fn workflow_requires_code_review_summary(
    patchset: Option<&JsonValue>,
    attestation: Option<&JsonValue>,
    policy: Option<&JsonValue>,
) -> bool {
    let Some(patchset) = patchset else {
        return false;
    };
    let effective_requirements = policy
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("effective_requirements"))
        .and_then(JsonValue::as_object);
    if let Some(value) =
        effective_requirements.and_then(|value| value.get("require_code_review_summary"))
    {
        return value.as_bool().unwrap_or(false);
    }
    let author_mode = attestation
        .and_then(JsonValue::as_object)
        .and_then(|value| value.get("author_mode"))
        .and_then(JsonValue::as_str)
        .or_else(|| patchset.get("author_mode").and_then(JsonValue::as_str))
        .unwrap_or_default();
    let content_class = if workflow_patchset_changed_paths(Some(patchset)).is_empty() {
        "non_code_change"
    } else {
        "code_change"
    };
    content_class == "code_change" && AI_RELATED_AUTHOR_MODES.contains(&author_mode)
}

pub(in crate::primitives) fn workflow_relevant_landing_summary(
    landing_summary: Option<&JsonValue>,
    patchset: Option<&JsonValue>,
) -> Option<JsonValue> {
    let summary = landing_summary?.as_object()?;
    let patchset_id = patchset.and_then(|value| string_field(value, "patchset_id"));
    let landing_patchset_id = string_field(&JsonValue::Object(summary.clone()), "patchset_id");
    if let (Some(selected_patchset_id), Some(land_patchset_id)) = (patchset_id, landing_patchset_id)
    {
        if selected_patchset_id != land_patchset_id {
            return None;
        }
    }
    Some(JsonValue::Object(summary.clone()))
}

pub(in crate::primitives) fn workflow_base_stale_converged_snapshot_id(
    landing_summary: &JsonValue,
    patchset_revision_snapshot_id: Option<&str>,
) -> Option<String> {
    let patchset_revision_snapshot_id = patchset_revision_snapshot_id?;
    let status = string_field(landing_summary, "status")
        .unwrap_or_default()
        .to_ascii_lowercase();
    if status != "blocked" {
        return None;
    }
    let result = landing_summary
        .get("result")
        .and_then(JsonValue::as_object)?;
    let blocker_class = result
        .get("blocker_class")
        .and_then(JsonValue::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    if blocker_class != "BASE_STALE" {
        return None;
    }
    let target_line_head = result
        .get("target_line_head")
        .or_else(|| result.get("target_line_head_snapshot_id"))
        .and_then(JsonValue::as_str)
        .and_then(|value| normalized_text(Some(value)))?;
    (target_line_head == patchset_revision_snapshot_id).then_some(target_line_head)
}

pub(in crate::primitives) fn workflow_normalize_base_stale_converged_landing_summary(
    landing_summary: Option<JsonValue>,
    patchset_revision_snapshot_id: Option<&str>,
) -> Option<JsonValue> {
    let mut landing_summary = landing_summary?;
    let Some(landed_snapshot_id) =
        workflow_base_stale_converged_snapshot_id(&landing_summary, patchset_revision_snapshot_id)
    else {
        return Some(landing_summary);
    };
    let Some(summary) = landing_summary.as_object_mut() else {
        return Some(landing_summary);
    };
    if !summary.contains_key("original_status") {
        let original_status = summary
            .get("status")
            .cloned()
            .unwrap_or_else(|| JsonValue::String("blocked".to_string()));
        summary.insert("original_status".to_string(), original_status);
    }
    summary.insert(
        "status".to_string(),
        JsonValue::String("landed".to_string()),
    );
    summary.insert(
        "status_source".to_string(),
        JsonValue::String("base_stale_target_line_already_at_revision".to_string()),
    );
    summary.insert("base_stale_converged".to_string(), JsonValue::Bool(true));
    let result = summary.entry("result").or_insert_with(|| json!({}));
    if let Some(result) = result.as_object_mut() {
        result
            .entry("landed_snapshot_id".to_string())
            .or_insert_with(|| JsonValue::String(landed_snapshot_id));
        result
            .entry("line_action".to_string())
            .or_insert_with(|| JsonValue::String("already_moved".to_string()));
        result.insert("base_stale_converged".to_string(), JsonValue::Bool(true));
    }
    Some(landing_summary)
}

pub(in crate::primitives) fn workflow_target_line_converged_landing_summary(
    landing_summary: Option<JsonValue>,
    patchset: Option<&JsonValue>,
    change_id: &str,
    target_line: &str,
    remote_base_snapshot_id: Option<&str>,
    patchset_base_snapshot_id: Option<&str>,
    patchset_revision_snapshot_id: Option<&str>,
) -> Option<JsonValue> {
    let landing_summary = workflow_normalize_base_stale_converged_landing_summary(
        landing_summary,
        patchset_revision_snapshot_id,
    );
    if landing_summary.is_some() {
        return landing_summary;
    }
    let remote_base_snapshot_id = remote_base_snapshot_id?;
    let patchset_base_snapshot_id = patchset_base_snapshot_id?;
    let patchset_revision_snapshot_id = patchset_revision_snapshot_id?;
    if remote_base_snapshot_id != patchset_revision_snapshot_id {
        return None;
    }
    if patchset_base_snapshot_id == patchset_revision_snapshot_id {
        return None;
    }

    let base_stale_converged = patchset_base_snapshot_id != remote_base_snapshot_id;
    let mut result = JsonMap::from_iter([
        (
            "landed_snapshot_id".to_string(),
            JsonValue::String(patchset_revision_snapshot_id.to_string()),
        ),
        (
            "target_line_head".to_string(),
            JsonValue::String(remote_base_snapshot_id.to_string()),
        ),
        (
            "line_action".to_string(),
            JsonValue::String("already_moved".to_string()),
        ),
        (
            "target_line_already_at_revision".to_string(),
            JsonValue::Bool(true),
        ),
    ]);
    if base_stale_converged {
        result.insert("base_stale_converged".to_string(), JsonValue::Bool(true));
    }

    let mut summary = JsonMap::from_iter([
        (
            "status".to_string(),
            JsonValue::String("landed".to_string()),
        ),
        (
            "status_source".to_string(),
            JsonValue::String("target_line_already_at_revision".to_string()),
        ),
        (
            "target_line".to_string(),
            JsonValue::String(target_line.to_string()),
        ),
        (
            "target_line_already_at_revision".to_string(),
            JsonValue::Bool(true),
        ),
        (
            "change_id".to_string(),
            JsonValue::String(change_id.to_string()),
        ),
        ("result".to_string(), JsonValue::Object(result)),
    ]);
    if let Some(patchset_id) = patchset.and_then(|value| string_field(value, "patchset_id")) {
        summary.insert("patchset_id".to_string(), JsonValue::String(patchset_id));
    }
    if base_stale_converged {
        summary.insert("base_stale_converged".to_string(), JsonValue::Bool(true));
    }
    Some(JsonValue::Object(summary))
}

#[expect(
    clippy::too_many_arguments,
    reason = "refresh context mirrors independently optional workflow facts"
)]
pub(in crate::primitives) fn workflow_patchset_refresh_context(
    patchset: Option<&JsonValue>,
    worktree_retarget: Option<&JsonValue>,
    base_line_name: &str,
    remote_base_snapshot_id: Option<&str>,
    patchset_base_snapshot_id: Option<&str>,
    patchset_revision_snapshot_id: Option<&str>,
    revision_snapshot_id: Option<&str>,
    base_is_fresh: bool,
    workspace_matches_patchset: Option<bool>,
) -> Option<JsonValue> {
    let patchset = patchset?;
    let patchset_id = string_field(patchset, "patchset_id")?;
    let mut payload = json!({
        "patchset_id": patchset_id,
        "base_line": base_line_name,
        "patchset_base_snapshot_id": patchset_base_snapshot_id,
        "patchset_revision_snapshot_id": patchset_revision_snapshot_id,
        "current_head_snapshot_id": revision_snapshot_id,
        "remote_base_snapshot_id": remote_base_snapshot_id,
    });
    if let Some(worktree_retarget) = worktree_retarget.and_then(JsonValue::as_object) {
        let rebase_state = worktree_retarget
            .get("rebase_state")
            .and_then(JsonValue::as_str)
            .unwrap_or("idle");
        if rebase_state == "conflicted" {
            let conflict_paths = worktree_retarget
                .get("rebase_conflict_paths")
                .and_then(JsonValue::as_array)
                .map(|rows| {
                    rows.iter()
                        .filter_map(JsonValue::as_str)
                        .take(5)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "resolve conflicts first".to_string());
            payload["reason_code"] = JsonValue::String("rebase_conflicted".to_string());
            payload["rebase_required"] = JsonValue::Bool(true);
            payload["summary"] = JsonValue::String(
                "Resolve the rebase conflict before refreshing the selected patchset.".to_string(),
            );
            payload["detail"] = JsonValue::String(format!(
                "Selected patchset `{patchset_id}` is stale and the bound worktree is paused on conflicted rebase paths: {conflict_paths}. Finish `ait worktree rebase --continue` or abort that rebase before republishing."
            ));
            return Some(payload);
        }
        if worktree_retarget
            .get("needs_retarget")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false)
        {
            let fork_snapshot_id = worktree_retarget
                .get("fork_snapshot_id")
                .and_then(JsonValue::as_str)
                .and_then(|value| normalized_text(Some(value)));
            let target_base_snapshot_id = worktree_retarget
                .get("target_base_snapshot_id")
                .and_then(JsonValue::as_str)
                .and_then(|value| normalized_text(Some(value)))
                .or_else(|| remote_base_snapshot_id.map(str::to_string));
            payload["reason_code"] = JsonValue::String("rebase_required".to_string());
            payload["rebase_required"] = JsonValue::Bool(true);
            payload["worktree_fork_snapshot_id"] = fork_snapshot_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null);
            payload["target_base_snapshot_id"] = target_base_snapshot_id
                .clone()
                .map(JsonValue::String)
                .unwrap_or(JsonValue::Null);
            payload["summary"] =
                JsonValue::String("Rebase before refreshing the selected patchset.".to_string());
            payload["detail"] = JsonValue::String(format!(
                "Selected patchset `{patchset_id}` is stale because `{base_line_name}` now points at `{}` while the bound worktree still forks from `{}`. Run `ait worktree rebase --onto {base_line_name}` before republishing.",
                target_base_snapshot_id.unwrap_or_else(|| "unknown".to_string()),
                fork_snapshot_id.unwrap_or_else(|| "unknown".to_string()),
            ));
            return Some(payload);
        }
    }
    if !base_is_fresh {
        payload["reason_code"] = JsonValue::String("base_moved_republish".to_string());
        payload["rebase_required"] = JsonValue::Bool(false);
        payload["summary"] =
            JsonValue::String("Republish the current head on top of the newer base.".to_string());
        payload["detail"] = JsonValue::String(format!(
            "Selected patchset `{patchset_id}` still uses base `{}`, but `{base_line_name}` now points at `{}`. The current line head `{}` is already the refresh candidate, so no extra rebase is required; republish it if that newer base is the intended land candidate.",
            patchset_base_snapshot_id.unwrap_or("unknown"),
            remote_base_snapshot_id.unwrap_or("unknown"),
            revision_snapshot_id.unwrap_or("unknown"),
        ));
        return Some(payload);
    }
    if workspace_matches_patchset == Some(false) {
        payload["reason_code"] = JsonValue::String("head_diverged_republish".to_string());
        payload["rebase_required"] = JsonValue::Bool(false);
        payload["summary"] = JsonValue::String(
            "Republish the newer current head as the selected patchset.".to_string(),
        );
        payload["detail"] = JsonValue::String(format!(
            "Selected patchset `{patchset_id}` points at revision `{}`, but the current line head is `{}`. No rebase is required; refresh republishes this newer head as the next land candidate.",
            patchset_revision_snapshot_id.unwrap_or("unknown"),
            revision_snapshot_id.unwrap_or("unknown"),
        ));
        return Some(payload);
    }
    None
}
