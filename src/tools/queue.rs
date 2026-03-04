use std::time::Instant;

use rusqlite::OptionalExtension;
use serde_json::{Map, Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::DEFAULT_DB_ID;
use crate::contracts::common::{ToolEnvelope, ToolHint};
use crate::contracts::queue::{
    QueueJobData, QueuePushData, QueuePushRequest, QueueWaitData, QueueWaitRequest,
};
use crate::db::registry::DbRegistry;
use crate::errors::{AppError, AppResult};
use crate::policy::is_valid_identifier;
use crate::server::finalize::finalize_tool;

const CREATE_QUEUE_TABLE_SQL: &str = "
    CREATE TABLE IF NOT EXISTS _queue_jobs (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        queue TEXT NOT NULL,
        payload_json TEXT NOT NULL,
        metadata_json TEXT,
        created_at TEXT NOT NULL,
        visible_at TEXT NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_queue_jobs_queue_id ON _queue_jobs(queue, id);
    CREATE INDEX IF NOT EXISTS idx_queue_jobs_queue_visible_id ON _queue_jobs(queue, visible_at, id);
";

#[derive(Debug, Clone, Copy)]
pub struct QueueWaitLimits {
    pub timeout_default_ms: u64,
    pub timeout_max_ms: u64,
    pub poll_interval_default_ms: u64,
    pub poll_interval_min_ms: u64,
    pub poll_interval_max_ms: u64,
}

#[derive(Debug, Clone)]
pub struct QueueWaitPlan {
    pub db_id: String,
    pub queue: String,
    pub after_id: i64,
    pub timeout_ms: u64,
    pub poll_interval_ms: u64,
}

pub fn queue_push(
    registry: &DbRegistry,
    max_bytes: usize,
    request: QueuePushRequest,
) -> AppResult<ToolEnvelope<QueuePushData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    validate_queue_name(&request.queue)?;
    validate_payload_size(max_bytes, &request.payload, request.metadata.as_ref())?;

    let connection = registry.get_connection(Some(&db_id))?;
    ensure_queue_table(connection)?;

    let now = OffsetDateTime::now_utc();
    let created_at = format_rfc3339(now);
    let visible_at_text = resolve_visible_at_text(request.visible_at.as_deref(), now)?;

    let payload_json = serde_json::to_string(&request.payload).map_err(|error| {
        AppError::Dependency(format!("failed to encode queue payload: {error}"))
    })?;
    let metadata_json = request
        .metadata
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| {
            AppError::Dependency(format!("failed to encode queue metadata: {error}"))
        })?;

    connection.execute(
        "INSERT INTO _queue_jobs(queue, payload_json, metadata_json, created_at, visible_at) VALUES(?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            request.queue,
            payload_json,
            metadata_json,
            created_at,
            visible_at_text,
        ],
    )?;
    let id = connection.last_insert_rowid();

    Ok(finalize_tool(
        "Job queued.",
        QueuePushData {
            queue: request.queue,
            id,
            created_at,
            visible_at: visible_at_text,
        },
        started,
        Vec::new(),
        None,
        None,
    ))
}

pub fn build_wait_plan(
    registry: &DbRegistry,
    limits: QueueWaitLimits,
    request: QueueWaitRequest,
) -> AppResult<QueueWaitPlan> {
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    validate_queue_name(&request.queue)?;

    let timeout_ms = request.timeout_ms.unwrap_or(limits.timeout_default_ms);
    if timeout_ms == 0 || timeout_ms > limits.timeout_max_ms {
        return Err(AppError::InvalidInput(format!(
            "timeout_ms must be between 1 and {}",
            limits.timeout_max_ms
        )));
    }

    let poll_interval_ms = request
        .poll_interval_ms
        .unwrap_or(limits.poll_interval_default_ms);
    if poll_interval_ms < limits.poll_interval_min_ms
        || poll_interval_ms > limits.poll_interval_max_ms
    {
        return Err(AppError::InvalidInput(format!(
            "poll_interval_ms must be between {} and {}",
            limits.poll_interval_min_ms, limits.poll_interval_max_ms
        )));
    }

    let connection = registry.get_connection(Some(&db_id))?;
    ensure_queue_table(connection)?;

    let after_id = match request.after_id {
        Some(value) if value >= 0 => value,
        Some(_) => {
            return Err(AppError::InvalidInput(
                "after_id must be greater than or equal to zero".to_string(),
            ));
        }
        None if request.include_existing => 0,
        None => current_max_queue_id(connection, &request.queue)?,
    };

    Ok(QueueWaitPlan {
        db_id,
        queue: request.queue,
        after_id,
        timeout_ms,
        poll_interval_ms,
    })
}

pub fn poll_visible_job(
    registry: &DbRegistry,
    db_id: &str,
    queue: &str,
    after_id: i64,
) -> AppResult<Option<QueueJobData>> {
    let connection = registry.get_connection(Some(db_id))?;
    ensure_queue_table(connection)?;
    let now = format_rfc3339(OffsetDateTime::now_utc());

    connection
        .query_row(
            "SELECT id, payload_json, metadata_json, created_at, visible_at
             FROM _queue_jobs
             WHERE queue = ?1 AND id > ?2 AND visible_at <= ?3
             ORDER BY id ASC
             LIMIT 1",
            rusqlite::params![queue, after_id, now],
            |row| {
                let payload_json: String = row.get(1)?;
                let metadata_json: Option<String> = row.get(2)?;

                let payload = serde_json::from_str::<Value>(&payload_json).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;

                let metadata = metadata_json
                    .as_deref()
                    .map(serde_json::from_str::<Map<String, Value>>)
                    .transpose()
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;

                Ok(QueueJobData {
                    id: row.get(0)?,
                    payload,
                    metadata,
                    created_at: row.get(3)?,
                    visible_at: row.get(4)?,
                })
            },
        )
        .optional()
        .map_err(AppError::from)
}

pub fn queue_wait_timeout(queue: String, started: Instant) -> ToolEnvelope<QueueWaitData> {
    let hints = vec![ToolHint {
        tool: "queue_wait".to_string(),
        arguments: json!({
            "queue": queue.clone(),
            "include_existing": true,
        }),
        reason: "Set include_existing=true to consume queued jobs that were already visible."
            .to_string(),
    }];

    finalize_tool(
        "No new job arrived before timeout.",
        QueueWaitData {
            queue,
            timed_out: true,
            job: None,
        },
        started,
        hints,
        None,
        None,
    )
}

pub fn queue_wait_found(
    queue: String,
    job: QueueJobData,
    started: Instant,
) -> ToolEnvelope<QueueWaitData> {
    let hints = vec![ToolHint {
        tool: "queue_wait".to_string(),
        arguments: json!({
            "queue": queue.clone(),
            "after_id": job.id,
        }),
        reason: "Pass after_id to continue from the next job without re-reading earlier jobs."
            .to_string(),
    }];

    finalize_tool(
        "Job received.",
        QueueWaitData {
            queue,
            timed_out: false,
            job: Some(job),
        },
        started,
        hints,
        None,
        None,
    )
}

fn ensure_queue_table(connection: &rusqlite::Connection) -> AppResult<()> {
    connection.execute_batch(CREATE_QUEUE_TABLE_SQL)?;
    Ok(())
}

fn validate_queue_name(queue: &str) -> AppResult<()> {
    if !is_valid_identifier(queue) {
        return Err(AppError::InvalidInput(
            "queue must match ^[A-Za-z_][A-Za-z0-9_]*$".to_string(),
        ));
    }
    Ok(())
}

fn validate_payload_size(
    max_bytes: usize,
    payload: &Value,
    metadata: Option<&Map<String, Value>>,
) -> AppResult<()> {
    let payload_bytes = serde_json::to_vec(payload)
        .map_err(|error| AppError::Dependency(format!("failed to encode queue payload: {error}")))?
        .len();
    let metadata_bytes = metadata
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|error| AppError::Dependency(format!("failed to encode queue metadata: {error}")))?
        .map_or(0, |encoded| encoded.len());

    if payload_bytes + metadata_bytes > max_bytes {
        return Err(AppError::LimitExceeded(format!(
            "queue payload exceeds max_bytes ({max_bytes})"
        )));
    }

    Ok(())
}

fn current_max_queue_id(connection: &rusqlite::Connection, queue: &str) -> AppResult<i64> {
    let max_id = connection.query_row(
        "SELECT COALESCE(MAX(id), 0) FROM _queue_jobs WHERE queue = ?1",
        [queue],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(max_id)
}

fn resolve_visible_at_text(value: Option<&str>, now: OffsetDateTime) -> AppResult<String> {
    match value {
        Some(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return Err(AppError::InvalidInput(
                    "visible_at must not be empty when provided".to_string(),
                ));
            }
            Ok(trimmed.to_string())
        }
        None => Ok(format_rfc3339(now)),
    }
}

fn format_rfc3339(value: OffsetDateTime) -> String {
    value
        .to_offset(time::UtcOffset::UTC)
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::contracts::db::DbMode;
    use crate::contracts::queue::{QueuePushRequest, QueueWaitRequest};
    use crate::db::registry::DbRegistry;

    use super::{QueueWaitLimits, build_wait_plan, poll_visible_job, queue_push};

    fn setup_registry() -> DbRegistry {
        let mut registry = DbRegistry::default();
        registry
            .open_db(
                "default".to_string(),
                DbMode::Memory,
                None,
                false,
                None,
                u64::MAX,
            )
            .expect("memory db should open");
        registry
    }

    fn limits() -> QueueWaitLimits {
        QueueWaitLimits {
            timeout_default_ms: 30_000,
            timeout_max_ms: 120_000,
            poll_interval_default_ms: 250,
            poll_interval_min_ms: 50,
            poll_interval_max_ms: 5_000,
        }
    }

    #[test]
    fn queue_wait_defaults_to_new_rows_only() {
        let registry = setup_registry();
        queue_push(
            &registry,
            1_048_576,
            QueuePushRequest {
                db_id: None,
                queue: "jobs".to_string(),
                payload: json!({"kind": "existing"}),
                metadata: None,
                visible_at: None,
            },
        )
        .expect("push should succeed");

        let plan = build_wait_plan(
            &registry,
            limits(),
            QueueWaitRequest {
                db_id: None,
                queue: "jobs".to_string(),
                after_id: None,
                timeout_ms: None,
                poll_interval_ms: None,
                include_existing: false,
            },
        )
        .expect("plan should build");

        assert_eq!(plan.after_id, 1);

        let job = poll_visible_job(&registry, &plan.db_id, &plan.queue, plan.after_id)
            .expect("poll should work");
        assert!(job.is_none());
    }

    #[test]
    fn queue_wait_can_include_existing_rows() {
        let registry = setup_registry();
        queue_push(
            &registry,
            1_048_576,
            QueuePushRequest {
                db_id: None,
                queue: "jobs".to_string(),
                payload: json!({"kind": "existing"}),
                metadata: None,
                visible_at: None,
            },
        )
        .expect("push should succeed");

        let plan = build_wait_plan(
            &registry,
            limits(),
            QueueWaitRequest {
                db_id: None,
                queue: "jobs".to_string(),
                after_id: None,
                timeout_ms: None,
                poll_interval_ms: None,
                include_existing: true,
            },
        )
        .expect("plan should build");

        assert_eq!(plan.after_id, 0);

        let job = poll_visible_job(&registry, &plan.db_id, &plan.queue, plan.after_id)
            .expect("poll should work")
            .expect("job should be visible");
        assert_eq!(job.id, 1);
    }

    #[test]
    fn queue_push_and_poll_round_trips_payload_and_metadata() {
        let registry = setup_registry();
        let pushed = queue_push(
            &registry,
            1_048_576,
            QueuePushRequest {
                db_id: None,
                queue: "jobs".to_string(),
                payload: json!({"task": "send_email", "attempt": 1}),
                metadata: Some(serde_json::Map::from_iter([(
                    "tenant".to_string(),
                    json!("acme"),
                )])),
                visible_at: None,
            },
        )
        .expect("push should succeed");

        let job = poll_visible_job(&registry, "default", "jobs", 0)
            .expect("poll should work")
            .expect("job should exist");

        assert_eq!(job.id, pushed.data.id);
        assert_eq!(job.payload["task"], "send_email");
        assert_eq!(
            job.metadata
                .as_ref()
                .and_then(|meta| meta.get("tenant"))
                .cloned(),
            Some(json!("acme"))
        );
        assert_eq!(job.created_at, pushed.data.created_at);
        assert_eq!(job.visible_at, pushed.data.visible_at);
    }

    #[test]
    fn poll_respects_after_id_and_returns_first_newest_ordered_job() {
        let registry = setup_registry();
        for kind in ["one", "two", "three"] {
            queue_push(
                &registry,
                1_048_576,
                QueuePushRequest {
                    db_id: None,
                    queue: "jobs".to_string(),
                    payload: json!({"kind": kind}),
                    metadata: None,
                    visible_at: None,
                },
            )
            .expect("push should succeed");
        }

        let first_after_two = poll_visible_job(&registry, "default", "jobs", 2)
            .expect("poll should work")
            .expect("row id 3 should exist");
        assert_eq!(first_after_two.id, 3);
        assert_eq!(first_after_two.payload["kind"], "three");

        let none_after_three =
            poll_visible_job(&registry, "default", "jobs", 3).expect("poll should work");
        assert!(none_after_three.is_none());
    }

    #[test]
    fn poll_ignores_rows_with_future_visible_at() {
        let registry = setup_registry();
        queue_push(
            &registry,
            1_048_576,
            QueuePushRequest {
                db_id: None,
                queue: "jobs".to_string(),
                payload: json!({"kind": "future"}),
                metadata: None,
                visible_at: Some("9999-12-31T23:59:59Z".to_string()),
            },
        )
        .expect("push should succeed");

        let job = poll_visible_job(&registry, "default", "jobs", 0).expect("poll should work");
        assert!(job.is_none());
    }

    #[test]
    fn rejects_payloads_larger_than_max_bytes() {
        let registry = setup_registry();
        let result = queue_push(
            &registry,
            32,
            QueuePushRequest {
                db_id: None,
                queue: "jobs".to_string(),
                payload: json!({"blob": "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"}),
                metadata: None,
                visible_at: None,
            },
        );

        assert!(result.is_err());
    }

    #[test]
    fn build_wait_plan_rejects_invalid_wait_bounds() {
        let registry = setup_registry();
        let result = build_wait_plan(
            &registry,
            limits(),
            QueueWaitRequest {
                db_id: None,
                queue: "jobs".to_string(),
                after_id: Some(-1),
                timeout_ms: Some(0),
                poll_interval_ms: Some(1),
                include_existing: false,
            },
        );

        assert!(result.is_err());
    }

    #[test]
    fn rejects_invalid_queue_name() {
        let registry = setup_registry();
        let result = queue_push(
            &registry,
            1_048_576,
            QueuePushRequest {
                db_id: None,
                queue: "bad-name".to_string(),
                payload: json!({}),
                metadata: None,
                visible_at: None,
            },
        );

        assert!(result.is_err());
    }
}
