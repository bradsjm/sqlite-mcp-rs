use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueuePushRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub queue: String,
    #[schemars(schema_with = "crate::contracts::schema::any_json_value_schema")]
    pub payload: Value,
    #[serde(default)]
    pub metadata: Option<Map<String, Value>>,
    #[serde(default)]
    pub visible_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueuePushData {
    pub queue: String,
    pub id: i64,
    pub created_at: String,
    pub visible_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueueWaitRequest {
    #[serde(default)]
    pub db_id: Option<String>,
    pub queue: String,
    #[serde(default)]
    pub after_id: Option<i64>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
    #[serde(default)]
    pub include_existing: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueueJobData {
    pub id: i64,
    pub payload: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
    pub created_at: String,
    pub visible_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueueWaitData {
    pub queue: String,
    pub timed_out: bool,
    pub job: QueueJobSlot,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct QueueJobSlot {
    #[serde(default)]
    pub id: Option<i64>,
    #[serde(default)]
    pub payload: Option<Value>,
    #[serde(default)]
    pub metadata: Option<Map<String, Value>>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub visible_at: Option<String>,
}
