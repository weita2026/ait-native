use super::*;

pub(super) async fn run_repository_call<T, F>(callback: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, NativeRepositoryError> + Send + 'static,
{
    task::spawn_blocking(callback)
        .await
        .map_err(|error| ApiError::internal(format!("numeric Repository worker failed: {error}")))?
        .map_err(map_native_repository_error)
}

fn parse_json_body<T: DeserializeOwned>(bytes: &[u8], context: &str) -> Result<T, ApiError> {
    serde_json::from_slice(bytes)
        .map_err(|error| ApiError::bad_request(format!("{context} must be valid JSON: {error}")))
}

fn parse_line_name(line_tail: &str) -> Result<String, ApiError> {
    let line_name = line_tail.trim_matches('/');
    if line_name.is_empty() {
        return Err(ApiError::bad_request("line_name path segment is required."));
    }
    Ok(line_name.to_string())
}

fn parse_close_line_name(line_tail: &str) -> Result<String, ApiError> {
    let line_name = line_tail
        .strip_suffix(":close")
        .ok_or_else(|| ApiError::not_found("unknown Line route"))?;
    parse_line_name(line_name)
}

fn parse_snapshot_tail(snapshot_tail: &str) -> Result<String, ApiError> {
    let snapshot_id = snapshot_tail.trim_matches('/');
    if snapshot_id.is_empty() {
        return Err(ApiError::bad_request(
            "snapshot_id path segment is required.",
        ));
    }
    Ok(snapshot_id.to_string())
}

pub(super) async fn native_list_repository_lines(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let service = state.repository_service.clone();
    let payload = run_repository_call(move || service.list_lines(&repository_index)).await?;
    Ok(ok_json(payload))
}

pub(super) async fn native_get_repository_line(
    State(state): State<ServerState>,
    Path((repository_index, line_tail)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let line_name = parse_line_name(&line_tail)?;
    let service = state.repository_service.clone();
    let payload =
        run_repository_call(move || service.get_line(&repository_index, &line_name)).await?;
    Ok(ok_json(payload))
}

pub(super) async fn native_put_repository_line(
    State(state): State<ServerState>,
    Path((repository_index, line_tail)): Path<(String, String)>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let line_name = parse_line_name(&line_tail)?;
    let request = parse_json_body::<LineUpdateRequest>(&body, "Line update payload")?;
    let service = state.repository_service.clone();
    let payload = run_repository_call(move || {
        let repository = service.get_repository(&repository_index)?;
        let default_line = repository
            .get("default_line")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                NativeRepositoryError::internal(format!(
                    "Repository {repository_index} is missing default_line authority."
                ))
            })?;
        if line_name == default_line {
            let current_line = service.get_line(&repository_index, &line_name)?;
            require_remote_sync_line_update_authority(
                default_line,
                &line_name,
                current_line
                    .get("head_snapshot_id")
                    .and_then(JsonValue::as_str),
                request.head_snapshot_id.as_deref(),
            )?;
        }
        service.update_line(&repository_index, &line_name, request)
    })
    .await?;
    Ok(ok_json(payload))
}

pub(super) async fn native_post_repository_line(
    State(state): State<ServerState>,
    Path((repository_index, line_tail)): Path<(String, String)>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let line_name = parse_close_line_name(&line_tail)?;
    let request = parse_json_body::<LineCloseRequest>(&body, "Line close payload")?;
    let service = state.repository_service.clone();
    let payload =
        run_repository_call(move || service.close_line(&repository_index, &line_name, request))
            .await?;
    Ok(ok_json(payload))
}

pub(super) async fn native_repository_authority_zstd_bulk_plan(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(request): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.plan_repository_zstd_bulk(&repository_index, &request)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository zstd plan worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_repository_authority_zstd_bulk_commit(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(request): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.commit_repository_zstd_bulk(&repository_index, &request)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository zstd commit worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_repository_authority_get_zstd_import_manifest(
    State(state): State<ServerState>,
    Path((repository_index, snapshot_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.get_repository_zstd_import_manifest(&repository_index, &snapshot_id)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository zstd manifest worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_repository_authority_get_zstd_pull_manifest(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(request): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.get_repository_zstd_pull_manifest(&repository_index, &request)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository zstd pull-manifest worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_repository_authority_put_zstd_bulk_object_pack(
    State(state): State<ServerState>,
    Path((repository_index, pack_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    validate_zstd_bulk_content_type(
        &headers,
        ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE,
        "zstd object-pack upload",
    )?;
    if body.is_empty() {
        return Err(ApiError::bad_request("zstd object-pack body is empty"));
    }
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.put_repository_zstd_object_pack(&repository_index, &pack_id, body.to_vec())
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository object-pack worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_repository_authority_get_zstd_bulk_object_pack(
    State(state): State<ServerState>,
    Path((repository_index, pack_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let runtime = state.runtime_service.clone();
    let pack_bytes = task::spawn_blocking(move || {
        runtime.get_repository_zstd_object_pack(&repository_index, &pack_id)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository object-pack worker failed: {error}"
        ))
    })?
    .map_err(map_runtime_error)?;
    Ok((
        StatusCode::OK,
        [("content-type", ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE)],
        pack_bytes,
    )
        .into_response())
}

pub(super) async fn native_repository_authority_put_zstd_bulk_tree_pack(
    State(state): State<ServerState>,
    Path((repository_index, pack_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    validate_zstd_bulk_content_type(
        &headers,
        ZSTD_BULK_TREE_PACK_MEDIA_TYPE,
        "zstd tree-pack upload",
    )?;
    if body.is_empty() {
        return Err(ApiError::bad_request("zstd tree-pack body is empty"));
    }
    let runtime = state.runtime_service.clone();
    let result = task::spawn_blocking(move || {
        runtime.put_repository_zstd_tree_pack(&repository_index, &pack_id, body.to_vec())
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository tree-pack worker failed: {error}"
        ))
    })?;
    map_json_result(result)
}

pub(super) async fn native_repository_authority_get_zstd_bulk_tree_pack(
    State(state): State<ServerState>,
    Path((repository_index, pack_id)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let runtime = state.runtime_service.clone();
    let pack_bytes = task::spawn_blocking(move || {
        runtime.get_repository_zstd_tree_pack(&repository_index, &pack_id)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "numeric Repository tree-pack worker failed: {error}"
        ))
    })?
    .map_err(map_runtime_error)?;
    Ok((
        StatusCode::OK,
        [("content-type", ZSTD_BULK_TREE_PACK_MEDIA_TYPE)],
        pack_bytes,
    )
        .into_response())
}

fn validate_zstd_bulk_content_type(
    headers: &HeaderMap,
    expected: &str,
    label: &str,
) -> Result<(), ApiError> {
    let actual = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(';').next().unwrap_or_default().trim())
        .unwrap_or("");
    if actual.eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    let message = if actual.is_empty() {
        format!("{label} content type must be `{expected}`")
    } else {
        format!("{label} content type `{actual}` is unsupported; expected `{expected}`")
    };
    Err(ApiError::bad_request(message))
}

pub(super) async fn native_get_snapshot_tail(
    State(state): State<ServerState>,
    Path((repository_index, snapshot_tail)): Path<(String, String)>,
    Query(query): Query<SnapshotExportQuery>,
) -> Result<Response, ApiError> {
    let service = state.repository_service.clone();
    let snapshot_id = parse_snapshot_tail(&snapshot_tail)?;
    let payload = run_repository_call(move || {
        service.export_snapshot(&repository_index, &snapshot_id, query)
    })
    .await?;
    Ok(ok_json(payload).into_response())
}
