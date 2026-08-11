fn run_binary_db_command(command: BinaryDbCommand) -> Result<(), String> {
    match command {
        BinaryDbCommand::UpgradeU64Seconds(args) => {
            let staged = stage_binary_db_u64_second_upgrade(
                StageBinaryDbU64SecondUpgradeOptions {
                    repo_root: args.repo_root.clone(),
                    output_root: args.generation_root.clone(),
                    source_time_width: args.source_time_width,
                },
            )?;
            let activation = if args.activate {
                Some(activate_binary_db_generation(
                    BinaryDbGenerationActivationOptions {
                        repo_root: args.repo_root,
                        generation_root: args.generation_root,
                        expected_current_authority_fingerprint: Some(
                            staged.source_authority_fingerprint.clone(),
                        ),
                    },
                )?)
            } else {
                None
            };
            if args.json {
                print_json(&json!({
                    "schema": "ait.binary-db-u64-second-upgrade.v1",
                    "repo_name": staged.repo_name,
                    "source_time_width": staged.source_time_width,
                    "target_time_width": staged.target_time_width,
                    "source_authority_fingerprint": staged.source_authority_fingerprint,
                    "content_fingerprint": staged.content_fingerprint,
                    "converted_file_count": staged.converted_file_count,
                    "rebuilt_index_file_count": staged.rebuilt_index_file_count,
                    "copied_file_count": staged.copied_file_count,
                    "source_bytes": staged.source_bytes,
                    "target_bytes": staged.target_bytes,
                    "generation_root": staged.output_root.display().to_string(),
                    "activated": activation.is_some(),
                    "authority_root": activation.as_ref().map(|value| value.authority_root.clone()),
                    "retained_previous_authority": activation.as_ref().and_then(|value| value.retained_previous_authority.clone()),
                    "activation_strategy": activation.as_ref().map(|value| value.activation_strategy.clone()),
                }))?;
            } else {
                print_key_values(
                    "ait binary-db upgrade-u64-seconds",
                    &[
                        ("repository", staged.repo_name),
                        ("source time width", staged.source_time_width),
                        ("target time width", staged.target_time_width),
                        (
                            "generation root",
                            staged.output_root.display().to_string(),
                        ),
                        ("source bytes", staged.source_bytes.to_string()),
                        ("target bytes", staged.target_bytes.to_string()),
                        (
                            "rebuilt content indexes",
                            staged.rebuilt_index_file_count.to_string(),
                        ),
                        ("activated", activation.is_some().to_string()),
                        (
                            "retained previous authority",
                            activation
                                .and_then(|value| value.retained_previous_authority)
                                .unwrap_or_default(),
                        ),
                    ],
                );
            }
            Ok(())
        }
    }
}
