use std::path::PathBuf;

use crate::errors::AppError;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub persist_root: Option<PathBuf>,
    pub log_level: String,
    pub max_sql_length: usize,
    pub max_statements: usize,
    pub max_rows: usize,
    pub max_bytes: usize,
    pub max_db_bytes: u64,
    pub cursor_ttl_seconds: u64,
    pub cursor_capacity: usize,
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
    pub endpoint: Option<String>,
    pub size: usize,
}

#[cfg(feature = "vector")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbeddingProvider {
    Builtin,
}

#[cfg(feature = "vector")]
#[derive(Debug, Clone)]
pub struct RerankerConfig {
    pub provider: RerankerProvider,
    pub model: String,
    pub endpoint: Option<String>,
    pub timeout_ms: u64,
}

#[cfg(feature = "vector")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RerankerProvider {
    Builtin,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, AppError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup<F>(lookup: F) -> Result<Self, AppError>
    where
        F: Fn(&str) -> Option<String>,
    {
        Ok(Self {
            persist_root: optional_path(&lookup, "SQLITE_PERSIST_ROOT")?,
            log_level: log_level(&lookup, "SQLITE_LOG_LEVEL", "info")?,
            max_sql_length: positive_usize(&lookup, "SQLITE_MAX_SQL_LENGTH", 20_000)?,
            max_statements: positive_usize(&lookup, "SQLITE_MAX_STATEMENTS", 50)?,
            max_rows: positive_usize(&lookup, "SQLITE_MAX_ROWS", 500)?,
            max_bytes: positive_usize(&lookup, "SQLITE_MAX_BYTES", 1_048_576)?,
            max_db_bytes: positive_u64(&lookup, "SQLITE_MAX_DB_BYTES", 100_000_000)?,
            cursor_ttl_seconds: positive_u64(&lookup, "SQLITE_CURSOR_TTL_SECONDS", 600)?,
            cursor_capacity: positive_usize(&lookup, "SQLITE_CURSOR_CAPACITY", 500)?,
            #[cfg(feature = "vector")]
            embedding: embedding_config(&lookup)?,
            #[cfg(feature = "vector")]
            reranker: reranker_config(&lookup)?,
        })
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
    let provider = required_string(lookup, "SQLITE_EMBEDDING_PROVIDER")?;
    let provider = match provider.as_str() {
        "builtin" => EmbeddingProvider::Builtin,
        _ => {
            return Err(AppError::InvalidInput(
                "SQLITE_EMBEDDING_PROVIDER must be one of: builtin".to_string(),
            ));
        }
    };

    Ok(EmbeddingConfig {
        provider,
        model: required_string(lookup, "SQLITE_EMBEDDING_MODEL")?,
        endpoint: optional_non_empty_string(lookup, "SQLITE_EMBEDDING_ENDPOINT"),
        size: required_positive_usize(lookup, "SQLITE_EMBEDDING_SIZE")?,
    })
}

#[cfg(feature = "vector")]
fn reranker_config<F>(lookup: &F) -> Result<Option<RerankerConfig>, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    let provider = optional_non_empty_string(lookup, "SQLITE_RERANKER_PROVIDER");
    let model = optional_non_empty_string(lookup, "SQLITE_RERANKER_MODEL");
    let endpoint = optional_non_empty_string(lookup, "SQLITE_RERANKER_ENDPOINT");

    if provider.is_none() && model.is_none() && endpoint.is_none() {
        return Ok(None);
    }

    let Some(provider) = provider else {
        return Err(AppError::InvalidInput(
            "SQLITE_RERANKER_PROVIDER is required when reranker is configured".to_string(),
        ));
    };
    let Some(model) = model else {
        return Err(AppError::InvalidInput(
            "SQLITE_RERANKER_MODEL is required when reranker is configured".to_string(),
        ));
    };

    let provider = match provider.as_str() {
        "builtin" => RerankerProvider::Builtin,
        _ => {
            return Err(AppError::InvalidInput(
                "SQLITE_RERANKER_PROVIDER must be one of: builtin".to_string(),
            ));
        }
    };

    Ok(Some(RerankerConfig {
        provider,
        model,
        endpoint,
        timeout_ms: positive_u64(lookup, "SQLITE_RERANKER_TIMEOUT_MS", 10_000)?,
    }))
}

#[cfg(feature = "vector")]
fn required_string<F>(lookup: &F, key: &str) -> Result<String, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    optional_non_empty_string(lookup, key).ok_or_else(|| {
        AppError::ConfigMissing(format!("{key} is required when vector feature is enabled"))
    })
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

#[cfg(feature = "vector")]
fn required_positive_usize<F>(lookup: &F, key: &str) -> Result<usize, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    let value = lookup(key).ok_or_else(|| AppError::ConfigMissing(format!("{key} is required")))?;
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

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn uses_defaults() {
        let cfg = AppConfig::from_lookup(|key| {
            #[cfg(feature = "vector")]
            {
                match key {
                    "SQLITE_EMBEDDING_PROVIDER" => Some("builtin".to_string()),
                    "SQLITE_EMBEDDING_MODEL" => Some("default".to_string()),
                    "SQLITE_EMBEDDING_SIZE" => Some("16".to_string()),
                    _ => None,
                }
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
        assert_eq!(cfg.log_level, "info");
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

    #[cfg(feature = "vector")]
    #[test]
    fn validates_vector_configuration() {
        let cfg = AppConfig::from_lookup(|key| match key {
            "SQLITE_EMBEDDING_PROVIDER" => Some("builtin".to_string()),
            "SQLITE_EMBEDDING_MODEL" => Some("demo".to_string()),
            "SQLITE_EMBEDDING_SIZE" => Some("16".to_string()),
            _ => None,
        })
        .expect("vector config should parse");

        assert_eq!(cfg.embedding.size, 16);
        assert!(cfg.reranker.is_none());
    }
}
