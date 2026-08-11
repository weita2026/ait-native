fn run_attest(repo: RepoRuntime, command: AttestCommand) -> Result<(), String> {
    match command {
        AttestCommand::Put(args) => {
            let payload = attest_put(
                &repo,
                args.patchset_id.as_deref(),
                args.change.as_deref(),
                args.tests.as_deref(),
                args.lint.as_deref(),
                args.security.as_deref(),
                args.license.as_deref(),
                args.author_mode.as_deref(),
                args.model.as_deref(),
                args.remote.as_deref(),
                None,
            )?;
            emit_result(
                "ait-cli attest put",
                &payload,
                args.json,
                &["patchset_id", "author_mode", "tests", "lint"],
            )?;
            Ok(())
        }
        AttestCommand::Show(args) => {
            let payload = attest_show_cmd(&repo, &args.patchset_id, args.remote.as_deref(), None)?;
            emit_result(
                "ait-cli attest show",
                &payload,
                args.json,
                &["attestation_id", "patchset_id", "author_mode"],
            )?;
            Ok(())
        }
    }
}

fn run_policy(repo: RepoRuntime, command: PolicyCommand) -> Result<(), String> {
    match command {
        PolicyCommand::Eval(args) => {
            let payload = policy_eval(&repo, &args.patchset_id, args.remote.as_deref(), None)?;
            emit_result(
                "ait-cli policy eval",
                &payload,
                args.json,
                &["patchset_id", "lane", "decision", "evaluated_at"],
            )?;
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
            emit_result(
                "ait-cli policy waive",
                &payload,
                args.json,
                &["waiver_id", "patchset_id", "rule_name", "expires_at"],
            )?;
            Ok(())
        }
    }
}

