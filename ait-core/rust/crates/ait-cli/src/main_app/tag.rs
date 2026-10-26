fn run_tag(repo: RepoRuntime, command: TagCommand) -> Result<(), String> {
    match command {
        TagCommand::Create(args) => {
            let payload = tag_create_cmd(
                &repo,
                TagCreateRequest {
                    name: args.name,
                    snapshot_id: args.snapshot,
                    message: args.message,
                },
            )?;
            if args.json {
                print_json(&payload)
            } else {
                println!("{}", crate::binding_output::format_tag_create(&payload)?);
                Ok(())
            }
        }
        TagCommand::List(args) => {
            let payload = tag_list_cmd(&repo)?;
            if args.json {
                print_json(&payload)
            } else {
                println!("{}", crate::binding_output::format_tag_list(&payload)?);
                Ok(())
            }
        }
        TagCommand::Show(args) => {
            let payload = tag_show_cmd(&repo, &args.name)?;
            if args.json {
                print_json(&payload)
            } else {
                println!("{}", crate::binding_output::format_tag_show(&payload)?);
                Ok(())
            }
        }
        TagCommand::Delete(args) => {
            let payload = tag_delete_cmd(&repo, &args.name)?;
            if args.json {
                print_json(&payload)
            } else {
                println!("{}", crate::binding_output::format_tag_delete(&payload)?);
                Ok(())
            }
        }
    }
}
