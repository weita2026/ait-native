fn run_change(repo: RepoRuntime, command: ChangeCommand) -> Result<(), String> {
    match command {
        ChangeCommand::Create(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli change create", || {
                change_create_cmd(
                    &repo,
                    &args.task_id,
                    &args.title,
                    Some(&args.base_line),
                    args.local,
                    args.remote.as_deref(),
                )
            })?;
            emit_result(
                "ait-cli change create",
                &payload,
                args.json,
                &[
                    "change_id",
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
                print_json(&payload)?;
            } else if let Some(rows) = payload.as_array() {
                let rows = project_change_text_rows(rows);
                let include_publication = rows
                    .iter()
                    .any(|row| row.get("publication_state").is_some());
                let all_command =
                    scoped_all_command("ait change list", args.local, args.remote.as_deref());
                if include_publication {
                    print_agent_list(
                        &rows,
                        &[
                            "change",
                            "status",
                            "publication_state",
                            "title",
                        ],
                        args.all,
                        &["landed", "archived", "abandoned", "canceled"],
                        Some("open"),
                        &all_command,
                    );
                } else {
                    print_agent_list(
                        &rows,
                        &["change", "status", "title"],
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
            let payload = change_show_cmd(
                &repo,
                &args.change_id,
                args.local,
                args.remote.as_deref(),
                args.repo.as_deref(),
            )?;
            emit_result(
                "ait-cli change show",
                &payload,
                args.json,
                &[
                    "change_id",
                    "task_id",
                    "title",
                    "base_line",
                    "fork_snapshot_id",
                    "status",
                    "publication_state",
                    "published_change_id",
                ],
            )?;
            Ok(())
        }
        ChangeCommand::Revert(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli change revert", || {
                change_revert_cmd(
                    &repo,
                    &args.change_id,
                    args.force,
                    args.dry_run,
                    args.local,
                    args.remote.as_deref(),
                    args.repo.as_deref(),
                )
            })?;
            emit_result(
                "ait-cli change revert",
                &payload,
                args.json,
                &[
                    "change_id",
                    "fork_snapshot_id",
                    "latest_change_snapshot_id",
                    "current_line",
                    "applied",
                    "affected_path_count",
                ],
            )?;
            Ok(())
        }
        ChangeCommand::Replay(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli change replay", || {
                change_replay_cmd(
                    &repo,
                    &args.change_id,
                    &args.onto,
                    args.force,
                    args.dry_run,
                    args.local,
                    args.remote.as_deref(),
                    args.repo.as_deref(),
                )
            })?;
            emit_result(
                "ait-cli change replay",
                &payload,
                args.json,
                &[
                    "change_id",
                    "fork_snapshot_id",
                    "latest_change_snapshot_id",
                    "onto_line",
                    "applied",
                    "affected_path_count",
                ],
            )?;
            Ok(())
        }
        ChangeCommand::Close(args) => {
            let automatic_scope = if repo.task_uses_local_scope(args.local, args.remote.as_deref()) {
                AutomaticReconciliationScope::Local
            } else {
                AutomaticReconciliationScope::Remote(args.remote.clone())
            };
            let payload = run_locked_workspace_command(&repo, "ait-cli change close", || {
                let mut payload =
                    change_close_cmd(&repo, &args.change_id, args.local, args.remote.as_deref())?;
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
                &["change_id", "status", "publication_state"],
            )?;
            Ok(())
        }
        ChangeCommand::Publish(args) => {
            let payload = change_publish_cmd(&repo, &args.change_id, args.remote.as_deref())?;
            emit_result(
                "ait-cli change publish",
                &payload,
                args.json,
                &[
                    "change_id",
                    "publication_state",
                    "published_remote_name",
                    "published_change_id",
                ],
            )?;
            Ok(())
        }
    }
}
