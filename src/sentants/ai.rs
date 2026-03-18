//! AI sentant — pure FSM for NL→command→summary mediation.
//!
//! States: Idle(0) → Translating(1) → Executing(2) → Summarising(3) → Idle
//!
//! No I/O, no channels, no shared state. All data handling is in AiMediationPlugin.

use r2_engine::action::Action;
use r2_engine::action_buf::ActionBuf;
use r2_engine::event::{Event, Target};
use r2_engine::plugin::PluginId;
use r2_engine::sentant::{Sentant, StateId};

use crate::events::*;
use crate::plugins::ai as ai_plugin;

const STATE_IDLE: StateId = 0;
const STATE_TRANSLATING: StateId = 1;
const STATE_EXECUTING: StateId = 2;
const STATE_SUMMARISING: StateId = 3;

const COLLECT_DEBOUNCE_MS: u32 = 2000;

pub struct AiSentant {
    state: StateId,
    chat_id: i64,
    ai_plugin_id: PluginId,
    #[allow(dead_code)]
    pty_plugin_id: PluginId,
}

impl AiSentant {
    pub fn new(ai_plugin_id: PluginId, pty_plugin_id: PluginId) -> Self {
        Self {
            state: STATE_IDLE,
            chat_id: 0,
            ai_plugin_id,
            pty_plugin_id,
        }
    }

    fn encode_chat(chat_id: i64) -> Vec<u8> {
        let mut buf = Vec::with_capacity(12);
        buf.push(0xA1);
        buf.push(0x01);
        encode_uint_into(&mut buf, chat_id as u64);
        buf
    }
}

impl Sentant for AiSentant {
    fn handle_event(&mut self, event: &Event, actions: &mut ActionBuf) {
        match event.hash {
            RELAY_COMMAND => {
                if self.state != STATE_IDLE {
                    let chat_id = decode_uint_key(event.payload, 1).unwrap_or(0) as i64;
                    let payload = Self::encode_chat(chat_id);
                    // Tell plugin to send "busy" message.
                    actions.push(Action::plugin_call(
                        self.ai_plugin_id, ai_plugin::CMD_NO_OUTPUT, &payload,
                    ));
                    return;
                }

                let chat_id = decode_uint_key(event.payload, 1).unwrap_or(0) as i64;
                self.chat_id = chat_id;

                // Tell plugin to translate the user input.
                let payload = Self::encode_chat(chat_id);
                actions.push(Action::plugin_call(
                    self.ai_plugin_id, ai_plugin::CMD_TRANSLATE, &payload,
                ));
                self.state = STATE_TRANSLATING;
            }

            RELAY_AI_READY => {
                let kind = decode_uint_key(event.payload, 0).unwrap_or(0);

                match self.state {
                    STATE_TRANSLATING if kind == 0 => {
                        // Translation ready — pop it, emit RELAY_INPUT with the command.
                        let payload = Self::encode_chat(self.chat_id);
                        actions.push(Action::plugin_call(
                            self.ai_plugin_id, ai_plugin::CMD_POP_REPLY, &payload,
                        ));
                        // The plugin returns the command text in PluginResponse.
                        // We need to emit RELAY_INPUT — but we don't have the text here.
                        // Instead, encode a short payload with chat_id for RELAY_INPUT.
                        // The terminal sentant will get the command from the event.
                        // TODO: The plugin should emit RELAY_INPUT directly for large commands.
                        // For now, use a fixed-size payload.
                        self.state = STATE_EXECUTING;
                    }

                    STATE_SUMMARISING if kind == 1 => {
                        // Summary ready — send to Telegram.
                        let payload = Self::encode_chat(self.chat_id);
                        actions.push(Action::plugin_call(
                            self.ai_plugin_id, ai_plugin::CMD_SEND_TEXT, &payload,
                        ));
                        self.state = STATE_IDLE;
                    }

                    _ => {}
                }
            }

            RELAY_OUTPUT => {
                if self.state != STATE_EXECUTING {
                    return;
                }

                // Tell plugin to accumulate output.
                actions.push(Action::plugin_call(
                    self.ai_plugin_id, ai_plugin::CMD_APPEND_OUTPUT, event.payload,
                ));

                // Schedule debounce.
                let payload = Self::encode_chat(self.chat_id);
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

                // Tell plugin to summarise the accumulated output.
                let payload = Self::encode_chat(self.chat_id);
                actions.push(Action::plugin_call(
                    self.ai_plugin_id, ai_plugin::CMD_SUMMARISE, &payload,
                ));
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
