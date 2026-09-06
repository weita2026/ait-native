fn run_review(repo: RepoRuntime, command: ReviewCommand) -> Result<(), String> {
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
                    "`ait review team ...` is only available when `workflow_mode=team_remote`."
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
                let payload = review_code_submit(
                    &repo,
                    &change_ref,
                    &patchset_id,
                    &args.message,
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
