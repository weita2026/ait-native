// Shared Plan operations construct authoritative requests in Rust. These
// services never parse terminal arguments or dispatch a CLI command.
pub(crate) fn plan_list_service(
    repo: &RepoRuntime,
    scope: &crate::binding_services::PlanScope,
) -> Result<JsonValue, String> {
    let args = plan_query_scope(scope);
    let mut payload = execute_plan_list_command_request_json(&build_query_request(repo, &args)?)?;
    if matches!(
        resolve_plan_scope(repo, scope.local, scope.remote_name.as_deref())?,
        ResolvedPlanScope::Local
    ) {
        annotate_plan_rows_with_workspace_match(repo, &mut payload);
    }
    Ok(payload)
}

fn plan_query_scope(scope: &crate::binding_services::PlanScope) -> QueryScopeArgs {
    QueryScopeArgs {
        local: scope.local,
        remote: scope.remote_name.clone(),
        json: true,
    }
}

fn plan_selection_args(request: &crate::binding_services::PlanSelection) -> ShowArgs {
    ShowArgs {
        plan_id: request.plan_id.clone(),
        revision: request.revision.clone(),
        scope: plan_query_scope(&request.scope),
    }
}

pub(crate) fn plan_show_service(
    repo: &RepoRuntime,
    request: &crate::binding_services::PlanSelection,
) -> Result<JsonValue, String> {
    execute_plan_show_command_request_json(&build_show_request(
        repo,
        &plan_selection_args(request),
    )?)
}

pub(crate) fn plan_revisions_service(
    repo: &RepoRuntime,
    plan_id: &str,
    scope: &crate::binding_services::PlanScope,
) -> Result<JsonValue, String> {
    let args = PlanIdArgs {
        plan_id: plan_id.to_string(),
        scope: plan_query_scope(scope),
        all: true,
    };
    execute_plan_revisions_command_request_json(&build_plan_id_request(repo, &args)?)
}

pub(crate) fn plan_items_service(
    repo: &RepoRuntime,
    request: &crate::binding_services::PlanSelection,
) -> Result<JsonValue, String> {
    execute_plan_items_command_request_json(&build_show_request(
        repo,
        &plan_selection_args(request),
    )?)
}

pub(crate) fn plan_inspect_service(
    repo: &RepoRuntime,
    request: &crate::binding_services::PlanSelection,
) -> Result<JsonValue, String> {
    execute_plan_inspect_command_request_json(&build_show_request(
        repo,
        &plan_selection_args(request),
    )?)
}

pub(crate) fn plan_candidates_service(
    repo: &RepoRuntime,
    request: &crate::binding_services::PlanCandidatesRequest,
) -> Result<JsonValue, String> {
    let args = CandidatesArgs {
        local: request.scope.local,
        remote: request.scope.remote_name.clone(),
        include_all: request.include_all,
        contains: request.contains.clone(),
        json: true,
    };
    execute_plan_candidates_command_request_json(&build_candidates_request(repo, &args)?)
}

pub(crate) fn plan_sync_service(
    repo: &RepoRuntime,
    request: &crate::binding_services::PlanSyncRequest,
) -> Result<JsonValue, String> {
    let args = SyncArgs {
        target: request.target.clone(),
        plan_ref: request.plan_ref.clone(),
        prune: request.prune,
        local: request.scope.local,
        remote: request.scope.remote_name.clone(),
        reconcile: request.reconcile,
        json: true,
    };
    let mut payload = run_locked_workspace_command(repo, "ait-cli plan sync", || {
        crate::agent_teams::before_plan_sync(repo, &args.target, args.prune)?;
        execute_plan_sync_command_request_json(&build_sync_request(repo, &args)?)
    })
    .map_err(|error| plan_sync_missing_path_hint(&args, error))?;
    match payload.get("error").cloned() {
        Some(JsonValue::String(error)) => {
            payload["error"] = JsonValue::String(plan_sync_missing_path_hint(&args, error))
        }
        Some(JsonValue::Object(mut error)) => {
            if let Some(message) = error
                .get("message")
                .and_then(JsonValue::as_str)
                .map(ToString::to_string)
            {
                error.insert(
                    "message".to_string(),
                    JsonValue::String(plan_sync_missing_path_hint(&args, message)),
                );
                payload["error"] = JsonValue::Object(error);
            }
        }
        _ => {}
    }
    Ok(payload)
}
