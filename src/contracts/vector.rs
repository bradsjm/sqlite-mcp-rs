use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorCollectionCreateRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub collection: String,
    #[serde(default)]
    pub if_not_exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorCollectionCreateData {
    pub collection: String,
    pub docs_table: String,
    pub vec_table: String,
    pub created: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorCollectionListRequest {
    #[serde(default)]
    pub db_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorCollectionSummary {
    pub collection: String,
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub docs_count: usize,
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub dimension: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorCollectionListData {
    pub collections: Vec<VectorCollectionSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorStatusRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    #[serde(default = "default_true")]
    pub prewarm: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorBackendStatus {
    pub provider: String,
    pub model: String,
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub dimension: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_dir: Option<String>,
    pub ready: bool,
    #[serde(default)]
    pub issues: Vec<VectorIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorStatusData {
    pub db_id: String,
    pub ort_ready: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ort_dylib_path: Option<String>,
    pub prewarm_attempted: bool,
    pub embedding: VectorBackendStatus,
    pub reranker: VectorBackendStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum VectorConflictMode {
    #[default]
    Replace,
    Ignore,
    UpdateMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorDocument {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorUpsertRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub collection: String,
    #[serde(default)]
    pub on_conflict: VectorConflictMode,
    pub items: Vec<VectorDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorUpsertData {
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub upserted_count: usize,
    #[schemars(schema_with = "crate::contracts::schema::usize_schema")]
    pub skipped_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum RerankMode {
    #[default]
    Off,
    On,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorSearchRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub collection: String,
    pub query_text: String,
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::optional_usize_schema")]
    pub top_k: Option<usize>,
    #[serde(default)]
    pub include_text: bool,
    #[serde(default)]
    pub include_metadata: bool,
    #[serde(default)]
    pub filter: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub rerank: RerankMode,
    #[serde(default)]
    #[schemars(schema_with = "crate::contracts::schema::optional_usize_schema")]
    pub rerank_fetch_k: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorMatch {
    pub id: String,
    pub distance: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Map<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorIssue {
    pub stage: String,
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorSearchData {
    pub matches: Vec<VectorMatch>,
    pub truncated: bool,
    pub reranked: bool,
    pub rerank_model: String,
    #[serde(default)]
    pub issues: Vec<VectorIssue>,
}

const fn default_true() -> bool {
    true
}
