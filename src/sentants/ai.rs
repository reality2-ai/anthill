//! AI sentant — FSM that mediates between Telegram and PTY via Claude API.
//!
//! States: Idle(0) → Translating(1) → Executing(2) → Summarising(3) → Idle
//!
//! Translates natural language to shell commands (via Claude), executes them,
//! buffers PTY output, then summarises the output for mobile (via Claude).

use r2_engine::action::Action;
use r2_engine::action_buf::ActionBuf;
use r2_engine::event::{Event, Target};
use r2_engine::sentant::{Sentant, StateId};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::claude_worker::{AiKind, AiRequest, AiResponse, ConversationTurn};
use crate::events::*;

const STATE_IDLE: StateId = 0;
const STATE_TRANSLATING: StateId = 1;
const STATE_EXECUTING: StateId = 2;
const STATE_SUMMARISING: StateId = 3;

/// Output debounce delay — wait for PTY output to settle.
const COLLECT_DEBOUNCE_MS: u32 = 2000;

/// Maximum conversation turns to keep for context.
const MAX_HISTORY: usize = 20;

pub struct AiSentant {
    state: StateId,
    chat_id: i64,
    /// Channel to send requests to the background Claude worker.
    request_tx: mpsc::UnboundedSender<AiRequest>,
    /// Shared response queue (popped here, pushed by worker, polled by AiPlugin).
    response_queue: Arc<Mutex<VecDeque<AiResponse>>>,
    /// Direct channel to Telegram outgoing queue (bypasses engine for large text).
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
    /// Accumulated PTY output before summarisation.
    output_buffer: Vec<u8>,
    /// Conversation history for context.
    history: Vec<ConversationTurn>,
    /// The user's original input (for history tracking).
    current_user_input: String,
    /// The translated command (for history tracking).
    current_command: String,
}

impl AiSentant {
    pub fn new(
        request_tx: mpsc::UnboundedSender<AiRequest>,
        response_queue: Arc<Mutex<VecDeque<AiResponse>>>,
        telegram_tx: mpsc::UnboundedSender<(i64, String)>,
    ) -> Self {
        Self {
            state: STATE_IDLE,
            chat_id: 0,
            request_tx,
            response_queue,
            telegram_tx,
            output_buffer: Vec::with_capacity(8192),
            history: Vec::new(),
            current_user_input: String::new(),
            current_command: String::new(),
        }
    }

    fn send_telegram(&self, text: &str) {
        let _ = self.telegram_tx.send((self.chat_id, text.to_string()));
    }

    fn pop_response(&self) -> Option<AiResponse> {
        self.response_queue.lock().ok()?.pop_front()
    }
}

impl Sentant for AiSentant {
    fn handle_event(&mut self, event: &Event, actions: &mut ActionBuf) {
        match event.hash {
            RELAY_COMMAND => {
                if self.state != STATE_IDLE {
                    // Busy — tell user to wait.
                    let (_, chat_id) = decode_text_and_chat(event.payload);
                    let _ = self.telegram_tx.send((chat_id, "Still processing previous command...".into()));
                    return;
                }

                let (text, chat_id) = decode_text_and_chat(event.payload);
                if text.is_empty() {
                    return;
                }
                self.chat_id = chat_id;
                self.current_user_input = text.clone();

                // Send "Thinking..." to Telegram.
                self.send_telegram("Thinking...");

                // Push translate request to background worker.
                let _ = self.request_tx.send(AiRequest {
                    kind: AiKind::Translate,
                    chat_id,
                    content: text,
                    history: self.history.clone(),
                });

                self.state = STATE_TRANSLATING;
            }

            RELAY_AI_READY => {
                match self.state {
                    STATE_TRANSLATING => {
                        if let Some(resp) = self.pop_response() {
                            if resp.kind != AiKind::Translate {
                                return;
                            }

                            let command = resp.text.trim().to_string();
                            self.current_command = command.clone();

                            // Tell user what we're running.
                            self.send_telegram(&format!("Running: `{}`", command));

                            // Clear output buffer for this execution.
                            self.output_buffer.clear();

                            // Emit RELAY_INPUT so TerminalSentant executes the command.
                            let payload = encode_text_chat_cbor(&command, self.chat_id);
                            let capped = &payload[..payload.len().min(256)];
                            actions.push(Action::send(Target::Local, RELAY_INPUT, capped));

                            self.state = STATE_EXECUTING;
                        }
                    }

                    STATE_SUMMARISING => {
                        if let Some(resp) = self.pop_response() {
                            if resp.kind != AiKind::Summarise {
                                return;
                            }

                            // Send clean summary to Telegram.
                            self.send_telegram(&resp.text);

                            // Append to conversation history.
                            self.history.push(ConversationTurn {
                                user_input: self.current_user_input.clone(),
                                command: self.current_command.clone(),
                            });

                            // Cap history length.
                            if self.history.len() > MAX_HISTORY {
                                self.history.remove(0);
                            }

                            self.state = STATE_IDLE;
                        }
                    }

                    _ => {}
                }
            }

            RELAY_OUTPUT => {
                if self.state != STATE_EXECUTING {
                    return;
                }

                // Accumulate PTY output.
                if let Some(data) = decode_bytes_from_payload(event.payload) {
                    self.output_buffer.extend_from_slice(&data);
                }

                // Schedule (or reset) debounce timer.
                let payload = encode_uint_chat_cbor(self.chat_id);
                actions.push(Action::delayed_send(
                    COLLECT_DEBOUNCE_MS,
                    Target::Local,
                    RELAY_AI_COLLECT,
                    &payload,
                ));
            }

            RELAY_AI_COLLECT => {
                if self.state != STATE_EXECUTING {
                    return;
                }

                // Debounce fired — output has settled. Summarise it.
                let raw = String::from_utf8_lossy(&self.output_buffer).to_string();
                let raw = strip_ansi(&raw);

                if raw.trim().is_empty() {
                    self.send_telegram("(no output)");
                    self.history.push(ConversationTurn {
                        user_input: self.current_user_input.clone(),
                        command: self.current_command.clone(),
                    });
                    if self.history.len() > MAX_HISTORY {
                        self.history.remove(0);
                    }
                    self.state = STATE_IDLE;
                    return;
                }

                // Push summarise request.
                let _ = self.request_tx.send(AiRequest {
                    kind: AiKind::Summarise,
                    chat_id: self.chat_id,
                    content: raw,
                    history: Vec::new(),
                });

                self.state = STATE_SUMMARISING;
            }

            _ => {}
        }
    }

    fn state(&self) -> StateId {
        self.state
    }

    fn class_hash(&self) -> u32 {
        r2_fnv::fnv1a_32(b"ai.reality2.relay.ai")
    }

    fn name(&self) -> &str {
        "ai"
    }

    fn subscriptions(&self) -> &[u32] {
        &[RELAY_COMMAND, RELAY_AI_READY, RELAY_AI_COLLECT, RELAY_OUTPUT]
    }
}

// --- CBOR helpers ---

/// Decode text (key 0) and chat_id (key 1) from CBOR map.
fn decode_text_and_chat(data: &[u8]) -> (String, i64) {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let mut text = String::new();
    let mut chat_id: i64 = 0;

    let Ok(r2_cbor::Item::Map(n)) = dec.next() else { return (text, chat_id) };
    for _ in 0..n {
        let Ok(r2_cbor::Item::UInt(key)) = dec.next() else { break };
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
            _ => { let _ = dec.next(); }
        }
    }
    (text, chat_id)
}

/// Decode bytes from CBOR key 0: { 0: bytes(...) }
fn decode_bytes_from_payload(data: &[u8]) -> Option<Vec<u8>> {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let r2_cbor::Item::Map(_) = dec.next().ok()? else { return None };
    let r2_cbor::Item::UInt(0) = dec.next().ok()? else { return None };
    let r2_cbor::Item::Bytes(b) = dec.next().ok()? else { return None };
    Some(b.to_vec())
}

/// Hand-encode CBOR: { 0: text(command), 1: uint(chat_id) }
fn encode_text_chat_cbor(text: &str, chat_id: i64) -> Vec<u8> {
    let text_bytes = text.as_bytes();
    let mut buf = Vec::with_capacity(text_bytes.len() + 20);
    buf.push(0xA2); // map(2)
    // key 0, value text
    buf.push(0x00);
    let len = text_bytes.len();
    if len <= 23 {
        buf.push(0x60 | len as u8);
    } else if len <= 255 {
        buf.push(0x78);
        buf.push(len as u8);
    } else {
        buf.push(0x79);
        buf.extend_from_slice(&(len as u16).to_be_bytes());
    }
    buf.extend_from_slice(text_bytes);
    // key 1, value uint(chat_id)
    buf.push(0x01);
    encode_uint_into(&mut buf, chat_id as u64);
    buf
}

/// Hand-encode CBOR: { 0: uint(chat_id) }
fn encode_uint_chat_cbor(chat_id: i64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(12);
    buf.push(0xA1); // map(1)
    buf.push(0x00); // key 0
    encode_uint_into(&mut buf, chat_id as u64);
    buf
}

fn encode_uint_into(buf: &mut Vec<u8>, v: u64) {
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

/// Strip ANSI escape sequences from text.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            out.push(c);
        }
    }
    out
}
