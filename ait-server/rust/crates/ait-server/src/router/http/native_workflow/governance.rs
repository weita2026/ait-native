use super::*;

pub(in super::super) async fn native_record_repository_authority_review(
    State(state): State<ServerState>,
    Path((repo_id, change_ref)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native repository authority review record",
        state.runtime_service.clone(),
        move || {
            let (_, workflow) =
                repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
            let resolved_change = workflow.get_change(None, &change_ref)?;
            let change_ref = resolved_change
                .get("change_ref")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "resolved change is missing change_ref".to_string())?;
            workflow.record_review(change_ref, &payload)
        },
    )
    .await
}

pub(in super::super) async fn native_list_repository_authority_reviews(
    State(state): State<ServerState>,
    Path((repo_id, change_ref)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority review list", move || {
        let (_, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        let resolved_change = workflow.get_change(None, &change_ref)?;
        let change_ref = resolved_change
            .get("change_ref")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| "resolved change is missing change_ref".to_string())?;
        workflow.list_reviews(change_ref)
    })
    .await
}

pub(in super::super) async fn native_put_repository_authority_attestation(
    State(state): State<ServerState>,
    Path((repo_id, patchset_id)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native repository authority attestation put",
        state.runtime_service.clone(),
        move || {
            let (_, workflow) =
                repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
            workflow.put_attestation(&patchset_id, &payload)
        },
    )
    .await
}

pub(in super::super) async fn native_get_repository_authority_attestation(
    State(state): State<ServerState>,
    Path((repo_id, patchset_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority attestation read", move || {
        let (_, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        workflow.get_attestation(&patchset_id)
    })
    .await
}

pub(in super::super) async fn native_get_repository_authority_policy(
    State(state): State<ServerState>,
    Path((repo_id, patchset_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority policy read", move || {
        let (_, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        workflow.get_policy(&patchset_id)
    })
    .await
}

pub(in super::super) async fn native_get_repository_authority_patchset(
    State(state): State<ServerState>,
    Path((repo_id, patchset_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority patchset read", move || {
        let (_, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        workflow.get_patchset(None, &patchset_id)
    })
    .await
}

pub(in super::super) async fn native_repository_authority_patchset_action(
    State(state): State<ServerState>,
    Path((repo_id, patchset_tail)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    if let Ok(patchset_id) = parse_suffixed_tail(&patchset_tail, ":runCi", "patchset action") {
        let runtime = state.runtime_service.clone();
        return run_workflow_mutation(
            "native repository authority patchset runCi",
            state.runtime_service.clone(),
            move || runtime.run_repository_authority_patchset_ci(&repo_id, &patchset_id, &payload),
        )
        .await;
    }
    if let Ok(patchset_id) =
        parse_suffixed_tail(&patchset_tail, ":evaluatePolicy", "patchset action")
    {
        let routed_workflow = state.workflow_service.clone();
        return run_workflow_mutation(
            "native repository authority patchset policy",
            state.runtime_service.clone(),
            move || {
                let (_, workflow) =
                    repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
                workflow.evaluate_policy(&patchset_id)
            },
        )
        .await;
    }
    Err(ApiError::not_found(
        "unknown repository authority patchset action route",
    ))
}

pub(in super::super) async fn native_get_repository_authority_land(
    State(state): State<ServerState>,
    Path((repo_id, submission_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority land read", move || {
        let (_, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        workflow.get_land(None, &submission_id)
    })
    .await
}
