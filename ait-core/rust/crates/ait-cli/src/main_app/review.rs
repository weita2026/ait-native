fn run_review(repo: RepoRuntime, command: ReviewCommand) -> Result<(), String> {
    if let ReviewCommand::Task { command } = &command {
        let (task,selector,message,remote,action,json_output) = match command {
            ReviewTaskCommand::Approve(args) => (&args.change_id,Some(args.patchset_id.as_str()),Some(args.message.as_str()),args.remote.as_deref(),"task_approve",args.json),
            ReviewTaskCommand::RequestChanges(args) => (&args.change_id,args.patchset_id.as_deref(),args.message.as_deref(),args.remote.as_deref(),"task_request_changes",args.json),
            ReviewTaskCommand::Comment(args) => (&args.change_id,args.patchset_id.as_deref(),args.message.as_deref(),args.remote.as_deref(),"task_comment",args.json),
            ReviewTaskCommand::Defer(args) => (&args.change_id,args.patchset_id.as_deref(),args.message.as_deref(),args.remote.as_deref(),"task_defer",args.json),
        };
        if let Some(identity) = selector.map(read_package_review_identity).transpose()?.flatten() {
            let payload=crate::primitives::workflow_package_task_review(&repo,task,&identity,action,message,remote)?;
            emit_result("ait-cli review task",&payload,json_output,&["task_id","reviewer","action","recorded_at_s"])?;
            return Ok(());
        }
    }
    match command {
        ReviewCommand::Show(args) => {
            let change_ref =
                resolve_task_remote_change_input(&repo, &args.change_id, args.remote.as_deref())?;
            let payload = review_show(&repo, &change_ref, args.remote.as_deref(), None)?;
            emit_review_show_result(&payload, args.json)
        }
        ReviewCommand::Team { command } => {
            if !repo.team_review_enabled() {
                return Err(
                    "`ait review team ...` is only available when Agent Teams is enabled."
                        .to_string(),
                );
            }
            match command {
                ReviewTeamCommand::Request(args) => {
                    let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                        &repo,
                        &args.change_id,
                        args.patchset_id.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    let payload = review_request(
                        &repo,
                        &change_ref,
                        Some(&patchset_id),
                        &args.reviewer_groups,
                        args.note.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    emit_result(
                        "ait-cli review team request",
                        &payload,
                        args.json,
                        &["patchset_id", "requested_groups", "status"],
                    )?;
                    Ok(())
                }
                ReviewTeamCommand::Approve(args) => {
                    let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                        &repo,
                        &args.change_id,
                        args.patchset_id.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    let payload = review_team_approve(
                        &repo,
                        &change_ref,
                        Some(&patchset_id),
                        args.reviewer.as_deref(),
                        args.message.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    emit_result(
                        "ait-cli review team approve",
                        &payload,
                        args.json,
                        &["patchset_id", "reviewer", "action"],
                    )?;
                    Ok(())
                }
                ReviewTeamCommand::RequestChanges(args) => {
                    let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                        &repo,
                        &args.change_id,
                        args.patchset_id.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    emit_review_record_result(
                    "ait-cli review team request-changes",
                    review_record(
                        &repo,
                        &change_ref,
                        "request_changes",
                        true,
                        Some(&patchset_id),
                        args.reviewer.as_deref(),
                        args.message.as_deref(),
                        args.remote.as_deref(),
                    )?,
                    args.json,
                )
                }
                ReviewTeamCommand::Comment(args) => {
                    let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                        &repo,
                        &args.change_id,
                        args.patchset_id.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    emit_review_record_result(
                    "ait-cli review team comment",
                    review_record(
                        &repo,
                        &change_ref,
                        "comment",
                        false,
                        Some(&patchset_id),
                        args.reviewer.as_deref(),
                        args.message.as_deref(),
                        args.remote.as_deref(),
                    )?,
                    args.json,
                )
                }
                ReviewTeamCommand::Defer(args) => {
                    let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                        &repo,
                        &args.change_id,
                        args.patchset_id.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    emit_review_record_result(
                    "ait-cli review team defer",
                    review_record(
                        &repo,
                        &change_ref,
                        "defer",
                        false,
                        Some(&patchset_id),
                        args.reviewer.as_deref(),
                        args.message.as_deref(),
                        args.remote.as_deref(),
                    )?,
                    args.json,
                )
                }
            }
        }
        ReviewCommand::Task { command } => match command {
            ReviewTaskCommand::Approve(args) => {
                let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                    &repo,
                    &args.change_id,
                    Some(args.patchset_id.as_str()),
                    args.remote.as_deref(),
                )?;
                let payload = review_task_approve(
                    &repo,
                    &change_ref,
                    &patchset_id,
                    &args.message,
                    args.remote.as_deref(),
                )?;
                emit_result(
                    "ait-cli review task approve",
                    &payload,
                    args.json,
                    &["patchset_id", "reviewer", "action"],
                )?;
                Ok(())
            }
            ReviewTaskCommand::RequestChanges(args) => {
                let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                    &repo,
                    &args.change_id,
                    args.patchset_id.as_deref(),
                    args.remote.as_deref(),
                )?;
                emit_review_record_result(
                "ait-cli review task request-changes",
                review_task_record(
                    &repo,
                    &change_ref,
                    "task_request_changes",
                    true,
                    Some(&patchset_id),
                    args.message.as_deref(),
                    args.remote.as_deref(),
                )?,
                args.json,
            )
            }
            ReviewTaskCommand::Comment(args) => {
                let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                    &repo,
                    &args.change_id,
                    args.patchset_id.as_deref(),
                    args.remote.as_deref(),
                )?;
                emit_review_record_result(
                "ait-cli review task comment",
                review_task_record(
                    &repo,
                    &change_ref,
                    "task_comment",
                    false,
                    Some(&patchset_id),
                    args.message.as_deref(),
                    args.remote.as_deref(),
                )?,
                args.json,
            )
            }
            ReviewTaskCommand::Defer(args) => {
                let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                    &repo,
                    &args.change_id,
                    args.patchset_id.as_deref(),
                    args.remote.as_deref(),
                )?;
                emit_review_record_result(
                "ait-cli review task defer",
                review_task_record(
                    &repo,
                    &change_ref,
                    "task_defer",
                    false,
                    Some(&patchset_id),
                    args.message.as_deref(),
                    args.remote.as_deref(),
                )?,
                args.json,
            )
            }
        },
        ReviewCommand::Code { command } => match command {
            ReviewCodeCommand::Submit(args) => {
                let (change_ref, patchset_id) = resolve_task_remote_revision_input(
                    &repo,
                    &args.change_id,
                    Some(args.patchset_id.as_str()),
                    args.remote.as_deref(),
                )?;
                let payload = review_code_submit_with_reviewer(
                    &repo,
                    &change_ref,
                    &patchset_id,
                    &args.message,
                    args.reviewer.as_deref(),
                    args.remote.as_deref(),
                )?;
                emit_review_code_submit_result(&payload, args.json)?;
                Ok(())
            }
            ReviewCodeCommand::Template(args) => {
                let payload = review_code_template(Some(args.style.as_str()))?;
                emit_review_code_template_result(&payload, args.json)
            }
        },
    }
}

fn read_package_review_identity(selector:&str) -> Result<Option<ait_core::workflow_expected_identity::PackageWorkflowExpectedIdentity>,String> {
    let Some(path)=selector.strip_prefix('@') else {return Ok(None);};
    if !std::path::Path::new(path).is_absolute() {return Err("Package review selector path must be absolute".into());}
    let resolved=std::fs::canonicalize(path).map_err(|error|format!("Resolve package review identity: {error}"))?;
    use ait_core::workflow_expected_identity::WorkflowExpectedIdentityInput as Expected;
    match read_workflow_expected_identity_input(Some(&resolved))? {
        Some(Expected::Package(identity))=>Ok(Some(identity)),
        _=>Err("Package review selector requires only the exact five-field package identity".into()),
    }
}
