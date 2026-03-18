//! Bot registry — shared state between bot tasks and the web server.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::claude_cli::{CliRequest, StatsMap, TaskMap};

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
    /// User sent a message (for history and cross-device sync).
    #[serde(rename = "user_message")]
    UserMessage {
        bot: String,
        chat_id: i64,
        text: String,
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
}

impl BotRegistry {
    pub fn new() -> Self {
        let (global_tx, _) = broadcast::channel(256);
        Self {
            bots: RwLock::new(HashMap::new()),
            global_tx,
        }
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
