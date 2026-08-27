use super::*;

pub(crate) fn workflow_landed_steps_and_suggested_commands(
    facts: &JsonValue,
    next_action: &JsonValue,
) -> (JsonValue, JsonValue) {
    let patchset_label = optional_nonempty_string(facts, "patchset_label");
    let patchset_detail = if let Some(label) = patchset_label {
        format!("Patchset `{label}` is already part of the landed history for this change.")
    } else {
        "A landed change already implies patchset publication succeeded earlier.".to_string()
    };
    let steps = json!([
        workflow_land_step(
            "snapshot",
            "Snapshot",
            "done",
            "No additional authoring snapshot is required because the change is already landed.",
            None
        ),
        workflow_land_step("patchset", "Patchset", "done", &patchset_detail, None),
        workflow_land_step(
            "attestation",
            "Attestation",
            "done",
            "Landing already succeeded, so attestation gating has already cleared.",
            None
        ),
        workflow_land_step(
            "review",
            "Review",
            "done",
            "Landing already succeeded, so review requirements were already satisfied.",
            None
        ),
        workflow_land_step(
            "policy",
            "Policy",
            "done",
            "Landing already succeeded, so policy has already cleared.",
            None
        ),
        workflow_land_step(
            "land",
            "Land",
            "done",
            &format!(
                "Change `{}` already landed on `{}`.",
                optional_string_field(&field_obj(facts, "change"), "change_id")
                    .unwrap_or_else(|| "unknown".to_string()),
                string_field(facts, "target_line")
            ),
            None
        ),
    ]);
    let suggested = if let Some(command) = optional_string_field(next_action, "command") {
        json!([command])
    } else {
        json!([])
    };
    (steps, suggested)
}

pub(crate) fn workflow_land_full_steps(
    facts: &JsonValue,
    command_hints: &JsonValue,
    task_review_required: bool,
    auto_review_reviewer: Option<&str>,
    _apply_owned_continuation: bool,
) -> Vec<JsonValue> {
    let mut steps = Vec::new();
    let workspace = field_obj(facts, "workspace");
    let patchset = optional_obj_field(facts, "patchset");
    let patchset_id = patchset
        .as_ref()
        .and_then(|value| optional_string_field(value, "patchset_id"))
        .unwrap_or_default();
    let attestation = optional_obj_field(facts, "attestation");
    let policy = optional_obj_field(facts, "policy");
    let landing_summary = optional_obj_field(facts, "landing_summary");
    let worktree_retarget = optional_obj_field(facts, "worktree_retarget");
    let patchset_refresh = optional_obj_field(facts, "patchset_refresh");
    let change = field_obj(facts, "change");
    let publish_command = command_hint(command_hints, "publish_command");
    let patchset_ci_command = command_hint(command_hints, "patchset_ci_command");
    let attestation_command = command_hint(command_hints, "attestation_command")
        .or_else(|| command_hint(command_hints, "attest_command"));
    let review_command = command_hint(command_hints, "review_command");
    let land_command = command_hint(command_hints, "land_command");
    let policy_has_checks = workflow_land_policy_has_checks(policy.as_ref());

    if bool_field(facts, "ignore_workspace_authoring") {
        steps.push(workflow_land_step("snapshot", "Snapshot", "done", "Final-snapshot remote promotion is using the completed local target-line snapshot and does not require repo-root authoring state.", None));
    } else if !bool_field(&workspace, "clean") {
        steps.push(workflow_land_step(
            "snapshot",
            "Snapshot",
            "pending",
            "Workspace changes are still dirty, so publishable land state needs a fresh snapshot first.",
            Some("ait snapshot create --message \"reviewable snapshot\"".to_string()),
        ));
    } else {
        steps.push(workflow_land_step(
            "snapshot",
            "Snapshot",
            "done",
            &format!(
                "The current line `{}` is already captured at `{}`.",
                string_field(facts, "current_line_name"),
                optional_string_field(facts, "revision_snapshot_id")
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            None,
        ));
    }

    if patchset.is_none() {
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "pending",
            "No published patchset exists yet for this change.",
            publish_command,
        ));
    } else if bool_field(facts, "patchset_is_authoritative") {
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "done",
            &format!(
                "Patchset `{}` is already prepared for final-snapshot remote land.",
                patchset_id
            ),
            None,
        ));
    } else if let Some(retarget) = &worktree_retarget {
        let rebase_state =
            optional_string_field(retarget, "rebase_state").unwrap_or_else(|| "idle".to_string());
        if rebase_state == "conflicted" {
            let detail = patchset_refresh
                .as_ref()
                .and_then(|value| optional_string_field(value, "detail"))
                .unwrap_or_else(|| {
                    let paths = retarget
                        .get("rebase_conflict_paths")
                        .and_then(JsonValue::as_array)
                        .map(|values| {
                            values
                                .iter()
                                .take(5)
                                .filter_map(|value| value.as_str().map(str::to_string))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    format!(
                        "The bound worktree is paused on conflicted rebase paths: {}.",
                        if paths.is_empty() {
                            "resolve conflicts first".to_string()
                        } else {
                            paths.join(", ")
                        }
                    )
                });
            steps.push(workflow_land_step(
                "patchset",
                "Patchset",
                "stale",
                &detail,
                publish_command.clone(),
            ));
        } else if bool_field(retarget, "needs_retarget") {
            let detail = patchset_refresh
                .as_ref()
                .and_then(|value| optional_string_field(value, "detail"))
                .unwrap_or_else(|| {
                    format!(
                        "The bound worktree still forks from `{}` while `{}` now points at `{}`.",
                        optional_string_field(retarget, "fork_snapshot_id").unwrap_or_default(),
                        string_field(facts, "base_line_name"),
                        optional_string_field(retarget, "target_base_snapshot_id")
                            .unwrap_or_default()
                    )
                });
            steps.push(workflow_land_step(
                "patchset",
                "Patchset",
                "stale",
                &detail,
                publish_command.clone(),
            ));
        } else if !bool_field(facts, "base_is_fresh") {
            let detail = patchset_refresh
                .as_ref()
                .and_then(|value| optional_string_field(value, "detail"))
                .unwrap_or_else(|| format!("The base line `{}` moved to `{}` after patchset `{patchset_id}` was published.", string_field(facts, "base_line_name"), optional_string_field(facts, "remote_base_snapshot_id").unwrap_or_default()));
            steps.push(workflow_land_step(
                "patchset",
                "Patchset",
                "stale",
                &detail,
                publish_command.clone(),
            ));
        } else if matches!(
            optional_bool_field(facts, "workspace_matches_patchset"),
            Some(false)
        ) {
            let detail = patchset_refresh
                .as_ref()
                .and_then(|value| optional_string_field(value, "detail"))
                .unwrap_or_else(|| {
                    format!(
                        "The current line head `{}` no longer matches patchset `{patchset_id}`.",
                        optional_string_field(facts, "revision_snapshot_id").unwrap_or_default()
                    )
                });
            steps.push(workflow_land_step(
                "patchset",
                "Patchset",
                "stale",
                &detail,
                publish_command.clone(),
            ));
        } else {
            steps.push(workflow_land_step(
                "patchset",
                "Patchset",
                "done",
                &format!(
                    "Patchset `{}` is published for `{}`.",
                    patchset_id,
                    string_field(facts, "resolved_change_id")
                ),
                None,
            ));
        }
    } else if !bool_field(facts, "base_is_fresh") {
        let detail = patchset_refresh
            .as_ref()
            .and_then(|value| optional_string_field(value, "detail"))
            .unwrap_or_else(|| format!("The base line `{}` moved to `{}` after patchset `{patchset_id}` was published.", string_field(facts, "base_line_name"), optional_string_field(facts, "remote_base_snapshot_id").unwrap_or_default()));
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "stale",
            &detail,
            publish_command.clone(),
        ));
    } else if matches!(
        optional_bool_field(facts, "workspace_matches_patchset"),
        Some(false)
    ) {
        let detail = patchset_refresh
            .as_ref()
            .and_then(|value| optional_string_field(value, "detail"))
            .unwrap_or_else(|| {
                format!(
                    "The current line head `{}` no longer matches patchset `{patchset_id}`.",
                    optional_string_field(facts, "revision_snapshot_id").unwrap_or_default()
                )
            });
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "stale",
            &detail,
            publish_command.clone(),
        ));
    } else {
        steps.push(workflow_land_step(
            "patchset",
            "Patchset",
            "done",
            &format!(
                "Patchset `{}` is published for `{}`.",
                patchset_id,
                string_field(facts, "resolved_change_id")
            ),
            None,
        ));
    }

    if patchset.is_none() {
        steps.push(workflow_land_step(
            "attestation",
            "Attestation",
            "waiting",
            "Attestation starts after a patchset exists.",
            None,
        ));
    } else if attestation.is_none() {
        let detail = if patchset_ci_command.is_some() {
            format!("No attestation is recorded yet for patchset `{patchset_id}`. Run workflow ready so it completes Patchset CI and records the compact Attestation gate statement.")
        } else {
            format!("No attestation is recorded yet for patchset `{patchset_id}`.")
        };
        steps.push(workflow_land_step(
            "attestation",
            "Attestation",
            "pending",
            &detail,
            attestation_command,
        ));
    } else if !string_field(facts, "tests_state").is_empty()
        && !matches!(
            string_field(facts, "tests_state").as_str(),
            "pass" | "not_required"
        )
    {
        let tests_state = string_field(facts, "tests_state");
        let step_status = if tests_state == "pending" {
            "pending"
        } else {
            "blocked"
        };
        let detail = if patchset_ci_command.is_some() {
            format!(
                "Patchset CI currently reports tests `{tests_state}` for patchset `{patchset_id}`."
            )
        } else {
            format!("Tests are currently `{tests_state}` for patchset `{patchset_id}`.")
        };
        steps.push(workflow_land_step(
            "attestation",
            "Attestation",
            step_status,
            &detail,
            attestation_command,
        ));
    } else {
        let attestation_id = optional_string_field(attestation.as_ref().unwrap(), "attestation_id")
            .unwrap_or_default();
        let tests_state = string_field(facts, "tests_state");
        let suffix = if tests_state.is_empty() {
            ".".to_string()
        } else {
            format!(" with tests `{tests_state}`.")
        };
        steps.push(workflow_land_step(
            "attestation",
            "Attestation",
            "done",
            &format!("Attestation `{attestation_id}` is recorded{suffix}"),
            None,
        ));
    }

    let code_review_state = if int_field(facts, "code_review_summary_count") > 0 {
        "recorded".to_string()
    } else if bool_field(facts, "requires_code_review_summary") {
        "required, not recorded".to_string()
    } else {
        "not required".to_string()
    };

    if patchset.is_none() {
        steps.push(workflow_land_step(
            "review",
            "Review",
            "waiting",
            "Review starts after a patchset exists.",
            None,
        ));
    } else if int_field(facts, "review_blocking") > 0 {
        let task_review_state = if int_field(facts, "task_review_approvals") > 0 {
            "approved"
        } else {
            "pending"
        };
        let team_review_state = if int_field(facts, "team_review_approvals") > 0 {
            format!("{} approval(s)", int_field(facts, "team_review_approvals"))
        } else if command_hint(command_hints, "team_review_command").is_some() {
            "available in team_remote".to_string()
        } else {
            "unavailable outside team_remote".to_string()
        };
        steps.push(workflow_land_step("review", "Review", "blocked", &format!("Code review: {code_review_state}; Task review: {task_review_state}; Team review: {team_review_state}. Blocking review feedback exists on `{}`.", string_field(facts, "resolved_change_id")), review_command));
    } else if int_field(facts, "task_review_approvals") <= 0 {
        let team_review_state = if int_field(facts, "team_review_approvals") > 0 {
            format!("{} approval(s)", int_field(facts, "team_review_approvals"))
        } else if command_hint(command_hints, "team_review_command").is_some() {
            "available in team_remote".to_string()
        } else {
            "unavailable outside team_remote".to_string()
        };
        let detail = if task_review_required {
            format!(
                "Code review: {code_review_state}; Task review: pending; Team review: {team_review_state}."
            )
        } else if let Some(reviewer) = auto_review_reviewer {
            format!("Code review: {code_review_state}; Task review: pending; Team review: {team_review_state}. Reviewer-owned `ait workflow finish --apply` or a successful direct `ait review code submit` can record `task_approve` as `{reviewer}` before atomic Task Land.")
        } else {
            format!(
                "Code review: {code_review_state}; Task review: pending; Team review: {team_review_state}. `task_review=automatic` requires `ait config` `user_name` before approval can be recorded."
            )
        };
        steps.push(workflow_land_step(
            "review",
            "Review",
            "pending",
            &detail,
            review_command,
        ));
    } else {
        let task_review_state = if int_field(facts, "task_review_approvals") > 0 {
            "approved"
        } else {
            "not recorded"
        };
        let team_review_state = if int_field(facts, "team_review_approvals") > 0 {
            format!("{} approval(s)", int_field(facts, "team_review_approvals"))
        } else if command_hint(command_hints, "team_review_command").is_some() {
            "available in team_remote".to_string()
        } else {
            "unavailable outside team_remote".to_string()
        };
        steps.push(workflow_land_step("review", "Review", "done", &format!("Code review: {code_review_state}; Task review: {task_review_state}; Team review: {team_review_state}."), None));
    }

    if patchset.is_none() {
        steps.push(workflow_land_step(
            "policy",
            "Policy",
            "waiting",
            "Policy evaluation starts after a patchset exists.",
            None,
        ));
    } else if string_field(facts, "policy_decision") == "pass" {
        steps.push(workflow_land_step(
            "policy",
            "Policy",
            "done",
            &format!("Policy passed for patchset `{}`.", patchset_id),
            None,
        ));
    } else if policy_has_checks {
        let landing_submission_id =
            if string_field(facts, "landing_blocker_class") == "POLICY_BLOCKED" {
                optional_string_field(facts, "landing_submission_id")
            } else {
                None
            };
        let policy_decision = string_field(facts, "policy_decision");
        let status = if string_field(facts, "landing_blocker_class") == "POLICY_BLOCKED"
            || matches!(
                policy_decision.as_str(),
                "hard_fail" | "soft_fail" | "waived"
            ) {
            "blocked"
        } else {
            "pending"
        };
        steps.push(workflow_land_step(
            "policy",
            "Policy",
            status,
            &workflow_land_policy_blocker_detail(
                policy.as_ref(),
                landing_submission_id.as_deref(),
                Some(policy_decision.as_str()),
            ),
            None,
        ));
    } else {
        steps.push(workflow_land_step("policy", "Policy", "advisory", &format!("Policy is folded into `task finish` for Patchset `{patchset_id}`; Land preflight will run the authoritative evaluation."), None));
    }

    let change_status = optional_string_field(&change, "status").unwrap_or_default();
    if change_status == "landed" {
        steps.push(workflow_land_step(
            "land",
            "Land",
            "done",
            &format!(
                "Change `{}` already landed on `{}`.",
                string_field(facts, "resolved_change_id"),
                string_field(facts, "base_line_name")
            ),
            None,
        ));
    } else if patchset.is_none() {
        steps.push(workflow_land_step(
            "land",
            "Land",
            "waiting",
            "Landing starts after a patchset exists and clears review/policy.",
            None,
        ));
    } else if string_field(facts, "landing_status") == "blocked"
        && !bool_field(facts, "stale_policy_blocker_cleared")
    {
        let detail = if string_field(facts, "landing_blocker_class") == "POLICY_BLOCKED" {
            let policy_obj = field_obj(&field_obj_value(facts, "landing_result"), "policy");
            workflow_land_policy_blocker_detail(
                Some(&policy_obj),
                optional_string_field(facts, "landing_submission_id").as_deref(),
                Some(string_field(facts, "policy_decision").as_str()),
            )
        } else if let Some(submission_id) = optional_string_field(facts, "landing_submission_id") {
            let blocker = string_field(facts, "landing_blocker_class");
            if blocker.is_empty() {
                "Remote land is currently blocked and needs manual resolution before retrying."
                    .to_string()
            } else {
                format!("Remote land submission `{submission_id}` is blocked by `{blocker}`.")
            }
        } else {
            "Remote land is currently blocked and needs manual resolution before retrying."
                .to_string()
        };
        steps.push(workflow_land_step("land", "Land", "blocked", &detail, None));
    } else if int_field(facts, "review_blocking") > 0
        || int_field(facts, "review_approvals") <= 0
        || string_field(facts, "policy_decision") != "pass"
    {
        let detail = if policy_has_checks && string_field(facts, "policy_decision") != "pass" {
            workflow_land_policy_blocker_detail(
                policy.as_ref(),
                None,
                Some(string_field(facts, "policy_decision").as_str()),
            )
        } else {
            "Landing waits for review to clear first.".to_string()
        };
        let command = if policy_has_checks && string_field(facts, "policy_decision") != "pass" {
            None
        } else {
            land_command.clone()
        };
        steps.push(workflow_land_step(
            "land",
            "Land",
            if policy_has_checks && string_field(facts, "policy_decision") != "pass" {
                "blocked"
            } else {
                "waiting"
            },
            &detail,
            command,
        ));
    } else if matches!(
        string_field(facts, "landing_status").as_str(),
        "queued" | "running"
    ) {
        let submission_id = optional_string_field(facts, "landing_submission_id")
            .unwrap_or_else(|| "unknown".to_string());
        let detail = format!(
            "Remote land submission `{submission_id}` is currently `{}`.",
            optional_string_field(
                &landing_summary
                    .clone()
                    .map(JsonValue::Object)
                    .unwrap_or(JsonValue::Null),
                "status"
            )
            .unwrap_or_default()
        );
        let command = command_hint(command_hints, "apply_command");
        steps.push(workflow_land_step(
            "land", "Land", "pending", &detail, command,
        ));
    } else {
        steps.push(workflow_land_step("land", "Land", "ready", &format!("Change `{}` is ready to submit onto `{}`. `task finish` will re-evaluate Policy as part of Land preflight.", string_field(facts, "resolved_change_id"), string_field(facts, "base_line_name")), land_command));
    }

    steps
}

pub(crate) fn workflow_land_phase_steps(facts: &JsonValue) -> Vec<JsonValue> {
    if bool_field(facts, "ready_done") {
        let ready_next_action = field_obj(facts, "ready_next_action");
        let ready_step = workflow_land_step(
            "ready",
            "Ready",
            "done",
            &optional_string_field(&ready_next_action, "detail").unwrap_or_else(|| {
                "Patchset and attestation are ready for review and land.".to_string()
            }),
            None,
        );
        let full_steps = field_obj_value(facts, "full_steps");
        let review_step = nested_step_or_default(
            &full_steps,
            "review",
            workflow_land_step(
                "review",
                "Review",
                "waiting",
                "Review begins after a ready patchset exists.",
                None,
            ),
        );
        let policy_step = nested_step_or_default(
            &full_steps,
            "policy",
            workflow_land_step(
                "policy",
                "Policy",
                "advisory",
                "Land preflight will evaluate policy authoritatively.",
                None,
            ),
        );
        let land_step = nested_step_or_default(
            &full_steps,
            "land",
            workflow_land_step(
                "land",
                "Land",
                "waiting",
                "Landing begins after review clears.",
                None,
            ),
        );
        let task_step = if string_field(&field_obj(facts, "change"), "status") == "landed"
            && string_field(&field_obj(facts, "task"), "status") != "completed"
        {
            workflow_land_step(
                "task",
                "Task",
                "pending",
                "The Change is landed; run task finish again to close the Task slice.",
                optional_string_field(facts, "state_next_action_command").or_else(|| {
                    Some(format!(
                        "ait task finish {}",
                        optional_string_field(&field_obj(facts, "change"), "change_id")
                            .unwrap_or_default()
                    ))
                }),
            )
        } else if string_field(&field_obj(facts, "task"), "status") == "completed" {
            workflow_land_step(
                "task",
                "Task",
                "done",
                &format!(
                    "Task `{}` is already completed.",
                    optional_string_field(&field_obj(facts, "task"), "task_id")
                        .unwrap_or_else(|| "unknown".to_string())
                ),
                None,
            )
        } else {
            workflow_land_step(
                "task",
                "Task",
                "waiting",
                "Task completion follows a successful land.",
                None,
            )
        };
        vec![ready_step, review_step, policy_step, land_step, task_step]
    } else {
        vec![
            workflow_land_step(
                "ready",
                "Ready",
                "pending",
                &optional_string_field(&field_obj(facts, "ready_next_action"), "detail")
                    .unwrap_or_else(|| "Run workflow ready before review or land.".to_string()),
                optional_string_field(facts, "ready_command"),
            ),
            workflow_land_step("review", "Review", "waiting", "Review starts after `workflow ready` prepares a selected patchset and attestation.", None),
            workflow_land_step("policy", "Policy", "waiting", "Land preflight will evaluate policy after review clears on a ready patchset.", None),
            workflow_land_step("land", "Land", "waiting", "Landing starts after `workflow ready` and review are complete.", None),
            workflow_land_step("task", "Task", "waiting", "Task completion follows a successful land.", None),
        ]
    }
}

pub(crate) fn workflow_land_suggested_commands(
    facts: &JsonValue,
    commands: &JsonValue,
    next_action: &JsonValue,
    apply_owned_continuation: bool,
) -> JsonValue {
    let next_action_code = optional_string_field(next_action, "code").unwrap_or_default();
    let patchset = optional_obj_field(facts, "patchset");
    let requires_code_review_summary = bool_field(facts, "requires_code_review_summary");
    let change = field_obj(facts, "change");
    let mut candidates: Vec<Option<String>> = Vec::new();
    if next_action_code == "land_blocked" {
        candidates.extend([
            command_hint(commands, "ready_command"),
            if patchset.is_some() {
                command_hint(commands, "attestation_command")
            } else {
                None
            },
            command_hint(commands, "code_review_template_command"),
            if patchset.is_some() && requires_code_review_summary {
                command_hint(commands, "code_review_summary_command")
            } else {
                None
            },
            if patchset.is_some() {
                command_hint(commands, "review_command")
            } else {
                None
            },
        ]);
    } else if next_action_code == "workflow_ready" {
        candidates.extend([
            optional_string_field(next_action, "command"),
            command_hint(commands, "ready_command"),
        ]);
    } else if apply_owned_continuation
        && WORKFLOW_LAND_APPLY_OWNED_CODES.contains(&next_action_code.as_str())
    {
        candidates.extend([
            command_hint(commands, "apply_command"),
            optional_string_field(next_action, "command"),
        ]);
    } else {
        candidates.extend([
            optional_string_field(next_action, "command"),
            if patchset.is_none()
                || !bool_field(facts, "base_is_fresh")
                || matches!(
                    optional_bool_field(facts, "workspace_matches_patchset"),
                    Some(false)
                )
            {
                command_hint(commands, "publish_command")
            } else {
                None
            },
            if patchset.is_some() {
                command_hint(commands, "attestation_command")
            } else {
                None
            },
            command_hint(commands, "code_review_template_command"),
            if patchset.is_some() && requires_code_review_summary {
                command_hint(commands, "code_review_summary_command")
            } else {
                None
            },
            if patchset.is_some() {
                command_hint(commands, "review_command")
            } else {
                None
            },
            if patchset.is_some() {
                command_hint(commands, "land_command")
            } else {
                None
            },
            if string_field(&change, "status") == "landed" {
                command_hint(commands, "task_land_command")
            } else {
                None
            },
        ]);
    }
    JsonValue::Array(unique_command_values(candidates))
}
