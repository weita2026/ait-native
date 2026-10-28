// Native workflow orchestration shared by typed bindings and the CLI.
pub(crate) fn workflow_ready_service(
    repo: &RepoRuntime,
    request: &crate::binding_services::WorkflowReadyRequest,
) -> Result<crate::binding_services::WorkflowOperationResult, String> {
    use ait_core::workflow_expected_identity::WorkflowExpectedIdentityInput as Expected;
    let expected_path = request
        .expected_identity
        .as_deref()
        .map(std::fs::canonicalize)
        .transpose()
        .map_err(|error| format!("Resolve expected identity: {error}"))?;
    let input = read_workflow_expected_identity_input(expected_path.as_deref())?;
    let mut runner_outcome = None;
    let mut ci_wait_hook = |pending: &mut JsonValue| {
        attach_runner_hint_best_effort(repo, request.remote_name.as_deref(), pending);
        let outcome = match pending.get(RUNNER_HINT_PAYLOAD_KEY) {
            Some(hint) => run_ci_once(hint),
            None => no_runner_hint_outcome(pending, request.remote_name.as_deref()),
        };
        let wait = outcome["status"].as_str() == Some("succeeded");
        runner_outcome = Some(outcome);
        Ok(wait)
    };
    if matches!(input, Some(Expected::Package(_)))
        || (input.is_none()
            && crate::primitives::workflow_package_source_requested(repo, &request.task_id)?)
    {
        if request.snapshot_message.is_some() {
            return Err("Completed package history preserves its Snapshot messages; --snapshot-message is not applicable".into());
        }
        let expected = match &input {
            Some(Expected::Package(identity)) => Some(identity),
            _ => None,
        };
        let mut payload = if let Some(payload) = crate::primitives::workflow_package_accepted_retry(
            repo,
            &request.task_id,
            request.remote_name.as_deref(),
            expected,
            request.author_mode.as_deref(),
            request.apply && expected.is_some(),
            false,
        )? {
            payload
        } else {
            let result = run_task_scoped_workspace_command(
                repo,
                &request.task_id,
                true,
                true,
                request.remote_name.as_deref(),
                "ait-cli workflow ready",
                |execution_repo| {
                    crate::primitives::workflow_package_ready(
                        execution_repo,
                        &request.task_id,
                        request.remote_name.as_deref(),
                        expected,
                        request.apply,
                        crate::primitives::PackageReadyOptions {
                            summary: request.summary.as_deref(),
                            tests: request.tests.as_deref(),
                            lint: request.lint.as_deref(),
                            security: request.security.as_deref(),
                            license: request.license.as_deref(),
                            author_mode: request.author_mode.as_deref(),
                            model: request.model.as_deref(),
                        },
                        request.run_ci_once.then_some(&mut ci_wait_hook),
                    )
                },
            );
            workflow_ready_result_with_runner_diagnostics(result, runner_outcome.as_ref())?
        };
        attach_runner_hint_best_effort(repo, request.remote_name.as_deref(), &mut payload);
        if let Some(path) = expected_path.as_deref() {
            payload["expected_identity_path"] = json!(path);
        }
        if request.run_ci_once {
            payload[CI_RUNNER_ONCE_PAYLOAD_KEY] = runner_outcome.unwrap_or_else(|| {
                no_runner_hint_outcome(&payload, request.remote_name.as_deref())
            });
        }

        return Ok(crate::binding_services::WorkflowOperationResult {
            payload,
            exit_code: 0,
        });
    }
    let expected = match input {
        Some(Expected::Patchset(identity)) => Some(identity),
        _ => None,
    };
    let mut apply_once = || {
        run_task_scoped_workspace_command(
            repo,
            &request.task_id,
            true,
            true,
            request.remote_name.as_deref(),
            "ait-cli workflow ready --apply",
            |execution_repo| {
                workflow_ready_apply_guarded_with_ci_wait_hook(
                    execution_repo,
                    &request.task_id,
                    request.snapshot_message.as_deref(),
                    request.summary.as_deref(),
                    request.tests.as_deref(),
                    request.lint.as_deref(),
                    request.security.as_deref(),
                    request.license.as_deref(),
                    request.author_mode.as_deref(),
                    request.model.as_deref(),
                    request.remote_name.as_deref(),
                    expected.as_ref(),
                    None::<fn(&JsonValue) -> Result<(), String>>,
                    request.run_ci_once.then_some(&mut ci_wait_hook),
                )
            },
        )
    };
    let mut payload = if request.apply {
        let result = apply_once();
        workflow_ready_result_with_runner_diagnostics(result, runner_outcome.as_ref())?
    } else {
        let execution_repo = resolve_task_scoped_execution_repo(
            repo,
            &request.task_id,
            true,
            true,
            request.remote_name.as_deref(),
            "ait-cli workflow ready",
        )?;
        let state = workflow_ready_payload(
            &execution_repo,
            &request.task_id,
            request.remote_name.as_deref(),
        )?;
        validate_workflow_expected_identity(&execution_repo, &state, expected.as_ref())?;
        state
    };
    attach_runner_hint_best_effort(repo, request.remote_name.as_deref(), &mut payload);
    if request.run_ci_once {
        let outcome = runner_outcome
            .unwrap_or_else(|| no_runner_hint_outcome(&payload, request.remote_name.as_deref()));
        if let Some(object) = payload.as_object_mut() {
            object.insert(CI_RUNNER_ONCE_PAYLOAD_KEY.to_string(), outcome);
        }
    }

    Ok(crate::binding_services::WorkflowOperationResult {
        payload,
        exit_code: 0,
    })
}

fn workflow_ready_result_with_runner_diagnostics<T>(
    result: Result<T, String>,
    runner_outcome: Option<&JsonValue>,
) -> Result<T, String> {
    result.map_err(|error| match runner_outcome {
        Some(outcome) => format!("{error}\nCaptured one-shot runner diagnostics:\n{outcome:#}"),
        None => error,
    })
}

pub(crate) fn workflow_finish_service(
    repo: &RepoRuntime,
    request: &crate::binding_services::WorkflowFinishRequest,
) -> Result<crate::binding_services::WorkflowOperationResult, String> {
    let expected_path = request
        .expected_identity
        .as_deref()
        .map(|path| {
            std::fs::canonicalize(path)
                .map_err(|error| format!("Resolve expected identity {}: {error}", path.display()))
        })
        .transpose()?;
    use ait_core::workflow_expected_identity::WorkflowExpectedIdentityInput as Expected;
    let input = read_workflow_expected_identity_input(expected_path.as_deref())?;
    if matches!(input, Some(Expected::Package(_)))
        || (input.is_none()
            && crate::primitives::workflow_package_source_requested(repo, &request.task_id)?)
    {
        let expected = match &input {
            Some(Expected::Package(identity)) => Some(identity),
            _ => None,
        };
        let mut payload = if let Some(payload) = crate::primitives::workflow_package_accepted_retry(
            repo,
            &request.task_id,
            request.remote_name.as_deref(),
            expected,
            None,
            false,
            request.apply,
        )? {
            payload
        } else {
            let Some(identity) = expected else {
                return Err("Package finish requires --expected-identity <PATH> containing the exact identity returned by ready".into());
            };
            run_task_scoped_workspace_command(
                repo,
                &request.task_id,
                true,
                true,
                request.remote_name.as_deref(),
                "ait-cli workflow finish",
                |execution_repo| {
                    crate::primitives::workflow_package_finish(
                        execution_repo,
                        &request.task_id,
                        request.remote_name.as_deref(),
                        identity,
                        request.apply,
                        request.reviewer.as_deref(),
                        request.review_message.as_deref(),
                    )
                },
            )?
        };
        let done = payload["next_action"]["code"].as_str() == Some("done");
        if let Some(path) = expected_path.as_deref() {
            payload["expected_identity_path"] = json!(path);
        }

        return Ok(crate::binding_services::WorkflowOperationResult {
            payload,
            exit_code: u8::from(request.apply && !done),
        });
    }
    let expected = match input {
        Some(Expected::Patchset(identity)) => Some(identity),
        _ => None,
    };
    let mut payload = if request.apply {
        run_task_scoped_workspace_command(
            repo,
            &request.task_id,
            true,
            true,
            request.remote_name.as_deref(),
            "ait-cli workflow finish --apply",
            |execution_repo| {
                workflow_finish_apply_guarded_with_reviewer(
                    execution_repo,
                    &request.task_id,
                    request.review_message.as_deref(),
                    request.reviewer.as_deref(),
                    request.remote_name.as_deref(),
                    expected.as_ref(),
                    None::<fn(&JsonValue) -> Result<(), String>>,
                )
            },
        )?
    } else {
        let execution_repo = resolve_task_scoped_execution_repo(
            repo,
            &request.task_id,
            true,
            true,
            request.remote_name.as_deref(),
            "ait-cli workflow finish",
        )?;
        let state = workflow_land_payload(
            &execution_repo,
            &request.task_id,
            request.remote_name.as_deref(),
        )?;
        validate_workflow_expected_identity(&execution_repo, &state, expected.as_ref())?;
        state
    };
    if workflow_finish_should_reconcile(request.apply, expected.is_some()) {
        let task_id = workflow_payload_task_id(&payload);
        let reconciliation = run_automatic_reconciliation_locked(
            repo,
            AutomaticReconciliationScope::Remote(request.remote_name.clone()),
            task_id.as_deref(),
            task_land_automatic_trigger(&payload),
        );
        attach_automatic_reconciliation(&mut payload, reconciliation);
    } else if request.apply && expected.is_some() {
        attach_automatic_reconciliation(
            &mut payload,
            json!({
                "status": "skipped", "mutated": false,
                "reason": "expected_identity_scope",
                "detail": "Guarded finish retains only exact-candidate mutations; reconciliation is a separate explicit action."
            }),
        );
    }
    attach_runner_hint_best_effort(repo, request.remote_name.as_deref(), &mut payload);
    if request.apply && payload["closeout_status"].as_str() == Some("partial") {
        if let Some(path) = expected_path.as_deref() {
            let task =
                workflow_payload_task_id(&payload).unwrap_or_else(|| request.task_id.clone());
            let remote = payload["remote_name"]
                .as_str()
                .or(request.remote_name.as_deref());
            let remote_flag = remote
                .map(|name| format!(" --remote {}", shell_quote_text(name)))
                .unwrap_or_default();
            payload["closeout_recovery"]["command"] = json!(format!(
                "ait workflow finish {} --apply --expected-identity {}{remote_flag}",
                shell_quote_text(&task),
                shell_quote_text(&path.to_string_lossy())
            ));
            payload["closeout_recovery"]["detail"] = json!("Repair the reported closeout condition and replay this guarded finish with the same frozen expected identity; the original apply receipt is reused.");
        }
    }
    if request.apply {
        crate::team_remote::attach_finish_handoff(
            crate::agent_teams::enabled(repo),
            &repo.authoritative_repo_root(),
            crate::team_remote::FinishScope::Remote,
            &mut payload,
            &crate::agent_teams::CoordinatorSource,
        );
    }

    let exit_code = if request.apply {
        task_finish_exit_code(&payload)
    } else {
        0
    };
    Ok(crate::binding_services::WorkflowOperationResult { payload, exit_code })
}
