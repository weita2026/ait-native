fn run_tag(repo: RepoRuntime, command: TagCommand) -> Result<(), String> {
    match command {
        TagCommand::Create(args) => {
            let payload = tag_create_cmd(
                &repo,
                TagCreateRequest {
                    name: args.name,
                    snapshot_id: args.snapshot,
                    message: args.message,
                    force: args.force,
                },
            )?;
            emit_result(
                "ait-cli tag create",
                &payload,
                args.json,
                &["name", "snapshot_id", "message", "created_at", "source_line"],
            )
        }
        TagCommand::List(args) => {
            let payload = tag_list_cmd(&repo)?;
            if args.json {
                print_json(&payload)
            } else if let Some(rows) = payload.as_array() {
                print_list(rows, &["name", "snapshot_id", "message", "created_at"]);
                Ok(())
            } else {
                Err("tag list payload must decode to a list.".to_string())
            }
        }
        TagCommand::Show(args) => {
            let payload = tag_show_cmd(&repo, &args.name)?;
            emit_result(
                "ait-cli tag show",
                &payload,
                args.json,
                &["name", "snapshot_id", "message", "created_at"],
            )
        }
        TagCommand::Delete(args) => {
            let payload = tag_delete_cmd(&repo, &args.name)?;
            emit_result(
                "ait-cli tag delete",
                &payload,
                args.json,
                &["name", "snapshot_id", "message", "deleted"],
            )
        }
    }
}
