use super::*;
use crate::repository_retirement::REMOTE_AUTHORITY_FILE_MEDIA_TYPE;
use ait_server_core::foundation::remote_binary_db::{BinaryDbError, BinaryDbErrorKind};

#[derive(Debug, Deserialize)]
pub(super) struct OperationalWorkerJobsQuery {
    state_kind: Option<u8>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OperationalRepositoriesQuery {
    repository_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OperationalRepositoryRegistration {
    repository_name: String,
    namespace: String,
    policy_flags: u8,
}

pub(super) async fn native_operational_capabilities(
    State(state): State<ServerState>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = require_operational_runtime(&state)?;
    Ok(ok_json(runtime.capabilities()))
}

pub(super) async fn native_operational_repository_authority(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || runtime.repository_authority(repository_index))
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "Binary Repository authority worker failed: {error}"
            ))
        })?;
    map_operational_result(result)
}

pub(super) async fn native_operational_repository_authorities(
    State(state): State<ServerState>,
    Query(query): Query<OperationalRepositoriesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || {
        runtime.repository_authorities(query.repository_name.as_deref())
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "Binary Repository discovery worker failed: {error}"
        ))
    })?;
    map_operational_result(result)
}

pub(super) async fn native_register_operational_repository_authority(
    State(state): State<ServerState>,
    Json(request): Json<OperationalRepositoryRegistration>,
) -> Result<impl IntoResponse, ApiError> {
    let namespace_ascii = parse_namespace_ascii(&request.namespace)?;
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || {
        runtime.register_repository(
            &request.repository_name,
            namespace_ascii,
            request.policy_flags,
        )
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "Binary Repository registration worker failed: {error}"
        ))
    })?;
    match result {
        Ok(payload) => {
            let status = if payload["created"].as_bool() == Some(true) {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            Ok((status, Json(payload)))
        }
        Err(error)
            if error.kind() == BinaryDbErrorKind::InvalidDomainData
                && error.contains("already owned") =>
        {
            Err(ApiError::conflict(error.to_string()))
        }
        Err(error) => Err(map_operational_error(error)),
    }
}

pub(super) async fn native_begin_repository_retirement(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let runtime = require_operational_runtime(&state)?;
    let result =
        task::spawn_blocking(move || runtime.begin_repository_retirement(repository_index))
            .await
            .map_err(|error| {
                ApiError::internal(format!(
                    "Binary Repository retirement worker failed: {error}"
                ))
            })?;
    map_lifecycle_result(result)
}

pub(super) async fn native_repository_retirement(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || runtime.repository_retirement(repository_index))
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "Binary Repository retirement status worker failed: {error}"
            ))
        })?;
    map_lifecycle_result(result)
}

pub(super) async fn native_abort_repository_retirement(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let runtime = require_operational_runtime(&state)?;
    let result =
        task::spawn_blocking(move || runtime.abort_repository_retirement(repository_index))
            .await
            .map_err(|error| {
                ApiError::internal(format!(
                    "Binary Repository retirement abort worker failed: {error}"
                ))
            })?;
    map_lifecycle_result(result)
}

pub(super) async fn native_repository_retirement_file(
    State(state): State<ServerState>,
    Path((repository_index, file_path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || {
        runtime.repository_retirement_file(repository_index, &file_path)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "Binary Repository retirement file worker failed: {error}"
        ))
    })?;
    let bytes = result.map_err(map_lifecycle_error)?;
    let mut response = (StatusCode::OK, Bytes::from(bytes)).into_response();
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static(REMOTE_AUTHORITY_FILE_MEDIA_TYPE),
    );
    Ok(response)
}

pub(super) async fn native_purge_retired_repository(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let runtime = require_operational_runtime(&state)?;
    let result =
        task::spawn_blocking(move || runtime.purge_retired_repository(repository_index, &payload))
            .await
            .map_err(|error| {
                ApiError::internal(format!("Binary Repository purge worker failed: {error}"))
            })?;
    map_lifecycle_result(result)
}

pub(super) async fn native_begin_repository_restore(
    State(state): State<ServerState>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || runtime.begin_repository_restore(&payload))
        .await
        .map_err(|error| {
            ApiError::internal(format!(
                "Binary Repository restore session worker failed: {error}"
            ))
        })?;
    match result {
        Ok(payload) => Ok((StatusCode::CREATED, Json(payload))),
        Err(error) => Err(map_lifecycle_error(error)),
    }
}

pub(super) async fn native_upload_repository_restore_file(
    State(state): State<ServerState>,
    Path((restore_token, file_path)): Path<(String, String)>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    if headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        != Some(REMOTE_AUTHORITY_FILE_MEDIA_TYPE)
    {
        return Err(ApiError::bad_request(format!(
            "Repository restore file Content-Type must be {REMOTE_AUTHORITY_FILE_MEDIA_TYPE}"
        )));
    }
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || {
        runtime.upload_repository_restore_file(&restore_token, &file_path, &bytes)
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "Binary Repository restore upload worker failed: {error}"
        ))
    })?;
    map_lifecycle_result(result)
}

pub(super) async fn native_commit_repository_restore(
    State(state): State<ServerState>,
    Path(restore_token): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || runtime.commit_repository_restore(&restore_token))
        .await
        .map_err(|error| {
            ApiError::internal(format!("Binary Repository restore worker failed: {error}"))
        })?;
    map_lifecycle_result(result)
}

pub(super) async fn native_operational_worker_jobs(
    State(state): State<ServerState>,
    Path(repository_index): Path<String>,
    Query(query): Query<OperationalWorkerJobsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || {
        runtime.list_worker_jobs(
            repository_index,
            query.state_kind,
            query.limit.unwrap_or(50),
        )
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!("Binary Worker Job list worker failed: {error}"))
    })?;
    map_operational_result(result)
}

pub(super) async fn native_operational_worker_job(
    State(state): State<ServerState>,
    Path((repository_index, worker_job_index)): Path<(String, String)>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let worker_job_index = parse_canonical_index(&worker_job_index, "worker_job_index")?;
    let runtime = require_operational_runtime(&state)?;
    let result =
        task::spawn_blocking(move || runtime.worker_job(repository_index, worker_job_index))
            .await
            .map_err(|error| {
                ApiError::internal(format!("Binary Worker Job read worker failed: {error}"))
            })?;
    map_operational_result(result)
}

pub(super) async fn native_operational_worker_job_action(
    State(state): State<ServerState>,
    Path((repository_index, worker_job_action)): Path<(String, String)>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let repository_index = parse_canonical_index(&repository_index, "repository_index")?;
    let (worker_job_index, operation) = parse_worker_job_action(&worker_job_action)?;
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || match operation.as_str() {
        "claim" => runtime.claim_worker_job(repository_index, worker_job_index, &payload),
        "heartbeat" => runtime.heartbeat_worker_job(repository_index, worker_job_index, &payload),
        "complete" => runtime.complete_worker_job(repository_index, worker_job_index, &payload),
        "fail" => runtime.fail_worker_job(repository_index, worker_job_index, &payload),
        _ => unreachable!("Worker Job operation was parsed from a fixed set"),
    })
    .await
    .map_err(|error| {
        ApiError::internal(format!(
            "Binary Worker Job operation worker failed: {error}"
        ))
    })?;
    map_operational_result(result)
}

pub(super) async fn native_claim_next_worker_job(
    State(state): State<ServerState>,
    Json(payload): Json<JsonValue>,
) -> Result<impl IntoResponse, ApiError> {
    let runtime = require_operational_runtime(&state)?;
    let result = task::spawn_blocking(move || runtime.claim_next_worker_job(&payload))
        .await
        .map_err(|error| {
            ApiError::internal(format!("Binary Worker Job claim worker failed: {error}"))
        })?;
    map_operational_result(result)
}

fn require_operational_runtime(
    state: &ServerState,
) -> Result<Arc<OperationalBinaryRuntime>, ApiError> {
    Ok(state.operational_binary.clone())
}

fn parse_worker_job_action(value: &str) -> Result<(u32, String), ApiError> {
    for operation in ["claim", "heartbeat", "complete", "fail"] {
        let suffix = format!(":{operation}");
        if let Some(index) = value.strip_suffix(&suffix) {
            return Ok((
                parse_canonical_index(index, "worker_job_index")?,
                operation.to_string(),
            ));
        }
    }
    Err(ApiError::not_found("unknown Binary Worker Job operation"))
}

pub(super) fn parse_canonical_index(value: &str, field: &str) -> Result<u32, ApiError> {
    if value.is_empty()
        || value.bytes().any(|byte| !byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(ApiError::bad_request(format!(
            "{field} must be canonical unsigned base-10 without leading zeroes"
        )));
    }
    value
        .parse::<u32>()
        .map_err(|_| ApiError::bad_request(format!("{field} must fit an unsigned 32-bit integer")))
}

fn parse_namespace_ascii(value: &str) -> Result<[u8; 2], ApiError> {
    let bytes = value.as_bytes();
    if bytes.len() > 2 || !value.is_ascii() {
        return Err(ApiError::bad_request(
            "namespace must contain zero, one, or two ASCII bytes",
        ));
    }
    let mut namespace = [0_u8; 2];
    namespace[..bytes.len()].copy_from_slice(bytes);
    ait_server_core::foundation::operational_binary_v0::validate_namespace(namespace)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    Ok(namespace)
}

fn map_operational_result(
    result: Result<JsonValue, BinaryDbError>,
) -> Result<(StatusCode, Json<JsonValue>), ApiError> {
    result.map(ok_json).map_err(map_operational_error)
}

pub(super) fn map_operational_error(error: BinaryDbError) -> ApiError {
    match error.kind() {
        BinaryDbErrorKind::RetryableBusy => ApiError::service_unavailable(error.to_string()),
        BinaryDbErrorKind::MissingData => ApiError::not_found(error.to_string()),
        BinaryDbErrorKind::InvalidDomainData => ApiError::bad_request(error.to_string()),
        BinaryDbErrorKind::Corruption
        | BinaryDbErrorKind::LayoutMismatch
        | BinaryDbErrorKind::Io
        | BinaryDbErrorKind::Unsupported
        | BinaryDbErrorKind::Other => ApiError::internal(error.to_string()),
    }
}

fn map_lifecycle_result(
    result: Result<JsonValue, BinaryDbError>,
) -> Result<(StatusCode, Json<JsonValue>), ApiError> {
    result.map(ok_json).map_err(map_lifecycle_error)
}

pub(super) fn map_lifecycle_error(error: BinaryDbError) -> ApiError {
    if error.kind() == BinaryDbErrorKind::InvalidDomainData
        && (error.contains("retiring")
            || error.contains("purged")
            || error.contains("drain")
            || error.contains("already owned")
            || error.contains("acknowledgement")
            || error.contains("tombstoned"))
    {
        ApiError::conflict(error.to_string())
    } else {
        map_operational_error(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_authority_segments_are_canonical() {
        assert_eq!(parse_canonical_index("0", "repository_index").unwrap(), 0);
        assert_eq!(
            parse_worker_job_action("19:heartbeat").unwrap(),
            (19, "heartbeat".to_string())
        );
        assert!(parse_canonical_index("01", "repository_index").is_err());
        assert!(parse_worker_job_action("19:delete").is_err());
    }

    #[test]
    fn repository_namespace_input_uses_exact_zero_padded_ascii() {
        assert_eq!(parse_namespace_ascii("").unwrap(), [0, 0]);
        assert_eq!(parse_namespace_ascii("R").unwrap(), [b'R', 0]);
        assert_eq!(parse_namespace_ascii("RT").unwrap(), *b"RT");
        assert!(parse_namespace_ascii("R!").is_err());
        assert!(parse_namespace_ascii("RUN").is_err());
        assert!(parse_namespace_ascii("測").is_err());
    }
}
