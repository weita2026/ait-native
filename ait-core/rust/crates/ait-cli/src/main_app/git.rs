fn run_git_command(repo: RepoRuntime, command: GitCommand) -> Result<(), String> {
    match command {
        GitCommand::Migrate(args) => {
            let payload = crate::binding_git::migrate_git(&repo, &args.staging_root, args.apply)?;
            emit_git_result(
                &payload,
                args.json,
                crate::binding_output::format_git_migrate,
            )
        }
        GitCommand::Recover(args) => {
            let payload = crate::binding_git::recover_git(&repo, args.apply)?;
            emit_git_result(
                &payload,
                args.json,
                crate::binding_output::format_git_recover,
            )
        }
        GitCommand::Import(args) => {
            let payload = crate::binding_git::import_git(
                &repo,
                &args.source,
                args.all_branches_and_tags,
                args.dry_run,
            )?;
            emit_git_result(
                &payload,
                args.json,
                crate::binding_output::format_git_import,
            )
        }
        GitCommand::Export(args) => {
            let payload = crate::binding_git::export_git(
                &repo,
                &args.target,
                args.all_lines_and_tags,
                args.dry_run,
                args.generation.as_deref(),
                args.git_object.as_deref(),
            )?;
            emit_git_result(
                &payload,
                args.json,
                crate::binding_output::format_git_export,
            )
        }
        GitCommand::Restore(args) => {
            let payload = crate::binding_git::restore_git(
                &repo,
                &args.generation,
                &args.git_object,
                &args.target,
            )?;
            emit_git_result(
                &payload,
                args.json,
                crate::binding_output::format_git_restore,
            )
        }
        GitCommand::Mirror(args) => {
            let payload = crate::binding_git::mirror_git(
                &repo,
                &args.endpoint,
                &args.direction,
                args.dry_run,
            )?;
            emit_git_result(
                &payload,
                args.json,
                crate::binding_output::format_git_mirror,
            )
        }
    }
}

fn emit_git_result(
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
