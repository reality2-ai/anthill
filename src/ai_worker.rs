//! Background Claude CLI worker.
//!
//! Spawns `claude -p` tasks concurrently. Each request runs in its own
//! tokio task, allowing multiple requests to be in flight simultaneously.
//! Maintains per-user memory files and session continuity.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::mpsc;

/// A request to run claude CLI.
#[derive(Debug)]
pub struct CliRequest {
    pub chat_id: i64,
    pub message: String,
    /// If true, force a new session (don't use -c).
    pub new_session: bool,
    /// Unique task ID for tracking.
    pub task_id: u32,
    /// Where this message came from: "telegram", "slack", "web"
    pub source: String,
}

/// A response from claude CLI.
#[derive(Debug)]
pub struct CliResponse {
    pub chat_id: i64,
    pub text: String,
    #[allow(dead_code)]
    pub task_id: u32,
}

/// Configuration passed to the worker at startup.
#[derive(Debug, Clone)]
pub struct CliWorkerConfig {
    pub working_dir: String,
    pub memory_dir: PathBuf,
    pub repos_dir: PathBuf,
    pub system_prompt: Option<String>,
    pub skip_permissions: bool,
    pub sync_channels: bool,
    pub backends: Vec<String>,
    /// Worker timeout in seconds (0 = no timeout). Default: 600 (10 minutes).
    pub worker_timeout_secs: u64,
}

/// Per-user usage statistics (shared with sentant for /usage command).
#[derive(Debug, Default)]
pub struct UserStats {
    pub messages: u32,
    pub input_chars: u64,
    pub output_chars: u64,
    pub started: Option<Instant>,
}

/// Tracks a running task.
#[derive(Debug)]
pub struct RunningTask {
    pub task_id: u32,
    pub chat_id: i64,
    pub message_preview: String,
    pub started: Instant,
    pub handle: tokio::task::JoinHandle<()>,
    /// What this worker is doing right now (latest progress detail).
    pub last_progress: Arc<Mutex<Option<String>>>,
    /// Which AI backend is running this task.
    pub backend: Arc<Mutex<String>>,
}

/// A queued follow-up message for a running task's session.
#[derive(Debug)]
pub struct FollowUp {
    pub chat_id: i64,
    pub message: String,
    pub source: String,
}

/// Follow-up queue: messages queued to run after a task's current work completes.
pub type FollowUpQueue = Arc<Mutex<HashMap<u32, Vec<FollowUp>>>>;

/// All user stats, keyed by chat_id.
pub type StatsMap = Arc<Mutex<HashMap<i64, UserStats>>>;

/// All running tasks.
pub type TaskMap = Arc<Mutex<HashMap<u32, RunningTask>>>;

const MEMORY_PREAMBLE: &str = "\
You have a knowledge graph and a per-user memory file:\n\
- Knowledge graph (shared, structured): memory/knowledge.json — shown below as [KNOWLEDGE GRAPH]\n\
- User memory (per-user, freeform): shown below as [USER MEMORY]\n\n\
THE KNOWLEDGE GRAPH IS POPPERIAN — all edges are conjectures with confidence weights.\n\
Edges gain strength by surviving refutation, not by confirmation.\n\n\
AFTER EVERY RESPONSE, silently review and update the knowledge graph:\n\
  1. New entity? → add a node\n\
  2. New relationship? → add an edge as a CONJECTURE with appropriate basis and confidence\n\
  3. Does this conversation CONFIRM an existing edge? → increment 'survived' and 'tests'\n\
  4. Does this conversation CONTRADICT an existing edge? → increment 'tests' only (weakens it)\n\
  5. Direct contradiction with strong evidence? → set confidence *= 0.3\n\
  6. User-specific fact? → append to user memory file\n\
Do this WITHOUT telling the user — just quietly read, modify, and write the files.\n\n\
Knowledge graph JSON format:\n\
  nodes: [{\"label\": \"...\", \"kind\": \"person|project|server|tool|concept|decision|event|fact\",\n\
           \"summary\": \"...\", \"created\": \"YYYY-MM-DD\", \"updated\": \"YYYY-MM-DD\", \"tags\": [...]}]\n\
  edges: [[from_idx, to_idx, {\n\
    \"relation\": \"...\", \"context\": \"...\", \"since\": \"YYYY-MM-DD\",\n\
    \"confidence\": 0.0-1.0, \"tests\": N, \"survived\": N,\n\
    \"basis\": \"observed|told|inferred|assumed\", \"last_tested\": \"YYYY-MM-DD\"\n\
  }]]\n\
Initial confidence by basis: observed=0.7, told=0.6, inferred=0.4, assumed=0.3\n\
Confidence formula: blend(basis_prior, survived/tests) weighted by test count.\n\
Edges below 0.15 confidence are hidden from this prompt but kept in the graph.\n\
Importance: edges have an 'importance' field (0-1) and 'references' count.\n\
Set importance higher for knowledge central to the project. It grows with references.\n\
Keep nodes concise. Use tags for searchability. Date everything.\n\n\
EPISODIC MEMORY — memory/episodes.json:\n\
After significant conversations (not trivial questions), append an episode:\n\
{\"date\": \"YYYY-MM-DD\", \"participants\": [...], \"summary\": \"2-3 sentences\",\n\
 \"outcomes\": [\"key decisions or results\"], \"tags\": [\"searchable\", \"keywords\"]}\n\
Episodes capture WHAT HAPPENED — the narrative, not just facts.\n\
Recent episodes are shown below as [EPISODES].";

const WORKSPACE_PREAMBLE: &str = "\
Your working directory has the following structure:\
\n- memory/ — per-user persistent memory files (auto-backed up)\
\n- repos/ — for cloning git repositories (NOT backed up, repos have their own git history)\
\n\
\nWhen cloning repositories, ALWAYS clone into the repos/ subdirectory.\
\nThe working directory is a git repo that is automatically committed on a schedule. \
The repos/ folder is excluded from these backups via .gitignore since cloned repos \
already have their own version control.";

/// Detect which AI backends are installed on this system.
pub fn detect_backends() -> Vec<(String, bool)> {
    let backends = vec![
        ("claude", "claude"),
        ("codex", "codex"),
        ("gemini", "gemini"),
        ("ollama", "ollama"),
    ];
    backends
        .iter()
        .map(|(name, cmd)| {
            let installed = std::process::Command::new("which")
                .arg(cmd)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            (name.to_string(), installed)
        })
        .collect()
}

/// Build command args for a specific backend.
fn build_backend_command(
    backend: &str,
    message: &str,
    system_prompt: &str,
    _working_dir: &str,
    skip_permissions: bool,
    continue_session: bool,
) -> (String, Vec<String>) {
    match backend {
        "codex" => {
            let mut args = vec!["exec".to_string(), "--json".to_string()];
            args.push(message.to_string());
            ("codex".to_string(), args)
        }
        _ => {
            // Claude (default).
            let mut args = vec![
                "-p".to_string(),
                "--verbose".to_string(),
                "--output-format".to_string(),
                "stream-json".to_string(),
            ];
            if skip_permissions {
                args.push("--dangerously-skip-permissions".to_string());
            }
            if continue_session {
                args.push("-c".to_string());
            }
            args.push("--append-system-prompt".to_string());
            args.push(system_prompt.to_string());
            args.push(message.to_string());
            ("claude".to_string(), args)
        }
    }
}

/// Parse a response line from any backend. Returns (progress_detail, result_text) if applicable.
fn parse_backend_line(backend: &str, json: &serde_json::Value) -> (Option<(String, String)>, Option<String>) {
    match backend {
        "codex" => {
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
                            let short = if cmd.len() > 60 { &cmd[..57] } else { cmd };
                            (Some(("tool_use".into(), format!("Running: {}", short))), None)
                        }
                        _ => (None, None)
                    }
                }
                "item.started" => {
                    let item_type = json.pointer("/item/type").and_then(|t| t.as_str()).unwrap_or("");
                    if item_type == "command_execution" {
                        let cmd = json.pointer("/item/command").and_then(|c| c.as_str()).unwrap_or("");
                        let short = if cmd.len() > 60 { &cmd[..57] } else { cmd };
                        (Some(("tool_use".into(), format!("Running: {}", short))), None)
                    } else {
                        (None, None)
                    }
                }
                _ => (None, None)
            }
        }
        _ => {
            // Claude stream-json parsing.
            let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match msg_type {
                "assistant" => {
                    if let Some(content) = json.pointer("/message/content") {
                        if let Some(arr) = content.as_array() {
                            // Collect text blocks as partial result (in case no "result" event follows).
                            let mut text_parts = Vec::new();
                            for block in arr {
                                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                                if block_type == "text" {
                                    if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                        if !t.is_empty() { text_parts.push(t.to_string()); }
                                    }
                                }
                                if block_type == "tool_use" {
                                    let tool = block.get("name").and_then(|n| n.as_str()).unwrap_or("unknown");
                                    let detail = match tool {
                                        "Bash" => {
                                            let cmd = block.pointer("/input/command").and_then(|c| c.as_str()).unwrap_or("");
                                            let short = if cmd.len() > 60 { &cmd[..57] } else { cmd };
                                            format!("Running: {}", short)
                                        }
                                        "Read" => {
                                            let path = block.pointer("/input/file_path").and_then(|p| p.as_str()).unwrap_or("");
                                            format!("Reading: {}", path.rsplit('/').next().unwrap_or(path))
                                        }
                                        "Edit" | "Write" => {
                                            let path = block.pointer("/input/file_path").and_then(|p| p.as_str()).unwrap_or("");
                                            format!("Writing: {}", path.rsplit('/').next().unwrap_or(path))
                                        }
                                        "Glob" | "Grep" => {
                                            let pattern = block.pointer("/input/pattern").and_then(|p| p.as_str()).unwrap_or("");
                                            format!("Searching: {}", pattern)
                                        }
                                        "Agent" => {
                                            let desc = block.pointer("/input/description").and_then(|d| d.as_str()).unwrap_or("sub-task");
                                            format!("Spawned agent: {}", desc)
                                        }
                                        "AskUserQuestion" => {
                                            let question = block.pointer("/input/question")
                                                .and_then(|q| q.as_str())
                                                .unwrap_or("needs input");
                                            return (Some(("question".into(), format!("❓ {}", question))), None);
                                        }
                                        _ => format!("Using: {}", tool),
                                    };
                                    let kind = if tool == "Agent" { "agent_spawn" } else { "tool_use" };
                                    return (Some((kind.into(), detail)), None);
                                }
                            }
                            // Return text content as partial result (backup if no "result" event).
                            if !text_parts.is_empty() {
                                return (None, Some(text_parts.join("\n")));
                            }
                        }
                    }
                    (None, None)
                }
                "result" => {
                    let text = json.get("result").and_then(|r| r.as_str()).unwrap_or("");
                    // Check for permission denials — append to result so user knows.
                    let denials = json.get("permission_denials")
                        .and_then(|d| d.as_array())
                        .map(|arr| arr.iter()
                            .filter_map(|v| v.as_str())
                            .collect::<Vec<_>>()
                            .join(", "))
                        .filter(|s| !s.is_empty());
                    let result_text = if let Some(denied) = denials {
                        format!("{}\n\n⚠️ Permission denied: {}", text, denied)
                    } else {
                        text.to_string()
                    };
                    (None, Some(result_text))
                }
                _ => (None, None)
            }
        }
    }
}

/// Check if an error response indicates we should try a different backend.
fn is_retriable_error(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("rate limit")
        || lower.contains("quota")
        || lower.contains("insufficient")
        || lower.contains("billing")
        || lower.contains("credits")
        || lower.contains("exceeded")
        || lower.contains("overloaded")
        || lower.contains("capacity")
        || lower.contains("timeout")
        || lower.contains("api error")
}

/// Run the AI worker loop.
///
/// Each incoming request is spawned as a concurrent task. Multiple
/// requests can be in flight simultaneously.
pub async fn ai_worker_loop(
    mut rx: mpsc::UnboundedReceiver<CliRequest>,
    response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
    config: CliWorkerConfig,
    stats: StatsMap,
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
    tasks: TaskMap,
    follow_ups: FollowUpQueue,
    request_tx: mpsc::UnboundedSender<CliRequest>,
    event_tx: Option<tokio::sync::broadcast::Sender<crate::registry::WsEvent>>,
    bot_name: String,
) {
    // Per-source chat ID mapping for cross-channel forwarding.
    // Maps source ("telegram", "slack") → last known chat_id from that source.
    let mut source_chat_ids: HashMap<String, i64> = HashMap::new();

    // Ensure memory directory exists.
    if let Err(e) = std::fs::create_dir_all(&config.memory_dir) {
        log::warn!("Could not create memory dir {:?}: {}", config.memory_dir, e);
    }

    // Knowledge graph — cached in memory, reloads when file changes on disk.
    let knowledge_file = config.memory_dir.join("knowledge.json");
    let knowledge_cache = crate::knowledge::CachedGraph::new(&knowledge_file);

    // Episodic memory file.
    let episodes_file = config.memory_dir.join("episodes.json");

    // Periodic archiving of low-confidence edges (every 100 requests).
    let mut request_count: u32 = 0;

    while let Some(req) = rx.recv().await {
        // Remember chat IDs per source for cross-channel forwarding.
        if req.chat_id != 0 && req.source != "web" {
            source_chat_ids.insert(req.source.clone(), req.chat_id);
        }
        let user_memory_file = config.memory_dir.join(format!("{}.md", req.chat_id));

        // Create user memory file if it doesn't exist.
        if !user_memory_file.exists() {
            let header = format!("# Memory — user {}\n\n", req.chat_id);
            let _ = std::fs::write(&user_memory_file, header);
        }

        // Periodic maintenance.
        request_count += 1;
        if request_count % 50 == 0 {
            // Consolidate: merge duplicate nodes, parallel edges, collapse chains.
            knowledge_cache.consolidate();
        }
        if request_count % 100 == 0 {
            // Archive low-confidence edges to separate file.
            knowledge_cache.archive_stale();
        }

        // Build the command for the selected backend.
        // Knowledge graph + episodes + user memory pre-loaded into the prompt.
        let kg_rendered = knowledge_cache.render_for_prompt(&req.message, 4096);

        // Load relevant episodes.
        let episodes_mem = crate::knowledge::EpisodicMemory::load(&episodes_file);
        let relevant_episodes = episodes_mem.search(&req.message, 5);
        let episodes_rendered = episodes_mem.render(&relevant_episodes, 2048);

        let system_prompt = build_system_prompt(
            config.system_prompt.as_deref(),
            &knowledge_file,
            &kg_rendered,
            &episodes_rendered,
            &user_memory_file,
            &config.working_dir,
            &config.repos_dir,
        );
        let backends = config.backends.clone();
        let message_for_backends = req.message.clone();
        let system_prompt_for_backends = system_prompt.clone();
        let working_dir_for_backends = config.working_dir.clone();
        let skip_perms = config.skip_permissions;
        let continue_session = !req.new_session;

        let working_dir = config.working_dir.clone();
        let input_len = req.message.len() as u64;
        let chat_id = req.chat_id;
        let task_id = req.task_id;
        let req_source = req.source.clone();
        let sync = config.sync_channels;
        let tg_chat = source_chat_ids.get("telegram").copied().unwrap_or(0);
        let rq = Arc::clone(&response_queue);
        let st = Arc::clone(&stats);
        let tm = Arc::clone(&tasks);
        let ttx = telegram_tx.clone();
        let etx = event_tx.clone();
        let bname = bot_name.clone();
        let rq_tx = request_tx.clone();
        let timeout_secs = if config.worker_timeout_secs > 0 { config.worker_timeout_secs } else { 600 };

        // Broadcast user message (for history and cross-device sync).
        if let Some(ref tx) = etx {
            let _ = tx.send(crate::registry::WsEvent::UserMessage {
                bot: bname.clone(),
                chat_id,
                text: req.message.clone(),
                source: req.source.clone(),
            });
        }

        // Forward user message to Telegram if from another channel and sync is enabled.
        if config.sync_channels && req.source != "telegram" && tg_chat != 0 {
            let label = match req.source.as_str() {
                "web" => "🌐 web",
                "slack" => "💬 slack",
                _ => &req.source,
            };
            let _ = ttx.send((tg_chat, format!("[{}] {}", label, req.message)));
        }

        // Broadcast task started event.
        let preview = if req.message.len() > 50 {
            format!("{}...", &req.message[..47])
        } else {
            req.message.clone()
        };
        if let Some(ref tx) = etx {
            let _ = tx.send(crate::registry::WsEvent::TaskStarted {
                bot: bname.clone(),
                task_id,
                preview: preview.clone(),
            });
        }

        // Shared progress tracking — written by the spawned task, read by /status.
        let live_progress: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let live_backend: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let task_live_progress = Arc::clone(&live_progress);
        let task_live_backend = Arc::clone(&live_backend);
        let follow_ups_clone = Arc::clone(&follow_ups);

        // Spawn the task concurrently.
        let handle = tokio::spawn(async move {
            // Send typing indicator every 4 seconds.
            let typing_tx = ttx.clone();
            let typing_handle = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                    if typing_tx.send((chat_id, String::new())).is_err() {
                        break;
                    }
                }
            });

            // Try each backend in order — fallback on failure.
            let backend_list = if backends.is_empty() {
                vec!["claude".to_string()]
            } else {
                backends.clone()
            };
            let mut response_text = String::new();
            let mut _used_backend = String::new();

            for (idx, backend) in backend_list.iter().enumerate() {
                // Track which backend is active.
                if let Ok(mut b) = task_live_backend.lock() { *b = backend.clone(); }
                let (cmd_name, args) = build_backend_command(
                    backend,
                    &message_for_backends,
                    &system_prompt_for_backends,
                    &working_dir_for_backends,
                    skip_perms,
                    continue_session,
                );

                let cmd_name_clone = cmd_name.clone();
                let mut cmd = tokio::process::Command::new(&cmd_name);
                cmd.args(&args);
                cmd.current_dir(&working_dir);
                cmd.stdin(std::process::Stdio::null());  // Never wait for input.
                cmd.stdout(std::process::Stdio::piped());
                cmd.stderr(std::process::Stdio::piped());
                cmd.kill_on_drop(true);
                // Create new process group so we can kill the entire tree on cancel.
                #[cfg(unix)]
                cmd.process_group(0);

                let result = match cmd.spawn() {
                    Ok(mut child) => {
                        let stdout = child.stdout.take();
                        let stderr = child.stderr.take();

                        // Track last activity for stall detection.
                        let last_activity = Arc::new(Mutex::new(Instant::now()));
                        let activity_for_reader = Arc::clone(&last_activity);

                        // Read stdout (stream-json progress + results).
                        let stdout_handle = {
                            let bname = bname.clone();
                            let etx = etx.clone();
                            let progress = Arc::clone(&task_live_progress);
                            let ttx_for_reader = ttx.clone();
                            let tg_chat_for_reader = tg_chat;
                            tokio::spawn(async move {
                                let mut lines_result = String::new();
                                if let Some(stdout) = stdout {
                                    use tokio::io::{AsyncBufReadExt, BufReader};
                                    let mut reader = BufReader::new(stdout).lines();

                                    while let Ok(Some(line)) = reader.next_line().await {
                                        // Update activity timestamp.
                                        if let Ok(mut t) = activity_for_reader.lock() { *t = Instant::now(); }

                                        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                                            let (prog, result) = parse_backend_line(&cmd_name_clone, &json);

                                            if let Some((kind, detail)) = prog {
                                                if let Ok(mut p) = progress.lock() {
                                                    *p = Some(detail.clone());
                                                }
                                                // Forward questions to Telegram/Slack so chat users see them.
                                                if kind == "question" && tg_chat_for_reader != 0 {
                                                    let _ = ttx_for_reader.send((tg_chat_for_reader,
                                                        format!("[Task #{}] {}\n\nReply with /followup <answer>", task_id, detail)));
                                                }
                                                if let Some(ref tx) = etx {
                                                    let _ = tx.send(crate::registry::WsEvent::TaskProgress {
                                                        bot: bname.clone(),
                                                        task_id,
                                                        kind,
                                                        detail,
                                                    });
                                                }
                                            }

                                            if let Some(text) = result {
                                                if !text.is_empty() {
                                                    lines_result = text;
                                                }
                                            }
                                        }
                                    }
                                }
                                lines_result
                            })
                        };

                        // Read stderr (capture error output to prevent pipe deadlock).
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

                        // Stall watchdog — warns if no activity for 2 minutes,
                        // kills after the configured timeout.
                        let child_id = child.id();
                        let watchdog_activity = Arc::clone(&last_activity);
                        let watchdog_etx = etx.clone();
                        let watchdog_bname = bname.clone();
                        let watchdog_handle = tokio::spawn(async move {
                            let stall_warn_secs = 120u64;
                            let mut warned = false;
                            loop {
                                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                                let idle_secs = watchdog_activity.lock()
                                    .map(|t| t.elapsed().as_secs())
                                    .unwrap_or(0);

                                // Stall warning at 2 minutes of no output.
                                if !warned && idle_secs > stall_warn_secs {
                                    warned = true;
                                    if let Some(ref tx) = watchdog_etx {
                                        let _ = tx.send(crate::registry::WsEvent::TaskProgress {
                                            bot: watchdog_bname.clone(),
                                            task_id,
                                            kind: "warning".into(),
                                            detail: format!("No output for {}s — worker may be stalled", idle_secs),
                                        });
                                    }
                                }

                                // Reset warning if activity resumes.
                                if warned && idle_secs < stall_warn_secs {
                                    warned = false;
                                }

                                // Hard timeout — kill the process group.
                                if idle_secs > timeout_secs {
                                    log::warn!("[{}] Task #{} timed out ({}s idle) — killing",
                                        watchdog_bname, task_id, idle_secs);
                                    #[cfg(unix)]
                                    if let Some(pid) = child_id {
                                        unsafe { libc::killpg(pid as i32, libc::SIGKILL); }
                                    }
                                    return true; // timed out
                                }
                            }
                        });

                        // Wait for stdout reader, stderr reader, and child exit.
                        let (stdout_result, stderr_result, status) = tokio::join!(
                            stdout_handle,
                            stderr_handle,
                            child.wait()
                        );
                        watchdog_handle.abort();

                        let result_text = stdout_result.unwrap_or_default();
                        let stderr_text = stderr_result.unwrap_or_default();
                        let success = status.as_ref().map(|s| s.success()).unwrap_or(false);

                        if !result_text.is_empty() && success {
                            Ok(result_text)
                        } else if is_retriable_error(&result_text) || is_retriable_error(&stderr_text) || !success {
                            let detail = if !result_text.is_empty() {
                                result_text
                            } else if !stderr_text.is_empty() {
                                stderr_text
                            } else {
                                let code = status.as_ref().map(|s| format!("exit {}", s)).unwrap_or_else(|e| format!("{}", e));
                                format!("process {}", code)
                            };
                            Err(format!("{} failed: {}", backend, detail))
                        } else if result_text.is_empty() {
                            let hint = if !stderr_text.is_empty() {
                                format!(": {}", stderr_text.lines().next().unwrap_or(""))
                            } else {
                                String::new()
                            };
                            Err(format!("{}: no output{}", backend, hint))
                        } else {
                            Ok(result_text)
                        }
                    }
                    Err(e) => Err(format!("Failed to run {}: {}", backend, e)),
                };

                match result {
                    Ok(text) => {
                        response_text = text;
                        _used_backend = backend.clone();
                        break;
                    }
                    Err(err) => {
                        log::warn!("[{}] Backend '{}' failed: {}", bname, backend, err);
                        if idx + 1 < backend_list.len() {
                            log::info!("[{}] Falling back to '{}'", bname, backend_list[idx + 1]);
                            if let Some(ref tx) = etx {
                                let _ = tx.send(crate::registry::WsEvent::TaskProgress {
                                    bot: bname.clone(),
                                    task_id,
                                    kind: "fallback".into(),
                                    detail: format!("{} failed, trying {}...", backend, backend_list[idx + 1]),
                                });
                            }
                        } else {
                            response_text = format!("⚠️ All backends failed. Last error: {}", err);
                            _used_backend = backend.clone();
                            // Broadcast error event.
                            if let Some(ref tx) = etx {
                                let _ = tx.send(crate::registry::WsEvent::TaskError {
                                    bot: bname.clone(),
                                    task_id,
                                    error: err.clone(),
                                });
                            }
                        }
                    }
                }
            }

            typing_handle.abort();

            // Update stats.
            if let Ok(mut map) = st.lock() {
                let s = map.entry(chat_id).or_default();
                if s.started.is_none() {
                    s.started = Some(Instant::now());
                }
                s.messages += 1;
                s.input_chars += input_len;
                s.output_chars += response_text.len() as u64;
            }

            // Broadcast to WebSocket clients.
            if let Some(ref tx) = etx {
                let _ = tx.send(crate::registry::WsEvent::Message {
                    bot: bname.clone(),
                    chat_id,
                    text: response_text.clone(),
                    task_id,
                });
            }

            // Forward response to Telegram if from another channel and sync is enabled.
            if sync && req_source != "telegram" && tg_chat != 0 {
                let _ = ttx.send((tg_chat, response_text.clone()));
            }

            // Push response to R2 event bus (for Telegram plugin).
            if let Ok(mut q) = rq.lock() {
                q.push_back(CliResponse {
                    chat_id,
                    text: response_text,
                    task_id,
                });
            }

            // Remove from running tasks and broadcast completion.
            let duration_secs = {
                let mut dur = 0u64;
                if let Ok(mut map) = tm.lock() {
                    if let Some(task) = map.remove(&task_id) {
                        dur = task.started.elapsed().as_secs();
                    }
                }
                dur
            };
            if let Some(ref tx) = etx {
                let _ = tx.send(crate::registry::WsEvent::TaskCompleted {
                    bot: bname.clone(),
                    task_id,
                    duration_secs,
                });
            }

            // Process follow-up queue — dispatch queued messages with session continuity.
            if let Ok(mut fq) = follow_ups_clone.lock() {
                if let Some(follow_ups) = fq.remove(&task_id) {
                    for fu in follow_ups {
                        log::info!("[{}] Dispatching follow-up for task #{}: {}",
                            bname, task_id,
                            if fu.message.len() > 50 { &fu.message[..47] } else { &fu.message });
                        // Re-queue as a new request (with session continuity via -c).
                        let _ = rq_tx.send(CliRequest {
                            chat_id: fu.chat_id,
                            message: fu.message,
                            new_session: false, // continue session
                            task_id: 0, // will be assigned by the worker loop
                            source: fu.source,
                        });
                    }
                }
            }
        });

        // Track the running task.
        if let Ok(mut map) = tasks.lock() {
            map.insert(
                task_id,
                RunningTask {
                    task_id,
                    chat_id,
                    message_preview: preview,
                    started: Instant::now(),
                    handle,
                    last_progress: live_progress,
                    backend: live_backend,
                },
            );
        }
    }
}

fn build_system_prompt(
    custom: Option<&str>,
    knowledge_file: &Path,
    kg_rendered: &str,
    episodes_rendered: &str,
    user_memory_file: &Path,
    working_dir: &str,
    repos_dir: &Path,
) -> String {
    let mut prompt = String::new();

    if let Some(custom) = custom {
        prompt.push_str(custom);
        prompt.push_str("\n\n");
    }

    prompt.push_str(WORKSPACE_PREAMBLE);
    prompt.push_str(&format!(
        "\n\nWorking directory: {}\nRepos directory: {}\n\n",
        working_dir,
        repos_dir.display()
    ));

    prompt.push_str(MEMORY_PREAMBLE);
    prompt.push_str(&format!(
        "\nKnowledge graph: {}\nUser memory file: {}\n",
        knowledge_file.display(),
        user_memory_file.display()
    ));

    // Include pre-rendered knowledge graph context.
    if !kg_rendered.is_empty() {
        prompt.push_str("\n[KNOWLEDGE GRAPH]\n");
        prompt.push_str(kg_rendered);
        prompt.push_str("[/KNOWLEDGE GRAPH]\n");
    }

    // Include relevant episodes.
    if !episodes_rendered.is_empty() {
        prompt.push_str("\n[EPISODES]\n");
        prompt.push_str(episodes_rendered);
        prompt.push_str("[/EPISODES]\n");
    }

    // Pre-load user memory.
    let user_memory = std::fs::read_to_string(user_memory_file).unwrap_or_default();
    if !user_memory.trim().is_empty() {
        prompt.push_str("\n[USER MEMORY]\n");
        if user_memory.len() > 4096 {
            prompt.push_str(&user_memory[..4096]);
            prompt.push_str("\n... (truncated — read the full file for more)\n");
        } else {
            prompt.push_str(&user_memory);
        }
        prompt.push_str("\n[/USER MEMORY]\n");
    }

    prompt
}
