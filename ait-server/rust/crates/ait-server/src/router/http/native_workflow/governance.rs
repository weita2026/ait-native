use super::*;

pub(in super::super) async fn native_record_review(
    State(state): State<ServerState>,
    Path(change_id): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native review record",
        state.runtime_service.clone(),
        move || workflow.record_review(&change_id, &payload),
    )
    .await
}

pub(in super::super) async fn native_record_repository_review(
    State(state): State<ServerState>,
    Path((repo_name, change_ref)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    let resolve_workflow = workflow.clone();
    let resolved_change =
        task::spawn_blocking(move || resolve_workflow.get_change(Some(&repo_name), &change_ref))
            .await
            .map_err(|exc| {
                ApiError::internal(format!(
                    "native repository review resolve worker failed: {exc}"
                ))
            })?
            .map_err(ApiError::bad_request)?;
    let change_ref = resolved_change
        .get("change_ref")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ApiError::internal("resolved change is missing change_ref"))?
        .to_string();
    run_workflow_mutation(
        "native review record",
        state.runtime_service.clone(),
        move || workflow.record_review(&change_ref, &payload),
    )
    .await
}

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

pub(in super::super) async fn native_list_reviews(
    State(state): State<ServerState>,
    Path(change_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native review list", move || {
        workflow.list_reviews(&change_id)
    })
    .await
}

pub(in super::super) async fn native_list_repository_reviews(
    State(state): State<ServerState>,
    Path((repo_name, change_ref)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    let resolve_workflow = workflow.clone();
    let resolved_change =
        task::spawn_blocking(move || resolve_workflow.get_change(Some(&repo_name), &change_ref))
            .await
            .map_err(|exc| {
                ApiError::internal(format!(
                    "native repository review resolve worker failed: {exc}"
                ))
            })?
            .map_err(ApiError::bad_request)?;
    let change_ref = resolved_change
        .get("change_ref")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ApiError::internal("resolved change is missing change_ref"))?
        .to_string();
    run_workflow_call("native review list", move || {
        workflow.list_reviews(&change_ref)
    })
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

pub(in super::super) async fn native_put_attestation(
    State(state): State<ServerState>,
    Path(patchset_id): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native attestation put",
        state.runtime_service.clone(),
        move || workflow.put_attestation(&patchset_id, &payload),
    )
    .await
}

pub(in super::super) async fn native_get_attestation(
    State(state): State<ServerState>,
    Path(patchset_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native attestation read", move || {
        workflow.get_attestation(&patchset_id)
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

pub(in super::super) async fn native_get_policy(
    State(state): State<ServerState>,
    Path(patchset_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native policy read", move || {
        workflow.get_policy(&patchset_id)
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

pub(in super::super) async fn native_patchset_action(
    State(state): State<ServerState>,
    Path(patchset_tail): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    if let Ok(patchset_id) = parse_suffixed_tail(&patchset_tail, ":runCi", "patchset action") {
        let runtime = state.runtime_service.clone();
        return run_workflow_mutation(
            "native patchset runCi",
            state.runtime_service.clone(),
            move || runtime.run_patchset_ci(&patchset_id, &payload),
        )
        .await;
    }
    if let Ok(patchset_id) =
        parse_suffixed_tail(&patchset_tail, ":evaluatePolicy", "patchset action")
    {
        let workflow = state.workflow_service.clone();
        return run_workflow_mutation(
            "native patchset policy",
            state.runtime_service.clone(),
            move || workflow.evaluate_policy(&patchset_id),
        )
        .await;
    }
    Err(ApiError::not_found("unknown patchset action route"))
}

pub(in super::super) async fn native_repository_patchset_action(
    State(state): State<ServerState>,
    Path((repo_name, patchset_tail)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    if let Ok(patchset_id) = parse_suffixed_tail(&patchset_tail, ":runCi", "patchset action") {
        let runtime = state.runtime_service.clone();
        return run_workflow_mutation(
            "native repository patchset runCi",
            state.runtime_service.clone(),
            move || runtime.run_repository_patchset_ci(&repo_name, &patchset_id, &payload),
        )
        .await;
    }
    Err(ApiError::not_found(
        "unknown repository patchset action route",
    ))
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

pub(in super::super) async fn native_get_land(
    State(state): State<ServerState>,
    Path(submission_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native land read", move || {
        workflow.get_land(None, &submission_id)
    })
    .await
}

pub(in super::super) async fn native_get_repository_land(
    State(state): State<ServerState>,
    Path((repo_name, submission_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native land read", move || {
        workflow.get_land(Some(&repo_name), &submission_id)
    })
    .await
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
