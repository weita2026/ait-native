use crate::json_support::{json, JsonMap as Map, JsonValue};

use crate::workflow_closeout_model_support::{
    bool_field, command_hint, command_hint_json, external_readiness_blocker_detail,
    external_readiness_is_ready, field_obj, int_field, optional_bool_field, optional_obj_field,
    optional_string_field, string_field, workflow_land_change_effectively_landed,
    workflow_land_policy_blocker_detail, workflow_land_result, workflow_land_result_blocker_class,
    workflow_land_stale_policy_blocker_cleared, workflow_land_submission_id,
    workflow_land_submission_status,
};

pub(crate) const WORKFLOW_LAND_PENDING_STATUSES: &[&str] = &["queued", "running"];
pub(crate) const WORKFLOW_READY_APPLY_OWNED_CODES: &[&str] = &[
    "snapshot_create",
    "publish_patchset",
    "refresh_patchset",
    "run_patchset_ci",
    "record_attestation",
    "waiting_for_ci",
];
pub(crate) const WORKFLOW_LAND_APPLY_OWNED_CODES: &[&str] = &[
    "record_code_review_summary",
    "record_review",
    "evaluate_policy",
    "submit_land",
    "complete_task",
    "waiting_for_land",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PatchsetCiGateState {
    Pass,
    PendingWithJob,
    NeedsRun,
}

fn patchset_ci_gate_state(status: Option<&Map<String, JsonValue>>) -> PatchsetCiGateState {
    let Some(status) = status else {
        return PatchsetCiGateState::NeedsRun;
    };
    let tests_status = optional_string_field(status, "tests_status")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let has_evidence = patchset_ci_status_has_runnable_evidence(status);
    let has_completed_patchset_state = patchset_ci_status_has_completed_patchset_state(status);
    if patchset_ci_status_latest_job_is_pending(status) {
        return PatchsetCiGateState::PendingWithJob;
    }
    if tests_status == "pass" && has_evidence && has_completed_patchset_state {
        return PatchsetCiGateState::Pass;
    }
    if patchset_ci_status_recent_jobs_have_pending(status) {
        return PatchsetCiGateState::PendingWithJob;
    }
    match tests_status.as_str() {
        "pending" if has_evidence => PatchsetCiGateState::PendingWithJob,
        _ => PatchsetCiGateState::NeedsRun,
    }
}

fn patchset_ci_status_has_completed_patchset_state(status: &Map<String, JsonValue>) -> bool {
    let run_seq = status
        .get("ci_run_seq")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default();
    let completed_at_s = status
        .get("ci_completed_at_s")
        .and_then(JsonValue::as_u64)
        .unwrap_or_default();
    run_seq > 0 && completed_at_s > 0
}

fn patchset_ci_status_latest_job_is_pending(status: &Map<String, JsonValue>) -> bool {
    status
        .get("latest_job")
        .and_then(JsonValue::as_object)
        .is_some_and(patchset_ci_job_is_pending)
}

fn patchset_ci_status_recent_jobs_have_pending(status: &Map<String, JsonValue>) -> bool {
    status
        .get("recent_jobs")
        .and_then(JsonValue::as_array)
        .is_some_and(|rows| {
            rows.iter()
                .any(|row| row.as_object().is_some_and(patchset_ci_job_is_pending))
        })
}

fn patchset_ci_job_is_pending(job: &Map<String, JsonValue>) -> bool {
    if !patchset_ci_job_is_patchset_ci(job) {
        return false;
    }
    let state = optional_string_field(job, "state")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let diagnostic_status = optional_string_field(job, "diagnostic_status")
        .unwrap_or_default()
        .to_ascii_lowercase();
    let retry_pending = job
        .get("retry_pending")
        .and_then(JsonValue::as_bool)
        .unwrap_or(false);
    retry_pending
        || matches!(state.as_str(), "queued" | "running")
        || matches!(diagnostic_status.as_str(), "queued" | "running")
}

fn patchset_ci_status_has_runnable_evidence(status: &Map<String, JsonValue>) -> bool {
    if let Some(has_runnable_evidence) = status
        .get("has_runnable_evidence")
        .and_then(JsonValue::as_bool)
    {
        return has_runnable_evidence;
    }
    status
        .get("selected_suite_ids")
        .and_then(JsonValue::as_array)
        .is_some_and(|rows| !rows.is_empty())
        || status
            .get("suite_results")
            .and_then(JsonValue::as_array)
            .is_some_and(|rows| !rows.is_empty())
        || status
            .get("latest_job")
            .and_then(JsonValue::as_object)
            .is_some_and(patchset_ci_job_is_patchset_ci)
        || status
            .get("recent_jobs")
            .and_then(JsonValue::as_array)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.as_object().is_some_and(patchset_ci_job_is_patchset_ci))
            })
}

fn patchset_ci_job_is_patchset_ci(job: &Map<String, JsonValue>) -> bool {
    optional_string_field(job, "job_type").as_deref() == Some("patchset.ci")
}

pub(crate) fn change_effectively_landed(
    change: &Map<String, JsonValue>,
    landing_summary: Option<&Map<String, JsonValue>>,
) -> bool {
    workflow_land_change_effectively_landed(change, landing_summary)
}

pub(crate) fn workflow_ready_next_action(
    facts: &JsonValue,
    commands: &JsonValue,
    ignore_workspace_authoring: bool,
    patchset_is_authoritative: bool,
    apply_owned_continuation: bool,
) -> JsonValue {
    let change = field_obj(facts, "change");
    let task = field_obj(facts, "task");
    let workspace = field_obj(facts, "workspace");
    let patchset = optional_obj_field(facts, "patchset");
    let landing_summary = optional_obj_field(facts, "landing_summary");
    let base_is_fresh = bool_field(&field_obj(facts, "freshness"), "base_is_fresh");
    let workspace_matches_patchset =
        optional_bool_field(&field_obj(facts, "workspace"), "workspace_matches_patchset");
    let attestation = optional_obj_field(facts, "attestation");
    let patchset_ci_status = optional_obj_field(facts, "patchset_ci_status");
    let external_readiness = optional_obj_field(facts, "external_readiness");
    let tests_state = string_field(facts, "tests_state");
    let change_id = optional_string_field(&change, "change_id").unwrap_or_default();
    let apply_command = command_hint(commands, "apply_command")
        .unwrap_or_else(|| format!("ait workflow ready {change_id} --apply"));
    let land_command = command_hint(commands, "land_command");
    let patchset_id = patchset
        .as_ref()
        .and_then(|value| optional_string_field(value, "patchset_id"))
        .unwrap_or_default();
    let patchset_ci_required = command_hint(commands, "patchset_ci_command").is_some();
    if change_effectively_landed(&change, landing_summary.as_ref()) {
        let task_status = string_field(&task, "status");
        let detail = if task_status != "completed" {
            "The change is already landed; `task finish` can still close the Task."
        } else {
            "No further ready-workflow action is required."
        };
        return json!({
            "code": "done",
            "summary": "Ready phase is already complete.",
            "detail": detail,
            "command": if task_status != "completed" { land_command } else { None },
        });
    }
    if !ignore_workspace_authoring && !bool_field(&workspace, "clean") {
        return json!({
            "code": "snapshot_create",
            "summary": "Capture a fresh snapshot before publishing or refreshing the patchset.",
            "detail": "The workspace still has unsaved changes.",
            "command": workflow_ready_owned_command(
                "snapshot_create",
                command_hint(commands, "apply_command"),
                apply_owned_continuation,
                Some(apply_command.clone()),
                Some("ait snapshot create --message \"reviewable snapshot\"".to_string()),
            ),
        });
    }
    if patchset.is_none()
        || !base_is_fresh
        || (matches!(workspace_matches_patchset, Some(false)) && !patchset_is_authoritative)
    {
        if patchset.is_some() {
            if let Some(refresh_context) = optional_obj_field(facts, "patchset_refresh") {
                return json!({
                    "code": "refresh_patchset",
                    "summary": optional_string_field(&refresh_context, "summary").unwrap_or_else(|| "Refresh the selected patchset from the current line.".to_string()),
                    "detail": optional_string_field(&refresh_context, "detail").unwrap_or_else(|| "The selected patchset is stale and needs a fresh publish from the current line.".to_string()),
                    "command": workflow_ready_owned_command(
                        "refresh_patchset",
                        command_hint(commands, "apply_command"),
                        apply_owned_continuation,
                        Some(apply_command.clone()),
                        command_hint(commands, "publish_command"),
                    ),
                    "refresh_context": JsonValue::Object(refresh_context),
                });
            }
        }
        let publish_code = if patchset.is_none() {
            "publish_patchset"
        } else {
            "refresh_patchset"
        };
        return json!({
            "code": publish_code,
            "summary": "Publish the current line as the reviewable patchset.",
            "detail": "The ready workflow still needs a fresh published patchset from the current line.",
            "command": workflow_ready_owned_command(
                publish_code,
                command_hint(commands, "apply_command"),
                apply_owned_continuation,
                Some(apply_command.clone()),
                command_hint(commands, "publish_command"),
            ),
        });
    }
    if !external_readiness_is_ready(external_readiness.as_ref()) {
        return json!({
            "code": "external_readiness_blocked",
            "summary": "Resolve external readiness blockers before CI and remote land.",
            "detail": external_readiness_blocker_detail(external_readiness.as_ref()),
            "command": "ait external doctor",
            "external_readiness": external_readiness
                .clone()
                .map(JsonValue::Object)
                .unwrap_or(JsonValue::Null),
        });
    }
    if patchset_ci_required {
        match patchset_ci_gate_state(patchset_ci_status.as_ref()) {
            PatchsetCiGateState::Pass => {}
            PatchsetCiGateState::PendingWithJob => {
                return json!({
                    "code": "waiting_for_ci",
                    "summary": "Wait for patchset CI to reach a terminal result.",
                    "detail": "Patchset CI already started for the selected patchset and workflow ready apply can keep ownership until tests pass, fail, or time out.",
                    "command": if apply_owned_continuation {
                        JsonValue::String(apply_command.clone())
                    } else {
                        command_hint_json(commands, "patchset_ci_command")
                    },
                });
            }
            PatchsetCiGateState::NeedsRun => {
                return json!({
                    "code": "run_patchset_ci",
                    "summary": if matches!(tests_state.as_str(), "fail" | "failed" | "hard_fail" | "soft_fail") {
                        "Rerun patchset CI for the selected patchset."
                    } else {
                        "Run patchset CI so completed CI state is recorded on the selected Patchset."
                    },
                    "detail": if matches!(tests_state.as_str(), "fail" | "failed" | "hard_fail" | "soft_fail") {
                        format!("Patchset CI last reported tests `{tests_state}` for the selected patchset.")
                    } else {
                        "Routine patchsets must rely on completed remote CI state embedded in the selected Patchset; local or manual tests pass is not enough for this repository.".to_string()
                    },
                    "command": workflow_ready_owned_command(
                        "run_patchset_ci",
                        command_hint(commands, "apply_command"),
                        apply_owned_continuation,
                        Some(apply_command.clone()),
                        command_hint(commands, "patchset_ci_command"),
                    ),
                });
            }
        }
    }
    if attestation.is_none()
        || (!tests_state.is_empty() && !matches!(tests_state.as_str(), "pass" | "not_required"))
    {
        return json!({
            "code": "record_attestation",
            "summary": "Record attestation for the selected patchset.",
            "detail": "Policy and landing should work from the compact Attestation gate statement; completed CI evidence remains embedded in the selected Patchset.",
            "command": if patchset_ci_required {
                JsonValue::String(apply_command.clone())
            } else {
                workflow_ready_owned_command(
                    "record_attestation",
                    command_hint(commands, "apply_command"),
                    apply_owned_continuation,
                    Some(apply_command.clone()),
                    command_hint(commands, "attest_command")
                        .or_else(|| command_hint(commands, "attestation_command")),
                )
            },
        });
    }
    let patchset_id = if patchset_id.is_empty() {
        "unknown".to_string()
    } else {
        patchset_id
    };
    json!({
        "code": "done",
        "summary": "Ready phase is complete.",
        "detail": format!("Patchset `{patchset_id}` and its CI/Attestation evidence are ready for reviewer-owned `workflow finish`."),
        "command": land_command,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn workflow_land_next_action(
    facts: &JsonValue,
    commands: &JsonValue,
    apply_owned_continuation: bool,
    landed_fast_path: bool,
    base_is_fresh_override: bool,
    workspace_matches_patchset_override: bool,
    policy_override: Option<Map<String, JsonValue>>,
    policy_decision_override: &str,
    landing_summary_override: Option<Map<String, JsonValue>>,
    task_review_required: bool,
) -> JsonValue {
    let change = field_obj(facts, "change");
    let task = field_obj(facts, "task");
    let workspace = field_obj(facts, "workspace");
    let patchset = optional_obj_field(facts, "patchset");
    let attestation = optional_obj_field(facts, "attestation");
    let patchset_ci_status = optional_obj_field(facts, "patchset_ci_status");
    let tests_state = string_field(facts, "tests_state");
    let review_blocking = int_field(facts, "review_blocking");
    let target_line = string_field(facts, "target_line");
    let ignore_workspace_authoring = bool_field(facts, "ignore_workspace_authoring");
    let policy = policy_override
        .clone()
        .or_else(|| optional_obj_field(facts, "policy"));
    let landing_summary = landing_summary_override
        .clone()
        .or_else(|| optional_obj_field(facts, "landing_summary"));
    let policy_decision = if policy_decision_override.is_empty() {
        string_field(facts, "policy_decision")
    } else {
        policy_decision_override.to_string()
    };
    let change_id = optional_string_field(&change, "change_id").unwrap_or_default();
    let apply_command = command_hint(commands, "apply_command")
        .unwrap_or_else(|| format!("ait task finish {change_id}"));
    let ready_command = command_hint(commands, "ready_command")
        .unwrap_or_else(|| format!("ait workflow ready {change_id} --apply"));
    let landing_status = workflow_land_submission_status(landing_summary.as_ref());
    let landing_submission_id = workflow_land_submission_id(landing_summary.as_ref());
    let landing_result = workflow_land_result(landing_summary.as_ref());
    let landing_blocker_class = workflow_land_result_blocker_class(&landing_result);
    let stale_policy_blocker_cleared = workflow_land_stale_policy_blocker_cleared(
        &landing_status,
        &landing_blocker_class,
        &policy_decision,
    );
    let change_is_landed = change_effectively_landed(&change, landing_summary.as_ref());
    if change_is_landed && string_field(&task, "status") != "completed" {
        return json!({
            "code": "complete_task",
            "summary": "Complete the task now that the change is landed.",
            "detail": "The workflow record should match the landed reality.",
            "command": command_hint_json(commands, "task_land_command"),
        });
    }
    if landed_fast_path || change_is_landed {
        return json!({
            "code": "done",
            "summary": "This change and task are already fully landed.",
            "detail": "No further land workflow action is required.",
            "command": JsonValue::Null,
        });
    }
    if !ignore_workspace_authoring && !bool_field(&workspace, "clean") {
        return json!({
            "code": "workflow_ready",
            "summary": "Run workflow ready before review or land.",
            "detail": "The workspace still has unsaved changes.",
            "command": ready_command,
        });
    }
    if patchset.is_none()
        || !base_is_fresh_override
        || (!workspace_matches_patchset_override && !bool_field(facts, "patchset_is_authoritative"))
    {
        if patchset.is_some() {
            if let Some(refresh_context) = optional_obj_field(facts, "patchset_refresh") {
                return json!({
                    "code": "workflow_ready",
                    "summary": "Run workflow ready before review or land.",
                    "detail": optional_string_field(&refresh_context, "detail").unwrap_or_else(|| "The selected patchset is stale and needs a fresh publish from the current line.".to_string()),
                    "command": ready_command,
                    "refresh_context": JsonValue::Object(refresh_context),
                });
            }
        }
        return json!({
            "code": "workflow_ready",
            "summary": "Run workflow ready before review or land.",
            "detail": "The land workflow still needs a ready patchset from the current line.",
            "command": ready_command,
        });
    }
    if command_hint(commands, "patchset_ci_command").is_some() {
        match patchset_ci_gate_state(patchset_ci_status.as_ref()) {
            PatchsetCiGateState::Pass => {}
            PatchsetCiGateState::PendingWithJob => {
                return json!({
                    "code": "workflow_ready",
                    "summary": "Run workflow ready before review or land.",
                    "detail": "Patchset CI is still running for the selected patchset.",
                    "command": ready_command,
                });
            }
            PatchsetCiGateState::NeedsRun => {
                return json!({
                    "code": "workflow_ready",
                    "summary": "Run workflow ready before review or land.",
                    "detail": if matches!(tests_state.as_str(), "fail" | "failed" | "hard_fail" | "soft_fail") {
                        format!("Patchset CI last reported tests `{tests_state}` for the selected patchset.")
                    } else {
                        "Remote patchset CI evidence is missing for the selected patchset; local or manual tests pass is not enough for land.".to_string()
                    },
                    "command": ready_command,
                });
            }
        }
    }
    if attestation.is_none()
        || (!tests_state.is_empty() && !matches!(tests_state.as_str(), "pass" | "not_required"))
    {
        return json!({
            "code": "workflow_ready",
            "summary": "Run workflow ready before review or land.",
            "detail": "Attestation evidence is still missing or incomplete for the selected patchset.",
            "command": ready_command,
        });
    }
    if review_blocking > 0 {
        return json!({
            "code": "address_blocking_review",
            "summary": "Resolve the blocking review feedback before land.",
            "detail": "A blocking review is already recorded on this change.",
            "command": command_hint_json(commands, "review_command"),
        });
    }
    if bool_field(facts, "requires_code_review_summary")
        && int_field(facts, "code_review_summary_count") <= 0
    {
        return json!({
            "code": "record_code_review_summary",
            "summary": "Record AI code review before Task approval or Land.",
            "detail": "An AI agent must inspect this exact Patchset and submit the structured pass-ready summary. Task Land only consumes already-ready review state.",
            "command": command_hint_json(commands, "code_review_summary_command"),
        });
    }
    if int_field(facts, "task_review_approvals") <= 0 {
        let auto_review_reviewer = command_hint(commands, "auto_review_reviewer");
        let team_review_available = command_hint(commands, "team_review_command").is_some();
        return json!({
            "code": "record_review",
            "summary": if task_review_required {
                "Record the required task review for this change."
            } else if auto_review_reviewer.is_some() {
                "Auto-record the required task approval for this change."
            } else {
                "Record the required task approval for this change."
            },
            "detail": if task_review_required {
                "Land still needs task/outcome approval.".to_string()
            } else if let Some(reviewer) = auto_review_reviewer.clone() {
                let mut detail = format!(
                    "Task/outcome review auto approval is configured. Reviewer-owned Workflow Finish or a successful direct AI code review can record `task_approve` as `{reviewer}` before atomic Task Land."
                );
                if team_review_available {
                    detail.push_str(" Preserved team review remains available separately in `team_remote`.");
                }
                detail
            } else {
                let mut detail =
                    "Task/outcome review auto approval is configured, but `ait config` `user_name` is not set."
                        .to_string();
                if team_review_available {
                    detail.push_str(" Preserved team review remains available separately in `team_remote`.");
                }
                detail
            },
            "command": workflow_land_owned_command(
                "record_review",
                command_hint(commands, "apply_command"),
                apply_owned_continuation,
                Some(apply_command.clone()),
                command_hint(commands, "review_command"),
            ),
        });
    }
    if policy_decision != "pass"
        && matches!(
            policy_decision.trim().to_ascii_lowercase().as_str(),
            "" | "pending" | "not_evaluated" | "stale" | "unknown"
        )
    {
        return json!({
            "code": "evaluate_policy",
            "summary": "Evaluate policy after reviewer approval.",
            "detail": "Workflow Finish owns the final Policy evaluation before it delegates atomic closeout to Task Land.",
            "command": workflow_land_owned_command(
                "evaluate_policy",
                command_hint(commands, "apply_command"),
                apply_owned_continuation,
                Some(apply_command.clone()),
                command_hint(commands, "policy_command"),
            ),
        });
    }
    if policy_decision != "pass" {
        return json!({
            "code": "land_blocked",
            "summary": "Clear the current land preflight blocker before retrying remote land.",
            "detail": workflow_land_policy_blocker_detail(policy.as_ref(), None, Some(policy_decision.as_str())),
            "command": JsonValue::Null,
        });
    }
    if WORKFLOW_LAND_PENDING_STATUSES.contains(&landing_status.as_str()) {
        return json!({
            "code": "waiting_for_land",
            "summary": "Wait for the remote land submission to reach a terminal result.",
            "detail": if let Some(submission_id) = &landing_submission_id {
                format!("Remote land submission `{submission_id}` is currently `{landing_status}`.")
            } else {
                "Remote land submission is still pending.".to_string()
            },
            "command": JsonValue::String(apply_command.clone()),
        });
    }
    if landing_status == "blocked" && !stale_policy_blocker_cleared {
        let detail = if landing_blocker_class == "POLICY_BLOCKED" {
            workflow_land_policy_blocker_detail(
                landing_result.get("policy").and_then(JsonValue::as_object),
                landing_submission_id.as_deref(),
                Some(policy_decision.as_str()),
            )
        } else if let Some(submission_id) = &landing_submission_id {
            if !landing_blocker_class.is_empty() {
                format!(
                    "Remote land submission `{submission_id}` is blocked by `{landing_blocker_class}`."
                )
            } else {
                "Remote land is blocked and needs manual resolution before retrying.".to_string()
            }
        } else {
            "Remote land is blocked and needs manual resolution before retrying.".to_string()
        };
        return json!({
            "code": "land_blocked",
            "summary": "Clear the current land blocker before retrying remote land.",
            "detail": detail,
            "command": JsonValue::Null,
        });
    }
    json!({
        "code": "submit_land",
        "summary": "Submit the approved patchset for landing.",
        "detail": format!("The selected Patchset is ready to submit onto `{target_line}`. `task finish` will re-evaluate Policy as part of Land preflight."),
        "command": workflow_land_owned_command(
            "submit_land",
            command_hint(commands, "apply_command"),
            apply_owned_continuation,
            Some(apply_command),
            command_hint(commands, "land_command"),
        ),
    })
}

pub(crate) fn workflow_land_owned_command(
    code: &str,
    apply_command: Option<String>,
    apply_owned_continuation: bool,
    resolved_apply_command: Option<String>,
    fallback_command: Option<String>,
) -> JsonValue {
    workflow_owned_command(
        WORKFLOW_LAND_APPLY_OWNED_CODES,
        code,
        apply_command,
        apply_owned_continuation,
        resolved_apply_command,
        fallback_command,
    )
}

pub(crate) fn workflow_ready_owned_command(
    code: &str,
    apply_command: Option<String>,
    apply_owned_continuation: bool,
    resolved_apply_command: Option<String>,
    fallback_command: Option<String>,
) -> JsonValue {
    workflow_owned_command(
        WORKFLOW_READY_APPLY_OWNED_CODES,
        code,
        apply_command,
        apply_owned_continuation,
        resolved_apply_command,
        fallback_command,
    )
}

fn workflow_owned_command(
    apply_owned_codes: &[&str],
    code: &str,
    apply_command: Option<String>,
    apply_owned_continuation: bool,
    resolved_apply_command: Option<String>,
    fallback_command: Option<String>,
) -> JsonValue {
    let owned_command = apply_command.or(resolved_apply_command);
    match (
        apply_owned_continuation && apply_owned_codes.contains(&code),
        owned_command,
    ) {
        (true, Some(command)) => JsonValue::String(command),
        _ => fallback_command
            .map(JsonValue::String)
            .unwrap_or(JsonValue::Null),
    }
}

#[cfg(test)]
mod tests;
