fn run_review(repo: RepoRuntime, command: ReviewCommand) -> Result<(), String> {
    match command {
        ReviewCommand::Show(args) => {
            let payload = review_show(&repo, &args.change_id, args.remote.as_deref(), None)?;
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
                    let payload = review_request(
                        &repo,
                        &args.change_id,
                        args.patchset_id.as_deref(),
                        &args.reviewer_groups,
                        args.note.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    emit_result(
                        "ait-cli review team request",
                        &payload,
                        args.json,
                        &["change_id", "patchset_id", "requested_groups", "status"],
                    )?;
                    Ok(())
                }
                ReviewTeamCommand::Approve(args) => {
                    let payload = review_team_approve(
                        &repo,
                        &args.change_id,
                        args.patchset_id.as_deref(),
                        args.reviewer.as_deref(),
                        args.message.as_deref(),
                        args.remote.as_deref(),
                    )?;
                    emit_result(
                        "ait-cli review team approve",
                        &payload,
                        args.json,
                        &["change_id", "patchset_id", "reviewer", "action"],
                    )?;
                    Ok(())
                }
                ReviewTeamCommand::RequestChanges(args) => emit_review_record_result(
                    "ait-cli review team request-changes",
                    review_record(
                        &repo,
                        &args.change_id,
                        "request_changes",
                        true,
                        args.patchset_id.as_deref(),
                        args.reviewer.as_deref(),
                        args.message.as_deref(),
                        args.remote.as_deref(),
                    )?,
                    args.json,
                ),
                ReviewTeamCommand::Comment(args) => emit_review_record_result(
                    "ait-cli review team comment",
                    review_record(
                        &repo,
                        &args.change_id,
                        "comment",
                        false,
                        args.patchset_id.as_deref(),
                        args.reviewer.as_deref(),
                        args.message.as_deref(),
                        args.remote.as_deref(),
                    )?,
                    args.json,
                ),
                ReviewTeamCommand::Defer(args) => emit_review_record_result(
                    "ait-cli review team defer",
                    review_record(
                        &repo,
                        &args.change_id,
                        "defer",
                        false,
                        args.patchset_id.as_deref(),
                        args.reviewer.as_deref(),
                        args.message.as_deref(),
                        args.remote.as_deref(),
                    )?,
                    args.json,
                ),
            }
        }
        ReviewCommand::Task { command } => match command {
            ReviewTaskCommand::Approve(args) => {
                let payload = review_task_approve(
                    &repo,
                    &args.change_id,
                    &args.patchset_id,
                    &args.message,
                    args.remote.as_deref(),
                )?;
                emit_result(
                    "ait-cli review task approve",
                    &payload,
                    args.json,
                    &["change_id", "patchset_id", "reviewer", "action"],
                )?;
                Ok(())
            }
            ReviewTaskCommand::RequestChanges(args) => emit_review_record_result(
                "ait-cli review task request-changes",
                review_task_record(
                    &repo,
                    &args.change_id,
                    "task_request_changes",
                    true,
                    args.patchset_id.as_deref(),
                    args.message.as_deref(),
                    args.remote.as_deref(),
                )?,
                args.json,
            ),
            ReviewTaskCommand::Comment(args) => emit_review_record_result(
                "ait-cli review task comment",
                review_task_record(
                    &repo,
                    &args.change_id,
                    "task_comment",
                    false,
                    args.patchset_id.as_deref(),
                    args.message.as_deref(),
                    args.remote.as_deref(),
                )?,
                args.json,
            ),
            ReviewTaskCommand::Defer(args) => emit_review_record_result(
                "ait-cli review task defer",
                review_task_record(
                    &repo,
                    &args.change_id,
                    "task_defer",
                    false,
                    args.patchset_id.as_deref(),
                    args.message.as_deref(),
                    args.remote.as_deref(),
                )?,
                args.json,
            ),
        },
        ReviewCommand::Code { command } => match command {
            ReviewCodeCommand::Submit(args) => {
                let payload = review_code_submit(
                    &repo,
                    &args.change_id,
                    &args.patchset_id,
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
