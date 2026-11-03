fn run_binary_db_command(command: BinaryDbCommand) -> Result<(), String> {
    use crate::binding_binary_db as service;
    let (payload, json_output, text) = match command {
        BinaryDbCommand::AgentPool50(args) => {
            let payload = service::convert_agent_pool_50(
                args.repo_root,
                args.generation_root,
                args.activate,
            )?;
            let text = (!args.json).then(|| service::format_agent_pool_50(&payload));
            (payload, args.json, text)
        }
        BinaryDbCommand::UpgradeU64Seconds(args) => {
            let payload = service::upgrade_u64_seconds(
                args.repo_root,
                args.generation_root,
                args.source_time_width,
                args.activate,
            )?;
            let text = (!args.json).then(|| service::format_upgrade_u64_seconds(&payload));
            (payload, args.json, text)
        }
    };
    if json_output {
        print_json(&payload)
    } else {
        println!("{}", text.unwrap_or_default());
        Ok(())
    }
}
