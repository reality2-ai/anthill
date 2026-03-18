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
}

/// All user stats, keyed by chat_id.
pub type StatsMap = Arc<Mutex<HashMap<i64, UserStats>>>;

/// All running tasks.
pub type TaskMap = Arc<Mutex<HashMap<u32, RunningTask>>>;

const MEMORY_PREAMBLE: &str = "\
You have a persistent memory file for this user at the path shown below. \
Read it at the start of each conversation to recall context. \
When you learn something worth remembering (user preferences, project context, \
key decisions, names, ongoing work), append it to the file. \
Keep entries concise — one line per fact. Remove outdated entries when you notice them.";

const WORKSPACE_PREAMBLE: &str = "\
Your working directory has the following structure:\
\n- memory/ — per-user persistent memory files (auto-backed up)\
\n- repos/ — for cloning git repositories (NOT backed up, repos have their own git history)\
\n\
\nWhen cloning repositories, ALWAYS clone into the repos/ subdirectory.\
\nThe working directory is a git repo that is automatically committed on a schedule. \
The repos/ folder is excluded from these backups via .gitignore since cloned repos \
already have their own version control.";

/// Run the Claude CLI worker loop.
///
/// Each incoming request is spawned as a concurrent task. Multiple
/// requests can be in flight simultaneously.
pub async fn claude_cli_worker(
    mut rx: mpsc::UnboundedReceiver<CliRequest>,
    response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
    config: CliWorkerConfig,
    stats: StatsMap,
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
    tasks: TaskMap,
    event_tx: Option<tokio::sync::broadcast::Sender<crate::registry::WsEvent>>,
    bot_name: String,
) {
    // Ensure memory directory exists.
    if let Err(e) = std::fs::create_dir_all(&config.memory_dir) {
        log::warn!("Could not create memory dir {:?}: {}", config.memory_dir, e);
    }

    while let Some(req) = rx.recv().await {
        let memory_file = config.memory_dir.join(format!("{}.md", req.chat_id));

        // Create memory file if it doesn't exist.
        if !memory_file.exists() {
            let header = format!("# Memory — user {}\n\n", req.chat_id);
            let _ = std::fs::write(&memory_file, header);
        }

        // Build the command.
        let mut args = vec![
            "-p".to_string(),
            "--verbose".to_string(),
            "--output-format".to_string(),
            "stream-json".to_string(),
        ];
        if config.skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }
        if !req.new_session {
            args.push("-c".to_string());
        }

        let system_prompt = build_system_prompt(
            config.system_prompt.as_deref(),
            &memory_file,
            &config.working_dir,
            &config.repos_dir,
        );
        args.push("--append-system-prompt".to_string());
        args.push(system_prompt);
        args.push(req.message.clone());

        let working_dir = config.working_dir.clone();
        let input_len = req.message.len() as u64;
        let chat_id = req.chat_id;
        let task_id = req.task_id;
        let rq = Arc::clone(&response_queue);
        let st = Arc::clone(&stats);
        let tm = Arc::clone(&tasks);
        let ttx = telegram_tx.clone();
        let etx = event_tx.clone();
        let bname = bot_name.clone();

        // Broadcast user message (for history and cross-device sync).
        if let Some(ref tx) = etx {
            let _ = tx.send(crate::registry::WsEvent::UserMessage {
                bot: bname.clone(),
                chat_id,
                text: req.message.clone(),
                source: req.source.clone(),
            });
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

            let mut cmd = tokio::process::Command::new("claude");
            cmd.args(&args);
            cmd.current_dir(&working_dir);
            cmd.stdout(std::process::Stdio::piped());
            cmd.stderr(std::process::Stdio::piped());

            let response_text = match cmd.spawn() {
                Ok(mut child) => {
                    let stdout = child.stdout.take();
                    let mut result_text = String::new();

                    if let Some(stdout) = stdout {
                        use tokio::io::{AsyncBufReadExt, BufReader};
                        let mut reader = BufReader::new(stdout).lines();

                        while let Ok(Some(line)) = reader.next_line().await {
                            // Parse stream-json line.
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&line) {
                                let msg_type = json.get("type").and_then(|t| t.as_str()).unwrap_or("");

                                match msg_type {
                                    "assistant" => {
                                        // Check for tool_use in content.
                                        if let Some(content) = json.pointer("/message/content") {
                                            if let Some(arr) = content.as_array() {
                                                for block in arr {
                                                    let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
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
                                                            _ => format!("Using: {}", tool),
                                                        };

                                                        if let Some(ref tx) = etx {
                                                            let _ = tx.send(crate::registry::WsEvent::TaskProgress {
                                                                bot: bname.clone(),
                                                                task_id,
                                                                kind: if tool == "Agent" { "agent_spawn".into() } else { "tool_use".into() },
                                                                detail,
                                                            });
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    "result" => {
                                        // Final result — extract the text.
                                        if let Some(text) = json.get("result").and_then(|r| r.as_str()) {
                                            result_text = text.to_string();
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    // Wait for process to finish.
                    let status = child.wait().await;

                    if result_text.is_empty() {
                        // Fallback: read stderr.
                        match status {
                            Ok(s) if s.success() => "(no output)".to_string(),
                            Ok(s) => format!("Process exited with code {}", s),
                            Err(e) => format!("Error: {}", e),
                        }
                    } else {
                        result_text
                    }
                }
                Err(e) => format!("Failed to run claude: {}", e),
            };

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
                    bot: bname,
                    task_id,
                    duration_secs,
                });
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
                },
            );
        }
    }
}

fn build_system_prompt(
    custom: Option<&str>,
    memory_file: &Path,
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
        "\nMemory file: {}",
        memory_file.display()
    ));

    prompt
}
