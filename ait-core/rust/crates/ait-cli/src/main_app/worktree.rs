const WORKTREE_DESTRUCTIVE_CONFIRMATION_ERROR: &str =
    "Pass --yes to apply this destructive worktree operation, or use --dry-run to preview it.";

fn require_worktree_destructive_confirmation(dry_run: bool, yes: bool) -> Result<(), String> {
    if !dry_run && !yes {
        return Err(WORKTREE_DESTRUCTIVE_CONFIRMATION_ERROR.to_string());
    }
    Ok(())
}

fn run_worktree(repo: RepoRuntime, command: WorktreeCommand) -> Result<ExitCode, String> {
    match command {
        WorktreeCommand::Status(args) => {
            let payload = worktree_status(
                &repo,
                args.name.as_deref(),
                args.snapshot_id.as_deref(),
                args.line_name.as_deref(),
            )?;
            emit_worktree_status_result(&payload, args.json, args.verbose)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Restore(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli worktree restore", || {
                worktree_restore(
                    &repo,
                    args.name.as_deref(),
                    args.snapshot_id.as_deref(),
                    args.line_name.as_deref(),
                    &args.paths,
                    args.force,
                    args.dry_run,
                )
            })?;
            emit_result(
                "ait-cli worktree restore",
                &payload,
                args.json,
                &[
                    "worktree_name",
                    "workspace_root",
                    "current_line",
                    "line_name",
                    "line_head_snapshot_id",
                    "target_snapshot_id",
                    "applied",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Show(args) => {
            let payload = worktree_get(&repo, args.name.as_deref(), true)?;
            emit_worktree_show_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Path(args) => {
            let payload = touch_worktree_payload(&repo, args.name.as_deref())?;
            if args.json {
                print_json(&worktree_path_payload(&payload)?)?;
            } else if args.shell_output {
                println!(
                    "{}",
                    required_object_string_field(&payload, "shell_command")?
                );
            } else {
                println!("{}", worktree_open_path_text(&payload)?);
            }
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Doctor(args) => {
            let payload = worktree_doctor(&repo, args.refresh)?;
            emit_worktree_doctor_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::CleanupCandidates(args) => {
            let payload = worktree_cleanup_candidates(
                &repo,
                Some(args.older_than.as_str()),
                args.cleanup_policy.as_deref(),
                args.include_protected,
                args.allow_manual_only,
            )?;
            emit_worktree_cleanup_candidates_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Cleanup(args) => {
            require_worktree_destructive_confirmation(args.dry_run, args.yes)?;
            let payload = run_locked_workspace_command(&repo, "ait-cli worktree cleanup", || {
                worktree_cleanup(
                    &repo,
                    Some(args.older_than.as_str()),
                    args.cleanup_policy.as_deref(),
                    args.allow_manual_only,
                    args.limit,
                    args.dry_run,
                )
            })?;
            emit_worktree_cleanup_report_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::PruneStale(args) => {
            require_worktree_destructive_confirmation(args.dry_run, args.yes)?;
            let payload = worktree_prune_stale(&repo, args.dry_run)?;
            emit_worktree_prune_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::List(args) => {
            let payload = worktree_list(&repo, args.refresh)?;
            emit_worktree_list_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Sync(args) => {
            if args.all_worktrees && args.name.is_some() {
                return Err("Choose either a worktree name or --all".to_string());
            }
            if args.all_worktrees && args.line_name.is_some() {
                return Err(
                    "--line cannot be combined with --all; each worktree syncs to its own current line"
                        .to_string(),
                );
            }
            let payload = run_locked_workspace_command(&repo, "ait-cli worktree sync", || {
                if args.all_worktrees {
                    worktree_sync_all(&repo, args.force, args.dry_run)
                } else {
                    worktree_sync(
                        &repo,
                        args.name.as_deref(),
                        args.line_name.as_deref(),
                        args.force,
                        args.dry_run,
                    )
                }
            })?;
            emit_worktree_sync_result(&payload, args.json)?;
            let exit = if args.all_worktrees
                && !payload
                    .get("ok")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(true)
            {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            };
            Ok(exit)
        }
        WorktreeCommand::Recreate(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli worktree recreate", || {
                worktree_recreate(&repo, args.name.as_deref(), args.dry_run)
            })?;
            emit_result(
                "ait-cli worktree recreate",
                &payload,
                args.json,
                &[
                    "name",
                    "path",
                    "status",
                    "current_line",
                    "target_base_line",
                    "fork_snapshot_id",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::RecoverTask(args) => {
            let payload =
                run_locked_workspace_command(&repo, "ait-cli worktree recover-task", || {
                    worktree_recover_task(
                        &repo,
                        &args.task_id,
                        &args.change,
                        args.remote.as_deref(),
                        args.dry_run,
                    )
                })?;
            emit_result(
                "ait-cli worktree recover-task",
                &payload,
                args.json,
                &[
                    "status",
                    "task_id",
                    "change_id",
                    "name",
                    "path",
                    "current_line",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::RestoreOwnedHead(args) => {
            let payload =
                run_locked_workspace_command(&repo, "ait-cli worktree restore-owned-head", || {
                    worktree_restore_owned_head(&repo, args.name.as_deref(), args.dry_run)
                })?;
            emit_result(
                "ait-cli worktree restore-owned-head",
                &payload,
                args.json,
                &[
                    "name",
                    "path",
                    "status",
                    "restored_snapshot_id",
                    "owned_head_snapshot_id",
                    "current_line",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Rebase(args) => {
            if args.continue_rebase && args.abort_rebase {
                return Err("Choose either --continue or --abort".to_string());
            }
            if args.dry_run && (args.continue_rebase || args.abort_rebase) {
                return Err("--dry-run cannot be combined with --continue or --abort".to_string());
            }
            let payload = if args.continue_rebase {
                run_locked_workspace_command(&repo, "ait-cli worktree rebase --continue", || {
                    worktree_continue_rebase(&repo, args.name.as_deref())
                })?
            } else if args.abort_rebase {
                run_locked_workspace_command(&repo, "ait-cli worktree rebase --abort", || {
                    worktree_abort_rebase(&repo, args.name.as_deref())
                })?
            } else {
                run_locked_workspace_command(&repo, "ait-cli worktree rebase", || {
                    if args.dry_run {
                        worktree_preview_rebase(
                            &repo,
                            args.name.as_deref(),
                            args.onto_line.as_deref(),
                        )
                    } else {
                        worktree_rebase(&repo, args.name.as_deref(), args.onto_line.as_deref())
                    }
                })?
            };
            emit_result(
                "ait-cli worktree rebase",
                &payload,
                args.json,
                &[
                    "name",
                    "path",
                    "status",
                    "current_line",
                    "target_base_line",
                    "rebase_state",
                    "conflict_count",
                ],
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Remove(args) => {
            if args.all_stale && !args.names.is_empty() {
                return Err("Choose either one or more worktree names or --all-stale".to_string());
            }
            if !args.all_stale && args.names.is_empty() {
                return Err("Provide one or more worktree names or use --all-stale".to_string());
            }
            if args.all_stale && args.delete_path {
                return Err("--delete-path cannot be combined with --all-stale".to_string());
            }
            if args.all_stale && args.force {
                return Err("--force cannot be combined with --all-stale".to_string());
            }
            require_worktree_destructive_confirmation(args.dry_run, args.yes)?;
            let payload = run_locked_workspace_command(&repo, "ait-cli worktree remove", || {
                if args.all_stale {
                    worktree_prune_stale(&repo, args.dry_run)
                } else {
                    worktree_remove(
                        &repo,
                        &args.names,
                        false,
                        args.delete_path,
                        args.force,
                        args.dry_run,
                    )
                }
            })?;
            if args.all_stale {
                emit_worktree_prune_result(&payload, args.json)?;
            } else if args.json {
                print_json(&payload)?;
            } else if args.dry_run || args.names.len() > 1 {
                emit_worktree_cleanup_rows(
                    "ait-cli worktree remove",
                    &payload,
                    "planned_count",
                    "planned_rows",
                    "removed_count",
                    "removed_rows",
                )?;
            } else {
                let row = payload
                    .get("removed_rows")
                    .and_then(JsonValue::as_array)
                    .and_then(|rows| rows.first())
                    .ok_or("worktree remove payload is missing removed_rows[0].")?;
                emit_result(
                    "ait-cli worktree remove",
                    row,
                    false,
                    &[
                        "name",
                        "path",
                        "deleted_path",
                        "alias_path",
                        "target_base_line",
                    ],
                )?;
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
