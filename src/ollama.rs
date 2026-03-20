//! Ollama integration — embeddings and chat via local REST API.
//!
//! Two capabilities:
//! 1. Embeddings: encode text as vectors for semantic knowledge graph search
//! 2. Chat: use Ollama models as an AI backend (alternative to Claude/Codex)
//!
//! Ollama runs locally on the same machine (or network) at http://localhost:11434.
//! No API key needed.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Default Ollama API base URL.
const DEFAULT_BASE_URL: &str = "http://localhost:11434";

/// Default embedding model.
const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";

/// Maximum number of cached embeddings (prevents unbounded memory growth).
const MAX_CACHE_ENTRIES: usize = 500;

/// Ollama client for embeddings and chat.
#[derive(Clone)]
pub struct OllamaClient {
    base_url: String,
    embed_model: String,
    client: reqwest::Client,
    /// Cache: label → embedding vector. Avoids re-embedding unchanged nodes.
    embed_cache: Arc<Mutex<std::collections::HashMap<String, Vec<f32>>>>,
    /// Path for persisting the cache to disk. Empty = no persistence.
    cache_file: Option<std::path::PathBuf>,
}

impl OllamaClient {
    /// Create a new Ollama client.
    pub fn new(base_url: Option<&str>, embed_model: Option<&str>) -> Self {
        Self {
            base_url: base_url.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/').into(),
            embed_model: embed_model.unwrap_or(DEFAULT_EMBED_MODEL).into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            embed_cache: Arc::new(Mutex::new(std::collections::HashMap::new())),
            cache_file: None,
        }
    }

    /// Create with persistent disk cache.
    pub fn with_cache(base_url: Option<&str>, embed_model: Option<&str>, cache_path: std::path::PathBuf) -> Self {
        let mut client = Self::new(base_url, embed_model);
        // Load existing cache from disk.
        if cache_path.exists() {
            if let Ok(data) = std::fs::read_to_string(&cache_path) {
                if let Ok(cache) = serde_json::from_str::<std::collections::HashMap<String, Vec<f32>>>(&data) {
                    log::info!("Loaded {} cached embeddings from {}", cache.len(), cache_path.display());
                    client.embed_cache = Arc::new(Mutex::new(cache));
                }
            }
        }
        client.cache_file = Some(cache_path);
        client
    }

    /// Persist the cache to disk (non-blocking, best-effort).
    fn save_cache(&self) {
        if let Some(ref path) = self.cache_file {
            let cache = self.embed_cache.clone();
            let path = path.clone();
            // Spawn blocking to avoid holding the async lock.
            tokio::spawn(async move {
                let data = cache.lock().await;
                if let Ok(json) = serde_json::to_string(&*data) {
                    let tmp = path.with_extension("json.tmp");
                    if std::fs::write(&tmp, &json).is_ok() {
                        let _ = std::fs::rename(&tmp, &path);
                    }
                }
            });
        }
    }

    /// Check if Ollama is available.
    pub async fn is_available(&self) -> bool {
        self.client.get(&self.base_url)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    // --- Embeddings ---

    /// Generate embeddings for one or more texts.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let input: Vec<String> = texts.iter().map(|t| t.to_string()).collect();

        let body = EmbedRequest {
            model: self.embed_model.clone(),
            input,
        };

        let resp = self.client
            .post(format!("{}/api/embed", self.base_url))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Ollama embed request failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama embed {} — {}", status, text));
        }

        let result: EmbedResponse = resp.json().await
            .map_err(|e| format!("Ollama embed parse error: {}", e))?;

        Ok(result.embeddings)
    }

    /// Embed a single text (with caching and LRU eviction).
    pub async fn embed_cached(&self, key: &str, text: &str) -> Result<Vec<f32>, String> {
        {
            let cache = self.embed_cache.lock().await;
            if let Some(vec) = cache.get(key) {
                return Ok(vec.clone());
            }
        }

        let results = self.embed(&[text]).await?;
        let vec = results.into_iter().next()
            .ok_or_else(|| "No embedding returned".to_string())?;

        {
            let mut cache = self.embed_cache.lock().await;
            // Evict oldest entries if cache exceeds limit.
            if cache.len() >= MAX_CACHE_ENTRIES {
                let keys: Vec<String> = cache.keys().take(cache.len() / 4).cloned().collect();
                for k in keys { cache.remove(&k); }
                log::debug!("Embedding cache evicted to {} entries", cache.len());
            }
            cache.insert(key.to_string(), vec.clone());
        }

        // Persist to disk periodically (after every new embedding).
        self.save_cache();

        Ok(vec)
    }

    /// Cosine similarity between two vectors.
    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
        if a.len() != b.len() || a.is_empty() { return 0.0; }
        let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
        let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm_a == 0.0 || norm_b == 0.0 { return 0.0; }
        dot / (norm_a * norm_b)
    }

    /// Find the top-N most similar texts from a set of embeddings.
    /// Filters out candidates below a minimum similarity threshold (0.3).
    pub fn top_similar(
        query_embedding: &[f32],
        candidates: &[(String, Vec<f32>)],
        top_n: usize,
    ) -> Vec<(String, f32)> {
        const MIN_SIMILARITY: f32 = 0.3;
        let mut scored: Vec<(String, f32)> = candidates.iter()
            .map(|(label, vec)| (label.clone(), Self::cosine_similarity(query_embedding, vec)))
            .filter(|(_, sim)| *sim >= MIN_SIMILARITY)
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_n);
        scored
    }

    // --- Chat (AI backend) ---

    /// Generate a response using Ollama's /api/generate endpoint.
    /// Returns the full response text (non-streaming).
    pub async fn generate(&self, model: &str, prompt: &str, system: Option<&str>) -> Result<String, String> {
        let body = GenerateRequest {
            model: model.into(),
            prompt: prompt.into(),
            system: system.map(|s| s.into()),
            stream: false,
        };

        let resp = self.client
            .post(format!("{}/api/generate", self.base_url))
            .json(&body)
            .timeout(std::time::Duration::from_secs(300))
            .send()
            .await
            .map_err(|e| format!("Ollama generate failed: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Ollama generate {} — {}", status, text));
        }

        let result: GenerateResponse = resp.json().await
            .map_err(|e| format!("Ollama generate parse error: {}", e))?;

        Ok(result.response)
    }

    /// List available models.
    #[allow(dead_code)]
    pub async fn list_models(&self) -> Result<Vec<String>, String> {
        let resp = self.client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
            .map_err(|e| format!("Ollama list models failed: {}", e))?;

        if !resp.status().is_success() {
            return Err("Ollama not available".into());
        }

        let result: TagsResponse = resp.json().await
            .map_err(|e| format!("Ollama tags parse error: {}", e))?;

        Ok(result.models.into_iter().map(|m| m.name).collect())
    }
}

// --- Request/Response types ---

#[derive(Serialize)]
struct EmbedRequest {
    model: String,
    input: Vec<String>,
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}

#[derive(Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    stream: bool,
}

#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct TagsResponse {
    models: Vec<ModelInfo>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ModelInfo {
    name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((OllamaClient::cosine_similarity(&a, &b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(OllamaClient::cosine_similarity(&a, &b).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((OllamaClient::cosine_similarity(&a, &b) + 1.0).abs() < 0.001);
    }

    #[test]
    fn top_similar_ranking() {
        let query = vec![1.0, 0.0, 0.0];
        let candidates = vec![
            ("close".into(), vec![0.9, 0.1, 0.0]),
            ("far".into(), vec![0.0, 0.0, 1.0]),
            ("medium".into(), vec![0.5, 0.5, 0.0]),
        ];
        let top = OllamaClient::top_similar(&query, &candidates, 2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "close");
        assert_eq!(top[1].0, "medium");
    }

    #[test]
    fn empty_vectors() {
        assert_eq!(OllamaClient::cosine_similarity(&[], &[]), 0.0);
        assert_eq!(OllamaClient::cosine_similarity(&[0.0], &[0.0]), 0.0);
    }
}
