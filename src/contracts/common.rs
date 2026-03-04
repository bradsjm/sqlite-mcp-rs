//! Common types for MCP tool responses.
//!
//! Provides shared structures for tool response envelopes, hints, and metadata.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A hint suggesting a follow-up tool call.
///
/// Hints guide clients toward next steps, such as pagination cursors
/// or related operations.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolHint {
    /// Name of the suggested tool to call.
    pub tool: String,
    /// Arguments to pass to the suggested tool.
    #[serde(default)]
    pub arguments: Value,
    /// Human-readable explanation of why this tool is suggested.
    pub reason: String,
}

/// Metadata included in every tool response.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolMeta {
    /// Current UTC timestamp in RFC 3339 format.
    pub now_utc: String,
    /// Request processing time in milliseconds (operational logging only).
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    pub elapsed_ms: u64,
    /// Unique identifier for this request (operational logging only).
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    pub request_id: String,
    /// Whether results were truncated due to limits (operational logging only).
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    pub truncated: Option<bool>,
    /// Cursor for fetching the next page (operational logging only).
    #[serde(skip_serializing, skip_deserializing)]
    #[schemars(skip)]
    pub next_cursor: Option<String>,
}

/// Standard envelope for all tool responses.
///
/// Wraps the response data with a summary, optional hints for follow-up
/// actions, and metadata about the request processing.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(bound = "T: JsonSchema")]
pub struct ToolEnvelope<T>
where
    T: Serialize,
{
    /// Human-readable summary of the operation result.
    pub summary: String,
    /// Response data payload (type varies by tool).
    pub data: T,
    /// Suggested follow-up tool calls (e.g., for pagination).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub hints: Vec<ToolHint>,
    /// Request metadata including timing and tracing information.
    pub _meta: ToolMeta,
}
