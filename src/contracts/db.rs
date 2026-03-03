//! Database management request/response types.
//!
//! Types for opening, closing, and listing SQLite databases.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Database storage mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DbMode {
    /// In-memory database (ephemeral).
    Memory,
    /// File-persisted database.
    Persist,
}

/// Request to open a database connection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbOpenRequest {
    /// Database identifier (defaults to "default" if not specified).
    #[serde(default)]
    pub db_id: Option<String>,
    /// Storage mode (memory or persisted).
    pub mode: DbMode,
    /// File path for persisted databases (relative to persist root).
    #[serde(default)]
    pub path: Option<String>,
    /// Whether to close and reopen if already open with different parameters.
    #[serde(default)]
    pub reset: bool,
}

/// Information about loaded SQLite extensions.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionsLoaded {
    /// Whether sqlite-vec extension is loaded.
    pub vec: bool,
    /// Whether rembed extension is loaded.
    pub rembed: bool,
    /// Whether regex extension is loaded.
    pub regex: bool,
}

/// Response data for successful database open.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbOpenData {
    /// Database identifier.
    pub db_id: String,
    /// Storage mode used.
    pub mode: DbMode,
    /// Resolved file path for persisted databases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Whether this database is now the active database.
    pub active: bool,
    /// Information about loaded extensions.
    pub extensions_loaded: ExtensionsLoaded,
}

/// Request to list open and persisted databases.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbListRequest {}

/// Request to close a database connection.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbCloseRequest {
    /// Database identifier to close (defaults to active database).
    #[serde(default)]
    pub db_id: Option<String>,
}

/// Response data for successful database close.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbCloseData {
    /// Database identifier that was closed.
    pub db_id: String,
    /// Whether the close operation succeeded.
    pub closed: bool,
    /// New active database identifier.
    pub active_db_id: String,
}

/// Summary information about an open database.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbSummary {
    /// Database identifier.
    pub db_id: String,
    /// Storage mode.
    pub mode: DbMode,
    /// File path for persisted databases.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// Response data for database list operation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbListData {
    /// Currently active database identifier.
    pub active_db_id: String,
    /// List of open database handles.
    pub open: Vec<DbSummary>,
    /// List of persisted database file paths.
    #[serde(default)]
    pub persisted: Vec<String>,
    /// Whether the persisted list was truncated.
    #[serde(default)]
    pub persisted_truncated: bool,
}
