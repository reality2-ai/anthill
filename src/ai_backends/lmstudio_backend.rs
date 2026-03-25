//! LM Studio backend — local inference using LM Studio's OpenAI-compatible API.
//!
//! Uses the tool-calling proxy to give LM Studio access to MCP tools
//! (knowledge graph, colony communication, etc.).

use super::types::*;

#[derive(Debug)]
pub struct LmStudioBackend {
    id: String,
    display_name: String,
    base_url: String,
    model: String,
    tags: EngineTags,
}

impl LmStudioBackend {
    pub fn new(model: &str, base_url: Option<&str>) -> Self {
        let base = base_url.unwrap_or("http://localhost:1234/v1");
        Self {
            id: format!("lmstudio-{}", model.replace(['/', ':', '.'], "-")),
            display_name: format!("LM Studio ({})", model),
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
impl AiBackend for LmStudioBackend {
    fn id(&self) -> &str { &self.id }
    fn name(&self) -> &str { &self.display_name }
    fn tags(&self) -> &EngineTags { &self.tags }

    async fn is_available(&self) -> bool {
        let url = format!("{}/models", self.base_url);
        reqwest::get(&url).await.is_ok()
    }

    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: ProgressTx,
    ) -> Result<AiResponse, AiError> {
        let _ = progress_tx.send(AiProgress {
            task_id: request.task_id,
            kind: "thinking".into(),
            detail: format!("Calling LM Studio ({})...", self.model),
        });

        if let Some(ref memory_dir) = request.memory_dir {
            // Use tool-calling proxy for MCP access.
            let client = reqwest::Client::new();
            let (text, tokens) = super::tool_proxy::run_tool_loop(
                &client,
                &self.base_url,
                "", // No API key for local LM Studio.
                &self.model,
                &request.system_prompt,
                &request.message,
                memory_dir,
                &progress_tx,
                request.task_id,
                &self.id,
            ).await?;

            if text.is_empty() {
                return Err(AiError::retriable(format!("LM Studio ({}): empty response", self.model)));
            }

            Ok(AiResponse {
                text,
                backend_id: self.id.clone(),
                tokens,
                cost_microdollars: Some(0),
            })
        } else {
            // Fallback: simple chat completion without tools.
            let client = reqwest::Client::new();
            let payload = serde_json::json!({
                "model": self.model,
                "messages": [
                    {"role": "system", "content": request.system_prompt},
                    {"role": "user", "content": request.message}
                ],
                "stream": false,
            });

            let resp = client
                .post(format!("{}/chat/completions", self.base_url))
                .json(&payload)
                .send()
                .await
                .map_err(|e| AiError::network(e.to_string()))?;

            if !resp.status().is_success() {
                let err = resp.text().await.unwrap_or_default();
                return Err(AiError::api(format!("LM Studio error: {}", err)));
            }

            let data: serde_json::Value = resp.json().await
                .map_err(|e| AiError::parse(e.to_string()))?;

            let text = data["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("")
                .to_string();

            if text.is_empty() {
                return Err(AiError::retriable(format!("LM Studio ({}): empty response", self.model)));
            }

            Ok(AiResponse {
                text,
                backend_id: self.id.clone(),
                tokens: None,
                cost_microdollars: Some(0),
            })
        }
    }
}
