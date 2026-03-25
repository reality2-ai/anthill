//! Tool-calling proxy — enables API backends (Ollama, LM Studio, OpenAI) to
//! use MCP tools by running a multi-turn tool-call loop.
//!
//! The proxy:
//! 1. Sends the prompt + tool definitions to the model
//! 2. If the model returns tool calls, executes them via the MCP handler
//! 3. Feeds tool results back to the model
//! 4. Repeats until the model gives a final text response (max 10 rounds)

use std::path::Path;

use crate::store::live::LiveKnowledgeStore;

/// OpenAI-compatible tool definition for API backends.
pub fn openai_tool_definitions() -> Vec<serde_json::Value> {
    crate::mcp::tool_definitions()
        .into_iter()
        .map(|tool| {
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": tool["name"],
                    "description": tool["description"],
                    "parameters": tool["inputSchema"],
                }
            })
        })
        .collect()
}

/// Execute a tool call using the in-process MCP handler.
pub fn execute_tool(
    name: &str,
    arguments: &serde_json::Value,
    store: &LiveKnowledgeStore,
    memory_dir: &std::path::Path,
) -> String {
    crate::mcp::handle_tool_call(name, arguments, store, memory_dir)
}

/// Run a multi-turn tool-calling loop with an OpenAI-compatible API.
///
/// Returns the final text response after all tool calls are resolved.
pub async fn run_tool_loop(
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    system_prompt: &str,
    user_message: &str,
    memory_dir: &Path,
    progress_tx: &crate::ai_backends::ProgressTx,
    task_id: u32,
    backend_id: &str,
) -> Result<(String, Option<(usize, usize)>), crate::ai_backends::AiError> {
    let store = LiveKnowledgeStore::new(memory_dir.to_path_buf());
    let tools = openai_tool_definitions();

    let mut messages = vec![
        serde_json::json!({"role": "system", "content": system_prompt}),
        serde_json::json!({"role": "user", "content": user_message}),
    ];

    let max_rounds = 10;
    let mut total_input = 0usize;
    let mut total_output = 0usize;

    for round in 0..max_rounds {
        let mut payload = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": false,
        });

        // Only include tools if we have them.
        if !tools.is_empty() {
            payload["tools"] = serde_json::json!(tools);
        }

        let mut req = client.post(format!("{}/chat/completions", api_url))
            .json(&payload);
        if !api_key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", api_key));
        }

        let resp = req.send().await
            .map_err(|e| crate::ai_backends::AiError::network(e.to_string()))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err = resp.text().await.unwrap_or_default();
            return Err(crate::ai_backends::AiError::classify(
                &format!("{} ({}): {}", backend_id, status, err)
            ));
        }

        let data: serde_json::Value = resp.json().await
            .map_err(|e| crate::ai_backends::AiError::parse(e.to_string()))?;

        // Track tokens.
        if let Some(usage) = data.get("usage") {
            total_input += usage.get("prompt_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
            total_output += usage.get("completion_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
        }

        let choice = &data["choices"][0];
        let message = &choice["message"];
        let finish_reason = choice.get("finish_reason").and_then(|r| r.as_str()).unwrap_or("");

        // Check for tool calls.
        let tool_calls = message.get("tool_calls").and_then(|tc| tc.as_array());

        if let Some(calls) = tool_calls {
            if !calls.is_empty() {
                // Add the assistant's tool-call message to the conversation.
                messages.push(message.clone());

                for call in calls {
                    let call_id = call.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let func = &call["function"];
                    let tool_name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let args_str = func.get("arguments").and_then(|a| a.as_str()).unwrap_or("{}");
                    let args: serde_json::Value = serde_json::from_str(args_str)
                        .unwrap_or(serde_json::json!({}));

                    let _ = progress_tx.send(crate::ai_backends::AiProgress {
                        task_id,
                        kind: "tool_use".into(),
                        detail: format!("Calling {}...", tool_name),
                    });

                    log::info!("[{}] Tool call round {}: {}({})", backend_id, round + 1, tool_name, args_str);
                    let result = execute_tool(tool_name, &args, &store, memory_dir);

                    // Add tool result to conversation.
                    messages.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": result,
                    }));
                }
                continue; // Next round.
            }
        }

        // No tool calls — extract final text response.
        let text = message.get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();

        if text.is_empty() && round < max_rounds - 1 && finish_reason != "stop" {
            continue;
        }

        let tokens = if total_input > 0 || total_output > 0 {
            Some((total_input, total_output))
        } else {
            None
        };

        return Ok((text, tokens));
    }

    Err(crate::ai_backends::AiError::retriable(
        format!("{}: tool loop exceeded {} rounds", backend_id, max_rounds)
    ))
}
