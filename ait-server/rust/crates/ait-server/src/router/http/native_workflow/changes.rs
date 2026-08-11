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

pub(in super::super) async fn native_list_changes(
    State(state): State<ServerState>,
    Path(repo_name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native change list", move || {
        workflow.list_changes(&repo_name)
    })
    .await
}

pub(in super::super) async fn native_get_change(
    State(state): State<ServerState>,
    Path(change_ref): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native change read", move || {
        workflow.get_change(None, &change_ref)
    })
    .await
}

pub(in super::super) async fn native_get_repository_change(
    State(state): State<ServerState>,
    Path((repo_name, change_ref)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
    run_workflow_call("native repository change read", move || {
        workflow.get_change(Some(&repo_name), &change_ref)
    })
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

pub(in super::super) async fn native_read_change_detail_global(
    State(state): State<ServerState>,
    Path(change_ref): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    read_change_detail_response(state, None, change_ref).await
}

pub(in super::super) async fn native_read_repository_change_detail(
    State(state): State<ServerState>,
    Path((repo_name, change_ref)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    read_change_detail_response(state, Some(repo_name), change_ref).await
}

async fn read_change_detail_response(
    state: ServerState,
    repo_name: Option<String>,
    change_ref: String,
) -> Result<(StatusCode, Json<JsonValue>), ApiError> {
    let base_workflow = state.workflow_service.clone();
    let base_repositories = state.repository_service.clone();
    let base_repo_name = repo_name.clone();
    let base = task::spawn_blocking(move || {
        read_change_detail_base_json(
            base_workflow.as_ref(),
            base_repositories.as_ref(),
            base_repo_name.as_deref(),
            &change_ref,
        )
    })
    .await
    .map_err(|exc| ApiError::internal(format!("native change detail worker failed: {exc}")))?;
    let base = match base {
        Ok(base) => base,
        Err(error) => return map_json_result(Err(error)),
    };

    let ci_runtime = state.runtime_service.clone();
    let ci_base = base.clone();
    let ci_worker = task::spawn_blocking(move || {
        read_change_detail_ci_status_json(ci_runtime.as_ref(), &ci_base)
    });

    let projection_workflow = state.workflow_service.clone();
    let projection_repositories = state.repository_service.clone();
    let projection_base = base.clone();
    let projection_worker = task::spawn_blocking(move || {
        read_change_detail_repository_projection_json(
            projection_workflow.as_ref(),
            projection_repositories.as_ref(),
            &projection_base,
        )
    });

    let (ci_result, projection_result) = tokio::join!(ci_worker, projection_worker);
    let patchset_ci_status = ci_result.map_err(|exc| {
        ApiError::internal(format!(
            "native change detail CI projection worker failed: {exc}"
        ))
    })?;
    let projection_result = projection_result.map_err(|exc| {
        ApiError::internal(format!(
            "native change detail repository projection worker failed: {exc}"
        ))
    })?;
    let (delta, base_diff, freshness) = match projection_result {
        Ok(result) => result,
        Err(error) => return map_json_result(Err(error)),
    };
    map_json_result(complete_change_detail_json(
        base,
        patchset_ci_status,
        delta,
        base_diff,
        freshness,
    ))
}

pub(in super::super) async fn native_change_action(
    State(state): State<ServerState>,
    Path(change_tail): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    if let Ok(change_id) = parse_suffixed_tail(&change_tail, ":close", "change action") {
        let workflow = state.workflow_service.clone();
        return run_workflow_mutation(
            "native change close",
            state.runtime_service.clone(),
            move || workflow.close_change(&change_id, &payload),
        )
        .await;
    }
    if let Ok(change_id) = parse_suffixed_tail(&change_tail, ":selectPatchset", "change action") {
        let workflow = state.workflow_service.clone();
        return run_workflow_mutation(
            "native patchset select",
            state.runtime_service.clone(),
            move || workflow.select_patchset(&change_id, &payload),
        )
        .await;
    }
    if let Ok(change_id) = parse_suffixed_tail(&change_tail, ":requestReview", "change action") {
        let workflow = state.workflow_service.clone();
        return run_workflow_mutation(
            "native review request",
            state.runtime_service.clone(),
            move || workflow.request_review(&change_id, &payload),
        )
        .await;
    }
    if let Ok(change_id) = parse_suffixed_tail(&change_tail, ":submit", "change action") {
        let workflow = state.workflow_service.clone();
        let runtime = state.runtime_service.clone();
        return run_workflow_mutation("native land submit", runtime.clone(), move || {
            let mut land = workflow.submit_land(&change_id, &payload)?;
            runtime.complete_post_land_delivery(&mut land);
            Ok(land)
        })
        .await;
    }
    Err(ApiError::not_found("unknown change action route"))
}

pub(in super::super) async fn native_repository_change_action(
    State(state): State<ServerState>,
    Path((repo_name, change_tail)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let workflow = state.workflow_service.clone();
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
            "unknown repository change action route",
        ));
    };
    let suffix = format!(":{action}");
    let change_ref = parse_suffixed_tail(&change_tail, &suffix, "repository change action")?;
    let resolve_workflow = workflow.clone();
    let resolved_change =
        task::spawn_blocking(move || resolve_workflow.get_change(Some(&repo_name), &change_ref))
            .await
            .map_err(|exc| {
                ApiError::internal(format!(
                    "native repository change resolve worker failed: {exc}"
                ))
            })?
            .map_err(ApiError::bad_request)?;
    let change_ref = resolved_change
        .get("change_ref")
        .and_then(JsonValue::as_str)
        .ok_or_else(|| ApiError::internal("resolved change is missing change_ref"))?
        .to_string();
    match action {
        "close" => {
            let workflow = workflow.clone();
            run_workflow_mutation(
                "native change close",
                state.runtime_service.clone(),
                move || workflow.close_change(&change_ref, &payload),
            )
            .await
        }
        "selectPatchset" => {
            let workflow = workflow.clone();
            run_workflow_mutation(
                "native patchset select",
                state.runtime_service.clone(),
                move || workflow.select_patchset(&change_ref, &payload),
            )
            .await
        }
        "requestReview" => {
            let workflow = workflow.clone();
            run_workflow_mutation(
                "native review request",
                state.runtime_service.clone(),
                move || workflow.request_review(&change_ref, &payload),
            )
            .await
        }
        "submit" => {
            let workflow = workflow.clone();
            let runtime = state.runtime_service.clone();
            run_workflow_mutation("native land submit", runtime.clone(), move || {
                let mut land = workflow.submit_land(&change_ref, &payload)?;
                runtime.complete_post_land_delivery(&mut land);
                Ok(land)
            })
            .await
        }
        _ => Err(ApiError::not_found(
            "unknown repository change action route",
        )),
    }
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
