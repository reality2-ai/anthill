//! ClaudeCliPlugin — manages Claude Code invocations and responses.
//!
//! This plugin absorbs all I/O that was previously in the sentant:
//! - Stores incoming messages (from Telegram plugin via data plane)
//! - Spawns concurrent claude -p tasks
//! - Tracks running tasks and stats
//! - Sends responses to Telegram (plugin-to-plugin data plane)
//! - Handles /help, /ants, /usage, /cancel formatting
//!
//! The sentant only makes decisions (which command to execute).
//! The plugin does all the work.

use r2_engine::plugin::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::claude_cli::{CliRequest, CliResponse, StatsMap, TaskMap};
use crate::events::RELAY_AI_READY;

/// Plugin commands from the sentant.
pub const CMD_DISPATCH: u8 = 0x01;  // Dispatch a message to Claude
pub const CMD_CANCEL: u8 = 0x02;    // Cancel a task by ID
pub const CMD_CANCEL_ALL: u8 = 0x03; // Cancel all tasks
pub const CMD_HELP: u8 = 0x04;      // Send help text
pub const CMD_ANTS: u8 = 0x05;      // Send task list
pub const CMD_USAGE: u8 = 0x06;     // Send usage stats
pub const CMD_REPLY: u8 = 0x07;     // Pop response and send to Telegram
pub const CMD_NEW_SESSION: u8 = 0x08; // Start new session

const HELP_TEXT: &str = "\
**anthill commands:**

/help — show this message
/ants — show running workers and what they're working on
/usage — show session statistics
/cancel — cancel a running task (or /cancel <id>, /cancel all)
/new — start a fresh conversation

**Claude Code commands** (passed through):

/compact — condense conversation context
/cost — show token/cost usage for the session
/model — show or change the AI model
/memory — manage Claude's memory files
/clear — clear conversation history

Everything else is sent to Claude Code as a prompt.
Multiple messages can run concurrently.";

const TELEGRAM_MAX: usize = 4000;

pub struct ClaudeCliPlugin {
    id: PluginId,
    /// Response queue — worker pushes, poll() checks.
    response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
    /// Pre-encoded CBOR for poll().
    poll_buf: Vec<u8>,
    /// Send requests to the background worker.
    request_tx: mpsc::UnboundedSender<CliRequest>,
    /// Direct channel to Telegram (plugin-to-plugin data plane).
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
    /// Shared message queue — data plane from Telegram plugin.
    /// Full message text stored here by Telegram, consumed by dispatch.
    message_queue: crate::plugins::telegram_bot::MessageQueue,
    /// Running tasks.
    tasks: TaskMap,
    /// Usage stats.
    stats: StatsMap,
    /// Next task ID counter.
    next_task_id: u32,
    /// Whether to forward user messages across channels.
    sync_channels: bool,
}

impl ClaudeCliPlugin {
    pub fn new(
        id: PluginId,
        response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
        request_tx: mpsc::UnboundedSender<CliRequest>,
        telegram_tx: mpsc::UnboundedSender<(i64, String)>,
        tasks: TaskMap,
        stats: StatsMap,
        message_queue: crate::plugins::telegram_bot::MessageQueue,
        sync_channels: bool,
    ) -> Self {
        Self {
            id,
            response_queue,
            poll_buf: Vec::new(),
            request_tx,
            telegram_tx,
            message_queue,
            tasks,
            stats,
            next_task_id: 1,
            sync_channels,
        }
    }

    fn send_telegram(&self, chat_id: i64, text: &str) {
        if text.len() <= TELEGRAM_MAX {
            let _ = self.telegram_tx.send((chat_id, text.to_string()));
        } else {
            for chunk in text.as_bytes().chunks(TELEGRAM_MAX) {
                let chunk_str = String::from_utf8_lossy(chunk);
                let _ = self.telegram_tx.send((chat_id, chunk_str.to_string()));
            }
        }
    }

    fn send_typing(&self, chat_id: i64) {
        let _ = self.telegram_tx.send((chat_id, String::new()));
    }

    fn decode_chat_id(data: &[u8]) -> i64 {
        decode_uint_key(data, 1).unwrap_or(0) as i64
    }

    fn decode_task_id(data: &[u8]) -> u32 {
        decode_uint_key(data, 0).unwrap_or(0) as u32
    }

    fn handle_dispatch(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        // Pop the stored message from the shared data plane queue.
        let (_, text, source) = match self.message_queue.lock().ok().and_then(|mut q| q.pop_front()) {
            Some(msg) => msg,
            None => return,
        };

        // Forward user message to other channels if sync is enabled.
        if self.sync_channels && source != "telegram" {
            let label = match source.as_str() {
                "web" => "🌐 web",
                "slack" => "💬 slack",
                _ => &source,
            };
            self.send_telegram(chat_id, &format!("[{}] {}", label, text));
        }

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        // Send typing + thinking.
        self.send_typing(chat_id);
        self.send_telegram(chat_id, "Thinking...");

        // Dispatch to worker.
        let _ = self.request_tx.send(CliRequest {
            chat_id,
            message: text,
            new_session: false,
            task_id,
            source,
        });
    }

    fn handle_new_session(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        self.send_telegram(chat_id, "Starting fresh conversation...");

        let _ = self.request_tx.send(CliRequest {
            chat_id,
            message: "Summarise our conversation so far in a few bullet points, then say 'Ready for a new conversation.'".into(),
            new_session: true,
            task_id,
            source: "system".into(),
        });
    }

    fn handle_cancel(&mut self, data: &[u8]) {
        let task_id = Self::decode_task_id(data);
        let chat_id = Self::decode_chat_id(data);

        if let Ok(mut map) = self.tasks.lock() {
            if task_id == 0 {
                // Cancel most recent for this chat.
                let latest = map
                    .values()
                    .filter(|t| t.chat_id == chat_id)
                    .max_by_key(|t| t.task_id)
                    .map(|t| t.task_id);
                if let Some(id) = latest {
                    if let Some(task) = map.remove(&id) {
                        task.handle.abort();
                        self.send_telegram(chat_id, &format!("Cancelled task #{}.", id));
                    }
                } else {
                    self.send_telegram(chat_id, "No running tasks.");
                }
            } else if let Some(task) = map.remove(&task_id) {
                task.handle.abort();
                self.send_telegram(chat_id, &format!("Cancelled task #{}.", task_id));
            } else {
                self.send_telegram(chat_id, &format!("No task with ID {}.", task_id));
            }
        }
    }

    fn handle_cancel_all(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        if let Ok(mut map) = self.tasks.lock() {
            let count = map.len();
            for task in map.values() {
                task.handle.abort();
            }
            map.clear();
            self.send_telegram(chat_id, &format!("Cancelled {} task(s).", count));
        }
    }

    fn handle_help(&self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);
        self.send_telegram(chat_id, HELP_TEXT);
    }

    fn handle_ants(&self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        let map = match self.tasks.lock() {
            Ok(m) => m,
            Err(_) => {
                self.send_telegram(chat_id, "Status unavailable.");
                return;
            }
        };

        if map.is_empty() {
            self.send_telegram(chat_id, "All workers idle. Send a message to dispatch one.");
            return;
        }

        let count = map.len();
        let mut out = format!(
            "**{} worker{} active:**\n\n",
            count,
            if count == 1 { "" } else { "s" }
        );
        let mut tasks: Vec<_> = map.values().collect();
        tasks.sort_by_key(|t| t.task_id);

        for task in tasks {
            let elapsed = task.started.elapsed().as_secs();
            let duration = format_duration(elapsed);
            out.push_str(&format!(
                "**#{}** — {}\n  Working for {}\n\n",
                task.task_id, task.message_preview, duration
            ));
        }
        out.push_str("Use /cancel or /cancel <id> to stop a worker.");
        self.send_telegram(chat_id, &out);
    }

    fn handle_usage(&self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        let map = match self.stats.lock() {
            Ok(m) => m,
            Err(_) => {
                self.send_telegram(chat_id, "Stats unavailable.");
                return;
            }
        };

        if map.is_empty() {
            self.send_telegram(chat_id, "No usage yet.");
            return;
        }

        let mut out = String::from("**Session statistics:**\n\n");
        for (&cid, s) in map.iter() {
            let uptime = s
                .started
                .map(|t| format_duration(t.elapsed().as_secs()))
                .unwrap_or_else(|| "—".into());
            if map.len() > 1 {
                out.push_str(&format!("*User {}:*\n", cid));
            }
            out.push_str(&format!("  Messages: {}\n", s.messages));
            out.push_str(&format!("  Input: {} chars\n", s.input_chars));
            out.push_str(&format!("  Output: {} chars\n", s.output_chars));
            out.push_str(&format!("  Session: {}\n", uptime));
            if map.len() > 1 {
                out.push('\n');
            }
        }
        if let Ok(tasks) = self.tasks.lock() {
            if !tasks.is_empty() {
                out.push_str(&format!("\n  Running tasks: {}", tasks.len()));
            }
        }
        self.send_telegram(chat_id, &out);
    }

    fn handle_reply(&mut self, _data: &[u8]) {
        if let Ok(mut q) = self.response_queue.lock() {
            if let Some(resp) = q.pop_front() {
                self.send_telegram(resp.chat_id, &resp.text);
            }
        }
    }

}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{}s", secs)
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

fn decode_uint_key(data: &[u8], target_key: u64) -> Option<u64> {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let r2_cbor::Item::Map(n) = dec.next().ok()? else { return None };
    for _ in 0..n {
        let r2_cbor::Item::UInt(key) = dec.next().ok()? else { return None };
        if key == target_key {
            let r2_cbor::Item::UInt(v) = dec.next().ok()? else { return None };
            return Some(v);
        }
        let _ = dec.next().ok()?;
    }
    None
}

fn encode_uint(buf: &mut Vec<u8>, v: u64) {
    if v <= 23 {
        buf.push(v as u8);
    } else if v <= 0xFF {
        buf.push(0x18);
        buf.push(v as u8);
    } else if v <= 0xFFFF {
        buf.push(0x19);
        buf.extend_from_slice(&(v as u16).to_be_bytes());
    } else if v <= 0xFFFF_FFFF {
        buf.push(0x1A);
        buf.extend_from_slice(&(v as u32).to_be_bytes());
    } else {
        buf.push(0x1B);
        buf.extend_from_slice(&v.to_be_bytes());
    }
}

impl Plugin for ClaudeCliPlugin {
    fn execute(&mut self, command: PluginCommand, data: &[u8]) -> PluginResult {
        match command {
            CMD_DISPATCH => { self.handle_dispatch(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_CANCEL => { self.handle_cancel(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_CANCEL_ALL => { self.handle_cancel_all(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_HELP => { self.handle_help(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_ANTS => { self.handle_ants(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_USAGE => { self.handle_usage(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_REPLY => { self.handle_reply(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_NEW_SESSION => { self.handle_new_session(data); PluginResult::Ok(PluginResponse::empty()) }
            _ => PluginResult::Error(PluginError::new(0xFF, "unknown command")),
        }
    }

    fn name(&self) -> &str {
        "claude-cli"
    }

    fn id(&self) -> PluginId {
        self.id
    }

    fn poll(&mut self) -> Option<(u32, &[u8])> {
        let queue = self.response_queue.lock().ok()?;
        let front = queue.front()?;
        let chat_id = front.chat_id as u64;
        drop(queue);

        self.poll_buf.clear();
        self.poll_buf.push(0xA2); // map(2)
        self.poll_buf.push(0x00); // key 0
        self.poll_buf.push(0x00); // value 0
        self.poll_buf.push(0x01); // key 1
        encode_uint(&mut self.poll_buf, chat_id);

        Some((RELAY_AI_READY, &self.poll_buf))
    }
}
