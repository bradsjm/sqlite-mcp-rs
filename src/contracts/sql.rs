use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SqlParams {
    Positional(Vec<Value>),
    Named(serde_json::Map<String, Value>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlQueryRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    #[serde(default)]
    pub sql: Option<String>,
    #[serde(default)]
    pub params: Option<SqlParams>,
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::optional_usize_schema")]
    pub max_rows: Option<usize>,
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::optional_usize_schema")]
    pub max_bytes: Option<usize>,
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlQueryData {
    pub columns: Vec<String>,
    pub rows: Vec<serde_json::Map<String, Value>>,
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub row_count: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlExecuteRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub sql: String,
    #[serde(default)]
    pub params: Option<SqlParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlExecuteData {
    #[schemars(schema_with = "crate::contracts::schema::u64_schema")]
    pub rows_affected: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_insert_rowid: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BatchTransactionMode {
    Required,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchStatement {
    pub sql: String,
    #[serde(default)]
    pub params: Option<SqlParams>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlBatchRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub transaction: BatchTransactionMode,
    #[serde(default)]
    pub confirm_destructive: bool,
    pub statements: Vec<BatchStatement>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BatchResultKind {
    Query,
    Execute,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlBatchResult {
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub index: usize,
    pub kind: BatchResultKind,
    #[schemars(schema_with = "crate::contracts::schema::u64_schema")]
    pub rows_affected: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_insert_rowid: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SqlBatchData {
    pub transaction: BatchTransactionMode,
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub executed: usize,
    pub results: Vec<SqlBatchResult>,
}
