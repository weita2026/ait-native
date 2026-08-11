use super::*;

#[derive(Debug, Deserialize)]
pub(in super::super) struct NativePatchsetDeltaQuery {
    against: Option<String>,
    change_ref: Option<String>,
}

pub(in super::super) async fn native_publish_patchset(
    State(state): State<ServerState>,
    Path(change_id): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native patchset publish",
        state.runtime_service.clone(),
        move || workflow.publish_patchset(&change_id, &payload),
    )
    .await
}

pub(in super::super) async fn native_publish_repository_patchset(
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
                    "native repository patchset resolve worker failed: {exc}"
                ))
            })?
            .map_err(ApiError::bad_request)?;
    let change_ref = resolved_change
        .get("change_ref")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ApiError::internal("resolved change is missing change_ref"))?
        .to_string();
    run_workflow_mutation(
        "native patchset publish",
        state.runtime_service.clone(),
        move || workflow.publish_patchset(&change_ref, &payload),
    )
    .await
}

pub(in super::super) async fn native_publish_repository_authority_patchset(
    State(state): State<ServerState>,
    Path((repo_id, change_ref)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native repository authority patchset publish",
        state.runtime_service.clone(),
        move || {
            let (_, workflow) =
                repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
            let resolved_change = workflow.get_change(None, &change_ref)?;
            let change_ref = resolved_change
                .get("change_ref")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "resolved change is missing change_ref".to_string())?;
            workflow.publish_patchset(change_ref, &payload)
        },
    )
    .await
}

pub(in super::super) async fn native_list_patchsets(
    State(state): State<ServerState>,
    Path(change_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native patchset list", move || {
        workflow.list_patchsets(None, &change_id)
    })
    .await
}

pub(in super::super) async fn native_list_repository_patchsets(
    State(state): State<ServerState>,
    Path((repo_name, change_ref)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native patchset list", move || {
        workflow.list_patchsets(Some(&repo_name), &change_ref)
    })
    .await
}

pub(in super::super) async fn native_list_repository_authority_patchsets(
    State(state): State<ServerState>,
    Path((repo_id, change_ref)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority patchset list", move || {
        let (_, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        workflow.list_patchsets(None, &change_ref)
    })
    .await
}

pub(in super::super) async fn native_read_patchset_delta_global(
    State(state): State<ServerState>,
    Path(patchset_id): Path<String>,
    Query(query): Query<NativePatchsetDeltaQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    let repositories = state.repository_service.clone();
    run_workflow_call("native patchset delta", move || {
        read_patchset_delta_json(
            workflow.as_ref(),
            repositories.as_ref(),
            None,
            &patchset_id,
            query.against.as_deref().unwrap_or("previous"),
            None,
        )
    })
    .await
}

pub(in super::super) async fn native_read_repository_patchset_delta(
    State(state): State<ServerState>,
    Path((repo_name, patchset_ref)): Path<(String, String)>,
    Query(query): Query<NativePatchsetDeltaQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    let repositories = state.repository_service.clone();
    run_workflow_call("native repository patchset delta", move || {
        read_patchset_delta_json(
            workflow.as_ref(),
            repositories.as_ref(),
            Some(&repo_name),
            &patchset_ref,
            query.against.as_deref().unwrap_or("previous"),
            query.change_ref.as_deref(),
        )
    })
    .await
}
