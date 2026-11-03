fn run_attest(repo: RepoRuntime, command: AttestCommand) -> Result<(), String> {
    match command {
        AttestCommand::Put(args) => {
            let payload = attest_put(
                &repo,
                Some(&args.patchset_id),
                None,
                args.tests.as_deref(),
                args.lint.as_deref(),
                args.security.as_deref(),
                args.license.as_deref(),
                args.author_mode.as_deref(),
                args.model.as_deref(),
                args.remote.as_deref(),
                None,
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_attest_put(&payload)?);
            }
            Ok(())
        }
        AttestCommand::Show(args) => {
            let payload = attest_show_cmd(&repo, &args.patchset_id, args.remote.as_deref(), None)?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_attest_show(&payload)?);
            }
            Ok(())
        }
    }
}

fn run_policy(repo: RepoRuntime, command: PolicyCommand) -> Result<(), String> {
    match command {
        PolicyCommand::Eval(args) => {
            let payload = policy_eval(&repo, &args.patchset_id, args.remote.as_deref(), None)?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_policy_eval(&payload)?);
            }
            Ok(())
        }
        PolicyCommand::Show(args) => {
            let payload = policy_show(&repo, &args.patchset_id, args.remote.as_deref(), None)?;
            emit_policy_show_result(&payload, args.json)
        }
        PolicyCommand::Waive(args) => {
            let payload = policy_waive(
                &repo,
                &args.patchset_id,
                &args.rule_name,
                &args.reason,
                args.expires_at.as_deref(),
                args.remote.as_deref(),
                None,
            )?;
            if args.json {
                print_json(&payload)?;
            } else {
                println!("{}", crate::binding_output::format_policy_waive(&payload)?);
            }
            Ok(())
        }
    }
}
