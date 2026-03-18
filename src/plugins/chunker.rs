//! ChunkerPlugin — buffers PTY output and sends to Telegram in chunks.
//!
//! Accumulates raw bytes, strips ANSI escapes, splits into Telegram-safe
//! chunks, and sends via the Telegram data plane.
//!
//! Commands from the sentant:
//!   CMD_APPEND — append bytes to the buffer (data from event payload)
//!   CMD_FLUSH  — flush the buffer to Telegram

use r2_engine::plugin::*;
use tokio::sync::mpsc;

pub const CMD_APPEND: u8 = 0x01;
pub const CMD_FLUSH: u8 = 0x02;

/// Maximum Telegram message size (leave room for formatting).
const TELEGRAM_CHUNK_MAX: usize = 4000;

pub struct ChunkerPlugin {
    id: PluginId,
    buffer: Vec<u8>,
    chat_id: i64,
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
}

impl ChunkerPlugin {
    pub fn new(id: PluginId, telegram_tx: mpsc::UnboundedSender<(i64, String)>) -> Self {
        Self {
            id,
            buffer: Vec::with_capacity(8192),
            chat_id: 0,
            telegram_tx,
        }
    }
}

impl Plugin for ChunkerPlugin {
    fn execute(&mut self, command: PluginCommand, data: &[u8]) -> PluginResult {
        match command {
            CMD_APPEND => {
                // Decode bytes from payload: { 0: bytes(data), 1: uint(chat_id) }
                if let Some(bytes) = decode_bytes_key0(data) {
                    self.buffer.extend_from_slice(&bytes);
                }
                if let Some(cid) = decode_uint_key(data, 1) {
                    self.chat_id = cid as i64;
                }
                PluginResult::Ok(PluginResponse::empty())
            }
            CMD_FLUSH => {
                if !self.buffer.is_empty() {
                    let text = String::from_utf8_lossy(&self.buffer).to_string();
                    let text = strip_ansi(&text);
                    self.buffer.clear();

                    for chunk in text.as_bytes().chunks(TELEGRAM_CHUNK_MAX) {
                        let chunk_str = String::from_utf8_lossy(chunk);
                        let formatted = format!("```\n{}\n```", chunk_str);
                        let _ = self.telegram_tx.send((self.chat_id, formatted));
                    }
                }
                PluginResult::Ok(PluginResponse::empty())
            }
            _ => PluginResult::Error(PluginError::new(0xFF, "unknown command")),
        }
    }

    fn name(&self) -> &str {
        "chunker"
    }

    fn id(&self) -> PluginId {
        self.id
    }
}

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

fn decode_bytes_key0(data: &[u8]) -> Option<Vec<u8>> {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let r2_cbor::Item::Map(_) = dec.next().ok()? else { return None };
    let r2_cbor::Item::UInt(0) = dec.next().ok()? else { return None };
    let r2_cbor::Item::Bytes(b) = dec.next().ok()? else { return None };
    Some(b.to_vec())
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
