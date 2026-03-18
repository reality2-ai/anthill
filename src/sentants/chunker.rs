//! Chunker sentant — batches and splits PTY output for Telegram.
//!
//! Accumulates RELAY_OUTPUT events, debounces for 500ms via DelayedSend,
//! then splits into <=4000-char chunks sent directly to Telegram via
//! a side-channel (bypasses engine for large payloads).

use r2_engine::action::Action;
use r2_engine::action_buf::ActionBuf;
use r2_engine::event::{Event, Target};
use r2_engine::sentant::{Sentant, StateId};
use tokio::sync::mpsc;

use crate::events::*;

const STATE_IDLE: StateId = 0;
const STATE_BUFFERING: StateId = 1;

/// Maximum Telegram message size (leave room for formatting).
const TELEGRAM_CHUNK_MAX: usize = 4000;

/// Debounce delay in milliseconds.
const DEBOUNCE_MS: u32 = 500;

// TODO: This sentant holds a Telegram channel (violation of R2 pure FSM principle).
// The chunking and Telegram sending should move to a ChunkerPlugin.
// This is only used in raw mode which is a secondary concern.
pub struct ChunkerSentant {
    state: StateId,
    buffer: Vec<u8>,
    chat_id: i64,
    /// Direct channel to Telegram outgoing queue (bypasses engine for large payloads).
    telegram_tx: mpsc::UnboundedSender<(i64, String)>,
}

impl ChunkerSentant {
    pub fn new(telegram_tx: mpsc::UnboundedSender<(i64, String)>) -> Self {
        Self {
            state: STATE_IDLE,
            buffer: Vec::with_capacity(8192),
            chat_id: 0,
            telegram_tx,
        }
    }

    fn flush(&mut self, _actions: &mut ActionBuf) {
        if self.buffer.is_empty() {
            self.state = STATE_IDLE;
            return;
        }

        // Convert to lossy UTF-8, strip ANSI escapes.
        let text = String::from_utf8_lossy(&self.buffer).to_string();
        let text = strip_ansi(&text);
        self.buffer.clear();

        // Split into Telegram-safe chunks and send each directly.
        for chunk in text.as_bytes().chunks(TELEGRAM_CHUNK_MAX) {
            let chunk_str = String::from_utf8_lossy(chunk);
            let formatted = format!("```\n{}\n```", chunk_str);
            let _ = self.telegram_tx.send((self.chat_id, formatted));
        }

        self.state = STATE_IDLE;
    }
}

impl Sentant for ChunkerSentant {
    fn handle_event(&mut self, event: &Event, actions: &mut ActionBuf) {
        match event.hash {
            RELAY_OUTPUT => {
                // Decode bytes from payload: { 0: bytes(data) }
                if let Some(data) = decode_bytes_from_payload(event.payload) {
                    self.buffer.extend_from_slice(&data);
                }

                // Extract chat_id if present (key 1).
                if let Some(cid) = decode_uint_key(event.payload, 1) {
                    self.chat_id = cid as i64;
                }

                if self.state == STATE_IDLE {
                    self.state = STATE_BUFFERING;
                    // Schedule debounce flush.
                    actions.push(Action::delayed_send(
                        DEBOUNCE_MS,
                        Target::Local,
                        RELAY_FLUSH,
                        &[0xA0], // empty CBOR map
                    ));
                }
            }

            RELAY_FLUSH => {
                self.flush(actions);
            }

            _ => {}
        }
    }

    fn state(&self) -> StateId {
        self.state
    }

    fn class_hash(&self) -> u32 {
        r2_fnv::fnv1a_32(b"ai.reality2.relay.chunker")
    }

    fn name(&self) -> &str {
        "chunker"
    }

    fn subscriptions(&self) -> &[u32] {
        &[RELAY_OUTPUT, RELAY_FLUSH]
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

/// Decode bytes from CBOR key 0: { 0: bytes(...) }
fn decode_bytes_from_payload(data: &[u8]) -> Option<Vec<u8>> {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let r2_cbor::Item::Map(_) = dec.next().ok()? else { return None };
    let r2_cbor::Item::UInt(0) = dec.next().ok()? else { return None };
    let r2_cbor::Item::Bytes(b) = dec.next().ok()? else { return None };
    Some(b.to_vec())
}

/// Decode a uint value for a given key from a CBOR map.
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
