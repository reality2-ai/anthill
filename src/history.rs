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
    pub role: String, // "user" or "bot"
    pub text: String,
    #[serde(default)]
    pub task_id: u32,
    #[serde(default)]
    pub timestamp: u64,
}

/// Per-bot chat history, backed by a file.
pub struct BotHistory {
    messages: Vec<ChatMessage>,
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
