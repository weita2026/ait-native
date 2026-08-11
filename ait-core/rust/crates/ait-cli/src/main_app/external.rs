fn run_external(repo: RepoRuntime, command: ExternalCommand) -> Result<(), String> {
    match command {
        ExternalCommand::Status(args) => {
            emit_external_payload("status", args.json, external_status_cmd(&repo)?)
        }
        ExternalCommand::Doctor(args) => {
            emit_external_payload("doctor", args.json, external_doctor_cmd(&repo)?)
        }
        ExternalCommand::Update(args) => {
            let options = external_update_options_from_args(&args)?;
            emit_external_payload("update", args.json, external_update_cmd(&repo, options)?)
        }
        ExternalCommand::Link(args) => emit_external_payload(
            "link",
            args.json,
            external_link_cmd(&repo, &args.name, &args.path)?,
        ),
        ExternalCommand::Unlink(args) => emit_external_payload(
            "unlink",
            args.json,
            external_unlink_cmd(&repo, &args.name)?,
        ),
    }
}

fn emit_external_payload(command_name: &str, json_output: bool, payload: JsonValue) -> Result<(), String> {
    if json_output {
        print_json(&payload)
    } else {
        println!("{}", render_external_text(command_name, &payload)?);
        Ok(())
    }
}

fn external_update_options_from_args(args: &ExternalUpdateArgs) -> Result<ExternalUpdateOptions, String> {
    if args.snapshot.is_some() && args.latest {
        return Err("`ait external update` accepts either `--to <snapshot>` or `--latest`, not both.".to_string());
    }
    if args.locked && (args.name.is_some() || args.snapshot.is_some() || args.latest) {
        return Err("`ait external update --locked` materializes the committed lockfile and does not accept a target, `--to`, or `--latest`.".to_string());
    }
    let mut options = match (&args.name, &args.snapshot, args.latest) {
        (Some(name), Some(snapshot), false) => ExternalUpdateOptions::exact(name, snapshot),
        (Some(name), None, true) => ExternalUpdateOptions::latest(name),
        (None, None, false) => ExternalUpdateOptions::manifest_pins(),
        (None, Some(_), false) => {
            return Err("`ait external update --to <snapshot>` requires an external name.".to_string());
        }
        (None, None, true) => {
            return Err("`ait external update --latest` requires an external name.".to_string());
        }
        (Some(_), None, false) => {
            return Err("`ait external update <name>` requires `--to <snapshot>` or `--latest`.".to_string());
        }
        (_, Some(_), true) => unreachable!("--to and --latest conflict is handled above"),
    };
    options = options
        .with_locked(args.locked)
        .with_validate(args.validate)
        .with_no_recursive(args.no_recursive);
    Ok(options)
}
