use super::*;

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Debug)]
pub(super) struct ApiError {
    status: StatusCode,
    message: String,
    retry_after_seconds: Option<u64>,
}

impl ApiError {
    pub(super) fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    pub(super) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    pub(super) fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    pub(super) fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            retry_after_seconds: None,
        }
    }

    pub(super) fn service_unavailable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: message.into(),
            retry_after_seconds: Some(1),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let retry_after_seconds = self.retry_after_seconds;
        let mut response = (
            self.status,
            Json(ErrorResponse {
                error: self.message,
            }),
        )
            .into_response();
        if let Some(seconds) = retry_after_seconds {
            let value = HeaderValue::from_str(&seconds.to_string())
                .expect("static retry-after seconds must be a valid header");
            response.headers_mut().insert(RETRY_AFTER, value);
        }
        response
    }
}

pub(super) fn map_native_repository_error(error: NativeRepositoryError) -> ApiError {
    match error.kind {
        NativeRepositoryErrorKind::BadRequest => ApiError::bad_request(error.message),
        NativeRepositoryErrorKind::NotFound => ApiError::not_found(error.message),
        NativeRepositoryErrorKind::Conflict => ApiError::conflict(error.message),
        NativeRepositoryErrorKind::ServiceUnavailable => {
            ApiError::service_unavailable(error.message)
        }
        NativeRepositoryErrorKind::Internal => ApiError::internal(error.message),
    }
}
