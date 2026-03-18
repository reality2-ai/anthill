//! Terminal sentant — manages PTY lifecycle.
//!
//! States: Idle(0) → Active(1)
//! On RELAY_INPUT: if idle, spawn PTY then write; if active, just write.
//! On RELAY_OUTPUT: re-emit with chat_id attached (PTY doesn't know chat_id).
//! On RELAY_PTY_EXIT: → Idle.

use r2_engine::action::Action;
use r2_engine::action_buf::ActionBuf;
use r2_engine::event::{Event, Target};
use r2_engine::plugin::PluginId;
use r2_engine::sentant::{Sentant, StateId};

use crate::events::*;
use crate::plugins::pty;

const STATE_IDLE: StateId = 0;
const STATE_ACTIVE: StateId = 1;

pub struct TerminalSentant {
    state: StateId,
    pty_plugin_id: PluginId,
    /// Current chat_id from the most recent input.
    chat_id: i64,
}

impl TerminalSentant {
    pub fn new(pty_plugin_id: PluginId) -> Self {
        Self {
            state: STATE_IDLE,
            pty_plugin_id,
            chat_id: 0,
        }
    }
}

impl Sentant for TerminalSentant {
    fn handle_event(&mut self, event: &Event, actions: &mut ActionBuf) {
        match event.hash {
            RELAY_INPUT => {
                // Decode text and chat_id from payload: { 0: text, 1: uint(chat_id) }
                let (text, chat_id) = decode_text_and_chat(event.payload);
                if text.is_empty() {
                    return;
                }
                self.chat_id = chat_id;

                if self.state == STATE_IDLE {
                    // Spawn PTY first.
                    actions.push(Action::plugin_call(
                        self.pty_plugin_id,
                        pty::CMD_SPAWN,
                        &[],
                    ));
                    self.state = STATE_ACTIVE;
                }

                // Map special commands to control sequences for TUI apps.
                let input = match text.as_str() {
                    "/enter" => b"\r".to_vec(),
                    "/esc" => b"\x1b".to_vec(),
                    "/up" => b"\x1b[A".to_vec(),
                    "/down" => b"\x1b[B".to_vec(),
                    "/left" => b"\x1b[D".to_vec(),
                    "/right" => b"\x1b[C".to_vec(),
                    "/tab" => b"\x09".to_vec(),
                    "/ctrl-c" => b"\x03".to_vec(),
                    "/ctrl-d" => b"\x04".to_vec(),
                    "/ctrl-z" => b"\x1a".to_vec(),
                    "/space" => b" ".to_vec(),
                    _ => {
                        let mut v = text.as_bytes().to_vec();
                        v.push(b'\n');
                        v
                    }
                };
                actions.push(Action::plugin_call(
                    self.pty_plugin_id,
                    pty::CMD_WRITE,
                    &input,
                ));
            }

            RELAY_PTY_RAW => {
                // PTY output arrived (from PtyPlugin.poll). It has { 0: bytes }
                // but no chat_id. Re-emit as RELAY_OUTPUT with chat_id so
                // the chunker knows where to send the reply.
                if let Some(data) = decode_bytes_key0(event.payload) {
                    let payload = encode_bytes_chat_cbor(&data, self.chat_id);
                    // Truncate to 256 bytes if needed (PayloadBuf limit).
                    let capped = &payload[..payload.len().min(256)];
                    actions.push(Action::send(Target::Local, RELAY_OUTPUT, capped));
                }
            }

            RELAY_PTY_EXIT => {
                self.state = STATE_IDLE;
            }

            _ => {}
        }
    }

    fn state(&self) -> StateId {
        self.state
    }

    fn class_hash(&self) -> u32 {
        r2_fnv::fnv1a_32(b"ai.reality2.relay.terminal")
    }

    fn name(&self) -> &str {
        "terminal"
    }

    fn subscriptions(&self) -> &[u32] {
        &[RELAY_INPUT, RELAY_PTY_RAW, RELAY_PTY_EXIT]
    }
}

/// Extract text (key 0) and chat_id (key 1) from CBOR map.
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

/// Decode bytes from CBOR key 0.
fn decode_bytes_key0(data: &[u8]) -> Option<Vec<u8>> {
    let mut dec = r2_cbor::Decoder::new_with_mode(data, r2_cbor::Mode::Standard);
    let r2_cbor::Item::Map(_) = dec.next().ok()? else { return None };
    let r2_cbor::Item::UInt(0) = dec.next().ok()? else { return None };
    let r2_cbor::Item::Bytes(b) = dec.next().ok()? else { return None };
    Some(b.to_vec())
}

/// Hand-encode CBOR: { 0: bytes(data), 1: uint(chat_id) }
fn encode_bytes_chat_cbor(data: &[u8], chat_id: i64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(data.len() + 20);
    buf.push(0xA2); // map(2)
    buf.push(0x00); // key 0
    // bstr header
    let len = data.len();
    if len <= 23 {
        buf.push(0x40 | len as u8);
    } else if len <= 255 {
        buf.push(0x58);
        buf.push(len as u8);
    } else {
        buf.push(0x59);
        buf.extend_from_slice(&(len as u16).to_be_bytes());
    }
    buf.extend_from_slice(data);
    // key 1, value uint(chat_id)
    buf.push(0x01);
    let cid = chat_id as u64;
    if cid <= 23 {
        buf.push(cid as u8);
    } else if cid <= 0xFF {
        buf.push(0x18);
        buf.push(cid as u8);
    } else if cid <= 0xFFFF_FFFF {
        buf.push(0x1A);
        buf.extend_from_slice(&(cid as u32).to_be_bytes());
    } else {
        buf.push(0x1B);
        buf.extend_from_slice(&cid.to_be_bytes());
    }
    buf
}
