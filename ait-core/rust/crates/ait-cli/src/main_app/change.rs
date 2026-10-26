fn run_change(repo: RepoRuntime, command: ChangeCommand) -> Result<(), String> {
    match command {
        ChangeCommand::Create(args) => {
            let payload = crate::binding_changes::create_change(
                &repo,
                &crate::binding_changes::ChangeCreateRequest {
                    task_id: args.task_id,
                    title: args.title,
                    base_line: args.base_line,
                    fork_snapshot: args.fork_snapshot,
                    scope: crate::binding_changes::ChangeScope {
                        local: args.local,
                        remote_name: args.remote,
                    },
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_change_create(&payload)?);
            }
            Ok(())
        }
        ChangeCommand::List(args) => {
            let payload = change_list_cmd(&repo, args.local, args.remote.as_deref())?;
            if args.json {
                print_json(&crate::binding_output::project_change_list(
                    &payload, args.all,
                ))?;
            } else if payload.is_array() {
                println!(
                    "{}",
                    crate::binding_output::format_change_list(
                        &payload,
                        args.all,
                        &crate::binding_services::PlanScope {
                            local: args.local,
                            remote_name: args.remote
                        }
                    )?
                );
            }
            Ok(())
        }
        ChangeCommand::Show(args) => {
            let payload = crate::binding_changes::inspect_change(
                &repo,
                &args.change_id,
                &crate::binding_changes::ChangeScope {
                    local: args.local,
                    remote_name: args.remote,
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_change_show(&payload)?);
            }
            Ok(())
        }
        ChangeCommand::Revert(args) => {
            let payload = crate::binding_changes::revert_change(
                &repo,
                &args.change_id,
                &crate::binding_changes::ChangeMutationOptions {
                    force: args.force,
                    dry_run: args.dry_run,
                    scope: crate::binding_changes::ChangeScope {
                        local: args.local,
                        remote_name: args.remote,
                    },
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_change_revert(&payload)?);
            }
            Ok(())
        }
        ChangeCommand::Replay(args) => {
            let payload = crate::binding_changes::replay_change(
                &repo,
                &args.change_id,
                args.onto.as_deref(),
                &crate::binding_changes::ChangeMutationOptions {
                    force: args.force,
                    dry_run: args.dry_run,
                    scope: crate::binding_changes::ChangeScope {
                        local: args.local,
                        remote_name: args.remote,
                    },
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_change_replay(&payload)?);
            }
            Ok(())
        }
        ChangeCommand::Close(args) => {
            let payload = crate::binding_changes::close_change(
                &repo,
                &args.change_id,
                &crate::binding_changes::ChangeScope {
                    local: args.local,
                    remote_name: args.remote,
                },
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_change_close(&payload)?);
            }
            Ok(())
        }
        ChangeCommand::Publish(args) => {
            let payload = crate::binding_changes::publish_change(
                &repo,
                &args.change_id,
                args.remote.as_deref(),
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!(
                    "{}",
                    crate::binding_output::format_change_publish(&payload)?
                );
            }
            Ok(())
        }
    }
}
