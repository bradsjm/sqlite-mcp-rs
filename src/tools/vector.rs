use std::cmp::Ordering;
#[cfg(test)]
use std::collections::HashMap;
use std::time::Instant;

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::DEFAULT_DB_ID;
use crate::adapters::embeddings::{EmbeddingClient, serialize_embedding_json};
use crate::adapters::ort_runtime::current_ort_dylib_path;
use crate::adapters::reranker::RerankerClient;
use crate::config::{EmbeddingConfig, RerankerConfig};
use crate::contracts::common::{ToolEnvelope, ToolHint};
use crate::contracts::vector::{
    RerankMode, VectorBackendStatus, VectorCollectionCreateData, VectorCollectionCreateRequest,
    VectorCollectionListData, VectorCollectionListRequest, VectorCollectionSummary,
    VectorConflictMode, VectorIssue, VectorMatch, VectorSearchData, VectorSearchRequest,
    VectorStatusData, VectorStatusRequest, VectorUpsertData, VectorUpsertRequest,
};
use crate::db::persistence::enforce_db_size_limit;
use crate::db::registry::DbRegistry;
use crate::errors::{AppError, AppResult};
use crate::policy::is_valid_identifier;
use crate::server::finalize::finalize_tool;

pub struct VectorRuntime {
    embedding: EmbeddingClient,
    reranker: Option<RerankerClient>,
}

impl VectorRuntime {
    pub fn new(embedding: EmbeddingConfig, reranker: Option<RerankerConfig>) -> Self {
        Self {
            embedding: EmbeddingClient::new(embedding),
            reranker: reranker.map(RerankerClient::new),
        }
    }

    #[cfg(test)]
    fn with_test_embeddings(
        embedding: EmbeddingConfig,
        reranker: Option<RerankerConfig>,
        embeddings: HashMap<String, Vec<f32>>,
    ) -> Self {
        Self {
            embedding: EmbeddingClient::new_test(embedding, embeddings),
            reranker: reranker.map(RerankerClient::new),
        }
    }

    #[cfg(test)]
    fn with_test_clients(
        embedding: EmbeddingConfig,
        reranker: Option<RerankerConfig>,
        embeddings: HashMap<String, Vec<f32>>,
        rerank_scores: Option<HashMap<String, Vec<f64>>>,
    ) -> Self {
        Self {
            embedding: EmbeddingClient::new_test(embedding, embeddings),
            reranker: match (reranker, rerank_scores) {
                (Some(config), Some(scores)) => Some(RerankerClient::new_test(config, scores)),
                (Some(config), None) => Some(RerankerClient::new(config)),
                (None, _) => None,
            },
        }
    }

    fn prewarm_embedding(&self) -> AppResult<()> {
        self.embedding.prewarm()
    }

    fn prewarm_reranker(&self) -> AppResult<()> {
        match &self.reranker {
            Some(client) => client.prewarm(),
            None => Ok(()),
        }
    }

    pub fn prewarm_startup(&self) -> AppResult<()> {
        tracing::info!(
            embedding_provider = %embedding_provider_name(self),
            embedding_model = %self.embedding.model(),
            embedding_cache_dir = %self
                .embedding
                .cache_dir_path()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "<default>".to_string()),
            "prewarming embedding runtime"
        );
        self.prewarm_embedding()?;
        tracing::info!("embedding runtime prewarm complete");

        if let Some(reranker) = &self.reranker {
            tracing::info!(
                reranker_provider = %reranker_provider_name(reranker),
                reranker_model = %reranker.model(),
                reranker_cache_dir = %reranker
                    .cache_dir_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "<default>".to_string()),
                "prewarming reranker runtime"
            );
            self.prewarm_reranker()?;
            tracing::info!("reranker runtime prewarm complete");
        } else {
            tracing::info!("reranker prewarm skipped (not configured)");
        }

        Ok(())
    }
}

pub fn vector_status(
    runtime: &VectorRuntime,
    request: VectorStatusRequest,
) -> AppResult<ToolEnvelope<VectorStatusData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let prewarm_attempted = request.prewarm;

    let mut embedding_issues = Vec::new();
    let mut reranker_issues = Vec::new();
    let mut ort_issues = Vec::new();
    let mut ort_ready = current_ort_dylib_path().is_some_and(|path| path.exists());

    if request.prewarm {
        if let Err(error) = runtime.prewarm_embedding() {
            embedding_issues.push(vector_issue_from_error("embedding_init", &error));
            ort_issues.push(vector_issue_from_error("ort_runtime", &error));
        }

        if let Err(error) = runtime.prewarm_reranker() {
            reranker_issues.push(vector_issue_from_error("reranker_init", &error));
            ort_issues.push(vector_issue_from_error("ort_runtime", &error));
        }
    }

    let ort_path = current_ort_dylib_path();
    ort_ready = ort_ready || ort_path.as_ref().is_some_and(|path| path.exists());

    let embedding = VectorBackendStatus {
        provider: embedding_provider_name(runtime).to_string(),
        model: runtime.embedding.model().to_string(),
        dimension: runtime.embedding.dimension(),
        cache_dir: runtime
            .embedding
            .cache_dir_path()
            .map(|path| path.display().to_string()),
        ready: embedding_issues.is_empty(),
        issues: embedding_issues,
    };

    let reranker = if let Some(client) = runtime.reranker.as_ref() {
        VectorBackendStatus {
            provider: reranker_provider_name(client).to_string(),
            model: client.model().to_string(),
            dimension: 0,
            cache_dir: client
                .cache_dir_path()
                .map(|path| path.display().to_string()),
            ready: reranker_issues.is_empty(),
            issues: reranker_issues,
        }
    } else {
        VectorBackendStatus {
            provider: "none".to_string(),
            model: "not_configured".to_string(),
            dimension: 0,
            cache_dir: None,
            ready: false,
            issues: vec![VectorIssue {
                stage: "reranker_init".to_string(),
                code: "RERANK_UNAVAILABLE".to_string(),
                message: "no reranker provider is configured".to_string(),
                retryable: false,
            }],
        }
    };

    let mut hints = Vec::new();
    if !ort_issues.is_empty() {
        hints.push(ToolHint {
            tool: "vector_status".to_string(),
            arguments: serde_json::json!({
                "db_id": db_id.clone(),
                "prewarm": true,
            }),
            reason: "Re-run prewarm checks after fixing network/cache/runtime availability."
                .to_string(),
        });
    }

    let summary = if embedding.ready && reranker.ready {
        "Vector runtime is ready."
    } else {
        "Vector runtime is not fully ready."
    };

    Ok(finalize_tool(
        summary,
        VectorStatusData {
            db_id,
            ort_ready,
            ort_dylib_path: ort_path.map(|path| path.display().to_string()),
            prewarm_attempted,
            embedding,
            reranker,
        },
        started,
        hints,
        None,
        None,
    ))
}

pub fn vector_collection_create(
    registry: &DbRegistry,
    runtime: &VectorRuntime,
    request: VectorCollectionCreateRequest,
    max_db_bytes: u64,
) -> AppResult<ToolEnvelope<VectorCollectionCreateData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let connection = registry.get_connection(Some(&db_id))?;
    let persisted_path = registry.persisted_path(Some(&db_id))?;

    if !is_valid_identifier(&request.collection) {
        return Err(AppError::InvalidInput(
            "collection must match ^[A-Za-z_][A-Za-z0-9_]*$".to_string(),
        ));
    }

    runtime
        .prewarm_embedding()
        .map_err(|error| vector_dependency_error("embedding_init", runtime, error))?;

    let dimension = runtime.embedding.dimension();
    if dimension == 0 {
        return Err(AppError::Dependency(
            "embedding model returned invalid dimension".to_string(),
        ));
    }

    let docs_table = format!("{}_docs", request.collection);
    let vec_table = format!("{}_vec", request.collection);

    connection.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;

    let create_result = (|| -> AppResult<bool> {
        connection.execute_batch(
            "create table if not exists _vector_collections(\n            collection text primary key,\n            docs_table text not null,\n            vec_table text not null,\n            dimension integer not null,\n            embedding_model text not null,\n            last_updated text not null\n        )",
        )?;

        let exists = connection
            .query_row(
                "select 1 from _vector_collections where collection = ?1",
                [request.collection.as_str()],
                |_| Ok(()),
            )
            .optional()?
            .is_some();

        if exists && !request.if_not_exists {
            return Err(AppError::Conflict(format!(
                "collection already exists: {}",
                request.collection
            )));
        }

        if !exists {
            let create_docs_sql = format!(
                "create table if not exists {docs_table}(id text not null unique, text text not null, metadata text null, updated_at text not null default current_timestamp)"
            );
            let create_vec_sql = format!(
                "create virtual table if not exists {vec_table} using vec0(embedding float[{dimension}])"
            );
            connection.execute_batch(&create_docs_sql)?;
            connection.execute_batch(&create_vec_sql)?;
            connection.execute(
                "insert into _vector_collections(collection, docs_table, vec_table, dimension, embedding_model, last_updated) values(?1, ?2, ?3, ?4, ?5, current_timestamp)",
                (
                    request.collection.as_str(),
                    docs_table.as_str(),
                    vec_table.as_str(),
                    dimension as i64,
                    runtime.embedding.model(),
                ),
            )?;

            enforce_db_size_limit(persisted_path.as_deref(), max_db_bytes)?;
        }

        connection.execute_batch("COMMIT")?;
        Ok(!exists)
    })();

    let created = match create_result {
        Ok(created) => created,
        Err(error) => {
            let _ = connection.execute_batch("ROLLBACK");
            return Err(error);
        }
    };

    Ok(finalize_tool(
        "Vector collection ready.",
        VectorCollectionCreateData {
            collection: request.collection,
            docs_table,
            vec_table,
            created,
        },
        started,
        Vec::new(),
        None,
        None,
    ))
}

pub fn vector_collection_list(
    registry: &DbRegistry,
    request: VectorCollectionListRequest,
) -> AppResult<ToolEnvelope<VectorCollectionListData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let connection = registry.get_connection(Some(&db_id))?;

    let metadata_exists = connection
        .query_row(
            "select 1 from sqlite_master where type='table' and name='_vector_collections'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !metadata_exists {
        return Ok(finalize_tool(
            "Listed vector collections.",
            VectorCollectionListData {
                collections: Vec::new(),
            },
            started,
            Vec::new(),
            None,
            None,
        ));
    }

    let mut statement = connection.prepare(
        "select collection, docs_table, dimension, last_updated from _vector_collections order by collection",
    )?;
    let rows = statement.query_map([], |row| {
        let collection: String = row.get(0)?;
        let docs_table: String = row.get(1)?;
        let dimension: i64 = row.get(2)?;
        let last_updated: String = row.get(3)?;
        Ok((collection, docs_table, dimension, last_updated))
    })?;

    let mut collections = Vec::new();
    for row in rows {
        let (collection, docs_table, dimension, last_updated) = row?;
        let count_sql = format!("select count(*) from {docs_table}");
        let docs_count = connection.query_row(&count_sql, [], |r| r.get::<_, i64>(0))?;

        collections.push(VectorCollectionSummary {
            collection,
            docs_count: docs_count.max(0) as usize,
            dimension: dimension.max(0) as usize,
            last_updated: Some(last_updated),
        });
    }

    Ok(finalize_tool(
        "Listed vector collections.",
        VectorCollectionListData { collections },
        started,
        Vec::new(),
        None,
        None,
    ))
}

pub fn vector_upsert(
    registry: &DbRegistry,
    runtime: &VectorRuntime,
    request: VectorUpsertRequest,
    max_db_bytes: u64,
) -> AppResult<ToolEnvelope<VectorUpsertData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let connection = registry.get_connection(Some(&db_id))?;
    let persisted_path = registry.persisted_path(Some(&db_id))?;

    if request.items.is_empty() {
        return Err(AppError::InvalidInput(
            "items must contain at least one document".to_string(),
        ));
    }

    runtime
        .prewarm_embedding()
        .map_err(|error| vector_dependency_error("embedding_init", runtime, error))?;

    let collection = load_collection(connection, &request.collection)?;
    let conflict_mode = request.on_conflict;
    let mut upserted_count = 0usize;
    let mut skipped_count = 0usize;

    connection.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;

    let upsert_result = (|| -> AppResult<()> {
        for item in request.items {
            let embedding = runtime
                .embedding
                .embed(&item.text)
                .map_err(|error| vector_dependency_error("embedding_upsert", runtime, error))?;
            if embedding.len() != collection.dimension {
                return Err(AppError::Dependency(format!(
                    "embedding dimension mismatch: expected {}, got {}",
                    collection.dimension,
                    embedding.len()
                )));
            }
            let embedding_json = serialize_embedding_json(&embedding)?;
            let metadata_json = item
                .metadata
                .as_ref()
                .map(|map| Value::Object(map.clone()).to_string());

            match conflict_mode {
                VectorConflictMode::Ignore => {
                    let docs_sql = format!(
                        "insert or ignore into {}(id, text, metadata, updated_at) values(?1, ?2, ?3, current_timestamp)",
                        collection.docs_table
                    );
                    let inserted =
                        connection.execute(&docs_sql, (&item.id, &item.text, metadata_json))?;
                    if inserted == 0 {
                        skipped_count += 1;
                        continue;
                    }
                    let rowid = load_doc_rowid(connection, &collection.docs_table, &item.id)?;
                    let vec_sql = format!(
                        "insert or replace into {}(rowid, embedding) values(?1, ?2)",
                        collection.vec_table
                    );
                    connection.execute(&vec_sql, (rowid, &embedding_json))?;
                    upserted_count += 1;
                }
                VectorConflictMode::Replace => {
                    let docs_sql = format!(
                        "insert into {}(id, text, metadata, updated_at) values(?1, ?2, ?3, current_timestamp) on conflict(id) do update set text = excluded.text, metadata = excluded.metadata, updated_at = current_timestamp",
                        collection.docs_table
                    );
                    connection.execute(&docs_sql, (&item.id, &item.text, metadata_json))?;
                    let rowid = load_doc_rowid(connection, &collection.docs_table, &item.id)?;

                    let vec_sql = format!(
                        "insert or replace into {}(rowid, embedding) values(?1, ?2)",
                        collection.vec_table
                    );
                    connection.execute(&vec_sql, (rowid, &embedding_json))?;
                    upserted_count += 1;
                }
                VectorConflictMode::UpdateMetadata => {
                    let exists_sql =
                        format!("select 1 from {} where id = ?1", collection.docs_table);
                    let exists = connection
                        .query_row(&exists_sql, [item.id.as_str()], |_| Ok(()))
                        .optional()?
                        .is_some();

                    if exists {
                        let update_sql = format!(
                            "update {} set metadata = ?1, updated_at = current_timestamp where id = ?2",
                            collection.docs_table
                        );
                        connection.execute(&update_sql, (&metadata_json, &item.id))?;
                    } else {
                        let docs_sql = format!(
                            "insert into {}(id, text, metadata, updated_at) values(?1, ?2, ?3, current_timestamp)",
                            collection.docs_table
                        );
                        connection.execute(&docs_sql, (&item.id, &item.text, metadata_json))?;
                        let rowid = load_doc_rowid(connection, &collection.docs_table, &item.id)?;
                        let vec_sql = format!(
                            "insert or replace into {}(rowid, embedding) values(?1, ?2)",
                            collection.vec_table
                        );
                        connection.execute(&vec_sql, (rowid, &embedding_json))?;
                    }

                    upserted_count += 1;
                }
            }
        }

        touch_collection(connection, &request.collection)?;
        enforce_db_size_limit(persisted_path.as_deref(), max_db_bytes)?;
        connection.execute_batch("COMMIT")?;
        Ok(())
    })();

    if let Err(error) = upsert_result {
        let _ = connection.execute_batch("ROLLBACK");
        return Err(error);
    }

    Ok(finalize_tool(
        "Vector upsert completed.",
        VectorUpsertData {
            upserted_count,
            skipped_count,
        },
        started,
        Vec::new(),
        None,
        None,
    ))
}

pub fn vector_search(
    registry: &DbRegistry,
    runtime: &VectorRuntime,
    request: VectorSearchRequest,
    max_top_k: usize,
    max_rerank_fetch_k: usize,
) -> AppResult<ToolEnvelope<VectorSearchData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let connection = registry.get_connection(Some(&db_id))?;

    let collection = load_collection(connection, &request.collection)?;
    runtime
        .prewarm_embedding()
        .map_err(|error| vector_dependency_error("embedding_init", runtime, error))?;
    let query_embedding = runtime
        .embedding
        .embed(&request.query_text)
        .map_err(|error| vector_dependency_error("embedding_query", runtime, error))?;
    if query_embedding.len() != collection.dimension {
        return Err(AppError::Dependency(format!(
            "query embedding dimension mismatch: expected {}, got {}",
            collection.dimension,
            query_embedding.len()
        )));
    }

    let top_k = request.top_k.unwrap_or(10);
    if top_k == 0 {
        return Err(AppError::InvalidInput(
            "top_k must be greater than zero".to_string(),
        ));
    }
    if top_k > max_top_k {
        return Err(AppError::LimitExceeded(format!(
            "top_k exceeds maximum of {max_top_k}"
        )));
    }

    let rerank_mode = request.rerank;
    let fetch_k = request.rerank_fetch_k.unwrap_or(top_k);
    if fetch_k == 0 {
        return Err(AppError::InvalidInput(
            "rerank_fetch_k must be greater than zero".to_string(),
        ));
    }
    if fetch_k > max_rerank_fetch_k {
        return Err(AppError::LimitExceeded(format!(
            "rerank_fetch_k exceeds maximum of {max_rerank_fetch_k}"
        )));
    }
    if rerank_mode == RerankMode::On && fetch_k < top_k {
        return Err(AppError::InvalidInput(
            "rerank_fetch_k must be greater than or equal to top_k when rerank is enabled"
                .to_string(),
        ));
    }

    let target_k = if rerank_mode == RerankMode::On {
        fetch_k
    } else {
        top_k
    };
    let candidate_limit = if request.filter.is_some() {
        target_k.saturating_mul(10).max(100)
    } else {
        target_k
    };

    let query_vector = serialize_embedding_json(&query_embedding)?;
    let knn_sql = format!(
        "select d.id, d.text, d.metadata, v.distance from {vec_table} v inner join {docs_table} d on d.rowid = v.rowid where v.embedding match ?1 and v.k = ?2 order by v.distance",
        vec_table = collection.vec_table,
        docs_table = collection.docs_table,
    );
    let mut statement = connection.prepare(&knn_sql)?;
    let rows = statement.query_map((query_vector, candidate_limit as i64), |row| {
        let id: String = row.get(0)?;
        let text: String = row.get(1)?;
        let metadata_raw: Option<String> = row.get(2)?;
        let distance: f64 = row.get(3)?;
        Ok((id, text, metadata_raw, distance))
    })?;

    let mut candidates = Vec::new();
    for row in rows {
        let (id, text, metadata_raw, distance) = row?;
        let metadata = parse_metadata(metadata_raw)?;
        if !metadata_matches(request.filter.as_ref(), metadata.as_ref()) {
            continue;
        }
        candidates.push(SearchCandidate {
            id,
            text,
            metadata,
            distance,
            score: None,
        });
    }

    candidates.sort_by(|left, right| {
        left.distance
            .partial_cmp(&right.distance)
            .unwrap_or(Ordering::Equal)
    });

    let mut issues = Vec::new();
    let mut reranked = false;
    let mut rerank_model = String::new();

    let mut selected: Vec<SearchCandidate> = if rerank_mode == RerankMode::On {
        candidates.iter().take(fetch_k).cloned().collect()
    } else {
        candidates.iter().take(top_k).cloned().collect()
    };

    if rerank_mode == RerankMode::On {
        if let Some(reranker) = &runtime.reranker {
            let docs = selected
                .iter()
                .map(|candidate| candidate.text.clone())
                .collect::<Vec<_>>();

            match reranker.rerank(&request.query_text, &docs) {
                Ok(scores) => {
                    if scores.len() != selected.len() {
                        issues.push(VectorIssue {
                            stage: "rerank".to_string(),
                            code: "RERANK_FAILED".to_string(),
                            message: format!(
                                "reranker returned {} scores for {} candidates",
                                scores.len(),
                                selected.len()
                            ),
                            retryable: true,
                        });
                        selected.sort_by(|left, right| {
                            left.distance
                                .partial_cmp(&right.distance)
                                .unwrap_or(Ordering::Equal)
                        });
                        selected.truncate(top_k);
                    } else {
                        for (candidate, score) in selected.iter_mut().zip(scores.into_iter()) {
                            candidate.score = Some(score);
                        }
                        selected.sort_by(|left, right| {
                            right
                                .score
                                .partial_cmp(&left.score)
                                .unwrap_or(Ordering::Equal)
                        });
                        selected.truncate(top_k);
                        reranked = true;
                        rerank_model = reranker.model().to_string();
                    }
                }
                Err(error) => {
                    issues.push(VectorIssue {
                        stage: "rerank".to_string(),
                        code: "RERANK_FAILED".to_string(),
                        message: vector_dependency_message("reranker", &error.to_string()),
                        retryable: true,
                    });
                    selected.sort_by(|left, right| {
                        left.distance
                            .partial_cmp(&right.distance)
                            .unwrap_or(Ordering::Equal)
                    });
                    selected.truncate(top_k);
                }
            }
        } else {
            issues.push(VectorIssue {
                stage: "rerank".to_string(),
                code: "RERANK_UNAVAILABLE".to_string(),
                message: "rerank requested but no reranker provider is configured".to_string(),
                retryable: false,
            });
            selected.sort_by(|left, right| {
                left.distance
                    .partial_cmp(&right.distance)
                    .unwrap_or(Ordering::Equal)
            });
            selected.truncate(top_k);
        }
    }

    let matches = selected
        .into_iter()
        .map(|candidate| VectorMatch {
            id: candidate.id,
            distance: candidate.distance,
            score: candidate.score,
            text: request.include_text.then_some(candidate.text),
            metadata: request
                .include_metadata
                .then_some(candidate.metadata)
                .flatten(),
        })
        .collect::<Vec<_>>();

    Ok(finalize_tool(
        "Vector search completed.",
        VectorSearchData {
            matches,
            truncated: candidates.len() > top_k,
            reranked,
            rerank_model,
            issues,
        },
        started,
        Vec::new(),
        None,
        None,
    ))
}

#[derive(Debug, Clone)]
struct CollectionMeta {
    docs_table: String,
    vec_table: String,
    dimension: usize,
}

#[derive(Debug, Clone)]
struct SearchCandidate {
    id: String,
    text: String,
    metadata: Option<serde_json::Map<String, Value>>,
    distance: f64,
    score: Option<f64>,
}

fn load_collection(
    connection: &rusqlite::Connection,
    collection: &str,
) -> AppResult<CollectionMeta> {
    connection
        .query_row(
            "select docs_table, vec_table, dimension from _vector_collections where collection = ?1",
            [collection],
            |row| {
                let docs_table: String = row.get(0)?;
                let vec_table: String = row.get(1)?;
                let dimension: i64 = row.get(2)?;
                Ok(CollectionMeta {
                    docs_table,
                    vec_table,
                    dimension: dimension.max(0) as usize,
                })
            },
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("unknown collection: {collection}")))
}

fn load_doc_rowid(connection: &rusqlite::Connection, docs_table: &str, id: &str) -> AppResult<i64> {
    let sql = format!("select rowid from {docs_table} where id = ?1");
    connection
        .query_row(&sql, [id], |row| row.get::<_, i64>(0))
        .map_err(Into::into)
}

fn touch_collection(connection: &rusqlite::Connection, collection: &str) -> AppResult<()> {
    connection.execute(
        "update _vector_collections set last_updated = current_timestamp where collection = ?1",
        [collection],
    )?;
    Ok(())
}

fn parse_metadata(raw: Option<String>) -> AppResult<Option<serde_json::Map<String, Value>>> {
    match raw {
        Some(value) => serde_json::from_str::<serde_json::Map<String, Value>>(&value)
            .map(Some)
            .map_err(|error| {
                AppError::Dependency(format!("invalid stored metadata payload: {error}"))
            }),
        None => Ok(None),
    }
}

fn metadata_matches(
    filter: Option<&serde_json::Map<String, Value>>,
    metadata: Option<&serde_json::Map<String, Value>>,
) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    if filter.is_empty() {
        return true;
    }

    let Some(metadata) = metadata else {
        return false;
    };
    filter
        .iter()
        .all(|(key, value)| metadata.get(key) == Some(value))
}

fn vector_dependency_error(stage: &str, runtime: &VectorRuntime, error: AppError) -> AppError {
    let ort_path = current_ort_dylib_path().map(|path| path.display().to_string());
    let message = vector_dependency_message(stage, &error.to_string());
    AppError::Dependency(format!(
        "{message}; embedding_model={}; embedding_cache_dir={}; ort_dylib_path={}; run vector_status with {{\"prewarm\":true}} for diagnostics",
        runtime.embedding.model(),
        runtime
            .embedding
            .cache_dir_path()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "<default>".to_string()),
        ort_path.unwrap_or_else(|| "<unset>".to_string()),
    ))
}

fn vector_dependency_message(stage: &str, raw: &str) -> String {
    if raw.contains("Failed to retrieve onnx/model.onnx") {
        return format!(
            "{stage} failed because model artifacts were not retrievable (Failed to retrieve onnx/model.onnx)"
        );
    }
    if raw.contains("ORT runtime not initialized") {
        return format!("{stage} failed because ORT runtime is not initialized");
    }
    if raw.contains("failed downloading ONNX Runtime") {
        return format!("{stage} failed while downloading ONNX Runtime");
    }
    format!("{stage} failed: {raw}")
}

fn vector_issue_from_error(stage: &str, error: &AppError) -> VectorIssue {
    VectorIssue {
        stage: stage.to_string(),
        code: "DEPENDENCY_ERROR".to_string(),
        message: vector_dependency_message(stage, &error.to_string()),
        retryable: true,
    }
}

fn embedding_provider_name(runtime: &VectorRuntime) -> &'static str {
    match runtime.embedding.provider() {
        crate::config::EmbeddingProvider::Fastembed => "fastembed",
    }
}

fn reranker_provider_name(client: &RerankerClient) -> &'static str {
    match client.provider() {
        crate::config::RerankerProvider::Fastembed => "fastembed",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use serde_json::{Map, Value};

    use crate::config::{EmbeddingConfig, EmbeddingProvider, RerankerConfig, RerankerProvider};
    use crate::contracts::db::DbMode;
    use crate::contracts::vector::{
        RerankMode, VectorCollectionCreateRequest, VectorConflictMode, VectorDocument,
        VectorSearchRequest, VectorStatusRequest, VectorUpsertRequest,
    };
    use crate::db::registry::DbRegistry;
    use crate::errors::AppError;

    use super::{
        VectorRuntime, vector_collection_create, vector_search, vector_status, vector_upsert,
    };

    fn embedding_config(dimension: usize) -> EmbeddingConfig {
        EmbeddingConfig {
            provider: EmbeddingProvider::Fastembed,
            model: "BAAI/bge-small-en-v1.5".to_string(),
            cache_dir: None,
            dimension,
        }
    }

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

    fn reranker_config() -> RerankerConfig {
        RerankerConfig {
            provider: RerankerProvider::Fastembed,
            model: "BAAI/bge-reranker-base".to_string(),
            cache_dir: None,
        }
    }

    #[test]
    fn vector_happy_path_creates_upserts_and_searches() {
        let registry = setup_registry();
        let runtime = VectorRuntime::with_test_embeddings(
            embedding_config(3),
            None,
            HashMap::from([
                ("doc-alpha".to_string(), vec![1.0, 0.0, 0.0]),
                ("doc-beta".to_string(), vec![0.0, 1.0, 0.0]),
                ("query-alpha".to_string(), vec![1.0, 0.0, 0.0]),
            ]),
        );

        let created = vector_collection_create(
            &registry,
            &runtime,
            VectorCollectionCreateRequest {
                db_id: None,
                collection: "items".to_string(),
                if_not_exists: false,
            },
            u64::MAX,
        )
        .expect("collection create should succeed");
        assert!(created.data.created);

        let mut alpha_metadata = Map::new();
        alpha_metadata.insert("topic".to_string(), Value::String("alpha".to_string()));
        let mut beta_metadata = Map::new();
        beta_metadata.insert("topic".to_string(), Value::String("beta".to_string()));

        let upserted = vector_upsert(
            &registry,
            &runtime,
            VectorUpsertRequest {
                db_id: None,
                collection: "items".to_string(),
                on_conflict: VectorConflictMode::Replace,
                items: vec![
                    VectorDocument {
                        id: "a".to_string(),
                        text: "doc-alpha".to_string(),
                        metadata: Some(alpha_metadata),
                    },
                    VectorDocument {
                        id: "b".to_string(),
                        text: "doc-beta".to_string(),
                        metadata: Some(beta_metadata),
                    },
                ],
            },
            u64::MAX,
        )
        .expect("upsert should succeed");
        assert_eq!(upserted.data.upserted_count, 2);
        assert_eq!(upserted.data.skipped_count, 0);

        let mut filter = Map::new();
        filter.insert("topic".to_string(), Value::String("alpha".to_string()));
        let searched = vector_search(
            &registry,
            &runtime,
            VectorSearchRequest {
                db_id: None,
                collection: "items".to_string(),
                query_text: "query-alpha".to_string(),
                top_k: Some(2),
                include_text: true,
                include_metadata: true,
                filter: Some(filter),
                rerank: RerankMode::Off,
                rerank_fetch_k: None,
            },
            200,
            500,
        )
        .expect("search should succeed");

        assert_eq!(searched.data.matches.len(), 1);
        let first = &searched.data.matches[0];
        assert_eq!(first.id, "a");
        assert_eq!(first.text.as_deref(), Some("doc-alpha"));
        assert_eq!(
            first
                .metadata
                .as_ref()
                .and_then(|map| map.get("topic"))
                .and_then(Value::as_str),
            Some("alpha")
        );
        assert!(!searched.data.reranked);
        assert!(searched.data.issues.is_empty());
    }

    #[test]
    fn vector_upsert_fails_on_dimension_mismatch() {
        let registry = setup_registry();
        let runtime = VectorRuntime::with_test_embeddings(
            embedding_config(3),
            None,
            HashMap::from([("doc-alpha".to_string(), vec![1.0, 0.0])]),
        );

        vector_collection_create(
            &registry,
            &runtime,
            VectorCollectionCreateRequest {
                db_id: None,
                collection: "items".to_string(),
                if_not_exists: false,
            },
            u64::MAX,
        )
        .expect("collection create should succeed");

        let error = vector_upsert(
            &registry,
            &runtime,
            VectorUpsertRequest {
                db_id: None,
                collection: "items".to_string(),
                on_conflict: VectorConflictMode::Replace,
                items: vec![VectorDocument {
                    id: "a".to_string(),
                    text: "doc-alpha".to_string(),
                    metadata: None,
                }],
            },
            u64::MAX,
        )
        .expect_err("dimension mismatch must fail");

        match error {
            AppError::Dependency(message) => {
                assert!(message.contains("embedding dimension mismatch"));
            }
            other => panic!("expected dependency error, got: {other}"),
        }
    }

    #[test]
    fn vector_search_reports_rerank_unavailable() {
        let registry = setup_registry();
        let runtime = VectorRuntime::with_test_embeddings(
            embedding_config(3),
            None,
            HashMap::from([
                ("doc-alpha".to_string(), vec![1.0, 0.0, 0.0]),
                ("doc-beta".to_string(), vec![0.0, 1.0, 0.0]),
                ("query-alpha".to_string(), vec![1.0, 0.0, 0.0]),
            ]),
        );

        vector_collection_create(
            &registry,
            &runtime,
            VectorCollectionCreateRequest {
                db_id: None,
                collection: "items".to_string(),
                if_not_exists: false,
            },
            u64::MAX,
        )
        .expect("collection create should succeed");

        vector_upsert(
            &registry,
            &runtime,
            VectorUpsertRequest {
                db_id: None,
                collection: "items".to_string(),
                on_conflict: VectorConflictMode::Replace,
                items: vec![
                    VectorDocument {
                        id: "a".to_string(),
                        text: "doc-alpha".to_string(),
                        metadata: None,
                    },
                    VectorDocument {
                        id: "b".to_string(),
                        text: "doc-beta".to_string(),
                        metadata: None,
                    },
                ],
            },
            u64::MAX,
        )
        .expect("upsert should succeed");

        let searched = vector_search(
            &registry,
            &runtime,
            VectorSearchRequest {
                db_id: None,
                collection: "items".to_string(),
                query_text: "query-alpha".to_string(),
                top_k: Some(2),
                include_text: false,
                include_metadata: false,
                filter: None,
                rerank: RerankMode::On,
                rerank_fetch_k: Some(2),
            },
            200,
            500,
        )
        .expect("search should succeed");

        assert!(!searched.data.reranked);
        assert_eq!(searched.data.issues.len(), 1);
        assert_eq!(searched.data.issues[0].code, "RERANK_UNAVAILABLE");
    }

    #[test]
    fn vector_search_uses_reranker_scores_when_available() {
        let registry = setup_registry();
        let runtime = VectorRuntime::with_test_clients(
            embedding_config(3),
            Some(reranker_config()),
            HashMap::from([
                ("doc-alpha".to_string(), vec![1.0, 0.0, 0.0]),
                ("doc-beta".to_string(), vec![0.0, 1.0, 0.0]),
                ("query-alpha".to_string(), vec![1.0, 0.0, 0.0]),
            ]),
            Some(HashMap::from([("query-alpha".to_string(), vec![0.1, 0.9])])),
        );

        vector_collection_create(
            &registry,
            &runtime,
            VectorCollectionCreateRequest {
                db_id: None,
                collection: "items".to_string(),
                if_not_exists: false,
            },
            u64::MAX,
        )
        .expect("collection create should succeed");

        vector_upsert(
            &registry,
            &runtime,
            VectorUpsertRequest {
                db_id: None,
                collection: "items".to_string(),
                on_conflict: VectorConflictMode::Replace,
                items: vec![
                    VectorDocument {
                        id: "a".to_string(),
                        text: "doc-alpha".to_string(),
                        metadata: None,
                    },
                    VectorDocument {
                        id: "b".to_string(),
                        text: "doc-beta".to_string(),
                        metadata: None,
                    },
                ],
            },
            u64::MAX,
        )
        .expect("upsert should succeed");

        let searched = vector_search(
            &registry,
            &runtime,
            VectorSearchRequest {
                db_id: None,
                collection: "items".to_string(),
                query_text: "query-alpha".to_string(),
                top_k: Some(1),
                include_text: false,
                include_metadata: false,
                filter: None,
                rerank: RerankMode::On,
                rerank_fetch_k: Some(2),
            },
            200,
            500,
        )
        .expect("search should succeed");

        assert!(searched.data.reranked);
        assert_eq!(searched.data.rerank_model, "BAAI/bge-reranker-base");
        assert!(searched.data.issues.is_empty());
        assert_eq!(searched.data.matches.len(), 1);
        assert_eq!(searched.data.matches[0].id, "b");
        assert_eq!(searched.data.matches[0].score, Some(0.9));
    }

    #[test]
    fn vector_search_rejects_top_k_above_limit() {
        let registry = setup_registry();
        let runtime = VectorRuntime::with_test_embeddings(
            embedding_config(3),
            None,
            HashMap::from([
                ("doc-alpha".to_string(), vec![1.0, 0.0, 0.0]),
                ("query-alpha".to_string(), vec![1.0, 0.0, 0.0]),
            ]),
        );

        vector_collection_create(
            &registry,
            &runtime,
            VectorCollectionCreateRequest {
                db_id: None,
                collection: "items".to_string(),
                if_not_exists: false,
            },
            u64::MAX,
        )
        .expect("collection create should succeed");

        vector_upsert(
            &registry,
            &runtime,
            VectorUpsertRequest {
                db_id: None,
                collection: "items".to_string(),
                on_conflict: VectorConflictMode::Replace,
                items: vec![VectorDocument {
                    id: "a".to_string(),
                    text: "doc-alpha".to_string(),
                    metadata: None,
                }],
            },
            u64::MAX,
        )
        .expect("upsert should succeed");

        let error = vector_search(
            &registry,
            &runtime,
            VectorSearchRequest {
                db_id: None,
                collection: "items".to_string(),
                query_text: "query-alpha".to_string(),
                top_k: Some(51),
                include_text: false,
                include_metadata: false,
                filter: None,
                rerank: RerankMode::Off,
                rerank_fetch_k: None,
            },
            50,
            100,
        )
        .expect_err("top_k over configured cap must fail");

        match error {
            AppError::LimitExceeded(message) => {
                assert!(message.contains("top_k exceeds maximum"));
            }
            other => panic!("expected limit exceeded, got: {other}"),
        }
    }

    #[test]
    fn vector_search_rejects_rerank_fetch_k_below_top_k() {
        let registry = setup_registry();
        let runtime = VectorRuntime::with_test_embeddings(
            embedding_config(3),
            None,
            HashMap::from([
                ("doc-alpha".to_string(), vec![1.0, 0.0, 0.0]),
                ("query-alpha".to_string(), vec![1.0, 0.0, 0.0]),
            ]),
        );

        vector_collection_create(
            &registry,
            &runtime,
            VectorCollectionCreateRequest {
                db_id: None,
                collection: "items".to_string(),
                if_not_exists: false,
            },
            u64::MAX,
        )
        .expect("collection create should succeed");

        vector_upsert(
            &registry,
            &runtime,
            VectorUpsertRequest {
                db_id: None,
                collection: "items".to_string(),
                on_conflict: VectorConflictMode::Replace,
                items: vec![VectorDocument {
                    id: "a".to_string(),
                    text: "doc-alpha".to_string(),
                    metadata: None,
                }],
            },
            u64::MAX,
        )
        .expect("upsert should succeed");

        let error = vector_search(
            &registry,
            &runtime,
            VectorSearchRequest {
                db_id: None,
                collection: "items".to_string(),
                query_text: "query-alpha".to_string(),
                top_k: Some(5),
                include_text: false,
                include_metadata: false,
                filter: None,
                rerank: RerankMode::On,
                rerank_fetch_k: Some(3),
            },
            50,
            100,
        )
        .expect_err("rerank_fetch_k below top_k must fail");

        match error {
            AppError::InvalidInput(message) => {
                assert!(message.contains("rerank_fetch_k must be greater than or equal to top_k"));
            }
            other => panic!("expected invalid input, got: {other}"),
        }
    }

    #[test]
    fn vector_status_reports_ready_with_test_embeddings() {
        let runtime = VectorRuntime::with_test_embeddings(
            embedding_config(3),
            None,
            HashMap::from([("query-alpha".to_string(), vec![1.0, 0.0, 0.0])]),
        );

        let status = vector_status(
            &runtime,
            VectorStatusRequest {
                db_id: None,
                prewarm: true,
            },
        )
        .expect("status call should succeed");

        assert_eq!(status.data.db_id, "default");
        assert!(status.data.prewarm_attempted);
        assert!(status.data.embedding.ready);
        assert!(status.data.embedding.issues.is_empty());
        assert!(!status.data.reranker.ready);
        assert_eq!(status.data.reranker.provider, "none");
    }

    #[test]
    fn vector_status_reports_reranker_ready_with_test_clients() {
        let runtime = VectorRuntime::with_test_clients(
            embedding_config(3),
            Some(reranker_config()),
            HashMap::from([("query-alpha".to_string(), vec![1.0, 0.0, 0.0])]),
            Some(HashMap::from([("query-alpha".to_string(), vec![0.2, 0.1])])),
        );

        let status = vector_status(
            &runtime,
            VectorStatusRequest {
                db_id: Some("test_db".to_string()),
                prewarm: true,
            },
        )
        .expect("status call should succeed");

        assert_eq!(status.data.db_id, "test_db");
        assert!(status.data.embedding.ready);
        assert!(status.data.embedding.issues.is_empty());
        let reranker = &status.data.reranker;
        assert!(reranker.ready);
        assert!(reranker.issues.is_empty());
    }
}
