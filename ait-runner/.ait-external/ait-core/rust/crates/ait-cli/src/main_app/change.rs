fn run_change(repo: RepoRuntime, command: ChangeCommand) -> Result<(), String> {
    match command {
        ChangeCommand::Create(args) => {
            let payload = run_task_scoped_workspace_command(
                &repo,
                &args.task_id,
                false,
                false,
                args.remote.as_deref(),
                "ait-cli change create",
                |execution_repo| {
                change_create_cmd(
                    execution_repo,
                    &args.task_id,
                    &args.title,
                    args.base_line.as_deref(),
                    args.local,
                    args.remote.as_deref(),
                )
                },
            )?;
            emit_result(
                "ait-cli change create",
                &payload,
                args.json,
                &[
                    "task_id",
                    "title",
                    "base_line",
                    "fork_snapshot_id",
                    "status",
                ],
            )?;
            Ok(())
        }
        ChangeCommand::List(args) => {
            let payload = change_list_cmd(&repo, args.local, args.remote.as_deref())?;
            if args.json {
                let payload = agent_list_json_payload(
                    &payload,
                    args.all,
                    &["landed", "archived", "abandoned", "canceled"],
                );
                print_json(&payload)?;
            } else if let Some(rows) = payload.as_array() {
                let include_publication = rows
                    .iter()
                    .any(|row| row.get("publication_state").is_some());
                let all_command =
                    scoped_all_command("ait change list", args.local, args.remote.as_deref());
                if include_publication {
                    print_agent_list(
                        rows,
                        &["task_id", "status", "publication_state", "title"],
                        args.all,
                        &["landed", "archived", "abandoned", "canceled"],
                        Some("open"),
                        &all_command,
                    );
                } else {
                    print_agent_list(
                        rows,
                        &["task_id", "status", "title"],
                        args.all,
                        &["landed", "archived", "abandoned", "canceled"],
                        Some("open"),
                        &all_command,
                    );
                }
            }
            Ok(())
        }
        ChangeCommand::Show(args) => {
            let change_ref = resolve_task_finish_change_input(
                &repo,
                &args.change_id,
                args.local,
                args.remote.as_deref(),
            )?;
            let payload = change_show_cmd(
                &repo,
                &change_ref,
                args.local,
                args.remote.as_deref(),
                None,
            )?;
            emit_result(
                "ait-cli change show",
                &payload,
                args.json,
                &[
                    "task_id",
                    "title",
                    "base_line",
                    "fork_snapshot_id",
                    "status",
                    "publication_state",
                ],
            )?;
            Ok(())
        }
        ChangeCommand::Revert(args) => {
            let change_ref = resolve_task_author_change_input(
                &repo,
                &args.change_id,
                args.local,
                args.remote.as_deref(),
            )?;
            let payload = run_locked_workspace_command(&repo, "ait-cli change revert", || {
                change_revert_cmd(
                    &repo,
                    &change_ref,
                    args.force,
                    args.dry_run,
                    args.local,
                    args.remote.as_deref(),
                    None,
                )
            })?;
            emit_result(
                "ait-cli change revert",
                &payload,
                args.json,
                &[
                    "fork_snapshot_id",
                    "latest_change_snapshot_id",
                    "current_line",
                    "applied",
                    "affected_path_count",
                    "conflict_paths",
                ],
            )?;
            Ok(())
        }
        ChangeCommand::Replay(args) => {
            let change_ref = resolve_task_author_change_input(
                &repo,
                &args.change_id,
                args.local,
                args.remote.as_deref(),
            )?;
            let payload = run_locked_workspace_command(&repo, "ait-cli change replay", || {
                let onto_line = match args.onto.as_deref() {
                    Some(onto_line) => onto_line.to_string(),
                    None => repo.current_line_name()?,
                };
                change_replay_cmd(
                    &repo,
                    &change_ref,
                    &onto_line,
                    args.force,
                    args.dry_run,
                    args.local,
                    args.remote.as_deref(),
                    None,
                )
            })?;
            emit_result(
                "ait-cli change replay",
                &payload,
                args.json,
                &[
                    "fork_snapshot_id",
                    "latest_change_snapshot_id",
                    "onto_line",
                    "applied",
                    "affected_path_count",
                    "conflict_paths",
                ],
            )?;
            Ok(())
        }
        ChangeCommand::Close(args) => {
            let change_ref = resolve_task_finish_change_input(
                &repo,
                &args.change_id,
                args.local,
                args.remote.as_deref(),
            )?;
            let automatic_scope = if repo.change_uses_local_scope(args.local, args.remote.as_deref()) {
                AutomaticReconciliationScope::Local
            } else {
                AutomaticReconciliationScope::Remote(args.remote.clone())
            };
            let payload = run_locked_workspace_command(&repo, "ait-cli change close", || {
                let mut payload =
                    change_close_cmd(&repo, &change_ref, args.local, args.remote.as_deref())?;
                let task_id = workflow_payload_task_id(&payload);
                let reconciliation = workflow_reconcile_automatic_best_effort(
                    &repo,
                    automatic_scope,
                    task_id.as_deref(),
                    AutomaticReconciliationTrigger::ChangeTerminal,
                    None,
                );
                attach_automatic_reconciliation(&mut payload, reconciliation);
                Ok(payload)
            })?;
            emit_result(
                "ait-cli change close",
                &payload,
                args.json,
                &["task_id", "status", "publication_state"],
            )?;
            Ok(())
        }
        ChangeCommand::Publish(args) => {
            let change_ref = resolve_task_finish_change_input(&repo, &args.change_id, true, None)?;
            let payload = change_publish_cmd(&repo, &change_ref, args.remote.as_deref())?;
            emit_result(
                "ait-cli change publish",
                &payload,
                args.json,
                &[
                    "task_id",
                    "publication_state",
                    "published_remote_name",
                ],
            )?;
            Ok(())
        }
    }
}
