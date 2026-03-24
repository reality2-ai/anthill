//! HTTP API-based AI backends.
//!
//! Supports OpenAI, Anthropic, Groq, DeepSeek, Together AI, OpenRouter,
//! and any OpenAI-compatible endpoint.  All use streaming responses so
//! progress events flow back in real-time.

use std::collections::HashMap;

use super::types::*;

// ===========================================================================
// OpenAI-compatible backend (covers OpenAI, DeepSeek, Together, OpenRouter, Groq)
// ===========================================================================

/// Generic OpenAI-compatible chat completion backend.
///
/// Works with any API that implements the `/v1/chat/completions` endpoint
/// with the same JSON schema as OpenAI.  This covers:
///
/// - **OpenAI** (GPT-4, GPT-4o, etc.)
/// - **DeepSeek** (`https://api.deepseek.com/v1`)
/// - **Groq** (`https://api.groq.com/openai/v1`)
/// - **Together AI** (`https://api.together.xyz/v1`)
/// - **OpenRouter** (`https://openrouter.ai/api/v1`)
/// - **LocalAI**, **vLLM**, and other local OpenAI-compatible servers
#[derive(Debug)]
pub struct OpenAiCompatibleBackend {
    id: String,
    display_name: String,
    api_base: String,
    api_key: String,
    model: String,
    max_tokens: usize,
    temperature: f64,
    tags: EngineTags,
    /// Extra headers (e.g. OpenRouter requires HTTP-Referer).
    extra_headers: HashMap<String, String>,
    client: reqwest::Client,
}

impl OpenAiCompatibleBackend {
    /// Create a new OpenAI-compatible backend.
    pub fn new(
        id: impl Into<String>,
        display_name: impl Into<String>,
        api_base: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        tags: EngineTags,
    ) -> Self {
        Self {
            id: id.into(),
            display_name: display_name.into(),
            api_base: api_base.into().trim_end_matches('/').to_string(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: 4096,
            temperature: 0.7,
            tags,
            extra_headers: HashMap::new(),
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = temperature;
        self
    }

    pub fn with_extra_headers(mut self, headers: HashMap<String, String>) -> Self {
        self.extra_headers = headers;
        self
    }

    /// Convenience constructors for well-known providers.

    pub fn openai(model: &str, api_key: &str) -> Self {
        Self::new(
            format!("openai-{}", model.replace(['/', '.'], "-")),
            format!("OpenAI {}", model),
            "https://api.openai.com/v1",
            api_key,
            model,
            EngineTags {
                categories: vec![EngineCategory::Intellectual, EngineCategory::Balanced],
                capabilities: vec!["code".into(), "function-calling".into()],
                cost_tier: 4,
                speed_tier: 3,
                quality_tier: 5,
            },
        )
    }

    pub fn deepseek(model: &str, api_key: &str) -> Self {
        Self::new(
            format!("deepseek-{}", model.replace(['/', '.'], "-")),
            format!("DeepSeek {}", model),
            "https://api.deepseek.com/v1",
            api_key,
            model,
            EngineTags {
                categories: vec![EngineCategory::CostEffective, EngineCategory::Specialized("coding".into())],
                capabilities: vec!["code".into()],
                cost_tier: 1,
                speed_tier: 3,
                quality_tier: 3,
            },
        )
    }

    pub fn groq(model: &str, api_key: &str) -> Self {
        Self::new(
            format!("groq-{}", model.replace(['/', '.'], "-")),
            format!("Groq {}", model),
            "https://api.groq.com/openai/v1",
            api_key,
            model,
            EngineTags {
                categories: vec![EngineCategory::Fast, EngineCategory::CostEffective],
                capabilities: vec!["code".into()],
                cost_tier: 1,
                speed_tier: 5,
                quality_tier: 3,
            },
        )
    }

    pub fn together(model: &str, api_key: &str) -> Self {
        Self::new(
            format!("together-{}", model.replace(['/', '.'], "-")),
            format!("Together {}", model),
            "https://api.together.xyz/v1",
            api_key,
            model,
            EngineTags {
                categories: vec![EngineCategory::CostEffective, EngineCategory::Balanced],
                capabilities: vec!["code".into()],
                cost_tier: 2,
                speed_tier: 3,
                quality_tier: 3,
            },
        )
    }

    pub fn openrouter(model: &str, api_key: &str) -> Self {
        let mut backend = Self::new(
            format!("openrouter-{}", model.replace(['/', '.'], "-")),
            format!("OpenRouter {}", model),
            "https://openrouter.ai/api/v1",
            api_key,
            model,
            EngineTags {
                categories: vec![EngineCategory::Balanced],
                capabilities: vec!["code".into()],
                cost_tier: 3,
                speed_tier: 3,
                quality_tier: 4,
            },
        );
        backend.extra_headers.insert(
            "HTTP-Referer".into(),
            "https://anthill.ai".into(),
        );
        backend
    }
}

#[async_trait::async_trait]
impl AiBackend for OpenAiCompatibleBackend {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.display_name }
    fn tags(&self) -> &EngineTags { &self.tags }

    async fn is_available(&self) -> bool {
        // Check: API key is non-empty and endpoint is reachable.
        if self.api_key.is_empty() {
            return false;
        }
        // Quick HEAD/GET to the base URL to confirm it's up.
        // We don't want to burn tokens, so just check /models.
        self.client
            .get(format!("{}/models", self.api_base))
            .bearer_auth(&self.api_key)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map(|r| r.status().is_success() || r.status().as_u16() == 401)
            .unwrap_or(false)
    }

    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: ProgressTx,
    ) -> Result<AiResponse, AiError> {
        let _ = progress_tx.send(AiProgress {
            task_id: request.task_id,
            kind: "thinking".into(),
            detail: format!("Calling {} ({})...", self.display_name, self.model),
        });

        // Build the chat completion request.
        let messages = serde_json::json!([
            { "role": "system", "content": request.system_prompt },
            { "role": "user", "content": request.message },
        ]);

        let body = serde_json::json!({
            "model": self.model,
            "messages": messages,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "stream": true,
        });

        let mut req = self.client
            .post(format!("{}/chat/completions", self.api_base))
            .bearer_auth(&self.api_key)
            .header("Content-Type", "application/json")
            .json(&body);

        for (key, value) in &self.extra_headers {
            req = req.header(key.as_str(), value.as_str());
        }

        let response = req.send().await.map_err(|e| {
            AiError::classify(&format!("{} request failed: {}", self.display_name, e))
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AiError::classify(&format!(
                "{} {} — {}", self.display_name, status, body
            )));
        }

        // Read streaming SSE response.
        let mut full_text = String::new();
        let mut input_tokens = 0usize;
        let mut output_tokens = 0usize;

        let bytes_stream = response.bytes_stream();
        use futures_util::StreamExt;
        let mut stream = bytes_stream;
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| AiError::retriable(format!("Stream error: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            // Process complete SSE lines.
            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim().to_string();
                buffer = buffer[newline_pos + 1..].to_string();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                if let Some(data) = line.strip_prefix("data: ") {
                    if data.trim() == "[DONE]" {
                        continue;
                    }

                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        // Extract delta content.
                        if let Some(delta) = json.pointer("/choices/0/delta/content")
                            .and_then(|c| c.as_str())
                        {
                            full_text.push_str(delta);
                        }

                        // Extract usage info (some providers include it in the final chunk).
                        if let Some(usage) = json.get("usage") {
                            if let Some(in_tok) = usage.get("prompt_tokens").and_then(|t| t.as_u64()) {
                                input_tokens = in_tok as usize;
                            }
                            if let Some(out_tok) = usage.get("completion_tokens").and_then(|t| t.as_u64()) {
                                output_tokens = out_tok as usize;
                            }
                        }

                        // Send periodic progress.
                        if full_text.len() % 500 < 20 {
                            let _ = progress_tx.send(AiProgress {
                                task_id: request.task_id,
                                kind: "thinking".into(),
                                detail: format!("Generating... ({} chars)", full_text.len()),
                            });
                        }
                    }
                }
            }
        }

        if full_text.is_empty() {
            return Err(AiError::retriable(format!("{}: empty response", self.display_name)));
        }

        let cost = self.estimate_cost(input_tokens, output_tokens);

        Ok(AiResponse {
            text: full_text,
            backend_id: self.id.clone(),
            tokens: if input_tokens > 0 || output_tokens > 0 {
                Some((input_tokens, output_tokens))
            } else {
                None
            },
            cost_microdollars: if cost > 0 { Some(cost) } else { None },
        })
    }

    fn estimate_cost(&self, input_tokens: usize, output_tokens: usize) -> u64 {
        // Rough per-million-token pricing in microdollars.
        // These are approximate — real pricing varies by model.
        let (input_ppm, output_ppm) = estimate_pricing(&self.api_base, &self.model);
        let input_cost = (input_tokens as u64 * input_ppm) / 1_000_000;
        let output_cost = (output_tokens as u64 * output_ppm) / 1_000_000;
        input_cost + output_cost
    }
}

// ===========================================================================
// Anthropic Messages API backend
// ===========================================================================

/// Anthropic Messages API backend (Claude via direct API, not CLI).
#[derive(Debug)]
pub struct AnthropicApiBackend {
    id: String,
    display_name: String,
    api_key: String,
    model: String,
    max_tokens: usize,
    tags: EngineTags,
    client: reqwest::Client,
}

impl AnthropicApiBackend {
    pub fn new(model: &str, api_key: &str) -> Self {
        let (categories, quality) = match model {
            m if m.contains("opus") => (
                vec![EngineCategory::Intellectual],
                5,
            ),
            m if m.contains("sonnet") => (
                vec![EngineCategory::Intellectual, EngineCategory::Balanced],
                5,
            ),
            m if m.contains("haiku") => (
                vec![EngineCategory::Fast, EngineCategory::CostEffective],
                3,
            ),
            _ => (vec![EngineCategory::Balanced], 4),
        };

        Self {
            id: format!("anthropic-{}", model.replace(['/', '.'], "-")),
            display_name: format!("Anthropic {}", model),
            api_key: api_key.to_string(),
            model: model.to_string(),
            max_tokens: 8192,
            tags: EngineTags {
                categories,
                capabilities: vec!["code".into(), "function-calling".into()],
                cost_tier: if model.contains("haiku") { 1 } else if model.contains("opus") { 5 } else { 3 },
                speed_tier: if model.contains("haiku") { 5 } else { 3 },
                quality_tier: quality,
            },
            client: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(30))
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = max_tokens;
        self
    }

    pub fn with_tags(mut self, tags: EngineTags) -> Self {
        self.tags = tags;
        self
    }
}

#[async_trait::async_trait]
impl AiBackend for AnthropicApiBackend {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.display_name }
    fn tags(&self) -> &EngineTags { &self.tags }

    async fn is_available(&self) -> bool {
        !self.api_key.is_empty()
    }

    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: ProgressTx,
    ) -> Result<AiResponse, AiError> {
        let _ = progress_tx.send(AiProgress {
            task_id: request.task_id,
            kind: "thinking".into(),
            detail: format!("Calling {} ({})...", self.display_name, self.model),
        });

        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "system": request.system_prompt,
            "messages": [
                { "role": "user", "content": request.message }
            ],
            "stream": true,
        });

        let response = self.client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::classify(&format!("Anthropic request failed: {}", e)))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AiError::classify(&format!("Anthropic {} — {}", status, body)));
        }

        // Read streaming SSE response (Anthropic format).
        let mut full_text = String::new();
        let mut input_tokens = 0usize;
        let mut output_tokens = 0usize;

        let bytes_stream = response.bytes_stream();
        use futures_util::StreamExt;
        let mut stream = bytes_stream;
        let mut buffer = String::new();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| AiError::retriable(format!("Stream error: {}", e)))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));

            while let Some(newline_pos) = buffer.find('\n') {
                let line = buffer[..newline_pos].trim().to_string();
                buffer = buffer[newline_pos + 1..].to_string();

                if line.is_empty() || line.starts_with(':') {
                    continue;
                }

                // Anthropic sends "event: <type>" followed by "data: <json>"
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                        let event_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        match event_type {
                            "content_block_delta" => {
                                if let Some(text) = json.pointer("/delta/text").and_then(|t| t.as_str()) {
                                    full_text.push_str(text);
                                }
                            }
                            "message_start" => {
                                if let Some(usage) = json.pointer("/message/usage") {
                                    if let Some(in_tok) = usage.get("input_tokens").and_then(|t| t.as_u64()) {
                                        input_tokens = in_tok as usize;
                                    }
                                }
                            }
                            "message_delta" => {
                                if let Some(usage) = json.get("usage") {
                                    if let Some(out_tok) = usage.get("output_tokens").and_then(|t| t.as_u64()) {
                                        output_tokens = out_tok as usize;
                                    }
                                }
                            }
                            _ => {}
                        }

                        // Periodic progress.
                        if full_text.len() % 500 < 20 {
                            let _ = progress_tx.send(AiProgress {
                                task_id: request.task_id,
                                kind: "thinking".into(),
                                detail: format!("Generating... ({} chars)", full_text.len()),
                            });
                        }
                    }
                }
            }
        }

        if full_text.is_empty() {
            return Err(AiError::retriable("Anthropic: empty response"));
        }

        Ok(AiResponse {
            text: full_text,
            backend_id: self.id.clone(),
            tokens: if input_tokens > 0 || output_tokens > 0 {
                Some((input_tokens, output_tokens))
            } else {
                None
            },
            cost_microdollars: None, // TODO: calculate from model pricing
        })
    }
}

// ===========================================================================
// Helpers
// ===========================================================================

/// Rough pricing estimates (per million tokens, in microdollars).
/// Returns (input_price_per_million, output_price_per_million).
fn estimate_pricing(api_base: &str, model: &str) -> (u64, u64) {
    let model_lower = model.to_lowercase();

    // Groq — essentially free (very cheap inference).
    if api_base.contains("groq.com") {
        return (500_000, 500_000); // ~$0.50/M
    }

    // DeepSeek — very cheap.
    if api_base.contains("deepseek.com") {
        return (140_000, 280_000); // ~$0.14/$0.28 per M
    }

    // Together AI — moderate.
    if api_base.contains("together.xyz") {
        return (800_000, 800_000); // ~$0.80/M (varies by model)
    }

    // OpenRouter — varies wildly, use moderate estimate.
    if api_base.contains("openrouter.ai") {
        return (2_000_000, 6_000_000); // rough average
    }

    // OpenAI
    if model_lower.contains("gpt-4o-mini") {
        return (150_000, 600_000);
    }
    if model_lower.contains("gpt-4o") {
        return (2_500_000, 10_000_000);
    }
    if model_lower.contains("gpt-4") {
        return (30_000_000, 60_000_000);
    }
    if model_lower.contains("o1") || model_lower.contains("o3") {
        return (15_000_000, 60_000_000);
    }

    // Default moderate estimate.
    (2_000_000, 6_000_000)
}

// ===========================================================================
// Factory: create backend from config
// ===========================================================================

/// Create an API backend from a [`BackendConfig`].
///
/// Resolves API keys from environment variables if `api_key_env` is set.
pub fn create_from_config(id: &str, config: &BackendConfig) -> Option<Box<dyn AiBackend>> {
    let api_key = resolve_api_key(config);

    match config.backend_type.as_str() {
        "openai" => {
            let model = if config.model.is_empty() { "gpt-4o" } else { &config.model };
            let api_base = if config.api_base.is_empty() { "https://api.openai.com/v1" } else { &config.api_base };
            let mut backend = OpenAiCompatibleBackend::new(
                id, format!("OpenAI {}", model), api_base, &api_key, model,
                config.tags.clone().unwrap_or_else(|| EngineTags {
                    categories: vec![EngineCategory::Intellectual, EngineCategory::Balanced],
                    capabilities: vec!["code".into(), "function-calling".into()],
                    cost_tier: 4, speed_tier: 3, quality_tier: 5,
                }),
            );
            if let Some(max) = config.max_tokens { backend = backend.with_max_tokens(max); }
            if let Some(temp) = config.temperature { backend = backend.with_temperature(temp); }
            Some(Box::new(backend))
        }

        "anthropic" => {
            let model = if config.model.is_empty() { "claude-sonnet-4-20250514" } else { &config.model };
            let mut backend = AnthropicApiBackend::new(model, &api_key);
            if let Some(max) = config.max_tokens { backend = backend.with_max_tokens(max); }
            if let Some(ref tags) = config.tags { backend = backend.with_tags(tags.clone()); }
            Some(Box::new(backend))
        }

        "deepseek" => {
            let model = if config.model.is_empty() { "deepseek-coder" } else { &config.model };
            let mut backend = OpenAiCompatibleBackend::deepseek(model, &api_key);
            if let Some(max) = config.max_tokens { backend = backend.with_max_tokens(max); }
            if let Some(temp) = config.temperature { backend = backend.with_temperature(temp); }
            if let Some(ref tags) = config.tags {
                // Override default tags — need a new backend with those tags.
                backend = OpenAiCompatibleBackend::new(
                    &backend.id, &backend.display_name, &backend.api_base,
                    &backend.api_key, &backend.model, tags.clone(),
                );
            }
            Some(Box::new(backend))
        }

        "groq" => {
            let model = if config.model.is_empty() { "llama-3.1-70b-versatile" } else { &config.model };
            let mut backend = OpenAiCompatibleBackend::groq(model, &api_key);
            if let Some(max) = config.max_tokens { backend = backend.with_max_tokens(max); }
            if let Some(temp) = config.temperature { backend = backend.with_temperature(temp); }
            Some(Box::new(backend))
        }

        "together" => {
            let model = if config.model.is_empty() { "meta-llama/Llama-3-70b-chat-hf" } else { &config.model };
            let mut backend = OpenAiCompatibleBackend::together(model, &api_key);
            if let Some(max) = config.max_tokens { backend = backend.with_max_tokens(max); }
            if let Some(temp) = config.temperature { backend = backend.with_temperature(temp); }
            Some(Box::new(backend))
        }

        "openrouter" => {
            let model = if config.model.is_empty() { "anthropic/claude-3.5-sonnet" } else { &config.model };
            let mut backend = OpenAiCompatibleBackend::openrouter(model, &api_key);
            if let Some(max) = config.max_tokens { backend = backend.with_max_tokens(max); }
            if let Some(temp) = config.temperature { backend = backend.with_temperature(temp); }
            Some(Box::new(backend))
        }

        "openai-compatible" => {
            if config.api_base.is_empty() {
                log::warn!("Backend '{}': openai-compatible requires api_base", id);
                return None;
            }
            let model = if config.model.is_empty() { "default" } else { &config.model };
            let mut backend = OpenAiCompatibleBackend::new(
                id, format!("Custom ({})", model), &config.api_base, &api_key, model,
                config.tags.clone().unwrap_or_default(),
            );
            if let Some(max) = config.max_tokens { backend = backend.with_max_tokens(max); }
            if let Some(temp) = config.temperature { backend = backend.with_temperature(temp); }
            Some(Box::new(backend))
        }

        other => {
            log::warn!("Unknown backend type '{}' for '{}'", other, id);
            None
        }
    }
}

/// Resolve an API key: prefer env var, fall back to literal value.
fn resolve_api_key(config: &BackendConfig) -> String {
    if !config.api_key_env.is_empty() {
        if let Ok(key) = std::env::var(&config.api_key_env) {
            return key;
        }
    }
    config.api_key.clone()
}
