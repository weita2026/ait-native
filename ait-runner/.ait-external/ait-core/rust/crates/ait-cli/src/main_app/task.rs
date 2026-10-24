fn run_task(repo: RepoRuntime, command: TaskCommand) -> Result<ExitCode, String> {
    match command {
        TaskCommand::Start(args) => {
            let emit_human_progress = !args.json && std::io::stdout().is_terminal();
            let automatic_scope = if repo.task_uses_local_scope(args.local, args.remote.as_deref())? {
                AutomaticReconciliationScope::Local
            } else {
                AutomaticReconciliationScope::Remote(args.remote.clone())
            };
            let payload = run_locked_workspace_command(&repo, "ait-cli task start", || {
                let reconciliation = workflow_reconcile_automatic_best_effort(
                    &repo,
                    automatic_scope,
                    None,
                    AutomaticReconciliationTrigger::PreTaskStart,
                    None,
                );
                let mut emit_progress = |event: &JsonValue| {
                    if let Some(line) = task_start_progress_line(event) {
                        println!("{line}");
                    }
                    Ok(())
                };
                let mut payload = if let Some(source) = args.source.as_deref() {
                    task_start_from_with_edit_root_and_progress(
                        &repo,
                        source,
                        &args.intent,
                        args.local,
                        args.remote.as_deref(),
                        args.edit_root.as_deref(),
                        None,
                        emit_human_progress.then_some(&mut emit_progress),
                    )?
                } else {
                    let title = args.title.as_deref().ok_or_else(|| {
                        "`--title` is required unless `--from` is provided.".to_string()
                    })?;
                    task_start_with_progress(
                        &repo,
                        title,
                        &args.intent,
                        args.local,
                        args.remote.as_deref(),
                        None,
                        None,
                        None,
                        args.edit_root.as_deref(),
                        None,
                        emit_human_progress.then_some(&mut emit_progress),
                    )?
                };
                attach_automatic_reconciliation(&mut payload, reconciliation);
                attach_plan_backed_markdown_summary(&repo, &mut payload);
                Ok(payload)
            })?;
            emit_task_start_result(&payload, args.json, args.full)?;
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::Quick(args) => {
            let emit_human_progress = !args.json && std::io::stdout().is_terminal();
            let automatic_scope = if repo.task_uses_local_scope(args.local, args.remote.as_deref())? {
                AutomaticReconciliationScope::Local
            } else {
                AutomaticReconciliationScope::Remote(args.remote.clone())
            };
            let payload = run_locked_workspace_command(&repo, "ait-cli task quick", || {
                let reconciliation = workflow_reconcile_automatic_best_effort(
                    &repo,
                    automatic_scope,
                    None,
                    AutomaticReconciliationTrigger::PreTaskStart,
                    None,
                );
                let mut emit_progress = |event: &JsonValue| {
                    if let Some(line) = task_start_progress_line(event) {
                        println!("{line}");
                    }
                    Ok(())
                };
                let mut payload = task_quick_with_edit_root_and_progress(
                    &repo,
                    args.title.as_deref(),
                    &args.intent,
                    args.local,
                    args.remote.as_deref(),
                    args.edit_root.as_deref(),
                    emit_human_progress.then_some(&mut emit_progress),
                )?;
                attach_automatic_reconciliation(&mut payload, reconciliation);
                attach_plan_backed_markdown_summary(&repo, &mut payload);
                Ok(payload)
            })?;
            emit_task_start_result(&payload, args.json, args.full)?;
            if !args.json {
                if let Some(card) = payload
                    .get("quick_card")
                    .and_then(|card| card.get("artifact_path"))
                    .and_then(JsonValue::as_str)
                {
                    println!("card: {card}");
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::List(args) => {
            let payload = task_list(&repo, args.local, args.remote.as_deref())?;
            if let Some(rows) = payload.as_array() {
                let include_publication = rows
                    .iter()
                    .any(|row| row.get("publication_state").is_some());
                let all_command =
                    scoped_all_command("ait task list", args.local, args.remote.as_deref());
                let terminal_statuses = [
                    "completed",
                    "abandoned",
                    "canceled",
                    "later_promotion_excluded",
                ];
                if args.json {
                    let (selected, _, _) =
                        select_agent_list_rows(rows, args.all, &terminal_statuses);
                    print_json(&JsonValue::Array(selected))?;
                    return Ok(ExitCode::SUCCESS);
                }
                if include_publication {
                    print_agent_list(
                        rows,
                        &["task_id", "status", "publication_state", "title"],
                        args.all,
                        &terminal_statuses,
                        Some("open"),
                        &all_command,
                    );
                } else {
                    print_agent_list(
                        rows,
                        &["task_id", "status", "title"],
                        args.all,
                        &terminal_statuses,
                        Some("open"),
                        &all_command,
                    );
                }
            }
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::Show(args) => {
            let payload = task_show(
                &repo,
                &args.task_id,
                args.local,
                args.remote.as_deref(),
            )?;
            emit_result(
                "ait-cli task show",
                &payload,
                args.json,
                &[
                    "task_id",
                    "title",
                    "status",
                    "publication_state",
                    "published_task_id",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::Audit(args) => {
            let payload = task_audit(
                &repo,
                &args.task_id,
                args.local,
                args.remote.as_deref(),
            )?;
            emit_task_audit_result(&args.task_id, &payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::Finish(args) => {
            let (use_local_scope, scoped_remote_name) =
                resolve_task_land_scope(&repo, args.local, args.remote.as_deref())?;
            if !use_local_scope && args.message.is_some() {
                return Err("`--message` is available only for local `ait task finish`; remote finish consumes an already-ready selected Patchset.".to_string());
            }
            if !use_local_scope && args.sync_plan_markdown {
                return Err("`--sync-plan-markdown` is available only for local `ait task finish`; sync Plan-backed Markdown with `ait plan sync <path> --local` before a remote finish.".to_string());
            }
            let presync = if use_local_scope && args.sync_plan_markdown {
                Some(run_locked_workspace_command(
                    &repo,
                    "ait-cli task finish --sync-plan-markdown",
                    || presync_plan_markdown_drift(&repo),
                )?)
            } else {
                None
            };
            let pre_finish = if use_local_scope {
                run_pre_finish_hooks_gate(&repo, &args.task_or_change_id)?
            } else {
                None
            };
            let mut payload = run_task_scoped_workspace_command(
                &repo,
                &args.task_or_change_id,
                use_local_scope,
                !use_local_scope,
                scoped_remote_name.as_deref(),
                "ait-cli task finish",
                |execution_repo| {
                    task_land_apply_scoped(
                        execution_repo,
                        &args.task_or_change_id,
                        use_local_scope,
                        scoped_remote_name.as_deref(),
                        args.message.as_deref(),
                        None::<fn(&JsonValue) -> Result<(), String>>,
                    )
                },
            )?;
            let task_id = workflow_payload_task_id(&payload);
            let scope = if use_local_scope {
                AutomaticReconciliationScope::Local
            } else {
                AutomaticReconciliationScope::Remote(scoped_remote_name.clone())
            };
            let reconciliation = run_automatic_reconciliation_locked(
                &repo,
                scope,
                task_id.as_deref(),
                task_land_automatic_trigger(&payload),
            );
            attach_automatic_reconciliation(&mut payload, reconciliation);
            if let (Some(presync), Some(object)) = (presync, payload.as_object_mut()) {
                object.insert(PLAN_MARKDOWN_PRESYNC_PAYLOAD_KEY.to_string(), presync);
            }
            if let (Some(pre_finish), Some(object)) = (pre_finish, payload.as_object_mut()) {
                object.insert(PRE_FINISH_HOOKS_PAYLOAD_KEY.to_string(), pre_finish);
            }
            if use_local_scope {
                attach_post_finish_hooks(&repo, &mut payload);
            } else {
                attach_runner_hint_best_effort(&repo, scoped_remote_name.as_deref(), &mut payload);
            }
            emit_task_finish_result(&payload, &args.task_or_change_id, args.json, args.full)?;
            Ok(ExitCode::from(task_finish_exit_code(&payload)))
        }
        TaskCommand::Abandon(args) => {
            let use_local_scope = repo.task_uses_local_scope(args.local, args.remote.as_deref())?;
            let automatic_scope = if use_local_scope {
                AutomaticReconciliationScope::Local
            } else {
                AutomaticReconciliationScope::Remote(args.remote.clone())
            };
            let payload = run_locked_workspace_command(&repo, "ait-cli task abandon", || {
                let mut payload = task_abandon(
                    &repo,
                    &args.task_id,
                    args.local,
                    args.remote.as_deref(),
                )?;
                let reconciliation = workflow_reconcile_automatic_best_effort(
                    &repo,
                    automatic_scope,
                    Some(&args.task_id),
                    AutomaticReconciliationTrigger::TaskTerminal,
                    None,
                );
                attach_automatic_reconciliation(&mut payload, reconciliation);
                Ok(payload)
            })?;
            emit_result(
                "ait-cli task abandon",
                &payload,
                args.json,
                &["task_id", "status", "published_task_id"],
            )?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Run `hooks.post_finish` once after a fresh local apply. An already-finished
/// closeout recovery and every non-applied outcome skip the hooks.
fn attach_post_finish_hooks(repo: &RepoRuntime, payload: &mut JsonValue) {
    if payload.get("apply_status").and_then(JsonValue::as_str) != Some("done") {
        return;
    }
    if payload.get("execution_status").and_then(JsonValue::as_str) == Some("already_landed") {
        return;
    }
    let repo_root = repo.authoritative_repo_root();
    let result = match post_finish_hooks_for_repo(repo) {
        Ok(hooks) if hooks.is_empty() => {
            run_post_finish_hooks(&repo_root, &hooks, &PostFinishHookContext::default())
        }
        Ok(hooks) => match acquire_hooks_lock(&repo_root) {
            Ok(_lock) => {
                let mut context = PostFinishHookContext::from_finish_payload(payload);
                context.current_head_snapshot_id =
                    line_show(repo, Some(&repo.default_line_name()))
                        .ok()
                        .and_then(|row| row.get("head_snapshot_id").and_then(JsonValue::as_str).map(ToString::to_string));
                let mut result = run_post_finish_hooks(&repo_root, &hooks, &context);
                if let Some(object) = result.as_object_mut() {
                    object.insert("serialized".to_string(), JsonValue::Bool(true));
                    object.insert(
                        "current_head_snapshot_id".to_string(),
                        context
                            .current_head_snapshot_id
                            .clone()
                            .map(JsonValue::String)
                            .unwrap_or(JsonValue::Null),
                    );
                }
                result
            }
            Err(error) => json!({
                "status": "failed",
                "error": error,
                "repo_root": repo_root.to_string_lossy(),
                "configured_count": hooks.len(),
                "failed_count": 0,
                "hooks": [],
                "detail": "The finish hook lock could not be acquired, so no hook ran. The Task finish itself is applied; rerun the hook commands manually.",
            }),
        },
        Err(error) => json!({
            "status": "failed",
            "error": error,
                "repo_root": repo_root.to_string_lossy(),
            "configured_count": 0,
            "failed_count": 0,
            "hooks": [],
            "detail": "hooks.post_finish in .ait/config.json could not be parsed, so no hook ran. The Task finish itself is applied; fix the configuration with `ait config set --post-finish-hooks <json>`.",
        }),
    };
    if let Some(object) = payload.as_object_mut() {
        object.insert(POST_FINISH_HOOKS_PAYLOAD_KEY.to_string(), result);
    }
}

/// Tell the agent at start time which Markdown in the new worktree travels
/// through Plan lineage, so the finish gate is not the first place it learns.
fn attach_plan_backed_markdown_summary(repo: &RepoRuntime, payload: &mut JsonValue) {
    let Some(edit_root) = payload
        .get("worktree")
        .and_then(|worktree| worktree.get("path"))
        .and_then(JsonValue::as_str)
        .map(PathBuf::from)
    else {
        return;
    };
    let summary = plan_backed_markdown_summary(repo, &edit_root);
    if let Some(object) = payload.as_object_mut() {
        object.insert(PLAN_BACKED_MARKDOWN_PAYLOAD_KEY.to_string(), summary);
    }
}

/// Run tracked `pre_finish` hooks in the Task worktree before any Snapshot is
/// created. A `fail` hook refuses the finish with its output tail; nothing has
/// been applied yet, so the refusal is safe to retry after the fix.
fn run_pre_finish_hooks_gate(
    repo: &RepoRuntime,
    requested: &str,
) -> Result<Option<JsonValue>, String> {
    if !repo.is_worktree() {
        return Ok(None);
    }
    let workspace_root = repo.workspace_root();
    let Some((path, hooks)) =
        pre_finish_hooks_for_roots(&workspace_root, &repo.authoritative_repo_root())?
    else {
        return Ok(None);
    };
    let context = PostFinishHookContext {
        task_id: Some(requested.to_string()),
        ..PostFinishHookContext::default()
    };
    let mut result = run_post_finish_hooks(&workspace_root, &hooks, &context);
    if let Some(object) = result.as_object_mut() {
        object.insert(
            "source".to_string(),
            JsonValue::String(path.to_string_lossy().to_string()),
        );
        object.insert(
            "phase".to_string(),
            JsonValue::String("pre_finish".to_string()),
        );
    }
    if result.get("status").and_then(JsonValue::as_str) == Some("failed") {
        let mut lines = vec![format!(
            "Refusing `ait task finish {requested}`: a pre-finish hook from {} failed. Nothing was applied; fix the reported command and rerun `ait task finish {requested} --local`.",
            path.display()
        )];
        lines.extend(
            pre_finish_hooks_text_lines(&json!({ PRE_FINISH_HOOKS_PAYLOAD_KEY: result }))
                .into_iter()
                .skip(1),
        );
        return Err(lines.join("\n"));
    }
    Ok(Some(result))
}
