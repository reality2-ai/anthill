//! AiPlugin — manages Claude Code invocations and responses.
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

use crate::ai_worker::{CliRequest, CliResponse, FollowUp, FollowUpQueue, StatsMap, TaskMap};
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
pub const CMD_STATUS: u8 = 0x09;    // Show live status of workers
pub const CMD_FOLLOWUP: u8 = 0x0A;  // Queue follow-up for a running task
pub const CMD_ANALYSE: u8 = 0x0B;   // Thematic analysis of a file
pub const CMD_REFLECT: u8 = 0x0C;   // Meta-analysis / reflect on knowledge graph
pub const CMD_RUMINATE: u8 = 0x0F;  // Trigger rumination cycle manually
#[allow(dead_code)]
pub const CMD_QUESTIONS: u8 = 0x10; // Show pending questions from rumination
pub const CMD_SPECIFY: u8 = 0x0D;   // Generate spec from code
pub const CMD_TEST_VECTORS: u8 = 0x0E; // Generate test vectors from code/spec

const HELP_TEXT: &str = "\
**anthill commands:**

/help — show this message
/status — live view of what each worker is doing right now
/ants — show running workers and what they're working on
/usage — show session statistics
/cancel — cancel a running task (or /cancel <id>, /cancel all)
/followup — queue a message for when the current task finishes
/new — start a fresh conversation
/analyse <file> — thematic analysis on a file → knowledge graph
/reflect — review and consolidate the knowledge graph
/ruminate — trigger a rumination cycle now (refute, synthesise, compete)
/questions — show pending questions from rumination
/specify <file> — generate a specification from code
/test-vectors <file> — generate test vectors from code

**AI commands** (passed through to the active backend):

/compact — condense conversation context
/cost — show token/cost usage for the session
/model — show or change the AI model
/memory — manage memory files
/clear — clear conversation history

Everything else is sent as a prompt. Multiple messages run concurrently.";

const TELEGRAM_MAX: usize = 4000;

pub struct AiPlugin {
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
    /// Follow-up queue for running tasks.
    follow_ups: FollowUpQueue,
    /// Next task ID counter.
    next_task_id: u32,
    /// Whether to forward user messages across channels.
    sync_channels: bool,
}

impl AiPlugin {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: PluginId,
        response_queue: Arc<Mutex<VecDeque<CliResponse>>>,
        request_tx: mpsc::UnboundedSender<CliRequest>,
        telegram_tx: mpsc::UnboundedSender<(i64, String)>,
        tasks: TaskMap,
        stats: StatsMap,
        follow_ups: FollowUpQueue,
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
            follow_ups,
            next_task_id: 1,
            sync_channels,
        }
    }

    fn send_telegram(&self, chat_id: i64, text: &str) {
        if text.len() <= TELEGRAM_MAX {
            let _ = self.telegram_tx.send((chat_id, text.to_string()));
        } else {
            // Split on char boundaries to avoid corrupting multibyte UTF-8.
            let mut start = 0;
            while start < text.len() {
                let mut end = (start + TELEGRAM_MAX).min(text.len());
                // Walk back to a char boundary if we landed mid-character.
                while end > start && !text.is_char_boundary(end) {
                    end -= 1;
                }
                if end == start {
                    // Pathological case: advance to next char boundary.
                    end = start + 1;
                    while end < text.len() && !text.is_char_boundary(end) {
                        end += 1;
                    }
                }
                let _ = self.telegram_tx.send((chat_id, text[start..end].to_string()));
                start = end;
            }
        }
    }

    fn send_typing(&self, chat_id: i64) {
        let _ = self.telegram_tx.send((chat_id, String::new()));
    }

    fn decode_chat_id(data: &[u8]) -> i64 {
        decode_uint_key(data, 1).unwrap_or(0) as i64
    }

    /// Check if a source is allowed for sensitive operations (file access, graph modification).
    /// Telegram and Slack can see responses but shouldn't trigger file reads.
    fn is_sensitive_allowed(source: &str) -> bool {
        matches!(source, "web" | "system")
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

        // Auto-followup: if exactly one task is running for this chat,
        // queue the message as a follow-up instead of starting a new task.
        // Exception: if the message starts with "!" it interrupts — cancels
        // the current task and restarts with both prompts combined.
        if let Ok(mut map) = self.tasks.lock() {
            let running_for_chat: Vec<u32> = map.values()
                .filter(|t| t.chat_id == chat_id)
                .map(|t| t.task_id)
                .collect();
            if running_for_chat.len() == 1 {
                let target_task = running_for_chat[0];

                if text.starts_with('!') {
                    // Interrupt: cancel current task and restart with combined context.
                    let new_text = text.strip_prefix('!').unwrap_or(&text).trim().to_string();
                    let original_preview = map.get(&target_task)
                        .map(|t| t.message_preview.clone())
                        .unwrap_or_default();
                    if let Some(task) = map.remove(&target_task) {
                        task.handle.abort();
                    }
                    drop(map);
                    // Combined message: original context + new instruction.
                    let combined = format!(
                        "{}\n\nADDITIONAL CONTEXT (added while you were working):\n{}",
                        original_preview, new_text
                    );
                    self.send_telegram(chat_id, &format!(
                        "🔄 Interrupted task #{} — restarting with your addition.",
                        target_task
                    ));
                    // Dispatch the combined message as a new task.
                    let task_id = self.next_task_id;
                    self.next_task_id += 1;
                    let _ = self.request_tx.send(CliRequest {
                        chat_id,
                        message: combined,
                        new_session: false,
                        task_id,
                        source,
                    });
                    return;
                }

                // Default: queue as follow-up.
                drop(map);
                if let Ok(mut fq) = self.follow_ups.lock() {
                    fq.entry(target_task).or_default().push(
                        crate::ai_worker::FollowUp {
                            chat_id,
                            message: text,
                            source,
                        }
                    );
                }
                self.send_telegram(chat_id, &format!(
                    "📋 Queued for after task #{}. Start with ! to interrupt and restart instead.",
                    target_task
                ));
                return;
            }
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
                        if let Ok(mut s) = task.state.lock() {
                            *s = crate::ai_worker::TaskState::Cancelled;
                        }
                        task.handle.abort();
                        self.send_telegram(chat_id, &format!("Cancelled task #{}.", id));
                    }
                } else {
                    self.send_telegram(chat_id, "No running tasks.");
                }
            } else if let Some(task) = map.remove(&task_id) {
                if let Ok(mut s) = task.state.lock() {
                    *s = crate::ai_worker::TaskState::Cancelled;
                }
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

    fn handle_status(&self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        let map = match self.tasks.lock() {
            Ok(m) => m,
            Err(_) => {
                self.send_telegram(chat_id, "Status unavailable.");
                return;
            }
        };

        if map.is_empty() {
            self.send_telegram(chat_id, "All workers idle.");
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

            let backend = task.backend.lock()
                .map(|b| if b.is_empty() { "starting".to_string() } else { b.clone() })
                .unwrap_or_else(|_| "?".into());

            let progress = task.last_progress.lock()
                .ok()
                .and_then(|p| p.clone())
                .unwrap_or_else(|| "waiting...".into());

            // Check for queued follow-ups.
            let follow_count = self.follow_ups.lock()
                .map(|fq| fq.get(&task.task_id).map(|v| v.len()).unwrap_or(0))
                .unwrap_or(0);
            let follow_str = if follow_count > 0 {
                format!("\n  📋 {} follow-up{} queued", follow_count, if follow_count == 1 { "" } else { "s" })
            } else {
                String::new()
            };

            out.push_str(&format!(
                "**#{}** [{backend}] — {}\n  ⏱ {}\n  → {}{}\n\n",
                task.task_id, task.message_preview, duration, progress, follow_str
            ));
        }
        out.push_str("Use /followup <text> to queue context for the current task.\nUse /cancel <id> to stop a worker.");
        self.send_telegram(chat_id, &out);
    }

    fn handle_followup(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        // Pop the follow-up message text from the data plane queue.
        let (_, text, source) = match self.message_queue.lock().ok().and_then(|mut q| q.pop_front()) {
            Some(msg) => msg,
            None => {
                self.send_telegram(chat_id, "Usage: /followup <message>\nQueues a message for when the current task finishes.");
                return;
            }
        };

        let map = match self.tasks.lock() {
            Ok(m) => m,
            Err(_) => {
                self.send_telegram(chat_id, "No running tasks — sending as a new message instead.");
                let _ = self.request_tx.send(CliRequest {
                    chat_id,
                    message: text,
                    new_session: false,
                    task_id: 0,
                    source,
                });
                return;
            }
        };

        if map.is_empty() {
            drop(map);
            // No tasks running — dispatch immediately.
            self.send_telegram(chat_id, "No running tasks — sending as a new message.");
            let task_id = self.next_task_id;
            self.next_task_id += 1;
            let _ = self.request_tx.send(CliRequest {
                chat_id,
                message: text,
                new_session: false,
                task_id,
                source,
            });
            return;
        }

        // Find the most recent task for this chat.
        let target_task = map
            .values()
            .filter(|t| t.chat_id == chat_id)
            .max_by_key(|t| t.task_id)
            .or_else(|| map.values().max_by_key(|t| t.task_id))
            .map(|t| t.task_id);

        drop(map);

        if let Some(task_id) = target_task {
            if let Ok(mut fq) = self.follow_ups.lock() {
                fq.entry(task_id).or_default().push(FollowUp {
                    chat_id,
                    message: text,
                    source,
                });
            }
            self.send_telegram(chat_id, &format!(
                "📋 Queued as follow-up for task #{}. It will run when the task finishes.",
                task_id
            ));
        } else {
            self.send_telegram(chat_id, "Could not find a task to follow up on.");
        }
    }

    fn handle_analyse(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        // Pop the file path from the message queue.
        let (_, text, source) = match self.message_queue.lock().ok().and_then(|mut q| q.pop_front()) {
            Some(msg) => msg,
            None => {
                self.send_telegram(chat_id, "Usage: /analyse <file path>\nRuns thematic analysis on a file and integrates results into the knowledge graph.");
                return;
            }
        };

        let file_path = text.trim().to_string();
        if file_path.is_empty() {
            self.send_telegram(chat_id, "Usage: /analyse <file path>");
            return;
        }

        if !Self::is_sensitive_allowed(&source) {
            self.send_telegram(chat_id, "⚠️ /analyse reads files — use the web dashboard for security.");
            return;
        }

        self.send_telegram(chat_id, &format!("📊 Starting thematic analysis of '{}'...", file_path));

        // Read the file content and build the analysis prompt.
        // The AI will do the actual analysis — we build a combined prompt for short files
        // or chunk for long ones.
        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let _ = self.request_tx.send(CliRequest {
            chat_id,
            message: format!("/analyse {}", file_path),
            new_session: false,
            task_id,
            source,
        });
    }

    fn handle_reflect(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        self.send_telegram(chat_id, "🔍 Reflecting on knowledge graph...");

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let _ = self.request_tx.send(CliRequest {
            chat_id,
            message: "/reflect".into(),
            new_session: false,
            task_id,
            source: "system".into(),
        });
    }

    fn handle_ruminate(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        self.send_telegram(chat_id, "🧠 Starting rumination cycle — refuting, synthesising, competing...");

        // Trigger rumination by sending a meta-prompt that tells the AI to
        // review its knowledge graph, challenge beliefs, and improve ideas.
        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let _ = self.request_tx.send(CliRequest {
            chat_id,
            message: "RUMINATION — MANUAL TRIGGER\n\n\
                You have been asked to ruminate — to actively think about and improve \
                your knowledge graph.\n\n\
                Do ALL of the following:\n\
                1. Read your topic graphs in memory/graphs/\n\
                2. Pick 2-3 important beliefs with moderate confidence (40-80%) and \
                   ATTEMPT TO REFUTE them. Remember: not finding counter-evidence is \
                   'inconsequential_search' (no change), NOT 'refutation_survived'\n\
                3. Look for pairs of strong edges (A→B, B→C) where no A→C exists — \
                   conjecture new transitive relationships with basis 'inferred'\n\
                4. Find competing hypotheses (different relations between the same nodes) \
                   and evaluate which is best supported\n\
                5. Look for cross-domain patterns — similar relationships in different \
                   topic graphs that could inform each other\n\
                6. Set beneficial_impact on edges where relevant (positive for ideas \
                   beneficial to people and planet)\n\
                7. Update the graph files with all changes\n\n\
                Output a summary of what you thought about and what changed.\n\n\
                IMPORTANT: Complete this work and STOP. Do not ask follow-up questions.".into(),
            new_session: true,
            task_id,
            source: "rumination".into(),
        });
    }

    fn handle_specify(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        let (_, text, source) = match self.message_queue.lock().ok().and_then(|mut q| q.pop_front()) {
            Some(msg) => msg,
            None => {
                self.send_telegram(chat_id, "Usage: /specify <file path>\nGenerates a formal specification from source code.");
                return;
            }
        };

        let file_path = text.trim().to_string();
        if file_path.is_empty() {
            self.send_telegram(chat_id, "Usage: /specify <file path>");
            return;
        }

        if !Self::is_sensitive_allowed(&source) {
            self.send_telegram(chat_id, "⚠️ /specify reads files — use the web dashboard for security.");
            return;
        }

        self.send_telegram(chat_id, &format!("📝 Generating specification from '{}'...", file_path));

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let _ = self.request_tx.send(CliRequest {
            chat_id,
            message: format!("/specify {}", file_path),
            new_session: false,
            task_id,
            source,
        });
    }

    fn handle_test_vectors(&mut self, data: &[u8]) {
        let chat_id = Self::decode_chat_id(data);

        let (_, text, source) = match self.message_queue.lock().ok().and_then(|mut q| q.pop_front()) {
            Some(msg) => msg,
            None => {
                self.send_telegram(chat_id, "Usage: /test-vectors <file path>\nGenerates test vectors from source code or a specification.");
                return;
            }
        };

        let file_path = text.trim().to_string();
        if file_path.is_empty() {
            self.send_telegram(chat_id, "Usage: /test-vectors <file path>");
            return;
        }

        if !Self::is_sensitive_allowed(&source) {
            self.send_telegram(chat_id, "⚠️ /test-vectors reads files — use the web dashboard for security.");
            return;
        }

        self.send_telegram(chat_id, &format!("🧪 Generating test vectors from '{}'...", file_path));

        let task_id = self.next_task_id;
        self.next_task_id += 1;

        let _ = self.request_tx.send(CliRequest {
            chat_id,
            message: format!("/test-vectors {}", file_path),
            new_session: false,
            task_id,
            source,
        });
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

impl Plugin for AiPlugin {
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
            CMD_STATUS => { self.handle_status(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_FOLLOWUP => { self.handle_followup(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_ANALYSE => { self.handle_analyse(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_REFLECT => { self.handle_reflect(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_RUMINATE => { self.handle_ruminate(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_SPECIFY => { self.handle_specify(data); PluginResult::Ok(PluginResponse::empty()) }
            CMD_TEST_VECTORS => { self.handle_test_vectors(data); PluginResult::Ok(PluginResponse::empty()) }
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
