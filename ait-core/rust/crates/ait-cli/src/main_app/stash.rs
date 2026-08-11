fn run_stash(repo: RepoRuntime, command: StashCommand) -> Result<(), String> {
    match command {
        StashCommand::Save(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli stash save", || {
                stash_save(&repo, args.message.as_deref(), args.keep_workspace)
            })?;
            emit_stash_record("ait-cli stash save", &payload, args.json)?;
        }
        StashCommand::List(args) => {
            let payload = stash_list(&repo)?;
            if args.json {
                print_json(&payload)?;
            } else {
                let rows = payload
                    .as_array()
                    .ok_or_else(|| "stash list payload must decode to a list.".to_string())?;
                print_list(
                    rows,
                    &[
                        "stash_id",
                        "source_line_name",
                        "snapshot_id",
                        "file_count",
                        "created_at",
                        "message",
                    ],
                );
            }
        }
        StashCommand::Show(args) => {
            let payload = stash_show(&repo, &args.stash_id)?;
            emit_stash_record("ait-cli stash show", &payload, args.json)?;
        }
        StashCommand::Apply(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli stash apply", || {
                stash_apply(&repo, &args.stash_id, args.force)
                    .map_err(|err| stash_restore_error(err, args.force))
            })?;
            emit_stash_restore("ait-cli stash apply", &payload, args.json)?;
        }
        StashCommand::Pop(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli stash pop", || {
                stash_pop(&repo, &args.stash_id, args.force)
                    .map_err(|err| stash_restore_error(err, args.force))
            })?;
            emit_stash_restore("ait-cli stash pop", &payload, args.json)?;
        }
        StashCommand::Drop(args) => {
            let payload = run_locked_workspace_command(&repo, "ait-cli stash drop", || {
                stash_drop(&repo, &args.stash_id)
            })?;
            emit_result(
                "ait-cli stash drop",
                &payload,
                args.json,
                &["stash_id", "snapshot_id", "dropped", "snapshot_deleted"],
            )?;
        }
    }
    Ok(())
}

fn stash_restore_error(err: String, force: bool) -> String {
    if !force && err.starts_with("Workspace has unsaved changes") {
        return format!("{err}. Re-run with --force to replace those workspace changes.");
    }
    err
}

fn emit_stash_record(title: &str, payload: &JsonValue, json_output: bool) -> Result<(), String> {
    emit_result(
        title,
        payload,
        json_output,
        &[
            "stash_id",
            "source_line_name",
            "snapshot_id",
            "file_count",
            "total_bytes",
            "workspace_cleared",
            "created_at",
            "message",
        ],
    )
}

fn emit_stash_restore(title: &str, payload: &JsonValue, json_output: bool) -> Result<(), String> {
    emit_result(
        title,
        payload,
        json_output,
        &[
            "stash_id",
            "source_line_name",
            "snapshot_id",
            "current_line",
            "applied",
            "dropped",
            "snapshot_deleted",
            "created_at",
            "message",
        ],
    )
}
