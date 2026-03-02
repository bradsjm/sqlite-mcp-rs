use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportFormat {
    Csv,
    Json,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImportConflictMode {
    None,
    Ignore,
    Replace,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ImportPayload {
    Text(String),
    JsonRows(Vec<Map<String, Value>>),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbImportRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub format: ImportFormat,
    pub table: String,
    #[serde(default)]
    pub columns: Vec<String>,
    pub data: ImportPayload,
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::optional_usize_schema")]
    pub batch_size: Option<usize>,
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::import_conflict_mode_schema")]
    pub on_conflict: Option<ImportConflictMode>,
    #[serde(default)]
    pub truncate_first: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbImportData {
    pub table: String,
    pub columns: Vec<String>,
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub rows_inserted: usize,
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub rows_skipped: usize,
}
