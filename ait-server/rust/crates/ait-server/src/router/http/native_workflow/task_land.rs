use super::*;
use crate::router::http::{map_operational_error, ok_json, parse_canonical_index};

fn atomic_task_land_requested_ref(payload: &JsonValue) -> Result<String, String> {
    payload
        .get("task_or_change_ref")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "Atomic Task Land requires task_or_change_ref.".to_string())
}

fn submit_atomic_task_land(
    runtime: &dyn ServerRuntimeService,
    workflow: &dyn ServerWorkflowStore,
    payload: &JsonValue,
) -> Result<JsonValue, String> {
    let task_or_change_ref = atomic_task_land_requested_ref(payload)?;
    let mut result = workflow.submit_task_land(&task_or_change_ref, payload)?;
    if result.get("replayed").and_then(JsonValue::as_bool) != Some(true) {
        if let Some(land) = result.get_mut("land") {
            runtime.complete_post_land_delivery(land);
        }
    }
    Ok(result)
}

pub(in super::super) async fn native_repository_authority_task_land(
    State(state): State<ServerState>,
    Path(repo_id): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    if payload.get("contract").and_then(JsonValue::as_str)
        == Some(ait_server_core::foundation::workflow_package_contract::PACKAGE_FINISH_CONTRACT)
    {
        ait_server_core::foundation::workflow_package_contract::PackageFinishRequest::parse(
            &payload,
        )
        .map_err(ApiError::bad_request)?;
        let repository_index = parse_canonical_index(&repo_id, "repository_index")?;
        let operational = state.operational_binary.clone();
        return task::spawn_blocking(move||operational.finish_workflow_package(repository_index,&payload))
            .await.map_err(|error|ApiError::internal(format!("Package finish worker failed: {error}")))?
            .map(ok_json).map_err(|error| if error.kind()==ait_server_core::foundation::remote_binary_db::BinaryDbErrorKind::InvalidDomainData {ApiError::conflict(error.to_string())} else {map_operational_error(error)});
    }
    let routed = state.workflow_service.clone();
    let runtime = state.runtime_service.clone();
    let delivery_runtime = runtime.clone();
    run_workflow_mutation(
        "native repository authority atomic task land",
        runtime,
        move || {
            let (_, workflow) = repository_authority_workflow_store(routed.as_ref(), &repo_id)?;
            submit_atomic_task_land(delivery_runtime.as_ref(), workflow.as_ref(), &payload)
        },
    )
    .await
}
