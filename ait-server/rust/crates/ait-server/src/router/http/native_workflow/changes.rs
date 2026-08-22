use super::*;

pub(in super::super) async fn native_create_change(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_mutation(
        "native change create",
        state.runtime_service.clone(),
        move || {
            let (repo_name, workflow) =
                repository_authority_workflow_store(routed_workflow.as_ref(), &repository_index)?;
            workflow.create_change(&repo_name, &payload)
        },
    )
    .await
}

pub(in super::super) async fn native_list_repository_authority_changes(
    State(state): State<ServerState>,
    Path(repo_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority change list", move || {
        let (repo_name, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        workflow.list_changes(&repo_name)
    })
    .await
}

pub(in super::super) async fn native_get_repository_authority_change(
    State(state): State<ServerState>,
    Path((repo_id, change_ref)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let routed_workflow = state.workflow_service.clone();
    run_workflow_call("native repository authority change read", move || {
        let (_, workflow) =
            repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
        workflow.get_change(None, &change_ref)
    })
    .await
}

pub(in super::super) async fn native_repository_authority_change_action(
    State(state): State<ServerState>,
    Path((repo_id, change_tail)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let action = if change_tail.ends_with(":close") {
        "close"
    } else if change_tail.ends_with(":selectPatchset") {
        "selectPatchset"
    } else if change_tail.ends_with(":requestReview") {
        "requestReview"
    } else if change_tail.ends_with(":submit") {
        "submit"
    } else {
        return Err(ApiError::not_found(
            "unknown repository authority change action route",
        ));
    };
    let suffix = format!(":{action}");
    let change_ref = parse_suffixed_tail(&change_tail, &suffix, "repository change action")?;
    let routed_workflow = state.workflow_service.clone();
    let runtime = state.runtime_service.clone();
    run_workflow_mutation(
        "native repository authority change action",
        runtime.clone(),
        move || {
            let (_, workflow) =
                repository_authority_workflow_store(routed_workflow.as_ref(), &repo_id)?;
            let resolved_change = workflow.get_change(None, &change_ref)?;
            let change_ref = resolved_change
                .get("change_ref")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| "resolved change is missing change_ref".to_string())?
                .to_string();
            match action {
                "close" => workflow.close_change(&change_ref, &payload),
                "selectPatchset" => workflow.select_patchset(&change_ref, &payload),
                "requestReview" => workflow.request_review(&change_ref, &payload),
                "submit" => {
                    let mut land = workflow.submit_land(&change_ref, &payload)?;
                    runtime.complete_post_land_delivery(&mut land);
                    Ok(land)
                }
                _ => unreachable!("validated repository authority change action"),
            }
        },
    )
    .await
}
