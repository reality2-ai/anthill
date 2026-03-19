//! Bot registry — shared state between bot tasks and the web server.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::ai_worker::{CliRequest, StatsMap, TaskMap};

/// Event broadcast to WebSocket clients.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum WsEvent {
    /// Bot sent a chat response.
    #[serde(rename = "message")]
    Message {
        bot: String,
        chat_id: i64,
        text: String,
        task_id: u32,
    },
    /// A new task started.
    #[serde(rename = "task_started")]
    TaskStarted {
        bot: String,
        task_id: u32,
        preview: String,
    },
    /// A task completed.
    #[serde(rename = "task_completed")]
    TaskCompleted {
        bot: String,
        task_id: u32,
        duration_secs: u64,
    },
    /// Real-time progress from a running task.
    #[serde(rename = "task_progress")]
    TaskProgress {
        bot: String,
        task_id: u32,
        /// What's happening: "tool_use", "agent_spawn", "text"
        kind: String,
        /// Human-readable description: "Running: ls -la", "Reading: src/main.rs"
        detail: String,
    },
    /// User sent a message (for history and cross-channel sync).
    #[serde(rename = "user_message")]
    UserMessage {
        bot: String,
        chat_id: i64,
        text: String,
        /// Where the message came from: "telegram", "slack", "web"
        source: String,
    },
    /// Typing indicator.
    #[serde(rename = "typing")]
    Typing { bot: String },
    /// Bot status changed.
    #[serde(rename = "bot_status")]
    BotStatus { bot: String, status: String },
}

/// Status of a bot.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub enum BotStatusKind {
    Running,
    Stopped,
    Error(String),
}

/// Handle to a running bot — used by the web server to interact with it.
#[allow(dead_code)]
pub struct BotHandle {
    pub name: String,
    pub display_name: String,
    pub working_dir: std::path::PathBuf,
    pub request_tx: mpsc::UnboundedSender<CliRequest>,
    pub stats: StatsMap,
    pub tasks: TaskMap,
    pub event_tx: broadcast::Sender<WsEvent>,
    pub status: Arc<RwLock<BotStatusKind>>,
}

/// Registry of all bots, shared between supervisor and web server.
pub struct BotRegistry {
    pub bots: RwLock<HashMap<String, BotHandle>>,
    /// Global event broadcast — web server subscribes here.
    pub global_tx: broadcast::Sender<WsEvent>,
    /// Path to the ants config directory (for creating/editing configs).
    pub ants_dir: std::path::PathBuf,
}

impl BotRegistry {
    pub fn new(ants_dir: std::path::PathBuf) -> Self {
        let (global_tx, _) = broadcast::channel(256);
        Self {
            bots: RwLock::new(HashMap::new()),
            global_tx,
            ants_dir,
        }
    }

    /// Read an ANT's config file.
    pub fn read_config(&self, ant_id: &str) -> Option<String> {
        let path = self.ants_dir.join(ant_id).join("ant.toml");
        std::fs::read_to_string(path).ok()
    }

    /// Write an ANT's config file.
    pub fn write_config(&self, ant_id: &str, content: &str) -> Result<(), String> {
        let dir = self.ants_dir.join(ant_id);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join("ant.toml");
        std::fs::write(path, content).map_err(|e| e.to_string())
    }

    /// Delete an ANT's config directory.
    pub fn delete_config(&self, ant_id: &str) -> Result<(), String> {
        let dir = self.ants_dir.join(ant_id);
        if dir.exists() {
            std::fs::remove_dir_all(dir).map_err(|e| e.to_string())
        } else {
            Ok(())
        }
    }

    /// List ANT directory names on disk (may include ones not running).
    #[allow(dead_code)]
    pub fn list_config_dirs(&self) -> Vec<String> {
        let mut dirs = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.ants_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() && path.join("ant.toml").exists() {
                    if let Some(name) = path.file_name() {
                        dirs.push(name.to_string_lossy().to_string());
                    }
                }
            }
        }
        dirs.sort();
        dirs
    }

    /// List bot names and their status.
    pub async fn list_bots(&self) -> Vec<BotInfo> {
        let bots = self.bots.read().await;
        let mut list = Vec::new();
        for (name, handle) in bots.iter() {
            let status = handle.status.read().await.clone();
            let task_count = handle.tasks.lock().map(|m| m.len()).unwrap_or(0);
            let message_count = handle.stats.lock().map(|m| {
                m.values().map(|s| s.messages).sum::<u32>()
            }).unwrap_or(0);
            list.push(BotInfo {
                id: name.clone(),
                name: handle.display_name.clone(),
                status,
                running_tasks: task_count,
                total_messages: message_count,
            });
        }
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    /// Send a message to a specific bot.
    pub async fn send_message(&self, bot_name: &str, chat_id: i64, message: String) -> bool {
        let bots = self.bots.read().await;
        if let Some(handle) = bots.get(bot_name) {
            let task_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();
            handle.request_tx.send(CliRequest {
                chat_id,
                message,
                new_session: false,
                task_id,
                source: "web".into(),
            }).is_ok()
        } else {
            false
        }
    }
}

/// Summary info for a bot (serializable for API).
#[derive(Debug, Serialize)]
pub struct BotInfo {
    /// Stable identifier (directory name). Used for API calls and history.
    pub id: String,
    /// Display name from ant.toml (shown in UI).
    pub name: String,
    pub status: BotStatusKind,
    pub running_tasks: usize,
    pub total_messages: u32,
}
