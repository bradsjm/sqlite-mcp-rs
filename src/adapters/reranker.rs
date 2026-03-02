use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

#[cfg(test)]
use std::collections::HashMap;

use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

use crate::adapters::ort_runtime::ensure_ort_dylib_configured;
use crate::config::{RerankerConfig, RerankerProvider};
use crate::errors::{AppError, AppResult};

#[derive(Debug)]
pub struct RerankerClient {
    config: RerankerConfig,
    state: Mutex<Option<TextRerank>>,
    model: RerankerModel,
    #[cfg(test)]
    test_scores: Option<HashMap<String, Vec<f64>>>,
}

impl RerankerClient {
    pub fn new(config: RerankerConfig) -> Self {
        let model = parse_reranker_model(&config.model);
        Self {
            config,
            state: Mutex::new(None),
            model,
            #[cfg(test)]
            test_scores: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_test(config: RerankerConfig, test_scores: HashMap<String, Vec<f64>>) -> Self {
        let model = parse_reranker_model(&config.model);
        Self {
            config,
            state: Mutex::new(None),
            model,
            test_scores: Some(test_scores),
        }
    }

    pub fn model(&self) -> &str {
        self.config.model.as_str()
    }

    pub fn rerank(&self, query: &str, docs: &[String]) -> AppResult<Vec<f64>> {
        if docs.is_empty() {
            return Ok(Vec::new());
        }

        #[cfg(test)]
        {
            if let Some(scores) = self.test_scores_for(query, docs.len())? {
                return Ok(scores);
            }
        }

        match self.config.provider {
            RerankerProvider::Fastembed => self.rerank_fastembed(query, docs),
        }
    }

    #[cfg(test)]
    fn test_scores_for(&self, query: &str, expected_len: usize) -> AppResult<Option<Vec<f64>>> {
        let Some(scores_map) = &self.test_scores else {
            return Ok(None);
        };
        let scores = scores_map.get(query).cloned().ok_or_else(|| {
            AppError::Dependency(format!("missing test rerank scores for query: {query}"))
        })?;
        if scores.len() != expected_len {
            return Err(AppError::Dependency(format!(
                "rerank score count mismatch: expected {expected_len}, got {}",
                scores.len()
            )));
        }
        Ok(Some(scores))
    }

    fn rerank_fastembed(&self, query: &str, docs: &[String]) -> AppResult<Vec<f64>> {
        let mut guard = self
            .state
            .lock()
            .map_err(|_| AppError::Dependency("reranker model lock poisoned".to_string()))?;

        if guard.is_none() {
            ensure_ort_dylib_configured()?;

            let mut options = RerankInitOptions::new(self.model.clone());
            if let Some(cache_dir) = self.cache_dir() {
                options = options.with_cache_dir(cache_dir);
            }
            options = options.with_show_download_progress(false);

            let model = TextRerank::try_new(options).map_err(|error| {
                AppError::Dependency(format!("failed to initialize reranker model: {error}"))
            })?;
            *guard = Some(model);
        }

        let model = guard
            .as_mut()
            .ok_or_else(|| AppError::Dependency("reranker model unavailable".to_string()))?;

        let doc_refs = docs.iter().map(String::as_str).collect::<Vec<_>>();
        let results = model
            .rerank(query, &doc_refs, false, None)
            .map_err(|error| AppError::Dependency(format!("reranking failed: {error}")))?;

        let mut scores = vec![f64::NEG_INFINITY; docs.len()];
        for item in results {
            if item.index >= docs.len() {
                return Err(AppError::Dependency(format!(
                    "reranker returned out-of-range index {} for {} docs",
                    item.index,
                    docs.len()
                )));
            }
            scores[item.index] = item.score as f64;
        }
        if scores.iter().any(|score| !score.is_finite()) {
            return Err(AppError::Dependency(
                "reranker did not produce scores for all candidates".to_string(),
            ));
        }

        Ok(scores)
    }

    fn cache_dir(&self) -> Option<PathBuf> {
        self.config.cache_dir.clone()
    }
}

fn parse_reranker_model(model: &str) -> RerankerModel {
    match model {
        "BAAI/bge-reranker-base" | "bge-reranker-base" | "BGERerankerBase" | "bgererankerbase" => {
            RerankerModel::BGERerankerBase
        }
        _ => RerankerModel::from_str(model).unwrap_or(RerankerModel::BGERerankerBase),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use crate::adapters::embeddings::EmbeddingClient;
    use crate::config::{EmbeddingConfig, EmbeddingProvider, RerankerConfig, RerankerProvider};

    use super::RerankerClient;

    fn test_config() -> RerankerConfig {
        RerankerConfig {
            provider: RerankerProvider::Fastembed,
            model: "BAAI/bge-reranker-base".to_string(),
            cache_dir: None,
        }
    }

    #[test]
    fn uses_test_rerank_override_without_model_download() {
        let client = RerankerClient::new_test(
            test_config(),
            HashMap::from([("q".to_string(), vec![0.9, 0.1])]),
        );
        let scores = client
            .rerank("q", &["a".to_string(), "b".to_string()])
            .expect("rerank should resolve");
        assert_eq!(scores, vec![0.9, 0.1]);
    }

    #[test]
    fn test_override_enforces_score_count() {
        let client =
            RerankerClient::new_test(test_config(), HashMap::from([("q".to_string(), vec![0.9])]));
        let error = client
            .rerank("q", &["a".to_string(), "b".to_string()])
            .expect_err("score length mismatch must fail");
        assert!(error.to_string().contains("rerank score count mismatch"));
    }

    #[test]
    #[ignore = "downloads fastembed models; run explicitly"]
    fn downloads_and_uses_real_fastembed_models() {
        let cache_dir =
            std::env::temp_dir().join(format!("sqlite-mcp-fastembed-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&cache_dir).expect("cache directory should be creatable");

        let embedding = EmbeddingClient::new(EmbeddingConfig {
            provider: EmbeddingProvider::Fastembed,
            model: "BAAI/bge-small-en-v1.5".to_string(),
            cache_dir: Some(cache_dir.clone()),
            dimension: 384,
        })
        .embed("vector search in sqlite")
        .expect("real embedding model should initialize and produce a vector");
        assert_eq!(embedding.len(), 384);

        let scores = RerankerClient::new(RerankerConfig {
            provider: RerankerProvider::Fastembed,
            model: "BAAI/bge-reranker-base".to_string(),
            cache_dir: Some(cache_dir.clone()),
        })
        .rerank(
            "sqlite vector search",
            &[
                "sqlite-vec provides KNN in SQLite".to_string(),
                "weather report for tomorrow".to_string(),
            ],
        )
        .expect("real reranker model should initialize and produce scores");

        assert_eq!(scores.len(), 2);
        assert!(scores.iter().all(|score| score.is_finite()));

        let has_cache_content = fs::read_dir(&cache_dir)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(false);
        assert!(
            has_cache_content,
            "expected fastembed model artifacts in {}",
            cache_dir.display()
        );

        let _ = fs::remove_dir_all(&cache_dir);
    }
}
