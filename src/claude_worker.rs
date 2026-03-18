//! Background Claude API worker.
//!
//! Reads `AiRequest`s from a channel, calls the Anthropic Messages API,
//! and pushes `AiResponse`s into a shared queue for AiPlugin to poll.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Kind of AI request / response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiKind {
    /// Translate natural language → shell command.
    Translate = 0,
    /// Summarise terminal output for mobile.
    Summarise = 1,
}

/// A request to the Claude API.
#[derive(Debug)]
pub struct AiRequest {
    pub kind: AiKind,
    pub chat_id: i64,
    /// The user message or terminal output to process.
    pub content: String,
    /// Conversation history for context.
    pub history: Vec<ConversationTurn>,
}

/// A response from the Claude API.
#[derive(Debug)]
pub struct AiResponse {
    pub kind: AiKind,
    pub chat_id: i64,
    /// The Claude output (shell command or summary text).
    pub text: String,
}

/// One turn of conversation context.
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    pub user_input: String,
    pub command: String,
}

/// Payload types for the Messages API.
#[derive(serde::Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: u32,
    system: String,
    messages: Vec<ApiMessage>,
}

#[derive(serde::Serialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(serde::Deserialize)]
struct ApiResponse {
    content: Option<Vec<ContentBlock>>,
    error: Option<ApiError>,
}

#[derive(serde::Deserialize)]
struct ContentBlock {
    text: Option<String>,
}

#[derive(serde::Deserialize)]
struct ApiError {
    message: String,
}

const TRANSLATE_SYSTEM: &str =
    "Output ONLY the shell command. No explanation, no markdown, no backticks. Just the command.";

const SUMMARISE_SYSTEM: &str =
    "Summarise this terminal output concisely for a mobile screen. Use plain text, not markdown. Be brief.";

/// Run the Claude API worker loop.
///
/// Reads requests from `rx`, calls the API, and pushes responses into
/// the shared `response_queue`. Runs until the channel is closed.
pub async fn claude_worker(
    mut rx: mpsc::UnboundedReceiver<AiRequest>,
    response_queue: Arc<Mutex<VecDeque<AiResponse>>>,
    api_key: String,
    model: String,
) {
    let client = reqwest::Client::new();

    while let Some(req) = rx.recv().await {
        let system = match req.kind {
            AiKind::Translate => TRANSLATE_SYSTEM,
            AiKind::Summarise => SUMMARISE_SYSTEM,
        };

        // Build messages array with conversation history.
        let mut messages = Vec::new();

        if req.kind == AiKind::Translate {
            // Include history for context.
            for turn in &req.history {
                messages.push(ApiMessage {
                    role: "user".into(),
                    content: turn.user_input.clone(),
                });
                messages.push(ApiMessage {
                    role: "assistant".into(),
                    content: turn.command.clone(),
                });
            }
        }

        messages.push(ApiMessage {
            role: "user".into(),
            content: req.content.clone(),
        });

        let api_req = ApiRequest {
            model: model.clone(),
            max_tokens: 1024,
            system: system.to_string(),
            messages,
        };

        let response_text = match client
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&api_req)
            .send()
            .await
        {
            Ok(resp) => match resp.json::<ApiResponse>().await {
                Ok(api_resp) => {
                    if let Some(err) = api_resp.error {
                        format!("API error: {}", err.message)
                    } else if let Some(blocks) = api_resp.content {
                        blocks
                            .into_iter()
                            .filter_map(|b| b.text)
                            .collect::<Vec<_>>()
                            .join("")
                    } else {
                        "API error: empty response".into()
                    }
                }
                Err(e) => format!("API error: {}", e),
            },
            Err(e) => format!("API error: {}", e),
        };

        let response = AiResponse {
            kind: req.kind,
            chat_id: req.chat_id,
            text: response_text,
        };

        if let Ok(mut q) = response_queue.lock() {
            q.push_back(response);
        }
    }
}
