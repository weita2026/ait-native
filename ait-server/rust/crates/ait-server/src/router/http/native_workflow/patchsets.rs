use super::*;

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
