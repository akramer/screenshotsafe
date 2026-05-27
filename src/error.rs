use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Not found")]
    NotFound,

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Forbidden")]
    Forbidden,

    #[error("Denied: {reason}")]
    Denied { status: StatusCode, reason: String },

    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Gone: {0}")]
    Gone(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error(transparent)]
    Rusqlite(#[from] rusqlite::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    ImageError(#[from] image::ImageError),
}

impl AppError {
    pub fn unauthorized(reason: impl Into<String>) -> Self {
        Self::Denied {
            status: StatusCode::UNAUTHORIZED,
            reason: reason.into(),
        }
    }

    pub fn forbidden(reason: impl Into<String>) -> Self {
        Self::Denied {
            status: StatusCode::FORBIDDEN,
            reason: reason.into(),
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message, denial_reason) = match &self {
            AppError::NotFound => (StatusCode::NOT_FOUND, self.to_string(), None),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                self.to_string(),
                Some(self.to_string()),
            ),
            AppError::Forbidden => (
                StatusCode::FORBIDDEN,
                self.to_string(),
                Some(self.to_string()),
            ),
            AppError::Denied { status, reason } => {
                let message = match *status {
                    StatusCode::UNAUTHORIZED => "Unauthorized",
                    StatusCode::FORBIDDEN => "Forbidden",
                    _ => "Request denied",
                };
                (*status, message.to_string(), Some(reason.clone()))
            }
            AppError::BadRequest(msg) => (StatusCode::BAD_REQUEST, msg.clone(), Some(msg.clone())),
            AppError::Gone(msg) => (StatusCode::GONE, msg.clone(), Some(msg.clone())),
            AppError::Internal(msg) => {
                tracing::error!("Internal error: {}", msg);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                    None,
                )
            }
            AppError::Rusqlite(e) => {
                tracing::error!("Database error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                    None,
                )
            }
            AppError::Io(e) => {
                tracing::error!("IO error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal server error".to_string(),
                    None,
                )
            }
            AppError::ImageError(e) => {
                tracing::error!("Image processing error: {}", e);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Image processing error".to_string(),
                    None,
                )
            }
        };

        if let Some(reason) = denial_reason {
            tracing::warn!(
                response_code = status.as_u16(),
                denial_reason = %reason,
                "Request denied"
            );
        }

        let body = serde_json::json!({ "error": message });
        (status, axum::Json(body)).into_response()
    }
}
