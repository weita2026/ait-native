fn run_stash(repo: RepoRuntime, command: StashCommand) -> Result<(), String> {
    match command {
        StashCommand::Save(args) => {
            let payload = crate::binding_services::save_stash(
                &repo,
                args.message.as_deref(),
                args.keep_workspace,
            )?;
            emit_stash_record("ait-cli stash save", &payload, args.json)?;
        }
        StashCommand::List(args) => {
            let payload = stash_list(&repo)?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_stash_list(&payload)?);
            }
        }
        StashCommand::Show(args) => {
            let payload = stash_show(&repo, &args.stash_id)?;
            emit_stash_record("ait-cli stash show", &payload, args.json)?;
        }
        StashCommand::Apply(args) => {
            let payload = crate::binding_services::apply_stash(&repo, &args.stash_id, args.force)?;
            emit_stash_restore("ait-cli stash apply", &payload, args.json)?;
        }
        StashCommand::Pop(args) => {
            let payload = crate::binding_services::pop_stash(&repo, &args.stash_id, args.force)?;
            emit_stash_restore("ait-cli stash pop", &payload, args.json)?;
        }
        StashCommand::Drop(args) => {
            let payload = crate::binding_services::drop_stash(&repo, &args.stash_id)?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_stash_drop(&payload)?);
            }
        }
    }
    Ok(())
}

fn emit_stash_record(title: &str, payload: &JsonValue, json_output: bool) -> Result<(), String> {
    if json_output {
        return print_json(payload);
    }
    println!("{}", stash_record_text(title, payload)?);
    Ok(())
}

pub(crate) fn stash_record_text(title: &str, payload: &JsonValue) -> Result<String, String> {
    result_text(
        title,
        payload,
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
    if json_output {
        return print_json(payload);
    }
    println!("{}", stash_restore_text(title, payload)?);
    Ok(())
}

pub(crate) fn stash_restore_text(title: &str, payload: &JsonValue) -> Result<String, String> {
    result_text(
        title,
        payload,
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
