//! Claude CLI sentant — conductor that dispatches work to concurrent Claude tasks.
//!
//! Always accepts new messages. Each message spawns a concurrent Claude Code
//! invocation via the background worker. Responses arrive asynchronously.
//! /status shows running tasks, /cancel aborts a running task.

use r2_engine::action_buf::ActionBuf;
use r2_engine::event::Event;
use r2_engine::sentant::{Sentant, StateId};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::claude_cli::{CliRequest, CliResponse, StatsMap, TaskMap};
use crate::events::*;

const STATE_READY: StateId = 0;

/// Maximum Telegram message size.
const TELEGRAM_MAX: usize = 4000;

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

pub struct ClaudeCliSentant {
    chat_id: i64,
    next_task_id: u32,
    /// Send requests to the background CLI worker.
    request_tx: mpsc::UnboundedSender<CliRequest>,
    /// Shared response queue (popped here, polled by ClaudeCliPlugin).
    response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
    /// Direct channel to Telegram (bypasses engine for large text).
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
    /// Shared usage stats from the worker.
    stats: StatsMap,
    /// Running tasks (shared with worker).
    tasks: TaskMap,
}

impl ClaudeCliSentant {
    pub fn new(
        request_tx: mpsc::UnboundedSender<CliRequest>,
        response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
        telegram_tx: mpsc::UnboundedSender<(i64, String)>,
        stats: StatsMap,
        tasks: TaskMap,
    ) -> Self {
        Self {
            chat_id: 0,
            next_task_id: 1,
            request_tx,
            response_queue,
            telegram_tx,
            stats,
            tasks,
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

    fn pop_response(&self) -> Option<CliResponse> {
        self.response_queue.lock().ok()?.pop_front()
    }

    /// Handle slash commands. Returns true if the message was handled locally.
    fn handle_command(&mut self, text: &str, chat_id: i64) -> bool {
        match text.trim() {
            "/help" | "/start" => {
                self.send_telegram(chat_id, HELP_TEXT);
                true
            }
            "/usage" => {
                self.send_telegram(chat_id, &self.format_usage());
                true
            }
            "/ants" | "/bots" | "/status" => {
                self.send_telegram(chat_id, &self.format_ants());
                true
            }
            s if s == "/cancel" || s.starts_with("/cancel ") => {
                self.handle_cancel(s, chat_id);
                true
            }
            "/new" => {
                // Send a /new request — will skip -c on this invocation.
                let task_id = self.next_task_id;
                self.next_task_id += 1;
                let _ = self.request_tx.send(CliRequest {
                    chat_id,
                    message: "Summarise our conversation so far in a few bullet points, then say 'Ready for a new conversation.'".into(),
                    new_session: true,
                    task_id,
                });
                self.send_telegram(chat_id, "Starting fresh conversation...");
                false // let it go through so we get the summary back
            }
            _ => false,
        }
    }

    fn handle_cancel(&self, text: &str, chat_id: i64) {
        let arg = text.strip_prefix("/cancel").unwrap_or("").trim();

        let map = match self.tasks.lock() {
            Ok(m) => m,
            Err(_) => {
                self.send_telegram(chat_id, "Could not access task list.");
                return;
            }
        };

        if map.is_empty() {
            self.send_telegram(chat_id, "No running tasks.");
            return;
        }

        if arg == "all" {
            let count = map.len();
            for task in map.values() {
                task.handle.abort();
            }
            drop(map);
            if let Ok(mut m) = self.tasks.lock() {
                m.clear();
            }
            self.send_telegram(chat_id, &format!("Cancelled {} task(s).", count));
            return;
        }

        if let Ok(id) = arg.parse::<u32>() {
            if let Some(task) = map.get(&id) {
                task.handle.abort();
                drop(map);
                if let Ok(mut m) = self.tasks.lock() {
                    m.remove(&id);
                }
                self.send_telegram(chat_id, &format!("Cancelled task #{}.", id));
            } else {
                self.send_telegram(chat_id, &format!("No task with ID {}.", id));
            }
            return;
        }

        // No argument — cancel the most recent task for this user.
        let latest = map
            .values()
            .filter(|t| t.chat_id == chat_id)
            .max_by_key(|t| t.task_id);

        if let Some(task) = latest {
            let id = task.task_id;
            task.handle.abort();
            drop(map);
            if let Ok(mut m) = self.tasks.lock() {
                m.remove(&id);
            }
            self.send_telegram(chat_id, &format!("Cancelled task #{}.", id));
        } else {
            self.send_telegram(chat_id, "No running tasks for you.");
        }
    }

    fn format_ants(&self) -> String {
        let map = match self.tasks.lock() {
            Ok(m) => m,
            Err(_) => return "Status unavailable.".into(),
        };

        if map.is_empty() {
            return "All workers idle. Send a message to dispatch one.".into();
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
        out
    }

    fn format_usage(&self) -> String {
        let map = match self.stats.lock() {
            Ok(m) => m,
            Err(_) => return "Stats unavailable.".into(),
        };

        if map.is_empty() {
            return "No usage yet.".into();
        }

        let mut out = String::from("**Session statistics:**\n\n");

        for (&chat_id, s) in map.iter() {
            let uptime = s
                .started
                .map(|t| format_duration(t.elapsed().as_secs()))
                .unwrap_or_else(|| "—".into());

            if map.len() > 1 {
                out.push_str(&format!("*User {}:*\n", chat_id));
            }
            out.push_str(&format!("  Messages: {}\n", s.messages));
            out.push_str(&format!("  Input: {} chars\n", s.input_chars));
            out.push_str(&format!("  Output: {} chars\n", s.output_chars));
            out.push_str(&format!("  Session: {}\n", uptime));
            if map.len() > 1 {
                out.push('\n');
            }
        }

        // Show running task count.
        if let Ok(tasks) = self.tasks.lock() {
            if !tasks.is_empty() {
                out.push_str(&format!("\n  Running tasks: {}", tasks.len()));
            }
        }

        out
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

impl Sentant for ClaudeCliSentant {
    fn handle_event(&mut self, event: &Event, _actions: &mut ActionBuf) {
        match event.hash {
            RELAY_COMMAND => {
                let (text, chat_id) = decode_text_and_chat(event.payload);
                if text.is_empty() {
                    return;
                }
                self.chat_id = chat_id;

                // Handle slash commands locally.
                if text.starts_with('/') && self.handle_command(&text, chat_id) {
                    return;
                }

                // Show typing indicator + "Thinking..." message.
                let _ = self.telegram_tx.send((chat_id, String::new()));
                self.send_telegram(chat_id, "Thinking...");

                let task_id = self.next_task_id;
                self.next_task_id += 1;

                let new_session = text.trim() == "/new";
                let _ = self.request_tx.send(CliRequest {
                    chat_id,
                    message: text,
                    new_session,
                    task_id,
                });
            }

            RELAY_AI_READY => {
                if let Some(resp) = self.pop_response() {
                    self.send_telegram(resp.chat_id, &resp.text);
                }
            }

            _ => {}
        }
    }

    fn state(&self) -> StateId {
        STATE_READY
    }

    fn class_hash(&self) -> u32 {
        r2_fnv::fnv1a_32(b"ai.reality2.relay.claude_cli")
    }

    fn name(&self) -> &str {
        "claude-cli"
    }

    fn subscriptions(&self) -> &[u32] {
        &[RELAY_COMMAND, RELAY_AI_READY]
    }
}

/// Decode text (key 0) and chat_id (key 1) from CBOR map.
fn decode_text_and_chat(data: &[u8]) -> (String, i64) {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let mut text = String::new();
    let mut chat_id: i64 = 0;

    let Ok(r2_cbor::Item::Map(n)) = dec.next() else {
        return (text, chat_id);
    };
    for _ in 0..n {
        let Ok(r2_cbor::Item::UInt(key)) = dec.next() else {
            break;
        };
        match key {
            0 => {
                if let Ok(r2_cbor::Item::Text(t)) = dec.next() {
                    text = std::str::from_utf8(t).unwrap_or("").to_string();
                }
            }
            1 => {
                if let Ok(r2_cbor::Item::UInt(v)) = dec.next() {
                    chat_id = v as i64;
                }
            }
            _ => {
                let _ = dec.next();
            }
        }
    }
    (text, chat_id)
}
