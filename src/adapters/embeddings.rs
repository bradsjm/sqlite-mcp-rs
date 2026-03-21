#[cfg(test)]
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Mutex;

use fastembed::{EmbeddingModel, ModelTrait, TextEmbedding, TextInitOptions};

use crate::adapters::ort_runtime::ensure_ort_dylib_configured;
use crate::config::{EmbeddingConfig, EmbeddingProvider};
use crate::errors::{AppError, AppResult};

pub struct EmbeddingClient {
    config: EmbeddingConfig,
    state: Mutex<Option<TextEmbedding>>,
    model: EmbeddingModel,
    #[cfg(test)]
    test_embeddings: Option<HashMap<String, Vec<f32>>>,
}

impl EmbeddingClient {
    pub fn new(config: EmbeddingConfig) -> Self {
        let model = parse_embedding_model(&config.model);
        Self {
            config,
            state: Mutex::new(None),
            model,
            #[cfg(test)]
            test_embeddings: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_test(
        config: EmbeddingConfig,
        test_embeddings: HashMap<String, Vec<f32>>,
    ) -> Self {
        let model = parse_embedding_model(&config.model);
        Self {
            config,
            state: Mutex::new(None),
            model,
            test_embeddings: Some(test_embeddings),
        }
    }

    pub fn model(&self) -> &str {
        self.config.model.as_str()
    }

    pub fn dimension(&self) -> usize {
        self.config.dimension
    }

    pub fn provider(&self) -> EmbeddingProvider {
        self.config.provider
    }

    pub fn cache_dir_path(&self) -> Option<PathBuf> {
        self.cache_dir()
    }

    pub fn prewarm(&self) -> AppResult<()> {
        #[cfg(test)]
        if self.test_embeddings.is_some() {
            return Ok(());
        }

        match self.config.provider {
            EmbeddingProvider::Fastembed => {
                let mut guard = self.state.lock().map_err(|_| {
                    AppError::Dependency("embedding model lock poisoned".to_string())
                })?;
                self.ensure_fastembed_loaded(&mut guard)
            }
        }
    }

    pub fn embed(&self, text: &str) -> AppResult<Vec<f32>> {
        #[cfg(test)]
        {
            if let Some(embedding) = self.test_embedding(text)? {
                return Ok(embedding);
            }
        }

        match self.config.provider {
            EmbeddingProvider::Fastembed => self.embed_fastembed(text),
        }
    }

    #[cfg(test)]
    fn test_embedding(&self, text: &str) -> AppResult<Option<Vec<f32>>> {
        let Some(embeddings) = &self.test_embeddings else {
            return Ok(None);
        };
        let embedding = embeddings.get(text).cloned().ok_or_else(|| {
            AppError::Dependency(format!("missing test embedding for text: {text}"))
        })?;
        if embedding.len() != self.config.dimension {
            return Err(AppError::Dependency(format!(
                "embedding dimension mismatch: expected {}, got {}",
                self.config.dimension,
                embedding.len()
            )));
        }
        Ok(Some(embedding))
    }

    fn embed_fastembed(&self, text: &str) -> AppResult<Vec<f32>> {
        let mut guard = self
            .state
            .lock()
            .map_err(|_| AppError::Dependency("embedding model lock poisoned".to_string()))?;

        self.ensure_fastembed_loaded(&mut guard)?;

        let model = guard
            .as_mut()
            .ok_or_else(|| AppError::Dependency("embedding model unavailable".to_string()))?;

        let mut batches = model.embed(vec![text], None).map_err(|error| {
            AppError::Dependency(format!("embedding generation failed: {error}"))
        })?;
        let mut embedding = batches.pop().ok_or_else(|| {
            AppError::Dependency("embedding model returned no vectors".to_string())
        })?;

        if embedding.len() != self.config.dimension {
            return Err(AppError::Dependency(format!(
                "embedding dimension mismatch: expected {}, got {}",
                self.config.dimension,
                embedding.len()
            )));
        }

        normalize_embedding(&mut embedding);
        Ok(embedding)
    }

    fn ensure_fastembed_loaded(&self, guard: &mut Option<TextEmbedding>) -> AppResult<()> {
        if guard.is_some() {
            return Ok(());
        }

        ensure_ort_dylib_configured()?;

        let mut options = TextInitOptions::new(self.model.clone());
        if let Some(cache_dir) = self.cache_dir() {
            options = options.with_cache_dir(cache_dir);
        }
        options = options.with_show_download_progress(false);

        let model = catch_init_panic("failed to initialize embedding model", || {
            TextEmbedding::try_new(options)
        })?
        .map_err(|error| {
            AppError::Dependency(format!("failed to initialize embedding model: {error}"))
        })?;
        *guard = Some(model);
        Ok(())
    }

    fn cache_dir(&self) -> Option<PathBuf> {
        self.config.cache_dir.clone()
    }
}

pub fn serialize_embedding_json(embedding: &[f32]) -> AppResult<String> {
    serde_json::to_string(embedding)
        .map_err(|error| AppError::Dependency(format!("failed to serialize embedding: {error}")))
}

fn parse_embedding_model(model: &str) -> EmbeddingModel {
    match model {
        "BAAI/bge-small-en-v1.5" | "bge-small-en-v1.5" | "BGESmallENV15" | "bgesmallenv15" => {
            EmbeddingModel::BGESmallENV15
        }
        _ => EmbeddingModel::from_str(model).unwrap_or(EmbeddingModel::BGESmallENV15),
    }
}

fn normalize_embedding(embedding: &mut [f32]) {
    let norm = embedding
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt();
    if norm <= 0.0 {
        return;
    }
    for value in embedding {
        *value = (*value as f64 / norm) as f32;
    }
}

fn catch_init_panic<T, F>(context: &str, init: F) -> AppResult<T>
where
    F: FnOnce() -> T,
{
    catch_unwind(AssertUnwindSafe(init)).map_err(|panic_payload| {
        AppError::Dependency(format!("{context}: {}", panic_message(panic_payload)))
    })
}

fn panic_message(panic_payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = panic_payload.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic_payload.downcast_ref::<String>() {
        return message.clone();
    }
    "panic during model initialization".to_string()
}

#[allow(dead_code)]
fn _model_dimension(model: &EmbeddingModel) -> Option<usize> {
    EmbeddingModel::get_model_info(model).map(|info| info.dim)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    use crate::config::{EmbeddingConfig, EmbeddingProvider};

    use super::{EmbeddingClient, catch_init_panic, panic_message, serialize_embedding_json};

    fn test_config(dimension: usize) -> EmbeddingConfig {
        EmbeddingConfig {
            provider: EmbeddingProvider::Fastembed,
            model: "BAAI/bge-small-en-v1.5".to_string(),
            cache_dir: None,
            dimension,
        }
    }

    #[test]
    fn uses_test_embedding_override_without_model_download() {
        let mut vectors = HashMap::new();
        vectors.insert("hello".to_string(), vec![1.0, 0.0, 0.0]);
        let client = EmbeddingClient::new_test(test_config(3), vectors);
        let output = client.embed("hello").expect("embedding should resolve");
        assert_eq!(output, vec![1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_override_enforces_dimension() {
        let mut vectors = HashMap::new();
        vectors.insert("hello".to_string(), vec![1.0, 0.0]);
        let client = EmbeddingClient::new_test(test_config(3), vectors);
        let error = client
            .embed("hello")
            .expect_err("dimension mismatch must fail");
        assert!(error.to_string().contains("embedding dimension mismatch"));
    }

    #[test]
    fn serializes_embeddings_to_json() {
        let serialized =
            serialize_embedding_json(&[0.5, -0.25, 1.0]).expect("serialize should work");
        assert_eq!(serialized, "[0.5,-0.25,1.0]");
    }

    #[test]
    fn panic_payload_stringifies_message() {
        let payload = catch_unwind(AssertUnwindSafe(|| panic!("embedding panic")))
            .expect_err("panic expected");
        assert_eq!(panic_message(payload), "embedding panic");
    }

    #[test]
    fn catch_init_panic_maps_panics_to_dependency_errors() {
        let error = catch_init_panic::<(), _>("failed to initialize embedding model", || {
            panic!("embedding panic")
        })
        .expect_err("panic should be converted");
        assert!(
            error
                .to_string()
                .contains("failed to initialize embedding model: embedding panic")
        );
    }
}
