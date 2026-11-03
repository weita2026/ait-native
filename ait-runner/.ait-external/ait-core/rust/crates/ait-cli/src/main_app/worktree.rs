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
            let payload = crate::binding_worktrees::restore_worktree(
                &repo,
                &crate::binding_worktrees::WorktreeRestoreRequest {
                    name: args.name,
                    snapshot_id: args.snapshot_id,
                    line_name: args.line_name,
                    paths: args.paths,
                    force: args.force,
                    dry_run: args.dry_run,
                },
            )?;
            emit_worktree_operation_result(
                &payload,
                args.json,
                crate::binding_output::format_worktree_restore,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Show(args) => {
            let payload = crate::binding_worktrees::show_worktree(&repo, args.name.as_deref())?;
            emit_worktree_show_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Path(args) => {
            let payload =
                crate::binding_worktrees::open_worktree_path(&repo, args.name.as_deref())?;
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
            let payload = crate::binding_worktrees::cleanup_worktrees(
                &repo,
                &crate::binding_worktrees::WorktreeCleanupRequest {
                    older_than: Some(args.older_than),
                    cleanup_policy: args.cleanup_policy,
                    allow_manual_only: args.allow_manual_only,
                    limit: args.limit,
                    dry_run: args.dry_run,
                    yes: args.yes,
                },
            )?;
            emit_worktree_cleanup_report_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::PruneStale(args) => {
            let payload =
                crate::binding_worktrees::prune_stale_worktrees(&repo, args.dry_run, args.yes)?;
            emit_worktree_prune_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::List(args) => {
            let payload = worktree_list(&repo, args.refresh)?;
            emit_worktree_list_result(&payload, args.json)?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Sync(args) => {
            let payload = crate::binding_worktrees::sync_worktree(
                &repo,
                &crate::binding_worktrees::WorktreeSyncRequest {
                    name: args.name,
                    line_name: args.line_name,
                    all_worktrees: args.all_worktrees,
                    force: args.force,
                    dry_run: args.dry_run,
                },
            )?;
            emit_worktree_sync_result(&payload, args.json)?;
            Ok(ExitCode::from(
                crate::binding_worktrees::worktree_sync_exit_code(&payload, args.all_worktrees),
            ))
        }
        WorktreeCommand::Recreate(args) => {
            let payload = crate::binding_worktrees::recreate_worktree(
                &repo,
                args.name.as_deref(),
                args.dry_run,
            )?;
            emit_worktree_operation_result(
                &payload,
                args.json,
                crate::binding_output::format_worktree_recreate,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::RecoverTask(args) => {
            let payload = crate::binding_worktrees::recover_task_worktree(
                &repo,
                &args.task_id,
                args.change.as_deref(),
                args.remote.as_deref(),
                args.dry_run,
            )?;
            emit_worktree_operation_result(
                &payload,
                args.json,
                crate::binding_output::format_worktree_recover_task,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::RestoreOwnedHead(args) => {
            let payload = crate::binding_worktrees::restore_worktree_owned_head(
                &repo,
                args.name.as_deref(),
                args.dry_run,
            )?;
            emit_worktree_operation_result(
                &payload,
                args.json,
                crate::binding_output::format_worktree_restore_owned_head,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Rebase(args) => {
            let payload = crate::binding_worktrees::rebase_worktree(
                &repo,
                &crate::binding_worktrees::WorktreeRebaseRequest {
                    name: args.name,
                    onto_line: args.onto_line,
                    continue_rebase: args.continue_rebase,
                    abort_rebase: args.abort_rebase,
                    dry_run: args.dry_run,
                },
            )?;
            emit_worktree_operation_result(
                &payload,
                args.json,
                crate::binding_output::format_worktree_rebase,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        WorktreeCommand::Remove(args) => {
            let payload = crate::binding_worktrees::remove_worktrees(
                &repo,
                &crate::binding_worktrees::WorktreeRemoveRequest {
                    names: args.names.clone(),
                    all_stale: args.all_stale,
                    delete_path: args.delete_path,
                    force: args.force,
                    dry_run: args.dry_run,
                    yes: args.yes,
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_worktree_remove(
                        &payload,
                        args.all_stale,
                        args.dry_run,
                        args.names.len() > 1
                    )?
                );
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn emit_worktree_operation_result(
    payload: &JsonValue,
    json_output: bool,
    formatter: fn(&JsonValue) -> Result<String, String>,
) -> Result<(), String> {
    if json_output {
        print_json(payload)
    } else {
        println!("{}", formatter(payload)?);
        Ok(())
    }
}
