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
                    task_start_from_with_progress(
                        &repo,
                        source,
                        &args.intent,
                        args.local,
                        args.remote.as_deref(),
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
                        None,
                        emit_human_progress.then_some(&mut emit_progress),
                    )?
                };
                attach_automatic_reconciliation(&mut payload, reconciliation);
                Ok(payload)
            })?;
            emit_task_start_result(&payload, args.json)?;
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
        TaskCommand::Land(args) => {
            let (use_local_scope, scoped_remote_name) =
                resolve_task_land_scope(&repo, args.local, args.remote.as_deref())?;
            let mut payload = task_land_apply_scoped(
                &repo,
                &args.task_or_change_id,
                use_local_scope,
                scoped_remote_name.as_deref(),
                None::<fn(&JsonValue) -> Result<(), String>>,
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
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", render_task_land_text(&payload)?);
            }
            Ok(ExitCode::from(task_land_exit_code(&payload)))
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
