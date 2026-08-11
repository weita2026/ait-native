use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct NativeQueueReadQuery {
    status: Option<String>,
}

pub(super) async fn native_read_repository_queue_summary(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Query(query): Query<NativeQueueReadQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.read_repository_queue_summary(&repository_index, query.status.as_deref())
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository queue-summary worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_read_repository_task_queue(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Query(query): Query<NativeQueueReadQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.read_repository_task_queue(&repository_index, query.status.as_deref())
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository task-queue worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_read_repository_reviewer_inbox(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result =
        task::spawn_blocking(move || runtime.read_repository_reviewer_inbox(&repository_index))
            .await
            .map_err(|error| {
                ApiError::internal(format!(
                    "numeric Repository reviewer-inbox worker failed: {error}"
                ))
            })?;
    map_json_result(result)
}
