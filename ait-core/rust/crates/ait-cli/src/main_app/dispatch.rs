fn dispatch_with_args(args: Vec<OsString>) -> ExitCode {
    match run(args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::from(1)
        }
    }
}

fn run(mut argv: Vec<OsString>) -> Result<ExitCode, String> {
    let _run_range = perfetto_range!("ait.cli.run");
    if argv.is_empty() {
        argv.push(OsString::from("ait"));
    }
    #[cfg(feature = "perfetto-tracing")]
    let mut forwarded_current_source_cli = false;
    let command = loop {
        let cli = match {
            let _range = perfetto_range!("ait.cli.parse_args");
            parse_cli_from(&argv)
        } {
            Ok(cli) => cli,
            Err(error) => {
                let exit_code = u8::try_from(error.exit_code()).unwrap_or(2);
                error
                    .print()
                    .map_err(|print_error| format!("Failed to render command help: {print_error}"))?;
                return Ok(ExitCode::from(exit_code));
            }
        };
        match cli.command {
            Commands::CurrentSourceCache {
                command: CurrentSourceCacheCommand::RunCli(args),
            } => {
                argv = validated_forwarded_cli_argv(args)?;
                #[cfg(feature = "perfetto-tracing")]
                {
                    forwarded_current_source_cli = true;
                }
            }
            command => break command,
        }
    };
    #[cfg(feature = "perfetto-tracing")]
    let _forwarded_command_range = forwarded_current_source_cli
        .then(|| perfetto_range!("ait.cli.current_source_bootstrap.forwarded_command"));
    let command = match command {
        Commands::Init(args) => {
            run_init(args)?;
            return Ok(ExitCode::SUCCESS);
        }
        Commands::Install(args) => {
            run_install(args)?;
            return Ok(ExitCode::SUCCESS);
        }
        Commands::BinaryDb { command } => {
            run_binary_db_command(command)?;
            return Ok(ExitCode::SUCCESS);
        }
        Commands::Agent { command } => {
            return ait_cli::agent_surface::run_command(command);
        }
        Commands::Doctor { command } => return run_doctor(command),
        Commands::CurrentSourceCache { command } => {
            run_current_source_cache(command)?;
            return Ok(ExitCode::SUCCESS);
        }
        Commands::Remote {
            command: RemoteCommand::RecoverHead(args),
        } => {
            run_remote_head_recovery(args)?;
            return Ok(ExitCode::SUCCESS);
        }
        command => command,
    };
    let repo = {
        let _range = perfetto_range!("ait.cli.repo_discover");
        if matches!(
            &command,
            Commands::Test {
                command: TestCommand::PatchsetCi { .. }
            }
        ) {
            RepoRuntime::discover_for_patchset_ci()?
        } else {
            RepoRuntime::discover()?
        }
    };
    match command {
        Commands::Init(_) => unreachable!("init is handled before repo discovery"),
        Commands::Install(_) => unreachable!("install is handled before repo discovery"),
        Commands::BinaryDb { .. } => {
            unreachable!("binary-db is handled before repo discovery")
        }
        Commands::Agent { .. } => unreachable!("agent is handled before repo discovery"),
        Commands::Doctor { .. } => unreachable!("doctor is handled before repo discovery"),
        Commands::CurrentSourceCache { .. } => {
            unreachable!("current-source-cache is handled before repo discovery")
        }
        Commands::Line { command } => {
            run_line(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Git { command } => {
            run_git_command(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Blame(args) => {
            run_blame(repo, args)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Queue { command } => run_queue(repo, command),
        Commands::Remote { command } => {
            run_remote(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Release { command } => {
            run_release(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Repo { command } => {
            run_repo(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Test { command } => {
            run_test(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Auth { command } => {
            run_auth(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Config { command } => {
            run_config(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::External { command } => {
            run_external(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Status(args) => {
            run_status(repo, args)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Diff(args) => {
            run_diff(repo, args)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Pull(args) => {
            run_pull(repo, args)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Push(args) => {
            run_push(repo, args)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Gc { command } => {
            run_gc(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Stash { command } => {
            run_stash(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Plan { command } => {
            run_plan(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Task { command } => run_task(repo, command),
        Commands::Change { command } => {
            run_change(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Snapshot { command } => {
            run_snapshot(repo, command)
        }
        Commands::Tag { command } => {
            run_tag(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Patchset { command } => {
            run_patchset(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Review { command } => {
            run_review(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Attest { command } => {
            run_attest(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Policy { command } => {
            run_policy(repo, command)?;
            Ok(ExitCode::SUCCESS)
        }
        Commands::Worktree { command } => run_worktree(repo, command),
        Commands::Workflow { command } => run_workflow(repo, command),
    }
}

fn parse_cli_from(args: &[OsString]) -> Result<Cli, clap::Error> {
    let mut public_args = args.to_vec();
    if let Some(binary_name) = public_args.first_mut() {
        *binary_name = OsString::from("ait");
    } else {
        public_args.push(OsString::from("ait"));
    }
    match Cli::try_parse_from(&public_args) {
        Ok(cli) => Ok(cli),
        Err(mut error) => {
            if is_retired_workflow_json_invocation(&public_args) {
                for context in [
                    ContextKind::Suggested,
                    ContextKind::SuggestedArg,
                    ContextKind::SuggestedSubcommand,
                    ContextKind::SuggestedValue,
                ] {
                    error.remove(context);
                }
            }
            Err(error)
        }
    }
}

fn exit_code_value(code: ExitCode) -> u8 {
    (0..=u8::MAX)
        .find(|value| code == ExitCode::from(*value))
        .unwrap_or(1)
}

fn validated_forwarded_cli_argv(args: CurrentSourceRunCliArgs) -> Result<Vec<OsString>, String> {
    let _range = perfetto_range!("ait.cli.current_source_bootstrap.validate");
    let Some(namespace) = args.command.first().and_then(|value| value.to_str()) else {
        return Err("current-source-cache run-cli requires a UTF-8 public command namespace after `--`.".to_string());
    };
    if namespace == "current-source-cache" {
        return Err("current-source-cache run-cli refuses recursive internal command dispatch.".to_string());
    }
    let executable_path = env::current_exe()
        .map_err(|err| format!("Failed to resolve the current ait-cli executable: {err}"))?;
    validate_current_source_cli_bootstrap(&CurrentSourceCliBootstrapRequest {
        core_repo_root: args.core_repo_root,
        metadata_path: args.metadata_path,
        executable_path: executable_path.clone(),
    })?;
    let mut argv = Vec::with_capacity(args.command.len() + 1);
    argv.push(executable_path.into_os_string());
    argv.extend(args.command);
    Ok(argv)
}

fn is_retired_workflow_json_invocation(args: &[std::ffi::OsString]) -> bool {
    matches!(
        args.get(1).and_then(|value| value.to_str()),
        Some("workflow")
    ) && matches!(
        args.get(2).and_then(|value| value.to_str()),
        Some("ready" | "land")
    ) && args
        .iter()
        .skip(3)
        .any(|value| {
            value
                .to_str()
                .is_some_and(|value| value == "--json" || value.starts_with("--json="))
        })
}
