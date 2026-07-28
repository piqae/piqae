use crate::repository::RepositoryError;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use ulid::Ulid;

#[derive(Debug)]
pub struct AppError {
    status: StatusCode,
    code: &'static str,
    message: String,
    retryable: bool,
    compatibility: bool,
}

#[derive(Debug, Serialize)]
struct NativeEnvelope {
    error: NativeError,
}

#[derive(Debug, Serialize)]
struct NativeError {
    code: &'static str,
    message: String,
    request_id: String,
    retryable: bool,
    details: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct CompatibilityError {
    uid: String,
    code: &'static str,
    message: String,
}

impl AppError {
    pub fn invalid(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code,
            message: message.into(),
            retryable: false,
            compatibility: false,
        }
    }

    #[must_use]
    pub fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code: "invalid_api_key",
            message: "Authentication failed.".into(),
            retryable: false,
            compatibility: false,
        }
    }

    #[must_use]
    pub fn compatibility_unauthorized() -> Self {
        Self {
            compatibility: true,
            ..Self::unauthorized()
        }
    }

    #[must_use]
    pub const fn compatibility(mut self) -> Self {
        self.compatibility = true;
        self
    }
}

impl From<RepositoryError> for AppError {
    fn from(error: RepositoryError) -> Self {
        match error {
            RepositoryError::NotFound => Self {
                status: StatusCode::NOT_FOUND,
                code: "not_found",
                message: "The requested resource does not exist.".into(),
                retryable: false,
                compatibility: false,
            },
            RepositoryError::IdempotencyConflict => Self {
                status: StatusCode::CONFLICT,
                code: "idempotency_conflict",
                message: "The idempotency key was already used for a different request.".into(),
                retryable: false,
                compatibility: false,
            },
            RepositoryError::ConcurrentStateChange | RepositoryError::InvalidTransition => Self {
                status: StatusCode::CONFLICT,
                code: "invalid_job_state",
                message: "The job state no longer permits this operation.".into(),
                retryable: false,
                compatibility: false,
            },
            RepositoryError::Persistence(_) => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                code: "service_unavailable",
                message: "A required service is temporarily unavailable.".into(),
                retryable: true,
                compatibility: false,
            },
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(_: serde_json::Error) -> Self {
        Self::invalid("invalid_json", "The request JSON is invalid.")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let request_id = format!("req_{}", Ulid::new());
        if self.compatibility {
            return (
                self.status,
                Json(CompatibilityError {
                    uid: request_id,
                    code: self.code,
                    message: self.message,
                }),
            )
                .into_response();
        }
        (
            self.status,
            Json(NativeEnvelope {
                error: NativeError {
                    code: self.code,
                    message: self.message,
                    request_id,
                    retryable: self.retryable,
                    details: serde_json::json!({}),
                },
            }),
        )
            .into_response()
    }
}
