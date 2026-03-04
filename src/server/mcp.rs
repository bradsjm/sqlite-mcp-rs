use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::{ErrorData as McpError, Json, ServerHandler, model::*, tool, tool_handler, tool_router};
#[cfg(not(feature = "vector"))]
use schemars::JsonSchema;
#[cfg(not(feature = "vector"))]
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::contracts::common::ToolEnvelope;
use crate::contracts::db::{
    DbCloseData, DbCloseRequest, DbListData, DbListRequest, DbMode, DbOpenData, DbOpenRequest,
};
use crate::contracts::import::{DbImportData, DbImportRequest};
use crate::contracts::queue::{QueuePushData, QueuePushRequest, QueueWaitData, QueueWaitRequest};
use crate::contracts::sql::{
    SqlBatchData, SqlBatchRequest, SqlExecuteData, SqlExecuteRequest, SqlQueryData, SqlQueryRequest,
};
#[cfg(feature = "vector")]
use crate::contracts::vector::{
    VectorCollectionCreateData, VectorCollectionCreateRequest, VectorCollectionListData,
    VectorCollectionListRequest, VectorSearchData, VectorSearchRequest, VectorUpsertData,
    VectorUpsertRequest,
};
use crate::db::registry::DbRegistry;
use crate::errors::{AppError, AppResult, ErrorCode};
use crate::pagination::cursor_store::CursorStore;
use crate::policy::SqlPolicy;
use crate::tools;
#[cfg(feature = "vector")]
use crate::tools::vector::VectorRuntime;

#[derive(Clone)]
pub struct SqliteMcpServer {
    registry: Arc<Mutex<DbRegistry>>,
    cursors: Arc<Mutex<CursorStore>>,
    config: Arc<AppConfig>,
    persist_root: Option<PathBuf>,
    #[cfg(feature = "vector")]
    vector_runtime: Arc<VectorRuntime>,
    tool_router: ToolRouter<Self>,
}

#[cfg(not(feature = "vector"))]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct VectorDisabledRequest {}

#[cfg(not(feature = "vector"))]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct VectorDisabledData {
    message: String,
}

impl SqliteMcpServer {
    pub fn new(config: AppConfig) -> Self {
        let persist_root = config.persist_root.clone();
        let cursor_ttl = Duration::from_secs(config.cursor_ttl_seconds);
        let cursor_capacity = config.cursor_capacity;
        let mut registry = DbRegistry::default();
        let mut cursors = CursorStore::new(cursor_ttl, cursor_capacity);

        if let Ok(path) = std::env::var("SQLITE_INSPECTOR_DB_PATH") {
            let request = DbOpenRequest {
                db_id: Some(crate::DEFAULT_DB_ID.to_string()),
                mode: DbMode::Persist,
                path: Some(path.clone()),
                reset: false,
            };
            if let Err(error) = tools::db::db_open(
                &mut registry,
                &mut cursors,
                request,
                persist_root.as_deref(),
                config.max_db_bytes,
            ) {
                tracing::warn!(path, error = %error, "failed to bootstrap persisted default db");
                let fallback_request = DbOpenRequest {
                    db_id: Some(crate::DEFAULT_DB_ID.to_string()),
                    mode: DbMode::Memory,
                    path: None,
                    reset: false,
                };
                if let Err(fallback_error) = tools::db::db_open(
                    &mut registry,
                    &mut cursors,
                    fallback_request,
                    persist_root.as_deref(),
                    config.max_db_bytes,
                ) {
                    tracing::error!(
                        error = %fallback_error,
                        "failed to bootstrap in-memory default db"
                    );
                }
            }
        } else {
            let request = DbOpenRequest {
                db_id: Some(crate::DEFAULT_DB_ID.to_string()),
                mode: DbMode::Memory,
                path: None,
                reset: false,
            };
            if let Err(error) = tools::db::db_open(
                &mut registry,
                &mut cursors,
                request,
                persist_root.as_deref(),
                config.max_db_bytes,
            ) {
                tracing::error!(error = %error, "failed to bootstrap in-memory default db");
            }
        }

        #[cfg(feature = "vector")]
        let vector_runtime = Arc::new(VectorRuntime::new(
            config.embedding.clone(),
            config.reranker.clone(),
        ));
        Self {
            registry: Arc::new(Mutex::new(registry)),
            cursors: Arc::new(Mutex::new(cursors)),
            config: Arc::new(config),
            persist_root,
            #[cfg(feature = "vector")]
            vector_runtime,
            tool_router: Self::tool_router(),
        }
    }

    fn sql_policy(&self) -> SqlPolicy {
        SqlPolicy {
            max_sql_length: self.config.max_sql_length,
            max_statements: self.config.max_statements,
            max_rows: self.config.max_rows,
            max_bytes: self.config.max_bytes,
            max_db_bytes: self.config.max_db_bytes,
        }
    }

    fn queue_wait_limits(&self) -> tools::queue::QueueWaitLimits {
        tools::queue::QueueWaitLimits {
            timeout_default_ms: self.config.queue_wait_timeout_ms_default,
            timeout_max_ms: self.config.queue_wait_timeout_ms_max,
            poll_interval_default_ms: self.config.queue_poll_interval_ms_default,
            poll_interval_min_ms: self.config.queue_poll_interval_ms_min,
            poll_interval_max_ms: self.config.queue_poll_interval_ms_max,
        }
    }

    fn log_tool_ok(
        tool: &str,
        db_id: &str,
        response_meta: &crate::contracts::common::ToolMeta,
        partial: bool,
    ) {
        if partial {
            tracing::warn!(
                request_id = %response_meta.request_id,
                tool,
                db_id,
                status = "partial",
                elapsed_ms = response_meta.elapsed_ms,
                "tool completed with fallback"
            );
            return;
        }

        tracing::info!(
            request_id = %response_meta.request_id,
            tool,
            db_id,
            status = "ok",
            elapsed_ms = response_meta.elapsed_ms,
            "tool completed"
        );
    }

    fn log_tool_error(
        tool: &str,
        db_id: &str,
        request_id: &str,
        started_at: Instant,
        error: &AppError,
    ) {
        let protocol = error.to_protocol_error();
        let elapsed_ms = started_at.elapsed().as_millis() as u64;

        if protocol.details.retryable {
            tracing::warn!(
                request_id,
                tool,
                db_id,
                status = "error",
                elapsed_ms,
                error_code = ?protocol.details.code,
                "tool failed"
            );
        } else {
            tracing::error!(
                request_id,
                tool,
                db_id,
                status = "error",
                elapsed_ms,
                error_code = ?protocol.details.code,
                "tool failed"
            );
        }
    }

    fn map_error(error: AppError, request_id: Option<&str>) -> McpError {
        let protocol = error.to_protocol_error();
        let code = match protocol.details.code {
            ErrorCode::Internal | ErrorCode::SqlError | ErrorCode::DependencyError => {
                rmcp::model::ErrorCode::INTERNAL_ERROR
            }
            _ => rmcp::model::ErrorCode::INVALID_PARAMS,
        };

        let context = match (request_id, protocol.details.context) {
            (Some(request_id), Some(mut context)) => {
                if let Some(object) = context.as_object_mut() {
                    object.insert(
                        "request_id".to_string(),
                        serde_json::Value::String(request_id.to_string()),
                    );
                    Some(context)
                } else {
                    Some(serde_json::json!({
                        "request_id": request_id,
                        "context": context,
                    }))
                }
            }
            (Some(request_id), None) => Some(serde_json::json!({ "request_id": request_id })),
            (None, context) => context,
        };

        McpError::new(
            code,
            protocol.message,
            Some(serde_json::json!({
                "code": protocol.details.code,
                "retryable": protocol.details.retryable,
                "context": context,
            })),
        )
    }

    #[cfg(not(feature = "vector"))]
    fn vector_disabled_error() -> McpError {
        Self::map_error(
            AppError::InvalidInput("vector feature is not enabled".to_string()),
            None,
        )
    }

    async fn run_blocking<T, F>(&self, task_fn: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut DbRegistry, &mut CursorStore) -> AppResult<T> + Send + 'static,
    {
        let registry = Arc::clone(&self.registry);
        let cursors = Arc::clone(&self.cursors);
        task::spawn_blocking(move || {
            let mut registry = registry.blocking_lock();
            let mut cursors = cursors.blocking_lock();
            task_fn(&mut registry, &mut cursors)
        })
        .await
        .map_err(|_| AppError::Internal)?
    }

    fn stamp_response_request_id<T>(
        mut response: ToolEnvelope<T>,
        request_id: String,
    ) -> ToolEnvelope<T>
    where
        T: serde::Serialize,
    {
        response._meta.request_id = request_id;
        response
    }
}

#[tool_router]
impl SqliteMcpServer {
    #[tool(
        name = "db_open",
        description = "Open an in-memory or persisted SQLite database and activate it"
    )]
    async fn db_open(
        &self,
        Parameters(request): Parameters<DbOpenRequest>,
    ) -> Result<Json<ToolEnvelope<DbOpenData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let persist_root = self.persist_root.clone();
        let max_db_bytes = self.config.max_db_bytes;
        let response = match self
            .run_blocking(move |registry, cursors| {
                tools::db::db_open(
                    registry,
                    cursors,
                    request,
                    persist_root.as_deref(),
                    max_db_bytes,
                )
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("db_open", &resolved_db_id, &request_id, started_at, &error);
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("db_open", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(
        name = "db_list",
        description = "List open and persisted database handles"
    )]
    async fn db_list(
        &self,
        Parameters(request): Parameters<DbListRequest>,
    ) -> Result<Json<ToolEnvelope<DbListData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let persist_root = self.persist_root.clone();
        let persisted_limit = self.config.max_persisted_list_entries;
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::db::db_list(registry, request, persist_root.as_deref(), persisted_limit)
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "db_list",
                    crate::DEFAULT_DB_ID,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("db_list", crate::DEFAULT_DB_ID, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(
        name = "db_close",
        description = "Close an open SQLite database handle"
    )]
    async fn db_close(
        &self,
        Parameters(request): Parameters<DbCloseRequest>,
    ) -> Result<Json<ToolEnvelope<DbCloseData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let response = match self
            .run_blocking(move |registry, cursors| tools::db::db_close(registry, cursors, request))
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("db_close", &resolved_db_id, &request_id, started_at, &error);
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("db_close", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(
        name = "sql_query",
        description = "Execute one read-only SQL statement"
    )]
    async fn sql_query(
        &self,
        Parameters(request): Parameters<SqlQueryRequest>,
    ) -> Result<Json<ToolEnvelope<SqlQueryData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let policy = self.sql_policy();
        let response = match self
            .run_blocking(move |registry, cursors| {
                tools::sql::sql_query(registry, cursors, &policy, request)
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "sql_query",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("sql_query", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(
        name = "sql_execute",
        description = "Execute one non-read SQL statement"
    )]
    async fn sql_execute(
        &self,
        Parameters(request): Parameters<SqlExecuteRequest>,
    ) -> Result<Json<ToolEnvelope<SqlExecuteData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let policy = self.sql_policy();
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::sql::sql_execute(registry, &policy, request)
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "sql_execute",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("sql_execute", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(
        name = "sql_batch",
        description = "Execute multiple write-only SQL statements (no SELECT)"
    )]
    async fn sql_batch(
        &self,
        Parameters(request): Parameters<SqlBatchRequest>,
    ) -> Result<Json<ToolEnvelope<SqlBatchData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let policy = self.sql_policy();
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::sql::sql_batch(registry, &policy, request)
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "sql_batch",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("sql_batch", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(
        name = "db_import",
        description = "Import CSV or JSON rows into a table; creates the table by default when missing"
    )]
    async fn db_import(
        &self,
        Parameters(request): Parameters<DbImportRequest>,
    ) -> Result<Json<ToolEnvelope<DbImportData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let policy = self.sql_policy();
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::import::db_import(registry, &policy, request)
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "db_import",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("db_import", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(name = "queue_push", description = "Enqueue a JSON job")]
    async fn queue_push(
        &self,
        Parameters(request): Parameters<QueuePushRequest>,
    ) -> Result<Json<ToolEnvelope<QueuePushData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let max_bytes = self.config.max_bytes;
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::queue::queue_push(registry, max_bytes, request)
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "queue_push",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("queue_push", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(
        name = "queue_wait",
        description = "Wait for a visible JSON job; set include_existing=true to consume queued jobs"
    )]
    async fn queue_wait(
        &self,
        Parameters(request): Parameters<QueueWaitRequest>,
    ) -> Result<Json<ToolEnvelope<QueueWaitData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());

        let limits = self.queue_wait_limits();
        let plan = match self
            .run_blocking(move |registry, _cursors| {
                tools::queue::build_wait_plan(registry, limits, request)
            })
            .await
        {
            Ok(plan) => plan,
            Err(error) => {
                Self::log_tool_error(
                    "queue_wait",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };

        let timeout_at = started_at + Duration::from_millis(plan.timeout_ms);
        loop {
            let poll_db_id = plan.db_id.clone();
            let poll_queue = plan.queue.clone();
            let poll_after_id = plan.after_id;

            let maybe_job = match self
                .run_blocking(move |registry, _cursors| {
                    tools::queue::poll_visible_job(
                        registry,
                        &poll_db_id,
                        &poll_queue,
                        poll_after_id,
                    )
                })
                .await
            {
                Ok(job) => job,
                Err(error) => {
                    Self::log_tool_error(
                        "queue_wait",
                        &plan.db_id,
                        &request_id,
                        started_at,
                        &error,
                    );
                    return Err(Self::map_error(error, Some(&request_id)));
                }
            };

            if let Some(job) = maybe_job {
                let response = tools::queue::queue_wait_found(plan.queue, job, started_at);
                let response = Self::stamp_response_request_id(response, request_id);
                Self::log_tool_ok("queue_wait", &plan.db_id, &response._meta, false);
                return Ok(Json(response));
            }

            if Instant::now() >= timeout_at {
                let response = tools::queue::queue_wait_timeout(plan.queue, started_at);
                let response = Self::stamp_response_request_id(response, request_id);
                Self::log_tool_ok("queue_wait", &plan.db_id, &response._meta, false);
                return Ok(Json(response));
            }

            tokio::time::sleep(Duration::from_millis(plan.poll_interval_ms)).await;
        }
    }

    #[tool(
        name = "vector_collection_create",
        description = "Create a vector collection backing tables"
    )]
    #[cfg(not(feature = "vector"))]
    async fn vector_collection_create(
        &self,
        Parameters(_request): Parameters<VectorDisabledRequest>,
    ) -> Result<Json<ToolEnvelope<VectorDisabledData>>, McpError> {
        Err(Self::vector_disabled_error())
    }

    #[tool(
        name = "vector_collection_create",
        description = "Create a vector collection backing tables"
    )]
    #[cfg(feature = "vector")]
    async fn vector_collection_create(
        &self,
        Parameters(request): Parameters<VectorCollectionCreateRequest>,
    ) -> Result<Json<ToolEnvelope<VectorCollectionCreateData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let max_db_bytes = self.config.max_db_bytes;
        let vector_runtime = Arc::clone(&self.vector_runtime);
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::vector::vector_collection_create(
                    registry,
                    &vector_runtime,
                    request,
                    max_db_bytes,
                )
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "vector_collection_create",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok(
            "vector_collection_create",
            &resolved_db_id,
            &response._meta,
            false,
        );
        Ok(Json(response))
    }

    #[tool(
        name = "vector_collection_list",
        description = "List vector collections"
    )]
    #[cfg(not(feature = "vector"))]
    async fn vector_collection_list(
        &self,
        Parameters(_request): Parameters<VectorDisabledRequest>,
    ) -> Result<Json<ToolEnvelope<VectorDisabledData>>, McpError> {
        Err(Self::vector_disabled_error())
    }

    #[tool(
        name = "vector_collection_list",
        description = "List vector collections"
    )]
    #[cfg(feature = "vector")]
    async fn vector_collection_list(
        &self,
        Parameters(request): Parameters<VectorCollectionListRequest>,
    ) -> Result<Json<ToolEnvelope<VectorCollectionListData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::vector::vector_collection_list(registry, request)
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "vector_collection_list",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok(
            "vector_collection_list",
            &resolved_db_id,
            &response._meta,
            false,
        );
        Ok(Json(response))
    }

    #[tool(name = "vector_upsert", description = "Upsert vector documents")]
    #[cfg(not(feature = "vector"))]
    async fn vector_upsert(
        &self,
        Parameters(_request): Parameters<VectorDisabledRequest>,
    ) -> Result<Json<ToolEnvelope<VectorDisabledData>>, McpError> {
        Err(Self::vector_disabled_error())
    }

    #[tool(name = "vector_upsert", description = "Upsert vector documents")]
    #[cfg(feature = "vector")]
    async fn vector_upsert(
        &self,
        Parameters(request): Parameters<VectorUpsertRequest>,
    ) -> Result<Json<ToolEnvelope<VectorUpsertData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let max_db_bytes = self.config.max_db_bytes;
        let vector_runtime = Arc::clone(&self.vector_runtime);
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::vector::vector_upsert(registry, &vector_runtime, request, max_db_bytes)
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "vector_upsert",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        Self::log_tool_ok("vector_upsert", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(name = "vector_search", description = "Search vector collections")]
    #[cfg(not(feature = "vector"))]
    async fn vector_search(
        &self,
        Parameters(_request): Parameters<VectorDisabledRequest>,
    ) -> Result<Json<ToolEnvelope<VectorDisabledData>>, McpError> {
        Err(Self::vector_disabled_error())
    }

    #[tool(name = "vector_search", description = "Search vector collections")]
    #[cfg(feature = "vector")]
    async fn vector_search(
        &self,
        Parameters(request): Parameters<VectorSearchRequest>,
    ) -> Result<Json<ToolEnvelope<VectorSearchData>>, McpError> {
        let started_at = Instant::now();
        let request_id = Uuid::new_v4().to_string();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let max_vector_top_k = self.config.max_vector_top_k;
        let max_rerank_fetch_k = self.config.max_rerank_fetch_k;
        let vector_runtime = Arc::clone(&self.vector_runtime);
        let response = match self
            .run_blocking(move |registry, _cursors| {
                tools::vector::vector_search(
                    registry,
                    &vector_runtime,
                    request,
                    max_vector_top_k,
                    max_rerank_fetch_k,
                )
            })
            .await
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "vector_search",
                    &resolved_db_id,
                    &request_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error, Some(&request_id)));
            }
        };
        let response = Self::stamp_response_request_id(response, request_id);
        let partial = !response.data.issues.is_empty();
        Self::log_tool_ok("vector_search", &resolved_db_id, &response._meta, partial);
        Ok(Json(response))
    }
}

#[tool_handler]
impl ServerHandler for SqliteMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "sqlite-mcp-rs executes bounded SQLite operations through typed MCP tools."
                    .to_string(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
