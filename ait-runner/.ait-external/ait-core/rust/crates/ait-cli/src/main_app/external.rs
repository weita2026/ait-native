fn run_external(repo: RepoRuntime, command: ExternalCommand) -> Result<ExitCode, String> {
    match command {
        ExternalCommand::Status(args) => {
            emit_external_payload("status", args.json, external_status_cmd(&repo)?)?;
            Ok(ExitCode::SUCCESS)
        }
        ExternalCommand::Doctor(args) => {
            let result = crate::binding_external::diagnose_external(&repo, args.fail_on_blocking)?;
            emit_external_payload("doctor", args.json, result.payload)?;
            Ok(ExitCode::from(result.exit_code))
        }

        ExternalCommand::Update(args) => {
            let options = external_update_options_from_args(&args)?;
            emit_external_payload("update", args.json, external_update_cmd(&repo, options)?)?;
            Ok(ExitCode::SUCCESS)
        }
        ExternalCommand::Link(args) => {
            emit_external_payload(
                "link",
                args.json,
                external_link_cmd(&repo, &args.name, &args.path)?,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        ExternalCommand::Unlink(args) => {
            emit_external_payload("unlink", args.json, external_unlink_cmd(&repo, &args.name)?)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn emit_external_payload(
    command_name: &str,
    json_output: bool,
    payload: JsonValue,
) -> Result<(), String> {
    if json_output {
        print_json(&payload)
    } else {
        println!("{}", render_external_text(command_name, &payload)?);
        Ok(())
    }
}

fn external_update_options_from_args(
    args: &ExternalUpdateArgs,
) -> Result<ExternalUpdateOptions, String> {
    crate::binding_external::external_update_options(
        &crate::binding_external::ExternalUpdateRequest {
            name: args.name.clone(),
            snapshot: args.snapshot.clone(),
            latest: args.latest,
            locked: args.locked,
            validate: args.validate,
            no_recursive: args.no_recursive,
        },
    )
}
