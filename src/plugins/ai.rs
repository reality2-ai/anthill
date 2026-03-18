//! AiPlugin — handles Claude API calls and output buffering for ai mode.
//!
//! Manages: translate requests, summarise requests, output accumulation,
//! conversation history, and Telegram messaging.
//!
//! Commands from the sentant:
//!   CMD_TRANSLATE    — translate user input to a shell command
//!   CMD_SEND_COMMAND — pop translated command, send to Telegram, emit RELAY_INPUT
//!   CMD_APPEND_OUTPUT — accumulate PTY output bytes
//!   CMD_SUMMARISE    — summarise accumulated output
//!   CMD_SEND_SUMMARY — pop summary, send to Telegram
//!   CMD_SEND_TEXT    — send a text message to Telegram (e.g. "Thinking...")
//!   CMD_BUSY         — send "Still processing..." to Telegram

use r2_engine::plugin::*;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use crate::claude_worker::{AiKind, AiRequest, AiResponse, ConversationTurn};
use crate::events::RELAY_AI_READY;

pub const CMD_TRANSLATE: u8 = 0x01;
pub const CMD_POP_REPLY: u8 = 0x02;
pub const CMD_APPEND_OUTPUT: u8 = 0x03;
pub const CMD_SUMMARISE: u8 = 0x04;
pub const CMD_SEND_TEXT: u8 = 0x05;
pub const CMD_NO_OUTPUT: u8 = 0x06;

const MAX_HISTORY: usize = 20;

pub struct AiMediationPlugin {
    id: PluginId,
    response_queue: Arc<Mutex<VecDeque<AiResponse>>>,
    poll_buf: Vec<u8>,
    request_tx: mpsc::UnboundedSender<AiRequest>,
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
    /// Shared message queue from Telegram (data plane).
    message_queue: crate::plugins::telegram_bot::MessageQueue,
    /// Accumulated PTY output.
    output_buffer: Vec<u8>,
    /// Conversation history.
    history: Vec<ConversationTurn>,
    /// Current user input (for history).
    current_user_input: String,
    /// Current translated command (for history).
    current_command: String,
    /// Current chat_id.
    chat_id: i64,
}

impl AiMediationPlugin {
    pub fn new(
        id: PluginId,
        response_queue: Arc<Mutex<VecDeque<AiResponse>>>,
        request_tx: mpsc::UnboundedSender<AiRequest>,
        telegram_tx: mpsc::UnboundedSender<(i64, String)>,
        message_queue: crate::plugins::telegram_bot::MessageQueue,
    ) -> Self {
        Self {
            id,
            response_queue,
            poll_buf: Vec::new(),
            request_tx,
            telegram_tx,
            message_queue,
            output_buffer: Vec::with_capacity(8192),
            history: Vec::new(),
            current_user_input: String::new(),
            current_command: String::new(),
            chat_id: 0,
        }
    }

    fn send_telegram(&self, chat_id: i64, text: &str) {
        let _ = self.telegram_tx.send((chat_id, text.to_string()));
    }

    fn decode_chat_id(data: &[u8]) -> i64 {
        decode_uint_key(data, 1).unwrap_or(0) as i64
    }
}

impl Plugin for AiMediationPlugin {
    fn execute(&mut self, command: PluginCommand, data: &[u8]) -> PluginResult {
        match command {
            CMD_TRANSLATE => {
                let chat_id = Self::decode_chat_id(data);
                self.chat_id = chat_id;

                // Pop message text from data plane.
                let text = self.message_queue.lock().ok()
                    .and_then(|mut q| q.pop_front())
                    .map(|(_, t)| t)
                    .unwrap_or_default();

                if text.is_empty() {
                    return PluginResult::Ok(PluginResponse::empty());
                }

                self.current_user_input = text.clone();
                self.send_telegram(chat_id, "Thinking...");

                let _ = self.request_tx.send(AiRequest {
                    kind: AiKind::Translate,
                    chat_id,
                    content: text,
                    history: self.history.clone(),
                });

                PluginResult::Ok(PluginResponse::empty())
            }

            CMD_POP_REPLY => {
                // Pop response, store command, send to Telegram, return command text
                // in the response data so the sentant can emit RELAY_INPUT.
                if let Ok(mut q) = self.response_queue.lock() {
                    if let Some(resp) = q.pop_front() {
                        let command = resp.text.trim().to_string();
                        self.current_command = command.clone();
                        self.send_telegram(self.chat_id, &format!("Running: `{}`", command));
                        self.output_buffer.clear();

                        // Return the command text in the response (for RELAY_INPUT).
                        let cmd_bytes = command.as_bytes();
                        let response_data = if cmd_bytes.len() <= 128 {
                            PluginResponse::with_data(cmd_bytes)
                        } else {
                            PluginResponse::with_data(&cmd_bytes[..128])
                        };
                        return PluginResult::Ok(response_data);
                    }
                }
                PluginResult::Ok(PluginResponse::empty())
            }

            CMD_APPEND_OUTPUT => {
                if let Some(bytes) = decode_bytes_key0(data) {
                    self.output_buffer.extend_from_slice(&bytes);
                }
                PluginResult::Ok(PluginResponse::empty())
            }

            CMD_SUMMARISE => {
                let raw = String::from_utf8_lossy(&self.output_buffer).to_string();
                let raw = strip_ansi(&raw);

                if raw.trim().is_empty() {
                    // No output — handled by CMD_NO_OUTPUT instead.
                    return PluginResult::Ok(PluginResponse::empty());
                }

                let _ = self.request_tx.send(AiRequest {
                    kind: AiKind::Summarise,
                    chat_id: self.chat_id,
                    content: raw,
                    history: Vec::new(),
                });

                PluginResult::Ok(PluginResponse::empty())
            }

            CMD_SEND_TEXT => {
                let chat_id = Self::decode_chat_id(data);
                // Pop response and send to Telegram.
                if let Ok(mut q) = self.response_queue.lock() {
                    if let Some(resp) = q.pop_front() {
                        self.send_telegram(chat_id, &resp.text);

                        // Update history.
                        self.history.push(ConversationTurn {
                            user_input: self.current_user_input.clone(),
                            command: self.current_command.clone(),
                        });
                        if self.history.len() > MAX_HISTORY {
                            self.history.remove(0);
                        }
                    }
                }
                PluginResult::Ok(PluginResponse::empty())
            }

            CMD_NO_OUTPUT => {
                let chat_id = Self::decode_chat_id(data);
                self.send_telegram(chat_id, "(no output)");
                self.history.push(ConversationTurn {
                    user_input: self.current_user_input.clone(),
                    command: self.current_command.clone(),
                });
                if self.history.len() > MAX_HISTORY {
                    self.history.remove(0);
                }
                PluginResult::Ok(PluginResponse::empty())
            }

            _ => PluginResult::Error(PluginError::new(0xFF, "unknown command")),
        }
    }

    fn name(&self) -> &str {
        "ai-mediation"
    }

    fn id(&self) -> PluginId {
        self.id
    }

    fn poll(&mut self) -> Option<(u32, &[u8])> {
        let queue = self.response_queue.lock().ok()?;
        let front = queue.front()?;
        let kind = front.kind as u64;
        let chat_id = front.chat_id as u64;
        drop(queue);

        self.poll_buf.clear();
        self.poll_buf.push(0xA2);
        self.poll_buf.push(0x00);
        self.poll_buf.push(kind as u8);
        self.poll_buf.push(0x01);
        encode_uint(&mut self.poll_buf, chat_id);

        Some((RELAY_AI_READY, &self.poll_buf))
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

fn decode_bytes_key0(data: &[u8]) -> Option<Vec<u8>> {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let r2_cbor::Item::Map(_) = dec.next().ok()? else { return None };
    let r2_cbor::Item::UInt(0) = dec.next().ok()? else { return None };
    let r2_cbor::Item::Bytes(b) = dec.next().ok()? else { return None };
    Some(b.to_vec())
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

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_escape = false;
    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() { in_escape = false; }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            out.push(c);
        }
    }
    out
}
