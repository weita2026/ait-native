use super::*;

pub(super) fn ok_json(value: JsonValue) -> (StatusCode, Json<JsonValue>) {
    (StatusCode::OK, Json(value))
}

pub(super) fn map_json_result(
    result: Result<JsonValue, String>,
) -> Result<(StatusCode, Json<JsonValue>), ApiError> {
    result.map(ok_json).map_err(map_runtime_error)
}

pub(super) fn map_runtime_error(message: String) -> ApiError {
    let native_repository_error = NativeRepositoryError::from_wrapped_string(message.clone());
    if native_repository_error.kind == NativeRepositoryErrorKind::Conflict {
        return map_native_repository_error(native_repository_error);
    }
    if message.starts_with("Unknown plan: ") {
        return ApiError::not_found(message);
    }
    match binary_db_runtime_error_kind(&message) {
        Some(BinaryDbErrorKind::RetryableBusy) => ApiError::service_unavailable(message),
        Some(BinaryDbErrorKind::MissingData) => ApiError::not_found(message),
        Some(BinaryDbErrorKind::InvalidDomainData) => ApiError::bad_request(message),
        Some(
            BinaryDbErrorKind::Corruption
            | BinaryDbErrorKind::LayoutMismatch
            | BinaryDbErrorKind::Io
            | BinaryDbErrorKind::Unsupported
            | BinaryDbErrorKind::Other,
        ) => ApiError::internal(message),
        None => ApiError::bad_request(message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ait_server_core::foundation::remote_binary_db::{binary_db_runtime_error, BinaryDbError};

    #[test]
    fn retryable_binary_db_runtime_error_maps_to_503_with_retry_after() {
        let message = binary_db_runtime_error("Plan read", BinaryDbError::retryable_busy("busy"));
        let response = map_runtime_error(message).into_response();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            response.headers().get(RETRY_AFTER),
            Some(&HeaderValue::from_static("1"))
        );
    }

    #[test]
    fn canonical_unknown_plan_error_maps_to_404() {
        assert_eq!(
            map_runtime_error("Unknown plan: PR-1".to_string())
                .into_response()
                .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn non_retryable_binary_db_runtime_error_preserves_legacy_bad_request_mapping() {
        let message = binary_db_runtime_error(
            "Plan read",
            BinaryDbError::invalid_domain_data("invalid plan"),
        );
        assert_eq!(message, "Plan read: invalid plan");
        assert!(binary_db_runtime_error_kind(&message).is_none());
        assert_eq!(
            map_runtime_error(message).into_response().status(),
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn wrapped_native_repository_conflict_preserves_conflict_status() {
        let response = map_runtime_error(
            NativeRepositoryError::conflict("GOVERNED_TARGET_LINE_REQUIRES_LAND").to_string(),
        )
        .into_response();
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
}
