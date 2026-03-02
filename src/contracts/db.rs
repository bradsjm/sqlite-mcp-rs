use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DbMode {
    Memory,
    Persist,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbOpenRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub mode: DbMode,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub reset: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExtensionsLoaded {
    pub vec: bool,
    pub rembed: bool,
    pub regex: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbOpenData {
    pub db_id: String,
    pub mode: DbMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub active: bool,
    pub extensions_loaded: ExtensionsLoaded,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbListRequest {}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbCloseRequest {
    #[serde(default)]
    pub db_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbCloseData {
    pub db_id: String,
    pub closed: bool,
    pub active_db_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbSummary {
    pub db_id: String,
    pub mode: DbMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DbListData {
    pub active_db_id: String,
    pub open: Vec<DbSummary>,
    #[serde(default)]
    pub persisted: Vec<String>,
    #[serde(default)]
    pub persisted_truncated: bool,
}
