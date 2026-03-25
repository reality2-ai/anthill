//! Ollama backend — wraps the existing [`crate::ollama::OllamaClient`].

use super::types::*;

/// Ollama local inference backend.
///
/// Uses the Ollama HTTP API (`/api/generate`) for inference.
/// Supports any model installed via `ollama pull`.
#[derive(Debug)]
pub struct OllamaBackend {
    id: String,
    display_name: String,
    base_url: String,
    model: String,
    tags: EngineTags,
}

impl OllamaBackend {
    /// Create an Ollama backend with a specific model.
    pub fn new(model: &str, base_url: Option<&str>) -> Self {
        let base = base_url.unwrap_or("http://localhost:11434");
        Self {
            id: format!("ollama-{}", model.replace(['/', ':', '.'], "-")),
            display_name: format!("Ollama ({})", model),
            base_url: base.trim_end_matches('/').to_string(),
            model: model.to_string(),
            tags: EngineTags {
                categories: vec![EngineCategory::Local, EngineCategory::CostEffective],
                capabilities: vec!["code".into()],
                cost_tier: 1,
                speed_tier: 3,
                quality_tier: 3,
            },
        }
    }

    pub fn with_tags(mut self, tags: EngineTags) -> Self {
        self.tags = tags;
        self
    }
}

#[async_trait::async_trait]
impl AiBackend for OllamaBackend {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.display_name }
    fn tags(&self) -> &EngineTags { &self.tags }

    async fn is_available(&self) -> bool {
        let client = crate::ollama::OllamaClient::new(Some(&self.base_url), None);
        client.is_available().await
    }

    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: ProgressTx,
    ) -> Result<AiResponse, AiError> {
        let _ = progress_tx.send(AiProgress {
            task_id: request.task_id,
            kind: "thinking".into(),
            detail: format!("Calling Ollama ({})...", self.model),
        });

        // If memory_dir is available, use the tool-calling proxy via
        // Ollama's OpenAI-compatible endpoint for MCP tool access.
        if let Some(ref memory_dir) = request.memory_dir {
            let client = reqwest::Client::new();
            let api_url = format!("{}/v1", self.base_url);
            let (text, tokens) = super::tool_proxy::run_tool_loop(
                &client,
                &api_url,
                "", // No API key for local Ollama.
                &self.model,
                &request.system_prompt,
                &request.message,
                memory_dir,
                &progress_tx,
                request.task_id,
                &self.id,
            ).await?;

            if text.is_empty() {
                return Err(AiError::retriable(format!(
                    "Ollama ({}): empty response", self.model
                )));
            }

            return Ok(AiResponse {
                text,
                backend_id: self.id.clone(),
                tokens,
                cost_microdollars: Some(0),
            });
        }

        // Fallback: use the existing generate API without tools.
        let client = crate::ollama::OllamaClient::new(Some(&self.base_url), None);
        let result = client.generate(
            &self.model,
            &request.message,
            Some(&request.system_prompt),
        ).await;

        match result {
            Ok(text) if !text.is_empty() => Ok(AiResponse {
                text,
                backend_id: self.id.clone(),
                tokens: None,
                cost_microdollars: Some(0),
            }),
            Ok(_) => Err(AiError::retriable(format!(
                "Ollama ({}): empty response", self.model
            ))),
            Err(err) => Err(AiError::classify(&format!(
                "Ollama ({}) failed: {}", self.model, err
            ))),
        }
    }

    fn estimate_cost(&self, _input: usize, _output: usize) -> u64 {
        0 // Local inference is free.
    }
}

/// Create an Ollama backend from a [`BackendConfig`].
pub fn create_from_config(id: &str, config: &super::types::BackendConfig) -> Option<Box<dyn AiBackend>> {
    let model = if config.model.is_empty() { "llama3.2" } else { &config.model };
    let base_url = if config.api_base.is_empty() { None } else { Some(config.api_base.as_str()) };
    let mut backend = OllamaBackend::new(model, base_url);
    if let Some(ref tags) = config.tags {
        backend = backend.with_tags(tags.clone());
    }
    // Override the auto-generated ID with the config key.
    backend.id = id.to_string();
    Some(Box::new(backend))
}
