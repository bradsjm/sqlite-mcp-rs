use schemars::JsonSchema;
use serde::Serialize;
use thiserror::Error;

/// Result type alias using [`AppError`] as the error type.
pub type AppResult<T> = Result<T, AppError>;

/// Application error types for domain-specific failures.
///
/// These errors map to MCP protocol errors with appropriate codes and retryability.
#[derive(Debug, Error)]
pub enum AppError {
    /// Invalid input parameters provided by the client.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Requested resource (database, collection, cursor, etc.) was not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// Resource conflict, such as opening a database with different parameters.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Precondition required, such as confirming destructive operations.
    #[error("precondition required: {0}")]
    PreconditionRequired(String),
    /// Feature is disabled or not compiled in.
    #[error("feature disabled: {0}")]
    FeatureDisabled(String),
    /// Required configuration is missing or invalid.
    #[error("missing configuration: {0}")]
    ConfigMissing(String),
    /// Operation exceeded configured limits (rows, bytes, statements, etc.).
    #[error("limit exceeded: {0}")]
    LimitExceeded(String),
    /// SQL execution error from the underlying database.
    #[error("sql error: {0}")]
    Sql(String),
    /// External dependency error (embedding model, file system, etc.).
    #[error("dependency error: {0}")]
    Dependency(String),
    /// Internal server error.
    #[error("internal error")]
    Internal,
}

/// Error response structure for MCP protocol errors.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProtocolError {
    /// Human-readable error message.
    pub message: String,
    /// Detailed error information including code and retryability.
    pub details: ProtocolErrorDetails,
}

/// Detailed error information for protocol responses.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ProtocolErrorDetails {
    /// Error code categorizing the failure type.
    pub code: ErrorCode,
    /// Whether the operation can be retried.
    pub retryable: bool,
    /// Additional context for debugging (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

/// Error codes for MCP protocol responses.
///
/// These codes are serialized in SCREAMING_SNAKE_CASE format.
/// Error codes for MCP protocol responses.
///
/// These codes are serialized in SCREAMING_SNAKE_CASE format.
#[derive(Debug, Clone, Copy, Serialize, JsonSchema)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Input validation failed.
    InvalidInput,
    /// Requested resource not found.
    NotFound,
    /// Resource conflict detected.
    Conflict,
    /// Precondition required but not met.
    PreconditionRequired,
    /// Feature is disabled.
    FeatureDisabled,
    /// Required configuration missing.
    ConfigMissing,
    /// Operation exceeded limits.
    LimitExceeded,
    /// SQL execution error.
    SqlError,
    /// External dependency error.
    DependencyError,
    /// Internal server error.
    Internal,
}

impl AppError {
    /// Converts this application error to a protocol error for MCP responses.
    ///
    /// Maps each error variant to an appropriate [`ErrorCode`] and sets
    /// retryability based on the error type. Dependency errors are marked
    /// as retryable, while most other errors are not.
    ///
    /// # Returns
    ///
    /// A [`ProtocolError`] with message, error code, and retryability flag.
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
    /// Converts a `rusqlite::Error` to an `AppError::Sql`.
    ///
    /// This allows the `?` operator to work with rusqlite operations.
    fn from(value: rusqlite::Error) -> Self {
        Self::Sql(value.to_string())
    }
}
