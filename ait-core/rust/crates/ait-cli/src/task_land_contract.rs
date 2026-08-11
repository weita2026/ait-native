use ait_core::json_support::{json, JsonValue};

pub const TASK_LAND_CONTRACT_VERSION: &str = "task-land-plan-closeout/v1";

pub const TASK_LAND_COMMAND_ABOUT: &str = "Land one task or change using workflow-mode scope defaults. solo_local lands only local draft state and, when the final Change finishes a bound Task, closes and locally syncs its exact Plan item. Final-Task closeout removes the worktree and archives the exact accepted-head feature Line. Remote closeout consumes an already-ready Patchset and leaves Plan state untouched; it archives the matching local and remote feature Line. --local or --remote overrides the configured scope.";

pub const PLAN_SYNC_COMMAND_ABOUT: &str = "Reconcile file-backed Markdown into Plan revision lineage. --local writes only local Plan state; --remote publishes the touched local heads. Plan sync never creates a Snapshot or advances a Line.";

pub const LOCAL_PLAN_CLOSEOUT_POLICY: &str = "automatic_exact_local_when_final_task_completed";
pub const REMOTE_PLAN_CLOSEOUT_POLICY: &str = "separate_after_land";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TaskLandScopeContract {
    pub scope: &'static str,
    pub readiness_policy: &'static str,
    pub plan_closeout_policy: &'static str,
    pub remote_contact_policy: &'static str,
    pub recovery_policy: &'static str,
}

pub const LOCAL_TASK_LAND_CONTRACT: TaskLandScopeContract = TaskLandScopeContract {
    scope: "local",
    readiness_policy: "local_admission",
    plan_closeout_policy: LOCAL_PLAN_CLOSEOUT_POLICY,
    remote_contact_policy: "explicit_only",
    recovery_policy: "idempotent_phase_resume",
};

pub const REMOTE_TASK_LAND_CONTRACT: TaskLandScopeContract = TaskLandScopeContract {
    scope: "remote",
    readiness_policy: "already_ready_selected_patchset",
    plan_closeout_policy: REMOTE_PLAN_CLOSEOUT_POLICY,
    remote_contact_policy: "configured_or_explicit_remote",
    recovery_policy: "authoritative_remote_receipt_resume",
};

pub fn task_land_scope_contract(use_local_scope: bool) -> TaskLandScopeContract {
    if use_local_scope {
        LOCAL_TASK_LAND_CONTRACT
    } else {
        REMOTE_TASK_LAND_CONTRACT
    }
}

pub fn task_land_scope_contract_json(use_local_scope: bool) -> JsonValue {
    let contract = task_land_scope_contract(use_local_scope);
    json!({
        "version": TASK_LAND_CONTRACT_VERSION,
        "scope": contract.scope,
        "readiness_policy": contract.readiness_policy,
        "plan_closeout_policy": contract.plan_closeout_policy,
        "remote_contact_policy": contract.remote_contact_policy,
        "recovery_policy": contract.recovery_policy,
    })
}

pub fn attach_task_land_contract(output: &mut JsonValue, use_local_scope: bool) {
    let outcome = task_land_closeout_outcome(output, use_local_scope);
    let recovery = (outcome == "partial").then(|| task_land_recovery(output, use_local_scope));
    let Some(object) = output.as_object_mut() else {
        return;
    };
    object.insert(
        "task_land_contract".to_string(),
        task_land_scope_contract_json(use_local_scope),
    );
    object.insert(
        "closeout_status".to_string(),
        JsonValue::String(outcome.to_string()),
    );
    if let Some(recovery) = recovery {
        object.insert("closeout_recovery".to_string(), recovery);
    } else {
        object.remove("closeout_recovery");
    }
}

pub fn task_land_closeout_outcome(output: &JsonValue, use_local_scope: bool) -> &'static str {
    if output.get("apply_status").and_then(JsonValue::as_str) != Some("done") {
        return "preview";
    }
    for key in ["local_line_sync", "main_seed_sync"] {
        if output
            .get(key)
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("status"))
            .and_then(JsonValue::as_str)
            == Some("failed")
        {
            return "partial";
        }
    }
    for key in [
        "repo_root_restore",
        "bound_worktree_cleanup",
        "bound_line_closeout",
    ] {
        if output
            .get(key)
            .and_then(JsonValue::as_object)
            .and_then(|value| value.get("status"))
            .and_then(JsonValue::as_str)
            == Some("failed")
        {
            return "partial";
        }
    }
    let plan_closeout = output
        .get("plan_checklist_closeout")
        .and_then(JsonValue::as_object);
    if plan_closeout
        .and_then(|value| value.get("reason"))
        .and_then(JsonValue::as_str)
        == Some("task_still_active")
    {
        return "change_landed_task_active";
    }
    if !use_local_scope {
        return "execution_complete_plan_separate";
    }
    if output.get("task_status").and_then(JsonValue::as_str) != Some("completed") {
        return "change_landed_task_active";
    }
    match plan_closeout
        .and_then(|value| value.get("status"))
        .and_then(JsonValue::as_str)
    {
        Some("synced") => "complete",
        Some("skipped")
            if plan_closeout
                .and_then(|value| value.get("reason"))
                .and_then(JsonValue::as_str)
                == Some("no_plan_binding") =>
        {
            "complete_unbound"
        }
        Some("deferred")
            if plan_closeout
                .and_then(|value| value.get("reason"))
                .and_then(JsonValue::as_str)
                == Some("task_still_active") =>
        {
            "change_landed_task_active"
        }
        _ => "partial",
    }
}

fn task_land_recovery(output: &JsonValue, use_local_scope: bool) -> JsonValue {
    let change_id = output
        .get("change")
        .and_then(|change| change.get("change_ref"))
        .and_then(JsonValue::as_str)
        .or_else(|| output.get("change_ref").and_then(JsonValue::as_str))
        .or_else(|| output.get("change_id").and_then(JsonValue::as_str))
        .or_else(|| {
            output
                .get("change")
                .and_then(|change| change.get("change_id"))
                .and_then(JsonValue::as_str)
        })
        .unwrap_or("<task-or-change-id>");
    let scope_flag = if use_local_scope { " --local" } else { "" };
    json!({
        "code": "resume_task_land_closeout",
        "idempotent": true,
        "command": format!("ait task land {change_id}{scope_flag}"),
        "detail": "The code land is already authoritative. Repair the reported Plan, worktree, or feature-Line condition, then rerun task land; completed phases are inspected and reused instead of being applied twice.",
    })
}

pub fn task_land_exit_code(output: &JsonValue) -> u8 {
    if output.get("closeout_status").and_then(JsonValue::as_str) == Some("partial") {
        2
    } else {
        0
    }
}

pub fn attach_task_audit_land_contract(output: &mut JsonValue, use_local_scope: bool) {
    let contract = task_land_scope_contract_json(use_local_scope);
    let task = output.get("task").filter(|value| value.is_object());
    let task_id = task
        .and_then(|value| value.get("task_id"))
        .and_then(JsonValue::as_str)
        .or_else(|| output.get("task_id").and_then(JsonValue::as_str))
        .unwrap_or("<task-id>");
    let task_status = task
        .and_then(|value| value.get("status"))
        .and_then(JsonValue::as_str)
        .unwrap_or("unknown");
    let open_changes = output
        .get("summary")
        .and_then(|value| value.get("open_changes"))
        .and_then(JsonValue::as_u64)
        .unwrap_or(0);
    let plan_id = task
        .and_then(|value| value.get("plan_id"))
        .and_then(JsonValue::as_str);
    let plan_item_ref = task
        .and_then(|value| value.get("plan_item_ref"))
        .and_then(JsonValue::as_str);
    let plan_evidence = output
        .get("bound_plan_closeout")
        .filter(|value| value.is_object())
        .cloned();
    let (status, recovery) = if task_status == "completed" && use_local_scope {
        (
            "inspect_or_resume_local_closeout",
            json!({
                "code": "resume_task_land_closeout",
                "idempotent": true,
                "command": format!("ait task land {task_id} --local"),
                "detail": "The Task is completed locally. Rerun task land to inspect and resume any exact Plan or bound-worktree closeout that did not converge.",
            }),
        )
    } else if task_status == "completed" && plan_id.is_none() {
        ("complete_unbound", JsonValue::Null)
    } else if task_status == "completed"
        && plan_evidence
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(JsonValue::as_str)
            == Some("done")
    {
        ("complete", JsonValue::Null)
    } else if task_status == "completed"
        && plan_evidence
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(JsonValue::as_str)
            == Some("pending")
        && plan_evidence
            .as_ref()
            .and_then(|value| value.get("artifact_path"))
            .and_then(JsonValue::as_str)
            .is_some()
    {
        let artifact_path = plan_evidence
            .as_ref()
            .and_then(|value| value.get("artifact_path"))
            .and_then(JsonValue::as_str)
            .unwrap_or("<bound-sprint-card-path>");
        let remote_name = plan_evidence
            .as_ref()
            .and_then(|value| value.get("remote"))
            .and_then(JsonValue::as_str)
            .unwrap_or("<remote>");
        (
            "execution_complete_plan_separate",
            json!({
                "code": "sync_bound_plan_separately",
                "idempotent": true,
                "command": format!("ait plan sync {artifact_path} --remote {remote_name}"),
                "detail": "The exact bound item is still open in the current remote Plan head. Mark it complete in the bound sprint card, then synchronize that card separately.",
            }),
        )
    } else if task_status == "completed" {
        let plan_id = plan_id.unwrap_or("<plan-id>");
        let remote_name = plan_evidence
            .as_ref()
            .and_then(|value| value.get("remote"))
            .and_then(JsonValue::as_str)
            .unwrap_or("<remote>");
        (
            "plan_closeout_unverified",
            json!({
                "code": "inspect_bound_plan",
                "idempotent": true,
                "command": format!("ait plan show {plan_id} --remote {remote_name}"),
                "detail": if plan_item_ref.is_some() {
                    "The remote Task is complete, but current evidence does not prove whether the exact bound Plan item is done. Inspect the Plan before deciding whether a separate sync is needed."
                } else {
                    "The remote Task is complete, but its Plan binding has no exact item reference. Inspect the Plan binding before attempting closeout."
                },
            }),
        )
    } else if open_changes > 0 {
        ("pending_open_changes", JsonValue::Null)
    } else {
        ("pending_task_land", JsonValue::Null)
    };
    let Some(object) = output.as_object_mut() else {
        return;
    };
    object.insert("task_land_contract".to_string(), contract.clone());
    object.insert(
        "task_land_closeout".to_string(),
        json!({
            "status": status,
            "scope": contract["scope"].clone(),
            "plan_closeout_policy": contract["plan_closeout_policy"].clone(),
            "evidence": plan_evidence.unwrap_or(JsonValue::Null),
            "recovery": recovery,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_matrix_keeps_local_and_remote_plan_policies_distinct() {
        assert_eq!(
            task_land_scope_contract(true).plan_closeout_policy,
            LOCAL_PLAN_CLOSEOUT_POLICY
        );
        assert_eq!(
            task_land_scope_contract(false).plan_closeout_policy,
            REMOTE_PLAN_CLOSEOUT_POLICY
        );
        assert_eq!(
            task_land_scope_contract(true).remote_contact_policy,
            "explicit_only"
        );
    }

    #[test]
    fn local_bound_closeout_failure_is_partial_and_uses_exit_two() {
        let mut payload = json!({
            "apply_status": "done",
            "task_status": "completed",
            "plan_checklist_closeout": {"status": "failed", "error": "fixture"},
        });
        attach_task_land_contract(&mut payload, true);
        assert_eq!(payload["closeout_status"], "partial");
        assert_eq!(task_land_exit_code(&payload), 2);
    }

    #[test]
    fn remote_land_records_separate_plan_closeout_as_successful_execution() {
        let mut payload = json!({
            "apply_status": "done",
            "task_status": "completed",
            "plan_checklist_closeout": {
                "status": "deferred",
                "reason": "remote_plan_sync_is_separate_from_task_land"
            },
        });
        attach_task_land_contract(&mut payload, false);
        assert_eq!(
            payload["closeout_status"],
            "execution_complete_plan_separate"
        );
        assert_eq!(task_land_exit_code(&payload), 0);
    }

    #[test]
    fn non_final_local_change_does_not_close_the_task_or_fail_the_command() {
        let mut payload = json!({
            "apply_status": "done",
            "task_status": "active",
            "plan_checklist_closeout": {
                "status": "deferred",
                "reason": "task_still_active"
            },
        });
        attach_task_land_contract(&mut payload, true);
        assert_eq!(payload["closeout_status"], "change_landed_task_active");
        assert_eq!(task_land_exit_code(&payload), 0);
        assert!(payload.get("closeout_recovery").is_none());
    }

    #[test]
    fn remote_post_land_cleanup_failure_is_partial_and_recoverable() {
        let mut payload = json!({
            "apply_status": "done",
            "change_id": "RCT-4/C-02",
            "task_status": "completed",
            "bound_worktree_cleanup": {
                "status": "failed",
                "error": "fixture"
            },
            "plan_checklist_closeout": {
                "status": "deferred",
                "reason": "remote_plan_sync_is_separate_from_task_land"
            },
        });
        attach_task_land_contract(&mut payload, false);
        assert_eq!(payload["closeout_status"], "partial");
        assert_eq!(task_land_exit_code(&payload), 2);
        assert_eq!(
            payload["closeout_recovery"]["command"],
            "ait task land RCT-4/C-02"
        );
        assert_eq!(payload["closeout_recovery"]["idempotent"], true);
    }

    #[test]
    fn feature_line_closeout_failure_is_partial_and_recoverable() {
        let mut payload = json!({
            "apply_status": "done",
            "change_id": "RCT-5/C-01",
            "task_status": "completed",
            "bound_line_closeout": {
                "status": "failed",
                "reason": "feature_line_closeout_failed",
                "error": "head drift"
            },
            "plan_checklist_closeout": {
                "status": "deferred",
                "reason": "remote_plan_sync_is_separate_from_task_land"
            },
        });
        attach_task_land_contract(&mut payload, false);
        assert_eq!(payload["closeout_status"], "partial");
        assert_eq!(task_land_exit_code(&payload), 2);
        assert!(payload["closeout_recovery"]["detail"]
            .as_str()
            .unwrap()
            .contains("feature-Line"));
    }

    #[test]
    fn task_audit_requires_evidence_before_recommending_remote_plan_sync() {
        let mut payload = json!({
            "task": {
                "task_id": "RCT-8",
                "status": "completed",
                "plan_id": "PR-8",
                "plan_item_ref": "release/fix"
            },
            "summary": {"open_changes": 0},
        });
        attach_task_audit_land_contract(&mut payload, false);
        assert_eq!(
            payload["task_land_closeout"]["status"],
            "plan_closeout_unverified"
        );
        assert_eq!(
            payload["task_land_closeout"]["plan_closeout_policy"],
            REMOTE_PLAN_CLOSEOUT_POLICY
        );
        assert_eq!(
            payload["task_land_closeout"]["recovery"]["code"],
            "inspect_bound_plan"
        );
    }

    #[test]
    fn task_audit_done_plan_evidence_closes_without_recovery() {
        let mut payload = json!({
            "task": {
                "task_id": "RCT-8",
                "status": "completed",
                "plan_id": "PR-8",
                "plan_item_ref": "release/fix"
            },
            "summary": {"open_changes": 0},
            "bound_plan_closeout": {
                "status": "done",
                "remote": "origin",
                "plan_id": "PR-8",
                "plan_item_ref": "release/fix"
            }
        });
        attach_task_audit_land_contract(&mut payload, false);
        assert_eq!(payload["task_land_closeout"]["status"], "complete");
        assert!(payload["task_land_closeout"]["recovery"].is_null());
    }

    #[test]
    fn task_audit_open_plan_evidence_emits_exact_sync_command() {
        let mut payload = json!({
            "task": {
                "task_id": "RCT-8",
                "status": "completed",
                "plan_id": "PR-8",
                "plan_item_ref": "release/fix"
            },
            "summary": {"open_changes": 0},
            "bound_plan_closeout": {
                "status": "pending",
                "remote": "origin",
                "artifact_path": "docs/sprints/release.md"
            }
        });
        attach_task_audit_land_contract(&mut payload, false);
        assert_eq!(
            payload["task_land_closeout"]["status"],
            "execution_complete_plan_separate"
        );
        assert_eq!(
            payload["task_land_closeout"]["recovery"]["command"],
            "ait plan sync docs/sprints/release.md --remote origin"
        );
    }
}
