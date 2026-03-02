use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolHint {
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolMeta {
    pub now_utc: String,
    #[schemars(schema_with = "crate::contracts::schema::u64_schema")]
    pub elapsed_ms: u64,
    pub request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(bound = "T: JsonSchema")]
pub struct ToolEnvelope<T>
where
    T: Serialize,
{
    pub summary: String,
    pub data: T,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub hints: Vec<ToolHint>,
    pub _meta: ToolMeta,
}
