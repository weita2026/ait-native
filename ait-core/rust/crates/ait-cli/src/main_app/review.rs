fn run_review(repo: RepoRuntime, command: ReviewCommand) -> Result<(), String> {
    use crate::binding_review as service;
    match command {
        ReviewCommand::Show(args) => {
            let payload = service::inspect_review(&repo, &args.change_id, args.remote.as_deref())?;
            emit_review_show_result(&payload, args.json)
        }
        ReviewCommand::Team { command } => match command {
            ReviewTeamCommand::Request(args) => {
                let payload = service::request_team_review(
                    &repo,
                    &args.change_id,
                    &args.reviewer_groups,
                    &service::ReviewRequestOptions {
                        patchset_id: args.patchset_id,
                        note: args.note,
                        remote_name: args.remote,
                    },
                )?;
                if args.json {
                    print_json(&payload)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_team_request(&payload)?
                    );
                    Ok(())
                }
            }
            ReviewTeamCommand::Approve(args) => {
                let payload = service::approve_team_review(
                    &repo,
                    &args.change_id,
                    &service::TeamReviewOptions {
                        patchset_id: args.patchset_id,
                        reviewer: args.reviewer,
                        message: args.message,
                        remote_name: args.remote,
                    },
                )?;
                if args.json {
                    print_json(&payload)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_team_approve(&payload)?
                    );
                    Ok(())
                }
            }
            ReviewTeamCommand::RequestChanges(args) => {
                let payload = service::request_team_changes(
                    &repo,
                    &args.change_id,
                    &service::TeamReviewOptions {
                        patchset_id: args.patchset_id,
                        reviewer: args.reviewer,
                        message: args.message,
                        remote_name: args.remote,
                    },
                )?;
                if args.json {
                    print_json(&payload)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_team_request_changes(&payload)?
                    );
                    Ok(())
                }
            }
            ReviewTeamCommand::Comment(args) => {
                let payload = service::comment_team_review(
                    &repo,
                    &args.change_id,
                    &service::TeamReviewOptions {
                        patchset_id: args.patchset_id,
                        reviewer: args.reviewer,
                        message: args.message,
                        remote_name: args.remote,
                    },
                )?;
                if args.json {
                    print_json(&payload)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_team_comment(&payload)?
                    );
                    Ok(())
                }
            }
            ReviewTeamCommand::Defer(args) => {
                let payload = service::defer_team_review(
                    &repo,
                    &args.change_id,
                    &service::TeamReviewOptions {
                        patchset_id: args.patchset_id,
                        reviewer: args.reviewer,
                        message: args.message,
                        remote_name: args.remote,
                    },
                )?;
                if args.json {
                    print_json(&payload)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_team_defer(&payload)?
                    );
                    Ok(())
                }
            }
        },
        ReviewCommand::Task { command } => match command {
            ReviewTaskCommand::Approve(args) => {
                let operation = service::approve_task_review(
                    &repo,
                    &args.change_id,
                    &args.patchset_id,
                    &args.message,
                    args.remote.as_deref(),
                )?;
                if args.json {
                    print_json(&crate::binding_output::project_review_task_operation(
                        &operation,
                    )?)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_task_approve(&operation)?
                    );
                    Ok(())
                }
            }
            ReviewTaskCommand::RequestChanges(args) => {
                let operation = service::request_task_changes(
                    &repo,
                    &args.change_id,
                    &service::TaskReviewOptions {
                        patchset_id: args.patchset_id,
                        message: args.message,
                        remote_name: args.remote,
                    },
                )?;
                if args.json {
                    print_json(&crate::binding_output::project_review_task_operation(
                        &operation,
                    )?)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_task_request_changes(&operation)?
                    );
                    Ok(())
                }
            }
            ReviewTaskCommand::Comment(args) => {
                let operation = service::comment_task_review(
                    &repo,
                    &args.change_id,
                    &service::TaskReviewOptions {
                        patchset_id: args.patchset_id,
                        message: args.message,
                        remote_name: args.remote,
                    },
                )?;
                if args.json {
                    print_json(&crate::binding_output::project_review_task_operation(
                        &operation,
                    )?)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_task_comment(&operation)?
                    );
                    Ok(())
                }
            }
            ReviewTaskCommand::Defer(args) => {
                let operation = service::defer_task_review(
                    &repo,
                    &args.change_id,
                    &service::TaskReviewOptions {
                        patchset_id: args.patchset_id,
                        message: args.message,
                        remote_name: args.remote,
                    },
                )?;
                if args.json {
                    print_json(&crate::binding_output::project_review_task_operation(
                        &operation,
                    )?)
                } else {
                    println!(
                        "{}",
                        crate::binding_output::format_review_task_defer(&operation)?
                    );
                    Ok(())
                }
            }
        },
        ReviewCommand::Code { command } => match command {
            ReviewCodeCommand::Submit(args) => {
                let payload = service::submit_code_review(
                    &repo,
                    &args.change_id,
                    &args.patchset_id,
                    &args.message,
                    args.reviewer.as_deref(),
                    args.remote.as_deref(),
                )?;
                emit_review_code_submit_result(&payload, args.json)
            }
            ReviewCodeCommand::Template(args) => {
                let payload = review_code_template(Some(args.style.as_str()))?;
                emit_review_code_template_result(&payload, args.json)
            }
        },
    }
}

pub(crate) fn read_package_review_identity(
    selector: &str,
) -> Result<Option<ait_core::workflow_expected_identity::PackageWorkflowExpectedIdentity>, String> {
    let Some(path) = selector.strip_prefix('@') else {
        return Ok(None);
    };
    if !std::path::Path::new(path).is_absolute() {
        return Err("Package review selector path must be absolute".into());
    }
    let resolved = std::fs::canonicalize(path)
        .map_err(|error| format!("Resolve package review identity: {error}"))?;
    use ait_core::workflow_expected_identity::WorkflowExpectedIdentityInput as Expected;
    match read_workflow_expected_identity_input(Some(&resolved))? {
        Some(Expected::Package(identity)) => Ok(Some(identity)),
        _ => Err(
            "Package review selector requires only the exact five-field package identity".into(),
        ),
    }
}
