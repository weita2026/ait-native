fn run_task(repo: RepoRuntime, command: TaskCommand) -> Result<ExitCode, String> {
    match command {
        TaskCommand::Start(args) => {
            let emit_human_progress = !args.json && std::io::stdout().is_terminal();
            let mut emit_progress = crate::binding_services::emit_task_start_progress;
            let payload = crate::binding_services::start_task(
                &repo,
                &crate::binding_services::TaskStartRequest {
                    title: args.title.clone(),
                    source: args.source.clone(),
                    intent: args.intent.clone(),
                    local: args.local,
                    remote_name: args.remote.clone(),
                    edit_root: args.edit_root.clone(),
                },
                emit_human_progress.then_some(&mut emit_progress),
            )?;
            emit_task_start_result(&payload, args.json, args.full)?;
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::Quick(args) => {
            let emit_human_progress = !args.json && std::io::stdout().is_terminal();
            let mut emit_progress = crate::binding_services::emit_task_start_progress;
            let payload = crate::binding_services::quick_task(
                &repo,
                &crate::binding_services::TaskQuickRequest {
                    title: args.title.clone(),
                    intent: args.intent.clone(),
                    local: args.local,
                    remote_name: args.remote.clone(),
                    edit_root: args.edit_root.clone(),
                },
                emit_human_progress.then_some(&mut emit_progress),
            )?;
            if args.json {
                emit_task_start_result(&payload, true, args.full)?;
            } else {
                println!("{}", crate::binding_output::format_task_quick(&payload)?);
            }
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::List(args) => {
            let payload = task_list(&repo, args.local, args.remote.as_deref())?;
            if args.json {
                print_json(&crate::binding_output::project_task_list(
                    &payload, args.all,
                ))?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_task_list(
                        &payload,
                        args.all,
                        &crate::binding_services::PlanScope {
                            local: args.local,
                            remote_name: args.remote
                        }
                    )?
                );
            }
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::Show(args) => {
            let payload = task_show(&repo, &args.task_id, args.local, args.remote.as_deref())?;
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
            let payload = task_audit(&repo, &args.task_id, args.local, args.remote.as_deref())?;
            emit_task_audit_result(&args.task_id, &payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        TaskCommand::Finish(args) => {
            let payload = crate::binding_services::finish_task(
                &repo,
                &crate::binding_services::TaskFinishRequest {
                    task_id: args.task_or_change_id.clone(),
                    local: args.local,
                    remote_name: args.remote.clone(),
                    message: args.message.clone(),
                    sync_plan_markdown: args.sync_plan_markdown,
                },
            )?;
            emit_task_finish_result(&payload, &args.task_or_change_id, args.json, args.full)?;
            Ok(ExitCode::from(task_finish_exit_code(&payload)))
        }
        TaskCommand::Abandon(args) => {
            let result = crate::binding_services::abandon_task(
                &repo,
                &args.task_id,
                &crate::binding_services::PlanScope {
                    local: args.local,
                    remote_name: args.remote,
                },
            )?;
            if args.json {
                print_json(&result.payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_task_abandon(&result.payload, &args.task_id)?
                );
            }
            Ok(ExitCode::from(result.exit_code))
        }
    }
}

pub(crate) fn task_abandon_service(
    repo: &RepoRuntime,
    task_id: &str,
    scope: &crate::binding_services::PlanScope,
) -> Result<crate::binding_services::WorkflowOperationResult, String> {
    if task_id.starts_with('@') {
        if scope.local
            || scope
                .remote_name
                .as_deref()
                .is_none_or(|name| name.trim().is_empty())
        {
            return Err(
                "Package abandonment requires explicit --remote <name> and rejects --local".into(),
            );
        }
        let identity = read_package_review_identity(task_id)?
            .ok_or("Package abandonment requires an exact identity file")?;
        let payload = crate::primitives::workflow_package_abandon(
            repo,
            scope.remote_name.as_deref().expect("explicit remote"),
            &identity,
        )?;

        let exit_code = u8::from(payload["complete"].as_bool() != Some(true));
        return Ok(crate::binding_services::WorkflowOperationResult { payload, exit_code });
    }
    let use_local_scope = repo.task_uses_local_scope(scope.local, scope.remote_name.as_deref())?;
    let automatic_scope = if use_local_scope {
        AutomaticReconciliationScope::Local
    } else {
        AutomaticReconciliationScope::Remote(scope.remote_name.clone())
    };
    let operation_repo = if use_local_scope {
        RepoRuntime::discover_from_path(&repo.authoritative_repo_root())?
    } else {
        repo.clone()
    };
    let operation = || {
        let mut payload = task_abandon(repo, task_id, scope.local, scope.remote_name.as_deref())?;
        let reconciliation = if use_local_scope && payload["already_abandoned"] == true {
            json!({"status":"skipped","reason":"already_abandoned_cleanup_only","mutated":false})
        } else {
            workflow_reconcile_automatic_best_effort(
                &operation_repo,
                automatic_scope,
                Some(task_id),
                AutomaticReconciliationTrigger::TaskTerminal,
                None,
            )
        };
        attach_automatic_reconciliation(&mut payload, reconciliation);
        Ok(payload)
    };
    let payload = if use_local_scope {
        let _writer = crate::workspace_lock::WorkspaceCommandLock::try_acquire(
            &operation_repo,
            "ait-cli task abandon",
        )?;
        operation()?
    } else {
        run_locked_workspace_command(repo, "ait-cli task abandon", operation)?
    };

    Ok(crate::binding_services::WorkflowOperationResult {
        payload,
        exit_code: 0,
    })
}
