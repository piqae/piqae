use crate::{repository::RepositoryError, request_id};
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

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

    pub fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code,
            message: message.into(),
            retryable: false,
            compatibility: false,
        }
    }

    #[must_use]
    pub fn not_found() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: "The requested resource does not exist.".into(),
            retryable: false,
            compatibility: false,
        }
    }

    pub fn payload_too_large(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::PAYLOAD_TOO_LARGE,
            code,
            message: message.into(),
            retryable: false,
            compatibility: false,
        }
    }

    #[must_use]
    pub fn billing_disabled() -> Self {
        Self {
            status: StatusCode::CONFLICT,
            code: "billing_disabled",
            message: "Cloud billing is disabled for this deployment.".into(),
            retryable: false,
            compatibility: false,
        }
    }

    #[must_use]
    pub fn too_many_requests() -> Self {
        Self {
            status: StatusCode::TOO_MANY_REQUESTS,
            code: "too_many_requests",
            message: "Too many requests. Retry shortly.".into(),
            retryable: true,
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
    pub fn forbidden() -> Self {
        Self {
            status: StatusCode::FORBIDDEN,
            code: "insufficient_scope",
            message: "The API key does not grant the required scope.".into(),
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
    pub fn service_unavailable(code: &'static str) -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code,
            message: "A required service is temporarily unavailable.".into(),
            retryable: true,
            compatibility: false,
        }
    }

    #[must_use]
    pub fn device_unauthorized(code: &'static str) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            code,
            message: "Agent request authentication failed.".into(),
            retryable: false,
            compatibility: false,
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
            RepositoryError::NodeIdentityRevisionConflict(_) => Self::conflict(
                "node_identity_revision_conflict",
                "The node identity changed; reconcile before saving.",
            ),
            RepositoryError::QuotaExceeded => Self {
                status: StatusCode::PAYMENT_REQUIRED,
                code: "quota_exceeded",
                message: "The Free plan reported-complete job quota has been reached.".into(),
                retryable: false,
                compatibility: false,
            },
            RepositoryError::BillingBlocked => Self {
                status: StatusCode::PAYMENT_REQUIRED,
                code: "billing_blocked",
                message: "Billing status does not permit new Cloud jobs.".into(),
                retryable: false,
                compatibility: false,
            },
            RepositoryError::NodeQuotaExceeded => Self {
                status: StatusCode::PAYMENT_REQUIRED,
                code: "node_quota_exceeded",
                message: "The plan's active node limit has been reached.".into(),
                retryable: false,
                compatibility: false,
            },
            RepositoryError::PlatformAlreadyEnabled => Self {
                status: StatusCode::CONFLICT,
                code: "platform_already_enabled",
                message: "Platform mode is already enabled for this workspace.".into(),
                retryable: false,
                compatibility: false,
            },
            RepositoryError::Persistence(message) => {
                tracing::error!(
                    database.error = %message,
                    error.type = "repository_persistence_failure",
                    "repository operation failed"
                );
                Self {
                    status: StatusCode::SERVICE_UNAVAILABLE,
                    code: "service_unavailable",
                    message: "A required service is temporarily unavailable.".into(),
                    retryable: true,
                    compatibility: false,
                }
            }
        }
    }
}

impl From<piqae_storage_postgres::StorageError> for AppError {
    fn from(error: piqae_storage_postgres::StorageError) -> Self {
        RepositoryError::from(error).into()
    }
}

impl From<serde_json::Error> for AppError {
    fn from(_: serde_json::Error) -> Self {
        Self::invalid("invalid_json", "The request JSON is invalid.")
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let request_id = request_id::current();
        let span = tracing::Span::current();
        span.record("error.type", self.code);
        if self.status.is_server_error() {
            span.record("otel.status_code", "ERROR");
            tracing::error!(
                request_id,
                error.type = self.code,
                http.response.status_code = self.status.as_u16(),
                retryable = self.retryable,
                "request failed"
            );
        } else {
            tracing::warn!(
                request_id,
                error.type = self.code,
                http.response.status_code = self.status.as_u16(),
                retryable = self.retryable,
                "request rejected"
            );
        }
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
