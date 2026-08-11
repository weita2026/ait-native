use super::*;
use crate::runtime_service::plan_linkage::{
    normalize_plan_contains_query, read_plan_candidate_inputs_with_runtime,
};

#[derive(Debug, Deserialize)]
pub(super) struct NativeSprintsQuery {
    artifact_path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct NativePlanCandidateInputsQuery {
    contains: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct NativePatchsetCiStatusQuery {
    recent_limit: Option<i64>,
    projection: Option<NativePatchsetCiStatusProjection>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NativePatchsetCiStatusProjection {
    Readiness,
}

impl NativePatchsetCiStatusProjection {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Readiness => "readiness",
        }
    }
}

pub(super) async fn native_read_repository_authority_patchset_ci_status(
    State(state): State<ServerState>,
    Path((repository_index, patchset_id)): Path<(String, String)>,
    Query(query): Query<NativePatchsetCiStatusQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.read_repository_authority_patchset_ci_status(
            &repository_index,
            &patchset_id,
            query.recent_limit.unwrap_or(10),
            query
                .projection
                .as_ref()
                .map(NativePatchsetCiStatusProjection::as_str),
        )
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository Patchset CI-status worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_list_repository_plans(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Query(query): Query<NativeSprintsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let artifact_path = query.artifact_path;
    let result = task::spawn_blocking(move || {
        runtime.list_repository_plans(&repository_index, artifact_path.as_deref())
    })
    .await
    .map_err(|error| ApiError::internal(format!("numeric Plan list worker failed: {error}")))?;
    map_json_result(result)
}

pub(super) async fn native_create_repository_plan(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result =
        task::spawn_blocking(move || runtime.create_repository_plan(&repository_index, &payload))
            .await
            .map_err(|error| {
                ApiError::internal(format!("numeric Plan create worker failed: {error}"))
            })?;
    map_json_result(result)
}

pub(super) async fn native_get_repository_plan(
    State(state): State<ServerState>,
    Path((repository_index, plan_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result =
        task::spawn_blocking(move || runtime.get_repository_plan(&repository_index, &plan_id))
            .await
            .map_err(|error| {
                ApiError::internal(format!("numeric Plan read worker failed: {error}"))
            })?;
    map_json_result(result)
}

pub(super) async fn native_update_repository_plan_status(
    State(state): State<ServerState>,
    Path((repository_index, plan_id)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.update_repository_plan_status(&repository_index, &plan_id, &payload)
    })
    .await
    .map_err(|error| ApiError::internal(format!("numeric Plan status worker failed: {error}")))?;
    map_json_result(result)
}

pub(super) async fn native_list_repository_plan_revisions(
    State(state): State<ServerState>,
    Path((repository_index, plan_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.list_repository_plan_revisions(&repository_index, &plan_id)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!("numeric Plan revision list worker failed: {error}"))
    })?;
    map_json_result(result)
}

pub(super) async fn native_revise_repository_plan(
    State(state): State<ServerState>,
    Path((repository_index, plan_id)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.revise_repository_plan(&repository_index, &plan_id, &payload)
    })
    .await
    .map_err(|error| ApiError::internal(format!("numeric Plan revise worker failed: {error}")))?;
    map_json_result(result)
}

pub(super) async fn native_get_repository_plan_revision(
    State(state): State<ServerState>,
    Path((repository_index, plan_id, plan_revision_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.get_repository_plan_revision(&repository_index, &plan_id, &plan_revision_id)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!("numeric Plan revision read worker failed: {error}"))
    })?;
    map_json_result(result)
}

pub(super) async fn native_put_repository_plan_revision_artifacts(
    State(state): State<ServerState>,
    Path((repository_index, plan_id, plan_revision_id)): Path<(String, String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.put_repository_plan_revision_artifacts(
            &repository_index,
            &plan_id,
            &plan_revision_id,
            &payload,
        )
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Plan revision artifact worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_resolve_task_plan_linkage(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.resolve_task_plan_linkage(&repository_index, &payload)
    })
    .await
    .map_err(|error| ApiError::internal(format!("numeric Plan linkage worker failed: {error}")))?;
    map_json_result(result)
}

pub(super) async fn native_find_plan_ids_by_contains(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.list_plan_ids_matching_contains(&repository_index, &payload)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Plan contains-query worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_read_plan_candidate_inputs(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Query(query): Query<NativePlanCandidateInputsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let contains_terms = normalize_plan_contains_query(query.contains.as_deref());
    let runtime = state.runtime_service.clone();
    let workflow = state.workflow_service.clone();
    let result = task::spawn_blocking(move || {
        read_plan_candidate_inputs_with_runtime(
            runtime.as_ref(),
            workflow.as_ref(),
            &repository_index,
            &contains_terms,
        )
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Plan candidate-input worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}
