//! SQL execution request/response types.
//!
//! Types for executing queries, statements, and batches.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// SQL parameter binding type (positional or named).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SqlParams {
    /// Positional parameters bound by index (1-based).
    Positional(Vec<Value>),
    /// Named parameters bound by name (e.g., ":name", "@name", "$name").
    Named(serde_json::Map<String, Value>),
}

/// Request to execute a read-only SQL query.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlQueryRequest {
    /// Database identifier (defaults to active database).
    #[serde(default)]
    pub db_id: Option<String>,
    /// SQL query statement (required unless using cursor).
    #[serde(default)]
    pub sql: Option<String>,
    /// Parameters to bind to the query.
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::sql_params_schema")]
    pub params: Option<SqlParams>,
    /// Maximum rows to return (overrides default policy).
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::optional_usize_schema")]
    pub max_rows: Option<usize>,
    /// Maximum response size in bytes (overrides default policy).
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::optional_usize_schema")]
    pub max_bytes: Option<usize>,
    /// Cursor for paginating through large result sets.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Response data for SQL query execution.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlQueryData {
    /// Column names from the query result.
    pub columns: Vec<String>,
    /// Result rows as JSON objects.
    pub rows: Vec<serde_json::Map<String, Value>>,
    /// Number of rows returned.
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub row_count: usize,
    /// Whether results were truncated due to limits.
    pub truncated: bool,
    /// Cursor for fetching the next page (if truncated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Request to execute a non-read SQL statement.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlExecuteRequest {
    /// Database identifier (defaults to active database).
    #[serde(default)]
    pub db_id: Option<String>,
    /// SQL statement to execute.
    pub sql: String,
    /// Parameters to bind to the statement.
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::sql_params_schema")]
    pub params: Option<SqlParams>,
}

/// Response data for SQL statement execution.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlExecuteData {
    /// Number of rows affected by the statement.
    #[schemars(schema_with = "crate::contracts::schema::u64_schema")]
    pub rows_affected: u64,
    /// Row ID of the last inserted row (for INSERT statements).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_insert_rowid: Option<i64>,
}

/// Transaction mode for batch execution.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchTransactionMode {
    /// Execute batch within a transaction (required for consistency).
    Required,
    /// Execute without explicit transaction management.
    None,
}

/// Single statement within a batch.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchStatement {
    /// SQL statement to execute.
    pub sql: String,
    /// Parameters to bind to the statement.
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::sql_params_schema")]
    pub params: Option<SqlParams>,
}

/// Request to execute multiple SQL statements in a batch.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlBatchRequest {
    /// Database identifier (defaults to active database).
    #[serde(default)]
    pub db_id: Option<String>,
    /// Transaction mode for the batch.
    pub transaction: BatchTransactionMode,
    /// Confirm destructive operations (DROP, TRUNCATE, DELETE without WHERE).
    #[serde(default)]
    pub confirm_destructive: bool,
    /// Statements to execute in order.
    pub statements: Vec<BatchStatement>,
}

/// Type of batch statement result.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchResultKind {
    /// Read-only query result.
    Query,
    /// Write operation result.
    Execute,
}

/// Result of a single statement in a batch.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlBatchResult {
    /// Index of the statement in the batch (0-based).
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub index: usize,
    /// Type of result.
    pub kind: BatchResultKind,
    /// Number of rows affected.
    #[schemars(schema_with = "crate::contracts::schema::u64_schema")]
    pub rows_affected: u64,
    /// Row ID of last inserted row (for INSERT).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_insert_rowid: Option<i64>,
}

/// Response data for batch execution.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlBatchData {
    /// Transaction mode used.
    pub transaction: BatchTransactionMode,
    /// Number of statements successfully executed.
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub executed: usize,
    /// Results for each executed statement.
    pub results: Vec<SqlBatchResult>,
}
