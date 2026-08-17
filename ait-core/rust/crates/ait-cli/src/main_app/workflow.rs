fn reconciliation_scope_label(scope: &AutomaticReconciliationScope) -> &'static str {
    match scope {
        AutomaticReconciliationScope::Local => "local",
        AutomaticReconciliationScope::Remote(_) => "remote",
    }
}

fn run_automatic_reconciliation_locked(
    repo: &RepoRuntime,
    scope: AutomaticReconciliationScope,
    task_filter: Option<&str>,
    trigger: AutomaticReconciliationTrigger,
) -> JsonValue {
    let scope_label = reconciliation_scope_label(&scope);
    let command = format!("ait-cli automatic reconciliation {}", trigger.label());
    let root_repo = match RepoRuntime::discover_from_path(&repo.authoritative_repo_root()) {
        Ok(root_repo) => root_repo,
        Err(error) => {
            return json!({
                "contract": "workflow-automatic-reconciliation/v1",
                "automatic": true,
                "status": "failed_non_blocking",
                "trigger": trigger.label(),
                "scope": scope_label,
                "task_filter": task_filter,
                "safe_only": true,
                "mutated": false,
                "error": format!("Failed to bind automatic reconciliation to the authoritative repository root: {error}"),
                "next_command": "ait workflow reconcile --dry-run",
            });
        }
    };
    match run_locked_workspace_command(&root_repo, &command, || {
        Ok(workflow_reconcile_automatic_best_effort(
            &root_repo,
            scope,
            task_filter,
            trigger,
            None,
        ))
    }) {
        Ok(payload) => payload,
        Err(error) => json!({
            "contract": "workflow-automatic-reconciliation/v1",
            "automatic": true,
            "status": "failed_non_blocking",
            "trigger": trigger.label(),
            "scope": scope_label,
            "task_filter": task_filter,
            "safe_only": true,
            "mutated": false,
            "error": error,
            "next_command": "ait workflow reconcile --dry-run",
        }),
    }
}

fn attach_automatic_reconciliation(payload: &mut JsonValue, reconciliation: JsonValue) {
    if let Some(object) = payload.as_object_mut() {
        object.insert("automatic_reconciliation".to_string(), reconciliation);
    }
}

fn workflow_payload_task_id(payload: &JsonValue) -> Option<String> {
    payload
        .get("task_id")
        .and_then(JsonValue::as_str)
        .or_else(|| {
            payload
                .get("task")
                .and_then(JsonValue::as_object)
                .and_then(|task| task.get("task_id"))
                .and_then(JsonValue::as_str)
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn task_land_automatic_trigger(payload: &JsonValue) -> AutomaticReconciliationTrigger {
    let recovery = payload
        .get("execution_status")
        .and_then(JsonValue::as_str)
        .is_some_and(|status| status == "already_landed")
        || payload
            .get("status")
            .and_then(JsonValue::as_str)
            .is_some_and(|status| status.contains("recovery"))
        || payload.get("closeout_recovery").is_some();
    if recovery {
        AutomaticReconciliationTrigger::LandRecovery
    } else {
        AutomaticReconciliationTrigger::LandTerminal
    }
}

fn run_workflow(repo: RepoRuntime, command: WorkflowCommand) -> Result<ExitCode, String> {
    match command {
        WorkflowCommand::Guide(args) => {
            let payload = workflow_guide_payload(args.topic.as_deref())?;
            println!("{}", render_workflow_guide_text(&payload)?);
            Ok(ExitCode::SUCCESS)
        }
        WorkflowCommand::Reconcile(args) => {
            let payload = if args.scheduled {
                run_locked_workspace_command(
                    &repo,
                    "ait-cli workflow reconcile --scheduled",
                    || {
                        workflow_reconcile_automatic(
                            &repo,
                            AutomaticReconciliationScope::Remote(args.remote.clone()),
                            args.task.as_deref(),
                            AutomaticReconciliationTrigger::ScheduledRemote,
                            Some(args.limit),
                        )
                    },
                )?
            } else if args.apply {
                run_locked_workspace_command(&repo, "ait-cli workflow reconcile --apply", || {
                    workflow_reconcile_apply(
                        &repo,
                        args.remote.as_deref(),
                        args.task.as_deref(),
                        args.safe_only,
                        Some(args.limit),
                    )
                })?
            } else {
                workflow_reconcile_inventory(
                    &repo,
                    args.remote.as_deref(),
                    args.task.as_deref(),
                    args.safe_only,
                    Some(args.limit),
                )?
            };
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", render_workflow_reconcile_text(&payload)?);
            }
            Ok(ExitCode::SUCCESS)
        }
        WorkflowCommand::Ready(args) => {
            let payload = if args.apply {
                workflow_ready_apply(
                    &repo,
                    &args.change_id,
                    args.snapshot_message.as_deref(),
                    args.summary.as_deref(),
                    args.tests.as_deref(),
                    args.lint.as_deref(),
                    args.security.as_deref(),
                    args.license.as_deref(),
                    args.author_mode.map(ConfigAuthorModeArg::as_str),
                    args.model.as_deref(),
                    args.remote.as_deref(),
                    None::<fn(&JsonValue) -> Result<(), String>>,
                )?
            } else {
                workflow_ready_payload(&repo, &args.change_id, args.remote.as_deref())?
            };
            println!("{}", render_workflow_phase_text(&payload, "ready")?);
            Ok(ExitCode::SUCCESS)
        }
        WorkflowCommand::Land(args) => {
            let mut payload = if args.apply {
                workflow_land_apply(
                    &repo,
                    &args.change_id,
                    args.review_message.as_deref(),
                    args.remote.as_deref(),
                    None::<fn(&JsonValue) -> Result<(), String>>,
                )?
            } else {
                workflow_land_payload(&repo, &args.change_id, args.remote.as_deref())?
            };
            if args.apply {
                let task_id = workflow_payload_task_id(&payload);
                let reconciliation = run_automatic_reconciliation_locked(
                    &repo,
                    AutomaticReconciliationScope::Remote(args.remote.clone()),
                    task_id.as_deref(),
                    task_land_automatic_trigger(&payload),
                );
                attach_automatic_reconciliation(&mut payload, reconciliation);
            }
            println!("{}", render_workflow_phase_text(&payload, "land")?);
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn render_workflow_reconcile_text(payload: &JsonValue) -> Result<String, String> {
    let object = payload
        .as_object()
        .ok_or_else(|| "workflow reconcile payload must decode to an object.".to_string())?;
    let summary = object
        .get("summary")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| "workflow reconcile payload is missing summary.".to_string())?;
    let mut lines = vec!["ait workflow reconcile".to_string(), String::new()];
    lines.push(format!(
        "- status: {}",
        string_field(object.get("status"))
    ));
    lines.push(format!(
        "- mode: {}",
        string_field(object.get("mode"))
    ));
    lines.push(format!(
        "- repository: {}",
        string_field(object.get("repo_name"))
    ));
    let remote = string_field(object.get("remote_name"));
    lines.push(format!(
        "- remote: {}",
        if remote.is_empty() { "(local only)" } else { &remote }
    ));
    lines.push(format!(
        "- findings: {} returned, {} remaining",
        string_field(summary.get("returned_finding_count")),
        string_field(summary.get("remaining_count"))
    ));
    lines.push(String::new());
    lines.push("Findings".to_string());
    let findings = object
        .get("findings")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if findings.is_empty() {
        lines.push("- none".to_string());
    } else {
        for finding in findings.iter().take(25) {
            let identities = finding
                .get("identities")
                .and_then(JsonValue::as_object)
                .map(|identities| {
                    identities
                        .values()
                        .filter_map(JsonValue::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            lines.push(format!(
                "- [{}] {}{}",
                string_field(finding.get("disposition")),
                string_field(finding.get("code")),
                if identities.is_empty() {
                    String::new()
                } else {
                    format!(" · {identities}")
                }
            ));
        }
        if findings.len() > 25 {
            lines.push(format!("- … {} more in JSON output", findings.len() - 25));
        }
    }
    lines.push(String::new());
    lines.push("Next action".to_string());
    lines.push(format!(
        "- {}",
        string_field(object.get("next_command"))
    ));
    Ok(lines.join("\n"))
}

fn workflow_nested_value<'a>(
    payload: &'a JsonValue,
    section: &str,
    key: &str,
) -> Option<&'a JsonValue> {
    payload.as_object()?.get(section)?.as_object()?.get(key)
}

fn workflow_default_text<F>(value: String, fallback: F) -> String
where
    F: FnOnce() -> String,
{
    if value.is_empty() {
        fallback()
    } else {
        value
    }
}

fn render_workflow_phase_text(payload: &JsonValue, phase: &str) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| format!("workflow {phase} payload must decode to an object."))?;
    let change_id = workflow_default_text(string_field(obj.get("change_id")), || {
        string_field(workflow_nested_value(payload, "change", "change_id"))
    });
    let task_id = workflow_default_text(string_field(obj.get("task_id")), || {
        string_field(workflow_nested_value(payload, "task", "task_id"))
    });
    let base_line = string_field(workflow_nested_value(payload, "change", "base_line"));
    let current_line = string_field(workflow_nested_value(payload, "workspace", "current_line"));
    let workspace_status = string_field(workflow_nested_value(
        payload,
        "workspace",
        "workspace_status",
    ));
    let workspace_clean = workflow_nested_value(payload, "workspace", "clean")
        .and_then(JsonValue::as_bool);
    let workspace_status = workflow_default_text(workspace_status, || match workspace_clean {
        Some(true) => "clean".to_string(),
        Some(false) => "dirty".to_string(),
        None => "unknown".to_string(),
    });
    let changed_count = workflow_default_text(
        string_field(workflow_nested_value(payload, "workspace", "changed_count")),
        || "0".to_string(),
    );
    let patchset_id = string_field(workflow_nested_value(payload, "patchset", "patchset_id"));
    let next_action_code = string_field(workflow_nested_value(payload, "next_action", "code"));
    let next_action_summary =
        string_field(workflow_nested_value(payload, "next_action", "summary"));
    let next_action_detail = string_field(workflow_nested_value(payload, "next_action", "detail"));
    let next_action_command =
        string_field(workflow_nested_value(payload, "next_action", "command"));
    let apply_status = string_field(obj.get("apply_status"));
    let stopped_reason = string_field(obj.get("apply_stopped_reason"));

    let mut lines = vec![format!(
        "ait workflow {phase} · {}",
        if change_id.is_empty() {
            "(unknown change)".to_string()
        } else {
            change_id
        }
    )];
    lines.push(String::new());
    lines.push(format!(
        "- status: {}",
        workflow_default_text(
            string_field(workflow_nested_value(payload, "change", "status")),
            || "unknown".to_string(),
        )
    ));
    lines.push(format!(
        "- task: {}",
        if task_id.is_empty() {
            "unknown".to_string()
        } else {
            task_id
        }
    ));
    lines.push(format!(
        "- base line: {}",
        if base_line.is_empty() {
            "unknown".to_string()
        } else {
            base_line
        }
    ));
    lines.push(format!(
        "- current line: {}",
        if current_line.is_empty() {
            "unknown".to_string()
        } else {
            current_line
        }
    ));
    lines.push(format!(
        "- workspace: {} ({} changed)",
        workspace_status,
        changed_count
    ));
    lines.push(format!(
        "- patchset: {}",
        if patchset_id.is_empty() {
            "none".to_string()
        } else {
            patchset_id
        }
    ));
    if !next_action_code.is_empty()
        || !next_action_summary.is_empty()
        || !next_action_detail.is_empty()
    {
        lines.push(String::new());
        lines.push("Next action".to_string());
        if !next_action_summary.is_empty() {
            lines.push(format!("- {}", next_action_summary));
            if !next_action_detail.is_empty() {
                lines.push(format!("  {}", next_action_detail));
            }
        } else if !next_action_detail.is_empty() {
            lines.push(format!("- {}", next_action_detail));
        } else {
            lines.push(format!("- {}", next_action_code));
        }
        if !next_action_command.is_empty() {
            lines.push(format!("  {}", next_action_command));
        }
    }
    if !apply_status.is_empty() {
        lines.push(String::new());
        lines.push("Apply status".to_string());
        lines.push(format!("- {}", apply_status));
        if !stopped_reason.is_empty() {
            lines.push(format!("  {}", stopped_reason));
        }
    }
    if let Some(cleanup) = obj
        .get("bound_worktree_cleanup")
        .and_then(JsonValue::as_object)
    {
        let cleanup_status = workflow_default_text(string_field(cleanup.get("status")), || {
            "unknown".to_string()
        });
        let cleanup_reason = string_field(cleanup.get("reason"));
        let cleanup_worktree = workflow_default_text(
            string_field(
                cleanup
                    .get("worktree")
                    .and_then(JsonValue::as_object)
                    .and_then(|worktree| worktree.get("name"))
                    .or_else(|| cleanup.get("worktree_name")),
            ),
            || "unknown".to_string(),
        );
        lines.push(String::new());
        lines.push("Worktree cleanup".to_string());
        lines.push(format!("- {cleanup_status}: {cleanup_worktree}"));
        if !cleanup_reason.is_empty() {
            lines.push(format!("  reason: {cleanup_reason}"));
        }
    }
    if let Some(closeout) = obj
        .get("bound_line_closeout")
        .and_then(JsonValue::as_object)
    {
        let status = workflow_default_text(string_field(closeout.get("status")), || {
            "unknown".to_string()
        });
        let line_name = workflow_default_text(string_field(closeout.get("line_name")), || {
            "unbound".to_string()
        });
        let reason = string_field(closeout.get("reason"));
        let error = string_field(closeout.get("error"));
        lines.push(String::new());
        lines.push("Feature Line closeout".to_string());
        lines.push(format!("- {status}: {line_name}"));
        if !reason.is_empty() {
            lines.push(format!("  reason: {reason}"));
        }
        if !error.is_empty() {
            lines.push(format!("  error: {error}"));
        }
    }
    if let Some(closeout) = obj
        .get("plan_checklist_closeout")
        .and_then(JsonValue::as_object)
    {
        let status = workflow_default_text(string_field(closeout.get("status")), || {
            "unknown".to_string()
        });
        let reason = string_field(closeout.get("reason"));
        let error = string_field(closeout.get("error"));
        let detail = string_field(closeout.get("detail"));
        let command = string_field(closeout.get("command"));
        lines.push(String::new());
        lines.push("Sprint checklist closeout".to_string());
        lines.push(format!("- {status}"));
        if !reason.is_empty() {
            lines.push(format!("  reason: {reason}"));
        }
        if !error.is_empty() {
            lines.push(format!("  error: {error}"));
        }
        if !detail.is_empty() {
            lines.push(format!("  {detail}"));
        }
        if !command.is_empty() {
            lines.push(format!("  {command}"));
        }
        if let Some(retention) = closeout.get("retention").and_then(JsonValue::as_object) {
            let retention_status = workflow_default_text(
                string_field(retention.get("status")),
                || "unknown".to_string(),
            );
            let removed = retention
                .get("removed_count")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0);
            let retained = retention
                .get("retained_completed_count")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0);
            lines.push(format!(
                "  retention: {retention_status} ({removed} removed, {retained} completed retained)"
            ));
        }
    }
    if let Some(reconciliation) = obj
        .get("automatic_reconciliation")
        .and_then(JsonValue::as_object)
    {
        let trigger = reconciliation
            .get("automatic_trigger")
            .and_then(JsonValue::as_object)
            .and_then(|trigger| trigger.get("trigger"))
            .or_else(|| reconciliation.get("trigger"));
        let remaining_safe = reconciliation
            .get("apply_summary")
            .and_then(JsonValue::as_object)
            .and_then(|summary| summary.get("remaining_safe_count"));
        let status = string_field_or_default(reconciliation.get("status"), "unknown");
        let mutated = reconciliation
            .get("mutated")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let error = string_field(reconciliation.get("error"));
        let remaining_safe_count = remaining_safe.and_then(JsonValue::as_i64).unwrap_or(0);
        if mutated
            || !error.is_empty()
            || remaining_safe_count > 0
            || !matches!(status.as_str(), "complete" | "completed" | "success" | "ok")
        {
            lines.push(String::new());
            lines.push("Automatic reconciliation".to_string());
            lines.push(format!(
                "- {}: {status} (mutated {mutated}, {remaining_safe_count} safe remaining)",
                string_field_or_default(trigger, "terminal"),
            ));
            if !error.is_empty() {
                lines.push(format!("  non-blocking error: {error}"));
            }
        }
    }
    Ok(lines.join("\n"))
}

fn render_task_land_text(payload: &JsonValue) -> Result<String, String> {
    let rendered = if payload.get("mode").and_then(JsonValue::as_str) == Some("local")
        && payload.get("apply_status").and_then(JsonValue::as_str) == Some("done")
    {
        render_local_task_land_text(payload)?
    } else {
        let old_title = format!("ait workflow {}", "land");
        render_workflow_phase_text(payload, "land")?.replacen(&old_title, "ait task land", 1)
    };
    Ok(append_task_land_contract_text(rendered, payload))
}

fn append_task_land_contract_text(mut rendered: String, payload: &JsonValue) -> String {
    let Some(contract) = payload
        .get("task_land_contract")
        .and_then(JsonValue::as_object)
    else {
        return rendered;
    };
    let version = workflow_default_text(string_field(contract.get("version")), || {
        "unknown".to_string()
    });
    let closeout_status = workflow_default_text(string_field(payload.get("closeout_status")), || {
        "unknown".to_string()
    });
    if matches!(
        closeout_status.as_str(),
        "complete" | "complete_unbound" | "already_complete"
    ) {
        return rendered;
    }
    rendered.push_str(&format!(
        "\n\ntask-land contract: {version}\ncloseout: {closeout_status}"
    ));
    if let Some(recovery) = payload
        .get("closeout_recovery")
        .and_then(JsonValue::as_object)
    {
        let detail = string_field(recovery.get("detail"));
        let command = string_field(recovery.get("command"));
        if !detail.is_empty() {
            rendered.push_str(&format!("\nrecovery: {detail}"));
        }
        if !command.is_empty() {
            rendered.push_str(&format!("\n{command}"));
        }
    }
    rendered
}

fn render_local_task_land_text(payload: &JsonValue) -> Result<String, String> {
    let obj = payload
        .as_object()
        .ok_or_else(|| "local Task Land payload must decode to an object.".to_string())?;
    let cleanup = obj
        .get("bound_worktree_cleanup")
        .and_then(JsonValue::as_object);
    let change_id = workflow_default_text(string_field(obj.get("change_ref")), || {
        workflow_default_text(string_field(obj.get("change_id")), || {
            "(unknown change)".to_string()
        })
    });
    let target_line = workflow_default_text(string_field(obj.get("target_line")), || {
        "unknown".to_string()
    });
    let snapshot_id = workflow_default_text(string_field(obj.get("landed_snapshot_id")), || {
        "unknown".to_string()
    });
    let closeout_status = string_field(obj.get("closeout_status"));
    let closeout_incomplete = !closeout_status.is_empty()
        && !matches!(
            closeout_status.as_str(),
            "complete" | "complete_unbound" | "already_complete"
        );
    let mut lines = vec![format!(
        "landed: {change_id} -> {target_line} @ {snapshot_id}"
    )];
    let mut closed = Vec::new();
    let task_status = string_field(obj.get("task_status"));
    if task_status == "completed" {
        closed.push("task");
    } else if !task_status.is_empty() {
        lines.push(format!("task: {task_status}"));
    }
    if let Some(cleanup) = cleanup {
        let status = string_field(cleanup.get("status"));
        if matches!(status.as_str(), "removed" | "already_removed") {
            closed.push("worktree");
        } else if closeout_incomplete && !status.is_empty() {
            lines.push(format!("worktree: {status}"));
        }
    }
    if let Some(closeout) = obj
        .get("bound_line_closeout")
        .and_then(JsonValue::as_object)
    {
        let status = workflow_default_text(string_field(closeout.get("status")), || {
            "unknown".to_string()
        });
        let line_name = workflow_default_text(string_field(closeout.get("line_name")), || {
            "unbound".to_string()
        });
        if matches!(status.as_str(), "archived" | "already_archived") {
            closed.push("line");
        } else if closeout_incomplete {
            lines.push(format!("line: {status} ({line_name})"));
        }
        let error = string_field(closeout.get("error"));
        if !error.is_empty() {
            lines.push(format!("line error: {error}"));
        }
    }
    if let Some(closeout) = obj
        .get("plan_checklist_closeout")
        .and_then(JsonValue::as_object)
    {
        let status = workflow_default_text(string_field(closeout.get("status")), || {
            "unknown".to_string()
        });
        let reason = string_field(closeout.get("reason"));
        let error = string_field(closeout.get("error"));
        if matches!(status.as_str(), "synced" | "already_synced") {
            closed.push("sprint");
        } else if closeout_incomplete {
            lines.push(format!("sprint: {status}"));
            if !reason.is_empty() {
                lines.push(format!("sprint reason: {reason}"));
            }
        }
        if !error.is_empty() {
            lines.push(format!("sprint error: {error}"));
        }
        if let Some(retention) = closeout.get("retention").and_then(JsonValue::as_object) {
            let retention_status = workflow_default_text(
                string_field(retention.get("status")),
                || "unknown".to_string(),
            );
            let removed = retention
                .get("removed_count")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0);
            let retained = retention
                .get("retained_completed_count")
                .and_then(JsonValue::as_u64)
                .unwrap_or(0);
            if removed > 0 {
                lines.push(format!(
                    "retention: {retention_status} ({removed} removed, {retained} retained)"
                ));
            }
        }
    }
    if !closed.is_empty() {
        lines.insert(1, format!("closed: {}", closed.join(", ")));
    }
    if closeout_incomplete {
        lines.push(format!("closeout: {closeout_status}"));
    }
    if let Some(reconciliation) = obj
        .get("automatic_reconciliation")
        .and_then(JsonValue::as_object)
    {
        let status = string_field_or_default(reconciliation.get("status"), "unknown");
        let mutated = reconciliation
            .get("mutated")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let error = string_field(reconciliation.get("error"));
        if mutated {
            lines.push(format!("reconciled: {status}"));
        }
        if !error.is_empty() {
            lines.push(format!("reconciliation error: {error}"));
        } else if !matches!(status.as_str(), "complete" | "completed" | "success" | "ok")
            && status != "unknown"
        {
            lines.push(format!("reconciliation: {status}"));
        }
    }
    Ok(lines.join("\n"))
}

fn workflow_guide_payload(topic: Option<&str>) -> Result<JsonValue, String> {
    let local_land_contract = task_land_scope_contract_json(true);
    let remote_land_contract = task_land_scope_contract_json(false);
    let local_plan_closeout_policy = string_field(
        local_land_contract.get("plan_closeout_policy"),
    );
    let remote_plan_closeout_policy = string_field(
        remote_land_contract.get("plan_closeout_policy"),
    );
    let inventory = json!({
        "topic": "inventory",
        "summary": "Use one inventory surface first, then drill down only where the workflow actually points.",
        "when_to_use": [
            "You need to answer what remains or what should land next.",
            "You are about to rerun queue, task list, or change list in the same turn."
        ],
        "commands": [
            {
                "label": "Shared queue",
                "command": "ait queue summary",
                "detail": "Use this first for the current actionable Task, review, local draft, workspace, and worktree picture."
            },
            {
                "label": "Task history",
                "command": "ait task list --all",
                "detail": "Use the Task inventory instead of widening the actionable queue to terminal history."
            },
            {
                "label": "Change history",
                "command": "ait change list --all",
                "detail": "Use the Change inventory instead of adding every non-landed Change to the queue."
            },
            {
                "label": "One task readiness",
                "command": "ait task audit <task-id>",
                "detail": "Prefer this over rebuilding one task from `task show` plus task-scoped `change list`."
            },
            {
                "label": "One change detail",
                "command": "ait change show <change-id>",
                "detail": "Open the focus change only after the queue or task audit points you there."
            }
        ],
        "avoid": [
            "Do not rerun the same queue or list command in the same turn unless workflow state changed."
        ]
    });
    let land = json!({
        "topic": "land",
        "contract_version": TASK_LAND_CONTRACT_VERSION,
        "scope_contracts": {
            "local": local_land_contract.clone(),
            "remote": remote_land_contract.clone(),
        },
        "summary": "Use `workflow ready` then `workflow land` instead of rediscovering low-level remote gates by hand.",
        "when_to_use": [
            "You want to see what still blocks one remote change from landing.",
            "You want the helper to advance safe remote-land steps without teaching the low-level gate commands first."
        ],
        "commands": [
            {
                "label": "Workflow ready apply",
                "command": "ait workflow ready <change-id> --apply",
                "detail": "Create any needed snapshot or patchset updates, run patchset CI, and stop once attestation-backed ready state exists."
            },
            {
                "label": "Workflow land apply",
                "command": "ait workflow land <change-id> --apply",
                "detail": "Run the reviewer-owned exact-Patchset code-review and Task-approval gates, evaluate final Policy, then delegate the already-ready final mutation, target-Line sync, Task completion, and cleanup to atomic Task Land. Add --review-message with the structured review when code-review evidence is required."
            },
            {
                "label": "Task land direct",
                "command": "ait task land <task-or-change-id>",
                "detail": format!("Direct already-ready finalizer and recovery entry. It creates no Review evidence. Contract {TASK_LAND_CONTRACT_VERSION}: solo_local uses `{local_plan_closeout_policy}` Plan closeout; solo_remote uses `{remote_plan_closeout_policy}`. A partial post-land closeout is resumed by rerunning the reported idempotent task-land command.")
            },
        ],
        "avoid": [
            "Do not rediscover the same land path with many separate low-level gate or help commands in one turn.",
            "Do not use the removed top-level `ait land`; `ait workflow land` routes shared landing and task closeout through `ait task land`."
        ]
    });
    match topic.map(|value| value.trim().to_ascii_lowercase()) {
        None => Ok(json!({
            "topics": [
                {
                    "topic": "inventory",
                    "summary": inventory["summary"],
                    "command": "ait workflow guide inventory"
                },
                {
                    "topic": "land",
                    "summary": land["summary"],
                    "command": "ait workflow guide land"
                }
            ]
        })),
        Some(value) if value == "inventory" => Ok(inventory),
        Some(value) if value == "land" => Ok(land),
        Some(value) => Err(format!(
            "Unknown workflow guide topic: {value}. Available topics: inventory, land"
        )),
    }
}

fn render_workflow_guide_text(data: &JsonValue) -> Result<String, String> {
    let topic = data
        .get("topic")
        .and_then(JsonValue::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if topic.is_empty() {
        let mut lines = vec!["ait workflow guides".to_string(), String::new()];
        if let Some(rows) = data.get("topics").and_then(JsonValue::as_array) {
            for row in rows {
                let topic = row
                    .get("topic")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .trim();
                let summary = row
                    .get("summary")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .trim();
                let command = row
                    .get("command")
                    .and_then(JsonValue::as_str)
                    .unwrap_or("")
                    .trim();
                if !topic.is_empty() {
                    lines.push(format!("- {topic}: {summary} ({command})"));
                }
            }
        }
        return Ok(lines.join("\n"));
    }

    let mut lines = vec![
        format!("ait workflow guide · {topic}"),
        String::new(),
        data.get("summary")
            .and_then(JsonValue::as_str)
            .unwrap_or("")
            .trim()
            .to_string(),
    ];

    let when_to_use = data
        .get("when_to_use")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if !when_to_use.is_empty() {
        lines.push(String::new());
        lines.push("When to use".to_string());
        for item in when_to_use {
            if let Some(text) = item.as_str() {
                let text = text.trim();
                if !text.is_empty() {
                    lines.push(format!("- {text}"));
                }
            }
        }
    }

    let commands = data
        .get("commands")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if !commands.is_empty() {
        lines.push(String::new());
        lines.push("Recommended commands".to_string());
        for row in commands {
            let label = row
                .get("label")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let command = row
                .get("command")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            let detail = row
                .get("detail")
                .and_then(JsonValue::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if !label.is_empty() && !command.is_empty() {
                lines.push(format!("- {label}: {command}"));
            } else if !command.is_empty() {
                lines.push(format!("- {command}"));
            }
            if !detail.is_empty() {
                lines.push(format!("  {detail}"));
            }
        }
    }

    let avoid = data
        .get("avoid")
        .and_then(JsonValue::as_array)
        .cloned()
        .unwrap_or_default();
    if !avoid.is_empty() {
        lines.push(String::new());
        lines.push("Avoid".to_string());
        for item in avoid {
            if let Some(text) = item.as_str() {
                let text = text.trim();
                if !text.is_empty() {
                    lines.push(format!("- {text}"));
                }
            }
        }
    }

    Ok(lines.join("\n"))
}
