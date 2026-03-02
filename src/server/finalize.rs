use std::time::Instant;

use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::contracts::common::{ToolEnvelope, ToolHint, ToolMeta};

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
