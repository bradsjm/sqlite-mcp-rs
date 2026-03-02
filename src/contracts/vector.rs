use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorCollectionCreateRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub collection: String,
    pub dimension: usize,
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
    pub docs_count: usize,
    pub dimension: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorCollectionListData {
    pub collections: Vec<VectorCollectionSummary>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VectorConflictMode {
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
    pub on_conflict: Option<VectorConflictMode>,
    pub items: Vec<VectorDocument>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct VectorUpsertData {
    pub upserted_count: usize,
    pub skipped_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RerankMode {
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
    pub top_k: Option<usize>,
    #[serde(default)]
    pub include_text: bool,
    #[serde(default)]
    pub include_metadata: bool,
    #[serde(default)]
    pub filter: Option<serde_json::Map<String, Value>>,
    #[serde(default)]
    pub rerank: Option<RerankMode>,
    #[serde(default)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rerank_model: Option<String>,
    #[serde(default)]
    pub issues: Vec<VectorIssue>,
}
