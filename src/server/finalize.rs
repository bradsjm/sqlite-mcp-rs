//! Response finalization utilities.
//!
//! Provides functions for creating standardized tool response envelopes
//! with timing, metadata, and request tracking.

use std::time::Instant;

use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::contracts::common::{ToolEnvelope, ToolHint, ToolMeta};

/// Creates a finalized tool response envelope.
///
/// Wraps the response data with a summary, metadata including timing
/// and request ID, and optional hints for follow-up actions.
///
/// # Arguments
///
/// * `summary` - Human-readable summary of the operation result
/// * `data` - Response data payload
/// * `started_at` - Instant when the operation started (for elapsed time)
/// * `hints` - Suggested follow-up tool calls
/// * `truncated` - Whether results were truncated
/// * `next_cursor` - Cursor for fetching the next page
///
/// # Returns
///
/// A [`ToolEnvelope`] containing the data and all metadata.
pub fn finalize_tool<T>(
    summary: impl Into<String>,
    data: T,
    started_at: Instant,
    hints: Vec<ToolHint>,
    truncated: Option<bool>,
    next_cursor: Option<String>,
) -> ToolEnvelope<T>
where
    T: serde::Serialize,
{
    let now_utc = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string());

    ToolEnvelope {
        summary: summary.into(),
        data,
        hints,
        _meta: ToolMeta {
            now_utc,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            request_id: Uuid::new_v4().to_string(),
            truncated,
            next_cursor,
        },
    }
}
