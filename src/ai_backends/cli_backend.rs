//! CLI-based AI backends — wraps `claude`, `codex`, and `gemini` binaries.
//!
//! These backends spawn a child process, read stream-JSON from stdout,
//! and parse progress + result lines.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use super::types::*;

// ---------------------------------------------------------------------------
// Claude CLI backend
// ---------------------------------------------------------------------------

/// Claude Code CLI backend (`claude -p --output-format stream-json`).
#[derive(Debug)]
pub struct ClaudeCliBackend {
    tags: EngineTags,
}

impl ClaudeCliBackend {
    pub fn new() -> Self {
        Self {
            tags: EngineTags {
                categories: vec![
                    EngineCategory::Intellectual,
                    EngineCategory::Balanced,
                    EngineCategory::Specialized("coding".into()),
                ],
                capabilities: vec!["code".into(), "function-calling".into(), "file-access".into()],
                cost_tier: 4,
                speed_tier: 3,
                quality_tier: 5,
            },
        }
    }

    pub fn with_tags(tags: EngineTags) -> Self {
        Self { tags }
    }
}

#[async_trait::async_trait]
impl AiBackend for ClaudeCliBackend {
    fn id(&self) -> &str { "claude-cli" }
    fn name(&self) -> &str { "Claude Code (CLI)" }
    fn tags(&self) -> &EngineTags { &self.tags }

    async fn is_available(&self) -> bool {
        is_command_available("claude")
    }

    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: ProgressTx,
    ) -> Result<AiResponse, AiError> {
        let mut args = vec![
            "-p".to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];
        if request.skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }
        if request.continue_session {
            args.push("-c".to_string());
        }
        if !request.working_dir.is_empty() {
            args.push("--add-dir".to_string());
            args.push(request.working_dir.clone());
        }
        args.push("--append-system-prompt".to_string());
        args.push(request.system_prompt.clone());
        args.push(request.message.clone());

        run_cli_backend("claude", &args, &request.working_dir, request.task_id, "claude-cli", progress_tx).await
    }
}

// ---------------------------------------------------------------------------
// Codex CLI backend
// ---------------------------------------------------------------------------

/// OpenAI Codex CLI backend (`codex exec --json`).
#[derive(Debug)]
pub struct CodexCliBackend {
    tags: EngineTags,
}

impl CodexCliBackend {
    pub fn new() -> Self {
        Self {
            tags: EngineTags {
                categories: vec![
                    EngineCategory::Balanced,
                    EngineCategory::Specialized("coding".into()),
                ],
                capabilities: vec!["code".into(), "function-calling".into()],
                cost_tier: 3,
                speed_tier: 3,
                quality_tier: 4,
            },
        }
    }

    pub fn with_tags(tags: EngineTags) -> Self {
        Self { tags }
    }
}

#[async_trait::async_trait]
impl AiBackend for CodexCliBackend {
    fn id(&self) -> &str { "codex-cli" }
    fn name(&self) -> &str { "OpenAI Codex (CLI)" }
    fn tags(&self) -> &EngineTags { &self.tags }

    async fn is_available(&self) -> bool {
        is_command_available("codex")
    }

    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: ProgressTx,
    ) -> Result<AiResponse, AiError> {
        let args = vec![
            "exec".to_string(),
            "--json".to_string(),
            request.message.clone(),
        ];

        run_cli_backend("codex", &args, &request.working_dir, request.task_id, "codex-cli", progress_tx).await
    }
}

// ---------------------------------------------------------------------------
// Gemini CLI backend
// ---------------------------------------------------------------------------

/// Google Gemini CLI backend (`gemini -p --output-format stream-json`).
#[derive(Debug)]
pub struct GeminiCliBackend {
    tags: EngineTags,
}

impl GeminiCliBackend {
    pub fn new() -> Self {
        Self {
            tags: EngineTags {
                categories: vec![EngineCategory::Balanced, EngineCategory::Fast],
                capabilities: vec!["code".into(), "function-calling".into()],
                cost_tier: 3,
                speed_tier: 4,
                quality_tier: 4,
            },
        }
    }

    pub fn with_tags(tags: EngineTags) -> Self {
        Self { tags }
    }
}

#[async_trait::async_trait]
impl AiBackend for GeminiCliBackend {
    fn id(&self) -> &str { "gemini-cli" }
    fn name(&self) -> &str { "Google Gemini (CLI)" }
    fn tags(&self) -> &EngineTags { &self.tags }

    async fn is_available(&self) -> bool {
        is_command_available("gemini")
    }

    async fn execute(
        &self,
        request: &AiRequest,
        progress_tx: ProgressTx,
    ) -> Result<AiResponse, AiError> {
        let mut args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];
        args.push("--append-system-prompt".to_string());
        args.push(request.system_prompt.clone());
        args.push(request.message.clone());

        run_cli_backend("gemini", &args, &request.working_dir, request.task_id, "gemini-cli", progress_tx).await
    }
}

// ---------------------------------------------------------------------------
// Shared CLI execution logic
// ---------------------------------------------------------------------------

/// Check if a command-line tool is installed.
fn is_command_available(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Slice a string at a char boundary (no suffix added).
fn slice_safe(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes { return s; }
    let mut end = max_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) { end -= 1; }
    &s[..end]
}

/// Run a CLI backend, streaming progress and collecting the result.
///
/// This is the shared core for claude, codex, and gemini.
async fn run_cli_backend(
    cmd_name: &str,
    args: &[String],
    working_dir: &str,
    task_id: u32,
    backend_id: &str,
    progress_tx: ProgressTx,
) -> Result<AiResponse, AiError> {
    let mut cmd = tokio::process::Command::new(cmd_name);
    cmd.args(args);
    if !working_dir.is_empty() {
        cmd.current_dir(working_dir);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    cmd.kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|e| {
        AiError::retriable(format!("Failed to run {}: {}", cmd_name, e))
    })?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let activity_for_reader = Arc::clone(&last_activity);

    // Read stdout (stream-json progress + results).
    let cmd_for_parse = cmd_name.to_string();
    let backend_id_str = backend_id.to_string();
    let stdout_handle = {
        let ptx = progress_tx.clone();
        tokio::spawn(async move {
            let mut result_text = String::new();
            if let Some(stdout) = stdout {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut reader = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = reader.next_line().await {
                    if let Ok(mut t) = activity_for_reader.lock() { *t = Instant::now(); }

                    match serde_json::from_str::<serde_json::Value>(&line) {
                        Ok(json) => {
                            let (prog, result) = parse_backend_line(&cmd_for_parse, &json);
                            if let Some((kind, detail)) = prog {
                                let _ = ptx.send(AiProgress { task_id, kind, detail });
                            }
                            if let Some(text) = result {
                                if !text.is_empty() {
                                    result_text = text;
                                }
                            }
                        }
                        Err(_) => {
                            // Non-JSON lines (e.g. raw stderr mixed into stdout) — ignore.
                        }
                    }
                }
            }
            result_text
        })
    };

    // Read stderr.
    let stderr_handle = tokio::spawn(async move {
        let mut stderr_text = String::new();
        if let Some(stderr) = stderr {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                if stderr_text.len() < 4096 {
                    if !stderr_text.is_empty() { stderr_text.push('\n'); }
                    stderr_text.push_str(&line);
                }
            }
        }
        stderr_text
    });

    // Stall watchdog — warns after 2 min idle, kills after 10 min.
    let child_id = child.id();
    let watchdog_activity = Arc::clone(&last_activity);
    let watchdog_ptx = progress_tx.clone();
    let watchdog_handle = tokio::spawn(async move {
        let stall_warn_secs = 120u64;
        let hard_timeout_secs = 600u64;
        let mut warned = false;
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            let idle_secs = watchdog_activity.lock()
                .map(|t| t.elapsed().as_secs())
                .unwrap_or(0);

            if !warned && idle_secs > stall_warn_secs {
                warned = true;
                let _ = watchdog_ptx.send(AiProgress {
                    task_id,
                    kind: "warning".into(),
                    detail: format!("No output for {}s — worker may be stalled", idle_secs),
                });
            }
            if warned && idle_secs < stall_warn_secs {
                warned = false;
            }

            if idle_secs > hard_timeout_secs {
                #[cfg(unix)]
                if let Some(pid) = child_id {
                    unsafe { libc::killpg(pid as i32, libc::SIGKILL); }
                }
                break;
            }
        }
    });

    // Wait for completion.
    let (stdout_result, stderr_result, status) = tokio::join!(
        stdout_handle, stderr_handle, child.wait()
    );
    watchdog_handle.abort();

    let result_text = stdout_result.unwrap_or_default();
    let stderr_text = stderr_result.unwrap_or_default();
    let success = status.as_ref().map(|s| s.success()).unwrap_or(false);

    if !result_text.is_empty() && success {
        Ok(AiResponse {
            text: result_text,
            backend_id: backend_id_str,
            tokens: None,
            cost_microdollars: None,
        })
    } else if !result_text.is_empty() && !stderr_text.is_empty() {
        // Partial output with error — return what we got.
        Ok(AiResponse {
            text: format!("{}\n\n⚠️ Backend error (output may be incomplete): {}",
                result_text, stderr_text.lines().next().unwrap_or("")),
            backend_id: backend_id_str,
            tokens: None,
            cost_microdollars: None,
        })
    } else {
        let detail = if !result_text.is_empty() {
            result_text
        } else if !stderr_text.is_empty() {
            stderr_text
        } else {
            let code = status.as_ref()
                .map(|s| format!("exit {}", s))
                .unwrap_or_else(|e| format!("{}", e));
            format!("process {}", code)
        };
        Err(AiError::classify(&format!("{} failed: {}", cmd_name, detail)))
    }
}

// ---------------------------------------------------------------------------
// Line parsing — extracted from the original ai_worker.rs
// ---------------------------------------------------------------------------

/// Parse a stream-JSON line from any CLI backend.
/// Returns (progress, result_text) where each is optional.
pub fn parse_backend_line(
    backend: &str,
    json: &serde_json::Value,
) -> (Option<(String, String)>, Option<String>) {
    match backend {
        "codex" => parse_codex_line(json),
        "gemini" => parse_gemini_line(json),
        _ => parse_claude_line(json), // claude is the default
    }
}

fn parse_claude_line(json: &serde_json::Value) -> (Option<(String, String)>, Option<String>) {
    let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match msg_type {
        "assistant" => {
            // Extract the result text from the message content.
            let content = json.get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_array());
            if let Some(blocks) = content {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                        if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                            return (None, Some(text.to_string()));
                        }
                    }
                }
            }
            (None, None)
        }
        "result" => {
            let result = json.get("result").and_then(|r| r.as_str());
            let _cost_usd = json.get("cost_usd").and_then(|c| c.as_f64());
            if let Some(text) = result {
                return (None, Some(text.to_string()));
            }
            (None, None)
        }
        "tool_use" | "tool_result" => {
            let tool_name = json.get("tool")
                .or_else(|| json.pointer("/content/name"))
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            let detail = match tool_name {
                "Read" | "read_file" => {
                    let path = json.pointer("/input/file_path")
                        .or_else(|| json.pointer("/content/input/file_path"))
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    format!("Reading: {}", path)
                }
                "Edit" | "edit_file" => {
                    let path = json.pointer("/input/file_path")
                        .or_else(|| json.pointer("/content/input/file_path"))
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    format!("Editing: {}", path)
                }
                "Write" | "write_file" => {
                    let path = json.pointer("/input/file_path")
                        .or_else(|| json.pointer("/content/input/file_path"))
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    format!("Writing: {}", path)
                }
                "Bash" | "bash" | "execute_command" => {
                    let cmd = json.pointer("/input/command")
                        .or_else(|| json.pointer("/content/input/command"))
                        .and_then(|c| c.as_str())
                        .unwrap_or("");
                    let short = if cmd.len() > 60 { slice_safe(cmd, 57) } else { cmd };
                    format!("Running: {}", short)
                }
                "Glob" | "glob" => {
                    let pattern = json.pointer("/input/pattern")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    format!("Searching: {}", pattern)
                }
                "Grep" | "grep" => {
                    let pattern = json.pointer("/input/pattern")
                        .and_then(|p| p.as_str())
                        .unwrap_or("");
                    format!("Grep: {}", pattern)
                }
                _ => format!("Tool: {}", tool_name),
            };
            (Some(("tool_use".into(), detail)), None)
        }
        "user_input_request" => {
            let question = json.get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Waiting for input...");
            (Some(("question".into(), question.to_string())), None)
        }
        _ => (None, None),
    }
}

fn parse_codex_line(json: &serde_json::Value) -> (Option<(String, String)>, Option<String>) {
    let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match msg_type {
        "item.completed" => {
            let item_type = json.pointer("/item/type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                "agent_message" => {
                    let text = json.pointer("/item/text").and_then(|t| t.as_str()).unwrap_or("");
                    (None, Some(text.to_string()))
                }
                "command_execution" => {
                    let cmd = json.pointer("/item/command").and_then(|c| c.as_str()).unwrap_or("");
                    let short = if cmd.len() > 60 { slice_safe(cmd, 57) } else { cmd };
                    (Some(("tool_use".into(), format!("Running: {}", short))), None)
                }
                _ => (None, None),
            }
        }
        _ => (None, None),
    }
}

fn parse_gemini_line(json: &serde_json::Value) -> (Option<(String, String)>, Option<String>) {
    // Gemini uses the same stream-json format as Claude.
    parse_claude_line(json)
}

// ---------------------------------------------------------------------------
// Auto-detect CLI backends
// ---------------------------------------------------------------------------

/// Detect which CLI backends are installed.
/// Returns (id, display_name, available).
pub fn detect_cli_backends() -> Vec<(&'static str, &'static str, bool)> {
    vec![
        ("claude-cli", "Claude Code", is_command_available("claude")),
        ("codex-cli", "OpenAI Codex", is_command_available("codex")),
        ("gemini-cli", "Google Gemini", is_command_available("gemini")),
    ]
}
