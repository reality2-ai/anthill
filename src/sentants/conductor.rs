//! Claude CLI sentant — pure FSM conductor.
//!
//! Receives events, makes decisions, emits actions. No I/O, no channels,
//! no shared state. All data handling is in the AiPlugin.
//!
//! The sentant only sees small event payloads (< 256 bytes):
//!   RELAY_COMMAND: { 0: uint(cmd_type), 1: uint(chat_id) }
//!   RELAY_AI_READY: { 0: uint(kind), 1: uint(chat_id) }
//!
//! It emits Action::plugin_call() to tell the plugin what to do.
//! The plugin handles all I/O (Telegram, Claude worker, file access).

use r2_engine::action::Action;
use r2_engine::action_buf::ActionBuf;
use r2_engine::event::Event;
use r2_engine::plugin::PluginId;
use r2_engine::sentant::{Sentant, StateId};

use crate::events::*;
use crate::plugins::ai_plugin as cli_plugin;

const STATE_READY: StateId = 0;

/// Command types extracted from the first bytes of user input.
/// These are carried in the event payload, not the full text.
const CMD_TYPE_MESSAGE: u8 = 0;     // Regular message → dispatch to AI
const CMD_TYPE_HELP: u8 = 1;
const CMD_TYPE_ANTS: u8 = 2;
const CMD_TYPE_USAGE: u8 = 3;
const CMD_TYPE_CANCEL: u8 = 4;
const CMD_TYPE_CANCEL_ALL: u8 = 5;
const CMD_TYPE_NEW: u8 = 6;
const CMD_TYPE_STATUS: u8 = 7;
const CMD_TYPE_FOLLOWUP: u8 = 8;
const CMD_TYPE_ANALYSE: u8 = 9;
const CMD_TYPE_REFLECT: u8 = 10;

pub struct ConductorSentant {
    plugin_id: PluginId,
}

impl ConductorSentant {
    pub fn new(plugin_id: PluginId) -> Self {
        Self { plugin_id }
    }

    /// Encode a small CBOR payload: { 0: uint(task_id), 1: uint(chat_id) }
    fn encode_ids(task_id: u32, chat_id: i64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(16);
        buf.push(0xA2); // map(2)
        buf.push(0x00); // key 0
        encode_uint_into(&mut buf, task_id as u64);
        buf.push(0x01); // key 1
        encode_uint_into(&mut buf, chat_id as u64);
        buf
    }

    /// Encode: { 1: uint(chat_id) }
    fn encode_chat(chat_id: i64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(12);
        buf.push(0xA1); // map(1)
        buf.push(0x01); // key 1
        encode_uint_into(&mut buf, chat_id as u64);
        buf
    }
}

impl Sentant for ConductorSentant {
    fn handle_event(&mut self, event: &Event, actions: &mut ActionBuf) {
        match event.hash {
            RELAY_COMMAND => {
                // Payload: { 0: uint(cmd_type), 1: uint(chat_id), 2: uint(cancel_task_id) }
                let cmd_type = decode_uint_key(event.payload, 0).unwrap_or(0) as u8;
                let chat_id = decode_uint_key(event.payload, 1).unwrap_or(0) as i64;

                match cmd_type {
                    CMD_TYPE_MESSAGE => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_DISPATCH, &payload,
                        ));
                    }
                    CMD_TYPE_HELP => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_HELP, &payload,
                        ));
                    }
                    CMD_TYPE_ANTS => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_ANTS, &payload,
                        ));
                    }
                    CMD_TYPE_USAGE => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_USAGE, &payload,
                        ));
                    }
                    CMD_TYPE_CANCEL => {
                        let cancel_id = decode_uint_key(event.payload, 2).unwrap_or(0) as u32;
                        let payload = Self::encode_ids(cancel_id, chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_CANCEL, &payload,
                        ));
                    }
                    CMD_TYPE_CANCEL_ALL => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_CANCEL_ALL, &payload,
                        ));
                    }
                    CMD_TYPE_NEW => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_NEW_SESSION, &payload,
                        ));
                    }
                    CMD_TYPE_STATUS => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_STATUS, &payload,
                        ));
                    }
                    CMD_TYPE_FOLLOWUP => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_FOLLOWUP, &payload,
                        ));
                    }
                    CMD_TYPE_ANALYSE => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_ANALYSE, &payload,
                        ));
                    }
                    CMD_TYPE_REFLECT => {
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_REFLECT, &payload,
                        ));
                    }
                    _ => {
                        // Unknown command type — treat as message.
                        let payload = Self::encode_chat(chat_id);
                        actions.push(Action::plugin_call(
                            self.plugin_id, cli_plugin::CMD_DISPATCH, &payload,
                        ));
                    }
                }
            }

            RELAY_AI_READY => {
                // Response is ready — tell plugin to pop and send.
                let chat_id = decode_uint_key(event.payload, 1).unwrap_or(0) as i64;
                let payload = Self::encode_chat(chat_id);
                actions.push(Action::plugin_call(
                    self.plugin_id, cli_plugin::CMD_REPLY, &payload,
                ));
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
