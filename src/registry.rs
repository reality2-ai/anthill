//! Bot registry — shared state between bot tasks and the web server.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};

use crate::ai_worker::{CliRequest, FollowUpQueue, StatsMap, TaskMap};

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
    /// A task failed or timed out.
    #[serde(rename = "task_error")]
    TaskError {
        bot: String,
        task_id: u32,
        error: String,
    },
    /// Typing indicator.
    #[serde(rename = "typing")]
    Typing { bot: String },
    /// Bot status changed.
    #[serde(rename = "bot_status")]
    BotStatus { bot: String, status: String },
    /// Knowledge graph was updated (triggers live graph refresh in dashboard).
    #[serde(rename = "graph_updated")]
    GraphUpdated {
        bot: String,
        /// Which graph changed: "meta", or a topic name like "anthill".
        graph: String,
        /// What caused the update: "rumination", "consolidation", "user".
        source: String,
    },
}

/// Status of a bot.
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub enum BotStatusKind {
    Running,
    Stopped,
    /// Configured on disk but not yet started.
    Configured,
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
    pub follow_ups: FollowUpQueue,
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

    /// List all ants — merges configured-on-disk with running-in-memory.
    /// Ants that exist on disk but aren't running show as `Configured`.
    pub async fn list_bots(&self) -> Vec<BotInfo> {
        let bots = self.bots.read().await;
        let mut list = Vec::new();
        let mut seen = std::collections::HashSet::new();

        // Running ants (from memory).
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
            seen.insert(name.clone());
        }

        // Configured-on-disk but not running.
        for dir_name in self.list_config_dirs() {
            if !seen.contains(&dir_name) {
                // Try to read the display name from config.
                let display_name = self.read_config(&dir_name)
                    .and_then(|toml_str| toml::from_str::<crate::config::Config>(&toml_str).ok())
                    .and_then(|cfg| cfg.name)
                    .unwrap_or_else(|| dir_name.clone());
                list.push(BotInfo {
                    id: dir_name,
                    name: display_name,
                    status: BotStatusKind::Configured,
                    running_tasks: 0,
                    total_messages: 0,
                });
            }
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

    /// Send a colony query from one ANT to another.
    /// The target ANT's AI will reason about the question using its own expertise.
    /// The response is broadcast on the global channel with colony source metadata
    /// so it can be forwarded back to the requesting ANT's chat.
    ///
    /// Source format: "colony:<from_ant>:<chat_id>" — carries the return address.
    pub async fn ask_ant(
        &self,
        from_ant: &str,
        target_ant: &str,
        chat_id: i64,
        question: String,
    ) -> bool {
        let bots = self.bots.read().await;
        if let Some(handle) = bots.get(target_ant) {
            let task_id = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .subsec_nanos();

            // Prefix the question with context about who is asking and why.
            let colony_message = format!(
                "COLONY QUERY from {} (chat_id {})\n\n\
                 Another ANT in your colony is asking for your expertise:\n\n\
                 {}\n\n\
                 Respond with your knowledge and reasoning. Be specific about your \
                 confidence levels and evidence. Your response will be treated as a \
                 conjecture by the asking ANT — it will evaluate your answer critically.\n\n\
                 IMPORTANT: Complete your response and STOP. Do not ask follow-up questions.",
                from_ant, chat_id, question
            );

            handle.request_tx.send(CliRequest {
                chat_id: -2, // Colony query — distinct from rumination (-1) and users (positive)
                message: colony_message,
                new_session: true,
                task_id,
                source: format!("colony:{}:{}", from_ant, chat_id),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn list_bots_shows_configured_not_running() {
        let dir = std::env::temp_dir().join("anthill-test-registry");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create two ant configs on disk.
        let ant1_dir = dir.join("ant-alpha");
        std::fs::create_dir_all(&ant1_dir).unwrap();
        std::fs::write(
            ant1_dir.join("ant.toml"),
            "name = \"Alpha\"\n",
        ).unwrap();

        let ant2_dir = dir.join("ant-beta");
        std::fs::create_dir_all(&ant2_dir).unwrap();
        std::fs::write(
            ant2_dir.join("ant.toml"),
            "name = \"Beta\"\n",
        ).unwrap();

        let registry = BotRegistry::new(dir.clone());

        // No bots running — both should show as Configured.
        let bots = registry.list_bots().await;
        assert_eq!(bots.len(), 2);
        assert!(bots.iter().all(|b| matches!(b.status, BotStatusKind::Configured)));
        assert!(bots.iter().any(|b| b.id == "ant-alpha" && b.name == "Alpha"));
        assert!(bots.iter().any(|b| b.id == "ant-beta" && b.name == "Beta"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn list_bots_merges_running_and_configured() {
        let dir = std::env::temp_dir().join("anthill-test-registry-merge");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Two on disk.
        for name in &["running-ant", "stopped-ant"] {
            let d = dir.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("ant.toml"), format!("name = \"{}\"\n", name)).unwrap();
        }

        let registry = BotRegistry::new(dir.clone());

        // Register one as running.
        let (tx, _rx) = mpsc::unbounded_channel();
        let (event_tx, _) = broadcast::channel(16);
        registry.bots.write().await.insert("running-ant".into(), BotHandle {
            name: "running-ant".into(),
            display_name: "Running Ant".into(),
            working_dir: dir.join("running-ant"),
            request_tx: tx,
            stats: Arc::new(std::sync::Mutex::new(HashMap::new())),
            tasks: Arc::new(std::sync::Mutex::new(HashMap::new())),
            follow_ups: Arc::new(std::sync::Mutex::new(HashMap::new())),
            event_tx,
            status: Arc::new(RwLock::new(BotStatusKind::Running)),
        });

        let bots = registry.list_bots().await;
        assert_eq!(bots.len(), 2);

        let running = bots.iter().find(|b| b.id == "running-ant").unwrap();
        assert!(matches!(running.status, BotStatusKind::Running));

        let configured = bots.iter().find(|b| b.id == "stopped-ant").unwrap();
        assert!(matches!(configured.status, BotStatusKind::Configured));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn send_message_to_nonexistent_bot_returns_false() {
        let dir = std::env::temp_dir().join("anthill-test-registry-send");
        let registry = BotRegistry::new(dir);
        assert!(!registry.send_message("no-such-bot", 0, "hello".into()).await);
    }

    #[test]
    fn config_dir_operations() {
        let dir = std::env::temp_dir().join("anthill-test-registry-ops");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let registry = BotRegistry::new(dir.clone());

        // Write config.
        registry.write_config("test-ant", "name = \"Test\"\n").unwrap();
        assert!(registry.read_config("test-ant").is_some());
        assert_eq!(registry.list_config_dirs(), vec!["test-ant".to_string()]);

        // Delete config.
        registry.delete_config("test-ant").unwrap();
        assert!(registry.read_config("test-ant").is_none());
        assert!(registry.list_config_dirs().is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }
}
