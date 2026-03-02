use std::cmp::Ordering;
use std::time::Instant;

use rusqlite::OptionalExtension;
use serde_json::Value;

use crate::DEFAULT_DB_ID;
use crate::adapters::embeddings::{EmbeddingClient, parse_embedding, serialize_embedding};
use crate::adapters::reranker::RerankerClient;
use crate::config::{EmbeddingConfig, RerankerConfig};
use crate::contracts::common::ToolEnvelope;
use crate::contracts::vector::{
    RerankMode, VectorCollectionCreateData, VectorCollectionCreateRequest,
    VectorCollectionListData, VectorCollectionListRequest, VectorCollectionSummary,
    VectorConflictMode, VectorIssue, VectorMatch, VectorSearchData, VectorSearchRequest,
    VectorUpsertData, VectorUpsertRequest,
};
use crate::db::persistence::enforce_db_size_limit;
use crate::db::registry::DbRegistry;
use crate::errors::{AppError, AppResult};
use crate::policy::is_valid_identifier;
use crate::server::finalize::finalize_tool;

#[derive(Debug, Clone)]
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
}

pub fn vector_collection_create(
    registry: &DbRegistry,
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
    if request.dimension == 0 {
        return Err(AppError::InvalidInput(
            "dimension must be greater than zero".to_string(),
        ));
    }

    let docs_table = format!("{}_docs", request.collection);
    let vec_table = format!("{}_vec", request.collection);

    connection.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;

    let create_result = (|| -> AppResult<bool> {
        connection.execute_batch(
            "create table if not exists _vector_collections(\n            collection text primary key,\n            docs_table text not null,\n            vec_table text not null,\n            dimension integer not null,\n            last_updated text not null\n        )",
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
                "create table if not exists {docs_table}(id text primary key, text text not null, metadata text null, updated_at text not null default current_timestamp)"
            );
            let create_vec_sql = format!(
                "create table if not exists {vec_table}(id text primary key, embedding text not null)"
            );
            connection.execute_batch(&create_docs_sql)?;
            connection.execute_batch(&create_vec_sql)?;
            connection.execute(
                "insert into _vector_collections(collection, docs_table, vec_table, dimension, last_updated) values(?1, ?2, ?3, ?4, current_timestamp)",
                (
                    request.collection.as_str(),
                    docs_table.as_str(),
                    vec_table.as_str(),
                    request.dimension as i64,
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

    let collection = load_collection(connection, &request.collection)?;
    let conflict_mode = request.on_conflict.unwrap_or(VectorConflictMode::Replace);
    let mut upserted_count = 0usize;
    let mut skipped_count = 0usize;

    connection.execute_batch("BEGIN IMMEDIATE TRANSACTION")?;

    let upsert_result = (|| -> AppResult<()> {
        for item in request.items {
            let embedding = runtime.embedding.embed(&item.text)?;
            if embedding.len() != collection.dimension {
                return Err(AppError::Dependency(format!(
                    "embedding dimension mismatch: expected {}, got {}",
                    collection.dimension,
                    embedding.len()
                )));
            }

            let metadata_json = item
                .metadata
                .as_ref()
                .map(|map| Value::Object(map.clone()).to_string());
            let embedding_json = serialize_embedding(&embedding)?;

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
                    } else {
                        let vec_sql = format!(
                            "insert or ignore into {}(id, embedding) values(?1, ?2)",
                            collection.vec_table
                        );
                        connection.execute(&vec_sql, (&item.id, &embedding_json))?;
                        upserted_count += 1;
                    }
                }
                VectorConflictMode::Replace => {
                    let docs_sql = format!(
                        "insert into {}(id, text, metadata, updated_at) values(?1, ?2, ?3, current_timestamp) on conflict(id) do update set text = excluded.text, metadata = excluded.metadata, updated_at = current_timestamp",
                        collection.docs_table
                    );
                    connection.execute(&docs_sql, (&item.id, &item.text, metadata_json))?;

                    let vec_sql = format!(
                        "insert into {}(id, embedding) values(?1, ?2) on conflict(id) do update set embedding = excluded.embedding",
                        collection.vec_table
                    );
                    connection.execute(&vec_sql, (&item.id, &embedding_json))?;
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

                        let vec_sql = format!(
                            "insert into {}(id, embedding) values(?1, ?2)",
                            collection.vec_table
                        );
                        connection.execute(&vec_sql, (&item.id, &embedding_json))?;
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
) -> AppResult<ToolEnvelope<VectorSearchData>> {
    let started = Instant::now();
    let db_id = request.db_id.unwrap_or_else(|| DEFAULT_DB_ID.to_string());
    let connection = registry.get_connection(Some(&db_id))?;

    let collection = load_collection(connection, &request.collection)?;
    let query_embedding = runtime.embedding.embed(&request.query_text)?;
    if query_embedding.len() != collection.dimension {
        return Err(AppError::Dependency(format!(
            "query embedding dimension mismatch: expected {}, got {}",
            collection.dimension,
            query_embedding.len()
        )));
    }

    let sql = format!(
        "select d.id, d.text, d.metadata, v.embedding from {} d inner join {} v on v.id = d.id",
        collection.docs_table, collection.vec_table
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map([], |row| {
        let id: String = row.get(0)?;
        let text: String = row.get(1)?;
        let metadata_raw: Option<String> = row.get(2)?;
        let embedding_raw: String = row.get(3)?;
        Ok((id, text, metadata_raw, embedding_raw))
    })?;

    let mut candidates = Vec::new();
    for row in rows {
        let (id, text, metadata_raw, embedding_raw) = row?;
        let metadata = parse_metadata(metadata_raw)?;
        if !metadata_matches(request.filter.as_ref(), metadata.as_ref()) {
            continue;
        }

        let embedding = parse_embedding(&embedding_raw)?;
        if embedding.len() != collection.dimension {
            continue;
        }
        let Some(distance) = cosine_distance(&query_embedding, &embedding) else {
            continue;
        };

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

    let top_k = request.top_k.unwrap_or(10).max(1);
    let rerank_mode = request.rerank.unwrap_or(RerankMode::Off);
    let fetch_k = request.rerank_fetch_k.unwrap_or(top_k).max(top_k);
    let mut issues = Vec::new();
    let mut reranked = false;
    let mut rerank_model = None;

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
                    rerank_model = Some(reranker.model().to_string());
                }
                Err(error) => {
                    issues.push(VectorIssue {
                        stage: "rerank".to_string(),
                        code: "RERANK_FAILED".to_string(),
                        message: error.to_string(),
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

fn cosine_distance(left: &[f32], right: &[f32]) -> Option<f64> {
    if left.is_empty() || left.len() != right.len() {
        return None;
    }

    let mut dot = 0.0f64;
    let mut left_norm = 0.0f64;
    let mut right_norm = 0.0f64;

    for (left_value, right_value) in left.iter().zip(right.iter()) {
        let l = *left_value as f64;
        let r = *right_value as f64;
        dot += l * r;
        left_norm += l * l;
        right_norm += r * r;
    }

    if left_norm == 0.0 || right_norm == 0.0 {
        return None;
    }

    Some(1.0 - (dot / (left_norm.sqrt() * right_norm.sqrt())))
}
