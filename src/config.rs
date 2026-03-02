use std::path::PathBuf;

use crate::errors::AppError;
#[cfg(feature = "vector")]
use fastembed::RerankerModel;
#[cfg(feature = "vector")]
use fastembed::{EmbeddingModel, ModelTrait};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub persist_root: Option<PathBuf>,
    pub log_level: String,
    pub max_sql_length: usize,
    pub max_statements: usize,
    pub max_rows: usize,
    pub max_bytes: usize,
    pub max_db_bytes: u64,
    pub max_persisted_list_entries: usize,
    pub cursor_ttl_seconds: u64,
    pub cursor_capacity: usize,
    pub queue_wait_timeout_ms_default: u64,
    pub queue_wait_timeout_ms_max: u64,
    pub queue_poll_interval_ms_default: u64,
    pub queue_poll_interval_ms_min: u64,
    pub queue_poll_interval_ms_max: u64,
    #[cfg(feature = "vector")]
    pub max_vector_top_k: usize,
    #[cfg(feature = "vector")]
    pub max_rerank_fetch_k: usize,
    #[cfg(feature = "vector")]
    pub embedding: EmbeddingConfig,
    #[cfg(feature = "vector")]
    pub reranker: Option<RerankerConfig>,
}

#[cfg(feature = "vector")]
#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub provider: EmbeddingProvider,
    pub model: String,
    pub cache_dir: Option<PathBuf>,
    pub dimension: usize,
}

#[cfg(feature = "vector")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    Fastembed,
}

#[cfg(feature = "vector")]
#[derive(Debug, Clone)]
pub struct RerankerConfig {
    pub provider: RerankerProvider,
    pub model: String,
    pub cache_dir: Option<PathBuf>,
}

#[cfg(feature = "vector")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankerProvider {
    Fastembed,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, AppError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup<F>(lookup: F) -> Result<Self, AppError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let config = Self {
            persist_root: optional_path(&lookup, "SQLITE_PERSIST_ROOT")?,
            log_level: log_level(&lookup, "SQLITE_LOG_LEVEL", "info")?,
            max_sql_length: positive_usize(&lookup, "SQLITE_MAX_SQL_LENGTH", 20_000)?,
            max_statements: positive_usize(&lookup, "SQLITE_MAX_STATEMENTS", 50)?,
            max_rows: positive_usize(&lookup, "SQLITE_MAX_ROWS", 500)?,
            max_bytes: positive_usize(&lookup, "SQLITE_MAX_BYTES", 1_048_576)?,
            max_db_bytes: positive_u64(&lookup, "SQLITE_MAX_DB_BYTES", 100_000_000)?,
            max_persisted_list_entries: positive_usize(
                &lookup,
                "SQLITE_MAX_PERSISTED_LIST_ENTRIES",
                500,
            )?,
            cursor_ttl_seconds: positive_u64(&lookup, "SQLITE_CURSOR_TTL_SECONDS", 600)?,
            cursor_capacity: positive_usize(&lookup, "SQLITE_CURSOR_CAPACITY", 500)?,
            queue_wait_timeout_ms_default: positive_u64(
                &lookup,
                "SQLITE_QUEUE_WAIT_TIMEOUT_MS_DEFAULT",
                30_000,
            )?,
            queue_wait_timeout_ms_max: positive_u64(
                &lookup,
                "SQLITE_QUEUE_WAIT_TIMEOUT_MS_MAX",
                120_000,
            )?,
            queue_poll_interval_ms_default: positive_u64(
                &lookup,
                "SQLITE_QUEUE_POLL_INTERVAL_MS_DEFAULT",
                250,
            )?,
            queue_poll_interval_ms_min: positive_u64(
                &lookup,
                "SQLITE_QUEUE_POLL_INTERVAL_MS_MIN",
                50,
            )?,
            queue_poll_interval_ms_max: positive_u64(
                &lookup,
                "SQLITE_QUEUE_POLL_INTERVAL_MS_MAX",
                5_000,
            )?,
            #[cfg(feature = "vector")]
            max_vector_top_k: positive_usize(&lookup, "SQLITE_MAX_VECTOR_TOP_K", 200)?,
            #[cfg(feature = "vector")]
            max_rerank_fetch_k: positive_usize(&lookup, "SQLITE_MAX_RERANK_FETCH_K", 500)?,
            #[cfg(feature = "vector")]
            embedding: embedding_config(&lookup)?,
            #[cfg(feature = "vector")]
            reranker: reranker_config(&lookup)?,
        };
        config.validate_queue_wait_bounds()?;
        Ok(config)
    }

    fn validate_queue_wait_bounds(&self) -> Result<(), AppError> {
        if self.queue_wait_timeout_ms_default > self.queue_wait_timeout_ms_max {
            return Err(AppError::InvalidInput(
                "SQLITE_QUEUE_WAIT_TIMEOUT_MS_DEFAULT must be less than or equal to SQLITE_QUEUE_WAIT_TIMEOUT_MS_MAX".to_string(),
            ));
        }

        if self.queue_poll_interval_ms_min > self.queue_poll_interval_ms_max {
            return Err(AppError::InvalidInput(
                "SQLITE_QUEUE_POLL_INTERVAL_MS_MIN must be less than or equal to SQLITE_QUEUE_POLL_INTERVAL_MS_MAX".to_string(),
            ));
        }

        if self.queue_poll_interval_ms_default < self.queue_poll_interval_ms_min
            || self.queue_poll_interval_ms_default > self.queue_poll_interval_ms_max
        {
            return Err(AppError::InvalidInput(
                "SQLITE_QUEUE_POLL_INTERVAL_MS_DEFAULT must be between SQLITE_QUEUE_POLL_INTERVAL_MS_MIN and SQLITE_QUEUE_POLL_INTERVAL_MS_MAX".to_string(),
            ));
        }

        Ok(())
    }
}

fn log_level<F>(lookup: &F, key: &str, default: &str) -> Result<String, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    let value = lookup(key).unwrap_or_else(|| default.to_string());
    let level = value.trim().to_ascii_lowercase();
    match level.as_str() {
        "trace" | "debug" | "info" | "warn" | "error" | "off" => Ok(level),
        _ => Err(AppError::InvalidInput(format!(
            "{key} must be one of: trace, debug, info, warn, error, off"
        ))),
    }
}

#[cfg(feature = "vector")]
fn embedding_config<F>(lookup: &F) -> Result<EmbeddingConfig, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    let provider = optional_non_empty_string(lookup, "SQLITE_EMBEDDING_PROVIDER")
        .unwrap_or_else(|| "fastembed".to_string());
    let provider = match provider.as_str() {
        "fastembed" => EmbeddingProvider::Fastembed,
        _ => {
            return Err(AppError::InvalidInput(
                "SQLITE_EMBEDDING_PROVIDER must be one of: fastembed".to_string(),
            ));
        }
    };

    let model = optional_non_empty_string(lookup, "SQLITE_EMBEDDING_MODEL")
        .unwrap_or_else(|| "BAAI/bge-small-en-v1.5".to_string());
    let parsed_model = parse_embedding_model(&model)?;
    let dimension = EmbeddingModel::get_model_info(&parsed_model)
        .map(|info| info.dim)
        .ok_or_else(|| AppError::InvalidInput(format!("unsupported embedding model: {model}")))?;

    Ok(EmbeddingConfig {
        provider,
        model,
        cache_dir: optional_path(lookup, "SQLITE_EMBEDDING_CACHE_DIR")?,
        dimension,
    })
}

#[cfg(feature = "vector")]
fn parse_embedding_model(value: &str) -> Result<EmbeddingModel, AppError> {
    match value {
        "BAAI/bge-small-en-v1.5" | "bge-small-en-v1.5" | "BGESmallENV15" | "bgesmallenv15" => {
            Ok(EmbeddingModel::BGESmallENV15)
        }
        _ => Err(AppError::InvalidInput(format!(
            "unsupported SQLITE_EMBEDDING_MODEL: {value}; currently supported: BAAI/bge-small-en-v1.5"
        ))),
    }
}

#[cfg(feature = "vector")]
fn reranker_config<F>(lookup: &F) -> Result<Option<RerankerConfig>, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    let provider = optional_non_empty_string(lookup, "SQLITE_RERANKER_PROVIDER");
    let model = optional_non_empty_string(lookup, "SQLITE_RERANKER_MODEL");
    let cache_dir = optional_path(lookup, "SQLITE_RERANKER_CACHE_DIR")?;

    if provider.is_none() && model.is_none() && cache_dir.is_none() {
        return Ok(None);
    }

    let provider = provider.unwrap_or_else(|| "fastembed".to_string());
    let model = model.unwrap_or_else(|| "BAAI/bge-reranker-base".to_string());

    let provider = match provider.as_str() {
        "fastembed" => RerankerProvider::Fastembed,
        _ => {
            return Err(AppError::InvalidInput(
                "SQLITE_RERANKER_PROVIDER must be one of: fastembed".to_string(),
            ));
        }
    };

    parse_reranker_model(&model)?;

    Ok(Some(RerankerConfig {
        provider,
        model,
        cache_dir,
    }))
}

#[cfg(feature = "vector")]
fn parse_reranker_model(value: &str) -> Result<RerankerModel, AppError> {
    match value {
        "BAAI/bge-reranker-base" | "bge-reranker-base" | "BGERerankerBase"
        | "bgererankerbase" => Ok(RerankerModel::BGERerankerBase),
        _ => value.parse::<RerankerModel>().map_err(|_| {
            AppError::InvalidInput(format!(
                "unsupported SQLITE_RERANKER_MODEL: {value}; currently supported default: BAAI/bge-reranker-base"
            ))
        }),
    }
}

#[cfg(feature = "vector")]
fn optional_non_empty_string<F>(lookup: &F, key: &str) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(key).and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}

fn optional_path<F>(lookup: &F, key: &str) -> Result<Option<PathBuf>, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(key) {
        Some(value) => {
            let trimmed = value.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            let raw = PathBuf::from(trimmed);
            let absolute = if raw.is_absolute() {
                raw
            } else {
                std::env::current_dir()
                    .map_err(|error| {
                        AppError::InvalidInput(format!(
                            "failed to resolve current directory: {error}"
                        ))
                    })?
                    .join(raw)
            };
            Ok(Some(
                absolute
                    .canonicalize()
                    .unwrap_or_else(|_| absolute.to_path_buf()),
            ))
        }
        None => Ok(None),
    }
}

fn positive_usize<F>(lookup: &F, key: &str, default: usize) -> Result<usize, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(key) {
        Some(value) => {
            let parsed = value
                .trim()
                .parse::<usize>()
                .map_err(|_| AppError::InvalidInput(format!("{key} must be a positive integer")))?;
            if parsed == 0 {
                return Err(AppError::InvalidInput(format!(
                    "{key} must be greater than zero"
                )));
            }
            Ok(parsed)
        }
        None => Ok(default),
    }
}

fn positive_u64<F>(lookup: &F, key: &str, default: u64) -> Result<u64, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(key) {
        Some(value) => {
            let parsed = value
                .trim()
                .parse::<u64>()
                .map_err(|_| AppError::InvalidInput(format!("{key} must be a positive integer")))?;
            if parsed == 0 {
                return Err(AppError::InvalidInput(format!(
                    "{key} must be greater than zero"
                )));
            }
            Ok(parsed)
        }
        None => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn uses_defaults() {
        let cfg = AppConfig::from_lookup(|key| {
            #[cfg(feature = "vector")]
            {
                let _ = key;
                None
            }

            #[cfg(not(feature = "vector"))]
            {
                let _ = key;
                None
            }
        })
        .expect("config should parse with defaults");
        assert_eq!(cfg.max_sql_length, 20_000);
        assert_eq!(cfg.max_statements, 50);
        assert_eq!(cfg.max_rows, 500);
        assert_eq!(cfg.max_persisted_list_entries, 500);
        assert_eq!(cfg.log_level, "info");
        assert_eq!(cfg.queue_wait_timeout_ms_default, 30_000);
        assert_eq!(cfg.queue_wait_timeout_ms_max, 120_000);
        assert_eq!(cfg.queue_poll_interval_ms_default, 250);
        assert_eq!(cfg.queue_poll_interval_ms_min, 50);
        assert_eq!(cfg.queue_poll_interval_ms_max, 5_000);
    }

    #[test]
    fn rejects_non_positive_values() {
        let cfg = AppConfig::from_lookup(|key| {
            if key == "SQLITE_MAX_ROWS" {
                Some("0".to_string())
            } else {
                None
            }
        });
        assert!(cfg.is_err());
    }

    #[test]
    fn rejects_unknown_log_level() {
        let cfg = AppConfig::from_lookup(|key| {
            if key == "SQLITE_LOG_LEVEL" {
                Some("verbose".to_string())
            } else {
                None
            }
        });
        assert!(cfg.is_err());
    }

    #[test]
    fn rejects_invalid_queue_timeout_bounds() {
        let cfg = AppConfig::from_lookup(|key| match key {
            "SQLITE_QUEUE_WAIT_TIMEOUT_MS_DEFAULT" => Some("2000".to_string()),
            "SQLITE_QUEUE_WAIT_TIMEOUT_MS_MAX" => Some("1000".to_string()),
            _ => None,
        });
        assert!(cfg.is_err());
    }

    #[test]
    fn rejects_invalid_queue_poll_bounds() {
        let cfg = AppConfig::from_lookup(|key| match key {
            "SQLITE_QUEUE_POLL_INTERVAL_MS_MIN" => Some("500".to_string()),
            "SQLITE_QUEUE_POLL_INTERVAL_MS_MAX" => Some("100".to_string()),
            _ => None,
        });
        assert!(cfg.is_err());
    }

    #[test]
    fn rejects_queue_poll_default_outside_bounds() {
        let cfg = AppConfig::from_lookup(|key| match key {
            "SQLITE_QUEUE_POLL_INTERVAL_MS_DEFAULT" => Some("50".to_string()),
            "SQLITE_QUEUE_POLL_INTERVAL_MS_MIN" => Some("100".to_string()),
            "SQLITE_QUEUE_POLL_INTERVAL_MS_MAX" => Some("500".to_string()),
            _ => None,
        });
        assert!(cfg.is_err());
    }

    #[cfg(feature = "vector")]
    #[test]
    fn validates_vector_configuration() {
        let cfg = AppConfig::from_lookup(|key| match key {
            "SQLITE_EMBEDDING_PROVIDER" => Some("fastembed".to_string()),
            "SQLITE_EMBEDDING_MODEL" => Some("BAAI/bge-small-en-v1.5".to_string()),
            _ => None,
        })
        .expect("vector config should parse");

        assert!(matches!(
            cfg.embedding.provider,
            super::EmbeddingProvider::Fastembed
        ));
        assert_eq!(cfg.embedding.model, "BAAI/bge-small-en-v1.5");
        assert_eq!(cfg.embedding.dimension, 384);
        assert!(cfg.embedding.cache_dir.is_none());
        assert!(cfg.reranker.is_none());
        assert_eq!(cfg.max_vector_top_k, 200);
        assert_eq!(cfg.max_rerank_fetch_k, 500);
    }

    #[cfg(feature = "vector")]
    #[test]
    fn resolves_embedding_cache_dir() {
        let cfg = AppConfig::from_lookup(|key| match key {
            "SQLITE_EMBEDDING_CACHE_DIR" => Some(".".to_string()),
            _ => None,
        })
        .expect("vector config should parse with cache dir");

        let cache_dir = cfg
            .embedding
            .cache_dir
            .expect("cache dir should be present");
        assert!(cache_dir.is_absolute());
    }

    #[cfg(feature = "vector")]
    #[test]
    fn uses_reranker_defaults_when_enabled() {
        let cfg = AppConfig::from_lookup(|key| match key {
            "SQLITE_RERANKER_PROVIDER" => Some("fastembed".to_string()),
            _ => None,
        })
        .expect("vector config should parse with reranker enabled");

        let reranker = cfg.reranker.expect("reranker should be configured");
        assert!(matches!(
            reranker.provider,
            super::RerankerProvider::Fastembed
        ));
        assert_eq!(reranker.model, "BAAI/bge-reranker-base");
        assert!(reranker.cache_dir.is_none());
    }

    #[cfg(feature = "vector")]
    #[test]
    fn resolves_reranker_cache_dir() {
        let cfg = AppConfig::from_lookup(|key| match key {
            "SQLITE_RERANKER_PROVIDER" => Some("fastembed".to_string()),
            "SQLITE_RERANKER_CACHE_DIR" => Some(".".to_string()),
            _ => None,
        })
        .expect("vector config should parse with reranker cache dir");

        let reranker = cfg.reranker.expect("reranker should be configured");
        let cache_dir = reranker.cache_dir.expect("cache dir should be present");
        assert!(cache_dir.is_absolute());
    }
}
