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
    RawBody(body): RawBody,
) -> Result<impl IntoResponse, ApiError> {
    let payload = stream_zstd_bulk_pack_upload(
        state,
        repository_index,
        pack_id,
        headers,
        body,
        NativeZstdPackKind::Object,
        ZSTD_BULK_OBJECT_PACK_MEDIA_TYPE,
    )
    .await?;
    Ok(ok_json(payload))
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
    RawBody(body): RawBody,
) -> Result<impl IntoResponse, ApiError> {
    let payload = stream_zstd_bulk_pack_upload(
        state,
        repository_index,
        pack_id,
        headers,
        body,
        NativeZstdPackKind::Tree,
        ZSTD_BULK_TREE_PACK_MEDIA_TYPE,
    )
    .await?;
    Ok(ok_json(payload))
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

fn declared_zstd_pack_content_length(headers: &HeaderMap) -> Result<Option<u64>, ApiError> {
    let Some(value) = headers.get(CONTENT_LENGTH) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::bad_request("zstd Pack Content-Length is not valid ASCII"))?;
    let bytes = value
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("zstd Pack Content-Length is not a valid integer"))?;
    if bytes > NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES as u64 {
        return Err(ApiError::payload_too_large(format!(
            "zstd Pack body exceeds the {} byte limit",
            NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES
        )));
    }
    Ok(Some(bytes))
}

pub(super) async fn stream_zstd_pack_body(
    body: &mut AxumBody,
    file: &mut tokio::fs::File,
    limit_bytes: u64,
    label: &str,
) -> Result<(u64, String), ApiError> {
    let mut payload_bytes = 0_u64;
    let mut payload_sha256 = Sha256::new();
    while let Some(chunk) = body.data().await {
        let chunk = chunk.map_err(|error| {
            ApiError::bad_request(format!("failed to receive zstd {label} body: {error}"))
        })?;
        payload_bytes = payload_bytes
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| ApiError::payload_too_large("zstd Pack body length overflow"))?;
        if payload_bytes > limit_bytes {
            return Err(ApiError::payload_too_large(format!(
                "zstd {label} body exceeds the {limit_bytes} byte limit"
            )));
        }
        file.write_all(&chunk).await.map_err(|error| {
            ApiError::internal(format!("failed to stage zstd {label} body: {error}"))
        })?;
        payload_sha256.update(&chunk);
    }
    Ok((payload_bytes, format!("{:x}", payload_sha256.finalize())))
}

async fn stream_zstd_bulk_pack_upload(
    state: ServerState,
    repository_index: String,
    pack_id: String,
    headers: HeaderMap,
    mut body: AxumBody,
    kind: NativeZstdPackKind,
    expected_media_type: &str,
) -> Result<JsonValue, ApiError> {
    let label = format!("{}-pack", kind.label());
    validate_zstd_bulk_content_type(
        &headers,
        expected_media_type,
        &format!("zstd {label} upload"),
    )?;
    let declared_bytes = declared_zstd_pack_content_length(&headers)?;
    if declared_bytes == Some(0) {
        return Err(ApiError::bad_request(format!("zstd {label} body is empty")));
    }
    let _permit = state
        .zstd_pack_upload_admission
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::service_unavailable(
                "zstd Pack upload capacity is exhausted; retry the request",
            )
        })?;

    let service = state.repository_service.clone();
    let begin_repository_index = repository_index.clone();
    let begin_pack_id = pack_id.clone();
    let mut upload = run_repository_call(move || {
        service.begin_zstd_bulk_pack_upload(&begin_repository_index, &begin_pack_id, kind)
    })
    .await?;
    let staged_file = upload.take_file().map_err(map_native_repository_error)?;
    let mut staged_file = tokio::fs::File::from_std(staged_file);
    let (payload_bytes, payload_sha256) = stream_zstd_pack_body(
        &mut body,
        &mut staged_file,
        NATIVE_ZSTD_BULK_BODY_LIMIT_BYTES as u64,
        &label,
    )
    .await?;
    if payload_bytes == 0 {
        return Err(ApiError::bad_request(format!("zstd {label} body is empty")));
    }
    if declared_bytes.is_some_and(|expected| expected != payload_bytes) {
        return Err(ApiError::bad_request(format!(
            "zstd {label} body length does not match Content-Length"
        )));
    }
    staged_file.sync_all().await.map_err(|error| {
        ApiError::internal(format!("failed to sync staged zstd {label}: {error}"))
    })?;
    drop(staged_file);

    let service = state.repository_service.clone();
    run_repository_call(move || {
        service.finish_zstd_bulk_pack_upload(
            &repository_index,
            upload,
            payload_bytes,
            &payload_sha256,
        )
    })
    .await
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
