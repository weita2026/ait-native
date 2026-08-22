use super::*;

pub(in super::super) async fn native_prepare_repository_authority_history_promotion(
    State(state): State<ServerState>,
    Path((repo_id, action)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    if action != ":prepare" {
        return Err(ApiError::not_found(format!(
            "unknown repository-authority history-promotion action {action:?}"
        )));
    }
    let routed = state.workflow_service.clone();
    let runtime = state.runtime_service.clone();
    run_workflow_mutation(
        "native repository authority history promotion prepare",
        runtime,
        move || {
            let (repo_name, workflow) =
                repository_authority_workflow_store(routed.as_ref(), &repo_id)?;
            workflow.prepare_history_promotion(&repo_name, &payload)
        },
    )
    .await
}
