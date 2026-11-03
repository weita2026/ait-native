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
        let mut latest_by_reviewer_lane: BTreeMap<(String, String, String), JsonValue> =
            BTreeMap::new();
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
                        workflow_reviewer_kind(review).to_string(),
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
            .map(|review| {
                (
                    workflow_reviewer_kind(review),
                    string_field(review, "reviewer").unwrap_or_default(),
                )
            })
            .collect::<BTreeSet<_>>();
        let human_approvals = approval_reviewers
            .iter()
            .filter(|(kind, _)| *kind == "user")
            .count();
        let human_task_approvals = latest_by_reviewer_lane
            .values()
            .filter(|review| {
                workflow_reviewer_kind(review) == "user"
                    && string_field(review, "action").as_deref() == Some("task_approve")
            })
            .count();
        return json!({
            "task_approvals": task_approvals,
            "team_approvals": team_approvals,
            "human_approvals": human_approvals as i64,
            "eligible_human_approvals": human_approvals as i64,
            "human_task_approvals": human_task_approvals as i64,
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
        "human_approvals": review_summary.get("human_approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "eligible_human_approvals": review_summary.get("eligible_human_approvals").and_then(JsonValue::as_i64).or_else(|| review_summary.get("human_approvals").and_then(JsonValue::as_i64)).unwrap_or_default(),
        "human_task_approvals": review_summary.get("human_task_approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "eligible_task_approvals": review_summary.get("task_approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "approvals": review_summary.get("approvals").and_then(JsonValue::as_i64).unwrap_or_default(),
        "blocking": review_summary.get("blocking").and_then(JsonValue::as_i64).unwrap_or_default(),
        "current_patchset_id": string_field(review_summary, "current_patchset_id"),
        "reviews": review_summary.get("reviews").cloned().unwrap_or(JsonValue::Array(Vec::new())),
    })
}

fn workflow_reviewer_kind(review: &JsonValue) -> &str {
    // Legacy requests may omit kind, but readback must positively establish
    // user kind before a record can count toward a human-only approval total.
    review
        .get("reviewer_kind")
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown")
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

#[cfg(test)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::primitives) enum WorkflowPatchsetHeadRelation {
    Same,
    CurrentDescendsFromPatchset,
    PatchsetDescendsFromCurrent,
    Diverged,
    Unknown,
}

impl WorkflowPatchsetHeadRelation {
    fn as_str(self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::CurrentDescendsFromPatchset => "current_descends_from_patchset",
            Self::PatchsetDescendsFromCurrent => "patchset_descends_from_current",
            Self::Diverged => "diverged",
            Self::Unknown => "unknown",
        }
    }
}

pub(in crate::primitives) fn workflow_patchset_head_relation(
    repo: &RepoRuntime,
    patchset_revision_snapshot_id: Option<&str>,
    current_head_snapshot_id: Option<&str>,
) -> Result<WorkflowPatchsetHeadRelation, String> {
    let Some(patchset_revision_snapshot_id) = normalized_text(patchset_revision_snapshot_id) else {
        return Ok(WorkflowPatchsetHeadRelation::Unknown);
    };
    let Some(current_head_snapshot_id) = normalized_text(current_head_snapshot_id) else {
        return Ok(WorkflowPatchsetHeadRelation::Unknown);
    };
    if patchset_revision_snapshot_id == current_head_snapshot_id {
        return Ok(WorkflowPatchsetHeadRelation::Same);
    }
    if !local_snapshot_exists(repo, &patchset_revision_snapshot_id)?
        || !local_snapshot_exists(repo, &current_head_snapshot_id)?
    {
        return Ok(WorkflowPatchsetHeadRelation::Unknown);
    }
    if snapshot_distance_if_ancestor(
        repo,
        Some(&patchset_revision_snapshot_id),
        Some(&current_head_snapshot_id),
    )?
    .is_some()
    {
        return Ok(WorkflowPatchsetHeadRelation::CurrentDescendsFromPatchset);
    }
    if snapshot_distance_if_ancestor(
        repo,
        Some(&current_head_snapshot_id),
        Some(&patchset_revision_snapshot_id),
    )?
    .is_some()
    {
        return Ok(WorkflowPatchsetHeadRelation::PatchsetDescendsFromCurrent);
    }
    Ok(WorkflowPatchsetHeadRelation::Diverged)
}

#[expect(
    clippy::too_many_arguments,
    reason = "refresh compares the captured candidate, workspace and target authority together"
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
    head_relation: WorkflowPatchsetHeadRelation,
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
        "head_relation": head_relation.as_str(),
    });
    if patchset["source_kind"].as_str() == Some("history_promotion_aggregate")
        && workspace_matches_patchset == Some(false)
    {
        let conflicted = worktree_retarget
            .and_then(|v| v.get("rebase_state"))
            .and_then(JsonValue::as_str)
            == Some("conflicted");
        if !base_is_fresh
            || conflicted
            || !matches!(
                head_relation,
                WorkflowPatchsetHeadRelation::CurrentDescendsFromPatchset
            )
        {
            payload["reason_code"] = json!("aggregate_correction_recovery_required");
            payload["republish_allowed"] = json!(false);
            payload["rebase_required"] = json!(false);
            payload["summary"] = json!(
                "Resolve aggregate identity, ancestry or base drift before preparing a correction."
            );
            payload["detail"] = json!("Ordinary rebase/publication cannot preserve this aggregate's source history closeout. Keep the selected candidate and resolve its exact authority first.");
        } else {
            payload["reason_code"] = json!("aggregate_correction");
            payload["republish_allowed"] = json!(true);
            payload["rebase_required"] = json!(false);
            payload["summary"] =
                json!("Prepare the descendant correction while preserving aggregate receipts.");
            payload["detail"] = json!("Native correction prepare retains the original receipt chain and Snapshot ancestry, binds the corrected revision and requires new candidate CI and review.");
        }
        return Some(payload);
    }
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
    if workspace_matches_patchset == Some(false)
        && !matches!(
            head_relation,
            WorkflowPatchsetHeadRelation::CurrentDescendsFromPatchset
        )
    {
        payload["republish_allowed"] = JsonValue::Bool(false);
        payload["rebase_required"] = JsonValue::Bool(false);
        match head_relation {
            WorkflowPatchsetHeadRelation::PatchsetDescendsFromCurrent => {
                payload["reason_code"] =
                    JsonValue::String("current_head_behind_patchset".to_string());
                payload["summary"] = JsonValue::String(
                    "Restore the selected Patchset revision before continuing.".to_string(),
                );
                payload["detail"] = JsonValue::String(format!(
                    "Selected patchset `{patchset_id}` points at revision `{}`, which is a proven descendant of the current line head `{}`. Do not republish the older current head; continue from the selected revision or repair the worktree binding first.",
                    patchset_revision_snapshot_id.unwrap_or("unknown"),
                    revision_snapshot_id.unwrap_or("unknown"),
                ));
            }
            WorkflowPatchsetHeadRelation::Diverged => {
                payload["reason_code"] =
                    JsonValue::String("head_diverged_recovery_required".to_string());
                payload["summary"] = JsonValue::String(
                    "Resolve the divergent head before replacing the selected Patchset."
                        .to_string(),
                );
                payload["detail"] = JsonValue::String(format!(
                    "Selected patchset `{patchset_id}` points at revision `{}`, while the current line head `{}` is on a divergent Snapshot branch. Automatic republish is disabled until the intended revision is selected explicitly.",
                    patchset_revision_snapshot_id.unwrap_or("unknown"),
                    revision_snapshot_id.unwrap_or("unknown"),
                ));
            }
            _ => {
                payload["reason_code"] = JsonValue::String("head_relation_unproven".to_string());
                payload["summary"] = JsonValue::String(
                    "Verify Snapshot ancestry before replacing the selected Patchset.".to_string(),
                );
                payload["detail"] = JsonValue::String(format!(
                    "Selected patchset `{patchset_id}` points at revision `{}`, while the current line head is `{}`. Their Snapshot relationship is not proven locally, so automatic republish is disabled.",
                    patchset_revision_snapshot_id.unwrap_or("unknown"),
                    revision_snapshot_id.unwrap_or("unknown"),
                ));
            }
        }
        return Some(payload);
    }
    if !base_is_fresh {
        payload["reason_code"] = JsonValue::String("base_moved_republish".to_string());
        payload["republish_allowed"] = JsonValue::Bool(true);
        payload["rebase_required"] = JsonValue::Bool(false);
        payload["summary"] =
            JsonValue::String("Republish the current head on top of the newer base.".to_string());
        payload["detail"] = JsonValue::String(format!(
            "Selected patchset `{patchset_id}` still uses base `{}`, but `{base_line_name}` now points at `{}`. The current line head `{}` is already the refresh candidate, so no extra rebase is required; republish it if that newer base is the intended finish candidate.",
            patchset_base_snapshot_id.unwrap_or("unknown"),
            remote_base_snapshot_id.unwrap_or("unknown"),
            revision_snapshot_id.unwrap_or("unknown"),
        ));
        return Some(payload);
    }
    if workspace_matches_patchset == Some(false) {
        payload["reason_code"] = JsonValue::String("head_diverged_republish".to_string());
        payload["republish_allowed"] = JsonValue::Bool(true);
        payload["rebase_required"] = JsonValue::Bool(false);
        payload["summary"] = JsonValue::String(
            "Republish the newer current head as the selected patchset.".to_string(),
        );
        payload["detail"] = JsonValue::String(format!(
            "Selected patchset `{patchset_id}` points at revision `{}`, but the current line head is `{}`. No rebase is required; refresh republishes this newer head as the next finish candidate.",
            patchset_revision_snapshot_id.unwrap_or("unknown"),
            revision_snapshot_id.unwrap_or("unknown"),
        ));
        return Some(payload);
    }
    None
}
