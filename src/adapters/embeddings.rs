use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::config::{EmbeddingConfig, EmbeddingProvider};
use crate::errors::{AppError, AppResult};

#[derive(Debug, Clone)]
pub struct EmbeddingClient {
    config: EmbeddingConfig,
}

impl EmbeddingClient {
    pub fn new(config: EmbeddingConfig) -> Self {
        Self { config }
    }

    pub fn model(&self) -> &str {
        self.config.model.as_str()
    }

    pub fn dimension(&self) -> usize {
        self.config.size
    }

    pub fn embed(&self, text: &str) -> AppResult<Vec<f32>> {
        match self.config.provider {
            EmbeddingProvider::Builtin => Ok(embed_builtin(text, self.config.size)),
        }
    }
}

fn embed_builtin(text: &str, dimension: usize) -> Vec<f32> {
    let mut values = vec![0.0f32; dimension.max(1)];
    for token in text.split_whitespace() {
        let mut hasher = DefaultHasher::new();
        token.hash(&mut hasher);
        let hash = hasher.finish() as usize;
        let slot = hash % values.len();
        let weight = ((hash >> 8) % 1_000) as f32 / 1_000.0 + 0.1;
        values[slot] += weight;
    }

    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut values {
            *value /= norm;
        }
    }

    values
}

pub fn parse_embedding(raw: &str) -> AppResult<Vec<f32>> {
    serde_json::from_str::<Vec<f32>>(raw)
        .map_err(|error| AppError::Dependency(format!("invalid stored embedding payload: {error}")))
}

pub fn serialize_embedding(embedding: &[f32]) -> AppResult<String> {
    serde_json::to_string(embedding)
        .map_err(|error| AppError::Dependency(format!("failed to serialize embedding: {error}")))
}
