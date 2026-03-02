use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("precondition required: {0}")]
    PreconditionRequired(String),
    #[error("feature disabled: {0}")]
    FeatureDisabled(String),
    #[error("missing configuration: {0}")]
    ConfigMissing(String),
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    #[error("sql error: {0}")]
    Sql(String),
    #[error("dependency error: {0}")]
    Dependency(String),
    #[error("internal error")]
    Internal,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProtocolError {
    pub message: String,
    pub details: ProtocolErrorDetails,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProtocolErrorDetails {
    pub code: ErrorCode,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidInput,
    NotFound,
    Conflict,
    PreconditionRequired,
    FeatureDisabled,
    ConfigMissing,
    LimitExceeded,
    SqlError,
    DependencyError,
    Internal,
}

impl AppError {
    pub fn to_protocol_error(&self) -> ProtocolError {
        match self {
            Self::InvalidInput(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::InvalidInput,
                    retryable: false,
                    context: None,
                },
            },
            Self::NotFound(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::NotFound,
                    retryable: false,
                    context: None,
                },
            },
            Self::Conflict(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::Conflict,
                    retryable: false,
                    context: None,
                },
            },
            Self::PreconditionRequired(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::PreconditionRequired,
                    retryable: false,
                    context: None,
                },
            },
            Self::FeatureDisabled(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::FeatureDisabled,
                    retryable: false,
                    context: None,
                },
            },
            Self::ConfigMissing(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::ConfigMissing,
                    retryable: false,
                    context: None,
                },
            },
            Self::LimitExceeded(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::LimitExceeded,
                    retryable: false,
                    context: None,
                },
            },
            Self::Sql(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::SqlError,
                    retryable: false,
                    context: None,
                },
            },
            Self::Dependency(message) => ProtocolError {
                message: message.clone(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::DependencyError,
                    retryable: true,
                    context: None,
                },
            },
            Self::Internal => ProtocolError {
                message: "internal error".to_string(),
                details: ProtocolErrorDetails {
                    code: ErrorCode::Internal,
                    retryable: false,
                    context: None,
                },
            },
        }
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value.to_string())
    }
}
