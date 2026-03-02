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
use uuid::Uuid;

use crate::config::AppConfig;
use crate::contracts::common::ToolEnvelope;
use crate::contracts::db::{DbListData, DbListRequest, DbMode, DbOpenData, DbOpenRequest};
use crate::contracts::import::{DbImportData, DbImportRequest};
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
use crate::errors::{AppError, ErrorCode};
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
            ) {
                tracing::warn!(path, error = %error, "failed to bootstrap persisted default db");
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

    fn log_tool_error(tool: &str, db_id: &str, started_at: Instant, error: &AppError) {
        let protocol = error.to_protocol_error();
        let request_id = Uuid::new_v4().to_string();
        let elapsed_ms = started_at.elapsed().as_millis() as u64;

        if protocol.details.retryable {
            tracing::warn!(
                request_id = %request_id,
                tool,
                db_id,
                status = "error",
                elapsed_ms,
                error_code = ?protocol.details.code,
                "tool failed"
            );
        } else {
            tracing::error!(
                request_id = %request_id,
                tool,
                db_id,
                status = "error",
                elapsed_ms,
                error_code = ?protocol.details.code,
                "tool failed"
            );
        }
    }

    fn map_error(error: AppError) -> McpError {
        let protocol = error.to_protocol_error();
        let code = match protocol.details.code {
            ErrorCode::Internal => rmcp::model::ErrorCode::INTERNAL_ERROR,
            _ => rmcp::model::ErrorCode::INVALID_PARAMS,
        };

        McpError::new(
            code,
            protocol.message,
            Some(serde_json::json!({
                "code": protocol.details.code,
                "retryable": protocol.details.retryable,
                "context": protocol.details.context,
            })),
        )
    }

    #[cfg(not(feature = "vector"))]
    fn vector_disabled_error() -> McpError {
        Self::map_error(AppError::InvalidInput(
            "vector feature is not enabled".to_string(),
        ))
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
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let mut registry = self.registry.lock().await;
        let mut cursors = self.cursors.lock().await;
        let response = match tools::db::db_open(
            &mut registry,
            &mut cursors,
            request,
            self.persist_root.as_deref(),
        ) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("db_open", &resolved_db_id, started_at, &error);
                return Err(Self::map_error(error));
            }
        };
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
        let registry = self.registry.lock().await;
        let response = match tools::db::db_list(
            &registry,
            request,
            self.persist_root.as_deref(),
            self.config.max_rows,
        ) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("db_list", crate::DEFAULT_DB_ID, started_at, &error);
                return Err(Self::map_error(error));
            }
        };
        Self::log_tool_ok("db_list", crate::DEFAULT_DB_ID, &response._meta, false);
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
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let registry = self.registry.lock().await;
        let mut cursors = self.cursors.lock().await;
        let policy = self.sql_policy();
        let response = match tools::sql::sql_query(&registry, &mut cursors, &policy, request) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("sql_query", &resolved_db_id, started_at, &error);
                return Err(Self::map_error(error));
            }
        };
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
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let registry = self.registry.lock().await;
        let policy = self.sql_policy();
        let response = match tools::sql::sql_execute(&registry, &policy, request) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("sql_execute", &resolved_db_id, started_at, &error);
                return Err(Self::map_error(error));
            }
        };
        Self::log_tool_ok("sql_execute", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(name = "sql_batch", description = "Execute multiple SQL statements")]
    async fn sql_batch(
        &self,
        Parameters(request): Parameters<SqlBatchRequest>,
    ) -> Result<Json<ToolEnvelope<SqlBatchData>>, McpError> {
        let started_at = Instant::now();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let registry = self.registry.lock().await;
        let policy = self.sql_policy();
        let response = match tools::sql::sql_batch(&registry, &policy, request) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("sql_batch", &resolved_db_id, started_at, &error);
                return Err(Self::map_error(error));
            }
        };
        Self::log_tool_ok("sql_batch", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
    }

    #[tool(
        name = "db_import",
        description = "Import CSV or JSON rows into a table"
    )]
    async fn db_import(
        &self,
        Parameters(request): Parameters<DbImportRequest>,
    ) -> Result<Json<ToolEnvelope<DbImportData>>, McpError> {
        let started_at = Instant::now();
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let registry = self.registry.lock().await;
        let policy = self.sql_policy();
        let response = match tools::import::db_import(&registry, &policy, request) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("db_import", &resolved_db_id, started_at, &error);
                return Err(Self::map_error(error));
            }
        };
        Self::log_tool_ok("db_import", &resolved_db_id, &response._meta, false);
        Ok(Json(response))
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
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let registry = self.registry.lock().await;
        let response = match tools::vector::vector_collection_create(
            &registry,
            request,
            self.config.max_db_bytes,
        ) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "vector_collection_create",
                    &resolved_db_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error));
            }
        };
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
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let registry = self.registry.lock().await;
        let response = match tools::vector::vector_collection_list(&registry, request) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error(
                    "vector_collection_list",
                    &resolved_db_id,
                    started_at,
                    &error,
                );
                return Err(Self::map_error(error));
            }
        };
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
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let registry = self.registry.lock().await;
        let response = match tools::vector::vector_upsert(
            &registry,
            &self.vector_runtime,
            request,
            self.config.max_db_bytes,
        ) {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("vector_upsert", &resolved_db_id, started_at, &error);
                return Err(Self::map_error(error));
            }
        };
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
        let resolved_db_id = request
            .db_id
            .clone()
            .unwrap_or_else(|| crate::DEFAULT_DB_ID.to_string());
        let registry = self.registry.lock().await;
        let response = match tools::vector::vector_search(&registry, &self.vector_runtime, request)
        {
            Ok(response) => response,
            Err(error) => {
                Self::log_tool_error("vector_search", &resolved_db_id, started_at, &error);
                return Err(Self::map_error(error));
            }
        };
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
