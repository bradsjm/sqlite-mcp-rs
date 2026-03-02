use std::collections::HashSet;

use crate::config::{RerankerConfig, RerankerProvider};
use crate::errors::AppResult;

#[derive(Debug, Clone)]
pub struct RerankerClient {
    config: RerankerConfig,
}

impl RerankerClient {
    pub fn new(config: RerankerConfig) -> Self {
        Self { config }
    }

    pub fn model(&self) -> &str {
        self.config.model.as_str()
    }

    pub fn rerank(&self, query: &str, docs: &[String]) -> AppResult<Vec<f64>> {
        match self.config.provider {
            RerankerProvider::Builtin => Ok(rerank_builtin(query, docs)),
        }
    }
}

fn rerank_builtin(query: &str, docs: &[String]) -> Vec<f64> {
    let query_tokens = tokenize(query);
    docs.iter()
        .map(|doc| {
            let doc_tokens = tokenize(doc);
            if query_tokens.is_empty() || doc_tokens.is_empty() {
                return 0.0;
            }

            let overlap = query_tokens.intersection(&doc_tokens).count() as f64;
            overlap / query_tokens.len() as f64
        })
        .collect()
}

fn tokenize(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|token| {
            token
                .chars()
                .filter(|ch| ch.is_ascii_alphanumeric())
                .collect::<String>()
                .to_ascii_lowercase()
        })
        .filter(|token| !token.is_empty())
        .collect()
}
