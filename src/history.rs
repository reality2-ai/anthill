//! Chat history persistence — stores messages to disk per bot.
//!
//! Messages are appended to a JSON lines file per bot in the working
//! directory. Loaded on startup and sent to new WebSocket clients.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// A single chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "user", "bot", or "system"
    pub text: String,
    #[serde(default)]
    pub task_id: u32,
    #[serde(default)]
    pub timestamp: u64,
    /// If set, this message represents compacted history with an inline graph.
    /// Value is the graph name, e.g. "conversation-saas_vs_ai".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_ref: Option<String>,
}

/// Per-bot chat history, backed by a file.
pub struct BotHistory {
    pub(crate) messages: Vec<ChatMessage>,
    file_path: PathBuf,
}

impl BotHistory {
    fn load(path: &Path) -> Self {
        let messages = if path.exists() {
            let contents = std::fs::read_to_string(path).unwrap_or_default();
            contents
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect()
        } else {
            Vec::new()
        };
        Self {
            messages,
            file_path: path.to_path_buf(),
        }
    }

    fn append(&mut self, msg: &ChatMessage) {
        self.messages.push(msg.clone());
        // Append to file (one JSON line per message).
        if let Ok(json) = serde_json::to_string(msg) {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.file_path)
            {
                let _ = writeln!(f, "{}", json);
            }
        }
        // Cap at 500 messages (keep last 500).
        if self.messages.len() > 500 {
            self.messages = self.messages.split_off(self.messages.len() - 500);
            // Rewrite file.
            self.rewrite();
        }
    }

    fn rewrite(&self) {
        // Atomic rewrite: write to temp file, then rename to avoid corruption
        // if the process is interrupted mid-write.
        let tmp = self.file_path.with_extension("jsonl.tmp");
        if let Ok(f) = std::fs::File::create(&tmp) {
            use std::io::Write;
            let mut w = std::io::BufWriter::new(f);
            for msg in &self.messages {
                if let Ok(json) = serde_json::to_string(msg) {
                    let _ = writeln!(w, "{}", json);
                }
            }
            let _ = w.flush();
        }
        let _ = std::fs::rename(&tmp, &self.file_path);
    }

    fn get_messages(&self) -> &[ChatMessage] {
        &self.messages
    }
}

/// All bot histories, keyed by bot name.
pub struct HistoryStore {
    histories: HashMap<String, BotHistory>,
    base_dir: PathBuf,
}

impl HistoryStore {
    pub fn new(base_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&base_dir).ok();
        Self {
            histories: HashMap::new(),
            base_dir,
        }
    }

    pub fn get_or_load(&mut self, bot_name: &str) -> &[ChatMessage] {
        if !self.histories.contains_key(bot_name) {
            let path = self.base_dir.join(format!("{}.jsonl", bot_name));
            self.histories
                .insert(bot_name.to_string(), BotHistory::load(&path));
        }
        self.histories[bot_name].get_messages()
    }

    pub fn append(&mut self, bot_name: &str, msg: ChatMessage) {
        if !self.histories.contains_key(bot_name) {
            let path = self.base_dir.join(format!("{}.jsonl", bot_name));
            self.histories
                .insert(bot_name.to_string(), BotHistory::load(&path));
        }
        if let Some(h) = self.histories.get_mut(bot_name) {
            h.append(&msg);
        }
    }

    pub fn get_history(&mut self, bot_name: &str) -> Vec<ChatMessage> {
        self.get_or_load(bot_name).to_vec()
    }

    pub fn replace_history(&mut self, bot_name: &str, msgs: Vec<ChatMessage>) {
        let path = self.base_dir.join(format!("{}.jsonl", bot_name));
        // Rewrite the JSONL file.
        let content: String = msgs.iter()
            .filter_map(|m| serde_json::to_string(m).ok())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(&path, content);
        // Update in-memory.
        if let Some(h) = self.histories.get_mut(bot_name) {
            h.messages = msgs;
        }
    }

    /// Compact oldest messages, keeping the most recent `keep_recent`.
    /// Returns the removed messages (for entity extraction).
    /// Inserts a system message with `graph_ref` pointing to the conversation graph.
    pub fn compact_oldest(
        &mut self,
        bot_name: &str,
        keep_recent: usize,
        graph_name: &str,
    ) -> Vec<ChatMessage> {
        // Ensure loaded.
        if !self.histories.contains_key(bot_name) {
            let path = self.base_dir.join(format!("{}.jsonl", bot_name));
            self.histories
                .insert(bot_name.to_string(), BotHistory::load(&path));
        }
        let msgs = match self.histories.get(bot_name) {
            Some(h) => h.messages.clone(),
            None => return Vec::new(),
        };
        if msgs.len() <= keep_recent {
            return Vec::new();
        }
        let split_at = msgs.len() - keep_recent;
        let evicted: Vec<ChatMessage> = msgs[..split_at].to_vec();
        let kept: Vec<ChatMessage> = msgs[split_at..].to_vec();

        let mut new_msgs = vec![ChatMessage {
            role: "system".into(),
            text: format!(
                "{} earlier messages compacted to conversation graph.",
                evicted.len()
            ),
            task_id: 0,
            timestamp: crate::web::now_secs(),
            graph_ref: Some(graph_name.to_string()),
        }];
        new_msgs.extend(kept);
        self.replace_history(bot_name, new_msgs);
        evicted
    }

    /// Get all history for all bots (for snapshot).
    pub fn all_history(&mut self, bot_names: &[String]) -> HashMap<String, Vec<ChatMessage>> {
        let mut result = HashMap::new();
        for name in bot_names {
            let msgs = self.get_or_load(name).to_vec();
            result.insert(name.clone(), msgs);
        }
        result
    }
}

/// Thread-safe wrapper.
pub type SharedHistory = Arc<Mutex<HistoryStore>>;

pub fn create_history_store(base_dir: PathBuf) -> SharedHistory {
    Arc::new(Mutex::new(HistoryStore::new(base_dir)))
}
